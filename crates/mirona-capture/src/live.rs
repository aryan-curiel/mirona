//! The live recorder (NGR-137/139): device audio in, per-source chunks to one STT worker,
//! events out, mixed recording to disk. Devices are in `macos.rs`; everything here is
//! platform-independent so it is tested on every OS by feeding it audio directly.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use mirona_providers::SttProvider;

use crate::{
    Chunk, Chunker, Event, Resampler, SAMPLE_RATE, Source, VAD_MODEL, VadConfig, transcribe_chunk,
};

pub struct RecorderConfig {
    /// Holds `silero_vad.onnx` (see `scripts/fetch-models.sh`).
    pub models_dir: PathBuf,
    pub vad: VadConfig,
    /// The mixed recording (16 kHz mono 16-bit WAV); created, or overwritten.
    pub recording: PathBuf,
    pub mic: bool,
    pub system: bool,
}

/// What device callbacks send to the audio thread.
pub(crate) enum Msg {
    Audio {
        source: Source,
        rate: u32,
        mono: Vec<f32>,
    },
    /// The stream died (device unplugged…); the audio thread rebuilds it.
    Lost {
        source: Source,
        message: String,
    },
    Stop,
}

type Threads = (JoinHandle<Result<(), String>>, JoinHandle<()>);

/// A running recording. Drop it (or call [`Recorder::stop`]) to end it.
pub struct Recorder {
    tx: Sender<Msg>,
    /// The audio thread and the STT worker.
    threads: Option<Threads>,
}

impl Recorder {
    /// Start capturing. `stt` is shared so the model stays loaded between meetings.
    /// Fails up front on a bad config (missing VAD model, unwritable recording path), a missing
    /// permission (ask first with `request_mic_permission` / `request_system_audio_permission`,
    /// off the UI thread: they block until answered) or an unsupported OS. Device problems
    /// arrive later as [`Event::Error`]s and are retried.
    pub fn start<S: SttProvider + Send + Sync + 'static>(
        cfg: RecorderConfig,
        stt: Arc<S>,
        events: Sender<Event>,
    ) -> Result<Self, String> {
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (cfg, stt, events);
            Err("live capture is only supported on macOS for now".into())
        }
        #[cfg(target_os = "macos")]
        {
            crate::macos::check_permissions(&cfg)?;
            let (chunk_tx, chunk_rx) = std::sync::mpsc::channel();
            let pipeline = Pipeline::new(&cfg, chunk_tx)?;
            let (tx, rx) = std::sync::mpsc::channel();
            let stt_events = events.clone();
            let worker = std::thread::Builder::new()
                .name("mirona-stt".into())
                .spawn(move || stt_worker(&*stt, &chunk_rx, &stt_events))
                .map_err(|e| e.to_string())?;
            let audio_tx = tx.clone();
            let audio = std::thread::Builder::new()
                .name("mirona-audio".into())
                .spawn(move || crate::macos::run(&cfg, pipeline, &rx, &audio_tx, &events))
                .map_err(|e| e.to_string())?;
            Ok(Self {
                tx,
                threads: Some((audio, worker)),
            })
        }
    }

    /// Stop capture, transcribe the last chunk and finalise the recording. Every segment has
    /// been sent by the time this returns.
    pub fn stop(mut self) -> Result<(), String> {
        self.finish()
    }

    fn finish(&mut self) -> Result<(), String> {
        let _ = self.tx.send(Msg::Stop);
        let Some((audio, worker)) = self.threads.take() else {
            return Ok(());
        };
        let res = audio
            .join()
            .map_err(|_| "audio thread panicked".to_owned())?;
        worker.join().map_err(|_| "STT thread panicked")?;
        res
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

/// One worker for both sources, in arrival order: one decode holds the CPU at a time
/// (ADR 0001), and each source's segments stay in order. Ends when the pipeline is dropped.
pub(crate) fn stt_worker<S: SttProvider>(
    stt: &S,
    chunks: &Receiver<(Source, Chunk)>,
    events: &Sender<Event>,
) {
    for (source, chunk) in chunks {
        let ev = match transcribe_chunk(stt, source, &chunk) {
            Ok(Some(seg)) => Event::Segment(seg),
            Ok(None) => continue,
            Err(message) => Event::Error {
                source: Some(source),
                message,
            },
        };
        let _ = events.send(ev);
    }
}

/// One source on its way to 16 kHz chunks and the mix.
struct Lane {
    source: Source,
    resampler: Resampler,
    chunker: Chunker,
    /// 16 kHz samples seen so far: this source's clock.
    samples: u64,
    /// Not yet mixed into the recording.
    unmixed: Vec<f32>,
    sum_sq: f32,
    level_n: u32,
}

impl Lane {
    fn feed(&mut self, pcm: &[f32], chunks: &Sender<(Source, Chunk)>) {
        self.samples += pcm.len() as u64;
        self.unmixed.extend_from_slice(pcm);
        self.sum_sq += pcm.iter().map(|x| x * x).sum::<f32>();
        self.level_n += u32::try_from(pcm.len()).unwrap_or(u32::MAX);
        for c in self.chunker.push(pcm) {
            let _ = chunks.send((self.source, c));
        }
    }

    fn level(&mut self) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        let rms = (self.sum_sq / self.level_n.max(1) as f32).sqrt();
        (self.sum_sq, self.level_n) = (0.0, 0);
        rms
    }
}

/// Resample, chunk and mix both sources; the device side only pushes audio into it.
pub(crate) struct Pipeline {
    lanes: Vec<Lane>,
    wav: hound::WavWriter<BufWriter<File>>,
    chunks: Sender<(Source, Chunk)>,
}

impl Pipeline {
    pub(crate) fn new(
        cfg: &RecorderConfig,
        chunks: Sender<(Source, Chunk)>,
    ) -> Result<Self, String> {
        let vad = cfg.models_dir.join(VAD_MODEL);
        let mut lanes = Vec::new();
        for (on, source) in [(cfg.mic, Source::Mic), (cfg.system, Source::System)] {
            if on {
                lanes.push(Lane {
                    source,
                    resampler: Resampler::default(),
                    chunker: Chunker::new(&vad, cfg.vad)?,
                    samples: 0,
                    unmixed: Vec::new(),
                    sum_sq: 0.0,
                    level_n: 0,
                });
            }
        }
        if lanes.is_empty() {
            return Err("nothing to record: enable the mic, system audio or both".into());
        }
        Ok(Self {
            lanes,
            wav: create_wav(&cfg.recording)?,
            chunks,
        })
    }

    pub(crate) fn push(&mut self, source: Source, rate: u32, mono: &[f32]) {
        let Some(lane) = self.lanes.iter_mut().find(|l| l.source == source) else {
            return;
        };
        let pcm = lane.resampler.to_16k(rate, mono);
        lane.feed(&pcm, &self.chunks);
        self.mix(false);
    }

    /// Pad sources that fell behind the wall clock (device switch, a tap that only sends while
    /// something plays, slow start) with silence, so timestamps and the mix stay aligned.
    pub(crate) fn catch_up(&mut self, elapsed: Duration) {
        let now =
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX) * u64::from(SAMPLE_RATE) / 1000;
        let slack = u64::from(SAMPLE_RATE / 2);
        for lane in &mut self.lanes {
            if lane.samples + slack < now {
                let gap = usize::try_from(now - lane.samples).unwrap_or(0);
                lane.feed(&vec![0.0; gap], &self.chunks);
            }
        }
        self.mix(false);
        let _ = self.wav.flush(); // rewrites the header: a crash keeps a playable file
    }

    /// RMS since the last call, for the mic and the system lane (0 when off).
    pub(crate) fn levels(&mut self) -> (f32, f32) {
        let mut level = |s| {
            self.lanes
                .iter_mut()
                .find(|l| l.source == s)
                .map_or(0.0, Lane::level)
        };
        (level(Source::Mic), level(Source::System))
    }

    /// Write the stretch every source has reached (all of it at the end, padding with silence).
    fn mix(&mut self, all: bool) {
        let lens = self.lanes.iter().map(|l| l.unmixed.len());
        let n = if all { lens.max() } else { lens.min() }.unwrap_or(0);
        for i in 0..n {
            let s: f32 = self
                .lanes
                .iter()
                .map(|l| l.unmixed.get(i).copied().unwrap_or(0.0))
                .sum();
            #[allow(clippy::cast_possible_truncation)]
            let _ = self.wav.write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16);
        }
        for l in &mut self.lanes {
            l.unmixed.drain(..n.min(l.unmixed.len()));
        }
    }

    /// Flush the last chunks to the STT worker and finalise the recording.
    pub(crate) fn finish(mut self) -> Result<(), String> {
        for lane in &mut self.lanes {
            for c in lane.chunker.flush() {
                let _ = self.chunks.send((lane.source, c));
            }
        }
        self.mix(true);
        self.wav.finalize().map_err(|e| e.to_string())
    }
}

// 16 kHz mono WAV while recording (~115 MB/hour; crash-safe through the header rewrite in
// `catch_up`), compressed to Opus in CAF once the meeting ends (`crate::compress`, ~10 MB/hour).
fn create_wav(path: &Path) -> Result<hound::WavWriter<BufWriter<File>>, String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    hound::WavWriter::create(path, spec).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{fixture, fixture_pcm};
    use mirona_providers::fake::Fake;
    use std::sync::mpsc::channel;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("mirona-capture-{}-{name}", std::process::id()))
    }

    /// The fixture as "them" at 48 kHz stereo-ish chunks and silence as "me": what a call sounds
    /// like to the pipeline, without a device.
    #[test]
    fn pipeline_chunks_per_source_and_writes_the_mix() {
        let path = temp("mix.wav");
        let cfg = RecorderConfig {
            models_dir: fixture(""),
            vad: VadConfig::default(),
            recording: path.clone(),
            mic: true,
            system: true,
        };
        let (chunk_tx, chunk_rx) = channel();
        let mut p = Pipeline::new(&cfg, chunk_tx).unwrap();
        let speech = fixture_pcm();
        // System audio at 16 kHz in 10 ms callbacks; the mic sends silence at 48 kHz.
        for block in speech.chunks(160) {
            p.push(Source::System, 16_000, block);
            p.push(Source::Mic, 48_000, &[0.0; 480]);
        }
        let (mic, system) = p.levels();
        assert!(mic < 1e-6 && system > 0.01, "levels {mic} {system}");
        p.finish().unwrap();

        let (ev_tx, ev_rx) = channel();
        stt_worker(&Fake("fake"), &chunk_rx, &ev_tx);
        drop(ev_tx);
        let segs: Vec<_> = ev_rx
            .iter()
            .filter_map(|e| match e {
                Event::Segment(s) => Some(s),
                _ => None,
            })
            .collect();
        assert_eq!(segs.len(), 2, "{segs:?}");
        assert!(segs.iter().all(|s| s.source == Source::System));
        assert!(segs[0].end_ms <= segs[1].start_ms);

        let wav = hound::WavReader::open(&path).unwrap();
        let ms = wav.duration() as usize / 16;
        assert!((9_400..9_600).contains(&ms), "{ms} ms");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_stalled_source_is_padded_to_the_wall_clock() {
        let path = temp("pad.wav");
        let cfg = RecorderConfig {
            models_dir: fixture(""),
            vad: VadConfig::default(),
            recording: path.clone(),
            mic: true,
            system: true,
        };
        let (chunk_tx, _chunk_rx) = channel();
        let mut p = Pipeline::new(&cfg, chunk_tx).unwrap();
        p.push(Source::Mic, 16_000, &vec![0.1; 32_000]); // 2 s of mic, no system audio
        p.catch_up(Duration::from_secs(2));
        p.finish().unwrap();
        let secs = hound::WavReader::open(&path).unwrap().duration() / 16_000;
        assert_eq!(secs, 2);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn bad_configs_fail_at_start() {
        let cfg = |models_dir, mic| RecorderConfig {
            models_dir,
            vad: VadConfig::default(),
            recording: temp("never.wav"),
            mic,
            system: false,
        };
        let (tx, _rx) = channel();
        let e = Pipeline::new(&cfg(fixture(""), false), tx.clone())
            .err()
            .unwrap();
        assert!(e.contains("nothing to record"), "{e}");
        let e = Pipeline::new(&cfg(PathBuf::from("/nonexistent"), true), tx)
            .err()
            .unwrap();
        assert!(e.contains("VAD model missing"), "{e}");
        #[cfg(not(target_os = "macos"))]
        {
            let (etx, _erx) = channel();
            let e = Recorder::start(cfg(fixture(""), true), Arc::new(Fake("f")), etx).err();
            assert!(e.unwrap().contains("only supported on macOS"));
        }
    }

    /// Plays the fixture through the speakers while recording: the system lane must transcribe
    /// it. Run it locally; the terminal app needs Microphone and System Audio Recording access.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs a mic, speakers, permissions and models"]
    fn records_mic_and_system_audio() {
        use crate::tests::{REFERENCE, test_models_dir, wer};
        use std::time::Instant;

        println!(
            "permissions: mic {:?}, system audio {:?}",
            crate::mic_permission(),
            crate::system_audio_permission()
        );
        let dir = test_models_dir();
        let stt = Arc::new(crate::Stt::load("parakeet", &dir).unwrap());
        let path = temp("live.wav");
        let (tx, rx) = channel();
        let t0 = Instant::now();
        let rec = Recorder::start(
            RecorderConfig {
                models_dir: dir,
                vad: VadConfig::default(),
                recording: path.clone(),
                // MIRONA_TEST_MIC=0 records system audio only.
                mic: std::env::var("MIRONA_TEST_MIC").as_deref() != Ok("0"),
                system: true,
            },
            stt,
            tx,
        )
        .unwrap();
        // Timestamp events as they arrive, for latency.
        let collector =
            std::thread::spawn(move || rx.iter().map(|e| (t0.elapsed(), e)).collect::<Vec<_>>());
        std::thread::sleep(Duration::from_secs(1));
        let played = std::process::Command::new("afplay")
            .arg(fixture("librispeech-2utt.wav"))
            .status()
            .unwrap();
        assert!(played.success());
        std::thread::sleep(Duration::from_millis(1500));
        let stopping = Instant::now();
        rec.stop().unwrap();
        println!("stop took {:?}", stopping.elapsed());
        let events = collector.join().unwrap();

        let (mut max_mic, mut max_sys) = (0f32, 0f32);
        let mut system_text = Vec::new();
        for (at, e) in &events {
            match e {
                Event::Levels { mic, system } => {
                    max_mic = max_mic.max(*mic);
                    max_sys = max_sys.max(*system);
                }
                Event::Segment(s) => {
                    let lag =
                        i64::try_from(at.as_millis()).unwrap_or(i64::MAX) - i64::from(s.end_ms);
                    println!(
                        "{:?} {}-{} ms (+{lag} ms after speech end): {}",
                        s.source, s.start_ms, s.end_ms, s.text
                    );
                    if s.source == Source::System {
                        system_text.push(s.text.clone());
                    }
                }
                other => println!("{other:?}"),
            }
        }
        println!("peak levels: mic {max_mic:.3}, system {max_sys:.3}");
        let secs = hound::WavReader::open(&path).unwrap().duration() / 16_000;
        println!("recording {} s at {}", secs, path.display());
        let wer = wer(REFERENCE, &system_text.join(" "));
        println!("system audio WER {:.1}%", 100.0 * wer);
        assert!(wer < 0.25, "system audio WER {wer}: {system_text:?}");
        assert!((11..=14).contains(&secs), "{secs} s");
    }
}

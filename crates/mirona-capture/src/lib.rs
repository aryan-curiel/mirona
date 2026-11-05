//! Audio capture (mic + system), resampling, VAD chunking and live transcription (NGR-137/138/139).
//!
//! - Live: [`Recorder::start`] captures the mic ("me") and system audio ("them") on macOS,
//!   resamples each to 16 kHz mono, VAD-chunks and transcribes each source separately (so the
//!   speaker is known from the source until diarization lands in M2), sends [`Event`]s, and
//!   writes a mixed recording. [`Recorder::stop`] flushes the last chunk and finalises the file.
//! - Offline: [`transcribe_file`] runs an audio file (wav/mp3/m4a/flac, our Opus CAF) through
//!   the same path. [`compress`] turns a finished recording's WAV into Opus in CAF.
//! - STT: [`Stt::load`] picks the provider from config (`[providers] stt`), Parakeet by default.
//!
//! Models live in one dir laid out by `scripts/fetch-models.sh`.

mod chunk;
mod compress;
mod live;
#[cfg(target_os = "macos")]
mod macos;

use std::path::Path;

pub use chunk::{Chunk, Chunker, Resampler, SAMPLE_RATE, VadConfig, downmix};
pub use compress::compress;
pub use live::{Recorder, RecorderConfig};
use mirona_core::Locality;
#[cfg(feature = "whisper")]
use mirona_providers::whisper::Whisper;
use mirona_providers::{Provider, SttProvider, parakeet::Parakeet};

/// Silero VAD, in the models dir.
pub const VAD_MODEL: &str = "silero_vad.onnx";
/// Parakeet-TDT 0.6B v2 int8 (sherpa-onnx export), in the models dir.
pub const PARAKEET_DIR: &str = "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8";
/// Whisper models in order of preference; the first one present is used.
pub const WHISPER_MODELS: [&str; 2] = ["ggml-small.en.bin", "ggml-base.en.bin"];

/// Where audio came from. Until diarization (M2) it also says who spoke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// The local microphone: "me".
    Mic,
    /// Everything the Mac plays (the call app): "them".
    System,
    /// An imported audio file.
    File,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub source: Source,
    /// From the start of the recording (or file).
    pub start_ms: u32,
    pub end_ms: u32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// RMS level (0..1) of the last ~100 ms per source, for meters.
    Levels { mic: f32, system: f32 },
    /// Transcribed speech. Segments of one source arrive in order.
    Segment(Segment),
    /// Capture (re)started on this device: at start, and after a device change mid-call.
    Device { source: Source, name: String },
    /// Something failed; capture keeps going where it can and retries lost devices.
    Error {
        source: Option<Source>,
        message: String,
    },
}

/// A loaded speech-to-text model, chosen by config id. Load once and share it between meetings.
pub enum Stt {
    Parakeet(Parakeet),
    #[cfg(feature = "whisper")]
    Whisper(Whisper),
}

impl Stt {
    /// `id` is `[providers] stt` from config: `parakeet` (default) or `whisper`.
    pub fn load(id: &str, models_dir: &Path) -> Result<Self, String> {
        let threads = 4; // ADR 0001
        match id {
            "parakeet" => Parakeet::new(&models_dir.join(PARAKEET_DIR), threads)
                .map(Self::Parakeet)
                .map_err(|e| e.to_string()),
            #[cfg(feature = "whisper")]
            "whisper" => {
                let model = WHISPER_MODELS
                    .iter()
                    .map(|m| models_dir.join(m))
                    .find(|p| p.is_file())
                    .ok_or_else(|| {
                        format!(
                            "no Whisper model ({WHISPER_MODELS:?}) in {}",
                            models_dir.display()
                        )
                    })?;
                Whisper::new(&model, threads)
                    .map(Self::Whisper)
                    .map_err(|e| e.to_string())
            }
            #[cfg(not(feature = "whisper"))]
            "whisper" => Err("this build has no Whisper support (feature `whisper`)".into()),
            other => Err(format!("unknown speech-to-text provider {other}")),
        }
    }
}

impl Provider for Stt {
    fn id(&self) -> &str {
        match self {
            Self::Parakeet(p) => p.id(),
            #[cfg(feature = "whisper")]
            Self::Whisper(w) => w.id(),
        }
    }
    fn locality(&self) -> Locality {
        Locality::Local
    }
}

impl SttProvider for Stt {
    async fn transcribe(&self, pcm: &[f32]) -> mirona_providers::Result<String> {
        match self {
            Self::Parakeet(p) => p.transcribe(pcm).await,
            #[cfg(feature = "whisper")]
            Self::Whisper(w) => w.transcribe(pcm).await,
        }
    }
}

/// Transcribe one chunk; empty text (noise the VAD let through) gives `None`.
fn transcribe_chunk<S: SttProvider>(
    stt: &S,
    source: Source,
    chunk: &Chunk,
) -> Result<Option<Segment>, String> {
    let text = block_on(stt.transcribe(&chunk.pcm)).map_err(|e| e.to_string())?;
    Ok((!text.trim().is_empty()).then(|| Segment {
        source,
        start_ms: chunk.start_ms,
        end_ms: chunk.end_ms,
        text: text.trim().to_owned(),
    }))
}

/// Transcribe an audio file offline through the same VAD + STT path as live capture.
/// Decodes and transcribes as it goes, so memory stays flat for long files.
///
/// `on_progress` gets the share of the file transcribed so far (0..=1, rising in 1 % steps,
/// 1.0 once done), or `None` when the file doesn't say how long it is.
pub fn transcribe_file<S: SttProvider>(
    path: &Path,
    models_dir: &Path,
    vad: VadConfig,
    stt: &S,
    on_progress: &mut dyn FnMut(Option<f32>),
) -> Result<Vec<Segment>, String> {
    let mut chunker = Chunker::new(&models_dir.join(VAD_MODEL), vad)?;
    let mut resampler = Resampler::default();
    let mut chunks = Vec::new();
    let mut out = Vec::new();
    let mut run = |chunks: &mut Vec<Chunk>| -> Result<(), String> {
        for c in chunks.drain(..) {
            out.extend(transcribe_chunk(stt, Source::File, &c)?);
        }
        Ok(())
    };
    let (mut decoded, mut reported) = (0u64, None::<u64>);
    decode_inner(path, &mut |rate, mono, total| {
        chunks.extend(chunker.push(&resampler.to_16k(rate, mono)));
        run(&mut chunks)?;
        decoded += mono.len() as u64;
        // Percent of the file, capped below 100 until the last chunk is transcribed too.
        let pct = total
            .filter(|t| *t > 0)
            .map(|t| (decoded * 100 / t).min(99));
        if reported.is_none() || pct > reported {
            reported = Some(pct.unwrap_or(0));
            #[allow(clippy::cast_precision_loss)]
            on_progress(pct.map(|p| p as f32 / 100.0));
        }
        Ok(())
    })?;
    chunks.extend(chunker.flush());
    run(&mut chunks)?;
    on_progress(Some(1.0));
    Ok(out)
}

/// Receives decoded audio: sample rate and a block of mono samples.
pub type OnAudio<'a> = dyn FnMut(u32, &[f32]) -> Result<(), String> + 'a;

/// Decode any supported file, handing mono f32 blocks and their sample rate to `on_audio`.
pub fn decode(path: &Path, on_audio: &mut OnAudio<'_>) -> Result<(), String> {
    decode_inner(path, &mut |rate, mono, _| on_audio(rate, mono))
}

/// [`OnAudio`] plus the file's length in frames, when its header gives one.
pub(crate) type OnAudioOf<'a> = dyn FnMut(u32, &[f32], Option<u64>) -> Result<(), String> + 'a;

/// [`decode`], also passing the file's length in frames when its header gives one.
fn decode_inner(path: &Path, on_audio: &mut OnAudioOf<'_>) -> Result<(), String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
    use symphonia::core::errors::Error as E;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    if compress::is_caf(path) {
        return compress::decode_caf(path, on_audio); // Opus: symphonia has no decoder for it
    }
    let err = |e: E| format!("{}: {e}", path.display());
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .format(
            &hint,
            MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default()),
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(err)?
        .format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| format!("{}: no audio track", path.display()))?;
    let track_id = track.id;
    let total = track.codec_params.n_frames;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(err)?;
    let mut buf: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(E::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(err(e)),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let audio = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(E::DecodeError(_)) => continue, // a corrupt packet: skip it, like players do
            Err(e) => return Err(err(e)),
        };
        let spec = *audio.spec();
        let frames = audio.capacity() as u64;
        let b = match &mut buf {
            Some(b) if b.capacity() >= audio.capacity() * spec.channels.count() => b,
            _ => buf.insert(SampleBuffer::new(frames, spec)),
        };
        b.copy_interleaved_ref(audio);
        on_audio(
            spec.rate,
            &downmix(b.samples(), spec.channels.count()),
            total,
        )?;
    }
    Ok(())
}

/// Run a future to completion on this thread (the STT worker, never an async executor).
fn block_on<F: Future>(fut: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl std::task::Wake for Unpark {
        fn wake(self: std::sync::Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = std::sync::Arc::new(Unpark(std::thread::current())).into();
    let mut cx = std::task::Context::from_waker(&waker);
    let mut fut = std::pin::pin!(fut);
    loop {
        if let std::task::Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::park();
    }
}

/// Microphone or system-audio permission, as macOS reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Granted,
    Denied,
    /// Never asked: the first capture (or a `request_*` call) shows the prompt.
    NotDetermined,
}

/// Microphone access. Outside macOS there is no OS prompt and this says `Granted`.
pub fn mic_permission() -> Permission {
    #[cfg(target_os = "macos")]
    return macos::mic_permission();
    #[cfg(not(target_os = "macos"))]
    Permission::Granted
}

/// Show the microphone prompt if never asked; blocks until answered. True if granted.
pub fn request_mic_permission() -> bool {
    #[cfg(target_os = "macos")]
    return macos::request_mic_permission();
    #[cfg(not(target_os = "macos"))]
    true
}

/// System-audio recording access ("Screen & System Audio Recording" › "System Audio Recording
/// Only"). Without it macOS hands the tap silence instead of an error, so check before recording.
pub fn system_audio_permission() -> Permission {
    #[cfg(target_os = "macos")]
    return macos::system_audio_permission();
    #[cfg(not(target_os = "macos"))]
    Permission::Granted
}

/// Show the system-audio prompt if never asked; blocks until answered. True if granted.
pub fn request_system_audio_permission() -> bool {
    #[cfg(target_os = "macos")]
    return macos::request_system_audio_permission();
    #[cfg(not(target_os = "macos"))]
    true
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use mirona_providers::fake::Fake;
    use std::path::PathBuf;

    /// `MIRONA_MODELS_DIR`, else the NGR-128 spike's models.
    pub fn test_models_dir() -> PathBuf {
        std::env::var_os("MIRONA_MODELS_DIR").map_or_else(
            || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spikes/parakeet/models"),
            PathBuf::from,
        )
    }

    pub fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    /// Two `LibriSpeech` test-clean utterances (CC-BY-4.0), 16 kHz mono, 9.5 s.
    pub fn fixture_pcm() -> Vec<f32> {
        hound::WavReader::open(fixture("librispeech-2utt.wav"))
            .unwrap()
            .samples::<i16>()
            .map(|s| f32::from(s.unwrap()) / 32768.0)
            .collect()
    }

    pub const REFERENCE: &str = "concord returned to its place amidst the tents \
        congratulations were poured in upon the princess everywhere during her journey";

    fn words(s: &str) -> Vec<String> {
        s.to_lowercase()
            .replace('-', " ")
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '\'')
            .collect::<String>()
            .split_whitespace()
            .map(String::from)
            .collect()
    }

    /// Word error rate: word-level Levenshtein distance / reference length.
    #[allow(clippy::cast_precision_loss)]
    pub fn wer(reference: &str, hyp: &str) -> f64 {
        let (r, h) = (words(reference), words(hyp));
        let mut prev: Vec<usize> = (0..=h.len()).collect();
        for (i, rw) in r.iter().enumerate() {
            let mut cur = vec![i + 1; h.len() + 1];
            for (j, hw) in h.iter().enumerate() {
                cur[j + 1] = (prev[j] + usize::from(rw != hw))
                    .min(prev[j + 1] + 1)
                    .min(cur[j] + 1);
            }
            prev = cur;
        }
        prev[h.len()] as f64 / r.len().max(1) as f64
    }

    #[test]
    fn decodes_wav_mp3_m4a_to_the_same_speech() {
        for name in [
            "librispeech-2utt.wav",
            "librispeech-2utt.mp3",
            "librispeech-2utt.m4a",
        ] {
            let mut rs = Resampler::default();
            let mut pcm = Vec::new();
            decode(&fixture(name), &mut |rate, m| {
                pcm.extend(rs.to_16k(rate, m));
                Ok(())
            })
            .unwrap();
            #[allow(clippy::cast_precision_loss)]
            let secs = pcm.len() as f32 / 16_000.0;
            // Encoders add a little padding.
            assert!((9.4..9.7).contains(&secs), "{name}: {secs} s");
            let mut progress = vec![];
            let found = transcribe_file(
                &fixture(name),
                &fixture(""),
                VadConfig {
                    max_chunk_s: 10.0,
                    ..VadConfig::default()
                },
                &Fake("fake"),
                &mut |p| progress.push(p),
            )
            .unwrap();
            assert_eq!(found.len(), 2, "{name}: {found:?}");
            // Known length: rises step by step, ends at 1.0. Unknown: None, then 1.0.
            let known: Vec<f32> = progress.iter().flatten().copied().collect();
            assert!(
                known.windows(2).all(|w| w[0] < w[1]),
                "{name}: {progress:?}"
            );
            assert_eq!(progress.last(), Some(&Some(1.0)), "{name}");
            println!("{name}: {} progress steps", progress.len());
            assert!(found.iter().all(|s| s.source == Source::File));
        }
    }

    #[test]
    fn missing_file_and_unknown_provider_are_errors() {
        let e = transcribe_file(
            Path::new("/nope.wav"),
            &fixture(""),
            VadConfig::default(),
            &Fake("fake"),
            &mut |_| {},
        );
        assert!(e.unwrap_err().contains("/nope.wav"));
        assert!(Stt::load("nope", Path::new("/")).is_err());
        let Err(e) = Stt::load("parakeet", Path::new("/nonexistent")) else {
            panic!("loaded parakeet from nowhere");
        };
        assert!(e.contains("encoder.int8.onnx"), "{e}");
    }

    #[test]
    fn wer_helper() {
        assert!(wer("the cat sat", "The cat, sat.") < f64::EPSILON);
        assert!((wer("the cat sat down", "the bat sat") - 0.5).abs() < 1e-9);
    }

    /// Needs the models: `MIRONA_MODELS_DIR` (or the NGR-128 spike's `spikes/parakeet/models`).
    #[test]
    #[ignore = "needs models (MIRONA_MODELS_DIR)"]
    fn imports_a_file_with_each_provider() {
        let dir = test_models_dir();
        let mut ids = vec!["parakeet"];
        if cfg!(feature = "whisper") {
            ids.push("whisper");
        }
        for id in ids {
            let stt = Stt::load(id, &dir).unwrap();
            for name in ["librispeech-2utt.wav", "librispeech-2utt.m4a"] {
                let t = std::time::Instant::now();
                let segs = transcribe_file(
                    &fixture(name),
                    &dir,
                    VadConfig::default(),
                    &stt,
                    &mut |_| {},
                )
                .unwrap();
                let text: Vec<&str> = segs.iter().map(|s| s.text.as_str()).collect();
                let wer = wer(REFERENCE, &text.join(" "));
                println!(
                    "{id} {name}: {} segments in {:?}, WER {:.1}%: {text:?}",
                    segs.len(),
                    t.elapsed(),
                    100.0 * wer
                );
                // Whisper small.en stumbles on the short first utterance in the 32 kbps m4a.
                let max = if id == "parakeet" { 0.1 } else { 0.25 };
                assert!(wer < max, "{id} {name}: WER {wer}");
            }
        }
    }

    /// The NGR-128 spike's 2-minute clips (`spikes/parakeet/fetch.sh`): long input, ADR numbers.
    #[test]
    #[ignore = "needs models and spikes/parakeet/audio"]
    fn imports_the_two_minute_spike_clips() {
        let dir = test_models_dir();
        let audio = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spikes/parakeet/audio");
        let stt = Stt::load("parakeet", &dir).unwrap();
        // Spike WER at VAD <= 5 s: 0.9 / 3.1 / 16.6 %.
        for (clip, max_wer) in [
            ("librispeech-clean", 0.02),
            ("librispeech-other", 0.05),
            ("ami-meeting", 0.2),
        ] {
            let t = std::time::Instant::now();
            let segs = transcribe_file(
                &audio.join(format!("{clip}.wav")),
                &dir,
                VadConfig::default(),
                &stt,
                &mut |_| {},
            )
            .unwrap();
            let took = t.elapsed();
            let text: Vec<&str> = segs.iter().map(|s| s.text.as_str()).collect();
            let reference = std::fs::read_to_string(audio.join(format!("{clip}.txt"))).unwrap();
            let wer = wer(&reference, &text.join(" "));
            let longest = segs.iter().map(|s| s.end_ms - s.start_ms).max().unwrap();
            println!(
                "parakeet {clip}: {} segments (longest {longest} ms) in {took:?}, WER {:.1}%",
                segs.len(),
                100.0 * wer
            );
            assert!(wer < max_wer, "{clip}: WER {wer}");
            assert!(longest <= 7_800, "{clip}: longest chunk {longest} ms");
            assert!(segs.windows(2).all(|w| w[0].end_ms <= w[1].start_ms));
        }
    }
}

//! 16 kHz mono resampling and Silero VAD chunking (NGR-138).
//!
//! Chunks end at pauses: Silero closes a chunk after `min_silence_s` of silence, and when
//! speech runs past `max_chunk_s` it raises its threshold to end at the next short pause. Speech
//! that still hasn't paused by 1.5 × `max_chunk_s` is cut here at the quietest 20 ms in the
//! second half of the first `max_chunk_s`. Energy dips also fall inside words, so cutting at
//! them right at 1 × cost 2 WER points on the NGR-128 clean clip (3.1% vs 0.9%); in this order
//! the spike's numbers hold (0.9 / 3.1 / 16.9% on clean / other / AMI). Blind fixed cuts cost
//! 3–14 points (ADR 0001).

use std::path::Path;

use sherpa_onnx::{LinearResampler, VadModelConfig, VoiceActivityDetector};

/// What the VAD and every STT model take.
pub const SAMPLE_RATE: u32 = 16_000;
/// Silero's window at 16 kHz.
const WINDOW: usize = 512;

/// Chunking knobs. Smaller `max_chunk_s` = text appears sooner; larger = more context per decode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VadConfig {
    /// Target chunk length in seconds: past it the VAD looks hard for a pause. Chunks stay
    /// under 1.5 × this (a forced cut, if needed, lands within it).
    pub max_chunk_s: f32,
    /// A pause at least this long ends a chunk.
    pub min_silence_s: f32,
    /// Speech probability above which a window counts as speech.
    pub threshold: f32,
}

impl Default for VadConfig {
    /// ADR 0001: at most 5 s, cut at pauses of at least 0.25 s.
    fn default() -> Self {
        Self {
            max_chunk_s: 5.0,
            min_silence_s: 0.25,
            threshold: 0.5,
        }
    }
}

/// One stretch of speech, 16 kHz mono, with its position in the source's timeline.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub start_ms: u32,
    pub end_ms: u32,
    pub pcm: Vec<f32>,
}

pub struct Chunker {
    vad: VoiceActivityDetector,
    carry: Vec<f32>,
    max: u64,
    /// How long speech may run before a forced cut: `max` plus the pause that would end it.
    patience: u64,
    /// Samples fed to the VAD so far.
    pos: u64,
    /// Where the speech in progress starts, as far as it hasn't been emitted yet.
    base: u64,
    /// Audio before this was already emitted by a forced cut.
    cut_until: u64,
    /// Audio from `base` on, for forced cuts.
    hist: Vec<f32>,
}

/// Speech is detected a little after it starts; keep this much before detection (sherpa-onnx
/// backdates a segment by two windows plus the minimum speech length, 0.31 s).
const LEAD: u64 = 6_000;
/// A forced cut leaves the VAD a tail to close; tails shorter than this are noise.
const MIN_TAIL: u64 = 3_200;
/// Energy frame for finding the quietest cut point (20 ms, hop 10 ms).
const FRAME: usize = 320;

impl Chunker {
    /// `model` is `silero_vad.onnx`.
    pub fn new(model: &Path, cfg: VadConfig) -> Result<Self, String> {
        if !model.is_file() {
            return Err(format!("VAD model missing: {}", model.display()));
        }
        let mut c = VadModelConfig::default();
        c.silero_vad.model = Some(model.to_string_lossy().into_owned());
        c.silero_vad.threshold = cfg.threshold;
        c.silero_vad.min_silence_duration = cfg.min_silence_s;
        c.silero_vad.min_speech_duration = 0.25;
        c.silero_vad.window_size = 512;
        c.silero_vad.max_speech_duration = cfg.max_chunk_s;
        c.sample_rate = 16_000;
        c.num_threads = 1;
        // The buffer holds audio of the chunk in progress; keep it well above the max chunk.
        let buffer_s = (cfg.max_chunk_s * 4.0).max(30.0);
        let vad = VoiceActivityDetector::create(&c, buffer_s).ok_or("VAD failed to load")?;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (max, pause) = (
            (cfg.max_chunk_s.max(1.0) * 16_000.0) as u64,
            (cfg.min_silence_s.max(0.0) * 16_000.0) as u64,
        );
        Ok(Self {
            vad,
            carry: Vec::new(),
            max,
            patience: max * 3 / 2 + pause + WINDOW as u64,
            pos: 0,
            base: 0,
            cut_until: 0,
            hist: Vec::new(),
        })
    }

    /// Feed 16 kHz mono audio; returns the chunks that finished.
    pub fn push(&mut self, pcm: &[f32]) -> Vec<Chunk> {
        self.carry.extend_from_slice(pcm);
        let mut out = Vec::new();
        let windows = std::mem::take(&mut self.carry);
        let (whole, rest) = windows.as_chunks::<WINDOW>();
        for w in whole {
            self.window(w, &mut out);
        }
        self.carry = rest.to_vec();
        out
    }

    /// End of audio: returns the chunk still in progress, if any.
    pub fn flush(&mut self) -> Vec<Chunk> {
        let mut out = Vec::new();
        if !self.carry.is_empty() {
            let mut w = std::mem::take(&mut self.carry);
            w.resize(WINDOW, 0.0);
            self.window(&w, &mut out);
        }
        self.vad.flush();
        self.drain(&mut out);
        out
    }

    fn window(&mut self, w: &[f32], out: &mut Vec<Chunk>) {
        self.vad.accept_waveform(w);
        self.hist.extend_from_slice(w);
        self.pos += WINDOW as u64;
        if !self.vad.detected() {
            self.forget_until(self.pos.saturating_sub(LEAD));
        } else if self.pos - self.base > self.patience {
            // Silero only force-cuts when it finds a pause; speech without one would wait for
            // the speaker to stop. Cut it here at the quietest 20 ms instead.
            let cut = self.quietest(self.base + self.max / 2, self.base + self.max);
            out.push(self.chunk(self.base, cut));
            self.cut_until = cut;
            self.forget_until(cut);
        }
        self.drain(out);
    }

    /// The VAD's own chunks, minus whatever a forced cut already sent.
    fn drain(&mut self, out: &mut Vec<Chunk>) {
        while let Some(seg) = self.vad.front() {
            let start = u64::try_from(seg.start()).unwrap_or(0);
            let end = start + u64::try_from(seg.n()).unwrap_or(0);
            let from = start.max(self.cut_until);
            if end >= from + MIN_TAIL {
                #[allow(clippy::cast_possible_truncation)]
                let skip = (from - start) as usize;
                out.push(Chunk {
                    start_ms: ms(from),
                    end_ms: ms(end),
                    pcm: seg.samples()[skip..].to_vec(),
                });
            }
            self.vad.pop();
            // Whatever comes next starts after this, even if the VAD backdates it.
            self.cut_until = self.cut_until.max(end);
            self.forget_until(end);
        }
    }

    fn forget_until(&mut self, at: u64) {
        if at > self.base {
            #[allow(clippy::cast_possible_truncation)]
            self.hist
                .drain(..((at - self.base) as usize).min(self.hist.len()));
            self.base = at;
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    fn chunk(&self, from: u64, to: u64) -> Chunk {
        Chunk {
            start_ms: ms(from),
            end_ms: ms(to),
            pcm: self.hist[(from - self.base) as usize..(to - self.base) as usize].to_vec(),
        }
    }

    /// Middle of the lowest-energy 20 ms frame between `from` and `to`.
    #[allow(clippy::cast_possible_truncation)]
    fn quietest(&self, from: u64, to: u64) -> u64 {
        let (a, b) = ((from - self.base) as usize, (to - self.base) as usize);
        let energy = |i: usize| self.hist[i..i + FRAME].iter().map(|x| x * x).sum::<f32>();
        (a..b.min(self.hist.len()).saturating_sub(FRAME))
            .step_by(FRAME / 2)
            .min_by(|&i, &j| energy(i).total_cmp(&energy(j)))
            .map_or(to, |i| self.base + (i + FRAME / 2) as u64)
    }
}

fn ms(samples: u64) -> u32 {
    u32::try_from(samples / 16).unwrap_or(u32::MAX)
}

/// Converts any device or file rate to 16 kHz (sherpa-onnx's band-limited resampler).
#[derive(Default)]
pub struct Resampler {
    inner: Option<(u32, LinearResampler)>,
}

impl Resampler {
    pub fn to_16k(&mut self, rate: u32, pcm: &[f32]) -> Vec<f32> {
        if rate == SAMPLE_RATE {
            return pcm.to_vec();
        }
        if self.inner.as_ref().is_none_or(|(r, _)| *r != rate) {
            // A new rate (device switch) starts a fresh filter.
            let r = i32::try_from(rate).unwrap_or(i32::MAX);
            self.inner = LinearResampler::create(r, 16_000).map(|l| (rate, l));
        }
        self.inner
            .as_ref()
            .map(|(_, l)| l.resample(pcm, false))
            .unwrap_or_default()
    }
}

/// Average interleaved channels down to mono.
pub fn downmix(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    #[allow(clippy::cast_precision_loss)]
    let n = channels as f32;
    interleaved
        .chunks_exact(channels)
        .map(|f| f.iter().sum::<f32>() / n)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{fixture, fixture_pcm};

    fn chunks(cfg: VadConfig, pcm: &[f32]) -> Vec<Chunk> {
        let mut c = Chunker::new(&fixture("silero_vad.onnx"), cfg).unwrap();
        // Odd-sized pushes, like device callbacks.
        let mut out: Vec<Chunk> = pcm.chunks(441).flat_map(|p| c.push(p)).collect();
        out.extend(c.flush());
        out
    }

    /// RMS of the 20 ms around `ms` in the fixture.
    fn rms_at(pcm: &[f32], ms: u32) -> f32 {
        let i = ms as usize * 16;
        let w = &pcm[i.saturating_sub(160)..(i + 160).min(pcm.len())];
        #[allow(clippy::cast_precision_loss)]
        let n = w.len() as f32;
        (w.iter().map(|x| x * x).sum::<f32>() / n).sqrt()
    }

    #[test]
    fn two_utterances_make_two_chunks_at_the_pause() {
        // Utterance 1 is 0–3.5 s, then 0.5 s of silence, utterance 2 is 4.0–9.0 s.
        let pcm = fixture_pcm();
        let got = chunks(
            VadConfig {
                max_chunk_s: 10.0,
                ..VadConfig::default()
            },
            &pcm,
        );
        let spans: Vec<_> = got.iter().map(|c| (c.start_ms, c.end_ms)).collect();
        assert_eq!(got.len(), 2, "{spans:?}");
        assert!(
            got[0].end_ms <= 3_800 && got[1].start_ms >= 3_500,
            "{spans:?}"
        );
        for c in &got {
            assert_eq!(c.pcm.len() / 16, (c.end_ms - c.start_ms) as usize);
        }
    }

    #[test]
    fn long_speech_is_cut_at_quiet_points_not_mid_word() {
        let pcm = fixture_pcm();
        #[allow(clippy::cast_precision_loss)]
        let speech_rms = (pcm.iter().map(|x| x * x).sum::<f32>() / pcm.len() as f32).sqrt();
        let short = VadConfig {
            max_chunk_s: 2.0,
            ..VadConfig::default()
        };
        let got = chunks(short, &pcm);
        let spans: Vec<_> = got.iter().map(|c| (c.start_ms, c.end_ms)).collect();
        println!("max 2 s: {spans:?}");
        assert!(
            got.len() >= 4,
            "max 2 s should split the 5 s utterance: {spans:?}"
        );
        for c in &got {
            // At most 1.5 × the 2 s target.
            assert!(c.end_ms - c.start_ms <= 3_000, "chunk too long: {spans:?}");
            // Every cut lands where the audio is quiet compared with speech on average.
            let at_cut = rms_at(&pcm, c.end_ms);
            println!(
                "cut at {} ms: rms {at_cut:.4} vs speech {speech_rms:.4}",
                c.end_ms
            );
            assert!(
                at_cut < 0.15 * speech_rms,
                "cut at {} ms is loud ({at_cut} vs {speech_rms}): {spans:?}",
                c.end_ms
            );
        }
        // Tunable: a longer max gives fewer, longer chunks.
        assert!(got.len() > chunks(VadConfig::default(), &pcm).len());
    }

    #[test]
    fn silence_makes_no_chunks() {
        assert_eq!(chunks(VadConfig::default(), &vec![0.0; 48_000]), vec![]);
    }

    #[test]
    fn resamples_and_downmixes() {
        assert_eq!(downmix(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
        let mut r = Resampler::default();
        let out = r.to_16k(48_000, &vec![0.1; 48_000]);
        // The filter holds back a few samples; roughly 1 s comes out.
        assert!((15_900..=16_000).contains(&out.len()), "{}", out.len());
        assert_eq!(r.to_16k(16_000, &[0.5; 10]).len(), 10);
    }
}

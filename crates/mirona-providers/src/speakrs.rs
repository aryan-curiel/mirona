//! speakrs (pyannote community-1 in Rust): diarization and voiceprints (ADR 0001, NGR-152/153).

use std::path::Path;
use std::sync::Mutex;

use speakrs::{DiarizationResult, ExecutionMode, OwnedDiarizationPipeline, PipelineBuilder};

use crate::{Diarization, Diarizer, Error, FoundSpeaker, Locality, Provider, Result, Turn};

/// Clusters with less speech than this are spurious "extra speakers" and are dropped (ADR 0001).
pub const MIN_SPEAKER_SPEECH_MS: u32 = 20_000;
/// Enrollment clips must be at least this long (ADR 0001).
pub const MIN_ENROLL_MS: u32 = 30_000;
/// Auto-match a voiceprint at this cosine or above…
pub const MATCH_COSINE: f32 = 0.55;
/// …and only if it beats the second-best person by this much.
pub const MATCH_MARGIN: f32 = 0.15;

/// Which backend diarizes. `CoreML` peaks at 7–13 GB, so callers pick it only with ≥ 16 GB RAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    CoreMl,
    Cpu,
}

pub struct Speakrs {
    diarize: Mutex<OwnedDiarizationPipeline>,
    /// Voiceprints always run on the CPU (`CoreML` embeddings are wrong on short clips).
    /// `None` when `diarize` is already the CPU pipeline.
    cpu: Option<Mutex<OwnedDiarizationPipeline>>,
}

fn build(dir: &Path, mode: ExecutionMode) -> Result<Mutex<OwnedDiarizationPipeline>> {
    PipelineBuilder::from_dir(dir, mode)
        .and_then(PipelineBuilder::build)
        .map(Mutex::new)
        .map_err(|e| Error::Provider(format!("speakrs {mode:?} models in {}: {e}", dir.display())))
}

impl Speakrs {
    /// `models_dir` holds the speakrs model set (the HF `avencera/speakrs-models` snapshot).
    pub fn new(models_dir: &Path, mode: Mode) -> Result<Self> {
        Ok(match mode {
            Mode::Cpu => Self {
                diarize: build(models_dir, ExecutionMode::Cpu)?,
                cpu: None,
            },
            Mode::CoreMl => Self {
                diarize: build(models_dir, ExecutionMode::CoreMl)?,
                cpu: Some(build(models_dir, ExecutionMode::Cpu)?),
            },
        })
    }

    fn run(pipe: &Mutex<OwnedDiarizationPipeline>, pcm: &[f32]) -> Result<Diarization> {
        let res = pipe
            .lock()
            .map_err(|_| Error::Provider("speakrs pipeline poisoned".into()))?
            .run(pcm)
            .map_err(|e| Error::Provider(format!("speakrs: {e}")))?;
        Ok(found(&res))
    }

    fn voiceprint_now(&self, pcm: &[f32]) -> Result<Vec<f32>> {
        if pcm.len() < MIN_ENROLL_MS as usize * 16 {
            return Err(Error::Provider(format!(
                "enrollment needs at least {} s of audio",
                MIN_ENROLL_MS / 1000
            )));
        }
        let all = Self::run(self.cpu.as_ref().unwrap_or(&self.diarize), pcm)?;
        // The enrolled person is whoever talks most in their own clip.
        all.speakers
            .into_iter()
            .max_by_key(|s| s.speech_ms)
            .map(|s| s.embedding)
            .ok_or_else(|| Error::Provider("no speech in enrollment clip".into()))
    }
}

/// All clusters of a speakrs result: non-overlapping turns + one centroid per cluster.
fn found(res: &DiarizationResult) -> Diarization {
    let mut turns: Vec<Turn> = res
        .exclusive_segments
        .iter()
        .filter_map(|s| {
            Some(Turn {
                start_ms: ms(s.start),
                end_ms: ms(s.end),
                speaker: s.speaker.strip_prefix("SPEAKER_")?.parse().ok()?,
            })
        })
        .collect();
    turns.sort_by_key(|t| t.start_ms);

    // Centroid = mean of L2-normalized chunk embeddings per cluster (NGR-129 spike).
    let (chunks, local, dim) = res.embeddings.dim();
    let mut sums: Vec<Vec<f32>> = Vec::new();
    if res.hard_clusters.dim() == (chunks, local) {
        for c in 0..chunks {
            for s in 0..local {
                let Ok(k) = usize::try_from(res.hard_clusters[[c, s]]) else {
                    continue; // inactive
                };
                let v: Vec<f32> = (0..dim).map(|d| res.embeddings[[c, s, d]]).collect();
                if !v.iter().all(|x| x.is_finite()) {
                    continue;
                }
                if sums.len() <= k {
                    sums.resize(k + 1, vec![0.0; dim]);
                }
                let v = normalize(v);
                sums[k].iter_mut().zip(&v).for_each(|(a, b)| *a += b);
            }
        }
    }
    let n = turns
        .iter()
        .map(|t| t.speaker + 1)
        .max()
        .unwrap_or(0)
        .max(sums.len());
    let mut speakers = vec![
        FoundSpeaker {
            embedding: Vec::new(),
            speech_ms: 0
        };
        n
    ];
    for t in &turns {
        speakers[t.speaker].speech_ms += t.end_ms.saturating_sub(t.start_ms);
    }
    for (spk, sum) in speakers.iter_mut().zip(sums) {
        spk.embedding = normalize(sum); // the mean's direction == the sum's direction
    }
    Diarization { turns, speakers }
}

/// Drop speakers with less than `min_ms` of speech and renumber the turns that remain.
fn drop_small(d: Diarization, min_ms: u32) -> Diarization {
    let mut new_index = vec![None; d.speakers.len()];
    let mut speakers = Vec::new();
    for (i, s) in d.speakers.into_iter().enumerate() {
        if s.speech_ms >= min_ms {
            new_index[i] = Some(speakers.len());
            speakers.push(s);
        }
    }
    let turns = d
        .turns
        .into_iter()
        .filter_map(|t| {
            Some(Turn {
                speaker: new_index[t.speaker]?,
                ..t
            })
        })
        .collect();
    Diarization { turns, speakers }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn ms(secs: f64) -> u32 {
    (secs * 1000.0).round() as u32
}

fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        for x in &mut v {
            *x /= n;
        }
    }
    v
}

/// Cosine similarity; 0 when either vector is zero.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    let n = norm(a) * norm(b);
    if n > 0.0 { dot / n } else { 0.0 }
}

/// ADR 0001 auto-match: best cosine ≥ `MATCH_COSINE` and ≥ `MATCH_MARGIN` above the runner-up.
pub fn match_voiceprint<Id: Clone>(candidate: &[f32], enrolled: &[(Id, Vec<f32>)]) -> Option<Id> {
    let mut scores: Vec<(f32, &Id)> = enrolled
        .iter()
        .map(|(id, v)| (cosine(candidate, v), id))
        .collect();
    scores.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (best, id) = *scores.first()?;
    let second = scores.get(1).map_or(f32::NEG_INFINITY, |s| s.0);
    (best >= MATCH_COSINE && best - second >= MATCH_MARGIN).then(|| id.clone())
}

impl Provider for Speakrs {
    fn id(&self) -> &'static str {
        "speakrs"
    }
    fn locality(&self) -> Locality {
        Locality::Local
    }
}

impl Diarizer for Speakrs {
    fn diarize(&self, pcm: &[f32]) -> impl Future<Output = Result<Diarization>> + Send {
        // ponytail: runs right here, blocking, and hands back a ready future; callers must call
        // it in spawn_blocking (once, after the call, ADR 0001). Move that in here with NGR-152.
        let d = Self::run(&self.diarize, pcm).map(|d| drop_small(d, MIN_SPEAKER_SPEECH_MS));
        std::future::ready(d)
    }

    fn voiceprint(&self, pcm: &[f32]) -> impl Future<Output = Result<Vec<f32>>> + Send {
        // ponytail: blocking, same as `diarize`; NGR-153.
        std::future::ready(self.voiceprint_now(pcm))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spk(embedding: Vec<f32>, speech_ms: u32) -> FoundSpeaker {
        FoundSpeaker {
            embedding,
            speech_ms,
        }
    }
    fn turn(start_ms: u32, end_ms: u32, speaker: usize) -> Turn {
        Turn {
            start_ms,
            end_ms,
            speaker,
        }
    }

    #[test]
    fn cosine_basics() {
        assert!((cosine(&[1.0, 0.0], &[2.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 3.0]).abs() < 1e-6);
        assert!((cosine(&[1.0, 0.0], &[-1.0, 0.0]) + 1.0).abs() < 1e-6);
        assert!(cosine(&[0.0, 0.0], &[1.0, 0.0]).abs() < f32::EPSILON);
    }

    #[test]
    fn match_rule() {
        // Unit vector at cosine `c` from [1, 0].
        let at = |c: f32| vec![c, (1.0 - c * c).sqrt()];
        let probe = [1.0, 0.0];
        assert_eq!(
            match_voiceprint(&probe, &[("ana", at(0.9)), ("bo", at(0.3))]),
            Some("ana")
        );
        assert_eq!(match_voiceprint(&probe, &[("ana", at(0.9))]), Some("ana"));
        // Below threshold.
        assert_eq!(match_voiceprint(&probe, &[("ana", at(0.5))]), None);
        // Margin too small.
        assert_eq!(
            match_voiceprint(&probe, &[("ana", at(0.9)), ("bo", at(0.8))]),
            None
        );
        assert_eq!(match_voiceprint::<&str>(&probe, &[]), None);
    }

    #[test]
    fn small_clusters_dropped_and_turns_reindexed() {
        let d = Diarization {
            turns: vec![
                turn(0, 30_000, 0),
                turn(30_000, 40_000, 1),
                turn(40_000, 70_000, 2),
            ],
            speakers: vec![
                spk(vec![1.0], 30_000),
                spk(vec![2.0], 10_000),
                spk(vec![3.0], 30_000),
            ],
        };
        let d = drop_small(d, MIN_SPEAKER_SPEECH_MS);
        assert_eq!(d.turns, vec![turn(0, 30_000, 0), turn(40_000, 70_000, 1)]);
        assert_eq!(
            d.speakers,
            vec![spk(vec![1.0], 30_000), spk(vec![3.0], 30_000)]
        );
    }

    #[test]
    fn missing_models_is_a_provider_error() {
        assert!(matches!(
            Speakrs::new(Path::new("/nonexistent"), Mode::Cpu),
            Err(Error::Provider(_))
        ));
    }
}

/// Needs the model set: `MIRONA_SPEAKRS_DIR=spikes/speakrs/models/hf/.../snapshots/<rev>`.
#[cfg(test)]
mod model_tests {
    use super::*;

    const AUDIO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../spikes/speakrs");

    fn speakrs(mode: Mode) -> Speakrs {
        let dir = std::env::var("MIRONA_SPEAKRS_DIR").expect("MIRONA_SPEAKRS_DIR");
        Speakrs::new(Path::new(&dir), mode).unwrap()
    }

    fn wav(path: &str) -> Vec<f32> {
        hound::WavReader::open(format!("{AUDIO}/{path}"))
            .unwrap()
            .samples::<i16>()
            .map(|s| f32::from(s.unwrap()) / 32768.0)
            .collect()
    }

    /// RTTM rows as `(start_ms, end_ms, speaker)`.
    fn rttm(path: &str) -> Vec<(u32, u32, String)> {
        std::fs::read_to_string(format!("{AUDIO}/{path}"))
            .unwrap()
            .lines()
            .map(|l| {
                let f: Vec<&str> = l.split_whitespace().collect();
                let (start, dur): (f64, f64) = (f[3].parse().unwrap(), f[4].parse().unwrap());
                (ms(start), ms(start + dur), f[7].to_owned())
            })
            .collect()
    }

    /// The future has no await points, so one poll finishes it.
    fn now<T>(fut: impl Future<Output = T>) -> T {
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        match std::pin::pin!(fut).poll(&mut cx) {
            std::task::Poll::Ready(v) => v,
            std::task::Poll::Pending => panic!("not ready"),
        }
    }

    #[test]
    #[ignore = "needs MIRONA_SPEAKRS_DIR"]
    fn diarizes_two_speaker_voxconverse_clip() {
        let d = now(speakrs(Mode::CoreMl).diarize(&wav("audio/vc/crixb.wav"))).unwrap();
        assert_eq!(
            d.speakers.len(),
            2,
            "{:?}",
            d.speakers.iter().map(|s| s.speech_ms)
        );
        assert!(d.turns.windows(2).all(|w| w[0].start_ms <= w[1].start_ms));
        // Share of reference speech (10 ms frames) that some turn covers.
        let frames = |spans: &mut dyn Iterator<Item = (u32, u32)>| {
            let mut on = vec![false; 40_000];
            for (s, e) in spans {
                on[(s / 10) as usize..(e / 10) as usize].fill(true);
            }
            on
        };
        let reference = frames(&mut rttm("out/ref/crixb.rttm").into_iter().map(|r| (r.0, r.1)));
        let hyp = frames(&mut d.turns.iter().map(|t| (t.start_ms, t.end_ms)));
        let total = reference.iter().filter(|x| **x).count();
        let hit = reference
            .iter()
            .zip(&hyp)
            .filter(|(r, h)| **r && **h)
            .count();
        #[allow(clippy::cast_precision_loss)]
        let coverage = hit as f64 / total as f64;
        println!(
            "crixb speakers {} coverage {:.1}%",
            d.speakers.len(),
            100.0 * coverage
        );
        assert!(coverage > 0.9, "coverage {coverage}");
        for s in &d.speakers {
            assert!((cosine(&s.embedding, &s.embedding) - 1.0).abs() < 1e-4);
        }
    }

    /// `secs` of `who`'s longest turns in an AMI meeting (others may chip in briefly).
    fn solo(pcm: &[f32], rows: &[(u32, u32, String)], who: &str, secs: usize) -> Vec<f32> {
        let mut rows: Vec<_> = rows.iter().filter(|r| r.2 == who).collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.1 - r.0));
        let mut out = Vec::new();
        for (s, e, _) in rows {
            out.extend_from_slice(&pcm[*s as usize * 16..*e as usize * 16]);
            if out.len() >= secs * 16_000 {
                break;
            }
        }
        out.truncate(secs * 16_000);
        out
    }

    #[test]
    #[ignore = "needs MIRONA_SPEAKRS_DIR"]
    fn voiceprints_match_across_meetings() {
        let sp = speakrs(Mode::Cpu);
        let people = ["FEE013", "FEE016", "MEE014", "MEO015"];
        let (a, b) = (wav("audio/ami/ES2004a.wav"), wav("audio/ami/ES2004b.wav"));
        let (ra, rb) = (
            rttm("audio/ami/ES2004a.rttm"),
            rttm("audio/ami/ES2004b.rttm"),
        );
        let t = std::time::Instant::now();
        let enrolled: Vec<(&str, Vec<f32>)> = people
            .iter()
            .map(|p| (*p, now(sp.voiceprint(&solo(&a, &ra, p, 35))).unwrap()))
            .collect();
        for p in people {
            let probe = now(sp.voiceprint(&solo(&b, &rb, p, 35))).unwrap();
            let scores: Vec<String> = enrolled
                .iter()
                .map(|(id, v)| format!("{id} {:.2}", cosine(&probe, v)))
                .collect();
            println!("ES2004b {p}: {}", scores.join(", "));
            assert_eq!(match_voiceprint(&probe, &enrolled), Some(p));
        }
        println!("8 voiceprints in {:.1}s", t.elapsed().as_secs_f64());
        assert!(now(sp.voiceprint(&vec![0.0; 29 * 16_000])).is_err());
    }
}

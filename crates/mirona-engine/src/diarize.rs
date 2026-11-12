//! Who spoke (NGR-152/153): after the call, speakrs diarizes the other side's own recording
//! (`system.caf`; the mix of mic + system audio for meetings recorded before it and imports),
//! each "them" segment goes to the voice it overlaps most, and voices are
//! matched against known people's voiceprints (ADR 0001: cosine ≥ 0.55 with a 0.15 margin;
//! 0.45–0.55 is only a suggestion). Mic segments stay "me"; so do voices made mostly of mic
//! speech. Unknown voices become "Speaker N". The embeddings go to the store's private
//! registry (`.mirona/voices.json`), never into `meeting.md`.

use crate::{Engine, Result, block_on, stage};
use mirona_core::{Locality, NameSuggestion, PipelineEvent, Segment, Speaker, Tint};
use mirona_providers::questions::{self, NO_NAME, Reply};
use mirona_providers::speakrs::{self, COREML_MODEL, CPU_MODEL, MODELS_DIR, Mode, Speakrs};
use mirona_providers::{Classifier, Diarization, Diarizer};
use mirona_store::Cluster;
use std::path::Path;
use std::sync::mpsc::Sender;

/// `NameSuggestion.evidence` of a guess made from the voice, not from what was said.
const SIMILAR_VOICE: &str = "Similar voice";
/// Segments are VAD chunks: never longer than 1.5 × 5 s (`VadConfig`).
const MAX_SEGMENT_MS: u32 = 7_500;
/// `CoreML` peaks at 7–13 GB, so it needs at least this much RAM (ADR 0001).
pub(crate) const COREML_MIN_RAM: u64 = 16 << 30;

/// A segment's stretch of the recording, and whether the mic heard it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start_ms: u32,
    pub end_ms: u32,
    pub mic: bool,
}

/// Spans of a transcript: each segment's exact end, or for transcripts saved before segments
/// kept one (`end_ms` 0), an estimate: up to the next segment of the same source, at most
/// `MAX_SEGMENT_MS`, and about 2 words a second.
pub fn spans(segs: &[Segment]) -> Vec<Span> {
    segs.iter()
        .enumerate()
        .map(|(i, s)| {
            let end = if s.end_ms > s.start_ms {
                s.end_ms
            } else {
                let next = segs[i + 1..]
                    .iter()
                    .filter(|n| n.speaker_id == s.speaker_id && n.start_ms >= s.start_ms)
                    .map(|n| n.start_ms)
                    .min()
                    .unwrap_or(u32::MAX);
                let words = u32::try_from(s.text.split_whitespace().count()).unwrap_or(u32::MAX);
                let by_words = 1_000u32.saturating_add(words.saturating_mul(500));
                s.start_ms
                    .saturating_add(MAX_SEGMENT_MS.min(by_words))
                    .min(next)
                    .max(s.start_ms + 1)
            };
            Span {
                start_ms: s.start_ms,
                end_ms: end,
                mic: s.speaker_id == "me",
            }
        })
        .collect()
}

fn overlap(a: (u32, u32), b: (u32, u32)) -> u32 {
    a.1.min(b.1).saturating_sub(a.0.max(b.0))
}

/// Which voice (index into `d.speakers`) each segment gets; `None` keeps its current speaker.
/// Mic segments stay; voices made mostly of mic speech (when `mixed`: the audio held the mic
/// too) are never given out. Any other segment goes to the voice it overlaps most, else the
/// nearest one. Voices too small to be real were already dropped by the diarizer
/// (`speakrs::min_speech_ms`).
/// Only speaker ids change, so segment order (and overlapping speech) is untouched. Lines the
/// user moved by hand are reset to `None` by the caller ([`Engine::apply_diarization`]).
pub fn assign(spans: &[Span], d: &Diarization, mixed: bool) -> Vec<Option<usize>> {
    let mut mic_ms = vec![0u32; d.speakers.len()];
    for s in spans.iter().filter(|s| s.mic && mixed) {
        for t in &d.turns {
            mic_ms[t.speaker] += overlap((s.start_ms, s.end_ms), (t.start_ms, t.end_ms));
        }
    }
    let eligible = |k: usize| mic_ms[k].saturating_mul(2) <= d.speakers[k].speech_ms;
    let turns: Vec<_> = d.turns.iter().filter(|t| eligible(t.speaker)).collect();
    spans
        .iter()
        .map(|s| {
            if s.mic || turns.is_empty() {
                return None;
            }
            let mut by = vec![0u32; d.speakers.len()];
            for t in &turns {
                by[t.speaker] += overlap((s.start_ms, s.end_ms), (t.start_ms, t.end_ms));
            }
            // The first of the largest wins ties.
            let best = (0..by.len()).rev().max_by_key(|k| by[*k]);
            if let Some(k) = best.filter(|k| by[*k] > 0) {
                return Some(k);
            }
            let gap = |t: &&&mirona_providers::Turn| {
                t.start_ms
                    .saturating_sub(s.end_ms)
                    .max(s.start_ms.saturating_sub(t.end_ms))
            };
            turns.iter().min_by_key(gap).map(|t| t.speaker)
        })
        .collect()
}

/// A known person a speaker is addressed as nearby. "Inés, can you…" votes for the next other
/// speaker, "Thanks, Inés." for the previous one. The fallback for [`suggest_names`] when
/// classification is off or fails.
pub fn context_suggestion(
    segs: &[(&str, &str)],
    speaker: &str,
    names: &[String],
) -> Option<NameSuggestion> {
    let mut best: Option<(usize, &String, &str)> = None;
    for name in names {
        let mut votes = 0;
        let mut evidence = "";
        for (i, (who, text)) in segs.iter().enumerate() {
            if *who == speaker {
                continue;
            }
            for dir in addressed(text, name) {
                let to = match dir {
                    Dir::Next => segs[i + 1..].iter().find(|(w, _)| w != who),
                    Dir::Prev => segs[..i].iter().rev().find(|(w, _)| w != who),
                };
                if to.is_some_and(|(w, _)| *w == speaker) {
                    votes += 1;
                    evidence = text;
                }
            }
        }
        if votes > 0 && best.is_none_or(|b| votes > b.0) {
            best = Some((votes, name, evidence));
        }
    }
    best.map(|(votes, name, evidence)| {
        #[allow(clippy::cast_precision_loss)]
        let probability = votes as f32 / (votes as f32 + 1.0);
        NameSuggestion {
            name: name.clone(),
            probability,
            evidence: evidence.chars().take(80).collect(),
        }
    })
}

/// Who a vocative is said to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dir {
    /// "Inés, can you…", "Hi Inés": whoever speaks next.
    Next,
    /// "Thanks, Inés.", "…, Inés?": whoever just spoke.
    Prev,
}

/// Each use of `name` to address someone, followed by punctuation or the end of the line.
fn addressed(text: &str, name: &str) -> Vec<Dir> {
    text.match_indices(name)
        .filter_map(|(at, _)| {
            let before = text[..at].trim_end().to_lowercase();
            let after = text[at + name.len()..].chars().next();
            if !after.is_none_or(|c| matches!(c, ',' | '.' | '?' | '!' | ';' | ':')) {
                return None;
            }
            let greeting = ["hi", "hey", "hello"].iter().any(|g| before.ends_with(g));
            if before.is_empty() || before.ends_with(['.', '?', '!']) || greeting {
                Some(Dir::Next)
            } else if before.ends_with(',')
                || before.ends_with("thanks")
                || before.ends_with("thank you")
            {
                Some(Dir::Prev)
            } else {
                None
            }
        })
        .collect()
}

/// Lines on each side of an unknown speaker's turn sent with the name question.
const NAME_CONTEXT: usize = 2;
/// Most transcript characters sent per name question (~1.5k tokens).
const NAME_CHARS: usize = 6_000;

/// Names for the unknown voices of a diarized meeting from how the others address them
/// (NGR-153): for each "Speaker N" without a voice-based guess, the classifier picks among
/// known people not in this meeting, or [`NO_NAME`], over the lines around their turns. Its
/// answer replaces the text heuristic's: a name at/above the `speaker_name` threshold that is
/// said in those lines becomes the suggestion (quoting the line), anything else clears it.
/// Sensitive lines stay off cloud classifiers, redacted ones off all. On an error the
/// heuristic's suggestion stays. Returns how many speakers got a suggestion.
pub fn suggest_names(
    store: &crate::Shared,
    cfg: &mirona_providers::config::Config,
    id: &str,
    c: &impl Classifier,
) -> Result<usize> {
    let (m, voices) = {
        let s = store.lock().unwrap();
        (s.meeting(id)?, s.voices())
    };
    let names: Vec<String> = voices
        .people
        .iter()
        .filter(|p| !m.speakers.iter().any(|s| s.id == p.id))
        .map(|p| p.name.clone())
        .collect();
    if names.is_empty() {
        return Ok(0);
    }
    let cloud = c.locality() == Locality::Cloud;
    let name_of = |s: &Segment| {
        m.speakers
            .iter()
            .find(|p| p.id == s.speaker_id)
            .map_or_else(|| s.speaker_id.clone(), |p| p.name.clone())
    };
    let unknown = m.speakers.iter().filter(|sp| {
        !["me", "them", "speaker"].contains(&sp.id.as_str())
            && voices.people.iter().all(|p| p.id != sp.id)
            && sp
                .suggestion
                .as_ref()
                .is_none_or(|g| g.evidence != SIMILAR_VOICE)
    });
    let mut suggested = 0;
    for sp in unknown {
        let near: Vec<usize> = m
            .segments
            .iter()
            .enumerate()
            .filter(|(_, s)| s.speaker_id == sp.id)
            .flat_map(|(i, _)| i.saturating_sub(NAME_CONTEXT)..=i + NAME_CONTEXT)
            .filter(|&i| i < m.segments.len())
            .collect();
        let mut lines: Vec<(usize, String)> = vec![];
        let mut chars = 0;
        for i in near {
            let s = &m.segments[i];
            let sensitive = s.is_sensitive();
            if lines.iter().any(|(j, _)| *j == i) || s.redacted || (cloud && sensitive) {
                continue;
            }
            let line = format!("{}: {}", name_of(s), s.text);
            chars += line.len() + 1;
            if chars > NAME_CHARS {
                break;
            }
            lines.push((i, line));
        }
        if lines.is_empty() {
            continue;
        }
        let text: Vec<&str> = lines.iter().map(|(_, l)| l.as_str()).collect();
        let q = questions::speaker_name_question(&sp.name, &names);
        let replies = block_on(c.ask(&text.join("\n"), std::slice::from_ref(&q)))
            .map_err(|e| e.to_string())?;
        let threshold = cfg.threshold("speaker_name", !cloud);
        let suggestion = replies.first().and_then(|r| {
            let Reply::Choice { value, .. } = r else {
                return None;
            };
            let probability = r.confidence();
            if value == NO_NAME || probability < threshold {
                return None;
            }
            // Only a name actually said here: quote the line (others' lines first).
            let said = |(i, l): &&(usize, String)| {
                m.segments[*i].speaker_id != sp.id && l.contains(value.as_str())
            };
            let evidence = lines
                .iter()
                .find(said)
                .or_else(|| lines.iter().find(|(_, l)| l.contains(value.as_str())))?;
            let quote = evidence.1.split_once(": ").map_or("", |(_, t)| t);
            Some(NameSuggestion {
                name: value.clone(),
                probability,
                evidence: quote.chars().take(80).collect(),
            })
        });
        suggested += usize::from(suggestion.is_some());
        store
            .lock()
            .unwrap()
            .set_suggestion(id, &sp.id, suggestion)?;
    }
    Ok(suggested)
}

/// `CoreML` on macOS with ≥ 16 GB RAM and its models present; the CPU otherwise, or when the
/// config says `[providers] diarization = "speakrs-cpu"`.
fn mode(diarization: &str, models: &Path) -> Mode {
    let coreml = cfg!(target_os = "macos")
        && diarization != "speakrs-cpu"
        && models.join(COREML_MODEL).is_file()
        && ram_bytes().is_some_and(|b| b >= COREML_MIN_RAM);
    if coreml { Mode::CoreMl } else { Mode::Cpu }
}

/// Physical memory: `sysctl hw.memsize` on macOS, `/proc/meminfo` on Linux.
pub(crate) fn ram_bytes() -> Option<u64> {
    if cfg!(target_os = "macos") {
        let out = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()?;
        return String::from_utf8_lossy(&out.stdout).trim().parse().ok();
    }
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb: u64 = info
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    Some(kb * 1024)
}

/// The recording as 16 kHz mono PCM (works on the WAV and on the Opus CAF).
fn read_16k(path: &Path) -> Result<Vec<f32>> {
    let mut resampler = mirona_capture::Resampler::default();
    let mut pcm = Vec::new();
    mirona_capture::decode(path, &mut |rate, mono| {
        pcm.extend(resampler.to_16k(rate, mono));
        Ok(())
    })?;
    Ok(pcm)
}

impl Engine {
    /// Diarize a finished meeting: assign a speaker to each segment, name known voices and
    /// save the voices heard. Sends `Diarized` with the number of voices besides "me".
    /// Blocking (seconds on `CoreML`, ~4 min per hour of audio on the CPU): call it off the UI
    /// thread. Off with `[providers] diarization = "off"`; a meeting without audio is skipped.
    pub fn diarize(&self, id: &str, events: &Sender<PipelineEvent>) -> Result<u32> {
        self.diarize_tuned(id, events, None)
    }

    /// [`Engine::diarize`] at a sensitivity in 0..=1 (higher: more voices; `None`: the default,
    /// 0.5). Reads the other side's own recording when the meeting has one.
    pub fn diarize_tuned(
        &self,
        id: &str,
        events: &Sender<PipelineEvent>,
        sensitivity: Option<f32>,
    ) -> Result<u32> {
        let which = stage(&self.config(), "diarization", "speakrs");
        if which == "off" {
            return Ok(0);
        }
        let audio = {
            let s = self.store.lock().unwrap();
            match s.plain_system_audio(id)? {
                Some(p) => Some(p),
                None => s.plain_audio(id)?,
            }
        };
        let Some(audio) = audio else {
            return Ok(0); // nothing recorded, nothing to separate
        };
        let dir = self.paths.models.join(MODELS_DIR);
        if !dir.join(CPU_MODEL).is_file() {
            return Err(format!(
                "speaker models are missing in {} (download them in Settings › AI models)",
                dir.display()
            ));
        }
        let d = {
            let pcm = read_16k(&audio)?;
            let sp =
                Speakrs::tuned(&dir, mode(&which, &dir), sensitivity).map_err(|e| e.to_string())?;
            block_on(sp.diarize(&pcm)).map_err(|e| e.to_string())?
        };
        let n = self.apply_diarization(id, &d)?;
        events
            .send(PipelineEvent::Diarized {
                meeting_id: id.into(),
                speakers: n,
            })
            .ok();
        Ok(n)
    }

    /// Assign, match and save a diarization of meeting `id` (the model-free half of
    /// [`Engine::diarize`]). Returns the number of voices given segments.
    pub fn apply_diarization(&self, id: &str, d: &Diarization) -> Result<u32> {
        let mut store = self.store.lock().unwrap();
        let meeting = store.meeting(id)?;
        let mixed = store.system_audio_path(id)?.is_none();
        let mut labels = assign(&spans(&meeting.segments), d, mixed);
        // Lines the user moved keep their speaker (NGR-201).
        for (l, s) in labels.iter_mut().zip(&meeting.segments) {
            if s.by_hand {
                *l = None;
            }
        }
        let prints = store.voiceprints();
        let voices = store.voices();
        let name_of = |pid: &str| voices.people.iter().find(|p| p.id == pid).map(|p| &p.name);

        // Voices in order of first appearance.
        let mut order: Vec<usize> = vec![];
        for k in labels.iter().flatten() {
            if !order.contains(k) {
                order.push(*k);
            }
        }
        // Voices that lines moved by hand still point at: a new voice must not take their id.
        let kept: Vec<&str> = meeting
            .segments
            .iter()
            .filter(|s| s.by_hand)
            .map(|s| s.speaker_id.as_str())
            .collect();
        let mut ids = vec![String::new(); d.speakers.len()];
        let mut speakers: Vec<Speaker> = vec![];
        let mut anon = 0;
        for &k in &order {
            let emb = &d.speakers[k].embedding;
            let known = speakrs::match_voiceprint(emb, &prints)
                .filter(|pid| !ids.contains(pid))
                .and_then(|pid| Some((name_of(&pid)?.clone(), pid)));
            let tint = if speakers.len().is_multiple_of(2) {
                Tint::Indigo
            } else {
                Tint::Amber
            };
            let sp = if let Some((name, pid)) = known {
                let mut sp = store
                    .people()
                    .iter()
                    .find(|p| p.id == pid)
                    .cloned()
                    .unwrap_or_else(|| speaker(&pid, &name, tint));
                sp.suggestion = None;
                sp.subtitle = "Recognized by voice".into();
                sp
            } else {
                anon += 1;
                while kept.contains(&format!("{id}-s{anon}").as_str()) {
                    anon += 1;
                }
                let mut sp = speaker(&format!("{id}-s{anon}"), &format!("Speaker {anon}"), tint);
                sp.initial = anon.to_string();
                sp.suggestion = speakrs::closest(emb, &prints).and_then(|(pid, cos)| {
                    Some(NameSuggestion {
                        name: name_of(&pid)?.clone(),
                        probability: cos,
                        evidence: SIMILAR_VOICE.into(),
                    })
                });
                sp
            };
            ids[k].clone_from(&sp.id);
            speakers.push(sp);
        }

        let seg_ids: Vec<String> = meeting
            .segments
            .iter()
            .zip(&labels)
            .map(|(s, l)| l.map_or_else(|| s.speaker_id.clone(), |k| ids[k].clone()))
            .collect();
        // Context names for the voices still unnamed, from known people not in this meeting.
        let names: Vec<String> = voices
            .people
            .iter()
            .filter(|p| !seg_ids.contains(&p.id))
            .map(|p| p.name.clone())
            .collect();
        let lines: Vec<(&str, &str)> = seg_ids
            .iter()
            .zip(&meeting.segments)
            .map(|(w, s)| (w.as_str(), s.text.as_str()))
            .collect();
        for sp in speakers.iter_mut().filter(|s| s.subtitle.is_empty()) {
            if sp.suggestion.is_none() {
                sp.suggestion = context_suggestion(&lines, &sp.id, &names);
            }
        }
        let clusters = order
            .iter()
            .map(|&k| Cluster {
                meeting_id: id.into(),
                speaker_id: ids[k].clone(),
                speech_ms: d.speakers[k].speech_ms,
                embedding: d.speakers[k].embedding.clone(),
            })
            .collect();
        store.set_diarization(id, &seg_ids, speakers, clusters)?;
        Ok(u32::try_from(order.len()).unwrap_or(u32::MAX))
    }
}

fn speaker(id: &str, name: &str, tint: Tint) -> Speaker {
    Speaker {
        id: id.into(),
        name: name.into(),
        initial: name.chars().next().unwrap_or('?').to_string(),
        tint,
        subtitle: String::new(),
        voiceprint: None,
        suggestion: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirona_providers::{FoundSpeaker, Provider, Turn};

    fn span(start_ms: u32, end_ms: u32, mic: bool) -> Span {
        Span {
            start_ms,
            end_ms,
            mic,
        }
    }
    fn turn(start_ms: u32, end_ms: u32, speaker: usize) -> Turn {
        Turn {
            start_ms,
            end_ms,
            speaker,
        }
    }
    fn voices(speech: &[u32]) -> Vec<FoundSpeaker> {
        speech
            .iter()
            .map(|&speech_ms| FoundSpeaker {
                embedding: vec![1.0],
                speech_ms,
            })
            .collect()
    }

    #[test]
    fn mic_stays_me_and_them_goes_to_the_most_overlapping_voice() {
        // Voice 0 = me (on the mic), 1 and 2 = remote people; 1 and 2 overlap at 25–27 s.
        let d = Diarization {
            turns: vec![
                turn(0, 20_000, 0),
                turn(20_000, 27_000, 1),
                turn(25_000, 50_000, 2),
                turn(50_000, 70_000, 1),
                turn(70_000, 80_000, 0),
            ],
            speakers: voices(&[30_000, 27_000, 25_000]),
        };
        let spans = [
            span(0, 20_000, true),
            span(21_000, 26_000, false), // mostly voice 1
            span(25_500, 27_000, true),  // the mic during the overlap: still me
            span(24_000, 30_000, false), // mostly voice 2; starts before the one above ends
            span(55_000, 60_000, false), // voice 1
            span(70_000, 80_000, true),
            span(70_000, 75_000, false), // only me's voice here: the nearest other voice
            span(200_000, 201_000, false), // after the end: the nearest voice
        ];
        assert_eq!(
            assign(&spans, &d, true),
            [
                None,
                Some(1),
                None,
                Some(2),
                Some(1),
                None,
                Some(1),
                Some(1)
            ]
        );
        // The other side's own recording never held the mic: voice 0 is someone else there.
        assert_eq!(assign(&spans, &d, false)[6], Some(0));
    }

    #[test]
    fn every_voice_the_diarizer_kept_is_given_out() {
        let d = Diarization {
            turns: vec![
                turn(0, 30_000, 0),
                turn(30_000, 35_000, 1), // 5 s: kept at a high sensitivity
                turn(35_000, 70_000, 2),
            ],
            speakers: voices(&[30_000, 5_000, 35_000]),
        };
        let spans = [
            span(1_000, 4_000, false),
            span(31_000, 34_000, false),
            span(40_000, 45_000, false),
        ];
        assert_eq!(assign(&spans, &d, true), [Some(0), Some(1), Some(2)]);
        // Nothing found: everyone keeps their speaker.
        let none = Diarization {
            turns: vec![],
            speakers: vec![],
        };
        assert_eq!(assign(&spans, &none, true), [None, None, None]);
    }

    #[test]
    fn spans_use_exact_ends_else_estimate_them() {
        let seg = |who: &str, start_ms: u32, end_ms: u32, text: &str| Segment {
            id: String::new(),
            speaker_id: who.into(),
            start_ms,
            end_ms,
            text: text.into(),
            redacted: false,
            tags: vec![],
            answers: vec![],
            owners: vec![],
            due: None,
            by_hand: false,
        };
        // Old transcripts (end 0): estimated.
        let segs = [
            seg(
                "them",
                0,
                0,
                "one two three four five six seven eight nine ten eleven twelve",
            ),
            seg("me", 1_000, 0, "hi"),
            seg(
                "them",
                4_000,
                0,
                "a b c d e f g h i j k l m n o p q r s t u v w x y z",
            ),
        ];
        assert_eq!(
            spans(&segs),
            [
                span(0, 4_000, false),      // cut at the next "them"
                span(1_000, 2_500, true),   // 1 s + 1 word
                span(4_000, 11_500, false), // capped at a VAD chunk
            ]
        );
        // Ends kept by capture are used as they are, even past the next segment's start.
        let exact = [
            seg("them", 0, 4_800, "one two"),
            seg("me", 1_000, 1_200, "hi"),
            seg("them", 4_000, 6_000, "x"),
        ];
        assert_eq!(
            spans(&exact),
            [
                span(0, 4_800, false),
                span(1_000, 1_200, true),
                span(4_000, 6_000, false),
            ]
        );
    }

    #[test]
    fn names_from_context() {
        let names = vec!["Inés".to_string(), "Bo".to_string()];
        let lines = [
            ("me", "So that's the plan."),
            ("s1", "Sounds good to me."),
            ("me", "Thanks, Inés. Bo, anything?"),
            ("s2", "Nothing from me."),
            ("me", "Inés, can you send it?"),
            ("s1", "Sure."),
        ];
        let s1 = context_suggestion(&lines, "s1", &names).unwrap();
        assert_eq!(s1.name, "Inés");
        assert!((s1.probability - 2.0 / 3.0).abs() < 1e-6);
        assert_eq!(s1.evidence, "Inés, can you send it?");
        // "Bo," addresses whoever answers next.
        assert_eq!(context_suggestion(&lines, "s2", &names).unwrap().name, "Bo");
        assert!(context_suggestion(&[("me", "I met Inés yesterday.")], "s1", &names).is_none());
        assert_eq!(addressed("Bob, hi", "Bo"), []);
        assert_eq!(addressed("Hi Inés, welcome", "Inés"), [Dir::Next]);
    }

    fn engine(dir: &Path, models: &Path) -> Engine {
        let meetings = dir.join("meetings");
        let store = mirona_store::Store::open(&meetings).unwrap();
        let paths = crate::Paths {
            meetings,
            models: models.into(),
            templates: dir.join("templates"),
        };
        let mut cfg = mirona_providers::config::Config::default();
        cfg.providers.insert("llm".into(), "none".into()); // no summaries here
        Engine::new(
            std::sync::Arc::new(std::sync::Mutex::new(store)),
            cfg,
            paths,
        )
    }

    /// A finished meeting: `(speaker id, start, end, text)` segments, plus `audio` copied in.
    fn meeting(e: &Engine, id: &str, segs: &[(&str, u32, u32, String)], audio: Option<&Path>) {
        let mut s = e.store.lock().unwrap();
        s.start_meeting(id).unwrap();
        for (n, (who, start_ms, end_ms, text)) in segs.iter().enumerate() {
            let seg = Segment {
                id: format!("{id}-{n}"),
                speaker_id: (*who).into(),
                start_ms: *start_ms,
                end_ms: *end_ms,
                text: text.clone(),
                redacted: false,
                tags: vec![],
                answers: vec![],
                owners: vec![],
                due: None,
                by_hand: false,
            };
            s.push_segment(id, seg).unwrap();
        }
        if let Some(a) = audio {
            std::fs::copy(a, s.meeting_dir(id).unwrap().join("audio.wav")).unwrap();
            s.set_audio(id, "audio.wav").unwrap();
        }
        s.finish_meeting(id, 60_000).unwrap();
    }

    fn names(e: &Engine, id: &str) -> Vec<String> {
        let m = e.store.lock().unwrap().meeting(id).unwrap();
        let name = |sid: &str| {
            m.speakers
                .iter()
                .find(|p| p.id == sid)
                .unwrap()
                .name
                .clone()
        };
        m.segments.iter().map(|g| name(&g.speaker_id)).collect()
    }

    #[test]
    fn rerunning_detection_keeps_lines_moved_by_hand() {
        let dir = tempfile::tempdir().unwrap();
        let e = engine(dir.path(), dir.path());
        let segs = [
            ("me", 1_000, 2_000, "Hi.".to_string()),
            ("them", 26_000, 27_000, "Hello.".to_string()),
            ("them", 40_000, 42_000, "Prices are up.".to_string()),
            ("them", 61_000, 62_000, "Agreed.".to_string()),
        ];
        let found = |speech_ms| FoundSpeaker {
            embedding: vec![1.0, 0.0],
            speech_ms,
        };
        let d = Diarization {
            turns: vec![turn(0, 25_000, 0), turn(25_000, 90_000, 1)],
            speakers: vec![found(25_000), found(65_000)],
        };
        meeting(&e, "m-a", &segs, None);
        e.apply_diarization("m-a", &d).unwrap();
        assert_eq!(
            names(&e, "m-a"),
            ["Me", "Speaker 1", "Speaker 1", "Speaker 1"]
        );
        let zoe = e
            .store
            .lock()
            .unwrap()
            .set_line_speaker("m-a", "m-a-1", "Zoe")
            .unwrap();
        e.apply_diarization("m-a", &d).unwrap();
        assert_eq!(names(&e, "m-a"), ["Me", "Zoe", "Speaker 1", "Speaker 1"]);
        let m = e.store.lock().unwrap().meeting("m-a").unwrap();
        assert!(m.segments[1].by_hand);
        assert!(m.speakers.iter().any(|p| p.id == zoe.id));
    }

    #[test]
    fn a_rerun_never_reuses_a_voice_id_kept_by_a_moved_line() {
        let dir = tempfile::tempdir().unwrap();
        let e = engine(dir.path(), dir.path());
        let segs = [
            ("me", 1_000, 2_000, "Hi.".to_string()),
            ("them", 26_000, 27_000, "Hello.".to_string()),
            ("them", 40_000, 42_000, "Prices are up.".to_string()),
        ];
        let d = Diarization {
            turns: vec![turn(0, 25_000, 0), turn(25_000, 90_000, 1)],
            speakers: vec![
                FoundSpeaker {
                    embedding: vec![1.0, 0.0],
                    speech_ms: 25_000,
                },
                FoundSpeaker {
                    embedding: vec![0.0, 1.0],
                    speech_ms: 65_000,
                },
            ],
        };
        meeting(&e, "m-a", &segs, None);
        e.apply_diarization("m-a", &d).unwrap();
        // The mic line was really Speaker 1 (a leak), then detection runs again.
        e.store
            .lock()
            .unwrap()
            .set_line_speaker("m-a", "m-a-0", "Speaker 1")
            .unwrap();
        e.apply_diarization("m-a", &d).unwrap();
        let m = e.store.lock().unwrap().meeting("m-a").unwrap();
        let ids: Vec<&str> = m.segments.iter().map(|g| g.speaker_id.as_str()).collect();
        assert_eq!(ids, ["m-a-s1", "m-a-s2", "m-a-s2"]);
        assert_eq!(names(&e, "m-a"), ["Speaker 1", "Speaker 2", "Speaker 2"]);
    }

    #[test]
    #[allow(clippy::many_single_char_names)]
    fn known_voices_are_named_in_the_next_meeting() {
        let dir = tempfile::tempdir().unwrap();
        let e = engine(dir.path(), dir.path());
        let segs = |extra: &str| {
            vec![
                ("me", 1_000, 2_000, "Hi both.".to_string()),
                ("them", 26_000, 27_000, "Hello.".to_string()),
                ("them", 40_000, 42_000, "Prices are up.".to_string()),
                ("them", 61_000, 62_000, "Agreed.".to_string()),
                ("me", 80_000, 81_000, extra.to_string()),
            ]
        };
        let d = |a: Vec<f32>, b: Vec<f32>| Diarization {
            turns: vec![
                turn(0, 25_000, 0),
                turn(25_000, 60_000, 1),
                turn(60_000, 90_000, 2),
            ],
            speakers: vec![
                FoundSpeaker {
                    embedding: vec![0.5, 0.5],
                    speech_ms: 25_000,
                },
                FoundSpeaker {
                    embedding: a,
                    speech_ms: 35_000,
                },
                FoundSpeaker {
                    embedding: b,
                    speech_ms: 30_000,
                },
            ],
        };
        meeting(&e, "m-a", &segs("Bye."), None);
        let n = e
            .apply_diarization("m-a", &d(vec![1.0, 0.0], vec![0.0, 1.0]))
            .unwrap();
        assert_eq!(n, 2);
        assert_eq!(
            names(&e, "m-a"),
            ["Me", "Speaker 1", "Speaker 1", "Speaker 2", "Me"]
        );
        e.store
            .lock()
            .unwrap()
            .name_speaker("m-a", "m-a-s1", "Ana")
            .unwrap();

        // Next meeting: Ana's voice again (cosine 0.95), and someone new who is thanked by name.
        meeting(&e, "m-b", &segs("Thanks, Bo."), None);
        e.store
            .lock()
            .unwrap()
            .name_speaker("m-a", "m-a-s2", "Bo")
            .unwrap();
        e.apply_diarization("m-b", &d(vec![0.95, 0.312], vec![0.0, -1.0]))
            .unwrap();
        assert_eq!(names(&e, "m-b"), ["Me", "Ana", "Ana", "Speaker 1", "Me"]);
        let m = e.store.lock().unwrap().meeting("m-b").unwrap();
        let ana = m.speakers.iter().find(|s| s.name == "Ana").unwrap();
        assert_eq!(ana.voiceprint.as_ref().unwrap().meetings, 2);
        let new = m.speakers.iter().find(|s| s.name == "Speaker 1").unwrap();
        let hint = new.suggestion.as_ref().unwrap();
        assert_eq!(
            (hint.name.as_str(), hint.evidence.as_str()),
            ("Bo", "Thanks, Bo.")
        );

        // With a classifier its answer replaces the heuristic's; a failure keeps it.
        let cfg = mirona_providers::config::Config::default();
        let hint = |e: &Engine| {
            let m = e.store.lock().unwrap().meeting("m-b").unwrap();
            let s = m.speakers.iter().find(|s| s.name == "Speaker 1").unwrap();
            s.suggestion
                .clone()
                .map(|g| (g.name, g.probability, g.evidence))
        };
        assert!(suggest_names(&e.store, &cfg, "m-b", &Namer(None)).is_err());
        assert_eq!(hint(&e).unwrap().1, 0.5, "the heuristic's");
        assert_eq!(
            suggest_names(&e.store, &cfg, "m-b", &Namer(Some(0.9))).unwrap(),
            1
        );
        assert_eq!(hint(&e), Some(("Bo".into(), 0.9, "Thanks, Bo.".into())));
        // Unsure (below the 0.6 default threshold): no suggestion.
        assert_eq!(
            suggest_names(&e.store, &cfg, "m-b", &Namer(Some(0.4))).unwrap(),
            0
        );
        assert_eq!(hint(&e), None);
    }

    /// Picks the first option said in the text, with probability `.0`; `None` fails.
    struct Namer(Option<f32>);

    impl mirona_providers::Provider for Namer {
        fn id(&self) -> &'static str {
            "namer"
        }
        fn locality(&self) -> Locality {
            Locality::Cloud
        }
    }

    impl Classifier for Namer {
        fn ask(
            &self,
            text: &str,
            qs: &[questions::Question],
        ) -> impl Future<Output = mirona_providers::Result<Vec<Reply>>> + Send {
            let reply = self.0.map(|p| {
                let opts = questions::options(&qs[0]);
                let value = opts
                    .iter()
                    .find(|o| text.contains(o.as_str()))
                    .unwrap_or(&opts[opts.len() - 1])
                    .clone();
                vec![Reply::Choice {
                    probabilities: vec![(value.clone(), p)],
                    value,
                }]
            });
            std::future::ready(reply.ok_or(mirona_providers::Error::Provider("down".into())))
        }
    }

    /// Unknown voices named from how they're addressed, by Jev and by qwen3:4b, where the text
    /// heuristic misses one ("Over to you, Keiko, …" reads as said to the previous speaker,
    /// and there is none) and a third known person is only mentioned.
    #[test]
    #[ignore = "needs TYPESAFE_API_KEY, network and Ollama serving qwen3:4b"]
    #[allow(clippy::too_many_lines)]
    fn live_names_from_context() {
        let dir = tempfile::tempdir().unwrap();
        let e = engine(dir.path(), dir.path());
        let voice = |i: usize, speech_ms: u32| FoundSpeaker {
            embedding: (0..6).map(|k| if k == i { 1.0 } else { 0.0 }).collect(),
            speech_ms,
        };
        // Meeting A: three voices, named Keiko, Tom and Elena.
        let segs_a = vec![
            ("them", 1_000, 20_000, "Hello.".to_string()),
            ("them", 26_000, 45_000, "Hi.".to_string()),
            ("them", 51_000, 70_000, "Hey.".to_string()),
        ];
        meeting(&e, "m-a", &segs_a, None);
        let d_a = Diarization {
            turns: vec![
                turn(0, 25_000, 0),
                turn(25_000, 50_000, 1),
                turn(50_000, 75_000, 2),
            ],
            speakers: vec![voice(0, 25_000), voice(1, 25_000), voice(2, 25_000)],
        };
        e.apply_diarization("m-a", &d_a).unwrap();
        for (sid, name) in [("m-a-s1", "Keiko"), ("m-a-s2", "Tom"), ("m-a-s3", "Elena")] {
            e.store
                .lock()
                .unwrap()
                .name_speaker("m-a", sid, name)
                .unwrap();
        }
        // Meeting B: two new voices.
        let segs_b = vec![
            (
                "me",
                0,
                4_000,
                "Okay, let's get started. Over to you, Keiko, for the numbers.".to_string(),
            ),
            (
                "them",
                6_000,
                27_000,
                "Sure. Revenue is up eight percent and churn is flat.".to_string(),
            ),
            (
                "me",
                28_000,
                30_000,
                "Great. Tom, where are we on hiring?".to_string(),
            ),
            (
                "them",
                31_000,
                54_000,
                "Two offers out, one accepted. Elena is helping with the second.".to_string(),
            ),
            (
                "me",
                55_000,
                58_000,
                "Thanks, Tom. Let's wrap up.".to_string(),
            ),
        ];
        meeting(&e, "m-b", &segs_b, None);
        let d_b = Diarization {
            turns: vec![
                turn(0, 5_000, 0),
                turn(5_000, 28_000, 1),
                turn(28_000, 30_000, 0),
                turn(30_000, 55_000, 2),
                turn(55_000, 60_000, 0),
            ],
            speakers: vec![voice(3, 12_000), voice(4, 23_000), voice(5, 25_000)],
        };
        e.apply_diarization("m-b", &d_b).unwrap();
        let cfg = mirona_providers::config::Config::default();
        let key = std::env::var("TYPESAFE_API_KEY").expect("TYPESAFE_API_KEY");
        let jev = crate::classify::AnyClassifier::Jev(mirona_providers::jev::Jev::new(key));
        let local =
            crate::classify::AnyClassifier::Local(mirona_providers::ollama::Ollama::default());
        for c in [jev, local] {
            for sid in ["m-b-s1", "m-b-s2"] {
                e.store
                    .lock()
                    .unwrap()
                    .set_suggestion("m-b", sid, None)
                    .unwrap();
            }
            let t = std::time::Instant::now();
            let n = suggest_names(&e.store, &cfg, "m-b", &c).unwrap();
            let m = e.store.lock().unwrap().meeting("m-b").unwrap();
            let got: Vec<_> = m
                .speakers
                .iter()
                .filter(|s| s.id.starts_with("m-b-s"))
                .map(|s| (s.name.clone(), s.suggestion.clone()))
                .collect();
            println!(
                "{}: {n} named in {:.1}s: {got:?}",
                c.id(),
                t.elapsed().as_secs_f64()
            );
            let name = |i: usize| got[i].1.as_ref().map(|g| g.name.as_str());
            assert_eq!(
                (name(0), name(1)),
                (Some("Keiko"), Some("Tom")),
                "{}",
                c.id()
            );
        }
    }

    // --- Real models (spike data): `cargo test -p mirona-engine -- --ignored` ---
    // Symlinked model dirs: Unix only.
    #[cfg(unix)]
    mod real {
        use super::*;

        // (test bodies use short names: e = engine, m = meeting, t = timer, n = voices)

        const SPIKE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../spikes/speakrs");

        /// A models dir holding `speakrs/` (`MIRONA_SPEAKRS_DIR`, else the spike's snapshot) and
        /// the speech models (`MIRONA_MODELS_DIR`, else the Parakeet spike's), by symlink.
        fn models(dir: &Path) -> std::path::PathBuf {
            let m = dir.join("models");
            std::fs::create_dir_all(&m).unwrap();
            let speakrs = std::env::var_os("MIRONA_SPEAKRS_DIR").map_or_else(
                || {
                    let snaps =
                        format!("{SPIKE}/models/hf/models--avencera--speakrs-models/snapshots");
                    std::fs::read_dir(snaps)
                        .unwrap()
                        .next()
                        .unwrap()
                        .unwrap()
                        .path()
                },
                std::path::PathBuf::from,
            );
            std::os::unix::fs::symlink(speakrs, m.join(MODELS_DIR)).unwrap();
            let speech = std::env::var_os("MIRONA_MODELS_DIR").map_or_else(
                || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spikes/parakeet/models"),
                std::path::PathBuf::from,
            );
            for f in std::fs::read_dir(speech).unwrap() {
                let f = f.unwrap();
                std::os::unix::fs::symlink(f.path(), m.join(f.file_name())).unwrap();
            }
            m
        }

        /// RTTM rows as transcript segments of `who`, cut into ≤ 5 s chunks like the VAD. The text
        /// is the reference speaker, then one word per 0.5 s: "o" where another reference speaker
        /// talks at the same time (overlap: not scored), "w" elsewhere.
        fn rttm_segments(path: &str, who: &'static str) -> Vec<(&'static str, u32, u32, String)> {
            let text = std::fs::read_to_string(format!("{SPIKE}/{path}")).unwrap();
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let rows: Vec<(u32, u32, &str)> = text
                .lines()
                .map(|l| {
                    let f: Vec<&str> = l.split_whitespace().collect();
                    let (start, dur): (f64, f64) = (f[3].parse().unwrap(), f[4].parse().unwrap());
                    (
                        (start * 1000.0) as u32,
                        ((start + dur) * 1000.0) as u32,
                        f[7],
                    )
                })
                .collect();
            let mut out = vec![];
            for &(start, end, spk) in &rows {
                let mut at = start;
                while at < end {
                    let len = (end - at).min(5_000);
                    let crossed = rows
                        .iter()
                        .any(|r| r.2 != spk && overlap((at, at + len), (r.0, r.1)) > 0);
                    let fill = if crossed { "o" } else { "w" };
                    let words = (len / 500).max(1) as usize;
                    let text = std::iter::once(spk).chain(std::iter::repeat_n(fill, words - 1));
                    out.push((who, at, at + len, text.collect::<Vec<_>>().join(" ")));
                    at += len;
                }
            }
            out.sort_by_key(|s| s.1);
            out
        }

        /// Words of a test segment that count: none for overlapped speech.
        fn scored(t: &str) -> usize {
            if t.contains(" o") {
                0
            } else {
                t.split_whitespace().count()
            }
        }

        fn distinct(v: &[String]) -> usize {
            let mut v = v.to_vec();
            v.sort();
            v.dedup();
            v.len()
        }

        #[test]
        #[ignore = "needs the speakrs spike models and audio"]
        #[allow(clippy::many_single_char_names)]
        fn voxconverse_speaker_counts_and_purity() {
            // Reference speakers with ≥ 20 s of speech (smaller voices are dropped by design).
            for (clip, want) in [
                ("crixb", 2),
                ("qvtia", 2),
                ("houcx", 2), // + 1 speaker with 16.9 s
                ("bspxd", 2), // + 1 with 12.7 s
                ("dhorc", 4),
                ("qfdpp", 3), // + 1 with 8.9 s
            ] {
                let dir = tempfile::tempdir().unwrap();
                let e = engine(dir.path(), &models(dir.path()));
                let segs = rttm_segments(&format!("out/ref/{clip}.rttm"), "them");
                let wav = format!("{SPIKE}/audio/vc/{clip}.wav");
                meeting(&e, clip, &segs, Some(Path::new(&wav)));
                let t = std::time::Instant::now();
                let (tx, rx) = std::sync::mpsc::channel();
                let n = e.diarize(clip, &tx).unwrap();
                let got = names(&e, clip);
                // Purity: share of non-overlapped speech (in words, ~ time) whose voice is its
                // reference speaker's most common voice.
                let m = e.store.lock().unwrap().meeting(clip).unwrap();
                let refs: Vec<&str> = m.segments.iter().map(|s| first_word(&s.text)).collect();
                let w: Vec<usize> = m.segments.iter().map(|s| scored(&s.text)).collect();
                let mut right = 0;
                for r in distinct_strs(&refs) {
                    let mine = |v: &String| {
                        (0..got.len())
                            .filter(|&i| refs[i] == r && got[i] == *v)
                            .map(|i| w[i])
                            .sum::<usize>()
                    };
                    right += got.iter().map(mine).max().unwrap_or(0);
                }
                #[allow(clippy::cast_precision_loss)]
                let purity = right as f64 / w.iter().sum::<usize>() as f64;
                println!(
                    "{clip}: {n} voices, {} speakers on segments (want {want}), purity {:.1}%, {:.1}s",
                    distinct(&got),
                    100.0 * purity,
                    t.elapsed().as_secs_f64()
                );
                assert!(
                    matches!(rx.try_recv(), Ok(PipelineEvent::Diarized { speakers, .. }) if speakers == n)
                );
                assert_eq!(distinct(&got), want, "{clip}");
                assert!(purity > 0.9, "{clip} purity {purity}");
            }
        }

        fn first_word(t: &str) -> &str {
            t.split(' ').next().unwrap_or_default()
        }

        fn distinct_strs<'a>(v: &[&'a str]) -> Vec<&'a str> {
            let mut v = v.to_vec();
            v.sort_unstable();
            v.dedup();
            v
        }

        #[test]
        #[ignore = "needs the speakrs spike models and audio"]
        fn ami_people_enrolled_in_one_meeting_are_named_in_the_next() {
            let dir = tempfile::tempdir().unwrap();
            let e = engine(dir.path(), &models(dir.path()));
            let (tx, _rx) = std::sync::mpsc::channel();
            for (id, enroll) in [("ES2004a", true), ("ES2004b", false)] {
                let segs = rttm_segments(&format!("audio/ami/{id}.rttm"), "them");
                let wav = format!("{SPIKE}/audio/ami/{id}.wav");
                meeting(&e, id, &segs, Some(Path::new(&wav)));
                let t = std::time::Instant::now();
                e.diarize(id, &tx).unwrap();
                let took = t.elapsed().as_secs_f64();
                let m = e.store.lock().unwrap().meeting(id).unwrap();
                let refs: Vec<String> = m
                    .segments
                    .iter()
                    .map(|s| s.text.split(' ').next().unwrap().to_string())
                    .collect();
                if enroll {
                    // Name each voice after the reference speaker most of its segments belong to.
                    for sp in &m.speakers {
                        let mine: Vec<&String> = m
                            .segments
                            .iter()
                            .zip(&refs)
                            .filter(|(s, _)| s.speaker_id == sp.id)
                            .map(|(_, r)| r)
                            .collect();
                        let top = mine
                            .iter()
                            .max_by_key(|r| mine.iter().filter(|x| x == r).count())
                            .unwrap();
                        e.store
                            .lock()
                            .unwrap()
                            .name_speaker(id, &sp.id, top)
                            .unwrap();
                    }
                }
                let got = names(&e, id);
                // Share of non-overlapped speech (in words, ~ time) named as its reference speaker.
                let w: Vec<usize> = m.segments.iter().map(|s| scored(&s.text)).collect();
                let right: usize = (0..got.len())
                    .filter(|&i| got[i] == refs[i])
                    .map(|i| w[i])
                    .sum();
                #[allow(clippy::cast_precision_loss)]
                let acc = right as f64 / w.iter().sum::<usize>() as f64;
                let m = e.store.lock().unwrap().meeting(id).unwrap();
                let who: Vec<String> = m
                    .speakers
                    .iter()
                    .map(|s| format!("{} ({})", s.name, s.subtitle))
                    .collect();
                println!(
                    "{id}: {:.1}s, speakers {who:?}, segments named right {:.1}%",
                    took,
                    100.0 * acc
                );
                if !enroll {
                    for p in ["FEE013", "FEE016", "MEE014", "MEO015"] {
                        let sp = m.speakers.iter().find(|s| s.name == p);
                        assert!(
                            sp.is_some_and(|s| s.subtitle == "Recognized by voice"),
                            "{p} not named: {who:?}"
                        );
                    }
                    assert!(acc > 0.85, "accuracy {acc}");
                }
            }
        }

        #[test]
        #[ignore = "needs the speech + speakrs models (spikes)"]
        fn import_gives_segments_several_speakers() {
            let dir = tempfile::tempdir().unwrap();
            let e = engine(dir.path(), &models(dir.path()));
            let (tx, rx) = std::sync::mpsc::channel();
            let t = std::time::Instant::now();
            let id = e
                .import(Path::new(&format!("{SPIKE}/audio/vc/crixb.wav")), &tx)
                .unwrap();
            let got = names(&e, &id);
            let events: Vec<_> = rx.try_iter().collect();
            println!(
                "crixb import: {} segments, {} speakers, {:.1}s",
                got.len(),
                distinct(&got),
                t.elapsed().as_secs_f64()
            );
            assert!(
                events
                    .iter()
                    .any(|e| matches!(e, PipelineEvent::Diarized { speakers: 2, .. })),
                "{events:?}"
            );
            assert_eq!(distinct(&got), 2, "{got:?}");
        }
    }
}

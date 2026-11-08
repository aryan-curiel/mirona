//! On-disk format of one meeting folder (NGR-140): `meeting.md` + `transcript.jsonl`.
//!
//! `meeting.md` is YAML frontmatter (all structured data) and a Markdown body. In the body only
//! `## Notes` is read back (the user's raw notes, editable in Obsidian); everything after the
//! `GENERATED` marker is rendered from the frontmatter on every write and ignored on read.

use crate::Result;
use crate::crypt::{self, Key};
use mirona_core::{
    ActionItem, EnhancedNote, Id, Meeting, MeetingAnswers, Note, Segment, Speaker, Summary,
};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

pub const MEETING: &str = "meeting.md";
pub const TRANSCRIPT: &str = "transcript.jsonl";
const GENERATED: &str =
    "<!-- mirona: everything below is rendered from the frontmatter and rewritten on save -->";

/// One meeting as the store holds it.
pub struct Entry {
    /// `when` and `date_label` are filled on the way out (they're relative to now).
    pub meeting: Meeting,
    /// RFC 3339 with the local offset, e.g. `2026-11-05T09:30:00+01:00`.
    pub created: String,
    /// File name of the recording inside `dir`.
    pub audio: Option<String>,
    pub items: Vec<ActionItem>,
    pub dir: PathBuf,
    /// One embedding per transcript line, for search (NGR-167).
    pub vectors: crate::search::Vectors,
}

#[derive(Serialize, Deserialize)]
struct Front {
    id: Id,
    title: String,
    created: String,
    #[serde(default)]
    duration_min: u32,
    #[serde(default)]
    language: String,
    #[serde(default)]
    tags: Vec<String>,
    /// Before tags: one client per meeting. Read as the tag `project:<client>`.
    #[serde(default, skip_serializing)]
    client: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    audio: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    sample: bool,
    #[serde(default)]
    speakers: Vec<Speaker>,
    #[serde(default)]
    answers: Option<MeetingAnswers>,
    #[serde(default)]
    summary: Option<Summary>,
    #[serde(default)]
    enhanced_by: String,
    #[serde(default)]
    enhanced_notes: Vec<EnhancedNote>,
    #[serde(default)]
    action_items: Vec<ActionItem>,
    /// When each note line was written (NGR-144). Keyed by the line's text, so editing notes
    /// in another app only loses the times of the lines that changed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    note_times: Vec<NoteTime>,
}

#[derive(Serialize, Deserialize)]
struct NoteTime {
    line: String,
    at_ms: u32,
}

/// Reads a meeting folder. Bad transcript lines are skipped and returned as errors.
pub fn read(dir: &Path, key: Option<&Key>) -> Result<(Entry, Vec<String>)> {
    let text = std::fs::read_to_string(dir.join(MEETING)).map_err(|e| e.to_string())?;
    let (f, raw) = parse(&text)?;
    let raw_at_ms = match_times(&raw, f.note_times);
    let (segments, errors) = read_transcript(&dir.join(TRANSCRIPT), key);
    let mut e = Entry {
        meeting: Meeting {
            id: f.id,
            title: f.title,
            tags: if f.tags.is_empty() {
                legacy_tags(&f.client)
            } else {
                f.tags
            },
            when: String::new(),
            date_label: String::new(),
            duration_min: f.duration_min,
            language: f.language,
            sample: f.sample,
            speakers: f.speakers,
            segments,
            answers: f.answers,
            notes: Note {
                raw,
                raw_at_ms,
                enhanced: f.enhanced_notes,
                enhanced_by: f.enhanced_by,
            },
            summary: f.summary,
        },
        created: f.created,
        audio: f.audio,
        items: f.action_items,
        dir: dir.to_path_buf(),
        vectors: read_vectors(dir, key),
    };
    normalize(&mut e);
    Ok((e, errors))
}

/// A meeting written before tags: its client becomes `project:<client>` ("Unsorted": none).
fn legacy_tags(client: &str) -> Vec<String> {
    let c = client.trim();
    if c.is_empty() || c == "Unsorted" {
        return vec![];
    }
    vec![format!("{}:{c}", mirona_providers::config::PROJECT_GROUP)]
}

/// Action items repeat their meeting's id, title and tags; the meeting wins.
pub fn normalize(e: &mut Entry) {
    for a in &mut e.items {
        a.meeting_id.clone_from(&e.meeting.id);
        a.meeting_title.clone_from(&e.meeting.title);
        a.tags.clone_from(&e.meeting.tags);
    }
}

fn parse(text: &str) -> Result<(Front, Vec<String>)> {
    let text = text.replace("\r\n", "\n");
    let rest = text
        .strip_prefix("---\n")
        .ok_or("no YAML frontmatter (must start with ---)")?;
    let (yaml, body) = rest
        .split_once("\n---\n")
        .or_else(|| rest.strip_suffix("\n---").map(|y| (y, "")))
        .ok_or("frontmatter has no closing ---")?;
    let front: Front = serde_yaml_ng::from_str(yaml).map_err(|e| format!("frontmatter: {e}"))?;
    Ok((front, notes(body)))
}

/// Give each note line the time saved for the same text (first unused match, in order).
fn match_times(raw: &[String], mut times: Vec<NoteTime>) -> Vec<Option<u32>> {
    if times.is_empty() {
        return vec![];
    }
    raw.iter()
        .map(|l| {
            let i = times.iter().position(|t| t.line == *l)?;
            Some(times.remove(i).at_ms)
        })
        .collect()
}

/// The lines under `## Notes`, up to the generated part. Blank lines at either end are dropped.
fn notes(body: &str) -> Vec<String> {
    let mut lines = body.lines().skip_while(|l| l.trim_end() != "## Notes");
    if lines.next().is_none() {
        return vec![];
    }
    let mut raw: Vec<String> = lines
        .take_while(|l| l.trim() != GENERATED)
        .map(String::from)
        .collect();
    while raw.last().is_some_and(|l| l.trim().is_empty()) {
        raw.pop();
    }
    let blank = raw.iter().take_while(|l| l.trim().is_empty()).count();
    raw.drain(..blank);
    raw
}

pub fn read_transcript(path: &Path, key: Option<&Key>) -> (Vec<Segment>, Vec<String>) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (vec![], vec![]); // no transcript yet
    };
    let mut segs = vec![];
    let mut errors = vec![];
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let line = match crypt::open_line(key, line) {
            Ok(l) => l,
            Err(e) => {
                errors.push(format!("{}:{}: {e}", path.display(), n + 1));
                continue;
            }
        };
        match serde_json::from_str(&line) {
            Ok(s) => segs.push(s),
            Err(e) => errors.push(format!("{}:{}: {e}", path.display(), n + 1)),
        }
    }
    (segs, errors)
}

pub fn render(e: &Entry) -> Result<String> {
    let m = &e.meeting;
    let front = Front {
        id: m.id.clone(),
        title: m.title.clone(),
        created: e.created.clone(),
        duration_min: m.duration_min,
        language: m.language.clone(),
        tags: m.tags.clone(),
        client: String::new(),
        audio: e.audio.clone(),
        sample: m.sample,
        speakers: m.speakers.clone(),
        answers: m.answers.clone(),
        summary: m.summary.clone(),
        enhanced_by: m.notes.enhanced_by.clone(),
        enhanced_notes: m.notes.enhanced.clone(),
        action_items: e.items.clone(),
        note_times: m
            .notes
            .raw
            .iter()
            .zip(&m.notes.raw_at_ms)
            .filter_map(|(l, t)| {
                t.map(|at_ms| NoteTime {
                    line: l.clone(),
                    at_ms,
                })
            })
            .collect(),
    };
    let yaml = serde_yaml_ng::to_string(&front).map_err(|e| e.to_string())?;
    let mut s = format!("---\n{yaml}---\n\n# {}\n\n## Notes\n\n", m.title);
    for l in &m.notes.raw {
        s += l;
        s += "\n";
    }
    s += "\n";
    s += GENERATED;
    s += "\n";
    // Writing to a String can't fail.
    if let Some(sum) = &m.summary {
        let _ = write!(s, "\n## Summary\n\n{}\n", sum.overview);
        for (head, list) in [
            ("Decisions", &sum.decisions),
            ("Open questions", &sum.open_questions),
        ] {
            if !list.is_empty() {
                let _ = write!(s, "\n### {head}\n\n");
                for d in list {
                    let _ = writeln!(s, "- {} ({})", d.text, mmss(d.at_ms));
                }
            }
        }
    }
    if !e.items.is_empty() {
        s += "\n## Action items\n\n";
        for a in &e.items {
            let done = if a.done { "x" } else { " " };
            let due = a
                .due
                .as_ref()
                .map(|d| format!(", due {d}"))
                .unwrap_or_default();
            let at = a
                .at_ms
                .map(|t| format!(" ({})", mmss(t)))
                .unwrap_or_default();
            let _ = writeln!(s, "- [{done}] {} ({}{due}){at}", a.text, a.owner);
        }
    }
    if !m.notes.enhanced.is_empty() {
        s += "\n## Enhanced notes\n\n";
        for n in &m.notes.enhanced {
            let _ = writeln!(s, "- {} ({})", n.text, mmss(n.at_ms));
        }
    }
    Ok(s)
}

fn mmss(ms: u32) -> String {
    format!("{:02}:{:02}", ms / 60_000, ms / 1000 % 60)
}

/// Write via a temp file + rename, so a crash never leaves a half-written file.
pub fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn write_meeting(e: &Entry) -> Result<()> {
    write_atomic(&e.dir.join(MEETING), &render(e)?)
}

pub fn write_transcript(e: &Entry, key: Option<&Key>) -> Result<()> {
    let mut s = String::new();
    for seg in &e.meeting.segments {
        s += &crypt::seal_line(key, &serde_json::to_string(seg).map_err(|e| e.to_string())?)?;
        s += "\n";
    }
    write_atomic(&e.dir.join(TRANSCRIPT), &s)
}

/// Missing or unreadable embeddings are just recomputed.
fn read_vectors(dir: &Path, key: Option<&Key>) -> crate::search::Vectors {
    crypt::read_string(key, &dir.join(crate::search::EMBEDDINGS))
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn write_vectors(e: &Entry, key: Option<&Key>) -> Result<()> {
    let s = serde_json::to_string(&e.vectors).map_err(|e| e.to_string())?;
    write_sealed(&e.dir.join(crate::search::EMBEDDINGS), key, s.as_bytes())
}

/// Write a whole file sealed with `key` (or plain), via a temp file + rename.
pub fn write_sealed(path: &Path, key: Option<&Key>, plain: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, crypt::seal(key, plain)?)
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn append_segment(dir: &Path, seg: &Segment, key: Option<&Key>) -> Result<()> {
    let path = dir.join(TRANSCRIPT);
    let line = crypt::seal_line(key, &serde_json::to_string(seg).map_err(|e| e.to_string())?)?;
    let line = line + "\n";
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| f.write_all(line.as_bytes()))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Folder-safe slug: lowercase letters and digits (any script), the rest becomes `-`.
pub fn slug(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                '-'
            }
        })
        .collect();
    let s = s
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let s: String = s.chars().take(40).collect();
    let s = s.trim_end_matches('-');
    if s.is_empty() {
        "meeting".into()
    } else {
        s.into()
    }
}

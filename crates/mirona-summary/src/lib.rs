//! Summary templates, summaries and enhanced notes over any `LlmProvider` (NGR-146/147).
//!
//! - Templates are editable Markdown files (`<dir>/<id>.md`). The four defaults are embedded and
//!   written to the dir when missing; a file the user edited always wins.
//! - The transcript goes to the model as numbered lines (`[12 03:45] Nadia: …`). The model cites
//!   line numbers (or clock times), which map back to `at_ms`; items citing nothing real are
//!   dropped (decisions, questions, notes) or flagged `needs_review` (action items).
//! - Long meetings are cut into chunks of `CHUNK_CHARS` (map), and the partial overviews are
//!   merged by one more call (reduce). Items keep their global line numbers, so citations hold.

use mirona_core::{ActionItem, Decision, EnhancedNote, Meeting, Summary, TagKind};
use mirona_providers::{LlmProvider, Prompt};
use serde_json::Value;
use std::io::Write;
use std::path::Path;

pub type Result<T> = std::result::Result<T, String>;

/// Transcript characters per model call (~6k tokens). ponytail: one size for every model; size
/// it from the provider's real context window if a small-context model needs it.
pub const CHUNK_CHARS: usize = 24_000;

pub const DEFAULT_TEMPLATES: [(&str, &str); 4] = [
    ("standup", include_str!("../templates/standup.md")),
    ("client-call", include_str!("../templates/client-call.md")),
    ("one-on-one", include_str!("../templates/one-on-one.md")),
    ("planning", include_str!("../templates/planning.md")),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    /// File stem, e.g. "client-call".
    pub id: String,
    /// The first `# ` heading, e.g. "Client call" (the id when there is none).
    pub name: String,
    /// The Markdown the model follows.
    pub body: String,
}

impl Template {
    pub fn new(id: &str, body: &str) -> Self {
        let name = body
            .lines()
            .find_map(|l| l.strip_prefix("# "))
            .map_or(id, str::trim);
        Self {
            id: id.into(),
            name: name.into(),
            body: body.into(),
        }
    }

    /// The embedded default, without touching the disk.
    pub fn default_for(id: &str) -> Option<Self> {
        DEFAULT_TEMPLATES
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(i, body)| Self::new(i, body))
    }
}

/// Write each default that has no file yet. Existing files are never overwritten.
fn write_defaults(dir: &Path) -> Result<()> {
    let err = |e: std::io::Error| format!("templates in {}: {e}", dir.display());
    std::fs::create_dir_all(dir).map_err(err)?;
    for (id, body) in DEFAULT_TEMPLATES {
        let path = dir.join(format!("{id}.md"));
        match std::fs::File::create_new(&path) {
            Ok(mut f) => f.write_all(body.as_bytes()).map_err(err)?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(err(e)),
        }
    }
    Ok(())
}

/// Every template in `dir` (defaults written first), sorted by id. User-added files count too.
pub fn templates(dir: &Path) -> Result<Vec<Template>> {
    write_defaults(dir)?;
    let mut out = vec![];
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|e| e == "md")
            && let Some(id) = path.file_stem().and_then(|s| s.to_str())
        {
            let body = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            out.push(Template::new(id, &body));
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// One template by id, read from `dir` (so a user-edited file wins over the default).
pub fn template(dir: &Path, id: &str) -> Result<Template> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("bad template id {id:?}"));
    }
    write_defaults(dir)?;
    let path = dir.join(format!("{id}.md"));
    let body = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Template::new(id, &body))
}

// ---------- transcript in, citations out ----------

fn clock(ms: u32) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

/// `"mm:ss"` or `"h:mm:ss"` → ms.
fn parse_clock(s: &str) -> Option<u32> {
    let parts: Vec<u32> = s
        .split(':')
        .map(|p| p.trim().parse().ok())
        .collect::<Option<_>>()?;
    let secs = match parts[..] {
        [m, s] if s < 60 => m * 60 + s,
        [h, m, s] if m < 60 && s < 60 => h * 3600 + m * 60 + s,
        _ => return None,
    };
    secs.checked_mul(1000)
}

/// Numbered lines; redacted segments never reach the model.
fn transcript_lines(m: &Meeting) -> Vec<String> {
    m.segments
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let who = m
                .speakers
                .iter()
                .find(|p| p.id == s.speaker_id)
                .map_or(s.speaker_id.as_str(), |p| p.name.as_str());
            let text = if s.redacted { "[redacted]" } else { &s.text };
            format!("[{i} {}] {who}: {text}", clock(s.start_ms))
        })
        .collect()
}

/// Group lines into chunks of at most `max` chars (a longer single line is its own chunk).
fn chunks(lines: &[String], max: usize) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for line in lines {
        match out.last_mut() {
            Some(c) if c.len() + line.len() < max => {
                c.push('\n');
                c.push_str(line);
            }
            _ => out.push(line.clone()),
        }
    }
    out
}

/// Map a model citation (line number, `"#12"`, segment id, or `"03:45"`) to a segment start.
fn cite(v: &Value, m: &Meeting) -> Option<u32> {
    let segs = &m.segments;
    let by_index = |i: u64| {
        usize::try_from(i)
            .ok()
            .and_then(|i| segs.get(i))
            .map(|s| s.start_ms)
    };
    match v {
        Value::Number(n) => by_index(n.as_u64()?),
        Value::Array(a) => cite(a.first()?, m),
        Value::String(s) => {
            let s = s.trim().trim_matches(['[', ']', '#']).trim();
            if let Ok(i) = s.parse::<u64>() {
                return by_index(i);
            }
            if let Some(seg) = segs.iter().find(|x| x.id == s) {
                return Some(seg.start_ms);
            }
            // An echoed line prefix, "12 03:45".
            if let Some((first, _)) = s.split_once(' ')
                && let Ok(i) = first.parse::<u64>()
            {
                return by_index(i);
            }
            // A clock time: the line being spoken then, if it lies inside the meeting.
            let ms = parse_clock(s)?;
            let last = segs.last()?.start_ms;
            if ms > last.saturating_add(60_000) {
                return None;
            }
            segs.iter()
                .rev()
                .find(|x| x.start_ms <= ms.saturating_add(999))
                .map(|x| x.start_ms)
        }
        _ => None,
    }
}

/// The first JSON object in a reply, ignoring code fences, `<think>` blocks and chatter.
fn json_object(text: &str) -> Result<Value> {
    let text = text.split_once("</think>").map_or(text, |(_, after)| after);
    for (i, _) in text.match_indices('{') {
        let mut stream = serde_json::Deserializer::from_str(&text[i..]).into_iter::<Value>();
        if let Some(Ok(v @ Value::Object(_))) = stream.next() {
            return Ok(v);
        }
    }
    let preview: String = text.trim().chars().take(200).collect();
    Err(format!(
        "the model did not return a JSON object: {preview:?}"
    ))
}

struct Item {
    text: String,
    owner: String,
    due: Option<String>,
    at_ms: Option<u32>,
    kind: String,
}

fn field(v: &Value, key: &str) -> Option<String> {
    let s = v[key].as_str()?.trim();
    let blank = s.is_empty() || ["none", "null", "n/a", "unknown"].contains(&&*s.to_lowercase());
    (!blank).then(|| s.to_string())
}

/// The list under the first key present; plain strings count as items without a citation.
fn items(v: &Value, keys: &[&str], m: &Meeting) -> Vec<Item> {
    let list = keys.iter().find_map(|k| v[*k].as_array());
    list.into_iter()
        .flatten()
        .filter_map(|it| {
            let text = it
                .as_str()
                .map(str::to_string)
                .or_else(|| field(it, "text"))?;
            let at = ["segment", "line", "at", "timestamp", "time"]
                .iter()
                .map(|k| &it[*k])
                .find(|c| !c.is_null());
            Some(Item {
                text: text.trim().to_string(),
                owner: field(it, "owner").unwrap_or_default(),
                due: field(it, "due"),
                at_ms: at.and_then(|c| cite(c, m)),
                kind: field(it, "kind").unwrap_or_default().to_lowercase(),
            })
        })
        .filter(|i| !i.text.is_empty())
        .collect()
}

async fn ask(llm: &impl LlmProvider, system: &str, user: &str, json: bool) -> Result<String> {
    let prompt = Prompt { system, user, json };
    let done = llm
        .complete(&prompt, &mut |_| {})
        .await
        .map_err(|e| e.to_string())?;
    Ok(done.text)
}

fn header(m: &Meeting, part: usize, parts: usize) -> String {
    let client = if m.client.is_empty() {
        String::new()
    } else {
        format!(" (client: {})", m.client)
    };
    let part = if parts > 1 {
        format!(
            "\nThis is part {} of {parts} of the transcript; cover only this part.",
            part + 1
        )
    } else {
        String::new()
    };
    format!("Meeting: {}{client}{part}", m.title)
}

// ---------- summaries (NGR-146) ----------

const SUMMARY_RULES: &str = r#"You summarize a meeting transcript for the people who were in it.
Each transcript line starts with [line number, time]. Only use what was actually said.
Reply with one JSON object and nothing else, in this shape:
{"overview": "...",
 "decisions": [{"text": "...", "segment": 12}],
 "action_items": [{"text": "...", "owner": "Name or empty", "due": "as said, or null", "segment": 12}],
 "open_questions": [{"text": "...", "segment": 12}]}
"segment" is the line number where the item was said. Use empty lists when there is nothing.
Write in the language of the transcript. Follow this template for what each section holds:"#;

pub fn summary_prompt(template: &Template) -> String {
    format!("{SUMMARY_RULES}\n\n{}", template.body.trim())
}

/// Summarize a meeting with a hand-picked template. `Summary.written_by` is the provider id.
pub async fn summarize(
    meeting: &Meeting,
    template: &Template,
    llm: &impl LlmProvider,
) -> Result<(Summary, Vec<ActionItem>)> {
    if meeting.segments.is_empty() {
        return Err("nothing to summarize: the transcript is empty".into());
    }
    let system = summary_prompt(template);
    let parts = chunks(&transcript_lines(meeting), CHUNK_CHARS);
    let (mut overviews, mut decisions, mut questions, mut actions) =
        (vec![], vec![], vec![], vec![]);
    let mut dropped = 0;
    for (i, part) in parts.iter().enumerate() {
        let user = format!("{}\n\nTranscript:\n{part}", header(meeting, i, parts.len()));
        let v = json_object(&ask(llm, &system, &user, true).await?)?;
        let overview = field(&v, "overview")
            .ok_or_else(|| format!("the model's JSON has no overview: {v}"))?;
        overviews.push(overview);
        for (keys, out) in [
            (&["decisions"][..], &mut decisions),
            (&["open_questions", "openQuestions"][..], &mut questions),
        ] {
            for it in items(&v, keys, meeting) {
                match it.at_ms {
                    Some(at_ms) => out.push(Decision {
                        text: it.text,
                        at_ms,
                    }),
                    None => dropped += 1,
                }
            }
        }
        actions.extend(items(&v, &["action_items", "actionItems"], meeting));
    }
    let overview = if overviews.len() == 1 {
        overviews.remove(0)
    } else {
        let system = "Merge these partial overviews of one meeting, in order, into a single \
                      overview of two to four sentences. Reply with the overview only.";
        ask(llm, system, &overviews.join("\n\n"), false)
            .await?
            .trim()
            .to_string()
    };
    let actions = actions
        .into_iter()
        .enumerate()
        .map(|(n, it)| ActionItem {
            id: format!("{}-ai-{}", meeting.id, n + 1),
            meeting_id: meeting.id.clone(),
            meeting_title: meeting.title.clone(),
            at_ms: it.at_ms,
            needs_review: it.at_ms.is_none() || it.owner.is_empty(),
            text: it.text,
            owner: it.owner,
            due: it.due,
            client: meeting.client.clone(),
            done: false,
        })
        .collect();
    let quality = if dropped > 0 {
        format!("{dropped} item(s) left out: no valid transcript reference")
    } else {
        String::new()
    };
    let summary = Summary {
        template: template.name.clone(),
        overview,
        decisions,
        open_questions: questions,
        written_by: llm.id().into(),
        quality,
    };
    Ok((summary, actions))
}

// ---------- enhanced notes (NGR-147) ----------

const NOTES_RULES: &str = r#"You turn a person's rough meeting notes into clear notes, using the transcript.
Each transcript line starts with [line number, time]. Rules:
- Keep the person's own wording where it exists; complete it into a short sentence with the details from the transcript (who, what, when).
- Add the important decisions, action items and open questions the person did not write down.
- A note may start with (at m:ss): when the person wrote it during the call. The matching moment is usually just before that time.
- Every note cites the transcript line it comes from. Only use what was actually said.
Reply with one JSON object and nothing else, in this shape:
{"notes": [{"kind": "decision" | "action" | "question", "text": "...", "segment": 12}]}
Write in the language of the notes."#;

fn note_kind(kind: &str) -> TagKind {
    match kind {
        "decision" => TagKind::Decision,
        "action" | "action_item" | "action item" | "task" => TagKind::Action,
        "question" | "open_question" | "open question" => TagKind::OpenQuestion,
        _ => TagKind::Review,
    }
}

/// Merge the user's rough notes with the transcript. Returns new notes only; `meeting.notes.raw`
/// is never touched (the store keeps both).
pub async fn enhance_notes(meeting: &Meeting, llm: &impl LlmProvider) -> Result<Vec<EnhancedNote>> {
    if meeting.segments.is_empty() {
        return Err("nothing to enhance: the transcript is empty".into());
    }
    // Lines typed during the call carry the meeting time they were written (NGR-144).
    let raw: Vec<String> = meeting
        .notes
        .raw
        .iter()
        .enumerate()
        .filter(|(_, n)| !n.trim().is_empty())
        .map(
            |(i, n)| match meeting.notes.raw_at_ms.get(i).copied().flatten() {
                Some(ms) => format!("- (at {}) {}", clock(ms), n.trim()),
                None => format!("- {}", n.trim()),
            },
        )
        .collect();
    let notes = if raw.is_empty() {
        "(none: write the notes from the transcript)".to_string()
    } else {
        raw.join("\n")
    };
    let parts = chunks(&transcript_lines(meeting), CHUNK_CHARS);
    let mut out = vec![];
    for (i, part) in parts.iter().enumerate() {
        let user = format!(
            "{}\n\nMy notes:\n{notes}\n\nTranscript:\n{part}",
            header(meeting, i, parts.len())
        );
        let v = json_object(&ask(llm, NOTES_RULES, &user, true).await?)?;
        if !v["notes"].is_array() {
            return Err(format!("the model's JSON has no notes list: {v}"));
        }
        // ponytail: a note discussed in two chunks can come back twice; dedupe if it shows up.
        out.extend(items(&v, &["notes"], meeting).into_iter().filter_map(|it| {
            Some(EnhancedNote {
                kind: note_kind(&it.kind),
                text: it.text,
                at_ms: it.at_ms?,
            })
        }));
    }
    out.sort_by_key(|n| n.at_ms);
    Ok(out)
}

#[cfg(test)]
mod tests;

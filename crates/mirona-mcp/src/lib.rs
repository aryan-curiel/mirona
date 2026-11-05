//! Mirona's MCP server (NGR-148): read-only tools over the meeting store, on stdio, built on
//! the official `rmcp` SDK. Shared by the `mirona-mcp` binary and `mirona mcp`.
//!
//! Every call reloads `config.toml` and reads the meeting folders afresh into a private
//! in-memory index ([`Store::open_snapshot`]), so the server sees what the app wrote a moment
//! ago, works with the app closed, and never touches the app's `index.sqlite`.
//! ponytail: a full re-read per call; fine for hundreds of meetings, cache by folder mtime
//! past that.

use mirona_core::Meeting;
use mirona_engine::Paths;
use mirona_providers::config::Config;
use mirona_store::Store;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};
use std::fmt::Write;
use std::path::{Path, PathBuf};

/// Most transcript moments `search_meetings` returns.
const MAX_MOMENTS: usize = 30;

pub const DISABLED: &str = "The Mirona MCP server is turned off. Turn MCP on in Mirona's \
settings, or set `mcp_enabled = true` in config.toml.";

/// The per-user app folder; `MIRONA_APP_DIR` overrides it (tests, a second profile).
pub fn app_dir() -> PathBuf {
    std::env::var_os("MIRONA_APP_DIR").map_or_else(mirona_engine::app_dir, PathBuf::from)
}

/// Config and folders from `app_dir`, the same way the app resolves them.
pub fn load(app_dir: &Path) -> Result<(Config, Paths), String> {
    let cfg = Config::load(&mirona_engine::config_path(app_dir))?;
    let paths = Paths::resolve(&cfg, app_dir, &mirona_engine::documents_dir());
    Ok((cfg, paths))
}

/// Serve MCP on stdin/stdout until the client hangs up. Refuses when MCP is turned off.
pub fn serve() -> Result<(), String> {
    let server = Server { app_dir: app_dir() };
    if !load(&server.app_dir)?.0.mcp_enabled {
        return Err(DISABLED.into());
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let running = server
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|e| e.to_string())?;
        running.waiting().await.map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[derive(Clone)]
pub struct Server {
    app_dir: PathBuf,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SearchArgs {
    /// Keywords (every word is prefix-matched). Searches transcripts, titles, clients and
    /// notes. Empty: lists every meeting, newest first.
    pub query: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MeetingArgs {
    /// Meeting id, as returned by `search_meetings` (e.g. "m-1762331400000").
    pub id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct TranscriptArgs {
    /// Meeting id, as returned by `search_meetings`.
    pub id: String,
    /// Start of the range, "m:ss", "h:mm:ss" or seconds. Default: the start.
    pub from: Option<String>,
    /// End of the range (exclusive), same format. Default: the end.
    pub to: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ActionArgs {
    /// Only items for this client (case-insensitive, part of the name is enough).
    pub client: Option<String>,
    /// Only items not done yet.
    pub open_only: Option<bool>,
}

#[tool_router]
impl Server {
    #[tool(
        description = "Keyword search over all meetings. Returns the matching transcript \
        moments (meeting, timestamp, snippet with the hit in [brackets]) and the matching \
        meetings. Use it first to find meeting ids.",
        annotations(read_only_hint = true)
    )]
    fn search_meetings(&self, Parameters(a): Parameters<SearchArgs>) -> Result<String, String> {
        let store = self.store()?;
        Ok(search(&store, &a.query)?.to_string())
    }

    #[tool(
        description = "One meeting, compact: summary overview, decisions, action items, open \
        questions, the user's notes and enhanced notes, each with its timestamp. No transcript \
        (use get_transcript).",
        annotations(read_only_hint = true)
    )]
    fn get_meeting(&self, Parameters(a): Parameters<MeetingArgs>) -> Result<String, String> {
        let store = self.store()?;
        Ok(meeting(&store, &a.id)?.to_string())
    }

    #[tool(
        description = "The transcript of one meeting as \"[m:ss] Speaker: text\" lines, \
        optionally only a time range (from/to). Can be long: prefer get_meeting or a range.",
        annotations(read_only_hint = true)
    )]
    fn get_transcript(&self, Parameters(a): Parameters<TranscriptArgs>) -> Result<String, String> {
        let store = self.store()?;
        let from = a.from.as_deref().map(parse_at).transpose()?;
        let to = a.to.as_deref().map(parse_at).transpose()?;
        Ok(transcript(&store.meeting(&a.id)?, from, to))
    }

    #[tool(
        description = "Action items across all meetings, newest meeting first: text, owner, \
        due date, done, client and the meeting it came from. Filter by client and/or open only.",
        annotations(read_only_hint = true)
    )]
    fn list_action_items(&self, Parameters(a): Parameters<ActionArgs>) -> Result<String, String> {
        let store = self.store()?;
        let client = a.client.unwrap_or_default().to_lowercase();
        let items: Vec<Value> = store
            .action_items()
            .into_iter()
            .filter(|i| i.client.to_lowercase().contains(&client))
            .filter(|i| !(a.open_only.unwrap_or(false) && i.done))
            .map(|i| {
                json!({
                    "id": i.id, "text": i.text, "owner": i.owner, "due": i.due, "done": i.done,
                    "client": i.client, "meeting_id": i.meeting_id, "meeting": i.meeting_title,
                    "at": i.at_ms.map(at),
                })
            })
            .collect();
        Ok(json!({ "count": items.len(), "items": items }).to_string())
    }
}

#[tool_handler(
    name = "mirona",
    instructions = "Mirona is the user's local meeting memory: transcripts, summaries, \
    decisions and action items from their calls. Search first (search_meetings) to find meeting \
    ids, then get_meeting for the compact digest; fetch get_transcript only when exact wording \
    matters, ideally with a from/to range around a search hit."
)]
#[allow(clippy::unused_async_trait_impl)] // the macro's generated handlers
impl ServerHandler for Server {}

impl Server {
    /// A fresh read of the meetings folder, unless MCP was turned off in the meantime.
    fn store(&self) -> Result<Store, String> {
        let (cfg, paths) = load(&self.app_dir)?;
        if !cfg.mcp_enabled {
            return Err(DISABLED.into());
        }
        Store::open_snapshot(&paths.meetings)
    }
}

pub fn search(store: &Store, query: &str) -> Result<Value, String> {
    let meetings = store.search(query);
    let head = |id: &str| -> Result<Value, String> {
        let m = store.meeting(id)?;
        Ok(
            json!({ "meeting_id": m.id, "title": m.title, "client": m.client, "date": m.date_label }),
        )
    };
    let hits = store.search_hits(query);
    let truncated = hits.len() > MAX_MOMENTS;
    let mut moments = vec![];
    for h in hits.into_iter().take(MAX_MOMENTS) {
        let mut v = head(&h.meeting_id)?;
        v["at"] = at(h.start_ms).into();
        v["snippet"] = h.snippet.into();
        moments.push(v);
    }
    let meetings: Vec<Value> = meetings
        .iter()
        .map(|m| head(&m.id))
        .collect::<Result<_, _>>()?;
    Ok(json!({ "moments": moments, "moments_truncated": truncated, "meetings": meetings }))
}

pub fn meeting(store: &Store, id: &str) -> Result<Value, String> {
    let m = store.meeting(id)?;
    let s = m.summary.as_ref();
    let timed = |v: &[mirona_core::Decision]| -> Vec<Value> {
        v.iter()
            .map(|d| json!({ "at": at(d.at_ms), "text": d.text }))
            .collect()
    };
    let items: Vec<Value> = store
        .action_items()
        .into_iter()
        .filter(|a| a.meeting_id == m.id)
        .map(|a| {
            json!({ "id": a.id, "text": a.text, "owner": a.owner, "due": a.due, "done": a.done,
                    "at": a.at_ms.map(at) })
        })
        .collect();
    let enhanced: Vec<Value> = m
        .notes
        .enhanced
        .iter()
        .map(|n| json!({ "at": at(n.at_ms), "kind": n.kind, "text": n.text }))
        .collect();
    Ok(json!({
        "id": m.id,
        "title": m.title,
        "client": m.client,
        "date": m.date_label,
        "duration_min": m.duration_min,
        "speakers": m.speakers.iter().map(|p| &p.name).collect::<Vec<_>>(),
        "transcript_segments": m.segments.len(),
        "overview": s.map(|s| &s.overview),
        "decisions": s.map(|s| timed(&s.decisions)).unwrap_or_default(),
        "open_questions": s.map(|s| timed(&s.open_questions)).unwrap_or_default(),
        "action_items": items,
        "notes": m.notes.raw,
        "enhanced_notes": enhanced,
        "summary_by": s.map(|s| &s.written_by),
    }))
}

/// `[m:ss] Speaker: text` lines for segments starting in `[from, to)` (milliseconds).
pub fn transcript(m: &Meeting, from: Option<u32>, to: Option<u32>) -> String {
    let mut out = format!("# {} · {} · {}\n", m.title, m.client, m.date_label);
    for s in &m.segments {
        if from.is_some_and(|f| s.start_ms < f) || to.is_some_and(|t| s.start_ms >= t) {
            continue;
        }
        let who = m
            .speakers
            .iter()
            .find(|p| p.id == s.speaker_id)
            .map_or(s.speaker_id.as_str(), |p| &p.name);
        let text = if s.redacted { "[redacted]" } else { &s.text };
        writeln!(out, "[{}] {who}: {text}", at(s.start_ms)).ok();
    }
    out
}

/// Milliseconds → "m:ss" or "h:mm:ss".
pub fn at(ms: u32) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// "m:ss", "h:mm:ss" or plain seconds → milliseconds.
pub fn parse_at(t: &str) -> Result<u32, String> {
    let mut secs = 0u32;
    for part in t.trim().split(':') {
        let n: u32 = part
            .parse()
            .map_err(|_| format!("bad time {t:?}: use m:ss, h:mm:ss or seconds"))?;
        secs = secs * 60 + n;
    }
    Ok(secs * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_round_trip() {
        assert_eq!(at(65_000), "1:05");
        assert_eq!(at(3_725_000), "1:02:05");
        assert_eq!(parse_at("1:05").unwrap(), 65_000);
        assert_eq!(parse_at("1:02:05").unwrap(), 3_725_000);
        assert_eq!(parse_at("90").unwrap(), 90_000);
        assert!(parse_at("soon").is_err());
    }
}

//! Mirona's MCP server (NGR-148): tools over the meeting store, on stdio and (NGR-168)
//! streamable HTTP on localhost, built on the official `rmcp` SDK. Shared by the `mirona-mcp`
//! binary, `mirona mcp` and the app. Write tools (`write.rs`) are listed only while
//! `mcp_write` is on; every tool call lands in the activity log (`activity.rs`).
//!
//! Every call reloads `config.toml` and reads the meeting folders afresh into a private
//! in-memory index ([`Store::open_snapshot`]), so the server sees what the app wrote a moment
//! ago, works with the app closed, and never touches the app's `index.sqlite`.
//! Classification (NGR-158): answers below their threshold are never served as fact. Action
//! items carry `needs_review`, and `get_meeting` lists what is unconfirmed. Segments tagged
//! sensitive are hidden from agents (transcript and search snippets).
//! ponytail: a full re-read per call; fine for hundreds of meetings, cache by folder mtime
//! past that.

use mirona_core::{Meeting, SearchFilters, TagKind};
use mirona_engine::Paths;
use mirona_engine::search::Embed;
use mirona_providers::config::Config;
use mirona_store::Store;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolRequestParams, CallToolResponse};
use rmcp::service::RequestContext;
use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::fmt::Write;
use std::path::{Path, PathBuf};

pub mod activity;
mod web;
mod write;
pub use activity::{Activity, activity};
pub use web::{HttpServer, PATH as HTTP_PATH, new_token, serve_http};

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
    mirona_providers::net::set_privacy(cfg.privacy);
    let paths = Paths::resolve(&cfg, app_dir, &mirona_engine::documents_dir());
    Ok((cfg, paths))
}

/// Serve MCP on stdin/stdout until the client hangs up. Refuses when MCP is turned off.
pub fn serve() -> Result<(), String> {
    let server = Server::new(app_dir());
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

impl Server {
    pub fn new(app_dir: PathBuf) -> Self {
        Self { app_dir }
    }

    /// The read tools, plus the write tools while `mcp_write` is on (checked per request).
    fn router(&self) -> ToolRouter<Self> {
        let mut r = Self::tool_router();
        if load(&self.app_dir).is_ok_and(|(c, _)| c.mcp_write) {
            r.merge(Self::write_router());
        }
        r
    }
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct SearchArgs {
    /// What to look for: keywords (prefix-matched) and, when the local embedding model is
    /// available, meaning ("pricing worries" finds "the budget is tight"). Searches
    /// transcripts, titles, clients and notes. Empty: lists the meetings that pass the filters.
    pub query: String,
    /// Only this client (case-insensitive, part of the name is enough).
    pub client: Option<String>,
    /// First day, YYYY-MM-DD.
    pub from: Option<String>,
    /// Last day, YYYY-MM-DD (inclusive).
    pub to: Option<String>,
    /// Only lines said by this person (part of the name is enough).
    pub speaker: Option<String>,
    /// Only lines tagged: `action`, `decision`, `open_question`, `small_talk` or `sensitive`.
    pub tag: Option<String>,
    /// Include lines tagged sensitive. Only when the user asked for that content.
    pub include_sensitive: Option<bool>,
}

impl SearchArgs {
    pub fn query(q: &str) -> Self {
        Self {
            query: q.into(),
            ..Self::default()
        }
    }

    fn filters(&self) -> Result<SearchFilters, String> {
        let tag = match self
            .tag
            .as_deref()
            .map(|t| t.to_lowercase().replace([' ', '-'], "_"))
        {
            None => None,
            Some(t) => Some(match t.as_str() {
                "action" | "action_item" => TagKind::Action,
                "decision" => TagKind::Decision,
                "open_question" | "question" | "openquestion" => TagKind::OpenQuestion,
                "small_talk" | "smalltalk" => TagKind::SmallTalk,
                "sensitive" => TagKind::Sensitive,
                other => {
                    return Err(format!(
                        "unknown tag {other:?}: use action, decision, open_question, small_talk or sensitive"
                    ));
                }
            }),
        };
        let day = |d: &Option<String>| -> Result<Option<String>, String> {
            d.as_ref()
                .map(|d| {
                    d.trim()
                        .parse::<jiff::civil::Date>()
                        .map(|d| d.to_string())
                        .map_err(|_| format!("bad date {d:?}: use YYYY-MM-DD"))
                })
                .transpose()
        };
        Ok(SearchFilters {
            client: self.client.clone(),
            from: day(&self.from)?,
            to: day(&self.to)?,
            speaker: self.speaker.clone(),
            tag,
        })
    }
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
    /// Include lines tagged sensitive (passwords, salaries, health). Only when the user asked
    /// for that content. Redacted lines stay hidden regardless.
    pub include_sensitive: Option<bool>,
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
        description = "Search all meetings by keywords and meaning. Returns the matching \
        transcript moments, best first (meeting, timestamp, speaker, snippet with keyword hits \
        in [brackets]), and the matching meetings. Filter by client, date range, speaker or \
        tag. Use it first to find meeting ids.",
        annotations(read_only_hint = true)
    )]
    fn search_meetings(&self, Parameters(a): Parameters<SearchArgs>) -> Result<String, String> {
        let store = self.store()?;
        let (cfg, _) = load(&self.app_dir)?;
        let emb = mirona_engine::search::embedder(&cfg);
        Ok(search(&store, emb.as_ref(), &a)?.to_string())
    }

    #[tool(
        description = "One meeting, compact: summary overview, decisions, action items, open \
        questions, the user's notes and enhanced notes, each with its timestamp. No transcript \
        (use get_transcript). `unconfirmed` lists classifier guesses below their threshold \
        (meeting type, client, tags, action items): never treat them as fact.",
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
        let all = a.include_sensitive.unwrap_or(false);
        Ok(transcript(&store.meeting(&a.id)?, from, to, all))
    }

    #[tool(
        description = "Action items across all meetings, newest meeting first: text, owner, \
        due date, done, client and the meeting it came from. Filter by client and/or open only. \
        Items with needs_review: true are unconfirmed guesses: never treat them as fact.",
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
                let m = store.meeting(&i.meeting_id).ok();
                json!({
                    "id": i.id, "text": item_text(m.as_ref(), &i), "owner": i.owner, "due": i.due, "done": i.done,
                    "client": i.client, "meeting_id": i.meeting_id, "meeting": i.meeting_title,
                    "at": i.at_ms.map(at), "needs_review": i.needs_review,
                })
            })
            .collect();
        Ok(json!({ "count": items.len(), "items": items }).to_string())
    }
}

#[tool_handler(
    router = self.router(),
    name = "mirona",
    instructions = "Mirona is the user's local meeting memory: transcripts, summaries, \
    decisions and action items from their calls. Search first (search_meetings) to find meeting \
    ids, then get_meeting for the compact digest; fetch get_transcript only when exact wording \
    matters, ideally with a from/to range around a search hit."
)]
#[allow(clippy::unused_async_trait_impl)] // the macro's generated handlers
impl ServerHandler for Server {
    /// Routes the call and logs who called what (never arguments or results).
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let client = context
            .extensions
            .get::<http::request::Parts>()
            .and_then(|p| p.extensions.get::<web::ClientName>())
            .map_or_else(|| "stdio".to_string(), |c| c.0.clone());
        let tool = request.name.to_string();
        let res = self
            .router()
            .call(ToolCallContext::new(self, request, context))
            .await;
        let ok = matches!(&res, Ok(CallToolResponse::Complete(r)) if r.is_error != Some(true));
        activity::record(&self.app_dir, &client, &tool, ok);
        res
    }
}

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

/// `search_meetings`: the same hybrid search as the app (NGR-167). Sensitive moments are left
/// out unless asked for.
pub fn search(store: &Store, emb: Option<&impl Embed>, a: &SearchArgs) -> Result<Value, String> {
    let filters = a.filters()?;
    let all = a.include_sensitive.unwrap_or(false);
    // Generous limit: sensitive moments are dropped after.
    let r = mirona_engine::search::search_with(store, emb, &a.query, &filters, 4 * MAX_MOMENTS);
    let mut moments: Vec<Value> = r
        .moments
        .iter()
        .filter(|m| all || !m.sensitive)
        .map(|m| {
            let matched = match (m.by_keyword, m.by_meaning) {
                (true, true) => "keywords and meaning",
                (true, false) => "keywords",
                _ => "meaning",
            };
            json!({ "meeting_id": m.meeting_id, "title": m.title, "client": m.client,
                    "date": m.date, "at": at(m.start_ms), "speaker": m.speaker,
                    "snippet": m.snippet, "matched": matched })
        })
        .collect();
    let truncated = moments.len() > MAX_MOMENTS;
    moments.truncate(MAX_MOMENTS);
    let meetings: Vec<Value> = r
        .meetings
        .iter()
        .map(|m| {
            let d = store.meeting(&m.id)?;
            Ok(json!({ "meeting_id": d.id, "title": d.title, "client": d.client, "date": d.date_label }))
        })
        .collect::<Result<_, String>>()?;
    Ok(json!({
        "moments": moments,
        "moments_truncated": truncated,
        "meetings": meetings,
        "search_by_meaning": r.semantic,
    }))
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
            json!({ "id": a.id, "text": item_text(Some(&m), &a), "owner": a.owner, "due": a.due, "done": a.done,
                    "at": a.at_ms.map(at), "needs_review": a.needs_review })
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
        "speakers": m.speakers.iter().map(|p| json!({ "id": p.id, "name": p.name })).collect::<Vec<_>>(),
        "transcript_segments": m.segments.len(),
        "overview": s.map(|s| &s.overview),
        "decisions": s.map(|s| timed(&s.decisions)).unwrap_or_default(),
        "open_questions": s.map(|s| timed(&s.open_questions)).unwrap_or_default(),
        "action_items": items,
        "notes": m.notes.raw,
        "enhanced_notes": enhanced,
        "summary_by": s.map(|s| &s.written_by),
        "meeting_type": m.answers.as_ref()
            .filter(|a| a.meeting_type.confirmed || a.meeting_type.probability >= a.meeting_type.threshold)
            .map(|a| &a.meeting_type.value),
        "unconfirmed": store.review(id)?.iter().map(|r| json!({
            "question": r.question, "value": r.value, "at": r.at_ms.map(at),
        })).collect::<Vec<_>>(),
    }))
}

/// `[m:ss] Speaker: text` lines for segments starting in `[from, to)` (milliseconds).
/// Sensitive lines read `[redacted]` unless `include_sensitive`; redacted ones always do.
pub fn transcript(
    m: &Meeting,
    from: Option<u32>,
    to: Option<u32>,
    include_sensitive: bool,
) -> String {
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
        let text = if s.redacted || (sensitive(s) && !include_sensitive) {
            "[redacted]"
        } else {
            &s.text
        };
        writeln!(out, "[{}] {who}: {text}", at(s.start_ms)).ok();
    }
    out
}

/// An action item said on a sensitive or redacted line shows `[redacted]` instead.
fn item_text(m: Option<&Meeting>, a: &mirona_core::ActionItem) -> String {
    let hidden = m.is_some_and(|m| {
        m.segments
            .iter()
            .any(|s| Some(s.start_ms) == a.at_ms && (s.redacted || sensitive(s)))
    });
    if hidden {
        "[redacted]".into()
    } else {
        a.text.clone()
    }
}

/// Tagged sensitive (at/above threshold, or confirmed by the user): kept from agents.
fn sensitive(s: &mirona_core::Segment) -> bool {
    s.tags
        .iter()
        .any(|t| t.kind == mirona_core::TagKind::Sensitive)
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
    fn agents_never_get_sensitive_lines_or_unconfirmed_guesses_as_fact() {
        let store = Store::sample();
        let (m, seg) = store
            .list_meetings()
            .into_iter()
            .find_map(|m| {
                let m = store.meeting(&m.id).unwrap();
                let s = m.segments.iter().find(|s| sensitive(s))?.clone();
                Some((m, s))
            })
            .expect("the demo data has a sensitive line");
        let t = transcript(&m, None, None, false);
        assert!(!t.contains(&seg.text) && t.contains("[redacted]"), "{t}");
        let v = meeting(&store, &m.id).unwrap();
        assert!(v["unconfirmed"].is_array());
        assert!(
            v["action_items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|a| a["needs_review"].is_boolean())
        );
    }

    /// Writes back everything it was shown, so anything that reached the model would show up
    /// in the summary.
    struct Echo;
    impl mirona_providers::Provider for Echo {
        fn id(&self) -> &'static str {
            "echo"
        }
        fn locality(&self) -> mirona_core::Locality {
            mirona_core::Locality::Local
        }
    }
    impl mirona_engine::Labelled for Echo {
        fn label(&self) -> String {
            "echo".into()
        }
    }
    impl mirona_providers::LlmProvider for Echo {
        fn complete(
            &self,
            p: &mirona_providers::Prompt<'_>,
            on_text: &mut (dyn FnMut(&str) + Send),
        ) -> impl Future<Output = mirona_providers::Result<mirona_providers::Completion>> + Send
        {
            let text = if p.system.contains("\"notes\"") {
                json!({ "notes": [{ "kind": "note", "text": p.user, "segment": 0 }] })
            } else {
                json!({ "overview": p.user, "decisions": [{ "text": p.user, "segment": 0 }],
                        "action_items": [], "open_questions": [] })
            }
            .to_string();
            on_text(&text);
            std::future::ready(Ok(mirona_providers::Completion {
                text,
                usage: mirona_providers::Usage::default(),
            }))
        }
    }

    /// NGR-166: a meeting with a password and a salary leaks neither through the summary,
    /// the enhanced notes, nor any MCP tool, unless the agent asks for sensitive lines.
    #[test]
    fn a_password_and_a_salary_never_leak() {
        use mirona_core::{Tag, TagKind};
        const SECRETS: [&str; 2] = ["Tulip-4471", "182,000"];
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        let id = store.start_meeting("m-1").unwrap().id;
        let lines = [
            ("me", "Let's go through the onboarding."),
            ("them", "The staging password is Tulip-4471, by the way."),
            ("them", "And my salary would be 182,000 a year."),
            ("me", "I'll send the onboarding checklist."),
        ];
        for (n, (who, text)) in lines.iter().enumerate() {
            let sensitive = text.contains("Tulip") || text.contains("salary");
            let n32 = u32::try_from(n).unwrap();
            store
                .push_segment(
                    &id,
                    mirona_core::Segment {
                        id: format!("{id}-{n}"),
                        speaker_id: (*who).into(),
                        start_ms: n32 * 5000,
                        end_ms: n32 * 5000 + 4000,
                        text: (*text).into(),
                        redacted: false,
                        tags: if sensitive {
                            vec![Tag {
                                kind: TagKind::Sensitive,
                                label: "Sensitive".into(),
                            }]
                        } else {
                            vec![]
                        },
                        answers: vec![],
                        owners: vec![],
                        due: None,
                    },
                )
                .unwrap();
        }
        store
            .set_notes(&id, vec!["onboarding".into(), "checklist".into()])
            .unwrap();
        let paths = mirona_engine::Paths {
            meetings: dir.path().into(),
            models: dir.path().join("models"),
            templates: dir.path().join("templates"),
        };
        let shared = std::sync::Arc::new(std::sync::Mutex::new(store));
        mirona_engine::summarize_with(&shared, &paths, &id, "client-call", &Echo, &mut |_| {})
            .unwrap();
        let store = shared.lock().unwrap();
        let m = store.meeting(&id).unwrap();
        let summary = serde_json::to_string(&(&m.summary, &m.notes.enhanced)).unwrap();
        assert!(summary.contains("onboarding"), "the echo saw the meeting");

        let outputs = [
            summary,
            meeting(&store, &id).unwrap().to_string(),
            transcript(&m, None, None, false),
            search(&store, NO_EMB, &SearchArgs::query("Tulip"))
                .unwrap()
                .to_string(),
            search(&store, NO_EMB, &SearchArgs::query("salary"))
                .unwrap()
                .to_string(),
        ];
        for out in &outputs {
            for s in SECRETS {
                assert!(!out.contains(s), "{s} leaked: {out}");
            }
        }
        let asked = transcript(&m, None, None, true);
        assert!(SECRETS.iter().all(|s| asked.contains(s)), "{asked}");
    }

    const NO_EMB: Option<&mirona_providers::ollama::OllamaEmbedder> = None;

    #[test]
    fn search_filters_are_checked() {
        let store = Store::sample();
        let ok = |a: SearchArgs| search(&store, NO_EMB, &a);
        assert!(
            ok(SearchArgs {
                tag: Some("vibes".into()),
                ..SearchArgs::default()
            })
            .is_err()
        );
        assert!(
            ok(SearchArgs {
                from: Some("last tuesday".into()),
                ..SearchArgs::default()
            })
            .is_err()
        );
        let v = ok(SearchArgs {
            query: "SOW".into(),
            tag: Some("action".into()),
            from: Some("2000-01-01".into()),
            ..SearchArgs::default()
        })
        .unwrap();
        assert!(v["moments"].as_array().is_some(), "{v}");
        assert_eq!(v["search_by_meaning"], false);
    }

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

//! Which provider runs each stage, read from `config.toml` instead of code (NGR-133).
//!
//! ```toml
//! privacy = true
//! [providers]
//! stt = "parakeet"
//! classification = "local-llm"
//! ```

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)] // independent on/off settings
#[serde(default)]
pub struct Config {
    /// Privacy mode: cloud providers are refused. On by default.
    pub privacy: bool,
    /// Stage id → provider id. Stages left out keep their default.
    pub providers: BTreeMap<String, String>,
    /// Where meeting folders live. `None`: the app's default (`~/Documents/Mirona`).
    pub meetings_dir: Option<PathBuf>,
    /// Speech, VAD and diarization models (`scripts/fetch-models.sh`). `None`: app default.
    pub models_dir: Option<PathBuf>,
    /// Summary template id used after each recording (NGR-146).
    pub template: String,
    pub llm: LlmSettings,
    /// The local model behind `classification = "local-llm"` (Ollama). Empty model:
    /// `qwen3:4b`; empty URL: the summary's Ollama server ([`LlmSettings`]).
    pub classifier: LlmSettings,
    /// Let MCP clients read meetings. Off: `mirona mcp` refuses to serve (NGR-148/150).
    pub mcp_enabled: bool,
    /// Also serve MCP over streamable HTTP on 127.0.0.1 (`mcp_port`), for clients that can't
    /// spawn a process. Every request needs one of `mcp_tokens` (NGR-168).
    pub mcp_http: bool,
    pub mcp_port: u16,
    /// MCP client name → its bearer token. The name is what the activity log shows.
    pub mcp_tokens: BTreeMap<String, String>,
    /// Let MCP clients change meetings: mark action items done, rename speakers.
    pub mcp_write: bool,
    /// Let MCP clients read sensitive lines when they ask (`include_sensitive`). Off, they
    /// always read `[redacted]`.
    pub mcp_sensitive: bool,
    /// Global start/stop-recording shortcut, as a Tauri accelerator ("CmdOrCtrl+Alt+Shift+R").
    pub shortcut: String,
    /// Global mute/unmute-my-mic shortcut while recording.
    pub mute_shortcut: String,
    /// Question id → threshold, overriding the defaults in [`default_threshold`] (NGR-158).
    pub thresholds: BTreeMap<String, f32>,
    /// Question id → floor, overriding [`default_floor`]: a model "yes" below it is rejected
    /// outright instead of asked about (only counted, viewable on demand).
    pub floors: BTreeMap<String, f32>,
    /// The user's tags: `group:name` (or `name`). The classifier asks one yes/no per tag over
    /// each meeting; a meeting can have many. Values are always empty: tags had descriptions
    /// before NGR-202, a map keeps those files loading (they're emptied on load).
    pub tags: BTreeMap<String, String>,
    /// Before tags: client name → description. Read once and moved to `project:<name>` tags.
    #[serde(skip_serializing)]
    pub clients: BTreeMap<String, String>,
    /// Suggest recording when a call app starts using the mic (NGR-170).
    pub detect_calls: bool,
    /// Read the title of the calendar event happening now to name the meeting (NGR-170).
    pub calendar_titles: bool,
    /// Tags whose calls (calendar title names the tag) start recording without a click.
    pub auto_record: Vec<String>,
    /// Encrypt recordings, transcripts, embeddings and voiceprints at rest (NGR-165). The key
    /// is in the OS keychain (account [`STORE_KEY`]).
    pub encrypt: bool,
    pub retention: Retention,
    /// Remind me to get consent before each recording (NGR-169). On unless turned off.
    pub consent_reminder: bool,
    /// What to paste in the meeting chat.
    pub consent_message: String,
    /// Tags whose people already agreed: no reminder when recording them.
    pub consent_given: Vec<String>,
    /// Check GitHub for a new version on start and daily. Off until the user turns it on.
    pub check_updates: bool,
}

pub const DEFAULT_CONSENT_MESSAGE: &str = "Heads-up: I'm recording this call to take notes. The \
recording and transcript stay on my computer. Tell me if you'd rather I didn't.";

/// The group clients move into (`project:<client>`).
pub const PROJECT_GROUP: &str = "project";

/// Keychain account of the meeting store's encryption key.
pub const STORE_KEY: &str = "store-key";

/// What to forget, and when (NGR-165). Runs on app start, then hourly; `mirona retention`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Retention {
    /// Delete recordings this many days old; transcripts, notes and summaries stay.
    pub audio_days: Option<u32>,
    /// Tag → delete meetings with that tag entirely once this many days old.
    pub tags: BTreeMap<String, u32>,
    /// Before tags (client → days); moved to `tags` on load.
    #[serde(skip_serializing)]
    pub clients: BTreeMap<String, u32>,
}

/// Set to `1` to force privacy mode on, whatever `config.toml` says.
pub const PRIVACY_ENV: &str = "MIRONA_PRIVACY";

/// Cmd+Shift+R is a browser's hard reload, so one more modifier keeps it out of other apps' way.
/// Default port for MCP over HTTP.
pub const DEFAULT_MCP_PORT: u16 = 7337;

pub const DEFAULT_SHORTCUT: &str = "CmdOrCtrl+Alt+Shift+R";
pub const DEFAULT_MUTE_SHORTCUT: &str = "CmdOrCtrl+Alt+Shift+M";

/// Which model the summary stage uses. API keys live in the OS keychain, never here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmSettings {
    /// e.g. `qwen3:4b`, `gpt-4.1-mini`, `claude-opus-5-5`. Empty: the provider's default.
    pub model: String,
    /// OpenAI-compatible endpoint (LM Studio, vLLM…). Empty: the provider's default.
    pub base_url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            privacy: true,
            providers: BTreeMap::new(),
            meetings_dir: None,
            models_dir: None,
            template: "client-call".into(),
            llm: LlmSettings::default(),
            classifier: LlmSettings::default(),
            // Off until the user lets agents in (onboarding or Settings).
            mcp_enabled: false,
            mcp_http: false,
            mcp_port: DEFAULT_MCP_PORT,
            mcp_tokens: BTreeMap::new(),
            mcp_write: false,
            mcp_sensitive: false,
            shortcut: DEFAULT_SHORTCUT.into(),
            mute_shortcut: DEFAULT_MUTE_SHORTCUT.into(),
            thresholds: BTreeMap::new(),
            floors: BTreeMap::new(),
            tags: BTreeMap::new(),
            clients: BTreeMap::new(),
            detect_calls: true,
            calendar_titles: false,
            auto_record: vec![],
            encrypt: false,
            retention: Retention::default(),
            consent_reminder: true,
            consent_message: DEFAULT_CONSENT_MESSAGE.into(),
            consent_given: vec![],
            check_updates: false,
        }
    }
}

/// Thresholds from the NGR-130 eval (ADR 0001), raised to 0.85 for the questions that end up
/// in "Needs review" (0.60 sent too much there as fact); 0.95 for `decision` (Jev over-calls
/// it). The local fallback (`local`) is overconfident: 0.99 for `action`, 0.95 for `decision`
/// and `unanswered`, and 0.50 for `sensitive` (no threshold fixes its misses, so any lean
/// towards yes counts). `sensitive`, `owner`, `due`, `speaker_name` and `coverage` stay 0.60.
pub fn default_threshold(question: &str, local: bool) -> f32 {
    match (question, local) {
        ("decision", _) | ("unanswered", true) => 0.95,
        ("action", true) => 0.99,
        ("sensitive", true) => 0.5,
        ("action" | "unanswered" | "smalltalk" | "meeting_type" | "tag" | "follow_up", _) => 0.85,
        _ => 0.6,
    }
}

/// Below its floor a model "yes" is rejected without asking: 0.60 for the questions that end
/// up in "Needs review". `sensitive` has none: a line the model leans sensitive on is always
/// shown, never silently dropped.
pub fn default_floor(question: &str) -> f32 {
    match question {
        "action" | "decision" | "unanswered" | "smalltalk" | "meeting_type" | "tag"
        | "follow_up" => 0.6,
        _ => 0.0,
    }
}

impl Config {
    /// The threshold for a question: the user's, or the default for this classifier.
    pub fn threshold(&self, question: &str, local: bool) -> f32 {
        self.thresholds
            .get(question)
            .copied()
            .unwrap_or_else(|| default_threshold(question, local))
    }

    /// The floor for a question: the user's, or [`default_floor`].
    pub fn floor(&self, question: &str) -> f32 {
        self.floors
            .get(question)
            .copied()
            .unwrap_or_else(|| default_floor(question))
    }

    /// A missing file means defaults; a broken one is an error, never silently reset.
    /// `MIRONA_PRIVACY=1` (the CLI's `--privacy`) forces privacy mode on for this process.
    pub fn load(path: &Path) -> Result<Self, String> {
        let mut cfg: Self = match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).map_err(|e| format!("{}: {e}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        if std::env::var_os(PRIVACY_ENV).is_some_and(|v| v == "1") {
            cfg.privacy = true;
        }
        cfg.migrate_clients();
        cfg.tags.values_mut().for_each(String::clear);
        Ok(cfg)
    }

    /// Clients (before tags) become `project:<name>` tags, and the rules keyed by client
    /// follow them. Saved in the new shape on the next save.
    pub fn migrate_clients(&mut self) {
        let clients = std::mem::take(&mut self.clients);
        let days = std::mem::take(&mut self.retention.clients);
        if clients.is_empty() && days.is_empty() {
            return;
        }
        let tag = |c: &str| format!("{PROJECT_GROUP}:{}", c.trim());
        let known = |c: &str| clients.keys().any(|k| k.eq_ignore_ascii_case(c.trim()));
        for (c, about) in &clients {
            self.tags.entry(tag(c)).or_insert_with(|| about.clone());
        }
        for (c, d) in days {
            self.retention.tags.entry(tag(&c)).or_insert(d);
        }
        for list in [&mut self.auto_record, &mut self.consent_given] {
            for c in list.iter_mut().filter(|c| known(c)) {
                *c = tag(c);
            }
        }
        if let Some(t) = self.thresholds.remove("client") {
            self.thresholds.entry("tag".into()).or_insert(t);
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let s = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, s).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clients_become_project_tags_with_their_rules() {
        let mut cfg: Config = toml::from_str(
            r#"
auto_record = ["Northwind", "standup"]
consent_given = ["northwind"]
[clients]
Northwind = "a bank"
[thresholds]
client = 0.7
[retention.clients]
Northwind = 90
"#,
        )
        .unwrap();
        cfg.migrate_clients();
        assert!(cfg.clients.is_empty() && cfg.retention.clients.is_empty());
        assert_eq!(cfg.tags["project:Northwind"], "a bank");
        assert_eq!(cfg.retention.tags["project:Northwind"], 90);
        assert_eq!(cfg.auto_record, ["project:Northwind", "standup"]);
        assert_eq!(cfg.consent_given, ["project:northwind"]);
        assert!((cfg.threshold("tag", false) - 0.7).abs() < 1e-6);
        // The old keys are never written back.
        let s = toml::to_string_pretty(&cfg).unwrap();
        assert!(!s.contains("[clients]") && !s.contains("retention.clients"));
    }

    #[test]
    fn old_described_tags_load_with_descriptions_dropped() {
        let dir = std::env::temp_dir().join(format!("mirona-tags-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            r#"
[tags]
"project:setr" = "Setr app work"
[tag_groups]
project = "Clients"
[clients]
Atlas = "Atlas Corp"
"#,
        )
        .unwrap();
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.tags["project:setr"], "");
        assert_eq!(cfg.tags["project:Atlas"], "");
        let s = toml::to_string_pretty(&cfg).unwrap();
        assert!(!s.contains("tag_groups") && !s.contains("Setr app"), "{s}");
    }

    #[test]
    fn missing_file_is_default_and_round_trips() {
        let dir = std::env::temp_dir().join(format!("mirona-cfg-{}", std::process::id()));
        let path = dir.join("config.toml");
        assert_eq!(Config::load(&path).unwrap(), Config::default());

        let mut cfg = Config {
            privacy: false,
            ..Config::default()
        };
        cfg.providers.insert("classification".into(), "jev".into());
        cfg.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), cfg);

        // Files written before the shortcut setting existed get the default.
        std::fs::write(&path, "privacy = false").unwrap();
        assert_eq!(Config::load(&path).unwrap().shortcut, DEFAULT_SHORTCUT);
        assert_eq!(
            Config::load(&path).unwrap().mute_shortcut,
            DEFAULT_MUTE_SHORTCUT
        );
        cfg.shortcut = "CmdOrCtrl+Shift+F9".into();
        cfg.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), cfg);

        // Thresholds and tags round-trip; unset thresholds keep their defaults.
        cfg.thresholds.insert("action".into(), 0.8);
        cfg.floors.insert("action".into(), 0.4);
        cfg.tags
            .insert("project:Northwind Bank".into(), String::new());
        cfg.save(&path).unwrap();
        let back = Config::load(&path).unwrap();
        assert_eq!(back, cfg);
        assert!((back.threshold("action", true) - 0.8).abs() < 1e-6);
        assert!((back.threshold("decision", false) - 0.95).abs() < 1e-6);
        assert!((back.threshold("smalltalk", true) - 0.85).abs() < 1e-6);
        assert!((back.threshold("owner", false) - 0.6).abs() < 1e-6);
        // Floors: the user's, 0.60 for review questions, none for sensitive.
        assert!((back.floor("action") - 0.4).abs() < 1e-6);
        assert!((back.floor("tag") - 0.6).abs() < 1e-6);
        assert!(back.floor("sensitive") < f32::EPSILON);

        // MCP over HTTP: off, read-only and on the default port unless set.
        assert_eq!(back.mcp_port, DEFAULT_MCP_PORT);
        assert!(!back.mcp_http && !back.mcp_write);
        cfg.mcp_http = true;
        cfg.mcp_write = true;
        cfg.mcp_port = 9000;
        cfg.mcp_tokens
            .insert("Claude Desktop".into(), "abc123".into());
        // Call detection: on by default, calendar and auto-record opt in.
        std::fs::write(&path, "privacy = false").unwrap();
        let old = Config::load(&path).unwrap();
        assert!(old.detect_calls && !old.calendar_titles && old.auto_record.is_empty());
        cfg.detect_calls = false;
        cfg.calendar_titles = true;
        cfg.auto_record = vec!["Northwind Bank".into()];
        cfg.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), cfg);

        // Encryption and retention: off and keep everything unless set.
        assert!(!old.encrypt && old.retention == Retention::default());
        // The consent reminder is on unless turned off, with a message to paste.
        assert!(old.consent_reminder && old.consent_message == DEFAULT_CONSENT_MESSAGE);
        cfg.consent_reminder = false;
        cfg.consent_given = vec!["Helio Health".into()];
        cfg.encrypt = true;
        cfg.retention.audio_days = Some(30);
        cfg.retention
            .tags
            .insert("project:Northwind Bank".into(), 90);
        cfg.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), cfg);

        std::fs::write(&path, "privacy = \"maybe\"").unwrap();
        assert!(Config::load(&path).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

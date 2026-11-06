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
    /// Let MCP clients read meetings. Off: `mirona mcp` refuses to serve (NGR-148/150).
    pub mcp_enabled: bool,
    /// Global start/stop-recording shortcut, as a Tauri accelerator ("CmdOrCtrl+Alt+Shift+R").
    pub shortcut: String,
    /// Question id → threshold, overriding the defaults in [`default_threshold`] (NGR-158).
    pub thresholds: BTreeMap<String, f32>,
    /// The user's clients/projects: name → short description (contact, what they do). The
    /// classifier picks one per meeting (NGR-157).
    pub clients: BTreeMap<String, String>,
    /// Suggest recording when a call app starts using the mic (NGR-170).
    pub detect_calls: bool,
    /// Read the title of the calendar event happening now to name the meeting (NGR-170).
    pub calendar_titles: bool,
    /// Clients whose calls (matched by calendar title) start recording without a click.
    pub auto_record: Vec<String>,
}

/// Cmd+Shift+R is a browser's hard reload, so one more modifier keeps it out of other apps' way.
pub const DEFAULT_SHORTCUT: &str = "CmdOrCtrl+Alt+Shift+R";

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
            mcp_enabled: true,
            shortcut: DEFAULT_SHORTCUT.into(),
            thresholds: BTreeMap::new(),
            clients: BTreeMap::new(),
            detect_calls: true,
            calendar_titles: false,
            auto_record: vec![],
        }
    }
}

/// Thresholds from the NGR-130 eval (ADR 0001): 0.60, but 0.95 for `decision` (Jev over-calls
/// it). The local fallback (`local`) is overconfident: 0.99 for `action`, 0.95 for `decision`
/// and `unanswered`, and 0.50 for `sensitive` (no threshold fixes its misses, so any lean
/// towards yes counts).
pub fn default_threshold(question: &str, local: bool) -> f32 {
    match (question, local) {
        ("decision", _) | ("unanswered", true) => 0.95,
        ("action", true) => 0.99,
        ("sensitive", true) => 0.5,
        _ => 0.6,
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

    /// A missing file means defaults; a broken one is an error, never silently reset.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
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
        cfg.shortcut = "CmdOrCtrl+Shift+F9".into();
        cfg.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), cfg);

        // Thresholds and clients round-trip; unset thresholds keep their defaults.
        cfg.thresholds.insert("action".into(), 0.8);
        cfg.clients
            .insert("Northwind Bank".into(), "contact: Karen".into());
        cfg.save(&path).unwrap();
        let back = Config::load(&path).unwrap();
        assert_eq!(back, cfg);
        assert!((back.threshold("action", true) - 0.8).abs() < 1e-6);
        assert!((back.threshold("decision", false) - 0.95).abs() < 1e-6);
        assert!((back.threshold("smalltalk", true) - 0.6).abs() < 1e-6);

        // Call detection: on by default, calendar and auto-record opt in.
        std::fs::write(&path, "privacy = false").unwrap();
        let old = Config::load(&path).unwrap();
        assert!(old.detect_calls && !old.calendar_titles && old.auto_record.is_empty());
        cfg.detect_calls = false;
        cfg.calendar_titles = true;
        cfg.auto_record = vec!["Northwind Bank".into()];
        cfg.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), cfg);

        std::fs::write(&path, "privacy = \"maybe\"").unwrap();
        assert!(Config::load(&path).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

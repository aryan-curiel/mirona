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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
}

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
        }
    }
}

impl Config {
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

        std::fs::write(&path, "privacy = \"maybe\"").unwrap();
        assert!(Config::load(&path).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

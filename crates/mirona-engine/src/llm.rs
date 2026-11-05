//! The summary LLM picked in config: `[providers] llm = "ollama" | "openai" | "anthropic"`.

use crate::Labelled;
use mirona_core::{Locality, ProviderOption};
use mirona_providers::anthropic::Anthropic;
use mirona_providers::config::Config;
use mirona_providers::ollama::Ollama;
use mirona_providers::openai::OpenAi;
use mirona_providers::{
    Completion, LlmProvider, Prompt, Provider, Result, check_privacy, keychain,
};

pub enum Llm {
    Ollama(Ollama, String),
    OpenAi(OpenAi, String),
    Anthropic(Anthropic, String),
}

impl Llm {
    /// Build the configured LLM. Cloud ones are refused in privacy mode, and need their API
    /// key in the keychain (service "mirona", account = provider id).
    pub fn from_config(cfg: &Config) -> std::result::Result<Self, String> {
        let id = crate::stage(cfg, "llm", "ollama");
        let model = |default: &str| {
            if cfg.llm.model.is_empty() {
                default.to_string()
            } else {
                cfg.llm.model.clone()
            }
        };
        let base = |default: &str| {
            if cfg.llm.base_url.is_empty() {
                default.to_string()
            } else {
                cfg.llm.base_url.clone()
            }
        };
        let key = || keychain::api_key(&id).ok().flatten();
        let llm = match id.as_str() {
            "ollama" => {
                let m = model(mirona_providers::ollama::MODEL);
                Self::Ollama(Ollama::new(&m, base(mirona_providers::ollama::URL)), m)
            }
            "openai" => {
                let m = model("gpt-4.1-mini");
                let llm = OpenAi::new(base(mirona_providers::openai::OPENAI_URL), &m, key());
                Self::OpenAi(llm, m)
            }
            "anthropic" => {
                let k = key().ok_or("no Anthropic API key in the keychain (Settings)")?;
                let m = model(mirona_providers::anthropic::MODEL);
                Self::Anthropic(Anthropic::new(k, &m), m)
            }
            other => return Err(format!("unknown LLM provider {other}")),
        };
        check_privacy(
            &ProviderOption {
                id: id.clone(),
                name: llm.label(),
                locality: llm.locality(),
            },
            cfg.privacy,
        )
        .map_err(|e| e.to_string())?;
        Ok(llm)
    }
}

impl Labelled for Llm {
    fn label(&self) -> String {
        match self {
            Self::Ollama(_, m) => format!("{m} (Ollama)"),
            Self::OpenAi(_, m) => format!("{m} (OpenAI-compatible)"),
            Self::Anthropic(_, m) => format!("{m} (Anthropic)"),
        }
    }
}

impl Provider for Llm {
    fn id(&self) -> &str {
        match self {
            Self::Ollama(p, _) => p.id(),
            Self::OpenAi(p, _) => p.id(),
            Self::Anthropic(p, _) => p.id(),
        }
    }
    fn locality(&self) -> Locality {
        match self {
            Self::Ollama(p, _) => p.locality(),
            Self::OpenAi(p, _) => p.locality(),
            Self::Anthropic(p, _) => p.locality(),
        }
    }
}

impl LlmProvider for Llm {
    async fn complete(
        &self,
        prompt: &Prompt<'_>,
        on_text: &mut (dyn FnMut(&str) + Send),
    ) -> Result<Completion> {
        match self {
            Self::Ollama(p, _) => p.complete(prompt, on_text).await,
            Self::OpenAi(p, _) => p.complete(prompt, on_text).await,
            Self::Anthropic(p, _) => p.complete(prompt, on_text).await,
        }
    }
}

//! One trait per pipeline stage (NGR-133). Real providers (Parakeet, speakrs, Jev, Ollama…)
//! land in later milestones; the `fake` module backs the prototype and tests.

pub mod config;

use mirona_core::{Answer, Locality, ProviderOption, Segment};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// A cloud provider was asked to run while privacy mode is on.
    PrivacyBlocked(String),
    Unknown(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PrivacyBlocked(p) => write!(f, "{p} is a cloud provider; privacy mode is on"),
            Self::Unknown(p) => write!(f, "unknown provider {p}"),
        }
    }
}

/// Every provider says where it runs, so privacy mode can refuse cloud ones in one place.
pub trait Provider {
    fn id(&self) -> &str;
    fn locality(&self) -> Locality;
}

/// The single privacy gate. Every provider choice goes through here.
pub fn check_privacy(option: &ProviderOption, privacy_mode: bool) -> Result<()> {
    if privacy_mode && option.locality == Locality::Cloud {
        return Err(Error::PrivacyBlocked(option.name.clone()));
    }
    Ok(())
}

pub trait SttProvider: Provider {
    /// Transcribe a chunk of 16 kHz mono PCM.
    fn transcribe(&self, pcm: &[f32]) -> impl Future<Output = Result<String>> + Send;
}

pub trait Diarizer: Provider {
    /// Speaker index for each chunk.
    fn diarize(&self, pcm: &[f32]) -> impl Future<Output = Result<Vec<u32>>> + Send;
}

pub trait Classifier: Provider {
    /// Answer every typed question about a segment in one call.
    fn classify(&self, segment: &Segment) -> impl Future<Output = Result<Vec<Answer>>> + Send;
}

pub trait LlmProvider: Provider {
    fn complete(&self, prompt: &str) -> impl Future<Output = Result<String>> + Send;
}

pub trait Embedder: Provider {
    fn embed(&self, text: &str) -> impl Future<Output = Result<Vec<f32>>> + Send;
}

/// Deterministic fakes for the prototype and tests.
pub mod fake {
    use super::{
        Answer, Classifier, Diarizer, Embedder, LlmProvider, Locality, Provider, Result, Segment,
        SttProvider,
    };
    use mirona_core::{AnswerSource, QuestionKind};

    pub struct Fake(pub &'static str);

    impl Provider for Fake {
        fn id(&self) -> &str {
            self.0
        }
        fn locality(&self) -> Locality {
            Locality::Local
        }
    }

    impl SttProvider for Fake {
        async fn transcribe(&self, _pcm: &[f32]) -> Result<String> {
            Ok("Sample transcript line.".into())
        }
    }

    impl Diarizer for Fake {
        async fn diarize(&self, pcm: &[f32]) -> Result<Vec<u32>> {
            Ok(vec![0; pcm.len().div_ceil(16_000)])
        }
    }

    impl Classifier for Fake {
        async fn classify(&self, segment: &Segment) -> Result<Vec<Answer>> {
            // ponytail: keyword heuristic stands in for Jev until NGR-155.
            let action = ["I'll", "I will", "can you", "send"]
                .iter()
                .any(|k| segment.text.contains(k));
            Ok(vec![Answer {
                id: "action".into(),
                question: "Is this an action item?".into(),
                kind: QuestionKind::YesNo,
                answer: action,
                probability: if action { 0.82 } else { 0.08 },
                source: AnswerSource::Model,
            }])
        }
    }

    impl LlmProvider for Fake {
        async fn complete(&self, prompt: &str) -> Result<String> {
            Ok(format!("Summary of {} characters.", prompt.len()))
        }
    }

    impl Embedder for Fake {
        async fn embed(&self, text: &str) -> Result<Vec<f32>> {
            #[allow(clippy::cast_precision_loss)]
            Ok(vec![text.len() as f32])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privacy_blocks_cloud_only() {
        let opt = |l| ProviderOption {
            id: "x".into(),
            name: "X".into(),
            locality: l,
        };
        assert!(check_privacy(&opt(Locality::Local), true).is_ok());
        assert!(check_privacy(&opt(Locality::Cloud), false).is_ok());
        assert_eq!(
            check_privacy(&opt(Locality::Cloud), true),
            Err(Error::PrivacyBlocked("X".into()))
        );
    }
}

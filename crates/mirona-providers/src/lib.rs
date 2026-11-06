//! One trait per pipeline stage (NGR-133). Real providers (Parakeet, speakrs, Jev, Ollama…)
//! land in later milestones; the `fake` module backs the prototype and tests.

pub mod config;
pub mod questions;

#[cfg(feature = "anthropic")]
pub mod anthropic;
#[cfg(feature = "jev")]
pub mod jev;
#[cfg(feature = "keychain")]
pub mod keychain;
#[cfg(feature = "ollama")]
pub mod ollama;
#[cfg(feature = "openai")]
pub mod openai;
#[cfg(feature = "parakeet")]
pub mod parakeet;
pub mod speakrs;
#[cfg(feature = "whisper")]
pub mod whisper;

use mirona_core::{Locality, ProviderOption};
use questions::{Question, Reply};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// A cloud provider was asked to run while privacy mode is on.
    PrivacyBlocked(String),
    Unknown(String),
    /// The provider ran and failed (model missing, decode error, HTTP error…).
    Provider(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PrivacyBlocked(p) => write!(f, "{p} is a cloud provider; privacy mode is on"),
            Self::Unknown(p) => write!(f, "unknown provider {p}"),
            Self::Provider(e) => write!(f, "provider failed: {e}"),
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

/// One stretch of speech by one (anonymous) speaker.
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    pub start_ms: u32,
    pub end_ms: u32,
    /// Index into `Diarization::speakers`.
    pub speaker: usize,
}

/// A speaker found in a recording, with the averaged embedding used to match voiceprints.
#[derive(Debug, Clone, PartialEq)]
pub struct FoundSpeaker {
    pub embedding: Vec<f32>,
    pub speech_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Diarization {
    /// Ordered by start; may overlap.
    pub turns: Vec<Turn>,
    pub speakers: Vec<FoundSpeaker>,
}

/// Who spoke when. Runs once after the call (ADR 0001); voiceprint matching lives in the store.
pub trait Diarizer: Provider {
    /// Diarize a whole recording of 16 kHz mono PCM.
    fn diarize(&self, pcm: &[f32]) -> impl Future<Output = Result<Diarization>> + Send;
    /// Speaker embedding of an enrollment clip (≥ 30 s of one person).
    fn voiceprint(&self, pcm: &[f32]) -> impl Future<Output = Result<Vec<f32>>> + Send;
}

/// Typed questions (yes/no, choice, score) about one text, all in one call (NGR-155).
pub trait Classifier: Provider {
    /// One reply per question, in question order.
    fn ask(
        &self,
        text: &str,
        questions: &[Question],
    ) -> impl Future<Output = Result<Vec<Reply>>> + Send;
}

/// What an LLM is asked: a system prompt, the user content, and whether to answer in JSON.
#[derive(Debug, Clone, Copy)]
pub struct Prompt<'a> {
    pub system: &'a str,
    pub user: &'a str,
    /// Ask for a single JSON object; providers that can enforce it do.
    pub json: bool,
}

/// Token counts reported by the provider (0 when it doesn't report them).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub text: String,
    pub usage: Usage,
}

/// The only stage that writes prose (summaries, enhanced notes). NGR-145.
pub trait LlmProvider: Provider {
    /// Stream a completion: `on_text` gets each chunk as it arrives; the full text and token
    /// use come back at the end.
    fn complete(
        &self,
        prompt: &Prompt<'_>,
        on_text: &mut (dyn FnMut(&str) + Send),
    ) -> impl Future<Output = Result<Completion>> + Send;
}

pub trait Embedder: Provider {
    fn embed(&self, text: &str) -> impl Future<Output = Result<Vec<f32>>> + Send;
}

/// Deterministic fakes for the prototype and tests.
pub mod fake {
    use super::{
        Classifier, Completion, Diarization, Diarizer, Embedder, FoundSpeaker, LlmProvider,
        Locality, Prompt, Provider, Question, Reply, Result, SttProvider, Turn, Usage,
    };
    use crate::questions::{Kind, options};

    pub struct Fake(pub &'static str);

    impl Provider for Fake {
        fn id(&self) -> &str {
            self.0
        }
        fn locality(&self) -> Locality {
            Locality::Local
        }
    }

    // Fakes answer instantly, so they hand back ready futures instead of `async fn`.
    use std::future::ready;

    impl SttProvider for Fake {
        fn transcribe(&self, _pcm: &[f32]) -> impl Future<Output = Result<String>> + Send {
            ready(Ok("Sample transcript line.".into()))
        }
    }

    /// One speaker talking the whole time.
    impl Diarizer for Fake {
        fn diarize(&self, pcm: &[f32]) -> impl Future<Output = Result<Diarization>> + Send {
            let ms = u32::try_from(pcm.len() / 16).unwrap_or(u32::MAX);
            ready(Ok(Diarization {
                turns: vec![Turn {
                    start_ms: 0,
                    end_ms: ms,
                    speaker: 0,
                }],
                speakers: vec![FoundSpeaker {
                    embedding: vec![1.0, 0.0],
                    speech_ms: ms,
                }],
            }))
        }
        fn voiceprint(&self, _pcm: &[f32]) -> impl Future<Output = Result<Vec<f32>>> + Send {
            ready(Ok(vec![1.0, 0.0]))
        }
    }

    /// Keyword heuristics, so offline runs (CI's `mirona eval --with fake`, the demo) have
    /// something deterministic. Not a classifier anyone should trust.
    impl Classifier for Fake {
        fn ask(
            &self,
            text: &str,
            questions: &[Question],
        ) -> impl Future<Output = Result<Vec<Reply>>> + Send {
            let has = |ks: &[&str]| ks.iter().any(|k| text.contains(k));
            let replies = questions
                .iter()
                .map(|q| match &q.kind {
                    Kind::YesNo { .. } => {
                        let yes = match q.id.as_str() {
                            "action" => has(&["I'll", "I will", "can you", "Can you", "send"]),
                            "decision" => has(&["let's go with", "we go with", "decided"]),
                            "unanswered" => has(&["?"]) && has(&["not sure", "don't know"]),
                            "smalltalk" => has(&["weekend", "vacation", "weather"]),
                            "sensitive" => has(&["password", "salary", "doctor"]),
                            "due" => has(&["by ", "Monday", "Friday", "Thursday", "tomorrow"]),
                            _ => false,
                        };
                        Reply::YesNo(if yes { 0.82 } else { 0.08 })
                    }
                    Kind::Choice(_) | Kind::Score(_) => {
                        let opts = options(q);
                        // The first option named in the text, else the first one.
                        let pick = opts.iter().position(|o| text.contains(o.as_str()));
                        let pick = pick.unwrap_or(0);
                        #[allow(clippy::cast_precision_loss)]
                        let rest = 0.3 / (opts.len().max(2) - 1) as f32;
                        let probabilities: Vec<(String, f32)> = opts
                            .iter()
                            .enumerate()
                            .map(|(i, o)| (o.clone(), if i == pick { 0.7 } else { rest }))
                            .collect();
                        if matches!(q.kind, Kind::Score(_)) {
                            #[allow(clippy::cast_precision_loss)]
                            let score = probabilities
                                .iter()
                                .enumerate()
                                .map(|(i, (_, p))| i as f32 * p)
                                .sum();
                            Reply::Score {
                                score,
                                probabilities: probabilities.iter().map(|(_, p)| *p).collect(),
                            }
                        } else {
                            Reply::Choice {
                                value: opts[pick].clone(),
                                probabilities,
                            }
                        }
                    }
                })
                .collect();
            ready(Ok(replies))
        }
    }

    impl LlmProvider for Fake {
        fn complete(
            &self,
            prompt: &Prompt<'_>,
            on_text: &mut (dyn FnMut(&str) + Send),
        ) -> impl Future<Output = Result<Completion>> + Send {
            let text = format!("Summary of {} characters.", prompt.user.len());
            on_text(&text);
            ready(Ok(Completion {
                text,
                usage: Usage::default(),
            }))
        }
    }

    impl Embedder for Fake {
        fn embed(&self, text: &str) -> impl Future<Output = Result<Vec<f32>>> + Send {
            #[allow(clippy::cast_precision_loss)]
            ready(Ok(vec![text.len() as f32]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fakes are ready immediately, so one poll is enough.
    pub(crate) fn now<T>(fut: impl Future<Output = T>) -> T {
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        match std::pin::pin!(fut).poll(&mut cx) {
            std::task::Poll::Ready(v) => v,
            std::task::Poll::Pending => panic!("fake was not ready"),
        }
    }

    #[test]
    fn fakes_answer() {
        let f = fake::Fake("fake");
        let one_second = vec![0.0; 16_000];
        let d = now(f.diarize(&one_second)).unwrap();
        assert_eq!(d.turns[0].end_ms, 1000);
        assert_eq!(d.speakers[0].speech_ms, 1000);
        assert_eq!(
            now(f.voiceprint(&one_second)).unwrap(),
            d.speakers[0].embedding
        );
        assert_eq!(
            now(f.transcribe(&one_second)).unwrap(),
            "Sample transcript line."
        );
        assert_eq!(now(f.embed("abc")).unwrap(), vec![3.0]);
        let mut streamed = String::new();
        let prompt = Prompt {
            system: "",
            user: "hi",
            json: false,
        };
        let done = now(f.complete(&prompt, &mut |t| streamed.push_str(t))).unwrap();
        assert!(done.text.contains('2'));
        assert_eq!(streamed, done.text);
    }

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

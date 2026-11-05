//! Jev (`TypeSafe`'s hosted "System One" model) as the default `Classifier` (ADR 0001, NGR-155).
//!
//! One `POST /v1/systemone` per segment carries every yes/no question. Jev is a cloud provider,
//! so privacy mode refuses it through `check_privacy`; the local fallback is `ollama::Ollama`.
//!
//! Each user brings their own key: Settings → API keys stores it in the OS keychain under the
//! account `jev` (`keychain::api_key("jev")`), never in `config.toml`.

use crate::questions::{SEGMENT_QUESTIONS, answer};
use crate::{Classifier, Error, Locality, Provider, Result};
use mirona_core::{Answer, Segment};
use serde_json::{Value, json};

pub const URL: &str = "https://api.typesafe.ai/v1/systemone";
/// Pinned: `jev-latest` moves, and the thresholds were tuned on this version.
pub const MODEL: &str = "jev-1.14.0";

pub struct Jev {
    key: String,
    model: String,
    url: String,
    http: ureq::Agent,
}

impl Jev {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            key: api_key.into(),
            model: MODEL.into(),
            url: URL.into(),
            http: ureq::Agent::new_with_defaults(),
        }
    }
}

/// The request body for one segment.
pub fn request(model: &str, text: &str) -> Value {
    let questions: serde_json::Map<String, Value> = SEGMENT_QUESTIONS
        .iter()
        .map(|q| {
            let spec = json!({
                "type": "noul",
                "instructions": q.instructions,
                "criteria": { "true": q.yes, "false": q.no },
            });
            (q.id.to_string(), spec)
        })
        .collect();
    json!({ "model": model, "state": text, "questions": questions })
}

/// Turn a Jev response into one `Answer` per question, in `SEGMENT_QUESTIONS` order.
pub fn parse(resp: &Value) -> Result<Vec<Answer>> {
    SEGMENT_QUESTIONS
        .iter()
        .map(|q| {
            #[allow(clippy::cast_possible_truncation)]
            resp["answers"][q.id]["noul"]
                .as_f64()
                .map(|p| answer(q, p as f32))
                .ok_or_else(|| Error::Provider(format!("jev: no answer for {}", q.id)))
        })
        .collect()
}

impl Provider for Jev {
    fn id(&self) -> &'static str {
        "jev"
    }
    fn locality(&self) -> Locality {
        Locality::Cloud
    }
}

impl Classifier for Jev {
    fn classify(&self, segment: &Segment) -> impl Future<Output = Result<Vec<Answer>>> + Send {
        let body = request(&self.model, &segment.text);
        // ponytail: blocking HTTP inside the future; run classifiers on a worker thread
        // (NGR-155), or switch to an async client if they ever share the UI executor.
        async move {
            let resp: Value = self
                .http
                .post(&self.url)
                .header("Authorization", &format!("Bearer {}", self.key))
                .send_json(&body)
                .and_then(|mut r| r.body_mut().read_json())
                .map_err(|e| Error::Provider(format!("jev: {e}")))?;
            parse(&resp)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../tests/fixtures/jev-response.json")).unwrap()
    }

    #[test]
    fn request_asks_every_question_once() {
        let body = request(MODEL, "hello");
        assert_eq!(body["model"], MODEL);
        assert_eq!(body["state"], "hello");
        let qs = body["questions"].as_object().unwrap();
        assert_eq!(qs.len(), SEGMENT_QUESTIONS.len());
        assert_eq!(qs["action"]["type"], "noul");
        assert!(qs["sensitive"]["criteria"]["true"].is_string());
    }

    #[test]
    fn parses_a_recorded_response() {
        // Recorded from jev-1.14.0: "I'll send the revised SOW… then we go with fixed-price."
        let answers = parse(&fixture()).unwrap();
        let get = |id: &str| answers.iter().find(|a| a.id == id).unwrap();
        assert!(get("action").answer && (get("action").probability - 0.96).abs() < 1e-6);
        assert!(get("decision").answer);
        assert!(!get("smalltalk").answer && !get("sensitive").answer);
        assert_eq!(answers.len(), 5);
    }

    #[test]
    fn missing_answer_is_an_error() {
        let mut resp = fixture();
        resp["answers"].as_object_mut().unwrap().remove("sensitive");
        assert!(matches!(parse(&resp), Err(Error::Provider(_))));
    }

    #[test]
    fn is_refused_in_privacy_mode() {
        let jev = Jev::new("k");
        let opt = mirona_core::ProviderOption {
            id: jev.id().into(),
            name: "Jev".into(),
            locality: jev.locality(),
        };
        assert!(crate::check_privacy(&opt, true).is_err());
    }

    /// Hits the real API. `TYPESAFE_API_KEY=… cargo test -p mirona-providers --features jev -- --ignored`
    #[test]
    #[ignore = "needs TYPESAFE_API_KEY and network"]
    fn live_classifies_an_action_item() {
        let key = std::env::var("TYPESAFE_API_KEY").expect("TYPESAFE_API_KEY");
        let seg = Segment {
            id: "s".into(),
            speaker_id: "p".into(),
            start_ms: 0,
            text: "Marco: Can you send Karen the revised SOW by Thursday? Keiko: Sure, I'll do it."
                .into(),
            redacted: false,
            tags: vec![],
            answers: vec![],
            owners: vec![],
        };
        let answers = crate::tests::now(Jev::new(key).classify(&seg)).unwrap();
        let action = answers.iter().find(|a| a.id == "action").unwrap();
        assert!(action.answer && action.probability >= 0.6, "{action:?}");
        let small = answers.iter().find(|a| a.id == "smalltalk").unwrap();
        assert!(!small.answer, "{small:?}");
    }
}

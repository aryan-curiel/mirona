//! Local LLM `Classifier` through Ollama: the privacy-mode fallback for Jev (ADR 0001).
//!
//! Same questions as Jev, answered in one call as JSON constrained by a schema. An LLM has no
//! calibrated probability, so P(yes) comes from the logprobs of each answer's first token.

use crate::questions::{SEGMENT_QUESTIONS, answer};
use crate::{Classifier, Error, Locality, Provider, Result};
use mirona_core::{Answer, Segment};
use serde_json::{Value, json};

pub const URL: &str = "http://localhost:11434/api/chat";
/// Best local model in the NGR-130 eval (0.87 mean accuracy vs 0.81 for llama3.2:3b).
pub const MODEL: &str = "qwen3:4b";

pub struct Ollama {
    model: String,
    url: String,
    http: ureq::Agent,
}

impl Ollama {
    pub fn new(model: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            url: url.into(),
            http: ureq::Agent::new_with_defaults(),
        }
    }
}

impl Default for Ollama {
    fn default() -> Self {
        Self::new(MODEL, URL)
    }
}

pub fn request(model: &str, text: &str) -> Value {
    let ids: Vec<&str> = SEGMENT_QUESTIONS.iter().map(|q| q.id).collect();
    let props: serde_json::Map<String, Value> = ids
        .iter()
        .map(|id| {
            (
                id.to_string(),
                json!({ "type": "string", "enum": ["yes", "no"] }),
            )
        })
        .collect();
    let defs: Vec<String> = SEGMENT_QUESTIONS
        .iter()
        .map(|q| format!("- {}: yes/no. YES if: {} NO if: {}", q.id, q.yes, q.no))
        .collect();
    json!({
        "model": model,
        "stream": false,
        "think": false,
        "format": { "type": "object", "required": ids, "additionalProperties": false, "properties": props },
        "logprobs": true,
        "top_logprobs": 10,
        "options": { "temperature": 0, "seed": 0 },
        "messages": [
            { "role": "system", "content": format!("You classify meeting transcript chunks. Answer every field.\n{}", defs.join("\n")) },
            { "role": "user", "content": text },
        ],
    })
}

pub fn parse(resp: &Value) -> Result<Vec<Answer>> {
    let content = resp["message"]["content"].as_str().unwrap_or_default();
    let values: Value = serde_json::from_str(content)
        .map_err(|e| Error::Provider(format!("ollama: bad JSON answer: {e}")))?;
    let tokens = resp["logprobs"].as_array().map_or(&[][..], Vec::as_slice);
    SEGMENT_QUESTIONS
        .iter()
        .map(|q| {
            let yes = match values[q.id].as_str() {
                Some("yes") => true,
                Some("no") => false,
                _ => return Err(Error::Provider(format!("ollama: no answer for {}", q.id))),
            };
            // ponytail: without logprobs (older Ollama) the answer counts as fully confident.
            let p_chosen = value_alternatives(tokens, q.id).map_or(1.0, |alts| p_yes_no(alts, yes));
            Ok(answer(q, if yes { p_chosen } else { 1.0 - p_chosen }))
        })
        .collect()
}

/// The `top_logprobs` of the first token of `"key": "<value>"` in the generated JSON.
fn value_alternatives<'a>(tokens: &'a [Value], key: &str) -> Option<&'a [Value]> {
    let needle = format!("\"{key}\"");
    let mut text = String::new();
    for (i, t) in tokens.iter().enumerate() {
        text.push_str(t["token"].as_str().unwrap_or_default());
        // Opening quote of this key's value: `"key"` then `:` then `"`, spaces allowed.
        let Some(rest) = text.trim_end().strip_suffix('"') else {
            continue;
        };
        let Some(rest) = rest.trim_end().strip_suffix(':') else {
            continue;
        };
        if rest.trim_end().ends_with(&needle) {
            return tokens.get(i + 1)?["top_logprobs"]
                .as_array()
                .map(Vec::as_slice);
        }
    }
    None
}

/// P(chosen) renormalised over alternatives that start "yes" or "no".
fn p_yes_no(alts: &[Value], chosen_yes: bool) -> f32 {
    let (mut yes, mut no) = (0.0_f64, 0.0_f64);
    for a in alts {
        let tok = a["token"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .trim_matches('"');
        let p = a["logprob"].as_f64().unwrap_or(f64::NEG_INFINITY).exp();
        if !tok.is_empty() && "yes".starts_with(tok) {
            yes += p;
        } else if !tok.is_empty() && "no".starts_with(tok) {
            no += p;
        }
    }
    let total = yes + no;
    #[allow(clippy::cast_possible_truncation)]
    if total > 0.0 {
        (if chosen_yes { yes } else { no } / total) as f32
    } else {
        1.0
    }
}

impl Provider for Ollama {
    fn id(&self) -> &'static str {
        "local-llm"
    }
    fn locality(&self) -> Locality {
        Locality::Local
    }
}

impl Classifier for Ollama {
    fn classify(&self, segment: &Segment) -> impl Future<Output = Result<Vec<Answer>>> + Send {
        let body = request(&self.model, &segment.text);
        // ponytail: blocking HTTP inside the future, same as Jev (worker thread in NGR-155).
        async move {
            let resp: Value = self
                .http
                .post(&self.url)
                .send_json(&body)
                .and_then(|mut r| r.body_mut().read_json())
                .map_err(|e| Error::Provider(format!("ollama ({}): {e}", self.model)))?;
            parse(&resp)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../tests/fixtures/ollama-response.json")).unwrap()
    }

    #[test]
    fn request_constrains_every_answer_to_yes_or_no() {
        let body = request(MODEL, "hi");
        let props = body["format"]["properties"].as_object().unwrap();
        assert_eq!(props.len(), SEGMENT_QUESTIONS.len());
        assert_eq!(props["sensitive"]["enum"], json!(["yes", "no"]));
        assert_eq!(body["logprobs"], true);
        assert_eq!(body["messages"][1]["content"], "hi");
    }

    #[test]
    fn parses_a_recorded_response_with_logprobs() {
        // Recorded from qwen3:4b: "I'll send the revised SOW… then we go with fixed-price."
        let answers = parse(&fixture()).unwrap();
        let get = |id: &str| answers.iter().find(|a| a.id == id).unwrap();
        assert!(get("action").answer && get("decision").answer);
        assert!(!get("smalltalk").answer && !get("sensitive").answer);
        for a in &answers {
            assert!((0.0..=1.0).contains(&a.probability), "{a:?}");
            // The probability comes from logprobs and agrees with the answer.
            assert_eq!(a.answer, a.probability >= 0.5, "{a:?}");
        }
    }

    #[test]
    fn logprob_mass_is_renormalised_over_yes_and_no() {
        let alts = json!([
            { "token": "yes", "logprob": (0.6_f64).ln() },
            { "token": "no", "logprob": (0.2_f64).ln() },
            { "token": "maybe", "logprob": (0.2_f64).ln() },
        ]);
        let p = p_yes_no(alts.as_array().unwrap(), true);
        assert!((p - 0.75).abs() < 1e-6, "{p}");
    }

    #[test]
    fn bad_content_is_an_error() {
        let mut resp = fixture();
        resp["message"]["content"] = json!("not json");
        assert!(matches!(parse(&resp), Err(Error::Provider(_))));
    }

    /// `ollama serve` + `ollama pull qwen3:4b`, then
    /// `cargo test -p mirona-providers --features ollama -- --ignored`
    #[test]
    #[ignore = "needs a running Ollama with qwen3:4b"]
    fn live_classifies_sensitive_content() {
        let seg = Segment {
            id: "s".into(),
            speaker_id: "p".into(),
            start_ms: 0,
            text: "Nadia: The admin password for the staging VPN is Tulip-4471, don't share it."
                .into(),
            redacted: false,
            tags: vec![],
            answers: vec![],
            owners: vec![],
        };
        let answers = crate::tests::now(Ollama::default().classify(&seg)).unwrap();
        let sensitive = answers.iter().find(|a| a.id == "sensitive").unwrap();
        assert!(sensitive.answer, "{sensitive:?}");
    }
}

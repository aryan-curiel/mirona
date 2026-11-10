//! Jev (`TypeSafe`'s hosted "System One" model) as the default `Classifier` (ADR 0001, NGR-155).
//!
//! One `POST /v1/systemone` carries every typed question: yes/no as `noul`, `choice` and
//! `score`, evaluated in parallel against one `state`. Jev is a cloud provider, so privacy mode
//! refuses it through `check_privacy`; the local fallback is `ollama::Ollama`.
//!
//! Each user brings their own key: Settings → API keys stores it in the OS keychain under the
//! account `jev` (`keychain::api_key("jev")`), never in `config.toml`.

use crate::questions::{Kind, Question, Reply, options};
use crate::{Classifier, Error, Locality, Provider, Result};
use serde_json::{Value, json};
use std::time::Duration;

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
        Self::with_url(api_key, URL)
    }

    /// Another endpoint (a proxy, or a test server).
    pub fn with_url(api_key: impl Into<String>, url: &str) -> Self {
        let http = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            key: api_key.into(),
            model: MODEL.into(),
            url: url.into(),
            http,
        }
    }
}

/// The request body: every question about `text` in one call.
pub fn request(model: &str, text: &str, questions: &[Question]) -> Value {
    let qs: serde_json::Map<String, Value> = questions
        .iter()
        .map(|q| {
            let spec = match &q.kind {
                Kind::YesNo { yes, no } => json!({
                    "type": "noul",
                    "instructions": q.instructions,
                    "criteria": { "true": yes, "false": no },
                }),
                Kind::Choice(opts) => {
                    let criteria: serde_json::Map<String, Value> = opts
                        .iter()
                        .map(|(o, d)| (o.clone(), Value::from(d.as_str())))
                        .collect();
                    json!({ "type": "choice", "instructions": q.instructions, "criteria": criteria })
                }
                Kind::Score(levels) => {
                    json!({ "type": "score", "instructions": q.instructions, "criteria": levels })
                }
            };
            (q.id.clone(), spec)
        })
        .collect();
    json!({ "model": model, "state": text, "questions": qs })
}

#[allow(clippy::cast_possible_truncation)]
fn f(v: &Value) -> Option<f32> {
    v.as_f64().map(|x| x as f32)
}

/// One `Reply` per question, in question order.
pub fn parse(resp: &Value, questions: &[Question]) -> Result<Vec<Reply>> {
    questions
        .iter()
        .map(|q| {
            let a = &resp["answers"][&q.id];
            let missing = || Error::Provider(format!("jev: no answer for {}", q.id));
            let probs = |keys: Vec<String>| -> Option<Vec<(String, f32)>> {
                keys.into_iter()
                    .map(|k| Some((k.clone(), f(&a["probabilities"][&k])?)))
                    .collect()
            };
            let reply = || match &q.kind {
                Kind::YesNo { .. } => f(&a["noul"]).map(Reply::YesNo),
                Kind::Choice(_) => {
                    let value = a["choice"].as_str()?.to_string();
                    Some(Reply::Choice {
                        value,
                        probabilities: probs(options(q))?,
                    })
                }
                Kind::Score(_) => Some(Reply::Score {
                    score: f(&a["score"])?,
                    probabilities: probs(options(q))?.into_iter().map(|(_, p)| p).collect(),
                }),
            };
            reply().ok_or_else(missing)
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
    fn ask(
        &self,
        text: &str,
        questions: &[Question],
    ) -> impl Future<Output = Result<Vec<Reply>>> + Send {
        let body = request(&self.model, text, questions);
        // ponytail: blocking HTTP inside the future; the engine classifies on its own worker
        // thread. Switch to an async client if classifiers ever share the UI executor.
        async move {
            let err = |e: &dyn std::fmt::Display| Error::Provider(format!("jev: {e}"));
            crate::net::allow(&self.url, self.locality())?;
            for attempt in 0..4 {
                let mut r = self
                    .http
                    .post(&self.url)
                    .header("Authorization", &format!("Bearer {}", self.key))
                    .send_json(&body)
                    .map_err(|e| err(&e))?;
                let status = r.status().as_u16();
                // Rate limited or overloaded: back off and retry (docs: 429, 529).
                if matches!(status, 429 | 529) && attempt < 3 {
                    std::thread::sleep(Duration::from_millis(500 << attempt));
                    continue;
                }
                if status != 200 {
                    let msg = r.body_mut().read_to_string().unwrap_or_default();
                    return Err(err(&format!("HTTP {status} {msg}")));
                }
                let resp: Value = r.body_mut().read_json().map_err(|e| err(&e))?;
                return parse(&resp, questions);
            }
            unreachable!("the last attempt returns")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::questions::{NO_OWNER, meeting_questions, segment_questions};

    fn fixture(name: &str) -> Value {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn speakers() -> Vec<String> {
        vec!["Marco".into(), "Keiko".into()]
    }

    #[test]
    fn request_sends_each_question_type() {
        let qs = segment_questions(&speakers());
        let body = request(MODEL, "hello", &qs);
        assert_eq!(body["model"], MODEL);
        assert_eq!(body["state"], "hello");
        let sent = body["questions"].as_object().unwrap();
        assert_eq!(sent.len(), qs.len());
        assert_eq!(sent["action"]["type"], "noul");
        assert!(sent["sensitive"]["criteria"]["true"].is_string());
        assert_eq!(sent["owner"]["type"], "choice");
        assert!(sent["owner"]["criteria"][NO_OWNER].is_string());
        let tags = [(
            "project:Northwind Bank".to_string(),
            "contact: Karen".to_string(),
        )]
        .into();
        let mq = meeting_questions(&tags, &[]);
        let body = request(MODEL, "x", &mq);
        assert_eq!(
            body["questions"]["tag:project:Northwind Bank"]["type"],
            "noul"
        );
        assert_eq!(body["questions"]["priority"]["type"], "score");
        assert_eq!(
            body["questions"]["priority"]["criteria"][5],
            "Critical: drop everything"
        );
    }

    #[test]
    fn parses_a_recorded_segment_response() {
        // Recorded from jev-1.14.0 (`live_records_fixtures`): Marco asks for the SOW by
        // Thursday, Keiko takes it, then they settle on fixed price.
        let qs = segment_questions(&speakers());
        let replies = parse(&fixture("jev-segment.json"), &qs).unwrap();
        let p = |i: usize| replies[i].p_yes().unwrap();
        assert!(p(0) >= 0.9, "action {}", p(0));
        assert!(p(3) < 0.5 && p(4) < 0.5, "not small talk or sensitive");
        assert!(p(5) >= 0.5, "due date said");
        let Reply::Choice { value, .. } = &replies[6] else {
            panic!("owner is a choice")
        };
        assert_eq!(value, "Keiko");
    }

    #[test]
    fn parses_a_recorded_meeting_response() {
        // Recorded before tags (it still carries a `client` answer, ignored here); tag answers
        // are `noul`, parsed like the segment ones above. Re-record with `live_records_fixtures`.
        let qs = meeting_questions(&std::collections::BTreeMap::new(), &[]);
        let replies = parse(&fixture("jev-meeting.json"), &qs).unwrap();
        let Reply::Score {
            score,
            probabilities,
        } = &replies[1]
        else {
            panic!("priority is a score")
        };
        assert_eq!(probabilities.len(), 6);
        assert!((0.0..=5.0).contains(score));
        assert!(
            replies[3].p_yes().unwrap() >= 0.5,
            "client call wants a follow-up"
        );
    }

    #[test]
    fn missing_answer_is_an_error() {
        let qs = segment_questions(&speakers());
        let mut resp = fixture("jev-segment.json");
        resp["answers"].as_object_mut().unwrap().remove("owner");
        assert!(matches!(parse(&resp, &qs), Err(Error::Provider(_))));
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

    pub(crate) const SEGMENT: &str = "Marco: Can you send Karen the revised SOW by Thursday?\nPriya: Sure, I'll do it.\nMarco: Great, then we go with fixed-price.";
    pub(crate) const MEETING: &str = "Karen: Thanks for joining. Our compliance audit is in two weeks and the export still drops rows.\nMarco: We found the bug, the fix ships Friday. I'll send you a written plan today.\nKaren: Please do, my director is not happy about the delay. Also, can you add the audit log to the export?\nPriya: Yes, we'll scope that and get back to you by Monday.";

    /// Hits the real API and (re)records the fixtures.
    /// `TYPESAFE_API_KEY=… cargo test -p mirona-providers --features jev -- --ignored`
    #[test]
    #[ignore = "needs TYPESAFE_API_KEY and network"]
    fn live_records_fixtures() {
        let jev = Jev::new(std::env::var("TYPESAFE_API_KEY").expect("TYPESAFE_API_KEY"));
        for (name, text, qs) in [
            ("jev-segment.json", SEGMENT, segment_questions(&speakers())),
            (
                "jev-meeting.json",
                MEETING,
                meeting_questions(&crate::questions::tests::tags(), &[]),
            ),
        ] {
            let body = request(MODEL, text, &qs);
            let resp: Value = jev
                .http
                .post(URL)
                .header("Authorization", &format!("Bearer {}", jev.key))
                .send_json(&body)
                .unwrap()
                .body_mut()
                .read_json()
                .unwrap();
            assert!(parse(&resp, &qs).is_ok(), "{resp}");
            let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
            std::fs::write(path, serde_json::to_string_pretty(&resp).unwrap()).unwrap();
        }
        // And through the trait, as the engine calls it.
        let qs = segment_questions(&speakers());
        let replies = crate::tests::now(jev.ask(SEGMENT, &qs)).unwrap();
        assert!(replies[0].p_yes().unwrap() >= 0.6, "{replies:?}");
    }
}

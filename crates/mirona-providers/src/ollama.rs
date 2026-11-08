//! Local LLM `Classifier` through Ollama: the privacy-mode fallback for Jev (ADR 0001).
//!
//! The same typed questions as Jev, answered in one call as JSON constrained by a schema: each
//! answer is a string enum (yes/no, the choice's options, or the score's level numbers). An LLM
//! has no calibrated probability, so each answer's distribution comes from the logprobs of its
//! first token, renormalised over the allowed values.

use crate::questions::{Kind, Question, Reply, options};
use crate::{Classifier, Error, Locality, Provider, Result};
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

/// One line per question in the system prompt (the NGR-130 wording for yes/no and choices).
fn definition(q: &Question) -> String {
    match &q.kind {
        Kind::YesNo { yes, no } => format!("- {}: yes/no. YES if: {yes} NO if: {no}", q.id),
        Kind::Choice(opts) => {
            let o: Vec<String> = opts.iter().map(|(k, d)| format!("{k} = {d}")).collect();
            format!("- {}: {} One of: {}", q.id, q.instructions, o.join("; "))
        }
        Kind::Score(levels) => {
            let o: Vec<String> = levels
                .iter()
                .enumerate()
                .map(|(i, l)| format!("{i} = {l}"))
                .collect();
            format!("- {}: {} One of: {}", q.id, q.instructions, o.join("; "))
        }
    }
}

pub fn request(model: &str, text: &str, questions: &[Question]) -> Value {
    let ids: Vec<&str> = questions.iter().map(|q| q.id.as_str()).collect();
    let props: serde_json::Map<String, Value> = questions
        .iter()
        .map(|q| {
            (
                q.id.clone(),
                json!({ "type": "string", "enum": options(q) }),
            )
        })
        .collect();
    let defs: Vec<String> = questions.iter().map(definition).collect();
    json!({
        "model": model,
        "stream": false,
        "think": false,
        "format": { "type": "object", "required": ids, "additionalProperties": false, "properties": props },
        "logprobs": true,
        "top_logprobs": 10,
        // A whole meeting goes through here too: Ollama's default context (4k) would cut it.
        "options": { "temperature": 0, "seed": 0, "num_ctx": LLM_NUM_CTX },
        "messages": [
            { "role": "system", "content": format!("You classify meeting transcript chunks. Answer every field.\n{}", defs.join("\n")) },
            { "role": "user", "content": text },
        ],
    })
}

pub fn parse(resp: &Value, questions: &[Question]) -> Result<Vec<Reply>> {
    let content = resp["message"]["content"].as_str().unwrap_or_default();
    let values: Value = serde_json::from_str(content)
        .map_err(|e| Error::Provider(format!("ollama: bad JSON answer: {e}")))?;
    let tokens = resp["logprobs"].as_array().map_or(&[][..], Vec::as_slice);
    questions
        .iter()
        .map(|q| {
            let opts = options(q);
            let chosen = values[&q.id]
                .as_str()
                .and_then(|v| opts.iter().position(|o| o == v))
                .ok_or_else(|| Error::Provider(format!("ollama: no answer for {}", q.id)))?;
            let mass = value_alternatives(tokens, &q.id)
                .map(|alts| distribution(alts, &opts))
                .filter(|m| m[chosen] > 0.0)
                // ponytail: without logprobs (older Ollama), or when the chosen value's first
                // token is missing from the top 10, the answer counts as fully confident.
                .unwrap_or_else(|| {
                    let mut m = vec![0.0; opts.len()];
                    m[chosen] = 1.0;
                    m
                });
            Ok(match &q.kind {
                Kind::YesNo { .. } => Reply::YesNo(mass[0]),
                Kind::Choice(_) => Reply::Choice {
                    value: opts[chosen].clone(),
                    probabilities: opts.into_iter().zip(mass).collect(),
                },
                Kind::Score(_) => Reply::Score {
                    #[allow(clippy::cast_precision_loss)]
                    score: mass.iter().enumerate().map(|(i, p)| i as f32 * p).sum(),
                    probabilities: mass,
                },
            })
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

/// Each option's probability: the top alternatives' mass, given to the options the token is a
/// prefix of (split evenly when it starts several), renormalised to sum to 1.
fn distribution(alts: &[Value], options: &[String]) -> Vec<f32> {
    let mut mass = vec![0.0_f64; options.len()];
    for a in alts {
        let tok = a["token"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .trim_matches('"');
        if tok.is_empty() {
            continue;
        }
        let p = a["logprob"].as_f64().unwrap_or(f64::NEG_INFINITY).exp();
        let hits: Vec<usize> = (0..options.len())
            .filter(|&i| options[i].starts_with(tok))
            .collect();
        for &i in &hits {
            #[allow(clippy::cast_precision_loss)]
            let share = p / hits.len() as f64;
            mass[i] += share;
        }
    }
    let total: f64 = mass.iter().sum();
    #[allow(clippy::cast_possible_truncation)]
    mass.iter()
        .map(|m| if total > 0.0 { (m / total) as f32 } else { 0.0 })
        .collect()
}

impl Provider for Ollama {
    fn id(&self) -> &'static str {
        "local-llm"
    }
    /// Ollama on another machine is someone else's server.
    fn locality(&self) -> Locality {
        if crate::net::is_loopback(&self.url) {
            Locality::Local
        } else {
            Locality::Cloud
        }
    }
}

impl Classifier for Ollama {
    fn ask(
        &self,
        text: &str,
        questions: &[Question],
    ) -> impl Future<Output = Result<Vec<Reply>>> + Send {
        let body = request(&self.model, text, questions);
        // ponytail: blocking HTTP inside the future, same as Jev (the engine's worker thread).
        async move {
            crate::net::allow(&self.url, self.locality())?;
            let resp: Value = self
                .http
                .post(&self.url)
                .send_json(&body)
                .and_then(|mut r| r.body_mut().read_json())
                .map_err(|e| Error::Provider(format!("ollama ({}): {e}", self.model)))?;
            parse(&resp, questions)
        }
    }
}

// --- LlmProvider (NGR-145): summaries and notes through the same local Ollama. ---

/// Context window asked of Ollama for prose: its default (4k) would silently cut a long
/// transcript chunk. qwen3:4b handles 32k; 16k keeps memory modest.
pub const LLM_NUM_CTX: u32 = 16_384;

/// A streamed `/api/chat` request. `think: false` keeps qwen3's reasoning out of the answer.
pub fn chat_request(model: &str, prompt: &crate::Prompt<'_>) -> Value {
    let mut body = json!({
        "model": model,
        "stream": true,
        "think": false,
        "options": { "num_ctx": LLM_NUM_CTX },
        "messages": [
            { "role": "system", "content": prompt.system },
            { "role": "user", "content": prompt.user },
        ],
    });
    if prompt.json {
        body["format"] = json!("json");
    }
    body
}

/// Read Ollama's NDJSON stream: one object per line, the last has `done` and token counts.
pub fn read_chat_stream(
    reader: impl std::io::BufRead,
    on_text: &mut (dyn FnMut(&str) + Send),
) -> Result<crate::Completion> {
    let err = |e: &dyn std::fmt::Display| Error::Provider(format!("ollama: {e}"));
    let count = |v: &Value| {
        v.as_u64()
            .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX))
    };
    let mut text = String::new();
    for line in reader.lines() {
        let line = line.map_err(|e| err(&e))?;
        if line.trim().is_empty() {
            continue;
        }
        let chunk: Value = serde_json::from_str(&line).map_err(|e| err(&e))?;
        if let Some(e) = chunk.get("error") {
            return Err(err(&e));
        }
        let t = chunk["message"]["content"].as_str().unwrap_or_default();
        if !t.is_empty() {
            on_text(t);
            text.push_str(t);
        }
        if chunk["done"] == true {
            let usage = crate::Usage {
                input_tokens: count(&chunk["prompt_eval_count"]),
                output_tokens: count(&chunk["eval_count"]),
            };
            return Ok(crate::Completion { text, usage });
        }
    }
    Err(err(&"stream ended before done"))
}

impl crate::LlmProvider for Ollama {
    fn complete(
        &self,
        prompt: &crate::Prompt<'_>,
        on_text: &mut (dyn FnMut(&str) + Send),
    ) -> impl Future<Output = Result<crate::Completion>> + Send {
        let body = chat_request(&self.model, prompt);
        // ponytail: blocking HTTP inside the future, same as Jev (worker thread in the engine).
        async move {
            let err = |e: &dyn std::fmt::Display| {
                Error::Provider(format!("ollama ({}): {e}", self.model))
            };
            crate::net::allow(&self.url, self.locality())?;
            let mut resp = self
                .http
                .post(&self.url)
                .config()
                .http_status_as_error(false)
                .build()
                .send_json(&body)
                .map_err(|e| err(&e))?;
            if !resp.status().is_success() {
                let msg = resp.body_mut().read_to_string().unwrap_or_default();
                return Err(err(&format!("{} {msg}", resp.status())));
            }
            read_chat_stream(
                std::io::BufReader::new(resp.into_body().into_reader()),
                on_text,
            )
        }
    }
}

/// Embeddings for search (NGR-167), from Ollama on this machine.
pub const EMBED_URL: &str = "http://localhost:11434/api/embed";
/// Small (270 MB), good at retrieval, wants `search_query:` / `search_document:` prefixes.
pub const EMBED_MODEL: &str = "nomic-embed-text";

pub struct OllamaEmbedder {
    pub model: String,
    url: String,
    http: ureq::Agent,
}

impl Default for OllamaEmbedder {
    fn default() -> Self {
        Self::new(EMBED_MODEL, EMBED_URL)
    }
}

impl OllamaEmbedder {
    pub fn new(model: impl Into<String>, url: impl Into<String>) -> Self {
        let http = ureq::Agent::config_builder()
            .timeout_connect(Some(std::time::Duration::from_secs(2)))
            .timeout_global(Some(std::time::Duration::from_secs(120)))
            .build()
            .into();
        Self {
            model: model.into(),
            url: url.into(),
            http,
        }
    }

    /// One vector per text, in one call.
    pub fn embed_many(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        crate::net::allow(&self.url, self.locality())?;
        let err = |e: &dyn std::fmt::Display| Error::Provider(format!("ollama embed: {e}"));
        let resp: Value = self
            .http
            .post(&self.url)
            .send_json(json!({ "model": self.model, "input": texts }))
            .and_then(|mut r| r.body_mut().read_json())
            .map_err(|e| err(&e))?;
        let out: Vec<Vec<f32>> = serde_json::from_value(resp["embeddings"].clone())
            .map_err(|e| err(&format!("{e}: {resp}")))?;
        if out.len() != texts.len() {
            return Err(err(&format!(
                "{} vectors for {} texts",
                out.len(),
                texts.len()
            )));
        }
        Ok(out)
    }
}

impl Provider for OllamaEmbedder {
    fn id(&self) -> &'static str {
        "ollama"
    }
    fn locality(&self) -> Locality {
        if crate::net::is_loopback(&self.url) {
            Locality::Local
        } else {
            Locality::Cloud
        }
    }
}

impl crate::Embedder for OllamaEmbedder {
    fn embed(&self, text: &str) -> impl Future<Output = Result<Vec<f32>>> + Send {
        let r = self
            .embed_many(&[text.to_string()])
            .map(|mut v| v.remove(0));
        std::future::ready(r)
    }
}

#[cfg(test)]
mod llm_tests {
    use super::*;
    use crate::{LlmProvider, Prompt, Usage};

    #[test]
    fn chat_request_streams_and_asks_for_json() {
        let p = Prompt {
            system: "sys",
            user: "hi",
            json: true,
        };
        let body = chat_request(MODEL, &p);
        assert_eq!(body["stream"], true);
        assert_eq!(body["format"], "json");
        assert_eq!(body["options"]["num_ctx"], LLM_NUM_CTX);
        let p = Prompt { json: false, ..p };
        assert!(chat_request(MODEL, &p).get("format").is_none());
    }

    #[test]
    fn reads_a_recorded_stream() {
        // Recorded from qwen3:4b (Ollama 0.40), 77 content chunks then `done`.
        let nd = include_str!("../tests/fixtures/ollama-chat-stream.ndjson");
        let mut chunks = 0;
        let done = read_chat_stream(nd.as_bytes(), &mut |_| chunks += 1).unwrap();
        assert!(chunks > 10, "{chunks}");
        assert!(
            done.text.contains("\"overview\": \"Nadia has requested"),
            "{}",
            done.text
        );
        assert_eq!(
            done.usage,
            Usage {
                input_tokens: 50,
                output_tokens: 78
            }
        );
    }

    #[test]
    fn errors_and_cut_streams_are_errors() {
        let e = read_chat_stream(&b"{\"error\":\"model 'x' not found\"}\n"[..], &mut |_| {});
        assert!(e.unwrap_err().to_string().contains("not found"));
        let cut = "{\"message\":{\"content\":\"hi\"},\"done\":false}\n";
        assert!(read_chat_stream(cut.as_bytes(), &mut |_| {}).is_err());
    }

    /// `cargo test -p mirona-providers --features ollama -- --ignored live_streams`
    #[test]
    #[ignore = "needs a running Ollama with qwen3:4b"]
    fn live_streams_a_completion() {
        let p = Prompt {
            system: "Answer with a JSON object {\"word\": string}.",
            user: "Say hello.",
            json: true,
        };
        let mut n = 0;
        let done = crate::tests::now(Ollama::default().complete(&p, &mut |_| n += 1)).unwrap();
        assert!(
            n > 1 && serde_json::from_str::<Value>(&done.text).is_ok(),
            "{done:?}"
        );
        assert!(
            done.usage.input_tokens > 0 && done.usage.output_tokens > 0,
            "{done:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::questions::{Question, meeting_questions, segment_questions};

    fn fixture(name: &str) -> Value {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn speakers() -> Vec<String> {
        vec!["Marco".into(), "Keiko".into()]
    }

    const SEGMENT: &str = "Marco: Can you send Karen the revised SOW by Thursday?\nPriya: Sure, I'll do it.\nMarco: Great, then we go with fixed-price.";
    const MEETING: &str = "Karen: Thanks for joining. Our compliance audit is in two weeks and the export still drops rows.\nMarco: We found the bug, the fix ships Friday. I'll send you a written plan today.\nKaren: Please do, my director is not happy about the delay. Also, can you add the audit log to the export?\nPriya: Yes, we'll scope that and get back to you by Monday.";

    fn questions() -> Vec<Question> {
        meeting_questions(
            &crate::questions::tests::tags(),
            &std::collections::BTreeMap::new(),
        )
    }

    #[test]
    fn request_constrains_every_answer_to_its_options() {
        let qs = segment_questions(&speakers());
        let body = request(MODEL, "hi", &qs);
        let props = body["format"]["properties"].as_object().unwrap();
        assert_eq!(props.len(), qs.len());
        assert_eq!(props["sensitive"]["enum"], json!(["yes", "no"]));
        assert_eq!(
            props["owner"]["enum"],
            json!(["Marco", "Keiko", "nobody here"])
        );
        assert_eq!(body["logprobs"], true);
        assert_eq!(body["messages"][1]["content"], "hi");
        let body = request(MODEL, "hi", &questions());
        let props = &body["format"]["properties"];
        assert_eq!(props["tone"]["enum"], json!(["0", "1", "2", "3", "4"]));
    }

    #[test]
    fn parses_a_recorded_segment_response_with_logprobs() {
        // Recorded from qwen3:4b (`live_records_fixtures`).
        let qs = segment_questions(&speakers());
        let replies = parse(&fixture("ollama-segment.json"), &qs).unwrap();
        assert!(replies[0].p_yes().unwrap() >= 0.5, "action: {replies:?}");
        assert!(replies[4].p_yes().unwrap() < 0.5, "not sensitive");
        for r in &replies {
            assert!((0.0..=1.0).contains(&r.confidence()), "{r:?}");
        }
        let Reply::Choice {
            value,
            probabilities,
        } = &replies[6]
        else {
            panic!("owner is a choice")
        };
        assert_eq!(value, "Keiko");
        let total: f32 = probabilities.iter().map(|(_, p)| p).sum();
        assert!((total - 1.0).abs() < 1e-4, "{probabilities:?}");
    }

    #[test]
    fn parses_a_recorded_meeting_response_with_scores() {
        let qs = questions();
        let replies = parse(&fixture("ollama-meeting.json"), &qs).unwrap();
        let p = |id: &str| {
            replies[qs.iter().position(|q| q.id == id).unwrap()]
                .p_yes()
                .unwrap()
        };
        assert!(p("tag:project:Northwind Bank") >= 0.5, "{replies:?}");
        assert!(p("tag:project:Helio Health") < 0.5, "{replies:?}");
        let Reply::Score {
            score,
            probabilities,
        } = &replies[1]
        else {
            panic!("priority is a score")
        };
        assert_eq!(probabilities.len(), 6);
        assert!((0.0..=5.0).contains(score), "{score}");
    }

    #[test]
    fn logprob_mass_is_renormalised_and_shared_prefixes_split() {
        let alts = json!([
            { "token": "yes", "logprob": (0.6_f64).ln() },
            { "token": "no", "logprob": (0.2_f64).ln() },
            { "token": "maybe", "logprob": (0.2_f64).ln() },
        ]);
        let opts = ["yes".to_string(), "no".to_string()];
        let m = distribution(alts.as_array().unwrap(), &opts);
        assert!((m[0] - 0.75).abs() < 1e-6, "{m:?}");
        // "Ma" starts both Marco and Mateo: its mass is split between them.
        let alts = json!([
            { "token": "Ma", "logprob": (0.8_f64).ln() },
            { "token": "Kei", "logprob": (0.2_f64).ln() },
        ]);
        let opts = [
            "Marco".to_string(),
            "Mateo".to_string(),
            "Keiko".to_string(),
        ];
        let m = distribution(alts.as_array().unwrap(), &opts);
        assert!(
            (m[0] - 0.4).abs() < 1e-6 && (m[2] - 0.2).abs() < 1e-6,
            "{m:?}"
        );
    }

    #[test]
    fn bad_content_is_an_error() {
        let qs = segment_questions(&speakers());
        let mut resp = fixture("ollama-segment.json");
        resp["message"]["content"] = json!("not json");
        assert!(matches!(parse(&resp, &qs), Err(Error::Provider(_))));
    }

    /// `ollama serve` + `ollama pull qwen3:4b`, then
    /// `cargo test -p mirona-providers --features ollama -- --ignored`. Rewrites the fixtures.
    #[test]
    #[ignore = "needs a running Ollama with qwen3:4b"]
    fn live_records_fixtures() {
        let o = Ollama::default();
        for (name, text, qs) in [
            (
                "ollama-segment.json",
                SEGMENT,
                segment_questions(&speakers()),
            ),
            ("ollama-meeting.json", MEETING, questions()),
        ] {
            let resp: Value = o
                .http
                .post(URL)
                .send_json(request(MODEL, text, &qs))
                .unwrap()
                .body_mut()
                .read_json()
                .unwrap();
            assert!(parse(&resp, &qs).is_ok(), "{resp}");
            let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
            std::fs::write(path, serde_json::to_string_pretty(&resp).unwrap()).unwrap();
        }
    }

    #[test]
    #[ignore = "needs a running Ollama with qwen3:4b"]
    fn live_classifies_sensitive_content() {
        let qs = segment_questions(&[]);
        let text = "Nadia: The admin password for the staging VPN is Tulip-4471, don't share it.";
        let replies = crate::tests::now(Ollama::default().ask(text, &qs)).unwrap();
        assert!(replies[4].p_yes().unwrap() >= 0.5, "{replies:?}");
    }
}

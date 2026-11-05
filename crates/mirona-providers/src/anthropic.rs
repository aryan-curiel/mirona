//! Anthropic `LlmProvider` (NGR-145): the Messages API, streamed over SSE. Always cloud.

use crate::{Completion, Error, LlmProvider, Locality, Prompt, Provider, Result, Usage};
use serde_json::{Value, json};
use std::io::BufRead;

pub const URL: &str = "https://api.anthropic.com/v1/messages";
pub const API_VERSION: &str = "2026-10-21";
/// Configurable through `Anthropic::new`.
pub const MODEL: &str = "claude-opus-5-5";

pub struct Anthropic {
    key: String,
    model: String,
    url: String,
    http: ureq::Agent,
}

impl Anthropic {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            key: api_key.into(),
            model: model.into(),
            url: URL.into(),
            http: ureq::Agent::new_with_defaults(),
        }
    }
}

/// The Messages API has no plain "JSON mode"; the prompt asks for JSON and the caller parses it
/// leniently (prefill is gone on current models).
pub fn request(model: &str, prompt: &Prompt<'_>) -> Value {
    json!({
        "model": model,
        "max_tokens": 64_000,
        "stream": true,
        "system": prompt.system,
        "messages": [{ "role": "user", "content": prompt.user }],
    })
}

/// Read a Messages SSE stream: text deltas go to `on_text`; thinking blocks are skipped.
pub fn read_stream(
    reader: impl BufRead,
    on_text: &mut (dyn FnMut(&str) + Send),
) -> Result<Completion> {
    let err = |e: &dyn std::fmt::Display| Error::Provider(format!("anthropic: {e}"));
    let mut text = String::new();
    let mut usage = Usage::default();
    for line in reader.lines() {
        let line = line.map_err(|e| err(&e))?;
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            continue;
        };
        let event: Value = serde_json::from_str(data).map_err(|e| err(&e))?;
        match event["type"].as_str().unwrap_or_default() {
            "message_start" => {
                usage.input_tokens = count(&event["message"]["usage"]["input_tokens"]);
                usage.output_tokens = count(&event["message"]["usage"]["output_tokens"]);
            }
            "content_block_delta" if event["delta"]["type"] == "text_delta" => {
                let t = event["delta"]["text"].as_str().unwrap_or_default();
                on_text(t);
                text.push_str(t);
            }
            "message_delta" => {
                // Cumulative, so the last one wins.
                usage.output_tokens = count(&event["usage"]["output_tokens"]);
                if event["delta"]["stop_reason"] == "refusal" {
                    return Err(err(&"the model declined this request"));
                }
            }
            "error" => return Err(err(&event["error"])),
            "message_stop" => break,
            _ => {}
        }
    }
    Ok(Completion { text, usage })
}

fn count(v: &Value) -> u32 {
    v.as_u64()
        .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX))
}

impl Provider for Anthropic {
    fn id(&self) -> &'static str {
        "anthropic"
    }
    fn locality(&self) -> Locality {
        Locality::Cloud
    }
}

impl LlmProvider for Anthropic {
    fn complete(
        &self,
        prompt: &Prompt<'_>,
        on_text: &mut (dyn FnMut(&str) + Send),
    ) -> impl Future<Output = Result<Completion>> + Send {
        let body = request(&self.model, prompt);
        // ponytail: blocking HTTP inside the future, same as Jev (worker thread in the engine).
        async move {
            let err = |e: &dyn std::fmt::Display| Error::Provider(format!("anthropic: {e}"));
            let mut resp = self
                .http
                .post(&self.url)
                .header("x-api-key", &self.key)
                .header("anthropic-version", API_VERSION)
                .config()
                .http_status_as_error(false)
                .build()
                .send_json(&body)
                .map_err(|e| err(&e))?;
            if !resp.status().is_success() {
                let msg = resp.body_mut().read_to_string().unwrap_or_default();
                return Err(err(&format!("{} {msg}", resp.status())));
            }
            read_stream(
                std::io::BufReader::new(resp.into_body().into_reader()),
                on_text,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_streams_with_system_and_model() {
        let p = Prompt {
            system: "sys",
            user: "hi",
            json: true,
        };
        let body = request(MODEL, &p);
        assert_eq!(body["model"], MODEL);
        assert_eq!(body["stream"], true);
        assert_eq!(body["system"], "sys");
        assert_eq!(body["messages"][0]["content"], "hi");
        assert!(body.get("temperature").is_none(), "400s on {API_VERSION}");
    }

    #[test]
    fn reads_a_streamed_message_and_skips_thinking() {
        let sse = include_str!("../tests/fixtures/anthropic-messages-stream.sse");
        let mut chunks = vec![];
        let done = read_stream(sse.as_bytes(), &mut |t| chunks.push(t.to_string())).unwrap();
        assert_eq!(done.text, r#"{"overview": "Ship on Monday."}"#);
        assert_eq!(chunks.len(), 2);
        assert_eq!(
            done.usage,
            Usage {
                input_tokens: 42,
                output_tokens: 15
            }
        );
    }

    #[test]
    fn error_event_and_refusal_are_errors() {
        let overloaded = "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n";
        let e = read_stream(overloaded.as_bytes(), &mut |_| {}).unwrap_err();
        assert!(e.to_string().contains("Overloaded"), "{e}");
        let refusal = "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"refusal\"},\"usage\":{\"output_tokens\":3}}\n";
        assert!(read_stream(refusal.as_bytes(), &mut |_| {}).is_err());
    }

    #[test]
    fn is_cloud_so_privacy_mode_refuses_it() {
        let a = Anthropic::new("k", MODEL);
        let opt = mirona_core::ProviderOption {
            id: a.id().into(),
            name: "Anthropic".into(),
            locality: a.locality(),
        };
        assert!(crate::check_privacy(&opt, true).is_err());
    }

    /// `ANTHROPIC_API_KEY=… cargo test -p mirona-providers --features anthropic -- --ignored`
    #[test]
    #[ignore = "needs ANTHROPIC_API_KEY and network"]
    fn live_streams_a_completion() {
        let key = std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY");
        let p = Prompt {
            system: "Answer with only a JSON object {\"word\": string}.",
            user: "Say hello.",
            json: true,
        };
        let mut n = 0;
        let done =
            crate::tests::now(Anthropic::new(key, MODEL).complete(&p, &mut |_| n += 1)).unwrap();
        assert!(n > 0 && done.text.contains("word"), "{done:?}");
        assert!(done.usage.input_tokens > 0 && done.usage.output_tokens > 0);
    }
}

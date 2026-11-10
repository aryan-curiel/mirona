//! OpenAI-compatible `LlmProvider` (NGR-145): `POST {base_url}/chat/completions`, streamed.
//!
//! Covers `OpenAI` itself and anything that speaks its API (LM Studio, vLLM, llama.cpp server,
//! Ollama's `/v1`…). Loopback endpoints count as local; every other host is cloud, so privacy
//! mode refuses it. The key comes from `keychain` (optional for local servers).

use crate::{Completion, Error, LlmProvider, Locality, Prompt, Provider, Result, Usage};
use serde_json::{Value, json};
use std::io::BufRead;

pub const OPENAI_URL: &str = "https://api.openai.com/v1";
pub const LM_STUDIO_URL: &str = "http://localhost:1234/v1";

pub struct OpenAi {
    base_url: String,
    model: String,
    key: Option<String>,
    http: ureq::Agent,
}

impl OpenAi {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>, key: Option<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
            key,
            http: ureq::Agent::new_with_defaults(),
        }
    }
}

pub fn request(model: &str, base_url: &str, prompt: &Prompt<'_>) -> Value {
    let mut body = json!({
        "model": model,
        "stream": true,
        "stream_options": { "include_usage": true },
        "messages": [
            { "role": "system", "content": prompt.system },
            { "role": "user", "content": prompt.user },
        ],
    });
    // ponytail: only OpenAI itself gets `json_object`; LM Studio rejects it (it wants a schema),
    // so other servers rely on the prompt asking for JSON and the caller parsing leniently.
    if prompt.json && base_url.contains("api.openai.com") {
        body["response_format"] = json!({ "type": "json_object" });
    }
    body
}

/// Read a `chat.completion.chunk` SSE stream until `[DONE]`.
pub fn read_stream(
    reader: impl BufRead,
    on_text: &mut (dyn FnMut(&str) + Send),
) -> Result<Completion> {
    let err = |e: &dyn std::fmt::Display| Error::Provider(format!("openai: {e}"));
    let mut text = String::new();
    let mut usage = Usage::default();
    for line in reader.lines() {
        let line = line.map_err(|e| err(&e))?;
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            continue;
        };
        if data == "[DONE]" {
            break;
        }
        let chunk: Value = serde_json::from_str(data).map_err(|e| err(&e))?;
        if let Some(e) = chunk.get("error") {
            return Err(err(&e));
        }
        if let Some(t) = chunk["choices"][0]["delta"]["content"].as_str()
            && !t.is_empty()
        {
            on_text(t);
            text.push_str(t);
        }
        if chunk["usage"].is_object() {
            usage = Usage {
                input_tokens: count(&chunk["usage"]["prompt_tokens"]),
                output_tokens: count(&chunk["usage"]["completion_tokens"]),
            };
        }
    }
    Ok(Completion { text, usage })
}

fn count(v: &Value) -> u32 {
    v.as_u64()
        .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX))
}

impl Provider for OpenAi {
    fn id(&self) -> &'static str {
        "openai"
    }
    fn locality(&self) -> Locality {
        if crate::net::is_local(&self.base_url) {
            Locality::Local
        } else {
            Locality::Cloud
        }
    }
}

impl LlmProvider for OpenAi {
    fn complete(
        &self,
        prompt: &Prompt<'_>,
        on_text: &mut (dyn FnMut(&str) + Send),
    ) -> impl Future<Output = Result<Completion>> + Send {
        let body = request(&self.model, &self.base_url, prompt);
        // ponytail: blocking HTTP inside the future, same as Jev; the engine runs LLM calls on a
        // worker thread. Switch to an async client if they ever share the UI executor.
        async move {
            let err = |e: &dyn std::fmt::Display| Error::Provider(format!("openai: {e}"));
            crate::net::allow(&self.base_url, self.locality())?;
            let mut req = self
                .http
                .post(format!("{}/chat/completions", self.base_url));
            if let Some(key) = &self.key {
                req = req.header("Authorization", &format!("Bearer {key}"));
            }
            let mut resp = req
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
    fn only_this_machine_and_the_local_network_are_local() {
        let local = |u| OpenAi::new(u, "m", None).locality() == Locality::Local;
        assert!(local(LM_STUDIO_URL));
        assert!(local("http://127.0.0.1:8080/v1"));
        assert!(local("http://[::1]:11434/v1/"));
        assert!(!local(OPENAI_URL));
        assert!(local("http://192.168.1.20:1234/v1"));
        assert!(!local("http://203.0.113.7:1234/v1"));
        assert!(!local("https://localhost.evil.com/v1"));
        assert!(!local("https://user@api.groq.com/openai/v1"));
    }

    #[test]
    fn request_streams_with_usage_and_json_only_for_openai() {
        let p = Prompt {
            system: "sys",
            user: "hi",
            json: true,
        };
        let body = request("gpt-4o-mini", OPENAI_URL, &p);
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["messages"][0]["content"], "sys");
        assert_eq!(body["response_format"]["type"], "json_object");
        assert!(
            request("m", LM_STUDIO_URL, &p)
                .get("response_format")
                .is_none()
        );
    }

    #[test]
    fn reads_a_streamed_completion() {
        let sse = include_str!("../tests/fixtures/openai-chat-stream.sse");
        let mut chunks = vec![];
        let done = read_stream(sse.as_bytes(), &mut |t| chunks.push(t.to_string())).unwrap();
        assert_eq!(done.text, r#"{"overview": "Ship on Monday."}"#);
        assert_eq!(chunks.len(), 2);
        assert_eq!(
            done.usage,
            Usage {
                input_tokens: 42,
                output_tokens: 9
            }
        );
    }

    #[test]
    fn error_chunk_is_an_error() {
        let sse = "data: {\"error\":{\"message\":\"model not found\"}}\n\n";
        let e = read_stream(sse.as_bytes(), &mut |_| {}).unwrap_err();
        assert!(e.to_string().contains("model not found"), "{e}");
    }

    /// `OPENAI_API_KEY=… cargo test -p mirona-providers --features openai -- --ignored`
    /// (or `MIRONA_OPENAI_URL=http://localhost:1234/v1 MIRONA_OPENAI_MODEL=…` for LM Studio).
    #[test]
    #[ignore = "needs OPENAI_API_KEY (or a local OpenAI-compatible server) and network"]
    fn live_streams_a_completion() {
        let url = std::env::var("MIRONA_OPENAI_URL").unwrap_or(OPENAI_URL.into());
        let model = std::env::var("MIRONA_OPENAI_MODEL").unwrap_or("gpt-4o-mini".into());
        let llm = OpenAi::new(url, model, std::env::var("OPENAI_API_KEY").ok());
        let _g = crate::net::tests::SWITCH.lock().unwrap();
        crate::net::set_privacy(false); // a cloud call, on purpose
        let p = Prompt {
            system: "Answer with a JSON object {\"word\": string}.",
            user: "Say hello.",
            json: true,
        };
        let mut n = 0;
        let done = crate::tests::now(llm.complete(&p, &mut |_| n += 1)).unwrap();
        assert!(n > 0 && done.text.contains("word"), "{done:?}");
        assert!(done.usage.output_tokens > 0, "{done:?}");
    }
}

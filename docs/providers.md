# Adding a provider

Every pipeline stage is a trait in `crates/mirona-providers/src/lib.rs`. A provider implements
one of them, says where it runs, and is picked by an id in `config.toml`:

```toml
privacy = true

[providers]
stt = "parakeet"            # parakeet | whisper (needs the `whisper` feature)
diarization = "speakrs"     # speakrs | speakrs-cpu | off
classification = "jev"      # jev | local-llm | off
llm = "ollama"              # ollama | openai | anthropic
embeddings = "ollama"       # ollama | off
```

| Stage | Trait | Built-in providers | Where the id is matched |
| --- | --- | --- | --- |
| Speech to text | `SttProvider` | `parakeet.rs`, `whisper.rs` | `Stt::load` in `crates/mirona-capture/src/lib.rs` |
| Speakers | `Diarizer` | `speakrs.rs` | `crates/mirona-engine/src/diarize.rs` |
| Classification | `Classifier` | `jev.rs`, `ollama.rs` (`local-llm`) | `crates/mirona-engine/src/classify.rs` |
| Summaries and notes | `LlmProvider` | `ollama.rs`, `openai.rs`, `anthropic.rs` | `crates/mirona-engine/src/llm.rs` |
| Search embeddings | `Embedder` | `ollama.rs` (`OllamaEmbedder`) | `crates/mirona-engine/src/search.rs` |

The `fake` module in `lib.rs` has a deterministic implementation of each trait for tests.

## 1. Implement the trait

Add `crates/mirona-providers/src/<name>.rs` behind its own feature, so builds that don't use it
don't pull in its dependencies:

```toml
# crates/mirona-providers/Cargo.toml
[features]
mistral = ["dep:ureq", "dep:serde_json"]
```

```rust
// crates/mirona-providers/src/lib.rs
#[cfg(feature = "mistral")]
pub mod mistral;
```

```rust
// crates/mirona-providers/src/mistral.rs
use crate::{Completion, Error, LlmProvider, Prompt, Provider, Result, net};
use mirona_core::Locality;

pub struct Mistral { /* client, model, key */ }

impl Provider for Mistral {
    fn id(&self) -> &str { "mistral" }
    fn locality(&self) -> Locality { Locality::Cloud }
}

impl LlmProvider for Mistral {
    fn complete(
        &self,
        prompt: &Prompt<'_>,
        on_text: &mut (dyn FnMut(&str) + Send),
    ) -> impl Future<Output = Result<Completion>> + Send {
        async move {
            net::allow(URL, self.locality())?; // before every request
            // ...stream the answer, call on_text for each chunk, return text + token usage
        }
    }
}
```

The rules every provider follows:

- **Say where it runs.** `locality()` is `Cloud` for anything that sends data off the machine.
  Privacy mode refuses cloud providers in `check_privacy`.
- **Every request goes through the network gate.** Call `net::allow(url, locality)` before each
  HTTP request. In privacy mode it lets only this machine and the local network through
  (`net::is_local`: loopback, private and Tailscale IPs, `.local`), so a "local" provider
  pointed at a public address is refused too. Base `locality()` on `net::is_local`. Don't open connections any other way.
- **No API keys in config.** Keys live in the OS keychain (`keychain::api_key("<id>")`), entered
  in Settings › AI models › API keys. Add the id to `KEY_PROVIDERS` in
  `app/src-tauri/src/lib.rs` and an `ApiKey` field in `app/src/views/Settings.tsx`.
- **Sensitive lines are already filtered.** The engine and `mirona-summary` drop or redact lines
  where `Segment::is_sensitive` is true before summaries, notes, follow-up drafts and
  embeddings, and keep them from cloud classifiers for the meeting questions. A classifier still
  reads each line once to tag it. Don't re-add raw transcript text from elsewhere.
- **Errors are `Error::Provider(message)`.** The message is shown to the user, so make it say
  what to do ("model not found: run `ollama pull …`").
- Providers block inside their futures. The engine's `block_on` is the whole executor, so don't
  depend on tokio.

## 2. Test it

- Unit tests next to the code, with recorded responses where possible (see `jev.rs` and
  `ollama.rs` fixtures).
- Tests that need a model, an API key or a running server are `#[ignore]`d, so CI stays offline:
  `cargo test -p mirona-providers --features mistral -- --ignored`.
- For a classifier, `mirona eval --with <id>` scores it on the public set (`eval/chunks.jsonl`)
  and on your own corrections. Add the id to `run_eval` in `crates/mirona-cli/src/main.rs`
  first.

## 3. Wire it in

1. **Engine:** match the new id where the stage is picked (table above). For an LLM, add a
   variant to `Llm` in `crates/mirona-engine/src/llm.rs` and a branch in `Llm::from_config`.
   It goes through `check_privacy` there.
2. **Feature:** enable it in `crates/mirona-engine/Cargo.toml` (the `mirona-providers`
   features line).
3. **Settings:** add the option to the stage's list in `app/src/views/Settings.tsx`, and the id
   to the allowed values in `Settings` (`app/src-tauri/src/lib.rs`).
4. **Licenses:** a new model or library goes in `NOTICE` and `docs/licenses.md`.

A downloadable model goes in the pinned manifest in `crates/mirona-engine/src/models.rs`, with
its URL, size and SHA-256, so the app can fetch and verify it.

## Before opening a PR

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p mirona-providers --all-targets --all-features
cargo test --workspace
```

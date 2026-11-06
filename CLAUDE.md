# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Mirona is a local-first meeting memory app (transcription, speakers, typed classification via hosted Jev (local LLM fallback in privacy mode; Kev is dropped), summaries, MCP server). Tauri 2 + Rust workspace + React. **Status: M3 (privacy and search)**: records mic + system audio on macOS, transcribes live (Parakeet), diarizes and names voices (speakrs), tags lines and meetings with typed questions (Jev, or qwen3:4b via Ollama in privacy mode), summarizes with a local or cloud LLM, and serves meetings over MCP (stdio and localhost HTTP) and a CLI. Privacy mode is enforced at one network gate; meetings can be encrypted at rest; search is hybrid (FTS5 + local embeddings); retention rules, a consent reminder and call detection are in. Gates 1–3 need real use / a manual audit (`docs/testing-checklist.md`, `docs/privacy-audit.md`).

## Commands

```bash
# App (from app/; needs Rust stable, Node 22+, pnpm)
pnpm install
pnpm tauri dev          # runs Vite on :1420 + the Tauri shell
pnpm build              # tsc + vite build (CI runs this)
pnpm lint               # Biome lint over src/**/*.ts(x) (CSS excluded: Tailwind v4 syntax)

# Rust (from repo root) — exactly what CI runs on macOS/Windows/Linux
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings   # pedantic on; CI fails on any warning
cargo test --workspace
cargo test -p mirona-store privacy_and_corrections   # single test
python3 design-kit/tokens/check-contrast.py          # WCAG 4.5:1 check, light + dark tokens

# Models (Parakeet, Silero VAD, Whisper): the app downloads them (Settings › Models, onboarding);
# from a source checkout, into the app's models dir:
scripts/fetch-models.sh "$HOME/Library/Application Support/com.nagare.mirona/models"

# Backend without the UI (CLI over the same engine; MIRONA_APP_DIR overrides the app dir)
cargo run -p mirona-cli -- list            # record | stop | list | show | search | actions | import | reindex | mcp | eval
cargo run -p mirona-cli -- eval --with fake   # classifier eval offline (CI); --with jev / local-llm for real
cargo run -p mirona-cli -- mcp             # MCP server over stdio (same as the mirona-mcp binary)

# Real-model / live tests are #[ignore]d; models from MIRONA_MODELS_DIR or spikes/parakeet/models
cargo test -p mirona-engine -- --ignored   # import a clip → transcript → qwen3:4b summary (needs Ollama)
```

## Architecture

Workspace crates (`crates/*`) plus the Tauri crate `app/src-tauri` (`mirona-app`):

- **mirona-core** — domain types and `PipelineEvent`. Every type derives `ts_rs::TS` with `#[ts(export)]`. `cargo test -p mirona-core` regenerates `app/src/bindings/*.ts` (output dir set via `TS_RS_EXPORT_DIR` in `.cargo/config.toml`). After changing a core type, rerun it and commit the regenerated bindings; never hand-edit `app/src/bindings/`.
- **mirona-providers** — one trait per pipeline stage (`SttProvider`, `Diarizer`, `Classifier`, `LlmProvider`, `Embedder`) plus deterministic `fake` impls. `check_privacy` is the single privacy gate: every provider selection must route through it so privacy mode refuses cloud providers in one place. `config::Config` (`config.toml` in the OS app-config dir: `privacy` + `[providers] stage = "id"`) is applied via `Store::apply_config`, which goes through the same gate; the app saves it on every privacy/provider change.
  Real providers sit behind features (`jev`, `ollama`, `parakeet`, `speakrs`; off by default): `jev::Jev` and `ollama::Ollama` (`Classifier::ask`: typed yes/no, choice and score questions from `questions.rs`, one call per text), `parakeet::Parakeet` (SttProvider), `speakrs::Speakrs` (Diarizer + voiceprint matching rule). Model/live tests are `#[ignore]`d; run them with:
  `MIRONA_PARAKEET_DIR=… MIRONA_SPEAKRS_DIR=… TYPESAFE_API_KEY=… cargo test -p mirona-providers --all-features -- --include-ignored` (models come from `spikes/*/fetch.sh`; Ollama must be serving qwen3:4b). `mirona eval` reruns the 50-chunk eval (plus the user's corrections from `<meetings>/.mirona/labels.jsonl`) through the Rust classifiers; `live_records_fixtures` re-records the Jev/Ollama fixtures.
  `net.rs` is the network gate: every HTTP request calls `net::allow(url, locality)` first; in privacy mode only loopback hosts pass. The switch is process-wide, starts on, and is set from config by `Engine::new`/`set_config` and `mirona_mcp::load`. Any new network code must go through it. `ollama::OllamaEmbedder` (`nomic-embed-text`) embeds for search.
  `config::Config` also holds `meetings_dir`, `models_dir`, the summary `template`, `llm` (model, base URL), `mcp_enabled`, the record `shortcut`, per-question `thresholds` and the `clients` list (name → description); `[providers] classification` is `jev` (default), `local-llm` or `off`. LLM providers (`ollama`, `openai`, `anthropic` features) stream and report usage; API keys live in the OS keychain (`keychain` feature; accounts `openai`, `anthropic`, `jev`), entered by each user in Settings → API keys, never in config.
- **mirona-store** — files are the source of truth: one folder per meeting (`meeting.md` = YAML frontmatter + `## Notes` body, `transcript.jsonl` append-only, `audio.wav` while recording → `audio.caf` (Opus) after), plus `<root>/.mirona/index.sqlite` (FTS5) rebuilt by `reindex()`. `Store::open` rebuilds the index; readers in other processes (MCP, CLI) use `Store::open_snapshot` so they never rewrite the app's index. `crypt.rs`: optional XChaCha20-Poly1305 at rest (transcript lines, audio, `embeddings.json`, `voices.json`, `labels.jsonl`; key from the engine's keychain lookup, `open_with`/`open_snapshot_with`); while encrypted the index is in memory only. `search.rs`: hybrid search (FTS5 + per-line vectors in `<meeting>/embeddings.json`, RRF, filters). `lifecycle.rs`: encryption on/off, `plain_audio` (decrypted copy for playback/diarization), retention building blocks, delete everywhere. `Segment::is_sensitive` (core) is the one rule for what never reaches a model or an agent. Errors are `Result<T, String>`. `sample.rs` seeds demo meetings for tests and "Add demo meetings".
- **mirona-capture** — macOS capture (cpal: mic + system audio via a Core Audio process tap), resampling, Silero VAD chunking (≤5 s target, cut at pauses), per-source live STT on a worker, offline `transcribe_file`. Streams are rebuilt when the default device or its sample rate changes (AirPods flip rate when their mic opens). Non-macOS builds compile; live capture returns an error there. `examples/record_probe.rs` checks real hardware from inside an app bundle. Recordings are compressed to Opus in CAF (`compress.rs`, libopus built by cmake through `opusic-sys`); `decode` reads them back on every OS.
- **mirona-summary** — editable Markdown templates (`templates/*.md`, written to the app dir), `summarize` and `enhance_notes` over any `LlmProvider`, citing transcript line numbers (0-based) mapped back to timestamps.
- **mirona-engine** — the pipeline the app and CLI share: `Engine::record` → `Session::stop` (finish → summary + enhanced notes), `import`, `summarize`. `classify.rs` tags segments live on a worker (`Classified` events) and, after the call, answers the meeting questions (type → template, client, priority, tone, follow-up), checks the summary covers every action item and drafts the follow-up email; below-threshold answers become "Needs review" (`Store::review`). `search.rs` embeds lines after each call (and backfills on app start) and is the one search the app, CLI and MCP run; `models.rs` is the model downloader (pinned URLs + SHA-256 manifest, resume from `.part`, the `CoreML` speaker set only on macOS with ≥ 16 GB; requests go through `net::allow_model`, the one privacy-mode exception); `keep.rs` holds the store key (keychain account `store-key`) and retention (`Engine::retention`, logged to `<meetings>/.mirona/retention.log`). `app_dir()` (Tauri's app-config dir, or `MIRONA_APP_DIR`) holds `config.toml`, models and templates. Providers block inside their futures; `block_on` is the whole executor.
- **mirona-mcp / mirona-cli** — `rmcp` server on stdio or localhost HTTP with per-client tokens (`web.rs`; `search_meetings`, `get_meeting`, `get_transcript`, `list_action_items`, `ask_meeting` routed by the classifier in `ask.rs`, write tools behind `mcp_write` in `write.rs`, activity log in `activity.rs`) rereading the store per call; the CLI wraps the engine (`record` writes `recording.lock`, `stop` drops `recording.stop`). Claude Code skill in `skills/mirona/`; Homebrew formula (HEAD) in `packaging/homebrew/`; docs in `docs/cli-and-mcp.md`.
- **app/src-tauri/src/lib.rs** — commands over the engine; recording from the sidebar, tray and global shortcut (`shortcut` in config, default ⌘⌥⇧R, re-registered live from Settings); long work on worker threads, progress on the `"pipeline"` and `"recording"` event channels. Closing the window hides it to the menu bar. Settings open in their own window (`open_settings`, label `settings`, same bundle: `main.tsx` picks `SettingsWindow` by window label; ⌘, and the tray open it); commands that change settings call `changed()`, which emits `settings` so every window refetches. The onboarding is a third window (`show_onboarding`, label `onboarding`, `OnboardingWindow` in `views/Setup.tsx`): on first run the main window starts hidden (`visible: false` in the tauri confs) until it closes, which writes `onboarded` in the app dir; from Settings it opens as a child window centred on the caller. `Info.plist` carries the mic and system-audio usage strings.
- **app/src** — React 19 + Tailwind v4 + zustand. `lib/api.ts` holds typed `invoke` wrappers (one per Tauri command; args are camelCase on the TS side). `store.ts` is the single zustand store and applies pipeline events. Adding a command means: Rust fn → register in `generate_handler!` → wrapper in `api.ts` → action in `store.ts`.

### UI / design

- Views follow `design-kit/` screen by screen (`prototype/index.html` opens directly in a browser; `screenshots/` for reference). Tokens live in `app/src/tokens.css` (`.mi-*` surface classes) mirroring `design-kit/tokens/`.
- `app/src/tokens.css` must stay byte-identical to `design-kit/tokens/tokens.css` (CI diffs them). Dark theme tokens live under `[data-theme="dark"]` (defined, not shipped).
- Platform skins key off `html[data-platform]` in `app/src/styles.css`; per-OS window config is in `app/src-tauri/tauri.{macos,windows}.conf.json`. `Cmd/Ctrl+Shift+L` toggles the other platform's skin in dev.
- macOS icon is built from `design-kit/brand/png/icon-macos-1024.png` (has Apple's margin); Windows uses full-bleed `icon-1024.png`.
- Accessibility rules from the design kit: text contrast ≥ 4.5:1 on glass (`--mi-muted` is the lightest allowed text), focus ring on every control, 44 px main actions, respect `prefers-reduced-transparency`.

## Spikes and decisions

- `spikes/*` are standalone cargo/uv projects (own `[workspace]`), not workspace members; their `models/` and `audio/` dirs are gitignored. Write-ups live in `docs/spikes/`, decisions in `docs/adr/`, licenses in `docs/licenses.md`.

## Conventions

- Rust edition 2024, `max_width = 100`; 2-space indent elsewhere.
- Deliberate shortcuts are marked with `// ponytail:` comments naming the ceiling and the Linear issue (NGR-xxx) that replaces them. Module docs reference their Linear issue too.

# Milestone 1 build plan (MVP: daily driver)

> Historical: the plan as written for milestone 1. Current behaviour is in `docs/user-guide.md` and `docs/architecture.md`.

The working contract between the people and agents building M1 in parallel. Linear: milestone "1 · MVP: daily driver". Model choices come from ADR 0001.

## Phase 1: three independent work packages (in parallel, separate files)

Each package owns only the files listed. Don't edit another package's files. If you need a change there, say so in your report. Every package must keep these passing:

- `cargo fmt --all --check`
- `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets` (default features, all three CI OSes)
- `cargo test --workspace`

Also run clippy and the tests with your own features enabled.

### WP1: Store (NGR-140), owner of `crates/mirona-store/**`

Real storage. Each meeting is a folder under a root dir (configurable; the default is decided by the app):

```
<root>/<YYYY-MM-DD-HHMM>-<slug>/
  meeting.md          YAML frontmatter (all structured data) + Markdown body (## Notes is the user's, ## Summary is rendered)
  transcript.jsonl    one Segment per line, append-only while recording
  audio.<ext>         the recording (written by the capture side; the store only records its file name)
<root>/.mirona/index.sqlite   index (meetings, segments FTS5, action items, speakers); rebuildable from the files
```

- `Store::open(root) -> Result<Store>` loads or creates. `Store::reindex()` rebuilds SQLite from the files. Search uses FTS5.
- Keep the method names the app, MCP and CLI already use: `list_meetings`, `meeting`, `search`, `toggle_action_item`, `enroll_speaker`, `correct`, `confirm_client`, `set_notes`, `start_meeting`, `push_segment`, `finish_meeting`, and `action_items()`/`people()` accessors. Writes go through to disk immediately.
- New writes the engine needs:
  - `set_summary(meeting_id, Summary, Vec<ActionItem>)`
  - `set_enhanced_notes(meeting_id, Vec<EnhancedNote>, by)`
  - `set_audio(meeting_id, file_name)`
  - `meeting_dir(meeting_id) -> PathBuf`
  - `edit_segment_text(meeting_id, segment_id, text)` (fixing a word, NGR-143)
  - `set_title`
- **Out of the store:** privacy, stages and MCP status leave `Store` and move to `mirona_providers::config::Config` / app state. Keep `sample` data only for tests and as an optional "seed demo meeting".
- Files must open cleanly in Obsidian, and meetings must survive reinstall.
- **Tests:** round-trip through the files, append-only transcript, reindex rebuilds identical results, FTS search, corrupt-file handling. Use temp dirs only.
- You may add fields to `mirona-core` types with `#[serde(default)]`, then regenerate the bindings (`cargo test -p mirona-core`). The other packages don't edit `mirona-core`.

### WP2: Capture, VAD and STT (NGR-137, 138, 139), owner of `crates/mirona-capture/**` and `crates/mirona-providers/src/whisper.rs` (+ the `whisper` feature deps)

- **macOS capture:** mic (cpal) and system audio (ScreenCaptureKit or Core Audio process taps; pick what works). Resample to 16 kHz mono f32 per source. Handle device changes without crashing. Request the permissions. Code that only works on macOS goes behind `cfg(target_os = "macos")`; the crate must still **build and test on Linux and Windows** (CI).
- **VAD:** Silero via sherpa-onnx. Chunks of at most 5 s, cut at pauses of at least 0.25 s (ADR 0001), tunable. Unit tests use recorded fixtures.
- **Live transcription:** each source is chunked and transcribed separately, so the speaker is known from the source: mic = "me", system = "them". Diarization comes in M2. Output is a stream of events over a channel or callback: levels, and segments (source, start_ms, end_ms, text). The mixed recording goes to a file the caller names.
- **Offline import:** transcribe an audio file (wav/mp3/m4a) through the same VAD and STT path.
- **Whisper:** whisper-rs `SttProvider` as the alternative, chosen by config.
- **Models:** found under a `models_dir` (Parakeet, Silero VAD, Whisper). Write `scripts/fetch-models.sh` (you own it) to download them into a dir.
- **Testing:** pipeline tests on files (no device needed). Real-device capture tests are `#[ignore]` and need to be run locally.

### WP3: LLM providers, summaries, enhanced notes (NGR-145, 146, 147), owner of `crates/mirona-summary/**`, `crates/mirona-providers/src/{openai,anthropic,keychain}.rs`, and the `LlmProvider` impl in `ollama.rs`

- **`LlmProvider`** (trait in `lib.rs`; don't change it): implement it for Ollama (local, default), any OpenAI-compatible endpoint, and Anthropic. Each must stream and report token use, and declare cloud/local correctly so privacy mode can refuse cloud ones. Keys go in the OS keychain (`keychain.rs`, `keyring` crate), never in config.
- **`mirona-summary`:**
  - Templates for standup, client call, 1:1 and planning, stored as editable Markdown files. The defaults are embedded and written out to a templates dir on first use.
  - `summarize(&Meeting, template, &impl LlmProvider) -> (Summary, Vec<ActionItem>)` with overview, decisions, action items (owner and due when said) and open questions, each pointing back to a timestamp.
  - `enhance_notes(&Meeting, &impl LlmProvider) -> Vec<EnhancedNote>`: keep the user's wording, expand from the transcript, link each line to a timestamp, never overwrite the raw notes.
- **Testing:** fake-LLM unit tests for prompt building and parsing (including bad JSON), recorded-response fixtures for each provider, and live `#[ignore]` tests (Ollama is running locally with qwen3:4b; no OpenAI or Anthropic keys exist, so those use fixtures plus ignored live tests).

## Phase 2: wiring (after phase 1)

- **Engine (owner: lead):** a `mirona-engine` crate that runs record → live segments → store → on stop → summary → store, with events. Used by the app and the CLI.
- **WP4 (NGR-148, 149):** MCP v1 on the official `rmcp` SDK over the real store, the CLI (`record`, `stop`, `list`, `show`, `search`, `import`, `reindex`, `mcp`), and the Claude Code skill.
- **WP5 (NGR-141, 142, 143, 144, 150):** the app on the real store and engine: tray and global shortcut, permissions onboarding, transcript features, notes editor, settings.

## Phase 3: integration (lead)

End-to-end test on real audio, CI, docs, Linear. NGR-151 (two weeks of real use) is for the user.

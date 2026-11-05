# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Mirona is a local-first meeting memory app (transcription, speakers, typed classification via hosted Jev (local LLM fallback in privacy mode; Kev is dropped), summaries, MCP server). Tauri 2 + Rust workspace + React. **Status: prototype** — the UI is wired end to end against a mocked backend; no audio is captured and no models run.

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

# Backend without the UI
cargo run -p mirona-cli -- actions        # meetings | show <id> | actions | search <q>
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | cargo run -q -p mirona-mcp
```

## Architecture

Workspace crates (`crates/*`) plus the Tauri crate `app/src-tauri` (`mirona-app`):

- **mirona-core** — domain types and `PipelineEvent`. Every type derives `ts_rs::TS` with `#[ts(export)]`. `cargo test -p mirona-core` regenerates `app/src/bindings/*.ts` (output dir set via `TS_RS_EXPORT_DIR` in `.cargo/config.toml`). After changing a core type, rerun it and commit the regenerated bindings; never hand-edit `app/src/bindings/`.
- **mirona-providers** — one trait per pipeline stage (`SttProvider`, `Diarizer`, `Classifier`, `LlmProvider`, `Embedder`) plus deterministic `fake` impls. `check_privacy` is the single privacy gate: every provider selection must route through it so privacy mode refuses cloud providers in one place. `config::Config` (`config.toml` in the OS app-config dir: `privacy` + `[providers] stage = "id"`) is applied via `Store::apply_config`, which goes through the same gate; the app saves it on every privacy/provider change.
  Real providers sit behind features (`jev`, `ollama`, `parakeet`, `speakrs`; off by default): `jev::Jev` and `ollama::Ollama` (Classifier, sharing `questions::SEGMENT_QUESTIONS`), `parakeet::Parakeet` (SttProvider), `speakrs::Speakrs` (Diarizer + voiceprint matching rule). Model/live tests are `#[ignore]`d; run them with:
  `MIRONA_PARAKEET_DIR=… MIRONA_SPEAKRS_DIR=… TYPESAFE_API_KEY=… cargo test -p mirona-providers --all-features -- --include-ignored` (models come from `spikes/*/fetch.sh`; Ollama must be serving qwen3:4b). `tests/classifier_eval.rs` reruns the 50-chunk eval through the Rust classifiers.
- **mirona-store** — `Store`, in memory, seeded by `Store::sample()` from `sample.rs` (which also scripts the fake live recording via `live_script()`). Errors are `Result<T, String>`. Target is Markdown + SQLite (NGR-140).
- **mirona-cli / mirona-mcp** — each builds its own `Store::sample()`; they do not share state with the app. MCP is hand-rolled JSON-RPC over stdin lines.
- **app/src-tauri/src/lib.rs** — `#[tauri::command]`s over a `Mutex<Store>`, and a fake capture thread that emits `PipelineEvent`s on the `"pipeline"` channel (captured → transcribed → classified; on stop: diarized → summarized).
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

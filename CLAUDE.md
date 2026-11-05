# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Mirona is a local-first meeting memory app (transcription, speakers, typed classification via hosted Jev (local LLM fallback in privacy mode; Kev is dropped), summaries, MCP server). Tauri 2 + Rust workspace + React. **Status: prototype** — the UI is wired end to end against a mocked backend; no audio is captured and no models run.

## Commands

```bash
# App (from app/; needs Rust stable, Node 22+, pnpm)
pnpm install
pnpm tauri dev          # runs Vite on :1420 + the Tauri shell
pnpm build              # tsc + vite build (CI runs this)

# Rust (from repo root) — exactly what CI runs on macOS/Windows/Linux
cargo fmt --all --check
cargo clippy --workspace --all-targets   # clippy::pedantic is on workspace-wide
cargo test --workspace
cargo test -p mirona-store privacy_and_corrections   # single test

# Backend without the UI
cargo run -p mirona-cli -- actions        # meetings | show <id> | actions | search <q>
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | cargo run -q -p mirona-mcp
```

## Architecture

Workspace crates (`crates/*`) plus the Tauri crate `app/src-tauri` (`mirona-app`):

- **mirona-core** — domain types and `PipelineEvent`. Every type derives `ts_rs::TS` with `#[ts(export)]`. `cargo test -p mirona-core` regenerates `app/src/bindings/*.ts` (output dir set via `TS_RS_EXPORT_DIR` in `.cargo/config.toml`). After changing a core type, rerun it and commit the regenerated bindings; never hand-edit `app/src/bindings/`.
- **mirona-providers** — one trait per pipeline stage (`SttProvider`, `Diarizer`, `Classifier`, `LlmProvider`, `Embedder`) plus deterministic `fake` impls. `check_privacy` is the single privacy gate: every provider selection must route through it so privacy mode refuses cloud providers in one place.
- **mirona-store** — `Store`, in memory, seeded by `Store::sample()` from `sample.rs` (which also scripts the fake live recording via `live_script()`). Errors are `Result<T, String>`. Target is Markdown + SQLite (NGR-140).
- **mirona-cli / mirona-mcp** — each builds its own `Store::sample()`; they do not share state with the app. MCP is hand-rolled JSON-RPC over stdin lines.
- **app/src-tauri/src/lib.rs** — `#[tauri::command]`s over a `Mutex<Store>`, and a fake capture thread that emits `PipelineEvent`s on the `"pipeline"` channel (captured → transcribed → classified; on stop: diarized → summarized).
- **app/src** — React 19 + Tailwind v4 + zustand. `lib/api.ts` holds typed `invoke` wrappers (one per Tauri command; args are camelCase on the TS side). `store.ts` is the single zustand store and applies pipeline events. Adding a command means: Rust fn → register in `generate_handler!` → wrapper in `api.ts` → action in `store.ts`.

### UI / design

- Views follow `design-kit/` screen by screen (`prototype/index.html` opens directly in a browser; `screenshots/` for reference). Tokens live in `app/src/tokens.css` (`.mi-*` surface classes) mirroring `design-kit/tokens/`.
- Platform skins key off `html[data-platform]` in `app/src/styles.css`; per-OS window config is in `app/src-tauri/tauri.{macos,windows}.conf.json`. `Cmd/Ctrl+Shift+L` toggles the other platform's skin in dev.
- macOS icon is built from `design-kit/brand/png/icon-macos-1024.png` (has Apple's margin); Windows uses full-bleed `icon-1024.png`.
- Accessibility rules from the design kit: text contrast ≥ 4.5:1 on glass (`--mi-muted` is the lightest allowed text), focus ring on every control, 44 px main actions, respect `prefers-reduced-transparency`.

## Conventions

- Rust edition 2024, `max_width = 100`; 2-space indent elsewhere.
- Deliberate shortcuts are marked with `// ponytail:` comments naming the ceiling and the Linear issue (NGR-xxx) that replaces them. Module docs reference their Linear issue too.

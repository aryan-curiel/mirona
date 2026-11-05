# Mirona

Local-first, agent-native meeting memory: transcription, speakers, typed classification (hosted Jev; local LLM fallback in privacy mode), summaries and an MCP server. Tauri 2 + Rust + React.

> **Status: prototype.** The UI is wired end to end against a mocked backend with sample data. No audio is captured and no models run yet.

## Run the demo

```bash
cd app
pnpm install
pnpm tauri dev
```

Requires Rust (stable) and Node 22+ with pnpm.

### Demo script

1. **Meetings**: open *Claims Desk weekly sync*. Click any segment's **Inspect** to see the classifier's typed questions, probabilities and the 0.60 threshold. Click **Correct** to fix an answer (it retags the segment and counts as a labelled example).
2. Click the dashed **Inés? … Confirm** chip to enroll Speaker 3 as Inés everywhere.
3. In the inspector, confirm the client and open **Draft follow-up email**.
4. **Notes** and **Summary** tabs: timestamps jump back to the transcript.
5. **Start recording**: segments stream in live with speakers and tags. **Stop and process** writes the summary and adds the action item.
6. **Privacy mode** off unlocks cloud providers in **Models and MCP**; turning it back on resets every stage to local.
7. `Cmd/Ctrl+Shift+L` previews the other platform's skin (macOS ⇄ Windows).

## Platform look

| | macOS | Windows 11 |
| --- | --- | --- |
| Window chrome | Native traffic lights over the sidebar (`titleBarStyle: Overlay`) | Custom caption bar with Fluent min/max/close |
| Material | Sidebar vibrancy | Mica |
| Details | Rounded nav highlight, macOS switch | Accent pill on active nav, Fluent toggle, tighter corners, Segoe UI Variable |

The macOS icon (`icon.icns`) is built from `design-kit/brand/png/icon-macos-1024.png`, which has Apple's ~100 px margin and shadow; Windows icons use the full-bleed `icon-1024.png`.

Per-platform window settings live in `app/src-tauri/tauri.{macos,windows}.conf.json`; CSS skins key off `html[data-platform]` in `app/src/styles.css`.

## Layout

```
crates/
  mirona-core/       Domain types + pipeline events (NGR-132). `cargo test -p mirona-core` regenerates app/src/bindings/*.ts
  mirona-providers/  One trait per stage + fakes; privacy gate refuses cloud providers in one place (NGR-133)
  mirona-store/      Meeting store. Prototype: in memory with sample data. Target: Markdown + SQLite (NGR-140)
  mirona-mcp/        MCP server over stdio: search_meetings, get_meeting, get_transcript, list_action_items (NGR-148)
  mirona-cli/        `mirona meetings | show <id> | actions | search <q>` (NGR-149)
app/
  src-tauri/         Tauri shell: typed commands over the store + fake capture pipeline emitting events
  src/               React + Tailwind v4 + zustand. Views follow design-kit/ screen by screen
design-kit/          Prototype, tokens, brand and screenshots
```

Try the backend without the UI:

```bash
cargo run -p mirona-cli -- actions
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | cargo run -q -p mirona-mcp
```

## What's mocked

Audio capture, STT, diarization, Jev classification and LLM summaries are scripted (`crates/mirona-store/src/sample.rs`). The store is in memory and resets on restart. The real implementations follow the Linear milestones: Foundations → MVP → Who and what → Privacy and search → Open-source release.

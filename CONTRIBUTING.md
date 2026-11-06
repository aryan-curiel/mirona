# Contributing

Thanks for helping. Bug reports, fixes, providers, docs and design feedback are all welcome.

## Before you start

- **Bugs and small fixes:** open a PR, or an issue first if you're unsure.
- **Bigger changes** (a new feature, a new dependency, anything that touches privacy or the file
  format): open an issue to agree on the approach first.
- **Security or privacy problems:** don't open a public issue. See [SECURITY.md](SECURITY.md).
- Issues labelled [`good first issue`](https://github.com/aryan-curiel/mirona/labels/good%20first%20issue)
  are a good place to start.

## Set up

You need:

- macOS 14.6+ on Apple silicon to record (other OSes build and run the tests, but can't capture
  audio yet; on Linux, install Tauri's deps first: `libwebkit2gtk-4.1-dev libappindicator3-dev
  librsvg2-dev patchelf`)
- Rust stable, Node 22+ with pnpm, cmake
- [Ollama](https://ollama.com) with `qwen3:4b` for summaries, and `nomic-embed-text` for
  meaning-based search (optional)

```bash
git clone https://github.com/aryan-curiel/mirona.git
cd mirona/app
pnpm install
pnpm tauri dev        # Vite on :1420 plus the app
```

On first run the onboarding offers to download the speech and speaker models. From a checkout you can also fetch them with
`scripts/fetch-models.sh "$HOME/Library/Application Support/com.nagare.mirona/models"`.

To try the backend without the UI: `cargo run -p mirona-cli -- list` (also `record`, `show`,
`search`, `import`, `mcp`; see [docs/cli-and-mcp.md](docs/cli-and-mcp.md)). Set `MIRONA_APP_DIR`
to keep test data apart from your own.

## Checks

CI runs these on Linux for every PR. Run them before pushing:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings   # clippy::pedantic is on
cargo test --workspace
cd app && pnpm lint && pnpm build
```

If you changed provider code, also run
`cargo clippy -p mirona-providers --all-targets --all-features` and
`cargo test -p mirona-providers --all-features` (macOS). Tests that need a model, an API key or
Ollama are `#[ignore]`d; run them with `-- --ignored` when you touch that code.

## How the code fits together

Start with [docs/architecture.md](docs/architecture.md). A few rules that keep things working:

- **Privacy.** Every HTTP request calls `net::allow` first, and every provider choice goes
  through `check_privacy`. New network code that skips them won't be merged.
- **Sensitive lines.** Anything sent to a model or an agent must skip segments where
  `Segment::is_sensitive` is true.
- **Files are the source of truth.** The SQLite index is rebuilt from the meeting folders, so
  don't store anything only in the index.
- **Types shared with the UI** live in `mirona-core` and derive `ts_rs::TS`. After changing one,
  run `cargo test -p mirona-core` and commit the regenerated `app/src/bindings/`. Don't edit
  those files by hand.
- **A new app command:** a Rust function in `app/src-tauri/src/lib.rs` → register it in
  `generate_handler!` → a wrapper in `app/src/lib/api.ts` → an action in `app/src/store.ts`.
- **UI** follows `design-kit/`. `app/src/tokens.css` must stay identical to
  `design-kit/tokens/tokens.css`, and text contrast must be at least 4.5:1
  (`python3 design-kit/tokens/check-contrast.py`).
- **New providers:** [docs/providers.md](docs/providers.md).

## Style

- Rust edition 2024, `rustfmt` with `max_width = 100`. Errors that reach the user say what to do.
- Small, focused PRs. Prefer the standard library and dependencies already in the workspace; a
  new dependency needs a reason in the PR.
- Comments explain why, not what. A deliberate shortcut gets a `// ponytail:` comment naming its
  limit and what would replace it.
- User-facing text is short and plain.

## Pull requests

- Describe what changed and why, and how you tested it (the PR template has the list).
- Include screenshots for UI changes.
- By contributing, you agree your work is licensed under [Apache-2.0](LICENSE).

## Conduct

Be kind and assume good intent. See [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

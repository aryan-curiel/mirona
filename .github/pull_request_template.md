## What and why

<!-- What changed, and the problem it solves. Link the issue: Fixes #123 -->

## How I tested it

- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
- [ ] `cd app && pnpm lint && pnpm build`
- [ ] Tried it in the app (screenshots below for UI changes)

## Privacy

- [ ] No new network calls, or they go through `net::allow` and respect privacy mode
- [ ] Nothing new sends sensitive lines (`Segment::is_sensitive`) to a model or an agent

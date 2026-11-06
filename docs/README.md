# Mirona docs

The same pages are published as a website with the Rust API reference
(`scripts/build-docs.sh` builds it locally into `target/book`).

**Using Mirona**

- [User guide](user-guide.md): recording, speakers, summaries, search, privacy, encryption
- [CLI and MCP](cli-and-mcp.md): the `mirona` command, MCP tools, connecting Claude and others
- [Privacy](../PRIVACY.md): what leaves your Mac, and when

**Working on Mirona**

- [Contributing](../CONTRIBUTING.md): set up, checks, pull requests
- [Architecture](architecture.md): crates, the recording pipeline, files, privacy checks
- [Adding a provider](providers.md): speech, speakers, classifier, LLM, embeddings
- [Testing checklist](testing-checklist.md): the manual pass before a release
- [Privacy audit](privacy-audit.md): proving privacy mode, socket by socket
- [Releasing](releasing.md): signed, notarized builds from a tag
- Rust API: `cargo doc --workspace --no-deps --open`

**Background**

- [Model and code licenses](licenses.md)
- [Decisions](adr/) and [spike write-ups](spikes/)
- Milestone plans: [1](m1-plan.md), [2](m2-plan.md) (historical)

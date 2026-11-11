# Mirona docs

The same pages are published as a website with the Rust API reference
(`scripts/build-docs.sh` builds it locally into `target/book`). It lives at
[mirona.app/docs](https://mirona.app/docs/), behind the landing page in `site/`
(`scripts/build-site.sh` puts both into `target/site`).

**Using Mirona**

- [User guide](user-guide.md): [getting started](guide/getting-started.md),
  [recording](guide/recording.md), [a meeting](guide/meetings.md), [speakers](guide/speakers.md),
  [tags](guide/tags.md), [action items and search](guide/actions-and-search.md),
  [summaries and models](guide/summaries.md), [privacy](guide/privacy.md),
  [agents](guide/agents.md), [workflows](guide/workflows.md), [settings](guide/settings.md),
  [files](guide/files.md), [FAQ](guide/faq.md)
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

# Privacy audit (Gate 3, NGR-171)

Run before every release. Gate 3 passes when, with privacy mode on, a full meeting (record or
import, summarize, search, MCP) opens **no connection that leaves this machine**, and **sensitive
content said in the call never appears in a summary or an MCP result**.

## What enforces it

- **One network gate** (`crates/mirona-providers/src/net.rs`): every HTTP request a provider
  makes calls `net::allow` first. In privacy mode only loopback hosts pass (Ollama, LM Studio on
  this machine), and cloud providers are refused even when pointed at one. The switch starts on
  in every process and is set from `config.toml` by the app, the CLI and the MCP server.
  `privacy_mode_opens_no_connection` proves it with a real socket.
- **Update check** (`app/src-tauri/src/update.rs`): off by default; runs only with Settings ›
  About › Check for updates on, or on Check now. It calls `net::allow_update` (HTTPS to
  `github.com` only) before the check and the download, and sends no meeting data.
- **No other network code**: no telemetry. The other exception is the model
  downloader (NGR-174, `crates/mirona-engine/src/models.rs`): it runs only when the user clicks
  Download, calls `net::allow_model` (HTTPS to `huggingface.co` or `github.com` only), fetches
  pinned revisions and checks each file's SHA-256. It sends no meeting data, so privacy mode lets
  it through. Run the audit with the models already in place.
- **Sensitive lines** (`Segment::is_sensitive`): tagged by the classifier, or naming a password,
  a salary, an account or card number or health details. They never reach any model (summaries,
  notes, follow-up drafts, `ask_meeting`), are never embedded, and MCP shows them as
  `[redacted]` unless the agent passes `include_sensitive` and the user allowed it
  (`mcp_sensitive`, off by default). A line the keyword check flags never goes to a cloud
  classifier. Covered by `a_password_and_a_salary_never_leak`,
  `open_questions_never_see_sensitive_lines`, `sensitive_lines_need_the_users_permission` and
  `obvious_secrets_never_reach_a_cloud_classifier`.

## 1. Automated run (CLI + MCP)

Needs macOS, the models (Settings › AI models › Download, or `scripts/fetch-models.sh`) and Ollama with `qwen3:4b` and
`nomic-embed-text`.

```bash
scripts/privacy-audit.sh
```

It makes a throwaway profile with privacy on, synthesizes a short call that says a password and
a salary (`say`), then imports it (transcribe, tag, diarize, summarize, embed), searches, shows
it, and calls every MCP read tool plus `ask_meeting`, sampling every socket the processes open
(`lsof`, every 0.1 s). It fails on any non-loopback connection or any secret in the output.

## 2. The app, with Little Snitch (or LuLu)

1. Settings › Privacy and storage: privacy mode on. Quit and reopen Mirona.
2. In Little Snitch, set Mirona to *Deny all* with alerts on, and open its Network Monitor
   filtered to Mirona. (Without Little Snitch: `scripts/privacy-audit.sh watch $(pgrep -n Mirona) 600`.)
3. Record a 3-minute call (a video in the browser is fine) in which someone says
   "the password is Tulip 4471" and "the salary is 182,000". Stop and wait for the summary.
4. Search for a word from the call in the sidebar; open the meeting; play the audio.
5. From Claude Code with Mirona's MCP connected: "What did we discuss in my last meeting?" and
   "What's the staging password?"
6. Pass if Little Snitch shows no connection attempt to any host except `localhost`/`127.0.0.1`
   (Ollama), the summary and Claude's answers never contain "Tulip", "4471" or "182,000", and
   the transcript shows those lines hidden as sensitive.

## Results

| Date | Version | Automated run | App + Little Snitch | Notes |
| --- | --- | --- | --- | --- |
| 2026-11-06 | main (M3) | PASS: 6 connections, all to Ollama on 127.0.0.1; no secret in output | not run yet | First run caught the local model missing a salary line (sensitive 0.01): fixed with the keyword safety net |
| 2026-11-06 | main (M3) | PASS (rerun) | PASS with `privacy-audit.sh watch` (no Little Snitch installed): a 34 s recording with a spoken password and salary, live tags, summary, embedding: 5 connections, all to Ollama on 127.0.0.1. Summary, follow-up draft and Claude Desktop's answers had no secret | Caught the review list quoting a sensitive line (fixed); encryption on/off and encrypted playback checked in the app |

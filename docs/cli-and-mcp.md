# CLI, MCP server and Claude Code skill (NGR-148, NGR-149, NGR-168)

The `mirona` CLI and the `mirona-mcp` server use the same engine and meeting folders as the
app. They read `config.toml` from the app folder (`~/Library/Application Support/com.nagare.mirona`
on macOS); set `MIRONA_APP_DIR` to use another one.

## Install

```bash
cargo install --path crates/mirona-cli   # installs `mirona`
cargo install --path crates/mirona-mcp   # optional: standalone `mirona-mcp` (same as `mirona mcp`)
```

Homebrew: `packaging/homebrew/mirona.rb` is ready but unpublished (private repo, no releases
yet). From a checkout: `brew install --HEAD --formula ./packaging/homebrew/mirona.rb`.

## CLI

| Command | What it does |
|---|---|
| `mirona record` | Records mic + system audio, prints segments as they arrive; Ctrl-C or `mirona stop` ends it, then it summarizes. Needs Microphone and System Audio Recording access for the terminal app. |
| `mirona stop` | Stops a recording running in another terminal (`recording.lock` / `recording.stop` files in the app folder, so it works the same on Windows). Clears a stale lock if no recorder answers within 5 s. |
| `mirona list` | Meetings, newest first |
| `mirona show <id>` | Summary, decisions, action items, notes, transcript |
| `mirona search <words…> [--client X] [--from YYYY-MM-DD] [--to …] [--speaker X] [--tag action\|decision\|open-question\|small-talk\|sensitive] [--sensitive]` | Transcript moments by keywords and meaning (hybrid search, NGR-167), best first; sensitive lines only with `--sensitive` |
| `mirona actions [--client X] [--open]` | Action items across meetings |
| `mirona import <audio>` | Transcribes (wav/mp3/m4a/flac) and summarizes a file into a new meeting |
| `mirona reindex` | Rebuilds `<meetings>/.mirona/index.sqlite` from the files |
| `mirona privacy [on\|off]` | Shows or sets privacy mode, the same setting as the app's switch. `--privacy` on any command forces it on for that run |
| `mirona retention` | Applies the retention rules now (old recordings, old meetings per client) and prints what it removed; also logged to `<meetings>/.mirona/retention.log` |
| `mirona delete <id>` / `mirona delete --client X` | Deletes a meeting (or every meeting of a client) everywhere: folder, index, embeddings, voices heard only there, corrections |
| `mirona compress` | Compresses recordings still kept as `audio.wav` (older meetings, crashed sessions) to `audio.caf` (Opus); the app does this at start |
| `mirona mcp [--http [--port N]]` | The MCP server on stdio, or on `http://127.0.0.1:N/mcp` (default `mcp_port`, 7337) |
| `mirona eval [--with jev\|local-llm\|fake] [--set file] [--no-labels] [--context] [--limit N]` | Scores the classifiers on the public synthetic set (`eval/chunks.jsonl`) plus your own corrections (`<meetings>/.mirona/labels.jsonl`, local only): accuracy and precision/recall per question, latency per call. Default: the configured classifier plus the local baseline. `--with fake` needs no network (CI). Jev uses the keychain key, or `TYPESAFE_API_KEY` |

Read commands and the MCP server index into memory and never write the app's index, so they are
safe to run while the app is open.

## MCP server

Read tools (results are compact JSON, the transcript is plain text):

- `search_meetings { query, client?, from?, to?, speaker?, tag?, include_sensitive? }`: transcript
  moments by keywords and meaning, best first (`at` = `m:ss`, speaker, snippet, `matched`), and
  the meetings they come from. Meaning needs Ollama with `nomic-embed-text`
  (`search_by_meaning` says whether it took part).
- `ask_meeting { question, meeting_id? }`: a plain question, routed by the classifier (`route`):
  `search`, `decisions`, `action_items` and `summary` are answered straight from the meetings;
  only `open_question` goes to the configured LLM, with the relevant transcript.
- `get_meeting { id }`: overview, decisions, open questions, action items, notes. No transcript.
- `get_transcript { id, from?, to?, include_sensitive? }`: `[m:ss] Speaker: text` lines,
  optionally a time range.

Sensitive lines (tagged by the classifier, or naming a password, a salary, an account number…)
read `[redacted]` everywhere unless `include_sensitive` is set, and never reach a model.
- `list_action_items { client?, open_only? }`.

Write tools, listed and callable only with `mcp_write = true` (Settings → Models and MCP →
Write tools; off by default). They change the meeting files the way the app does, and the open
app picks the change up within a few seconds:

- `set_action_item_done { id, done }`: mark an action item done, or open again.
- `rename_speaker { meeting_id, speaker_id, name }`: name a speaker (ids from `get_meeting`'s
  `speakers`); like naming them in the app, the name follows the voice to other meetings.

Every tool call, on either transport, appends one line to `<app dir>/mcp-activity.jsonl`:
`{"time", "client", "tool", "ok"}`, where `client` is `stdio` or the HTTP client's name. Never
arguments or results. The newest ~1000 lines are kept; the app shows them under MCP activity.

Every call rereads `config.toml` and the meeting folders, so it sees what the app just wrote and
works with the app closed. With `mcp_enabled = false` the server refuses to start (and calls
fail if it is turned off while running).

**Claude Code**

```bash
# With the app installed, nothing else needed (Settings → Models and MCP shows the exact path):
claude mcp add mirona -- /Applications/Mirona.app/Contents/MacOS/mirona-app mcp
# Or with the CLI installed:
claude mcp add mirona -- mirona mcp
# or, without the CLI on PATH:
claude mcp add mirona -- /path/to/mirona-mcp
```

**Claude Desktop** (`~/Library/Application Support/Claude/claude_desktop_config.json`; use the
absolute path from `which mirona`, Claude Desktop does not use your shell's PATH):

```json
{
  "mcpServers": {
    "mirona": { "command": "/Users/you/.cargo/bin/mirona", "args": ["mcp"] }
  }
}
```

### Over HTTP

For clients that connect to a URL rather than start a process. It listens on `127.0.0.1` only,
and every request needs `Authorization: Bearer <token>` (401 otherwise). Each client gets its
own token, so the activity log shows who called what and one client can be cut off alone.

1. Settings → Models and MCP → MCP over HTTP: turn it on (the app runs the server while it is
   open; `mirona mcp --http` runs it without the app). Default port 7337.
2. Client tokens: type a name (e.g. "Claude Desktop") and Create token. The token is shown once,
   with a ready `claude mcp add` line. Tokens live in `config.toml` under `[mcp_tokens]`
   (name = token); Revoke removes one and takes effect on the next request.

```bash
claude mcp add --transport http mirona http://127.0.0.1:7337/mcp \
  --header "Authorization: Bearer <token>"
```

Other clients take the URL `http://127.0.0.1:7337/mcp` plus that header (e.g. Cursor's
`mcp.json`: `{"mcpServers": {"mirona": {"url": "…", "headers": {"Authorization": "Bearer …"}}}}`).

## Claude Code skill

`skills/mirona/SKILL.md` teaches Claude when to use the tools (preparing for a meeting, finding
what a client agreed, turning action items into tasks). Install it for every project:

```bash
mkdir -p ~/.claude/skills && cp -R skills/mirona ~/.claude/skills/
```

or for one project, copy it to `<project>/.claude/skills/mirona/`. It needs the MCP server above.

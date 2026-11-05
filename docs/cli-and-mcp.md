# CLI, MCP server and Claude Code skill (NGR-148, NGR-149)

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
| `mirona search <words…>` | Matching meetings and transcript moments |
| `mirona actions [--client X] [--open]` | Action items across meetings |
| `mirona import <audio>` | Transcribes (wav/mp3/m4a/flac) and summarizes a file into a new meeting |
| `mirona reindex` | Rebuilds `<meetings>/.mirona/index.sqlite` from the files |
| `mirona mcp` | The MCP server on stdio |

Read commands and the MCP server index into memory and never write the app's index, so they are
safe to run while the app is open.

## MCP server

Tools (all read-only; results are compact JSON, the transcript is plain text):

- `search_meetings { query }`: matching transcript moments (`at` = `m:ss`, snippet) and meetings.
- `get_meeting { id }`: overview, decisions, open questions, action items, notes. No transcript.
- `get_transcript { id, from?, to? }`: `[m:ss] Speaker: text` lines, optionally a time range.
- `list_action_items { client?, open_only? }`.

Every call rereads `config.toml` and the meeting folders, so it sees what the app just wrote and
works with the app closed. With `mcp_enabled = false` the server refuses to start (and calls
fail if it is turned off while running).

**Claude Code**

```bash
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

## Claude Code skill

`skills/mirona/SKILL.md` teaches Claude when to use the tools (preparing for a meeting, finding
what a client agreed, turning action items into tasks). Install it for every project:

```bash
mkdir -p ~/.claude/skills && cp -R skills/mirona ~/.claude/skills/
```

or for one project, copy it to `<project>/.claude/skills/mirona/`. It needs the MCP server above.

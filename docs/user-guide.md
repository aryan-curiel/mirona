# User guide

Mirona needs macOS 14.6 or later on Apple silicon.

## First run

Install from the DMG ([README](../README.md#install)) and open Mirona. A short setup runs before
the main window:

1. **Welcome:** what Mirona does.
2. **Permissions and models:**
   - **Microphone:** your side of the call.
   - **System audio:** everyone else, through your speakers. Mirona records the sound, never the
     screen.
   - **Speech and speaker models:** click Download. About 1 GB on a Mac with 16 GB of memory or
     more, about 700 MB otherwise. It's checked file by file and resumes if you cancel or lose
     the connection.
3. **Make it yours:** privacy mode, the consent reminder, call detection, whether AI agents
   may read your meetings, and the meetings folder. All of these can be changed later in
   Settings.
4. **You're ready.**

Settings › About › Show the welcome tour replays it.

If you turned a permission down, go to System Settings › Privacy & Security › Microphone (or
Screen & System Audio Recording) and switch Mirona on.

## Recording

Start a recording from any of these:

- the record button in the sidebar
- the menu bar icon › Start recording
- the shortcut, <kbd>⌘⌥⇧R</kbd> by default (Settings › General › Record shortcut)
- the suggestion that appears when Zoom, Teams, Meet or another call app opens the microphone
  (Settings › General › Call detection)

Before each recording, the **consent reminder** asks who the call is with and shows a message you
can paste in the chat. Recording rules differ by country. Mark a client as "already agreed" and it
won't ask again for them.

While recording, the transcript appears live, split into "Me" and "Them", and lines are tagged
as action items, decisions, open questions or small talk as they come in. Write notes in the
**Notes** tab as you go. Stop with the same button or shortcut. Mirona then:

1. compresses the audio, splits "Them" into each speaker and matches voices it knows
2. joins lines split mid-sentence and tags any lines still untagged
3. answers the meeting questions: type, client, priority, tone, whether it needs a follow-up
4. writes the summary and enhanced notes, and drafts a follow-up email when one is needed
5. indexes the lines for search by meaning

Closing the window keeps Mirona in the menu bar, so recording and the shortcut still work.
Quit from the menu bar icon.

**Import audio…** in the sidebar transcribes an existing recording (wav, mp3, m4a, flac, aac,
mp4) the same way.

## A meeting

- **Transcript:** speakers, timestamps and tags. Click a line to inspect it: the answers the
  classifier gave, with their confidence, and one-click corrections. Corrections are kept as
  labelled examples (Settings › Advanced › Your corrections).
- **Notes:** your notes, and the version enhanced with the transcript.
- **Summary:** written from a template. Timestamps jump back to the transcript. Pick another
  template to rewrite it.

Answers below their confidence threshold show as **Needs review** instead of being filed as
fact. Thresholds are in Settings › Advanced.

## Speakers

Mirona separates speakers after each call. In **People and voices**, name a speaker once and the
name follows that voice to other meetings. You can also merge two speakers that are the same
person. Voiceprints stay on your Mac.

## Action items and search

- **Action items** lists them across all meetings, by client, with owners and due dates when
  someone said them.
- **Search all meetings** (top of the sidebar) matches words and meaning. Filter by client, tag,
  dates or speaker. Meaning-based search uses Ollama with `nomic-embed-text`
  (`ollama pull nomic-embed-text`); without it, search matches keywords only.

## Summaries and models

Settings › AI models picks the provider for each stage:

| Stage | Local | Cloud (privacy mode off) |
| --- | --- | --- |
| Speech to text | Parakeet (default); Whisper only in builds with the `whisper` feature | |
| Speaker detection | speakrs (Neural Engine on 16 GB+ Macs, CPU otherwise) | |
| Classification | Local LLM through Ollama | Jev (default when privacy mode is off) |
| Summaries and notes | Ollama (`qwen3:4b` by default) | OpenAI-compatible, Anthropic |

API keys go in Settings › AI models › API keys. They're stored in the macOS keychain, never in
config files.

Summary templates are Markdown files in
`~/Library/Application Support/com.nagare.mirona/templates/`. Edit them, or add your own.
After each call the meeting type picks the template when the classifier is sure of it; otherwise
Settings › General › Default summary template is used.

## Privacy mode

On by default. In privacy mode:

- every cloud provider is blocked at one network gate, so only this Mac (Ollama, LM Studio) is
  reachable
- classification uses the local LLM instead of Jev, with less accurate tags

The one thing that goes out in privacy mode is the model download you start yourself. It
fetches fixed files from Hugging Face and GitHub and sends nothing about your meetings. See
[PRIVACY.md](../PRIVACY.md).

**Sensitive lines** (tagged by the classifier, or mentioning a password, salary, account or card
number, or health details) stay out of summaries, notes and follow-up emails, are never indexed
for meaning search, and show as `[redacted]` to AI agents unless the agent asks for them
(`include_sensitive`, meant for when you asked it to). The classifier does read each
line to tag it, so with privacy mode off and Jev picked, lines go to Jev once for tagging.

## Encryption and retention

Settings › Privacy and storage:

- **Encryption:** recordings, transcripts, search vectors, voiceprints and your corrections are
  encrypted on disk, and the search index is kept in memory only. The key stays in your
  keychain. Meeting notes and summaries (`meeting.md`) stay readable. The audio of a call in
  progress is sealed when the recording stops.
- **Delete old recordings:** removes audio after some days and keeps transcripts, notes and
  summaries.
- **Per client:** deletes a client's meetings entirely after some days, or right now.

## AI agents (MCP)

Settings › Agents (MCP) lets Claude and other MCP clients read your meetings. They can search,
read summaries and transcripts, list action items and ask questions. With **Write tools** on,
they can also mark action items done and rename speakers. Every call is logged under **Agent
activity**. Setup for each client is in [CLI and MCP](cli-and-mcp.md).

## Where things are

| What | Where |
| --- | --- |
| Meetings | `~/Documents/Mirona/` (Settings › General › Meetings folder) |
| Settings | `~/Library/Application Support/com.nagare.mirona/config.toml` |
| Models | `~/Library/Application Support/com.nagare.mirona/models/` |
| Summary templates | `~/Library/Application Support/com.nagare.mirona/templates/` |
| API keys and the encryption key | macOS keychain, service `mirona` |

To uninstall, delete the app, the folder in Application Support, and the meetings folder if you
don't want to keep it.

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

Before each recording, the **consent reminder** asks what the call is about (a tag) and shows a
message you can paste in the chat. Recording rules differ by country. Mark a tag as "already
agreed" and it won't ask again for calls with that tag.

With **Use calendar event titles** on (Settings › General › Call detection), you can tick tags
to record without asking: a call is recorded when the event title names the tag
(`project:setr` matches "Setr sync"). Nothing records on its own otherwise.

While recording, the transcript appears live, split into "Me" and "Them" (a note at the top
says so: the other side is told apart into speakers after the call). A line still being
spoken shows in italics ("Still speaking…") and grows as the sentence goes on; once the sentence
ends it becomes a normal line and is tagged as an action item, decision, open question or small
talk. Write notes in the
**Notes** tab as you go. Muted in the call and talking to someone in the room? Click **Mute me**
in the recording bar, the menu bar icon › Mute me, or press <kbd>⌘⌥⇧M</kbd> (Settings ›
General › Mute shortcut): until you unmute, your mic records silence, so none of it reaches the
transcript or the audio file. On macOS the menu bar icon shows it too: its bars move with
a red dot while recording, and the dot becomes a crossed-out mic while you're muted. Stop with
the same button or shortcut. Mirona then:

1. compresses the audio, splits "Them" into each speaker and matches voices it knows
2. joins lines still split mid-sentence (now that each line has its speaker) and tags any lines
   still untagged
3. answers the meeting questions: type, priority, tone, whether it needs a follow-up, and which
   of your tags fit
4. writes the summary and enhanced notes, and drafts a follow-up email when one is needed; if no
   tag fits, it suggests one new tag for you to accept or reject
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
  template to rewrite it. A meeting still called "New meeting" takes a short title from its
  summary; a name you or the calendar gave is never replaced.

Each question has two confidence levels, in Settings › Advanced › Thresholds:

- **Approve at** (0.85 by default; 0.95 for decisions): at or above it, an answer is filed as fact.
- **Reject below** (0.60 by default): below it, the answer is treated as "no" without asking.

Answers in between show as **Needs review**. Rejected ones aren't listed under **To check**; a
line there says how many were rejected for low confidence, and **Show** lists them so you can
still say Yes to one. Sensitive lines are never rejected without asking.

## Tags

A meeting can have several tags, such as the client or project it is for. A tag is `group:name`
(`project:setr`) or just a name. Case doesn't matter.

- **Settings › Tags** lists your tags by name. The classifier goes by the name alone, so make it
  clear (`project:setr`, not `p1`). Renaming a tag renames it on every meeting and in its rules.
  Removing a tag from the list leaves it on past meetings. Descriptions from earlier versions
  are dropped.
- After each call the classifier asks about each tag. Sure answers add the tag; unsure ones
  wait in the inspector's review list as "Tag this meeting?". It never removes a tag, and never
  changes a tag you set, removed or answered.
- When no tag fits, Mirona may suggest one new tag from the summary ("New tag?"). Accept it and
  it's added to the meeting and to Settings › Tags; reject it and it's dropped.
- Tags in the same group share a colour everywhere (all `project:` tags look alike); tags without
  a group are grey. Colours are picked for you.
- In the inspector, **This meeting › Tags** shows the tags. × removes one; type in the box to add
  one. Suggestions match any part of a tag: "setr", "proj" or even "pset" find `project:setr`.
  Use the arrow keys and Enter, or press Enter on new text to add a new tag.

Clients from Mirona 0.1.0 become `project:` tags automatically, with their rules.

## Speakers

Mirona separates speakers after each call. In **People and voices**, name a speaker once and the
name follows that voice to other meetings. You can also merge two speakers that are the same
person. Voiceprints stay on your Mac.

When a call ends with voices nobody has named yet, a note above the transcript says how many.
**Name them** goes through them one at a time: each shows its three clearest lines (play them to
hear who it is) and a field to type a name or pick someone, people from this meeting first. Skip
a voice to leave it for later; the note stays until every voice has a name.

If one line went to the wrong person, hover it and click **Who said this?** to move just that line
to someone else, or to a new name. A moved line keeps its person when you re-run speaker
detection, and moving lines never changes anyone's voiceprint. Your own lines ("You") can't be
moved.

## Action items and search

- **Action items** lists them across all meetings, filtered by tag, with owners and due dates when
  someone said them. When you mark a transcript line as an action, the summary model rewrites it
  as a short task (a sensitive line is never sent to a cloud model for this; it keeps its words).
  A line the classifier is sure is an action is listed even when the summary leaves it out,
  reworded the same way; regenerating the summary keeps it.
- **Search all meetings** (top of the sidebar) matches words and meaning. Filter by meeting tag, line tag,
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

What goes out in privacy mode is what you start yourself: the model download (fixed files from
Hugging Face and GitHub) and the update check (GitHub, only if you turned it on in Settings ›
About or clicked Check now). Neither sends anything about your meetings. When an update is out, a
banner offers Install and restart. See
[PRIVACY.md](../PRIVACY.md).

**Sensitive lines** (tagged by the classifier, or mentioning a password, salary, account or card
number, or health details) stay out of summaries, notes and follow-up emails, are never indexed
for meaning search, and show as `[redacted]` to AI agents, unless you allow agents to read them
(Settings › Agents (MCP) › Sensitive lines, off by default). The classifier does read each line
to tag it, so with privacy mode off and Jev picked, lines go to Jev once for tagging. Lines with
an obvious password, salary or account number are tagged on your Mac and never sent.

## Encryption and retention

Settings › Privacy and storage:

- **Storage used:** how much disk meeting audio, transcripts and notes, the search index, the
  speech and speaker models, and Ollama's models (shared with other apps) take on this Mac.
- **Encryption:** recordings, transcripts, search vectors, voiceprints and your corrections are
  encrypted on disk, and the search index is kept in memory only. The key stays in your
  keychain. Meeting notes and summaries (`meeting.md`) stay readable. The audio of a call in
  progress is sealed when the recording stops.
- **Delete old recordings:** removes audio after some days and keeps transcripts, notes and
  summaries.
- **Per tag:** deletes a tag's meetings entirely after some days, or right now. With several
  tags, the shortest rule wins.

## AI agents (MCP)

Off until you turn it on, in onboarding or Settings › Agents (MCP). Then Claude and other MCP
clients can read your meetings. They can search,
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

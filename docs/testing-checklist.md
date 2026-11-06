# Mirona testing checklist (milestone 1 → gate 1)

Tick items as you go (`- [x]`). Anything that fails: note what you saw under **Findings** at the end
(or open a Linear issue in the Mirona project) with the meeting folder name and time.

## 0. One-time setup

- [ ] **Build and install the app**
  ```bash
  cd app && pnpm install && pnpm tauri build --bundles app
  ```
  Copy `target/release/bundle/macos/Mirona.app` to `/Applications`.
- [ ] **Download the speech models** (about 700 MB)
  ```bash
  scripts/fetch-models.sh "$HOME/Library/Application Support/com.nagare.mirona/models"
  ```
- [ ] **Ollama for summaries** (already installed on this Mac): `ollama serve` is running and
  `ollama list` shows `qwen3:4b`. If not: `brew install ollama && ollama pull qwen3:4b`.
- [ ] **Clean slate (optional):** delete the 4 demo meetings in `~/Documents/Mirona`, and remove
  "Mirona Probe" from System Settings → Privacy & Security → Microphone and → Screen & System
  Audio Recording.
- [ ] **Open Mirona from /Applications.** The setup screen asks for the microphone and system
  audio, and shows any missing models.
  Expected: both permissions show as granted and no models are missing.

## 1. Recording (the core loop)

- [ ] **Mic only.** Start recording (sidebar button), say two sentences, stop.
  Expected: lines labelled "Me" appear a few seconds after you speak; after Stop, a summary appears
  in the Summary tab within ~30 s.
- [ ] **System audio, speakers.** Start recording, play a YouTube video with speech for 30 s, stop.
  Expected: lines labelled "Them".
- [ ] **System audio, AirPods.** Same with AirPods connected.
  Expected: "Them" lines still appear (this was the bug fixed for AirPods call mode).
- [ ] **Device change mid-call.** Start recording on the speakers, connect AirPods after 20 s, keep
  talking and playing audio, stop.
  Expected: no crash, transcript continues after the switch, a small notice names the new device.
- [ ] **Long recording.** Record for 30+ minutes (a real meeting counts).
  Expected: the transcript keeps up, the app stays responsive, the summary covers the whole meeting.
- [ ] **Recording file.** In the meeting folder (`~/Documents/Mirona/<date>-…/`) after stopping:
  there's `audio.caf` (no `audio.wav`), and it's roughly 10 MB per hour.
- [ ] **Crash safety.** Start recording, talk for a minute, then force-quit Mirona
  (Activity Monitor). Reopen it.
  Expected: the meeting is there with the transcript so far; its `audio.wav` gets compressed to
  `audio.caf` shortly after launch.

## 2. Start and stop from everywhere

- [ ] **Menu-bar icon:** Start recording / Stop from the tray menu.
- [ ] **Global shortcut** (default ⌘⌥⇧R) while another app is in front: starts and stops recording.
- [ ] **Change the shortcut:** Settings → Record shortcut, press a new combination, then use it.
  Expected: the new one works, the old one doesn't, and the tray and recording bar show the new one.
- [ ] **Closing the window** keeps Mirona in the menu bar; clicking the Dock icon brings the window
  back.
- [ ] **One recording at a time:** while the app is recording, run `mirona record` in a terminal.
  Expected: it refuses with "Already recording into this meetings folder".

## 3. Real calls (needed for NGR-137)

For each call: record the whole call, then check the transcript has both "Me" and "Them" lines.

- [ ] **Zoom** (desktop app). A quick check: zoom.us/test.
- [ ] **Google Meet** in a browser.
- [ ] **Microsoft Teams** (desktop app).

## 4. Transcript, notes, summary

- [ ] **Timestamp playback:** click a timestamp; audio plays from that moment. Click a later one; it
  jumps there.
- [ ] **Search** in the transcript highlights matches.
- [ ] **Fix text:** double-click a line, correct a word, close and reopen the meeting.
  Expected: the fix is kept.
- [ ] **Notes during a call:** type notes while recording.
  Expected: each line shows the time it was written; after stopping, enhanced notes appear beside
  yours and yours are unchanged.
- [ ] **Summary:** overview, decisions, action items (owner and due date when said) and open
  questions; clicking an item's time jumps to that moment.
- [ ] **Regenerate** with another template (Summary tab).
  Expected: the text streams in as it's written and the template name changes.
- [ ] **Import:** sidebar → Import an audio file (an mp3 or m4a of a meeting).
  Expected: a progress bar, then a new meeting with transcript and summary.

## 5. Settings (Models and MCP)

- [ ] **Speech-to-text:** switch to Whisper, record 20 s, switch back to Parakeet.
  (Whisper needs `scripts/fetch-models.sh … --whisper` first.)
- [ ] **Privacy mode on:** OpenAI and Anthropic can't be selected.
- [ ] **API keys:** add a key for Anthropic (and OpenAI, Jev if you have them).
  Expected: "saved in the keychain"; the key isn't in `config.toml`.
- [ ] **Anthropic summaries:** privacy mode off → Summaries: Anthropic → Regenerate a summary.
  Expected: the summary says it was written by the Claude model. Back to Ollama afterwards if you
  want everything local.
- [ ] **Meetings folder:** pick another folder, record something, check it lands there; switch back.
- [ ] **config.toml:** edit a value in the file shown at the bottom of Settings (for example
  `template`), restart Mirona, and see the change in Settings.

## 6. Claude Code and the CLI

- [ ] **Connect Claude Code** with the command shown in Settings → MCP server
  (`claude mcp add mirona -- "/Applications/Mirona.app/Contents/MacOS/mirona-app" mcp`).
- [ ] **Ask Claude Code:** "What did we decide in my last meeting?", "List my open action items",
  "Find where we talked about <topic>".
  Expected: answers cite meetings and timestamps.
- [ ] **MCP switch off** in Settings, then ask again.
  Expected: Claude reports the Mirona server is turned off.
- [ ] **Skill (optional):** install `skills/mirona` (see `docs/cli-and-mcp.md`) and ask Claude Code
  to prepare for a meeting with a client.
- [ ] **CLI (optional):** `cargo install --path crates/mirona-cli`, then `mirona list`,
  `mirona search <word>`, `mirona actions --open`.

## 6b. Milestone 2: who and what

- [ ] **Jev key:** Settings → API keys → Jev. Turn privacy mode off and set Classification to Jev.
  Privacy mode on means the local model answers instead, with less accurate tags.
- [ ] **Tags live:** during a real call, lines get Action / Decision / Question / Small talk /
  Sensitive tags a second or two after they appear.
- [ ] **Speakers:** after Stop, "Them" is split into Speaker 1, Speaker 2… Name one (click the
  name). Next meeting with that person: they're named automatically.
- [ ] **Name suggestions:** when someone is addressed by name ("over to you, Keiko"), a
  "Keiko? Confirm" chip appears.
- [ ] **Fix a wrong match:** rename a wrongly named speaker; the People view updates.
- [ ] **People view:** merge two entries that are the same person; delete a voiceprint.
- [ ] **Inspector:** Inspect a line → its questions show at the top with bars and thresholds;
  Correct one → the tag and action items update at once.
- [ ] **Needs review:** items below threshold are in the inspector's review list; Confirm / Not
  this settles them.
- [ ] **Meeting chips:** type, client (add your clients in Settings first), priority, tone,
  follow-up. The summary uses the template matching the meeting type.
- [ ] **Follow-up draft:** shown with Copy when a follow-up is needed.
- [ ] **Action items view:** filter by client and status; links jump to the moment.
- [ ] **Claude Code:** "What do I owe Northwind?" returns action items, with unconfirmed ones marked.

## 7b. Gate 2: labelled set (NGR-163)

- [ ] Over your meetings, correct tags in the inspector (each correction is a labelled example)
  until you have 200 or more.
- [ ] Run `mirona eval --with jev --with local-llm` and record: does Jev match or beat the local
  model on action items and decisions, at lower latency?

## 7. Gate 1: two weeks of daily use (NGR-151)

Record every work call for two weeks and use Claude Code with Mirona at least once a day.
Gate 1 is done when the last week had no lost recordings.

| Day | Calls recorded | Lost or broken recordings | Transcript quality (1–5) | MCP questions asked (and failures) | Wishes |
| --- | --- | --- | --- | --- | --- |
| 1 | | | | | |
| 2 | | | | | |
| 3 | | | | | |
| 4 | | | | | |
| 5 | | | | | |
| 6 | | | | | |
| 7 | | | | | |
| 8 | | | | | |
| 9 | | | | | |
| 10 | | | | | |
| 11 | | | | | |
| 12 | | | | | |
| 13 | | | | | |
| 14 | | | | | |

## Findings

<!-- One bullet per problem: what you did, what you expected, what happened, meeting folder + time. -->

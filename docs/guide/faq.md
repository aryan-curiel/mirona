# FAQ and troubleshooting

## General

### Is Mirona free?

Yes. Mirona is open source under the Apache 2.0 license. Cloud models you choose to use (OpenAI,
Anthropic, Jev) are billed by their providers, on your own keys.

### Does anything I record leave my Mac?

Not with privacy mode on (the default): speech-to-text, speaker detection, tagging, summaries and
search all run on your Mac. With privacy mode off, transcript text goes only to the cloud models
you pick. There is no telemetry. See [Privacy](privacy.md) and [PRIVACY.md](../../PRIVACY.md).

### Do the other people on the call know they're being recorded?

Only if you tell them. Mirona doesn't join the call as a bot; it records your Mac's audio. The
[consent reminder](recording.md#the-consent-reminder) gives you a message to paste in the chat.
Recording laws differ by country: in many places everyone has to agree.

### Which call apps does it work with?

All of them. Mirona records your mic and your Mac's system audio, so any app works: Zoom, Teams,
Meet, Webex, Slack huddles, FaceTime, Discord, phone calls through your Mac. Call detection
suggests recording for the apps listed in [Call detection](recording.md#call-detection).

### Does Mirona work in other languages?

Not well yet. The default speech model (Parakeet TDT 0.6B v2) is English-only. Builds with the
`whisper` feature can use Whisper, which handles other languages; a multilingual Parakeet model is
being evaluated.

### Does it work on Intel Macs, Windows or Linux?

Not yet. Mirona needs Apple silicon and macOS 14.6 or later. Windows and Linux capture are on the
roadmap.

### How much disk space does it use?

About 1 GB for the speech and speaker models, about 11 MB per hour of recorded audio, and a little
for text. Ollama's `qwen3:4b` takes about 2.5 GB more. Settings › Privacy and storage › Storage
used shows the exact numbers, and [retention rules](privacy.md#delete-old-recordings) can delete
old audio for you.

### Can I use Mirona without Ollama?

Yes. Without Ollama, recording, transcription, speakers and keyword search work, but there are no
summaries, enhanced notes or follow-up emails, and tagging falls back to Jev (privacy mode off,
with a key) or nothing. You can also pick a cloud model for summaries, or another local server
(LM Studio) through **OpenAI-compatible**.

### Can I use it for in-person meetings?

Yes, but everyone in the room is heard through your mic, and the mic side is always *You*. To get
each person told apart, record on your phone and [import](recording.md#importing-a-recording)
the file. See [In-person meetings](workflows.md#in-person-meetings-and-voice-memos).

### Can I edit the transcript?

Yes. Hover a line and click **Fix text** to correct it, or **Who said this?** to move it to
another person.

### Can I export a meeting?

Every meeting already is a Markdown file (`meeting.md`) in your meetings folder, readable by any
editor. `mirona show <id>` prints a meeting as text. The **Follow-up email** has a **Copy** button.
See [Files and folders](files.md).

### Can I use it with Obsidian?

Yes: point a vault at the meetings folder. Each meeting's `meeting.md` is Obsidian-friendly
Markdown with YAML front matter. Write your own notes in Mirona, since it rewrites `meeting.md`.

### What happens to my data if I uninstall?

Nothing, until you delete it. Your meetings stay in the meetings folder. See
[Uninstalling](files.md#uninstalling).

## Recording

### The other side of the call isn't in the transcript

Mirona needs **System Audio Recording** access. Open System Settings › Privacy & Security ›
Screen & System Audio Recording › *System Audio Recording Only*, switch Mirona on, and start a new
recording. Also check the **Call** meter in the recording bar: if it stays flat while people talk,
macOS isn't giving Mirona the audio.

### My side isn't in the transcript

Check System Settings › Privacy & Security › **Microphone**, and that you aren't muted in Mirona
(**Unmute me** in the recording bar, or the crossed-out mic in the menu bar). Mirona records your
Mac's default input device: check System Settings › Sound › Input.

### Start recording asks me to finish setup

The speech models aren't downloaded, or a permission is missing. **Finish setup** in the sidebar
takes you to the step that needs doing.

### The shortcut doesn't work

Another app may use the same combination. Pick a different one in Settings › General › Record
shortcut. It needs ⌘, ⌃ or ⌥, and the mute shortcut must differ from it.

### Call detection didn't offer to record

Check Settings › General › Call detection is on, and that the app is in the
[list](recording.md#call-detection). The banner shows when the app starts using the microphone,
so it may not show if you joined the call before opening Mirona: start recording by hand.

### My AirPods sound different when Mirona records

Bluetooth headsets switch to a lower-quality mode when their microphone is in use. That's how
Bluetooth works, not Mirona; the call app does the same. Mirona follows the switch. For the best
transcript, use the Mac's built-in mic or a wired one.

### Can I record a call I'm only listening to (a webinar)?

Yes. Start recording as usual; with your mic silent (or **Mute me** on), only the other side is
transcribed.

## After the call

### There's no summary

The summary model wasn't reachable. With the default setup, make sure Ollama is running (its icon
is in the menu bar) and has the model:

```bash
ollama list            # qwen3:4b should be listed
ollama pull qwen3:4b   # if it isn't
```

Then open the meeting's **Summary** tab and click **Write summary**. The transcript is saved either
way; nothing is lost.

### The summary missed something or got it wrong

- **Regenerate** with another template, or [edit the template](summaries.md#editing-a-template)
  to ask for what you need.
- Try a bigger local model (`qwen3:8b`) or a cloud model in Settings › AI models.
- Action items the classifier is sure of get listed even if the summary leaves them out.

### Too many things in "To check"

Lower **Approve at** for the questions you trust, in Settings › Advanced › Thresholds. If the
classifier keeps suggesting wrong things, raise **Reject below** instead. Each answer you correct
is saved as an example; `mirona eval` shows how the classifier does on them.

### Lines are tagged wrong

Click the line and **Change** the wrong answer. With privacy mode off and a Jev key, tagging is
more accurate than with the local model. **More › Re-run classification** tags the whole meeting
again, keeping your corrections.

### Speakers are mixed up

- Click a speaker's name on a line, pick who said it, then choose **Only this line** or **All N
  lines**. Pick **Me** for lines that were you.
- **Split voices…** above the transcript: drag lines between voices, or find voices again with
  more or less sensitivity. Lines you moved keep their person.
- Merge duplicates on **People and voices** with **Same person as…**.
- Recognition improves as a voice is named in more meetings.

### Search by meaning doesn't work

It needs Ollama with `nomic-embed-text` (`ollama pull nomic-embed-text`). Without it, search
matches keywords only. Older meetings are indexed when the app starts.

### Where's my follow-up email?

Only calls that need one get a draft. Open the inspector's **This meeting › Follow-up email**:
**Show follow-up draft** if it's *Needed*, or **Change** it to *Needed* and a draft is written.

## Settings and data

### I moved my meetings folder and my meetings are gone

Mirona opens the folder you pick; it doesn't move files. Move your meeting folders (and the
`.mirona` folder) into the new folder, or point the setting back at the old one.

### How do I use Mirona on two Macs?

Each Mac keeps its own meetings. You can copy the meetings folder between them, but don't run two
Mirona apps on the same synced folder at the same time. With encryption on, the second Mac also
needs the key from the first Mac's keychain.

### I turned encryption on. Can other apps still read my meetings?

`meeting.md` (title, notes, summary, action items) stays readable. Transcripts, audio, voiceprints,
search vectors and corrections are encrypted, and only Mirona can read them, with the key in your
keychain.

### How do I check for updates?

Settings › About › **Check now**, or turn on **Check for updates automatically**. New versions
are also on the [Releases](https://github.com/aryan-curiel/mirona/releases) page.

### How do I reset Mirona?

Quit it, delete `~/Library/Application Support/com.nagare.mirona`, and open it again: the setup
runs as on first install. Your meetings folder isn't touched.

## Still stuck?

Open an issue on [GitHub](https://github.com/aryan-curiel/mirona/issues) with what you did,
what you expected and what happened. For anything security-related, see
[SECURITY.md](../../SECURITY.md).

# Files and folders

Your meetings are plain files. Mirona's database (the search index) is built from them and can be
rebuilt at any time, so the files are all you need to back up.

## Where things are

| What | Where |
| --- | --- |
| Meetings | `~/Documents/Mirona/` (Settings › General › Meetings folder) |
| Settings | `~/Library/Application Support/com.nagare.mirona/config.toml` |
| Speech and speaker models | `~/Library/Application Support/com.nagare.mirona/models/` |
| Summary templates | `~/Library/Application Support/com.nagare.mirona/templates/` |
| Agent activity log | `~/Library/Application Support/com.nagare.mirona/mcp-activity.jsonl` |
| API keys and the encryption key | macOS keychain, service `mirona` |
| Ollama's models | `~/.ollama/models/` (Ollama's own folder, shared with other apps) |

## The meetings folder

```
~/Documents/Mirona/
  2026-11-10-0930-claims-desk-weekly-sync/
    meeting.md          title, tags, answers, summary, action items, your notes
    transcript.jsonl    one line per transcript line
    audio.caf           the recording (Opus); audio.wav while recording
    embeddings.json     search vectors, one per line
  2026-11-08-1400-sprint-planning/
    …
  .mirona/
    index.sqlite        search index, rebuilt from the folders
    labels.jsonl        your corrections
    voices.json         named people and their voiceprints
    retention.log       what retention rules removed
```

Each meeting folder is named after its start time and title, and is renamed when you rename the
meeting.

### meeting.md

YAML front matter with everything Mirona knows about the meeting, then Markdown you can read
anywhere. Here's the demo client call, shortened:

```markdown
---
id: claims-desk
title: Claims Desk weekly sync
created: 2026-11-10T09:30:00+01:00
duration_min: 8
language: English
tags:
- project:Alder Restoration
speakers:
- id: aryan
  name: Aryan
- id: nadia
  name: Nadia
- id: s3
  name: Speaker 3
answers:
  meetingType:
    value: Client call
    probability: 0.93
    confirmed: true
  priority: 3.6
  followUp: true
  followUpDraft: |-
    Hi Nadia,
    …
summary:
  template: Client call
  overview: Weekly check-in on the Claims Desk collections workspace. …
action_items:
- id: a1
  text: Announce the insurer grouping to the field team
  owner: Speaker 3
  due: Fri
  done: false
---

# Claims Desk weekly sync

## Notes

insurer grouping → monday?
overdue filter
paid claims before migration??

<!-- mirona: everything below is rendered from the frontmatter and rewritten on save -->

## Summary

Weekly check-in on the Claims Desk collections workspace. Insurer grouping is ready and ships to
the field team on Monday. One migration question is still open.

### Decisions

- Ship insurer grouping to the field team on Monday (03:02)

## Action items

- [ ] Announce the insurer grouping to the field team (Speaker 3, due Fri) (03:20)
- [ ] Add the overdue filter and send a preview build (Aryan, due Thu) (04:45)
```

- **Your notes** (the `## Notes` section) are best edited in Mirona's Notes tab. You can edit
  them in another editor while Mirona is closed; Mirona reads them when it starts. While it's open,
  Mirona may rewrite `meeting.md` (when you tag, tick an action item…) and overwrite a hand edit.
- Everything below the `<!-- mirona: … -->` line is generated from the front matter and rewritten
  whenever the meeting changes. Edit the meeting in Mirona instead.
- `meeting.md` stays readable when encryption is on.

### transcript.jsonl

One JSON object per line of the transcript: who spoke, when (milliseconds from the start), what
they said, its tags and the classifier's answers.

```json
{"id":"c4","speakerId":"nadia","startMs":182000,"text":"Yes, let's ship it Monday. Inés, can you let the field team know?","tags":[{"kind":"decision","label":"Decision"}],"answers":[…]}
```

The file is append-only while recording, and encrypted when encryption is on.

## Backing up

- **Time Machine** covers everything above. Nothing else to set up.
- **Copy the meetings folder** anywhere: it's self-contained. On a new Mac, point Settings ›
  General › Meetings folder at it and the index is rebuilt.
- **Encrypted meetings** also need the key from your keychain. See
  [Encryption](privacy.md#encryption).
- **Sync tools** (iCloud Drive, Dropbox, Syncthing) work on the meetings folder for keeping a
  copy. Avoid running Mirona on two Macs against the same synced folder at once: two apps writing
  the same files can conflict.

## Moving things

- **Meetings folder:** Settings › General › Meetings folder › **Choose…**. Move or copy your
  existing meetings there first; Mirona opens the folder you pick and doesn't move files for you.
- **Models folder:** Settings › AI models › Models folder › **Choose…**. Point it at a folder that
  already has the models and only missing files are downloaded.

## Rebuilding the index

If search ever looks wrong, rebuild the index from the files:

```bash
mirona reindex
```

The app also rebuilds it when it starts.

## Uninstalling

1. Quit Mirona (menu bar icon › **Quit Mirona**).
2. Delete **Mirona** from Applications.
3. Delete `~/Library/Application Support/com.nagare.mirona` (settings, models, templates).
4. Delete the meetings folder if you don't want to keep your meetings.
5. Optional: in Keychain Access, delete the items for service `mirona` (API keys, encryption key).

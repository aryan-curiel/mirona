# Naming voices, moving lines, and tag colours

Date: 2026-11-10. Status: ready for review.

Two independent changes that ship together: (1) naming unnamed voices after speaker
detection, plus moving a single line to another person; (2) simpler tags: no descriptions,
colour by group, and autocomplete that matches anywhere in the tag.

## 1. Unnamed voices

### Why

Today a voice is named by clicking its name on one of its lines (`SpeakerName` in
`app/src/views/Meeting.tsx`). On a long meeting the user has to scroll to find the lines of
each "Speaker N", and has no quick way to hear who it is.

### What the user sees

- When speaker detection has finished on a meeting (after the call, after import, or after
  "Re-run speaker detection") and the meeting has unnamed voices, a banner sits at the top of
  the transcript: "2 voices aren't named yet. **Name them**". Chosen by the user over a card
  in the inspector's "To check".
- **Name them** opens a sheet that walks the voices one at a time: "Voice 1 of 2 — Speaker 3",
  its line count and total talk time, then up to 3 sample lines, each with a play button that
  plays that line from the meeting audio.
- Under the samples: one field "Name or pick a person…". It suggests people in this meeting
  first, then everyone else (`list_people`), with the same fuzzy matcher as tags (below).
  Typing a new name creates a person. If the voice already has a `suggestion`, it's
  pre-filled.
- Actions: **Save** (names it and moves on), **Skip** (next voice, stays unnamed), **Next ›**,
  close. When the last voice is named the sheet closes and the banner disappears.
- The banner reappears whenever unnamed voices remain; skipped voices count as unnamed. There
  is no "never ask again" for a meeting (user's call: keep it simple; add if it nags).

### Rules

- **Unnamed** = a speaker the user never named: still "Speaker N" from diarization, or the
  leftover "Them" voice (detection keeps lines as "them" when it can't place them; naming it
  creates a person for those lines, as `name_speaker` already does). "You"/"me" is never
  listed. The id pattern doesn't work (`enroll_speaker` keeps `-s{n}` ids) and the store's
  real test (`voices.person(id).is_none()`) isn't visible to the UI, so expose it: builder's
  call between a `named: bool` on `Speaker` (core type → regenerate bindings) or an equivalent
  the UI can read. Don't infer it from `voiceprint == None`.
- **Sample lines** (my call): the voice's 3 longest lines by `endMs - startMs`, at least
  ~2 s, spread across the meeting (no two within 60 s of each other if avoidable), skipping
  sensitive lines unless "Show sensitive text" is on. Lines from before `endMs` was kept
  (`endMs == 0`) rank by text length instead. Computed in the UI from `m.segments`; no backend.
  Overlap isn't tracked per segment, so "clearest" means "longest"; good enough for picking
  who it is.
- **Naming** goes through the existing `name_speaker` command unchanged: a known name merges
  into that person, a new name enrols a new person, and the voiceprint updates as today.
- Play uses the existing audio hook in `Meeting.tsx` (`play(ms)`); add stopping at `endMs` if
  it's cheap, otherwise playing from the line's start is fine.

### Move one line to someone else

- Each transcript line gets a "Who said this?" control next to the existing hover buttons
  (same `reveal` pattern). It opens the same person picker: people in this meeting first, then
  everyone, then "New person: <typed name>".
- Moves to or from "You" (the mic) are not offered: detection uses that label to tell mic
  audio apart (my call).
- Picking a person moves only that line: its `speakerId` changes in `transcript.jsonl`. If the
  person isn't a speaker in the meeting yet they're added to `m.speakers`; if the old speaker
  has no lines left they're dropped from `m.speakers`.
- "New person: <name>" adds them to the voice registry with no voiceprint (id
  `{meeting}-p{n}`, as `name_speaker` does for "them"), so naming a voice with that name later
  merges instead of duplicating (my call). Existing names match a known person, like
  `name_speaker`.
- Voiceprints are **not** touched by moving a line, even the last line of a voice: its sample
  for this meeting stays in `voices.json`. To say "Speaker 3 is Ana", name the voice instead
  (my call: one line is too short to teach a
  voice, and a wrong click shouldn't pollute it). Mark with a `ponytail:` comment.
- Moves survive "Re-run speaker detection" (user's call). A moved line carries a saved
  "speaker set by hand" mark (new `Segment` field in `mirona-core`, default false, so old
  transcripts load; regenerate bindings), and `diarize.rs` leaves marked lines alone, the way
  it already skips mic lines. Test: move a line, re-run assignment, the line keeps its person.
- The summary, action-item owners and search index are not regenerated; a line's speaker
  shows wherever segments are read. Owners stay as they were.
- New store fn + Tauri command, e.g. `Store::set_line_speaker(meeting_id, segment_id,
  person_id_or_new_name)`, then the usual `api.ts` wrapper and `store.ts` action. Works on
  encrypted meetings (transcript lines are written through the same path as other segment
  edits). Unit test in `mirona-store`: move a line to an existing person, to a new name, and
  the "last line of a speaker" case.
- Also exposed nowhere else (no MCP/CLI tool) for now.

## 2. Tags

### No descriptions

- Settings › Tags becomes a plain list of tags (add, rename, remove). The "Groups" row with
  descriptions is removed: a group is just the part before `:`.
- The classifier's tag question (`questions.rs::tag_question`) no longer gets an `about` or a
  group description; it uses the tag name and group name only. Accepted cost: tags need clear
  names. `mirona eval` doesn't exercise `tag_question` (it uses the old client question,
  left as is), so no new eval; update the `questions.rs` unit tests (my call).
- Config: existing `tags` descriptions and `tag_groups` are dropped. Builder's call on shape,
  but old `config.toml` files must keep loading (a `tags` table with descriptions, a
  `tag_groups` table, and the older `[clients]` migration). Simplest is likely to keep reading
  them and write tags with empty descriptions / omit `tag_groups` on save.
- `propose_tag` (LLM suggests one new tag) keeps working; its description, if any, is no
  longer stored.
- Update `docs/user-guide.md` and the CLAUDE.md note on `tags`/`tag_groups`.

### Colour by group

- Every tag chip is coloured by its group: all `project:*` share one colour, all `team:*`
  another. Tags with no group stay the current neutral chip.
- Applies everywhere tag chips or tag text appear as chips: inspector, meeting list
  (`Pages.tsx`), sidebar search filters (`Shell.tsx`), Settings › Tags, retention list.
  Plain-text tag lists (e.g. "project:setr · Tue" subtitles) can stay text.
- Colour is picked automatically (user's call): a stable hash of the group name into a fixed
  palette, so the same group is the same colour on every launch with nothing stored.
- Palette: new tokens `--mi-tag-1 … --mi-tag-8`, each a text + tint pair, in the existing
  style (muted, like `--mi-decision`/`--mi-decision-tint`). They must be visibly different from
  the semantic chips (action teal, decision indigo, question amber, review, sensitive red) so a
  tag never reads as a line kind — e.g. plum, rose, olive, slate blue, ochre-brown, moss,
  cyan-grey, mauve. Defined for light and `[data-theme="dark"]`, in
  `design-kit/tokens/tokens.css` + `tokens.json` and copied byte-identical to
  `app/src/tokens.css`; `check-contrast.py` covers them (≥ 4.5:1 text on tint). One chip class
  per slot, `.mi-chip--tag-N`.
- More groups than colours: they repeat. Fine.

### Autocomplete

- Replace the `<datalist>` in the inspector's tag field with a small listbox under the field:
  arrow keys, Enter to pick, Escape to close, mouse click; ARIA combobox roles; focus ring.
- Matching is not prefix-only: case-insensitive; rank exact > prefix of the tag or of the name
  after `:` > substring > subsequence (letters in order, e.g. "pset" → `project:setr`). Show
  up to ~8. Hide tags already on the meeting. Enter with no highlighted suggestion adds the
  typed text as a new tag (today's behaviour).
- Suggestion rows show the tag in its group colour.
- One shared component + one small match function (with a small test or self-check) used by
  both the tag field and the person picker in section 1. No new dependency.

## Non-goals

- No manual colour picker per group.
- No re-training of voiceprints from single-line moves; no bulk "select several lines and move".
- No MCP/CLI changes beyond what falls out of the config change.
- No change to how diarization itself splits voices.

## Left to the builder

Exact component names and file split, the anonymous-speaker test, the config serde shape, the
hash function, the palette hex values (within the constraints above), sheet styling within the
design kit (glass card, 44 px main actions, focus rings, reduced transparency).

# Naming voices, moving lines, and tag colours — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** After speaker detection, walk the user through naming unnamed voices with sample clips; let any line be moved to another person (kept across re-detection); simplify tags (no descriptions), colour them by group, and give the tag field fuzzy autocomplete.

**Architecture:** Two new store operations (`unnamed_speakers`, `set_line_speaker`) and one new `Segment` field (`by_hand`) in Rust, exposed as Tauri commands. One shared React combobox + fuzzy matcher drives the person picker and the tag field. Tag descriptions are removed end to end; group colour is a pure function of the group name over 8 new design tokens.

**Tech Stack:** Rust 2024 workspace (mirona-core / -store / -engine / -providers), Tauri 2, React 19 + Tailwind v4 + zustand, ts-rs bindings.

**Spec:** `docs/superpowers/specs/2026-11-10-voices-and-tags-design.md` (read it first).

## Global Constraints

- CI gates: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` (pedantic), `cargo test --workspace`, `pnpm build`, `pnpm lint` (from `app/`), `python3 design-kit/tokens/check-contrast.py`.
- `app/src/tokens.css` stays byte-identical to `design-kit/tokens/tokens.css` (CI diffs them).
- After changing a `mirona-core` type, run `cargo test -p mirona-core` and commit the regenerated `app/src/bindings/*.ts`; never hand-edit bindings.
- New Tauri command = Rust fn → `generate_handler!` in `app/src-tauri/src/lib.rs` → wrapper in `app/src/lib/api.ts` (camelCase args) → action in `app/src/store.ts`.
- Errors are `Result<T, String>`; user-facing error strings are plain sentences.
- Accessibility: ≥ 4.5:1 text contrast, focus ring on every control, 44 px main actions in the sheet, ARIA combobox/listbox roles on the picker.
- "You" (`speaker_id == "me"`) is never listed as unnamed, never a move source, never a move target.
- No new dependencies (Rust or npm).
- Commit messages: plain sentences in the repo's style. No AI attribution, co-author lines or session links (user rule).
- Update `docs/user-guide.md` and the `tags`/`tag_groups` note in `CLAUDE.md` where behaviour changes.

## Review Focus

1. **Encrypted meetings:** moving a line must round-trip through the sealed transcript like any other edit. Test `set_line_speaker` on a store opened with a key (Task 1).
2. **Old configs:** a `config.toml` with described `[tags]`, a `[tag_groups]` table, and a legacy `[clients]` table must still load, with descriptions emptied. Test it (Task 6).
3. **Re-detection after a move to a brand-new person:** the hand-set line keeps its person, and that person stays in `m.speakers` even though detection never produced them. Test it (Task 2).
4. **Moving the only line of the only remaining voice, and a sensitive line:** the old speaker disappears from `m.speakers`, and sensitive lines are never offered as samples while "Show sensitive text" is off. Test the first (Task 1), check the second in Task 4's matcher-adjacent `samples` self-check.
5. **Typing a name that differs only in case** from a known person ("ana" vs "Ana") in the line picker must reuse Ana, not create a duplicate. Test it (Task 1).

---

### Task 1: `Segment.by_hand`, `Store::set_line_speaker`, `Store::unnamed_speakers`

**Files:**
- Modify: `crates/mirona-core/src/lib.rs` (struct `Segment`, ~line 192)
- Modify: every `Segment { … }` literal the compiler flags (≈21 sites, mostly tests): add `by_hand: false`
- Modify: `crates/mirona-store/src/people.rs` (new fns + tests in its `mod tests`)
- Regenerate: `app/src/bindings/Segment.ts`

**Interfaces:**
- Produces:
  - `Segment.by_hand: bool` — `#[serde(default)]`, doc: "Speaker set by the user; speaker detection leaves it alone." TS: `byHand: boolean`.
  - `Store::set_line_speaker(&mut self, meeting_id: &str, segment_id: &str, name: &str) -> Result<Speaker>` — returns the speaker the line now has.
  - `Store::unnamed_speakers(&self, meeting_id: &str) -> Result<Vec<Id>>` — ids in `m.speakers`, in order, where `id != "me"` and `self.voices().person(id).is_none()`.
  - private `fn new_person(v: &mut Voices, people: &[Speaker], meeting_id: &str, name: &str) -> Id` — the `{meeting_id}-p{n}` allocation lifted out of `name_speaker`'s third match arm; `name_speaker` calls it (no behaviour change).

`set_line_speaker` rules:
- Empty/whitespace name → `Err("A person needs a name.")`. Unknown meeting/segment → `Err`. Segment's current speaker is `"me"` → `Err("Lines from your mic can't be moved.")`.
- Target = `voices().by_name(name)` (case-insensitive) if present, else `new_person(...)` saved with `save_voices` (no clusters). Target id `"me"` → `Err("Lines can't be moved to you.")`.
- Set `speaker_id = target`, `by_hand = true`. If target not in `m.speakers`, push `person_speaker(target)` if known in `self.people`, else `named(target, name)`. If old speaker has no segments left, remove it from `m.speakers`. Do not touch clusters, owners, tags, summary, items.
- `self.save(i, true)?; self.refresh_people();` then return the target `Speaker`.
- Add above it: `// ponytail: a moved line never updates voiceprints (one line is too short, a misclick shouldn't train a voice); NGR-201 if users ask for it.` (NGR-201).

- [ ] **Step 1: Write failing tests** in `people.rs` `mod tests` (reuse `diarized`, `names`):

```rust
#[test]
fn a_line_moves_to_a_known_person_and_the_empty_voice_drops() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(dir.path()).unwrap();
    diarized(&mut s, "m-a", &[("m-a-s1", vec![1.0, 0.0]), ("m-a-s2", vec![0.0, 1.0])]);
    s.name_speaker("m-a", "m-a-s1", "Ana").unwrap();
    let sp = s.set_line_speaker("m-a", "m-a-2", "ana").unwrap(); // case differs
    assert_eq!(sp.id, "m-a-s1");
    assert_eq!(names(&s, "m-a"), ["Me", "Ana", "Ana"]);
    let m = s.meeting("m-a").unwrap();
    assert!(m.segments[2].by_hand);
    assert!(!m.speakers.iter().any(|p| p.id == "m-a-s2"));
    assert_eq!(s.voices().clusters.len(), 2); // voiceprints untouched
}

#[test]
fn a_line_moves_to_a_new_person_who_joins_the_registry() {
    // diarized m-a with one voice; set_line_speaker("m-a", "m-a-1", "Bo")
    // → speaker id starts with "m-a-p", name "Bo", s.voices().people has Bo,
    //   s.unnamed_speakers("m-a") == [] ; a later name_speaker(.., "Bo") on another
    //   meeting's voice resolves to the same id (no duplicate Bo).
}

#[test]
fn mic_lines_and_moves_to_me_are_refused() {
    // set_line_speaker("m-a", "m-a-0", "Ana") is Err; empty name is Err.
}

#[test]
fn unnamed_speakers_lists_unknown_voices_but_not_me() {
    // diarized m-a with s1, s2; name s1 "Ana" → unnamed == ["m-a-s2"].
    // An undiarized meeting with "them" lines → unnamed == ["them"].
}

#[test]
fn moving_a_line_works_on_an_encrypted_store() {
    // Store::open_with(dir, Some(key)) (see crypt.rs tests for how a key is made);
    // move a line; reopen with the same key; the line keeps its speaker and by_hand.
}
```

- [ ] **Step 2:** `cargo test -p mirona-store people` → FAIL (`set_line_speaker` not found).
- [ ] **Step 3:** Add `by_hand` to `Segment`; fix every literal the compiler reports; implement `new_person`, `set_line_speaker`, `unnamed_speakers`; make `name_speaker` use `new_person`.
- [ ] **Step 4:** `cargo test -p mirona-store` → PASS; `cargo test -p mirona-core` regenerates `Segment.ts` with `byHand: boolean`; `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] **Step 5:** Commit: `Lines can be moved to another person (Store::set_line_speaker), marked by_hand; Store::unnamed_speakers lists voices nobody named`.

### Task 2: Speaker detection keeps hand-set lines

**Files:**
- Modify: `crates/mirona-engine/src/diarize.rs` (`apply_diarization`, ~line 379; tests at the bottom)

**Interfaces:**
- Consumes: `Segment.by_hand`, `Store::set_line_speaker` (Task 1).

- [ ] **Step 1: Failing test** next to the existing `apply_diarization` tests (reuse their `meeting`, `segs`, `d`, `names` helpers):

```rust
#[test]
fn rerunning_detection_keeps_lines_moved_by_hand() {
    // apply_diarization("m-a", &d(..)) as in the first test → names
    //   ["Me", "Speaker 1", "Speaker 1", "Speaker 2", "Me"].
    // store.set_line_speaker("m-a", <id of segment 1>, "Zoe") (a new person).
    // apply_diarization("m-a", &d(..)) again.
    // names(&e, "m-a")[1] == "Zoe"; segment 1 still by_hand;
    // meeting.speakers contains Zoe's id.
}
```

- [ ] **Step 2:** `cargo test -p mirona-engine rerunning_detection` → FAIL (segment 1 back to "Speaker 1").
- [ ] **Step 3:** In `apply_diarization`, right after `let labels = assign(...)`, make it `mut` and set `labels[i] = None` for every segment with `by_hand`. (`set_diarization` already keeps speakers still used by some segment, so Zoe stays.) Update `assign`'s doc comment: "hand-set lines keep their speaker".
- [ ] **Step 4:** `cargo test -p mirona-engine` → PASS.
- [ ] **Step 5:** Commit: `Re-running speaker detection keeps lines the user moved`.

### Task 3: Commands, API wrappers, store actions

**Files:**
- Modify: `app/src-tauri/src/lib.rs` (next to `name_speaker`, ~line 245; `generate_handler!` ~line 2007)
- Modify: `app/src/lib/api.ts`, `app/src/store.ts`

**Interfaces:**
- Consumes: Task 1 store fns.
- Produces:
  - Tauri `set_line_speaker(meeting_id, segment_id, name) -> Res<Speaker>`, `unnamed_speakers(meeting_id) -> Res<Vec<String>>` (lock store, call through, like `name_speaker`).
  - `api.setLineSpeaker(meetingId: string, segmentId: string, name: string): Promise<Speaker>`
  - `api.unnamedSpeakers(meetingId: string): Promise<string[]>`
  - store action `moveLine(segmentId: string, name: string): Promise<void>` — like `nameSpeaker`: `run(...)`, call api, `afterFix()`, flash `Moved the line to ${p.name}.`

- [ ] **Step 1:** Add both commands and register them.
- [ ] **Step 2:** Add the wrappers and `moveLine` (type in the store interface next to `nameSpeaker`).
- [ ] **Step 3:** `cargo clippy --workspace --all-targets -- -D warnings` and `cd app && pnpm build && pnpm lint` → clean.
- [ ] **Step 4:** Commit: `Commands to move a line and list unnamed voices`.

### Task 4: Fuzzy matcher and shared `Combobox`

**Files:**
- Create: `app/src/lib/match.ts`
- Create: `app/src/lib/match.check.ts` (self-check, not bundled)
- Create: `app/src/components/Combobox.tsx`

**Interfaces:**
- Produces:
  - `rank(query: string, items: string[], limit = 8): string[]` in `match.ts`. Case-insensitive. Score buckets, best first: exact (0) > prefix of the whole item or of the part after `:` (1) > substring (2) > subsequence (3); no match → dropped. Ties: shorter item, then alphabetical. Empty query → first `limit` items in given order.
  - `samples(segs: Segment[], speakerId: string, showSensitive: boolean): Segment[]` in `match.ts` too (keeps the pure helpers together; it's the spec's "clearest lines" rule): that speaker's lines, skip `is sensitive` ones unless `showSensitive` (sensitive = `seg.redacted || seg.tags.some(t => t.kind === "sensitive")`, the test `SegmentRow` uses at `Meeting.tsx:492`; export one `isHidden` helper from `match.ts` and use it in both places), length = `endMs - startMs` or `text.length * 60` when `endMs == 0`, drop under 2000 ms unless that leaves none, pick greedily by length skipping any within 60 000 ms of one already picked, max 3, return sorted by `startMs`.
  - `<Combobox label: string; placeholder: string; options: { value: string; group?: string; render?: ReactNode }[]; onPick: (value: string) => void; allowNew?: (typed: string) => string | null; initial?: string />` — input + listbox under it. Filters `options` by `rank` over `value`, preserving `group` headings (e.g. "In this meeting" / "Everyone else") in original group order. Keys: ↓/↑ move, Enter picks highlighted (or, with nothing highlighted, the typed text when `allowNew` returns a label), Escape closes, Tab closes. Mouse click picks. Roles: `role="combobox"`, `aria-expanded`, `aria-controls`, `aria-activedescendant`, `role="listbox"`/`"option"`. Input uses `mi-field`; listbox is a glass card (`mi-*` card class used by other popovers in `ui.tsx`). When `allowNew` returns a label, the last option reads `New person: Zoe` (label is the caller's).

- [ ] **Step 1: Write the self-check** `match.check.ts`:

```ts
import assert from "node:assert/strict";
import { rank } from "./match.ts";
const tags = ["project:setr", "project:atlas", "team:design", "setup", "hiring"];
assert.deepEqual(rank("setr", tags)[0], "project:setr");      // prefix after ':'
assert.deepEqual(rank("set", tags).slice(0, 2), ["setup", "project:setr"]);
assert.deepEqual(rank("pset", tags), ["project:setr"]);       // subsequence
assert.deepEqual(rank("SIGN", tags), ["team:design"]);         // case, substring
assert.deepEqual(rank("zzz", tags), []);
assert.equal(rank("", tags, 2).length, 2);
console.log("match ok");
```
Plus three `samples` asserts: picks ≤ 3, skips a sensitive line when `showSensitive` is false, spreads picks ≥ 60 s apart.

- [ ] **Step 2:** `node --experimental-strip-types app/src/lib/match.check.ts` → FAIL (module missing).
- [ ] **Step 3:** Implement `match.ts` (import types with `import type`, `.ts` extension imports only in the check file) and `Combobox.tsx`.
- [ ] **Step 4:** Self-check prints `match ok`; `pnpm build && pnpm lint` clean. Make sure `match.check.ts` isn't pulled into the Vite bundle (nothing imports it) and passes Biome.
- [ ] **Step 5:** Commit: `Fuzzy matcher and a shared combobox`.

### Task 5: "Name the voices" banner + sheet, and "Who said this?" on each line

**Files:**
- Create: `app/src/views/NameVoices.tsx` (banner + sheet)
- Modify: `app/src/views/Meeting.tsx` (render banner above the transcript list, near the live-note at ~line 335; add the line control in `SegmentRow` ~line 481 using the `reveal` class)

**Interfaces:**
- Consumes: `api.unnamedSpeakers`, `api.listPeople`, store `nameSpeaker`, store `moveLine` (Task 3); `Combobox`, `rank`, `samples` (Task 4); `useAudio(m).play` from `Meeting.tsx` (pass `play` down as a prop).
- Produces: `<NameVoices m: Meeting; play?: (ms: number) => void; showSensitive: boolean />`.

Behaviour (copy from the spec, exact strings):
- Fetch `unnamedSpeakers(m.id)` in an effect keyed on `m` (it reloads after every fix). Hidden while this meeting is recording or in `processing`. Nothing unnamed → render nothing.
- Banner: `${n} voice${n === 1 ? "" : "s"} aren't named yet.` (use "isn't" for 1) + button **Name them**. Review-tint styling (`--mi-review-tint` / dashed `--mi-review-edge`, like "To check").
- Sheet (inline glass card under the banner, not a modal): heading `Voice ${k} of ${n} — ${sp.name}`, sub `${lines} lines · ${talk}` (talk time from `endMs - startMs` summed, `m:ss`), then `samples(...)` each as a row: play button (`aria-label="Play line at 04:12"`, disabled without audio) + timestamp + text. Then a `Combobox` labelled `Name or pick a person…`, options = named speakers of this meeting (group "In this meeting") then the rest of `listPeople()` whose id isn't in the unnamed list (group "Everyone else"), `allowNew = t => t.trim() ? \`New person: ${t.trim()}\` : null`, `initial = sp.suggestion?.name`. Picking calls `nameSpeaker(sp.id, value)` and advances. Buttons: **Skip**, **Next ›**, **Close** (44 px). After the last voice, close.
- Line control in `SegmentRow`: a small "Who said this?" button (`reveal`), hidden for `seg.speakerId === "me"` and during recording. Opens a `Combobox` popover: options = named people of this meeting then everyone else (same grouping, excluding unnamed voices and "me"), `allowNew` as above; pick → `moveLine(seg.id, value)`.

- [ ] **Step 1:** Build `NameVoices.tsx` and wire it into `Meeting.tsx`.
- [ ] **Step 2:** Add the line control.
- [ ] **Step 3:** `pnpm build && pnpm lint` clean.
- [ ] **Step 4: Manual check** (`pnpm tauri dev`, Settings › "Add demo meetings" or an imported clip with detection run): banner shows with the right count; sheet plays a sample; naming advances and the count drops; Skip keeps the banner; moving a line updates the transcript; "Re-run speaker detection" keeps the moved line; keyboard-only run-through works with visible focus rings.
- [ ] **Step 5:** Update `docs/user-guide.md` (speakers section: naming voices after the call, moving a line, moves survive re-detection). Commit: `Name unnamed voices from a banner after detection, with sample lines; move any line to another person`.

### Task 6: Tags without descriptions

**Files:**
- Modify: `crates/mirona-providers/src/questions.rs` (`tag_question`, `tag_questions`, `meeting_questions`, their tests)
- Modify: `crates/mirona-providers/src/config.rs` (`tags` doc, remove `tag_groups` field + its default + the `PROJECT_GROUP` description insert in migrate; empty descriptions on load; tests)
- Modify: `crates/mirona-engine/src/classify.rs` (calls at ~498 and ~754), `crates/mirona-engine/src/lib.rs` (`add_tag` drops `about`; `rename_tag` keeps an empty value), and every `add_tag` caller (Tauri / MCP — `rg -n "add_tag\("`)
- Modify: `app/src/lib/api.ts` (`Settings.tagGroups` removed; `tags` stays `Record<string, string>`), `app/src/views/Settings.tsx` (`Tags`, `Described` removed if unused)
- Modify: `CLAUDE.md` (the `tags`/`tag_groups` sentence), `docs/user-guide.md`

**Interfaces:**
- Produces: `tag_question(tag: &str) -> Question`; `tag_questions(tags: &BTreeMap<String, String>) -> Vec<Question>` (keys only); `meeting_questions(tags: &BTreeMap<String, String>)`. `Config` has no `tag_groups`. `Engine::add_tag(&self, app_dir: &Path, tag: &str) -> Result<()>`.
- Config shape stays `tags: BTreeMap<String, String>` so old files parse; values are always `""` after load (`migrate` clears them). `Config` has no `deny_unknown_fields`, so an old `[tag_groups]` table is ignored by serde. Callers of `add_tag`: `app/src-tauri/src/lib.rs:323` and `:370` (drop the `about` argument; the `about` computed for :370 goes away).

`tag_question` text with no descriptions: `yes = "The meeting discusses {name}{group}. A meeting can be about several topics; a real part of it is enough."` where `group` is ` (a {g})` or empty — i.e. today's text with the `about` parts removed.

- [ ] **Step 1: Failing tests** in `config.rs`:

```rust
#[test]
fn old_described_tags_load_with_descriptions_dropped() {
    let toml = r#"
[tags]
"project:setr" = "Setr app work"
[tag_groups]
project = "Clients"
[clients]
Atlas = "Atlas Corp"
"#;
    let cfg: Config = /* parse the same way Config::load does, then migrate */;
    assert_eq!(cfg.tags.get("project:setr").map(String::as_str), Some(""));
    assert_eq!(cfg.tags.get("project:atlas").map(String::as_str), Some(""));
}
```
and in `questions.rs`: `tag_question("project:setr")` yields `id == "tag:project:setr"` (use `TAG_PREFIX`), and its `yes` text contains `setr (a project)` and no `:` after it.
- [ ] **Step 2:** `cargo test -p mirona-providers` → FAIL (signature mismatch / description kept).
- [ ] **Step 3:** Make the Rust changes above; fix callers until `cargo clippy --workspace --all-targets -- -D warnings` is clean.
- [ ] **Step 4:** Settings › Tags: replace `Described` with a one-column list (name input, Remove, Add tag; rename on blur through the existing `api.renameTag`), saving `tags` as `{ name: "" }`. Row sub: `A meeting can have several. The classifier asks about each one after every call, so name them clearly.` Delete the Groups row. Remove `Described`/`toRows`/`Row` type if nothing else uses them.
- [ ] **Step 5:** `cargo test --workspace`, `cargo run -p mirona-cli -- eval --with fake`, `pnpm build && pnpm lint` → all pass.
- [ ] **Step 6:** Update `CLAUDE.md` (`tags` = list of names, `group:name` or `name`; no `tag_groups`) and the user guide. Commit: `Tags have no descriptions: the classifier asks by tag name, Settings lists names only, old descriptions are dropped on load`.

### Task 7: Group colours and tag autocomplete

**Files:**
- Modify: `design-kit/tokens/tokens.css`, `design-kit/tokens/tokens.json`, then copy to `app/src/tokens.css`
- Modify: `design-kit/tokens/check-contrast.py` (`CHIPS` += the 8 tag pairs)
- Modify: `app/src/lib/api.ts` (add `tagClass`), `app/src/views/Inspector.tsx` (tag chips + replace `<datalist>` with `Combobox`), `app/src/views/Pages.tsx` (~line 151: chips instead of `join(", ")`), `app/src/views/Settings.tsx` (tag list and retention list show the tag as a chip)

**Interfaces:**
- Consumes: `Combobox`, `rank` (Task 4).
- Produces:
  - Tokens `--mi-tag-1 … --mi-tag-8` (text) and `--mi-tag-1-tint … --mi-tag-8-tint`, light in `:root`, dark in `[data-theme="dark"]`; classes `.mi-chip--tag-1 … .mi-chip--tag-8 { background: var(--mi-tag-N-tint); color: var(--mi-tag-N); }` next to the other `.mi-chip--*`.
  - `tagClass(tag: string): string` → `"mi-chip--neutral"` when the tag has no `:`; else `mi-chip--tag-${1 + (hash(group) % 8)}` with `hash` = FNV-1a 32-bit over the lowercased group. Pure, stable across launches.

Palette constraints (pick hex values): plum, rose, olive, slate blue, ochre-brown, moss, cyan-grey, mauve. Muted like `--mi-decision`/`--mi-decision-tint`, and visibly different from accent teal `#0E7466`, decision indigo `#2F3C99`, question amber `#7A4800`, sensitive red `#9B2C22`. Every text/tint pair ≥ 4.5:1 in both themes (the checker enforces it).

- [ ] **Step 1:** Add the 8 pairs to `CHIPS` in `check-contrast.py`; run `python3 design-kit/tokens/check-contrast.py` → FAIL (`KeyError`, tokens missing).
- [ ] **Step 2:** Add the tokens and chip classes to `design-kit/tokens/tokens.css` (+ `tokens.json` in its existing structure), `cp design-kit/tokens/tokens.css app/src/tokens.css`. Re-run the checker → PASS; `diff design-kit/tokens/tokens.css app/src/tokens.css` → no output.
- [ ] **Step 3:** Add `tagClass` to `api.ts` and add one assert-based line to `match.check.ts`: `tagClass("project:a") === tagClass("Project:b")` and `tagClass("solo") === "mi-chip--neutral"` (import from `./api.ts` only if that file has no Tauri side-effects at import; otherwise put `tagClass` in `match.ts` and re-export from `api.ts`).
- [ ] **Step 4:** Use `tagClass(t)` on every tag chip (Inspector, Pages table cell, Settings tag list, retention list). Native `<select>` options in `Shell.tsx`/`Pages.tsx` stay plain text (options can't be styled).
- [ ] **Step 5:** Inspector tag field: replace the `<input list>` + `<datalist>` with `<Combobox label="Add a tag" placeholder="Add a tag, e.g. project:setr" options={known.map(t => ({ value: t, render: <span className={`mi-chip ${tagClass(t)}`}>{t}</span> }))} allowNew={t => t.trim() ? t.trim() : null} onPick={...existing add logic} />`. Keep today's rule: case-insensitive duplicate is ignored.
- [ ] **Step 6:** `node --experimental-strip-types app/src/lib/match.check.ts`, `pnpm build && pnpm lint`, contrast check → all pass. Manual: `project:*` chips share a colour everywhere; typing `pset` in the tag field offers `project:setr`; Enter on unmatched text adds it as a new tag.
- [ ] **Step 7:** User guide: tags are coloured by group, autocomplete matches anywhere. Commit: `Tags are coloured by group from 8 new palette tokens; the tag field autocompletes on any part of the tag`.

### Task 8: Full CI pass

- [ ] Run the whole CI list from Global Constraints plus the tokens diff. All green.
- [ ] `rg -n "tag_groups|tagGroups"` returns only the old-config test and migrate code.

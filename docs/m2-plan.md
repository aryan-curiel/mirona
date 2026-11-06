# Milestone 2 build plan: who and what

> Historical: the plan as written for milestone 2. Current behaviour is in `docs/user-guide.md` and `docs/architecture.md`.

This is the contract between the people and agents building M2 in parallel. Linear: milestone "2 · Who and what" (NGR-152 to NGR-163). The model choices come from ADR 0001: speakrs, plus Jev pinned to `jev-1.14.0` with qwen3:4b via Ollama as the privacy-mode fallback, and the thresholds recorded there.

Today, in M1, the speaker comes from the audio source: the mic is "me" and system audio is "them". The `Classifier`, `Diarizer`, `speakrs::match_voiceprint`, Jev and Ollama providers exist and are tested, but nothing calls them yet. The UI already has places for tags, filters, the inspector, people, the action items view and the follow-up draft, all from the prototype; some still show "Coming in M2".

Every package must keep these green:

- `cargo fmt --all --check`
- `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets` (default features, macOS, Linux and Windows)
- `cargo test --workspace`
- `cd app && pnpm lint && pnpm build`

Shared files (`crates/mirona-engine/src/lib.rs`, `crates/mirona-store/src/lib.rs`, `crates/mirona-core/src/lib.rs`, `app/src-tauri/src/lib.rs`): make small, local edits only. If an edit fails because the file changed, re-read it and retry. Never overwrite another package's work. Core type changes are additive (`#[serde(default)]`); regenerate the bindings with `cargo test -p mirona-core`.

## Phase 1, in parallel

### WP-A: Classification (NGR-155, 156, 157, 158, 160, 162 backend)

Owns:
- the `Classifier` trait and `jev.rs`, `ollama.rs` and `questions.rs` in `mirona-providers`
- a new `crates/mirona-engine/src/classify.rs`
- the `mirona eval` command in the CLI
- `eval/`

What to build:
- **Typed questions.** Redesign `Classifier` around typed questions: yes/no, choice and score, several per call, against one piece of text, as Jev's API supports natively. Ollama answers the same questions through a JSON schema plus logprobs. Keep a helper for the five segment questions.
- **Live, per segment.** Each segment is classified on a worker as it arrives, without blocking capture. Answers and tags go into the store and come out as a `Classified` event. Owner is a choice among the meeting's speakers; due date is captured when one is said.
- **Per meeting, after the call:**
  - meeting type, which picks the summary template automatically;
  - client, from a client list kept in config/Settings;
  - priority and client tone, as scores;
  - "needs a follow-up email?", and if yes, a follow-up draft written by the LLM from the decisions, action items and open questions, using confirmed names and only items above threshold;
  - a summary quality check ("does it cover every action item?"), which regenerates the summary once if not.
- **Thresholds.** Configurable per question, with defaults from ADR 0001: 0.60, or 0.95 for decision. In privacy mode, use the local fallback's higher defaults. Anything below its threshold is marked "Needs review" and is never filed as fact or sent to agents as fact (MCP results should say so).
- **Provider choice.** Jev by default, with its key from the keychain account `jev`. Privacy mode switches to the local LLM without losing questions. Selected in config.
- **Corrections and eval.** Every correction becomes a labelled example (local only, e.g. `<meetings>/.mirona/labels.jsonl`). `mirona eval` runs the labelled set (`eval/chunks.jsonl` plus your corrections) through the current classifier and the local LLM baseline, and reports accuracy, precision/recall per question, and latency. CI runs it against the public synthetic set in a mode that needs no network (recorded replies, or the fake).
- **Tests:** recorded Jev and Ollama responses for choice and score questions, live `#[ignore]` tests (the key is in `.env` as `TYPESAFE_API_KEY`; Ollama is running), engine tests with a scripted classifier, and eval tests.

### WP-B: Speakers (NGR-152, 153, 154 backend)

Owns:
- `speakrs.rs` in `mirona-providers`
- a new `crates/mirona-engine/src/diarize.rs`
- a new `crates/mirona-store/src/people.rs` (the people/voiceprint registry)
- the speakrs part of `scripts/fetch-models.sh`

What to build:
- **Diarization after the call.** Run speakrs on the recording, decoded to 16 kHz PCM: CoreML on machines with at least 16 GB, CPU otherwise. Re-assign each "them" segment to a speaker cluster by time overlap; mic segments stay "me". Clusters made mostly of mic speech are "me"; drop clusters with less than 20 s of speech (ADR). Overlapping speech must not break segment order. Each segment carries a speaker id; each speaker keeps its averaged embedding, stored privately (not in `meeting.md`; e.g. `<meetings>/.mirona/`). It runs after Stop, before the summary.
- **Voice enrollment.** A local-only registry of known people (name plus averaged voiceprint and meeting count). New speakers are matched by `speakrs::match_voiceprint`: cosine ≥ 0.55 and a margin of ≥ 0.15 over the next best. A match is named automatically; 0.45–0.55 is a suggestion. Unknown voices stay "Speaker N". Fixing a wrong match updates that person's voiceprint.
- **Name suggestions from context** ("thanks, Inés"): a classifier choice question over known people plus "none". Coordinate with WP-A's question API: if it isn't ready, use a simple placeholder and leave a `ponytail:` note.
- **People operations in the store:** name or enroll a speaker from a meeting, merge two people, delete a voiceprint. Renaming updates every segment, the summary and the action-item owners.
- **Tests:** diarization assignment logic with synthetic turns (no models), the registry round-trip, matching, merge and delete, and real-model `#[ignore]` tests on `spikes/speakrs/audio`. Run those locally.

## Phase 2: WP-C, the app (NGR-154 UI, 156 UI, 157 UI, 158 UI, 159, 161, 162 UI)

The inspector with real probabilities and Correct buttons, tags and filters in the transcript, meeting header chips, the review list, the People view with real operations, the action items view (client and status filters, review markers, matching `list_action_items`), the follow-up draft with a copy button, and Settings for the classifier, thresholds, client list and diarization.

## Phase 3: integration (lead)

End-to-end on real audio: record while a multi-speaker clip plays (VoxConverse), checking diarization, names and tags. Then CI and Linear.

Gate 2 (NGR-163) needs at least 200 labelled chunks from your own meetings, so it waits for real use.

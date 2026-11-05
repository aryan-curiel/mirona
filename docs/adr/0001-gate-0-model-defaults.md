# ADR 0001: Gate 0, default models per stage

- Status: accepted
- Date: 2026-11-05
- Issue: NGR-136 (gate 0), based on NGR-128, NGR-129, NGR-130 and NGR-135

## Context

Milestone 0 had to show that the core stages run well enough on an Apple Silicon Mac (M3 Pro, 36 GB) before building the MVP. Kev (a local classifier) was dropped before the spikes; hosted Jev replaces it, so the Kev packaging spike (NGR-131) was canceled. All tests used public audio (LibriSpeech, AMI, VoxConverse) and a 50-chunk synthetic eval set. They should be rerun on real meetings during gate 1.

## Decisions

### Speech-to-text: **Parakeet-TDT-0.6B v2 (int8, sherpa-onnx)** is the default; Whisper (whisper-rs) is the fallback

[Spike write-up](../spikes/ngr-128-parakeet.md)

| | Parakeet | Whisper small.en | Whisper base.en |
| --- | --- | --- | --- |
| Mean WER (clean / other / AMI meeting) | **6.9%** (0.9 / 3.1 / 16.6) | 9.5% | 11.3% |
| Time per 5 s segment, p50 / p95 | 197 / 490 ms | 268 / 349 ms (Metal) | 102 / 155 ms (Metal) |
| Peak memory while live | 1.25 GB | 0.80 GB | 0.38 GB |
| Model on disk | 661 MB | 488 MB | 148 MB |

- Cut segments with Silero VAD: at most 5 s, at least 0.25 s of silence between them. Cutting at fixed intervals costs 3–14 WER points.
- Process audio one segment at a time in a single worker, off the async executor. Never decode a whole recording at once: memory doubles.
- Whisper stays for low-RAM machines, battery saving, and non-English (Parakeet v2 is English-only; test v3 later).
- Parakeet runs on the CPU. The prebuilt sherpa-onnx has no CoreML support; a GPU build is an optimization for later.

### Diarization: **speakrs** (pyannote community-1 in Rust); pyannote-rs is rejected

[Spike write-up](../spikes/ngr-129-speakrs.md)

- DER of 5.7% overall with a ±0.25 s collar. Most clips are between 0.4% and 2.1%; AMI meetings score 6–12%, mostly from missed speech, with at most 1.9% speaker confusion.
- Speed: about 200× real time with CoreML (a 39-minute meeting in about 10 s), and 13–17× on the CPU.
- **Memory risk:** CoreML peaks at 7–13 GB (macOS footprint) whatever the audio length. Use CoreML only on machines with ≥ 16 GB; use the CPU otherwise.
- Run it once, after the call, inside `spawn_blocking`. Drop or merge clusters with less than ~20 s of speech, because they're spurious "extra speakers".
- **Voiceprints:** match on cosine ≥ 0.55 and a margin of ≥ 0.15 over the second-best person. Treat 0.45–0.55 as a suggestion. Enrollment needs ≥ 30 s of speech and **always runs in CPU mode**, because CoreML embeddings are wrong on 10–20 s clips.
- pyannote-rs 0.3.4 is out: it doesn't build as published, and its DER was 96.8% once fixed.

### Classification: **Jev (`jev-1.14.0`, pinned)** by default; **local LLM (qwen3:4b via Ollama, JSON schema)** in privacy mode

[Spike write-up](../spikes/ngr-130-jev.md)

| | Jev | qwen3:4b (local) |
| --- | --- | --- |
| Mean accuracy, 7 questions | **0.94** | 0.87 |
| `sensitive` recall | **1.00** | 0.55 |
| `action_item` P / R | 0.95 / 1.00 | 0.77 / 0.81 |
| Latency p50 / p95 per chunk (all questions) | 190 / 324 ms | 1.8 / 2.8 s |

- Thresholds: keep **0.60** as the default. Use **0.95 for `decision`**, which Jev over-calls (precision 0.73). Ask `meeting_type` once per meeting, not per chunk.
- The privacy-mode fallback is acceptable with conditions:
  - `meeting_type` and `client` are decided for the whole meeting and confirmed by the user
  - `sensitive` is backed by a deterministic pattern pass (credentials, pay, health), part of NGR-166
  - `action_item` and `open_question` use higher thresholds (0.99 and 0.95)
- The UI should say that tags are less accurate in privacy mode.
- Data: transcript text goes to TypeSafe (US). They don't train on it, but retention has no fixed period. Privacy mode blocks Jev, and the privacy policy must say this.

### License: **Apache-2.0**

See [licenses.md](../licenses.md). Large models (Parakeet, and speakrs CoreML) are downloaded on first run, pinned and checksummed. Small permissive ones (the speakrs CPU set at 33 MB, Silero VAD) may be bundled. CC-BY credits go in `NOTICE` and on an About → Models screen.

## Consequences

- `Diarizer` now returns turns with times plus one averaged embedding per speaker, and has a `voiceprint()` for enrollment clips. The old per-chunk `Vec<u32>` loses everything voice enrollment needs. `Error` gains `Provider(String)` for runtime failures. Both changes are made in `mirona-providers` alongside this ADR.
- NGR-138 (VAD): Silero, 5 s maximum, 0.25 s minimum silence.
- NGR-139 (STT): implement with sherpa-onnx using Parakeet, plus whisper-rs.
- NGR-152/153 (diarization, enrollment): speakrs, with the thresholds above.
- NGR-155/158 (classifier, thresholds): Jev pinned with the thresholds above, and qwen3:4b as the fallback.
- Rerun all three spikes on the user's own meetings during gate 1 (NGR-151). The numbers here come from public or synthetic data.

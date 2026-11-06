# NGR-129 — Spike: speakrs diarization on macOS (CoreML)

> Historical: the spike as run in November 2026. Current models and providers are in `docs/providers.md` and `docs/licenses.md`.

**Date:** 2026-11-05 · **Machine:** Apple M3 Pro (5P+6E), 36 GB, macOS 27 · **Crate:** `speakrs = "=0.7.0"`
(`coreml` + `cpu` features) · **Code:** `spikes/speakrs/` (reproduce steps in its README)

## TL;DR

- **Use speakrs.** It gets every VoxConverse clip's speaker count right (5 of 6) with **0.4–2.1 % DER**
  (±0.25 s collar) and runs at **~200x real time on CoreML** (a 39 min meeting in ~10.5 s).
  pyannote-rs 0.3.4 returned **no segments at all for 5 of the 8 clips** (97 % DER). It is not a real option.
- **Embeddings are exposed and work as voiceprints.** The same person in two different meetings
  scores cosine **0.92–0.97**. Two different people score **≤ 0.40**. A 30 s clean enrollment clip
  scores 0.89–0.92 against the person's meeting centroid. **Suggested match threshold: 0.55**, plus a
  margin rule (details below).
- **Three risks to track:**
  1. CoreML peak memory footprint is **7–13 GB** (RSS 2–3.5 GB). CPU mode stays at **0.35–0.6 GB**.
  2. CoreML embeddings are **wrong for inputs of ~10–20 s**. Compute short enrollment clips in CPU mode.
  3. Long meetings get **1–4 extra tiny clusters** (11–21 s each). Filter them or fold them into a
     bigger cluster.
- **The `Diarizer` trait must change.** `Vec<u32>` per chunk drops the turn boundaries, overlap and
  embeddings. Return turns (start/end/speaker) plus one centroid embedding per speaker.

## Test data

The audio is public and the reference labels are human-made. No synthetic audio was needed.

| Clip | Source | Length | Ref. speakers | Notes |
|---|---|---:|---:|---|
| crixb, qvtia | VoxConverse dev | 300 s, 365 s | 2, 2 | broadcast / YouTube debate audio |
| houcx, bspxd | VoxConverse dev | 308 s, 336 s | 3, 3 | |
| dhorc, qfdpp | VoxConverse dev | 303 s, 325 s | 4, 4 | |
| ES2004a | AMI, headset mix | 1049 s | 4 | real meeting; same 4 people as ES2004b |
| ES2004b | AMI, headset mix | 2345 s | 4 | |
| ES2004a far | AMI, Array1-01 (single far-field mic) | 1049 s | 4 | closest to a laptop mic |

- VoxConverse RTTMs come from `joonson/voxconverse`. The 6 WAVs were range-extracted from the
  official 2 GB dev zip (`scripts/remote_unzip.py`), so only ~60 MB was downloaded.
- AMI RTTMs are pyannote's `AMI-diarization-setup/only_words`.

**DER method:** `pyannote.metrics` `DiarizationErrorRate`, with overlap scored and optimal speaker mapping.

- Reported at collar 0 and at the standard md-eval **±0.25 s** collar (pyannote `collar=0.5`).
- The UEM is the union of the reference and hypothesis extents. See `scripts/der.py`.

## Accuracy (DER)

`coreml` mode. The `cpu` and `coreml-fast` columns show the ±0.25 s DER.

| Clip | Ref spk | Found | DER c=0 | **DER ±0.25 s** | miss | FA | confusion | cpu | coreml-fast (found) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| crixb | 2 | **2** | 6.6 % | **1.6 %** | 0.9 % | 0.2 % | 0.5 % | 1.6 % | 1.6 % (2) |
| qvtia | 2 | 3 | 21.5 % | **15.2 %** | 0.9 % | 14.3 % | 0.0 % | 15.2 % | 14.7 % (3) |
| houcx | 3 | **3** | 5.3 % | **1.1 %** | 1.0 % | 0.1 % | 0.0 % | 1.1 % | 1.7 % (4) |
| bspxd | 3 | **3** | 6.1 % | **1.8 %** | 0.5 % | 0.2 % | 1.2 % | 1.8 % | 2.1 % (3) |
| dhorc | 4 | **4** | 1.9 % | **0.4 %** | 0.4 % | 0.0 % | 0.0 % | 0.4 % | 0.4 % (4) |
| qfdpp | 4 | **4** | 2.1 % | **0.7 %** | 0.0 % | 0.3 % | 0.4 % | 0.7 % | 0.6 % (4) |
| ES2004a (AMI) | 4 | 5 | 19.2 % | **11.7 %** | 8.3 % | 1.5 % | 1.9 % | 11.2 % | 11.7 % (5) |
| ES2004b (AMI) | 4 | 6 | 12.7 % | **6.1 %** | 3.0 % | 1.8 % | 1.3 % | 6.1 % | 6.2 % (8) |
| ES2004a far-field | 4 | **4** | 25.1 % | **17.5 %** | 12.2 % | 3.0 % | 2.3 % | – | – |
| **Total (first 8)** | | | 11.7 % | **5.7 %** | | | | 5.6 % | 5.8 % |

**Notes**

- **Turns are right on 2–4 speaker conversations.**
  - Speaker confusion is ≤ 1.9 % on every clip.
  - Most of the remaining error is missed speech in AMI meetings, where backchannels and overlap are
    common. It is not wrong attribution.
- **qvtia:** the 15 % comes from false alarms. 14.5 s went to a third cluster, and the audio that
  speakrs calls speech is marked non-speech in the reference (probably laughter, applause or music).
  Confusion is 0 %.
- **Extra speakers are small spurious clusters.** In coreml mode:
  - ES2004a has one extra cluster of 13 s.
  - ES2004b has two extra clusters, of 11 s and 21 s.
  - qvtia's extra cluster is 14.5 s.
  - coreml-fast found up to 8 clusters on ES2004b.

  **Mirona needs a post-filter.** Drop or merge clusters under ~20 s of total speech, or under ~2 %
  of the meeting. Otherwise "Speaker 5" chips will appear for nobody.
- **Far-field mic:** speaker count was exactly right (4). DER rose to 17.5 %, mostly from missed
  speech (12.2 %). Confusion stayed low (2.3 %).
- **CPU and CoreML give the same answer.** Meeting-level centroids from the two backends have cosine
  ≥ 0.999. DER differs by ≤ 0.5 pt.
- **No speaker-count hint:** speakrs has no `min_speakers`/`max_speakers` option. Only the
  AHC/VBx thresholds can be tuned.

## Runtime (RTF), memory, model size

**Caveat:** another agent's Parakeet benchmark was running on the same machine for the whole run
(load average 12–20). All numbers are under contention. Idle numbers should be at least as good,
and CPU mode is the most likely to improve.

| Mode | 5 min clip | 17.5 min (ES2004a) | 39 min (ES2004b) | RTF (39 min) | Peak RSS | Peak footprint¹ |
|---|---:|---:|---:|---:|---:|---:|
| `coreml` | 1.5 s warm (≈200x); 3.1–3.9 s right after process start | 4.4–6.7 s (≈160–235x) | 9.9–11.0 s (≈215–235x) | **0.0045** | 2.1 GB (5 min) / 3.3 GB (17 min) / 3.5 GB (39 min) | 7.3 / 12.9 / 13.1 GB |
| `coreml-fast` | 0.84–0.9 s (≈350x) | 2.5–3.4 s (≈310–415x) | 4.8 s (≈490x) | **0.0021** | 0.26 / 2.6 / 3.4 GB | 2.0 / 8.9 / 12.3 GB |
| `cpu` (native Rust) | 17.9–20.5 s (≈15x) | 63.8–83.9 s (≈13–16x) | 171 s (≈14x) | **0.073** | 0.35 GB (1 min) / 0.40 GB (10 min) / 0.61 GB (all clips) | ≈ RSS |
| pyannote-rs 0.3.4 (CPU, ORT) | – | – | 4.1 s | (no usable output) | 0.54 GB | 0.53 GB |

¹ The peak footprint comes from macOS `/usr/bin/time -l`. It counts the IOSurface and ANE/GPU
buffers that CoreML maps, so it is real unified-memory pressure, not just address space.

**CoreML memory**

- The footprint is not linear in audio length.
  - 60 s of audio already peaks at 2.5 GB.
  - 600 s peaks at 12.4 GB, about the same as 39 min.
  - This suggests large batched inference buffers (b32/b64 models).
- Switching the embedding compute units to `CpuAndNeuralEngine` or `CpuOnly` did **not** reduce the
  footprint (11.5 GB on ES2004b). It made the run 3–4x slower.
- A 36 GB machine handles this. **On an 8 or 16 GB Mac, 12 GB of peak footprint is a real risk.**
  It needs a fix (upstream issue, or chunking) before CoreML becomes the default for everyone.

**Other timing**

- **Cold start:** the first CoreML run on a fresh machine took 0.9 s to load and 4.1 s for the first
  inference (CoreML compiles and caches for the ANE). Later processes load in 0.13–0.8 s and warm
  up in 0.2–0.5 s. CPU mode loads in about 0.1 s.
- **For context:** a 60-minute meeting takes about **16 s in `coreml`**, **~7 s in `coreml-fast`**
  and **~4 min in `cpu`** (all under load).

**Download / disk size** (HF `avencera/speakrs-models`, pinned revision `67f963b6…`)

| Mode | Files on disk | tar.gz |
|---|---:|---:|
| `cpu` (2 safetensors + PLDA) | **32.7 MB** | 30 MB |
| `coreml` (10 common + 5 chunk-embedding `.mlmodelc` + PLDA) | **272.6 MB** | 241 MB |
| `coreml-fast` | 327.6 MB | – |
| `coreml` + `cpu` (recommended pair) | 305 MB | – |
| everything (`ModelManager::ensure` for all 3 modes) | 502.6 MB | – |

- The release binary is 6.3 MB.
- No ONNX Runtime is needed for `cpu` or `coreml`. Both use native Rust plus objc2 CoreML.
- A clean build takes about 1.5 min.

## Embeddings / voiceprints

**API.** `DiarizationResult` exposes:

- `embeddings: ChunkEmbeddings`: an `Array3<f32>` of shape `[chunk, local_speaker(3), 256]`. These
  are raw WeSpeaker ResNet34 embeddings, before PLDA.
- `hard_clusters: Array2<i32>`: maps `[chunk, local_speaker]` to a global cluster `k`, or −2 when the
  speaker is inactive. Cluster `k` is the same as `SPEAKER_k` in `segments` and the RTTM.

speakrs has no "per-speaker centroid" helper. The spike computes the mean of L2-normalised chunk
embeddings per cluster in about 25 lines (`centroids()` in `spikes/speakrs/src/main.rs`).

**Test 1: same person in two different recordings.** AMI ES2004a and ES2004b are two meetings with
the same 4 people.

- Each hypothesis cluster is mapped to its true identity by maximum time overlap with the reference.
  Cluster purity was 86–96 %.
- The table compares whole-meeting centroids. Rows are ES2004a, columns are ES2004b.

| coreml | FEE013 | FEE016 | MEE014 | MEO015 |
|---|---:|---:|---:|---:|
| FEE013 | **0.937** | 0.314 | 0.100 | 0.149 |
| FEE016 | 0.395 | **0.973** | 0.100 | 0.124 |
| MEE014 | 0.059 | 0.078 | **0.957** | 0.100 |
| MEO015 | 0.176 | 0.065 | 0.215 | **0.921** |

**Test 2: across all conditions.**

| Condition (A vs B = ES2004b meeting centroid) | same person (n=4) | different people (n=12) |
|---|---|---|
| ES2004a meeting centroid (headset), coreml | min **0.921**, mean 0.947 | max **0.395**, mean 0.156 |
| same, cpu | min 0.923, mean 0.947 | max 0.401, mean 0.162 |
| **30 s enrollment clip** (clean single-speaker speech from ES2004a), cpu | min **0.885**, mean 0.902 | max **0.374**, mean 0.100 |
| 30 s enrollment clip, coreml | min 0.888, mean 0.903 | max 0.367, mean 0.097 |
| 10 s enrollment clip, cpu | min **0.552**, mean 0.699 | max 0.278, mean 0.100 |
| 10 s enrollment clip, **coreml** | min 0.009, mean 0.168 ❌ | max 0.099 |
| ES2004a **far-field** meeting centroid, coreml | min **0.688**, mean 0.799 | max **0.428**, mean 0.147 |
| far-field 30 s enrollment, coreml | min 0.654, mean 0.724 | max 0.388, mean 0.091 |
| different clusters within one recording (all 8 clips, clusters ≥ 30 s), coreml | – | n=25, max **0.375**, mean 0.152 |

**Suggested threshold.** Across 37 different-speaker pairs, the highest similarity was 0.43 (two
women, far-field against headset). One exception: in `cpu` mode, a 33 s spurious ES2004b cluster
with 45 % purity scored 0.52 against a real speaker. That is one more reason to drop tiny clusters
before matching. Same-speaker pairs were ≥ 0.65 whenever there was ≥ 30 s of audio
and the mics differed, and ≥ 0.88 when the mics matched.

- **Auto-match at cosine ≥ 0.55.** Also require a **margin of ≥ 0.15 over the second-best
  voiceprint**.
- **Between 0.45 and 0.55, show a suggestion** ("Is this Ana?") and do not auto-assign. This fits the
  existing `NameSuggestion`.
- **Update the voiceprint** after each confirmed meeting with a running mean weighted by speech
  duration (`Voiceprint.meetings` already counts meetings).
- **Require ≥ 30 s of speech for enrollment.** At 10 s, even CPU mode overlaps the margin
  (0.55 vs 0.28).

**Caveat:** this is 4 AMI speakers plus 25 within-recording pairs. The threshold is a starting
point, not a calibrated number. Re-check it on Mirona's own recordings, especially laptop mic
against headset.

**⚠ CoreML short-input bug.** For audio of ~10–20 s, `coreml` embeddings disagree with `cpu` on the
same audio:

| Clip length | cos(coreml, cpu) |
|---|---|
| 10 s | 0.02–0.05 |
| 12 s | 0.14–0.23 |
| 15 s | 0.35–0.52 |
| 20 s | 0.75–0.86 |
| 5 s (zero-padded to one window) | 1.000 |
| long audio | ≥ 0.999 |

The likely cause is edge handling in the CoreML chunk-embedding (`wespeaker-chunk-emb-p1s-w*`) path.
**Compute enrollment voiceprints in CPU mode.** CPU takes ~2 s for 30 s of audio, so this costs
nothing noticeable. The bug is worth reporting upstream.

A side check: averaging only a cluster's first 3 or 10 chunk embeddings (`emb3`/`emb10` in the JSON)
inside a long meeting was also unreliable (same-person min 0.13–0.19). Early chunks of a cluster are
often barely active or overlapped. Voiceprints should use the whole-cluster centroid.

## Licenses: bundle or download?

| Asset | License | Bundle in installer? |
|---|---|---|
| speakrs crate | Apache-2.0 | yes |
| `avencera/speakrs-models` (the converted files speakrs loads) | CC BY 4.0 (repo card). Not gated. | **Yes, with attribution** |
| ↳ derived from `pyannote/segmentation-3.0` | MIT (© CNRS) | yes |
| ↳ derived from `pyannote/wespeaker-voxceleb-resnet34-LM` | CC BY 4.0 | yes, with attribution |
| ↳ PLDA/VBx params from `pyannote/speaker-diarization-community-1` | CC BY 4.0 | yes, with attribution |
| pyannote-rs (comparison only) | MIT. Its CAM++ ONNX is from WeSpeaker (Apache-2.0). | n/a |

- MIT, CC BY 4.0 and Apache-2.0 all allow redistribution in a commercial installer. The obligation
  is attribution: credit pyannote.audio (Hervé Bredin et al.), WeSpeaker and the speakrs conversions,
  link the licenses, and state the changes ("converted to Core ML / safetensors").
- The upstream pyannote HF repos are click-through gated. The speakrs mirror is not, and the
  licenses themselves add no gating requirement.
- **Recommendation: bundle the `cpu` set (33 MB).** Then diarization and enrollment work offline from
  the first launch.
- **Download the `coreml` set (273 MB) on first use** to keep the installer small. Pin
  `MODEL_REVISION` and verify file hashes.
- Use `ModelManager::with_cache_dir(app_data_dir)` or `PipelineBuilder::from_dir(...)`. Do not use
  the default `~/.cache/huggingface` location.

## speakrs vs pyannote-rs

**pyannote-rs 0.3.4 needs a pin to build.** It declares `ort = "2.0.0-rc.10"`, which semver resolves
to rc.15 and fails to compile (24 errors). Pinning `ort = "=2.0.0-rc.10"` fixes the build
(`spikes/speakrs/pyannote-rs-cmp/`).

The comparison followed its `infinite` example (threshold 0.5):

| | speakrs (coreml) | pyannote-rs |
|---|---|---|
| Clips with any output | 8 / 8 | **3 / 8** (5 clips returned zero segments) |
| DER ±0.25 s, total | **5.7 %** | 96.8 % |
| Speakers found on the 2-speaker qvtia | 3 | 15 |
| Overlap / re-clustering | yes (powerset + VBx) | no (online threshold match) |

This matches speakrs' own README: pyannote-rs only emits a segment on a speech-to-silence transition,
which conversational audio rarely has. **Not viable.**

## How it plugs into `Diarizer`

The current trait:

```rust
fn diarize(&self, pcm: &[f32]) -> impl Future<Output = Result<Vec<u32>>> + Send; // speaker per chunk
```

**This shape should change.** speakrs is an offline, whole-recording pass (VBx re-clusters with
global evidence), so Mirona runs it once on stop. That is already where the fake pipeline emits
`diarized`. A per-chunk `Vec<u32>`:

- loses turn boundaries finer than the chunk;
- cannot represent overlapping speech;
- throws away the embeddings voice enrollment needs.

Proposed shape:

```rust
pub struct SpeakerTurn { pub start_ms: u32, pub end_ms: u32, pub speaker: u32 }
pub struct DiarizedSpeaker {
    pub speaker: u32,
    pub speech_ms: u32,          // for the tiny-cluster filter + weighting voiceprint updates
    pub embedding: Vec<f32>,     // 256-d centroid; stays on device, never in TS bindings
}
pub struct Diarization { pub turns: Vec<SpeakerTurn>, pub speakers: Vec<DiarizedSpeaker> }

pub trait Diarizer: Provider {
    /// Whole-recording diarization of 16 kHz mono PCM.
    fn diarize(&self, pcm: &[f32]) -> impl Future<Output = Result<Diarization>> + Send;
    /// Voiceprint for an enrollment clip (>= 30 s of one person).
    fn embed(&self, pcm: &[f32]) -> impl Future<Output = Result<Vec<f32>>> + Send;
}
```

**Implementation notes**

- **Async wrapper:** `OwnedDiarizationPipeline::run(&mut self, &[f32])` is synchronous, CPU/ANE
  heavy and needs `&mut`. Wrap it in `Mutex` + `tokio::task::spawn_blocking`, or use speakrs'
  `into_queued()` worker.
- **Turns:** use `result.exclusive_segments` (no overlap) to assign transcript segments by maximum
  overlap. `segments` keeps overlap if the UI wants it later.
- **Tiny clusters:** filter clusters below ~20 s of speech, or merge them into the nearest centroid,
  inside the provider. Matching against enrolled `Voiceprint`s (cosine ≥ 0.55 with a margin)
  belongs in the store/app layer, not the provider. The provider stays stateless.
- **Mode selection:** use `coreml` for meetings when RAM ≥ 16 GB, with `cpu` as the fallback.
  - Always use `cpu` for `embed()` because of the short-input bug.
  - On 8 GB Macs, use `cpu`: about 4 min per hour of audio after the meeting, at 0.6 GB.
  - Revisit the memory gate after an upstream fix.
- **Types:** `SpeakerTurn` probably belongs in `mirona-core`, behind `ts(export)`. Keep `embedding`
  out of the exported types, in line with the `Voiceprint` doc comment.

## Follow-ups

1. File upstream speakrs issues:
   - CoreML embeddings are wrong for 10–20 s inputs (repro in `spikes/speakrs`, `audio/short`).
   - CoreML peak footprint is 7–13 GB. Ask for a batch-size or memory knob.
2. Re-measure CPU and CoreML RTF on an idle machine and on an 8/16 GB Mac.
3. Re-check the 0.55 threshold on Mirona's own laptop-mic recordings.

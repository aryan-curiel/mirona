# NGR-128 spike: Parakeet-TDT in Rust with live latency

**Recommendation: make Parakeet-TDT-0.6B-v2 (int8, through `sherpa-onnx`) the default speech-to-text model.** Feed it Silero-VAD segments of at most **5 s**. Keep Whisper (whisper-rs) as the fallback for non-English meetings and for low-memory machines.

Parakeet was the most accurate model on every clip. On meeting audio its word error rate was about 5 points lower than Whisper small.en and 9 points lower than base.en. It runs live on the CPU with about 17× headroom: real-time factor 0.06, and a segment comes back in about 0.2 s (p50) and 0.5 s (p95). The costs are memory (about 1.25 GB resident while live, against 0.4–0.8 GB for Whisper) and CPU: four cores busy in short bursts, while Whisper on Metal runs mostly on the GPU.

Code: `spikes/parakeet/` (a standalone cargo project, not in the workspace). Reproduce:

```sh
cd spikes/parakeet && ./fetch.sh && ./bench.sh
```

## Setup

- **Machine:** M3 Pro (5 performance + 6 efficiency cores), 36 GB, macOS. Rust 1.97.1, release build.
- **Parakeet:** `sherpa-onnx` 2.0.1 crate (official k2-fsa wrapper; its build script downloads a static sherpa-onnx + onnxruntime). Model `sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8`, `model_type = "nemo_transducer"`, greedy search, CPU provider, 4 threads.
- **Whisper:** `whisper-rs` 0.16.0 with the `metal` feature, ggml `base.en` and `small.en` (f16), greedy decoding, 4 threads. `WHISPER_CPU=1` sets `use_gpu = false` for the CPU-only rows.
- **VAD:** Silero VAD through `sherpa-onnx` (threshold 0.5, min silence 0.25 s, min speech 0.25 s, `max_speech_duration` = chunk size).
- **Clips:** about 2 minutes each, built from public sets (`fetch_audio.py`). Utterances are joined with 0.5 s of silence.

| Clip | Source | Length | Character |
|---|---|---|---|
| librispeech-clean | LibriSpeech test-clean, first 15 utterances | 127 s | read speech, clean |
| librispeech-other | LibriSpeech test-other, first 20 utterances | 120 s | read speech, harder |
| ami-meeting | AMI ihm test, meeting EN2002a, 32 segments ≥ 3 words, in time order | 122 s | real meeting speech: disfluent, fragments, 4 speakers |

- **WER:** word-level Levenshtein distance divided by the number of reference words. Text is lowercased, punctuation is removed, and hyphens become spaces. Numbers and spelling are not normalised, so the AMI references' British spellings (for example "characterisation") count against every model.
- **Latency:** each chunk is decoded as an independent call, the way an `SttProvider::transcribe(pcm)` call works. Two kinds of chunk:
  - **fixed:** blind N-second cuts.
  - **vad:** Silero segments, force-cut at N seconds. Silero's force cut is not exact, and a few segments reached about 7 s.
- **RTF:** decode time divided by audio time. The timings exclude a 1 s warm-up decode.

> **Measurement caveat.** The machine was shared with other heavy jobs: load average 10–49 on 11 cores, including ollama, a VM and a Metal shader compiler. The Parakeet rows come from the quietest run (load 7–12). Whisper/Metal ran at load 10–49 and Whisper/CPU at load about 12–15. Under heavy load, single chunks sometimes took 1–4 s (a 10–20× spike). Take latency numbers as ±30%. WER is deterministic and was identical across runs.

## Word error rate

Columns: **full** = the whole 2 min clip in one call. **VAD ≤3s / ≤5s** = live-style segments. **fixed 5s** = blind cuts.

| Model | Clip | full | VAD ≤3s | VAD ≤5s | fixed 3s | fixed 5s |
|---|---|---|---|---|---|---|
| **Parakeet TDT 0.6B v2 int8** | librispeech-clean | **0.3%** | **1.2%** | **0.9%** | 14.0% | 3.1% |
| | librispeech-other | **1.3%** | **3.3%** | **3.1%** | 8.7% | 3.8% |
| | ami-meeting | **14.2%** | **15.6%** | **16.6%** | 21.6% | 25.3% |
| Whisper small.en | librispeech-clean | 82.6%* | 2.5% | 1.9% | 11.8% | 10.6% |
| | librispeech-other | 4.8% | 5.6% | 5.9% | 12.5% | 11.2% |
| | ami-meeting | 19.0% | 22.2% | 20.6% | 25.1% | 32.2% |
| Whisper base.en | librispeech-clean | 3.7% | 1.9% | 1.9% | 9.6% | 7.8% |
| | librispeech-other | 3.6% | 7.1% | 6.6% | 14.8% | 8.7% |
| | ami-meeting | 25.6% | 25.6% | 25.3% | 28.2% | 29.3% |

Mean WER at VAD ≤5s: **Parakeet 6.9%**, Whisper small.en 9.5%, base.en 11.3%. Whisper's WER was the same on Metal and on CPU.

\* On the 127 s clean clip, whisper small.en decoded the first ~20 s and then emitted `In the of the French.`, dropping the rest. This is Whisper's known long-form failure, and it happened on every run. Parakeet never truncated or hallucinated in these runs.

Blind fixed cuts split words and cost 3–14 points. Always cut with a VAD.

## Latency and real-time factor

Live mode, VAD segments (mean length 2.7 s for ≤3s, 3.1 s for ≤5s):

| Model / backend | Max chunk | p50 | p95 | max | RTF (live) | Full-file RTF |
|---|---|---|---|---|---|---|
| **Parakeet, CPU 4 threads** | 3 s | 195 ms | 437 ms | 535 ms | 0.059 | 0.080–0.100 |
| **Parakeet, CPU 4 threads** | 5 s | 197 ms | 490 ms | 637 ms | 0.057 | 0.081–0.083 |
| Parakeet, CPU 2 threads (load ~15) | 5 s | 390 ms | 1009 ms | 1599 ms | 0.117 | n/a |
| Whisper base.en, Metal | 5 s | 102 ms | 155 ms | 236 ms | 0.027 | 0.010–0.019 |
| Whisper small.en, Metal | 5 s | 268 ms | 349 ms | 388 ms | 0.070 | 0.012–0.037 |
| Whisper base.en, CPU | 5 s | 298 ms | 625 ms | 998 ms | 0.086 | 0.016–0.045 |
| Whisper small.en, CPU | 5 s | 756 ms | 1518 ms | 2387 ms | 0.221 | 0.049–0.056 |

With fixed 5 s chunks, Parakeet took 355 ms at p50 and 364 ms at p95, so its cost is close to linear in chunk length (about 70 ms per second of audio). Whisper pads every call to a 30 s window, so short chunks cost it nearly a full window each. That is why Whisper is fast on whole files but no faster on 3 s chunks.

Silero VAD itself costs about 0.28 s per 2-minute clip, or 0.2% CPU.

Model load times:
- Parakeet: 0.7–1.2 s.
- Whisper: 7–8 s on the first run, because Metal compiles its shaders. After that, 0.1–0.3 s.

## CPU and memory

Peak memory comes from `/usr/bin/time -l` and covers the whole process, including the VAD. "Live" means VAD and fixed chunks only, with no whole-file decode.

| Model / backend | Peak RSS, live | Peak RSS with a 2-min whole-file decode | CPU while live | Model on disk |
|---|---|---|---|---|
| Parakeet int8, CPU | 1.25 GB | 2.28 GB | ~0.28 core on average; 4 cores busy during each decode | 661 MB (encoder 652 MB) |
| Whisper base.en, Metal | 0.38 GB | 0.45 GB | ~0.01 core (the GPU does the work) | 148 MB |
| Whisper small.en, Metal | 0.80 GB | 0.87 GB | ~0.01 core plus GPU | 488 MB |
| Whisper base.en, CPU | n/a | 0.64 GB | ~0.16 core | 148 MB |
| Whisper small.en, CPU | n/a | 1.16 GB | ~0.39 core | 488 MB |
| Silero VAD | (included) | (included) | ~0.002 core | 0.6 MB |

"CPU while live" is user CPU-seconds divided by audio-seconds. `/usr/bin/time` does not see GPU time, and Metal buffers in unified memory may be under-counted in RSS.

The spike binary statically links both engines and is 28 MB.

## What I tried

1. **`sherpa-onnx` crate + `nemo_transducer` (works, used).** It built first try in about 25 s, because the build script downloads a prebuilt static library. The API is small: `OfflineRecognizer::create`, `create_stream`, `accept_waveform`, `decode`, `get_result`. The default `feature_dim = 80` worked; sherpa reads Parakeet's 128-bin setting from the model metadata.
2. **Parakeet on CoreML (`PROVIDER=coreml`) did not work.** The prebuilt onnxruntime has no CoreML execution provider. It logged "CoreML is for Apple only since onnxruntime>=1.15. Fallback to cpu!" and ran on the CPU. A GPU or ANE path would need one of:
   - sherpa-onnx's `shared` feature with an onnxruntime built with CoreML,
   - the `scriptrs` crate (Parakeet on Core ML),
   - a custom `ort` build.

   I did not try these. This is the main lever for cutting Parakeet's CPU use.
3. **`whisper-rs` 0.16 with Metal (works).** It built in about 30 s with cmake and no extra setup. Both engines link into one binary without symbol clashes.
4. **Not tried:** `parakeet-rs` and raw `ort`. They weren't needed once sherpa-onnx worked.

## Problems seen

- **Memory grows with input length.** A whole 2-minute decode took Parakeet's RSS from 1.25 GB to 2.3 GB, because onnxruntime's arena grows with the input shape. For any end-of-meeting re-pass, decode VAD segments or ≤30 s windows, never a whole recording.
- **Latency spikes.** Under heavy machine load, single chunks took 1–4 s, for both engines. On the quiet run the worst Parakeet chunk was 0.64 s. The live pipeline needs a queue so that one slow decode doesn't stall audio capture.
- **AMI is hard for every model.** The references include fragments ("y"), "um"/"uh" and British spelling, so a 14–16% WER is partly scoring noise. The ranking held on all three clips.
- **English only.** Parakeet v2 is English-only. `parakeet-tdt-0.6b-v3` (25 European languages, also packaged for sherpa-onnx) is the drop-in to test next.

## Recommendation

1. **Make Parakeet-TDT-0.6B-v2 int8 the default STT, through `sherpa-onnx`, on the CPU with 4 threads.** It was the most accurate on all three clips (the gap is largest on meeting speech), the most robust on long-form audio, and runs live at RTF ≈ 0.06.
2. **Chunking:** Silero VAD (the same crate, 0.6 MB model) with `max_speech_duration = 5 s` and `min_silence = 0.25 s`. 3 s and 5 s gave the same WER. 5 s means fewer calls and fewer mid-sentence cuts. Text appears about 0.5–0.8 s after a pause: 0.25 s of VAD silence plus 0.2–0.5 s of decode.
3. **Whisper as the fallback:** whisper-rs with Metal, small.en (or multilingual small) for non-English meetings, and base.en for machines with less than 8 GB of RAM. It is also the low-CPU option on battery: it uses about 0.01 CPU core, with the work on the GPU.
4. **Follow-ups:**
   - Try the CoreML or ANE path for Parakeet to cut its CPU use.
   - Evaluate v3 for multilingual meetings.
   - Measure on the user's own meetings once capture exists.

### Plugging into `SttProvider`

```rust
pub struct ParakeetStt { rec: Arc<sherpa_onnx::OfflineRecognizer> }   // Send + Sync

impl Provider for ParakeetStt {
    fn id(&self) -> &str { "parakeet-tdt-0.6b-v2" }
    fn locality(&self) -> Locality { Locality::Local }
}

impl SttProvider for ParakeetStt {
    // pcm: one VAD segment, 16 kHz mono f32, ≤ 5 s
    fn transcribe(&self, pcm: &[f32]) -> impl Future<Output = Result<String>> + Send {
        let (rec, pcm) = (self.rec.clone(), pcm.to_vec());
        async move {
            // ~0.2–0.5 s of CPU: keep it off the async executor
            run_blocking(move || {   // e.g. tauri::async_runtime::spawn_blocking
                let s = rec.create_stream();
                s.accept_waveform(16_000, &pcm);
                rec.decode(&s);
                s.get_result().map(|r| r.text).unwrap_or_default()
            }).await
        }
    }
}
```

Notes for the real implementation:
- Load the recognizer once (about 1 s) and share it.
- Run the VAD upstream of `transcribe`, in the capture stage.
- Process segments in order through one worker, so the transcript stays in sequence and only one decode holds the 4 threads at a time.
- `mirona_providers::Error` has no runtime-failure variant yet. A model load or decode failure needs something like `Error::Provider(String)`.
- `sherpa-onnx` already ships the Silero VAD, the speaker-embedding and diarization APIs, and a punctuation model. The diarizer can come from the same dependency.

## Licenses

| Item | License |
|---|---|
| `sherpa-onnx` / `sherpa-onnx-sys` crates | Apache-2.0 |
| onnxruntime (bundled static library) | MIT |
| Parakeet-TDT-0.6B-v2 weights (NVIDIA; sherpa-onnx int8 export) | **CC-BY-4.0**: the app must credit NVIDIA in its notices |
| Silero VAD | MIT |
| `whisper-rs` | Unlicense |
| whisper.cpp / ggml | MIT |
| Whisper base.en / small.en weights (OpenAI) | MIT |
| `hound` | Apache-2.0 |
| LibriSpeech (test data) | CC-BY-4.0 |
| AMI Meeting Corpus (test data) | CC-BY-4.0 |

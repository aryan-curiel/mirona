# NGR-128 spike: Parakeet-TDT vs Whisper in Rust

A throwaway benchmark. It is a separate cargo project with its own `[workspace]`, so it is not part of the Mirona workspace. Results and the recommendation are in `docs/spikes/ngr-128-parakeet.md`.

```sh
./fetch.sh     # models (~1.3 GB) + 3 public ~2-min clips with reference transcripts
./bench.sh     # all engines at 3 s and 5 s max chunk, plus peak RSS (takes ~10 min)

# single run
cargo run --release --features whisper -- parakeet 5 audio/*.wav
SHOW=1 cargo run --release -- parakeet 5 audio/ami-meeting.wav   # print the hypothesis
cargo test --release                                             # WER self-check
```

Engines: `parakeet` (sherpa-onnx, int8, CPU) and `whisper-base` / `whisper-small` (whisper-rs with Metal, needs `--features whisper`).
Env vars: `THREADS` (default 4), `PROVIDER` (Parakeet ONNX provider, default `cpu`), `NOFULL=1` (chunks only, to measure live peak memory), `SHOW=1`.

For each clip the tool does three things:
- Transcribes the whole file once, for full-file RTF and WER.
- Cuts the clip into fixed N-second chunks and transcribes each one. These are blind cuts, so this is the worst case.
- Cuts the clip with Silero VAD (forced cut at N seconds) and transcribes each segment. This is what a live pipeline would do.

It then reports per-chunk latency at p50, p95 and max. A 1-second warm-up decode runs first and is not timed.

Note: `sherpa-onnx-sys` downloads a prebuilt static sherpa-onnx + onnxruntime archive at build time.

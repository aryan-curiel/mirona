# Licenses (NGR-135)

**Mirona is licensed under Apache-2.0** (`LICENSE`). It's permissive, includes a patent grant, and is compatible with every dependency below.

## Models

Nothing is bundled. The app downloads the models the configured providers need (onboarding and Settings › AI models; `crates/mirona-engine/src/models.rs`, NGR-174): pinned revisions, a SHA-256 per file, resume. `scripts/fetch-models.sh` fetches the same models for source checkouts, without checksums (Parakeet from the sherpa-onnx GitHub release tarball, Whisper from `main`, the CoreML set on any Mac).

| Stage | Model | Source (app downloader) | License | Size | Notes |
| --- | --- | --- | --- | --- | --- |
| Speech-to-text | Parakeet-TDT-0.6B v2, int8 ONNX (sherpa-onnx export) | huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8 | CC-BY-4.0 (NVIDIA) | ~660 MB | Credit NVIDIA and link the license (`NOTICE`, Settings › About) |
| Speech-to-text (alternative) | Whisper small.en (`ggml`, whisper.cpp conversion) | huggingface.co/ggerganov/whisper.cpp | MIT | ~490 MB | Only when Whisper is picked; keep the MIT notice |
| VAD (NGR-138) | Silero VAD (ONNX) | github.com/k2-fsa/sherpa-onnx releases (`asr-models`) | MIT | ~1 MB | |
| Diarization | speakrs models: pyannote segmentation-3.0, WeSpeaker ResNet34 embedding, pyannote community-1 PLDA/clustering parameters | huggingface.co/avencera/speakrs-models | Segmentation MIT; embedding model and clustering parameters CC-BY-4.0 | 33 MB CPU set; +272 MB CoreML set only on macOS with ≥ 16 GB RAM | Credit pyannote, WeSpeaker and the speakrs conversions (avencera) |
| Classification | Jev (hosted by TypeSafe) | n/a, nothing ships | TypeSafe terms of service | | Transcript text is sent to TypeSafe (US-hosted). Their privacy policy says inputs are not used for training. Retention is "as long as necessary", with no fixed period; zero data retention is enterprise-only. Privacy mode blocks Jev, and the privacy policy and onboarding must say this. |
| Classification in privacy mode | qwen3:4b via Ollama | the user pulls it through Ollama | Apache-2.0 | | Mirona doesn't redistribute LLM weights |
| Embeddings (search) | nomic-embed-text via Ollama | the user pulls it through Ollama | Apache-2.0 | | |

**Rule:** models are downloaded on demand from pinned revisions with checksums, never bundled. Every CC-BY model is credited in `NOTICE` and in Settings › About.

## Code

| Dependency | License | Note |
| --- | --- | --- |
| speakrs | Apache-2.0 | |
| sherpa-onnx (Rust crate + C library) | Apache-2.0 | Brings ONNX Runtime (MIT) |
| whisper-rs | Unlicense | |
| libopus 1.6.1 (Xiph.Org), built from source through `opusic-sys` (BSD-3-Clause) and `opus` (MIT OR Apache-2.0) | BSD-3-Clause | Encodes and decodes recordings (`audio.caf`). Keep the BSD notice in `NOTICE`. Opus is royalty-free; its patent licenses are listed at opus-codec.org |
| rmcp (MCP server) | Apache-2.0 | |
| Tauri, React, zustand, Tailwind | MIT / Apache-2.0 | |
| Meetily (Zackriya Solutions), audio capture approach | MIT | Approach only, no code copied (NGR-137). Code copied later must keep its MIT notice in the files and in `NOTICE` |

## Name and domains (checked 2026-11-05)

- **Trademark:** a web search found no "Mirona" registration for software. Similar marks exist (MIRA, MIRIXA in class 42). This isn't a legal search; check USPTO, EUIPO and WIPO before launch.
- **Domains:** `mirona.com` (since 2000) and `mirona.ai` (since 2025) are taken. `mirona.app`, `mirona.dev` and `getmirona.com` were unregistered when checked.
- **GitHub:** the repo is `aryan-curiel/mirona` (private). Package names stay under Nagare Studio: app id `com.nagare.mirona`, Homebrew tap `nagare-studio/tap`.
- **Credits:** `NOTICE` (shipped inside the app bundle) and Settings › About.

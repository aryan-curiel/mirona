# Licenses (NGR-135)

**Mirona is licensed under Apache-2.0** (`LICENSE`). It's permissive, includes a patent grant, and is compatible with every dependency below.

## Models

"Bundle" means shipping the weights inside the installer. "Download" means fetching them on first run (NGR-174).

| Stage | Model | License | Bundle? | Notes |
| --- | --- | --- | --- | --- |
| Speech-to-text | Parakeet-TDT-0.6B v2, int8 ONNX (sherpa-onnx export) | CC-BY-4.0 | Allowed, but **download** (~640 MB) | Credit NVIDIA, link the license in About and the docs |
| Speech-to-text (alternative) | Whisper (`ggml` weights) | MIT | Allowed; download | Keep the MIT notice |
| Diarization | speakrs models (pyannote community-1: segmentation + embedding) | Segmentation MIT; embedding model and clustering parameters CC-BY-4.0 | **Bundle** the 33 MB CPU set; **download** the 273 MB CoreML set on first use, pinned to a fixed revision | Credit pyannote, WeSpeaker and the speakrs conversions (avencera) |
| Classification | Jev (hosted by TypeSafe) | TypeSafe terms of service | n/a, nothing ships | Transcript text is sent to TypeSafe (US-hosted). Their privacy policy says inputs are not used for training. Retention is "as long as necessary", with no fixed period; zero data retention is enterprise-only. Privacy mode blocks Jev, and the privacy policy and onboarding must say this. |
| Classification fallback | Local LLM via Ollama (user's choice of model) | The model's own license | n/a; the user pulls it through Ollama | Mirona doesn't redistribute LLM weights |
| Embeddings (milestone 3) | Not chosen yet. Candidates: bge-small-en-v1.5 (MIT), all-MiniLM-L6-v2 (Apache-2.0) | | Small enough to bundle | |
| VAD (NGR-138) | Silero VAD | MIT | Allowed (~2 MB) | |

**Rule:** large models are downloaded on first run with checksums. Small models with permissive licenses may be bundled. Every CC-BY model is credited in `NOTICE` and on the Models and credits section of the settings screen.

## Code

| Dependency | License | Note |
| --- | --- | --- |
| speakrs | Apache-2.0 | |
| sherpa-onnx (Rust crate + C library) | Apache-2.0 | |
| whisper-rs | Unlicense | |
| pyannote-rs | MIT | |
| ort (ONNX Runtime bindings) | MIT OR Apache-2.0 | ONNX Runtime itself is MIT |
| Tauri, React, zustand, Tailwind | MIT / Apache-2.0 | |
| Meetily (Zackriya Solutions), audio capture approach | MIT | Any code copied for NGR-137 must keep its MIT notice in the copied files and in `NOTICE` |

## Name and domains (checked 2026-11-05)

- **Trademark:** a web search found no "Mirona" registration for software. Similar marks exist (MIRA, MIRIXA in class 42). This isn't a legal search; check USPTO, EUIPO and WIPO before launch.
- **Domains:** `mirona.com` (since 2000) and `mirona.ai` (since 2025) are taken. `mirona.app`, `mirona.dev` and `getmirona.com` were unregistered when checked.
- **GitHub:** the repo is `aryan-curiel/mirona` (private). Package names stay under Nagare Studio: app id `com.nagare.mirona`, Homebrew tap `nagare-studio/tap`.
- **Credits:** `NOTICE` (shipped inside the app bundle) and Settings → Models and MCP → Models and credits.

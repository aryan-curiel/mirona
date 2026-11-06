#!/bin/sh
# Downloads the speech models mirona-capture loads (NGR-139, ADR 0001) into <dir>:
#   silero_vad.onnx                              Silero VAD, 0.6 MB (MIT)
#   sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/  Parakeet STT, ~660 MB (CC-BY-4.0, NVIDIA)
#   speakrs/                                     speaker diarization + voiceprints (CC-BY-4.0,
#                                                pyannote/WeSpeaker via avencera/speakrs-models):
#                                                CPU set ~33 MB everywhere, CoreML set ~273 MB on macOS
#   ggml-small.en.bin / ggml-base.en.bin         Whisper, only with --whisper[=base] (MIT)
# Usage: scripts/fetch-models.sh <dir> [--whisper | --whisper=base]
# Re-running skips what is already there. The app downloads the same files itself
# (crates/mirona-engine/src/models.rs, pinned + SHA-256); keep the two in step.
set -eu
[ $# -ge 1 ] || { echo "usage: $0 <dir> [--whisper | --whisper=base]" >&2; exit 2; }
dir=$1
whisper=${2:-}
mkdir -p "$dir"
cd "$dir"
R=https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models
P=sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8

if [ ! -f silero_vad.onnx ]; then
  curl -fL --retry 3 -o silero_vad.onnx.part "$R/silero_vad.onnx"
  mv silero_vad.onnx.part silero_vad.onnx
fi

if [ ! -f "$P/tokens.txt" ]; then
  # Extract to a temp dir and move into place, so an interrupted download never looks done.
  rm -rf "$P.part" && mkdir "$P.part"
  curl -fL --retry 3 "$R/$P.tar.bz2" | tar xj -C "$P.part"
  rm -rf "$P" && mv "$P.part/$P" "$P" && rm -rf "$P.part"
fi

# speakrs models, pinned to the revision speakrs 0.7.0 loads (speakrs::models::MODEL_REVISION).
H=https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8
get() { # <file> into speakrs/, skipped if there
  [ -f "speakrs/$1" ] && return 0
  mkdir -p "$(dirname "speakrs/$1")"
  curl -fL --retry 3 -o "speakrs/$1.part" "$H/$1"
  mv "speakrs/$1.part" "speakrs/$1"
}
# PLDA + metadata first, the weights speakrs checks for last: a partial set never looks done.
for f in plda_lda.npy plda_tr.npy plda_mu.npy plda_psi.npy plda_mean1.npy plda_mean2.npy \
  wespeaker-voxceleb-resnet34.min_num_samples.txt segmentation-3.0.safetensors \
  wespeaker-multimask-tail.safetensors; do get "$f"; done
if [ "$(uname)" = Darwin ]; then
  for m in segmentation-3.0 segmentation-3.0-b32 segmentation-3.0-b64 wespeaker-fbank \
    wespeaker-fbank-b32 wespeaker-fbank-30s wespeaker-multimask-tail-b32 \
    wespeaker-voxceleb-resnet34-tail wespeaker-voxceleb-resnet34-tail-b3 \
    wespeaker-voxceleb-resnet34-tail-b32 wespeaker-chunk-emb-p1s-w21 wespeaker-chunk-emb-p1s-w36 \
    wespeaker-chunk-emb-p1s-w51 wespeaker-chunk-emb-p1s-w81 wespeaker-chunk-emb-p1s-w111; do
    for f in model.mil coremldata.bin analytics/coremldata.bin weights/weight.bin; do
      get "$m.mlmodelc/$f"
    done
  done
fi

case $whisper in
  "") ;;
  --whisper) m=small.en ;;
  --whisper=base) m=base.en ;;
  *) echo "unknown option $whisper" >&2; exit 2 ;;
esac
if [ -n "$whisper" ] && [ ! -f "ggml-$m.bin" ]; then
  curl -fL --retry 3 -o "ggml-$m.bin.part" \
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-$m.bin"
  mv "ggml-$m.bin.part" "ggml-$m.bin"
fi
echo "models in $(pwd)"

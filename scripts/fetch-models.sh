#!/bin/sh
# Downloads the speech models mirona-capture loads (NGR-139, ADR 0001) into <dir>:
#   silero_vad.onnx                              Silero VAD, 0.6 MB (MIT)
#   sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/  Parakeet STT, ~660 MB (CC-BY-4.0, NVIDIA)
#   ggml-small.en.bin / ggml-base.en.bin         Whisper, only with --whisper[=base] (MIT)
# Usage: scripts/fetch-models.sh <dir> [--whisper | --whisper=base]
# Re-running skips what is already there.
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

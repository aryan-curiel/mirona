#!/bin/sh
# Downloads models (~1.3 GB on disk) and builds the three test clips into audio/.
set -e
cd "$(dirname "$0")"
mkdir -p models audio
cd models
R=https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models
[ -d sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8 ] ||
  curl -sSL "$R/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8.tar.bz2" | tar xj
[ -f silero_vad.onnx ] || curl -sSLO "$R/silero_vad.onnx"
for m in base.en small.en; do
  [ -f "ggml-$m.bin" ] ||
    curl -sSLO "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-$m.bin"
done
cd ..
python3 -I fetch_audio.py audio

#!/usr/bin/env bash
# NGR-129: fetch test audio + reference RTTMs + models (~620 MB total; models/ and audio/ are gitignored).
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p audio/vc audio/ami audio/ami-far models/pyannote-rs out/ref

# VoxConverse dev RTTMs (CC BY 4.0) + 6 clips pulled out of the 2 GB dev zip via HTTP range requests.
VC="crixb qvtia houcx bspxd dhorc qfdpp"   # 2,2,3,3,4,4 reference speakers, ~5 min each
if [ ! -d audio/voxconverse-master ]; then
  curl -sfL -o audio/vc.zip https://github.com/joonson/voxconverse/archive/refs/heads/master.zip
  (cd audio && unzip -q vc.zip 'voxconverse-master/dev/*' && rm vc.zip)
fi
python3 -I scripts/remote_unzip.py https://www.robots.ox.ac.uk/~vgg/data/voxconverse/data/voxconverse_dev_wav.zip \
  audio/vc $(for i in $VC; do echo "$i.wav"; done)
for i in $VC; do cp audio/voxconverse-master/dev/$i.rttm out/ref/; done

# AMI ES2004a/b (CC BY 4.0): same 4 people in two meetings. Headset mix + far-field array mic for a.
AMI=https://groups.inf.ed.ac.uk/ami/AMICorpusMirror/amicorpus
for m in ES2004a ES2004b; do
  curl -sfL -o out/ref/$m.rttm https://raw.githubusercontent.com/pyannote/AMI-diarization-setup/main/only_words/rttms/test/$m.rttm
  cp out/ref/$m.rttm audio/ami/
  [ -f audio/ami/$m.wav ] || curl -sfL -o audio/ami/$m.wav $AMI/$m/audio/$m.Mix-Headset.wav
done
[ -f audio/ami-far/ES2004a.wav ] || curl -sfL -o audio/ami-far/ES2004a.wav $AMI/ES2004a/audio/ES2004a.Array1-01.wav

# Enrollment clips (10 s / 30 s of single-speaker speech per person, from ES2004a).
python3 -I scripts/make_enroll.py audio/ami/ES2004a.wav audio/ami/ES2004a.rttm audio/enroll
python3 -I scripts/make_enroll.py audio/ami-far/ES2004a.wav audio/ami/ES2004a.rttm audio/enroll-far

# speakrs models (HF avencera/speakrs-models, pinned revision) for coreml, coreml-fast and cpu.
cargo build --release
./target/release/speakrs-spike download models/hf

# pyannote-rs comparison models.
for f in segmentation-3.0.onnx "wespeaker_en_voxceleb_CAM++.onnx"; do
  [ -f "models/pyannote-rs/$f" ] || curl -sfL -o "models/pyannote-rs/$f" \
    "https://github.com/thewh1teagle/pyannote-rs/releases/download/v0.1.0/$f"
done

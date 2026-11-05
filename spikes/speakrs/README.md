# speakrs diarization spike (NGR-129)

Standalone cargo project (own `[workspace]`, not in the Mirona workspace). Write-up:
[`docs/spikes/ngr-129-speakrs.md`](../../docs/spikes/ngr-129-speakrs.md).

```bash
cd spikes/speakrs
./fetch.sh            # audio + reference RTTMs + models (~620 MB), builds the binary
M=$(echo models/hf/models--avencera--speakrs-models/snapshots/*)

# Diarize: writes out/<mode>/<id>.rttm and <id>.emb.json (per-speaker centroid embeddings)
for mode in coreml coreml-fast cpu; do
  /usr/bin/time -l ./target/release/speakrs-spike run $mode $M out/$mode audio/vc/*.wav audio/ami/*.wav
done
./target/release/speakrs-spike run coreml $M out/coreml-far audio/ami-far/ES2004a.wav
./target/release/speakrs-spike run coreml $M out/enroll      audio/enroll/*.wav
./target/release/speakrs-spike run coreml $M out/enroll-far  audio/enroll-far/*.wav
./target/release/speakrs-spike run cpu    $M out/enroll-cpu  audio/enroll/*.wav

# DER (pyannote.metrics, collar 0 and +/-0.25 s, overlap scored)
IDS="bspxd crixb dhorc houcx qfdpp qvtia ES2004a ES2004b"
uv run --with pyannote.metrics python scripts/der.py out/ref out/coreml $IDS

# Voiceprints: same person across ES2004a vs ES2004b
python3 -I scripts/embeddings.py out/ref out/coreml emb out/coreml emb          # meeting vs meeting
python3 -I scripts/embeddings.py out/ref out/enroll-cpu enroll30 out/coreml emb # 30 s enrollment vs meeting
python3 -I scripts/embeddings.py out/ref out/enroll-far enroll30 out/coreml emb # far-field enrollment

# CoreML short-input embedding bug: same clip cut to 5..20 s, coreml vs cpu centroid
mkdir -p audio/short && for t in 5 10 12 15 20; do
  ffmpeg -loglevel error -y -i audio/enroll/ES2004a_FEE013_30s.wav -t $t audio/short/FEE013_$t.wav; done
for m in coreml cpu; do ./target/release/speakrs-spike run $m $M out/short-$m audio/short/*.wav; done
# then compare out/short-coreml/*.emb.json vs out/short-cpu/*.emb.json ("emb" of the largest cluster)

# pyannote-rs 0.3.4 baseline
(cd pyannote-rs-cmp && cargo build --release)
./pyannote-rs-cmp/target/release/pyannote-rs-cmp models/pyannote-rs out/pyannote-rs audio/vc/*.wav audio/ami/*.wav
uv run --with pyannote.metrics python scripts/der.py out/ref out/pyannote-rs $IDS
```

`SPEAKRS_CU=cpu-ne|cpu` switches the CoreML embedding compute units (memory experiment).

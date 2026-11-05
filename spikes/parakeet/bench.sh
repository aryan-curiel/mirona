#!/bin/sh
# Runs every engine at 3 s and 5 s max chunk; peak RSS / CPU time from /usr/bin/time -l.
# Usage: ./bench.sh [engines...]   (default: parakeet whisper-base whisper-small)
set -e
cd "$(dirname "$0")"
cargo build --release --features whisper -q
for e in ${@:-parakeet whisper-base whisper-small}; do
  for c in 3 5; do
    echo "== $e chunk=${c}s load: $(uptime | sed 's/.*averages: //')"
    /usr/bin/time -l target/release/parakeet-spike "$e" "$c" audio/*.wav 2>&1 |
      grep -E 'engine=|audio/|chunks<=|real|maximum resident|peak memory'
  done
  echo "== $e live-only (NOFULL) 5s"
  NOFULL=1 /usr/bin/time -l target/release/parakeet-spike "$e" 5 audio/*.wav 2>&1 |
    grep -E 'chunks<=|real|maximum resident|peak memory'
done

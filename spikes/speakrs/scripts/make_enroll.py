"""Cut single-speaker "enrollment" WAVs (10 s / 30 s) per AMI speaker from reference turns.

Uses the parts of each reference turn where no other speaker talks (pieces >= 1 s), in time order.
usage: python3 -I scripts/make_enroll.py <wav> <rttm> <out_dir>
"""

import sys
import wave
from collections import defaultdict
from pathlib import Path

wav, rttm, out = sys.argv[1:]
turns = defaultdict(list)
for line in Path(rttm).read_text().splitlines():
    f = line.split()
    turns[f[7]].append((float(f[3]), float(f[3]) + float(f[4])))

with wave.open(wav) as w:
    rate, pcm = w.getframerate(), w.readframes(w.getnframes())  # 16 kHz mono s16
Path(out).mkdir(parents=True, exist_ok=True)
for spk, segs in turns.items():
    others = [t for o, ts in turns.items() if o != spk for t in ts]
    clean = []
    for s, e in sorted(segs):
        pieces = [(s, e)]
        for os_, oe in others:  # subtract every other speaker's turn
            pieces = [p for a, b in pieces for p in ((a, min(b, os_)), (max(a, oe), b)) if p[1] > p[0]]
        clean += [(a, b) for a, b in pieces if b - a >= 1.0]
    for secs in (10, 30):
        buf, got = bytearray(), 0.0
        for s, e in clean:
            take = min(e - s, secs - got)
            buf += pcm[int(s * rate) * 2 : int((s + take) * rate) * 2]
            got += take
            if got >= secs:
                break
        name = f"{Path(wav).stem}_{spk}_{secs}s.wav"
        with wave.open(f"{out}/{name}", "wb") as o:
            o.setnchannels(1), o.setsampwidth(2), o.setframerate(rate), o.writeframes(bytes(buf))
        print(name, f"{got:.1f}s")

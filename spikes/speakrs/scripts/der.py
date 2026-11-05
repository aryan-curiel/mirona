"""DER per clip with pyannote.metrics (overlap scored, optimal speaker mapping).

usage: uv run --with pyannote.metrics python scripts/der.py <ref_dir> <hyp_dir> <id>...
Reports collar 0 and the standard md-eval +/-0.25 s collar (pyannote collar=0.5 = total width).
"""

import sys
from pathlib import Path

from pyannote.core import Annotation, Segment
from pyannote.metrics.diarization import DiarizationErrorRate


def load(path):
    ann = Annotation()
    for line in Path(path).read_text().splitlines():
        f = line.split()
        if f and f[0] == "SPEAKER":
            s, d = float(f[3]), float(f[4])
            ann[Segment(s, s + d)] = f[7]
    return ann


ref_dir, hyp_dir, *ids = sys.argv[1:]
m0, m25 = DiarizationErrorRate(collar=0.0), DiarizationErrorRate(collar=0.5)
print("id\tref_spk\thyp_spk\tDER_c0\tDER_c25\tmiss\tfa\tconf (c25)")
for i in ids:
    ref, hyp = load(f"{ref_dir}/{i}.rttm"), load(f"{hyp_dir}/{i}.rttm")
    d0 = m0(ref, hyp)
    c = m25(ref, hyp, detailed=True)
    t = c["total"]
    print(
        f"{i}\t{len(ref.labels())}\t{len(hyp.labels())}\t{d0:.1%}\t{c['diarization error rate']:.1%}"
        f"\t{c['missed detection'] / t:.1%}\t{c['false alarm'] / t:.1%}\t{c['confusion'] / t:.1%}"
    )
print(f"TOTAL\t\t\t{abs(m0):.1%}\t{abs(m25):.1%}")

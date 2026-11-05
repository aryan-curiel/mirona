"""Cross-recording voiceprint test on AMI ES2004a vs ES2004b (same 4 people, different meetings).

Each hypothesis cluster is mapped to the reference speaker it overlaps most (by time), so
SPEAKER_k centroids get a true identity. Then: cosine(centroid in a, centroid in b) for
same vs different person. Also within-recording pairs across all clips (different people).

usage: python3 scripts/embeddings.py <ref_dir> <hyp_dir_a> <key_a> <hyp_dir_b> <key_b>
  key = emb (whole-meeting centroid) | emb10 / emb3 (first 10 / 3 chunk embeddings only)
  key_a = enroll10 / enroll30: hyp_dir_a holds runs on scripts/make_enroll.py clips; the voiceprint
  is the centroid of the largest cluster in ES2004a_<spk>_<N>s.
"""

import itertools
import json
import math
import sys
from collections import defaultdict
from pathlib import Path


def rttm(path):
    out = defaultdict(list)
    for line in Path(path).read_text().splitlines():
        f = line.split()
        if f and f[0] == "SPEAKER":
            s = float(f[3])
            out[f[7]].append((s, s + float(f[4])))
    return out


def overlap(a, b):
    return sum(max(0.0, min(e1, e2) - max(s1, s2)) for s1, e1 in a for s2, e2 in b)


def cos(a, b):
    return sum(x * y for x, y in zip(a, b)) / math.sqrt(sum(x * x for x in a) * sum(y * y for y in b))


def labelled(hyp_dir, ref_dir, fid, key, min_s=30.0):
    """{true_label: centroid} for clusters with >= min_s seconds of speech, best-overlap mapped."""
    hyp, ref = rttm(f"{hyp_dir}/{fid}.rttm"), rttm(f"{ref_dir}/{fid}.rttm")
    emb = json.loads(Path(f"{hyp_dir}/{fid}.emb.json").read_text())
    out = {}
    for k, segs in hyp.items():
        dur = sum(e - s for s, e in segs)
        if dur < min_s or k not in emb:
            continue
        best = max(ref, key=lambda r: overlap(segs, ref[r]))
        ov = overlap(segs, ref[best])
        if best not in out or ov > out[best][0]:  # keep the cluster covering this person most
            out[best] = (ov, k, dur, ov / dur, emb[k][key])
    return {r: v[1:] for r, v in out.items()}


ref_dir, hyp_a, key_a, hyp_b, key_b = sys.argv[1:]
hyp_dir = hyp_a
b = labelled(hyp_b, ref_dir, "ES2004b", key_b)
if key_a.startswith("enroll"):
    a = {}
    for p in Path(hyp_a).glob(f"ES2004a_*_{key_a[6:]}s.emb.json"):
        e = json.loads(p.read_text())
        k = max(e, key=lambda c: e[c]["n"])
        a[p.name.split("_")[1]] = (k, 0.0, 1.0, e[k]["emb"])
else:
    a = labelled(hyp_a, ref_dir, "ES2004a", key_a)
for name, d in (("ES2004a", a), ("ES2004b", b)):
    for r, (k, dur, pur, _) in sorted(d.items()):
        print(f"{name} {k} -> {r}  {dur:.0f}s  purity={pur:.0%}")

same, diff = [], []
print("\ncross-recording cosine (rows ES2004a, cols ES2004b):")
cols = sorted(b)
print("        " + " ".join(f"{c:>7}" for c in cols))
for ra in sorted(a):
    row = []
    for rb in cols:
        c = cos(a[ra][3], b[rb][3])
        (same if ra == rb else diff).append(c)
        row.append(f"{c:7.3f}")
    print(f"{ra:>7} " + " ".join(row))

# Within-recording different-speaker pairs, every clip (VoxConverse ids are per-file so only "diff").
within = []
for p in sorted(Path(hyp_dir).glob("*.emb.json")):
    e = json.loads(p.read_text())
    hyp = rttm(p.with_name(p.name.replace(".emb.json", ".rttm")))
    big = [k for k in e if sum(x - y for y, x in hyp.get(k, [])) >= 30]
    within += [cos(e[i]["emb"], e[j]["emb"]) for i, j in itertools.combinations(big, 2)]

print(f"\nsame person, cross-recording: n={len(same)} min={min(same):.3f} mean={sum(same)/len(same):.3f}")
print(f"diff person, cross-recording: n={len(diff)} max={max(diff):.3f} mean={sum(diff)/len(diff):.3f}")
print(f"diff cluster, within-recording (all clips, >=30s): n={len(within)} max={max(within):.3f} "
      f"mean={sum(within)/len(within):.3f}")

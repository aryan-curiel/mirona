"""NGR-130 spike: Jev (TypeSafe System One) vs a local Ollama model on eval/chunks.jsonl.

  uv run eval.py jev       # needs TYPESAFE_API_KEY (env or repo-root .env)
  uv run eval.py ollama    # needs `ollama serve` + the model pulled
  uv run eval.py report    # metrics from whatever results/*.json exist
  uv run eval.py selftest

Stdlib only. One request per chunk carrying all seven questions.
"""

import json
import math
import os
import re
import statistics
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
CHUNKS = ROOT / "eval" / "chunks.jsonl"
RESULTS = HERE / "results"
JEV_URL = "https://api.typesafe.ai/v1/systemone"
JEV_MODEL = os.environ.get("JEV_MODEL", "jev-latest")
OLLAMA_URL = os.environ.get("OLLAMA_URL", "http://localhost:11434/api/chat")
OLLAMA_MODEL = os.environ.get("OLLAMA_MODEL", "qwen3:4b")

# Same wording goes to both models.
YESNO = {
    "action_item": (
        "Someone in this transcript commits to, or is assigned, a concrete work task to do after the meeting.",
        {"true": "A person takes on or is given a specific work task (explicitly or implicitly, e.g. 'I can ping them', 'Sure' to a request).",
         "false": "Only status updates, vague intentions with no owner ('we should talk about that'), or social plans."},
    ),
    "decision": (
        "The participants settle a choice in this transcript: something is approved, chosen, rejected, or announced as decided.",
        {"true": "A proposal is accepted or rejected, or an option is chosen or announced as final.",
         "false": "Options are only discussed, deferred, or left open."},
    ),
    "open_question": (
        "A question raised in this transcript is left unanswered or unresolved by the end of it.",
        {"true": "Someone asks a question and nobody answers it, or the answer is deferred ('I don't know', 'let me check').",
         "false": "No question is asked, or every question asked is answered within the transcript."},
    ),
    "small_talk": (
        "This transcript is mainly social small talk rather than work discussion.",
        {"true": "Most of the exchange is personal chit-chat, greetings, or social plans.",
         "false": "Most of the exchange is about work."},
    ),
    "sensitive": (
        "This transcript contains sensitive information that should not leave the device.",
        {"true": "Passwords or credentials, salaries or compensation budgets, health or medical details, HR or performance matters about a named person, or information explicitly marked confidential.",
         "false": "Ordinary work content with none of the above."},
    ),
}
CHOICES = {
    "meeting_type": (
        "What kind of meeting is this transcript from?",
        {"standup": "Daily standup: short status updates and blockers.",
         "client_call": "A call with an external client or customer.",
         "one_on_one": "A private 1:1 between a manager and a report or two colleagues.",
         "planning": "Sprint, roadmap, or project planning.",
         "other": "Anything else: retro, all-hands, interview, incident review, vendor demo, training, social."},
    ),
    "client": (
        "Which client is this transcript about?",
        {"northwind_bank": "Northwind Bank (contact: Karen, compliance).",
         "helio_health": "Helio Health clinics (contact: Dr. Osei).",
         "bluepeak_retail": "Bluepeak Retail stores (contact: Ana).",
         "orbital_freight": "Orbital Freight logistics (contact: Felix).",
         "lumen_studios": "Lumen Studios film production (contact: Mia).",
         "none": "Internal discussion not about any of these clients."},
    ),
}
QUESTIONS = list(YESNO) + list(CHOICES)


def load_chunks():
    return [json.loads(line) for line in CHUNKS.read_text().splitlines() if line.strip()]


def post(url, body, headers, timeout=120):
    req = urllib.request.Request(url, json.dumps(body).encode(), {"Content-Type": "application/json", **headers})
    for attempt in range(4):
        try:
            t0 = time.perf_counter()
            with urllib.request.urlopen(req, timeout=timeout) as r:
                data = json.loads(r.read())
            return data, (time.perf_counter() - t0) * 1000
        except urllib.error.HTTPError as e:
            if e.code in (429, 529) and attempt < 3:
                time.sleep(2 ** attempt)
                continue
            raise RuntimeError(f"HTTP {e.code}: {e.read().decode()[:500]}") from None


# ---------- Jev ----------

def api_key():
    if os.environ.get("TYPESAFE_API_KEY"):
        return os.environ["TYPESAFE_API_KEY"].strip()
    env = ROOT / ".env"
    if env.exists():
        for line in env.read_text().splitlines():
            k, _, v = line.partition("=")
            if k.strip().removeprefix("export ").strip() == "TYPESAFE_API_KEY":
                return v.strip().strip("'\"")
    return None


def jev_request(text):
    qs = {k: {"type": "noul", "instructions": ins, "criteria": crit} for k, (ins, crit) in YESNO.items()}
    qs |= {k: {"type": "choice", "instructions": ins, "criteria": crit} for k, (ins, crit) in CHOICES.items()}
    return {"model": JEV_MODEL, "state": text, "questions": qs}


def jev_parse(ans):
    out = {}
    for k in YESNO:
        p = ans[k]["noul"]
        out[k] = {"answer": "yes" if p >= 0.5 else "no", "p_yes": p, "conf": max(p, 1 - p)}
    for k in CHOICES:
        a = ans[k]
        out[k] = {"answer": a["choice"], "conf": a["probabilities"][a["choice"]],
                  "jev_confidence": a.get("confidence"), "probabilities": a["probabilities"]}
    return out


def run_jev():
    key = api_key()
    if not key:
        sys.exit("No TYPESAFE_API_KEY in env or repo-root .env")
    rows = []
    for c in load_chunks():
        resp, ms = post(JEV_URL, jev_request(c["text"]), {"Authorization": f"Bearer {key}"})
        rows.append({"id": c["id"], "latency_ms": ms, "answers": jev_parse(resp["answers"]),
                     "model": resp.get("model"), "usage": resp.get("usage")})
        print(c["id"], f"{ms:.0f}ms", file=sys.stderr)
    write("jev", rows, {"endpoint": JEV_URL, "model_requested": JEV_MODEL})


# ---------- Ollama ----------

def ollama_body(text):
    defs = [f"- {k}: yes/no. YES if: {c['true']} NO if: {c['false']}" for k, (_, c) in YESNO.items()]
    defs += [f"- {k}: {ins} One of: " + "; ".join(f"{o} = {d}" for o, d in crit.items())
             for k, (ins, crit) in CHOICES.items()]
    schema = {"type": "object", "required": QUESTIONS, "additionalProperties": False,
              "properties": {**{k: {"type": "string", "enum": ["yes", "no"]} for k in YESNO},
                             **{k: {"type": "string", "enum": list(c)} for k, (_, c) in CHOICES.items()}}}
    return {
        "model": OLLAMA_MODEL, "stream": False, "think": False, "format": schema,
        "logprobs": True, "top_logprobs": 10,
        "options": {"temperature": 0, "seed": 0},
        "messages": [
            {"role": "system", "content": "You classify meeting transcript chunks. Answer every field.\n" + "\n".join(defs)},
            {"role": "user", "content": text},
        ],
    }


def value_token_probs(logprobs):
    """Map each JSON key -> top_logprobs of the first token of its string value."""
    out, text = {}, ""
    for i, t in enumerate(logprobs):
        text += t["token"]
        m = re.search(r'"(\w+)"\s*:\s*"$', text)
        if m and i + 1 < len(logprobs) and m.group(1) not in out:
            out[m.group(1)] = logprobs[i + 1]["top_logprobs"]
    return out


def option_prob(top, options, chosen):
    """P(chosen) renormalised over top tokens that are a prefix of exactly one option.
    ponytail: first-token approximation; options sharing a first token would collapse."""
    mass = dict.fromkeys(options, 0.0)
    for alt in top:
        tok = alt["token"].strip().strip('"')
        hits = [o for o in options if tok and o.startswith(tok)]
        if len(hits) == 1:
            mass[hits[0]] += math.exp(alt["logprob"])
    total = sum(mass.values())
    return mass[chosen] / total if total else None


def ollama_parse(resp):
    vals = json.loads(resp["message"]["content"])
    tops = value_token_probs(resp.get("logprobs") or [])
    out = {}
    for k in QUESTIONS:
        options = ["yes", "no"] if k in YESNO else list(CHOICES[k][1])
        p = option_prob(tops[k], options, vals[k]) if k in tops else None
        out[k] = {"answer": vals[k], "conf": p}
        if k in YESNO and p is not None:
            out[k]["p_yes"] = p if vals[k] == "yes" else 1 - p
    return out


def run_ollama():
    chunks = load_chunks()
    post(OLLAMA_URL, ollama_body(chunks[0]["text"]), {}, timeout=600)  # warm-up / model load, not timed
    rows = []
    for c in chunks:
        resp, ms = post(OLLAMA_URL, ollama_body(c["text"]), {}, timeout=600)
        rows.append({"id": c["id"], "latency_ms": ms, "answers": ollama_parse(resp)})
        print(c["id"], f"{ms:.0f}ms", file=sys.stderr)
    write("ollama-" + re.sub(r"\W", "-", OLLAMA_MODEL), rows, {"endpoint": OLLAMA_URL, "model": OLLAMA_MODEL})


def write(name, rows, meta):
    RESULTS.mkdir(exist_ok=True)
    (RESULTS / f"{name}.json").write_text(json.dumps({"meta": meta | {"run_at": time.strftime("%Y-%m-%dT%H:%M:%S")}, "rows": rows}, indent=1))
    print(f"wrote results/{name}.json", file=sys.stderr)


# ---------- report ----------

THRESHOLDS = [0.5, 0.6, 0.7, 0.8, 0.85, 0.9, 0.95, 0.99]


def pct(xs, q):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, math.ceil(q * len(xs)) - 1)]


def pick_threshold(pairs, target=0.95):
    """Lowest grid threshold whose covered-accuracy >= target; else the best-accuracy one with >=50% coverage."""
    sweep = []
    for t in THRESHOLDS:
        cov = [ok for conf, ok in pairs if conf is not None and conf >= t]
        if cov:
            sweep.append((t, len(cov) / len(pairs), sum(cov) / len(cov)))
    for t, cov, acc in sweep:
        if acc >= target:
            return t, cov, acc, sweep
    ok = [s for s in sweep if s[1] >= 0.5] or sweep
    return (*max(ok, key=lambda s: (s[2], -s[0])), sweep)


def evaluate(rows, gold):
    res = {}
    for q in QUESTIONS:
        pairs = [(r["answers"][q]["conf"], r["answers"][q]["answer"] == gold[r["id"]][q]) for r in rows]
        m = {"accuracy": sum(ok for _, ok in pairs) / len(pairs)}
        if q in YESNO:
            tp = sum(r["answers"][q]["answer"] == "yes" and gold[r["id"]][q] == "yes" for r in rows)
            fp = sum(r["answers"][q]["answer"] == "yes" and gold[r["id"]][q] == "no" for r in rows)
            fn = sum(r["answers"][q]["answer"] == "no" and gold[r["id"]][q] == "yes" for r in rows)
            m |= {"precision": tp / (tp + fp) if tp + fp else None, "recall": tp / (tp + fn) if tp + fn else None,
                  "positives": tp + fn}
        if any(c is not None for c, _ in pairs):
            t, cov, acc, sweep = pick_threshold(pairs)
            at60 = [ok for c, ok in pairs if c is not None and c >= 0.6]
            m |= {"threshold": t, "coverage_at_t": cov, "accuracy_at_t": acc, "sweep": sweep,
                  "coverage_at_0.60": len(at60) / len(pairs), "accuracy_at_0.60": sum(at60) / len(at60) if at60 else None}
        res[q] = m
    lat = [r["latency_ms"] for r in rows]
    return res, {"p50": pct(lat, 0.5), "p95": pct(lat, 0.95), "mean": statistics.mean(lat)}


def f(x, d=2):
    return "-" if x is None else f"{x:.{d}f}"


def report():
    gold = {c["id"]: c["labels"] for c in load_chunks()}
    models = {p.stem: json.loads(p.read_text()) for p in sorted(RESULTS.glob("*.json")) if p.stem != "summary"}
    if not models:
        sys.exit("no results yet")
    evals = {name: evaluate(d["rows"], gold) for name, d in models.items()}
    names = list(evals)
    lines = [f"# Classifier eval summary ({len(gold)} chunks)", "", "## Accuracy per question", "",
             "| question | " + " | ".join(names) + " |", "|---|" + "---|" * len(names)]
    for q in QUESTIONS:
        lines.append(f"| {q} | " + " | ".join(f(evals[n][0][q]["accuracy"]) for n in names) + " |")
    lines.append("| **mean** | " + " | ".join(f(statistics.mean(evals[n][0][q]["accuracy"] for q in QUESTIONS)) for n in names) + " |")
    lines += ["", "## Precision / recall (yes = positive)", "",
              "| question | positives | " + " | ".join(f"{n} P / R" for n in names) + " |", "|---|---|" + "---|" * len(names)]
    for q in YESNO:
        lines.append(f"| {q} | {evals[names[0]][0][q]['positives']} | " + " | ".join(
            f"{f(evals[n][0][q]['precision'])} / {f(evals[n][0][q]['recall'])}" for n in names) + " |")
    lines += ["", "## Latency per call (ms)", "", "| model | p50 | p95 | mean |", "|---|---|---|---|"]
    for n in names:
        l = evals[n][1]
        lines.append(f"| {n} | {l['p50']:.0f} | {l['p95']:.0f} | {l['mean']:.0f} |")
    for n in names:
        lines += ["", f"## Thresholds: {n}", "",
                  "conf = max(p, 1-p) for yes/no, probability of the chosen option for choices. "
                  "Recommended = lowest threshold with >=95% accuracy on answers above it.", "",
                  "| question | recommended | coverage | acc above | coverage@0.60 | acc@0.60 | sweep (t: cov/acc) |",
                  "|---|---|---|---|---|---|---|"]
        for q in QUESTIONS:
            m = evals[n][0][q]
            if "threshold" not in m:
                lines.append(f"| {q} | - | - | - | - | - | no probabilities |")
                continue
            sw = " ".join(f"{t}: {c:.2f}/{a:.2f}" for t, c, a in m["sweep"])
            lines.append(f"| {q} | {m['threshold']} | {f(m['coverage_at_t'])} | {f(m['accuracy_at_t'])} | "
                         f"{f(m['coverage_at_0.60'])} | {f(m['accuracy_at_0.60'])} | {sw} |")
    lines += ["", "## Errors", ""]
    for n in names:
        errs = [f"{r['id']}.{q}={r['answers'][q]['answer']}({f(r['answers'][q]['conf'])})"
                for r in models[n]["rows"] for q in QUESTIONS if r["answers"][q]["answer"] != gold[r["id"]][q]]
        lines.append(f"- **{n}** ({len(errs)}): " + ", ".join(errs))
    out = "\n".join(lines) + "\n"
    (RESULTS / "summary.md").write_text(out)
    print(out)


def selftest():
    assert pct([1, 2, 3, 4], 0.5) == 2 and pct(list(range(1, 101)), 0.95) == 95
    lp = [{"token": t, "logprob": 0, "top_logprobs": []} for t in ['{"', "a", '":"']]
    lp.append({"token": "yes", "logprob": math.log(0.8), "top_logprobs": [
        {"token": "yes", "logprob": math.log(0.8)}, {"token": "no", "logprob": math.log(0.2)}]})
    tops = value_token_probs(lp)
    assert abs(option_prob(tops["a"], ["yes", "no"], "yes") - 0.8) < 1e-9
    t, cov, acc, _ = pick_threshold([(0.99, True), (0.9, True), (0.7, False), (0.55, True)])
    assert t == 0.8 and cov == 0.5 and acc == 1.0
    assert jev_parse({**{k: {"noul": 0.3} for k in YESNO}, **{k: {"choice": "x", "probabilities": {"x": 0.7}} for k in CHOICES}})["action_item"] == {"answer": "no", "p_yes": 0.3, "conf": 0.7}
    print("ok")


if __name__ == "__main__":
    {"jev": run_jev, "ollama": run_ollama, "report": report, "selftest": selftest}[sys.argv[1]]()

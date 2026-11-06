# NGR-130 spike: Jev accuracy on labelled English transcript chunks

> Historical: the spike as run in November 2026. Current models and providers are in `docs/providers.md` and `docs/licenses.md`.

Status: **done.** Jev (`jev-1.14.0`) and two local baselines measured on the same 50 chunks.
Date: 2026-11-05. Machine: Apple M3 Pro, 36 GB.

## TL;DR

- **Jev (`jev-1.14.0`): mean accuracy 0.94**, `sensitive` recall 1.00, `action_item` P/R 0.95/1.00, **p50 190 ms, p95 324 ms** per chunk (all 7 questions in one call, including the network round trip). It clears every bar in the gate. Its weak spots: `decision` over-fires (precision 0.73), and `meeting_type` from one chunk (0.76).
- Local fallback (`qwen3:4b` via Ollama, JSON-schema output, temperature 0): **mean accuracy 0.87** over 7 questions. Latency per chunk (all 7 questions in one call): **p50 1.8 s, p95 2.8 s**.
- The fallback is usable for `small_talk` (0.98), `client` (0.96), `decision` (0.92) and `open_question` (0.90). It is weak on `meeting_type` from a single chunk (0.62). Its main problem is `sensitive`: recall is only **0.55**, and it misses with token probability ~1.0. Thresholds can't catch those misses.
- Gate decision: **Jev is the default classifier**, pinned to `jev-1.14.0`. qwen3:4b is acceptable as the privacy-mode fallback, with two conditions: meeting-level `meeting_type` and `client` should come from the whole meeting (or the user confirms them), and `sensitive` shouldn't depend on it alone (add a regex/keyword pass for credentials, money, health).

## Eval set (`eval/chunks.jsonl`)

**Synthetic, author-labelled.** All 50 chunks were written for this spike and labelled by hand by the same author. There is no real meeting data yet. Replace or extend it with real, consented transcript chunks before trusting any absolute number. With n=50, one chunk equals 2 points of accuracy, and the yes/no questions have only 6 to 21 positives. Treat differences under about 10 points as noise.

- 50 chunks of 3 to 5 speaker turns: 10 each of `standup`, `client_call`, `one_on_one`, `planning` and `other` (retro, all-hands, interview, incident review, vendor demo, training, social).
- Fictional clients: `northwind_bank`, `helio_health`, `bluepeak_retail`, `orbital_freight`, `lumen_studios`, plus `none`. Each client has a named contact, which is also given to the models in the choice criteria, the way the app's client list would. Counts: none 33, bluepeak 4, orbital 4, lumen 4, helio 3, northwind 2.
- Positives per yes/no label: action_item 21, decision 16, open_question 11, sensitive 11, small_talk 6.
- Tricky cases are included on purpose, and each row has a `note` explaining its label:
  - implicit action items ("I can ping their PM", a bare "Sure" to a request)
  - vague intentions with no owner, labelled as not an action item ("we should probably talk about that", "keep us posted")
  - social plans, labelled as not an action item ("I'll book the climbing gym for three")
  - rejected proposals, labelled as a decision
  - deferred proposals, labelled as no decision
  - questions that do get answered (not open), and questions that are deferred ("I don't know, let me check"), which count as open
  - sensitive content: spoken passwords, salary and compensation budget, cardiology and mental health, a PIP for a named employee, a candidate evaluation, confidential M&A
  - pure small talk, and all-negative chunks (vendor demo, training)

Label definitions are the ones sent to the models (see `YESNO` / `CHOICES` in `spikes/classifier-eval/eval.py`).

Caveat on `meeting_type` and `client`: in the app these are **meeting-level** questions (`MeetingAnswers`), and here they are asked per chunk. A 1:1 that opens with chit-chat about a trip doesn't look like a 1:1 in four lines. The chunk-level numbers for these two are therefore a pessimistic lower bound.

## Jev request shape (from the official docs)

Sources: docs.typesafe.ai `introduction`, `introduction/quickstart`, `api/system-one` (OpenAPI 0.2.0), `primitives/noul`, `primitives/choice`, `confidence`, `models`, `api/errors`, `legal`.

- `POST https://api.typesafe.ai/v1/systemone`
- Headers: `Authorization: Bearer $TYPESAFE_API_KEY`, `Content-Type: application/json`
- Body: `{ model, state, questions }`. `model` is `jev-latest`, an alias that currently points to `jev-1.14.0`. **Pin the versioned ID in the app once thresholds are tuned**, because the alias moves and answers can change. `state` is a string, object or array.
- Question types: `noul` (yes/no, optional `criteria: {"true": ..., "false": ...}`), `choice` (`criteria`: an option→description map, up to 255 options), `score` (an ordered list of up to 10 levels) and `box` (images only).
- All questions in one request are evaluated in parallel against one ingest of `state`. The docs say extra questions barely change latency.
- Answers come back keyed by question name:
  - noul: `{type, noul: P(yes)}`. There is **no** `confidence` field; use `max(p, 1-p)`.
  - choice: `{type, choice, probabilities: {...}, confidence}`. `confidence` is a spread statistic (1 = all mass on one option, 0 = uniform). It is not P(correct).
- Context: 64k tokens per request, 32k per question. Long context (262k) requires the header `TypeSafe-Danger-Zone-Long-Context: true`.
- Price: $0.042 per million input tokens. Output tokens are free.
- Errors: 400, 401, 413, 415, 422, 429 (rate limit, back off) and 529 (overloaded, back off). Rate limits are per organization, in tokens/s and requests/s.

Request used by the harness for one chunk (s07). All seven questions go in a single call:

```json
{
  "model": "jev-latest",
  "state": "Tom: Orbital's route optimizer is still timing out above 500 stops. Is it our solver or their geocoding API? Nobody knows yet.\nSara: Okay. Keep us posted.\nTom: Will do.",
  "questions": {
    "action_item": {
      "type": "noul",
      "instructions": "Someone in this transcript commits to, or is assigned, a concrete work task to do after the meeting.",
      "criteria": {
        "true": "A person takes on or is given a specific work task (explicitly or implicitly, e.g. 'I can ping them', 'Sure' to a request).",
        "false": "Only status updates, vague intentions with no owner ('we should talk about that'), or social plans."
      }
    },
    "decision":      { "type": "noul", "instructions": "...", "criteria": { "true": "...", "false": "..." } },
    "open_question": { "type": "noul", "instructions": "...", "criteria": { "true": "...", "false": "..." } },
    "small_talk":    { "type": "noul", "instructions": "...", "criteria": { "true": "...", "false": "..." } },
    "sensitive":     { "type": "noul", "instructions": "...", "criteria": { "true": "...", "false": "..." } },
    "meeting_type": {
      "type": "choice",
      "instructions": "What kind of meeting is this transcript from?",
      "criteria": {
        "standup": "Daily standup: short status updates and blockers.",
        "client_call": "A call with an external client or customer.",
        "one_on_one": "A private 1:1 between a manager and a report or two colleagues.",
        "planning": "Sprint, roadmap, or project planning.",
        "other": "Anything else: retro, all-hands, interview, incident review, vendor demo, training, social."
      }
    },
    "client": {
      "type": "choice",
      "instructions": "Which client is this transcript about?",
      "criteria": {
        "northwind_bank": "Northwind Bank (contact: Karen, compliance).",
        "helio_health": "Helio Health clinics (contact: Dr. Osei).",
        "bluepeak_retail": "Bluepeak Retail stores (contact: Ana).",
        "orbital_freight": "Orbital Freight logistics (contact: Felix).",
        "lumen_studios": "Lumen Studios film production (contact: Mia).",
        "none": "Internal discussion not about any of these clients."
      }
    }
  }
}
```

The full text of every question is in `eval.py` (`YESNO`, `CHOICES`). Print the exact body with:

```bash
uv run python -c "import eval,json;print(json.dumps(eval.jev_request(eval.load_chunks()[6]['text']),indent=2))"
```

Response shape, from the official OpenAPI examples. **This is not a response we observed.** Values are illustrative:

```json
{
  "model": "jev-1.14.0",
  "answers": {
    "action_item":  { "type": "noul", "noul": 0.12 },
    "open_question": { "type": "noul", "noul": 0.91 },
    "meeting_type": { "type": "choice", "choice": "standup", "confidence": 0.7,
                      "probabilities": { "standup": 0.82, "client_call": 0.06, "one_on_one": 0.02, "planning": 0.05, "other": 0.05 } }
  },
  "usage": { "input_tokens": 120, "output_tokens": 12 }
}
```

Mapping to the Rust types, for the later `Classifier` provider:

- noul → `Answer` with `probability = noul`, and yes when `noul >= 0.5`
- choice → `ChoiceAnswer { value: choice, probability: probabilities[choice], options: keys(criteria), threshold }`
- `ChoiceAnswer.probability` should be the chosen option's probability, not Jev's `confidence`. The harness records both.
- One HTTP call per segment fits `Classifier::classify` exactly.
- Log `response.model` so thresholds can be tied to a model version.

## Data sent to TypeSafe (privacy)

From docs.typesafe.ai/legal and the linked policies, as read on 2026-11-05:

- **Training:** the Privacy Policy says TypeSafe "will not train or fine tune any artificial intelligence or machine learning models on your prompts or other Input". The models page says Jev is not fine-tuned on customer data, and the same weights serve every account.
- **Retention:** there is no fixed period. The DPA says data is kept "as long as necessary taking into account the purpose of the Processing". The Privacy Policy says "as long as reasonably necessary to provide you with the Services", and deletion is available on request. **Zero data retention (ZDR) is offered for enterprise customers only.**
- **Location:** services are hosted in the US. EEA/UK data is transferred to the US. Sub-processors are listed at trust.typesafe.ai/subprocessors.
- Implication for Mirona: transcript text sent to Jev may be retained for an unspecified period, in the US. That supports the current design, where privacy mode blocks Jev entirely. It also means the onboarding/settings copy should say this plainly. If Mirona ships Jev as the default, ask TypeSafe about ZDR.

## Local fallback (Ollama)

- Ollama 0.40.14 (`brew install ollama`, `ollama serve`). The primary model is `qwen3:4b` (about 2.5 GB, pulled for this spike). `llama3.2:3b` (2.0 GB) was already on the machine and was run as a free second data point.
- `/api/chat` call settings:
  - `format` = a JSON schema with a string `enum` per question, so the output is always valid and in-vocabulary
  - `temperature 0`, `seed 0`
  - `think: false`, so qwen3 doesn't reason first; that would add seconds
  - one call per chunk with all 7 fields
- Probability: from **logprobs** (`logprobs: true, top_logprobs: 10`), not a verbalised confidence. For each field, the harness takes the top-10 alternatives for the first token of the value and renormalises over the tokens that prefix exactly one allowed option. These are the model's unconstrained next-token probabilities. They are not calibrated, and they are not a P(correct).
- Timing: one untimed warm-up call loads the model. All timings are warm, sequential calls.

## Results

### Accuracy per question

| question | Jev | qwen3:4b (local) | llama3.2:3b (local) |
|---|---|---|---|
| action_item | 0.98 | 0.82 | 0.86 |
| decision | 0.88 | 0.92 | 0.74 |
| open_question | 0.98 | 0.90 | 0.84 |
| small_talk | 0.98 | 0.98 | 0.96 |
| sensitive | 1.00 | 0.90 | 0.90 |
| meeting_type (per chunk) | 0.76 | 0.62 | 0.44 |
| client | 1.00 | 0.96 | 0.90 |
| **mean** | **0.94** | **0.87** | 0.81 |

### Precision / recall (yes = positive)

| question | positives | Jev P / R | qwen3:4b P / R | llama3.2:3b P / R |
|---|---|---|---|---|
| action_item | 21 | 0.95 / 1.00 | 0.77 / 0.81 | 0.82 / 0.86 |
| decision | 16 | 0.73 / 1.00 | 0.93 / 0.81 | 1.00 / 0.19 |
| open_question | 11 | 0.92 / 1.00 | 1.00 / 0.55 | 1.00 / 0.27 |
| small_talk | 6 | 1.00 / 0.83 | 0.86 / 1.00 | 0.75 / 1.00 |
| sensitive | 11 | 1.00 / 1.00 | 1.00 / 0.55 | 1.00 / 0.55 |

### Latency per call (all 7 questions)

| model | p50 | p95 | mean |
|---|---|---|---|
| Jev (`api.typesafe.ai`, network included) | 190 ms | 324 ms | 210 ms |
| qwen3:4b (M3 Pro, warm) | 1783 ms | 2784 ms | 2020 ms |
| llama3.2:3b (M3 Pro, warm) | 2116 ms | 2313 ms | 2136 ms |

### What the local model gets wrong (qwen3:4b, 45 wrong answers out of 350)

- **sensitive: 5 misses, 0 false alarms.** It missed the spoken password (s08), the cardiologist visit (s05), anxiety and therapy (o04), the confidential salary budget (p08) and the candidate evaluation (t08). Most misses have probability ≥ 0.98, so no threshold catches them. It did catch the client-shared VPN password, the salary negotiation, the PIP and the M&A chunk.
- **open_question: recall 0.55.** It treats a deferred answer ("I don't know", "let me check") or a non-answer as answered.
- **action_item:** it misses implicit or assigned tasks (o04, o08, s09) and flags commitments that aren't there (p08, t10, c03).
- **meeting_type:** 19 of the 50 chunk-level answers are wrong. Most go to `other`, and planning versus client call is confused.
- **client:** two false positives to `northwind_bank` (the vendor demo, and a conference-budget 1:1).

Full per-chunk errors are in `spikes/classifier-eval/results/summary.md`.

## Thresholds

Each answer's probability is defined as follows: `max(p, 1-p)` for yes/no, and the chosen option's probability for a choice. That matches `ChoiceAnswer.probability`.

The rule is to pick the lowest threshold whose accuracy, on the answers at or above it, is ≥ 95%. The grid is 0.5, 0.6, 0.7, 0.8, 0.85, 0.9, 0.95, 0.99. `report` prints the full coverage/accuracy sweep.

**Jev thresholds** (these become the defaults for NGR-158):

| question | rec. threshold | coverage | acc above | acc @ 0.60 (coverage) |
|---|---|---|---|---|
| action_item | 0.60 | 1.00 | 0.98 | 0.98 (1.00) |
| decision | 0.95 | 0.40 | 0.95 | 0.88 (0.98) |
| open_question | 0.60 | 0.98 | 0.98 | 0.98 (0.98) |
| small_talk | 0.60 | 1.00 | 0.98 | 0.98 (1.00) |
| sensitive | 0.60 | 0.98 | 1.00 | 1.00 (0.98) |
| meeting_type (per chunk) | 0.90 | 0.46 | 0.96 | 0.81 (0.86) |
| client | 0.60 | 1.00 | 1.00 | 1.00 (1.00) |

The sweep's raw minimum is 0.50 for several questions; we round up to the app's existing 0.60 default, which costs nothing here. Jev is well calibrated: accuracy rises with confidence on every question. **Keep 0.60 as the default** and raise only `decision` (0.95, or 0.85 at 0.89 accuracy and 0.76 coverage if 40% coverage is too little). `meeting_type` (0.90) should be asked over the whole meeting, not per chunk. Below threshold the item shows as "Needs review".

For comparison, the local qwen3:4b sweep (thresholds help less here because the logprobs are overconfident):

| question | rec. threshold | coverage | acc above | acc @ 0.60 (coverage) |
|---|---|---|---|---|
| action_item | 0.99 | 0.54 | 0.96 | 0.83 (0.96) |
| decision | 0.99 | 0.78 | 0.95 | 0.94 (0.98) |
| open_question | 0.95 | 0.82 | 0.95 | 0.92 (0.96) |
| small_talk | 0.50 | 1.00 | 0.98 | 0.98 (0.98) |
| sensitive | — (0.99 gives 0.93; never reaches 0.95) | 0.92 | 0.93 | 0.90 (1.00) |
| meeting_type | — (best 0.65 at 0.95) | 0.62 | 0.65 | 0.63 (0.82) |
| client | 0.50 | 1.00 | 0.96 | 0.96 (1.00) |

For the local fallback, keep 0.60 for `small_talk` and `client`. Use about 0.95 for `open_question` and `decision`, and 0.99 for `action_item`, accepting about half coverage. Below threshold, show the item as "unconfirmed" instead of dropping it. Don't threshold `sensitive` at all: no value fixes its misses.

## Gate decision

Proposed bar for Jev as the default classifier:

- mean accuracy ≥ 0.90 on this set
- `sensitive` recall ≥ 0.9
- `action_item` F1 ≥ 0.85
- p95 latency ≤ 1.5 s per chunk, so it keeps up with live chunks

Result: **Jev passes every bar** (0.94 mean, 1.00 sensitive recall, action_item F1 0.98, p95 324 ms).

1. **Jev as default: yes.** Pin `jev-1.14.0` (the `jev-latest` alias moves). Use the thresholds above. Watch `decision` precision on real meetings (rejected or tentative proposals get called decisions: s03, s05, c01, c06, c10, t07).
2. **Local LLM as privacy-mode fallback:** **acceptable, with conditions.** qwen3:4b reaches 0.87 mean accuracy on a laptop at about 2 s per chunk, which is fine for post-hoc and near-live use. The conditions:
   - `meeting_type` and `client` are asked once over the whole meeting, or a meeting summary, and shown for user confirmation (that's already `ChoiceAnswer.confirmed`)
   - `sensitive` is backed by a deterministic pass (credential patterns, currency amounts near salary/comp words, a health keyword list). The local model alone misses 45% of sensitive chunks.
   - `open_question` and `action_item` use the higher thresholds above
3. **How much worse the local fallback is:** 7 points lower mean accuracy (0.87 vs 0.94). The gap is mostly `sensitive` recall (0.55 vs 1.00), `open_question` recall (0.55 vs 1.00) and `action_item` (0.82 vs 0.98). It's also about 9× slower (p95 2.8 s vs 0.32 s). Privacy mode therefore means visibly worse tags, and the UI should say so.

Caveat: 50 synthetic, author-labelled chunks. Differences under about 10 points are noise. Gate 2 (NGR-163) reruns this on 200+ real labelled chunks.

## Reproduce

```bash
cd spikes/classifier-eval
uv run eval.py selftest                 # checks for the metric/logprob helpers
# local baseline (already run; results/ollama-qwen3-4b.json)
brew install ollama && ollama serve &   # once
ollama pull qwen3:4b
uv run eval.py ollama                   # OLLAMA_MODEL=llama3.2:latest for the 2nd baseline
# Jev: needs TYPESAFE_API_KEY in env or in the repo-root .env (never printed or written)
uv run eval.py jev                      # JEV_MODEL=jev-1.14.0 to pin a version
uv run eval.py report                   # writes results/summary.md, prints all tables
```

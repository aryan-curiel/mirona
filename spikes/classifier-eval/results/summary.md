# Classifier eval summary (50 chunks)

## Accuracy per question

| question | jev | ollama-llama3-2-latest | ollama-qwen3-4b |
|---|---|---|---|
| action_item | 0.98 | 0.86 | 0.82 |
| decision | 0.88 | 0.74 | 0.92 |
| open_question | 0.98 | 0.84 | 0.90 |
| small_talk | 0.98 | 0.96 | 0.98 |
| sensitive | 1.00 | 0.90 | 0.90 |
| meeting_type | 0.76 | 0.44 | 0.62 |
| client | 1.00 | 0.90 | 0.96 |
| **mean** | 0.94 | 0.81 | 0.87 |

## Precision / recall (yes = positive)

| question | positives | jev P / R | ollama-llama3-2-latest P / R | ollama-qwen3-4b P / R |
|---|---|---|---|---|
| action_item | 21 | 0.95 / 1.00 | 0.82 / 0.86 | 0.77 / 0.81 |
| decision | 16 | 0.73 / 1.00 | 1.00 / 0.19 | 0.93 / 0.81 |
| open_question | 11 | 0.92 / 1.00 | 1.00 / 0.27 | 1.00 / 0.55 |
| small_talk | 6 | 1.00 / 0.83 | 0.75 / 1.00 | 0.86 / 1.00 |
| sensitive | 11 | 1.00 / 1.00 | 1.00 / 0.55 | 1.00 / 0.55 |

## Latency per call (ms)

| model | p50 | p95 | mean |
|---|---|---|---|
| jev | 190 | 324 | 210 |
| ollama-llama3-2-latest | 2116 | 2313 | 2136 |
| ollama-qwen3-4b | 1783 | 2784 | 2020 |

## Thresholds: jev

conf = max(p, 1-p) for yes/no, probability of the chosen option for choices. Recommended = lowest threshold with >=95% accuracy on answers above it.

| question | recommended | coverage | acc above | coverage@0.60 | acc@0.60 | sweep (t: cov/acc) |
|---|---|---|---|---|---|---|
| action_item | 0.5 | 1.00 | 0.98 | 1.00 | 0.98 | 0.5: 1.00/0.98 0.6: 1.00/0.98 0.7: 0.98/0.98 0.8: 0.92/1.00 0.85: 0.84/1.00 0.9: 0.78/1.00 0.95: 0.48/1.00 |
| decision | 0.95 | 0.40 | 0.95 | 0.98 | 0.88 | 0.5: 1.00/0.88 0.6: 0.98/0.88 0.7: 0.86/0.91 0.8: 0.78/0.90 0.85: 0.76/0.89 0.9: 0.68/0.91 0.95: 0.40/0.95 |
| open_question | 0.5 | 1.00 | 0.98 | 0.98 | 0.98 | 0.5: 1.00/0.98 0.6: 0.98/0.98 0.7: 0.96/0.98 0.8: 0.96/0.98 0.85: 0.90/1.00 0.9: 0.76/1.00 0.95: 0.34/1.00 |
| small_talk | 0.5 | 1.00 | 0.98 | 1.00 | 0.98 | 0.5: 1.00/0.98 0.6: 1.00/0.98 0.7: 0.98/1.00 0.8: 0.96/1.00 0.85: 0.94/1.00 0.9: 0.94/1.00 0.95: 0.90/1.00 |
| sensitive | 0.5 | 1.00 | 1.00 | 0.98 | 1.00 | 0.5: 1.00/1.00 0.6: 0.98/1.00 0.7: 0.96/1.00 0.8: 0.96/1.00 0.85: 0.96/1.00 0.9: 0.92/1.00 0.95: 0.52/1.00 |
| meeting_type | 0.9 | 0.46 | 0.96 | 0.86 | 0.81 | 0.5: 0.98/0.78 0.6: 0.86/0.81 0.7: 0.76/0.82 0.8: 0.64/0.81 0.85: 0.54/0.89 0.9: 0.46/0.96 0.95: 0.32/1.00 0.99: 0.22/1.00 |
| client | 0.5 | 1.00 | 1.00 | 1.00 | 1.00 | 0.5: 1.00/1.00 0.6: 1.00/1.00 0.7: 0.98/1.00 0.8: 0.94/1.00 0.85: 0.92/1.00 0.9: 0.88/1.00 0.95: 0.84/1.00 0.99: 0.72/1.00 |

## Thresholds: ollama-llama3-2-latest

conf = max(p, 1-p) for yes/no, probability of the chosen option for choices. Recommended = lowest threshold with >=95% accuracy on answers above it.

| question | recommended | coverage | acc above | coverage@0.60 | acc@0.60 | sweep (t: cov/acc) |
|---|---|---|---|---|---|---|
| action_item | 0.99 | 0.50 | 1.00 | 0.98 | 0.86 | 0.5: 1.00/0.86 0.6: 0.98/0.86 0.7: 0.96/0.85 0.8: 0.90/0.89 0.85: 0.86/0.88 0.9: 0.78/0.92 0.95: 0.70/0.94 0.99: 0.50/1.00 |
| decision | 0.99 | 0.72 | 0.86 | 0.98 | 0.76 | 0.5: 1.00/0.74 0.6: 0.98/0.76 0.7: 0.96/0.77 0.8: 0.92/0.78 0.85: 0.90/0.80 0.9: 0.90/0.80 0.95: 0.86/0.81 0.99: 0.72/0.86 |
| open_question | 0.85 | 0.82 | 0.98 | 0.96 | 0.85 | 0.5: 1.00/0.84 0.6: 0.96/0.85 0.7: 0.92/0.87 0.8: 0.86/0.93 0.85: 0.82/0.98 0.9: 0.78/1.00 0.95: 0.76/1.00 0.99: 0.68/1.00 |
| small_talk | 0.5 | 1.00 | 0.96 | 1.00 | 0.96 | 0.5: 1.00/0.96 0.6: 1.00/0.96 0.7: 1.00/0.96 0.8: 1.00/0.96 0.85: 1.00/0.96 0.9: 0.98/0.98 0.95: 0.96/1.00 0.99: 0.92/1.00 |
| sensitive | 0.99 | 0.92 | 0.91 | 1.00 | 0.90 | 0.5: 1.00/0.90 0.6: 1.00/0.90 0.7: 1.00/0.90 0.8: 0.98/0.90 0.85: 0.98/0.90 0.9: 0.98/0.90 0.95: 0.96/0.90 0.99: 0.92/0.91 |
| meeting_type | 0.9 | 0.14 | 1.00 | 0.52 | 0.58 | 0.5: 0.76/0.53 0.6: 0.52/0.58 0.7: 0.38/0.74 0.8: 0.22/0.82 0.85: 0.18/0.89 0.9: 0.14/1.00 0.95: 0.06/1.00 |
| client | 0.95 | 0.82 | 0.95 | 0.94 | 0.91 | 0.5: 0.98/0.90 0.6: 0.94/0.91 0.7: 0.90/0.93 0.8: 0.88/0.93 0.85: 0.88/0.93 0.9: 0.86/0.93 0.95: 0.82/0.95 0.99: 0.70/1.00 |

## Thresholds: ollama-qwen3-4b

conf = max(p, 1-p) for yes/no, probability of the chosen option for choices. Recommended = lowest threshold with >=95% accuracy on answers above it.

| question | recommended | coverage | acc above | coverage@0.60 | acc@0.60 | sweep (t: cov/acc) |
|---|---|---|---|---|---|---|
| action_item | 0.99 | 0.54 | 0.96 | 0.96 | 0.83 | 0.5: 1.00/0.82 0.6: 0.96/0.83 0.7: 0.90/0.87 0.8: 0.84/0.90 0.85: 0.82/0.90 0.9: 0.80/0.93 0.95: 0.76/0.92 0.99: 0.54/0.96 |
| decision | 0.99 | 0.78 | 0.95 | 0.98 | 0.94 | 0.5: 1.00/0.92 0.6: 0.98/0.94 0.7: 0.98/0.94 0.8: 0.98/0.94 0.85: 0.98/0.94 0.9: 0.92/0.93 0.95: 0.92/0.93 0.99: 0.78/0.95 |
| open_question | 0.95 | 0.82 | 0.95 | 0.96 | 0.92 | 0.5: 1.00/0.90 0.6: 0.96/0.92 0.7: 0.96/0.92 0.8: 0.84/0.93 0.85: 0.84/0.93 0.9: 0.84/0.93 0.95: 0.82/0.95 0.99: 0.76/1.00 |
| small_talk | 0.5 | 1.00 | 0.98 | 0.98 | 0.98 | 0.5: 1.00/0.98 0.6: 0.98/0.98 0.7: 0.96/0.98 0.8: 0.96/0.98 0.85: 0.96/0.98 0.9: 0.96/0.98 0.95: 0.94/0.98 0.99: 0.94/0.98 |
| sensitive | 0.99 | 0.92 | 0.93 | 1.00 | 0.90 | 0.5: 1.00/0.90 0.6: 1.00/0.90 0.7: 0.98/0.90 0.8: 0.98/0.90 0.85: 0.96/0.92 0.9: 0.96/0.92 0.95: 0.96/0.92 0.99: 0.92/0.93 |
| meeting_type | 0.95 | 0.62 | 0.65 | 0.82 | 0.63 | 0.5: 0.96/0.62 0.6: 0.82/0.63 0.7: 0.82/0.63 0.8: 0.80/0.62 0.85: 0.74/0.62 0.9: 0.70/0.63 0.95: 0.62/0.65 0.99: 0.48/0.58 |
| client | 0.5 | 1.00 | 0.96 | 1.00 | 0.96 | 0.5: 1.00/0.96 0.6: 1.00/0.96 0.7: 0.98/0.98 0.8: 0.98/0.98 0.85: 0.98/0.98 0.9: 0.98/0.98 0.95: 0.98/0.98 0.99: 0.94/1.00 |

## Errors

- **jev** (21): s03.decision=yes(0.65), s04.open_question=yes(0.82), s04.meeting_type=other(0.58), s05.decision=yes(0.91), s05.meeting_type=other(0.40), s07.action_item=yes(0.71), c01.decision=yes(0.90), c02.meeting_type=planning(0.79), c03.meeting_type=planning(0.82), c05.meeting_type=one_on_one(0.86), c06.decision=yes(0.85), c08.meeting_type=planning(0.57), c09.meeting_type=planning(0.63), c10.decision=yes(0.68), c10.meeting_type=one_on_one(0.90), o03.meeting_type=other(0.81), o06.meeting_type=planning(0.82), o10.small_talk=no(0.60), p07.meeting_type=standup(0.56), t05.meeting_type=client_call(0.86), t07.decision=yes(0.96)
- **ollama-llama3-2-latest** (68): s01.action_item=no(0.73), s01.meeting_type=one_on_one(0.53), s02.meeting_type=other(0.44), s03.meeting_type=client_call(0.67), s04.meeting_type=other(0.80), s05.sensitive=no(1.00), s05.meeting_type=one_on_one(0.85), s06.decision=no(1.00), s07.open_question=no(0.78), s07.meeting_type=client_call(0.64), s08.meeting_type=client_call(0.52), s09.meeting_type=one_on_one(0.54), s10.meeting_type=other(0.41), s10.client=none(0.92), c02.action_item=no(0.90), c02.open_question=no(0.70), c03.action_item=yes(0.96), c03.meeting_type=planning(0.87), c04.decision=no(0.95), c05.meeting_type=one_on_one(0.72), c05.client=none(0.98), c08.open_question=no(0.57), c08.meeting_type=one_on_one(0.58), c09.decision=no(0.51), c09.meeting_type=planning(0.38), c09.client=none(0.62), c10.small_talk=yes(0.94), c10.sensitive=no(0.98), c10.meeting_type=other(0.50), c10.client=none(0.97), o01.sensitive=no(0.99), o02.open_question=no(0.89), o03.meeting_type=other(0.62), o04.decision=no(0.70), o04.small_talk=yes(0.85), o04.sensitive=no(1.00), o06.action_item=yes(0.77), o06.decision=no(0.97), o06.meeting_type=other(0.35), o08.action_item=no(0.97), o08.meeting_type=other(0.44), o09.decision=no(0.64), o09.meeting_type=client_call(0.67), p01.decision=no(0.85), p02.open_question=no(0.69), p02.meeting_type=other(0.52), p04.decision=no(0.98), p05.action_item=yes(0.90), p06.decision=no(0.99), p06.meeting_type=one_on_one(0.39), p06.client=none(0.52), p07.meeting_type=other(0.58), p08.action_item=yes(0.86), p08.decision=no(0.94), p09.open_question=no(0.81), p09.meeting_type=client_call(0.36), t01.decision=no(0.99), t01.meeting_type=planning(0.72), t02.decision=no(1.00), t03.meeting_type=one_on_one(0.65), t04.open_question=no(0.80), t04.meeting_type=client_call(0.49), t05.meeting_type=client_call(0.52), t06.decision=no(0.99), t06.meeting_type=one_on_one(0.49), t08.sensitive=no(1.00), t08.meeting_type=one_on_one(0.63), t10.open_question=no(0.78)
- **ollama-qwen3-4b** (45): s02.open_question=no(0.97), s03.meeting_type=one_on_one(0.53), s04.meeting_type=other(1.00), s05.action_item=no(0.76), s05.sensitive=no(1.00), s05.meeting_type=other(0.98), s06.decision=no(0.58), s07.meeting_type=other(0.88), s08.action_item=yes(0.51), s08.sensitive=no(0.84), s09.action_item=no(0.62), c02.open_question=no(0.92), c03.action_item=yes(1.00), c03.meeting_type=planning(1.00), c05.meeting_type=other(1.00), c07.open_question=no(0.59), c08.meeting_type=one_on_one(0.40), c09.meeting_type=planning(0.99), c10.meeting_type=other(0.99), o01.decision=yes(1.00), o03.meeting_type=other(1.00), o04.action_item=no(0.96), o04.decision=no(0.98), o04.small_talk=yes(1.00), o04.sensitive=no(0.98), o04.meeting_type=other(0.82), o05.meeting_type=planning(0.91), o07.action_item=yes(0.75), o08.action_item=no(0.97), o08.meeting_type=client_call(0.54), o08.client=northwind_bank(0.69), o10.meeting_type=other(1.00), p07.meeting_type=other(1.00), p08.action_item=yes(0.87), p08.sensitive=no(1.00), t01.meeting_type=planning(1.00), t02.decision=no(1.00), t04.open_question=no(0.96), t04.meeting_type=standup(0.54), t05.meeting_type=client_call(0.95), t05.client=northwind_bank(0.96), t08.sensitive=no(1.00), t08.meeting_type=one_on_one(1.00), t10.action_item=yes(0.64), t10.open_question=no(0.72)

---
name: mirona
description: Use the user's meeting memory (Mirona MCP tools search_meetings, ask_meeting, get_meeting, get_transcript, list_action_items) whenever a request depends on what was said in their calls - preparing for an upcoming meeting with a client or person, finding what a client agreed, decided or asked for, checking who owns a follow-up, or turning meeting action items into tasks, tickets or a to-do list.
---

# Mirona meeting memory

Mirona records and transcribes the user's meetings locally and keeps a summary, decisions,
open questions and action items for each one. Its MCP server (`mirona`) gives you read access;
write tools exist only if the user turned them on. Everything stays on the user's machine;
quote it back only to the user.

## Tools

| Tool | Use it for | Returns |
|---|---|---|
| `search_meetings(query, client?, from?, to?, speaker?, tag?)` | Finding meetings and moments by keywords and meaning (client, person, topic). Filter by client, dates (`YYYY-MM-DD`), speaker or tag (`action`, `decision`, `open_question`, `small_talk`). Empty query lists the meetings that pass the filters, newest first. | `moments` (`meeting_id`, title, date, `at`, speaker, snippet with keyword hits in `[brackets]`) and `meetings` (`meeting_id`, title, client, date) |
| `ask_meeting(question, meeting_id?)` | A plain question when you don't know which tool fits. Lookups come straight from the meetings; only open questions ("why", "how", advice) go to the user's LLM. | `route` plus moments, decisions, action items, a meeting digest, or an `answer` citing `[m:ss]` |
| `get_meeting(id)` | The digest of one meeting | overview, decisions, open questions, action items, notes, enhanced notes, speakers, each with an `at` timestamp; `unconfirmed` guesses |
| `get_transcript(id, from?, to?)` | Exact wording. Pass a range (`"6:30"`, `"7:30"`) around a search hit | `[m:ss] Speaker: text` lines |
| `list_action_items(client?, open_only?)` | Follow-ups across meetings | `id`, text, owner, due, done, client, meeting, `at`, `needs_review` |

Search matches keywords (every word prefix-matched) and, when the user's local embedding model
runs, meaning; `search_by_meaning: false` means keywords only, so then try the client name, a
distinctive word or synonyms. Prefer `get_meeting` over `get_transcript`; pull the transcript
only for the part you need to quote.

Anything with `needs_review: true` or listed under `unconfirmed` is the classifier's guess:
say so, don't state it as fact. Lines shown as `[redacted]` are sensitive; don't ask for them
with `include_sensitive` unless the user explicitly wants that content.

## Preparing for a meeting

1. `search_meetings` with the client or person's name; pick the most recent 2-3 meetings.
2. `get_meeting` on each.
3. `list_action_items(client=..., open_only=true)` for what is still owed, by whom.
4. Brief the user: where things stand, decisions already made, open questions to raise, open
   action items (theirs first), with meeting dates. Keep it to one screen.

## Finding what a client agreed

1. `search_meetings` with the client plus the topic ("Corvo pricing", "Acme deadline"), or
   `tag="decision"` and `client=...`.
2. `get_meeting` on the matches and look at `decisions`.
3. For anything the user may rely on (a price, a date, a commitment), confirm the exact words
   with `get_transcript(id, from, to)` around the timestamp and quote them with the meeting
   date and `[m:ss]`. Say so if the record is ambiguous or only one side said it.

## Turning action items into tasks

1. `list_action_items(open_only=true)`, filtered by client if the user named one.
2. Write one task per item: imperative title, owner, due date (as said; ask or leave blank if
   none), and a source line "From <meeting> (<date>) at <m:ss>".
3. If the user has a task tool connected (Linear, Jira, Todoist...), offer to create them there;
   otherwise give a checklist.
4. Don't mark items done in Mirona unless the user asks. `set_action_item_done(id, done)` and
   `rename_speaker(meeting_id, speaker_id, name)` exist only when the user enabled write tools
   (Settings › Agents (MCP) › Write tools); if they're missing, say where to turn them on.

## When the tools fail

- "The Mirona MCP server is turned off": the user disabled it; it's in Mirona's
  Settings › Agents (MCP) › MCP server.
- 401 / "Missing or unknown token" (HTTP): the client's token was revoked or never set; a new one
  comes from Settings › Agents (MCP) › Client tokens.
- No results: say what you searched for; don't invent meetings or quotes.
- Timestamps are from the start of each recording, not wall-clock time.

---
name: mirona
description: Use the user's meeting memory (Mirona MCP tools search_meetings, get_meeting, get_transcript, list_action_items) whenever a request depends on what was said in their calls - preparing for an upcoming meeting with a client or person, finding what a client agreed, decided or asked for, checking who owns a follow-up, or turning meeting action items into tasks, tickets or a to-do list.
---

# Mirona meeting memory

Mirona records and transcribes the user's meetings locally and keeps a summary, decisions,
open questions and action items for each one. Its MCP server (`mirona`) gives you read-only
access. Everything stays on the user's machine; quote it back only to the user.

## Tools

| Tool | Use it for | Returns |
|---|---|---|
| `search_meetings(query)` | Finding meetings and moments by keyword (client, person, topic). Empty query lists all meetings, newest first. | `moments` (meeting, `at` timestamp, snippet with hits in `[brackets]`) and `meetings` (id, title, client, date) |
| `get_meeting(id)` | The digest of one meeting | overview, decisions, open questions, action items, notes, each with an `at` timestamp |
| `get_transcript(id, from?, to?)` | Exact wording. Pass a range (`"6:30"`, `"7:30"`) around a search hit | `[m:ss] Speaker: text` lines |
| `list_action_items(client?, open_only?)` | Follow-ups across meetings | text, owner, due, done, client, meeting |

Search is keyword-based (every word prefix-matched), not semantic: search for the client
name or a distinctive word, then try synonyms if nothing comes back. Prefer `get_meeting` over
`get_transcript`; pull the transcript only for the part you need to quote.

## Preparing for a meeting

1. `search_meetings` with the client or person's name; pick the most recent 2-3 meetings.
2. `get_meeting` on each.
3. `list_action_items(client=..., open_only=true)` for what is still owed, by whom.
4. Brief the user: where things stand, decisions already made, open questions to raise, open
   action items (theirs first), with meeting dates. Keep it to one screen.

## Finding what a client agreed

1. `search_meetings` with the client plus the topic ("Corvo pricing", "Acme deadline").
2. `get_meeting` on the matches and look at `decisions`.
3. For anything the user may rely on (a price, a date, a commitment), confirm the exact words
   with `get_transcript(id, from, to)` around the timestamp and quote them with the meeting
   date and `[m:ss]`. Say so if the record is ambiguous or only one side said it.

## Turning action items into tasks

1. `list_action_items(open_only=true)`, filtered by client if the user named one.
2. Write one task per item: imperative title, owner, due date (as said; ask or leave blank if
   none), and a source line "From <meeting> (<date>) at <m:ss>".
3. If the user has a task tool connected (Linear, Jira, Todoist...), offer to create them there;
   otherwise give a checklist. Don't mark anything done: these tools are read-only.

## When the tools fail

- "MCP server is turned off": the user disabled it in Mirona's settings; tell them where.
- No results: say what you searched for; don't invent meetings or quotes.
- Timestamps are from the start of each recording, not wall-clock time.

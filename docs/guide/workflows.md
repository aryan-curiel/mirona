# Common workflows

How to set Mirona up for the meetings you have most. Each workflow lists the settings that matter
and what to do before, during and after the call.

## Client calls

You talk to several clients and need to remember what each one agreed.

**Set up once**

1. Settings › Tags: add one tag per client, e.g. `project:Alder Restoration`,
   `project:Juniper Apparel`.
2. Settings › General › Call detection: turn on **Use calendar event titles**, so calls are named
   after the calendar event and tagged when the event title names the client.
3. Edit the **Client call** template if your summaries need other sections, e.g. *Scope changes*
   or *Pricing*.

**Each call**

1. Accept the banner when the call starts, or press <kbd>⌘⌥⇧R</kbd>.
2. In the consent reminder, pick the client's tag and paste the message in the chat.
3. After the call, answer **To check**, then open **Follow-up email** › **Copy** and send it from
   your mail app.

**Before the next call**

- Open the client's last meeting, or search with **Filters › meeting tag**.
- **Action items** › **All tags** › the client: what you and they still owe.
- Or ask your agent: *"What's still open with Alder Restoration?"* (see [AI agents](agents.md)).

## Daily standups

Short, frequent, with the same people every day.

1. Add a tag for the team (`team:platform`) and, in the consent reminder, tick **They agreed:
   don't remind me again for team:platform calls**.
2. Turn on **Use calendar event titles** and tick `team:platform` under *Record without asking*,
   if the standup's calendar event title names the team. The standup then records on its own.
3. Name each voice once after the first standup. From then on, every line has a name.
4. Use the **Action items** page, filtered to the team, as the running to-do list.

## One-on-ones

1. Leave the recording untagged, or add a tag per report (`1on1:sam`).
2. The **1:1** template is picked automatically. Write notes in the **Notes** tab during the call:
   the enhanced notes link each one to the moment it came up.
3. Before the next 1:1, search the tag for what was promised: `Filters › meeting tag: 1on1:sam`,
   line tag: action.
4. Sensitive topics (salary, health) are tagged sensitive and stay out of summaries and agents.
   For these calls, keep privacy mode on.

## Interviews and user research

1. Create a template for it, e.g. `user-research.md` with *Participant*, *Pain points*, *Quotes*
   and *Ideas* sections (see [Your own templates](summaries.md#your-own-templates)).
2. Tag each study (`study:onboarding`). After a round of interviews, search the tag by meaning:
   "confusing", "gave up", "workaround".
3. Use **Who said this?** to fix any lines that went to the wrong person, so quotes are attributed
   correctly.

## In-person meetings and voice memos

- **Meeting in the room, Mac on the table:** just record. Everything comes through your mic, so
  every line is *You*; the summary and action items still work.
- **Want each person told apart?** Record on your phone instead, then use **Import audio…**: the
  imported file is split into speakers like the other side of a call.
- **Voice memos and lectures:** import them (m4a, mp3, wav, flac, aac, mp4) to get a transcript,
  a summary and search.

## A weekly review with your agent

With [agents](agents.md) turned on, a Friday review takes one prompt:

> "Look at all my meetings from this week. For each project tag, list what was decided, what I
> promised and by when, and anything still open. Then draft a short status update for each
> client."

Your agent calls `search_meetings`, `get_meeting` and `list_action_items`, and builds the review
from them. Sensitive lines read `[redacted]`.

## Scripting with the CLI

The `mirona` command reads the same meetings (see [CLI and MCP](../cli-and-mcp.md)):

```bash
mirona list                                   # every meeting, newest first
mirona actions --open --tag project:          # open action items for every project
mirona search "pricing" --meeting-tag project:corvo --from 2026-10-01
mirona show m-1762763400000 > claims-desk.md  # a meeting as text (ids from `mirona list`)
mirona import ~/Downloads/interview.m4a       # transcribe and summarize a file
```

Read commands never write the app's index, so they're safe to run while the app is open.

## Keeping meetings in your notes app

Each meeting is a folder with a `meeting.md`: YAML front matter, your notes, then the summary,
decisions, action items and enhanced notes as plain Markdown. Point an Obsidian vault (or any
Markdown tool) at the meetings folder and every meeting shows up as a note. See
[Files and folders](files.md).

Treat these notes as read-only in your notes app: Mirona rewrites `meeting.md` whenever the
meeting changes. Write your own notes in Mirona's Notes tab, or link to the meeting from a note of
your own.

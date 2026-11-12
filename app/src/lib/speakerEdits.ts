import type { Meeting } from "../bindings/Meeting";
import type { Segment } from "../bindings/Segment";
import type { SpeakerEdit } from "../bindings/SpeakerEdit";

/**
 * Speaker changes waiting for Save (NGR-201), for one meeting. A voice edit names every line of
 * that voice except the ones with a line edit of their own; a line edit always wins.
 */
export type Pending = { meetingId: string; speakers: Record<string, string>; lines: Record<string, string> };

export const pendingCount = (p: Pending | null) => (p ? Object.keys(p.speakers).length + Object.keys(p.lines).length : 0);

const tidy = (p: Pending): Pending | null => (pendingCount(p) ? p : null);

const into = (p: Pending | null, meetingId: string): Pending => (p?.meetingId === meetingId ? p : { meetingId, speakers: {}, lines: {} });

const omit = <T,>(rec: Record<string, T>, key: string) => {
  const { [key]: _, ...rest } = rec;
  return rest;
};

/** Name a voice everywhere it speaks. `original` is the name it has now: naming it that again undoes the edit. */
export function editVoice(p: Pending | null, meetingId: string, speakerId: string, original: string, name: string): Pending | null {
  const base = into(p, meetingId);
  const n = name.trim();
  const speakers = !n || n === original ? omit(base.speakers, speakerId) : { ...base.speakers, [speakerId]: n };
  return tidy({ ...base, speakers });
}

/** Move one line to `name`. Matching what the line shows without this edit (its voice's pending name, else `original`) undoes it. */
export function editLine(p: Pending | null, meetingId: string, segmentId: string, speakerId: string, original: string, name: string): Pending | null {
  const base = into(p, meetingId);
  const n = name.trim();
  const shown = base.speakers[speakerId] ?? original;
  const lines = !n || n === shown ? omit(base.lines, segmentId) : { ...base.lines, [segmentId]: n };
  return tidy({ ...base, lines });
}

export const undoVoice = (p: Pending | null, speakerId: string) => (p ? tidy({ ...p, speakers: omit(p.speakers, speakerId) }) : null);

export const undoLine = (p: Pending | null, segmentId: string) => (p ? tidy({ ...p, lines: omit(p.lines, segmentId) }) : null);

/** What a line is called once saved: its own edit, else its voice's edit; `null` when unchanged. */
export function renamedLine(p: Pending | null, meetingId: string, seg: Segment): { to: string; by: "line" | "voice" } | null {
  if (p?.meetingId !== meetingId) return null;
  const line = p.lines[seg.id];
  if (line) return { to: line, by: "line" };
  const voice = p.speakers[seg.speakerId];
  return voice ? { to: voice, by: "voice" } : null;
}

/** Old → new name of every renamed voice, to preview in the summary and action items. */
export function voiceRenames(p: Pending | null, m: Meeting): [from: string, to: string][] {
  if (p?.meetingId !== m.id) return [];
  return m.speakers.flatMap((s) => (p.speakers[s.id] ? [[s.name, p.speakers[s.id]] as [string, string]] : []));
}

/** The batch `apply_speaker_edits` takes. */
export const toEdits = (p: Pending): SpeakerEdit[] => [
  ...Object.entries(p.speakers).map(([speakerId, name]): SpeakerEdit => ({ scope: "speaker", speakerId, name })),
  ...Object.entries(p.lines).map(([segmentId, name]): SpeakerEdit => ({ scope: "line", segmentId, name })),
];

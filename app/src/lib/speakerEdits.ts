import type { Meeting } from "../bindings/Meeting";
import type { Segment } from "../bindings/Segment";
import type { SpeakerEdit } from "../bindings/SpeakerEdit";

/**
 * Speaker changes waiting for Save (NGR-201), for one meeting. `speakers` names a voice on
 * every line of it; `lines` moves one line to a speaker id of the meeting or a name. A line's
 * own change always wins over its voice's.
 */
export type Pending = { meetingId: string; speakers: Record<string, string>; lines: Record<string, string> };

export const pendingCount = (p: Pending | null) => (p ? Object.keys(p.speakers).length + Object.keys(p.lines).length : 0);

const tidy = (p: Pending): Pending | null => (pendingCount(p) ? p : null);

const into = (p: Pending | null, meetingId: string): Pending => (p?.meetingId === meetingId ? p : { meetingId, speakers: {}, lines: {} });

const omit = <T,>(rec: Record<string, T>, key: string) => {
  const { [key]: _, ...rest } = rec;
  return rest;
};

const same = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();

/** What a speaker id of the meeting (or a plain name) is called once saved. */
export function nameOf(p: Pending | null, m: Meeting, idOrName: string): string {
  const sp = m.speakers.find((s) => s.id === idOrName);
  if (!sp) return idOrName;
  return (p?.meetingId === m.id && p.speakers[sp.id]) || sp.name;
}

/** Name a voice everywhere it speaks. `original` is the name it has now: naming it that again undoes the edit. */
export function editVoice(p: Pending | null, meetingId: string, speakerId: string, original: string, name: string): Pending | null {
  const base = into(p, meetingId);
  const n = name.trim();
  const speakers = !n || n === original ? omit(base.speakers, speakerId) : { ...base.speakers, [speakerId]: n };
  return tidy({ ...base, speakers });
}

/** Move one line to `to` (a speaker id or a name). Moving it to what it shows without this edit undoes it. */
export function editLine(p: Pending | null, m: Meeting, seg: Segment, to: string): Pending | null {
  const base = into(p, m.id);
  const t = to.trim();
  const undo = !t || t === seg.speakerId || same(nameOf(base, m, t), nameOf(base, m, seg.speakerId));
  return tidy({ ...base, lines: undo ? omit(base.lines, seg.id) : { ...base.lines, [seg.id]: t } });
}

export const undoVoice = (p: Pending | null, speakerId: string) => (p ? tidy({ ...p, speakers: omit(p.speakers, speakerId) }) : null);

export const undoLine = (p: Pending | null, segmentId: string) => (p ? tidy({ ...p, lines: omit(p.lines, segmentId) }) : null);

/** What a line is called once saved: its own edit, else its voice's edit; `null` when unchanged. */
export function renamedLine(p: Pending | null, m: Meeting, seg: Segment): { to: string; by: "line" | "voice" } | null {
  if (p?.meetingId !== m.id) return null;
  const line = p.lines[seg.id];
  if (line) return { to: nameOf(p, m, line), by: "line" };
  const voice = p.speakers[seg.speakerId];
  return voice ? { to: voice, by: "voice" } : null;
}

/**
 * The voice a line sits with once saved, for Split voices: a speaker id, or `name:<name>` for a
 * person who isn't a voice of this meeting yet.
 */
export function lineVoice(p: Pending | null, m: Meeting, seg: Segment): string {
  const to = p?.meetingId === m.id ? p.lines[seg.id] : undefined;
  if (!to) return seg.speakerId;
  const sp = m.speakers.find((s) => s.id === to || same(s.name, to) || same(nameOf(p, m, s.id), to));
  return sp ? sp.id : `name:${to}`;
}

/** Old → new name of every renamed voice that keeps a line, to preview in the summary and action items. */
export function voiceRenames(p: Pending | null, m: Meeting): [from: string, to: string][] {
  if (p?.meetingId !== m.id) return [];
  const kept = new Set(m.segments.filter((x) => !p.lines[x.id]).map((x) => x.speakerId));
  return m.speakers.flatMap((s) => (p.speakers[s.id] && kept.has(s.id) ? [[s.name, p.speakers[s.id]] as [string, string]] : []));
}

/** The batch `apply_speaker_edits` takes. */
export const toEdits = (p: Pending): SpeakerEdit[] => [
  ...Object.entries(p.speakers).map(([speakerId, name]): SpeakerEdit => ({ scope: "speaker", speakerId, name })),
  ...Object.entries(p.lines).map(([segmentId, name]): SpeakerEdit => ({ scope: "line", segmentId, name })),
];

/** Pure helpers for pickers (NGR-201/202); self-check in match.check.ts. */
import type { Segment } from "../bindings/Segment";

/** Hidden behind "Show sensitive text": redacted or tagged sensitive. */
export const isHidden = (seg: Segment, showSensitive: boolean) =>
  (seg.redacted || seg.tags.some((t) => t.kind === "sensitive")) && !showSensitive;

/** 0 exact, 1 prefix (of the item or of its part after ":"), 2 substring, 3 letters in order. */
function score(q: string, item: string): number | null {
  const s = item.toLowerCase();
  if (s === q) return 0;
  if (s.startsWith(q) || s.slice(s.indexOf(":") + 1).startsWith(q)) return 1;
  if (s.includes(q)) return 2;
  let i = 0;
  for (const c of s) if (c === q[i]) i++;
  return i === q.length ? 3 : null;
}

/** Items matching `query` anywhere, best first (then shorter, then A–Z). */
export function rank(query: string, items: string[], limit = 8): string[] {
  const q = query.trim().toLowerCase();
  if (!q) return items.slice(0, limit);
  return items
    .map((item) => ({ item, s: score(q, item) }))
    .filter((x): x is { item: string; s: number } => x.s !== null)
    .sort((a, b) => a.s - b.s || a.item.length - b.item.length || a.item.localeCompare(b.item))
    .slice(0, limit)
    .map((x) => x.item);
}

/** Up to 3 of a voice's clearest lines: the longest, at least 2 s, a minute apart, in order. */
export function samples(segs: Segment[], speakerId: string, showSensitive: boolean): Segment[] {
  const len = (s: Segment) => (s.endMs > s.startMs ? s.endMs - s.startMs : s.text.length * 60);
  const mine = segs.filter((s) => s.speakerId === speakerId && !isHidden(s, showSensitive));
  const long = mine.filter((s) => len(s) >= 2000);
  const pool = (long.length ? long : mine).sort((a, b) => len(b) - len(a));
  const picked: Segment[] = [];
  for (const s of pool) {
    if (picked.length === 3) break;
    if (picked.every((p) => Math.abs(p.startMs - s.startMs) >= 60_000)) picked.push(s);
  }
  return picked.sort((a, b) => a.startMs - b.startMs);
}

/** Chip class for a tag: one of 8 colours per group (FNV-1a of the group name), neutral without one. */
export function tagClass(tag: string): string {
  const i = tag.indexOf(":");
  if (i < 0) return "mi-chip--neutral";
  let h = 0x811c9dc5;
  for (const c of tag.slice(0, i).trim().toLowerCase()) h = Math.imul(h ^ c.charCodeAt(0), 0x01000193) >>> 0;
  return `mi-chip--tag-${1 + (((h ^ (h >>> 16)) >>> 0) % 8)}`;
}

/** The item equal to `typed`, ignoring case. */
export const exactMatch = (typed: string, items: string[]) => {
  const t = typed.trim().toLowerCase();
  return t ? items.find((i) => i.toLowerCase() === t) : undefined;
};

/** What Enter picks in a picker: the highlighted row, an existing item typed in any case, the
 * typed text when new ones are allowed, or the only match. Nothing for empty text. */
export function enterChoice(typed: string, shown: string[], all: string[], active: string | undefined, canNew: boolean): string | null {
  if (active !== undefined) return active;
  const t = typed.trim();
  if (!t) return null;
  return exactMatch(t, all) ?? (canNew ? t : shown.length === 1 ? shown[0] : null);
}

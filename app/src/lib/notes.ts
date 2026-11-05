/**
 * Per-line note times (NGR-144): after an edit, lines that stayed keep their time, an edited
 * line keeps the time it was first written, and new lines get `now` (the meeting time, or
 * null outside a recording). Blank lines never carry a time.
 *
 * Unchanged lines are found as the common prefix and suffix of the old and new text; the
 * middle is the edit. ponytail: a multi-line paste over several lines re-stamps them; a real
 * line diff if that ever matters.
 */
export function retime(oldLines: string[], oldTimes: (number | null)[], newLines: string[], now: number | null): (number | null)[] {
  const t = (i: number) => oldTimes[i] ?? null;
  let pre = 0;
  while (pre < oldLines.length && pre < newLines.length && oldLines[pre] === newLines[pre]) pre++;
  let suf = 0;
  while (
    suf < oldLines.length - pre &&
    suf < newLines.length - pre &&
    oldLines[oldLines.length - 1 - suf] === newLines[newLines.length - 1 - suf]
  )
    suf++;
  const oldMid = oldLines.length - pre - suf;
  return newLines.map((line, i) => {
    if (!line.trim()) return null;
    if (i < pre) return t(i);
    if (i >= newLines.length - suf) return t(oldLines.length - (newLines.length - i));
    const k = i - pre; // position inside the edited middle
    return k < oldMid && oldLines[pre + k].trim() ? (t(pre + k) ?? now) : now;
  });
}

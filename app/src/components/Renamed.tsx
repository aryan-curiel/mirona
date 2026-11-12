import type { MouseEvent, ReactNode } from "react";

/** A pending speaker change. Undo is its own button: a click on the name must never drop a change. */
export function Renamed({ from, to, onUndo, undoHint, onEdit }: { from: string; to: string; onUndo?: () => void; undoHint?: string; onEdit?: (e: MouseEvent) => void }) {
  return (
    <span className="inline-flex items-baseline gap-px">
      <del className="mi-rename-old">{from}</del>
      {onEdit ? (
        <button type="button" className="mi-rename-new" title="Rename again" aria-label={`Rename ${to}`} onClick={onEdit}>
          {to}
        </button>
      ) : (
        <ins className="mi-rename-new">{to}</ins>
      )}
      {onUndo && (
        <button
          type="button"
          className="ml-1 rounded px-1 text-[14px] leading-none text-muted hover:text-ink"
          title={undoHint ?? "Undo this change"}
          aria-label={`Undo: ${from} to ${to}`}
          onClick={(e) => {
            e.stopPropagation();
            onUndo();
          }}
        >
          ×
        </button>
      )}
    </span>
  );
}

const escapeRegExp = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** `text` with every whole-word old name shown as `~~old~~new` (a preview of what Save rewrites). */
export function withRenames(text: string, renames: [from: string, to: string][]): ReactNode {
  if (!renames.length) return text;
  const to = new Map(renames);
  const word = new RegExp(`(?<![\\p{L}\\p{N}])(${renames.map(([from]) => escapeRegExp(from)).join("|")})(?![\\p{L}\\p{N}])`, "gu");
  return text.split(word).map((part, i) =>
    // split() with one capture group puts every match at an odd index.
    // biome-ignore lint/suspicious/noArrayIndexKey: a static split of one string, position is the identity
    i % 2 ? <Renamed key={i} from={part} to={to.get(part) ?? part} /> : part,
  );
}

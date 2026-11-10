import { type ReactNode, useId, useState } from "react";
import { rank } from "../lib/match";

export type ComboOption = { value: string; group?: string; render?: ReactNode };

/**
 * A text field with suggestions that match anywhere (NGR-201/202). ↑/↓ move, Enter picks the
 * highlighted one (or the typed text when `allowNew` labels it), Escape closes.
 */
export function Combobox({ label, placeholder, options, onPick, allowNew, initial = "", autoFocus = false, className = "" }: {
  label: string;
  placeholder: string;
  options: ComboOption[];
  onPick: (value: string) => void;
  /** Label for "use what I typed" (e.g. "New person: Zoe"), or null when it can't be used. */
  allowNew?: (typed: string) => string | null;
  initial?: string;
  autoFocus?: boolean;
  className?: string;
}) {
  const id = useId();
  const [text, setText] = useState(initial);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const byValue = new Map(options.map((o) => [o.value, o]));
  const hits = rank(text, [...byValue.keys()]).map((v) => byValue.get(v) as ComboOption);
  // Keep the caller's group order; matches stay best-first inside each group.
  const groups = [...new Set(options.map((o) => o.group ?? ""))];
  const shown = groups.flatMap((g) => hits.filter((o) => (o.group ?? "") === g));
  const typed = text.trim();
  const newLabel = typed && !byValue.has(typed) ? (allowNew?.(typed) ?? null) : null;
  const rows = [...shown.map((o) => ({ value: o.value, o })), ...(newLabel ? [{ value: typed, o: null }] : [])];

  const pick = (value: string) => {
    setText("");
    setOpen(false);
    setActive(-1);
    onPick(value);
  };

  return (
    <div className={`relative ${className}`}>
      <input
        // biome-ignore lint/a11y/noAutofocus: opened by the user's own click
        autoFocus={autoFocus}
        role="combobox"
        aria-label={label}
        aria-expanded={open && rows.length > 0}
        aria-controls={`${id}-list`}
        aria-autocomplete="list"
        aria-activedescendant={active >= 0 ? `${id}-${active}` : undefined}
        className="mi-field h-8 w-full text-[13px]"
        placeholder={placeholder}
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          setOpen(true);
          setActive(-1);
        }}
        onFocus={() => setOpen(true)}
        onBlur={() => setOpen(false)}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            setOpen(true);
            const step = e.key === "ArrowDown" ? 1 : -1;
            setActive((a) => (rows.length ? (a + step + rows.length) % rows.length : -1));
          } else if (e.key === "Enter") {
            e.preventDefault();
            if (active >= 0 && rows[active]) pick(rows[active].value);
            else if (newLabel) pick(typed);
            else if (shown.length === 1) pick(shown[0].value);
          } else if (e.key === "Escape") {
            setOpen(false);
            setActive(-1);
          }
        }}
      />
      {open && rows.length > 0 && (
        <div id={`${id}-list`} role="listbox" aria-label={label} className="mi-card absolute left-0 right-0 z-20 mt-1 max-h-64 overflow-auto p-1 text-[13px]">
          {rows.map((r, i) => {
            const head = r.o?.group && (i === 0 || rows[i - 1].o?.group !== r.o.group) ? r.o.group : null;
            return (
              <div key={`${r.o ? "o" : "n"}-${r.value}`} role="presentation">
                {head && <div className="px-2 pt-1.5 pb-0.5 text-[11px] text-muted">{head}</div>}
                <div
                  id={`${id}-${i}`}
                  role="option"
                  tabIndex={-1}
                  aria-selected={i === active}
                  className={`cursor-pointer rounded-md px-2 py-1.5 ${i === active ? "bg-[var(--mi-accent-tint)]" : "hover:bg-[var(--mi-neutral-tint)]"}`}
                  // Pick before the input's blur closes the list.
                  onMouseDown={(e) => {
                    e.preventDefault();
                    pick(r.value);
                  }}
                >
                  {r.o ? (r.o.render ?? r.o.value) : newLabel}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

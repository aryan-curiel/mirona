import { useEffect, useState } from "react";
import type { Meeting } from "../bindings/Meeting";
import { type ComboOption, Combobox } from "../components/Combobox";
import { Mono } from "../components/ui";
import { api, fmt } from "../lib/api";
import { exactMatch, samples } from "../lib/match";
import { useStore } from "../store";

/** Voices nobody named yet and everyone who has a name; refetched whenever the meeting changes. */
export function useVoices(m: Meeting) {
  const [unnamed, setUnnamed] = useState<string[]>([]);
  const [known, setKnown] = useState<string[]>([]);
  useEffect(() => {
    let off = false;
    api.unnamedSpeakers(m.id).then((u) => !off && setUnnamed(u), () => {});
    api.knownNames().then((k) => !off && setKnown(k), () => {});
    return () => {
      off = true;
    };
  }, [m]);
  return { unnamed, known };
}

/** People to pick from: named people heard in this meeting first, then everyone else. */
export function personOptions(m: Meeting, known: string[]): ComboOption[] {
  const here = new Set(m.speakers.filter((p) => p.id !== "me").map((p) => p.name));
  return [
    ...known.filter((n) => here.has(n)).map((value) => ({ value, group: "In this meeting" })),
    ...known.filter((n) => !here.has(n)).map((value) => ({ value, group: "Everyone else" })),
  ];
}

export const newPerson = (t: string) => (t.trim() ? `New person: ${t.trim()}` : null);

const talk = (ms: number) => `${Math.floor(ms / 60_000)}:${String(Math.floor(ms / 1000) % 60).padStart(2, "0")}`;

/** After speaker detection: name the voices nobody named, one at a time, from their clearest lines (NGR-201). */
export function NameVoices({ m, unnamed, known, play }: { m: Meeting; unnamed: string[]; known: string[]; play?: (ms: number) => void }) {
  const editSpeaker = useStore((s) => s.editSpeaker);
  const showSensitive = useStore((s) => s.filters.showSensitive);
  const busy = useStore((s) => s.recording?.meetingId === m.id || s.processing.includes(m.id));
  const [open, setOpen] = useState(false);
  const [at, setAt] = useState(0);
  const [text, setText] = useState("");
  if (busy || !unnamed.length) return null;
  const n = unnamed.length;
  const k = Math.min(at, n - 1);
  const sp = m.speakers.find((p) => p.id === unnamed[k]);
  const lines = m.segments.filter((x) => x.speakerId === sp?.id);
  const ms = lines.reduce((t, x) => t + Math.max(0, x.endMs - x.startMs), 0);
  const next = () => {
    setText("");
    if (k + 1 < n) setAt(k + 1);
    else setOpen(false);
  };
  const save = (raw: string) => {
    const name = exactMatch(raw, known) ?? raw.trim();
    if (!sp || !name) return;
    editSpeaker(sp.id, name);
    setText("");
    // The named voice leaves the list, so the next one moves into this slot.
    if (k + 1 >= n) setOpen(false);
  };

  return (
    <section aria-label="Unnamed voices" className="mb-4 rounded-lg border border-dashed border-[var(--mi-review-edge)] bg-[var(--mi-review-tint)] p-3 text-[14px]">
      <div className="flex items-center gap-3">
        <span className="flex-1">
          {n} {n === 1 ? "voice isn't" : "voices aren't"} named yet.
        </span>
        {!open && (
          <button type="button" className="mi-btn-primary h-9 px-3.5 text-[13px]" onClick={() => {
              setAt(0);
              setOpen(true);
            }}
          >
            Name them
          </button>
        )}
      </div>
      {open && sp && (
        <div className="mi-card mt-3 p-4">
          <h3 className="text-[15px] font-semibold">
            Voice {k + 1} of {n} — {sp.name}
          </h3>
          <p className="text-[13px] text-muted">
            {lines.length} line{lines.length === 1 ? "" : "s"}
            {ms > 0 && ` · ${talk(ms)}`}
          </p>
          <ul className="mt-2 flex flex-col gap-1.5">
            {samples(m.segments, sp.id, showSensitive).map((x) => (
              <li key={x.id} className="flex items-start gap-2 text-[14px]">
                <button
                  type="button"
                  disabled={!play}
                  aria-label={`Play line at ${fmt(x.startMs)}`}
                  className="mi-ghost h-8 shrink-0 px-2 disabled:opacity-50"
                  onClick={() => play?.(x.startMs)}
                >
                  <svg width="9" height="10" viewBox="0 0 9 10" aria-hidden="true">
                    <path d="M0 0l9 5-9 5z" fill="currentColor" />
                  </svg>
                </button>
                <Mono>{fmt(x.startMs)}</Mono>
                <span className="text-ink-2">“{x.text}”</span>
              </li>
            ))}
          </ul>
          <Combobox
            key={sp.id}
            className="mt-3"
            autoFocus
            label={`Name or pick a person for ${sp.name}`}
            placeholder="Name or pick a person…"
            options={personOptions(m, known)}
            allowNew={newPerson}
            initial={sp.suggestion?.name ?? ""}
            onText={setText}
            onPick={save}
          />
          <div className="mt-3 flex gap-2">
            <button type="button" className="mi-btn-primary h-11 px-4 text-[14px]" disabled={!(text || sp.suggestion?.name)} onClick={() => save(text || sp.suggestion?.name || "")}>
              Save
            </button>
            <button type="button" className="mi-ghost h-11 px-4 text-[14px]" onClick={next}>
              Skip
            </button>
            <button type="button" className="mi-ghost h-11 px-4 text-[14px]" disabled={k + 1 >= n} onClick={next}>
              Next ›
            </button>
            <span className="flex-1" />
            <button type="button" className="mi-ghost h-11 px-4 text-[14px]" onClick={() => setOpen(false)}>
              Close
            </button>
          </div>
        </div>
      )}
    </section>
  );
}

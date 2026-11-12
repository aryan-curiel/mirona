import { memo, useEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from "react";
import { Virtuoso, type VirtuosoHandle } from "react-virtuoso";
import { useStore, type Tab } from "../store";
import { api, fmt, sure, typeLabel } from "../lib/api";
import { retime } from "../lib/notes";
import { isHidden } from "../lib/match";
import { type ComboOption, Combobox } from "../components/Combobox";
import { NameVoices, newPerson, personOptions, useVoices } from "./NameVoices";
import { Avatar, Chip, Icon, Label, Logo, Mono } from "../components/ui";
import { Renamed, withRenames } from "../components/Renamed";
import { pendingCount, renamedLine, voiceRenames } from "../lib/speakerEdits";
import { ImportProgress, RecordButton, useModelNames } from "../components/Shell";
import type { Meeting } from "../bindings/Meeting";
import type { MeetingPart } from "../bindings/MeetingPart";
import type { Segment } from "../bindings/Segment";
import type { Speaker } from "../bindings/Speaker";
import type { TagKind } from "../bindings/TagKind";

const notePill: Record<string, string> = { action: "Action", decision: "Decision", openQuestion: "Open" };

/** A timestamp that jumps to its moment in the transcript. */
function TimeLink({ ms }: { ms: number }) {
  const jump = useStore((s) => s.jump);
  return (
    <button type="button" onClick={() => jump(ms)} className="font-mono text-[12px] text-muted hover:text-accent hover:underline">
      {fmt(ms)}
    </button>
  );
}

function EmptyState() {
  const s = useStore();
  return (
    <div className="mx-auto flex max-w-[560px] flex-col items-center px-10 pt-24 pb-16 text-center">
      <Logo size={56} />
      <h1 className="mt-5 text-[26px] font-bold tracking-[-0.015em]">Your first meeting starts here</h1>
      <p className="mt-2 text-[15px] leading-relaxed text-muted">
        Press Start recording when the call begins. Mirona transcribes you and the other side live on this Mac, then writes a summary and action items when you
        stop. Each meeting is a Markdown folder in <span className="font-mono text-[13px]">{s.settings?.meetingsDir}</span>.
      </p>
      <div className="mt-6 flex w-64 flex-col gap-3">
        <RecordButton />
        <button type="button" className="mi-ghost min-h-11!" onClick={s.importAudio} disabled={!!s.importing}>
          {s.importing ? "Importing…" : "Import an audio file…"}
        </button>
        <ImportProgress />
        <button type="button" className="mi-ghost min-h-11!" onClick={s.addDemo}>
          Add demo meetings
        </button>
      </div>
      <p className="mt-3 text-[12px] text-muted">Demo meetings are marked as sample data. Remove them all at once from All meetings.</p>
    </div>
  );
}

/** The meeting's recording, if any. `play(ms)` seeks and plays. */
function useAudio(m: Meeting) {
  const ref = useRef<HTMLAudioElement>(null);
  const [src, setSrc] = useState<string | null>(null);
  const live = useStore((s) => s.recording?.meetingId === m.id);
  // Stopping compresses audio.wav into audio.caf: ask again once that's done.
  const stopping = useStore((s) => s.processing.includes(m.id));
  // biome-ignore lint/correctness/useExhaustiveDependencies: `stopping` only triggers a refetch
  useEffect(() => {
    let off = false;
    setSrc(null);
    if (!live) api.audioSrc(m.id).then((s) => !off && setSrc(s), () => {});
    return () => {
      off = true;
    };
  }, [m.id, live, stopping]);
  const play = (ms: number) => {
    const a = ref.current;
    if (!a) return;
    a.currentTime = ms / 1000;
    void a.play();
  };
  return { ref, src, play };
}

export function MeetingView() {
  const m = useStore((s) => s.meeting);
  const tab = useStore((s) => s.tab);
  const setTab = useStore((s) => s.setTab);
  if (!m) return <EmptyState />;
  return (
    <div className="mx-auto max-w-[1040px] px-10 pt-8 pb-16">
      <MeetingHeader m={m} />
      <div role="tablist" className="mt-5 mb-5 flex gap-1 border-b border-line">
        {(["transcript", "notes", "summary"] as Tab[]).map((t) => (
          <button type="button" key={t} role="tab" aria-selected={tab === t} onClick={() => setTab(t)} className="mi-tab min-h-11 px-4 pb-2.5 text-[15px] font-medium text-muted capitalize">
            {t}
          </button>
        ))}
      </div>
      {tab === "transcript" && <Transcript key={m.id} m={m} />}
      {tab === "notes" && <Notes key={m.id} m={m} />}
      {tab === "summary" && <SummaryTab m={m} />}
    </div>
  );
}

function MeetingHeader({ m }: { m: Meeting }) {
  const rename = useStore((s) => s.rename);
  const busy = useStore((s) => s.processing.includes(m.id) || m.id in s.summarizing);
  const live = useStore((s) => s.recording?.meetingId === m.id);
  const reviews = useStore((s) => s.review.filter((r) => !r.rejected).length);
  const [title, setTitle] = useState(m.title);
  useEffect(() => setTitle(m.title), [m.title]);
  const rerunning = useStore((s) => s.rerunning[m.id]);
  const a = m.answers;
  return (
    <header data-tauri-drag-region>
      <div className="flex flex-wrap items-center gap-2">
        {/* Plain answers; scores live in the inspector under "Show model details". */}
        {a?.meetingType.value && <Chip kind={sure(a.meetingType) ? "neutral" : "review"}>{typeLabel(a.meetingType.value)}{sure(a.meetingType) ? "" : "?"}</Chip>}
        {m.tags.map((t) => (
          <Chip key={t} kind="neutral">
            {t}
          </Chip>
        ))}
        {reviews > 0 && <Chip kind="review">{reviews} to check</Chip>}
        {m.sample && <span className="text-[12px] font-medium text-muted">Sample data</span>}
        {busy && <span className="mi-chip mi-chip--review mi-pulse">Writing the summary on this device…</span>}
        {rerunning && <span className="mi-chip mi-chip--review mi-pulse">{rerunning}</span>}
        {!live && <MoreMenu m={m} />}
      </div>
      <input
        aria-label="Meeting title"
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        onBlur={() => (title.trim() ? rename(title) : setTitle(m.title))}
        onKeyDown={(e) => {
          if (e.key === "Enter") e.currentTarget.blur();
          if (e.key === "Escape") setTitle(m.title);
        }}
        className="selectable mt-3 -ml-1 w-full rounded-md bg-transparent px-1 text-[32px] font-bold leading-tight tracking-[-0.02em] outline-none hover:bg-white/40"
      />
      <p className="mt-1.5 text-[14px] text-muted">
        {m.dateLabel} · {live ? "live" : `${m.durationMin} min`} · {m.speakers.length} speaker{m.speakers.length === 1 ? "" : "s"} · {m.language}
      </p>
    </header>
  );
}

/** Redo and remove, out of the way until needed. */
function MoreMenu({ m }: { m: Meeting }) {
  const rerun = useStore((s) => s.rerun);
  const st = useStore((s) => s.settings);
  const busy = useStore((s) => s.processing.includes(m.id) || !!s.rerunning[m.id]);
  const menu = useRef<HTMLDetailsElement>(null);
  const [removing, setRemoving] = useState(false);
  // <details> doesn't close on an outside click by itself.
  useEffect(() => {
    const close = (e: PointerEvent) => {
      if (menu.current?.open && !menu.current.contains(e.target as Node)) menu.current.open = false;
    };
    window.addEventListener("pointerdown", close);
    return () => window.removeEventListener("pointerdown", close);
  }, []);
  const pick =(f: () => void) => () => {
    if (menu.current) menu.current.open = false;
    f();
  };
  const item = "mi-nav flex min-h-10 w-full items-center px-3 text-left text-[14px] disabled:text-muted";
  return (
    <>
      <details ref={menu} className="relative ml-auto">
        <summary className="mi-ghost flex cursor-default list-none items-center text-[13px] [&::-webkit-details-marker]:hidden">More</summary>
        <div className="mi-card absolute right-0 z-20 mt-1 flex w-64 flex-col p-1.5">
          <button type="button" className={item} disabled={busy || st?.diarization === "off"} onClick={pick(() => rerun("diarization"))}>
            Re-run speaker detection
          </button>
          <button type="button" className={item} disabled={busy || st?.classification === "off"} onClick={pick(() => rerun("classification"))}>
            Re-run classification
          </button>
          <hr className="my-1 border-line-soft" />
          <button type="button" className={`${item} text-record-text`} onClick={pick(() => setRemoving(true))}>
            Remove or delete…
          </button>
        </div>
      </details>
      {removing && <RemoveDialog m={m} onClose={() => setRemoving(false)} />}
    </>
  );
}

/**
 * Pick what goes: generated parts (the transcript and your notes stay), or the whole meeting.
 * A native modal dialog: focus stays inside and Escape closes it.
 */
function RemoveDialog({ m, onClose }: { m: Meeting; onClose: () => void }) {
  const row = useStore((s) => s.meetings.find((x) => x.id === m.id));
  const items = useStore((s) => s.actionItems.filter((a) => a.meetingId === m.id).length);
  const removeParts = useStore((s) => s.removeParts);
  const deleteMeetings = useStore((s) => s.deleteMeetings);
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => ref.current?.showModal(), []);
  const tagged = m.segments.filter((x) => x.tags.some((t) => t.kind !== "sensitive") || x.answers.length).length;
  const parts: { id: MeetingPart; label: string; detail: string; has: boolean }[] = [
    { id: "audio", label: "Recording", detail: `${m.durationMin} min of audio. The transcript stays.`, has: !!row?.hasAudio },
    { id: "summary", label: "Summary and action items", detail: `${items} action item${items === 1 ? "" : "s"}. Write a new one any time.`, has: !!m.summary },
    { id: "enhanced", label: "Enhanced notes", detail: `${m.notes.enhanced.length} lines. Your own notes stay.`, has: m.notes.enhanced.length > 0 },
    { id: "classification", label: "Tags and classification", detail: `${tagged} tagged lines, meeting type, meeting tags, follow-up. Sensitive lines stay hidden.`, has: tagged > 0 || !!m.answers },
  ];
  const [picked, setPicked] = useState<MeetingPart[]>([]);
  const [whole, setWhole] = useState(false);
  const flip = (id: MeetingPart) => setPicked(picked.includes(id) ? picked.filter((p) => p !== id) : [...picked, id]);
  const go = async () => {
    onClose();
    if (whole) await deleteMeetings([m.id]);
    else await removeParts(m.id, picked);
  };
  return (
    <dialog ref={ref} onClose={onClose} aria-labelledby="remove-title" className="mi-card mi-pop m-auto w-full max-w-[520px] p-6 text-ink backdrop:bg-black/20">
      <h2 id="remove-title" className="text-[18px] font-semibold">Remove from “{m.title}”</h2>
      <p className="mt-1 text-[14px] text-ink-3">Pick what Mirona generated and you no longer want. The transcript and your notes stay.</p>
      <fieldset className="mt-4 flex flex-col gap-1" disabled={whole}>
        {parts.map((p) => (
          <label key={p.id} className={`flex items-start gap-3 rounded-md px-2 py-2 ${p.has && !whole ? "hover:bg-white/40" : "opacity-55"}`}>
            <input type="checkbox" className="mt-1" disabled={!p.has} checked={picked.includes(p.id)} onChange={() => flip(p.id)} />
            <span>
              <span className="block text-[14px] font-medium">{p.label}</span>
              <span className="block text-[13px] text-muted">{p.has ? p.detail : "Nothing to remove."}</span>
            </span>
          </label>
        ))}
      </fieldset>
      <hr className="my-3 border-line-soft" />
      <label className="flex items-start gap-3 px-2 py-2">
        <input type="checkbox" className="mt-1" checked={whole} onChange={(e) => setWhole(e.target.checked)} />
        <span>
          <span className="block text-[14px] font-medium text-record-text">Delete the whole meeting</span>
          <span className="block text-[13px] text-muted">The folder, audio, transcript, notes, search vectors, voices heard only here and your corrections. Can't be undone.</span>
        </span>
      </label>
      <div className="mt-5 flex justify-end gap-2">
        <button type="button" className="mi-ghost min-h-11! px-4" onClick={onClose}>
          Cancel
        </button>
        <button
          type="button"
          className={`flex h-11 items-center gap-2 px-5 ${whole ? "rounded-md bg-record font-semibold text-white" : "mi-btn-primary"}`}
          disabled={!whole && !picked.length}
          onClick={() => void go()}
        >
          {whole ? "Delete for good" : picked.length ? `Remove ${picked.length} part${picked.length === 1 ? "" : "s"}` : "Remove"}
        </button>
      </div>
    </dialog>
  );
}

/** `text` with every case-insensitive match of `q` marked. */
function highlight(text: string, q: string): ReactNode {
  if (!q) return text;
  const lower = text.toLowerCase();
  const out: ReactNode[] = [];
  let i = 0;
  for (let j = lower.indexOf(q); j >= 0; j = lower.indexOf(q, i)) {
    out.push(text.slice(i, j), <mark key={j} className="rounded-sm bg-[var(--mi-review-tint)] text-ink">{text.slice(j, j + q.length)}</mark>);
    i = j + q.length;
  }
  out.push(text.slice(i));
  return out;
}

const has = (seg: Segment, k: TagKind) => seg.tags.some((t) => t.kind === k);

function Transcript({ m }: { m: Meeting }) {
  const s = useStore();
  const live = s.recording?.meetingId === m.id;
  const audio = useAudio(m);
  const voices = useVoices(m);
  const people = useMemo(() => personOptions(m, voices.known), [m, voices.known]);
  const list = useRef<VirtuosoHandle>(null);
  const [query, setQuery] = useState("");
  const q = query.trim().toLowerCase();
  // The list scrolls with the page (header and tabs above it), so it needs the page's scroller.
  const [scroller, setScroller] = useState<HTMLElement | null>(null);
  useEffect(() => setScroller(document.getElementById("main-scroll")), []);

  const counts = useMemo(() => {
    const count = (k: TagKind) => m.segments.filter((x) => has(x, k)).length;
    return (
      [
        [count("action"), "action item", "action items"],
        [count("decision"), "decision", "decisions"],
        [count("openQuestion"), "open question", "open questions"],
        [count("sensitive"), "sensitive", "sensitive"],
        [count("review"), "to review", "to review"],
      ] as const
    )
      .filter(([n]) => n)
      .map(([n, one, many]) => `${n} ${n === 1 ? one : many}`)
      .join(" · ");
  }, [m.segments]);

  const { hideSmallTalk, onlyTagged } = s.filters;
  const rows = useMemo(
    () =>
      m.segments.filter(
        (x) => !(hideSmallTalk && has(x, "smallTalk")) && !(onlyTagged && !x.tags.length) && (!q || x.text.toLowerCase().includes(q)),
      ),
    [m.segments, hideSmallTalk, onlyTagged, q],
  );
  // First segment of each speaker carries their name suggestion.
  const firstOf = useMemo(() => {
    const seen = new Map<string, string>();
    for (const x of m.segments) if (!seen.has(x.speakerId)) seen.set(x.speakerId, x.id);
    return new Set(seen.values());
  }, [m.segments]);
  const voiceLines = useMemo(() => {
    const count = new Map<string, number>();
    for (const x of m.segments) count.set(x.speakerId, (count.get(x.speakerId) ?? 0) + 1);
    return count;
  }, [m.segments]);
  // Entries of a voice that no pending line edit moved: what a voice-wide rename reaches.
  const lineEdits = s.pending?.meetingId === m.id ? s.pending.lines : null;
  const voiceFree = useMemo(() => {
    const count = new Map<string, number>();
    for (const x of m.segments) if (!lineEdits?.[x.id]) count.set(x.speakerId, (count.get(x.speakerId) ?? 0) + 1);
    return count;
  }, [m.segments, lineEdits]);

  // Jumps from the summary and notes.
  const { scrollTo } = s;
  useEffect(() => {
    if (!scrollTo) return;
    const index = rows.findIndex((r) => r.id === scrollTo);
    if (index >= 0) setTimeout(() => list.current?.scrollToIndex({ index, align: "center" }), 30);
    useStore.setState({ scrollTo: null });
  }, [scrollTo, rows]);

  // Follow new segments and the lines still being spoken while recording.
  const drafted = s.recording?.drafts.map((d) => d.segment.text).join("\n");
  useEffect(() => {
    if (!live || q) return;
    if (drafted) scroller?.scrollTo({ top: scroller.scrollHeight, behavior: "smooth" });
    else if (rows.length) list.current?.scrollToIndex({ index: rows.length - 1, align: "end", behavior: "smooth" });
  }, [live, rows.length, q, drafted, scroller]);

  return (
    <div>
      {audio.src && (
        <div className="sticky top-2 z-10 mb-4">
          {/* biome-ignore lint/a11y/useMediaCaption: no captions file; the transcript below is the text */}
          <audio ref={audio.ref} src={audio.src} controls preload="metadata" className="w-full" aria-label="Meeting recording" />
        </div>
      )}
      {live && (
        <p role="note" className="mb-3 flex items-start gap-2 rounded-lg bg-accent-tint px-3 py-2 text-[13px] leading-snug text-accent-strong">
          <Icon name="info" size={16} className="mt-px shrink-0" />
          Everyone on the other side shows as “Them” for now. Speakers are told apart, and voices you've named are recognized, once the call ends.
        </p>
      )}
      <SpeakerChanges m={m} below={!!audio.src} />
      <NameVoices m={m} unnamed={voices.unnamed.filter((id) => !(s.pending?.meetingId === m.id && id in s.pending.speakers))} known={voices.known} play={audio.src ? audio.play : undefined} />
      <div className="flex flex-wrap items-center gap-2 text-[14px]">
        <input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search this transcript"
          aria-label="Search this transcript"
          className="mi-field w-64"
        />
        {(
          [
            ["hideSmallTalk", "Hide small talk"],
            ["onlyTagged", "Only tagged"],
            ["showSensitive", "Show sensitive text"],
          ] as const
        ).map(([k, label]) => (
          <button
            type="button"
            key={k}
            aria-pressed={s.filters[k]}
            onClick={() => s.setFilter(k, !s.filters[k])}
            className={`min-h-9 rounded-full px-4 text-[13px] font-medium ${s.filters[k] ? "bg-ink text-white" : "mi-ghost rounded-full!"}`}
          >
            {label}
          </button>
        ))}
      </div>
      <p className="mt-3 mb-3 flex text-[13px] text-muted">
        <span className="flex-1">{q ? `${rows.length} segment${rows.length === 1 ? "" : "s"} match “${query.trim()}”` : `${m.segments.length} segments`}</span>
        <span>{counts}</span>
      </p>

      {scroller && <Virtuoso
        ref={list}
        customScrollParent={scroller}
        data={rows}
        computeItemKey={(_, seg) => seg.id}
        itemContent={(_, seg) => (
          // The row with the open picker must paint over the rows below it.
          <div className="relative pb-3 focus-within:z-10">
            <SegmentRow
              seg={seg}
              meetingId={m.id}
              speaker={m.speakers.find((p) => p.id === seg.speakerId)}
              suggest={firstOf.has(seg.id)}
              selected={s.segmentId === seg.id}
              showSensitive={s.filters.showSensitive}
              query={q}
              onPlay={audio.src ? audio.play : undefined}
              people={live ? undefined : people}
              voiceLines={voiceLines.get(seg.speakerId) ?? 1}
              voiceFree={voiceFree.get(seg.speakerId) ?? 0}
            />
          </div>
        )}
      />}
      {live && s.recording ? (
        <>
          {!q &&
            s.recording.drafts.map(({ segment: d, speaker: sp }) => (
              <DraftRow key={d.id} seg={d} speaker={m.speakers.find((p) => p.id === d.speakerId) ?? sp} showSensitive={s.filters.showSensitive} />
            ))}
          <div className="mi-live flex gap-4 px-5 py-4 text-[14px] text-ink-3">
            <Mono>{fmt(s.recording.elapsedMs)}</Mono>
            <span className="mi-pulse">Listening… lines appear as they are spoken and are tagged once each sentence ends.</span>
          </div>
        </>
      ) : (
        !rows.length && <p className="py-10 text-center text-muted">{m.segments.length ? "No segments match." : "No speech in this meeting."}</p>
      )}
    </div>
  );
}

/** A line still being spoken: in italics and without tags until its sentence ends and it's saved. */
function DraftRow({ seg, speaker: sp, showSensitive }: { seg: Segment; speaker: Speaker; showSensitive: boolean }) {
  const hidden = seg.tags.some((t) => t.kind === "sensitive") && !showSensitive;
  return (
    <div className="mb-3 flex gap-4 px-5 py-4">
      <Avatar tint={sp.tint} initial={sp.initial} />
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2.5">
          <span className="text-[15px] font-semibold">{sp.name}</span>
          <Mono>{fmt(seg.startMs)}</Mono>
          <span className="mi-pulse text-[12px] text-muted">Still speaking…</span>
        </div>
        <p className="mt-1 text-[15px] italic leading-relaxed text-ink-3">{hidden ? "[Sensitive: hidden]" : seg.text}</p>
      </div>
    </div>
  );
}

/** Speaker changes waiting for Save (NGR-201): every one with its undo, and Save / Discard for all of them. */
function SpeakerChanges({ m, below }: { m: Meeting; below: boolean }) {
  const pending = useStore((s) => s.pending);
  const saving = useStore((s) => s.savingSpeakers);
  const save = useStore((s) => s.saveSpeakerEdits);
  const discard = useStore((s) => s.discardSpeakerEdits);
  const undoSpeaker = useStore((s) => s.undoSpeaker);
  const undoLine = useStore((s) => s.undoLine);
  if (pending?.meetingId !== m.id) return null;
  const n = pendingCount(pending);
  const nameOf = (id: string) => m.speakers.find((p) => p.id === id)?.name ?? id;
  return (
    // The recording player is sticky at the top too; sit under it.
    <section aria-label="Unsaved speaker changes" className={`mi-card sticky z-10 mb-4 flex flex-wrap items-center gap-x-4 gap-y-2 p-3 text-[14px] ${below ? "top-16" : "top-2"}`}>
      <span className="font-medium">
        {n} speaker change{n === 1 ? "" : "s"}
      </span>
      <ul className="flex min-w-0 flex-1 flex-wrap gap-x-4 gap-y-1">
        {Object.entries(pending.speakers).map(([id, to]) => (
          <li key={id} className="flex items-baseline gap-1.5">
            <Renamed from={nameOf(id)} to={to} onUndo={() => undoSpeaker(id)} />
            <span className="text-[12px] text-muted">every line</span>
          </li>
        ))}
        {Object.entries(pending.lines).map(([id, to]) => {
          const seg = m.segments.find((x) => x.id === id);
          return (
            <li key={id} className="flex items-baseline gap-1.5">
              <Renamed from={nameOf(seg?.speakerId ?? "")} to={to} onUndo={() => undoLine(id)} />
              {seg && <Mono>{fmt(seg.startMs)}</Mono>}
            </li>
          );
        })}
      </ul>
      <button type="button" className="mi-ghost min-h-10!" disabled={saving} onClick={discard}>
        Discard
      </button>
      <button type="button" className="mi-btn-primary h-10 px-4 text-[14px]" disabled={saving} onClick={() => void save()}>
        {saving ? "Saving…" : "Save"}
      </button>
    </section>
  );
}

type RowProps = {
  seg: Segment;
  meetingId: string;
  speaker?: Speaker;
  suggest: boolean;
  selected: boolean;
  showSensitive: boolean;
  query: string;
  onPlay?: (ms: number) => void;
  /** Who a line can be moved to; none while recording. */
  people?: ComboOption[];
  /** Lines the line's voice has in the whole meeting, and those no pending line edit moved. */
  voiceLines: number;
  voiceFree: number;
};

type Scope = "line" | "voice";

/** Off: the change is for this entry only. On: for every entry of the voice that has no change of its own. */
function ScopeToggle({ scope, onScope, count, edited, compact = false }: { scope: Scope; onScope: (s: Scope) => void; count: number; edited: number; compact?: boolean }) {
  const on = scope === "voice";
  const tip = `Apply to all ${count} entries of this voice in the meeting${edited > 0 ? ` (${edited} with their own change are left as they are)` : ""}`;
  return (
    <button
      type="button"
      title={tip}
      aria-label={tip}
      aria-pressed={on}
      // The field next to it must keep focus, or its blur would save before the scope is set.
      onMouseDown={(e) => e.preventDefault()}
      onClick={() => onScope(on ? "line" : "voice")}
      className={`${compact ? "h-6 px-2.5 text-[11px]" : "h-8 px-3 text-[12px]"} inline-flex shrink-0 items-center justify-center whitespace-nowrap rounded-full font-semibold leading-none tabular-nums ${on ? "bg-accent-tint text-accent-strong ring-2 ring-[var(--mi-accent)]" : "bg-white text-ink-2 shadow-[0_1px_3px_rgba(0,0,0,0.3),0_0_0_1px_rgba(0,0,0,0.14)] hover:bg-[#f3f5f4]"}`}
    >
      All · {count}
    </button>
  );
}

/**
 * A speaker's name; click to rename. A voice with other lines shows, next to the field, whether the
 * new name is for this line only or for all of them (NGR-201). Nothing is written until Save.
 */
function SpeakerName({
  renamedBy,
  sp,
  renamedTo,
  lines,
  affected,
  onVoice,
  onLine,
  onUndo,
  undoHint,
}: {
  sp: Speaker;
  renamedTo?: string;
  renamedBy?: "line" | "voice";
  /** Lines this voice has in the meeting; one means there is no scope to choose. */
  lines: number;
  /** Of those, the ones a voice-wide rename reaches (the rest have a change of their own). */
  affected: number;
  onVoice: (name: string) => void;
  onLine: (name: string) => void;
  onUndo: () => void;
  undoHint: string;
}) {
  const shown = renamedTo ?? sp.name;
  const [draft, setDraft] = useState<string | null>(null);
  const [scope, setScope] = useState<Scope>(renamedBy === "voice" ? "voice" : "line");
  if (draft === null) {
    const open = (e: { stopPropagation: () => void }) => {
      e.stopPropagation();
      setScope(renamedBy === "voice" ? "voice" : "line");
      setDraft(shown);
    };
    if (renamedTo) return <Renamed from={sp.name} to={renamedTo} onUndo={onUndo} undoHint={undoHint} onEdit={open} />;
    return (
      <button
        type="button"
        title="Rename this speaker"
        aria-label={`Rename ${sp.name}`}
        className="rounded text-[15px] font-semibold hover:underline hover:decoration-dotted"
        onClick={open}
      >
        {sp.name}
      </button>
    );
  }
  const commit = () => {
    const n = draft.trim();
    setDraft(null);
    if (!n || n === shown) return;
    if (lines > 1 && scope === "line") onLine(n);
    else onVoice(n);
  };
  return (
    // The scope buttons must not take focus from the field, or its blur would save before one is picked.
    // biome-ignore lint/a11y/useKeyWithClickEvents: keeps clicks in the editor from selecting the line
    <fieldset
      aria-label={`Rename ${shown}`}
      className="relative m-0 w-52 min-w-0 border-0 p-0"
      onClick={(e) => e.stopPropagation()}
      onBlur={(e) => !e.currentTarget.contains(e.relatedTarget) && commit()}
    >
      <input
        // biome-ignore lint/a11y/noAutofocus: opened by the user's own click on the name
        autoFocus
        aria-label={`New name for ${shown}`}
        className={`mi-field h-8 w-full text-[14px] ${lines > 1 ? "pr-20" : ""}`}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") commit();
          if (e.key === "Escape") setDraft(null);
        }}
      />
      {lines > 1 && (
        <div className="absolute inset-y-0 right-1 flex items-center">
          <ScopeToggle compact scope={scope} onScope={setScope} count={affected} edited={lines - affected} />
        </div>
      )}
    </fieldset>
  );
}

/** Line buttons stay out of sight until the line is hovered or focused from the keyboard. */
const reveal = "opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100 focus-visible:opacity-100";

/** "Who said this?": move this line, or the whole voice, to another person (NGR-201). */
function WhoSaid({ seg, people, total, count, byVoice }: { seg: Segment; people: ComboOption[]; total: number; count: number; byVoice: boolean }) {
  const editLine = useStore((s) => s.editLine);
  const editSpeaker = useStore((s) => s.editSpeaker);
  const [open, setOpen] = useState(false);
  const [scope, setScope] = useState<Scope>(byVoice ? "voice" : "line");
  if (!open)
    return (
      <button
        type="button"
        className={`rounded px-1 text-[12px] font-medium text-accent-strong hover:underline ${reveal}`}
        onClick={(e) => {
          e.stopPropagation();
          setScope(byVoice ? "voice" : "line");
          setOpen(true);
        }}
      >
        Who said this?
      </button>
    );
  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: keeps clicks in the picker from selecting the line
    // biome-ignore lint/a11y/useKeyWithClickEvents: same as above
    <div className="w-56" onClick={(e) => e.stopPropagation()} onBlur={(e) => !e.currentTarget.contains(e.relatedTarget) && setOpen(false)}>
      <div className="w-full">
        <Combobox
          autoFocus
          label="Who said this line?"
          adornment={total > 1 && <ScopeToggle compact scope={scope} onScope={setScope} count={count} edited={total - count} />}
          placeholder="Search people…"
          options={people}
          allowNew={newPerson}
          onPick={(name) => {
            setOpen(false);
            if (scope === "voice") editSpeaker(seg.speakerId, name);
            else editLine(seg.id, name, true);
          }}
        />
      </div>
    </div>
  );
}

const SegmentRow = memo(function SegmentRow({ seg, meetingId, speaker: sp, suggest, selected, showSensitive, query, onPlay, people, voiceLines, voiceFree }: RowProps) {
  const select = useStore((s) => s.selectSegment);
  const editSpeaker = useStore((s) => s.editSpeaker);
  const editLine = useStore((s) => s.editLine);
  const undoSpeaker = useStore((s) => s.undoSpeaker);
  const undoLine = useStore((s) => s.undoLine);
  const edit = useStore((s) => s.editSegment);
  const details = useStore((s) => s.modelDetails);
  const renamed = renamedLine(useStore((s) => s.pending), meetingId, seg);
  const renamedTo = renamed?.to;
  const renamedBy = renamed?.by;
  // A voice-wide rename skips the entries that have a change of their own, except this one.
  const affected = voiceFree + (renamedBy === "line" ? 1 : 0);
  const [draft, setDraft] = useState<string | null>(null);
  const save = () => {
    if (draft?.trim() && draft.trim() !== seg.text) void edit(seg.id, draft);
    setDraft(null);
  };
  // Redacted lines and lines tagged sensitive stay hidden until "Show sensitive text".
  const hidden = isHidden(seg, showSensitive);
  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: mouse shortcut; the Inspect button is the keyboard path
    // biome-ignore lint/a11y/useKeyWithClickEvents: same as above
    <div className="mi-row group flex gap-4 px-5 py-4" aria-current={selected} onClick={() => select(seg.id)}>
      {sp && <Avatar tint={sp.tint} initial={sp.initial} />}
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2.5">
          {sp ? (
            <SpeakerName
              sp={sp}
              renamedTo={renamedTo}
              renamedBy={renamedBy}
              lines={sp.id === "me" ? 1 : voiceLines}
              affected={affected}
              onVoice={(n) => editSpeaker(sp.id, n)}
              onLine={(n) => editLine(seg.id, n, true)}
              onUndo={() => (renamedBy === "line" ? undoLine(seg.id) : undoSpeaker(sp.id))}
              undoHint={renamedBy === "line" ? "Undo this line's change" : `Undo the change for all of ${sp.name}'s lines`}
            />
          ) : (
            <span className="text-[15px] font-semibold">{seg.speakerId}</span>
          )}
          {onPlay ? (
            <button
              type="button"
              aria-label={`Play from ${fmt(seg.startMs)}`}
              className="flex items-center gap-1 rounded font-mono text-[12px] text-muted hover:text-accent hover:underline"
              onClick={(e) => {
                e.stopPropagation();
                onPlay(seg.startMs);
              }}
            >
              <svg width="9" height="10" viewBox="0 0 9 10" aria-hidden="true"><path d="M0 0l9 5-9 5z" fill="currentColor" /></svg>
              {fmt(seg.startMs)}
            </button>
          ) : (
            <Mono>{fmt(seg.startMs)}</Mono>
          )}
          {suggest && sp?.suggestion && !renamedTo && (
            <Chip
              kind="review"
              label={`Confirm ${sp.name} is ${sp.suggestion.name}`}
              onClick={(e) => {
                e.stopPropagation();
                if (sp.suggestion) editSpeaker(sp.id, sp.suggestion.name);
              }}
            >
              <span title={sp.suggestion.evidence ? `From “${sp.suggestion.evidence}”` : undefined}>
                {sp.suggestion.name}? {details && `${sp.suggestion.probability.toFixed(2)} `}· Confirm
              </span>
            </Chip>
          )}
          {seg.tags.map((t) => (
            <Chip key={t.label} kind={t.kind}>
              {t.kind === "action" && seg.due && !t.label.includes(seg.due) ? `${t.label} · ${seg.due}` : details ? t.label : t.label.replace(/ \d\.\d+$/, "")}
            </Chip>
          ))}
        </div>
        {draft !== null ? (
          <textarea
            // biome-ignore lint/a11y/noAutofocus: opened by the user's own Edit click
            autoFocus
            aria-label="Segment text"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={save}
            onClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                save();
              }
              if (e.key === "Escape") setDraft(null);
            }}
            rows={Math.max(2, Math.ceil(draft.length / 90))}
            className="mi-field selectable mt-1 w-full resize-none text-[15px] leading-relaxed"
          />
        ) : hidden ? (
          <p className="mt-1 text-[15px] italic text-sensitive">[Sensitive: hidden. Tick “Show sensitive text” to read it]</p>
        ) : (
          <p className="selectable mt-1 text-[15px] leading-relaxed text-ink-2" onDoubleClick={() => setDraft(seg.text)}>
            {highlight(seg.text, query)}
          </p>
        )}
      </div>
      <div className="flex flex-col items-end gap-1.5 self-start">
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            select(seg.id);
          }}
          className={selected ? "mi-btn-primary h-9 px-3.5 text-[13px]" : `mi-ghost text-[13px] ${reveal}`}
        >
          {selected ? "Inspecting" : "Inspect"}
        </button>
        {!hidden && draft === null && (
          <button
            type="button"
            className={`rounded px-1 text-[12px] font-medium text-accent-strong hover:underline ${selected ? "" : reveal}`}
            onClick={(e) => {
              e.stopPropagation();
              setDraft(seg.text);
            }}
          >
            Fix text
          </button>
        )}
        {people && seg.speakerId !== "me" && draft === null && <WhoSaid seg={seg} people={people} total={voiceLines} count={affected} byVoice={renamedBy === "voice"} />}
      </div>
    </div>
  );
});

const toLines = (raw: string[]) => (raw.length ? raw : [""]);

function Notes({ m }: { m: Meeting }) {
  const recording = useStore((s) => s.recording);
  const regenerate = useStore((s) => s.regenerate);
  const regenerating = useStore((s) => s.regenerating === m.id);
  const template = useStore((s) => s.settings?.template ?? "client-call");
  const flash = useStore((s) => s.flash);
  const [lines, setLines] = useState(() => toLines(m.notes.raw));
  const [times, setTimes] = useState<(number | null)[]>(() => m.notes.rawAtMs);
  const [saved, setSaved] = useState(true);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const latest = useRef({ lines, times, saved });
  latest.current = { lines, times, saved };

  const persist = async () => {
    clearTimeout(timer.current);
    const { lines, times } = latest.current;
    try {
      // Trailing blank lines are dropped by the store anyway.
      await api.saveNotes(m.id, lines, times);
      setSaved(true);
    } catch (e) {
      flash(`Notes not saved: ${e}`, true);
    }
  };
  // Save whatever is pending when leaving the meeting or the tab.
  // biome-ignore lint/correctness/useExhaustiveDependencies: run on unmount only
  useEffect(
    () => () => {
      if (!latest.current.saved) void persist();
    },
    [],
  );

  const onChange = (text: string) => {
    const next = text.split("\n");
    const now = recording?.meetingId === m.id ? recording.elapsedMs : null;
    setTimes(retime(lines, times, next, now));
    setLines(next);
    setSaved(false);
    clearTimeout(timer.current);
    timer.current = setTimeout(persist, 800);
  };

  return (
    <div className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)] gap-5">
      <section className="mi-card flex flex-col p-6">
        <div className="flex items-center">
          <Label className="flex-1">Your notes · Markdown</Label>
          <span className="text-[12px] text-muted" aria-live="polite">{saved ? "Saved to meeting.md" : "Saving…"}</span>
        </div>
        <EditorWithTimes lines={lines} times={times} onChange={onChange} onBlur={persist} />
        <p className="mt-3 text-[12px] text-muted">Lines typed while recording are stamped with the meeting time, so enhanced notes can link them to the transcript.</p>
      </section>
      <section className="mi-card p-6">
        <div className="flex items-center gap-2">
          <Label className="flex-1">Enhanced notes</Label>
          <button type="button" className="mi-ghost text-[13px]" disabled={regenerating || !lines.some((l) => l.trim())} onClick={() => persist().then(() => regenerate(template))}>
            {regenerating ? "Enhancing…" : m.notes.enhanced.length ? "Enhance again" : "Enhance with transcript"}
          </button>
        </div>
        {m.notes.enhanced.length ? (
          <>
            <ul className="mt-3 flex flex-col gap-3.5">
              {m.notes.enhanced.map((n) => (
                <li key={`${n.atMs}-${n.text}`} className="flex items-start gap-3">
                  <Chip kind={n.kind}>{notePill[n.kind] ?? n.kind}</Chip>
                  <span className="selectable flex-1 text-[15px] leading-snug">{n.text}</span>
                  <TimeLink ms={n.atMs} />
                </li>
              ))}
            </ul>
            <p className="mt-5 text-[13px] text-muted">Your lines, expanded with the transcript by {m.notes.enhancedBy}. Your own notes are never changed.</p>
          </>
        ) : (
          <p className="mt-3 text-[14px] text-muted">Written next to your notes when the recording stops (it also rewrites the summary).</p>
        )}
      </section>
    </div>
  );
}

/** A plain Markdown textarea with the meeting time of each line in a gutter. */
function EditorWithTimes({ lines, times, onChange, onBlur }: { lines: string[]; times: (number | null)[]; onChange: (t: string) => void; onBlur: () => void }) {
  const area: RefObject<HTMLTextAreaElement | null> = useRef(null);
  return (
    <div className="mt-3 flex min-h-48 flex-1 font-mono text-[14px] leading-[28px]">
      <div aria-hidden className="w-12 shrink-0 pt-px text-[11px] text-muted select-none">
        {lines.map((_, i) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: one gutter cell per line, position is the identity
          <div key={i} className="h-[28px]">{times[i] != null ? fmt(times[i]) : ""}</div>
        ))}
      </div>
      <textarea
        ref={area}
        aria-label="Your notes"
        value={lines.join("\n")}
        onChange={(e) => onChange(e.target.value)}
        onBlur={onBlur}
        wrap="off"
        rows={Math.max(6, lines.length + 1)}
        placeholder="- type rough notes here"
        className="selectable min-w-0 flex-1 resize-none overflow-x-auto overflow-y-hidden bg-transparent outline-none"
      />
    </div>
  );
}

function SummaryTab({ m }: { m: Meeting }) {
  const s = useStore();
  const { llm } = useModelNames();
  const sum = m.summary;
  const items = s.actionItems.filter((a) => a.meetingId === m.id);
  const renames = voiceRenames(s.pending, m);
  const stream = s.summarizing[m.id];
  const busy = s.regenerating === m.id || s.processing.includes(m.id) || !!stream;
  const current = s.templates.find((t) => t.name === sum?.template || t.id === sum?.template)?.id ?? s.settings?.template ?? "";
  const [template, setTemplate] = useState(current);
  useEffect(() => setTemplate(current), [current]);
  const live = s.recording?.meetingId === m.id;

  return (
    <div>
      <div className="mi-card flex flex-wrap items-center gap-3 p-4">
        <label className="flex items-center gap-2 text-[14px]">
          <span className="text-muted">Template</span>
          <select className="mi-field" value={template} onChange={(e) => setTemplate(e.target.value)}>
            {s.templates.map((t) => (
              <option key={t.id} value={t.id}>{t.name}</option>
            ))}
          </select>
        </label>
        <button type="button" className="mi-btn-primary h-11 px-4 text-[14px]" disabled={busy || live || !m.segments.length} onClick={() => s.regenerate(template)}>
          {busy ? "Writing…" : sum ? "Regenerate" : "Write summary"}
        </button>
        <span className="text-[13px] text-muted">
          with {llm}.{" "}
          <button type="button" className="font-medium text-accent-strong underline underline-offset-2" onClick={() => s.openSettings("models")}>
            Change model
          </button>
        </span>
      </div>

      {stream ? (
        <section className="mi-card mt-5 p-6" aria-busy="true">
          <div className="flex flex-wrap items-baseline gap-x-3">
            <Label>Writing the summary</Label>
            <span className="mi-pulse text-[13px] text-muted">
              {llm} · {stream.words} word{stream.words === 1 ? "" : "s"} so far
            </span>
          </div>
          <p className="selectable mt-3 text-[15px] leading-relaxed">
            {stream.overview || <span className="text-muted">Reading the transcript…</span>}
          </p>
          <p className="mt-4 text-[13px] text-muted">Decisions, action items and questions appear when it's done.</p>
        </section>
      ) : sum ? (
        <section className={`mi-card mi-enter mt-5 p-6 ${busy ? "opacity-60" : ""}`}>
          <div className="flex flex-wrap items-baseline gap-x-3">
            <Label>Template: {sum.template}</Label>
            <span className="text-[13px] text-muted">
              {m.answers && sure(m.answers.meetingType) && m.answers.meetingType.value.toLowerCase().replace(/[ _]/g, "-") === current
                ? `picked for a ${typeLabel(m.answers.meetingType.value).toLowerCase()} · `
                : ""}
              Written by {sum.writtenBy || "an unknown model"}
            </span>
          </div>
          <p className="selectable mt-3 text-[15px] leading-relaxed">{withRenames(sum.overview, renames)}</p>

          {sum.decisions.length > 0 && (
            <>
              <h3 className="mt-5 mb-1.5 text-[15px] font-semibold">Decisions</h3>
              {sum.decisions.map((d) => (
                <p key={`${d.atMs}-${d.text}`} className="selectable text-[15px]">
                  {withRenames(d.text, renames)} <TimeLink ms={d.atMs} />
                </p>
              ))}
            </>
          )}

          {items.length > 0 && (
            <>
              <h3 className="mt-5 mb-1.5 text-[15px] font-semibold">Action items</h3>
              {items.map((a) => (
                <div key={a.id} className="flex items-center gap-3 py-1 text-[15px]">
                  <input type="checkbox" checked={a.done} onChange={() => s.toggleAction(a.id)} aria-label={a.text} />
                  <span className={`selectable flex-1 ${a.done ? "text-muted line-through" : ""}`}>
                    {withRenames(a.owner, renames)}: {withRenames(a.text, renames)} {a.due && `· due ${a.due}`} {a.needsReview && <span className="mi-chip mi-chip--review ml-1">check this</span>}
                  </span>
                  {a.atMs != null && <TimeLink ms={a.atMs} />}
                </div>
              ))}
            </>
          )}

          {sum.openQuestions.length > 0 && (
            <>
              <h3 className="mt-5 mb-1.5 text-[15px] font-semibold">Open questions</h3>
              {sum.openQuestions.map((d) => (
                <p key={`${d.atMs}-${d.text}`} className="selectable text-[15px]">
                  {withRenames(d.text, renames)} <TimeLink ms={d.atMs} />
                </p>
              ))}
            </>
          )}

          {sum.quality && (
            <div className="mt-5 flex items-center gap-2 border-t border-line pt-4 text-[14px] text-ink-3">
              <Icon name="tick" size={16} className="text-accent" />
              {sum.quality}
            </div>
          )}
        </section>
      ) : (
        <section className="mi-card mt-5 p-6 text-[14px] text-muted">
          {busy ? "Writing the summary on this device…" : live ? "The summary is written when you stop recording." : "No summary yet. Pick a template and write one."}
        </section>
      )}
    </div>
  );
}

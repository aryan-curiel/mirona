import { memo, useEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from "react";
import { Virtuoso, type VirtuosoHandle } from "react-virtuoso";
import { useStore, type Tab } from "../store";
import { api, fmt } from "../lib/api";
import { retime } from "../lib/notes";
import { Avatar, Chip, Icon, Label, Logo, Mono } from "../components/ui";
import { RecordButton, useModelNames } from "../components/Shell";
import type { Meeting } from "../bindings/Meeting";
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
        <button type="button" className="mi-ghost min-h-11!" onClick={s.importAudio}>
          Import an audio file…
        </button>
        <button type="button" className="mi-ghost min-h-11!" onClick={s.addDemo}>
          Add demo meetings
        </button>
      </div>
      <p className="mt-3 text-[12px] text-muted">Demo meetings are marked as sample data; delete their folders to remove them.</p>
    </div>
  );
}

/** The meeting's recording, if any. `play(ms)` seeks and plays. */
function useAudio(m: Meeting) {
  const ref = useRef<HTMLAudioElement>(null);
  const [src, setSrc] = useState<string | null>(null);
  const live = useStore((s) => s.recording?.meetingId === m.id);
  useEffect(() => {
    let off = false;
    setSrc(null);
    if (!live) api.audioSrc(m.id).then((s) => !off && setSrc(s), () => {});
    return () => {
      off = true;
    };
  }, [m.id, live]);
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
  const busy = useStore((s) => s.processing.includes(m.id));
  const live = useStore((s) => s.recording?.meetingId === m.id);
  const [title, setTitle] = useState(m.title);
  useEffect(() => setTitle(m.title), [m.title]);
  const a = m.answers;
  return (
    <header data-tauri-drag-region>
      <div className="flex flex-wrap items-center gap-2">
        {a && <Chip kind="action">{a.meetingType.value} · {a.meetingType.probability.toFixed(2)}</Chip>}
        {a &&
          (a.client.confirmed ? (
            <Chip kind="action">{a.client.value}</Chip>
          ) : (
            <Chip kind="review">{a.client.value}? · {a.client.probability.toFixed(2)}</Chip>
          ))}
        {m.sample && <span className="text-[12px] font-medium text-muted">Sample data</span>}
        {busy && <span className="mi-chip mi-chip--review mi-pulse">Writing the summary on this device…</span>}
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

  // Jumps from the summary and notes.
  const { scrollTo } = s;
  useEffect(() => {
    if (!scrollTo) return;
    const index = rows.findIndex((r) => r.id === scrollTo);
    if (index >= 0) setTimeout(() => list.current?.scrollToIndex({ index, align: "center" }), 30);
    useStore.setState({ scrollTo: null });
  }, [scrollTo, rows]);

  // Follow new segments while recording.
  useEffect(() => {
    if (live && rows.length && !q) list.current?.scrollToIndex({ index: rows.length - 1, align: "end", behavior: "smooth" });
  }, [live, rows.length, q]);

  return (
    <div>
      {audio.src && (
        <div className="sticky top-2 z-10 mb-4">
          {/* biome-ignore lint/a11y/useMediaCaption: no captions file; the transcript below is the text */}
          <audio ref={audio.ref} src={audio.src} controls preload="metadata" className="w-full" aria-label="Meeting recording" />
        </div>
      )}
      <div className="flex flex-wrap items-center gap-x-6 gap-y-3 text-[14px]">
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
            ["onlyTagged", "Only tagged segments"],
            ["showSensitive", "Show sensitive text"],
          ] as const
        ).map(([k, label]) => (
          <label key={k} className="flex items-center gap-2">
            <input type="checkbox" checked={s.filters[k]} onChange={(e) => s.setFilter(k, e.target.checked)} />
            {label}
          </label>
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
          <div className="pb-3">
            <SegmentRow
              seg={seg}
              meetingId={m.id}
              speaker={m.speakers.find((p) => p.id === seg.speakerId)}
              suggest={firstOf.has(seg.id)}
              selected={s.segmentId === seg.id}
              showSensitive={s.filters.showSensitive}
              query={q}
              onPlay={audio.src ? audio.play : undefined}
            />
          </div>
        )}
      />}
      {live && s.recording ? (
        <div className="mi-live flex gap-4 px-5 py-4 text-[14px] text-ink-3">
          <Mono>{fmt(s.recording.elapsedMs)}</Mono>
          <span className="mi-pulse">Listening… new segments appear here a few seconds after they are spoken.</span>
        </div>
      ) : (
        !rows.length && <p className="py-10 text-center text-muted">{m.segments.length ? "No segments match." : "No speech in this meeting."}</p>
      )}
    </div>
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
};

const SegmentRow = memo(function SegmentRow({ seg, speaker: sp, suggest, selected, showSensitive, query, onPlay }: RowProps) {
  const select = useStore((s) => s.selectSegment);
  const enroll = useStore((s) => s.enroll);
  const edit = useStore((s) => s.editSegment);
  const [draft, setDraft] = useState<string | null>(null);
  const save = () => {
    if (draft?.trim() && draft.trim() !== seg.text) void edit(seg.id, draft);
    setDraft(null);
  };
  const hidden = seg.redacted && !showSensitive;
  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: mouse shortcut; the Inspect button is the keyboard path
    // biome-ignore lint/a11y/useKeyWithClickEvents: same as above
    <div className="mi-row flex gap-4 px-5 py-4" aria-current={selected} onClick={() => select(seg.id)}>
      {sp && <Avatar tint={sp.tint} initial={sp.initial} />}
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2.5">
          <span className="text-[15px] font-semibold">{sp?.name ?? seg.speakerId}</span>
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
          {suggest && sp?.suggestion && (
            <Chip kind="review" onClick={() => sp.suggestion && enroll(sp.id, sp.suggestion.name)}>
              {sp.suggestion.name}? {sp.suggestion.probability.toFixed(2)} from “{sp.suggestion.evidence}” · Confirm
            </Chip>
          )}
          {seg.tags.map((t) => (
            <Chip key={t.label} kind={t.kind}>{t.label}</Chip>
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
          <p className="mt-1 text-[15px] italic text-sensitive">[Credentials redacted on this device]</p>
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
          className={selected ? "mi-btn-primary h-9 px-3.5 text-[13px]" : "mi-ghost text-[13px]"}
        >
          {selected ? "Inspecting" : "Inspect"}
        </button>
        {!hidden && draft === null && (
          <button
            type="button"
            className="rounded px-1 text-[12px] font-medium text-accent-strong hover:underline"
            onClick={(e) => {
              e.stopPropagation();
              setDraft(seg.text);
            }}
          >
            Fix text
          </button>
        )}
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
  const busy = s.regenerating === m.id || s.processing.includes(m.id);
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
          <button type="button" className="font-medium text-accent-strong underline underline-offset-2" onClick={() => s.go("models")}>
            Change model
          </button>
        </span>
      </div>

      {sum ? (
        <section className={`mi-card mi-enter mt-5 p-6 ${busy ? "opacity-60" : ""}`}>
          <div className="flex flex-wrap items-baseline gap-x-3">
            <Label>Template: {sum.template}</Label>
            <span className="text-[13px] text-muted">Written by {sum.writtenBy || "an unknown model"}</span>
          </div>
          <p className="selectable mt-3 text-[15px] leading-relaxed">{sum.overview}</p>

          {sum.decisions.length > 0 && (
            <>
              <h3 className="mt-5 mb-1.5 text-[15px] font-semibold">Decisions</h3>
              {sum.decisions.map((d) => (
                <p key={`${d.atMs}-${d.text}`} className="selectable text-[15px]">
                  {d.text} <TimeLink ms={d.atMs} />
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
                    {a.owner}: {a.text} {a.due && `· due ${a.due}`} {a.needsReview && <span className="mi-chip mi-chip--review ml-1">check this</span>}
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
                  {d.text} <TimeLink ms={d.atMs} />
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

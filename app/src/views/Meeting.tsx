import { useEffect, useRef, useState } from "react";
import { useStore, type Tab } from "../store";
import { api, fmt } from "../lib/api";
import { Avatar, Chip, Icon, Label, Mono } from "../components/ui";
import type { Meeting } from "../bindings/Meeting";
import type { TagKind } from "../bindings/TagKind";

const notePill: Record<string, string> = { action: "Action", decision: "Decision", openQuestion: "Open" };

/** Switch to the transcript and select the segment at `ms`. */
function useJump() {
  const s = useStore();
  return (ms: number) => {
    const seg = s.meeting?.segments.find((x) => x.startMs === ms);
    if (seg) {
      s.setTab("transcript");
      s.selectSegment(seg.id);
      setTimeout(() => document.getElementById(`seg-${seg.id}`)?.scrollIntoView({ behavior: "smooth", block: "center" }), 50);
    }
  };
}

export function MeetingView() {
  const m = useStore((s) => s.meeting);
  const tab = useStore((s) => s.tab);
  const setTab = useStore((s) => s.setTab);
  if (!m) return null;
  return (
    <div className="mx-auto max-w-[1040px] px-10 pt-8 pb-16">
      <MeetingHeader m={m} />
      <div role="tablist" className="mt-5 mb-5 flex gap-1 border-b border-line">
        {(["transcript", "notes", "summary"] as Tab[]).map((t) => (
          <button type="button" key={t} role="tab" aria-selected={tab === t} onClick={() => setTab(t)} className="mi-tab px-4 pb-2.5 text-[15px] font-medium text-muted capitalize">
            {t}
          </button>
        ))}
      </div>
      {tab === "transcript" && <Transcript m={m} />}
      {tab === "notes" && <Notes m={m} />}
      {tab === "summary" && <SummaryTab m={m} />}
    </div>
  );
}

function MeetingHeader({ m }: { m: Meeting }) {
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
      </div>
      <h1 className="mt-3 text-[32px] font-bold leading-tight tracking-[-0.02em]">{m.title}</h1>
      <p className="mt-1.5 text-[14px] text-muted">
        {m.dateLabel} · {m.durationMin ? `${m.durationMin} min` : "live"} · {m.speakers.length} speakers · {m.language}
      </p>
    </header>
  );
}

function Transcript({ m }: { m: Meeting }) {
  const s = useStore();
  const live = s.recording?.meetingId === m.id && !s.recording.processing;
  const end = useRef<HTMLDivElement>(null);
  // biome-ignore lint/correctness/useExhaustiveDependencies: scroll again whenever a segment arrives
  useEffect(() => {
    if (live) end.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [m.segments.length, live]);

  const has = (seg: (typeof m.segments)[number], k: TagKind) => seg.tags.some((t) => t.kind === k);
  const count = (k: TagKind) => m.segments.filter((x) => has(x, k)).length;
  const counts = [
    [count("action"), "action item", "action items"],
    [count("decision"), "decision", "decisions"],
    [count("openQuestion"), "open question", "open questions"],
    [count("sensitive"), "sensitive", "sensitive"],
    [count("review"), "to review", "to review"],
  ]
    .filter(([n]) => n)
    .map(([n, one, many]) => `${n} ${n === 1 ? one : many}`)
    .join(" · ");

  const rows = m.segments.filter(
    (x) => !(s.filters.hideSmallTalk && has(x, "smallTalk")) && !(s.filters.onlyTagged && !x.tags.length),
  );
  const firstSeen = new Set<string>();

  return (
    <div>
      <div className="flex flex-wrap items-center gap-6 text-[14px]">
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
      <p className="mt-3 mb-3 text-right text-[13px] text-muted">{counts || " "}</p>

      <div className="flex flex-col gap-3">
        {rows.map((seg) => {
          const sp = m.speakers.find((p) => p.id === seg.speakerId);
          const selected = s.segmentId === seg.id;
          const showSuggestion = sp?.suggestion && !firstSeen.has(sp.id);
          if (sp) firstSeen.add(sp.id);
          return (
            // biome-ignore lint/a11y/noStaticElementInteractions: mouse shortcut; the Inspect button is the keyboard path
            // biome-ignore lint/a11y/useKeyWithClickEvents: same as above
            <div
              key={seg.id}
              id={`seg-${seg.id}`}
              className="mi-row mi-enter flex gap-4 px-5 py-4"
              aria-current={selected}
              onClick={() => s.selectSegment(seg.id)}
            >
              {sp && <Avatar tint={sp.tint} initial={sp.initial} />}
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-2.5">
                  <span className="text-[15px] font-semibold">{sp?.name}</span>
                  <Mono>{fmt(seg.startMs)}</Mono>
                  {showSuggestion && sp.suggestion && (
                    <Chip kind="review" onClick={() => sp.suggestion && s.enroll(sp.id, sp.suggestion.name)}>
                      {sp.suggestion.name}? {sp.suggestion.probability.toFixed(2)} from “{sp.suggestion.evidence}” · Confirm
                    </Chip>
                  )}
                  {seg.tags.map((t) => (
                    <Chip key={t.label} kind={t.kind}>{t.label}</Chip>
                  ))}
                </div>
                {seg.redacted && !s.filters.showSensitive ? (
                  <p className="mt-1 text-[15px] italic text-sensitive">[Credentials redacted on this device]</p>
                ) : (
                  <p className="selectable mt-1 text-[15px] leading-relaxed text-ink-2">{seg.text}</p>
                )}
              </div>
              <button type="button"
                onClick={(e) => {
                  e.stopPropagation();
                  s.selectSegment(seg.id);
                }}
                className={selected ? "mi-btn-primary h-9 self-start px-3.5 text-[13px]" : "mi-ghost self-start text-[13px]"}
              >
                {selected ? "Inspecting" : "Inspect"}
              </button>
            </div>
          );
        })}
        {live && s.recording && (
          <div className="mi-live flex gap-4 px-5 py-4 text-[14px] text-ink-3">
            <Mono>{fmt(s.recording.elapsedMs)}</Mono>
            <span className="mi-pulse">Listening… new segments appear here, with speaker and tags, a few seconds after they are spoken.</span>
          </div>
        )}
        {!rows.length && !live && <p className="py-10 text-center text-muted">No segments match these filters.</p>}
        <div ref={end} />
      </div>
    </div>
  );
}

function Notes({ m }: { m: Meeting }) {
  const save = useStore((s) => s.flash);
  const [text, setText] = useState(m.notes.raw.map((l) => `- ${l}`).join("\n"));
  const [enhanced, setEnhanced] = useState(true);
  const jump = useJump();
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset the editor only when switching meetings, not on every save
  useEffect(() => setText(m.notes.raw.map((l) => `- ${l}`).join("\n")), [m.id]);

  const persist = async () => {
    const raw = text.split("\n").map((l) => l.replace(/^-\s*/, "").trim()).filter(Boolean);
    await api.saveNotes(m.id, raw);
    save("Notes saved to this meeting.");
  };

  return (
    <div className={`grid gap-5 ${enhanced ? "grid-cols-[minmax(0,0.75fr)_minmax(0,1fr)]" : "grid-cols-1"}`}>
      <section className="mi-card flex flex-col p-6">
        <Label>Your notes · typed during the call</Label>
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          onBlur={persist}
          placeholder="- type rough notes here"
          className="selectable mt-3 min-h-40 flex-1 resize-none bg-transparent font-mono text-[14px] leading-[2] outline-none"
        />
        <button type="button" className="mi-ghost mt-4 self-start border-accent! text-accent-strong" onClick={() => setEnhanced(!enhanced)}>
          {enhanced ? "Show my notes only" : "Enhance with transcript"}
        </button>
      </section>
      {enhanced && (
        <section className="mi-card mi-enter p-6">
          <Label>Enhanced notes</Label>
          {m.notes.enhanced.length ? (
            <>
              <ul className="mt-3 flex flex-col gap-3.5">
                {m.notes.enhanced.map((n) => (
                  <li key={n.atMs} className="flex items-start gap-3">
                    <Chip kind={n.kind}>{notePill[n.kind] ?? n.kind}</Chip>
                    <span className="flex-1 text-[15px] leading-snug">{n.text}</span>
                    <button type="button" onClick={() => jump(n.atMs)} className="font-mono text-[12px] text-muted hover:text-accent hover:underline">
                      {fmt(n.atMs)}
                    </button>
                  </li>
                ))}
              </ul>
              <p className="mt-5 text-[13px] text-muted">
                Your lines, expanded with the transcript. Written by {m.notes.enhancedBy}; each line links back to its moment in the call.
              </p>
            </>
          ) : (
            <p className="mt-3 text-[14px] text-muted">Enhanced notes appear when you stop recording.</p>
          )}
        </section>
      )}
    </div>
  );
}

function SummaryTab({ m }: { m: Meeting }) {
  const s = useStore();
  const jump = useJump();
  const llm = s.stages.find((x) => x.id === "llm");
  const items = s.actionItems.filter((a) => a.meetingId === m.id);
  const sum = m.summary;
  return (
    <div>
      <div className="flex flex-wrap items-center gap-2">
        <span className="mr-1 text-[14px] text-muted">Written by</span>
        {llm?.options.map((o) => (
          <button type="button"
            key={o.id}
            className="mi-option text-[14px]"
            aria-pressed={llm.selected === o.id}
            disabled={s.privacy && o.locality === "cloud"}
            onClick={() => s.selectProvider("llm", o.id)}
          >
            {o.name} <span className="text-[11px] opacity-80">{o.locality === "local" ? "Local" : "Cloud"}</span>
          </button>
        ))}
      </div>
      {s.privacy && <p className="mt-2 text-[12px] text-muted">Cloud options are off while privacy mode is on.</p>}

      {sum ? (
        <section className="mi-card mi-enter mt-5 p-6">
          <div className="flex items-baseline gap-2">
            <Label>Template: {sum.template}</Label>
            {m.answers && <span className="text-[13px] text-muted">picked by classification (meeting type {m.answers.meetingType.probability.toFixed(2)})</span>}
          </div>
          <p className="selectable mt-3 text-[15px] leading-relaxed">{sum.overview}</p>

          {sum.decisions.length > 0 && (
            <>
              <h3 className="mt-5 mb-1.5 text-[15px] font-semibold">Decisions</h3>
              {sum.decisions.map((d) => (
                <p key={d.atMs} className="text-[15px]">
                  {d.text} <button type="button" className="font-mono text-[12px] text-muted hover:text-accent hover:underline" onClick={() => jump(d.atMs)}>{fmt(d.atMs)}</button>
                </p>
              ))}
            </>
          )}

          {items.length > 0 && (
            <>
              <h3 className="mt-5 mb-1.5 text-[15px] font-semibold">Action items</h3>
              {items.filter((a) => !a.needsReview).map((a) => (
                <label key={a.id} className="flex items-center gap-3 py-1 text-[15px]">
                  <input type="checkbox" checked={a.done} onChange={() => s.toggleAction(a.id)} />
                  <span className={a.done ? "text-muted line-through" : ""}>
                    {a.owner}: {a.text} {a.due && `· due ${a.due}`}
                  </span>
                </label>
              ))}
              {items.filter((a) => a.needsReview).map((a) => (
                <div key={a.id} className="mt-1.5">
                  <Chip kind="review" onClick={() => a.atMs != null && jump(a.atMs)}>
                    1 possible action item waiting for your review ({fmt(a.atMs ?? 0)})
                  </Chip>
                </div>
              ))}
            </>
          )}

          {sum.openQuestions.length > 0 && (
            <>
              <h3 className="mt-5 mb-1.5 text-[15px] font-semibold">Open questions</h3>
              {sum.openQuestions.map((d) => (
                <p key={d.atMs} className="text-[15px]">
                  {d.text} <button type="button" className="font-mono text-[12px] text-muted hover:text-accent hover:underline" onClick={() => jump(d.atMs)}>{fmt(d.atMs)}</button>
                </p>
              ))}
            </>
          )}

          <div className="mt-5 flex items-center gap-2 border-t border-line pt-4 text-[14px] text-ink-3">
            <Icon name="tick" size={16} className="text-accent" />
            {sum.quality}
          </div>
        </section>
      ) : (
        <section className="mi-card mt-5 p-6 text-[14px] text-muted">
          The summary is written when you stop recording, by the provider picked above.
        </section>
      )}
    </div>
  );
}

import { useState } from "react";
import { useStore } from "../store";
import { fmt, needsReview, priorityLabel, sure, toneLabel, typeLabel } from "../lib/api";
import { Label, Mono } from "../components/ui";
import type { Answer } from "../bindings/Answer";
import type { ChoiceAnswer } from "../bindings/ChoiceAnswer";
import type { Review } from "../bindings/Review";

/** Answers stored before thresholds were kept have 0: show them against the ADR default. */
const thresholdOf = (a: { threshold: number }) => a.threshold || 0.6;

/** A probability bar with the threshold marked. */
function Bar({ p, threshold, color = "var(--mi-accent)" }: { p: number; threshold?: number; color?: string }) {
  return (
    <div className="mi-bar flex-1">
      <i style={{ width: `${Math.max(p * 100, 1.5)}%`, background: color }} />
      {threshold != null && <b className="absolute -top-[4px] h-[14px] w-[2px] rounded-sm bg-ink" style={{ left: `${threshold * 100}%` }} title={`Threshold ${threshold.toFixed(2)}`} />}
    </div>
  );
}

function verdict(a: Answer) {
  if (a.source === "user") return { text: a.answer ? "Yes · you" : "No · you", cls: "mi-chip--action" };
  if (needsReview({ ...a, threshold: thresholdOf(a) })) return { text: "Needs review", cls: "mi-chip--review" };
  if (a.answer) return { text: "Yes", cls: "mi-chip--action" };
  return { text: "No", cls: "mi-chip--neutral" };
}

const link = "min-h-6 rounded px-1 text-[13px] font-medium text-accent-strong underline underline-offset-2";

function AnswerRow({ a, segmentId }: { a: Answer; segmentId: string }) {
  const correct = useStore((s) => s.correct);
  const t = thresholdOf(a);
  const v = verdict(a);
  const unsure = needsReview({ ...a, threshold: t });
  const color = a.source === "user" || (a.answer && !unsure) ? "var(--mi-accent)" : unsure ? "var(--mi-review-edge)" : "#B9B9B2";
  return (
    <div className="py-2">
      <div className="flex items-center justify-between gap-2 text-[14px]">
        <span>{a.question}</span>
        <span className={`mi-chip ${v.cls}`}>{v.text}</span>
      </div>
      <div className="mt-2 flex items-center gap-3">
        <Bar p={a.probability} threshold={t} color={color} />
        <Mono className="w-9 text-right text-ink-3!">{a.probability.toFixed(2)}</Mono>
        {unsure ? (
          <span className="flex gap-1">
            <button type="button" className={link} aria-label={`${a.question} Yes`} onClick={() => correct(segmentId, a.id, true)}>
              Yes
            </button>
            <button type="button" className={link} aria-label={`${a.question} No`} onClick={() => correct(segmentId, a.id, false)}>
              No
            </button>
          </span>
        ) : (
          <button type="button" className={link} aria-label={`Correct: ${a.question} ${a.answer ? "No" : "Yes"}`} onClick={() => correct(segmentId, a.id, !a.answer)}>
            Correct
          </button>
        )}
      </div>
    </div>
  );
}

const review = (question: string, value: string, c?: ChoiceAnswer): Review => ({
  segmentId: null,
  atMs: null,
  question,
  value,
  probability: c?.probability ?? 0,
  threshold: c?.threshold ?? 0,
});

/** Meeting type or client: the answer, and option chips to confirm one when unsure. */
function ChoiceRow({ question, title, c, label }: { question: "meeting_type" | "client"; title: string; c: ChoiceAnswer; label: (v: string) => string }) {
  const resolve = useStore((s) => s.resolve);
  const [open, setOpen] = useState(false);
  const ok = sure(c);
  const show = !ok || open;
  return (
    <div className="mt-3">
      <p className="text-[14px] text-muted">
        {title} · choice · threshold {thresholdOf(c).toFixed(2)}
      </p>
      <p className="text-[15px] font-semibold">
        {c.value ? label(c.value) : "None"}{" "}
        {c.confirmed ? (
          <span className="mi-chip mi-chip--action ml-1">Confirmed</span>
        ) : (
          <Mono>
            {c.probability.toFixed(2)}
            {ok ? "" : ", below threshold. Confirm:"}
          </Mono>
        )}
        {ok && (
          <button type="button" className={`${link} ml-2`} aria-expanded={open} onClick={() => setOpen(!open)}>
            {open ? "Done" : "Correct"}
          </button>
        )}
      </p>
      {show && c.options.length > 0 && (
        <div className="mt-2 flex flex-wrap gap-2">
          {c.options.map((o) => (
            <button
              type="button"
              key={o}
              aria-pressed={c.confirmed && c.value === o}
              className="mi-option text-[13px]"
              onClick={() => {
                setOpen(false);
                void resolve(review(question, o, c), true);
              }}
            >
              {o === "none" ? "No client" : label(o)}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function FollowUp() {
  const m = useStore((s) => s.meeting);
  const resolve = useStore((s) => s.resolve);
  const flash = useStore((s) => s.flash);
  const [shown, setShown] = useState(true);
  const a = m?.answers;
  if (!a) return null;
  const t = a.followUpThreshold || 0.6;
  const unsure = a.followUp && a.followUpProbability < t && a.followUpProbability !== 1;
  const r = review("follow_up", "yes");
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(a.followUpDraft);
      flash("Draft copied. Paste it into your mail app; Mirona never sends email.");
    } catch (e) {
      flash(`Couldn't copy: ${e}`, true);
    }
  };
  return (
    <>
      <div className="mt-3 flex items-center justify-between gap-2 text-[14px]">
        <span className="text-muted">Needs a follow-up email? · yes/no</span>
        <span className={`mi-chip ${unsure ? "mi-chip--review" : a.followUp ? "mi-chip--action" : "mi-chip--neutral"}`}>
          {unsure ? "Needs review" : a.followUp ? "Yes" : "No"} · {a.followUpProbability.toFixed(2)}
        </span>
      </div>
      <div className="mt-2 flex items-center gap-3">
        <Bar p={a.followUpProbability} threshold={t} />
        {unsure ? (
          <span className="flex gap-1">
            <button type="button" className={link} aria-label="Follow-up email needed: Yes" onClick={() => resolve(r, true)}>Yes</button>
            <button type="button" className={link} aria-label="Follow-up email needed: No" onClick={() => resolve(r, false)}>No</button>
          </span>
        ) : (
          <button type="button" className={link} aria-label={`Correct: follow-up email ${a.followUp ? "not needed" : "needed"}`} onClick={() => resolve(r, !a.followUp)}>
            Correct
          </button>
        )}
      </div>
      {a.followUp && !unsure &&
        (a.followUpDraft ? (
          <>
            <div className="mt-3 flex gap-2">
              <button type="button" className="mi-ghost min-h-11! flex-1" aria-expanded={shown} onClick={() => setShown(!shown)}>
                {shown ? "Hide follow-up draft" : "Show follow-up draft"}
              </button>
              <button type="button" className="mi-btn-primary h-11 px-4 text-[14px]" onClick={copy}>
                Copy
              </button>
            </div>
            {shown && (
              <div className="selectable mi-enter mt-3 rounded-lg bg-[rgba(255,250,240,0.7)] p-3.5 text-[13px] leading-relaxed whitespace-pre-line">{a.followUpDraft}</div>
            )}
            <p className="mt-2 text-[12px] text-muted">Sending stays with you: copy it into your mail app.</p>
          </>
        ) : (
          <button type="button" className="mi-ghost mt-3 min-h-11! w-full" onClick={() => resolve(r, true)}>
            Write follow-up draft
          </button>
        ))}
    </>
  );
}

const reviewLabel: Record<string, string> = {
  action: "Action item?",
  decision: "Decision?",
  unanswered: "Open question?",
  smalltalk: "Small talk?",
  sensitive: "Sensitive?",
  due: "Due date said?",
  follow_up: "Needs a follow-up email?",
  action_item: "Action item from the summary",
  meeting_type: "Meeting type",
  client: "Client",
};

/** Everything below its threshold, each settled in one click (NGR-158). */
function ReviewList() {
  const items = useStore((s) => s.review);
  const m = useStore((s) => s.meeting);
  const resolve = useStore((s) => s.resolve);
  const jump = useStore((s) => s.jump);
  const showSensitive = useStore((s) => s.filters.showSensitive);
  if (!m || !items.length) return null;
  // The line a review is about; a sensitive one is quoted only with "Show sensitive text" on.
  const textAt = (r: Review) => {
    const seg = r.segmentId ? m.segments.find((x) => x.id === r.segmentId) : m.segments.find((x) => x.startMs === r.atMs);
    if (seg && (seg.redacted || seg.tags.some((t) => t.kind === "sensitive")) && !showSensitive) return "[sensitive: hidden]";
    return r.segmentId ? seg?.text : r.question === "action_item" ? r.value : null;
  };
  return (
    <>
      <hr className="my-6 border-line" />
      <div className="flex items-center gap-2">
        <Label>Needs review</Label>
        <span className="mi-chip mi-chip--review">{items.length}</span>
      </div>
      <p className="mt-2 text-[13px] text-muted">Below its threshold, nothing here is filed or sent to agents as fact until you confirm it.</p>
      <ul className="mt-2 flex flex-col">
        {items.map((r) => {
          const text = textAt(r);
          const choice = r.question === "meeting_type" ? m.answers?.meetingType : r.question === "client" ? m.answers?.client : undefined;
          return (
            <li key={`${r.question}-${r.segmentId ?? r.value}-${r.atMs}`} className="border-t border-line-soft py-2.5 first:border-t-0">
              <div className="flex items-center gap-2 text-[14px]">
                <span className="flex-1 font-medium">{reviewLabel[r.question] ?? r.question}</span>
                {r.atMs != null && (
                  <button type="button" className="font-mono text-[12px] text-muted hover:text-accent hover:underline" aria-label={`Show ${fmt(r.atMs)} in the transcript`} onClick={() => jump(r.atMs ?? 0)}>
                    {fmt(r.atMs)}
                  </button>
                )}
                {r.probability > 0 && <Mono>{r.probability.toFixed(2)} / {r.threshold.toFixed(2)}</Mono>}
              </div>
              {text && <p className="mt-1 line-clamp-2 text-[13px] text-ink-3">“{text}”</p>}
              {choice ? (
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {choice.options.map((o) => (
                    <button type="button" key={o} className="mi-option text-[13px]" aria-pressed={o === r.value} onClick={() => resolve({ ...r, value: o }, true)}>
                      {o === "none" ? "No client" : r.question === "meeting_type" ? typeLabel(o) : o}
                    </button>
                  ))}
                </div>
              ) : (
                <div className="mt-2 flex gap-2">
                  <button type="button" className="mi-ghost text-[13px]" onClick={() => resolve(r, true)}>
                    Confirm
                  </button>
                  <button type="button" className="mi-ghost text-[13px]" onClick={() => resolve(r, false)}>
                    Not this
                  </button>
                </div>
              )}
            </li>
          );
        })}
      </ul>
    </>
  );
}

/** Who owns the action: the model's choice among speakers, and the user's pick (NGR-159). */
function Owners({ segmentId }: { segmentId: string }) {
  const m = useStore((s) => s.meeting);
  const setOwner = useStore((s) => s.setOwner);
  const seg = m?.segments.find((x) => x.id === segmentId);
  if (!m || !seg) return null;
  const isAction = seg.answers.some((a) => a.id === "action" && a.answer) || seg.tags.some((t) => t.kind === "action");
  if (!isAction && !seg.owners.length) return null;
  const extra = m.speakers.filter((p) => !seg.owners.some((o) => o.label === p.name)).map((p) => ({ label: p.name, probability: 0 }));
  const rows = [...seg.owners, ...extra];
  const chosen = seg.owners[0]?.probability === 1 ? seg.owners[0].label : null;
  return (
    <div className="mt-2" role="radiogroup" aria-label="Who owns it?">
      <p className="text-[14px]">Who owns it? · choice among speakers</p>
      {rows.map((o) => (
        // biome-ignore lint/a11y/useSemanticElements: a row with a bar; role=radio on a button keeps it one tab stop per option
        <button
          type="button"
          role="radio"
          aria-checked={chosen === o.label}
          key={o.label}
          className="mi-nav mt-1 flex min-h-9 w-full items-center gap-3 px-1.5 text-left text-[14px]"
          onClick={() => setOwner(segmentId, o.label)}
        >
          <span className="w-24 truncate">{o.label}</span>
          <Bar p={o.probability} />
          <Mono className="text-ink-3!">{chosen === o.label ? "you" : o.probability.toFixed(2)}</Mono>
        </button>
      ))}
      <p className="mt-1 text-[12px] text-muted">Pick the owner to fix it; the tag and action item follow.</p>
    </div>
  );
}

export function Inspector() {
  const s = useStore();
  const m = s.meeting;
  if (!m) return null;
  const a = m.answers;
  const seg = m.segments.find((x) => x.id === s.segmentId);
  const sp = seg && m.speakers.find((p) => p.id === seg.speakerId);
  const st = s.settings;
  const classifier = !st ? "" : st.classification === "off" ? "Off" : st.classification === "local-llm" || st.privacy ? "qwen3:4b · local" : "Jev";
  const live = s.recording?.meetingId === m.id;
  const busy = live || s.processing.includes(m.id) || !!s.rerunning[m.id];
  const hidden = seg && (seg.redacted || seg.tags.some((t) => t.kind === "sensitive")) && !s.filters.showSensitive;

  return (
    <aside
      ref={(el) => {
        if (el && seg && el.dataset.seg !== seg.id) {
          el.dataset.seg = seg.id;
          el.scrollTo({ top: 0, behavior: "smooth" });
        }
      }}
      className="mi-panel h-full w-[var(--mi-inspector-w)] shrink-0 overflow-y-auto border-l border-white/60 px-6 pt-6 pb-8"
      aria-label="Inspector"
    >
      {/* The line you just inspected comes first, so its answers are right where you look. */}
      {seg && (
        <>
      <div className="flex items-center justify-between">
        <Label>Selected segment</Label>
        {seg && (
          <button type="button" className="text-[13px] text-accent-strong underline-offset-2 hover:underline" onClick={() => s.selectSegment("")}>
            Close
          </button>
        )}
      </div>
      {seg && sp ? (
        <div key={seg.id} className="mi-swap">
          <p className="mt-3 text-[15px] font-semibold">
            {sp.name} <Mono>{fmt(seg.startMs, true)}</Mono>
          </p>
          <blockquote className="mt-3 rounded-lg bg-[rgba(255,250,240,0.7)] p-3 text-[14px]">“{hidden ? "[sensitive: hidden]" : seg.text}”</blockquote>
          {seg.answers.length ? (
            <>
              <p className="mt-3 text-[13px] text-muted">
                {classifier} answered {seg.answers.length + (seg.owners.length ? 1 : 0)} typed questions in one call. The mark on each bar is its threshold; below it, you decide.
              </p>
              {seg.due && <p className="mt-2 text-[14px]">Due: <span className="font-medium">{seg.due}</span></p>}
              <div className="mt-2">
                {seg.answers.map((x) => (
                  <AnswerRow key={x.id} a={x} segmentId={seg.id} />
                ))}
              </div>
              <Owners segmentId={seg.id} />
            </>
          ) : (
            <p className="mt-3 text-[13px] text-muted">{live ? "Being tagged; answers appear in a few seconds." : "Not classified yet."}</p>
          )}
          <p className="mt-4 rounded-lg bg-accent-tint/70 p-3 text-[13px] text-accent-strong">
            {s.corrections} correction{s.corrections === 1 ? "" : "s"} saved as labelled examples for evaluating the classifier.
          </p>
        </div>
      ) : (
        <p className="mt-3 text-[13px] text-muted">Pick a segment to see the typed questions behind its tags.</p>
      )}

          <hr className="my-6 border-line" />
        </>
      )}
      <div className="flex items-center justify-between">
        <Label>This meeting</Label>
        <span className="rounded-md bg-[var(--mi-neutral-tint)] px-2 py-0.5 font-mono text-[12px] text-ink-3" title="Classifier">{classifier}</span>
      </div>

      {a ? (
        <div className="mi-enter">
          <ChoiceRow question="meeting_type" title="Meeting type" c={a.meetingType} label={typeLabel} />
          {(a.client.options.length > 0 || a.client.value) && <ChoiceRow question="client" title="Client" c={a.client} label={(v) => v} />}

          <p className="mt-3 text-[14px] text-muted">Priority · score</p>
          <div className="mt-2 flex items-center gap-3">
            <Bar p={a.priority / 5} />
            <Mono className="text-ink-3!">
              {a.priority.toFixed(1)} / 5 · {priorityLabel(a.priority)}
            </Mono>
          </div>
          <p className="mt-3 text-[14px] text-muted">Client tone · score</p>
          <div className="mt-2 flex items-center gap-3">
            <Bar p={a.tone / 4} />
            <Mono className="text-ink-3!">{toneLabel(a.tone)}</Mono>
          </div>
          <FollowUp />
        </div>
      ) : (
        <p className="mt-3 text-[13px] text-muted">Meeting type, client, priority, tone and follow-up are answered when you stop recording.</p>
      )}

      <ReviewList />

      {!seg && <p className="mt-6 text-[13px] text-muted">Click Inspect on a line to see the typed questions behind its tags.</p>}

      <hr className="my-6 border-line" />
      <Label>Redo for this meeting</Label>
      <div className="mt-3 flex flex-col gap-2">
        <button type="button" className="mi-ghost min-h-11!" disabled={busy || st?.diarization === "off"} onClick={() => s.rerun("diarization")}>
          Re-run speaker detection
        </button>
        <button type="button" className="mi-ghost min-h-11!" disabled={busy || st?.classification === "off"} onClick={() => s.rerun("classification")}>
          Re-run classification
        </button>
        <p className="text-[12px] text-muted">
          {s.rerunning[m.id] ?? "Uses the providers set in Models and MCP. Corrections are kept; named voices are matched again by voiceprint."}
        </p>
      </div>

      <hr className="my-6 border-line" />
      <div className="flex items-center gap-2">
        <Label>Agents · MCP</Label>
        <span className={`mi-chip ${st?.mcpEnabled ? "mi-chip--action" : "mi-chip--neutral"}`}>{st?.mcpEnabled ? "On" : "Off"}</span>
      </div>
      <p className="mt-2 text-[13px] text-muted">
        {st?.mcpEnabled ? "Claude and other MCP clients can read this meeting through the mirona CLI. Items needing review are marked as unconfirmed." : "MCP clients can't read meetings. Turn it on in Models and MCP."}
      </p>
    </aside>
  );
}

import { useState } from "react";
import { useStore } from "../store";
import { allTags, fmt, needsReview, priorityLabel, rejected, sure, toneLabel, typeLabel } from "../lib/api";
import { Label, Mono, Toggle } from "../components/ui";
import { Combobox } from "../components/Combobox";
import { tagClass } from "../lib/match";
import type { Answer } from "../bindings/Answer";
import type { ChoiceAnswer } from "../bindings/ChoiceAnswer";
import type { Review } from "../bindings/Review";
import type { Segment } from "../bindings/Segment";

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
  if (a.source === "user") return { text: a.answer ? "Yes (you)" : "No (you)", cls: "mi-chip--action" };
  if (needsReview({ ...a, threshold: thresholdOf(a) })) return { text: "Needs review", cls: "mi-chip--review" };
  if (rejected({ ...a, threshold: thresholdOf(a) })) return { text: "No (low confidence)", cls: "mi-chip--neutral" };
  if (a.answer) return { text: "Yes", cls: "mi-chip--action" };
  return { text: "No", cls: "mi-chip--neutral" };
}

const link = "min-h-6 rounded px-1 text-[13px] font-medium text-accent-strong underline underline-offset-2";

function AnswerRow({ a, segmentId }: { a: Answer; segmentId: string }) {
  const correct = useStore((s) => s.correct);
  const details = useStore((s) => s.modelDetails);
  const t = thresholdOf(a);
  const v = verdict(a);
  const unsure = needsReview({ ...a, threshold: t });
  const no = rejected({ ...a, threshold: t });
  const color = a.source === "user" || (a.answer && !unsure && !no) ? "var(--mi-accent)" : unsure ? "var(--mi-review-edge)" : "#B9B9B2";
  return (
    <div className="py-1.5">
      <div className="flex min-h-8 items-center gap-2 text-[14px]">
        <span className="flex-1">{a.question}</span>
        {unsure ? (
          <span className="flex items-center gap-1">
            <span className="mr-1 text-[13px] text-muted">Not sure:</span>
            <button type="button" className={link} aria-label={`${a.question} Yes`} onClick={() => correct(segmentId, a.id, true)}>
              Yes
            </button>
            <button type="button" className={link} aria-label={`${a.question} No`} onClick={() => correct(segmentId, a.id, false)}>
              No
            </button>
          </span>
        ) : (
          <>
            <span className={`mi-chip ${v.cls}`}>{v.text}</span>
            <button type="button" className={link} aria-label={`Change ${a.question} to ${a.answer && !no ? "No" : "Yes"}`} onClick={() => correct(segmentId, a.id, !a.answer || no)}>
              Change
            </button>
          </>
        )}
      </div>
      {details && (
        <div className="mt-1.5 flex items-center gap-3">
          <Bar p={a.probability} threshold={t} color={color} />
          <Mono className="w-9 text-right text-ink-3!">{a.probability.toFixed(2)}</Mono>
        </div>
      )}
    </div>
  );
}

/** The yes and unsure answers; the "no"s fold away unless model details are on. */
function Answers({ seg }: { seg: Segment }) {
  const details = useStore((s) => s.modelDetails);
  const [all, setAll] = useState(false);
  const isNo = (a: Answer) => (!a.answer && !needsReview({ ...a, threshold: thresholdOf(a) })) || rejected({ ...a, threshold: thresholdOf(a) });
  const shown = details || all ? seg.answers : seg.answers.filter((a) => !isNo(a));
  const hidden = seg.answers.length - shown.length;
  return (
    <div className="mt-2">
      {shown.map((x) => (
        <AnswerRow key={x.id} a={x} segmentId={seg.id} />
      ))}
      {!shown.length && <p className="py-1.5 text-[14px] text-ink-3">Nothing to note on this line.</p>}
      {hidden > 0 && (
        <button type="button" className={`${link} mt-1`} onClick={() => setAll(true)}>
          {hidden} more answered No
        </button>
      )}
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
  rejected: false,
});

/** The meeting's tags: remove one with its ×, add a known one or type a new one (it joins the
 * list in Settings). Unsure ones wait in "To check". */
function TagsRow() {
  const m = useStore((s) => s.meeting);
  const st = useStore((s) => s.settings);
  const meetings = useStore((s) => s.meetings);
  const setTags = useStore((s) => s.setTags);
  if (!m) return null;
  const known = allTags(st, meetings).filter((t) => !m.tags.includes(t));
  const add = (raw: string) => {
    const t = raw.trim();
    if (t && !m.tags.some((x) => x.toLowerCase() === t.toLowerCase())) void setTags([...m.tags, t]);
  };
  return (
    <div className="mt-3">
      <div className="flex min-h-8 items-center gap-2 text-[14px]">
        <span className="shrink-0 text-muted">Tags</span>
        {m.tags.length === 0 && <span className="flex-1 text-right text-ink-3">None</span>}
      </div>
      <div className="mt-1 flex flex-wrap items-center gap-1.5">
        {m.tags.map((t) => (
          <span key={t} className={`mi-chip ${tagClass(t)} gap-1`}>
            {t}
            <button type="button" className="-mr-1 rounded px-1 opacity-70 hover:opacity-100" aria-label={`Remove tag ${t}`} onClick={() => void setTags(m.tags.filter((x) => x !== t))}>
              ×
            </button>
          </span>
        ))}
        <Combobox
          className="min-w-40 flex-1"
          label="Add a tag"
          placeholder="Add a tag, e.g. project:setr"
          options={known.map((t) => ({ value: t, render: <span className={`mi-chip ${tagClass(t)}`}>{t}</span> }))}
          allowNew={(t) => (t.trim() ? `Add “${t.trim()}”` : null)}
          onPick={add}
        />
      </div>
    </div>
  );
}

/** Meeting type: the answer, and option chips to confirm one when unsure. */
function ChoiceRow({ question, title, c, label }: { question: "meeting_type"; title: string; c: ChoiceAnswer; label: (v: string) => string }) {
  const resolve = useStore((s) => s.resolve);
  const details = useStore((s) => s.modelDetails);
  const [open, setOpen] = useState(false);
  const ok = sure(c);
  // Unsure answers are settled in "To check" above; here they only open on "Change".
  const show = open;
  return (
    <div className="mt-3">
      <div className="flex min-h-8 items-center gap-2 text-[14px]">
        {/* Long values are cut with "…"; the label, chip and link keep their size. */}
        <span className="shrink-0 whitespace-nowrap text-muted">{title}</span>
        <span className="min-w-0 flex-1 truncate text-right font-semibold" title={c.value && c.value !== "none" ? label(c.value) : undefined}>
          {c.value && c.value !== "none" ? label(c.value) : "None"}
          {ok ? "" : "?"}
        </span>
        {c.confirmed && <span className="mi-chip mi-chip--action shrink-0">You</span>}
        <button type="button" className={`${link} shrink-0`} aria-expanded={open} onClick={() => setOpen(!open)}>
          {open ? "Done" : "Change"}
        </button>
      </div>
      {details && !c.confirmed && (
        <div className="mt-1.5 flex items-center gap-3">
          <Bar p={c.probability} threshold={thresholdOf(c)} />
          <Mono className="text-ink-3!">{c.probability.toFixed(2)}</Mono>
        </div>
      )}
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
              {label(o)}
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
  const details = useStore((s) => s.modelDetails);
  const [shown, setShown] = useState(true);
  const a = m?.answers;
  if (!a) return null;
  const t = a.followUpThreshold || 0.6;
  const below = a.followUp && a.followUpProbability < t && a.followUpProbability !== 1;
  const unsure = below && a.followUpProbability >= a.followUpFloor;
  // Below the floor it counts as "not needed" (it shows under "Rejected" in To check).
  const needed = a.followUp && !(below && !unsure);
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
      <div className="mt-3 flex min-h-8 items-center gap-2 text-[14px]">
        <span className="flex-1 whitespace-nowrap text-muted">Follow-up email</span>
        {unsure ? (
          <span className="flex items-center gap-1">
            <span className="mr-1 text-[13px] text-muted">Not sure:</span>
            <button type="button" className={link} aria-label="Follow-up email needed: Yes" onClick={() => resolve(r, true)}>Yes</button>
            <button type="button" className={link} aria-label="Follow-up email needed: No" onClick={() => resolve(r, false)}>No</button>
          </span>
        ) : (
          <>
            <span className="font-semibold">{needed ? "Needed" : "Not needed"}</span>
            <button type="button" className={link} aria-label={`Change: follow-up email ${needed ? "not needed" : "needed"}`} onClick={() => resolve(r, !needed)}>
              Change
            </button>
          </>
        )}
      </div>
      {details && (
        <div className="mt-1.5 flex items-center gap-3">
          <Bar p={a.followUpProbability} threshold={t} />
          <Mono className="text-ink-3!">{a.followUpProbability.toFixed(2)}</Mono>
        </div>
      )}
      {needed && !unsure &&
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
  tag: "Tag this meeting?",
  new_tag: "New tag? (only when none fits)",
};

/** Everything below its threshold, each settled in one click (NGR-158). Answers below their
 * floor were rejected without asking: only counted, listed on demand to undo. */
function ReviewList() {
  const all = useStore((s) => s.review);
  const [showLow, setShowLow] = useState(false);
  const m = useStore((s) => s.meeting);
  const resolve = useStore((s) => s.resolve);
  const jump = useStore((s) => s.jump);
  const showSensitive = useStore((s) => s.filters.showSensitive);
  const details = useStore((s) => s.modelDetails);
  if (!m || !all.length) return null;
  const items = all.filter((r) => !r.rejected);
  const low = all.filter((r) => r.rejected);
  // The line a review is about; a sensitive one is quoted only with "Show sensitive text" on.
  const textAt = (r: Review) => {
    const seg = r.segmentId ? m.segments.find((x) => x.id === r.segmentId) : m.segments.find((x) => x.startMs === r.atMs);
    if (seg && (seg.redacted || seg.tags.some((t) => t.kind === "sensitive")) && !showSensitive) return "[sensitive: hidden]";
    if (r.question === "new_tag") {
      const about = m.answers?.tags.find((t) => t.tag === r.value)?.about;
      return about ? `${r.value}: ${about}` : r.value;
    }
    return r.segmentId ? seg?.text : r.question === "action_item" || r.question === "tag" ? r.value : null;
  };
  const row = (r: Review) => {
          const text = textAt(r);
          const choice = r.question === "meeting_type" ? m.answers?.meetingType : undefined;
          return (
            <li key={`${r.question}-${r.segmentId ?? r.value}-${r.atMs}`} className="border-t border-[var(--mi-review-edge)]/40 py-2.5 first:border-t-0">
              <div className="flex items-center gap-2 text-[14px]">
                <span className="flex-1 font-medium">{reviewLabel[r.question] ?? r.question}</span>
                {r.atMs != null && (
                  <button type="button" className="font-mono text-[12px] text-muted hover:text-accent hover:underline" aria-label={`Show ${fmt(r.atMs)} in the transcript`} onClick={() => jump(r.atMs ?? 0)}>
                    {fmt(r.atMs)}
                  </button>
                )}
                {details && r.probability > 0 && <Mono>{r.probability.toFixed(2)} / {r.threshold.toFixed(2)}</Mono>}
              </div>
              {text && <p className="mt-1 line-clamp-2 text-[13px] text-ink-3">“{text}”</p>}
              {choice ? (
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {choice.options.map((o: string) => (
                    <button type="button" key={o} className="mi-option text-[13px]" aria-pressed={o === r.value} onClick={() => resolve({ ...r, value: o }, true)}>
                      {typeLabel(o)}
                    </button>
                  ))}
                </div>
              ) : (
                <div className="mt-2 flex gap-2">
                  <button type="button" className="mi-ghost text-[13px]" onClick={() => resolve(r, true)}>
                    Yes
                  </button>
                  <button type="button" className="mi-ghost text-[13px]" onClick={() => resolve(r, false)}>
                    No
                  </button>
                </div>
              )}
            </li>
          );
  };
  return (
    <section className="mb-6 rounded-lg border border-dashed border-[var(--mi-review-edge)] bg-[var(--mi-review-tint)] p-4">
      {items.length > 0 && (
        <>
          <div className="flex items-center gap-2">
            <Label>To check</Label>
            <span className="mi-chip mi-chip--review">{items.length}</span>
          </div>
          <p className="mt-1.5 text-[13px] text-ink-3">Mirona wasn't sure about these. Agents see them as unconfirmed until you answer.</p>
          <ul className="mt-2 flex flex-col">{items.map(row)}</ul>
        </>
      )}
      {low.length > 0 && (
        <div className={items.length ? "mt-3 border-t border-[var(--mi-review-edge)]/40 pt-3" : ""}>
          <button type="button" className={`${link} -ml-1`} aria-expanded={showLow} onClick={() => setShowLow(!showLow)}>
            {low.length} rejected for low confidence · {showLow ? "Hide" : "Show"}
          </button>
          {showLow && (
            <>
              <p className="mt-1.5 text-[13px] text-ink-3">Below their floor (Settings › Advanced › Thresholds), so treated as “no”. Say Yes to keep one.</p>
              <ul className="mt-2 flex flex-col">{low.map(row)}</ul>
            </>
          )}
        </div>
      )}
    </section>
  );
}

/** Who owns the action: the model's pick among speakers, changed with one select (NGR-159). */
function Owners({ segmentId }: { segmentId: string }) {
  const m = useStore((s) => s.meeting);
  const setOwner = useStore((s) => s.setOwner);
  const details = useStore((s) => s.modelDetails);
  const seg = m?.segments.find((x) => x.id === segmentId);
  if (!m || !seg) return null;
  const isAction = seg.answers.some((a) => a.id === "action" && a.answer) || seg.tags.some((t) => t.kind === "action");
  if (!isAction && !seg.owners.length) return null;
  const extra = m.speakers.filter((p) => !seg.owners.some((o) => o.label === p.name)).map((p) => ({ label: p.name, probability: 0 }));
  const rows = [...seg.owners, ...extra];
  const chosen = seg.owners[0]?.probability === 1 ? seg.owners[0].label : null;
  return (
    <div className="mt-1.5">
      <label className="flex min-h-8 items-center gap-2 text-[14px]">
        <span className="flex-1">Who owns it?</span>
        {chosen && <span className="mi-chip mi-chip--action">You</span>}
        <select className="mi-field h-9 text-[14px]" value={seg.owners[0]?.label ?? ""} onChange={(e) => e.target.value && setOwner(segmentId, e.target.value)}>
          {!seg.owners.length && <option value="">Nobody yet</option>}
          {rows.map((o) => (
            <option key={o.label} value={o.label}>
              {o.label}
            </option>
          ))}
        </select>
      </label>
      {details &&
        seg.owners.map((o) => (
          <div key={o.label} className="mt-1 flex items-center gap-3 text-[13px]">
            <span className="w-24 truncate">{o.label}</span>
            <Bar p={o.probability} />
            <Mono className="text-ink-3!">{o.probability.toFixed(2)}</Mono>
          </div>
        ))}
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
  const classifier = !st ? "" : st.classification === "off" ? "Off" : st.classification === "local-llm" || st.privacy ? "qwen3:4b on this device" : "Jev";
  const live = s.recording?.meetingId === m.id;
  const hidden = seg && (seg.redacted || seg.tags.some((t) => t.kind === "sensitive")) && !s.filters.showSensitive;

  return (
    <aside
      ref={(el) => {
        if (el && seg && el.dataset.seg !== seg.id) {
          el.dataset.seg = seg.id;
          el.scrollTo({ top: 0, behavior: "smooth" });
        }
      }}
      className="mi-panel flex h-full w-[var(--mi-inspector-w)] shrink-0 flex-col overflow-y-auto border-l border-white/60 px-6 pt-6 pb-6"
      aria-label="Inspector"
    >
      {/* What needs you comes first. */}
      <ReviewList />

      {seg && sp && (
        <div key={seg.id} className="mi-swap">
          <div className="flex items-center justify-between">
            <Label>This line</Label>
            <button type="button" className="text-[13px] text-accent-strong underline-offset-2 hover:underline" onClick={() => s.selectSegment("")}>
              Close
            </button>
          </div>
          <p className="mt-3 text-[15px] font-semibold">
            {sp.name} <Mono>{fmt(seg.startMs, true)}</Mono>
          </p>
          <blockquote className="mt-2 rounded-lg bg-[rgba(255,250,240,0.7)] p-3 text-[14px]">“{hidden ? "[sensitive: hidden]" : seg.text}”</blockquote>
          {seg.answers.length ? (
            <>
              {seg.due && (
                <p className="mt-3 flex text-[14px]">
                  <span className="flex-1">Due</span>
                  <span className="font-medium">{seg.due}</span>
                </p>
              )}
              <Answers seg={seg} />
              <Owners segmentId={seg.id} />
            </>
          ) : (
            <p className="mt-3 text-[13px] text-muted">{live ? "Being tagged; answers appear in a few seconds." : "Not classified yet."}</p>
          )}
          <hr className="my-6 border-line" />
        </div>
      )}

      <Label>This meeting</Label>
      <TagsRow />
      {a ? (
        <div className="mi-enter">
          <ChoiceRow question="meeting_type" title="Meeting type" c={a.meetingType} label={typeLabel} />
          <Score label="Priority" value={priorityLabel(a.priority)} p={a.priority / 5} raw={`${a.priority.toFixed(1)} / 5`} />
          <Score label="Client tone" value={toneLabel(a.tone)} p={a.tone / 4} raw={`${a.tone.toFixed(1)} / 4`} />
          <FollowUp />
        </div>
      ) : (
        <p className="mt-3 text-[13px] text-muted">Meeting type, tags, priority, tone and follow-up are answered when you stop recording.</p>
      )}

      {!seg && <p className="mt-6 text-[13px] text-muted">Click a line in the transcript to see what Mirona noted about it.</p>}

      <div className="mt-auto flex items-center gap-3 border-t border-line pt-4">
        <Toggle on={s.modelDetails} onChange={s.setModelDetails} label="Show model details" />
        {/* Label over the model name: side by side they wrap in a narrow inspector. */}
        <span className="min-w-0 flex-1">
          <span className="block whitespace-nowrap text-[13px] text-ink-3">Show model details</span>
          <span className="block truncate font-mono text-[12px] text-muted" title={`Classifier: ${classifier}`}>{classifier}</span>
        </span>
      </div>
      {s.modelDetails && <p className="mt-2 text-[12px] text-muted">Bars show how sure the classifier was; the mark is the threshold. Below it, you decide. Change thresholds in Settings → Advanced.</p>}
    </aside>
  );
}

/** Priority or tone: a word, with the score behind model details. */
function Score({ label, value, p, raw }: { label: string; value: string; p: number; raw: string }) {
  const details = useStore((s) => s.modelDetails);
  return (
    <div className="mt-3">
      <div className="flex min-h-8 items-center gap-2 text-[14px]">
        <span className="shrink-0 whitespace-nowrap text-muted">{label}</span>
        <span className="min-w-0 flex-1 truncate text-right font-semibold" title={value}>{value}</span>
      </div>
      {details && (
        <div className="mt-1.5 flex items-center gap-3">
          <Bar p={p} />
          <Mono className="text-ink-3!">{raw}</Mono>
        </div>
      )}
    </div>
  );
}

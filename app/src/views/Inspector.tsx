import { useState } from "react";
import { useStore } from "../store";
import { fmt } from "../lib/api";
import { Label, Mono } from "../components/ui";
import type { Answer } from "../bindings/Answer";

const THRESHOLD = 0.6;
const UNSURE = 0.4;

function verdict(a: Answer) {
  if (a.source === "user") return { text: a.answer ? "Yes · you" : "No · you", cls: "mi-chip--action" };
  if (a.probability >= THRESHOLD) return { text: "Yes", cls: "mi-chip--action" };
  if (a.probability >= UNSURE) return { text: "Unsure", cls: "mi-chip--review" };
  return { text: "No", cls: "mi-chip--neutral" };
}

function AnswerRow({ a, segmentId }: { a: Answer; segmentId: string }) {
  const correct = useStore((s) => s.correct);
  const v = verdict(a);
  const color = a.answer || a.probability >= THRESHOLD ? "var(--mi-accent)" : a.probability >= UNSURE ? "var(--mi-review-edge)" : "#B9B9B2";
  return (
    <div className="py-2">
      <div className="flex items-center justify-between gap-2 text-[14px]">
        <span>{a.question}</span>
        <span className={`mi-chip ${v.cls}`}>{v.text}</span>
      </div>
      <div className="mt-2 flex items-center gap-3">
        <div className="mi-bar flex-1">
          <i style={{ width: `${Math.max(a.probability * 100, 1.5)}%`, background: color }} />
          {/* threshold marker */}
          <b className="absolute -top-[4px] h-[14px] w-[2px] rounded-sm bg-ink" style={{ left: `${THRESHOLD * 100}%` }} />
        </div>
        <Mono className="w-9 text-right text-ink-3!">{a.probability.toFixed(2)}</Mono>
        <button type="button" className="text-[13px] font-medium text-accent underline underline-offset-2" onClick={() => correct(segmentId, a.id, !a.answer)}>
          Correct
        </button>
      </div>
    </div>
  );
}

export function Inspector() {
  const s = useStore();
  const [draft, setDraft] = useState(false);
  const m = s.meeting;
  if (!m) return null;
  const a = m.answers;
  const seg = m.segments.find((x) => x.id === s.segmentId);
  const sp = seg && m.speakers.find((p) => p.id === seg.speakerId);
  const classifier = "Classifier";

  return (
    <aside className="mi-panel h-full w-[var(--mi-inspector-w)] shrink-0 overflow-y-auto border-l border-white/60 px-6 pt-6 pb-8">
      <div className="flex items-center justify-between">
        <Label>This meeting</Label>
        <span className="rounded-md bg-[var(--mi-neutral-tint)] px-2 py-0.5 font-mono text-[12px] text-ink-3">{classifier}</span>
      </div>

      {a ? (
        <div className="mi-enter">
          <p className="mt-3 text-[14px] text-muted">Meeting type · choice</p>
          <p className="text-[15px] font-semibold">
            {a.meetingType.value} <Mono>{a.meetingType.probability.toFixed(2)}</Mono>
          </p>

          <p className="mt-3 text-[14px] text-muted">Client · choice · threshold {a.client.threshold.toFixed(2)}</p>
          {a.client.confirmed ? (
            <p className="text-[15px] font-semibold">{a.client.value} <span className="mi-chip mi-chip--action ml-1">Confirmed</span></p>
          ) : (
            <>
              <p className="text-[15px] font-semibold">
                {a.client.value} · {a.client.probability.toFixed(2)}, below threshold. Confirm:
              </p>
              <div className="mt-2 flex flex-wrap gap-2">
                {a.client.options.map((c) => (
                  <button type="button" key={c} className="mi-ghost text-[13px]" onClick={() => s.confirmClient(c)}>
                    {c}
                  </button>
                ))}
              </div>
            </>
          )}

          <p className="mt-3 text-[14px] text-muted">Priority · score</p>
          <div className="mt-2 flex items-center gap-3">
            <div className="mi-bar flex-1"><i style={{ width: `${(a.priority / 5) * 100}%` }} /></div>
            <Mono className="text-ink-3!">{a.priority.toFixed(1)} / 5</Mono>
          </div>

          <div className="mt-3 flex items-center justify-between text-[14px]">
            <span className="text-muted">Needs a follow-up email? · yes/no</span>
            <span className={`mi-chip ${a.followUp ? "mi-chip--action" : "mi-chip--neutral"}`}>
              {a.followUp ? "Yes" : "No"} · {a.followUpProbability.toFixed(2)}
            </span>
          </div>
          {a.followUp && (
            <>
              <button type="button" className="mi-ghost mt-3 w-full" onClick={() => setDraft(!draft)}>
                {draft ? "Hide follow-up draft" : "Draft follow-up email"}
              </button>
              {draft && (
                <div className="selectable mi-enter mt-3 rounded-lg bg-[rgba(255,250,240,0.7)] p-3.5 text-[13px] leading-relaxed whitespace-pre-line">
                  {a.followUpDraft}
                </div>
              )}
            </>
          )}
        </div>
      ) : (
        <p className="mt-3 text-[13px] text-muted">Meeting type, client, priority and follow-up are answered when you stop recording.</p>
      )}

      <hr className="my-6 border-line" />
      <Label>Selected segment</Label>
      {seg && sp ? (
        <div key={seg.id} className="mi-enter">
          <p className="mt-3 text-[15px] font-semibold">
            {sp.name} <Mono>{fmt(seg.startMs, true)}</Mono>
          </p>
          <blockquote className="mt-3 rounded-lg bg-[rgba(255,250,240,0.7)] p-3 text-[14px]">
            “{seg.redacted && !s.filters.showSensitive ? "[redacted]" : seg.text}”
          </blockquote>
          <p className="mt-3 text-[13px] text-muted">
            {classifier} answered {seg.answers.length + (seg.owners.length ? 1 : 0)} typed questions in one call. Threshold {THRESHOLD.toFixed(2)}; below it, you decide.
          </p>
          <div className="mt-2">
            {seg.answers.map((x) => (
              <AnswerRow key={x.id} a={x} segmentId={seg.id} />
            ))}
          </div>
          {seg.owners.length > 0 && (
            <div className="mt-2">
              <p className="text-[14px]">Who owns it? · choice among speakers</p>
              {seg.owners.map((o) => (
                <div key={o.label} className="mt-2 flex items-center gap-3 text-[14px]">
                  <span className="w-20 truncate">{o.label}</span>
                  <div className="mi-bar flex-1"><i style={{ width: `${Math.max(o.probability * 100, 3)}%` }} /></div>
                  <Mono className="text-ink-3!">{o.probability.toFixed(2)}</Mono>
                </div>
              ))}
            </div>
          )}
          <p className="mt-4 rounded-lg bg-accent-tint/70 p-3 text-[13px] text-accent-strong">
            {s.corrections} correction{s.corrections === 1 ? "" : "s"} saved as labelled examples for evaluating the classifier.
          </p>
        </div>
      ) : (
        <p className="mt-3 text-[13px] text-muted">Pick a segment to see the typed questions behind its tags.</p>
      )}

      <hr className="my-6 border-line" />
      <div className="flex items-center gap-2">
        <Label>Agents · MCP</Label>
        <span className={`mi-chip ${s.settings?.mcpEnabled ? "mi-chip--action" : "mi-chip--neutral"}`}>{s.settings?.mcpEnabled ? "On" : "Off"}</span>
      </div>
      <p className="mt-2 text-[13px] text-muted">
        {s.settings?.mcpEnabled ? "Claude and other MCP clients can read this meeting through the mirona CLI." : "MCP clients can't read meetings. Turn it on in Models and MCP."}
      </p>
    </aside>
  );
}

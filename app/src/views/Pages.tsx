import { useState } from "react";
import { useStore } from "../store";
import { fmt } from "../lib/api";
import { Avatar, Icon, PageHeader } from "../components/ui";

const clientChip: Record<string, string> = {
  "Alder Restoration": "mi-chip--action",
  "Juniper Apparel": "mi-chip--decision",
  Corvo: "mi-chip--question",
};

export function ActionItemsView() {
  const s = useStore();
  const [client, setClient] = useState<string | null>(null);
  const clients = [...new Set(s.actionItems.map((a) => a.client))];
  const rows = s.actionItems.filter((a) => !client || a.client === client);
  return (
    <div className="mx-auto max-w-[1100px] px-10 pt-8 pb-16">
      <PageHeader title="Action items">
        Detected by classification across meetings. Agents read the same list through <code className="font-mono text-[13px]">list_action_items</code>.
      </PageHeader>
      <div className="mb-4 flex flex-wrap gap-2">
        {[null, ...clients].map((c) => (
          <button type="button"
            key={c ?? "all"}
            onClick={() => setClient(c)}
            className={`h-9 rounded-full px-4 text-[14px] font-medium ${client === c ? "bg-ink text-white" : "mi-ghost rounded-full!"}`}
          >
            {c ?? "All"}
          </button>
        ))}
      </div>
      <div className="mi-card overflow-hidden">
        {rows.map((a, i) => (
          <div key={a.id} className={`mi-enter grid grid-cols-[auto_1fr_110px_56px_auto] items-center gap-4 px-6 py-4 ${i ? "border-t border-line-soft" : ""}`}>
            <input type="checkbox" checked={a.done} onChange={() => s.toggleAction(a.id)} aria-label={a.text} />
            <button type="button" className="text-left" onClick={() => s.openMeeting(a.meetingId)}>
              <p className={`text-[15px] ${a.done ? "text-muted line-through" : ""}`}>
                {a.text} {a.needsReview && <span className="mi-chip mi-chip--review ml-1">needs review</span>}
              </p>
              <p className="text-[13px] text-muted">
                {a.meetingTitle} · {a.atMs != null ? fmt(a.atMs) : "sample"}
              </p>
            </button>
            <span className="text-[15px]">{a.owner}</span>
            <span className="font-mono text-[13px] text-ink-3">{a.due ?? "—"}</span>
            <span className={`mi-chip justify-self-end ${clientChip[a.client] ?? "mi-chip--neutral"}`}>{a.client}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

export function PeopleView() {
  const s = useStore();
  return (
    <div className="mx-auto max-w-[1100px] px-10 pt-8 pb-16">
      <PageHeader title="People and voices">
        Diarization splits each call by voice; enrolled voiceprints turn “Speaker 2” into a name across meetings. Voiceprints never leave this device.
      </PageHeader>
      <div className="grid grid-cols-[repeat(auto-fill,minmax(280px,1fr))] gap-4">
        {s.people.map((p) => (
          <section key={p.id} className="mi-card mi-enter p-5">
            <div className="flex items-start gap-3">
              <Avatar tint={p.tint} initial={p.initial} size={44} />
              <div>
                <p className="text-[16px] font-semibold">{p.name}</p>
                <p className="text-[13px] text-muted">{p.subtitle}</p>
              </div>
            </div>
            {p.voiceprint && (
              <p className="mi-chip mi-chip--action mt-3 w-full">Voiceprint enrolled · {p.voiceprint.meetings} meeting{p.voiceprint.meetings === 1 ? "" : "s"}</p>
            )}
            {p.suggestion && (
              <>
                <p className="mi-chip mi-chip--review mt-3 w-full">
                  Suggested name: {p.suggestion.name} · {p.suggestion.probability.toFixed(2)} from context
                </p>
                <button type="button" className="mi-ghost mt-3 w-full" onClick={() => p.suggestion && s.enroll(p.id, p.suggestion.name)}>
                  Enroll as {p.suggestion.name}
                </button>
              </>
            )}
          </section>
        ))}
      </div>
    </div>
  );
}

export function ModelsView() {
  const s = useStore();
  const mcp = s.mcp;
  return (
    <div className="mx-auto max-w-[1100px] px-10 pt-8 pb-16">
      <PageHeader title="Models and MCP">
        Every stage is a provider you can swap. Local runs by default; cloud options unlock only when privacy mode is off.
      </PageHeader>
      <div className="mi-card overflow-hidden">
        {s.stages.map((st, i) => (
          <div key={st.id} className={`grid grid-cols-[260px_1fr] items-center gap-6 px-6 py-4 ${i ? "border-t border-line-soft" : ""}`}>
            <div>
              <p className="text-[15px] font-semibold">{st.name}</p>
              <p className="text-[13px] text-muted">{st.subtitle}</p>
            </div>
            <div className="flex flex-wrap gap-2">
              {st.options.map((o) => (
                <button type="button"
                  key={o.id}
                  className="mi-option text-[14px]"
                  aria-pressed={st.selected === o.id}
                  disabled={s.privacy && o.locality === "cloud"}
                  title={s.privacy && o.locality === "cloud" ? "Turn privacy mode off to use cloud providers" : undefined}
                  onClick={() => s.selectProvider(st.id, o.id)}
                >
                  {o.name} <span className="text-[11px] opacity-80">{o.locality === "local" ? "Local" : "Cloud"}</span>
                </button>
              ))}
            </div>
          </div>
        ))}
      </div>
      <p className="mt-3 text-[13px] text-muted">
        {s.privacy ? "Cloud options are off while privacy mode is on." : "Privacy mode is off: cloud providers can be picked. Turning it back on resets them to local."}
      </p>

      {mcp && (
        <section className="mi-card mt-5 p-6">
          <div className="flex items-center gap-3">
            <Icon name="plug" className="text-accent" />
            <span className="text-[16px] font-semibold">MCP server</span>
            <span className={`mi-chip ${mcp.running ? "mi-chip--action" : "mi-chip--neutral"}`}>{mcp.running ? "Running" : "Stopped"}</span>
            <button type="button" className="mi-ghost ml-auto" onClick={() => s.setMcpRunning(!mcp.running)}>
              {mcp.running ? "Stop server" : "Start server"}
            </button>
          </div>
          <p className="mt-4 font-mono text-[13px] text-ink-3">{mcp.transport}</p>
          <div className="mt-3 flex flex-wrap gap-2">
            {mcp.tools.map((t) => (
              <span key={t} className="rounded-md bg-[var(--mi-neutral-tint)] px-2.5 py-1 font-mono text-[13px]">{t}</span>
            ))}
          </div>
          <p className="mt-4 text-[14px]">
            <b>Connected:</b> {mcp.running ? mcp.clients.join(", ") : "none"}
            <span className="ml-8"><b>Also ships:</b> a Claude Code skill and the <code className="font-mono">mirona</code> CLI</span>
          </p>
        </section>
      )}
    </div>
  );
}

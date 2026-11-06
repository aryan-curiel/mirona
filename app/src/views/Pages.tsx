import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { useStore } from "../store";
import { api, fmt, LLM_LOCAL, type Person, type Settings } from "../lib/api";
import { CopyLine } from "./Setup";
import { Avatar, Icon, PageHeader, Toggle } from "../components/ui";
import { shortcutLabel } from "../components/Shell";

const TINTS = ["mi-chip--action", "mi-chip--decision", "mi-chip--question"];
/** A stable tint per client name. */
const clientChip = (c: string) => (c ? TINTS[[...c].reduce((h, ch) => (h * 31 + ch.charCodeAt(0)) % 997, 0) % TINTS.length] : "mi-chip--neutral");

type Status = "all" | "open" | "done" | "review";
const STATUS: [Status, string][] = [
  ["all", "All"],
  ["open", "Open"],
  ["done", "Done"],
  ["review", "Needs review"],
];

function Pills<T extends string | null>({ value, options, onPick, label }: { value: T; options: [T, string][]; onPick: (v: T) => void; label: string }) {
  return (
    <fieldset className="flex flex-wrap gap-2" aria-label={label}>
      {options.map(([v, name]) => (
        <button
          type="button"
          key={v ?? "all"}
          aria-pressed={value === v}
          onClick={() => onPick(v)}
          className={`min-h-9 rounded-full px-4 text-[14px] font-medium ${value === v ? "bg-ink text-white" : "mi-ghost rounded-full!"}`}
        >
          {name}
        </button>
      ))}
    </fieldset>
  );
}

/** Across meetings, filtered like MCP's `list_action_items`: client is a case-insensitive part
 * of the name, "Open" is `open_only`; "Needs review" lists the unconfirmed ones (NGR-161). */
export function ActionItemsView() {
  const s = useStore();
  const [client, setClient] = useState<string | null>(null);
  const [status, setStatus] = useState<Status>("all");
  const clients = [...new Set(s.actionItems.map((a) => a.client))].sort();
  const rows = s.actionItems.filter(
    (a) =>
      (client === null || a.client.toLowerCase().includes(client.toLowerCase())) &&
      (status === "all" || (status === "open" && !a.done) || (status === "done" && a.done) || (status === "review" && a.needsReview)),
  );
  return (
    <div className="mx-auto max-w-[1100px] px-10 pt-8 pb-16">
      <PageHeader title="Action items">
        Detected by classification across meetings. Agents read the same list through <code className="font-mono text-[13px]">list_action_items</code>; items marked
        “needs review” reach them as unconfirmed.
      </PageHeader>
      <div className="mb-3 flex flex-wrap items-center gap-x-6 gap-y-3">
        <Pills label="Filter by client" value={client} options={[[null, "All clients"], ...clients.map((c): [string, string] => [c, c || "No client"])]} onPick={setClient} />
        <Pills label="Filter by status" value={status} options={STATUS} onPick={setStatus} />
      </div>
      <p className="mb-3 text-[13px] text-muted" aria-live="polite">
        {rows.length} item{rows.length === 1 ? "" : "s"}
      </p>
      <div className="mi-card overflow-hidden">
        {rows.map((a, i) => (
          <div key={a.id} className={`mi-enter grid grid-cols-[auto_1fr_120px_110px_auto] items-center gap-4 px-6 py-4 ${i ? "border-t border-line-soft" : ""}`}>
            <input type="checkbox" checked={a.done} onChange={() => s.toggleAction(a.id)} aria-label={`Done: ${a.text}`} />
            <div className="min-w-0">
              <p className={`text-[15px] ${a.done ? "text-muted line-through" : ""}`}>
                {a.text} {a.needsReview && <span className="mi-chip mi-chip--review ml-1">Needs review</span>}
              </p>
              <button
                type="button"
                className="text-left text-[13px] text-muted hover:text-accent hover:underline"
                aria-label={`Open ${a.meetingTitle}${a.atMs != null ? ` at ${fmt(a.atMs)}` : ""}`}
                onClick={() => (a.atMs != null ? s.openAt(a.meetingId, a.atMs) : s.openMeeting(a.meetingId))}
              >
                {a.meetingTitle}
                {a.atMs != null && ` · ${fmt(a.atMs)}`}
              </button>
            </div>
            <span className="truncate text-[15px]" title={a.ownerConfirmed ? "Owner set by you" : undefined}>
              {a.owner || <span className="text-muted">No owner</span>}
            </span>
            <span className="truncate text-[13px] text-ink-3">{a.due ?? "—"}</span>
            <span className={`mi-chip justify-self-end ${clientChip(a.client)}`}>{a.client || "No client"}</span>
          </div>
        ))}
        {!rows.length && <p className="px-6 py-10 text-center text-[14px] text-muted">{s.actionItems.length ? "No action items match these filters." : "No action items yet. They come from each meeting's summary."}</p>}
      </div>
    </div>
  );
}

function PersonCard({ p }: { p: Person }) {
  const s = useStore();
  const [name, setName] = useState<string | null>(null);
  const [into, setInto] = useState("");
  const [sure, setSure] = useState(false);
  const others = s.people.filter((o) => o.id !== p.id);
  return (
    <section className="mi-card mi-enter flex flex-col p-5" aria-label={p.name}>
      <div className="flex items-start gap-3">
        <Avatar tint={p.tint} initial={p.initial} size={44} />
        <div className="min-w-0 flex-1">
          {name === null ? (
            <p className="text-[16px] font-semibold">{p.name}</p>
          ) : (
            <form
              className="flex gap-2"
              onSubmit={(e) => {
                e.preventDefault();
                if (name.trim() && name.trim() !== p.name) void s.enroll(p.id, name.trim());
                setName(null);
              }}
            >
              {/* biome-ignore lint/a11y/noAutofocus: opened by the user's Rename click */}
              <input autoFocus className="mi-field h-9 min-w-0 flex-1 text-[14px]" aria-label={`New name for ${p.name}`} value={name} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => e.key === "Escape" && setName(null)} />
              <button type="submit" className="mi-ghost text-[13px]">Save</button>
            </form>
          )}
          <p className="text-[13px] text-muted">{p.subtitle}</p>
          <p className="text-[13px] text-ink-3">
            {p.meetings} meeting{p.meetings === 1 ? "" : "s"}
            {p.lastSeen && ` · last seen ${p.lastSeen.when} in ${p.lastSeen.title}`}
          </p>
        </div>
      </div>
      {p.voiceprint ? (
        <p className="mi-chip mi-chip--action mt-3 w-full">Voiceprint enrolled · {p.voiceprint.meetings} meeting{p.voiceprint.meetings === 1 ? "" : "s"}</p>
      ) : (
        <p className="mi-chip mi-chip--neutral mt-3 w-full">No voiceprint: not recognized in new meetings</p>
      )}
      {p.suggestion && (
        <>
          <p className="mi-chip mi-chip--review mt-3 w-full">
            Suggested name: {p.suggestion.name} · {p.suggestion.probability.toFixed(2)} from context
          </p>
          <button type="button" className="mi-ghost mt-3 min-h-11! w-full" onClick={() => p.suggestion && s.enroll(p.id, p.suggestion.name)}>
            Enroll as {p.suggestion.name}
          </button>
        </>
      )}
      <div className="mt-auto flex flex-wrap items-center gap-2 pt-4">
        <button type="button" className="mi-ghost text-[13px]" onClick={() => setName(p.name)}>
          Rename
        </button>
        {p.lastSeen && (
          <button type="button" className="mi-ghost text-[13px]" onClick={() => p.lastSeen && s.openMeeting(p.lastSeen.id)}>
            Open last meeting
          </button>
        )}
        {p.voiceprint &&
          (sure ? (
            <button type="button" className="mi-ghost text-[13px] text-record-text!" onBlur={() => setSure(false)} onClick={() => s.deleteVoiceprint(p.id).then(() => setSure(false))}>
              Delete voiceprint for good
            </button>
          ) : (
            <button type="button" className="mi-ghost text-[13px]" onClick={() => setSure(true)}>
              Delete voiceprint
            </button>
          ))}
      </div>
      {others.length > 0 && (
        <div className="mt-3 flex items-center gap-2">
          <select className="mi-field min-w-0 flex-1 text-[13px]" aria-label={`Merge ${p.name} into`} value={into} onChange={(e) => setInto(e.target.value)}>
            <option value="">Same person as…</option>
            {others.map((o) => (
              <option key={o.id} value={o.id}>
                {o.name}
                {o.lastSeen ? ` (${o.lastSeen.title})` : ""}
              </option>
            ))}
          </select>
          <button type="button" className="mi-ghost text-[13px]" disabled={!into} onClick={() => s.merge(p.id, into).then(() => setInto(""))}>
            Merge
          </button>
        </div>
      )}
    </section>
  );
}

export function PeopleView() {
  const people = useStore((s) => s.people);
  return (
    <div className="mx-auto max-w-[1100px] px-10 pt-8 pb-16">
      <PageHeader title="People and voices">
        Diarization splits each call by voice; enrolled voiceprints turn “Speaker 2” into a name across meetings. Voiceprints never leave this device. Renaming
        updates every transcript, summary and action item.
      </PageHeader>
      <div className="grid grid-cols-[repeat(auto-fill,minmax(300px,1fr))] gap-4">
        {people.map((p) => (
          <PersonCard key={p.id} p={p} />
        ))}
      </div>
      {!people.length && <p className="text-[14px] text-muted">Nobody yet. Speakers show up here after a recording.</p>}
    </div>
  );
}

function Row({ title, sub, children }: { title: string; sub: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[240px_1fr] items-start gap-6 border-t border-line-soft px-6 py-4 first:border-t-0">
      <div>
        <p className="text-[15px] font-semibold">{title}</p>
        <p className="text-[13px] text-muted">{sub}</p>
      </div>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

function Options<T extends string>({ value, options, onPick }: { value: T; options: { id: T; name: string; cloud?: boolean; off?: boolean; tag?: string }[]; onPick: (id: T) => void }) {
  return (
    <div className="flex flex-wrap gap-2">
      {options.map((o) => (
        <button
          type="button"
          key={o.id}
          className="mi-option text-[14px]"
          aria-pressed={value === o.id}
          disabled={o.off}
          title={o.off ? "Turn privacy mode off to use cloud providers" : undefined}
          onClick={() => onPick(o.id)}
        >
          {o.name} <span className="text-[11px] opacity-80">{o.tag ?? (o.cloud ? "Cloud" : "Local")}</span>
        </button>
      ))}
    </div>
  );
}

/** A text setting saved on blur or Enter. */
function Field({ label, value, placeholder, onSave, type = "text" }: { label: string; value: string; placeholder?: string; onSave: (v: string) => void; type?: string }) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  return (
    <label className="flex flex-col gap-1 text-[13px] text-muted">
      {label}
      <input
        type={type}
        className="mi-field w-full text-[14px] text-ink"
        value={v}
        placeholder={placeholder}
        onChange={(e) => setV(e.target.value)}
        onBlur={() => v !== value && onSave(v)}
        onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
      />
    </label>
  );
}

const isMac = navigator.userAgent.includes("Mac");

/** A pressed combination as an accelerator, e.g. ⌘⌥⇧R → "CmdOrCtrl+Alt+Shift+R"; null while it's only modifiers or plain typing. */
function combo(e: KeyboardEvent): string | null {
  if (["Meta", "Control", "Alt", "Shift"].includes(e.key) || !(e.metaKey || e.ctrlKey || e.altKey)) return null;
  const mods = [
    e.metaKey && (isMac ? "CmdOrCtrl" : "Super"),
    e.ctrlKey && (isMac ? "Ctrl" : "CmdOrCtrl"),
    e.altKey && "Alt",
    e.shiftKey && "Shift",
  ].filter(Boolean);
  return [...mods, e.code.replace(/^(Key|Digit)(?=.)/, "")].join("+");
}

/** The record shortcut: press a combination (saved at once) or type one (saved on Enter or blur). */
function ShortcutField({ value, onSave }: { value: string; onSave: (v: string) => Promise<boolean> }) {
  const [v, setV] = useState(value);
  const [failed, setFailed] = useState<string | null>(null);
  const sent = useRef(value);
  useEffect(() => {
    setV(value);
    sent.current = value;
  }, [value]);
  const commit = async (next: string) => {
    next = next.trim();
    if (!next || next === sent.current) return setV(value);
    sent.current = next;
    setFailed(null);
    if (!(await onSave(next))) {
      sent.current = value;
      setV(value);
      setFailed(next);
    }
  };
  return (
    <div>
      <div className="flex flex-wrap items-center gap-3">
        <input
          className="mi-field w-72 font-mono text-[14px]"
          aria-label="Record shortcut"
          value={v}
          spellCheck={false}
          onChange={(e) => setV(e.target.value)}
          onBlur={() => void commit(v)}
          onKeyDown={(e) => {
            if (e.key === "Enter") return e.currentTarget.blur();
            if (e.key === "Escape") return setV(value);
            const c = combo(e);
            if (!c) return;
            e.preventDefault();
            setV(c);
            void commit(c);
          }}
        />
        <kbd className="mi-chip mi-chip--neutral font-mono">{shortcutLabel(value)}</kbd>
      </div>
      <p className="mt-2 text-[12px] text-muted">
        Click the field and press the new combination, or type one like <span className="font-mono">CmdOrCtrl+Alt+Shift+R</span>. Needs Cmd, Ctrl or Option.
      </p>
      {failed && (
        <p className="mt-1 text-[12px] text-record-text" role="alert">
          {shortcutLabel(failed)} can't be used: another app may own it, or it isn't a valid shortcut. Still using {shortcutLabel(value)}.
        </p>
      )}
    </div>
  );
}

/** Models Mirona runs, with the credit their licenses ask for (NOTICE has the full text). */
const CREDITS: [string, string, string][] = [
  ["Parakeet TDT 0.6B v2", "NVIDIA · ONNX export by sherpa-onnx", "CC-BY-4.0"],
  ["Whisper", "OpenAI · ggml weights by whisper.cpp", "MIT"],
  ["Silero VAD", "Silero Team", "MIT"],
  ["Speaker models (M2)", "pyannote, WeSpeaker · packaged by speakrs", "MIT · CC-BY-4.0"],
];

function ApiKey({ provider, label, saved }: { provider: string; label: string; saved: boolean }) {
  const refresh = useStore((s) => s.refreshStatus);
  const flash = useStore((s) => s.flash);
  const [key, setKey] = useState("");
  const save = async (k: string) => {
    try {
      await api.setApiKey(provider, k);
      setKey("");
      flash(k ? "API key saved in the system keychain." : "API key removed.");
      await refresh(true);
    } catch (e) {
      flash(String(e), true);
    }
  };
  return (
    <div className="flex items-end gap-2">
      <label className="flex flex-1 flex-col gap-1 text-[13px] text-muted">
        {label} {saved ? "· saved in the keychain" : "· not set"}
        <input type="password" className="mi-field text-[14px] text-ink" value={key} placeholder={saved ? "••••••••  (enter a new key to replace it)" : "Paste your key"} onChange={(e) => setKey(e.target.value)} autoComplete="off" />
      </label>
      <button type="button" className="mi-ghost" disabled={!key.trim()} onClick={() => save(key)}>
        Save key
      </button>
      {saved && (
        <button type="button" className="mi-ghost" onClick={() => save("")}>
          Remove
        </button>
      )}
    </div>
  );
}

function Folder({ label, path, onPick }: { label: string; path: string; onPick: (p: string) => void }) {
  return (
    <div className="flex items-center gap-2">
      <code className="selectable min-w-0 flex-1 truncate rounded-md bg-[var(--mi-neutral-tint)] px-3 py-2 font-mono text-[13px]" title={path}>{path}</code>
      <button type="button" className="mi-ghost shrink-0" aria-label={`Choose ${label}`} onClick={async () => {
        const p = await api.pickFolder();
        if (p) onPick(p);
      }}>
        Choose…
      </button>
    </div>
  );
}

const QUESTION_LABELS: [string, string][] = [
  ["action", "Is this an action item?"],
  ["decision", "Is this a decision?"],
  ["unanswered", "Is this a question nobody answered?"],
  ["smalltalk", "Is this small talk?"],
  ["sensitive", "Is this sensitive?"],
  ["due", "Is a due date said?"],
  ["owner", "Who owns it?"],
  ["meeting_type", "Meeting type"],
  ["client", "Client"],
  ["follow_up", "Needs a follow-up email?"],
  ["coverage", "Summary covers every action item"],
];

/** One number per question; empty means the classifier's default. */
function Thresholds({ st, onSave }: { st: Settings; onSave: (t: Record<string, number>) => void }) {
  const set = (q: string, raw: string) => {
    const next = { ...st.thresholds };
    const v = Number.parseFloat(raw);
    if (raw.trim() === "" || Number.isNaN(v)) delete next[q];
    else next[q] = Math.min(1, Math.max(0, v));
    onSave(next);
  };
  return (
    <div>
      <div className="grid grid-cols-[1fr_auto_auto] items-center gap-x-4 gap-y-1.5 text-[14px]">
        {QUESTION_LABELS.map(([q, label]) => {
          const own = st.thresholds[q];
          const def = st.thresholdDefaults[q] ?? 0.6;
          return (
            <ThresholdRow key={q} label={label} value={own} def={def} onSave={(raw) => set(q, raw)} />
          );
        })}
      </div>
      <p className="mt-2 text-[12px] text-muted">Defaults come from the eval (ADR 0001) and are stricter for the local LLM. Leave a field empty to use the default.</p>
    </div>
  );
}

function ThresholdRow({ label, value, def, onSave }: { label: string; value?: number; def: number; onSave: (raw: string) => void }) {
  const [v, setV] = useState(value?.toString() ?? "");
  useEffect(() => setV(value?.toString() ?? ""), [value]);
  return (
    <>
      <span>{label}</span>
      <input
        type="number"
        min={0}
        max={1}
        step={0.05}
        aria-label={`Threshold: ${label}`}
        placeholder={def.toFixed(2)}
        className="mi-field w-24 font-mono text-[13px]"
        value={v}
        onChange={(e) => setV(e.target.value)}
        onBlur={() => v !== (value?.toString() ?? "") && onSave(v)}
        onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
      />
      <span className="w-28 text-[12px] text-muted">{value == null ? "default" : `default ${def.toFixed(2)}`}</span>
    </>
  );
}

/** Client list: name + short description, saved on blur. */
function Clients({ clients, onSave }: { clients: Record<string, string>; onSave: (c: Record<string, string>) => void }) {
  const [rows, setRows] = useState<[string, string][]>(() => Object.entries(clients));
  useEffect(() => setRows(Object.entries(clients)), [clients]);
  const commit = (next: [string, string][]) => {
    const named = next.filter(([n]) => n.trim());
    const out = Object.fromEntries(named.map(([n, d]) => [n.trim(), d.trim()]));
    if (JSON.stringify(out) !== JSON.stringify(clients)) onSave(out);
  };
  const edit = (i: number, k: 0 | 1, v: string) => setRows(rows.map((r, j) => (j === i ? (k ? [r[0], v] : [v, r[1]]) : r)));
  return (
    <div className="flex flex-col gap-2">
      {rows.map(([name, about], i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: rows are edited in place; a name can change while typing
        <div key={i} className="flex gap-2">
          <input className="mi-field w-48 text-[14px]" aria-label="Client name" placeholder="Client name" value={name} onChange={(e) => edit(i, 0, e.target.value)} onBlur={() => commit(rows)} />
          <input
            className="mi-field min-w-0 flex-1 text-[14px]"
            aria-label={`What ${name || "this client"} is`}
            placeholder="Contact, what they do, project names"
            value={about}
            onChange={(e) => edit(i, 1, e.target.value)}
            onBlur={() => commit(rows)}
          />
          <button type="button" className="mi-ghost text-[13px]" aria-label={`Remove ${name || "client"}`} onClick={() => {
              const next = rows.filter((_, j) => j !== i);
              setRows(next);
              commit(next);
            }}>
            Remove
          </button>
        </div>
      ))}
      <div>
        <button type="button" className="mi-ghost text-[13px]" onClick={() => setRows([...rows, ["", ""]])}>
          Add client
        </button>
      </div>
      <p className="text-[12px] text-muted">Meetings are filed under the client the classifier picks (filter by client in the sidebar and action items, or over MCP).</p>
    </div>
  );
}

const llmDefaults = {
  ollama: { model: "qwen3:4b", url: "http://localhost:11434/api/chat" },
  openai: { model: "gpt-4.1-mini", url: "https://api.openai.com/v1" },
  anthropic: { model: "claude-opus-5-5", url: "" },
};

export function ModelsView() {
  const s = useStore();
  const st = s.settings;
  const status = s.status;
  if (!st || !status) return null;
  const save = s.saveSettings;
  const privacy = st.privacy;
  const d = llmDefaults[st.llm];
  const desktop = JSON.stringify({ mcpServers: { mirona: { command: status.cli, args: ["mcp"] } } }, null, 2);
  return (
    <div className="mx-auto max-w-[1100px] px-10 pt-8 pb-16">
      <PageHeader title="Models and MCP">
        Every stage is a provider you can swap. Local runs by default; cloud options unlock only when privacy mode is off.
      </PageHeader>

      <section className="mi-card overflow-hidden">
        <Row title="Privacy mode" sub="Blocks every cloud provider">
          <div className="flex items-center gap-3">
            <Toggle on={privacy} onChange={s.setPrivacy} label="Privacy mode" />
            <span className="text-[14px] text-ink-3">
              {privacy ? "On: cloud options are off. Turning it on moves the summary back to Ollama." : "Off: cloud providers can be picked below."}
            </span>
          </div>
        </Row>
        <Row title="Speech to text" sub="Live transcription, on this device">
          <Options value={st.stt} options={[{ id: "parakeet", name: "Parakeet TDT 0.6B" }, { id: "whisper", name: "Whisper" }]} onPick={(stt) => save({ stt })} />
        </Row>
        <Row title="Summaries and notes" sub="Writes the summary, action items and enhanced notes">
          <Options
            value={st.llm}
            options={[
              { id: "ollama", name: "Ollama" },
              { id: "openai", name: "OpenAI-compatible", cloud: !LLM_LOCAL({ ...st, llm: "openai" }), off: privacy && !LLM_LOCAL({ ...st, llm: "openai" }) },
              { id: "anthropic", name: "Anthropic", cloud: true, off: privacy },
            ]}
            onPick={(llm) => save({ llm, llmModel: "", llmBaseUrl: "" })}
          />
          <div className="mt-3 grid grid-cols-2 gap-3">
            <Field label="Model" value={st.llmModel} placeholder={d.model} onSave={(llmModel) => save({ llmModel })} />
            {st.llm !== "anthropic" && (
              <Field
                label={st.llm === "openai" ? "Base URL (LM Studio, vLLM… on localhost count as local)" : "Ollama chat URL"}
                value={st.llmBaseUrl}
                placeholder={d.url}
                onSave={(llmBaseUrl) => save({ llmBaseUrl })}
              />
            )}
          </div>
          {st.llm !== "ollama" && !status.keys.includes(st.llm) && (
            <p className="mt-2 text-[12px] text-record-text">Add your {st.llm === "openai" ? "OpenAI" : "Anthropic"} API key under API keys below.</p>
          )}
          {privacy && <p className="mt-2 text-[12px] text-muted">Cloud options are off while privacy mode is on.</p>}
        </Row>
        <Row title="Default summary template" sub="Used after each recording; any meeting can be rewritten with another">
          <select className="mi-field text-[14px]" value={st.template} onChange={(e) => save({ template: e.target.value })} aria-label="Default summary template">
            {s.templates.map((t) => (
              <option key={t.id} value={t.id}>{t.name}</option>
            ))}
          </select>
        </Row>
        <Row title="Record shortcut" sub="Starts and stops a recording from any app">
          <ShortcutField value={st.shortcut} onSave={(shortcut) => save({ shortcut })} />
        </Row>
        <Row title="Classification" sub="Typed questions: action, decision, open question, meeting type, client…">
          <Options
            value={st.classification}
            options={[
              { id: "jev", name: "Jev (hosted)", cloud: true },
              { id: "local-llm", name: "Local LLM (qwen3:4b)" },
              { id: "off", name: "Off", tag: "" },
            ]}
            onPick={(classification) => save({ classification })}
          />
          {st.classification === "jev" && privacy && (
            <p className="mt-2 text-[12px] text-ink-3">Privacy mode is on, so the local LLM answers instead of Jev. Tags are less accurate in privacy mode.</p>
          )}
          {st.classification === "local-llm" && <p className="mt-2 text-[12px] text-ink-3">Runs through Ollama on this device. Tags are less accurate than with Jev.</p>}
          {st.classification === "jev" && !privacy && !status.keys.includes("jev") && (
            <p className="mt-2 text-[12px] text-record-text">Add your Jev key under API keys below; until then the local LLM answers.</p>
          )}
        </Row>
        <Row title="Speaker detection" sub="Splits “them” into each person after the call">
          <Options
            value={st.diarization}
            options={[
              { id: "speakrs", name: "speakrs" },
              { id: "speakrs-cpu", name: "speakrs, CPU only" },
              { id: "off", name: "Off", tag: "" },
            ]}
            onPick={(diarization) => save({ diarization })}
          />
          <p className="mt-2 text-[12px] text-muted">speakrs uses the Neural Engine on Macs with 16 GB or more, the CPU otherwise. CPU only is slower (about 4 minutes per hour of audio) but uses less memory.</p>
        </Row>
        <Row title="Thresholds" sub="Below its threshold, an answer is marked “Needs review” instead of filed as fact">
          <Thresholds st={st} onSave={(thresholds) => save({ thresholds })} />
        </Row>
        <Row title="Clients" sub="The classifier picks one per meeting; describe each so it can tell them apart">
          <Clients clients={st.clients} onSave={(clients) => save({ clients })} />
        </Row>
      </section>

      <section className="mi-card mt-5 overflow-hidden">
        <Row title="API keys" sub="Bring your own. Stored in the system keychain on this Mac, never in config.toml or sent anywhere else">
          <div className="flex flex-col gap-4">
            <ApiKey provider="openai" label="OpenAI (summaries)" saved={status.keys.includes("openai")} />
            <ApiKey provider="anthropic" label="Anthropic (summaries)" saved={status.keys.includes("anthropic")} />
            <ApiKey provider="jev" label="Jev by TypeSafe (classification)" saved={status.keys.includes("jev")} />
          </div>
        </Row>
      </section>

      <section className="mi-card mt-5 overflow-hidden">
        <Row title="Meetings folder" sub="One Markdown folder per meeting">
          <Folder label="meetings folder" path={st.meetingsDir} onPick={(meetingsDir) => save({ meetingsDir })} />
        </Row>
        <Row title="Models folder" sub="Speech and voice-activity models">
          <Folder label="models folder" path={st.modelsDir} onPick={(modelsDir) => save({ modelsDir })} />
          {status.missingModels.length ? (
            <div className="mt-2 text-[13px] text-record-text">
              Missing: <span className="font-mono">{status.missingModels.join(", ")}</span>. From the Mirona source folder, run:
              <CopyLine text={status.fetchCommand} />
            </div>
          ) : (
            <p className="mt-2 text-[13px] text-accent-strong">All models for {st.stt === "whisper" ? "Whisper" : "Parakeet"} are installed.</p>
          )}
        </Row>
      </section>

      <section className="mi-card mt-5 p-6">
        <div className="flex items-center gap-3">
          <Icon name="plug" className="text-accent" />
          <span className="flex-1 text-[16px] font-semibold">MCP server</span>
          <span className="text-[14px] text-ink-3">{st.mcpEnabled ? "Agents can read your meetings" : "Off: mirona mcp refuses to serve"}</span>
          <Toggle on={st.mcpEnabled} onChange={(mcpEnabled) => save({ mcpEnabled })} label="Let MCP clients read meetings" />
        </div>
        <p className="mt-4 text-[14px] text-ink-3">Claude Code:</p>
        <CopyLine text={`claude mcp add mirona -- ${status.cli.includes(" ") ? `"${status.cli}"` : status.cli} mcp`} />
        <p className="mt-4 text-[14px] text-ink-3">Claude Desktop (claude_desktop_config.json):</p>
        <CopyLine text={desktop} />
      </section>

      <section className="mi-card mt-5 p-6" aria-labelledby="credits">
        <h2 id="credits" className="text-[16px] font-semibold">
          Models and credits
        </h2>
        <p className="mt-1 text-[14px] text-ink-3">Mirona runs these open models on your Mac. Full notices are in the NOTICE file that ships with the app.</p>
        <ul className="mt-3 flex flex-col gap-2 text-[14px]">
          {CREDITS.map(([name, by, license]) => (
            <li key={name} className="flex flex-wrap gap-x-2">
              <span className="font-medium">{name}</span>
              <span className="text-ink-3">{by}</span>
              <span className="mi-chip mi-chip--neutral">{license}</span>
            </li>
          ))}
        </ul>
      </section>

      <p className="mt-5 text-[13px] text-muted">
        Every setting lives in <span className="selectable font-mono">{status.configPath}</span> and can be edited there too; Mirona reads it on start. API keys stay in the system keychain, never in that file.
      </p>
    </div>
  );
}

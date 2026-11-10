import { getVersion } from "@tauri-apps/api/app";
import { Fragment, useEffect, useState, type KeyboardEvent, type ReactNode } from "react";
import { type SettingsTab, useStore } from "../store";
import { allTags, api, LLM_LOCAL, type Settings } from "../lib/api";
import { applyPlatform, detected } from "../lib/platform";
import { tagClass } from "../lib/match";
import { CopyLine, Folder, ModelDownload } from "./Setup";
import { Icon, type IconName, Logo, QwenNote, Toggle } from "../components/ui";
import { shortcutKeys, shortcutLabel, Toast } from "../components/Shell";

function Row({ title, sub, children }: { title: string; sub: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[200px_1fr] items-start gap-6 border-t border-line-soft px-6 py-4 first:border-t-0">
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
function Field({ label, value, placeholder, onSave, type = "text", hint, hideLabel }: { label: string; value: string; placeholder?: string; onSave: (v: string) => void; type?: string; hint?: string; hideLabel?: boolean }) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  return (
    <label className="flex flex-col gap-1 text-[13px] text-muted">
      <span className={hideLabel ? "sr-only" : ""}>{label}</span>
      <input
        type={type}
        className="mi-field w-full text-[14px] text-ink"
        value={v}
        placeholder={placeholder}
        onChange={(e) => setV(e.target.value)}
        onBlur={() => v !== value && onSave(v)}
        onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
      />
      {hint && <span className="text-[12px]">{hint}</span>}
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

/** A global shortcut as key caps: click, then press the new combination (saved at once; Esc cancels). */
function ShortcutField({ label, value, onSave }: { label: string; value: string; onSave: (v: string) => Promise<boolean> }) {
  const [listening, setListening] = useState(false);
  const [pending, setPending] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const commit = async (next: string) => {
    setListening(false);
    if (next === value) return;
    setPending(next);
    setFailed(null);
    if (!(await onSave(next))) setFailed(next);
    setPending(null);
  };
  const keys = listening ? [] : shortcutKeys(pending ?? value);
  return (
    <div>
      <button
        type="button"
        className="mi-field flex min-h-11 w-72 items-center gap-1.5 text-left"
        aria-label={`${label}: ${shortcutLabel(value)}. Click and press a new combination.`}
        onClick={() => setListening(true)}
        onBlur={() => setListening(false)}
        onKeyDown={(e) => {
          if (!listening) return;
          e.preventDefault();
          if (e.key === "Escape") return setListening(false);
          const c = combo(e);
          if (c) void commit(c);
        }}
      >
        {listening ? (
          <span className="text-[13px] text-muted">Press the new combination… (Esc to cancel)</span>
        ) : (
          keys.map((k, i) => (
            <kbd
              // biome-ignore lint/suspicious/noArrayIndexKey: keys can repeat only in a malformed value; order is the identity
              key={i}
              className="inline-flex h-7 min-w-7 items-center justify-center rounded-md border border-b-2 border-[var(--mi-control-line)] bg-[var(--mi-neutral-tint)] px-1.5 font-sans text-[13px] font-medium text-[var(--mi-ink-2)]"
            >
              {k}
            </kbd>
          ))
        )}
      </button>
      <p className="mt-2 text-[12px] text-muted">Click and press the new combination. Needs {isMac ? "⌘, ⌃ or ⌥" : "Ctrl or Alt"}.</p>
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
  ["Speaker models", "pyannote, WeSpeaker · packaged by speakrs", "MIT · CC-BY-4.0"],
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

const QUESTION_LABELS: [string, string][] = [
  ["action", "Is this an action item?"],
  ["decision", "Is this a decision?"],
  ["unanswered", "Is this a question nobody answered?"],
  ["smalltalk", "Is this small talk?"],
  ["sensitive", "Is this sensitive?"],
  ["due", "Is a due date said?"],
  ["owner", "Who owns it?"],
  ["meeting_type", "Meeting type"],
  ["tag", "Is the meeting about this tag?"],
  ["follow_up", "Needs a follow-up email?"],
  ["coverage", "Summary covers every action item"],
];

/** Two numbers per question: at/above "Approve at" an answer is filed as fact; between the two it
 * needs review; below "Reject below" it is dropped without asking (only counted). Empty means the
 * default. Questions that never go to review have no floor. */
function Thresholds({ st, save }: { st: Settings; save: (s: Partial<Settings>) => void }) {
  const set = (key: "thresholds" | "floors", q: string, raw: string) => {
    const next = { ...st[key] };
    const v = Number.parseFloat(raw);
    if (raw.trim() === "" || Number.isNaN(v)) delete next[q];
    else next[q] = Math.min(1, Math.max(0, v));
    save({ [key]: next });
  };
  return (
    <div>
      <div className="grid grid-cols-[1fr_auto_auto] items-center gap-x-4 gap-y-1.5 text-[14px]">
        <span />
        <span className="text-[12px] text-muted">Approve at</span>
        <span className="text-[12px] text-muted">Reject below</span>
        {QUESTION_LABELS.map(([q, label]) => (
          <Fragment key={q}>
            <span>{label}</span>
            <NumberField label={`Approve at: ${label}`} value={st.thresholds[q]} def={st.thresholdDefaults[q] ?? 0.85} onSave={(raw) => set("thresholds", q, raw)} />
            {(st.floorDefaults[q] ?? 0) > 0 || st.floors[q] != null ? (
              <NumberField label={`Reject below: ${label}`} value={st.floors[q]} def={st.floorDefaults[q] ?? 0} onSave={(raw) => set("floors", q, raw)} />
            ) : (
              <span className="w-24 text-center text-[12px] text-muted">never</span>
            )}
          </Fragment>
        ))}
      </div>
      <p className="mt-2 text-[12px] text-muted">
        Defaults come from the eval (ADR 0001) and are stricter for the local LLM. Leave a field empty to use the default (shown greyed). Sensitive lines are never rejected without asking.
      </p>
    </div>
  );
}

function NumberField({ label, value, def, onSave }: { label: string; value?: number; def: number; onSave: (raw: string) => void }) {
  const [v, setV] = useState(value?.toString() ?? "");
  useEffect(() => setV(value?.toString() ?? ""), [value]);
  return (
    <input
      type="number"
      min={0}
      max={1}
      step={0.05}
      aria-label={label}
      placeholder={def.toFixed(2)}
      title={value == null ? "Default" : `Default ${def.toFixed(2)}`}
      className="mi-field w-24 font-mono text-[13px]"
      value={v}
      onChange={(e) => setV(e.target.value)}
      onBlur={() => v !== (value?.toString() ?? "") && onSave(v)}
      onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
    />
  );
}

type Row = { orig: string | null; name: string };
const toRows = (m: Record<string, string>): Row[] => Object.keys(m).map((name) => ({ orig: name, name }));

/** Tag names, saved on blur. A changed name of an existing row is a rename (`onRename`). */
function TagList({ items, onSave, onRename }: { items: Record<string, string>; onSave: (m: Record<string, string>) => void; onRename: (old: string, next: string) => void }) {
  const [rows, setRows] = useState<Row[]>(() => toRows(items));
  useEffect(() => setRows(toRows(items)), [items]);
  const commit = (next: Row[]) => {
    const renamed = next.find((r) => r.orig && r.name.trim() && r.name.trim() !== r.orig);
    if (renamed?.orig) return onRename(renamed.orig, renamed.name.trim());
    const out = Object.fromEntries(next.filter((r) => r.name.trim()).map((r) => [r.name.trim(), ""]));
    if (JSON.stringify(Object.keys(out)) !== JSON.stringify(Object.keys(items))) onSave(out);
  };
  return (
    <div className="flex flex-col gap-2">
      {rows.map((r, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: rows are edited in place; a name can change while typing
        <div key={i} className="flex items-center gap-2">
          {/* The group's colour, so tags in a group visibly belong together. */}
          <span aria-hidden="true" className={`mi-chip ${tagClass(r.name)} h-4 w-4 shrink-0 p-0`} />
          <input
            className="mi-field min-w-0 flex-1 text-[14px]"
            aria-label="Tag name"
            placeholder="project:setr"
            value={r.name}
            onChange={(e) => setRows(rows.map((x, j) => (j === i ? { ...x, name: e.target.value } : x)))}
            onBlur={() => commit(rows)}
          />
          <button
            type="button"
            className="mi-ghost text-[13px]"
            aria-label={`Remove ${r.name || "tag"}`}
            onClick={() => {
              const next = rows.filter((_, j) => j !== i);
              setRows(next);
              commit(next);
            }}
          >
            Remove
          </button>
        </div>
      ))}
      <div>
        <button type="button" className="mi-ghost text-[13px]" onClick={() => setRows([...rows, { orig: null, name: "" }])}>
          Add tag
        </button>
      </div>
    </div>
  );
}

/** Tags (`group:name` or `name`). Renaming a tag renames it on every meeting. */
function Tags({ st, save }: { st: Settings; save: (patch: Partial<Settings>) => Promise<boolean> }) {
  const flash = useStore((s) => s.flash);
  const rename = async (old: string, next: string) => {
    try {
      const n = await api.renameTag(old, next);
      flash(`Renamed ${old} to ${next}${n ? ` in ${n} meeting${n === 1 ? "" : "s"}` : ""}.`);
    } catch (e) {
      flash(String(e), true);
    }
  };
  return (
    <Row title="Tags" sub="A meeting can have several. The classifier asks about each one after every call, so name them clearly">
      <TagList items={st.tags} onSave={(tags) => void save({ tags })} onRename={(o, n) => void rename(o, n)} />
      <p className="mt-2 text-[12px] text-muted">
        Write tags as group:name, or just a name. Tags in the same group share a colour. Keep the list short: when no tag fits a meeting, Mirona suggests one new tag
        under To check, and adds it only if you say yes. Removing a tag here leaves it on past meetings.
      </p>
    </Row>
  );
}

/** Call detection (NGR-170): suggest, name from the calendar, and per-tag auto-record. */
function CallSettings({ st, onSave }: { st: Settings; onSave: (patch: Partial<Settings>) => Promise<boolean> }) {
  const flash = useStore((s) => s.flash);
  const calendar = async (on: boolean) => {
    if (on && !(await api.requestPermission("calendar").catch(() => false))) {
      flash("Mirona needs calendar access: System Settings › Privacy & Security › Calendars.", true);
      return;
    }
    await onSave({ calendarTitles: on });
  };
  const tags = Object.keys(st.tags);
  const auto = (c: string, on: boolean) => onSave({ autoRecord: on ? [...st.autoRecord, c] : st.autoRecord.filter((x) => x !== c) });
  return (
    <div className="flex flex-col gap-3 text-[14px]">
      <div className="flex items-center gap-3">
        <Toggle on={st.detectCalls} onChange={(detectCalls) => onSave({ detectCalls })} label="Suggest recording when a call starts" />
        <span>Suggest recording when a call starts</span>
      </div>
      <div className="flex items-center gap-3">
        <Toggle on={st.calendarTitles} onChange={(on) => void calendar(on)} label="Use calendar event titles" />
        <span>Use calendar event titles</span>
      </div>
      {st.calendarTitles && tags.length > 0 && (
        <fieldset className="flex flex-col gap-1.5">
          <legend className="mb-1 text-[13px] text-ink-3">Record without asking when the event title names this tag (project:setr matches “Setr”):</legend>
          {tags.map((c) => (
            <label key={c} className="flex items-center gap-2">
              <input type="checkbox" checked={st.autoRecord.includes(c)} onChange={(e) => void auto(c, e.target.checked)} />
              <span className={`mi-chip ${tagClass(c)}`}>{c}</span>
            </label>
          ))}
        </fieldset>
      )}
      <p className="text-[12px] text-muted">Mirona never records a call on its own unless you tick its tag here. A browser using the microphone counts as a call, since Meet can't be told apart from other tabs.</p>
    </div>
  );
}

/** MCP over localhost HTTP (NGR-168): on/off and port, one token per client, write tools. */
function McpHttp({ st, save }: { st: Settings; save: (patch: Partial<Settings>) => Promise<boolean> }) {
  const flash = useStore((s) => s.flash);
  const [name, setName] = useState("");
  const [created, setCreated] = useState<{ name: string; token: string } | null>(null);
  const url = `http://127.0.0.1:${st.mcpPort}/mcp`;
  const reload = async () => useStore.setState({ settings: await api.getSettings() });
  const add = async () => {
    try {
      const token = await api.addMcpClient(name);
      setCreated({ name: name.trim(), token });
      setName("");
      await reload();
    } catch (e) {
      flash(String(e), true);
    }
  };
  const revoke = async (n: string) => {
    try {
      await api.revokeMcpClient(n);
      if (created?.name === n) setCreated(null);
      await reload();
    } catch (e) {
      flash(String(e), true);
    }
  };
  return (
    <section className="mi-card mt-5 overflow-hidden">
      <Row title="MCP over HTTP" sub="For clients that connect to a URL instead of starting Mirona. Only this Mac can reach it">
        <div className="flex flex-wrap items-end gap-3">
          <Toggle on={st.mcpHttp} onChange={(mcpHttp) => save({ mcpHttp })} label="Serve MCP over HTTP" />
          <span className="text-[14px] text-ink-3">{!st.mcpEnabled ? "Off while the MCP server is off" : st.mcpHttp ? `Listening on ${url}` : "Off"}</span>
          <div className="w-28">
            <Field label="Port" value={String(st.mcpPort)} onSave={(v) => save({ mcpPort: Number.parseInt(v, 10) || st.mcpPort })} />
          </div>
        </div>
      </Row>
      <Row title="Client tokens" sub="One per app. The name shows in the activity log; revoking cuts that app off at once">
        <div className="flex flex-col gap-2">
          {st.mcpClients.map((n) => (
            <div key={n} className="flex items-center gap-2">
              <span className="flex-1 text-[14px]">{n}</span>
              <button type="button" className="mi-ghost text-[13px]" aria-label={`Revoke ${n}`} onClick={() => revoke(n)}>
                Revoke
              </button>
            </div>
          ))}
          <div className="flex gap-2">
            <input
              className="mi-field w-64 text-[14px]"
              aria-label="Client name"
              placeholder="Client name, e.g. Claude Desktop"
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && name.trim() && add()}
            />
            <button type="button" className="mi-ghost text-[13px]" disabled={!name.trim()} onClick={add}>
              Create token
            </button>
          </div>
          {created && (
            <div className="mt-2 text-[13px] text-ink-3" role="status">
              Token for {created.name}, shown only now. Claude Code:
              <CopyLine text={`claude mcp add --transport http mirona ${url} --header "Authorization: Bearer ${created.token}"`} />
              <p className="mt-2">Other clients: URL {url}, header</p>
              <CopyLine text={`Authorization: Bearer ${created.token}`} />
            </div>
          )}
        </div>
      </Row>
      <Row title="Write tools" sub="Agents can mark action items done and rename speakers. Over stdio and HTTP">
        <div className="flex items-center gap-3">
          <Toggle on={st.mcpWrite} onChange={(mcpWrite) => save({ mcpWrite })} label="Let MCP clients change meetings" />
          <span className="text-[14px] text-ink-3">{st.mcpWrite ? "On: agents can change your meetings" : "Off: agents can only read"}</span>
        </div>
      </Row>
      <Row title="Sensitive lines" sub="Passwords, salaries, account numbers, health. Agents see [redacted] unless you allow them">
        <div className="flex items-center gap-3">
          <Toggle on={st.mcpSensitive} onChange={(mcpSensitive) => save({ mcpSensitive })} label="Let MCP clients read sensitive lines" />
          <span className="text-[14px] text-ink-3">{st.mcpSensitive ? "On: agents can read them when they ask" : "Off: always redacted"}</span>
        </div>
      </Row>
    </section>
  );
}

const llmDefaults = {
  ollama: { model: "qwen3:4b", url: "http://localhost:11434/api/chat" },
  openai: { model: "gpt-4.1-mini", url: "https://api.openai.com/v1" },
  anthropic: { model: "claude-opus-5-5", url: "" },
};

/** The reminder to get consent before recording (NGR-169). */
function ConsentSettings({ st, save }: { st: Settings; save: (patch: Partial<Settings>) => Promise<boolean> }) {
  const [msg, setMsg] = useState(st.consentMessage);
  useEffect(() => setMsg(st.consentMessage), [st.consentMessage]);
  return (
    <div className="flex flex-col gap-3 text-[14px]">
      <div className="flex items-center gap-3">
        <Toggle on={st.consentReminder} onChange={(consentReminder) => save({ consentReminder })} label="Remind me to get consent before recording" />
        <span>Remind me to get consent before recording</span>
      </div>
      <label className="flex flex-col gap-1 text-[13px] text-muted">
        Message to paste in the meeting chat
        <textarea className="mi-field min-h-20 text-[14px] text-ink" value={msg} onChange={(e) => setMsg(e.target.value)} onBlur={() => msg !== st.consentMessage && save({ consentMessage: msg })} />
      </label>
      {st.consentGiven.length > 0 && (
        <div className="flex flex-col gap-1.5">
          <p className="text-[13px] text-ink-3">Already agreed, no reminder:</p>
          {st.consentGiven.map((c) => (
            <div key={c} className="flex items-center gap-2">
              <span className="flex-1">
                <span className={`mi-chip ${tagClass(c)}`}>{c}</span>
              </span>
              <button type="button" className="mi-ghost text-[13px]" onClick={() => save({ consentGiven: st.consentGiven.filter((x) => x !== c) })}>
                Remind me again
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

/** Encryption at rest and retention (NGR-165). */
const size = (bytes: number) =>
  bytes >= 1e9 ? `${(bytes / 1e9).toFixed(1)} GB` : bytes >= 1e6 ? `${Math.round(bytes / 1e6)} MB` : `${Math.max(1, Math.round(bytes / 1e3))} KB`;

function Storage({ st, save, tags }: { st: Settings; save: (patch: Partial<Settings>) => Promise<boolean>; tags: string[] }) {
  const flash = useStore((s) => s.flash);
  const reload = useStore((s) => s.reload);
  const [busy, setBusy] = useState(false);
  const [sure, setSure] = useState<string | null>(null);
  const [log, setLog] = useState<string[] | null>(null);
  const [usage, setUsage] = useState<[string, number][] | null>(null);
  // Refetch after anything that may change sizes (retention, encryption, folder moves).
  // biome-ignore lint/correctness/useExhaustiveDependencies: st and log are refresh triggers
  useEffect(() => void api.storageUsage().then(setUsage).catch(() => setUsage([])), [st, log]);
  const encrypt = async (on: boolean) => {
    setBusy(true);
    try {
      useStore.setState({ settings: await api.setEncryption(on) });
      flash(on ? "Meetings are encrypted on this Mac. The key is in your keychain." : "Encryption is off. Files are readable again.");
    } catch (e) {
      flash(String(e), true);
    }
    setBusy(false);
  };
  const days = (raw: string) => {
    const n = Number.parseInt(raw, 10);
    return raw.trim() === "" || Number.isNaN(n) ? null : n;
  };
  const rule = (c: string, raw: string) => {
    const next = { ...st.retentionTags };
    const n = days(raw);
    if (n == null) delete next[c];
    else next[c] = n;
    return save({ retentionTags: next });
  };
  const forget = async (c: string) => {
    if (sure !== c) return setSure(c);
    setSure(null);
    try {
      const n = await api.deleteTagged(c);
      flash(`Deleted ${n} meetings tagged ${c}.`);
      await reload();
    } catch (e) {
      flash(String(e), true);
    }
  };
  const run = async () => {
    try {
      setLog(await api.runRetention());
      await reload();
    } catch (e) {
      flash(String(e), true);
    }
  };
  return (
    <section className="mi-card mt-5 overflow-hidden">
      <Row title="Storage used" sub="On this Mac, by kind">
        {!usage ? (
          <p className="text-[13px] text-muted">Measuring…</p>
        ) : (
          <dl className="grid grid-cols-[minmax(0,1fr)_auto] gap-x-4 gap-y-1 text-[14px]">
            {usage.map(([label, n]) => (
              <div key={label} className="contents">
                <dt className="text-ink-3">{label}</dt>
                <dd className="text-right tabular-nums">{size(n)}</dd>
              </div>
            ))}
            <dt className="mt-1 border-t border-line-soft pt-1 font-semibold">Total</dt>
            <dd className="mt-1 border-t border-line-soft pt-1 text-right font-semibold tabular-nums">{size(usage.reduce((a, [, n]) => a + n, 0))}</dd>
          </dl>
        )}
      </Row>
      <Row title="Encryption" sub="Recordings, transcripts, search vectors and voiceprints are encrypted on disk; the key stays in your keychain">
        <div className="flex items-center gap-3">
          <Toggle on={st.encrypt} onChange={(on) => void encrypt(on)} label="Encrypt meetings on this Mac" />
          <span className="text-[14px] text-ink-3">{busy ? "Rewriting files…" : st.encrypt ? "On" : "Off"}</span>
        </div>
        <p className="mt-2 text-[12px] text-muted">meeting.md (title, your notes, the summary) stays readable so other apps can open it. While a call is recorded, its audio is encrypted when the recording stops.</p>
      </Row>
      <Row title="Delete old recordings" sub="Audio only; transcripts, notes and summaries stay">
        <div className="w-40">
          <Field label="After days (empty: keep)" value={st.retentionAudioDays?.toString() ?? ""} placeholder="Keep" onSave={(v) => save({ retentionAudioDays: days(v) })} />
        </div>
      </Row>
      <Row title="Per tag" sub="Delete a tag's meetings entirely after some days, or right now. With several tags, the shortest rule wins">
        <div className="flex flex-col gap-2">
          {tags.length === 0 && <p className="text-[13px] text-muted">No tags yet.</p>}
          {/* One line per tag: name, days to keep, and the rare "delete everything" as a quiet link. */}
          {tags.length > 0 && (
            <div className="grid grid-cols-[minmax(0,1fr)_6rem_auto] items-center gap-x-4 gap-y-1.5">
              <span className="text-[12px] text-muted">Tag</span>
              <span className="text-[12px] text-muted">Delete after (days)</span>
              <span />
              {tags.map((c) => (
                <div key={c} className="contents">
                  <span className="truncate text-[14px]" title={c}>
                    <span className={`mi-chip ${tagClass(c)}`}>{c}</span>
                  </span>
                  <Field hideLabel label={`Delete ${c} meetings after days`} value={st.retentionTags[c]?.toString() ?? ""} placeholder="Keep" onSave={(v) => void rule(c, v)} />
                  <button
                    type="button"
                    className="min-h-9 whitespace-nowrap rounded px-1 text-[13px] font-medium text-record-text hover:underline"
                    aria-label={sure === c ? `Click again to delete every ${c} meeting for good` : `Delete all ${c} meetings`}
                    onBlur={() => setSure(null)}
                    onClick={() => void forget(c)}
                  >
                    {sure === c ? "Click again to delete" : "Delete all…"}
                  </button>
                </div>
              ))}
            </div>
          )}
          <div className="mt-2 flex items-center gap-3">
            <button type="button" className="mi-ghost shrink-0 whitespace-nowrap text-[13px]" onClick={() => void run()}>
              Apply rules now
            </button>
            <span className="text-[12px] text-muted">Rules also run when Mirona starts and every hour. Everything removed is logged in .mirona/retention.log.</span>
          </div>
          {log && <p className="text-[13px] text-ink-3" role="status">{log.length ? log.join(" · ") : "Nothing to remove."}</p>}
        </div>
      </Row>
    </section>
  );
}

const SETTINGS_TABS: [SettingsTab, string, IconName][] = [
  ["general", "General", "sliders"],
  ["privacy", "Privacy and storage", "shield"],
  ["models", "AI models", "mic"],
  ["tags", "Tags", "tag"],
  ["agents", "Agents (MCP)", "plug"],
  ["advanced", "Advanced", "list"],
  ["about", "About", "info"],
];

function About() {
  const [version, setVersion] = useState("");
  useEffect(() => void getVersion().then(setVersion).catch(() => {}), []);
  const flash = useStore((s) => s.flash);
  return (
    <>
      <section className="mi-card p-6">
        <div className="flex items-center gap-4">
          <Logo size={56} />
          <div className="flex-1">
            <h2 className="text-[20px] font-bold">Mirona</h2>
            <p className="text-[14px] text-ink-3">Version <span className="selectable font-mono text-[13px]">{version || "…"}</span> · prototype</p>
          </div>
          <button type="button" className="mi-btn-primary h-11 px-5" onClick={() => api.showOnboarding(0).catch((e) => flash(String(e), true))}>
            Show the welcome tour
          </button>
        </div>
        <div className="mt-5 flex max-w-[640px] flex-col gap-3 text-[14px] leading-relaxed text-ink-3">
          <p>
            Mirona is meeting memory that stays on your Mac. It records your calls, transcribes them on this device, works out who said what, and
            tags the lines that matter: action items, decisions and questions nobody answered.
          </p>
          <p>
            Most meeting tools send your audio to someone else's servers and keep the record there. Mirona keeps every meeting as plain files you
            own (Markdown and a transcript per meeting), runs local models by default, and blocks every cloud service in one place when privacy
            mode is on.
          </p>
          <p>Your AI agents can search and ask about your meetings over MCP, only when you let them.</p>
        </div>
      </section>
      <Updates />
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

      <p className="mt-5 text-[13px] text-muted">Open source under the Apache License 2.0.</p>
    </>
  );
}

/** Opt-in update check (GitHub releases) and a manual check; installing restarts the app. */
function Updates() {
  const st = useStore((s) => s.settings);
  const save = useStore((s) => s.saveSettings);
  const update = useStore((s) => s.update);
  const installing = useStore((s) => s.installing);
  const install = useStore((s) => s.installUpdate);
  const [checking, setChecking] = useState(false);
  const [result, setResult] = useState("");
  const check = async () => {
    setChecking(true);
    try {
      setResult((await api.checkUpdate()) ? "" : "Mirona is up to date.");
    } catch (e) {
      setResult(String(e));
    }
    setChecking(false);
  };
  if (!st) return null;
  return (
    <section className="mi-card mt-5 p-6" aria-labelledby="updates">
      <h2 id="updates" className="text-[16px] font-semibold">
        Updates
      </h2>
      <p className="mt-1 text-[14px] text-ink-3">
        Checking asks GitHub for the latest release, even in privacy mode. It sends nothing about your meetings. Updates are signed and install only
        when you click.
      </p>
      <div className="mt-4 flex items-center gap-3 text-[14px]">
        <Toggle
          on={st.checkUpdates}
          onChange={async (checkUpdates) => {
            if ((await save({ checkUpdates })) && checkUpdates) await check();
          }}
          label="Check for updates automatically"
        />
        <span>Check for updates automatically</span>
      </div>
      <div className="mt-4 flex flex-wrap items-center gap-3 text-[14px]">
        {update ? (
          <>
            <span>
              <span className="font-semibold">Mirona {update.version}</span> is available.
            </span>
            <button type="button" className="mi-ghost min-h-11! px-4" onClick={() => void api.openReleaseNotes(update.version)}>
              What's new
            </button>
            <button type="button" className="mi-btn-primary h-11 px-5" onClick={() => void install()} disabled={installing}>
              {installing ? "Installing…" : "Install and restart"}
            </button>
          </>
        ) : (
          <>
            <button type="button" className="mi-ghost min-h-11! px-4" onClick={() => void check()} disabled={checking}>
              {checking ? "Checking…" : "Check now"}
            </button>
            {result && <span className="text-ink-3" role="status">{result}</span>}
          </>
        )}
      </div>
    </section>
  );
}

/** The settings window (its own Tauri window, label "settings"): sections on the left. */
export function SettingsWindow() {
  const init = useStore((s) => s.initSettings);
  const tab = useStore((s) => s.settingsTab);
  useEffect(() => void init(), [init]);
  useEffect(() => applyPlatform(detected), []);
  const mac = detected === "macos";
  const title = SETTINGS_TABS.find(([id]) => id === tab)?.[1] ?? "Settings";
  return (
    <div className="flex h-full">
      <aside className="mi-sidebar flex h-full w-[220px] shrink-0 flex-col border-r border-white/60 px-3 pb-3">
        {/* macOS: room for the traffic lights, and the strip drags the window */}
        <div data-tauri-drag-region className={mac ? "h-[52px] shrink-0" : "h-3 shrink-0"} />
        <nav aria-label="Settings sections" className="flex flex-col gap-0.5">
          {SETTINGS_TABS.map(([id, name, icon]) => (
            <button
              type="button"
              key={id}
              aria-current={tab === id ? "page" : undefined}
              onClick={() => useStore.setState({ settingsTab: id })}
              className="mi-nav flex h-10 items-center gap-3 px-3 text-left text-[14px] font-medium"
            >
              <Icon name={icon} />
              {name}
            </button>
          ))}
        </nav>
      </aside>
      {/* Only the content scrolls: the title bar stays put instead of covering rows. */}
      <main className="mi-main flex min-w-0 flex-1 flex-col">
        <div data-tauri-drag-region className="flex h-[52px] shrink-0 items-center border-b border-line-soft px-8">
          <h1 data-tauri-drag-region className="text-[17px] font-semibold">{title}</h1>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto pt-4">
          <SettingsView />
        </div>
      </main>
      <Toast />
    </div>
  );
}

function SettingsView() {
  const s = useStore();
  const st = s.settings;
  const status = s.status;
  const tab = s.settingsTab;
  if (!st || !status) return null;
  const save = s.saveSettings;
  const privacy = st.privacy;
  const d = llmDefaults[st.llm];
  const desktop = JSON.stringify({ mcpServers: { mirona: { command: status.cli, args: ["mcp"] } } }, null, 2);
  const tagNames = allTags(st, s.meetings);
  return (
    <div className="px-8 pb-10">

      {tab === "general" && (
      <section className="mi-card overflow-hidden">
        <Row title="Default summary template" sub="Used after each recording; any meeting can be rewritten with another">
          <div className="flex flex-wrap items-center gap-2">
            <select className="mi-field text-[14px]" value={st.template} onChange={(e) => save({ template: e.target.value })} aria-label="Default summary template">
              {s.templates.map((t) => (
                <option key={t.id} value={t.id}>{t.name}</option>
              ))}
            </select>
            <button type="button" className="mi-ghost text-[13px]" onClick={() => void api.openTemplates().catch((e) => s.flash(String(e), true))}>
              Edit templates…
            </button>
          </div>
          <p className="mt-2 text-[13px] text-muted">
            Templates are Markdown files. Add one and its first line tells Mirona when to pick it after a call.
          </p>
        </Row>
        <Row title="Record shortcut" sub="Starts and stops a recording from any app">
          <ShortcutField label="Record shortcut" value={st.shortcut} onSave={(shortcut) => save({ shortcut })} />
        </Row>
        <Row title="Mute shortcut" sub="Mutes and unmutes your mic during a recording, from any app">
          <ShortcutField label="Mute shortcut" value={st.muteShortcut} onSave={(muteShortcut) => save({ muteShortcut })} />
        </Row>
        <Row title="Call detection" sub="When Zoom, Teams, Meet or another call app opens the microphone">
          <CallSettings st={st} onSave={save} />
        </Row>
        <Row title="Consent reminder" sub="Recording rules differ by country. Shown before each recording; remembered per client">
          <ConsentSettings st={st} save={save} />
        </Row>
        <Row title="Meetings folder" sub="One Markdown folder per meeting">
          <Folder label="meetings folder" path={st.meetingsDir} onPick={(meetingsDir) => save({ meetingsDir })} />
        </Row>
      </section>
      )}

      {tab === "privacy" && (
        <>
      <p className="mb-4 text-[14px] text-ink-3">Everything runs on this Mac by default; cloud options unlock only when privacy mode is off.</p>
      <section className="mi-card overflow-hidden">
        <Row title="Privacy mode" sub="Blocks every cloud provider">
          <div className="flex items-center gap-3">
            <Toggle on={privacy} onChange={s.setPrivacy} label="Privacy mode" />
            <span className="text-[14px] text-ink-3">
              {privacy ? "On: cloud options are off. Turning it on moves the summary back to Ollama." : "Off: cloud providers can be picked in AI models."}
            </span>
          </div>
        </Row>
        {privacy && st.classification === "jev" && <QwenNote />}
      </section>
          <Storage st={st} save={save} tags={tagNames} />
        </>
      )}

      {tab === "models" && (
        <>
      <section className="mi-card overflow-hidden">
        <Row title="Speech to text" sub="Live transcription, on this device">
          <Options value={st.stt} options={[{ id: "parakeet", name: "Parakeet TDT 0.6B" }, ...(status.whisper || st.stt === "whisper" ? [{ id: "whisper" as const, name: "Whisper" }] : [])]} onPick={(stt) => save({ stt })} />
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
          {/* Stacked: side by side, the URL label wraps and the URL is cut off. */}
          <div className="mt-3 flex max-w-[420px] flex-col gap-3">
            <Field label="Model" value={st.llmModel} placeholder={d.model} onSave={(llmModel) => save({ llmModel })} />
            {st.llm !== "anthropic" && (
              <Field
                label={st.llm === "openai" ? "Base URL" : "Ollama chat URL"}
                hint={st.llm === "openai" ? "LM Studio, vLLM and other servers on localhost count as local." : undefined}
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
        <Row title="Models folder" sub="Speech and voice-activity models">
          <Folder label="models folder" path={st.modelsDir} onPick={(modelsDir) => save({ modelsDir })} />
          {status.missingModels.length ? (
            <div className="mt-2 text-[13px]">
              <p className="text-record-text">
                Missing: <span className="font-mono">{status.missingModels.join(", ")}</span>
              </p>
              <ModelDownload />
            </div>
          ) : (
            <p className="mt-2 text-[13px] text-accent-strong">All models for {st.stt === "whisper" ? "Whisper" : "Parakeet"} are installed.</p>
          )}
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
        </>
      )}

      {tab === "tags" && (
        <section className="mi-card overflow-hidden">
          <Tags st={st} save={save} />
        </section>
      )}

      {tab === "agents" && (
        <>
      <section className="mi-card p-6">
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

          <McpHttp st={st} save={save} />
        </>
      )}

      {tab === "about" && <About />}
      {tab === "advanced" && (
        <>
      <section className="mi-card overflow-hidden">
        <Row title="Thresholds" sub="At or above “Approve at”, an answer is filed as fact. Between the two it is marked “Needs review”. Below “Reject below” it is dropped without asking, and shown only as a count under To check">
          <Thresholds st={st} save={save} />
        </Row>
        <Row title="Your corrections" sub="Every answer you change is kept as a labelled example">
          <p className="text-[14px] text-ink-3">
            Saved to <span className="selectable font-mono text-[13px]">.mirona/labels.jsonl</span> in the meetings folder. Run <span className="font-mono text-[13px]">mirona eval</span> to see how the classifier does on them.
          </p>
        </Row>
      </section>
      <p className="mt-5 text-[13px] text-muted">
        Every setting lives in <span className="selectable font-mono">{status.configPath}</span> and can be edited there too; Mirona reads it on start. API keys stay in the system keychain, never in that file.
      </p>
        </>
      )}
    </div>
  );
}

import { useEffect, useState, type ReactNode } from "react";
import { useStore } from "../store";
import { api, fmt, LLM_LOCAL } from "../lib/api";
import { CopyLine } from "./Setup";
import { Avatar, Icon, PageHeader, Toggle } from "../components/ui";

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

function Options<T extends string>({ value, options, onPick }: { value: T; options: { id: T; name: string; cloud?: boolean; off?: boolean }[]; onPick: (id: T) => void }) {
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
          {o.name} <span className="text-[11px] opacity-80">{o.cloud ? "Cloud" : "Local"}</span>
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
        <Row title="Classification" sub="Tags segments: action, decision, open question…">
          <span className="mi-chip mi-chip--neutral">Coming in M2</span>
        </Row>
        <Row title="Speaker diarization" sub="Splits “them” into each person">
          <span className="mi-chip mi-chip--neutral">Coming in M2</span>
        </Row>
      </section>

      <section className="mi-card mt-5 overflow-hidden">
        <Row title="API keys" sub="Bring your own. Stored in the system keychain on this Mac, never in config.toml or sent anywhere else">
          <div className="flex flex-col gap-4">
            <ApiKey provider="openai" label="OpenAI (summaries)" saved={status.keys.includes("openai")} />
            <ApiKey provider="anthropic" label="Anthropic (summaries)" saved={status.keys.includes("anthropic")} />
            <ApiKey provider="jev" label="Jev by TypeSafe (classification, M2)" saved={status.keys.includes("jev")} />
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

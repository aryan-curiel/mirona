import { getCurrentWindow } from "@tauri-apps/api/window";
import { needsSetup, useStore, type View } from "../store";
import { fmt } from "../lib/api";
import { Icon, Logo, Toggle } from "./ui";

const nav: { id: View; label: string; icon: Parameters<typeof Icon>[0]["name"] }[] = [
  { id: "meetings", label: "Meetings", icon: "list" },
  { id: "actions", label: "Action items", icon: "check" },
  { id: "people", label: "People and voices", icon: "people" },
  { id: "models", label: "Models and MCP", icon: "sliders" },
];

/** Windows 11 caption bar. On macOS the native traffic lights float over the sidebar instead. */
export function WindowsTitleBar() {
  const w = getCurrentWindow();
  return (
    <div data-tauri-drag-region className="flex h-8 shrink-0 items-center pl-3 text-[12px]">
      <Logo size={16} />
      <span data-tauri-drag-region className="ml-2.5 flex-1 text-ink-3">Mirona</span>
      <button type="button" className="mi-caption" aria-label="Minimize" onClick={() => w.minimize()}>
        <svg width="10" height="10" aria-hidden="true"><path d="M0 5h10" stroke="currentColor" /></svg>
      </button>
      <button type="button" className="mi-caption" aria-label="Maximize" onClick={() => w.toggleMaximize()}>
        <svg width="10" height="10" aria-hidden="true"><rect x=".5" y=".5" width="9" height="9" rx="1.5" fill="none" stroke="currentColor" /></svg>
      </button>
      <button type="button" className="mi-caption mi-caption--close" aria-label="Close" onClick={() => w.close()}>
        <svg width="10" height="10" aria-hidden="true"><path d="M0 0l10 10M10 0L0 10" stroke="currentColor" /></svg>
      </button>
    </div>
  );
}

/** "Parakeet on this device · qwen3:4b (Ollama)": the models a recording runs through. */
export function useModelNames() {
  const st = useStore((s) => s.settings);
  if (!st) return { stt: "", llm: "" };
  const stt = st.stt === "whisper" ? "Whisper" : "Parakeet";
  const llm =
    st.llm === "ollama"
      ? `${st.llmModel || "qwen3:4b"} (Ollama)`
      : st.llm === "anthropic"
        ? `${st.llmModel || "claude-opus-5-5"} (Anthropic)`
        : `${st.llmModel || "gpt-4.1-mini"} (OpenAI-compatible)`;
  return { stt, llm };
}

const mac = navigator.userAgent.includes("Mac");
/** "CmdOrCtrl+Alt+Shift+R" → "⌘⌥⇧R" on macOS, "Ctrl+Alt+Shift+R" elsewhere. */
export const shortcutLabel = (s: string) =>
  mac ? s.replace("CmdOrCtrl", "⌘").replace("Alt", "⌥").replace("Shift", "⇧").split("+").join("") : s.replace("CmdOrCtrl", "Ctrl");

export function RecordButton() {
  const rec = useStore((s) => s.recording);
  const start = useStore((s) => s.startRecording);
  const stop = useStore((s) => s.stopRecording);
  const shortcut = useStore((s) => s.status?.shortcut ?? "");
  const hint = shortcutLabel(shortcut);
  return rec ? (
    <button type="button" onClick={stop} title={hint} className="flex h-11 items-center justify-center gap-2 rounded-md border border-record bg-white/70 font-semibold text-record-text">
      <Icon name="mic" /> Stop recording
    </button>
  ) : (
    <button type="button" onClick={start} title={hint} className="mi-btn-primary flex h-11 items-center justify-center gap-2">
      <Icon name="mic" /> Start recording <span className="text-[12px] font-medium opacity-80">{hint}</span>
    </button>
  );
}

export function Sidebar({ mac }: { mac: boolean }) {
  const s = useStore();
  const open = s.actionItems.filter((a) => !a.done).length;
  const rec = s.recording;
  return (
    <aside className="mi-sidebar flex h-full w-[var(--mi-sidebar-w)] shrink-0 flex-col border-r border-white/60 px-3.5 pb-3.5">
      {/* macOS: room for the traffic lights, and the strip drags the window */}
      <div data-tauri-drag-region className={mac ? "h-11 shrink-0" : "h-2 shrink-0"} />
      <div className="mb-4 flex items-center gap-2.5 px-1.5">
        <Logo size={28} />
        <span className="text-[18px] font-bold tracking-[-0.01em]">Mirona</span>
        {s.settings?.privacy && <span className="mi-chip mi-chip--action ml-auto">Local only</span>}
      </div>

      <RecordButton />

      <nav className="mt-4 flex flex-col gap-0.5">
        {nav.map((n) => (
          <button type="button"
            key={n.id}
            aria-current={s.view === n.id ? "page" : undefined}
            onClick={() => s.go(n.id)}
            className="mi-nav flex h-10 items-center gap-3 px-3 text-left text-[14px] font-medium"
          >
            <Icon name={n.icon} />
            <span className="flex-1">{n.label}</span>
            {n.id === "actions" && open > 0 && (
              <span className="grid h-5 min-w-5 place-items-center rounded-full bg-white/80 px-1.5 text-[11px] font-semibold">{open}</span>
            )}
          </button>
        ))}
        {needsSetup(s.status) && (
          <button type="button" aria-current={s.view === "setup" ? "page" : undefined} onClick={() => s.go("setup")} className="mi-nav flex h-10 items-center gap-3 px-3 text-left text-[14px] font-medium text-record-text">
            <Icon name="shield" />
            <span className="flex-1">Finish setup</span>
          </button>
        )}
      </nav>

      <div className="mt-6 mb-1.5 flex items-center px-2">
        <span className="mi-label flex-1">Recent</span>
        <button type="button" className="rounded px-1.5 text-[12px] font-medium text-accent-strong hover:underline" onClick={s.importAudio}>
          Import audio…
        </button>
      </div>
      <div className="-mx-1 flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-1">
        {s.meetings.map((m) => {
          const active = s.view === "meetings" && s.meeting?.id === m.id;
          const live = rec?.meetingId === m.id;
          const busy = s.processing.includes(m.id);
          return (
            <button type="button"
              key={m.id}
              onClick={() => s.openMeeting(m.id)}
              aria-current={active ? "page" : undefined}
              className="mi-nav px-2.5 py-1.5 text-left"
            >
              <div className="flex items-center gap-1.5 text-[13px] font-semibold text-ink">
                {live && <span className="mi-pulse size-1.5 rounded-full bg-record" />}
                <span className="truncate">{m.title}</span>
              </div>
              <div className="truncate text-[12px] text-muted">
                {busy ? "Writing the summary…" : `${m.client} · ${m.when}`}
              </div>
            </button>
          );
        })}
        {!s.meetings.length && <p className="px-2 py-2 text-[12px] leading-snug text-muted">No meetings yet. Start a recording or import an audio file.</p>}
      </div>

      <div className="mi-card mt-3 p-3.5">
        <div className="flex items-center gap-2">
          <Icon name="shield" className="text-accent" />
          <span className="flex-1 text-[14px] font-semibold">Privacy mode</span>
          <Toggle on={!!s.settings?.privacy} onChange={s.setPrivacy} label="Privacy mode" />
        </div>
        <p className="mt-2 text-[12px] leading-snug text-muted">
          {s.settings?.privacy
            ? "On. Cloud providers are blocked: audio, transcripts and summaries stay on this device."
            : "Off. Cloud providers can be picked in Models and MCP. Audio and voiceprints still stay on this device."}
        </p>
      </div>
    </aside>
  );
}

/** One level meter: the engine sends [mic, system] as 0..1 RMS. */
function Meter({ label, level }: { label: string; level: number }) {
  // RMS of speech is small; a square root makes normal talking fill most of the bar.
  const pct = Math.min(100, Math.sqrt(Math.max(0, level)) * 140);
  return (
    <span className="flex items-center gap-1.5">
      <span className="text-[12px] text-ink-3">{label}</span>
      {/* biome-ignore lint/a11y/useSemanticElements: a styled live level bar; <meter> can't take the record color reliably */}
      <span className="relative h-1.5 w-16 overflow-hidden rounded-full bg-[rgba(194,54,43,0.15)]" role="meter" aria-label={`${label} level`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(pct)}>
        <i className="absolute inset-y-0 left-0 rounded-full bg-record transition-[width] duration-100" style={{ width: `${pct}%` }} />
      </span>
    </span>
  );
}

export function RecordingBar() {
  const rec = useStore((s) => s.recording);
  const stop = useStore((s) => s.stopRecording);
  const { stt, llm } = useModelNames();
  if (!rec) return null;
  const [mic = 0, system = 0] = rec.levels;
  return (
    <div data-tauri-drag-region className="flex h-14 shrink-0 items-center gap-4 border-b border-[rgba(194,54,43,0.15)] bg-[var(--mi-record-tint)] px-6 text-[13px]">
      <span className="mi-pulse size-2.5 rounded-full bg-record" />
      <span className="font-semibold text-record-text">Recording</span>
      <span className="font-mono text-record-text" role="timer">{fmt(rec.elapsedMs)}</span>
      <Meter label="You" level={mic} />
      <Meter label="Call" level={system} />
      <span className="flex-1 truncate text-ink-3">
        {stt} transcribes on this device · {llm} writes the summary
      </span>
      <button type="button" className="mi-ghost min-h-11! px-4" onClick={stop}>
        Stop and summarize
      </button>
    </div>
  );
}

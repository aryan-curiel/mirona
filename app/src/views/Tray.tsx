import { listen } from "@tauri-apps/api/event";
import { type ReactNode, useEffect, useState } from "react";
import type { PipelineEvent } from "../bindings/PipelineEvent";
import { shortcutLabel } from "../components/Shell";
import { Icon, type IconName, Logo } from "../components/ui";
import { api, fmt, type RecState } from "../lib/api";
import { applyPlatform, detected } from "../lib/platform";

/** The menu bar popover (window "tray"): record, mute, open, settings, quit. */
export function TrayWindow() {
  const [rec, setRec] = useState<RecState | null>(null);
  const [elapsed, setElapsed] = useState(0);
  const [keys, setKeys] = useState({ record: "", mute: "" });

  useEffect(() => {
    applyPlatform(detected);
    const loadKeys = () => api.status().then((s) => setKeys({ record: shortcutLabel(s.shortcut), mute: shortcutLabel(s.muteShortcut) }));
    void api.recordingState().then(setRec);
    void loadKeys();
    const offs = [
      listen<RecState>("recording", (e) => {
        setRec(e.payload);
        if (!e.payload.recording) setElapsed(0);
      }),
      listen<PipelineEvent>("pipeline", (e) => {
        if (e.payload.stage === "captured") setElapsed(e.payload.elapsedMs);
      }),
      listen("settings", loadKeys),
    ];
    const esc = (e: KeyboardEvent) => e.key === "Escape" && void api.trayAction("hide");
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("keydown", esc);
      for (const off of offs) void off.then((f) => f());
    };
  }, []);

  const recording = !!rec?.recording;
  const muted = !!rec?.micMuted;
  const busy = rec?.processing.length ?? 0;
  const status = recording ? (
    <span className="flex items-center gap-1.5 font-medium text-record-text">
      <span className="mi-pulse size-2 rounded-full bg-record" />
      {muted ? "Recording, you're muted" : "Recording"} · <span className="font-mono">{fmt(elapsed)}</span>
    </span>
  ) : busy ? (
    `Summarizing ${busy === 1 ? "a meeting" : `${busy} meetings`}…`
  ) : (
    "Ready to record"
  );

  return (
    <div className="mi-tray flex h-full flex-col gap-3 p-3.5">
      <header className="flex items-center gap-2.5 px-0.5">
        <Logo size={34} />
        <div className="min-w-0 leading-tight">
          <div className="text-[15px] font-semibold">Mirona</div>
          <div className="truncate text-[12px] text-muted" role="status">{status}</div>
        </div>
      </header>

      {recording ? (
        <button type="button" onClick={() => void api.trayAction("toggle")} className="mi-ghost flex min-h-14! items-center gap-3 border-record! px-3! text-left text-record-text">
          <span className="grid size-9 shrink-0 place-items-center rounded-full bg-record text-white"><Icon name="stop" size={16} className="fill-current" /></span>
          <span className="flex-1 font-semibold">Stop and summarize</span>
          <Kbd>{keys.record}</Kbd>
        </button>
      ) : (
        <button type="button" onClick={() => void api.trayAction("toggle")} className="mi-btn-primary flex h-14 items-center gap-3 px-3 text-left">
          <span className="grid size-9 shrink-0 place-items-center rounded-full bg-white/20"><Icon name="mic" size={18} /></span>
          <span className="flex-1">Start recording</span>
          <Kbd>{keys.record}</Kbd>
        </button>
      )}

      <div className="grid grid-cols-3 gap-2">
        <Tile icon={muted ? "micOff" : "mic"} label={muted ? "Unmute me" : "Mute me"} hint={recording ? keys.mute : "Only while recording"} pressed={muted} disabled={!recording} onClick={() => void api.trayAction("mute")} />
        <Tile icon="window" label="Open Mirona" onClick={() => void api.trayAction("show")} />
        <Tile icon="gear" label="Settings" onClick={() => void api.trayAction("settings")} />
      </div>

      <hr className="mt-auto -mb-1.5 border-line-soft" />
      <button type="button" onClick={() => void api.trayAction("quit")} className="mi-nav flex h-9 items-center gap-2.5 px-2 text-[13px] text-muted">
        <Icon name="power" size={15} />
        <span className="flex-1 text-left">Quit Mirona</span>
        {detected === "macos" && <span className="text-[12px]">⌘Q</span>}
      </button>
    </div>
  );
}

function Kbd({ children }: { children: ReactNode }) {
  return children ? <span className="rounded-md bg-black/10 px-1.5 py-0.5 font-mono text-[11px] font-medium opacity-90">{children}</span> : null;
}

function Tile({ icon, label, hint, pressed, disabled, onClick }: { icon: IconName; label: string; hint?: string; pressed?: boolean; disabled?: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-pressed={pressed}
      title={hint}
      className="mi-ghost flex h-[76px] flex-col items-center justify-center gap-1.5 px-1! text-[12px] font-medium aria-pressed:border-record! aria-pressed:text-record-text disabled:opacity-45"
    >
      <Icon name={icon} size={22} className={pressed ? "" : "text-accent-strong"} />
      {label}
    </button>
  );
}

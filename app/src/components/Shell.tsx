import { getCurrentWindow } from "@tauri-apps/api/window";
import { useStore, type View } from "../store";
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
        {s.privacy && <span className="mi-chip mi-chip--action ml-auto">Local only</span>}
      </div>

      {rec ? (
        <button type="button"
          onClick={s.stopRecording}
          disabled={rec.processing}
          className="flex h-11 items-center justify-center gap-2 rounded-md border border-record bg-white/70 font-semibold text-record-text disabled:opacity-60"
        >
          <Icon name="mic" />
          {rec.processing ? "Processing…" : "Stop recording"}
        </button>
      ) : (
        <button type="button" onClick={s.startRecording} className="mi-btn-primary flex h-11 items-center justify-center gap-2">
          <Icon name="mic" /> Start recording
        </button>
      )}

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
      </nav>

      <div className="mi-label mt-6 mb-1.5 px-2">Recent</div>
      <div className="-mx-1 flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-1">
        {s.meetings.map((m) => {
          const active = s.view === "meetings" && s.meeting?.id === m.id;
          const live = rec?.meetingId === m.id;
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
                {m.client} · {m.when}
              </div>
            </button>
          );
        })}
      </div>

      <div className="mi-card mt-3 p-3.5">
        <div className="flex items-center gap-2">
          <Icon name="shield" className="text-accent" />
          <span className="flex-1 text-[14px] font-semibold">Privacy mode</span>
          <Toggle on={s.privacy} onChange={s.setPrivacy} label="Privacy mode" />
        </div>
        <p className="mt-2 text-[12px] leading-snug text-muted">
          {s.privacy
            ? "On. No network calls: audio, transcripts and voiceprints stay on this device. Cloud providers are blocked."
            : "Off. Cloud providers can be picked in Models and MCP. Audio and voiceprints still stay on this device."}
        </p>
      </div>
    </aside>
  );
}

export function RecordingBar() {
  const rec = useStore((s) => s.recording);
  const stop = useStore((s) => s.stopRecording);
  const stt = useStore((s) => s.stages.find((x) => x.id === "stt"));
  if (!rec) return null;
  const sttName = stt?.options.find((o) => o.id === stt.selected)?.name ?? "";
  const levels = rec.levels.length ? rec.levels : Array(9).fill(0.15);
  return (
    <div data-tauri-drag-region className="flex h-14 shrink-0 items-center gap-4 border-b border-[rgba(194,54,43,0.15)] bg-[var(--mi-record-tint)] px-6 text-[13px]">
      <span className="mi-pulse size-2.5 rounded-full bg-record" />
      <span className="font-semibold text-record-text">{rec.processing ? "Processing" : "Recording"}</span>
      <span className="font-mono text-record-text">{fmt(rec.elapsedMs)}</span>
      <span className="flex h-5 items-end gap-[3px]" aria-hidden>
        {levels.map((l, i) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: fixed bars, position is the identity
          <i key={i} className="w-[3px] rounded-sm bg-record transition-[height] duration-100" style={{ height: `${rec.processing ? 15 : l * 100}%` }} />
        ))}
      </span>
      <span className="flex-1 truncate text-ink-3">
        {rec.processing ? "Diarizing, classifying and writing the summary on this device…" : `Mic + system audio · ${sttName} on this device · segments are tagged as they arrive`}
      </span>
      <button type="button" className="mi-ghost" onClick={stop} disabled={rec.processing}>
        Stop and process
      </button>
    </div>
  );
}

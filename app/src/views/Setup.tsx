import { useState, type ReactNode } from "react";
import { needsSetup, useStore } from "../store";
import { api, type Permission } from "../lib/api";
import { Icon, PageHeader } from "../components/ui";
import { shortcutLabel } from "../components/Shell";

export function CopyLine({ text }: { text: string }) {
  const [done, setDone] = useState(false);
  return (
    <div className="mt-2 flex items-center gap-2">
      <code className="selectable min-w-0 flex-1 overflow-x-auto rounded-md bg-[var(--mi-neutral-tint)] px-3 py-2 font-mono text-[13px] whitespace-pre">{text}</code>
      <button
        type="button"
        className="mi-ghost shrink-0 text-[13px]"
        onClick={() =>
          navigator.clipboard.writeText(text).then(() => {
            setDone(true);
            setTimeout(() => setDone(false), 1500);
          })
        }
      >
        {done ? "Copied" : "Copy"}
      </button>
    </div>
  );
}

function Step({ ok, title, children }: { ok: boolean; title: string; children: ReactNode }) {
  return (
    <section className="mi-card flex gap-4 p-5">
      <span className={`grid size-7 shrink-0 place-items-center rounded-full ${ok ? "bg-accent text-white" : "border-2 border-line"}`} aria-hidden>
        {ok && <Icon name="tick" size={16} />}
      </span>
      <div className="min-w-0 flex-1">
        <h2 className="text-[16px] font-semibold">
          {title} <span className="sr-only">{ok ? "(done)" : "(to do)"}</span>
        </h2>
        <div className="mt-1 text-[14px] leading-relaxed text-ink-3">{children}</div>
      </div>
    </section>
  );
}

const pane = {
  mic: "Microphone",
  system: "Screen & System Audio Recording",
};

function PermissionStep({ kind, state, title, why }: { kind: "mic" | "system"; state: Permission; title: string; why: string }) {
  const refresh = useStore((s) => s.refreshStatus);
  const [asking, setAsking] = useState(false);
  const ask = async () => {
    setAsking(true);
    try {
      await api.requestPermission(kind);
    } finally {
      setAsking(false);
      await refresh();
    }
  };
  return (
    <Step ok={state === "granted"} title={title}>
      <p>{why}</p>
      {state === "notDetermined" && (
        <button type="button" className="mi-btn-primary mt-3 h-11 px-5" onClick={ask} disabled={asking}>
          {asking ? "Waiting for macOS…" : "Allow"}
        </button>
      )}
      {state === "denied" && (
        <>
          <p className="mt-2 text-record-text">
            Turned off. Open System Settings › Privacy & Security › {pane[kind]}, switch Mirona on, then come back.
          </p>
          <button type="button" className="mi-ghost mt-3 min-h-11!" onClick={() => refresh()}>
            Check again
          </button>
        </>
      )}
    </Step>
  );
}

/** First run (NGR-142): permissions with the reason for each, and the speech models. */
export function SetupView() {
  const s = useStore();
  const st = s.status;
  if (!st) return null;
  return (
    <div className="mx-auto max-w-[760px] px-10 pt-8 pb-16">
      <PageHeader title="Set up Mirona">
        Three things before your first meeting. Everything runs on this Mac: nothing here sends audio anywhere.
      </PageHeader>
      <div className="flex flex-col gap-3">
        <PermissionStep
          kind="mic"
          state={st.mic}
          title="Microphone"
          why="Mirona hears your side of the call through the microphone and transcribes it on this Mac."
        />
        <PermissionStep
          kind="system"
          state={st.systemAudio}
          title="System audio"
          why="The other people in the call come through your Mac's speakers. Mirona records that system audio (not your screen) so it can transcribe them too. Without it, only your voice is captured."
        />
        <Step ok={st.missingModels.length === 0} title="Speech models">
          {st.missingModels.length === 0 ? (
            <p>Installed in <span className="font-mono text-[13px]">{s.settings?.modelsDir}</span>.</p>
          ) : (
            <>
              <p>
                Transcription runs on local models (about 700 MB), not bundled with the app yet. Missing:{" "}
                <span className="font-mono text-[13px]">{st.missingModels.join(", ")}</span>. From the Mirona source folder, run:
              </p>
              <CopyLine text={st.fetchCommand} />
              <button type="button" className="mi-ghost mt-3 min-h-11!" onClick={() => s.refreshStatus()}>
                Check again
              </button>
            </>
          )}
        </Step>
      </div>
      <p className="mt-5 text-[14px] text-muted">
        Meetings are saved as Markdown in <span className="font-mono text-[13px]">{s.settings?.meetingsDir}</span> (change it in Models and MCP). Start
        and stop recording from the sidebar, the menu bar icon, or <span className="font-mono text-[13px]">{shortcutLabel(st.shortcut)}</span> from any app.
      </p>
      <div className="mt-6 flex gap-3">
        <button type="button" className="mi-btn-primary h-11 px-5" onClick={s.finishSetup}>
          {needsSetup(st) ? "Skip for now" : "Done"}
        </button>
      </div>
    </div>
  );
}

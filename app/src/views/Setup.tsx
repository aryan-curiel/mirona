import { type CSSProperties, useEffect, useRef, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { applyPlatform, detected } from "../lib/platform";
import { needsSetup, useStore } from "../store";
import { api, type ModelProgress, type Permission } from "../lib/api";
import { Icon, type IconName, PageHeader, Toggle } from "../components/ui";
import { shortcutLabel, Toast } from "../components/Shell";

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

const mb = (bytes: number) => `${Math.round(bytes / 1e6)} MB`;

/** Downloads the missing models (NGR-174). Any window shows the progress, whichever started it. */
export function ModelDownload() {
  const st = useStore((s) => s.status);
  const refresh = useStore((s) => s.refreshStatus);
  const [progress, setProgress] = useState<ModelProgress>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    const off = listen<ModelProgress>("models", (ev) => setProgress(ev.payload));
    return () => void off.then((f) => f());
  }, []);
  if (!st || st.missingModels.length === 0) return null;
  const start = async () => {
    setError(null);
    setProgress({ done: 0, total: st.downloadSize });
    try {
      await api.downloadModels();
    } catch (e) {
      setError(String(e));
    } finally {
      setProgress(null);
      await refresh();
    }
  };
  if (progress) {
    return (
      <div className="mt-2 flex items-center gap-3" role="status">
        <div className="min-w-0 flex-1">
          <progress
            className="h-1.5 w-full appearance-none overflow-hidden rounded-full [&::-webkit-progress-bar]:bg-[var(--mi-neutral-tint)] [&::-webkit-progress-value]:bg-accent [&::-webkit-progress-value]:transition-[width]"
            max={progress.total || 1}
            value={progress.done}
            aria-label="Downloading models"
          />
          <p className="mt-1 text-[12px] text-muted">
            {mb(progress.done)} of {mb(progress.total)}
          </p>
        </div>
        <button type="button" className="mi-ghost min-h-11! shrink-0" onClick={() => void api.cancelDownload()}>
          Cancel
        </button>
      </div>
    );
  }
  return (
    <div className="mt-2 flex items-center gap-3">
      <p className="min-w-0 flex-1 text-[13px] leading-snug">
        {error ? (
          <span className="text-record-text">{error}</span>
        ) : (
          <>
            {mb(st.downloadSize)} from Hugging Face, once. Checked, then everything runs on this Mac.
          </>
        )}
      </p>
      <button type="button" className="mi-btn-primary h-11 shrink-0 px-5" onClick={() => void start()}>
        {error ? "Try again" : "Download"}
      </button>
    </div>
  );
}

export function Folder({ label, path, onPick }: { label: string; path: string; onPick: (p: string) => void }) {
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

/** A setup item: a check that pops when it turns done, the reason, and its action on the right. */
function Step({ ok, title, children, action }: { ok: boolean; title: string; children: ReactNode; action?: ReactNode }) {
  // Animate only the change to done, not items that were done already.
  const was = useRef(ok);
  const [popped, setPopped] = useState(false);
  useEffect(() => {
    const now = ok && !was.current;
    was.current = ok;
    if (!now) return;
    setPopped(true);
    const t = setTimeout(() => setPopped(false), 1200);
    return () => clearTimeout(t);
  }, [ok]);
  return (
    <section className={`mi-card flex items-center gap-4 px-5 py-4 transition-colors duration-500 ${popped ? "mi-done" : ""}`}>
      <span className={`mi-check grid size-7 shrink-0 place-items-center rounded-full ${ok ? "bg-accent text-white" : "border-2 border-line"}`} data-pop={popped || undefined} aria-hidden>
        {ok && <Icon name="tick" size={16} />}
      </span>
      <div className="min-w-0 flex-1">
        <h2 className="text-[15px] font-semibold">
          {title} <span className="sr-only">{ok ? "(done)" : "(to do)"}</span>
        </h2>
        <div className="text-[13px] leading-snug text-ink-3">{children}</div>
      </div>
      {action && <div className="shrink-0">{action}</div>}
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
  const action =
    state === "notDetermined" ? (
      <button type="button" className="mi-btn-primary h-11 px-5" onClick={ask} disabled={asking}>
        {asking ? "Waiting…" : "Allow"}
      </button>
    ) : state === "denied" ? (
      <button type="button" className="mi-ghost min-h-11!" onClick={() => refresh()}>
        Check again
      </button>
    ) : undefined;
  return (
    <Step ok={state === "granted"} title={title} action={action}>
      {state === "denied" ? (
        <p className="text-record-text">Turned off. In System Settings › Privacy & Security › {pane[kind]}, switch Mirona on.</p>
      ) : (
        <p>{why}</p>
      )}
    </Step>
  );
}

function Permissions() {
  const s = useStore();
  const st = s.status;
  if (!st) return null;
  const models = st.missingModels.length === 0;
  return (
    <div className="mi-stagger flex flex-col gap-2.5">
      <PermissionStep kind="mic" state={st.mic} title="Microphone" why="Your side of the call, transcribed on this Mac." />
      <PermissionStep
        kind="system"
        state={st.systemAudio}
        title="System audio"
        why="The others come through your speakers. Mirona records that sound, never your screen."
      />
      <Step ok={models} title="Speech and speaker models">
        {models ? <p>Installed, all on this Mac.</p> : <ModelDownload />}
      </Step>
    </div>
  );
}

const IDEAS: [IconName, string, string][] = [
  ["shield", "Private by default", "Transcribed on this Mac. One switch blocks every cloud service."],
  ["list", "Files you own", "Each meeting is a Markdown folder. Back it up, sync it, read it."],
  ["check", "Useful afterwards", "Actions, decisions and open questions, linked to the moment."],
  ["plug", "Ready for your agents", "Claude and other agents can ask about meetings, if you let them."],
];

function Welcome() {
  return (
    <>
      <h1 className="text-[28px] font-bold leading-tight tracking-[-0.015em]">Welcome to Mirona</h1>
      <p className="mt-3 text-[15px] leading-relaxed text-ink-3">
        Calls are where decisions are made and work is promised, and the details fade by the next day. Mirona records and transcribes them on
        this Mac, works out who said what, and keeps every meeting as files you own.
      </p>
      <ul className="mi-stagger mt-6 grid grid-cols-2 gap-2.5">
        {IDEAS.map(([icon, title, text]) => (
          <li key={title} className="mi-card px-4 py-3.5">
            <h2 className="flex items-center gap-2 text-[14px] font-semibold">
              <Icon name={icon} size={16} className="text-accent" />
              {title}
            </h2>
            <p className="mt-1 text-[13px] leading-snug text-ink-3">{text}</p>
          </li>
        ))}
      </ul>
    </>
  );
}

function Pref({ title, sub, children }: { title: string; sub: string; children: ReactNode }) {
  return (
    <div className="flex items-center gap-4 border-t border-line-soft px-5 py-3 first:border-t-0">
      <div className="min-w-0 flex-1">
        <p className="text-[14px] font-semibold">{title}</p>
        <p className="text-[13px] leading-snug text-muted">{sub}</p>
      </div>
      {children}
    </div>
  );
}

function Preferences() {
  const s = useStore();
  const st = s.settings;
  if (!st) return null;
  const save = s.saveSettings;
  return (
    <section className="mi-card mi-stagger overflow-hidden">
      <Pref title="Privacy mode" sub="No meeting data leaves this Mac. Off, cloud models can be picked in Settings.">
        <Toggle on={st.privacy} onChange={s.setPrivacy} label="Privacy mode" />
      </Pref>
      <Pref title="Consent reminder" sub="Before each recording, a nudge to tell people, with a message to paste.">
        <Toggle on={st.consentReminder} onChange={(consentReminder) => save({ consentReminder })} label="Consent reminder" />
      </Pref>
      <Pref title="Suggest recording when a call starts" sub="When Zoom, Teams or Meet opens the mic. It asks, never starts alone.">
        <Toggle on={st.detectCalls} onChange={(detectCalls) => save({ detectCalls })} label="Suggest recording when a call starts" />
      </Pref>
      <Pref title="Let AI agents read meetings" sub="For Claude and other MCP clients on this Mac.">
        <Toggle on={st.mcpEnabled} onChange={(mcpEnabled) => save({ mcpEnabled })} label="Let AI agents read meetings" />
      </Pref>
      <div className="border-t border-line-soft px-5 py-3">
        <p className="mb-1.5 text-[14px] font-semibold">Meetings folder</p>
        <Folder label="meetings folder" path={st.meetingsDir} onPick={(meetingsDir) => save({ meetingsDir })} />
      </div>
    </section>
  );
}

function Ready() {
  const st = useStore((s) => s.status);
  if (!st) return null;
  const rows: [string, ReactNode][] = [
    [
      "Record a meeting",
      <>
        From the sidebar, the menu bar or <span className="font-mono text-[12px]">{shortcutLabel(st.shortcut)}</span> in any app. Stop it and the
        summary writes itself.
      </>,
    ],
    ["Check the answers", "Anything Mirona isn't sure of says “Needs review”. Your fixes teach it your meetings."],
    ["Change anything later", "It's all in Settings (⌘,). This tour lives in Settings › About."],
  ];
  return (
    <ul className="mi-stagger flex flex-col gap-2.5">
      {rows.map(([title, text], i) => (
        <li key={title} className="mi-card flex gap-4 px-5 py-4">
          <span className="grid size-7 shrink-0 place-items-center rounded-full bg-accent-tint text-[13px] font-semibold text-accent-strong">{i + 1}</span>
          <div>
            <h2 className="text-[15px] font-semibold">{title}</h2>
            <p className="text-[13px] leading-snug text-ink-3">{text}</p>
          </div>
        </li>
      ))}
    </ul>
  );
}

const STEPS: { name: string; sub: string; line: string }[] = [
  { name: "Welcome", sub: "", line: "A mirror for your meetings." },
  { name: "Permissions and models", sub: "Three things before your first meeting. Nothing here sends audio anywhere.", line: "Hear both sides of the call." },
  { name: "Make it yours", sub: "All optional, saved as you go, and in Settings later.", line: "Your rules, on your Mac." },
  { name: "You're ready", sub: "Three things worth knowing.", line: "Look back at any call." },
];

/** Welcome: the logo's five bars speaking over their reflection. */
function Wave() {
  return (
    <div className="mi-wave">
      <div className="flex items-end gap-[13px]">
        {[62, 96, 110, 86, 58].map((h, i) => (
          <i key={h} style={{ height: h, animationDelay: `${i * -0.37}s`, animationDuration: `${1.3 + (i % 3) * 0.35}s` }} />
        ))}
      </div>
      <b />
    </div>
  );
}

/** Permissions: your side and theirs, each rippling once it's allowed; the bars between speak when both are. */
function Sources() {
  const st = useStore((s) => s.status);
  const mic = st?.mic === "granted";
  const sys = st?.systemAudio === "granted";
  const source = (on: boolean, icon: IconName) => (
    <div className="mi-source" data-on={on || undefined}>
      <span />
      <span />
      <span />
      <em>
        <Icon name={icon} size={22} />
      </em>
    </div>
  );
  return (
    <div className="flex items-center gap-4">
      {source(mic, "mic")}
      <div className="mi-link flex items-center gap-1.5" data-on={(mic && sys) || undefined}>
        {[0, 1, 2].map((i) => (
          <i key={i} style={{ animationDelay: `${i * -0.3}s` }} />
        ))}
      </div>
      {source(sys, "speaker")}
    </div>
  );
}

/** Preferences: everything orbits inside this Mac; with privacy off, one dot drifts past the edge. */
function Orbit() {
  const privacy = useStore((s) => s.settings?.privacy ?? true);
  const dots: [number, number, number][] = [
    [46, 7, 0],
    [64, 11, -3],
    [82, 15, -8],
    [64, 13, -9],
    [46, 9, -5],
  ];
  return (
    <div className="mi-orbit" data-open={!privacy || undefined}>
      <div className="mi-orbit-edge" />
      <em />
      {dots.map(([r, s, d]) => (
        <i key={`${r}-${s}`} style={{ "--r": `${r}px`, animationDuration: `${s}s`, animationDelay: `${d}s` } as CSSProperties} />
      ))}
      <i className="mi-orbit-away" style={{ animationDuration: "18s" }} />
    </div>
  );
}

const LINES: [number, string?][] = [[78], [58, "a"], [88], [44], [70, "d"], [52], [84, "q"], [62]];

/** Ready: the transcript drifts past a reading line; tagged lines carry their colour. */
function Transcript() {
  const rows = [...LINES, ...LINES];
  return (
    <div className="mi-transcript">
      <div className="mi-transcript-roll">
        {rows.map(([w, tag], i) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: a fixed decorative list
          <p key={i} style={{ width: `${w}%` }}>
            {tag && <b data-tag={tag} />}
            <i />
          </p>
        ))}
      </div>
      <div className="mi-transcript-line" />
    </div>
  );
}

const SCENES = [Wave, Sources, Orbit, Transcript];

/** The left panel: one scene per step, crossfading; a slow glow that moves with the step, and film grain. */
function Stage({ step }: { step: number }) {
  return (
    <div data-tauri-drag-region className="mi-stage relative flex w-[360px] shrink-0 flex-col overflow-hidden px-9 pt-16 pb-9 text-white" data-step={step}>
      <div className="mi-stage-glow" aria-hidden />
      <div className="mi-stage-grain" aria-hidden />
      <p data-tauri-drag-region className="relative text-[13px] font-semibold tracking-[0.18em] uppercase opacity-85">Mirona</p>
      <div data-tauri-drag-region className="relative flex-1" aria-hidden>
        {SCENES.map((Scene, i) => (
          <div key={STEPS[i].name} className="mi-scene" data-on={i === step || undefined}>
            <Scene />
          </div>
        ))}
      </div>
      <p key={step} className="mi-enter relative text-[22px] leading-snug font-semibold tracking-[-0.01em]">
        {STEPS[step].line}
      </p>
    </div>
  );
}

/** The onboarding window (label "onboarding"): what Mirona is, permissions and models, a few
 * choices. First run shows it before the main window; Settings › About and "Finish setup" reopen it. */
export function OnboardingWindow() {
  const init = useStore((s) => s.initSettings);
  const st = useStore((s) => s.status);
  const [[step, dir], setNav] = useState<[number, number]>(() => [(window as { miOnboardingStep?: number }).miOnboardingStep ?? 0, 1]);
  const at = Math.min(step, STEPS.length - 1);
  const go = (i: number) => setNav([i, i >= at ? 1 : -1]);
  useEffect(() => void init(), [init]);
  useEffect(() => applyPlatform(detected), []);
  useEffect(() => {
    const off = listen<number>("onboarding-step", (e) => setNav(([cur]) => [e.payload, e.payload >= cur ? 1 : -1]));
    return () => void off.then((f) => f());
  }, []);
  // No wait for status: the welcome needs none, and the steps that do render once it arrives.
  const last = at === STEPS.length - 1;
  const close = () => void getCurrentWindow().close();
  return (
    <div className="flex h-full">
      <Stage step={at} />
      <main className="mi-main flex min-w-0 flex-1 flex-col">
        <div data-tauri-drag-region className="flex h-[52px] shrink-0 items-center gap-1.5 px-10">
          {STEPS.map(({ name }, i) => (
            <button
              type="button"
              key={name}
              onClick={() => go(i)}
              aria-label={`Step ${i + 1}: ${name}`}
              aria-current={i === at ? "step" : undefined}
              className={`h-1.5 rounded-full transition-all duration-500 ease-[var(--mi-ease-out)] ${i === at ? "w-8 bg-accent" : i < at ? "w-4 bg-accent/40" : "w-4 bg-[var(--mi-neutral-tint)] hover:bg-line"}`}
            />
          ))}
        </div>
        <div key={at} className="mi-step min-h-0 flex-1 overflow-y-auto px-10 pt-3 pb-4" style={{ "--dir": dir } as CSSProperties}>
          {at === 0 ? <Welcome /> : <PageHeader title={STEPS[at].name}>{STEPS[at].sub}</PageHeader>}
          {at === 1 && <Permissions />}
          {at === 2 && <Preferences />}
          {at === 3 && <Ready />}
          {last && st && needsSetup(st) && (
            <p className="mt-3 text-[13px] text-record-text">Recording still needs step 2. “Finish setup” in the sidebar brings you back.</p>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-3 border-t border-line-soft px-10 py-4">
          {at > 0 && (
            <button type="button" className="mi-ghost min-h-11!" onClick={() => go(at - 1)}>
              Back
            </button>
          )}
          <span className="flex-1" />
          {!last && (
            <button type="button" className="mi-ghost min-h-11!" onClick={close}>
              Skip
            </button>
          )}
          <button type="button" className="mi-btn-primary h-11 px-5" onClick={last ? close : () => go(at + 1)}>
            {last ? "Start using Mirona" : at === 0 ? "Get started" : "Continue"}
          </button>
        </div>
      </main>
      <Toast />
    </div>
  );
}

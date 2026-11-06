import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useRef, useState } from "react";
import type { SearchFilters } from "../bindings/SearchFilters";
import type { SearchResults } from "../bindings/SearchResults";
import type { TagKind } from "../bindings/TagKind";
import { needsSetup, useStore, type View } from "../store";
import { api, dayGroup, fmt } from "../lib/api";
import { Icon, Logo, Reveal, Toggle, useLast } from "./ui";

const nav: { id: View; label: string; icon: Parameters<typeof Icon>[0]["name"] }[] = [
  { id: "all", label: "All meetings", icon: "list" },
  { id: "actions", label: "Action items", icon: "check" },
  { id: "people", label: "People and voices", icon: "people" },
  { id: "activity", label: "Agent activity", icon: "plug" },
];

/** The last message, kept mounted after it clears so it can fade out. */
export function Toast() {
  const toast = useStore((s) => s.toast);
  const [lastToast, setLastToast] = useState(toast);
  if (toast && toast !== lastToast) setLastToast(toast);
  if (!lastToast) return null;
  return (
    <div
      role={lastToast.error ? "alert" : "status"}
      data-open={!!toast}
      aria-hidden={!toast}
      className={`mi-card mi-toast fixed bottom-5 left-1/2 max-w-[640px] -translate-x-1/2 rounded-lg! px-4 py-2.5 text-[14px] font-medium ${lastToast.error ? "text-record-text" : ""}`}
    >
      {lastToast.msg}
    </div>
  );
}

/** App settings open in their own window (also ⌘, and the menus). Lives in the window chrome:
 * right of the traffic lights on macOS, beside the caption buttons on Windows. */
function SettingsButton() {
  const open = useStore((s) => s.openSettings);
  return (
    <button
      type="button"
      aria-label="Settings"
      title={`Settings (${mac ? "⌘," : "Ctrl+,"})`}
      onClick={() => open("general")}
      className="grid size-8 place-items-center rounded-md text-ink-3 hover:bg-white/60"
    >
      <Icon name="gear" />
    </button>
  );
}

/** Windows 11 caption bar. On macOS the native traffic lights float over the sidebar instead. */
export function WindowsTitleBar() {
  const w = getCurrentWindow();
  return (
    <div data-tauri-drag-region className="flex h-8 shrink-0 items-center pl-3 text-[12px]">
      <Logo size={16} />
      <span data-tauri-drag-region className="ml-2.5 flex-1 text-ink-3">Mirona</span>
      <SettingsButton />
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
const MAC_KEYS: Record<string, string> = { cmdorctrl: "⌘", commandorcontrol: "⌘", commandorctrl: "⌘", cmdorcontrol: "⌘", cmd: "⌘", command: "⌘", super: "⌘", alt: "⌥", option: "⌥", shift: "⇧", ctrl: "⌃", control: "⌃" };
/** "CmdOrCtrl+Alt+Shift+R" → ["⌘","⌥","⇧","R"] on macOS, ["Ctrl","Alt","Shift","R"] elsewhere. Also takes key codes ("KeyR"). */
export const shortcutKeys = (s: string) =>
  s
    .split("+")
    .filter((t) => t.trim())
    .map((t) => {
      const k = t.trim();
      const low = k.toLowerCase();
      if (mac && MAC_KEYS[low]) return MAC_KEYS[low];
      if (!mac && /^(cmd|command)or(ctrl|control)$/.test(low)) return "Ctrl";
      return k.replace(/^(Key|Digit)(?=.)/, "");
    });
/** "CmdOrCtrl+Alt+Shift+R" → "⌘⌥⇧R" on macOS, "Ctrl+Alt+Shift+R" elsewhere. */
export const shortcutLabel = (s: string) => shortcutKeys(s).join(mac ? "" : "+");

/** The audio file being imported: name, then percent transcribed (indeterminate until known). */
export function ImportProgress() {
  const imp = useStore((s) => s.importing);
  if (!imp) return null;
  const pct = imp.fraction == null ? null : Math.round(imp.fraction * 100);
  const label = pct === 100 ? "Writing the summary…" : pct == null ? "Getting ready to transcribe…" : `Transcribing on this device · ${pct}%`;
  return (
    <div className="flex flex-col gap-1 px-2.5 py-1.5 text-left" role="status">
      <span className="truncate text-[13px] font-semibold text-ink" title={imp.file}>{imp.file}</span>
      <progress
        className="h-1.5 w-full appearance-none overflow-hidden rounded-full [&::-webkit-progress-bar]:bg-[var(--mi-neutral-tint)] [&::-webkit-progress-value]:bg-accent [&::-webkit-progress-value]:transition-[width]"
        max={100}
        value={pct ?? undefined}
        aria-label={`Importing ${imp.file}`}
      />
      <span className="text-[12px] text-muted">{label}</span>
    </div>
  );
}

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

const TAGS: [TagKind, string][] = [
  ["action", "Action items"],
  ["decision", "Decisions"],
  ["openQuestion", "Open questions"],
  ["sensitive", "Sensitive"],
];

/** "the [budget] is tight" with the keyword hits marked. */
function Snippet({ text }: { text: string }) {
  return (
    <>
      {text.split(/(\[[^\]]*\])/).map((p, i) =>
        p.startsWith("[") && p.endsWith("]") ? (
          // biome-ignore lint/suspicious/noArrayIndexKey: parts of one fixed string
          <mark key={i} className="rounded-sm bg-[var(--mi-accent-tint)] px-0.5 text-ink">
            {p.slice(1, -1)}
          </mark>
        ) : (
          p
        ),
      )}
    </>
  );
}

/**
 * Search across meetings by keywords and meaning (NGR-167), with filters. Results are the
 * moments that matched; a click opens the meeting at that line.
 */
function Search({ clients, onActive }: { clients: string[]; onActive: (on: boolean) => void }) {
  const openAt = useStore((s) => s.openAt);
  const showSensitive = useStore((s) => s.filters.showSensitive);
  const [q, setQ] = useState("");
  const [f, setF] = useState<SearchFilters>({ client: null, from: null, to: null, speaker: null, tag: null });
  const [more, setMore] = useState(false);
  const [r, setR] = useState<SearchResults | null>(null);
  const active = q.trim() !== "" || Object.values(f).some((v) => v);
  useEffect(() => {
    onActive(active);
    if (!active) {
      setR(null);
      return;
    }
    const t = setTimeout(() => api.search(q, f).then(setR, () => setR(null)), 250);
    return () => clearTimeout(t);
  }, [q, f, active, onActive]);
  const set = (k: keyof SearchFilters, v: string) => setF({ ...f, [k]: v || null });
  return (
    <div className={`mx-1 mb-1.5 flex flex-col gap-1.5 ${active ? "min-h-0 flex-1" : ""}`}>
      <div className="flex gap-1.5">
        <input
          type="search"
          className="mi-field h-9 min-w-0 flex-1 text-[13px]"
          placeholder="Search all meetings"
          aria-label="Search all meetings"
          value={q}
          onChange={(e) => setQ(e.target.value)}
        />
        <button type="button" className="mi-ghost h-9 px-2.5 text-[12px]" aria-expanded={more} onClick={() => setMore(!more)}>
          Filters
        </button>
      </div>
      {more && (
        <div className="grid grid-cols-2 gap-1.5 text-[12px]">
          <select className="mi-field h-8" aria-label="Client" value={f.client ?? ""} onChange={(e) => set("client", e.target.value)}>
            <option value="">Any client</option>
            {clients.map((c) => (
              <option key={c}>{c}</option>
            ))}
          </select>
          <select className="mi-field h-8" aria-label="Tag" value={f.tag ?? ""} onChange={(e) => set("tag", e.target.value)}>
            <option value="">Any line</option>
            {TAGS.map(([k, label]) => (
              <option key={k} value={k}>
                {label}
              </option>
            ))}
          </select>
          <input className="mi-field h-8" type="date" aria-label="From" value={f.from ?? ""} onChange={(e) => set("from", e.target.value)} />
          <input className="mi-field h-8" type="date" aria-label="To" value={f.to ?? ""} onChange={(e) => set("to", e.target.value)} />
          <input className="mi-field col-span-2 h-8" placeholder="Speaker" aria-label="Speaker" value={f.speaker ?? ""} onChange={(e) => set("speaker", e.target.value)} />
        </div>
      )}
      {r && (
        <section className="-mx-1 flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-1" aria-label="Search results">
          <p className="px-1.5 text-[12px] text-muted">
            {r.moments.length || r.meetings.length
              ? `${r.moments.length} moments · ${r.meetings.length} meetings${q.trim() && !r.semantic ? " · keywords only" : ""}`
              : "Nothing found"}
          </p>
          {r.moments.map((m) => (
            <button type="button" key={`${m.meetingId}/${m.segmentId}`} className="mi-nav px-2.5 py-1.5 text-left" onClick={() => openAt(m.meetingId, m.startMs)}>
              <div className="truncate text-[12px] text-muted">
                {m.title} · {fmt(m.startMs)} · {m.speaker}
              </div>
              <div className="line-clamp-2 text-[13px] text-ink">
                {m.sensitive && !showSensitive ? <i className="text-sensitive">Sensitive line hidden</i> : <Snippet text={m.snippet} />}
              </div>
            </button>
          ))}
          {r.moments.length === 0 &&
            r.meetings.map((m) => (
              <button type="button" key={m.id} className="mi-nav px-2.5 py-1.5 text-left" onClick={() => openAt(m.id, 0)}>
                <div className="truncate text-[13px] font-semibold text-ink">{m.title}</div>
                <div className="truncate text-[12px] text-muted">{[m.client, m.when].filter(Boolean).join(" · ")}</div>
              </button>
            ))}
        </section>
      )}
    </div>
  );
}

/** How many meetings the sidebar lists before "See all". */
const RECENT = 15;

export function Sidebar({ mac }: { mac: boolean }) {
  const s = useStore();
  const open = s.actionItems.filter((a) => !a.done).length;
  const rec = s.recording;
  const clients = [...new Set(s.meetings.map((m) => m.client).filter(Boolean))].sort();
  // The newest few, by day; everything else is one click away in All meetings.
  const shown = s.meetings.slice(0, RECENT);
  const groups: [string, typeof shown][] = [];
  for (const m of shown) {
    const g = dayGroup(m.created);
    const last = groups[groups.length - 1];
    if (last?.[0] === g) last[1].push(m);
    else groups.push([g, [m]]);
  }
  const [searching, setSearching] = useState(false);
  const [privacyHelp, setPrivacyHelp] = useState(false);
  return (
    <aside className="mi-sidebar flex h-full w-[var(--mi-sidebar-w)] shrink-0 flex-col border-r border-white/60 px-3.5 pb-3.5">
      {/* macOS: room for the traffic lights, and the strip drags the window */}
      <div data-tauri-drag-region className={mac ? "flex h-11 shrink-0 items-center justify-end" : "h-2 shrink-0"}>
        {mac && <SettingsButton />}
      </div>
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
          <button type="button" onClick={() => s.openOnboarding(1)} className="mi-nav flex h-10 items-center gap-3 px-3 text-left text-[14px] font-medium text-record-text">
            <Icon name="shield" />
            <span className="flex-1">Finish setup</span>
          </button>
        )}
      </nav>

      <div className="mt-6 mb-1.5 flex items-center px-2">
        <span className="mi-label flex-1">Recent</span>
        <button type="button" className="rounded px-1.5 text-[12px] font-medium text-accent-strong hover:underline disabled:text-muted disabled:no-underline" onClick={s.importAudio} disabled={!!s.importing}>
          {s.importing ? "Importing…" : "Import audio…"}
        </button>
      </div>
      <Search clients={clients} onActive={setSearching} />
      <div className={`-mx-1 flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-1 ${searching ? "hidden" : ""}`}>
        <ImportProgress />
        {groups.map(([g, ms]) => (
          <div key={g} className="flex flex-col gap-0.5">
            <p className="px-2.5 pt-2 pb-0.5 text-[11px] font-semibold text-muted">{g}</p>
        {ms.map((m) => {
          const active = s.view === "meetings" && s.meeting?.id === m.id;
          const live = rec?.meetingId === m.id;
          const busy = s.processing.includes(m.id) || m.id in s.summarizing;
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
                {busy ? "Writing the summary…" : [m.client, m.when].filter(Boolean).join(" · ")}
              </div>
            </button>
          );
        })}
          </div>
        ))}
        {s.meetings.length > RECENT && (
          <button type="button" className="mt-1 rounded px-2.5 py-1.5 text-left text-[13px] font-medium text-accent-strong hover:underline" onClick={() => s.go("all")}>
            See all {s.meetings.length} meetings
          </button>
        )}
        {!s.meetings.length && !s.importing && <p className="px-2 py-2 text-[12px] leading-snug text-muted">No meetings yet. Start a recording or import an audio file.</p>}
      </div>

      {/* Privacy mode, with its explanation behind "?". */}
      <div className="mi-card relative mt-3 px-3 py-2.5">
        <div className="flex items-center gap-1">
          <span className="whitespace-nowrap text-[14px] font-semibold">Privacy mode</span>
          <button
            type="button"
            aria-label="What privacy mode does"
            aria-expanded={privacyHelp}
            aria-controls="privacy-help"
            onClick={() => setPrivacyHelp((v) => !v)}
            onBlur={() => setPrivacyHelp(false)}
            onKeyDown={(e) => e.key === "Escape" && setPrivacyHelp(false)}
            className="grid size-6 shrink-0 place-items-center rounded-full text-[12px] font-semibold text-ink-3 hover:bg-white/60"
          >
            ?
          </button>
          <span className="flex-1" />
          <Toggle on={!!s.settings?.privacy} onChange={s.setPrivacy} label="Privacy mode" />
        </div>
        {/* A bubble above the card: growing the card would push it out of the window. */}
        {privacyHelp && (
          <p id="privacy-help" role="status" className="mi-card absolute inset-x-0 bottom-full mb-2 px-3 py-2.5 text-[12px] leading-snug text-ink-3">
            {s.settings?.privacy
              ? "On. Cloud providers are blocked: audio, transcripts and summaries stay on this device."
              : "Off. Cloud providers can be picked in Settings. Audio and voiceprints still stay on this device."}
          </p>
        )}
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

/** "Zoom is using the microphone. Record this call?" (NGR-170). Never records on its own. */
export function CallBanner() {
  const current = useStore((s) => s.call);
  const rec = useStore((s) => s.recording);
  const record = useStore((s) => s.recordCall);
  const dismiss = useStore((s) => s.dismissCall);
  const open = !!current && !rec;
  const call = useLast(open ? current : null);
  if (!call) return null;
  return (
    <Reveal open={open}>
    <div role="alert" className="flex min-h-14 shrink-0 items-center gap-4 border-b border-line-soft bg-[var(--mi-neutral-tint)] px-6 py-2 text-[13px]">
      <Icon name="mic" className="text-accent" />
      <span className="flex-1 text-ink">
        <span className="font-semibold">{call.app}</span> is using the microphone. Record this call?
        {call.title && <span className="text-ink-3"> It will be named “{call.title}”{call.client ? ` for ${call.client}` : ""}.</span>}
      </span>
      <button type="button" className="mi-ghost min-h-11! px-4" onClick={dismiss}>
        Dismiss
      </button>
      <button type="button" className="mi-btn-primary flex h-11 items-center gap-2 px-4" onClick={record}>
        <Icon name="mic" /> Record
      </button>
    </div>
    </Reveal>
  );
}

/** A newer Mirona is out (only when the user checks for updates): install and restart. */
export function UpdateBanner() {
  const current = useStore((s) => s.update);
  const dismissed = useStore((s) => s.dismissedUpdate);
  const rec = useStore((s) => s.recording);
  const installing = useStore((s) => s.installing);
  const install = useStore((s) => s.installUpdate);
  const dismiss = useStore((s) => s.dismissUpdate);
  const open = !!current && current.version !== dismissed && !rec;
  const update = useLast(open ? current : null);
  if (!update) return null;
  return (
    <Reveal open={open}>
    <div role="status" className="flex min-h-14 shrink-0 items-center gap-4 border-b border-line-soft bg-[var(--mi-neutral-tint)] px-6 py-2 text-[13px]">
      <Icon name="info" className="text-accent" />
      <span className="flex-1 text-ink">
        <span className="font-semibold">Mirona {update.version}</span> is available.
        <span className="text-ink-3"> Installing restarts the app.</span>
      </span>
      <button type="button" className="mi-ghost min-h-11! px-4" onClick={dismiss} disabled={installing}>
        Later
      </button>
      <button type="button" className="mi-btn-primary h-11 px-4" onClick={() => void install()} disabled={installing}>
        {installing ? "Installing…" : "Install and restart"}
      </button>
    </div>
    </Reveal>
  );
}

export function RecordingBar() {
  const live = useStore((s) => s.recording);
  const rec = useLast(live);
  const stop = useStore((s) => s.stopRecording);
  const hint = shortcutLabel(useStore((s) => s.status?.shortcut ?? ""));
  const { stt, llm } = useModelNames();
  if (!rec) return null;
  const [mic = 0, system = 0] = rec.levels;
  return (
    <Reveal open={!!live}>
    <div data-tauri-drag-region className="flex h-14 shrink-0 items-center gap-4 border-b border-[rgba(194,54,43,0.15)] bg-[var(--mi-record-tint)] px-6 text-[13px]">
      <span className="mi-pulse size-2.5 rounded-full bg-record" />
      <span className="font-semibold text-record-text">Recording</span>
      <span className="font-mono text-record-text" role="timer">{fmt(rec.elapsedMs)}</span>
      <Meter label="You" level={mic} />
      <Meter label="Call" level={system} />
      <span className="flex-1 truncate text-ink-3">
        {stt} transcribes on this device · {llm} writes the summary
      </span>
      <button type="button" className="mi-ghost min-h-11! px-4" onClick={stop} title={hint}>
        Stop and summarize {hint && <span className="ml-1 text-[12px] font-medium opacity-80">{hint}</span>}
      </button>
    </div>
    </Reveal>
  );
}

/**
 * Before recording (NGR-169): who the call is with, a message to paste in the meeting chat,
 * and "they already agreed" so the reminder stays quiet for that client from then on.
 */
export function ConsentSheet() {
  const consent = useStore((s) => s.consent);
  const st = useStore((s) => s.settings);
  const meetings = useStore((s) => s.meetings);
  const call = useStore((s) => s.call);
  const confirm = useStore((s) => s.confirmConsent);
  const cancel = useStore((s) => s.cancelConsent);
  const [client, setClient] = useState("");
  const [agreed, setAgreed] = useState(false);
  const [never, setNever] = useState(false);
  const [copied, setCopied] = useState(false);
  // Stays mounted while it fades out; Escape closes at once (keyboard actions don't animate).
  const [shown, setShown] = useState(consent);
  const [closing, setClosing] = useState(false);
  const instant = useRef(false);
  if (consent && consent !== shown) {
    setShown(consent);
    setClosing(false);
  } else if (!consent && shown && !closing) {
    if (instant.current) setShown(null);
    else setClosing(true);
    instant.current = false;
  }
  useEffect(() => {
    if (!consent) return;
    setClient(call?.client ?? "");
    setAgreed(false);
    setNever(false);
    setCopied(false);
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      instant.current = true;
      cancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [consent, call, cancel]);
  if (!shown || !st) return null;
  const clients = [...new Set([...Object.keys(st.clients), ...meetings.map((m) => m.client)])].filter((c) => c && c !== "Unsorted").sort();
  const given = client !== "" && st.consentGiven.includes(client);
  const copy = async () => {
    await navigator.clipboard.writeText(st.consentMessage).catch(() => {});
    setCopied(true);
  };
  return (
    <div
      data-closing={closing}
      onAnimationEnd={(e) => closing && e.target === e.currentTarget && setShown(null)}
      className="mi-scrim fixed inset-0 z-50 grid place-items-center bg-black/20 p-6"
    >
      <div role="dialog" aria-modal="true" aria-labelledby="consent-title" className="mi-card mi-pop w-full max-w-[520px] p-6">
        <h2 id="consent-title" className="text-[18px] font-semibold">
          Before you record
        </h2>
        <p className="mt-1 text-[14px] text-ink-3">Recording rules differ by country: in many places everyone on the call has to agree.</p>
        <label className="mt-4 flex flex-col gap-1 text-[13px] text-muted">
          Who is the call with?
          <select className="mi-field h-10 text-[14px] text-ink" value={client} onChange={(e) => setClient(e.target.value)}>
            <option value="">Not sure yet</option>
            {clients.map((c) => (
              <option key={c}>{c}</option>
            ))}
          </select>
        </label>
        {given ? (
          <p className="mt-4 text-[14px] text-accent-strong">{client} already agreed to be recorded.</p>
        ) : (
          <>
            <p className="mt-4 text-[13px] text-muted">Paste this in the meeting chat:</p>
            <div className="mt-1 flex items-start gap-2">
              <p className="selectable flex-1 rounded-md bg-white/60 p-3 text-[14px]">{st.consentMessage}</p>
              <button type="button" className="mi-ghost text-[13px]" onClick={() => void copy()}>
                {copied ? "Copied" : "Copy"}
              </button>
            </div>
            {client && (
              <label className="mt-3 flex items-center gap-2 text-[14px]">
                <input type="checkbox" checked={agreed} onChange={(e) => setAgreed(e.target.checked)} />
                {client} agreed: don't remind me for them again
              </label>
            )}
          </>
        )}
        <label className="mt-2 flex items-center gap-2 text-[13px] text-muted">
          <input type="checkbox" checked={never} onChange={(e) => setNever(e.target.checked)} />
          Don't show this reminder again (turn it back on in Settings → General)
        </label>
        <div className="mt-5 flex justify-end gap-2">
          <button type="button" className="mi-ghost min-h-11! px-4" onClick={cancel}>
            Cancel
          </button>
          {/* biome-ignore lint/a11y/noAutofocus: the dialog's main action takes focus when it opens */}
          <button type="button" autoFocus className="mi-btn-primary flex h-11 items-center gap-2 px-5" onClick={() => void confirm(client || null, agreed, never)}>
            <Icon name="mic" /> Start recording
          </button>
        </div>
      </div>
    </div>
  );
}

// The real UI over a fake backend: every Tauri command answers from demo.json (the demo
// meetings, `cargo run -p mirona-store --example demo_json`). For screenshots only; nothing
// here ships. ?window= picks the window, ?tab= the settings tab, ?step= the onboarding step.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import demo from "./demo.json";

const q = new URLSearchParams(location.search);
const label = q.get("window") ?? "main";
const w = window as { miSettingsTab?: string; miOnboardingStep?: number };
if (q.get("tab")) w.miSettingsTab = q.get("tab") ?? undefined;
if (q.get("step")) w.miOnboardingStep = Number(q.get("step"));

const settings = {
  privacy: true,
  stt: "parakeet",
  llm: "ollama",
  llmModel: "qwen3:4b",
  llmBaseUrl: "http://localhost:11434",
  template: "client-call",
  mcpEnabled: true,
  mcpHttp: false,
  mcpPort: 7337,
  mcpWrite: false,
  mcpSensitive: false,
  mcpClients: ["claude-desktop"],
  shortcut: "CmdOrCtrl+Alt+Shift+R",
  muteShortcut: "CmdOrCtrl+Alt+Shift+M",
  meetingsDir: "~/Documents/Mirona",
  modelsDir: "~/Library/Application Support/com.nagare.mirona/models",
  classification: "local-llm",
  diarization: "speakrs",
  thresholds: {},
  thresholdDefaults: { action: 0.99, decision: 0.95, unanswered: 0.85, smalltalk: 0.85, sensitive: 0.6, meeting_type: 0.85, tag: 0.85, follow_up: 0.85 },
  floors: {},
  floorDefaults: { action: 0.6, decision: 0.6, unanswered: 0.6, smalltalk: 0.6, sensitive: 0, meeting_type: 0.6, tag: 0.6, follow_up: 0.6 },
  tags: Object.fromEntries(["project:Alder Restoration", "project:Juniper Apparel", "project:Corvo", "project:Lumenix", "topic:billing"].map((t) => [t, ""])),
  detectCalls: true,
  calendarTitles: true,
  autoRecord: [],
  encrypt: false,
  retentionAudioDays: 90,
  retentionTags: {},
  consentReminder: true,
  consentMessage:
    "Heads-up: I'm recording this call to take notes. The recording and transcript stay on my computer. Tell me if you'd rather I didn't.",
  consentGiven: ["project:Lumenix"],
  checkUpdates: true,
};

const status = {
  mic: "granted",
  systemAudio: "granted",
  missingModels: q.get("models") === "missing" ? ["parakeet", "speakrs"] : [],
  downloadSize: q.get("models") === "missing" ? 1_020_000_000 : 0,
  keys: [],
  configPath: "~/Library/Application Support/com.nagare.mirona/config.toml",
  cli: "/Applications/Mirona.app/Contents/MacOS/mirona-app",
  shortcut: settings.shortcut,
  muteShortcut: settings.muteShortcut,
  whisper: false,
  errors: [],
};

const GB = 1_000_000_000;
const handlers: Record<string, (a: Record<string, unknown>) => unknown> = {
  list_meetings: () => demo.list,
  get_meeting: (a) => (demo.meetings as Record<string, unknown>)[a.id as string],
  review: (a) => (demo.reviews as Record<string, unknown>)[a.meetingId as string] ?? [],
  list_action_items: () => demo.actions,
  list_people: () => demo.people,
  known_names: () => demo.knownNames,
  unnamed_speakers: () => (q.get("unnamed") ? ["s3"] : []),
  voiceprint_shape: (a) => Array.from({ length: 48 }, (_, i) => 0.5 + 0.45 * Math.sin(i / 3 + (a.personId as string).length)),
  get_settings: () => settings,
  save_settings: (a) => Object.assign(settings, a.settings),
  status: () => status,
  list_templates: () => [
    { id: "client-call", name: "Client call" },
    { id: "standup", name: "Standup" },
    { id: "one-on-one", name: "1:1" },
    { id: "planning", name: "Planning" },
    { id: "sprint-retro", name: "Sprint retro" },
  ],
  recording_state: () => ({ recording: null, micMuted: false, processing: [] }),
  pending_call: () => null,
  pending_update: () => null,
  refresh_store: () => false,
  audio_path: () => null,
  storage_usage: () => [
    ["Meeting audio", 0.41 * GB],
    ["Transcripts, notes and summaries", 0.004 * GB],
    ["Search index and logs", 0.012 * GB],
    ["Speech and speaker models", 1.02 * GB],
    ["Ollama models (shared with other apps)", 2.9 * GB],
  ],
  mcp_activity: () => [
    { time: "2026-11-10T09:58:12Z", client: "claude-code", tool: "search_meetings", ok: true },
    { time: "2026-11-10T09:58:14Z", client: "claude-code", tool: "get_meeting", ok: true },
    { time: "2026-11-10T09:59:02Z", client: "claude-desktop", tool: "list_action_items", ok: true },
    { time: "2026-11-10T10:01:40Z", client: "claude-desktop", tool: "ask_meeting", ok: true },
    { time: "2026-11-10T10:03:09Z", client: "cursor", tool: "get_transcript", ok: false },
  ],
  search: (a) => search(String(a.query)),
  "plugin:app|version": () => "0.2.0",
};

type Full = { segments: { id: string; text: string; startMs: number; speakerId: string }[]; speakers: { id: string; name: string }[] };
/** Keyword search over the demo lines, in the app's `SearchResults` shape. */
function search(query: string) {
  const words = query.toLowerCase().split(/\s+/).filter((w) => w.length > 2);
  const moments = [];
  for (const m of demo.list) {
    const full = (demo.meetings as unknown as Record<string, Full>)[m.id];
    for (const s of full.segments) {
      if (!words.some((w) => s.text.toLowerCase().includes(w))) continue;
      const speaker = full.speakers.find((p) => p.id === s.speakerId)?.name ?? "";
      moments.push({ meetingId: m.id, title: m.title, tags: m.tags, date: m.created, segmentId: s.id, startMs: s.startMs, speaker, snippet: s.text, sensitive: false, byKeyword: true, byMeaning: true });
    }
  }
  const ids = new Set(moments.map((x) => x.meetingId));
  return { moments, meetings: demo.list.filter((m) => ids.has(m.id)), semantic: true };
}

mockWindows(label);
mockIPC(
  (cmd, args) => {
    const h = handlers[cmd];
    return h ? h((args ?? {}) as Record<string, unknown>) : null;
  },
  { shouldMockEvents: true },
);

// Screenshots drive the store directly (a recording, a tab).
const { useStore } = await import("../../src/store");
(window as unknown as { miStore: typeof useStore }).miStore = useStore;
await import("../../src/main");

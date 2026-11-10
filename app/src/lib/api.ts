import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type { ActionItem } from "../bindings/ActionItem";
import type { Correction } from "../bindings/Correction";
import type { Meeting } from "../bindings/Meeting";
import type { MeetingPart } from "../bindings/MeetingPart";
import type { MeetingSummary } from "../bindings/MeetingSummary";
import type { Review } from "../bindings/Review";
import type { SearchFilters } from "../bindings/SearchFilters";
import type { SearchResults } from "../bindings/SearchResults";
import type { Segment } from "../bindings/Segment";
import type { Speaker } from "../bindings/Speaker";

/** Mirrors `Settings` in src-tauri/src/lib.rs (= config.toml). */
export type Settings = {
  privacy: boolean;
  stt: "parakeet" | "whisper";
  llm: "ollama" | "openai" | "anthropic";
  llmModel: string;
  llmBaseUrl: string;
  template: string;
  mcpEnabled: boolean;
  /** MCP over HTTP on 127.0.0.1:mcpPort; clients need a token (NGR-168). */
  mcpHttp: boolean;
  mcpPort: number;
  /** Let agents mark action items done and rename speakers. */
  mcpWrite: boolean;
  /** Let agents read sensitive lines when they ask (`include_sensitive`). */
  mcpSensitive: boolean;
  /** HTTP clients holding a token (read-only; tokens are shown once, on creation). */
  mcpClients: string[];
  /** Global record shortcut, a Tauri accelerator like "CmdOrCtrl+Alt+Shift+R". */
  shortcut: string;
  /** Global mute/unmute-my-mic shortcut while recording. */
  muteShortcut: string;
  meetingsDir: string;
  modelsDir: string;
  classification: "jev" | "local-llm" | "off";
  diarization: "speakrs" | "speakrs-cpu" | "off";
  /** Question id → the user's threshold; the rest use `thresholdDefaults`. */
  thresholds: Record<string, number>;
  /** Defaults for the classifier in use (read-only). */
  thresholdDefaults: Record<string, number>;
  /** Question id → the user's floor: a model "yes" below it is rejected without asking. */
  floors: Record<string, number>;
  /** Floor defaults (read-only); 0 means nothing is auto-rejected. */
  floorDefaults: Record<string, number>;
  /** Tag (`group:name` or `name`) → "" (no descriptions, NGR-202). The classifier asks one yes/no per tag. */
  tags: Record<string, string>;
  /** Suggest recording when a call app opens the mic (NGR-170). */
  detectCalls: boolean;
  /** Name meetings after the calendar event happening now. */
  calendarTitles: boolean;
  /** Tags whose calls start recording without a click (the tag's name in the event title). */
  autoRecord: string[];
  /** Encryption at rest; change it with `setEncryption` (NGR-165). */
  encrypt: boolean;
  /** Delete recordings this many days old (text stays); null keeps them. */
  retentionAudioDays: number | null;
  /** Tag → delete its meetings this many days old. */
  retentionTags: Record<string, number>;
  /** Remind to get consent before each recording (NGR-169). */
  consentReminder: boolean;
  consentMessage: string;
  /** Tags whose people already agreed: no reminder for them. */
  consentGiven: string[];
  /** Check GitHub for a new version on start and daily. */
  checkUpdates: boolean;
};

/** A newer Mirona on GitHub: the "update" event, null when there is none. */
export type UpdateInfo = { version: string; notes: string };

/** A call app started using the mic: the "call" event (NGR-170). */
export type CallSuggestion = { app: string; title: string | null; tag: string | null };

/** A person as `list_people` returns them: the speaker plus where they were last heard. */
export type Person = Speaker & { meetings: number; lastSeen: MeetingSummary | null };

export type Permission = "granted" | "denied" | "notDetermined";

/** A "models" event: bytes downloaded so far, or null when a run ends. */
export type ModelProgress = { done: number; total: number } | null;

/** Mirrors `Status` in src-tauri/src/lib.rs. */
export type Status = {
  mic: Permission;
  systemAudio: Permission;
  missingModels: string[];
  /** Bytes the model downloader would fetch. */
  downloadSize: number;
  keys: string[];
  configPath: string;
  cli: string;
  shortcut: string;
  muteShortcut: string;
  /** This build includes Whisper; otherwise Settings doesn't offer it. */
  whisper: boolean;
  errors: string[];
};

/** One MCP tool call, as `mcp_activity` returns them (newest first). */
export type McpActivity = { time: string; client: string; tool: string; ok: boolean };

/** Sent on the "recording" channel whenever the recorder changes. */
export type RecState = { recording: string | null; micMuted: boolean; processing: string[] };
export type Template = { id: string; name: string };

/** Typed wrappers over the Tauri commands in src-tauri/src/lib.rs. */
export const api = {
  listMeetings: () => invoke<MeetingSummary[]>("list_meetings"),
  refreshStore: () => invoke<boolean>("refresh_store"),
  search: (query: string, filters: SearchFilters) => invoke<SearchResults>("search", { query, filters }),
  getMeeting: (id: string) => invoke<Meeting>("get_meeting", { id }),
  listActionItems: () => invoke<ActionItem[]>("list_action_items"),
  toggleActionItem: (id: string) => invoke<ActionItem>("toggle_action_item", { id }),
  listPeople: () => invoke<Person[]>("list_people"),
  enrollSpeaker: (speakerId: string, name: string) => invoke<Speaker>("enroll_speaker", { speakerId, name }),
  nameSpeaker: (meetingId: string, speakerId: string, name: string) =>
    invoke<Speaker>("name_speaker", { meetingId, speakerId, name }),
  setLineSpeaker: (meetingId: string, segmentId: string, name: string) =>
    invoke<Speaker>("set_line_speaker", { meetingId, segmentId, name }),
  unnamedSpeakers: (meetingId: string) => invoke<string[]>("unnamed_speakers", { meetingId }),
  knownNames: () => invoke<string[]>("known_names"),
  mergePeople: (from: string, into: string) => invoke<Speaker>("merge_people", { from, into }),
  deleteVoiceprint: (personId: string) => invoke<void>("delete_voiceprint", { personId }),
  /** 48 values in 0..1 drawn from the voiceprint (lossy, for the People page only). */
  voiceprintShape: (personId: string) => invoke<number[] | null>("voiceprint_shape", { personId }),
  correctAnswer: (correction: Correction) => invoke<number>("correct_answer", { correction }),
  /** Tags not in the list yet join it. */
  setMeetingTags: (meetingId: string, tags: string[]) => invoke<Meeting>("set_meeting_tags", { meetingId, tags }),
  /** Renames the tag in the list, its rules and every meeting; returns how many meetings changed. */
  renameTag: (old: string, next: string) => invoke<number>("rename_tag", { old, new: next }),
  review: (meetingId: string) => invoke<Review[]>("review", { meetingId }),
  resolveReview: (meetingId: string, review: Review, accept: boolean) => invoke<void>("resolve_review", { meetingId, review, accept }),
  setOwner: (meetingId: string, segmentId: string, owner: string) => invoke<Segment>("set_owner", { meetingId, segmentId, owner }),
  rerunDiarization: (meetingId: string) => invoke<void>("rerun_diarization", { meetingId }),
  rerunClassification: (meetingId: string) => invoke<void>("rerun_classification", { meetingId }),
  renameMeeting: (meetingId: string, title: string) => invoke<Meeting>("rename_meeting", { meetingId, title }),
  saveNotes: (meetingId: string, raw: string[], atMs: (number | null)[]) => invoke<void>("save_notes", { meetingId, raw, atMs }),
  editSegment: (meetingId: string, segmentId: string, text: string) => invoke<Segment>("edit_segment", { meetingId, segmentId, text }),
  audioSrc: async (meetingId: string) => {
    const path = await invoke<string | null>("audio_path", { meetingId });
    return path ? convertFileSrc(path) : null;
  },
  addDemoMeetings: () => invoke<void>("add_demo_meetings"),
  deleteMeeting: (meetingId: string) => invoke<void>("delete_meeting", { meetingId }),
  clearMeetingParts: (meetingId: string, parts: MeetingPart[]) => invoke<void>("clear_meeting_parts", { meetingId, parts }),
  deleteTagged: (tag: string) => invoke<number>("delete_tagged", { tag }),
  runRetention: () => invoke<string[]>("run_retention"),
  storageUsage: () => invoke<[string, number][]>("storage_usage"),
  setEncryption: (on: boolean) => invoke<Settings>("set_encryption", { on }),
  listTemplates: () => invoke<Template[]>("list_templates"),
  /** Shows the summary templates folder in Finder. */
  openTemplates: () => invoke<void>("open_templates"),
  regenerate: (meetingId: string, template: string) => invoke<void>("regenerate", { meetingId, template }),
  /** Opens the file picker; the file name, or null when cancelled. Errors while one runs. */
  importAudio: () => invoke<string | null>("import_audio"),
  /** Resolves when every model is in place; progress arrives on the "models" event. */
  downloadModels: () => invoke<void>("download_models"),
  cancelDownload: () => invoke<void>("cancel_download"),
  startRecording: () => invoke<Meeting>("start_recording"),
  startCallRecording: (title: string | null, tag: string | null) => invoke<Meeting>("start_call_recording", { title, tag }),
  pendingCall: () => invoke<CallSuggestion | null>("pending_call"),
  dismissCall: () => invoke<void>("dismiss_call"),
  checkUpdate: () => invoke<UpdateInfo | null>("check_update"),
  pendingUpdate: () => invoke<UpdateInfo | null>("pending_update"),
  /** Downloads, installs and restarts the app; only returns on failure. */
  installUpdate: () => invoke<void>("install_update"),
  /** Opens the GitHub release page of `version` in the browser. */
  openReleaseNotes: (version: string) => invoke<void>("open_release_notes", { version }),
  stopRecording: () => invoke<string>("stop_recording"),
  setMicMuted: (muted: boolean) => invoke<void>("set_mic_muted", { muted }),
  recordingState: () => invoke<RecState>("recording_state"),
  requestPermission: (kind: "mic" | "system" | "calendar") => invoke<boolean>("request_permission", { kind }),
  status: (keys = false) => invoke<Status>("status", { keys }),
  setApiKey: (provider: string, key: string) => invoke<void>("set_api_key", { provider, key }),
  pickFolder: () => invoke<string | null>("pick_folder"),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
  /** Opens the settings window, or focuses it, on `tab`. */
  openSettings: (tab: string) => invoke<void>("open_settings", { tab }),
  showOnboarding: (step: number) => invoke<void>("show_onboarding", { step }),
  /** The menu bar popover's buttons; every one but "mute" closes it. */
  trayAction: (action: "toggle" | "mute" | "show" | "settings" | "quit" | "hide") => invoke<void>("tray_action", { action }),
  mcpActivity: (limit: number) => invoke<McpActivity[]>("mcp_activity", { limit }),
  /** Returns the new token, once. */
  addMcpClient: (name: string) => invoke<string>("add_mcp_client", { name }),
  revokeMcpClient: (name: string) => invoke<void>("revoke_mcp_client", { name }),
};

export const fmt = (ms: number, long = false) => {
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600);
  const mm = String(Math.floor(s / 60) % 60).padStart(2, "0");
  const ss = String(s % 60).padStart(2, "0");
  return long || h ? `${String(h).padStart(2, "0")}:${mm}:${ss}` : `${mm}:${ss}`;
};

/** "Today", "Yesterday", "This week", then the month ("October 2026"), from an RFC 3339 time. */
export const dayGroup = (created: string, now = new Date()) => {
  const d = new Date(created);
  if (Number.isNaN(d.getTime())) return "Undated";
  const day = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const ago = Math.round((day(now) - day(d)) / 86_400_000);
  if (ago <= 0) return "Today";
  if (ago === 1) return "Yesterday";
  if (ago < 7) return "This week";
  return d.toLocaleDateString(undefined, { month: "long", year: "numeric" });
};

export const LLM_LOCAL = (s: Settings) =>
  s.llm === "ollama" || (s.llm === "openai" && /^https?:\/\/(localhost|127\.0\.0\.1|\[::1\])(:|\/|$)/.test(s.llmBaseUrl.trim()));

/** Labels for the classifier's ids. */
export const MEETING_TYPES: Record<string, string> = {
  standup: "Standup",
  client_call: "Client call",
  one_on_one: "1:1",
  planning: "Planning",
  other: "Other",
};
/** A custom type is a template id: "sales-demo" → "Sales demo". */
export const typeLabel = (v: string) => MEETING_TYPES[v] ?? (v.charAt(0).toUpperCase() + v.slice(1)).replace(/[-_]/g, " ");
export const TONES = ["Very unhappy", "Concerned", "Neutral", "Positive", "Delighted"];
export const toneLabel = (t: number) => TONES[Math.min(4, Math.max(0, Math.round(t)))];
export const PRIORITIES = ["Nothing to do", "Low", "Normal", "High", "Very high", "Critical"];
export const priorityLabel = (p: number) => PRIORITIES[Math.min(5, Math.max(0, Math.round(p)))];
type Judged = { source: string; answer: boolean; probability: number; threshold: number; floor: number };
const unsure = (a: Judged) => a.source === "model" && a.answer && a.probability < a.threshold;
/** Same rule as `Answer::needs_review` in mirona-core. */
export const needsReview = (a: Judged) => unsure(a) && a.probability >= a.floor;
/** Same rule as `Answer::rejected`: below the floor, dropped without asking. */
export const rejected = (a: Judged) => unsure(a) && a.probability < a.floor;
/** A choice the model is sure enough of (or the user confirmed). */
export const sure = (c: { confirmed: boolean; probability: number; threshold: number }) => c.confirmed || c.probability >= c.threshold;
/** Every tag: the list in Settings plus any a meeting still carries, sorted. */
export const allTags = (st: Settings | null, meetings: { tags: string[] }[]) =>
  [...new Set([...Object.keys(st?.tags ?? {}), ...meetings.flatMap((m) => m.tags)])].sort((a, b) => a.localeCompare(b));

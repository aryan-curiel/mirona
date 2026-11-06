import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type { ActionItem } from "../bindings/ActionItem";
import type { Correction } from "../bindings/Correction";
import type { Meeting } from "../bindings/Meeting";
import type { MeetingSummary } from "../bindings/MeetingSummary";
import type { Review } from "../bindings/Review";
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
  /** Global record shortcut, a Tauri accelerator like "CmdOrCtrl+Alt+Shift+R". */
  shortcut: string;
  meetingsDir: string;
  modelsDir: string;
  classification: "jev" | "local-llm" | "off";
  diarization: "speakrs" | "speakrs-cpu" | "off";
  /** Question id → the user's threshold; the rest use `thresholdDefaults`. */
  thresholds: Record<string, number>;
  /** Defaults for the classifier in use (read-only). */
  thresholdDefaults: Record<string, number>;
  /** Client name → short description. */
  clients: Record<string, string>;
};

/** A person as `list_people` returns them: the speaker plus where they were last heard. */
export type Person = Speaker & { meetings: number; lastSeen: MeetingSummary | null };

export type Permission = "granted" | "denied" | "notDetermined";

/** Mirrors `Status` in src-tauri/src/lib.rs. */
export type Status = {
  mic: Permission;
  systemAudio: Permission;
  missingModels: string[];
  fetchCommand: string;
  keys: string[];
  configPath: string;
  cli: string;
  shortcut: string;
  errors: string[];
};

/** Sent on the "recording" channel whenever the recorder changes. */
export type RecState = { recording: string | null; processing: string[] };
export type Template = { id: string; name: string };

/** Typed wrappers over the Tauri commands in src-tauri/src/lib.rs. */
export const api = {
  listMeetings: () => invoke<MeetingSummary[]>("list_meetings"),
  getMeeting: (id: string) => invoke<Meeting>("get_meeting", { id }),
  listActionItems: () => invoke<ActionItem[]>("list_action_items"),
  toggleActionItem: (id: string) => invoke<ActionItem>("toggle_action_item", { id }),
  listPeople: () => invoke<Person[]>("list_people"),
  enrollSpeaker: (speakerId: string, name: string) => invoke<Speaker>("enroll_speaker", { speakerId, name }),
  nameSpeaker: (meetingId: string, speakerId: string, name: string) =>
    invoke<Speaker>("name_speaker", { meetingId, speakerId, name }),
  mergePeople: (from: string, into: string) => invoke<Speaker>("merge_people", { from, into }),
  deleteVoiceprint: (personId: string) => invoke<void>("delete_voiceprint", { personId }),
  correctAnswer: (correction: Correction) => invoke<number>("correct_answer", { correction }),
  confirmClient: (meetingId: string, client: string) => invoke<Meeting>("confirm_client", { meetingId, client }),
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
  listTemplates: () => invoke<Template[]>("list_templates"),
  regenerate: (meetingId: string, template: string) => invoke<void>("regenerate", { meetingId, template }),
  /** Opens the file picker; the file name, or null when cancelled. Errors while one runs. */
  importAudio: () => invoke<string | null>("import_audio"),
  startRecording: () => invoke<Meeting>("start_recording"),
  stopRecording: () => invoke<string>("stop_recording"),
  recordingState: () => invoke<RecState>("recording_state"),
  requestPermission: (kind: "mic" | "system") => invoke<boolean>("request_permission", { kind }),
  status: (keys = false) => invoke<Status>("status", { keys }),
  setApiKey: (provider: string, key: string) => invoke<void>("set_api_key", { provider, key }),
  pickFolder: () => invoke<string | null>("pick_folder"),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
};

export const fmt = (ms: number, long = false) => {
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600);
  const mm = String(Math.floor(s / 60) % 60).padStart(2, "0");
  const ss = String(s % 60).padStart(2, "0");
  return long || h ? `${String(h).padStart(2, "0")}:${mm}:${ss}` : `${mm}:${ss}`;
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
export const typeLabel = (v: string) => MEETING_TYPES[v] ?? v;
export const TONES = ["Very unhappy", "Concerned", "Neutral", "Positive", "Delighted"];
export const toneLabel = (t: number) => TONES[Math.min(4, Math.max(0, Math.round(t)))];
export const PRIORITIES = ["Nothing to do", "Low", "Normal", "High", "Very high", "Critical"];
export const priorityLabel = (p: number) => PRIORITIES[Math.min(5, Math.max(0, Math.round(p)))];
/** Same rule as `Answer::needs_review` in mirona-core. */
export const needsReview = (a: { source: string; answer: boolean; probability: number; threshold: number }) =>
  a.source === "model" && a.answer && a.probability < a.threshold;
/** A choice the model is sure enough of (or the user confirmed). */
export const sure = (c: { confirmed: boolean; probability: number; threshold: number }) => c.confirmed || c.probability >= c.threshold;

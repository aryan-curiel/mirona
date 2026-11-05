import { invoke } from "@tauri-apps/api/core";
import type { ActionItem } from "../bindings/ActionItem";
import type { Correction } from "../bindings/Correction";
import type { McpStatus } from "../bindings/McpStatus";
import type { Meeting } from "../bindings/Meeting";
import type { MeetingSummary } from "../bindings/MeetingSummary";
import type { Speaker } from "../bindings/Speaker";
import type { Stage } from "../bindings/Stage";

/** Typed wrappers over the Tauri commands in src-tauri/src/lib.rs. */
export const api = {
  listMeetings: () => invoke<MeetingSummary[]>("list_meetings"),
  getMeeting: (id: string) => invoke<Meeting>("get_meeting", { id }),
  listActionItems: () => invoke<ActionItem[]>("list_action_items"),
  toggleActionItem: (id: string) => invoke<ActionItem>("toggle_action_item", { id }),
  listPeople: () => invoke<Speaker[]>("list_people"),
  enrollSpeaker: (speakerId: string, name: string) => invoke<Speaker>("enroll_speaker", { speakerId, name }),
  correctAnswer: (correction: Correction) => invoke<number>("correct_answer", { correction }),
  confirmClient: (meetingId: string, client: string) => invoke<Meeting>("confirm_client", { meetingId, client }),
  saveNotes: (meetingId: string, raw: string[]) => invoke<void>("save_notes", { meetingId, raw }),
  listStages: () => invoke<Stage[]>("list_stages"),
  selectProvider: (stageId: string, optionId: string) => invoke<Stage>("select_provider", { stageId, optionId }),
  getPrivacy: () => invoke<boolean>("get_privacy"),
  setPrivacy: (on: boolean) => invoke<Stage[]>("set_privacy", { on }),
  getMcp: () => invoke<McpStatus>("get_mcp"),
  setMcpRunning: (running: boolean) => invoke<McpStatus>("set_mcp_running", { running }),
  startRecording: () => invoke<Meeting>("start_recording"),
  stopRecording: () => invoke<string>("stop_recording"),
};

export const fmt = (ms: number, long = false) => {
  const s = Math.floor(ms / 1000);
  const mm = String(Math.floor(s / 60)).padStart(2, "0");
  const ss = String(s % 60).padStart(2, "0");
  return long ? `00:${mm}:${ss}` : `${mm}:${ss}`;
};

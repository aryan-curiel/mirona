import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { api } from "./lib/api";
import type { ActionItem } from "./bindings/ActionItem";
import type { McpStatus } from "./bindings/McpStatus";
import type { Meeting } from "./bindings/Meeting";
import type { MeetingSummary } from "./bindings/MeetingSummary";
import type { PipelineEvent } from "./bindings/PipelineEvent";
import type { Speaker } from "./bindings/Speaker";
import type { Stage } from "./bindings/Stage";

export type View = "meetings" | "actions" | "people" | "models";
export type Tab = "transcript" | "notes" | "summary";

type Recording = { meetingId: string; elapsedMs: number; levels: number[]; processing: boolean };

type State = {
  view: View;
  tab: Tab;
  meetings: MeetingSummary[];
  meeting: Meeting | null;
  segmentId: string | null;
  filters: { hideSmallTalk: boolean; onlyTagged: boolean; showSensitive: boolean };
  privacy: boolean;
  actionItems: ActionItem[];
  people: Speaker[];
  stages: Stage[];
  mcp: McpStatus | null;
  corrections: number;
  recording: Recording | null;
  toast: string | null;

  init: () => Promise<void>;
  go: (view: View) => void;
  setTab: (tab: Tab) => void;
  openMeeting: (id: string) => Promise<void>;
  selectSegment: (id: string) => void;
  setFilter: (k: keyof State["filters"], v: boolean) => void;
  setPrivacy: (on: boolean) => Promise<void>;
  toggleAction: (id: string) => Promise<void>;
  correct: (segmentId: string, answerId: string, value: boolean) => Promise<void>;
  confirmClient: (client: string) => Promise<void>;
  enroll: (speakerId: string, name: string) => Promise<void>;
  selectProvider: (stageId: string, optionId: string) => Promise<void>;
  setMcpRunning: (on: boolean) => Promise<void>;
  startRecording: () => Promise<void>;
  stopRecording: () => Promise<void>;
  flash: (msg: string) => void;
};

let started = false;

export const useStore = create<State>((set, get) => {
  const reloadMeeting = async () => {
    const m = get().meeting;
    if (m) set({ meeting: await api.getMeeting(m.id) });
  };

  const onEvent = (e: PipelineEvent) => {
    const { recording, meeting } = get();
    const here = meeting?.id === e.meetingId;
    switch (e.stage) {
      case "captured":
        if (recording) set({ recording: { ...recording, elapsedMs: e.elapsedMs, levels: e.levels } });
        break;
      case "transcribed":
        if (here && meeting) {
          const speakers = meeting.speakers.some((s) => s.id === e.speaker.id) ? meeting.speakers : [...meeting.speakers, e.speaker];
          set({ meeting: { ...meeting, speakers, segments: [...meeting.segments, e.segment] } });
        }
        break;
      case "classified":
        if (here && meeting) {
          const segments = meeting.segments.map((s) => (s.id === e.segmentId ? { ...s, tags: e.tags } : s));
          set({ meeting: { ...meeting, segments } });
        }
        break;
      case "summarized":
        void (async () => {
          const [meetings, actionItems, mcp] = await Promise.all([api.listMeetings(), api.listActionItems(), api.getMcp()]);
          set({ meetings, actionItems, mcp, recording: null });
          if (here) {
            set({ meeting: await api.getMeeting(e.meetingId), tab: "summary" });
          }
          get().flash("Processed on this device. Summary is ready.");
        })();
        break;
    }
  };

  return {
    view: "meetings",
    tab: "transcript",
    meetings: [],
    meeting: null,
    segmentId: null,
    filters: { hideSmallTalk: false, onlyTagged: false, showSensitive: false },
    privacy: true,
    actionItems: [],
    people: [],
    stages: [],
    mcp: null,
    corrections: 0,
    recording: null,
    toast: null,

    init: async () => {
      if (started) return;
      started = true;
      const [meetings, actionItems, people, stages, mcp, privacy] = await Promise.all([
        api.listMeetings(),
        api.listActionItems(),
        api.listPeople(),
        api.listStages(),
        api.getMcp(),
        api.getPrivacy(),
      ]);
      set({ meetings, actionItems, people, stages, mcp, privacy });
      await get().openMeeting(meetings[0].id);
      set({ segmentId: "c5" });
      await listen<PipelineEvent>("pipeline", (ev) => onEvent(ev.payload));
    },

    go: (view) => set({ view }),
    setTab: (tab) => set({ tab }),

    openMeeting: async (id) => {
      const meeting = await api.getMeeting(id);
      const first = meeting.segments.find((s) => s.tags.some((t) => t.kind === "action")) ?? meeting.segments[0];
      set({ meeting, view: "meetings", segmentId: first?.id ?? null });
    },

    selectSegment: (id) => set({ segmentId: id }),
    setFilter: (k, v) => set({ filters: { ...get().filters, [k]: v } }),

    setPrivacy: async (on) => {
      const stages = await api.setPrivacy(on);
      set({ privacy: on, stages });
    },

    toggleAction: async (id) => {
      const item = await api.toggleActionItem(id);
      set({ actionItems: get().actionItems.map((a) => (a.id === id ? item : a)) });
    },

    correct: async (segmentId, answerId, value) => {
      const corrections = await api.correctAnswer({ segmentId, answerId, value });
      set({ corrections, actionItems: await api.listActionItems() });
      await reloadMeeting();
      get().flash("Correction saved as a labelled example.");
    },

    confirmClient: async (client) => {
      const m = get().meeting;
      if (!m) return;
      set({ meeting: await api.confirmClient(m.id, client), meetings: await api.listMeetings() });
    },

    enroll: async (speakerId, name) => {
      await api.enrollSpeaker(speakerId, name);
      const [people, actionItems] = await Promise.all([api.listPeople(), api.listActionItems()]);
      set({ people, actionItems });
      await reloadMeeting();
      get().flash(`Voiceprint enrolled. “Speaker 3” is now ${name} in every meeting.`);
    },

    selectProvider: async (stageId, optionId) => {
      try {
        const st = await api.selectProvider(stageId, optionId);
        set({ stages: get().stages.map((s) => (s.id === stageId ? st : s)) });
      } catch (e) {
        get().flash(String(e));
      }
    },

    setMcpRunning: async (on) => set({ mcp: await api.setMcpRunning(on) }),

    startRecording: async () => {
      const meeting = await api.startRecording();
      set({
        meeting,
        meetings: await api.listMeetings(),
        view: "meetings",
        tab: "transcript",
        segmentId: null,
        recording: { meetingId: meeting.id, elapsedMs: 0, levels: [], processing: false },
      });
    },

    stopRecording: async () => {
      const r = get().recording;
      if (!r) return;
      set({ recording: { ...r, processing: true } });
      await api.stopRecording();
    },

    flash: (msg) => {
      set({ toast: msg });
      setTimeout(() => get().toast === msg && set({ toast: null }), 3200);
    },
  };
});

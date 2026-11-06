import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { api, LLM_LOCAL, type RecState, type Settings, type Status, type Template } from "./lib/api";
import type { ActionItem } from "./bindings/ActionItem";
import type { Meeting } from "./bindings/Meeting";
import type { MeetingSummary } from "./bindings/MeetingSummary";
import type { PipelineEvent } from "./bindings/PipelineEvent";
import type { Speaker } from "./bindings/Speaker";

export type View = "meetings" | "actions" | "people" | "models" | "setup";
export type Tab = "transcript" | "notes" | "summary";

type Recording = { meetingId: string; elapsedMs: number; levels: number[] };

const SETUP_SEEN = "mirona.setupSeen";
const seen = () => {
  try {
    return localStorage.getItem(SETUP_SEEN) === "1";
  } catch {
    return false;
  }
};

/** Anything a first recording still needs: permissions or speech models. */
export const needsSetup = (s: Status | null) =>
  !!s && (s.mic !== "granted" || s.systemAudio !== "granted" || s.missingModels.length > 0);

type State = {
  view: View;
  tab: Tab;
  meetings: MeetingSummary[];
  meeting: Meeting | null;
  segmentId: string | null;
  /** Segment the transcript should scroll to on its next render. */
  scrollTo: string | null;
  filters: { hideSmallTalk: boolean; onlyTagged: boolean; showSensitive: boolean };
  settings: Settings | null;
  status: Status | null;
  templates: Template[];
  actionItems: ActionItem[];
  people: Speaker[];
  corrections: number;
  recording: Recording | null;
  /** Meetings being summarized after their recording stopped. */
  processing: string[];
  /** Meeting whose summary is being rewritten. */
  regenerating: string | null;
  /** Summaries streaming in, by meeting: the overview so far and the reply's length. */
  summarizing: Record<string, { overview: string; words: number }>;
  /** The audio file being imported; `fraction` is null until its length is known. */
  importing: { meetingId: string | null; file: string; fraction: number | null } | null;
  toast: { msg: string; error: boolean } | null;

  init: () => Promise<void>;
  go: (view: View) => void;
  finishSetup: () => void;
  setTab: (tab: Tab) => void;
  openMeeting: (id: string) => Promise<void>;
  selectSegment: (id: string) => void;
  jump: (ms: number) => void;
  setFilter: (k: keyof State["filters"], v: boolean) => void;
  refreshStatus: (keys?: boolean) => Promise<void>;
  saveSettings: (patch: Partial<Settings>) => Promise<boolean>;
  setPrivacy: (on: boolean) => Promise<void>;
  toggleAction: (id: string) => Promise<void>;
  correct: (segmentId: string, answerId: string, value: boolean) => Promise<void>;
  confirmClient: (client: string) => Promise<void>;
  rename: (title: string) => Promise<void>;
  editSegment: (segmentId: string, text: string) => Promise<void>;
  enroll: (speakerId: string, name: string) => Promise<void>;
  regenerate: (template: string) => Promise<void>;
  addDemo: () => Promise<void>;
  importAudio: () => Promise<void>;
  startRecording: () => Promise<void>;
  stopRecording: () => Promise<void>;
  flash: (msg: string, error?: boolean) => void;
};

let started = false;

const without = <T,>(rec: Record<string, T>, key: string) => {
  const { [key]: _, ...rest } = rec;
  return rest;
};

export const useStore = create<State>((set, get) => {
  const run = async (f: () => Promise<unknown>) => {
    try {
      await f();
    } catch (e) {
      get().flash(String(e), true);
    }
  };

  const reloadLists = async () => {
    const [meetings, actionItems, people] = await Promise.all([api.listMeetings(), api.listActionItems(), api.listPeople()]);
    set({ meetings, actionItems, people });
  };

  const reloadMeeting = async (id?: string) => {
    const m = get().meeting;
    if (m && (!id || id === m.id)) set({ meeting: await api.getMeeting(m.id) });
  };

  const onRecording = async (r: RecState) => {
    const was = get().recording;
    set({ processing: r.processing });
    if (r.recording && was?.meetingId !== r.recording) {
      set({ recording: { meetingId: r.recording, elapsedMs: 0, levels: [] } });
      await reloadLists();
      await get().openMeeting(r.recording);
      set({ tab: "transcript" });
    } else if (!r.recording && was) {
      set({ recording: null });
      await reloadLists();
      await reloadMeeting(was.meetingId);
    }
  };

  const onEvent = async (e: PipelineEvent) => {
    const { recording, meeting } = get();
    const here = meeting?.id === e.meetingId;
    switch (e.stage) {
      case "captured":
        if (recording?.meetingId === e.meetingId) set({ recording: { ...recording, elapsedMs: e.elapsedMs, levels: e.levels } });
        break;
      case "transcribed":
        if (here && meeting && !meeting.segments.some((s) => s.id === e.segment.id)) {
          const speakers = meeting.speakers.some((s) => s.id === e.speaker.id) ? meeting.speakers : [...meeting.speakers, e.speaker];
          set({ meeting: { ...meeting, speakers, segments: [...meeting.segments, e.segment] } });
        }
        break;
      case "classified":
        if (here && meeting) {
          const segments = meeting.segments.map((s) => (s.id === e.segmentId ? { ...s, tags: e.tags, answers: e.answers, owners: e.owners } : s));
          set({ meeting: { ...meeting, segments } });
        }
        break;
      case "importing":
        set({ importing: { meetingId: e.meetingId, file: e.file, fraction: e.fraction } });
        if (e.fraction === 1) await reloadLists(); // the meeting is saved; its summary follows
        break;
      case "summarizing":
        set({ summarizing: { ...get().summarizing, [e.meetingId]: { overview: e.overview, words: e.words } } });
        break;
      case "summarized":
        set({ summarizing: without(get().summarizing, e.meetingId) });
        if (get().regenerating === e.meetingId) set({ regenerating: null });
        await reloadLists();
        await reloadMeeting(e.meetingId);
        get().flash("Summary is ready.");
        break;
      case "notice": {
        if (e.error) set({ summarizing: without(get().summarizing, e.meetingId) });
        if (e.error && get().regenerating === e.meetingId) set({ regenerating: null });
        const imp = get().importing;
        // "Imported …" carries the meeting; "Import failed" has none.
        if (imp && (e.meetingId === imp.meetingId || e.message.startsWith("Import failed"))) set({ importing: null });
        get().flash(e.message, e.error);
        await reloadLists(); // an import may have added a meeting
        break;
      }
    }
  };

  return {
    view: "meetings",
    tab: "transcript",
    meetings: [],
    meeting: null,
    segmentId: null,
    scrollTo: null,
    filters: { hideSmallTalk: false, onlyTagged: false, showSensitive: false },
    settings: null,
    status: null,
    templates: [],
    actionItems: [],
    people: [],
    corrections: 0,
    recording: null,
    processing: [],
    regenerating: null,
    summarizing: {},
    importing: null,
    toast: null,

    init: async () => {
      if (started) return;
      started = true;
      await listen<PipelineEvent>("pipeline", (ev) => void onEvent(ev.payload));
      await listen<RecState>("recording", (ev) => void onRecording(ev.payload));
      const [settings, status, templates] = await Promise.all([api.getSettings(), api.status(), api.listTemplates().catch(() => [])]);
      set({ settings, status, templates });
      await reloadLists();
      for (const e of status.errors) get().flash(e, true);
      const first = get().meetings[0];
      if (first) await get().openMeeting(first.id);
      if (needsSetup(status) && !seen()) set({ view: "setup" });
      await onRecording(await api.recordingState());
    },

    go: (view) => {
      set({ view });
      if (view === "models" || view === "setup") void get().refreshStatus(view === "models");
    },

    finishSetup: () => {
      try {
        localStorage.setItem(SETUP_SEEN, "1");
      } catch {
        // private storage: the setup screen just shows again next time
      }
      set({ view: "meetings" });
    },

    setTab: (tab) => set({ tab }),

    openMeeting: async (id) => {
      const meeting = await api.getMeeting(id);
      set({ meeting, view: "meetings", segmentId: null, scrollTo: null });
    },

    selectSegment: (id) => set({ segmentId: id }),

    jump: (ms) => {
      const segs = get().meeting?.segments ?? [];
      // The last segment that starts at or before `ms`.
      const seg = segs.reduce<(typeof segs)[number] | undefined>((best, s) => (s.startMs <= ms ? s : best), undefined) ?? segs[0];
      if (seg) set({ tab: "transcript", segmentId: seg.id, scrollTo: seg.id });
    },

    setFilter: (k, v) => set({ filters: { ...get().filters, [k]: v } }),

    refreshStatus: (keys = false) => run(async () => set({ status: await api.status(keys) })),

    saveSettings: async (patch) => {
      const cur = get().settings;
      if (!cur) return false;
      try {
        const settings = await api.saveSettings({ ...cur, ...patch });
        const moved = settings.meetingsDir !== cur.meetingsDir;
        set({ settings });
        if (moved) {
          await reloadLists();
          const first = get().meetings[0];
          set({ meeting: null });
          if (first) await get().openMeeting(first.id);
          set({ view: "models" });
        }
        await get().refreshStatus(true);
        return true;
      } catch (e) {
        get().flash(String(e), true);
        return false;
      }
    },

    // Privacy on moves a cloud LLM back to the local default, like the gate expects.
    setPrivacy: async (on) => {
      const s = get().settings;
      if (!s) return;
      await get().saveSettings(on && !LLM_LOCAL(s) ? { privacy: on, llm: "ollama", llmModel: "", llmBaseUrl: "" } : { privacy: on });
    },

    toggleAction: (id) =>
      run(async () => {
        const item = await api.toggleActionItem(id);
        set({ actionItems: get().actionItems.map((a) => (a.id === id ? item : a)) });
      }),

    correct: (segmentId, answerId, value) =>
      run(async () => {
        const corrections = await api.correctAnswer({ segmentId, answerId, value });
        set({ corrections, actionItems: await api.listActionItems() });
        await reloadMeeting();
        get().flash("Correction saved as a labelled example.");
      }),

    confirmClient: (client) =>
      run(async () => {
        const m = get().meeting;
        if (!m) return;
        set({ meeting: await api.confirmClient(m.id, client), meetings: await api.listMeetings() });
      }),

    rename: (title) =>
      run(async () => {
        const m = get().meeting;
        if (!m || title.trim() === m.title) return;
        set({ meeting: await api.renameMeeting(m.id, title) });
        await reloadLists();
      }),

    editSegment: (segmentId, text) =>
      run(async () => {
        const m = get().meeting;
        if (!m) return;
        const seg = await api.editSegment(m.id, segmentId, text);
        const cur = get().meeting;
        if (cur?.id === m.id) set({ meeting: { ...cur, segments: cur.segments.map((s) => (s.id === seg.id ? seg : s)) } });
      }),

    enroll: (speakerId, name) =>
      run(async () => {
        await api.enrollSpeaker(speakerId, name);
        await reloadLists();
        await reloadMeeting();
        get().flash(`Voiceprint enrolled as ${name} in every meeting.`);
      }),

    regenerate: (template) =>
      run(async () => {
        const m = get().meeting;
        if (!m) return;
        set({ regenerating: m.id });
        await api.regenerate(m.id, template);
      }),

    addDemo: () =>
      run(async () => {
        await api.addDemoMeetings();
        await reloadLists();
        const first = get().meetings[0];
        if (first) await get().openMeeting(first.id);
      }),

    importAudio: () =>
      run(async () => {
        if (get().importing) return;
        const name = await api.importAudio();
        // Progress events may already have filled this in.
        if (name && !get().importing) set({ importing: { meetingId: null, file: name, fraction: null } });
      }),

    startRecording: () =>
      run(async () => {
        try {
          await api.startRecording();
        } catch (e) {
          await get().refreshStatus();
          if (needsSetup(get().status)) set({ view: "setup" });
          throw e;
        }
      }),

    stopRecording: () => run(() => api.stopRecording()),

    flash: (msg, error = false) => {
      const toast = { msg, error };
      set({ toast });
      setTimeout(() => get().toast === toast && set({ toast: null }), error ? 8000 : 3200);
    },
  };
});

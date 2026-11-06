import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { api, type CallSuggestion, LLM_LOCAL, type Person, type RecState, type Settings, type Status, type Template } from "./lib/api";
import type { ActionItem } from "./bindings/ActionItem";
import type { Meeting } from "./bindings/Meeting";
import type { MeetingSummary } from "./bindings/MeetingSummary";
import type { PipelineEvent } from "./bindings/PipelineEvent";
import type { Review } from "./bindings/Review";

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
  people: Person[];
  /** What in the open meeting waits for the user's confirmation (NGR-158). */
  review: Review[];
  /** Meetings whose speakers or tags are being redone, with what is running. */
  rerunning: Record<string, string>;
  /** Sidebar: only meetings for this client (null: all). */
  clientFilter: string | null;
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
  /** A call app just opened the mic: suggest recording it (NGR-170). */
  call: CallSuggestion | null;

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
  nameSpeaker: (speakerId: string, name: string) => Promise<void>;
  merge: (from: string, into: string) => Promise<void>;
  deleteVoiceprint: (personId: string) => Promise<void>;
  resolve: (review: Review, accept: boolean) => Promise<void>;
  setOwner: (segmentId: string, owner: string) => Promise<void>;
  rerun: (what: "diarization" | "classification") => Promise<void>;
  openAt: (meetingId: string, ms: number) => Promise<void>;
  setClientFilter: (client: string | null) => void;
  regenerate: (template: string) => Promise<void>;
  addDemo: () => Promise<void>;
  /** Reload the lists after meetings were deleted; reopens one if the open one is gone. */
  reload: () => Promise<void>;
  importAudio: () => Promise<void>;
  startRecording: () => Promise<void>;
  stopRecording: () => Promise<void>;
  recordCall: () => Promise<void>;
  dismissCall: () => void;
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
    if (m && (!id || id === m.id)) {
      const [meeting, review] = await Promise.all([api.getMeeting(m.id), api.review(m.id).catch(() => [])]);
      set({ meeting, review });
    }
  };

  /** After a correction: the meeting, its review list, action items and people. */
  const afterFix = async () => {
    await reloadMeeting();
    await reloadLists();
  };

  const onRecording = async (r: RecState) => {
    const was = get().recording;
    set({ processing: r.processing });
    if (r.recording && was?.meetingId !== r.recording) {
      set({ recording: { meetingId: r.recording, elapsedMs: 0, levels: [] }, call: null });
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
          set({ meeting: { ...meeting, segments }, review: await api.review(meeting.id).catch(() => get().review) });
        }
        break;
      case "diarized":
        await reloadLists();
        await reloadMeeting(e.meetingId);
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
        set({ rerunning: without(get().rerunning, e.meetingId) });
        await reloadLists(); // an import may have added a meeting
        await reloadMeeting(e.meetingId); // a rerun or follow-up draft may have changed it
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
    review: [],
    rerunning: {},
    clientFilter: null,
    corrections: 0,
    recording: null,
    processing: [],
    regenerating: null,
    summarizing: {},
    importing: null,
    toast: null,
    call: null,

    init: async () => {
      if (started) return;
      started = true;
      await listen<PipelineEvent>("pipeline", (ev) => void onEvent(ev.payload));
      await listen<RecState>("recording", (ev) => void onRecording(ev.payload));
      await listen<CallSuggestion>("call", (ev) => set({ call: ev.payload }));
      const [settings, status, templates] = await Promise.all([api.getSettings(), api.status(), api.listTemplates().catch(() => [])]);
      set({ settings, status, templates });
      await reloadLists();
      for (const e of status.errors) get().flash(e, true);
      const first = get().meetings[0];
      if (first) await get().openMeeting(first.id);
      if (needsSetup(status) && !seen()) set({ view: "setup" });
      await onRecording(await api.recordingState());
      // Pick up meetings the CLI or an import wrote while the window was open.
      const sync = async () => {
        if (await api.refreshStore().catch(() => false)) {
          await reloadLists();
          await reloadMeeting();
        }
      };
      window.addEventListener("focus", () => void sync());
      setInterval(() => void sync(), 5000);
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
      const [meeting, review] = await Promise.all([api.getMeeting(id), api.review(id).catch(() => [])]);
      set({ meeting, review, view: "meetings", segmentId: null, scrollTo: null });
    },

    openAt: async (id, ms) => {
      await get().openMeeting(id);
      get().jump(ms);
    },

    setClientFilter: (clientFilter) => set({ clientFilter }),

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
        set({ corrections });
        await afterFix();
        get().flash("Correction saved as a labelled example.");
      }),

    confirmClient: (client) =>
      run(async () => {
        const m = get().meeting;
        if (!m) return;
        await api.confirmClient(m.id, client);
        await afterFix();
      }),

    resolve: (review, accept) =>
      run(async () => {
        const m = get().meeting;
        if (!m) return;
        await api.resolveReview(m.id, review, accept);
        await afterFix();
      }),

    setOwner: (segmentId, owner) =>
      run(async () => {
        const m = get().meeting;
        if (!m) return;
        await api.setOwner(m.id, segmentId, owner);
        await afterFix();
        get().flash(`Owner set to ${owner}.`);
      }),

    rerun: (what) =>
      run(async () => {
        const m = get().meeting;
        if (!m) return;
        await (what === "diarization" ? api.rerunDiarization(m.id) : api.rerunClassification(m.id));
        set({ rerunning: { ...get().rerunning, [m.id]: what === "diarization" ? "Finding speakers…" : "Tagging again…" } });
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

    nameSpeaker: (speakerId, name) =>
      run(async () => {
        const m = get().meeting;
        if (!m) return;
        const p = await api.nameSpeaker(m.id, speakerId, name);
        await afterFix();
        get().flash(`Named ${p.name}: transcript, summary and action items updated.`);
      }),

    merge: (from, into) =>
      run(async () => {
        const p = await api.mergePeople(from, into);
        await afterFix();
        get().flash(`Merged into ${p.name} in every meeting.`);
      }),

    deleteVoiceprint: (personId) =>
      run(async () => {
        await api.deleteVoiceprint(personId);
        await afterFix();
        get().flash("Voiceprint deleted. Meetings keep the name.");
      }),

    regenerate: (template) =>
      run(async () => {
        const m = get().meeting;
        if (!m) return;
        set({ regenerating: m.id });
        await api.regenerate(m.id, template);
      }),

    reload: () =>
      run(async () => {
        await reloadLists();
        const m = get().meeting;
        if (m && !get().meetings.some((x) => x.id === m.id)) {
          set({ meeting: null, review: [] });
          const first = get().meetings[0];
          if (first) await get().openMeeting(first.id);
        }
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

    recordCall: () =>
      run(async () => {
        const call = get().call;
        if (!call) return;
        set({ call: null });
        await api.startCallRecording(call.title, call.client);
        // Named after the calendar event once it started: show the new title.
        await reloadLists();
        await reloadMeeting();
      }),

    dismissCall: () => set({ call: null }),

    flash: (msg, error = false) => {
      const toast = { msg, error };
      set({ toast });
      setTimeout(() => get().toast === toast && set({ toast: null }), error ? 8000 : 3200);
    },
  };
});

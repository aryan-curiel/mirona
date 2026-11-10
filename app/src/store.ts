import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { api, type CallSuggestion, LLM_LOCAL, type Person, type RecState, type Settings, type Status, type Template, type UpdateInfo } from "./lib/api";
import type { ActionItem } from "./bindings/ActionItem";
import type { Meeting } from "./bindings/Meeting";
import type { MeetingSummary } from "./bindings/MeetingSummary";
import type { MeetingPart } from "./bindings/MeetingPart";
import type { PipelineEvent } from "./bindings/PipelineEvent";
import type { Review } from "./bindings/Review";
import type { Segment } from "./bindings/Segment";
import type { Speaker } from "./bindings/Speaker";

export type View = "meetings" | "all" | "actions" | "people" | "activity";
export type SettingsTab = "general" | "privacy" | "models" | "tags" | "agents" | "advanced" | "about";
export type Tab = "transcript" | "notes" | "summary";

/** `drafts`: each audio lane's line still being spoken (not saved or tagged yet). */
type Recording = { meetingId: string; elapsedMs: number; levels: number[]; drafts: { segment: Segment; speaker: Speaker }[] };

const DETAILS = "mirona.modelDetails";
/** localStorage that never throws (private windows, blocked storage). */
const stored = (k: string) => {
  try {
    return localStorage.getItem(k);
  } catch {
    return null;
  }
};
const remember = (k: string, v: string) => {
  try {
    localStorage.setItem(k, v);
  } catch {
    // the preference just isn't remembered
  }
};

/** Anything a first recording still needs: permissions or speech models. */
export const needsSetup = (s: Status | null) =>
  !!s && (s.mic !== "granted" || s.systemAudio !== "granted" || s.missingModels.length > 0);

type State = {
  view: View;
  tab: Tab;
  settingsTab: SettingsTab;
  /** Inspector: show probabilities, thresholds and the classifier's raw answers. */
  modelDetails: boolean;
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
  corrections: number;
  recording: Recording | null;
  /** Meetings being summarized after their recording stopped. */
  processing: string[];
  /** The mic records silence in the current recording. */
  micMuted: boolean;
  /** Meeting whose summary is being rewritten. */
  regenerating: string | null;
  /** Summaries streaming in, by meeting: the overview so far and the reply's length. */
  summarizing: Record<string, { overview: string; words: number }>;
  /** The audio file being imported; `fraction` is null until its length is known. */
  importing: { meetingId: string | null; file: string; fraction: number | null } | null;
  toast: { msg: string; error: boolean } | null;
  /** A call app just opened the mic: suggest recording it (NGR-170). */
  call: CallSuggestion | null;
  /** The consent reminder is open before recording (NGR-169); `call`: from the call banner. */
  consent: { call: boolean } | null;
  /** A newer version is ready to install; `installing` while it downloads. */
  update: UpdateInfo | null;
  installing: boolean;
  dismissedUpdate: string | null;

  init: () => Promise<void>;
  /** The settings window: settings, status and the client list only. */
  initSettings: () => Promise<void>;
  go: (view: View) => void;
  /** Opens (or focuses) the settings window on `tab`. */
  openSettings: (tab: SettingsTab) => void;
  setModelDetails: (on: boolean) => void;
  /** Remove generated parts of a meeting; transcript and notes stay. */
  removeParts: (id: string, parts: MeetingPart[]) => Promise<void>;
  /** Delete whole meetings everywhere. */
  deleteMeetings: (ids: string[]) => Promise<void>;
  /** The onboarding window at `step` (0 welcome, 1 permissions and models), on top of this one. */
  openOnboarding: (step: number) => void;
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
  setTags: (tags: string[]) => Promise<void>;
  rename: (title: string) => Promise<void>;
  editSegment: (segmentId: string, text: string) => Promise<void>;
  enroll: (speakerId: string, name: string) => Promise<void>;
  nameSpeaker: (speakerId: string, name: string) => Promise<void>;
  /** Move one line to another person (by name; a new name is a new person). */
  moveLine: (segmentId: string, name: string) => Promise<void>;
  merge: (from: string, into: string) => Promise<void>;
  deleteVoiceprint: (personId: string) => Promise<void>;
  resolve: (review: Review, accept: boolean) => Promise<void>;
  setOwner: (segmentId: string, owner: string) => Promise<void>;
  rerun: (what: "diarization" | "classification") => Promise<void>;
  openAt: (meetingId: string, ms: number) => Promise<void>;
  regenerate: (template: string) => Promise<void>;
  addDemo: () => Promise<void>;
  /** Reload the lists after meetings were deleted; reopens one if the open one is gone. */
  reload: () => Promise<void>;
  importAudio: () => Promise<void>;
  startRecording: () => Promise<void>;
  stopRecording: () => Promise<void>;
  setMicMuted: (muted: boolean) => Promise<void>;
  recordCall: () => Promise<void>;
  /** From the consent reminder: start, optionally tagging the call with `tag`. */
  confirmConsent: (tag: string | null, agreed: boolean, stopReminding: boolean) => Promise<void>;
  cancelConsent: () => void;
  dismissCall: () => void;
  installUpdate: () => Promise<void>;
  dismissUpdate: () => void;
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

  /** Template files can change in Finder; refetch when a window comes back. */
  const reloadTemplates = () => void api.listTemplates().then((templates) => set({ templates })).catch(() => {});
  const reloadLists = async () => {
    const [meetings, actionItems, people] = await Promise.all([api.listMeetings(), api.listActionItems(), api.listPeople()]);
    set({ meetings, actionItems, people });
  };

  /** Any window saved settings or deleted meetings from them: catch up. */
  const onSettings = async () => {
    set({ settings: await api.getSettings() });
    await get().refreshStatus(true);
    await get().reload();
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

  /** Start recording now; `tag` tags the meeting right away. */
  const startNow = async (tag: string | null) => {
    try {
      const m = await api.startRecording();
      if (tag) await api.setMeetingTags(m.id, [tag]);
    } catch (e) {
      await get().refreshStatus();
      if (needsSetup(get().status)) get().openOnboarding(1);
      throw e;
    }
  };

  const recordCallNow = async () => {
    const call = get().call;
    if (!call) return;
    set({ call: null });
    await api.startCallRecording(call.title, call.tag);
    // Named after the calendar event once it started: show the new title.
    await reloadLists();
    await reloadMeeting();
  };

  const onRecording = async (r: RecState) => {
    const was = get().recording;
    set({ processing: r.processing, micMuted: r.micMuted });
    if (r.recording && was?.meetingId !== r.recording) {
      set({ recording: { meetingId: r.recording, elapsedMs: 0, levels: [], drafts: [] }, call: null });
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
      case "drafting":
        if (recording?.meetingId === e.meetingId) {
          const drafts = [...recording.drafts.filter((d) => d.segment.id !== e.segment.id), { segment: e.segment, speaker: e.speaker }];
          set({ recording: { ...recording, drafts: drafts.sort((a, b) => a.segment.startMs - b.segment.startMs) } });
        }
        break;
      case "transcribed":
        if (recording?.meetingId === e.meetingId) {
          set({ recording: { ...recording, drafts: recording.drafts.filter((d) => d.segment.id !== e.segment.id) } });
        }
        if (here && meeting && !meeting.segments.some((s) => s.id === e.segment.id)) {
          const speakers = meeting.speakers.some((s) => s.id === e.speaker.id) ? meeting.speakers : [...meeting.speakers, e.speaker];
          // A lane's line is confirmed when its sentence ends, so it can land before a later one.
          const segments = [...meeting.segments];
          let at = segments.length;
          while (at > 0 && segments[at - 1].startMs > e.segment.startMs) at--;
          segments.splice(at, 0, e.segment);
          set({ meeting: { ...meeting, speakers, segments } });
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
    settingsTab: "general",
    modelDetails: stored(DETAILS) === "1",
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
    corrections: 0,
    recording: null,
    processing: [],
    micMuted: false,
    regenerating: null,
    summarizing: {},
    importing: null,
    toast: null,
    call: null,
    consent: null,
    update: null,
    installing: false,
    dismissedUpdate: null,

    init: async () => {
      if (started) return;
      started = true;
      await listen<PipelineEvent>("pipeline", (ev) => void onEvent(ev.payload));
      await listen<RecState>("recording", (ev) => void onRecording(ev.payload));
      await listen<CallSuggestion | null>("call", (ev) => set({ call: ev.payload }));
      await listen("settings", () => void onSettings());
      await listen<UpdateInfo | null>("update", (ev) => set({ update: ev.payload }));
      window.addEventListener("focus", reloadTemplates);
      // A call that started before this window listened.
      set({ call: await api.pendingCall().catch(() => null), update: await api.pendingUpdate().catch(() => null) });
      const [settings, status, templates] = await Promise.all([api.getSettings(), api.status(), api.listTemplates().catch(() => [])]);
      set({ settings, status, templates });
      await reloadLists();
      for (const e of status.errors) get().flash(e, true);
      const first = get().meetings[0];
      if (first) await get().openMeeting(first.id);
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

    initSettings: async () => {
      if (started) return;
      started = true;
      const tab = (window as { miSettingsTab?: SettingsTab }).miSettingsTab;
      if (tab) set({ settingsTab: tab });
      await listen("settings", () => void onSettings());
      await listen<SettingsTab>("settings-tab", (ev) => set({ settingsTab: ev.payload }));
      await listen<UpdateInfo | null>("update", (ev) => set({ update: ev.payload }));
      window.addEventListener("focus", reloadTemplates);
      set({ update: await api.pendingUpdate().catch(() => null) });
      const [settings, status, templates] = await Promise.all([api.getSettings(), api.status(true), api.listTemplates().catch(() => [])]);
      set({ settings, status, templates });
      await reloadLists();
    },

    go: (view) => set({ view }),

    openSettings: (tab) => void run(() => api.openSettings(tab)),

    setModelDetails: (on) => {
      remember(DETAILS, on ? "1" : "0");
      set({ modelDetails: on });
    },

    removeParts: (id, parts) =>
      run(async () => {
        await api.clearMeetingParts(id, parts);
        await reloadLists();
        await reloadMeeting(id);
        get().flash("Removed. The transcript and your notes are kept.");
      }),

    deleteMeetings: (ids) =>
      run(async () => {
        const titles = new Map(get().meetings.map((m) => [m.id, m.title]));
        let n = 0;
        try {
          for (const id of ids) {
            await api.deleteMeeting(id);
            n++;
          }
        } finally {
          await get().reload();
        }
        get().flash(n === 1 ? `Deleted “${titles.get(ids[0])}” everywhere.` : `Deleted ${n} meetings everywhere.`);
      }),

    openOnboarding: (step) => void run(() => api.showOnboarding(step)),

    setTab: (tab) => set({ tab }),

    openMeeting: async (id) => {
      const [meeting, review] = await Promise.all([api.getMeeting(id), api.review(id).catch(() => [])]);
      set({ meeting, review, view: "meetings", segmentId: null, scrollTo: null });
    },

    openAt: async (id, ms) => {
      await get().openMeeting(id);
      get().jump(ms);
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
        // The "settings" event then refreshes status and the lists in every window.
        set({ settings: await api.saveSettings({ ...cur, ...patch }) });
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

    setTags: (tags) =>
      run(async () => {
        const m = get().meeting;
        if (!m) return;
        await api.setMeetingTags(m.id, tags);
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

    moveLine: (segmentId, name) =>
      run(async () => {
        const m = get().meeting;
        if (!m) return;
        const p = await api.setLineSpeaker(m.id, segmentId, name);
        await afterFix();
        get().flash(`Moved the line to ${p.name}.`);
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
        if (get().settings?.consentReminder) {
          set({ consent: { call: false } });
          return;
        }
        await startNow(null);
      }),

    confirmConsent: (tag, agreed, stopReminding) =>
      run(async () => {
        const c = get().consent;
        set({ consent: null });
        const st = get().settings;
        if (st && ((agreed && tag && !st.consentGiven.includes(tag)) || stopReminding)) {
          await get().saveSettings({
            consentGiven: agreed && tag ? [...st.consentGiven, tag] : st.consentGiven,
            consentReminder: !stopReminding,
          });
        }
        if (c?.call) await recordCallNow();
        else await startNow(tag);
      }),

    cancelConsent: () => set({ consent: null }),

    stopRecording: () => run(() => api.stopRecording()),

    setMicMuted: (muted) => run(() => api.setMicMuted(muted)),

    recordCall: () =>
      run(async () => {
        const call = get().call;
        if (!call) return;
        const st = get().settings;
        if (st?.consentReminder && !(call.tag && st.consentGiven.includes(call.tag))) {
          set({ consent: { call: true } });
          return;
        }
        await recordCallNow();
      }),

    dismissCall: () => {
      set({ call: null });
      void api.dismissCall();
    },

    installUpdate: async () => {
      set({ installing: true });
      try {
        await api.installUpdate();
      } catch (e) {
        get().flash(String(e), true);
      }
      set({ installing: false });
    },

    dismissUpdate: () => set({ dismissedUpdate: get().update?.version ?? null }),

    flash: (msg, error = false) => {
      const toast = { msg, error };
      set({ toast });
      setTimeout(() => get().toast === toast && set({ toast: null }), error ? 8000 : 3200);
    },
  };
});

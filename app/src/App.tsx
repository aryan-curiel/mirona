import { useEffect, useState } from "react";
import { useStore } from "./store";
import { applyPlatform, detected, type Platform } from "./lib/platform";
import { CallBanner, ConsentSheet, RecordingBar, Sidebar, Toast, UpdateBanner, WindowsTitleBar } from "./components/Shell";
import { MeetingView } from "./views/Meeting";
import { Inspector } from "./views/Inspector";
import { ActionItemsView, AgentActivityView, AllMeetingsView, PeopleView } from "./views/Pages";

export default function App() {
  const view = useStore((s) => s.view);
  const hasMeeting = useStore((s) => !!s.meeting);
  const init = useStore((s) => s.init);
  const [platform, setPlatform] = useState<Platform>(detected);
  useEffect(() => void init(), [init]);
  useEffect(() => applyPlatform(platform), [platform]);
  // Cmd/Ctrl+, opens Settings. Cmd/Ctrl+Shift+L previews the other platform's skin (demo only).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === ",") useStore.getState().openSettings("general");
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "l") setPlatform((p) => (p === "windows" ? "macos" : "windows"));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const mac = platform === "macos";

  return (
    <div className="flex h-full flex-col">
      {!mac && <WindowsTitleBar />}
      <div className="flex min-h-0 flex-1">
        <Sidebar mac={mac} />
        <div className="mi-main flex min-w-0 flex-1 flex-col">
          <RecordingBar />
          <CallBanner />
          <UpdateBanner />
          <div className="flex min-h-0 flex-1">
            <main id="main-scroll" className="min-w-0 flex-1 overflow-y-auto">
              {mac && <div data-tauri-drag-region className="sticky top-0 z-10 -mb-6 h-6" />}
              {view === "meetings" && <MeetingView />}
              {view === "actions" && <ActionItemsView />}
              {view === "people" && <PeopleView />}
              {view === "activity" && <AgentActivityView />}
              {view === "all" && <AllMeetingsView />}
            </main>
            {view === "meetings" && hasMeeting && <Inspector />}
          </div>
        </div>
      </div>
      <ConsentSheet />
      <Toast />
    </div>
  );
}

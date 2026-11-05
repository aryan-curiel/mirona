import { useEffect, useState } from "react";
import { useStore } from "./store";
import { applyPlatform, detected, type Platform } from "./lib/platform";
import { RecordingBar, Sidebar, WindowsTitleBar } from "./components/Shell";
import { MeetingView } from "./views/Meeting";
import { Inspector } from "./views/Inspector";
import { ActionItemsView, ModelsView, PeopleView } from "./views/Pages";

export default function App() {
  const view = useStore((s) => s.view);
  const toast = useStore((s) => s.toast);
  const init = useStore((s) => s.init);
  const [platform, setPlatform] = useState<Platform>(detected);
  useEffect(() => void init(), [init]);
  useEffect(() => applyPlatform(platform), [platform]);
  // Cmd/Ctrl+Shift+L previews the other platform's skin (demo only).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
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
          <div className="flex min-h-0 flex-1">
            <main className="min-w-0 flex-1 overflow-y-auto">
              {mac && <div data-tauri-drag-region className="sticky top-0 z-10 -mb-6 h-6" />}
              {view === "meetings" && <MeetingView />}
              {view === "actions" && <ActionItemsView />}
              {view === "people" && <PeopleView />}
              {view === "models" && <ModelsView />}
            </main>
            {view === "meetings" && <Inspector />}
          </div>
        </div>
      </div>
      {toast && (
        <div role="status" className="mi-card mi-enter fixed bottom-5 left-1/2 -translate-x-1/2 rounded-lg! px-4 py-2.5 text-[14px] font-medium">
          {toast}
        </div>
      )}
    </div>
  );
}

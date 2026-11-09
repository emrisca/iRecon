import { useState } from "react";

import { AppSidebar, type Section } from "./components/app-sidebar";
import {
  hideOverlay,
  isDesktop,
  showOverlay,
  startCapture,
  stopCapture,
} from "./lib/api";
import { useTheme } from "./hooks/use-theme";
import { CaptureLabPage } from "./pages/capture-lab";
import { HomePage } from "./pages/home";
import { DetectorsPage } from "./pages/detectors";
import { EvalPage } from "./pages/eval";
import { LabelPage } from "./pages/label";
import { LibraryPage } from "./pages/library";
import { ProjectsPage } from "./pages/projects";
import { ReviewPage } from "./pages/review";
import { SessionsPage } from "./pages/sessions";
import { SourcesPage } from "./pages/sources";

export default function App() {
  const [section, setSection] = useState<Section>("home");
  const [live, setLive] = useState(false);
  const [liveBusy, setLiveBusy] = useState(false);
  const [liveError, setLiveError] = useState<string | null>(null);
  const { theme, toggle } = useTheme();

  async function toggleLive() {
    setLiveBusy(true);
    setLiveError(null);
    try {
      if (live) {
        await hideOverlay();
        await stopCapture();
        setLive(false);
      } else {
        await startCapture();
        await showOverlay();
        setLive(true);
      }
    } catch (reason) {
      setLiveError(String(reason));
    } finally {
      setLiveBusy(false);
    }
  }

  return (
    <div className="flex h-full min-h-0 bg-background text-foreground">
      <AppSidebar
        section={section}
        onSelect={setSection}
        theme={theme}
        onToggleTheme={toggle}
        live={live}
        liveBusy={liveBusy}
        onToggleLive={toggleLive}
      />
      <main className="flex min-h-0 min-w-0 flex-1 flex-col">
        {liveError ? (
          <p role="alert" className="border-b border-border p-2 text-sm text-destructive">
            {liveError}
          </p>
        ) : null}
        {!isDesktop ? (
          <div className="border-b border-border bg-muted px-2.5 py-2 text-xs text-muted-foreground">
            Browser preview. Writes stay in memory and screen capture is unavailable.
          </div>
        ) : null}
        <div className="min-h-0 flex-1 overflow-y-auto">
          {section === "home" ? <HomePage onSelect={setSection} /> : null}
          {section === "projects" ? <ProjectsPage /> : null}
          {section === "sources" ? <SourcesPage /> : null}
          {section === "library" ? <LibraryPage /> : null}
          {section === "review" ? <ReviewPage /> : null}
          {section === "label" ? <LabelPage /> : null}
          {section === "capture" ? <CaptureLabPage /> : null}
          {section === "sessions" ? <SessionsPage /> : null}
          {section === "detectors" ? <DetectorsPage /> : null}
          {section === "eval" ? <EvalPage /> : null}
        </div>
      </main>
    </div>
  );
}

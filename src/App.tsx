import { useCallback, useEffect, useState } from "react";

import { AppSidebar, type Section } from "./components/app-sidebar";
import { ProjectProvider, useProject } from "./components/project-context";
import { SidebarContext } from "./components/sidebar-context";
import {
  getSetting,
  hideOverlay,
  isDesktop,
  listSources,
  setSetting,
  showOverlay,
  startCapture,
  stopCapture,
} from "./lib/api";
import { useTheme } from "./hooks/use-theme";
import type { SourceView } from "./lib/types";
import { CaptureLabPage } from "./pages/capture-lab";
import { DetectorsPage } from "./pages/detectors";
import { EvalPage } from "./pages/eval";
import { LabelPage } from "./pages/label";
import { LibraryPage } from "./pages/library";
import { ProjectsPage } from "./pages/projects";
import { ReviewPage } from "./pages/review";
import { SessionsPage } from "./pages/sessions";
import { SourcesPage } from "./pages/sources";

const SECTIONS: Section[] = [
  "projects",
  "sources",
  "library",
  "review",
  "label",
  "capture",
  "sessions",
  "detectors",
  "eval",
];

/// The last section survives a restart. A stored value from an older build that
/// no longer exists falls back to Projects rather than an empty shell.
function savedSection(): Section {
  if (typeof window === "undefined") return "projects";
  const stored = window.localStorage.getItem("irecon.section");
  return SECTIONS.includes(stored as Section) ? (stored as Section) : "projects";
}

export default function App() {
  return (
    <ProjectProvider>
      <AppShell />
    </ProjectProvider>
  );
}

function AppShell() {
  const [section, setSection] = useState<Section>(savedSection);
  const [sidebarOpen, setSidebarOpen] = useState(() => {
    if (typeof window === "undefined") return true;
    return window.localStorage.getItem("irecon.sidebar") !== "closed";
  });
  const [live, setLive] = useState(false);
  const [liveBusy, setLiveBusy] = useState(false);
  const [liveError, setLiveError] = useState<string | null>(null);
  const [liveSourceId, setLiveSourceId] = useState<number | null>(null);
  const [liveSource, setLiveSource] = useState("Primary display");
  const [liveSources, setLiveSources] = useState<SourceView[]>([]);
  const { projects, projectId, project, selectProject } = useProject();
  const { theme, toggle } = useTheme();

  const toggleSidebar = useCallback(() => {
    setSidebarOpen((current) => !current);
  }, []);

  useEffect(() => {
    window.localStorage.setItem("irecon.sidebar", sidebarOpen ? "open" : "closed");
  }, [sidebarOpen]);

  useEffect(() => {
    window.localStorage.setItem("irecon.section", section);
  }, [section]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing) return;
      if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== "b") return;
      const target = event.target;
      if (
        target instanceof Element &&
        target.closest("input, textarea, select, [contenteditable='true']")
      ) {
        return;
      }
      event.preventDefault();
      toggleSidebar();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [toggleSidebar]);

  // The working project is shared by every page. Only the live source belongs
  // to this shell, because it is a capture detail rather than project state.
  useEffect(() => {
    let alive = true;
    if (projectId === null) {
      setLiveSources([]);
      setLiveSourceId(null);
      setLiveSource("Primary display");
      return () => {
        alive = false;
      };
    }
    Promise.all([
      getSetting("capture.source"),
      listSources(projectId).catch(() => []),
    ])
      .then(([savedSource, sources]) => {
        if (!alive) return;
        setLiveSources(sources);
        const sourceId = savedSource ? Number(savedSource) : null;
        const source = sources.find((entry) => entry.id === sourceId) ?? null;
        setLiveSourceId(source?.id ?? null);
        setLiveSource(source?.label ?? "Primary display");
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [projectId]);

  /** The picker writes the same setting Capture lab reads. */
  async function selectLiveSource(id: number | null) {
    const source = liveSources.find((entry) => entry.id === id) ?? null;
    setLiveSourceId(source?.id ?? null);
    setLiveSource(source?.label ?? "Primary display");
    await setSetting("capture.source", source ? String(source.id) : "");
  }

  async function toggleLive() {
    setLiveBusy(true);
    setLiveError(null);
    try {
      if (live) {
        await hideOverlay();
        await stopCapture();
        setLive(false);
      } else {
        await startCapture(projectId ?? undefined, false, liveSourceId);
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
    <SidebarContext.Provider
      value={{
        open: sidebarOpen,
        toggle: toggleSidebar,
        live,
        liveBusy,
        liveProject: project?.name ?? null,
        onToggleLive: toggleLive,
      }}
    >
      <div className="flex h-full min-h-0 bg-background text-foreground">
        {sidebarOpen ? (
          <AppSidebar
            section={section}
            onSelect={setSection}
            theme={theme}
            onToggleTheme={toggle}
            live={live}
            liveBusy={liveBusy}
            liveProject={project?.name ?? null}
            liveSource={liveSource}
            projects={projects}
            projectId={projectId}
            sources={liveSources}
            liveSourceId={liveSourceId}
            onSelectProject={selectProject}
            onSelectLiveSource={selectLiveSource}
            onToggleLive={toggleLive}
          />
        ) : null}
        <main id="main-content" className="flex min-h-0 min-w-0 flex-1 flex-col">
          {liveError ? (
            <p
              role="alert"
              className="border-b border-border p-2 text-sm text-destructive"
            >
              {liveError}
            </p>
          ) : null}
          {!isDesktop ? (
            <div className="border-b border-border bg-muted px-2.5 py-2 text-xs text-muted-foreground">
              Browser preview. Writes stay in memory and screen capture is unavailable.
            </div>
          ) : null}
          <div className="min-h-0 flex-1 overflow-y-auto">
            {section === "projects" ? <ProjectsPage /> : null}
            {section === "sources" ? <SourcesPage /> : null}
            {section === "library" ? <LibraryPage /> : null}
            {section === "review" ? <ReviewPage /> : null}
            {section === "label" ? <LabelPage /> : null}
            {section === "capture" ? <CaptureLabPage onRunningChange={setLive} /> : null}
            {section === "sessions" ? <SessionsPage /> : null}
            {section === "detectors" ? <DetectorsPage /> : null}
            {section === "eval" ? <EvalPage /> : null}
          </div>
        </main>
      </div>
    </SidebarContext.Provider>
  );
}

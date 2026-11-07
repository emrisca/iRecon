import { useState } from "react";

import { AppSidebar, type Section } from "./components/app-sidebar";
import { isDesktop } from "./lib/api";
import { useTheme } from "./hooks/use-theme";
import { CaptureLabPage } from "./pages/capture-lab";
import { LabelPage } from "./pages/label";
import { ProjectsPage } from "./pages/projects";

export default function App() {
  const [section, setSection] = useState<Section>("projects");
  const { theme, toggle } = useTheme();

  return (
    <div className="flex h-full min-h-0 bg-background text-foreground">
      <AppSidebar
        section={section}
        onSelect={setSection}
        theme={theme}
        onToggleTheme={toggle}
      />
      <main className="flex min-h-0 min-w-0 flex-1 flex-col">
        {!isDesktop ? (
          <div className="border-b border-border bg-muted px-2.5 py-2 text-xs text-muted-foreground">
            Browser preview. Writes stay in memory and screen capture is unavailable.
          </div>
        ) : null}
        <div className="min-h-0 flex-1 overflow-y-auto">
          {section === "projects" ? <ProjectsPage /> : null}
          {section === "label" ? <LabelPage /> : null}
          {section === "capture" ? <CaptureLabPage /> : null}
        </div>
      </main>
    </div>
  );
}

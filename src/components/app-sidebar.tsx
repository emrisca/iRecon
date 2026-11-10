import {
  Cpu,
  Crosshair,
  Eye,
  Film,
  FolderKanban,
  Gauge,
  Images,
  ListChecks,
  ScanLine,
  Tags,
  Waypoints,
  type LucideIcon,
} from "lucide-react";

import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Separator } from "./ui/separator";
import { SettingsMenu } from "./settings-menu";
import { cn } from "../lib/utils";
import type { Project, SourceView } from "../lib/types";

export type Section =
  | "projects"
  | "sources"
  | "library"
  | "review"
  | "label"
  | "capture"
  | "sessions"
  | "detectors"
  | "eval";

type Item = { id: Section; label: string; icon: LucideIcon };

const groups: Array<{ label: string; items: Item[] }> = [
  {
    label: "Set up",
    items: [
      { id: "projects", label: "Projects", icon: FolderKanban },
      { id: "sources", label: "Sources", icon: Waypoints },
      { id: "library", label: "Library", icon: Images },
    ],
  },
  {
    label: "Teach",
    items: [
      { id: "label", label: "Label", icon: Tags },
      { id: "review", label: "Review", icon: ListChecks },
    ],
  },
  {
    label: "Run",
    items: [
      { id: "capture", label: "Capture lab", icon: ScanLine },
      { id: "sessions", label: "Sessions", icon: Film },
      { id: "detectors", label: "Detectors", icon: Cpu },
      { id: "eval", label: "Evaluate", icon: Gauge },
    ],
  },
];

type SidebarProps = {
  section: Section;
  onSelect: (section: Section) => void;
  theme: "light" | "dark";
  onToggleTheme: () => void;
  live: boolean;
  liveBusy: boolean;
  liveProject: string | null;
  liveSource: string;
  projects: Project[];
  projectId: number | null;
  sources: SourceView[];
  liveSourceId: number | null;
  onSelectProject: (id: number) => void;
  onSelectLiveSource: (id: number | null) => void;
  onToggleLive: () => void;
};

function NavRow({
  item,
  active,
  onSelect,
}: {
  item: Item;
  active: boolean;
  onSelect: (section: Section) => void;
}) {
  const Icon = item.icon;
  return (
    <Button
      variant={active ? "secondary" : "ghost"}
      size="sm"
      className={cn("w-full justify-start gap-2", active && "font-medium")}
      aria-current={active ? "page" : undefined}
      onClick={() => onSelect(item.id)}
    >
      <Icon className="size-5" aria-hidden="true" />
      {item.label}
    </Button>
  );
}

export function AppSidebar({
  section,
  onSelect,
  theme,
  onToggleTheme,
  live,
  liveBusy,
  liveProject,
  liveSource,
  projects,
  projectId,
  sources,
  liveSourceId,
  onSelectProject,
  onSelectLiveSource,
  onToggleLive,
}: SidebarProps) {
  return (
    <aside
      id="app-sidebar"
      aria-label="Main navigation"
      className="flex w-56 shrink-0 flex-col border-r border-border bg-card"
    >
      <div className="flex items-center gap-2 p-2.5">
        <div className="flex size-8 shrink-0 items-center justify-center rounded-md bg-primary text-primary-foreground">
          <Crosshair className="size-4" aria-hidden="true" />
        </div>
        <span className="text-sm font-medium">iRecon</span>
      </div>

      <Separator />

      <div className="flex flex-col gap-1.5 border-b border-border p-2">
        <label htmlFor="working-project" className="text-xs text-muted-foreground">
          Project
        </label>
        <select
          id="working-project"
          className="h-8 w-full rounded-md border border-input bg-background px-2 text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
          value={projectId ?? ""}
          disabled={live || liveBusy || projects.length === 0}
          title={
            live
              ? "Stop Live view to change the working project"
              : "The project every page works in"
          }
          onChange={(event) => onSelectProject(Number(event.target.value))}
        >
          {projects.length === 0 ? <option value="">No project yet</option> : null}
          {projects.map((project) => (
            <option key={project.id} value={project.id}>
              {project.name}
            </option>
          ))}
        </select>
      </div>

      <nav
        className="flex min-h-0 flex-1 flex-col gap-2.5 overflow-y-auto p-2"
        aria-label="Sections"
      >
        {groups.map((group) => (
          <div key={group.label} className="flex flex-col gap-1.5">
            <p className="px-2 text-xs text-muted-foreground">{group.label}</p>
            {group.items.map((item) => (
              <NavRow
                key={item.id}
                item={item}
                active={section === item.id}
                onSelect={onSelect}
              />
            ))}
          </div>
        ))}
      </nav>

      <div className="flex flex-col gap-2 border-t border-border p-2">
        <Button
          variant={live ? "secondary" : "ghost"}
          size="sm"
          className="w-full justify-start gap-2"
          disabled={liveBusy}
          aria-pressed={live}
          title={
            liveProject
              ? `Live capture uses ${liveProject} from ${liveSource}. Change the source below.`
              : "Live capture has no project to match against"
          }
          onClick={onToggleLive}
        >
          <Eye className="size-5" aria-hidden="true" />
          Live view
          <Badge variant={live ? "default" : "outline"} className="ml-auto">
            {live ? "on" : "off"}
          </Badge>
        </Button>
        <div className="flex flex-col gap-1.5 rounded-md border border-border p-2">
          <label htmlFor="live-source" className="text-xs text-muted-foreground">
            Live source
          </label>
          <select
            id="live-source"
            className="h-8 w-full rounded-md border border-input bg-background px-2 text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
            value={liveSourceId ?? ""}
            disabled={live || liveBusy || projectId === null}
            title={
              live
                ? "Stop Live view to change the source"
                : "The display, window, file or folder Live view captures"
            }
            onChange={(event) =>
              onSelectLiveSource(
                event.target.value === "" ? null : Number(event.target.value)
              )
            }
          >
            <option value="">Primary display</option>
            {sources.map((source) => (
              <option key={source.id} value={source.id}>
                {source.label}
              </option>
            ))}
          </select>
        </div>
        <SettingsMenu theme={theme} onToggleTheme={onToggleTheme} />
      </div>
    </aside>
  );
}

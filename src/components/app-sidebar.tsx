import {
  Crosshair,
  Eye,
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

export type Section =
  | "projects"
  | "sources"
  | "library"
  | "review"
  | "label"
  | "capture"
  | "eval";

const items: Array<{ id: Section; label: string; icon: LucideIcon }> = [
  { id: "projects", label: "Projects", icon: FolderKanban },
  { id: "sources", label: "Sources", icon: Waypoints },
  { id: "library", label: "Library", icon: Images },
  { id: "review", label: "Review", icon: ListChecks },
  { id: "label", label: "Label", icon: Tags },
  { id: "capture", label: "Capture lab", icon: ScanLine },
  { id: "eval", label: "Evaluate", icon: Gauge },
];

type SidebarProps = {
  section: Section;
  onSelect: (section: Section) => void;
  theme: "light" | "dark";
  onToggleTheme: () => void;
  live: boolean;
  liveBusy: boolean;
  onToggleLive: () => void;
};

export function AppSidebar({
  section,
  onSelect,
  theme,
  onToggleTheme,
  live,
  liveBusy,
  onToggleLive,
}: SidebarProps) {
  return (
    <aside className="flex w-56 shrink-0 flex-col border-r border-border bg-card">
      <div className="flex items-center gap-2 p-2.5">
        <div className="flex size-8 shrink-0 items-center justify-center rounded-md bg-primary text-primary-foreground">
          <Crosshair className="size-4" aria-hidden="true" />
        </div>
        <div className="flex min-w-0 flex-col">
          <span className="text-sm font-medium leading-tight">iRecon</span>
          <span className="text-xs text-muted-foreground">local only</span>
        </div>
        <Badge variant="outline" className="ml-auto tabular-nums">
          v0.1
        </Badge>
      </div>

      <Separator />

      <nav className="flex flex-col gap-2 p-2" aria-label="Sections">
        {items.map((item) => {
          const active = section === item.id;
          return (
            <Button
              key={item.id}
              variant={active ? "secondary" : "ghost"}
              size="sm"
              className={cn("justify-start gap-2", active && "font-medium")}
              aria-current={active ? "page" : undefined}
              onClick={() => onSelect(item.id)}
            >
              <item.icon className="size-5" aria-hidden="true" />
              {item.label}
            </Button>
          );
        })}

        <Button
          variant={live ? "secondary" : "ghost"}
          size="sm"
          className="justify-start gap-2"
          disabled={liveBusy}
          aria-pressed={live}
          onClick={onToggleLive}
        >
          <Eye className="size-5" aria-hidden="true" />
          Live view
          <Badge variant={live ? "default" : "outline"} className="ml-auto">
            {live ? "on" : "off"}
          </Badge>
        </Button>
      </nav>

      <div className="mt-auto flex flex-col gap-2 border-t border-border p-2">
        <SettingsMenu theme={theme} onToggleTheme={onToggleTheme} />
        <p className="px-2 text-xs text-muted-foreground">
          Frames stay on this machine.
        </p>
      </div>
    </aside>
  );
}

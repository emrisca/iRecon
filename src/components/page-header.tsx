import { PanelLeftClose, PanelLeftOpen } from "lucide-react";

import { Button } from "./ui/button";
import { useSidebar } from "./sidebar-context";

type PageHeaderProps = {
  title: string;
  description: string;
  action?: React.ReactNode;
};

export function PageHeader({ title, description, action }: PageHeaderProps) {
  const { open, toggle, live, liveBusy, liveProject, onToggleLive } = useSidebar();

  return (
    <div className="flex flex-wrap items-start justify-between gap-2.5 border-b border-border p-2.5">
      <div className="flex min-w-0 items-start gap-2">
        <Button
          type="button"
          size="icon"
          variant="ghost"
          className="shrink-0"
          aria-label={open ? "Close sidebar" : "Open sidebar"}
          aria-expanded={open}
          aria-controls="app-sidebar"
          aria-keyshortcuts="Control+B"
          title={open ? "Close sidebar (Ctrl+B)" : "Open sidebar (Ctrl+B)"}
          onClick={toggle}
        >
          {open ? (
            <PanelLeftClose className="size-4" aria-hidden="true" />
          ) : (
            <PanelLeftOpen className="size-4" aria-hidden="true" />
          )}
        </Button>
        <div className="flex min-w-0 flex-col gap-1.5">
          <h1 className="text-xl font-medium tracking-tight">{title}</h1>
          <p className="max-w-[65ch] text-sm text-muted-foreground">{description}</p>
        </div>
      </div>
      {action || (!open && live) ? (
        <div className="flex flex-wrap items-center gap-2">
          {!open && live ? (
            <Button
              size="sm"
              variant="outline"
              className="gap-2"
              disabled={liveBusy}
              aria-label={
                liveProject ? `Stop live capture for ${liveProject}` : "Stop live capture"
              }
              title={
                liveProject ? `Stop live capture for ${liveProject}` : "Stop live capture"
              }
              onClick={onToggleLive}
            >
              <span className="size-2 rounded-full bg-primary" aria-hidden="true" />
              Live
            </Button>
          ) : null}
          {action}
        </div>
      ) : null}
    </div>
  );
}

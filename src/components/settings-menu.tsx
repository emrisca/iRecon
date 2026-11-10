import { useEffect, useState } from "react";
import { Moon, RefreshCw, Settings, Sun } from "lucide-react";

import { Button } from "./ui/button";
import { isDesktop } from "../lib/api";

type Status =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "current" }
  | { kind: "available"; version: string; notes: string }
  | { kind: "downloading"; progress: number }
  | { kind: "restarting" }
  | { kind: "error"; message: string };

type Props = {
  theme: "light" | "dark";
  onToggleTheme: () => void;
};

/**
 * The bottom-left control row: settings on one side, light and dark on the
 * other. Update checks live in the panel, because that is the one screen a user
 * visits when they are explicitly looking for it.
 */
export function SettingsMenu({ theme, onToggleTheme }: Props) {
  const [open, setOpen] = useState(false);
  const [version, setVersion] = useState<string | null>(null);
  const [status, setStatus] = useState<Status>({ kind: "idle" });
  const [update, setUpdate] = useState<{ install: () => Promise<void> } | null>(null);

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open]);

  useEffect(() => {
    if (!isDesktop) return;
    let alive = true;
    import("@tauri-apps/api/app")
      .then((module) => module.getVersion())
      .then((value) => {
        if (alive) setVersion(value);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  async function checkNow() {
    if (!isDesktop) {
      setStatus({ kind: "error", message: "Update checks need the desktop app." });
      return;
    }
    setStatus({ kind: "checking" });
    setUpdate(null);
    try {
      const { check } = await import("@tauri-apps/plugin-updater");
      const found = await check();
      if (!found) {
        setStatus({ kind: "current" });
        return;
      }
      setUpdate({
        install: async () => {
          setStatus({ kind: "downloading", progress: 0 });
          let total = 0;
          let received = 0;
          await found.downloadAndInstall((event) => {
            if (event.event === "Started") {
              total = event.data.contentLength ?? 0;
            } else if (event.event === "Progress") {
              received += event.data.chunkLength;
              if (total > 0) {
                setStatus({ kind: "downloading", progress: received / total });
              }
            } else {
              setStatus({ kind: "restarting" });
            }
          });
          const { relaunch } = await import("@tauri-apps/plugin-process");
          await relaunch();
        },
      });
      setStatus({
        kind: "available",
        version: found.version,
        notes: found.body ?? "",
      });
    } catch (reason) {
      setStatus({ kind: "error", message: String(reason) });
    }
  }

  async function install() {
    if (!update) return;
    try {
      await update.install();
    } catch (reason) {
      setStatus({ kind: "error", message: String(reason) });
    }
  }

  return (
    <div className="relative flex items-center gap-1.5">
      <Button
        variant="ghost"
        size="sm"
        className="flex-1 justify-start gap-2"
        aria-expanded={open}
        aria-haspopup="dialog"
        aria-controls="settings-panel"
        onClick={() => setOpen((current) => !current)}
      >
        <Settings className="size-4" aria-hidden="true" />
        Settings
        {status.kind === "available" ? (
          <span className="ml-auto size-1.5 rounded-full bg-primary" aria-label="Update available" />
        ) : null}
      </Button>
      <Button
        variant="ghost"
        size="icon"
        aria-label={theme === "dark" ? "Switch to light mode" : "Switch to dark mode"}
        title={theme === "dark" ? "Switch to light mode" : "Switch to dark mode"}
        onClick={onToggleTheme}
      >
        {theme === "dark" ? (
          <Moon className="size-4" aria-hidden="true" />
        ) : (
          <Sun className="size-4" aria-hidden="true" />
        )}
      </Button>

      {open ? (
        <div
          id="settings-panel"
          role="dialog"
          aria-label="Settings"
          className="absolute bottom-full left-0 z-20 mb-2 flex w-52 flex-col gap-2 rounded-md border border-border bg-card p-2 shadow-lg"
        >
          <div className="flex items-center justify-between gap-2 px-1.5">
            <span className="text-sm font-medium">Settings</span>
            {version ? (
              <span className="text-xs text-muted-foreground tabular-nums">v{version}</span>
            ) : null}
          </div>

          <p className="px-1.5 text-xs text-muted-foreground">
            Updates are signed and installed in place. Nothing else leaves this machine.
          </p>

          <Button
            size="sm"
            variant="outline"
            className="justify-start gap-2"
            disabled={status.kind === "checking" || status.kind === "downloading"}
            onClick={checkNow}
          >
            <RefreshCw
              className={status.kind === "checking" ? "size-4 animate-spin" : "size-4"}
              aria-hidden="true"
            />
            {status.kind === "checking" ? "Checking" : "Check for updates"}
          </Button>

          {status.kind === "current" ? (
            <p className="px-1.5 text-xs text-muted-foreground">This is the latest version.</p>
          ) : null}
          {status.kind === "available" ? (
            <div className="flex flex-col gap-1.5">
              <p className="px-1.5 text-xs">
                Version {status.version} is available.
              </p>
              {status.notes ? (
                <p className="max-h-24 overflow-y-auto whitespace-pre-wrap px-1.5 text-xs text-muted-foreground">
                  {status.notes}
                </p>
              ) : null}
              <Button size="sm" className="gap-2" onClick={install}>
                Download and restart
              </Button>
            </div>
          ) : null}
          {status.kind === "downloading" ? (
            <div className="flex flex-col gap-1.5">
              <div className="h-1.5 overflow-hidden rounded-full bg-muted">
                <div
                  className="h-full bg-primary transition-[width]"
                  style={{ width: `${Math.round(status.progress * 100)}%` }}
                />
              </div>
              <p className="px-1.5 text-xs text-muted-foreground tabular-nums">
                Downloading {Math.round(status.progress * 100)}%
              </p>
            </div>
          ) : null}
          {status.kind === "restarting" ? (
            <p className="px-1.5 text-xs text-muted-foreground">Installed. Restarting.</p>
          ) : null}
          {status.kind === "error" ? (
            <p role="alert" className="px-1.5 text-xs text-destructive">
              {status.message}
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

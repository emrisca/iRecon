import { useEffect, useState } from "react";
import { Play, Square } from "lucide-react";

import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../components/ui/card";
import { Separator } from "../components/ui/separator";
import { Skeleton } from "../components/ui/skeleton";
import { PageHeader } from "../components/page-header";
import {
  hideOverlay,
  isDesktop,
  onFrame,
  onStats,
  showOverlay,
  startCapture,
  stopCapture,
} from "../lib/api";
import type { CaptureStats, FramePayload } from "../lib/types";

function metric(value: string, label: string) {
  return (
    <div className="flex flex-col gap-1.5 rounded-lg border border-border bg-card p-2">
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className="text-2xl font-medium tabular-nums">{value}</span>
    </div>
  );
}

export function CaptureLabPage() {
  const [stats, setStats] = useState<CaptureStats | null>(null);
  const [frame, setFrame] = useState<FramePayload | null>(null);
  const [running, setRunning] = useState(false);
  const [overlay, setOverlay] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    const cleanups: Array<() => void> = [];

    void (async () => {
      const offStats = await onStats((next) => {
        if (!alive) return;
        setStats(next);
        setRunning(next.running);
      });
      const offFrame = await onFrame((next) => {
        if (alive) setFrame(next);
      });
      cleanups.push(offStats, offFrame);
    })();

    return () => {
      alive = false;
      for (const cleanup of cleanups) cleanup();
    };
  }, []);

  async function toggleCapture() {
    setBusy(true);
    setError(null);
    try {
      if (running) {
        await stopCapture();
        setRunning(false);
      } else {
        await startCapture();
        setRunning(true);
      }
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function toggleOverlay() {
    setBusy(true);
    setError(null);
    try {
      if (overlay) {
        await hideOverlay();
        setOverlay(false);
      } else {
        await showOverlay();
        setOverlay(true);
      }
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  const resolution = stats && stats.width > 0 ? `${stats.width} by ${stats.height}` : "none";

  return (
    <div className="flex min-h-full flex-col">
      <PageHeader
        title="Capture lab"
        description="Phase 0 spike. It measures how fast this machine can grab the screen and proves the overlay window draws on top, before any detector exists."
        action={
          <Button
            size="sm"
            variant={running ? "outline" : "default"}
            className="gap-1.5"
            disabled={busy}
            onClick={toggleCapture}
          >
            {running ? (
              <Square className="size-4" aria-hidden="true" />
            ) : (
              <Play className="size-4" aria-hidden="true" />
            )}
            {running ? "Stop capture" : "Start capture"}
          </Button>
        }
      />

      {!isDesktop ? (
        <p className="border-b border-border p-2 text-sm text-muted-foreground">
          Capture is disabled in the browser preview. Open the desktop app to measure it.
        </p>
      ) : null}

      {error ? (
        <p role="alert" className="border-b border-border p-2 text-sm text-destructive">
          {error}
        </p>
      ) : null}

      <div className="flex flex-col gap-2.5 p-2.5">
        <div className="grid grid-cols-2 gap-2 lg:grid-cols-4">
          {metric(stats ? stats.fps.toFixed(1) : "0.0", "Frames per second")}
          {metric(stats ? `${stats.captureMs.toFixed(1)} ms` : "0 ms", "Cost per frame")}
          {metric(stats ? String(stats.frames) : "0", "Frames captured")}
          {metric(resolution, "Display")}
        </div>

        <Card>
          <CardHeader className="flex flex-row items-start justify-between gap-2">
            <div className="flex flex-col gap-1.5">
              <CardTitle>Latest frame</CardTitle>
              <CardDescription>
                Downsampled to 640 px so the preview stays cheap. The spike still captures
                the full display.
              </CardDescription>
            </div>
            <Badge variant="outline" className="tabular-nums">
              {frame ? `${frame.width} by ${frame.height}` : "waiting"}
            </Badge>
          </CardHeader>
          <CardContent>
            {frame ? (
              <img
                src={frame.data}
                alt="Latest captured frame"
                className="w-full rounded-md border border-border"
              />
            ) : (
              <Skeleton className="aspect-video w-full" />
            )}
            {stats?.note ? (
              <p className="text-xs text-muted-foreground">{stats.note}</p>
            ) : null}
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="flex flex-row items-start justify-between gap-2">
            <div className="flex flex-col gap-1.5">
              <CardTitle>Overlay</CardTitle>
              <CardDescription>
                A transparent window over the primary display. It ignores the mouse, so
                boxes never eat a click meant for the app underneath.
              </CardDescription>
            </div>
            <Badge variant={overlay ? "default" : "outline"}>
              {overlay ? "visible" : "hidden"}
            </Badge>
          </CardHeader>
          <CardContent>
            <div className="flex items-center gap-2">
              <Button size="sm" variant="outline" disabled={busy} onClick={toggleOverlay}>
                {overlay ? "Hide overlay" : "Show overlay"}
              </Button>
              <span className="text-xs text-muted-foreground">
                Three sample boxes move across the screen.
              </span>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>What this build does not do yet</CardTitle>
            <CardDescription>
              The boxes are an animation. Nothing is detected, nothing is stored beyond the
              project row, and the monitor is always the primary one.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <Separator />
            <ul className="flex flex-col gap-2 text-sm">
              <li className="flex items-start gap-2">
                <span className="mt-[7px] size-1 shrink-0 rounded-full bg-muted-foreground" />
                <span>
                  macOS asks for Screen Recording the first time capture runs. Grant it to
                  iRecon in System Settings, then restart the app or the frame stays black.
                </span>
              </li>
              <li className="flex items-start gap-2">
                <span className="mt-[7px] size-1 shrink-0 rounded-full bg-muted-foreground" />
                <span>
                  An exclusive fullscreen game owns the display, so the overlay cannot
                  appear over it. Borderless windowed is the supported mode.
                </span>
              </li>
              <li className="flex items-start gap-2">
                <span className="mt-[7px] size-1 shrink-0 rounded-full bg-muted-foreground" />
                <span>
                  Windows captures through Windows Graphics Capture, macOS through
                  ScreenCaptureKit. Both come from the same crate, so the spike runs on
                  either platform.
                </span>
              </li>
            </ul>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}

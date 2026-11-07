import { useEffect, useState } from "react";
import { Download, Play, Square } from "lucide-react";

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
  downloadModel,
  getModelStatus,
  hideOverlay,
  isDesktop,
  loadModel,
  onDetections,
  onFrame,
  onModelProgress,
  onStats,
  showOverlay,
  startCapture,
  stopCapture,
} from "../lib/api";
import type {
  CaptureStats,
  DetectionPayload,
  FramePayload,
  ModelProgress,
  ModelStatus,
} from "../lib/types";

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
  const [detections, setDetections] = useState<DetectionPayload | null>(null);
  const [running, setRunning] = useState(false);
  const [overlay, setOverlay] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [model, setModel] = useState<ModelStatus | null>(null);
  const [progress, setProgress] = useState<ModelProgress | null>(null);

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
      const offDetections = await onDetections((next) => {
        if (alive) setDetections(next);
      });
      const offModel = await onModelProgress((next) => {
        if (!alive) return;
        setProgress(next);
        if (next.phase === "ready" || next.phase === "loaded" || next.phase === "failed") {
          void getModelStatus().then((status) => {
            if (alive) setModel(status);
          });
        }
      });
      cleanups.push(offStats, offFrame, offDetections, offModel);
      const status = await getModelStatus();
      if (alive) setModel(status);
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

  async function startDownload() {
    setBusy(true);
    setError(null);
    try {
      await downloadModel();
      setProgress({
        downloaded: 0,
        total: model?.expectedBytes ?? 0,
        phase: "downloading",
        message: "Starting the download",
      });
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function load() {
    setBusy(true);
    setError(null);
    try {
      await loadModel();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  const resolution = stats && stats.width > 0 ? `${stats.width} by ${stats.height}` : "none";
  const megabytes = model ? (model.bytes / 1_000_000).toFixed(0) : "0";
  const expected = model ? (model.expectedBytes / 1_000_000).toFixed(0) : "115";
  const percent =
    progress && progress.total > 0
      ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100))
      : 0;

  return (
    <div className="flex min-h-full flex-col">
      <PageHeader
        title="Capture lab"
        description="Screen capture, the detector and the overlay in one place. Frames per second and inference cost here are the numbers the plan asks for."
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
          Capture and detection are disabled in the browser preview. Open the desktop app.
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
          {metric(stats ? `${stats.captureMs.toFixed(1)} ms` : "0 ms", "Capture cost")}
          {metric(
            detections ? `${detections.inferenceMs.toFixed(1)} ms` : "0 ms",
            "Inference cost"
          )}
          {metric(detections ? String(detections.boxes.length) : "0", "Detections")}
        </div>

        <Card>
          <CardHeader className="flex flex-row items-start justify-between gap-2">
            <div className="flex flex-col gap-1.5">
              <CardTitle>Latest frame</CardTitle>
              <CardDescription>
                Downsampled to 640 px for the preview. Detection runs on the full frame.
              </CardDescription>
            </div>
            <Badge variant="outline" className="tabular-nums">
              {frame ? `${frame.width} by ${frame.height}` : "waiting"}
            </Badge>
          </CardHeader>
          <CardContent>
            {frame ? (
              <div className="relative overflow-hidden rounded-md border border-border">
                <img src={frame.data} alt="Latest captured frame" className="w-full" />
                {detections?.boxes.map((box, index) => (
                  <div
                    key={`${box.label}-${index}`}
                    className="absolute border border-white ring-1 ring-black/60"
                    style={{
                      left: `${box.x * 100}%`,
                      top: `${box.y * 100}%`,
                      width: `${box.w * 100}%`,
                      height: `${box.h * 100}%`,
                    }}
                  >
                    <span className="absolute left-0 top-0 bg-black/70 px-1.5 text-xs text-white">
                      {box.label} {box.score.toFixed(2)}
                    </span>
                  </div>
                ))}
              </div>
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
              <CardTitle>Model</CardTitle>
              <CardDescription>
                RF-DETR Nano, Apache 2.0. Downloaded once, checked against a pinned
                SHA-256 on every load, and never uploaded anywhere.
              </CardDescription>
            </div>
            <Badge variant={model?.loaded ? "default" : "outline"}>
              {model?.loaded ? "loaded" : model?.installed ? "on disk" : "missing"}
            </Badge>
          </CardHeader>
          <CardContent>
            <div className="flex flex-wrap items-center gap-2">
              {!model?.installed ? (
                <Button size="sm" className="gap-1.5" disabled={busy} onClick={startDownload}>
                  <Download className="size-4" aria-hidden="true" />
                  Download model
                </Button>
              ) : (
                <Button size="sm" variant="outline" disabled={busy || model.loaded} onClick={load}>
                  Load model
                </Button>
              )}
              <span className="text-xs text-muted-foreground tabular-nums">
                {model?.installed ? `${megabytes} MB on disk` : `${expected} MB download`}
              </span>
              {model?.installed ? (
                <Badge variant={model.verified ? "secondary" : "destructive"}>
                  {model.verified ? "hash verified" : "hash mismatch"}
                </Badge>
              ) : null}
            </div>

            {progress && progress.phase === "downloading" ? (
              <div className="flex flex-col gap-1.5">
                <div className="h-1 w-full overflow-hidden rounded-sm bg-muted">
                  <div className="h-full bg-primary" style={{ width: `${percent}%` }} />
                </div>
                <span className="text-xs text-muted-foreground tabular-nums">
                  {percent}% of {expected} MB
                </span>
              </div>
            ) : null}

            {progress && progress.message ? (
              <p className="text-xs text-muted-foreground">{progress.message}</p>
            ) : null}

            {model ? (
              <p className="break-all text-xs text-muted-foreground">{model.path}</p>
            ) : null}
            {model?.detail ? (
              <p className="text-xs text-destructive">{model.detail}</p>
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
                Draws the last detections. Sample boxes appear only when none arrive.
              </span>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>What this build does not do yet</CardTitle>
            <CardDescription>
              Boxes are the shipped detector's proposals only. Nothing is matched to your
              own examples yet, and the capture source is always the primary display.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <Separator />
            <ul className="flex flex-col gap-2 text-sm">
              <li className="flex items-start gap-2">
                <span className="mt-2 size-1 shrink-0 rounded-full bg-muted-foreground" />
                <span>
                  macOS asks for Screen Recording the first time capture runs. Grant it to
                  iRecon in System Settings, then restart the app or the frame stays black.
                </span>
              </li>
              <li className="flex items-start gap-2">
                <span className="mt-2 size-1 shrink-0 rounded-full bg-muted-foreground" />
                <span>
                  An exclusive fullscreen game owns the display, so the overlay cannot
                  appear over it. Borderless windowed is the supported mode.
                </span>
              </li>
              <li className="flex items-start gap-2">
                <span className="mt-2 size-1 shrink-0 rounded-full bg-muted-foreground" />
                <span>
                  Detection runs every frame on the CPU by default. The inference number
                  above is the real cost on this machine, which is what Phase 0 asked for.
                </span>
              </li>
            </ul>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}

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
  getModelCatalog,
  hideOverlay,
  isDesktop,
  listProjects,
  loadModel,
  onDetections,
  onFrame,
  onModelProgress,
  onStats,
  showOverlay,
  startCapture,
  stopCapture,
  watchCapture,
} from "../lib/api";
import type {
  CaptureStats,
  DetectionPayload,
  FramePayload,
  ModelCatalog,
  ModelProgress,
  Project,
} from "../lib/types";

function metric(value: string, label: string) {
  return (
    <div className="flex flex-col gap-1.5 rounded-lg border border-border bg-card p-2">
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className="text-2xl font-medium tabular-nums">{value}</span>
    </div>
  );
}

function megabytes(bytes: number) {
  return (bytes / 1_000_000).toFixed(0);
}

export function CaptureLabPage() {
  const [stats, setStats] = useState<CaptureStats | null>(null);
  const [frame, setFrame] = useState<FramePayload | null>(null);
  const [detections, setDetections] = useState<DetectionPayload | null>(null);
  const [running, setRunning] = useState(false);
  const [overlay, setOverlay] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [catalog, setCatalog] = useState<ModelCatalog | null>(null);
  const [progress, setProgress] = useState<ModelProgress | null>(null);
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectId, setProjectId] = useState<number | null>(null);

  useEffect(() => {
    let alive = true;
    const cleanups: Array<() => void> = [];
    // Frames are only encoded while this page can show them.
    void watchCapture(true);

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
          void getModelCatalog().then((status) => {
            if (alive) setCatalog(status);
          });
        }
      });
      cleanups.push(offStats, offFrame, offDetections, offModel);
      const status = await getModelCatalog();
      if (alive) setCatalog(status);
    })();

    return () => {
      alive = false;
      void watchCapture(false);
      for (const cleanup of cleanups) cleanup();
    };
  }, []);

  useEffect(() => {
    let alive = true;
    listProjects()
      .then((rows) => {
        if (!alive) return;
        setProjects(rows);
        setProjectId((current) => current ?? rows[0]?.id ?? null);
      })
      .catch(() => {});
    return () => {
      alive = false;
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
        await startCapture(projectId ?? undefined);
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

  async function startDownload(id: string) {
    setBusy(true);
    setError(null);
    try {
      await downloadModel(id);
      setProgress({
        id,
        downloaded: 0,
        total: 0,
        phase: "downloading",
        message: "",
      });
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function load(id: string) {
    setBusy(true);
    setError(null);
    try {
      await loadModel(id);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  const resolution = stats && stats.width > 0 ? `${stats.width} by ${stats.height}` : "none";
  const activeName = catalog?.models.find((entry) => entry.active)?.name;

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
            disabled={busy || (!running && !projectId)}
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

      <div className="flex flex-wrap items-center gap-2 border-b border-border p-2.5">
        <span className="text-xs text-muted-foreground">Project</span>
        {projects.length === 0 ? (
          <span className="text-sm text-muted-foreground">
            Create a project on the Projects page so boxes can be matched.
          </span>
        ) : (
          projects.map((project) => (
            <Button
              key={project.id}
              size="sm"
              variant={project.id === projectId ? "secondary" : "ghost"}
              disabled={running}
              onClick={() => setProjectId(project.id)}
            >
              {project.name}
            </Button>
          ))
        )}
      </div>

      <div className="flex flex-col gap-2.5 p-2.5">
        <div className="grid grid-cols-2 gap-2 lg:grid-cols-5">
          {metric(stats ? stats.fps.toFixed(1) : "0.0", "Frames per second")}
          {metric(stats ? `${stats.captureMs.toFixed(1)} ms` : "0 ms", "Capture cost")}
          {metric(
            detections ? `${detections.inferenceMs.toFixed(1)} ms` : "0 ms",
            "Inference cost"
          )}
          {metric(stats ? `${stats.matchMs.toFixed(1)} ms` : "0 ms", "Matcher cost")}
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
            <div className="flex flex-wrap items-center gap-1.5">
              <Badge variant="outline" className="tabular-nums">
                {frame ? `${frame.width} by ${frame.height}` : "waiting"}
              </Badge>
              <Badge variant="outline" className="tabular-nums">
                {stats ? `${stats.poolClasses} classes, ${stats.poolExamples} examples` : "no pool"}
              </Badge>
            </div>
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
              <CardTitle>Models</CardTitle>
              <CardDescription>
                Two RF-DETR sizes, both Apache 2.0. A download is checked against a
                pinned SHA-256 before it is loaded, stays on this machine, and is never
                uploaded anywhere.
              </CardDescription>
            </div>
            <Badge variant={activeName ? "default" : "outline"}>
              {activeName ?? "none loaded"}
            </Badge>
          </CardHeader>
          <CardContent className="flex flex-col gap-2">
            {!isDesktop ? (
              <p className="text-sm text-muted-foreground">
                Model downloads need the desktop app.
              </p>
            ) : null}

            {catalog ? (
              catalog.models.map((entry) => {
                const rowProgress = progress?.id === entry.id ? progress : null;
                const rowPercent =
                  rowProgress && rowProgress.total > 0
                    ? Math.min(
                        100,
                        Math.round((rowProgress.downloaded / rowProgress.total) * 100)
                      )
                    : 0;
                return (
                  <div
                    key={entry.id}
                    className="flex flex-col gap-1.5 rounded-lg border border-border p-2"
                  >
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-sm font-medium">{entry.name}</span>
                      <Badge variant={entry.active ? "default" : "outline"}>
                        {entry.active
                          ? "loaded"
                          : entry.verified
                            ? "on disk"
                            : entry.installed
                              ? "hash mismatch"
                              : "missing"}
                      </Badge>
                      <span className="text-xs text-muted-foreground tabular-nums">
                        {entry.inputSize} px,{" "}
                        {entry.installed
                          ? `${megabytes(entry.bytes)} MB on disk`
                          : `${megabytes(entry.expectedBytes)} MB download`}
                      </span>
                      <div className="ml-auto">
                        {entry.verified ? (
                          <Button
                            size="sm"
                            variant="outline"
                            disabled={busy || entry.active}
                            onClick={() => load(entry.id)}
                          >
                            Load
                          </Button>
                        ) : (
                          <Button
                            size="sm"
                            className="gap-1.5"
                            disabled={busy}
                            onClick={() => startDownload(entry.id)}
                          >
                            <Download className="size-4" aria-hidden="true" />
                            Download
                          </Button>
                        )}
                      </div>
                    </div>
                    <span className="text-xs text-muted-foreground">{entry.note}</span>
                    {rowProgress && rowProgress.phase === "downloading" ? (
                      <div className="flex flex-col gap-1.5">
                        <div className="h-1 w-full overflow-hidden rounded-sm bg-muted">
                          <div
                            className="h-full bg-primary"
                            style={{ width: `${rowPercent}%` }}
                          />
                        </div>
                        <span className="text-xs text-muted-foreground tabular-nums">
                          {rowPercent}% of {megabytes(rowProgress.total)} MB
                        </span>
                      </div>
                    ) : null}
                    {entry.detail ? (
                      <span className="text-xs text-destructive">{entry.detail}</span>
                    ) : null}
                  </div>
                );
              })
            ) : isDesktop ? (
              <Skeleton className="h-16 w-full" />
            ) : null}

            {progress && progress.message ? (
              <p className="text-xs text-muted-foreground">{progress.message}</p>
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
              Boxes are the loaded model's proposals, kept only when a class in the
              selected project scores above that class's threshold. Each box keeps a track
              id while it overlaps from frame to frame, and a single missed frame no longer
              blanks it. The capture source is always the primary display.
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
                  above is the real cost on this machine, and it is the number to compare
                  between the two model sizes.
                </span>
              </li>
            </ul>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}

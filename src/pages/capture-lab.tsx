import { useEffect, useState } from "react";
import { Download, Play, RotateCcw, Square } from "lucide-react";

import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../components/ui/card";
import { Skeleton } from "../components/ui/skeleton";
import { PageHeader } from "../components/page-header";
import { useProject } from "../components/project-context";
import { useSidebar } from "../components/sidebar-context";
import {
  downloadModel,
  getModelCatalog,
  getOverlayState,
  getSetting,
  hideOverlay,
  isDesktop,
  listSources,
  loadModel,
  onDetections,
  onFrame,
  onModelProgress,
  onPaint,
  onStats,
  overlaySmoothing,
  projectMatchFloor,
  setCalibration,
  setProjectMatchFloor,
  setSetting,
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
  OverlayState,
  PaintStats,
  SourceView,
} from "../lib/types";

function Metric({ value, label }: { value: string; label: string }) {
  return (
    <div className="flex flex-col gap-1.5 rounded-md bg-muted p-2">
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className="text-2xl font-medium tabular-nums">{value}</span>
    </div>
  );
}

function megabytes(bytes: number) {
  return (bytes / 1_000_000).toFixed(0);
}

export function CaptureLabPage({
  onRunningChange,
}: {
  onRunningChange?: (running: boolean) => void;
} = {}) {
  const [stats, setStats] = useState<CaptureStats | null>(null);
  const [frame, setFrame] = useState<FramePayload | null>(null);
  const [detections, setDetections] = useState<DetectionPayload | null>(null);
  const [running, setRunning] = useState(false);
  const [overlay, setOverlay] = useState<OverlayState | null>(null);
  const [paint, setPaint] = useState<PaintStats | null>(null);
  const [busy, setBusy] = useState(false);
  const [record, setRecord] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [catalog, setCatalog] = useState<ModelCatalog | null>(null);
  const [progress, setProgress] = useState<ModelProgress | null>(null);
  const { projects, projectId, selectProject } = useProject();
  const { live } = useSidebar();
  const [sources, setSources] = useState<SourceView[]>([]);
  const [sourceId, setSourceId] = useState<number | null>(null);
  const [floor, setFloor] = useState(0);
  const [harvestCap, setHarvestCap] = useState(40);
  const [smoothMs, setSmoothMs] = useState(55);
  const [exampleCap, setExampleCap] = useState(400);
  const [settingsReady, setSettingsReady] = useState(!isDesktop);
  const [sourcesReady, setSourcesReady] = useState(!isDesktop);
  const [savedSourceId, setSavedSourceId] = useState<number | null>(null);

  useEffect(() => {
    if (!isDesktop) {
      setSettingsReady(true);
      return;
    }
    let alive = true;
    getSetting("capture.source")
      .then((source) => {
        if (!alive) return;
        const sourceId = source ? Number(source) : null;
        setSavedSourceId(Number.isFinite(sourceId) ? sourceId : null);
      })
      .catch(() => {})
      .finally(() => {
        if (alive) setSettingsReady(true);
      });
    overlaySmoothing()
      .then((value) => {
        if (alive && Number.isFinite(value)) setSmoothMs(Math.min(200, Math.max(0, Math.round(value))));
      })
      .catch(() => {});
    getSetting("matcher.max_examples")
      .then((value) => {
        if (!alive || value === null || value.trim() === "") return;
        const parsed = Number(value);
        if (Number.isFinite(parsed) && parsed >= 0) {
          setExampleCap(Math.min(100000, Math.round(parsed)));
        }
      })
      .catch(() => {});
    getSetting("capture.harvest_cap")
      .then((value) => {
        if (!alive || value === null || value.trim() === "") return;
        const parsed = Number(value);
        if (Number.isFinite(parsed)) {
          setHarvestCap(Math.min(400, Math.max(0, Math.round(parsed))));
        }
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

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
        onRunningChange?.(next.running);
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
      const offPaint = await onPaint((next) => {
        if (alive) setPaint(next);
      });
      cleanups.push(offStats, offFrame, offDetections, offModel, offPaint);
      const status = await getModelCatalog();
      if (alive) setCatalog(status);
      const overlayState = await getOverlayState();
      if (alive) setOverlay(overlayState);
    })();

    return () => {
      alive = false;
      void watchCapture(false);
      for (const cleanup of cleanups) cleanup();
    };
  }, []);

  // Saved sources are optional. Without one, capture reads the primary display,
  // which is what it did before sources could drive the live loop.
  useEffect(() => {
    let alive = true;
    if (projectId == null) {
      setSources([]);
      setSourceId(null);
      setSourcesReady(true);
      return () => {
        alive = false;
      };
    }
    setSourcesReady(false);
    listSources(projectId)
      .then((rows) => {
        if (!alive) return;
        setSources(rows);
        setSourceId((current) => {
          if (current !== null && rows.some((row) => row.id === current)) return current;
          if (savedSourceId !== null && rows.some((row) => row.id === savedSourceId)) {
            return savedSourceId;
          }
          return null;
        });
      })
      .catch(() => {})
      .finally(() => {
        if (alive) setSourcesReady(true);
      });
    return () => {
      alive = false;
    };
  }, [projectId, savedSourceId]);

  useEffect(() => {
    if (!isDesktop || !settingsReady || !sourcesReady || projectId == null) return;
    void setSetting("capture.source", sourceId === null ? "" : String(sourceId));
  }, [projectId, settingsReady, sourceId, sourcesReady]);

  useEffect(() => {
    let alive = true;
    if (projectId == null) {
      setFloor(0);
      return () => {
        alive = false;
      };
    }
    projectMatchFloor(projectId)
      .then((value) => {
        if (alive) setFloor(value);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [projectId]);

  async function toggleCapture() {
    setBusy(true);
    setError(null);
    try {
      if (running) {
        await stopCapture();
        setRunning(false);
        onRunningChange?.(false);
      } else {
        await startCapture(projectId ?? undefined, record, sourceId);
        setRunning(true);
        onRunningChange?.(true);
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
      const next = overlay?.visible ? await hideOverlay() : await showOverlay();
      setOverlay(next);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function toggleCalibration() {
    setBusy(true);
    setError(null);
    try {
      const next = await setCalibration(!overlay?.calibration);
      setOverlay(next);
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

  async function changeSmoothing(value: number) {
    const next = Math.min(200, Math.max(0, Math.round(value)));
    setSmoothMs(next);
    try {
      await overlaySmoothing(next);
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function changeExampleCap(value: number) {
    const next = Math.min(100000, Math.max(0, Math.round(value)));
    setExampleCap(next);
    try {
      await setSetting("matcher.max_examples", String(next));
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function changeHarvestCap(value: number) {
    const next = Math.min(400, Math.max(0, Math.round(value)));
    setHarvestCap(next);
    try {
      await setSetting("capture.harvest_cap", String(next));
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function changeFloor(percent: number) {
    const next = Math.min(100, Math.max(0, Math.round(percent)));
    setFloor(next / 100);
    if (projectId == null) return;
    try {
      const saved = await setProjectMatchFloor(projectId, next / 100);
      setFloor(saved);
    } catch (reason) {
      setError(String(reason));
    }
  }

  const activeName = catalog?.models.find((entry) => entry.active)?.name;

  return (
    <div className="flex min-h-full flex-col">
      <PageHeader
        title="Capture lab"
        description="Screen capture, the detector and the overlay. Every number here is measured on this machine."
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

      <div className="flex flex-col gap-2.5 border-b border-border p-2.5">
        <div className="grid gap-2.5 lg:grid-cols-2">
          <div className="flex min-w-0 flex-col gap-1.5">
            <span className="text-xs text-muted-foreground">Project</span>
            <div className="flex flex-wrap gap-1.5">
              {projects.length === 0 ? (
                <span className="text-sm text-muted-foreground">
                  Create a project on Projects so boxes can be matched.
                </span>
              ) : (
                projects.map((project) => (
                  <Button
                    key={project.id}
                    size="sm"
                    variant={project.id === projectId ? "secondary" : "ghost"}
                    disabled={running || live}
                    onClick={() => selectProject(project.id)}
                  >
                    {project.name}
                  </Button>
                ))
              )}
            </div>
          </div>

          <div className="flex min-w-0 flex-col gap-1.5">
            <span className="text-xs text-muted-foreground">Source</span>
            <div className="flex flex-wrap gap-1.5">
              <Button
                size="sm"
                variant={sourceId === null ? "secondary" : "ghost"}
                disabled={running}
                onClick={() => setSourceId(null)}
              >
                Primary display
              </Button>
              {sources.map((source) => (
                <Button
                  key={source.id}
                  size="sm"
                  variant={source.id === sourceId ? "secondary" : "ghost"}
                  disabled={running}
                  title={`${source.kind}: ${source.target}`}
                  onClick={() => setSourceId(source.id)}
                >
                  {source.label}
                </Button>
              ))}
            </div>
            {sources.length === 0 ? (
              <span className="text-xs text-muted-foreground">
                Add a source on Sources to replay a file or folder.
              </span>
            ) : null}
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <label className="flex h-8 items-center gap-1.5 text-xs text-muted-foreground">
            <input
              type="checkbox"
              className="size-4 accent-primary"
              checked={record}
              disabled={running}
              onChange={(event) => setRecord(event.target.checked)}
            />
            Record session
          </label>
          <div className="ml-auto flex flex-wrap items-center gap-2">
            <label htmlFor="box-smoothing" className="text-xs text-muted-foreground">
              Box smoothing
            </label>
            <input
              id="box-smoothing"
              type="range"
              min={0}
              max={200}
              step={5}
              value={smoothMs}
              onChange={(event) => void changeSmoothing(Number(event.target.value))}
              className="w-28 accent-primary"
              title="Milliseconds a box takes to reach its new position. Zero snaps."
            />
            <span className="text-sm tabular-nums">{smoothMs} ms</span>
            <Button
              size="sm"
              variant="ghost"
              className="h-8 gap-1.5"
              title="Back to the 55 ms default"
              onClick={() => void changeSmoothing(55)}
            >
              <RotateCcw className="size-4" aria-hidden="true" />
              Reset
            </Button>
            <label htmlFor="coverage-cap" className="text-xs text-muted-foreground">
              Coverage cap
            </label>
            <input
              id="coverage-cap"
              type="number"
              min={0}
              max={400}
              step={10}
              value={harvestCap}
              onChange={(event) => {
                const next = Number(event.target.value);
                setHarvestCap(
                  Number.isFinite(next) ? Math.min(400, Math.max(0, Math.round(next))) : 0
                );
              }}
              onBlur={() => void changeHarvestCap(harvestCap)}
              className="h-8 w-20 rounded-md border border-border bg-background px-2 text-sm tabular-nums"
            />
            <span className="text-xs text-muted-foreground">per run</span>
            <label htmlFor="example-cap" className="text-xs text-muted-foreground">
              Examples cap
            </label>
            <input
              id="example-cap"
              type="number"
              min={0}
              max={100000}
              step={50}
              value={exampleCap}
              onChange={(event) => {
                const next = Number(event.target.value);
                setExampleCap(
                  Number.isFinite(next) ? Math.min(100000, Math.max(0, Math.round(next))) : 0
                );
              }}
              onBlur={() => void changeExampleCap(exampleCap)}
              className="h-8 w-24 rounded-md border border-border bg-background px-2 text-sm tabular-nums"
              title="Embedded examples loaded per class and role. Zero uses every example."
            />
            <span className="text-xs text-muted-foreground">per role, 0 = all</span>
            <label htmlFor="match-floor" className="text-xs text-muted-foreground">
              Match floor
            </label>
            <input
              id="match-floor"
              type="range"
              min={0}
              max={100}
              step={5}
              value={Math.round(floor * 100)}
              onChange={(event) => void changeFloor(Number(event.target.value))}
              className="w-40 accent-primary"
            />
            <span className="text-sm tabular-nums">{Math.round(floor * 100)}%</span>
          </div>
        </div>
      </div>

      <div className="flex flex-col gap-2.5 p-2.5">
        <div className="grid grid-cols-2 gap-2 lg:grid-cols-4">
          <Metric value={stats ? stats.fps.toFixed(1) : "0.0"} label="Frames per second" />
          <Metric
            value={detections ? `${detections.inferenceMs.toFixed(1)} ms` : "0 ms"}
            label="Inference"
          />
          <Metric value={stats ? `${stats.matchMs.toFixed(1)} ms` : "0 ms"} label="Matcher" />
          <Metric
            value={detections ? `${detections.latencyMs.toFixed(1)} ms` : "0 ms"}
            label="Frame to box"
          />
        </div>
        <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          <Badge variant="outline">{stats?.backend ?? "idle"}</Badge>
          <span className="tabular-nums">
            {stats ? `${stats.frameGapMs.toFixed(1)} ms interval` : "no frames yet"}
          </span>
          {stats ? (
            <span className="tabular-nums">
              capture {stats.captureP50Ms.toFixed(1)} p50, {stats.captureP95Ms.toFixed(1)} p95
              {stats.captureMaxMs > 0 ? `, ${stats.captureMaxMs.toFixed(1)} worst` : ""}
            </span>
          ) : null}
          {stats ? (
            <span className="tabular-nums">{stats.preprocessMs.toFixed(1)} ms preprocess</span>
          ) : null}
          {stats ? (
            <span className="tabular-nums">
              {stats.dropped} dropped of {stats.frames}
            </span>
          ) : null}
          {stats?.width ? (
            <span className="tabular-nums">
              {stats.width} by {stats.height}
            </span>
          ) : null}
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
              <div className="flex aspect-video w-full flex-col items-center justify-center gap-2 rounded-md border border-border bg-muted p-2.5 text-center">
                <Play className="size-5 text-muted-foreground" aria-hidden="true" />
                <p className="text-sm font-medium">No frame yet</p>
                <p className="max-w-[45ch] text-xs text-muted-foreground">
                  Start capture to see the latest frame and its boxes here.
                </p>
              </div>
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
                Two RF-DETR sizes, both Apache 2.0. Each download is checked against a
                pinned SHA-256 and stays on this machine.
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
                    className="flex flex-col gap-1.5 border-t border-border py-2 first:border-t-0 first:pt-0"
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
            <Badge variant={overlay?.visible ? "default" : "outline"}>
              {overlay?.visible ? "visible" : "hidden"}
            </Badge>
          </CardHeader>
          <CardContent className="flex flex-col gap-2">
            <div className="flex flex-wrap items-center gap-2">
              <Button size="sm" variant="outline" disabled={busy} onClick={toggleOverlay}>
                {overlay?.visible ? "Hide overlay" : "Show overlay"}
              </Button>
              <Button size="sm" variant="outline" disabled={busy} onClick={toggleCalibration}>
                {overlay?.calibration ? "Calibration on" : "Calibration off"}
              </Button>
              <span className="text-xs text-muted-foreground">
                Hotkey {overlay?.hotkey ?? "Ctrl+Alt+O"}, and it works while another window is
                focused.
              </span>
            </div>
            <p className="text-xs text-muted-foreground tabular-nums">
              {paint
                ? `Paint ${paint.fps.toFixed(1)} fps, ${paint.drawMs.toFixed(2)} ms per frame, ${paint.boxes} boxes${paint.calibration ? ", calibration" : ""}`
                : "Paint numbers appear once the overlay has drawn a frame."}
            </p>
          </CardContent>
        </Card>

      </div>
    </div>
  );
}

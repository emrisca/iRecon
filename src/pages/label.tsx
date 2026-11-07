import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";
import { Crop, Download, Maximize2, Minimize2, Minus, Plus, Trash2 } from "lucide-react";

import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../components/ui/card";
import { Input } from "../components/ui/input";
import { Skeleton } from "../components/ui/skeleton";
import { PageHeader } from "../components/page-header";
import {
  createClass,
  createExample,
  deleteClass,
  embedExamples,
  exportDataset,
  grabFrame,
  listClasses,
  listExamples,
  listProjects,
  matchRegion,
  setClassThreshold,
} from "../lib/api";
import type {
  Box,
  ClassMatch,
  DatasetExport,
  ExampleView,
  FrameView,
  LabelClass,
  Project,
} from "../lib/types";

const MIN_SCALE = 1;
const MAX_SCALE = 12;

type FrameStageProps = {
  frame: FrameView;
  box: Box | null;
  onBox: (box: Box | null) => void;
  action?: ReactNode;
  className?: string;
};

function clamp01(value: number) {
  return Math.min(1, Math.max(0, value));
}

function clampScale(value: number) {
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, value));
}

/**
 * A small image viewer: wheel pans, Command or Control with the wheel zooms
 * around the pointer, Space-drag, right-drag or middle-drag pans, drag draws a
 * box, and double click fits the frame again. The picture is transformed with
 * CSS rather than resized, so zoom cannot feed back into layout and the box is
 * drawn in screen space so its border stays one pixel at any zoom.
 */
function FrameStage({
  frame,
  box,
  onBox,
  action,
  className = "h-[70vh]",
}: FrameStageProps) {
  const viewport = useRef<HTMLDivElement | null>(null);
  const picture = useRef<HTMLImageElement | null>(null);
  const [baseWidth, setBaseWidth] = useState(0);
  const [scale, setScale] = useState(1);
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const [draft, setDraft] = useState<Box | null>(null);
  const [panning, setPanning] = useState(false);
  const drawStart = useRef<{ x: number; y: number } | null>(null);
  const panStart = useRef<{ x: number; y: number; ox: number; oy: number } | null>(null);
  const spaceHeld = useRef(false);

  const baseHeight = baseWidth > 0 ? (baseWidth * frame.height) / frame.width : 0;
  const viewWidth = baseWidth * scale;
  const viewHeight = baseHeight * scale;

  useEffect(() => {
    const element = viewport.current;
    if (!element) return;
    // Measured once per resize and capped at the window width, so a bad
    // measurement can never ask for a bitmap wider than the screen.
    const measure = () => setBaseWidth(Math.min(element.clientWidth, window.innerWidth));
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    setScale(1);
    setOffset({ x: 0, y: 0 });
  }, [frame.id]);

  const limit = useCallback((value: number, content: number, view: number) => {
    const span = content - view;
    return span <= 0 ? span / 2 : Math.min(0, Math.max(-span, value));
  }, []);

  const contain = useCallback(
    (x: number, y: number, nextScale: number) => {
      const element = viewport.current;
      if (!element || baseWidth === 0) return { x, y };
      return {
        x: limit(x, baseWidth * nextScale, element.clientWidth),
        y: limit(y, baseHeight * nextScale, element.clientHeight),
      };
    },
    [baseWidth, baseHeight, limit]
  );

  useEffect(() => {
    setOffset((current) => contain(current.x, current.y, scale));
  }, [scale, baseWidth, contain]);

  useEffect(() => {
    const element = viewport.current;
    if (!element) return;
    const onWheel = (event: WheelEvent) => {
      if (baseWidth === 0) return;
      event.preventDefault();
      if (event.ctrlKey || event.metaKey) {
        const next = clampScale(scale * Math.exp(-event.deltaY * 0.0022));
        if (next === scale) return;
        const rect = element.getBoundingClientRect();
        const px = event.clientX - rect.left;
        const py = event.clientY - rect.top;
        const factor = next / scale;
        setOffset(contain(px - (px - offset.x) * factor, py - (py - offset.y) * factor, next));
        setScale(next);
        return;
      }
      setOffset(contain(offset.x - event.deltaX, offset.y - event.deltaY, scale));
    };
    element.addEventListener("wheel", onWheel, { passive: false });
    return () => element.removeEventListener("wheel", onWheel);
  }, [baseWidth, scale, offset, contain]);

  useEffect(() => {
    const onDown = (event: KeyboardEvent) => {
      if (event.code === "Space") spaceHeld.current = true;
    };
    const onUp = (event: KeyboardEvent) => {
      if (event.code === "Space") spaceHeld.current = false;
    };
    window.addEventListener("keydown", onDown);
    window.addEventListener("keyup", onUp);
    return () => {
      window.removeEventListener("keydown", onDown);
      window.removeEventListener("keyup", onUp);
    };
  }, []);

  function normalized(clientX: number, clientY: number) {
    const element = picture.current;
    if (!element) return { x: 0, y: 0 };
    const rect = element.getBoundingClientRect();
    return {
      x: clamp01((clientX - rect.left) / rect.width),
      y: clamp01((clientY - rect.top) / rect.height),
    };
  }

  function onPointerDown(event: ReactPointerEvent<HTMLDivElement>) {
    try {
      event.currentTarget.setPointerCapture(event.pointerId);
    } catch {
      // A pointer that already left the window cannot be captured; the drag
      // still works from the events that do arrive.
    }
    const pan = event.button === 1 || event.button === 2 || event.altKey || spaceHeld.current;
    if (pan) {
      panStart.current = { x: event.clientX, y: event.clientY, ox: offset.x, oy: offset.y };
      setPanning(true);
      return;
    }
    if (event.button !== 0) return;
    const at = normalized(event.clientX, event.clientY);
    drawStart.current = at;
    onBox(null);
    setDraft({ x: at.x, y: at.y, w: 0, h: 0 });
  }

  function onPointerMove(event: ReactPointerEvent<HTMLDivElement>) {
    if (panStart.current) {
      const start = panStart.current;
      setOffset(
        contain(start.ox + (event.clientX - start.x), start.oy + (event.clientY - start.y), scale)
      );
      return;
    }
    if (!drawStart.current) return;
    const at = normalized(event.clientX, event.clientY);
    const from = drawStart.current;
    setDraft({
      x: Math.min(from.x, at.x),
      y: Math.min(from.y, at.y),
      w: Math.abs(at.x - from.x),
      h: Math.abs(at.y - from.y),
    });
  }

  function onPointerUp(event: ReactPointerEvent<HTMLDivElement>) {
    if (panStart.current) {
      panStart.current = null;
      setPanning(false);
      return;
    }
    const from = drawStart.current;
    drawStart.current = null;
    setDraft(null);
    if (!from) return;
    const at = normalized(event.clientX, event.clientY);
    const next = {
      x: Math.min(from.x, at.x),
      y: Math.min(from.y, at.y),
      w: Math.abs(at.x - from.x),
      h: Math.abs(at.y - from.y),
    };
    if (next.w > 0.004 && next.h > 0.004) {
      onBox(next);
    }
  }

  function zoomBy(factor: number) {
    const element = viewport.current;
    if (!element) return;
    const next = clampScale(scale * factor);
    if (next === scale) return;
    const cx = element.clientWidth / 2;
    const cy = element.clientHeight / 2;
    const k = next / scale;
    setOffset(contain(cx - (cx - offset.x) * k, cy - (cy - offset.y) * k, next));
    setScale(next);
  }

  function fit() {
    setScale(1);
    setOffset({ x: 0, y: 0 });
  }

  const active = draft ?? box;

  return (
    <div className="flex flex-col gap-1.5">
      <div
        ref={viewport}
        className={`relative overflow-hidden rounded-md border border-border bg-muted ${className}`}
        style={{ cursor: panning ? "grabbing" : "crosshair" }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={() => {
          drawStart.current = null;
          panStart.current = null;
          setDraft(null);
          setPanning(false);
        }}
        onDoubleClick={fit}
        onContextMenu={(event) => event.preventDefault()}
      >
        <div
          className="relative origin-top-left"
          style={{
            width: baseWidth || 1,
            height: baseHeight || 1,
            transform: `translate(${offset.x}px, ${offset.y}px) scale(${scale})`,
          }}
        >
          <img
            ref={picture}
            src={frame.preview}
            alt="Grabbed frame"
            draggable={false}
            className="pointer-events-none block h-full w-full select-none"
          />
        </div>
        {active ? (
          <div
            className="pointer-events-none absolute border border-white ring-1 ring-black/60"
            style={{
              left: offset.x + active.x * viewWidth,
              top: offset.y + active.y * viewHeight,
              width: active.w * viewWidth,
              height: active.h * viewHeight,
            }}
          />
        ) : null}
      </div>

      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-1.5">
          <Button
            size="sm"
            variant="outline"
            aria-label="Zoom out"
            disabled={scale <= MIN_SCALE}
            onClick={() => zoomBy(1 / 1.4)}
          >
            <Minus className="size-4" aria-hidden="true" />
          </Button>
          <Badge variant="outline" className="min-w-14 justify-center tabular-nums">
            {Math.round(scale * 100)}%
          </Badge>
          <Button
            size="sm"
            variant="outline"
            aria-label="Zoom in"
            disabled={scale >= MAX_SCALE}
            onClick={() => zoomBy(1.4)}
          >
            <Plus className="size-4" aria-hidden="true" />
          </Button>
          <Button size="sm" variant="ghost" onClick={fit}>
            Fit
          </Button>
        </div>
        {action}
      </div>
    </div>
  );
}

export function LabelPage() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectId, setProjectId] = useState<number | null>(null);
  const [classes, setClasses] = useState<LabelClass[]>([]);
  const [frame, setFrame] = useState<FrameView | null>(null);
  const [box, setBox] = useState<Box | null>(null);
  const [fullscreen, setFullscreen] = useState(false);
  const [className, setClassName] = useState("");
  const [selected, setSelected] = useState<LabelClass | null>(null);
  const [examples, setExamples] = useState<ExampleView[]>([]);
  const [matches, setMatches] = useState<ClassMatch[]>([]);
  const [exportInfo, setExportInfo] = useState<DatasetExport | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmId, setConfirmId] = useState<number | null>(null);

  useEffect(() => {
    let alive = true;
    listProjects()
      .then((rows) => {
        if (!alive) return;
        setProjects(rows);
        setProjectId((current) => current ?? rows[0]?.id ?? null);
      })
      .catch((reason: unknown) => {
        if (alive) setError(String(reason));
      });
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    if (!projectId) {
      setClasses([]);
      return;
    }
    let alive = true;
    listClasses(projectId)
      .then((rows) => {
        if (alive) setClasses(rows);
      })
      .catch((reason: unknown) => {
        if (alive) setError(String(reason));
      });
    // Examples saved before the matcher existed carry no fingerprint; filling
    // them in here keeps the first box already comparable.
    embedExamples(projectId).catch(() => {});
    setFrame(null);
    setBox(null);
    setFullscreen(false);
    setSelected(null);
    setExamples([]);
    setMatches([]);
    setExportInfo(null);
    setClassName("");
    return () => {
      alive = false;
    };
  }, [projectId]);

  useEffect(() => {
    if (!selected) {
      setExamples([]);
      return;
    }
    let alive = true;
    listExamples(selected.id)
      .then((rows) => {
        if (alive) setExamples(rows);
      })
      .catch((reason: unknown) => {
        if (alive) setError(String(reason));
      });
    return () => {
      alive = false;
    };
  }, [selected]);

  useEffect(() => {
    if (!projectId || !frame || !box) {
      setMatches([]);
      return;
    }
    let alive = true;
    matchRegion(projectId, frame.id, box)
      .then((rows) => {
        if (alive) setMatches(rows);
      })
      .catch(() => {
        if (alive) setMatches([]);
      });
    return () => {
      alive = false;
    };
  }, [projectId, frame, box]);

  useEffect(() => {
    if (!fullscreen) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setFullscreen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [fullscreen]);

  async function refreshClasses(id: number) {
    const rows = await listClasses(id);
    setClasses(rows);
    return rows;
  }

  async function grab() {
    if (!projectId) return;
    setBusy(true);
    setError(null);
    try {
      const next = await grabFrame(projectId);
      setFrame(next);
      setBox(null);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function save() {
    if (!projectId || !frame || !box) return;
    const name = className.trim();
    if (!name) return;
    setBusy(true);
    setError(null);
    try {
      let target = classes.find((item) => item.name.toLowerCase() === name.toLowerCase());
      if (!target) {
        target = await createClass(projectId, name);
      }
      const created = await createExample(target.id, frame.id, box);
      setBox(null);
      setSelected(target);
      setExamples((current) => [created, ...current]);
      await refreshClasses(projectId);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function exportData() {
    if (!projectId) return;
    setBusy(true);
    setError(null);
    try {
      const result = await exportDataset(projectId);
      setExportInfo(result);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function adjustThreshold(item: LabelClass, delta: number) {
    const next = Math.min(1, Math.max(0, Math.round((item.threshold + delta) * 100) / 100));
    if (next === item.threshold) return;
    setBusy(true);
    setError(null);
    try {
      const updated = await setClassThreshold(item.id, next);
      setClasses((current) =>
        current.map((entry) => (entry.id === updated.id ? updated : entry))
      );
      setSelected((current) => (current?.id === updated.id ? updated : current));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function removeClass(id: number) {
    setBusy(true);
    try {
      await deleteClass(id);
      if (projectId) await refreshClasses(projectId);
      if (selected?.id === id) setSelected(null);
      setConfirmId(null);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  const projectName = projects.find((project) => project.id === projectId)?.name;
  const fullscreenButton = (
    <Button
      size="sm"
      variant="outline"
      className="gap-1.5"
      disabled={!frame}
      onClick={() => setFullscreen(true)}
    >
      <Maximize2 className="size-4" aria-hidden="true" />
      Full screen
    </Button>
  );

  return (
    <div className="flex min-h-full flex-col">
      <PageHeader
        title="Label"
        description="Grab a frame, drag a box around the thing you care about, name it. Each save writes a crop into the project and becomes a new example."
        action={
          <Button size="sm" className="gap-1.5" disabled={!projectId || busy} onClick={grab}>
            <Crop className="size-4" aria-hidden="true" />
            {busy ? "Working" : "Grab frame"}
          </Button>
        }
      />

      {error ? (
        <p role="alert" className="border-b border-border p-2 text-sm text-destructive">
          {error}
        </p>
      ) : null}

      <div className="flex flex-col gap-2.5 p-2.5">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-xs text-muted-foreground">Project</span>
          {projects.length === 0 ? (
            <span className="text-sm text-muted-foreground">
              Create a project on the Projects page first.
            </span>
          ) : (
            projects.map((project) => (
              <Button
                key={project.id}
                size="sm"
                variant={project.id === projectId ? "secondary" : "ghost"}
                onClick={() => setProjectId(project.id)}
              >
                {project.name}
              </Button>
            ))
          )}
        </div>

        <div className="grid gap-2.5 lg:grid-cols-[1.6fr_1fr]">
          <Card className="min-w-0">
            <CardHeader className="flex flex-row items-start justify-between gap-2">
              <div className="flex flex-col gap-1.5">
                <CardTitle>Frame</CardTitle>
                <CardDescription>
                  Drag a box around one object. The crop is taken from the full
                  resolution frame, not from the preview.
                </CardDescription>
              </div>
              {frame ? (
                <Badge variant="outline" className="tabular-nums">
                  {frame.width} by {frame.height}
                </Badge>
              ) : null}
            </CardHeader>
            <CardContent className="min-w-0">
              {frame ? (
                <FrameStage frame={frame} box={box} onBox={setBox} action={fullscreenButton} />
              ) : (
                <Skeleton className="aspect-video w-full" />
              )}
              <div className="flex flex-col gap-2">
                <p className="text-xs text-muted-foreground">
                  {frame
                    ? box
                      ? `Box ${(box.w * 100).toFixed(1)}% wide, ${(box.h * 100).toFixed(1)}% tall. Scroll to pan, Command or Control with scroll to zoom, Space-drag or right-drag to move, double click to fit.`
                      : "No box yet. Scroll to pan, Command or Control with scroll to zoom, then drag over the object."
                    : "Press Grab frame. The app hides itself for a moment so it is not in the shot."}
                </p>
                {matches.length > 0 ? (
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-xs text-muted-foreground">Nearest</span>
                    {matches.slice(0, 3).map((match) => (
                      <Badge
                        key={match.classId}
                        variant={match.score >= 0.85 ? "secondary" : "outline"}
                        className="tabular-nums"
                      >
                        {match.name} {match.score.toFixed(2)}
                      </Badge>
                    ))}
                  </div>
                ) : null}
                <div className="flex flex-wrap items-center gap-2">
                  <Button
                    size="sm"
                    variant="outline"
                    className="gap-1.5"
                    disabled={!projectId || busy}
                    onClick={exportData}
                  >
                    <Download className="size-4" aria-hidden="true" />
                    Export dataset
                  </Button>
                  {exportInfo ? (
                    <span className="text-xs text-muted-foreground tabular-nums">
                      {exportInfo.frames} frames, {exportInfo.boxes} boxes, {exportInfo.skipped} skipped
                    </span>
                  ) : null}
                </div>
              </div>
            </CardContent>
          </Card>

          <Card className="min-w-0">
            <CardHeader>
              <CardTitle>Class</CardTitle>
              <CardDescription>
                Type a new name or pick an existing class. Saving adds the crop to that
                class and updates its count.
              </CardDescription>
            </CardHeader>
            <CardContent>
              <Input
                value={className}
                onChange={(event) => setClassName(event.target.value)}
                placeholder="red car"
                aria-label="Class name"
                maxLength={40}
              />
              {classes.length > 0 ? (
                <div className="flex flex-wrap items-center gap-2">
                  {classes.map((item) => (
                    <div key={item.id} className="flex items-center gap-1.5">
                      <Button
                        size="sm"
                        variant={
                          className.toLowerCase() === item.name.toLowerCase()
                            ? "secondary"
                            : "outline"
                        }
                        className="gap-1.5"
                        onClick={() => {
                          setClassName(item.name);
                          setSelected(item);
                        }}
                      >
                        <span
                          className="size-2 rounded-full"
                          style={{ backgroundColor: item.color }}
                          aria-hidden="true"
                        />
                        {item.name}
                        <Badge variant="secondary" className="tabular-nums">
                          {item.examples}
                        </Badge>
                        <Badge variant="outline" className="tabular-nums">
                          {Math.round(item.threshold * 100)}%
                        </Badge>
                      </Button>
                      <Button
                        size="icon"
                        variant="ghost"
                        aria-label={`Delete ${item.name}`}
                        onClick={() => setConfirmId(confirmId === item.id ? null : item.id)}
                      >
                        <Trash2 className="size-4" aria-hidden="true" />
                      </Button>
                      {confirmId === item.id ? (
                        <Button
                          size="sm"
                          variant="destructive"
                          disabled={busy}
                          onClick={() => removeClass(item.id)}
                        >
                          Delete
                        </Button>
                      ) : null}
                    </div>
                  ))}
                </div>
              ) : (
                <p className="text-xs text-muted-foreground">
                  No classes yet. The first save creates one.
                </p>
              )}
              <Button
                size="sm"
                disabled={!frame || !box || !className.trim() || busy}
                onClick={save}
              >
                Save example
              </Button>
            </CardContent>
          </Card>
        </div>

        {selected ? (
          <Card>
            <CardHeader className="flex flex-row items-start justify-between gap-2">
              <div className="flex flex-col gap-1.5">
                <CardTitle>{selected.name}</CardTitle>
                <CardDescription>
                  {examples.length} examples. A box must score at least{" "}
                  {Math.round(selected.threshold * 100)}% against this class to be drawn.
                </CardDescription>
              </div>
              <div className="flex items-center gap-1.5">
                <span className="text-xs text-muted-foreground">Threshold</span>
                <Button
                  size="icon"
                  variant="ghost"
                  aria-label={`Lower threshold for ${selected.name}`}
                  disabled={busy || selected.threshold <= 0}
                  onClick={() => adjustThreshold(selected, -0.05)}
                >
                  <Minus className="size-4" aria-hidden="true" />
                </Button>
                <Badge variant="outline" className="tabular-nums">
                  {Math.round(selected.threshold * 100)}%
                </Badge>
                <Button
                  size="icon"
                  variant="ghost"
                  aria-label={`Raise threshold for ${selected.name}`}
                  disabled={busy || selected.threshold >= 1}
                  onClick={() => adjustThreshold(selected, 0.05)}
                >
                  <Plus className="size-4" aria-hidden="true" />
                </Button>
              </div>
            </CardHeader>
            <CardContent>
              {examples.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  No examples saved for this class yet.
                </p>
              ) : (
                <ul className="grid grid-cols-3 gap-2 sm:grid-cols-4 lg:grid-cols-6">
                  {examples.map((example) => (
                    <li key={example.id} className="flex flex-col gap-1.5">
                      <img
                        src={example.preview}
                        alt={`Example from ${example.createdAt}`}
                        className="aspect-square w-full rounded-md border border-border object-cover"
                      />
                      <span className="truncate text-xs text-muted-foreground tabular-nums">
                        {example.width} by {example.height}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </CardContent>
          </Card>
        ) : null}
      </div>

      {fullscreen && frame ? (
        <div className="fixed inset-0 z-50 flex flex-col gap-2 bg-background p-2.5">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="flex items-center gap-2">
              <span className="text-sm font-medium">
                {projectName ? `${projectName} frame` : "Frame"}
              </span>
              <Badge variant="outline" className="tabular-nums">
                {frame.width} by {frame.height}
              </Badge>
            </div>
            <Button
              size="sm"
              variant="outline"
              className="gap-1.5"
              onClick={() => setFullscreen(false)}
            >
              <Minimize2 className="size-4" aria-hidden="true" />
              Close
            </Button>
          </div>
          <FrameStage
            frame={frame}
            box={box}
            onBox={setBox}
            className="min-h-0 flex-1"
            action={
              <span className="text-xs text-muted-foreground">
                Escape closes. Scroll to pan, Command or Control with scroll to zoom.
              </span>
            }
          />
        </div>
      ) : null}
    </div>
  );
}

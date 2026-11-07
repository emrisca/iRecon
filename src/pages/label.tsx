import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { Crop, Maximize2, Minimize2, Trash2 } from "lucide-react";

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
  grabFrame,
  listClasses,
  listExamples,
  listProjects,
} from "../lib/api";
import type { Box, ExampleView, FrameView, LabelClass, Project } from "../lib/types";

type FrameStageProps = {
  frame: FrameView;
  zoom: number;
  box: Box | null;
  onBox: (box: Box | null) => void;
  className?: string;
};

function FrameStage({
  frame,
  zoom,
  box,
  onBox,
  className = "max-h-[70vh]",
}: FrameStageProps) {
  const [draft, setDraft] = useState<Box | null>(null);
  const start = useRef<{ x: number; y: number } | null>(null);

  function point(event: ReactPointerEvent<HTMLDivElement>) {
    const rect = event.currentTarget.getBoundingClientRect();
    return {
      x: Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width)),
      y: Math.min(1, Math.max(0, (event.clientY - rect.top) / rect.height)),
    };
  }

  function onPointerDown(event: ReactPointerEvent<HTMLDivElement>) {
    event.currentTarget.setPointerCapture(event.pointerId);
    const at = point(event);
    start.current = at;
    onBox(null);
    setDraft({ x: at.x, y: at.y, w: 0, h: 0 });
  }

  function onPointerMove(event: ReactPointerEvent<HTMLDivElement>) {
    if (!start.current) return;
    const at = point(event);
    const from = start.current;
    setDraft({
      x: Math.min(from.x, at.x),
      y: Math.min(from.y, at.y),
      w: Math.abs(at.x - from.x),
      h: Math.abs(at.y - from.y),
    });
  }

  function onPointerUp(event: ReactPointerEvent<HTMLDivElement>) {
    const from = start.current;
    start.current = null;
    setDraft(null);
    if (!from) return;
    const at = point(event);
    const next = {
      x: Math.min(from.x, at.x),
      y: Math.min(from.y, at.y),
      w: Math.abs(at.x - from.x),
      h: Math.abs(at.y - from.y),
    };
    if (next.w > 0.01 && next.h > 0.01) {
      onBox(next);
    }
  }

  const active = draft ?? box;

  return (
    <div
      className={`overflow-auto rounded-md border border-border ${className}`}
    >
      <div
        className="relative cursor-crosshair select-none"
        style={{ width: `${zoom * 100}%` }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={() => {
          start.current = null;
          setDraft(null);
        }}
      >
        <img
          src={frame.preview}
          alt="Grabbed frame"
          className="pointer-events-none w-full"
          draggable={false}
        />
        {active ? (
          <div
            className="pointer-events-none absolute border border-white ring-1 ring-black/60"
            style={{
              left: `${active.x * 100}%`,
              top: `${active.y * 100}%`,
              width: `${active.w * 100}%`,
              height: `${active.h * 100}%`,
            }}
          />
        ) : null}
      </div>
    </div>
  );
}

function ZoomControls({
  zoom,
  onChange,
}: {
  zoom: number;
  onChange: (zoom: number) => void;
}) {
  return (
    <div className="flex items-center gap-1.5">
      <span className="text-xs text-muted-foreground">Zoom</span>
      {[1, 2, 4, 8].map((level) => (
        <Button
          key={level}
          size="sm"
          variant={zoom === level ? "secondary" : "outline"}
          className="tabular-nums"
          aria-pressed={zoom === level}
          onClick={() => onChange(level)}
        >
          {level}x
        </Button>
      ))}
    </div>
  );
}

export function LabelPage() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectId, setProjectId] = useState<number | null>(null);
  const [classes, setClasses] = useState<LabelClass[]>([]);
  const [frame, setFrame] = useState<FrameView | null>(null);
  const [box, setBox] = useState<Box | null>(null);
  const [zoom, setZoom] = useState(1);
  const [fullscreen, setFullscreen] = useState(false);
  const [className, setClassName] = useState("");
  const [selected, setSelected] = useState<LabelClass | null>(null);
  const [examples, setExamples] = useState<ExampleView[]>([]);
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
    setFrame(null);
    setBox(null);
    setZoom(1);
    setFullscreen(false);
    setSelected(null);
    setExamples([]);
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

  return (
    <div className="flex min-h-full flex-col">
      <PageHeader
        title="Label"
        description="Grab a frame, drag a box around the thing you care about, name it. Each save writes a crop into the project and becomes a new example."
        action={
          <Button
            size="sm"
            className="gap-1.5"
            disabled={!projectId || busy}
            onClick={grab}
          >
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
          <Card>
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
            <CardContent>
              {frame ? (
                <FrameStage frame={frame} zoom={zoom} box={box} onBox={setBox} />
              ) : (
                <Skeleton className="aspect-video w-full" />
              )}
              <div className="flex flex-wrap items-center justify-between gap-2">
                <ZoomControls zoom={zoom} onChange={setZoom} />
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
              </div>
              <p className="text-xs text-muted-foreground">
                {frame
                  ? box
                    ? `Box ${(box.w * 100).toFixed(1)}% wide, ${(box.h * 100).toFixed(1)}% tall.`
                    : "No box yet. Drag over the object, or zoom in first."
                  : "Press Grab frame. The app hides itself for a moment so it is not in the shot."}
              </p>
            </CardContent>
          </Card>

          <Card>
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
                  {examples.length} examples. They are what the matcher will compare new
                  proposals against.
                </CardDescription>
              </div>
              <Badge variant="outline" className="tabular-nums">
                threshold {selected.threshold.toFixed(2)}
              </Badge>
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
            <div className="flex flex-wrap items-center gap-2">
              <ZoomControls zoom={zoom} onChange={setZoom} />
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
          </div>
          <FrameStage
            frame={frame}
            zoom={zoom}
            box={box}
            onBox={setBox}
            className="min-h-0 flex-1"
          />
          <p className="text-xs text-muted-foreground">
            Escape closes. Scroll to pan when zoomed.
          </p>
        </div>
      ) : null}
    </div>
  );
}

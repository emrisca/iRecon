import { useEffect, useState } from "react";
import {
  Crop,
  Download,
  Maximize2,
  Minimize2,
  Minus,
  Plus,
  RotateCcw,
  Trash2,
} from "lucide-react";

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
import { Separator } from "../components/ui/separator";
import { FrameStage } from "../components/frame-stage";
import { PageHeader } from "../components/page-header";
import {
  CLASS_COLORS,
  classAdvice,
  createClass,
  createExample,
  deleteClass,
  embedExamples,
  exportDataset,
  grabFrame,
  isDesktop,
  listClasses,
  listExamples,
  listFrames,
  listProjects,
  matchRegion,
  openFrame,
  resetDemoData,
  setClassThreshold,
  updateClass,
} from "../lib/api";
import type {
  Box,
  ClassAdvice,
  ClassMatch,
  DatasetExport,
  ExampleView,
  FrameView,
  LabelClass,
  Project,
} from "../lib/types";

function ExampleTile({ example }: { example: ExampleView }) {
  return (
    <li className="flex flex-col gap-1.5">
      <img
        src={example.preview}
        alt={`Example from ${example.createdAt}`}
        className="aspect-square w-full rounded-md border border-border object-cover"
      />
      <span className="truncate text-xs text-muted-foreground tabular-nums">
        {example.width} by {example.height}
      </span>
    </li>
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
  const [role, setRole] = useState<"positive" | "negative">("positive");
  const [library, setLibrary] = useState<FrameView[]>([]);
  const [rename, setRename] = useState("");
  const [advice, setAdvice] = useState<ClassAdvice[]>([]);

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

  // Coverage and threshold advice follow the class list, so every save or
  // threshold change refreshes it without a separate button.
  useEffect(() => {
    if (!projectId) {
      setAdvice([]);
      return;
    }
    let alive = true;
    classAdvice(projectId)
      .then((rows) => {
        if (alive) setAdvice(rows);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [projectId, classes]);

  async function applyAdvice(entry: ClassAdvice) {
    if (!projectId || entry.suggested === null) return;
    setBusy(true);
    setError(null);
    try {
      const updated = await setClassThreshold(entry.classId, entry.suggested);
      setClasses((current) =>
        current.map((item) => (item.id === updated.id ? updated : item))
      );
      setSelected((current) => (current?.id === updated.id ? updated : current));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

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
    listFrames(projectId)
      .then((rows) => {
        if (alive) setLibrary(rows);
      })
      .catch(() => {
        if (alive) setLibrary([]);
      });
    setFrame(null);
    setBox(null);
    setFullscreen(false);
    setSelected(null);
    setExamples([]);
    setMatches([]);
    setExportInfo(null);
    setClassName("");
    setRole("positive");
    setRename("");
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
    setRename(selected?.name ?? "");
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

  async function resetDemo() {
    setBusy(true);
    setError(null);
    try {
      await resetDemoData();
      if (projectId) await refreshClasses(projectId);
      setSelected(null);
      setExamples([]);
      setMatches([]);
      setBox(null);
      setExportInfo(null);
      setClassName("");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function grab() {
    if (!projectId) return;
    setBusy(true);
    setError(null);
    try {
      const next = await grabFrame(projectId);
      setFrame(next);
      setBox(null);
      setLibrary((current) => [next, ...current]);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function open(id: number) {
    setBusy(true);
    setError(null);
    try {
      const next = await openFrame(id);
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
      const created = await createExample(target.id, frame.id, box, role);
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

  async function renameSelected(item: LabelClass) {
    const name = rename.trim();
    if (!name || name === item.name) return;
    setBusy(true);
    setError(null);
    try {
      const updated = await updateClass(item.id, { name });
      setClasses((current) =>
        current.map((entry) => (entry.id === updated.id ? updated : entry))
      );
      setSelected(updated);
      setClassName(updated.name);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function recolour(item: LabelClass, color: string) {
    setBusy(true);
    setError(null);
    try {
      const updated = await updateClass(item.id, { color });
      setClasses((current) =>
        current.map((entry) => (entry.id === updated.id ? updated : entry))
      );
      setSelected(updated);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  const positives = examples.filter((example) => example.role !== "negative");
  const negatives = examples.filter((example) => example.role === "negative");
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
          {!isDesktop ? (
            <Button
              size="sm"
              variant="ghost"
              className="ml-auto gap-1.5"
              disabled={busy}
              onClick={resetDemo}
            >
              <RotateCcw className="size-4" aria-hidden="true" />
              Reset demo
            </Button>
          ) : null}
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
              {library.length > 0 ? (
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-xs text-muted-foreground">Library</span>
                  {library.slice(0, 8).map((entry) => (
                    <Button
                      key={entry.id}
                      size="icon"
                      variant={frame?.id === entry.id ? "secondary" : "outline"}
                      aria-label={`Open frame ${entry.id}`}
                      disabled={busy}
                      onClick={() => open(entry.id)}
                    >
                      <img
                        src={entry.preview}
                        alt=""
                        className="size-8 rounded-sm object-cover"
                      />
                    </Button>
                  ))}
                </div>
              ) : null}
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
                          setRename(item.name);
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
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  size="sm"
                  variant={role === "positive" ? "secondary" : "outline"}
                  onClick={() => setRole("positive")}
                >
                  Positive
                </Button>
                <Button
                  size="sm"
                  variant={role === "negative" ? "secondary" : "outline"}
                  onClick={() => setRole("negative")}
                >
                  Negative
                </Button>
                <Button
                  size="sm"
                  disabled={!frame || !box || !className.trim() || busy}
                  onClick={save}
                >
                  Save example
                </Button>
              </div>
              <p className="text-xs text-muted-foreground">
                {role === "positive"
                  ? "A positive is the thing itself, which is what teaches the matcher to box it."
                  : "A negative looks close but is not it, so saving one raises the bar for a match."}
              </p>
            </CardContent>
          </Card>
        </div>

        <Card>
          <CardHeader>
            <CardTitle>Coverage</CardTitle>
            <CardDescription>
              What the saved examples say about each class. The suggestion is the
              midpoint between how alike your positives are and how close the nearest
              negative sits; nothing changes until you apply it.
            </CardDescription>
          </CardHeader>
          <CardContent>
            {advice.length === 0 ? (
              <p className="text-sm text-muted-foreground">
                No classes to read yet.
              </p>
            ) : (
              <ul className="flex flex-col gap-2">
                {advice.map((entry) => (
                  <li key={entry.classId} className="rounded-lg border border-border p-2">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-sm font-medium">{entry.name}</span>
                      <Badge variant="secondary" className="tabular-nums">
                        {entry.positives} positive{entry.positives === 1 ? "" : "s"}
                      </Badge>
                      <Badge variant="outline" className="tabular-nums">
                        {entry.negatives} negative{entry.negatives === 1 ? "" : "s"}
                      </Badge>
                      {entry.positives >= 2 ? (
                        <Badge variant="outline" className="tabular-nums">
                          look-alike {Math.round(entry.similarity * 100)}%
                        </Badge>
                      ) : null}
                      {entry.suggested !== null ? (
                        <Button
                          size="sm"
                          variant="outline"
                          className="ml-auto tabular-nums"
                          disabled={busy || entry.suggested === entry.threshold}
                          onClick={() => applyAdvice(entry)}
                        >
                          Use {Math.round(entry.suggested * 100)}%
                        </Button>
                      ) : null}
                    </div>
                    {entry.hints.length > 0 ? (
                      <ul className="mt-1 flex list-disc flex-col gap-0.5 pl-4 text-xs text-muted-foreground">
                        {entry.hints.map((hint) => (
                          <li key={hint}>{hint}</li>
                        ))}
                      </ul>
                    ) : (
                      <p className="mt-1 text-xs text-muted-foreground">Looks healthy.</p>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </CardContent>
        </Card>

        {selected ? (
          <Card>
            <CardHeader className="flex flex-row items-start justify-between gap-2">
              <div className="flex flex-col gap-1.5">
                <CardTitle>{selected.name}</CardTitle>
                <CardDescription>
                  {positives.length} positives and {negatives.length} negatives. A box must
                  score at least {Math.round(selected.threshold * 100)}% against this class
                  to be drawn.
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
            <CardContent className="flex flex-col gap-2">
              <div className="flex flex-wrap items-center gap-2">
                <Input
                  value={rename}
                  onChange={(event) => setRename(event.target.value)}
                  aria-label={`Rename ${selected.name}`}
                  maxLength={40}
                  className="max-w-56"
                />
                <Button
                  size="sm"
                  variant="outline"
                  disabled={busy || !rename.trim() || rename.trim() === selected.name}
                  onClick={() => renameSelected(selected)}
                >
                  Rename
                </Button>
                <div className="flex flex-wrap items-center gap-1.5">
                  {CLASS_COLORS.map((color) => (
                    <button
                      key={color}
                      type="button"
                      aria-label={`Colour ${selected.name}`}
                      aria-pressed={selected.color === color}
                      className="size-6 rounded-full border border-border aria-pressed:ring-1 aria-pressed:ring-foreground"
                      style={{ backgroundColor: color }}
                      disabled={busy}
                      onClick={() => recolour(selected, color)}
                    />
                  ))}
                </div>
              </div>
              <Separator />
              {examples.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  No examples saved for this class yet.
                </p>
              ) : (
                <div className="flex flex-col gap-2">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-xs text-muted-foreground">Positives</span>
                    <Badge variant="secondary" className="tabular-nums">
                      {positives.length}
                    </Badge>
                    <span className="text-xs text-muted-foreground">Negatives</span>
                    <Badge variant="outline" className="tabular-nums">
                      {negatives.length}
                    </Badge>
                  </div>
                  <ul className="grid grid-cols-3 gap-2 sm:grid-cols-4 lg:grid-cols-6">
                    {positives.map((example) => (
                      <ExampleTile key={example.id} example={example} />
                    ))}
                  </ul>
                  {negatives.length > 0 ? (
                    <>
                      <span className="text-xs text-muted-foreground">Negatives</span>
                      <ul className="grid grid-cols-3 gap-2 sm:grid-cols-4 lg:grid-cols-6">
                        {negatives.map((example) => (
                          <ExampleTile key={example.id} example={example} />
                        ))}
                      </ul>
                    </>
                  ) : null}
                </div>
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

import { useEffect, useState } from "react";
import {
  Crop,
  Download,
  Keyboard,
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

const GRAB_SHORTCUT_KEY = "irecon.label.grabShortcut";

function readGrabShortcut(): string | null {
  if (typeof window === "undefined") return "G";
  const saved = window.localStorage.getItem(GRAB_SHORTCUT_KEY);
  if (saved === null) return "G";
  if (saved === "none") return null;
  return saved;
}

function shortcutLabel(key: string) {
  if (key === " ") return "Space";
  return key.length === 1 ? key.toUpperCase() : key;
}

function shortcutMatches(event: KeyboardEvent, shortcut: string) {
  if (shortcut === "Space") return event.code === "Space";
  return event.key.toLowerCase() === shortcut.toLowerCase();
}

type PendingBox = {
  id: string;
  box: Box;
  className: string;
  role: "positive" | "negative";
  color: string;
};

let pendingBoxId = 0;

function ExampleTile({ example }: { example: ExampleView }) {
  return (
    <li className="media-tile flex flex-col gap-1.5">
      <img
        src={example.preview}
        alt={`Example from ${example.createdAt}`}
        loading="lazy"
        decoding="async"
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
  const [pending, setPending] = useState<PendingBox[]>([]);
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
  const [grabShortcut, setGrabShortcut] = useState<string | null>(readGrabShortcut);
  const [recordingShortcut, setRecordingShortcut] = useState(false);

  useEffect(() => {
    window.localStorage.setItem(GRAB_SHORTCUT_KEY, grabShortcut ?? "none");
  }, [grabShortcut]);

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
    setPending([]);
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

  useEffect(() => {
    if (!grabShortcut || recordingShortcut) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || event.repeat) return;
      if (event.ctrlKey || event.metaKey || event.altKey) return;
      if (!shortcutMatches(event, grabShortcut)) return;
      const target = event.target;
      if (
        target instanceof Element &&
        target.closest("input, textarea, select, [contenteditable='true']")
      ) {
        return;
      }
      if (fullscreen || busy || !projectId || pending.length > 0) return;
      event.preventDefault();
      void grab();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busy, fullscreen, grabShortcut, pending.length, projectId, recordingShortcut]);

  useEffect(() => {
    if (!recordingShortcut) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.repeat) return;
      event.preventDefault();
      event.stopPropagation();
      if (event.key === "Escape" || event.key === "Tab") {
        setRecordingShortcut(false);
        return;
      }
      if (event.key === "Backspace" || event.key === "Delete") {
        setGrabShortcut(null);
        setRecordingShortcut(false);
        return;
      }
      if (
        event.key === "Shift" ||
        event.key === "Control" ||
        event.key === "Alt" ||
        event.key === "Meta" ||
        event.key === "CapsLock" ||
        event.key === "Dead" ||
        event.key === "Unidentified"
      ) {
        return;
      }
      setGrabShortcut(shortcutLabel(event.key));
      setRecordingShortcut(false);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [recordingShortcut]);

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
      setPending([]);
      setExportInfo(null);
      setClassName("");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function grab() {
    if (!projectId || pending.length > 0) return;
    setBusy(true);
    setError(null);
    try {
      const next = await grabFrame(projectId);
      setFrame(next);
      setBox(null);
      setPending([]);
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
      setPending([]);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function save() {
    if (!projectId || !frame) return;
    const queued = [...pending];
    let current: PendingBox | null = null;
    if (box && className.trim()) {
      const name = className.trim();
      pendingBoxId += 1;
      current = {
        id: `box-${pendingBoxId}`,
        box,
        className: name,
        role,
        color:
          classes.find((item) => item.name.toLowerCase() === name.toLowerCase())?.color ??
          "var(--primary)",
      };
      queued.push(current);
    }
    if (queued.length === 0) return;
    setBusy(true);
    setError(null);
    const saved = new Set<string>();
    let lastClass: LabelClass | null = null;
    let accounted = false;
    try {
      const byName = new Map<string, LabelClass>();
      for (const entry of queued) {
        const key = entry.className.toLowerCase();
        let target =
          byName.get(key) ?? classes.find((item) => item.name.toLowerCase() === key);
        if (!target) {
          target = await createClass(projectId, entry.className);
        }
        byName.set(key, target);
        await createExample(target.id, frame.id, entry.box, entry.role);
        saved.add(entry.id);
        lastClass = target;
      }
      setPending((rows) => rows.filter((entry) => !saved.has(entry.id)));
      if (current && saved.has(current.id)) setBox(null);
      setLibrary((rows) =>
        rows.map((entry) =>
          entry.id === frame.id ? { ...entry, examples: entry.examples + saved.size } : entry
        )
      );
      accounted = true;
      if (lastClass) {
        setSelected(lastClass);
        const rows = await listExamples(lastClass.id).catch(() => null);
        if (rows) setExamples(rows);
      }
      await refreshClasses(projectId);
    } catch (reason) {
      setPending((rows) => rows.filter((entry) => !saved.has(entry.id)));
      if (current && saved.has(current.id)) setBox(null);
      if (saved.size > 0 && !accounted) {
        setLibrary((rows) =>
          rows.map((entry) =>
            entry.id === frame.id ? { ...entry, examples: entry.examples + saved.size } : entry
          )
        );
        await refreshClasses(projectId).catch(() => {});
      }
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  function addBox() {
    if (!box || !className.trim()) return;
    const name = className.trim();
    pendingBoxId += 1;
    setPending((current) => [
      ...current,
      {
        id: `box-${pendingBoxId}`,
        box,
        className: name,
        role,
        color:
          classes.find((item) => item.name.toLowerCase() === name.toLowerCase())?.color ??
          "var(--primary)",
      },
    ]);
    setBox(null);
  }

  function removePending(id: string) {
    setPending((current) => current.filter((entry) => entry.id !== id));
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
  const unlabeled = library.filter((entry) => entry.examples === 0).length;
  const nextUnlabeled =
    library.find((entry) => entry.examples === 0 && entry.id !== frame?.id) ?? null;
  const readyCount = pending.length + (box && className.trim() ? 1 : 0);
  const pendingOverlay = pending.map((entry) => ({ box: entry.box, color: entry.color }));
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
        description="Grab a frame, draw a box, name it. Each save becomes an example for the matcher."
        action={
          <>
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
            <Button
              size="sm"
              className="gap-1.5"
              disabled={!projectId || busy || pending.length > 0}
              aria-keyshortcuts={grabShortcut ?? undefined}
              title={
                pending.length > 0
                  ? "Save or remove the boxes on this frame first"
                  : grabShortcut
                    ? `Grab frame (${grabShortcut})`
                    : "Grab frame"
              }
              onClick={grab}
            >
              <Crop className="size-4" aria-hidden="true" />
              {busy ? "Working" : "Grab frame"}
              {grabShortcut ? (
                <kbd className="rounded border border-current/30 px-1.5 text-xs font-medium opacity-70">
                  {grabShortcut}
                </kbd>
              ) : null}
            </Button>
          </>
        }
      />

      {exportInfo ? (
        <p className="border-b border-border p-2 text-xs text-muted-foreground tabular-nums">
          Exported {exportInfo.frames} frames, {exportInfo.boxes} boxes, {exportInfo.skipped}{" "}
          skipped.
        </p>
      ) : null}

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
                  Draw a box, add it, then draw the next object. The crop comes from the
                  full resolution frame, not the preview.
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
                <FrameStage
                  frame={frame}
                  box={box}
                  onBox={setBox}
                  overlay={pendingOverlay}
                  action={fullscreenButton}
                />
              ) : (
                <Skeleton className="aspect-video w-full" />
              )}
              {library.length > 0 ? (
                <div className="flex flex-col gap-1.5">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-xs text-muted-foreground">Captured</span>
                    <Badge variant="outline" className="tabular-nums">
                      {unlabeled} unlabeled
                    </Badge>
                    <Button
                      size="sm"
                      variant="outline"
                      disabled={busy || pending.length > 0 || !nextUnlabeled}
                      title={
                        pending.length > 0
                          ? "Save or remove the boxes on this frame first"
                          : "Open the next unlabeled frame"
                      }
                      onClick={() => {
                        if (nextUnlabeled) void open(nextUnlabeled.id);
                      }}
                    >
                      Next unlabeled
                    </Button>
                  </div>
                  <div className="flex gap-1.5 overflow-x-auto pb-1.5">
                    {library.map((entry) => (
                      <Button
                        key={entry.id}
                        size="icon"
                        variant={frame?.id === entry.id ? "secondary" : "outline"}
                        className="size-9 shrink-0"
                        aria-label={`Open frame ${entry.id}, ${entry.examples} labels`}
                        title={`Frame ${entry.id}, ${entry.examples} label${entry.examples === 1 ? "" : "s"}`}
                        disabled={busy || pending.length > 0}
                        onClick={() => open(entry.id)}
                      >
                        <img
                          src={entry.preview}
                          alt=""
                          loading="lazy"
                          decoding="async"
                          className="size-8 rounded-sm object-cover"
                        />
                      </Button>
                    ))}
                  </div>
                </div>
              ) : null}
              <div className="flex flex-col gap-2">
                <div className="flex flex-wrap items-start justify-between gap-2">
                  <p className="max-w-[52ch] text-xs text-muted-foreground">
                    {frame
                      ? box
                        ? `Box ${(box.w * 100).toFixed(1)}% wide, ${(box.h * 100).toFixed(1)}% tall. Add it to the frame, then draw the next object.`
                        : pending.length > 0
                          ? `${pending.length} box${pending.length === 1 ? "" : "es"} on this frame. Draw another, or save them together.`
                          : "No box yet. Shift-drag to pan, scroll to zoom, then drag over the object."
                      : grabShortcut
                        ? `Press Grab frame or ${grabShortcut}. iRecon hides its own windows; other windows stay in the shot.`
                        : "Press Grab frame. iRecon hides its own windows; other windows stay in the shot."}
                  </p>
                  <div className="flex flex-wrap items-center gap-1.5">
                    <span className="text-xs text-muted-foreground">Grab shortcut</span>
                    <Button
                      size="sm"
                      variant="outline"
                      className="min-w-24 gap-1.5"
                      aria-label="Change grab frame shortcut"
                      aria-pressed={recordingShortcut}
                      title="Click, then press a key. Backspace sets None. Escape cancels."
                      onClick={() => setRecordingShortcut((current) => !current)}
                      onBlur={() => setRecordingShortcut(false)}
                    >
                      <Keyboard className="size-4" aria-hidden="true" />
                      {recordingShortcut ? "Press a key" : grabShortcut ?? "None"}
                    </Button>
                    {grabShortcut ? (
                      <Button
                        size="sm"
                        variant="ghost"
                        title="Disable the grab frame shortcut"
                        onClick={() => setGrabShortcut(null)}
                      >
                        None
                      </Button>
                    ) : null}
                  </div>
                </div>
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
              </div>
            </CardContent>
          </Card>

          <Card className="min-w-0">
            <CardHeader>
              <CardTitle>Label boxes</CardTitle>
              <CardDescription>
                Pick or create a class, add each box, then save them together. A new name
                creates the class on save.
              </CardDescription>
            </CardHeader>
            <CardContent>
              <div className="flex flex-col gap-1.5">
                <label htmlFor="class-name" className="text-xs text-muted-foreground">
                  Class name
                </label>
                <Input
                  id="class-name"
                  value={className}
                  onChange={(event) => setClassName(event.target.value)}
                  placeholder="red car"
                  maxLength={40}
                />
              </div>
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
              {pending.length > 0 ? (
                <div className="flex flex-col gap-1.5">
                  <span className="text-xs text-muted-foreground">On this frame</span>
                  <ul className="overflow-hidden rounded-md border border-border">
                    {pending.map((entry, index) => (
                      <li key={entry.id}>
                        {index > 0 ? <Separator /> : null}
                        <div className="flex flex-wrap items-center gap-2 p-2">
                          <span
                            className="size-2 rounded-full"
                            style={{ backgroundColor: entry.color }}
                            aria-hidden="true"
                          />
                          <span className="min-w-0 flex-1 truncate text-sm">
                            {entry.className}
                          </span>
                          <Badge variant={entry.role === "positive" ? "secondary" : "outline"}>
                            {entry.role}
                          </Badge>
                          <span className="text-xs text-muted-foreground tabular-nums">
                            {Math.round(entry.box.w * 100)}% by {Math.round(entry.box.h * 100)}%
                          </span>
                          <Button
                            size="icon"
                            variant="ghost"
                            aria-label={`Remove ${entry.className} box`}
                            disabled={busy}
                            onClick={() => removePending(entry.id)}
                          >
                            <Trash2 className="size-4" aria-hidden="true" />
                          </Button>
                        </div>
                      </li>
                    ))}
                  </ul>
                </div>
              ) : null}
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
                  variant="outline"
                  className="gap-1.5"
                  disabled={!frame || !box || !className.trim() || busy}
                  onClick={addBox}
                >
                  <Plus className="size-4" aria-hidden="true" />
                  Add box
                </Button>
                <Button
                  size="sm"
                  disabled={!frame || readyCount === 0 || busy}
                  onClick={save}
                >
                  {readyCount === 1
                    ? "Save example"
                    : readyCount === 0
                      ? "Save examples"
                      : `Save ${readyCount} examples`}
                </Button>
              </div>
              <p className="text-xs text-muted-foreground">
                {pending.length > 0
                  ? `${pending.length} box${pending.length === 1 ? "" : "es"} on this frame. Save them together, or draw another.`
                  : role === "positive"
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
              How your positives and negatives compare. The suggestion moves the
              threshold only when you apply it.
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
                      <ul className="mt-1.5 flex list-disc flex-col gap-1.5 pl-2.5 text-xs text-muted-foreground">
                        {entry.hints.map((hint) => (
                          <li key={hint}>{hint}</li>
                        ))}
                      </ul>
                    ) : (
                      <p className="mt-1.5 text-xs text-muted-foreground">Looks healthy.</p>
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
            overlay={pendingOverlay}
            fitToFrame={false}
            className="h-[calc(100dvh-8rem)]"
            action={
              <span className="text-xs text-muted-foreground">
                Escape closes. Shift-drag to pan, scroll to zoom.
              </span>
            }
          />
        </div>
      ) : null}
    </div>
  );
}

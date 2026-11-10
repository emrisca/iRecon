import { useEffect, useState } from "react";
import {
  AppWindow,
  FileImage,
  FolderOpen,
  Monitor,
  Plus,
  RefreshCw,
  Trash2,
  type LucideIcon,
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
import { Separator } from "../components/ui/separator";
import { Skeleton } from "../components/ui/skeleton";
import { FrameStage } from "../components/frame-stage";
import { PageHeader } from "../components/page-header";
import {
  addSource,
  deleteSource,
  isDesktop,
  listDisplays,
  listProjects,
  listSources,
  listWindows,
  previewSource,
  setSourceCrop,
} from "../lib/api";
import type {
  Box,
  CaptureTarget,
  FramePayload,
  FrameView,
  Project,
  SourceKind,
  SourceView,
} from "../lib/types";

const KINDS: Array<{ id: SourceKind; label: string; icon: LucideIcon }> = [
  { id: "display", label: "Display", icon: Monitor },
  { id: "window", label: "Window", icon: AppWindow },
  { id: "file", label: "Image file", icon: FileImage },
  { id: "folder", label: "Folder", icon: FolderOpen },
];

const KIND_NAMES: Record<SourceKind, string> = {
  display: "Display",
  window: "Window",
  file: "Image file",
  folder: "Folder",
};

export function SourcesPage() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectId, setProjectId] = useState<number | null>(null);
  const [sources, setSources] = useState<SourceView[]>([]);
  const [kind, setKind] = useState<SourceKind>("display");
  const [targets, setTargets] = useState<CaptureTarget[]>([]);
  const [target, setTarget] = useState("");
  const [path, setPath] = useState("");
  const [loadingTargets, setLoadingTargets] = useState(false);
  const [active, setActive] = useState<SourceView | null>(null);
  const [preview, setPreview] = useState<FramePayload | null>(null);
  const [box, setBox] = useState<Box | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

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
      setSources([]);
      setActive(null);
      setPreview(null);
      setBox(null);
      return;
    }
    let alive = true;
    listSources(projectId)
      .then((rows) => {
        if (alive) setSources(rows);
      })
      .catch((reason: unknown) => {
        if (alive) setError(String(reason));
      });
    setActive(null);
    setPreview(null);
    setBox(null);
    return () => {
      alive = false;
    };
  }, [projectId]);

  useEffect(() => {
    if (kind !== "display" && kind !== "window") {
      setTargets([]);
      setTarget("");
      return;
    }
    let alive = true;
    setLoadingTargets(true);
    const load = kind === "display" ? listDisplays() : listWindows();
    load
      .then((rows) => {
        if (!alive) return;
        setTargets(rows);
        setTarget(rows[0]?.target ?? "");
      })
      .catch((reason: unknown) => {
        if (alive) setError(String(reason));
      })
      .finally(() => {
        if (alive) setLoadingTargets(false);
      });
    return () => {
      alive = false;
    };
  }, [kind]);

  async function refresh(id: number) {
    const rows = await listSources(id);
    setSources(rows);
    return rows;
  }

  async function select(source: SourceView) {
    setActive(source);
    setBox(source.crop ? { ...source.crop } : null);
    setPreview(null);
    setBusy(true);
    setError(null);
    try {
      const next = await previewSource(source.id);
      setPreview(next);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function add() {
    if (!projectId) return;
    setBusy(true);
    setError(null);
    try {
      const created = await addSource(
        projectId,
        kind,
        kind === "display" || kind === "window" ? target : path.trim()
      );
      await refresh(projectId);
      await select(created);
      setPath("");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function remove(id: number) {
    setBusy(true);
    setError(null);
    try {
      await deleteSource(id);
      if (active?.id === id) {
        setActive(null);
        setPreview(null);
        setBox(null);
      }
      if (projectId) await refresh(projectId);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function saveCrop() {
    if (!active || !box) return;
    setBusy(true);
    setError(null);
    try {
      const updated = await setSourceCrop(active.id, box);
      setActive(updated);
      setBox(updated.crop);
      if (projectId) await refresh(projectId);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function clearCrop() {
    if (!active) return;
    setBusy(true);
    setError(null);
    try {
      const updated = await setSourceCrop(active.id, null);
      setActive(updated);
      setBox(null);
      if (projectId) await refresh(projectId);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  const frame: FrameView | null =
    preview && active
      ? {
          id: active.id,
          path: "",
          width: preview.width,
          height: preview.height,
          preview: preview.data,
          createdAt: "",
          examples: 0,
        }
      : null;

  const canAdd =
    kind === "display" || kind === "window" ? target.length > 0 : path.trim().length > 0;

  return (
    <div className="flex min-h-full flex-col">
      <PageHeader
        title="Sources"
        description="Choose where frames come from. A display or window is live; an image or folder replays. A crop keeps only the region you need."
        action={
          <Button
            size="sm"
            variant="outline"
            className="gap-1.5"
            disabled={!projectId || busy}
            onClick={() => {
              if (projectId) void refresh(projectId);
            }}
          >
            <RefreshCw className="size-4" aria-hidden="true" />
            Refresh
          </Button>
        }
      />

      {!isDesktop ? (
        <p className="border-b border-border p-2 text-sm text-muted-foreground">
          Screen and window capture need the desktop app. The picker lists stand-in targets
          here so the flow can be walked through.
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

      <div className="grid gap-2.5 p-2.5 lg:grid-cols-[1fr_1.4fr]">
        <Card>
          <CardHeader>
            <CardTitle>Add a source</CardTitle>
            <CardDescription>
              Pick a display or a window, or point at an image or a folder of images.
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-2">
            <div className="flex flex-wrap gap-1.5">
              {KINDS.map((entry) => (
                <Button
                  key={entry.id}
                  size="sm"
                  variant={kind === entry.id ? "secondary" : "outline"}
                  className="gap-1.5"
                  onClick={() => setKind(entry.id)}
                >
                  <entry.icon className="size-4" aria-hidden="true" />
                  {entry.label}
                </Button>
              ))}
            </div>

            {kind === "display" || kind === "window" ? (
              loadingTargets ? (
                <Skeleton className="h-9 w-full" />
              ) : targets.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  {kind === "display"
                    ? "No displays were found."
                    : "No window with a title is open right now."}
                </p>
              ) : (
                <div className="flex flex-col gap-1.5">
                  {targets.map((entry) => (
                    <Button
                      key={entry.target}
                      variant={target === entry.target ? "secondary" : "outline"}
                      className="h-auto justify-start py-2"
                      onClick={() => setTarget(entry.target)}
                    >
                      <span className="flex min-w-0 flex-col items-start">
                        <span className="truncate text-sm">{entry.label}</span>
                        <span className="truncate text-xs text-muted-foreground">
                          {entry.detail}
                        </span>
                      </span>
                    </Button>
                  ))}
                </div>
              )
            ) : (
              <div className="flex flex-col gap-1.5">
                <label htmlFor="source-path" className="text-xs text-muted-foreground">
                  {kind === "folder" ? "Folder path" : "Image path"}
                </label>
                <Input
                  id="source-path"
                  value={path}
                  onChange={(event) => setPath(event.target.value)}
                  placeholder={kind === "folder" ? "C:\\shots\\traffic" : "C:\\shots\\frame.png"}
                />
              </div>
            )}

            <Button
              size="sm"
              className="gap-1.5 self-start"
              disabled={!projectId || busy || !canAdd}
              onClick={add}
            >
              <Plus className="size-4" aria-hidden="true" />
              Add source
            </Button>
          </CardContent>
        </Card>

        <Card className="min-w-0">
          <CardHeader className="flex flex-row items-start justify-between gap-2">
            <div className="flex flex-col gap-1.5">
              <CardTitle>Preview</CardTitle>
              <CardDescription>
                One frame from the selected source. Drag a box to keep part of it, or clear
                the crop to keep the whole frame.
              </CardDescription>
            </div>
            <div className="flex flex-wrap items-center gap-1.5">
              {preview ? (
                <Badge variant="outline" className="tabular-nums">
                  {preview.width} by {preview.height}
                </Badge>
              ) : null}
              {active ? <Badge variant="secondary">{active.label}</Badge> : null}
            </div>
          </CardHeader>
          <CardContent className="min-w-0">
            {frame ? (
              <FrameStage frame={frame} box={box} onBox={setBox} />
            ) : busy ? (
              <Skeleton className="aspect-video w-full" />
            ) : (
              <div className="flex aspect-video w-full items-center justify-center rounded-md border border-border bg-muted">
                <p className="max-w-[45ch] px-2.5 text-sm text-muted-foreground">
                  Add a source, then pick it from the list below to see a frame.
                </p>
              </div>
            )}
            <div className="flex flex-wrap items-center gap-2">
              <Button size="sm" disabled={!active || !box || busy} onClick={saveCrop}>
                Save crop
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={!active || busy}
                onClick={() => {
                  if (active) void select(active);
                }}
              >
                Refresh preview
              </Button>
              <Button
                size="sm"
                variant="ghost"
                disabled={!active || !box || busy}
                onClick={clearCrop}
              >
                Use whole frame
              </Button>
              <span className="text-xs text-muted-foreground tabular-nums">
                {box
                  ? `Crop ${Math.round(box.w * 100)}% by ${Math.round(box.h * 100)}% at ${Math.round(box.x * 100)}%, ${Math.round(box.y * 100)}%`
                  : "No crop, the whole frame is kept."}
              </span>
            </div>
          </CardContent>
        </Card>
      </div>

      <div className="px-2.5 pb-2.5">
        <Card>
          <CardHeader>
            <CardTitle>Project sources</CardTitle>
            <CardDescription>
              {sources.length === 0
                ? "No sources in this project yet."
                : `${sources.length === 1 ? "1 source" : `${sources.length} sources`} for this project. A source belongs to the project that picked it.`}
            </CardDescription>
          </CardHeader>
          <CardContent>
            {sources.length === 0 ? (
              <p className="text-sm text-muted-foreground">
                Add a display, a window, an image or a folder above.
              </p>
            ) : (
              <ul className="overflow-hidden rounded-lg border border-border">
                {sources.map((entry, index) => (
                  <li key={entry.id}>
                    {index > 0 ? <Separator /> : null}
                    <div className="flex flex-wrap items-center gap-2 p-2">
                      <Button
                        size="sm"
                        variant={active?.id === entry.id ? "secondary" : "ghost"}
                        onClick={() => {
                          void select(entry);
                        }}
                      >
                        {entry.label}
                      </Button>
                      <Badge variant="outline">{KIND_NAMES[entry.kind]}</Badge>
                      <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
                        {entry.target}
                      </span>
                      <Badge variant="secondary" className="tabular-nums">
                        {entry.crop ? `${Math.round(entry.crop.w * 100)}% wide` : "full frame"}
                      </Badge>
                      <Button
                        size="icon"
                        variant="ghost"
                        aria-label={`Delete ${entry.label}`}
                        disabled={busy}
                        onClick={() => remove(entry.id)}
                      >
                        <Trash2 className="size-4" aria-hidden="true" />
                      </Button>
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}

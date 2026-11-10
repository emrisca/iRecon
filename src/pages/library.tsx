import { useEffect, useState } from "react";
import { FolderInput, ImageUp, RefreshCw } from "lucide-react";

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
import { PageHeader } from "../components/page-header";
import {
  importFolder,
  importImages,
  isDesktop,
  listFrames,
  listProjects,
} from "../lib/api";
import { formatDate } from "../lib/utils";
import type { FrameView, ImportReport, Project } from "../lib/types";

export function LibraryPage() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectId, setProjectId] = useState<number | null>(null);
  const [frames, setFrames] = useState<FrameView[]>([]);
  const [selected, setSelected] = useState<FrameView | null>(null);
  const [paths, setPaths] = useState("");
  const [folder, setFolder] = useState("");
  const [report, setReport] = useState<ImportReport | null>(null);
  const [loading, setLoading] = useState(true);
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
      setFrames([]);
      setSelected(null);
      setLoading(false);
      return;
    }
    let alive = true;
    setLoading(true);
    listFrames(projectId)
      .then((rows) => {
        if (!alive) return;
        setFrames(rows);
        setSelected((current) => current ?? rows[0] ?? null);
      })
      .catch((reason: unknown) => {
        if (alive) setError(String(reason));
      })
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [projectId]);

  async function refresh(id: number) {
    const rows = await listFrames(id);
    setFrames(rows);
    return rows;
  }

  async function importPaths() {
    if (!projectId) return;
    const list = paths
      .split(",")
      .map((entry) => entry.trim())
      .filter(Boolean);
    if (list.length === 0) return;
    setBusy(true);
    setError(null);
    try {
      const result = await importImages(projectId, list);
      setReport(result);
      const rows = await refresh(projectId);
      setSelected(rows[0] ?? null);
      setPaths("");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function importFolderPath() {
    if (!projectId) return;
    const target = folder.trim();
    if (!target) return;
    setBusy(true);
    setError(null);
    try {
      const result = await importFolder(projectId, target);
      setReport(result);
      const rows = await refresh(projectId);
      setSelected(rows[0] ?? null);
      setFolder("");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex min-h-full flex-col">
      <PageHeader
        title="Library"
        description="Every frame the project can label, grabbed or imported. Imports are copied into the project folder."
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
          Imports need the desktop app. In this preview the paths stand in for files.
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
            <CardTitle>Import</CardTitle>
            <CardDescription>
              PNG and JPEG only. Each image becomes a frame the Label page can open, and
              nothing leaves this machine.
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-2">
            <div className="flex flex-col gap-1.5">
              <label htmlFor="image-paths" className="text-xs text-muted-foreground">
                Image paths
              </label>
              <Input
                id="image-paths"
                value={paths}
                onChange={(event) => setPaths(event.target.value)}
                placeholder="C:\\shots\\a.png, C:\\shots\\b.jpg"
              />
            </div>
            <Button
              size="sm"
              className="gap-1.5 self-start"
              disabled={!projectId || busy || paths.trim().length === 0}
              onClick={importPaths}
            >
              <ImageUp className="size-4" aria-hidden="true" />
              Import images
            </Button>

            <Separator />

            <div className="flex flex-col gap-1.5">
              <label htmlFor="import-folder" className="text-xs text-muted-foreground">
                Folder
              </label>
              <Input
                id="import-folder"
                value={folder}
                onChange={(event) => setFolder(event.target.value)}
                placeholder="C:\\shots\\traffic"
              />
            </div>
            <Button
              size="sm"
              variant="outline"
              className="gap-1.5 self-start"
              disabled={!projectId || busy || folder.trim().length === 0}
              onClick={importFolderPath}
            >
              <FolderInput className="size-4" aria-hidden="true" />
              Import a folder
            </Button>

            {report ? (
              <p className="text-xs text-muted-foreground">
                {report.imported} imported, {report.skipped} skipped.
                {report.errors.length > 0 ? ` ${report.errors.join(". ")}` : ""}
              </p>
            ) : null}
          </CardContent>
        </Card>

        <Card className="min-w-0">
          <CardHeader className="flex flex-row items-start justify-between gap-2">
            <div className="flex flex-col gap-1.5">
              <CardTitle>Selected frame</CardTitle>
              <CardDescription>
                Open any frame from the grid on the Label page to draw an example on it.
              </CardDescription>
            </div>
            {selected ? (
              <Badge variant="outline" className="tabular-nums">
                {selected.width} by {selected.height}
              </Badge>
            ) : null}
          </CardHeader>
          <CardContent>
            {selected ? (
              <div className="flex flex-col gap-2">
                <img
                  src={selected.preview}
                  alt={`Frame ${selected.id}`}
                  className="w-full rounded-md border border-border"
                />
                <p className="truncate text-xs text-muted-foreground">
                  {selected.path}
                </p>
              </div>
            ) : (
              <div className="flex aspect-video w-full items-center justify-center rounded-md border border-border bg-muted">
                <p className="max-w-[45ch] px-2.5 text-sm text-muted-foreground">
                  No frame yet. Grab one on the Label page, or import images above.
                </p>
              </div>
            )}
          </CardContent>
        </Card>
      </div>

      <div className="px-2.5 pb-2.5">
        <Card>
          <CardHeader>
            <CardTitle>Frames</CardTitle>
            <CardDescription>
              {frames.length === 1 ? "1 frame" : `${frames.length} frames`} in this project.
              Newest first.
            </CardDescription>
          </CardHeader>
          <CardContent>
            {loading ? (
              <div className="grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-5">
                <Skeleton className="aspect-video w-full" />
                <Skeleton className="aspect-video w-full" />
                <Skeleton className="aspect-video w-full" />
              </div>
            ) : frames.length === 0 ? (
              <p className="text-sm text-muted-foreground">
                The library is empty. Grab a frame on the Label page or import images above.
              </p>
            ) : (
              <ul className="grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-5">
                {frames.map((frame) => (
                  <li key={frame.id} className="media-tile">
                    <button
                      type="button"
                      className="flex w-full flex-col gap-1.5 text-left"
                      aria-pressed={selected?.id === frame.id}
                      onClick={() => setSelected(frame)}
                    >
                      <img
                        src={frame.preview}
                        alt={`Frame ${frame.id}`}
                        loading="lazy"
                        decoding="async"
                        className={
                          selected?.id === frame.id
                            ? "aspect-video w-full rounded-md border border-foreground object-cover"
                            : "aspect-video w-full rounded-md border border-border object-cover"
                        }
                      />
                      <span className="truncate text-xs text-muted-foreground tabular-nums">
                        {frame.width} by {frame.height}
                      </span>
                      <span className="truncate text-xs text-muted-foreground">
                        {formatDate(frame.createdAt)}
                      </span>
                    </button>
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

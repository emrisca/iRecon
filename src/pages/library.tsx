import { useEffect, useRef, useState } from "react";
import { Boxes, FolderInput, FolderOpen, ImageUp, RefreshCw, Trash2 } from "lucide-react";

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
import { useProject } from "../components/project-context";
import { useSidebar } from "../components/sidebar-context";
import {
  cancelRoboflowImport,
  deleteLabeledFrames,
  frameFullImage,
  frameImage,
  importFolder,
  importImages,
  importRoboflowDataset,
  isDesktop,
  listFrames,
  listFramesSince,
  onRoboflowProgress,
  pickFolder,
} from "../lib/api";
import { formatDate } from "../lib/utils";
import type {
  FrameView,
  ImportReport,
  RoboflowProgress,
  RoboflowReport,
} from "../lib/types";

function formatRemaining(seconds: number): string {
  const rounded = Math.max(1, Math.round(seconds));
  if (rounded < 60) return `${rounded}s`;
  const minutes = Math.floor(rounded / 60);
  if (minutes < 60) return `${minutes}m ${rounded % 60}s`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

export function LibraryPage() {
  const { projects, projectId, selectProject, refresh: refreshProjects } = useProject();
  const { live } = useSidebar();
  const [frames, setFrames] = useState<FrameView[]>([]);
  const [selected, setSelected] = useState<FrameView | null>(null);
  const [paths, setPaths] = useState("");
  const [folder, setFolder] = useState("");
  const [roboflowFolder, setRoboflowFolder] = useState("");
  const [roboflowReport, setRoboflowReport] = useState<RoboflowReport | null>(null);
  const [progress, setProgress] = useState<RoboflowProgress | null>(null);
  const [report, setReport] = useState<ImportReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmLabeled, setConfirmLabeled] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [frameLimit, setFrameLimit] = useState(400);
  const projectIdRef = useRef<number | null>(null);
  const lastSeenId = useRef(0);
  const lastLiveRefresh = useRef(0);
  const progressStart = useRef<{ at: number; processed: number } | null>(null);

  useEffect(() => {
    projectIdRef.current = projectId;
    setConfirmLabeled(false);
    setNotice(null);
    setFrameLimit(400);
  }, [projectId]);

  useEffect(() => {
    if (!projectId) {
      setFrames([]);
      setSelected(null);
      setLoading(false);
      return;
    }
    let alive = true;
    setLoading(true);
    listFrames(projectId, frameLimit)
      .then((rows) => {
        if (!alive) return;
        setFrames(rows);
        lastSeenId.current = rows.reduce(
          (max, entry) => Math.max(max, entry.id),
          0
        );
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
  }, [projectId, frameLimit]);

  useEffect(() => {
    if (!isDesktop) return;
    let alive = true;
    let off: (() => void) | null = null;
    void onRoboflowProgress((next) => {
      if (!alive) return;
      setProgress(next);
      if (!progressStart.current) {
        progressStart.current = { at: Date.now(), processed: next.processed };
      }
      // Only the frames that arrived since the last tick are fetched, so a
      // running import never rebuilds every preview on each update.
      const now = Date.now();
      const id = projectIdRef.current;
      if (id !== null && now - lastLiveRefresh.current > 1200) {
        lastLiveRefresh.current = now;
        void listFramesSince(id, lastSeenId.current, 100)
          .then((rows) => {
            if (!alive || rows.length === 0) return;
            lastSeenId.current = rows.reduce(
              (max, entry) => Math.max(max, entry.id),
              lastSeenId.current
            );
            setFrames((current) => {
              const known = new Set(current.map((entry) => entry.id));
              return [...rows.filter((entry) => !known.has(entry.id)), ...current];
            });
          })
          .catch(() => {});
      }
    }).then((cleanup) => {
      if (alive) off = cleanup;
      else cleanup();
    });
    return () => {
      alive = false;
      off?.();
    };
  }, []);

  async function refresh(id: number) {
    const rows = await listFrames(id, frameLimit);
    setFrames(rows);
    lastSeenId.current = rows.reduce(
      (max, entry) => Math.max(max, entry.id),
      0
    );
    return rows;
  }

  async function deleteLabeled() {
    if (!projectId) return;
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const count = await deleteLabeledFrames(projectId);
      const rows = await refresh(projectId);
      setSelected((current) =>
        current && rows.some((row) => row.id === current.id) ? current : rows[0] ?? null
      );
      setNotice(
        count === 0
          ? "No labeled screenshots to delete."
          : `Deleted ${count} labeled screenshot${count === 1 ? "" : "s"}. Saved example crops are unchanged.`
      );
      setConfirmLabeled(false);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
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

  async function browseFolder(setter: (value: string) => void) {
    try {
      const picked = await pickFolder();
      if (picked) setter(picked);
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function importRoboflow() {
    if (!projectId) return;
    const target = roboflowFolder.trim();
    if (!target) return;
    setBusy(true);
    setError(null);
    setRoboflowReport(null);
    setProgress(null);
    lastSeenId.current = frames.reduce(
      (max, entry) => Math.max(max, entry.id),
      0
    );
    progressStart.current = null;
    lastLiveRefresh.current = 0;
    try {
      const result = await importRoboflowDataset(projectId, target);
      setRoboflowReport(result);
      const rows = await refresh(projectId);
      setSelected(rows[0] ?? null);
      setRoboflowFolder("");
      await refreshProjects();
    } catch (reason) {
      setError(String(reason));
    } finally {
      progressStart.current = null;
      setProgress(null);
      setBusy(false);
    }
  }

  async function cancelRoboflow() {
    try {
      const stopped = await cancelRoboflowImport();
      if (!stopped) setProgress(null);
    } catch (reason) {
      setError(String(reason));
    }
  }

  const elapsedSeconds = progressStart.current
    ? (Date.now() - progressStart.current.at) / 1000
    : 0;
  const processedSinceStart = progress
    ? progress.processed - (progressStart.current?.processed ?? 0)
    : 0;
  const imagesPerSecond = elapsedSeconds > 1 ? processedSinceStart / elapsedSeconds : 0;
  const secondsLeft =
    imagesPerSecond > 0 && progress
      ? Math.max(0, progress.total - progress.processed) / imagesPerSecond
      : 0;
  const totalFrames =
    projects.find((project) => project.id === projectId)?.frames ?? frames.length;
  const labeledCount = frames.filter((frame) => frame.examples > 0).length;

  return (
    <div className="flex min-h-full flex-col">
      <PageHeader
        title="Library"
        description="Every frame the project can label, grabbed or imported. Imports are copied into the project folder."
        action={
          <div className="flex flex-wrap items-center gap-1.5">
            {confirmLabeled ? (
              <>
                <Button
                  size="sm"
                  variant="destructive"
                  disabled={busy || labeledCount === 0}
                  onClick={() => void deleteLabeled()}
                >
                  Delete {labeledCount} screenshot{labeledCount === 1 ? "" : "s"}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy}
                  onClick={() => setConfirmLabeled(false)}
                >
                  Cancel
                </Button>
              </>
            ) : (
              <Button
                size="sm"
                variant="outline"
                className="gap-1.5"
                title="Delete original screenshots that already have examples. The saved example crops stay."
                disabled={!projectId || busy || labeledCount === 0}
                onClick={() => setConfirmLabeled(true)}
              >
                <Trash2 className="size-4" aria-hidden="true" />
                Delete labeled
              </Button>
            )}
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
          </div>
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

      {notice ? (
        <p className="border-b border-border p-2 text-xs text-muted-foreground">{notice}</p>
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
              disabled={live}
              onClick={() => selectProject(project.id)}
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
              <div className="flex gap-1.5">
                <Input
                  id="import-folder"
                  value={folder}
                  onChange={(event) => setFolder(event.target.value)}
                  placeholder="C:\\shots\\traffic"
                />
                <Button
                  size="sm"
                  variant="outline"
                  className="h-9 shrink-0 gap-1.5"
                  disabled={!isDesktop || busy}
                  title="Choose a folder of images"
                  onClick={() => browseFolder(setFolder)}
                >
                  <FolderOpen className="size-4" aria-hidden="true" />
                  Browse
                </Button>
              </div>
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

            <Separator />

            <div className="flex flex-col gap-1.5">
              <label htmlFor="roboflow-folder" className="text-xs text-muted-foreground">
                Roboflow export folder
              </label>
              <div className="flex gap-1.5">
                <Input
                  id="roboflow-folder"
                  value={roboflowFolder}
                  onChange={(event) => setRoboflowFolder(event.target.value)}
                  placeholder="C:\\datasets\\overwatch-2-3"
                />
                <Button
                  size="sm"
                  variant="outline"
                  className="h-9 shrink-0 gap-1.5"
                  disabled={!isDesktop || busy}
                  title="Choose the folder holding data.yaml"
                  onClick={() => browseFolder(setRoboflowFolder)}
                >
                  <FolderOpen className="size-4" aria-hidden="true" />
                  Browse
                </Button>
              </div>
              <p className="text-xs text-muted-foreground">
                Download the dataset as YOLOv8 or YOLO26 TXT, unzip it, and point at the
                folder holding data.yaml. Class names come from that file.
              </p>
            </div>
            <Button
              size="sm"
              variant="outline"
              className="gap-1.5 self-start"
              disabled={!projectId || busy || roboflowFolder.trim().length === 0}
              onClick={importRoboflow}
            >
              <Boxes className="size-4" aria-hidden="true" />
              Import Roboflow dataset
            </Button>

            {progress ? (
              <div className="flex flex-col gap-1.5">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-xs text-muted-foreground tabular-nums">
                    {progress.processed} of {progress.total} images. {progress.frames} frames,{
                    " "}
                    {progress.examples} examples, {progress.skipped} skipped.
                    {imagesPerSecond > 0 && progress.processed < progress.total
                      ? ` ${imagesPerSecond.toFixed(1)} images/s, about ${formatRemaining(
                          secondsLeft
                        )} left.`
                      : ""}
                  </span>
                  <Button
                    size="sm"
                    variant="outline"
                    className="ml-auto"
                    onClick={cancelRoboflow}
                  >
                    Cancel import
                  </Button>
                </div>
                <div
                  role="progressbar"
                  aria-label="Roboflow import progress"
                  aria-valuemin={0}
                  aria-valuemax={progress.total}
                  aria-valuenow={progress.processed}
                  className="h-1.5 w-full overflow-hidden rounded-full bg-muted"
                >
                  <div
                    className="h-full bg-foreground transition-[width] duration-200"
                    style={{
                      width: `${
                        progress.total === 0
                          ? 0
                          : Math.round((progress.processed / progress.total) * 100)
                      }%`,
                    }}
                  />
                </div>
              </div>
            ) : null}

            {report ? (
              <p className="text-xs text-muted-foreground">
                {report.imported} imported, {report.skipped} skipped.
                {report.errors.length > 0 ? ` ${report.errors.join(". ")}` : ""}
              </p>
            ) : null}

            {roboflowReport ? (
              <p className="text-xs text-muted-foreground">
                {roboflowReport.frames} frames, {roboflowReport.examples} examples,{" "}
                {roboflowReport.classes} classes. {roboflowReport.skipped} skipped.
                {roboflowReport.cancelled ? " Import cancelled." : ""}
                {roboflowReport.errors.length > 0
                  ? ` ${roboflowReport.errors.join(". ")}`
                  : ""}
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
                  src={frameFullImage(selected)}
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
          <CardHeader className="flex flex-row items-start justify-between gap-2">
            <div className="flex flex-col gap-1.5">
              <CardTitle>Frames</CardTitle>
              <CardDescription>
                {frames.length === 1 ? "1 frame" : `${frames.length} frames`} shown
                {totalFrames > frames.length ? ` of ${totalFrames}` : ""}. Newest
                first.
              </CardDescription>
            </div>
            {totalFrames > frames.length ? (
              <Button
                size="sm"
                variant="outline"
                disabled={loading}
                onClick={() => setFrameLimit((current) => current + 400)}
              >
                Load older
              </Button>
            ) : null}
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
                        src={frameImage(frame)}
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

import { useEffect, useState } from "react";
import { Pause, Play, RefreshCw, RotateCcw, Trash2 } from "lucide-react";

import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../components/ui/card";
import { PageHeader } from "../components/page-header";
import { useProject } from "../components/project-context";
import { useSidebar } from "../components/sidebar-context";
import { Skeleton } from "../components/ui/skeleton";
import {
  deleteSession,
  evaluateSession,
  isDesktop,
  listSessionFrames,
  listSessions,
  sessionFrameImage,
} from "../lib/api";
import { formatDate } from "../lib/utils";
import type { Detection, SessionFrame, SessionView } from "../lib/types";

function duration(ms: number) {
  const seconds = Math.max(0, Math.round(ms / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

export function SessionsPage() {
  const { projects, projectId, selectProject } = useProject();
  const { live } = useSidebar();
  const [sessions, setSessions] = useState<SessionView[]>([]);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [frames, setFrames] = useState<SessionFrame[]>([]);
  const [frameIndex, setFrameIndex] = useState(0);
  const [speed, setSpeed] = useState(1);
  const [playing, setPlaying] = useState(false);
  const [image, setImage] = useState<string | null>(null);
  const [evaluated, setEvaluated] = useState<Map<number, Detection[]> | null>(null);
  const [loading, setLoading] = useState(true);
  const [framesLoading, setFramesLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!projectId) {
      setSessions([]);
      setSelectedId(null);
      setLoading(false);
      return;
    }
    let alive = true;
    setLoading(true);
    setError(null);
    listSessions(projectId)
      .then((rows) => {
        if (!alive) return;
        setSessions(rows);
        setSelectedId((current) =>
          rows.some((row) => row.id === current) ? current : rows[0]?.id ?? null
        );
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
  }, [projectId, refresh]);

  useEffect(() => {
    if (!selectedId) {
      setFrames([]);
      setFrameIndex(0);
      setPlaying(false);
      setEvaluated(null);
      return;
    }
    let alive = true;
    setFramesLoading(true);
    setError(null);
    listSessionFrames(selectedId)
      .then((rows) => {
        if (!alive) return;
        setFrames(rows);
        setFrameIndex(0);
        setPlaying(false);
        setEvaluated(null);
      })
      .catch((reason: unknown) => {
        if (alive) setError(String(reason));
      })
      .finally(() => {
        if (alive) setFramesLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [selectedId]);

  const current = frames[frameIndex];

  useEffect(() => {
    if (!current) {
      setImage(null);
      return;
    }
    let alive = true;
    setImage(null);
    sessionFrameImage(current.id)
      .then((data) => {
        if (alive) setImage(data);
      })
      .catch(() => {
        if (alive) setImage(null);
      });
    return () => {
      alive = false;
    };
  }, [current?.id]);

  useEffect(() => {
    if (!playing || !current) return;
    if (frameIndex >= frames.length - 1) {
      setPlaying(false);
      return;
    }
    const next = frames[frameIndex + 1];
    const delay = Math.max(40, Math.round((next.atMs - current.atMs) / speed));
    const timer = window.setTimeout(
      () => setFrameIndex((index) => Math.min(index + 1, frames.length - 1)),
      delay
    );
    return () => window.clearTimeout(timer);
  }, [playing, frameIndex, frames, speed, current?.atMs]);

  function togglePlay() {
    if (frames.length === 0) return;
    if (frameIndex >= frames.length - 1) setFrameIndex(0);
    setPlaying((value) => !value);
  }

  async function runEvaluation() {
    if (!selectedId) return;
    setBusy(true);
    setError(null);
    try {
      const rows = await evaluateSession(selectedId);
      setEvaluated(new Map(rows.map((row) => [row.frameId, row.boxes])));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function removeSession() {
    if (!selectedId) return;
    if (!window.confirm("Delete this recording and its frames?")) return;
    setBusy(true);
    setError(null);
    try {
      await deleteSession(selectedId);
      setSelectedId(null);
      setRefresh((value) => value + 1);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  const selected = sessions.find((session) => session.id === selectedId) ?? null;
  const boxes = current ? evaluated?.get(current.id) ?? current.boxes ?? [] : [];

  return (
    <div className="flex min-h-full flex-col">
      <PageHeader
        title="Sessions"
        description="Recorded capture with proposals per frame. Replay it, or re-score it with the current pool without launching the game."
        action={
          <Button
            size="sm"
            variant="outline"
            className="gap-1.5"
            disabled={!projectId || loading}
            onClick={() => setRefresh((value) => value + 1)}
          >
            <RefreshCw className="size-4" aria-hidden="true" />
            Refresh
          </Button>
        }
      />

      {!isDesktop ? (
        <p className="border-b border-border p-2 text-sm text-muted-foreground">
          Recording and replay need the desktop app. Open iRecon to record a capture session.
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
              disabled={live}
              onClick={() => selectProject(project.id)}
            >
              {project.name}
            </Button>
          ))
        )}
      </div>

      {loading ? (
        <div className="grid gap-2.5 p-2.5 lg:grid-cols-[18rem_1fr]">
          <Skeleton className="h-64 w-full" />
          <Skeleton className="h-64 w-full" />
        </div>
      ) : sessions.length === 0 ? (
        <div className="p-2.5">
          <div className="rounded-lg border border-border bg-muted p-2.5">
            <p className="text-sm font-medium">No recordings yet</p>
            <p className="mt-1.5 max-w-[65ch] text-xs text-muted-foreground">
              In Capture lab, enable Record session before starting capture. Frames and
              detector proposals are written to the project folder, and stop capture ends
              the session. Old recordings are pruned to a 1 GB project budget.
            </p>
          </div>
        </div>
      ) : (
        <div className="grid gap-2.5 p-2.5 lg:grid-cols-[18rem_1fr]">
          <Card className="min-w-0">
            <CardHeader>
              <CardTitle>Recordings</CardTitle>
              <CardDescription>
                {sessions.length === 1 ? "1 session" : `${sessions.length} sessions`} for
                this project.
              </CardDescription>
            </CardHeader>
            <CardContent className="flex flex-col gap-1.5">
              {sessions.map((session) => (
                <button
                  key={session.id}
                  type="button"
                  className={
                    session.id === selectedId
                      ? "flex flex-col gap-1.5 rounded-md border border-border bg-muted p-2 text-left"
                      : "flex flex-col gap-1.5 rounded-md border border-border p-2 text-left hover:bg-muted"
                  }
                  onClick={() => setSelectedId(session.id)}
                >
                  <span className="flex items-center gap-1.5">
                    <span className="text-sm font-medium">Session {session.id}</span>
                    <Badge variant="outline" className="ml-auto tabular-nums">
                      {session.frames} frames
                    </Badge>
                  </span>
                  <span className="text-xs text-muted-foreground">
                    {formatDate(session.startedAt)} · {duration(session.durationMs)}
                  </span>
                  <span className="truncate text-xs text-muted-foreground">
                    {session.model ?? "model not recorded"}
                  </span>
                </button>
              ))}
            </CardContent>
          </Card>

          <Card className="min-w-0">
            {selected ? (
              <>
                <CardHeader className="flex flex-row items-start justify-between gap-2">
                  <div className="flex flex-col gap-1.5">
                    <CardTitle>Session {selected.id}</CardTitle>
                    <CardDescription>
                      {selected.width} by {selected.height} · {duration(selected.durationMs)} ·{" "}
                      {selected.detections} detections recorded
                    </CardDescription>
                  </div>
                  <div className="flex flex-wrap items-center gap-1.5">
                    <Badge variant={evaluated ? "default" : "outline"}>
                      {evaluated ? "current pool" : "recorded"}
                    </Badge>
                    <Button
                      size="sm"
                      variant="ghost"
                      className="gap-1.5 text-destructive"
                      disabled={busy}
                      onClick={removeSession}
                    >
                      <Trash2 className="size-4" aria-hidden="true" />
                      Delete
                    </Button>
                  </div>
                </CardHeader>
                <CardContent className="flex flex-col gap-2">
                  {framesLoading ? (
                    <Skeleton className="aspect-video w-full" />
                  ) : current ? (
                    <>
                      <div className="relative overflow-hidden rounded-md border border-border bg-muted">
                        {image ? (
                          <img
                            src={image}
                            alt={`Recorded frame ${current.index + 1}`}
                            className="block w-full"
                          />
                        ) : (
                          <Skeleton className="aspect-video w-full" />
                        )}
                        {boxes.map((box, index) => (
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

                      <div className="flex flex-wrap items-center gap-2">
                        <Button
                          size="sm"
                          variant="outline"
                          className="gap-1.5"
                          onClick={togglePlay}
                        >
                          {playing ? (
                            <Pause className="size-4" aria-hidden="true" />
                          ) : (
                            <Play className="size-4" aria-hidden="true" />
                          )}
                          {playing ? "Pause" : "Play"}
                        </Button>
                        <Button
                          size="sm"
                          variant="ghost"
                          className="gap-1.5"
                          onClick={() => {
                            setPlaying(false);
                            setFrameIndex(0);
                          }}
                        >
                          <RotateCcw className="size-4" aria-hidden="true" />
                          Restart
                        </Button>
                        {[1, 10].map((value) => (
                          <Button
                            key={value}
                            size="sm"
                            variant={speed === value ? "secondary" : "ghost"}
                            onClick={() => setSpeed(value)}
                          >
                            {value}x
                          </Button>
                        ))}
                        <span className="ml-auto text-xs text-muted-foreground tabular-nums">
                          Frame {frameIndex + 1} / {frames.length} ·{" "}
                          {(current.atMs / 1000).toFixed(2)}s
                        </span>
                      </div>

                      <div className="flex flex-wrap items-center gap-2">
                        <Button
                          size="sm"
                          variant="outline"
                          className="gap-1.5"
                          disabled={busy}
                          onClick={runEvaluation}
                        >
                          <RefreshCw className="size-4" aria-hidden="true" />
                          Evaluate current pool
                        </Button>
                        <span className="text-xs text-muted-foreground">
                          Re-scores the recorded proposals with the current examples.
                        </span>
                      </div>

                      <div className="flex items-center gap-2">
                        <span className="w-24 shrink-0 text-xs text-muted-foreground tabular-nums">
                          Frame {current.index + 1}
                        </span>
                        <input
                          type="range"
                          min={0}
                          max={Math.max(0, frames.length - 1)}
                          value={frameIndex}
                          onChange={(event) => {
                            setPlaying(false);
                            setFrameIndex(Number(event.target.value));
                          }}
                          className="w-full accent-foreground"
                          aria-label="Recorded frame"
                        />
                      </div>
                    </>
                  ) : (
                    <p className="text-sm text-muted-foreground">
                      This session ended before a frame was recorded.
                    </p>
                  )}
                </CardContent>
              </>
            ) : (
              <CardContent>
                <p className="text-sm text-muted-foreground">Pick a recording to replay.</p>
              </CardContent>
            )}
          </Card>
        </div>
      )}
    </div>
  );
}

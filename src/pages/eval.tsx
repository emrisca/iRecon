import { useEffect, useMemo, useState } from "react";
import { Gauge, Play } from "lucide-react";

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
  evaluateFrames,
  frameImage,
  isDesktop,
  listClasses,
  listFrameTruth,
  listFrames,
  setFrameTruth,
} from "../lib/api";
import { formatDate } from "../lib/utils";
import type { EvalReport, FrameView, LabelClass } from "../lib/types";

function percent(value: number) {
  return `${Math.round(value * 100)}%`;
}

export function EvalPage() {
  const { projects, projectId, selectProject } = useProject();
  const { live } = useSidebar();
  const [frames, setFrames] = useState<FrameView[]>([]);
  const [frameLimit, setFrameLimit] = useState(400);
  const [classes, setClasses] = useState<LabelClass[]>([]);
  const [truth, setTruth] = useState<Record<number, number[]>>({});
  const [selected, setSelected] = useState<number[]>([]);
  const [report, setReport] = useState<EvalReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!projectId) {
      setFrames([]);
      setClasses([]);
      setTruth({});
      setSelected([]);
      setLoading(false);
      return;
    }
    let alive = true;
    setLoading(true);
    setError(null);
    setReport(null);
    Promise.all([
      listFrames(projectId, frameLimit),
      listClasses(projectId),
      listFrameTruth(projectId),
    ])
      .then(([frameRows, classRows, truthRows]) => {
        if (!alive) return;
        setFrames(frameRows);
        setClasses(classRows);
        const next: Record<number, number[]> = {};
        for (const row of truthRows) next[row.frameId] = row.classIds;
        setTruth(next);
        setSelected(frameRows.slice(0, 6).map((frame) => frame.id));
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

  async function loadOlder() {
    if (!projectId) return;
    const next = frameLimit + 400;
    try {
      const rows = await listFrames(projectId, next);
      setFrames(rows);
      setFrameLimit(next);
    } catch (reason) {
      setError(String(reason));
    }
  }

  const reportByFrame = useMemo(
    () => new Map((report?.frames ?? []).map((frame) => [frame.frameId, frame])),
    [report]
  );
  const classByName = useMemo(
    () => new Map(classes.map((entry) => [entry.name, entry])),
    [classes]
  );

  function toggleSelected(frameId: number) {
    setSelected((current) =>
      current.includes(frameId)
        ? current.filter((id) => id !== frameId)
        : [...current, frameId]
    );
    setReport(null);
  }

  async function toggleTruth(frameId: number, classId: number) {
    const present = !(truth[frameId] ?? []).includes(classId);
    setBusy(true);
    setError(null);
    setReport(null);
    try {
      await setFrameTruth(frameId, classId, present);
      setTruth((current) => {
        const ids = new Set(current[frameId] ?? []);
        if (present) ids.add(classId);
        else ids.delete(classId);
        return { ...current, [frameId]: [...ids] };
      });
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function run() {
    if (!projectId || selected.length === 0) return;
    setBusy(true);
    setError(null);
    try {
      setReport(await evaluateFrames(projectId, selected));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex min-h-full flex-col">
      <PageHeader
        title="Evaluate"
        description="Mark what each frame contains, then measure recall on a set you did not train on."
        action={
          <Button
            size="sm"
            className="gap-1.5"
            disabled={busy || selected.length === 0}
            onClick={run}
          >
            <Play className="size-4" aria-hidden="true" />
            Evaluate {selected.length}
          </Button>
        }
      />

      {!isDesktop ? (
        <p className="border-b border-border p-2 text-sm text-muted-foreground">
          Frame evaluation runs the local detector, which needs the desktop app. This
          preview shows the layout with empty results.
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
        <div className="ml-auto flex items-center gap-2">
          {frames.length >= frameLimit ? (
            <Button size="sm" variant="outline" onClick={loadOlder}>
              Load older
            </Button>
          ) : null}
          <Badge variant="secondary" className="tabular-nums">
            {selected.length} of {frames.length} frames
          </Badge>
        </div>
      </div>

      {loading ? (
        <div className="grid gap-2.5 p-2.5 md:grid-cols-2">
          <Skeleton className="h-64 w-full" />
          <Skeleton className="h-64 w-full" />
        </div>
      ) : frames.length === 0 ? (
        <div className="p-2.5">
          <div className="rounded-lg border border-border bg-muted p-2.5">
            <p className="text-sm font-medium">No frames to evaluate</p>
            <p className="mt-1.5 max-w-[65ch] text-xs text-muted-foreground">
              Grab or import frames on the Library page first. Held-out frames work
              best: label a different set than the one you drew examples on.
            </p>
          </div>
        </div>
      ) : (
        <div className="flex flex-col gap-2.5 p-2.5">
          {report ? (
            <Card>
              <CardHeader>
                <CardTitle>Recall</CardTitle>
                <CardDescription>
                  {report.matches} boxes across {report.framesWithMatch} of{" "}
                  {report.frames.length} frames, from {report.proposals} proposals.
                  False positives count boxes on frames that were not marked with
                  that class.
                </CardDescription>
              </CardHeader>
              <CardContent>
                {classes.length === 0 ? (
                  <p className="text-sm text-muted-foreground">
                    This project has no classes yet.
                  </p>
                ) : (
                  <table className="w-full text-sm">
                    <thead>
                      <tr className="text-left text-xs text-muted-foreground">
                        <th className="pb-1.5 font-medium">Class</th>
                        <th className="pb-1.5 text-right font-medium">Marked</th>
                        <th className="pb-1.5 text-right font-medium">Hit</th>
                        <th className="pb-1.5 text-right font-medium">Recall</th>
                        <th className="pb-1.5 text-right font-medium">False pos.</th>
                      </tr>
                    </thead>
                    <tbody>
                      {report.classRecall.map((row) => (
                        <tr key={row.classId} className="border-t border-border">
                          <td className="py-1.5">
                            <span className="flex items-center gap-1.5">
                              <span
                                className="size-2 shrink-0 rounded-full"
                                style={{ backgroundColor: row.color }}
                              />
                              {row.name}
                            </span>
                          </td>
                          <td className="py-1.5 text-right tabular-nums">{row.marked}</td>
                          <td className="py-1.5 text-right tabular-nums">{row.hits}</td>
                          <td className="py-1.5 text-right tabular-nums">
                            {row.marked === 0 ? "n/a" : percent(row.hits / row.marked)}
                          </td>
                          <td className="py-1.5 text-right tabular-nums">
                            {row.falsePositives}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                )}
                {report.classRecall.every((row) => row.marked === 0) ? (
                  <p className="mt-2 text-xs text-muted-foreground">
                    Nothing is marked yet, so recall is undefined. Toggle a class on a
                    frame below, then run again.
                  </p>
                ) : null}
              </CardContent>
            </Card>
          ) : (
            <div className="rounded-lg border border-border bg-muted p-2.5">
              <p className="flex items-center gap-1.5 text-sm font-medium">
                <Gauge className="size-4" aria-hidden="true" />
                Ground truth first
              </p>
              <p className="mt-1.5 max-w-[75ch] text-xs text-muted-foreground">
                Every frame starts unmarked. Click the class badges under a frame to
                record what it contains, check the frames to include, then press
                Evaluate. Marks persist, so a held-out set only needs labelling once.
              </p>
            </div>
          )}

          <div className="grid gap-2.5 md:grid-cols-2 xl:grid-cols-3">
            {frames.map((frame) => {
              const result = reportByFrame.get(frame.id);
              const isSelected = selected.includes(frame.id);
              const marked = truth[frame.id] ?? [];
              return (
                <Card key={frame.id} className="min-w-0 overflow-hidden">
                  <CardHeader className="gap-1.5">
                    <div className="flex items-center gap-2">
                      <input
                        type="checkbox"
                        className="size-4 accent-primary"
                        aria-label={`Evaluate frame ${frame.id}`}
                        checked={isSelected}
                        onChange={() => toggleSelected(frame.id)}
                      />
                      <CardTitle className="text-sm">Frame {frame.id}</CardTitle>
                      <span className="ml-auto flex items-center gap-1.5">
                        {result ? (
                          <Badge variant={result.matches ? "secondary" : "outline"}>
                            {result.matches} hit
                          </Badge>
                        ) : null}
                        {result?.bestScore != null ? (
                          <Badge variant="outline" className="tabular-nums">
                            best {percent(result.bestScore)}
                          </Badge>
                        ) : null}
                      </span>
                    </div>
                    <CardDescription>
                      {frame.width} by {frame.height} · {formatDate(frame.createdAt)}
                    </CardDescription>
                  </CardHeader>
                  <CardContent className="flex flex-col gap-2">
                    <div className="relative overflow-hidden rounded-md border border-border">
                      <img
                        src={frameImage(frame)}
                        alt={`Frame ${frame.id}`}
                        loading="lazy"
                        decoding="async"
                        className="block w-full"
                      />
                      {result?.matchedBoxes.map((box, index) => (
                        <div
                          key={`${box.label}-${index}`}
                          className="pointer-events-none absolute border-2"
                          style={{
                            left: `${box.x * 100}%`,
                            top: `${box.y * 100}%`,
                            width: `${box.w * 100}%`,
                            height: `${box.h * 100}%`,
                            borderColor:
                              classByName.get(box.label)?.color ?? "currentColor",
                          }}
                        >
                          <span className="absolute -top-5 left-0 whitespace-nowrap bg-background/90 px-1.5 text-xs leading-4">
                            {box.label} {percent(box.score)}
                          </span>
                        </div>
                      ))}
                    </div>
                    <div className="flex flex-wrap gap-1.5">
                      {classes.length === 0 ? (
                        <span className="text-xs text-muted-foreground">
                          No classes to mark.
                        </span>
                      ) : (
                        classes.map((entry) => {
                          const present = marked.includes(entry.id);
                          return (
                            <button
                              key={entry.id}
                              type="button"
                              aria-pressed={present}
                              disabled={busy}
                              className={
                                present
                                  ? "rounded-md border px-2 py-1.5 text-xs font-medium"
                                  : "rounded-md border border-border px-2 py-1.5 text-xs text-muted-foreground"
                              }
                              style={
                                present
                                  ? { borderColor: entry.color, color: entry.color }
                                  : undefined
                              }
                              onClick={() => toggleTruth(frame.id, entry.id)}
                            >
                              {present ? "✓ " : ""}
                              {entry.name}
                            </button>
                          );
                        })
                      )}
                    </div>
                  </CardContent>
                </Card>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}

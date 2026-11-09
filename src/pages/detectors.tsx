import { useEffect, useState } from "react";
import { RefreshCw, Trash2 } from "lucide-react";

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
import { Skeleton } from "../components/ui/skeleton";
import {
  activateProjectDetector,
  compareDetectors,
  deactivateProjectDetector,
  deleteProjectDetector,
  importProjectDetector,
  isDesktop,
  listProjectDetectors,
  listProjects,
  listSessions,
} from "../lib/api";
import { formatDate } from "../lib/utils";
import type {
  DetectorComparison,
  Project,
  ProjectDetector,
  SessionView,
} from "../lib/types";

export function DetectorsPage() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectId, setProjectId] = useState<number | null>(null);
  const [detectors, setDetectors] = useState<ProjectDetector[]>([]);
  const [sessions, setSessions] = useState<SessionView[]>([]);
  const [detectorId, setDetectorId] = useState<number | null>(null);
  const [sessionId, setSessionId] = useState<number | null>(null);
  const [path, setPath] = useState("");
  const [name, setName] = useState("");
  const [comparison, setComparison] = useState<DetectorComparison | null>(null);
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
      setDetectors([]);
      setSessions([]);
      setLoading(false);
      return;
    }
    let alive = true;
    setLoading(true);
    setError(null);
    setComparison(null);
    Promise.all([listProjectDetectors(projectId), listSessions(projectId)])
      .then(([detectorRows, sessionRows]) => {
        if (!alive) return;
        setDetectors(detectorRows);
        setSessions(sessionRows);
        setDetectorId((current) =>
          detectorRows.some((row) => row.id === current)
            ? current
            : detectorRows[0]?.id ?? null
        );
        setSessionId((current) =>
          sessionRows.some((row) => row.id === current)
            ? current
            : sessionRows[0]?.id ?? null
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
  }, [projectId]);

  async function importDetector() {
    if (!projectId || !path.trim()) return;
    setBusy(true);
    setError(null);
    try {
      const row = await importProjectDetector(projectId, path.trim(), name.trim() || undefined);
      setDetectors((current) => [row, ...current]);
      setDetectorId(row.id);
      setPath("");
      setName("");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function compare() {
    if (!projectId || !detectorId || !sessionId) return;
    setBusy(true);
    setError(null);
    try {
      setComparison(await compareDetectors(projectId, detectorId, sessionId));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function setActive(id: number, active: boolean) {
    if (!projectId) return;
    setBusy(true);
    setError(null);
    try {
      if (active) await deactivateProjectDetector(projectId);
      else await activateProjectDetector(projectId, id);
      setDetectors((current) =>
        current.map((row) => ({ ...row, active: active ? false : row.id === id }))
      );
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function remove(id: number) {
    if (!window.confirm("Delete this imported detector file?")) return;
    setBusy(true);
    setError(null);
    try {
      await deleteProjectDetector(id);
      setDetectors((current) => current.filter((row) => row.id !== id));
      if (detectorId === id) setDetectorId(null);
      setComparison(null);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex min-h-full flex-col">
      <PageHeader
        title="Detectors"
        description="Import a project-trained ONNX model, validate its input and output contract, warm it up, then compare it with the shipped detector on the same recorded session."
      />

      {!isDesktop ? (
        <p className="border-b border-border p-2 text-sm text-muted-foreground">
          Importing and comparing detectors needs the desktop app.
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

      {loading ? (
        <div className="grid gap-2.5 p-2.5 lg:grid-cols-[20rem_1fr]">
          <Skeleton className="h-64 w-full" />
          <Skeleton className="h-64 w-full" />
        </div>
      ) : (
        <div className="grid gap-2.5 p-2.5 lg:grid-cols-[20rem_1fr]">
          <div className="flex min-w-0 flex-col gap-2.5">
            <Card>
              <CardHeader>
                <CardTitle>Import ONNX</CardTitle>
                <CardDescription>
                  The model must take a square NCHW float tensor named input and expose
                  dets and labels outputs.
                </CardDescription>
              </CardHeader>
              <CardContent className="flex flex-col gap-2">
                <input
                  value={path}
                  onChange={(event) => setPath(event.target.value)}
                  placeholder="C:\\models\\robot.onnx"
                  aria-label="ONNX model path"
                  className="h-8 rounded-md border border-input bg-transparent px-2 text-sm"
                />
                <input
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  placeholder="Name (optional)"
                  aria-label="Detector name"
                  className="h-8 rounded-md border border-input bg-transparent px-2 text-sm"
                />
                <Button
                  size="sm"
                  disabled={busy || !path.trim()}
                  onClick={importDetector}
                >
                  {busy ? "Checking model..." : "Import and warm up"}
                </Button>
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle>Project models</CardTitle>
                <CardDescription>
                  {detectors.length === 1 ? "1 imported detector" : `${detectors.length} imported detectors`}
                </CardDescription>
              </CardHeader>
              <CardContent className="flex flex-col gap-1.5">
                {detectors.length === 0 ? (
                  <p className="text-sm text-muted-foreground">No project detector yet.</p>
                ) : (
                  detectors.map((row) => (
                    <div
                      key={row.id}
                      className={
                        row.id === detectorId
                          ? "flex items-start gap-2 rounded-md border border-border bg-muted p-2"
                          : "flex items-start gap-2 rounded-md border border-border p-2"
                      }
                    >
                      <button
                        type="button"
                        className="flex min-w-0 flex-1 flex-col gap-0.5 text-left"
                        onClick={() => setDetectorId(row.id)}
                      >
                        <span className="truncate text-sm font-medium">{row.name}</span>
                        <span className="text-xs text-muted-foreground">
                          {row.inputSize} px · {formatDate(row.createdAt)}
                        </span>
                      </button>
                      <Button
                        size="sm"
                        variant={row.active ? "secondary" : "outline"}
                        disabled={busy}
                        onClick={() => setActive(row.id, row.active)}
                      >
                        {row.active ? "Deactivate" : "Use live"}
                      </Button>
                      <Button
                        size="icon"
                        variant="ghost"
                        aria-label={`Delete ${row.name}`}
                        disabled={busy}
                        onClick={() => remove(row.id)}
                      >
                        <Trash2 className="size-4" aria-hidden="true" />
                      </Button>
                    </div>
                  ))
                )}
              </CardContent>
            </Card>
          </div>

          <Card className="min-w-0">
            <CardHeader className="flex flex-row items-start justify-between gap-2">
              <div className="flex flex-col gap-1.5">
                <CardTitle>Compare on a recording</CardTitle>
                <CardDescription>
                  Both detectors run over the same recorded frames and score their
                  proposals against the current matcher pool.
                </CardDescription>
              </div>
              <Badge variant="outline">{sessions.length} sessions</Badge>
            </CardHeader>
            <CardContent className="flex flex-col gap-2">
              {sessions.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  Record a session on Capture lab first.
                </p>
              ) : (
                <>
                  <div className="flex flex-wrap items-center gap-2">
                    <select
                      value={detectorId ?? ""}
                      onChange={(event) => setDetectorId(Number(event.target.value) || null)}
                      aria-label="Project detector"
                      className="h-8 rounded-md border border-input bg-transparent px-2 text-sm"
                    >
                      <option value="">Pick a detector</option>
                      {detectors.map((row) => (
                        <option key={row.id} value={row.id}>
                          {row.name}
                        </option>
                      ))}
                    </select>
                    <select
                      value={sessionId ?? ""}
                      onChange={(event) => setSessionId(Number(event.target.value) || null)}
                      aria-label="Recorded session"
                      className="h-8 rounded-md border border-input bg-transparent px-2 text-sm"
                    >
                      <option value="">Pick a session</option>
                      {sessions.map((row) => (
                        <option key={row.id} value={row.id}>
                          Session {row.id} · {row.frames} frames
                        </option>
                      ))}
                    </select>
                    <Button
                      size="sm"
                      className="gap-1.5"
                      disabled={busy || !detectorId || !sessionId}
                      onClick={compare}
                    >
                      <RefreshCw className="size-4" aria-hidden="true" />
                      {busy ? "Running..." : "Compare"}
                    </Button>
                  </div>

                  {comparison ? (
                    <>
                      <div className="grid gap-2 md:grid-cols-2">
                        {[comparison.shipped, comparison.project].map((side) => (
                          <div key={side.name} className="rounded-lg border border-border p-2">
                            <span className="text-sm font-medium">{side.name}</span>
                            <span className="mt-0.5 block text-xs text-muted-foreground">
                              {side.inputSize} px · {side.frames} frames ·{" "}
                              {side.averageMs.toFixed(1)} ms average
                            </span>
                            <span className="mt-1 block text-xs tabular-nums">
                              {side.proposals} proposals · {side.matches} matches
                            </span>
                          </div>
                        ))}
                      </div>
                      <div className="overflow-x-auto">
                        <table className="w-full min-w-[32rem] text-sm">
                          <thead>
                            <tr className="text-left text-xs text-muted-foreground">
                              <th className="pb-1.5 font-medium">Frame</th>
                              <th className="pb-1.5 text-right font-medium">Shipped</th>
                              <th className="pb-1.5 text-right font-medium">Project</th>
                              <th className="pb-1.5 text-right font-medium">Shipped ms</th>
                              <th className="pb-1.5 text-right font-medium">Project ms</th>
                            </tr>
                          </thead>
                          <tbody>
                            {comparison.frames.map((row) => (
                              <tr key={row.frameId} className="border-t border-border">
                                <td className="py-1.5 tabular-nums">{row.index + 1}</td>
                                <td className="py-1.5 text-right tabular-nums">
                                  {row.shippedMatches}/{row.shippedProposals}
                                </td>
                                <td className="py-1.5 text-right tabular-nums">
                                  {row.projectMatches}/{row.projectProposals}
                                </td>
                                <td className="py-1.5 text-right tabular-nums">
                                  {row.shippedMs.toFixed(0)}
                                </td>
                                <td className="py-1.5 text-right tabular-nums">
                                  {row.projectMs.toFixed(0)}
                                </td>
                              </tr>
                            ))}
                          </tbody>
                        </table>
                      </div>
                    </>
                  ) : null}
                </>
              )}
            </CardContent>
          </Card>
        </div>
      )}
    </div>
  );
}

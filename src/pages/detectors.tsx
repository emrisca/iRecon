import { useEffect, useState } from "react";
import { RefreshCw, Trash2 } from "lucide-react";

import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
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
  activateProjectClassifier,
  activateProjectDetector,
  compareDetectors,
  deactivateProjectClassifier,
  deactivateProjectDetector,
  deleteProjectClassifier,
  deleteProjectDetector,
  importProjectClassifier,
  importProjectDetector,
  isDesktop,
  listProjectClassifiers,
  listProjectDetectors,
  listSessions,
} from "../lib/api";
import { formatDate } from "../lib/utils";
import type {
  DetectorComparison,
  ProjectClassifier,
  ProjectDetector,
  SessionView,
} from "../lib/types";

export function DetectorsPage() {
  const { projects, projectId, selectProject } = useProject();
  const { live } = useSidebar();
  const [detectors, setDetectors] = useState<ProjectDetector[]>([]);
  const [classifiers, setClassifiers] = useState<ProjectClassifier[]>([]);
  const [classifierPath, setClassifierPath] = useState("");
  const [classifierName, setClassifierName] = useState("");
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
    if (!projectId) {
      setDetectors([]);
      setClassifiers([]);
      setSessions([]);
      setLoading(false);
      return;
    }
    let alive = true;
    setLoading(true);
    setError(null);
    setComparison(null);
    Promise.all([
      listProjectDetectors(projectId),
      listSessions(projectId),
      listProjectClassifiers(projectId),
    ])
      .then(([detectorRows, sessionRows, classifierRows]) => {
        if (!alive) return;
        setDetectors(detectorRows);
        setClassifiers(classifierRows);
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

  async function importClassifier() {
    if (!projectId || !classifierPath.trim()) return;
    setBusy(true);
    setError(null);
    try {
      const row = await importProjectClassifier(
        projectId,
        classifierPath.trim(),
        classifierName.trim() || undefined
      );
      setClassifiers((current) => [row, ...current]);
      setClassifierPath("");
      setClassifierName("");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function setClassifierActive(id: number, active: boolean) {
    if (!projectId) return;
    setBusy(true);
    setError(null);
    try {
      if (active) await deactivateProjectClassifier(projectId);
      else await activateProjectClassifier(projectId, id);
      setClassifiers((current) =>
        current.map((row) => ({ ...row, active: active ? false : row.id === id }))
      );
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function removeClassifier(id: number) {
    if (!window.confirm("Delete this imported classifier file?")) return;
    setBusy(true);
    setError(null);
    try {
      await deleteProjectClassifier(id);
      setClassifiers((current) => current.filter((row) => row.id !== id));
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
        description="Import project ONNX detectors and crop classifiers, check their contracts, then compare a detector with the shipped model on a recording."
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
              disabled={live}
              onClick={() => selectProject(project.id)}
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
                  RF-DETR exports need a square NCHW input named input plus dets and
                  labels outputs. A stock YOLO export with one box and class score
                  output also imports. Custom RF-DETR slots can be named in a
                  labels.json file beside the model.
                </CardDescription>
              </CardHeader>
              <CardContent className="flex flex-col gap-2">
                <div className="flex flex-col gap-1.5">
                  <label htmlFor="detector-path" className="text-xs text-muted-foreground">
                    Model file
                  </label>
                  <Input
                    id="detector-path"
                    value={path}
                    onChange={(event) => setPath(event.target.value)}
                    placeholder="C:\\models\\robot.onnx"
                  />
                </div>
                <div className="flex flex-col gap-1.5">
                  <label htmlFor="detector-name" className="text-xs text-muted-foreground">
                    Name
                  </label>
                  <Input
                    id="detector-name"
                    value={name}
                    onChange={(event) => setName(event.target.value)}
                    placeholder="Robot detector"
                    maxLength={64}
                  />
                </div>
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
                  {detectors.length === 0
                    ? "No project detector yet."
                    : detectors.length === 1
                      ? "1 imported detector"
                      : `${detectors.length} imported detectors`}
                </CardDescription>
              </CardHeader>
              <CardContent className="flex flex-col gap-1.5">
                {detectors.length === 0 ? null : (
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
                        className="flex min-w-0 flex-1 flex-col gap-1.5 text-left"
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

            <Card>
              <CardHeader>
                <CardTitle>Project classifier</CardTitle>
                <CardDescription>
                  Scores each live proposal with your ONNX crop classifier instead of the
                  fingerprint pool. Labels come from a JSON array beside the model, otherwise
                  from the project classes in id order.
                </CardDescription>
              </CardHeader>
              <CardContent className="flex flex-col gap-2">
                <div className="flex flex-col gap-1.5">
                  <label
                    htmlFor="classifier-path"
                    className="text-xs text-muted-foreground"
                  >
                    Model file
                  </label>
                  <Input
                    id="classifier-path"
                    value={classifierPath}
                    onChange={(event) => setClassifierPath(event.target.value)}
                    placeholder="C:\\models\\robot.onnx"
                  />
                </div>
                <div className="flex flex-col gap-1.5">
                  <label
                    htmlFor="classifier-name"
                    className="text-xs text-muted-foreground"
                  >
                    Name
                  </label>
                  <Input
                    id="classifier-name"
                    value={classifierName}
                    onChange={(event) => setClassifierName(event.target.value)}
                    placeholder="Crop classifier"
                    maxLength={64}
                  />
                </div>
                <Button
                  size="sm"
                  disabled={busy || !classifierPath.trim()}
                  onClick={importClassifier}
                >
                  {busy ? "Checking model..." : "Import and warm up"}
                </Button>

                {classifiers.length === 0 ? (
                  <p className="text-sm text-muted-foreground">
                    No project classifier yet.
                  </p>
                ) : (
                  classifiers.map((row) => (
                    <div
                      key={row.id}
                      className="flex items-start gap-2 rounded-md border border-border p-2"
                    >
                      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
                        <span className="truncate text-sm font-medium">{row.name}</span>
                        <span className="text-xs text-muted-foreground">
                          {row.inputSize} px · {row.labels.length} labels ·{" "}
                          {formatDate(row.createdAt)}
                        </span>
                      </div>
                      <Button
                        size="sm"
                        variant={row.active ? "secondary" : "outline"}
                        disabled={busy || live}
                        onClick={() => setClassifierActive(row.id, row.active)}
                      >
                        {row.active ? "Deactivate" : "Use live"}
                      </Button>
                      <Button
                        size="icon"
                        variant="ghost"
                        aria-label={`Delete ${row.name}`}
                        disabled={busy}
                        onClick={() => removeClassifier(row.id)}
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
                  <div className="grid gap-2 sm:grid-cols-2">
                    <div className="flex min-w-0 flex-col gap-1.5">
                      <label
                        htmlFor="compare-detector"
                        className="text-xs text-muted-foreground"
                      >
                        Detector
                      </label>
                      <select
                        id="compare-detector"
                        value={detectorId ?? ""}
                        onChange={(event) => setDetectorId(Number(event.target.value) || null)}
                        className="h-9 w-full rounded-md border border-input bg-background px-2.5 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
                      >
                        <option value="">Pick a detector</option>
                        {detectors.map((row) => (
                          <option key={row.id} value={row.id}>
                            {row.name}
                          </option>
                        ))}
                      </select>
                    </div>
                    <div className="flex min-w-0 flex-col gap-1.5">
                      <label
                        htmlFor="compare-session"
                        className="text-xs text-muted-foreground"
                      >
                        Session
                      </label>
                      <select
                        id="compare-session"
                        value={sessionId ?? ""}
                        onChange={(event) => setSessionId(Number(event.target.value) || null)}
                        className="h-9 w-full rounded-md border border-input bg-background px-2.5 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
                      >
                        <option value="">Pick a session</option>
                        {sessions.map((row) => (
                          <option key={row.id} value={row.id}>
                            Session {row.id} · {row.frames} frames
                          </option>
                        ))}
                      </select>
                    </div>
                  </div>
                  <Button
                    size="sm"
                    className="gap-1.5 self-start"
                    disabled={busy || !detectorId || !sessionId}
                    onClick={compare}
                  >
                    <RefreshCw className="size-4" aria-hidden="true" />
                    {busy ? "Comparing" : "Compare"}
                  </Button>

                  {comparison ? (
                    <>
                      <div className="grid gap-2 md:grid-cols-2">
                        {[comparison.shipped, comparison.project].map((side) => (
                          <div key={side.name} className="rounded-lg border border-border p-2">
                            <span className="text-sm font-medium">{side.name}</span>
                            <span className="mt-1.5 block text-xs text-muted-foreground">
                              {side.inputSize} px · {side.frames} frames ·{" "}
                              {side.averageMs.toFixed(1)} ms average
                            </span>
                            <span className="mt-1.5 block text-xs tabular-nums">
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

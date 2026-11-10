import { useEffect, useRef, useState } from "react";
import { Check, SkipForward, Trash2, X } from "lucide-react";

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
  autoAcceptCandidates,
  candidatePreview,
  deleteAllCandidates,
  deleteCandidate,
  exampleImage,
  getSetting,
  isDesktop,
  listCandidates,
  nearestExamples,
  resolveCandidate,
  setSetting,
} from "../lib/api";
import { formatDate } from "../lib/utils";
import type { CandidateView, ExampleView } from "../lib/types";
import { candidateImage } from "../lib/api";

const QUEUE_PAGE_SIZE = 40;
const QUEUE_FETCH = 400;

export function ReviewPage() {
  const { projects, projectId, selectProject } = useProject();
  const { live } = useSidebar();
  const [queue, setQueue] = useState<{
    candidates: CandidateView[];
    selected: CandidateView | null;
  }>({ candidates: [], selected: null });
  const { candidates, selected } = queue;
  const [showAll, setShowAll] = useState(false);
  const [selectedPreview, setSelectedPreview] = useState<string | null>(null);
  const [nearest, setNearest] = useState<ExampleView[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [visibleCount, setVisibleCount] = useState(QUEUE_PAGE_SIZE);
  const [queueMore, setQueueMore] = useState(false);
  const [queueLoading, setQueueLoading] = useState(false);
  const [autoFloor, setAutoFloor] = useState(80);
  const [autoBusy, setAutoBusy] = useState(false);
  const [autoInfo, setAutoInfo] = useState<string | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [confirmDeleteAll, setConfirmDeleteAll] = useState(false);
  const resolving = useRef(new Set<number>());
  const projectRef = useRef<number | null>(null);

  useEffect(() => {
    setVisibleCount(QUEUE_PAGE_SIZE);
    setAutoInfo(null);
    setConfirmDeleteAll(false);
  }, [projectId, showAll]);

  useEffect(() => {
    let alive = true;
    getSetting("review.auto_floor")
      .then((value) => {
        if (!alive || value === null) return;
        const parsed = Number(value);
        if (Number.isFinite(parsed) && parsed >= 0 && parsed <= 100) {
          setAutoFloor(Math.round(parsed));
        }
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    if (!selected) return;
    const index = candidates.findIndex((candidate) => candidate.id === selected.id);
    if (index < 0) return;
    setVisibleCount((current) => (index < current ? current : index + 1));
  }, [selected?.id, candidates]);

  useEffect(() => {
    projectRef.current = projectId;
  }, [projectId]);

  useEffect(() => {
    if (!projectId) {
      setQueue({ candidates: [], selected: null });
      setQueueMore(false);
      setLoading(false);
      return;
    }
    let alive = true;
    setLoading(true);
    listCandidates(projectId, showAll ? undefined : "pending", undefined, QUEUE_FETCH)
      .then((rows) => {
        if (!alive) return;
        setQueueMore(rows.length >= QUEUE_FETCH);
        setQueue((current) => ({
          candidates: rows,
          selected:
            rows.find((row) => row.id === current.selected?.id) ?? rows[0] ?? null,
        }));
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
  }, [projectId, showAll]);

  useEffect(() => {
    if (!selected) {
      setNearest([]);
      return;
    }
    let alive = true;
    setNearest([]);
    nearestExamples(selected.id, 3)
      .then((rows) => {
        if (alive) setNearest(rows);
      })
      .catch(() => {
        if (alive) setNearest([]);
      });
    return () => {
      alive = false;
    };
  }, [selected?.id]);

  useEffect(() => {
    if (!selected) {
      setSelectedPreview(null);
      return;
    }
    let alive = true;
    setSelectedPreview(null);
    candidatePreview(selected.id)
      .then((url) => {
        if (alive) setSelectedPreview(url);
      })
      .catch(() => {
        if (alive) setSelectedPreview(null);
      });
    return () => {
      alive = false;
    };
  }, [selected?.id]);

  async function loadOlderQueue() {
    if (!projectId || queueLoading || !queueMore) return;
    const oldest = candidates[candidates.length - 1];
    if (!oldest) return;
    setQueueLoading(true);
    try {
      const rows = await listCandidates(
        projectId,
        showAll ? undefined : "pending",
        oldest.id,
        QUEUE_FETCH
      );
      setQueue((current) => {
        const seen = new Set(current.candidates.map((row) => row.id));
        return {
          ...current,
          candidates: [...current.candidates, ...rows.filter((row) => !seen.has(row.id))],
        };
      });
      setQueueMore(rows.length >= QUEUE_FETCH);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setQueueLoading(false);
    }
  }

  function resolve(candidate: CandidateView, decision: "accept" | "reject" | "skip") {
    if (candidate.state !== "pending" || resolving.current.has(candidate.id)) return;
    resolving.current.add(candidate.id);
    const project = projectId;
    setError(null);

    // Move the queue first. The backend write happens after the next paint, so
    // the next candidate is readable while the crop is copied and embedded.
    setQueue((current) => {
      const index = current.candidates.findIndex((row) => row.id === candidate.id);
      if (index < 0) return current;
      const nextSelected =
        current.selected?.id === candidate.id
          ? current.candidates[index + 1] ?? current.candidates[index - 1] ?? null
          : current.selected;
      const nextState: CandidateView["state"] =
        decision === "accept" ? "accepted" : decision === "reject" ? "rejected" : "skipped";
      const nextCandidates = showAll
        ? current.candidates.map((row) =>
            row.id === candidate.id ? { ...row, state: nextState } : row
          )
        : current.candidates.filter((row) => row.id !== candidate.id);
      return { candidates: nextCandidates, selected: nextSelected };
    });

    void resolveCandidate(candidate.id, decision)
      .catch((reason: unknown) => {
        if (project !== null && projectRef.current !== project) return;
        setQueue((current) => {
          const exists = current.candidates.some((row) => row.id === candidate.id);
          if (showAll && exists) {
            return {
              ...current,
              candidates: current.candidates.map((row) =>
                row.id === candidate.id ? candidate : row
              ),
            };
          }
          if (exists) return current;
          return {
            candidates: [candidate, ...current.candidates].sort(
              (left, right) => right.id - left.id
            ),
            selected: current.selected ?? candidate,
          };
        });
        setError(String(reason));
      })
      .finally(() => resolving.current.delete(candidate.id));
  }

  async function deleteSelected() {
    if (!selected || deleteBusy) return;
    const candidate = selected;
    setDeleteBusy(true);
    setError(null);
    try {
      await deleteCandidate(candidate.id);
      setQueue((current) => {
        const remaining = current.candidates.filter((row) => row.id !== candidate.id);
        return { candidates: remaining, selected: remaining[0] ?? null };
      });
    } catch (reason) {
      setError(String(reason));
    } finally {
      setDeleteBusy(false);
    }
  }

  async function deleteAll() {
    if (!projectId || deleteBusy) return;
    setDeleteBusy(true);
    setError(null);
    try {
      const deleted = await deleteAllCandidates(projectId);
      setQueue({ candidates: [], selected: null });
      setConfirmDeleteAll(false);
      setAutoInfo(deleted === 1 ? "Deleted 1 candidate." : `Deleted ${deleted} candidates.`);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setDeleteBusy(false);
    }
  }

  async function autoAccept() {
    if (!projectId) return;
    setAutoBusy(true);
    setError(null);
    setAutoInfo(null);
    try {
      const report = await autoAcceptCandidates(projectId, autoFloor / 100);
      await setSetting("review.auto_floor", String(autoFloor)).catch(() => {});
      const rows = await listCandidates(
        projectId,
        showAll ? undefined : "pending",
        undefined,
        QUEUE_FETCH
      );
      setQueueMore(rows.length >= QUEUE_FETCH);
      setQueue((current) => ({
        candidates: rows,
        selected: rows.find((row) => row.id === current.selected?.id) ?? rows[0] ?? null,
      }));
      setAutoInfo(
        report.failed > 0
          ? `Accepted ${report.accepted}; ${report.failed} could not be accepted.`
          : report.accepted === 1
            ? "Accepted 1 candidate."
            : `Accepted ${report.accepted} candidates.`
      );
    } catch (reason) {
      setError(String(reason));
    } finally {
      setAutoBusy(false);
    }
  }

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (
        event.defaultPrevented ||
        event.repeat ||
        event.metaKey ||
        event.ctrlKey ||
        event.altKey ||
        event.isComposing
      ) {
        return;
      }
      const target = event.target;
      if (
        target instanceof Element &&
        target.closest(
          "input, textarea, select, [contenteditable='true'], [contenteditable=''], [role='textbox']"
        )
      ) {
        return;
      }
      if (loading || !selected || selected.state !== "pending") return;

      const key = event.key.toLowerCase();
      const decision =
        key === "a" ? "accept" : key === "r" ? "reject" : key === "s" ? "skip" : null;
      if (!decision) return;

      event.preventDefault();
      resolve(selected, decision);
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [loading, selected, showAll, projectId]);

  const pending = candidates.filter((candidate) => candidate.state === "pending").length;
  const eligible = candidates.filter(
    (candidate) => candidate.state === "pending" && candidate.score >= autoFloor / 100
  ).length;

  return (
    <div className="flex min-h-full flex-col">
      <PageHeader
        title="Review"
        description="Candidates between the match floor and their class threshold. Accept writes a positive example; reject writes a negative."
        action={
          <Button size="sm" variant="outline" onClick={() => setShowAll((current) => !current)}>
            {showAll ? "Show pending" : "Show reviewed"}
          </Button>
        }
      />

      {!isDesktop ? (
        <p className="border-b border-border p-2 text-sm text-muted-foreground">
          Harvesting runs during live capture, which needs the desktop app. This preview
          starts with two stand-in candidates.
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
        <Badge variant="secondary" className="ml-auto tabular-nums">
          {pending} pending
        </Badge>
      </div>

      <div className="flex flex-wrap items-center gap-2 border-b border-border p-2.5">
        <span className="text-xs text-muted-foreground">Auto accept at or above</span>
        <Input
          id="review-auto-floor"
          type="number"
          min={0}
          max={100}
          step={1}
          value={autoFloor}
          disabled={autoBusy}
          aria-label="Auto accept match floor percentage"
          className="w-20 tabular-nums"
          onChange={(event) => {
            const next = Number(event.target.value);
            setAutoFloor(
              Number.isFinite(next) ? Math.min(100, Math.max(0, Math.round(next))) : 0
            );
            setAutoInfo(null);
          }}
          onBlur={() => {
            void setSetting("review.auto_floor", String(autoFloor)).catch(() => {});
          }}
        />
        <span className="text-sm tabular-nums">%</span>
        <Button
          size="sm"
          className="gap-1.5"
          disabled={!projectId || autoBusy || loading || eligible === 0}
          onClick={() => void autoAccept()}
        >
          <Check className="size-4" aria-hidden="true" />
          {autoBusy ? "Accepting" : "Auto accept"}
        </Button>
        <span className="text-xs text-muted-foreground">
          {pending === 0
            ? "No pending candidates."
            : eligible === 0
              ? "None qualify at this floor."
              : `${eligible} of ${pending} pending candidates qualify.`}
        </span>
        <div className="ml-auto flex items-center gap-1.5">
          {autoInfo ? <span className="text-xs text-muted-foreground">{autoInfo}</span> : null}
          {confirmDeleteAll ? (
            <>
              <span className="text-xs text-muted-foreground">
                Delete all {candidates.length}?
              </span>
              <Button
                size="sm"
                variant="outline"
                disabled={deleteBusy}
                onClick={() => void deleteAll()}
              >
                {deleteBusy ? "Deleting" : "Confirm"}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                disabled={deleteBusy}
                onClick={() => setConfirmDeleteAll(false)}
              >
                Cancel
              </Button>
            </>
          ) : (
            <Button
              size="sm"
              variant="ghost"
              className="gap-1.5"
              disabled={!projectId || deleteBusy || candidates.length === 0}
              onClick={() => setConfirmDeleteAll(true)}
            >
              <Trash2 className="size-4" aria-hidden="true" />
              Delete all
            </Button>
          )}
        </div>
      </div>

      {loading ? (
        <div className="grid gap-2.5 p-2.5 lg:grid-cols-[1.2fr_1fr]">
          <Skeleton className="h-64 w-full" />
          <Skeleton className="h-64 w-full" />
        </div>
      ) : candidates.length === 0 ? (
        <div className="p-2.5">
          <div className="rounded-lg border border-border bg-muted p-2.5">
            <p className="text-sm font-medium">Nothing to review</p>
            <p className="mt-1.5 max-w-[65ch] text-xs text-muted-foreground">
              {showAll
                ? "No candidate has been harvested for this project yet."
                : "Start capture with a model loaded. Proposals that score between the match floor and their class threshold are saved here, up to the coverage cap."}
            </p>
          </div>
        </div>
      ) : (
        <>
          <div className="grid gap-2.5 p-2.5 lg:grid-cols-[1.2fr_1fr]">
            <Card className="min-w-0">
              <CardHeader className="flex flex-row items-start justify-between gap-2">
                <div className="flex flex-col gap-1.5">
                  <CardTitle>{selected ? selected.className : "Candidate"}</CardTitle>
                  <CardDescription>
                    The crop the matcher was unsure about. A box needs to clear the class
                    threshold to be drawn; this one stopped short of it.
                  </CardDescription>
                </div>
                <div className="flex flex-wrap items-center gap-1.5">
                  {selected ? (
                    <>
                      <Badge variant="outline" className="tabular-nums">
                        {Math.round(selected.score * 100)}%
                      </Badge>
                      <Badge variant={selected.state === "pending" ? "secondary" : "outline"}>
                        {selected.state}
                      </Badge>
                    </>
                  ) : null}
                </div>
              </CardHeader>
              <CardContent className="flex flex-col gap-2">
                {selected ? (
                  <>
                    <img
                      src={selectedPreview ?? selected.thumb}
                      alt={`Candidate ${selected.id}`}
                      decoding="async"
                      className="max-h-[38vh] w-full rounded-md border border-border object-contain"
                    />
                    <div className="flex flex-wrap items-center gap-2">
                      <Button
                        size="sm"
                        className="gap-1.5"
                        disabled={selected.state !== "pending"}
                        aria-keyshortcuts="A"
                        onClick={() => resolve(selected, "accept")}
                      >
                        <Check className="size-4" aria-hidden="true" />
                        Accept as positive
                        <kbd className="ml-1.5 rounded border border-current/30 px-1.5 text-xs font-medium opacity-70">
                          A
                        </kbd>
                      </Button>
                      <Button
                        size="sm"
                        variant="outline"
                        className="gap-1.5"
                        disabled={selected.state !== "pending"}
                        aria-keyshortcuts="R"
                        onClick={() => resolve(selected, "reject")}
                      >
                        <X className="size-4" aria-hidden="true" />
                        Reject as negative
                        <kbd className="ml-1.5 rounded border border-current/30 px-1.5 text-xs font-medium opacity-70">
                          R
                        </kbd>
                      </Button>
                      <Button
                        size="sm"
                        variant="ghost"
                        className="gap-1.5"
                        disabled={selected.state !== "pending"}
                        aria-keyshortcuts="S"
                        onClick={() => resolve(selected, "skip")}
                      >
                        <SkipForward className="size-4" aria-hidden="true" />
                        Skip
                        <kbd className="ml-1.5 rounded border border-current/30 px-1.5 text-xs font-medium opacity-70">
                          S
                        </kbd>
                      </Button>
                      <Button
                        size="sm"
                        variant="ghost"
                        className="ml-auto gap-1.5 text-muted-foreground"
                        disabled={deleteBusy}
                        onClick={() => void deleteSelected()}
                      >
                        <Trash2 className="size-4" aria-hidden="true" />
                        Delete
                      </Button>
                    </div>
                  </>
                ) : (
                  <p className="text-sm text-muted-foreground">
                    Pick a candidate from the queue below.
                  </p>
                )}
              </CardContent>
            </Card>

            <Card className="min-w-0">
              <CardHeader>
                <CardTitle>Nearest examples</CardTitle>
                <CardDescription>
                  The three examples in this class closest to the candidate, best first.
                  They are the reason its score landed where it did.
                </CardDescription>
              </CardHeader>
              <CardContent>
                {nearest.length === 0 ? (
                  <p className="text-sm text-muted-foreground">
                    This class has no embedded examples yet.
                  </p>
                ) : (
                  <ul className="grid grid-cols-3 gap-2">
                    {nearest.map((example) => (
                      <li key={example.id} className="flex flex-col gap-1.5">
                        <img
                          src={exampleImage(example)}
                          alt={`Example ${example.id}`}
                          loading="lazy"
                          decoding="async"
                          className="aspect-square w-full rounded-md border border-border object-cover"
                        />
                        <Badge variant="outline" className="justify-center">
                          {example.role}
                        </Badge>
                      </li>
                    ))}
                  </ul>
                )}
              </CardContent>
            </Card>
          </div>

          <div className="px-2.5 pb-2.5">
            <Card>
              <CardHeader>
                <CardTitle>Queue</CardTitle>
                <CardDescription>
                  {candidates.length === 1 ? "1 candidate" : `${candidates.length} candidates`}
                  {showAll ? ", every state" : ", pending only"}. Newest first.
                </CardDescription>
              </CardHeader>
              <CardContent>
                <ul className="overflow-hidden rounded-lg border border-border">
                  {candidates.slice(0, visibleCount).map((candidate, index) => (
                    <li key={candidate.id} className="queue-row">
                      {index > 0 ? <Separator /> : null}
                      <button
                        type="button"
                        aria-pressed={selected?.id === candidate.id}
                        className={
                          selected?.id === candidate.id
                            ? "flex w-full items-center gap-2 bg-muted p-2 text-left"
                            : "flex w-full items-center gap-2 p-2 text-left"
                        }
                        onClick={() =>
                          setQueue((current) => ({ ...current, selected: candidate }))
                        }
                      >
                        <img
                          src={candidateImage(candidate)}
                          alt=""
                          loading="lazy"
                          decoding="async"
                          draggable={false}
                          className="size-10 shrink-0 rounded-sm border border-border object-cover"
                        />
                        <span className="flex min-w-0 flex-1 flex-col">
                          <span className="truncate text-sm">{candidate.className}</span>
                          <span className="truncate text-xs text-muted-foreground">
                            {formatDate(candidate.createdAt)}
                          </span>
                        </span>
                        <Badge variant="secondary" className="tabular-nums">
                          {Math.round(candidate.score * 100)}%
                        </Badge>
                        <Badge variant={candidate.state === "pending" ? "outline" : "secondary"}>
                          {candidate.state}
                        </Badge>
                      </button>
                    </li>
                  ))}
                </ul>
                {visibleCount < candidates.length || queueMore ? (
                  <div className="flex flex-wrap items-center justify-between gap-2 px-1.5 pt-2">
                    <span className="text-xs text-muted-foreground tabular-nums">
                      {visibleCount < candidates.length
                        ? `Showing ${visibleCount} of ${candidates.length}`
                        : `${candidates.length} loaded`}
                    </span>
                    <div className="flex items-center gap-1.5">
                      {visibleCount < candidates.length ? (
                        <Button
                          size="sm"
                          variant="outline"
                          onClick={() =>
                            setVisibleCount((current) =>
                              Math.min(candidates.length, current + QUEUE_PAGE_SIZE)
                            )
                          }
                        >
                          Show {Math.min(QUEUE_PAGE_SIZE, candidates.length - visibleCount)} more
                        </Button>
                      ) : null}
                      {queueMore ? (
                        <Button
                          size="sm"
                          variant="outline"
                          disabled={queueLoading}
                          onClick={() => void loadOlderQueue()}
                        >
                          {queueLoading ? "Loading..." : "Load older"}
                        </Button>
                      ) : null}
                    </div>
                  </div>
                ) : null}
              </CardContent>
            </Card>
          </div>
        </>
      )}
    </div>
  );
}

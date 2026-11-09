import { useEffect, useState } from "react";
import { Check, SkipForward, X } from "lucide-react";

import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../components/ui/card";
import { Separator } from "../components/ui/separator";
import { Skeleton } from "../components/ui/skeleton";
import { PageHeader } from "../components/page-header";
import {
  isDesktop,
  listCandidates,
  listProjects,
  nearestExamples,
  resolveCandidate,
} from "../lib/api";
import { formatDate } from "../lib/utils";
import type { CandidateView, ExampleView, Project } from "../lib/types";

export function ReviewPage() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectId, setProjectId] = useState<number | null>(null);
  const [candidates, setCandidates] = useState<CandidateView[]>([]);
  const [showAll, setShowAll] = useState(false);
  const [selected, setSelected] = useState<CandidateView | null>(null);
  const [nearest, setNearest] = useState<ExampleView[]>([]);
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
      setCandidates([]);
      setSelected(null);
      setLoading(false);
      return;
    }
    let alive = true;
    setLoading(true);
    listCandidates(projectId, showAll ? undefined : "pending")
      .then((rows) => {
        if (!alive) return;
        setCandidates(rows);
        setSelected((current) => rows.find((row) => row.id === current?.id) ?? rows[0] ?? null);
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
  }, [selected]);

  async function refresh(id: number) {
    const rows = await listCandidates(id, showAll ? undefined : "pending");
    setCandidates(rows);
    return rows;
  }

  async function resolve(candidate: CandidateView, decision: "accept" | "reject" | "skip") {
    setBusy(true);
    setError(null);
    try {
      await resolveCandidate(candidate.id, decision);
      if (projectId) {
        const rows = await refresh(projectId);
        setSelected(rows[0] ?? null);
      }
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  const pending = candidates.filter((candidate) => candidate.state === "pending").length;

  return (
    <div className="flex min-h-full flex-col">
      <PageHeader
        title="Review"
        description="Detections that landed just under their class threshold, waiting for an answer. Accepting writes a positive example, rejecting writes a negative, and both change the next run."
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
              onClick={() => setProjectId(project.id)}
            >
              {project.name}
            </Button>
          ))
        )}
        <Badge variant="secondary" className="ml-auto tabular-nums">
          {pending} pending
        </Badge>
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
            <p className="mt-1 max-w-[65ch] text-xs text-muted-foreground">
              {showAll
                ? "No candidate has been harvested for this project yet."
                : "Start capture with a model loaded. Proposals that score within 12 percent under their class threshold are saved here, up to forty per session."}
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
                      src={selected.preview}
                      alt={`Candidate ${selected.id}`}
                      className="max-h-[38vh] w-full rounded-md border border-border object-contain"
                    />
                    <div className="flex flex-wrap items-center gap-2">
                      <Button
                        size="sm"
                        className="gap-1.5"
                        disabled={busy || selected.state !== "pending"}
                        onClick={() => resolve(selected, "accept")}
                      >
                        <Check className="size-4" aria-hidden="true" />
                        Accept as positive
                      </Button>
                      <Button
                        size="sm"
                        variant="outline"
                        className="gap-1.5"
                        disabled={busy || selected.state !== "pending"}
                        onClick={() => resolve(selected, "reject")}
                      >
                        <X className="size-4" aria-hidden="true" />
                        Reject as negative
                      </Button>
                      <Button
                        size="sm"
                        variant="ghost"
                        className="gap-1.5"
                        disabled={busy || selected.state !== "pending"}
                        onClick={() => resolve(selected, "skip")}
                      >
                        <SkipForward className="size-4" aria-hidden="true" />
                        Skip
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
                          src={example.preview}
                          alt={`Example ${example.id}`}
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
                  {candidates.map((candidate, index) => (
                    <li key={candidate.id}>
                      {index > 0 ? <Separator /> : null}
                      <button
                        type="button"
                        aria-pressed={selected?.id === candidate.id}
                        className={
                          selected?.id === candidate.id
                            ? "flex w-full items-center gap-2 bg-muted p-2 text-left"
                            : "flex w-full items-center gap-2 p-2 text-left"
                        }
                        onClick={() => setSelected(candidate)}
                      >
                        <img
                          src={candidate.preview}
                          alt=""
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
              </CardContent>
            </Card>
          </div>
        </>
      )}
    </div>
  );
}

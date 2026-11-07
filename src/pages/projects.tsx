import { useEffect, useState, type FormEvent } from "react";
import { FolderPlus, Plus, Trash2 } from "lucide-react";

import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { Separator } from "../components/ui/separator";
import { Skeleton } from "../components/ui/skeleton";
import { PageHeader } from "../components/page-header";
import { createProject, deleteProject, listProjects } from "../lib/api";
import { formatDate } from "../lib/utils";
import type { Project } from "../lib/types";

export function ProjectsPage() {
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [confirmId, setConfirmId] = useState<number | null>(null);

  useEffect(() => {
    let alive = true;
    listProjects()
      .then((rows) => {
        if (alive) setProjects(rows);
      })
      .catch((reason: unknown) => {
        if (!alive) return;
        setError(String(reason));
        setProjects([]);
      });
    return () => {
      alive = false;
    };
  }, []);

  async function submit(event: FormEvent) {
    event.preventDefault();
    const trimmed = name.trim();
    if (!trimmed || busy) return;
    setBusy(true);
    try {
      const project = await createProject(trimmed);
      setProjects((current) => [project, ...(current ?? [])]);
      setName("");
      setCreating(false);
      setError(null);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function remove(id: number) {
    setBusy(true);
    try {
      await deleteProject(id);
      setProjects((current) => (current ?? []).filter((project) => project.id !== id));
      setConfirmId(null);
      setError(null);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex min-h-full flex-col">
      <PageHeader
        title="Projects"
        description="One project per game or app. Each keeps its own examples, classes and database, so a traffic project and a HUD project never mix."
        action={
          <Button size="sm" className="gap-1.5" onClick={() => setCreating(true)}>
            <Plus className="size-4" aria-hidden="true" />
            New project
          </Button>
        }
      />

      {creating ? (
        <form onSubmit={submit} className="flex items-center gap-2 border-b border-border p-2">
          <Input
            autoFocus
            value={name}
            onChange={(event) => setName(event.target.value)}
            placeholder="Traffic watch"
            aria-label="Project name"
            maxLength={64}
          />
          <Button type="submit" size="sm" disabled={busy || name.trim().length === 0}>
            Create
          </Button>
          <Button
            type="button"
            size="sm"
            variant="ghost"
            onClick={() => {
              setCreating(false);
              setName("");
            }}
          >
            Cancel
          </Button>
        </form>
      ) : null}

      {error ? (
        <p role="alert" className="border-b border-border p-2 text-sm text-destructive">
          {error}
        </p>
      ) : null}

      <div className="flex flex-col gap-2.5 p-2.5">
        {projects === null ? (
          <div aria-busy="true" aria-live="polite" className="flex flex-col gap-2">
            <span className="sr-only">Loading projects</span>
            <Skeleton className="h-6 w-[16rem]" />
            <Skeleton className="h-4 w-[10rem]" />
            <Skeleton className="h-4 w-[7rem]" />
          </div>
        ) : projects.length === 0 ? (
          <div className="rounded-lg border border-border bg-muted p-2.5">
            <div className="flex items-start gap-2">
              <FolderPlus className="size-5 shrink-0 text-muted-foreground" aria-hidden="true" />
              <div className="flex flex-col gap-1.5">
                <p className="text-sm font-medium">No projects yet</p>
                <p className="max-w-[65ch] text-xs text-muted-foreground">
                  A project holds one game or app: the screenshots you feed it, the classes
                  you name, and its own database. Create one, then add a capture source.
                </p>
              </div>
            </div>
            <div className="mt-2.5 flex items-center gap-2">
              <Button size="sm" onClick={() => setCreating(true)}>
                New project
              </Button>
            </div>
          </div>
        ) : (
          <ul className="overflow-hidden rounded-lg border border-border">
            {projects.map((project, index) => (
              <li key={project.id}>
                {index > 0 ? <Separator /> : null}
                <div className="flex items-center gap-2 p-2">
                  <div className="flex min-w-0 flex-1 flex-col">
                    <span className="truncate text-sm font-medium">{project.name}</span>
                    <span className="truncate text-xs text-muted-foreground">
                      projects/{project.slug}
                    </span>
                  </div>
                  <span className="hidden text-xs text-muted-foreground tabular-nums sm:block">
                    {formatDate(project.createdAt)}
                  </span>
                  <Badge variant="secondary" className="tabular-nums">
                    0 examples
                  </Badge>
                  <Button
                    size="icon"
                    variant="ghost"
                    aria-label={`Delete ${project.name}`}
                    onClick={() => setConfirmId(confirmId === project.id ? null : project.id)}
                  >
                    <Trash2 className="size-4" aria-hidden="true" />
                  </Button>
                </div>

                {confirmId === project.id ? (
                  <div className="flex items-center gap-2 border-t border-border bg-muted p-2">
                    <p className="flex-1 text-xs text-muted-foreground">
                      Delete this project and its folder? Its examples go with it.
                    </p>
                    <Button
                      size="sm"
                      variant="destructive"
                      disabled={busy}
                      onClick={() => remove(project.id)}
                    >
                      Delete
                    </Button>
                    <Button size="sm" variant="ghost" onClick={() => setConfirmId(null)}>
                      Keep
                    </Button>
                  </div>
                ) : null}
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

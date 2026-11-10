import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

import { getSetting, listProjects, setSetting } from "../lib/api";
import type { Project } from "../lib/types";

type ProjectContextValue = {
  projects: Project[];
  projectId: number | null;
  project: Project | null;
  loading: boolean;
  error: string | null;
  selectProject: (id: number) => void;
  refresh: () => Promise<void>;
};

const ProjectContext = createContext<ProjectContextValue | null>(null);

const WORKING_PROJECT = "workspace.project";
const LEGACY_PROJECT = "capture.project";

/**
 * One project is the working project for the whole window. Pages used to pick
 * their own first project, which meant switching on Label did not switch
 * Review or Capture lab. This provider is the single source for that choice.
 */
export function ProjectProvider({ children }: { children: ReactNode }) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectId, setProjectId] = useState<number | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const rows = await listProjects();
      setProjects(rows);
      setProjectId((current) => {
        if (current !== null && rows.some((row) => row.id === current)) return current;
        return rows[0]?.id ?? null;
      });
      setError(null);
    } catch (reason) {
      setError(String(reason));
      setProjects([]);
      setProjectId(null);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    let alive = true;
    Promise.all([
      listProjects(),
      getSetting(WORKING_PROJECT),
      getSetting(LEGACY_PROJECT),
    ])
      .then(([rows, saved, legacy]) => {
        if (!alive) return;
        setProjects(rows);
        const wanted = Number(saved ?? legacy);
        const found = rows.find((row) => row.id === wanted) ?? rows[0] ?? null;
        setProjectId(found?.id ?? null);
        setError(null);
      })
      .catch((reason: unknown) => {
        if (!alive) return;
        setError(String(reason));
        setProjects([]);
        setProjectId(null);
      })
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    const onFocus = () => {
      void refresh();
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [refresh]);

  const selectProject = useCallback((id: number) => {
    setProjectId((current) => (current === id ? current : id));
    void setSetting(WORKING_PROJECT, String(id));
  }, []);

  const project = useMemo(
    () => projects.find((row) => row.id === projectId) ?? null,
    [projects, projectId]
  );

  const value = useMemo(
    () => ({
      projects,
      projectId,
      project,
      loading,
      error,
      selectProject,
      refresh,
    }),
    [projects, projectId, project, loading, error, selectProject, refresh]
  );

  return <ProjectContext.Provider value={value}>{children}</ProjectContext.Provider>;
}

export function useProject() {
  const context = useContext(ProjectContext);
  if (!context) throw new Error("useProject must be used inside ProjectProvider");
  return context;
}

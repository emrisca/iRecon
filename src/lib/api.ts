import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { CaptureStats, FramePayload, Project } from "./types";

export const isDesktop =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

let demoProjects: Project[] = [];
let demoId = 1;

function demo(name: string): Project {
  const slug = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
  return {
    id: demoId++,
    name,
    slug,
    path: `projects/${slug}`,
    createdAt: new Date().toISOString(),
  };
}

export async function listProjects(): Promise<Project[]> {
  if (!isDesktop) return demoProjects;
  return invoke<Project[]>("projects_list");
}

export async function createProject(name: string): Promise<Project> {
  if (!isDesktop) {
    const project = demo(name);
    demoProjects = [project, ...demoProjects];
    return project;
  }
  return invoke<Project>("project_create", { name });
}

export async function deleteProject(id: number): Promise<void> {
  if (!isDesktop) {
    demoProjects = demoProjects.filter((project) => project.id !== id);
    return;
  }
  await invoke("project_delete", { id });
}

export async function startCapture(): Promise<void> {
  if (!isDesktop) throw new Error("Screen capture needs the desktop app");
  await invoke("capture_start");
}

export async function stopCapture(): Promise<void> {
  if (!isDesktop) return;
  await invoke("capture_stop");
}

export async function showOverlay(): Promise<void> {
  if (!isDesktop) throw new Error("The overlay needs the desktop app");
  await invoke("overlay_show");
}

export async function hideOverlay(): Promise<void> {
  if (!isDesktop) return;
  await invoke("overlay_hide");
}

export async function onStats(
  handler: (stats: CaptureStats) => void
): Promise<UnlistenFn> {
  if (!isDesktop) return () => {};
  return listen<CaptureStats>("spike:stats", (event) => handler(event.payload));
}

export async function onFrame(
  handler: (frame: FramePayload) => void
): Promise<UnlistenFn> {
  if (!isDesktop) return () => {};
  return listen<FramePayload>("spike:frame", (event) => handler(event.payload));
}

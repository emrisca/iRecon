import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  Box,
  CaptureStats,
  DetectionPayload,
  ExampleView,
  FramePayload,
  FrameView,
  LabelClass,
  ModelProgress,
  ModelStatus,
  Project,
} from "./types";

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

export async function grabFrame(projectId: number): Promise<FrameView> {
  if (!isDesktop) throw new Error("Capturing a frame needs the desktop app");
  return invoke<FrameView>("frame_grab", { projectId });
}

export async function listClasses(projectId: number): Promise<LabelClass[]> {
  if (!isDesktop) return [];
  return invoke<LabelClass[]>("classes_list", { projectId });
}

export async function createClass(
  projectId: number,
  name: string
): Promise<LabelClass> {
  if (!isDesktop) throw new Error("Classes need the desktop app");
  return invoke<LabelClass>("class_create", { projectId, name });
}

export async function deleteClass(id: number): Promise<void> {
  if (!isDesktop) return;
  await invoke("class_delete", { id });
}

export async function createExample(
  classId: number,
  frameId: number,
  box: Box
): Promise<ExampleView> {
  if (!isDesktop) throw new Error("Examples need the desktop app");
  return invoke<ExampleView>("example_create", {
    classId,
    frameId,
    x: box.x,
    y: box.y,
    w: box.w,
    h: box.h,
  });
}

export async function listExamples(classId: number): Promise<ExampleView[]> {
  if (!isDesktop) return [];
  return invoke<ExampleView[]>("examples_list", { classId });
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

export async function getModelStatus(): Promise<ModelStatus | null> {
  if (!isDesktop) return null;
  return invoke<ModelStatus>("model_status");
}

export async function downloadModel(): Promise<void> {
  if (!isDesktop) throw new Error("Downloads need the desktop app");
  await invoke("model_download");
}

export async function loadModel(): Promise<void> {
  if (!isDesktop) throw new Error("Loading a model needs the desktop app");
  await invoke("model_load");
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

export async function onDetections(
  handler: (payload: DetectionPayload) => void
): Promise<UnlistenFn> {
  if (!isDesktop) return () => {};
  return listen<DetectionPayload>("spike:detections", (event) =>
    handler(event.payload)
  );
}

export async function onModelProgress(
  handler: (progress: ModelProgress) => void
): Promise<UnlistenFn> {
  if (!isDesktop) return () => {};
  return listen<ModelProgress>("model:progress", (event) =>
    handler(event.payload)
  );
}

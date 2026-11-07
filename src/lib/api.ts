import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  Box,
  CaptureStats,
  ClassMatch,
  DatasetExport,
  DetectionPayload,
  ExampleView,
  FramePayload,
  FrameView,
  LabelClass,
  ModelCatalog,
  ModelProgress,
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

let demoFrameId = 1;

/** A stand-in frame so the labeling and zoom controls can be exercised in a browser. */
function demoFrame(): FrameView {
  const width = 1600;
  const height = 900;
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}"><rect width="${width}" height="${height}" fill="#2b3440"/><rect x="150" y="320" width="420" height="250" rx="18" fill="#d05f3f"/><rect x="760" y="410" width="340" height="180" rx="14" fill="#4f7d5a"/><circle cx="1330" cy="220" r="100" fill="#c9c2a8"/><rect x="80" y="700" width="1440" height="14" fill="#8b93a1"/></svg>`;
  return {
    id: demoFrameId++,
    path: "preview/frame.png",
    width,
    height,
    preview: `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`,
    createdAt: new Date().toISOString(),
  };
}

export async function grabFrame(projectId: number): Promise<FrameView> {
  if (!isDesktop) return demoFrame();
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

export async function setClassThreshold(
  id: number,
  threshold: number
): Promise<LabelClass> {
  if (!isDesktop) throw new Error("Thresholds need the desktop app");
  return invoke<LabelClass>("class_threshold", { id, threshold });
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

export async function startCapture(projectId?: number): Promise<void> {
  if (!isDesktop) throw new Error("Screen capture needs the desktop app");
  await invoke("capture_start", { projectId });
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

export async function getModelCatalog(): Promise<ModelCatalog | null> {
  if (!isDesktop) return null;
  return invoke<ModelCatalog>("model_catalog");
}

export async function downloadModel(id: string): Promise<void> {
  if (!isDesktop) throw new Error("Downloads need the desktop app");
  await invoke("model_download", { id });
}

export async function loadModel(id: string): Promise<void> {
  if (!isDesktop) throw new Error("Loading a model needs the desktop app");
  await invoke("model_load", { id });
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

export async function matchRegion(
  projectId: number,
  frameId: number,
  box: Box
): Promise<ClassMatch[]> {
  if (!isDesktop) return [];
  return invoke<ClassMatch[]>("match_region", {
    projectId,
    frameId,
    x: box.x,
    y: box.y,
    w: box.w,
    h: box.h,
  });
}

export async function exportDataset(projectId: number): Promise<DatasetExport> {
  if (!isDesktop) {
    return {
      path: "preview/export",
      frames: boxes > 0 ? 1 : 0,
      boxes,
      classes: rows.length,
      crops: boxes,
      skipped: 0,
    };
  }
  return invoke<DatasetExport>("dataset_export", { projectId });
}

export async function embedExamples(projectId: number): Promise<number> {
  if (!isDesktop) return 0;
  return invoke<number>("examples_embed", { projectId });
}

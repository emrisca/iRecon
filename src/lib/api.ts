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

/**
 * A seeded project so the Label page can be exercised without the desktop app:
 * three classes with examples, and an anchor per class that lines up with a
 * shape in the demo frame, so a box drawn over that shape scores its class
 * highest and the nearest badges mean something.
 */
const demoSeed = demo("Demo project");
let demoProjects: Project[] = [demoSeed];

const DEMO_COLORS = ["#d05f3f", "#4f7d5a", "#c9a227", "#8b6fb0"];

const demoClasses = new Map<number, LabelClass[]>();
const demoExamples = new Map<number, ExampleView[]>();
const demoAnchors = new Map<number, Box>();
let demoClassId = 1;
let demoExampleId = 1;

function demoPreview(color: string, index: number): string {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="160" height="160" viewBox="0 0 160 160"><rect width="160" height="160" fill="#2b3440"/><rect x="${20 + (index % 3) * 8}" y="${24 + (index % 4) * 6}" width="${96 - (index % 3) * 10}" height="${88 - (index % 4) * 8}" rx="10" fill="${color}"/></svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

function demoExample(classId: number, color: string, index: number): ExampleView {
  return {
    id: demoExampleId++,
    classId,
    path: `examples/${classId}/${index + 1}.png`,
    width: 96 + (index % 3) * 12,
    height: 84 + (index % 4) * 9,
    preview: demoPreview(color, index),
    bbox: "0,0,1,1",
    createdAt: new Date(Date.now() - index * 60_000).toISOString(),
  };
}

function demoClassById(id: number): LabelClass | null {
  for (const rows of demoClasses.values()) {
    const found = rows.find((entry) => entry.id === id);
    if (found) return found;
  }
  return null;
}

function iou(a: Box, b: Box): number {
  const left = Math.max(a.x, b.x);
  const top = Math.max(a.y, b.y);
  const right = Math.min(a.x + a.w, b.x + b.w);
  const bottom = Math.min(a.y + a.h, b.y + b.h);
  const overlap = Math.max(0, right - left) * Math.max(0, bottom - top);
  const union = a.w * a.h + b.w * b.h - overlap;
  return union > 0 ? overlap / union : 0;
}

function seedDemoProject() {
  const seeds = [
    { name: "panel", threshold: 0.9, examples: 4, anchor: { x: 0.094, y: 0.356, w: 0.263, h: 0.278 } },
    { name: "field", threshold: 0.85, examples: 3, anchor: { x: 0.475, y: 0.456, w: 0.213, h: 0.2 } },
    { name: "marker", threshold: 0.7, examples: 2, anchor: { x: 0.769, y: 0.133, w: 0.125, h: 0.222 } },
  ];
  const classes: LabelClass[] = [];
  seeds.forEach((seed, position) => {
    const id = demoClassId++;
    const color = DEMO_COLORS[position % DEMO_COLORS.length];
    classes.push({
      id,
      name: seed.name,
      color,
      threshold: seed.threshold,
      examples: seed.examples,
    });
    demoAnchors.set(id, seed.anchor);
    demoExamples.set(
      id,
      Array.from({ length: seed.examples }, (_, index) => demoExample(id, color, index))
    );
  });
  demoClasses.set(demoSeed.id, classes);
}

seedDemoProject();

/** Put the browser demo back to its seeded classes, examples and thresholds. */
export async function resetDemoData(): Promise<void> {
  if (isDesktop) return;
  if (!demoProjects.some((project) => project.id === demoSeed.id)) {
    demoProjects = [demoSeed, ...demoProjects];
  }
  demoClasses.clear();
  demoExamples.clear();
  demoAnchors.clear();
  demoClassId = 1;
  demoExampleId = 1;
  seedDemoProject();
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
  if (!isDesktop) return (demoClasses.get(projectId) ?? []).map((entry) => ({ ...entry }));
  return invoke<LabelClass[]>("classes_list", { projectId });
}

export async function createClass(
  projectId: number,
  name: string
): Promise<LabelClass> {
  if (!isDesktop) {
    const rows = demoClasses.get(projectId) ?? [];
    if (rows.some((entry) => entry.name.toLowerCase() === name.toLowerCase())) {
      throw new Error("A class with that name already exists");
    }
    const created: LabelClass = {
      id: demoClassId++,
      name,
      color: DEMO_COLORS[rows.length % DEMO_COLORS.length],
      threshold: 0.85,
      examples: 0,
    };
    demoClasses.set(projectId, [...rows, created]);
    demoExamples.set(created.id, []);
    return { ...created };
  }
  return invoke<LabelClass>("class_create", { projectId, name });
}

export async function deleteClass(id: number): Promise<void> {
  if (!isDesktop) {
    for (const [projectId, rows] of demoClasses) {
      demoClasses.set(
        projectId,
        rows.filter((entry) => entry.id !== id)
      );
    }
    demoExamples.delete(id);
    demoAnchors.delete(id);
    return;
  }
  await invoke("class_delete", { id });
}

export async function setClassThreshold(
  id: number,
  threshold: number
): Promise<LabelClass> {
  if (!isDesktop) {
    const target = demoClassById(id);
    if (!target) throw new Error("That class is not in this project");
    target.threshold = Math.min(1, Math.max(0, threshold));
    return { ...target };
  }
  return invoke<LabelClass>("class_threshold", { id, threshold });
}

export async function createExample(
  classId: number,
  frameId: number,
  box: Box
): Promise<ExampleView> {
  if (!isDesktop) {
    const target = demoClassById(classId);
    if (!target) throw new Error("That class is not in this project");
    const created: ExampleView = {
      id: demoExampleId++,
      classId,
      path: `examples/${classId}/saved.png`,
      width: Math.max(8, Math.round(box.w * 1600)),
      height: Math.max(8, Math.round(box.h * 900)),
      preview: demoPreview(target.color, target.examples),
      bbox: `${box.x},${box.y},${box.w},${box.h}`,
      createdAt: new Date().toISOString(),
    };
    const rows = demoExamples.get(classId) ?? [];
    demoExamples.set(classId, [created, ...rows]);
    target.examples = rows.length + 1;
    return created;
  }
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
  if (!isDesktop) return (demoExamples.get(classId) ?? []).map((entry) => ({ ...entry }));
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
  if (!isDesktop) {
    const rows = demoClasses.get(projectId) ?? [];
    return rows
      .map((entry) => {
        const anchor = demoAnchors.get(entry.id);
        const similarity = anchor ? iou(anchor, box) : 0;
        const score = Math.min(1, 0.35 + 0.63 * similarity);
        return {
          classId: entry.id,
          name: entry.name,
          score,
          similarity,
          margin: Math.max(0, score - 0.15),
          bestExample: (demoExamples.get(entry.id) ?? [])[0]?.id ?? 0,
          positives: entry.examples,
          negatives: 0,
        };
      })
      .sort((left, right) => right.score - left.score);
  }
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
    const rows = demoClasses.get(projectId) ?? [];
    const boxes = rows.reduce((total, entry) => total + entry.examples, 0);
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

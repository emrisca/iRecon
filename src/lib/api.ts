import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  Box,
  CandidateView,
  ClassAdvice,
  CaptureStats,
  CaptureTarget,
  ClassMatch,
  DatasetExport,
  DetectionPayload,
  ExampleView,
  FramePayload,
  FrameView,
  ImportReport,
  LabelClass,
  ModelCatalog,
  ModelProgress,
  Project,
  SourceKind,
  SourceView,
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
    role: "positive",
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
  demoLibrary.clear();
  demoSources.clear();
  demoCandidates.clear();
  demoClassId = 1;
  demoExampleId = 1;
  demoSourceId = 1;
  demoCandidateId = 1;
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
let demoLibrary = new Map<number, FrameView[]>();

function demoLibraryFor(projectId: number): FrameView[] {
  let rows = demoLibrary.get(projectId);
  if (!rows) {
    rows = [demoFrame(), demoFrame()];
    demoLibrary.set(projectId, rows);
  }
  return rows;
}

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
  if (!isDesktop) {
    const frame = demoFrame();
    demoLibrary.set(projectId, [frame, ...demoLibraryFor(projectId)]);
    return frame;
  }
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
  box: Box,
  role: "positive" | "negative" = "positive"
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
      role,
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
    role,
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

/** The palette class colours come from, kept in step with the Rust side. */
export const CLASS_COLORS = [
  "oklch(0.62 0.11 250)",
  "oklch(0.62 0.11 160)",
  "oklch(0.62 0.11 25)",
  "oklch(0.62 0.11 300)",
  "oklch(0.62 0.11 200)",
  "oklch(0.62 0.11 90)",
  "oklch(0.62 0.11 340)",
  "oklch(0.62 0.11 45)",
];

export async function updateClass(
  id: number,
  patch: { name?: string; color?: string }
): Promise<LabelClass> {
  if (!isDesktop) {
    const target = demoClassById(id);
    if (!target) throw new Error("That class is not in this project");
    if (patch.name) target.name = patch.name;
    if (patch.color) target.color = patch.color;
    return { ...target };
  }
  return invoke<LabelClass>("class_update", {
    id,
    name: patch.name ?? null,
    color: patch.color ?? null,
  });
}

export async function classAdvice(projectId: number): Promise<ClassAdvice[]> {
  if (!isDesktop) {
    return (demoClasses.get(projectId) ?? []).map((entry) => {
      const rows = demoExamples.get(entry.id) ?? [];
      const positives = rows.filter((row) => row.role === "positive").length;
      const negatives = rows.length - positives;
      const hints: string[] = [];
      if (positives === 0) {
        hints.push("No positive example yet, so this class cannot match anything.");
      } else if (positives < 3) {
        hints.push(
          `Only ${positives} positive${positives === 1 ? "" : "s"}. Three is the minimum for a stable pool.`
        );
      }
      if (positives > 0 && negatives === 0) {
        hints.push("No negatives yet, so look-alike objects cannot be separated.");
      }
      return {
        classId: entry.id,
        name: entry.name,
        positives,
        negatives,
        similarity: 0.94,
        threshold: entry.threshold,
        suggested: positives >= 3 ? 0.8 : null,
        hints,
      };
    });
  }
  return invoke<ClassAdvice[]>("class_advice", { projectId });
}

export async function listFrames(projectId: number): Promise<FrameView[]> {
  if (!isDesktop) return demoLibraryFor(projectId).map((frame) => ({ ...frame }));
  return invoke<FrameView[]>("library_list", { projectId });
}

export async function openFrame(frameId: number): Promise<FrameView> {
  if (!isDesktop) {
    for (const rows of demoLibrary.values()) {
      const found = rows.find((frame) => frame.id === frameId);
      if (found) return { ...found };
    }
    throw new Error("That frame is gone");
  }
  return invoke<FrameView>("library_frame", { frameId });
}

export async function importImages(
  projectId: number,
  paths: string[]
): Promise<ImportReport> {
  if (!isDesktop) {
    const rows = demoLibraryFor(projectId);
    for (const path of paths) {
      const name = path.split(/[\\/]/).filter(Boolean).pop() ?? "frame.png";
      rows.unshift({ ...demoFrame(), path: name });
    }
    return { imported: paths.length, skipped: 0, errors: [] };
  }
  return invoke<ImportReport>("library_import", { projectId, paths });
}

export async function importFolder(
  projectId: number,
  folder: string
): Promise<ImportReport> {
  if (!isDesktop) {
    const rows = demoLibraryFor(projectId);
    const name = folder.split(/[\\/]/).filter(Boolean).pop() ?? "folder";
    for (let index = 0; index < 3; index += 1) {
      rows.unshift({ ...demoFrame(), path: `${name}/${index + 1}.png` });
    }
    return { imported: 3, skipped: 0, errors: [] };
  }
  return invoke<ImportReport>("library_import_folder", { projectId, folder });
}

let demoSources = new Map<number, SourceView[]>();
let demoSourceId = 1;

const DEMO_TARGETS: Record<"display" | "window", CaptureTarget[]> = {
  display: [{ target: "display-1", label: "Built-in display", detail: "1600 by 900, primary" }],
  window: [{ target: "window-1", label: "Demo window", detail: "Demo app, 1280 by 720" }],
};

export async function listDisplays(): Promise<CaptureTarget[]> {
  if (!isDesktop) return DEMO_TARGETS.display;
  return invoke<CaptureTarget[]>("displays_list");
}

export async function listWindows(): Promise<CaptureTarget[]> {
  if (!isDesktop) return DEMO_TARGETS.window;
  return invoke<CaptureTarget[]>("windows_list");
}

export async function listSources(projectId: number): Promise<SourceView[]> {
  if (!isDesktop) return (demoSources.get(projectId) ?? []).map((entry) => ({ ...entry }));
  return invoke<SourceView[]>("sources_list", { projectId });
}

export async function addSource(
  projectId: number,
  kind: SourceKind,
  target: string,
  label?: string
): Promise<SourceView> {
  if (!isDesktop) {
    const rows = demoSources.get(projectId) ?? [];
    const existing = rows.find((entry) => entry.kind === kind && entry.target === target);
    if (existing) return { ...existing };
    const created: SourceView = {
      id: demoSourceId++,
      projectId,
      kind,
      label: label?.trim() || target,
      target,
      crop: null,
    };
    demoSources.set(projectId, [...rows, created]);
    return { ...created };
  }
  return invoke<SourceView>("source_add", { projectId, kind, target, label });
}

export async function setSourceCrop(id: number, crop: Box | null): Promise<SourceView> {
  if (!isDesktop) {
    for (const rows of demoSources.values()) {
      const found = rows.find((entry) => entry.id === id);
      if (found) {
        found.crop = crop ? { ...crop } : null;
        return { ...found };
      }
    }
    throw new Error("That source no longer exists");
  }
  return invoke<SourceView>("source_crop", { id, crop });
}

export async function deleteSource(id: number): Promise<void> {
  if (!isDesktop) {
    for (const [projectId, rows] of demoSources) {
      demoSources.set(
        projectId,
        rows.filter((entry) => entry.id !== id)
      );
    }
    return;
  }
  await invoke("source_delete", { id });
}

let demoCandidates = new Map<number, CandidateView[]>();
let demoCandidateId = 1;

function demoCandidatesFor(projectId: number): CandidateView[] {
  let rows = demoCandidates.get(projectId);
  if (!rows) {
    const classes = demoClasses.get(projectId) ?? [];
    rows = classes.slice(0, 2).map((entry, index) => ({
      id: demoCandidateId++,
      projectId,
      classId: entry.id,
      className: entry.name,
      color: entry.color,
      path: `candidates/${entry.id}/${index + 1}.png`,
      preview: demoPreview(entry.color, index + 2),
      score: 0.72 + index * 0.06,
      state: "pending" as const,
      createdAt: new Date(Date.now() - index * 90_000).toISOString(),
    }));
    demoCandidates.set(projectId, rows);
  }
  return rows;
}

export async function listCandidates(
  projectId: number,
  state?: string
): Promise<CandidateView[]> {
  if (!isDesktop) {
    return demoCandidatesFor(projectId)
      .filter((entry) => !state || entry.state === state)
      .map((entry) => ({ ...entry }));
  }
  return invoke<CandidateView[]>("candidates_list", { projectId, state: state ?? null });
}

export async function nearestExamples(id: number, limit = 3): Promise<ExampleView[]> {
  if (!isDesktop) {
    for (const rows of demoCandidates.values()) {
      const found = rows.find((entry) => entry.id === id);
      if (found) {
        return (demoExamples.get(found.classId) ?? [])
          .slice(0, limit)
          .map((entry) => ({ ...entry }));
      }
    }
    return [];
  }
  return invoke<ExampleView[]>("candidate_nearest", { id, limit });
}

export async function resolveCandidate(
  id: number,
  decision: "accept" | "reject" | "skip",
  classId?: number
): Promise<void> {
  if (!isDesktop) {
    for (const rows of demoCandidates.values()) {
      const found = rows.find((entry) => entry.id === id);
      if (!found) continue;
      if (decision === "skip") {
        found.state = "skipped";
        return;
      }
      found.state = decision === "accept" ? "accepted" : "rejected";
      const target = demoClassById(classId ?? found.classId);
      if (target) {
        const example: ExampleView = {
          id: demoExampleId++,
          classId: target.id,
          path: `examples/${target.id}/harvest.png`,
          width: 120,
          height: 96,
          preview: demoPreview(target.color, target.examples),
          bbox: "",
          role: decision === "accept" ? "positive" : "negative",
          createdAt: new Date().toISOString(),
        };
        const classRows = demoExamples.get(target.id) ?? [];
        demoExamples.set(target.id, [example, ...classRows]);
        target.examples = classRows.length + 1;
      }
      return;
    }
    return;
  }
  await invoke("candidate_resolve", { id, decision, classId: classId ?? null });
}

export async function previewSource(id: number): Promise<FramePayload> {
  if (!isDesktop) {
    const frame = demoFrame();
    return { data: frame.preview, width: frame.width, height: frame.height };
  }
  return invoke<FramePayload>("source_preview", { id });
}

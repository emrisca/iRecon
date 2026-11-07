export type Project = {
  id: number;
  name: string;
  slug: string;
  path: string;
  createdAt: string;
};

export type CaptureStats = {
  running: boolean;
  monitor: string;
  width: number;
  height: number;
  fps: number;
  captureMs: number;
  inferenceMs: number;
  detections: number;
  poolClasses: number;
  poolExamples: number;
  frames: number;
  errors: number;
  elapsedS: number;
  model: string | null;
  note: string | null;
};

export type FramePayload = {
  data: string;
  width: number;
  height: number;
};

export type Detection = {
  label: string;
  score: number;
  x: number;
  y: number;
  w: number;
  h: number;
};

export type TrackedBox = Detection & {
  track: number;
  seen: number;
};

export type DetectionPayload = {
  width: number;
  height: number;
  inferenceMs: number;
  boxes: TrackedBox[];
};

export type Box = { x: number; y: number; w: number; h: number };

export type FrameView = {
  id: number;
  path: string;
  width: number;
  height: number;
  preview: string;
  createdAt: string;
};

export type LabelClass = {
  id: number;
  name: string;
  color: string;
  threshold: number;
  examples: number;
};

export type ExampleView = {
  id: number;
  classId: number;
  path: string;
  width: number;
  height: number;
  preview: string;
  bbox: string;
  createdAt: string;
};

export type ModelStatus = {
  id: string;
  name: string;
  note: string;
  path: string;
  installed: boolean;
  verified: boolean;
  bytes: number;
  expectedBytes: number;
  expectedSha: string;
  inputSize: number;
  url: string;
  active: boolean;
  detail: string | null;
};

export type ModelCatalog = {
  active: string | null;
  models: ModelStatus[];
};

export type ModelProgress = {
  id: string;
  downloaded: number;
  total: number;
  phase: "downloading" | "ready" | "loaded" | "failed" | string;
  message: string;
};

export type ClassMatch = {
  classId: number;
  name: string;
  score: number;
  similarity: number;
  margin: number;
  bestExample: number;
  positives: number;
  negatives: number;
};

export type DatasetExport = {
  path: string;
  frames: number;
  boxes: number;
  classes: number;
  crops: number;
  skipped: number;
};

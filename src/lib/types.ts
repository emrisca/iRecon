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

export type DetectionPayload = {
  width: number;
  height: number;
  inferenceMs: number;
  boxes: Detection[];
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
  path: string;
  installed: boolean;
  verified: boolean;
  bytes: number;
  expectedBytes: number;
  expectedSha: string;
  url: string;
  loaded: boolean;
  detail: string | null;
};

export type ModelProgress = {
  downloaded: number;
  total: number;
  phase: "downloading" | "ready" | "loaded" | "failed" | string;
  message: string;
};

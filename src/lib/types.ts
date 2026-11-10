export type Project = {
  id: number;
  name: string;
  slug: string;
  path: string;
  createdAt: string;
  /** Every saved example across the project's classes. */
  examples: number;
  frames: number;
  classes: number;
};

export type CaptureStats = {
  running: boolean;
  monitor: string;
  backend: string;
  source: string;
  width: number;
  height: number;
  fps: number;
  captureMs: number;
  captureP50Ms: number;
  captureP95Ms: number;
  captureMaxMs: number;
  frameGapMs: number;
  inferenceMs: number;
  preprocessMs: number;
  matchMs: number;
  latencyMs: number;
  dropped: number;
  detections: number;
  bestScore: number | null;
  bestClass: string | null;
  poolClasses: number;
  poolExamples: number;
  frames: number;
  errors: number;
  elapsedS: number;
  model: string | null;
  note: string | null;
};

export type PaintStats = {
  fps: number;
  drawMs: number;
  boxes: number;
  calibration: boolean;
};

export type OverlayState = {
  visible: boolean;
  calibration: boolean;
  hotkey: string;
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
  preprocessMs: number;
  latencyMs: number;
  capturedAtMs: number;
  publishedAtMs: number;
  crop: Box | null;
  colors: Record<string, string>;
  boxes: TrackedBox[];
};

export type Box = { x: number; y: number; w: number; h: number };

export type FrameView = {
  id: number;
  path: string;
  width: number;
  height: number;
  preview: string;
  /** Thumbnail on disk, loaded through the asset protocol in the desktop app. */
  thumbPath?: string;
  createdAt: string;
  /** Saved examples on this frame, across every class. */
  examples: number;
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
  /** Crop file on disk, loaded through the asset protocol in the desktop app. */
  thumbPath?: string;
  bbox: string;
  role: "positive" | "negative";
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

export type FrameTruth = {
  frameId: number;
  classIds: number[];
};

export type EvalFrame = {
  frameId: number;
  width: number;
  height: number;
  proposals: number;
  matches: number;
  bestScore: number | null;
  bestClass: string | null;
  matchedBoxes: Detection[];
  matchedClassIds: number[];
};

export type ClassRecall = {
  classId: number;
  name: string;
  color: string;
  marked: number;
  hits: number;
  falsePositives: number;
};

export type EvalReport = {
  frames: EvalFrame[];
  classRecall: ClassRecall[];
  framesWithMatch: number;
  proposals: number;
  matches: number;
};

export type DatasetExport = {
  path: string;
  frames: number;
  boxes: number;
  classes: number;
  crops: number;
  skipped: number;
};

export type SourceKind = "display" | "window" | "file" | "folder";

/** One display or window the source picker can offer. */
export type CaptureTarget = {
  target: string;
  label: string;
  detail: string;
};

export type SourceView = {
  id: number;
  projectId: number;
  kind: SourceKind;
  label: string;
  target: string;
  crop: Box | null;
};

export type ImportReport = {
  imported: number;
  skipped: number;
  errors: string[];
};

export type RoboflowReport = {
  frames: number;
  examples: number;
  classes: number;
  skipped: number;
  cancelled: boolean;
  errors: string[];
};

export type RoboflowProgress = {
  processed: number;
  total: number;
  frames: number;
  examples: number;
  skipped: number;
};

export type ClassAdvice = {
  classId: number;
  name: string;
  positives: number;
  negatives: number;
  similarity: number;
  threshold: number;
  suggested: number | null;
  hints: string[];
};

export type ProjectDetector = {
  id: number;
  projectId: number;
  name: string;
  path: string;
  inputSize: number;
  active: boolean;
  createdAt: string;
};

export type ComparisonSide = {
  name: string;
  inputSize: number;
  frames: number;
  proposals: number;
  matches: number;
  averageMs: number;
};

export type ComparisonFrame = {
  frameId: number;
  index: number;
  shippedProposals: number;
  shippedMatches: number;
  shippedMs: number;
  projectProposals: number;
  projectMatches: number;
  projectMs: number;
};

export type DetectorComparison = {
  shipped: ComparisonSide;
  project: ComparisonSide;
  frames: ComparisonFrame[];
};

export type SessionView = {
  id: number;
  projectId: number;
  startedAt: string;
  endedAt: string | null;
  durationMs: number;
  width: number;
  height: number;
  frames: number;
  detections: number;
  model: string | null;
};

export type SessionFrame = {
  id: number;
  sessionId: number;
  index: number;
  atMs: number;
  width: number;
  height: number;
  boxes: Detection[];
};

export type SessionEvalFrame = {
  frameId: number;
  boxes: Detection[];
};

export type SessionAuditFrame = {
  frameId: number;
  index: number;
  proposals: number;
  primary: number;
  vehicles: number;
};

export type SessionAudit = {
  sessionId: number;
  model: string;
  frames: number;
  proposals: number;
  primary: number;
  vehicles: number;
  averageMs: number;
  rows: SessionAuditFrame[];
};

export type CandidateView = {
  id: number;
  projectId: number;
  classId: number;
  className: string;
  color: string;
  path: string;
  thumb: string;
  thumbPath?: string;
  score: number;
  state: "pending" | "accepted" | "rejected" | "skipped";
  createdAt: string;
};

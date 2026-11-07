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
  frames: number;
  errors: number;
  elapsedS: number;
  note: string | null;
};

export type FramePayload = {
  data: string;
  width: number;
  height: number;
};

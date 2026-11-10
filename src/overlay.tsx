import { useEffect, useRef } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { Box, DetectionPayload, TrackedBox } from "./lib/types";

const LINE = "rgba(255, 255, 255, 0.92)";
const HALO = "rgba(0, 0, 0, 0.85)";
const STALE_MS = 1600;
/** Default time a box takes to ease toward its newest position. Detections
 *  arrive ten to thirty times a second, so easing fills the gap. Capture lab
 *  overrides this at run time through the overlay_smoothing command. */
const SMOOTH_DEFAULT_MS = 55;

type Sample = {
  boxes: TrackedBox[];
  at: number;
  crop: Box | null;
};

// Twenty boxes, because that is the number G4 is written against. They drift on
// sine paths rather than jumping, so the paint clock is measured while drawing
// and while repositioning every box each frame.
function calibrationBoxes(seconds: number): TrackedBox[] {
  const columns = 5;
  const rows = 4;
  return Array.from({ length: columns * rows }, (_, index) => {
    const column = index % columns;
    const row = Math.floor(index / columns);
    const cx = 0.1 + column * 0.2 + 0.035 * Math.sin(seconds * 0.9 + index * 0.7);
    const cy = 0.14 + row * 0.24 + 0.035 * Math.cos(seconds * 1.1 + index * 0.9);
    const w = 0.11 + 0.02 * Math.sin(seconds * 0.7 + index);
    const h = 0.1 + 0.02 * Math.cos(seconds * 0.6 + index * 0.5);
    return {
      track: index + 1,
      label: `calibration ${index + 1}`,
      score: 1,
      x: cx - w / 2,
      y: cy - h / 2,
      w,
      h,
      seen: 1,
    };
  });
}

export default function Overlay() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    // desynchronized keeps the paint off the compositor's normal schedule,
    // which is the difference between drawing on the frame and drawing late.
    const context = canvas.getContext("2d", { desynchronized: true });
    if (!context) return;

    let calibration = false;
    let latest: Sample | null = null;
    let smoothMs = SMOOTH_DEFAULT_MS;
    const easing = new Map<number, { x: number; y: number; w: number; h: number }>();
    // Previous sample positions and times, so each paint can carry a box along
    // its own velocity instead of waiting for the next detection.
    let previous: { boxes: Map<number, { x: number; y: number }>; at: number } | null = null;
    let velocities = new Map<number, { x: number; y: number }>();
    let lastPaint = 0;
    let colors: Record<string, string> = {};
    let size = { width: 0, height: 0 };
    let handle = 0;
    let reportAt = performance.now();
    let reported = 0;
    let drawTotal = 0;

    const resize = () => {
      const ratio = window.devicePixelRatio || 1;
      size = { width: window.innerWidth, height: window.innerHeight };
      canvas.width = Math.floor(size.width * ratio);
      canvas.height = Math.floor(size.height * ratio);
      canvas.style.width = `${size.width}px`;
      canvas.style.height = `${size.height}px`;
      context.setTransform(ratio, 0, 0, ratio, 0, 0);
    };

    const drawLabel = (text: string, x: number, y: number, color: string) => {
      context.font = "600 15px system-ui, sans-serif";
      const textWidth = context.measureText(text).width;
      context.fillStyle = HALO;
      context.fillRect(x, Math.max(0, y - 24), textWidth + 14, 24);
      context.fillStyle = color;
      context.fillText(text, x + 7, Math.max(16, y - 7));
    };

    const drawBox = (
      box: TrackedBox,
      rect: { x: number; y: number; w: number; h: number },
      crop: Box | null
    ) => {
      const originX = crop?.x ?? 0;
      const originY = crop?.y ?? 0;
      const scaleX = crop?.w ?? 1;
      const scaleY = crop?.h ?? 1;
      const left = (originX + rect.x * scaleX) * size.width;
      const top = (originY + rect.y * scaleY) * size.height;
      const width = rect.w * scaleX * size.width;
      const height = rect.h * scaleY * size.height;
      if (width < 2 || height < 2) return;

      context.lineWidth = 5;
      context.strokeStyle = HALO;
      context.strokeRect(left, top, width, height);
      const color = colors[box.label] ?? LINE;
      context.lineWidth = 2.5;
      context.strokeStyle = color;
      context.strokeRect(left, top, width, height);
      drawLabel(`${box.label} ${box.score.toFixed(2)}`, left, top, color);
    };

    const paint = (now: number) => {
      handle = requestAnimationFrame(paint);
      const drawStarted = performance.now();
      const frameMs = lastPaint > 0 ? now - lastPaint : 16.7;
      lastPaint = now;
      context.clearRect(0, 0, size.width, size.height);

      let boxes = 0;
      if (calibration) {
        easing.clear();
        for (const box of calibrationBoxes(now / 1000)) {
          drawBox(box, box, null);
          boxes += 1;
        }
      } else if (latest) {
        const age = Date.now() - latest.at;
        if (age > STALE_MS) {
          latest = null;
          previous = null;
          velocities.clear();
          easing.clear();
        } else {
          // Ease toward the newest box instead of predicting where it is going.
          // Prediction at ten to thirty detections a second overshoots between
          // samples and snaps back when the next one lands, which reads as
          // jitter even when the tracker itself is steady.
          const alpha = smoothMs > 0 ? 1 - Math.exp(-frameMs / smoothMs) : 1;
          const gap =
            previous && latest.at > previous.at
              ? Math.min(250, Math.max(8, latest.at - previous.at))
              : 16.7;
          // Carry each box forward by the sample age plus the easing time
          // constant. Easing alone puts the box a filter's worth of distance
          // behind a moving target; the lead cancels that without the wild
          // overshoot of extrapolating past the next detection.
          const lead = Math.min(gap + smoothMs, Math.max(0, age) + smoothMs);
          const live = new Set<number>();
          for (const box of latest.boxes) {
            live.add(box.track);
            const before = easing.get(box.track);
            const next = before
              ? {
                  x: before.x + (box.x - before.x) * alpha,
                  y: before.y + (box.y - before.y) * alpha,
                  w: before.w + (box.w - before.w) * alpha,
                  h: before.h + (box.h - before.h) * alpha,
                }
              : { x: box.x, y: box.y, w: box.w, h: box.h };
            easing.set(box.track, next);
            const velocity = velocities.get(box.track);
            const drawn = velocity
              ? {
                  x: next.x + velocity.x * lead,
                  y: next.y + velocity.y * lead,
                  w: next.w,
                  h: next.h,
                }
              : next;
            drawBox(box, drawn, latest.crop);
            boxes += 1;
          }
          for (const track of easing.keys()) {
            if (!live.has(track)) easing.delete(track);
          }
        }
      }

      const drawMs = performance.now() - drawStarted;
      reported += 1;
      drawTotal += drawMs;
      if (now - reportAt >= 1000) {
        const seconds = (now - reportAt) / 1000;
        void invoke("overlay_paint", {
          fps: reported / seconds,
          drawMs: reported > 0 ? drawTotal / reported : 0,
          boxes,
        }).catch(() => {});
        reportAt = now;
        reported = 0;
        drawTotal = 0;
      }
    };

    resize();
    window.addEventListener("resize", resize);
    handle = requestAnimationFrame(paint);

    let offDetections: (() => void) | undefined;
    let offCalibration: (() => void) | undefined;
    let offSmoothing: (() => void) | undefined;
    if (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window) {
      listen<DetectionPayload>("spike:detections", (event) => {
        const payload = event.payload;
        colors = payload.colors ?? {};
        const at = payload.publishedAtMs || Date.now();
        if (previous && at > previous.at) {
          const elapsed = at - previous.at;
          const next = new Map<number, { x: number; y: number }>();
          for (const box of payload.boxes) {
            const before = previous.boxes.get(box.track);
            if (!before) continue;
            next.set(box.track, {
              x: (box.x - before.x) / elapsed,
              y: (box.y - before.y) / elapsed,
            });
          }
          velocities = next;
        }
        previous = {
          boxes: new Map(payload.boxes.map((box) => [box.track, { x: box.x, y: box.y }])),
          at,
        };
        latest = { boxes: payload.boxes, at, crop: payload.crop };
      }).then((unlisten) => {
        offDetections = unlisten;
      });
      listen<boolean>("spike:calibration", (event) => {
        calibration = event.payload;
        latest = null;
        previous = null;
        velocities.clear();
        easing.clear();
      }).then((unlisten) => {
        offCalibration = unlisten;
      });
      listen<number>("spike:smoothing", (event) => {
        const next = Number(event.payload);
        if (Number.isFinite(next) && next >= 0) smoothMs = next;
      }).then((unlisten) => {
        offSmoothing = unlisten;
      });
      void invoke<number>("overlay_smoothing", { value: null })
        .then((value) => {
          if (Number.isFinite(value) && value >= 0) smoothMs = value;
        })
        .catch(() => {});
      void invoke<{ calibration: boolean }>("overlay_state")
        .then((state) => {
          calibration = state.calibration;
        })
        .catch(() => {});
    }

    return () => {
      cancelAnimationFrame(handle);
      offDetections?.();
      offCalibration?.();
      offSmoothing?.();
      window.removeEventListener("resize", resize);
    };
  }, []);

  return <canvas ref={canvasRef} />;
}

// The overlay window loads this module directly, so it mounts itself.
const container = document.getElementById("overlay-root");
if (container) {
  createRoot(container).render(<Overlay />);
}

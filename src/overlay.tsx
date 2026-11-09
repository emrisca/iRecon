import { useEffect, useRef } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { Box, DetectionPayload, TrackedBox } from "./lib/types";

const LINE = "rgba(255, 255, 255, 0.92)";
const HALO = "rgba(0, 0, 0, 0.85)";
const STALE_MS = 1600;
/** How far a box may be carried past its detection before the guess is worse
 *  than the staleness. At 12 detections a second this covers one interval. */
const MAX_EXTRAPOLATION_MS = 180;

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
    let previous: Sample | null = null;
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

    const drawLabel = (text: string, x: number, y: number) => {
      context.font = "600 15px system-ui, sans-serif";
      const textWidth = context.measureText(text).width;
      context.fillStyle = HALO;
      context.fillRect(x, Math.max(0, y - 24), textWidth + 14, 24);
      context.fillStyle = "#ffffff";
      context.fillText(text, x + 7, Math.max(16, y - 7));
    };

    const drawBox = (box: TrackedBox, sample: Sample | null, ageMs: number) => {
      const crop = sample?.crop;
      let x = box.x;
      let y = box.y;
      if (sample && previous && sample.at > previous.at) {
        const before = previous.boxes.find((entry) => entry.track === box.track);
        if (before) {
          const elapsed = sample.at - previous.at;
          x += ((box.x - before.x) * ageMs) / elapsed;
          y += ((box.y - before.y) * ageMs) / elapsed;
        }
      }
      const originX = crop?.x ?? 0;
      const originY = crop?.y ?? 0;
      const scaleX = crop?.w ?? 1;
      const scaleY = crop?.h ?? 1;
      const left = (originX + x * scaleX) * size.width;
      const top = (originY + y * scaleY) * size.height;
      const width = box.w * scaleX * size.width;
      const height = box.h * scaleY * size.height;
      if (width < 2 || height < 2) return;

      context.lineWidth = 5;
      context.strokeStyle = HALO;
      context.strokeRect(left, top, width, height);
      context.lineWidth = 2.5;
      context.strokeStyle = LINE;
      context.strokeRect(left, top, width, height);
      drawLabel(`${box.label} ${box.score.toFixed(2)}`, left, top);
    };

    const paint = (now: number) => {
      handle = requestAnimationFrame(paint);
      const drawStarted = performance.now();
      context.clearRect(0, 0, size.width, size.height);

      let boxes = 0;
      if (calibration) {
        for (const box of calibrationBoxes(now / 1000)) {
          drawBox(box, null, 0);
          boxes += 1;
        }
      } else if (latest) {
        const clock = Date.now();
        const age = clock - latest.at;
        if (age > STALE_MS) {
          latest = null;
          previous = null;
        } else {
          const carry = Math.min(MAX_EXTRAPOLATION_MS, Math.max(0, age));
          for (const box of latest.boxes) {
            drawBox(box, latest, carry);
            boxes += 1;
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
    if (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window) {
      listen<DetectionPayload>("spike:detections", (event) => {
        const payload = event.payload;
        previous = latest;
        latest = {
          boxes: payload.boxes,
          at: payload.publishedAtMs || Date.now(),
          crop: payload.crop,
        };
      }).then((unlisten) => {
        offDetections = unlisten;
      });
      listen<boolean>("spike:calibration", (event) => {
        calibration = event.payload;
        latest = null;
        previous = null;
      }).then((unlisten) => {
        offCalibration = unlisten;
      });
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

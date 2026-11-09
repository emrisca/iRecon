import { useEffect, useRef } from "react";
import { createRoot } from "react-dom/client";
import { listen } from "@tauri-apps/api/event";

import type { Detection, DetectionPayload } from "./lib/types";

const LINE = "rgba(255, 255, 255, 0.92)";
const HALO = "rgba(0, 0, 0, 0.85)";
const STALE_MS = 2000;

export default function Overlay() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const latest = useRef<{ boxes: Detection[] } | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const context = canvas.getContext("2d");
    if (!context) return;

    let clearTimer = 0;

    const drawBox = (box: Detection) => {
      const x = box.x * window.innerWidth;
      const y = box.y * window.innerHeight;
      const width = box.w * window.innerWidth;
      const height = box.h * window.innerHeight;
      const label = `${box.label} ${box.score.toFixed(2)}`;

      context.lineWidth = 5;
      context.strokeStyle = HALO;
      context.strokeRect(x, y, width, height);
      context.lineWidth = 2.5;
      context.strokeStyle = LINE;
      context.strokeRect(x, y, width, height);

      context.font = "600 15px system-ui, sans-serif";
      const textWidth = context.measureText(label).width;
      context.fillStyle = HALO;
      context.fillRect(x, Math.max(0, y - 24), textWidth + 14, 24);
      context.fillStyle = "#ffffff";
      context.fillText(label, x + 7, Math.max(16, y - 7));
    };

    const paint = () => {
      context.clearRect(0, 0, window.innerWidth, window.innerHeight);
      const current = latest.current;
      if (!current) return;
      for (const box of current.boxes) {
        drawBox(box);
      }
    };

    const resize = () => {
      const ratio = window.devicePixelRatio || 1;
      canvas.width = Math.floor(window.innerWidth * ratio);
      canvas.height = Math.floor(window.innerHeight * ratio);
      canvas.style.width = `${window.innerWidth}px`;
      canvas.style.height = `${window.innerHeight}px`;
      context.setTransform(ratio, 0, 0, ratio, 0, 0);
      paint();
    };

    resize();
    window.addEventListener("resize", resize);

    let off: (() => void) | undefined;
    if (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window) {
      listen<DetectionPayload>("spike:detections", (event) => {
        latest.current = { boxes: event.payload.boxes };
        paint();
        // One timer per burst: the screen clears itself once frames stop, so an
        // idle overlay costs nothing.
        window.clearTimeout(clearTimer);
        clearTimer = window.setTimeout(() => {
          latest.current = null;
          paint();
        }, STALE_MS);
      }).then((unlisten) => {
        off = unlisten;
      });
    }

    return () => {
      off?.();
      window.clearTimeout(clearTimer);
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

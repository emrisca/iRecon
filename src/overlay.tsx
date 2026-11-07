import { useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";

import type { Detection, DetectionPayload } from "./lib/types";

const LINE = "rgba(255, 255, 255, 0.92)";
const HALO = "rgba(0, 0, 0, 0.85)";
const STALE_MS = 2000;

type Sample = { width: number; height: number; cx: number; cy: number; label: string };

export default function Overlay() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const live = useRef<{ boxes: Detection[]; at: number } | null>(null);

  useEffect(() => {
    if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) return;
    let off: (() => void) | undefined;
    listen<DetectionPayload>("spike:detections", (event) => {
      live.current = { boxes: event.payload.boxes, at: performance.now() };
    }).then((unlisten) => {
      off = unlisten;
    });
    return () => {
      off?.();
    };
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const context = canvas.getContext("2d");
    if (!context) return;

    let animation = 0;

    const resize = () => {
      const ratio = window.devicePixelRatio || 1;
      canvas.width = Math.floor(window.innerWidth * ratio);
      canvas.height = Math.floor(window.innerHeight * ratio);
      canvas.style.width = `${window.innerWidth}px`;
      canvas.style.height = `${window.innerHeight}px`;
      context.setTransform(ratio, 0, 0, ratio, 0, 0);
    };

    const drawBox = (
      x: number,
      y: number,
      width: number,
      height: number,
      label: string
    ) => {
      context.lineWidth = 4;
      context.strokeStyle = HALO;
      context.strokeRect(x, y, width, height);
      context.lineWidth = 2;
      context.strokeStyle = LINE;
      context.strokeRect(x, y, width, height);

      context.font = "500 12px -apple-system, system-ui, sans-serif";
      const textWidth = context.measureText(label).width;
      context.fillStyle = HALO;
      context.fillRect(x, y - 20, textWidth + 12, 20);
      context.fillStyle = "#ffffff";
      context.fillText(label, x + 6, y - 6);
    };

    const draw = (time: number) => {
      const seconds = time / 1000;
      const width = window.innerWidth;
      const height = window.innerHeight;

      context.clearRect(0, 0, width, height);

      context.lineWidth = 3;
      context.strokeStyle = HALO;
      context.strokeRect(1, 1, width - 2, height - 2);
      context.lineWidth = 1;
      context.strokeStyle = LINE;
      context.strokeRect(1, 1, width - 2, height - 2);

      const current = live.current;
      const since = current ? performance.now() - current.at : Number.POSITIVE_INFINITY;

      if (current && since < STALE_MS) {
        for (const box of current.boxes) {
          drawBox(
            box.x * width,
            box.y * height,
            box.w * width,
            box.h * height,
            `${box.label} ${box.score.toFixed(2)}`
          );
        }
      } else {
        const samples: Sample[] = [
          {
            width: 0.16,
            height: 0.2,
            cx: 0.5 + Math.sin(seconds * 0.6) * 0.22,
            cy: 0.45,
            label: "sample, no detections",
          },
          {
            width: 0.1,
            height: 0.16,
            cx: 0.5 + Math.sin(seconds * 0.9 + 2) * 0.3,
            cy: 0.62 + Math.cos(seconds * 0.5) * 0.08,
            label: "sample",
          },
          {
            width: 0.22,
            height: 0.14,
            cx: 0.5 + Math.cos(seconds * 0.4) * 0.3,
            cy: 0.3 + Math.sin(seconds * 0.7) * 0.06,
            label: "sample",
          },
        ];
        for (const sample of samples) {
          drawBox(
            (sample.cx - sample.width / 2) * width,
            (sample.cy - sample.height / 2) * height,
            sample.width * width,
            sample.height * height,
            sample.label
          );
        }
      }

      context.font = "500 11px -apple-system, system-ui, sans-serif";
      context.fillStyle = "rgba(255, 255, 255, 0.75)";
      context.fillText("iRecon overlay, click-through", 10, height - 10);

      animation = requestAnimationFrame(draw);
    };

    resize();
    window.addEventListener("resize", resize);
    animation = requestAnimationFrame(draw);

    return () => {
      cancelAnimationFrame(animation);
      window.removeEventListener("resize", resize);
    };
  }, []);

  return <canvas ref={canvasRef} />;
}

import { useEffect, useRef } from "react";

const LINE = "rgba(255, 255, 255, 0.92)";
const HALO = "rgba(0, 0, 0, 0.85)";

type Box = { width: number; height: number; cx: number; cy: number; label: string };

export default function Overlay() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const context = canvas.getContext("2d");
    if (!context) return;

    let frame = 0;

    const resize = () => {
      const ratio = window.devicePixelRatio || 1;
      canvas.width = Math.floor(window.innerWidth * ratio);
      canvas.height = Math.floor(window.innerHeight * ratio);
      canvas.style.width = `${window.innerWidth}px`;
      canvas.style.height = `${window.innerHeight}px`;
      context.setTransform(ratio, 0, 0, ratio, 0, 0);
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

      const boxes: Box[] = [
        {
          width: 0.16,
          height: 0.2,
          cx: 0.5 + Math.sin(seconds * 0.6) * 0.22,
          cy: 0.45,
          label: "sample 1",
        },
        {
          width: 0.1,
          height: 0.16,
          cx: 0.5 + Math.sin(seconds * 0.9 + 2) * 0.3,
          cy: 0.62 + Math.cos(seconds * 0.5) * 0.08,
          label: "sample 2",
        },
        {
          width: 0.22,
          height: 0.14,
          cx: 0.5 + Math.cos(seconds * 0.4) * 0.3,
          cy: 0.3 + Math.sin(seconds * 0.7) * 0.06,
          label: "sample 3",
        },
      ];

      for (const box of boxes) {
        const boxWidth = box.width * width;
        const boxHeight = box.height * height;
        const x = box.cx * width - boxWidth / 2;
        const y = box.cy * height - boxHeight / 2;

        context.lineWidth = 4;
        context.strokeStyle = HALO;
        context.strokeRect(x, y, boxWidth, boxHeight);
        context.lineWidth = 2;
        context.strokeStyle = LINE;
        context.strokeRect(x, y, boxWidth, boxHeight);

        context.font = "500 12px -apple-system, system-ui, sans-serif";
        const textWidth = context.measureText(box.label).width;
        context.fillStyle = HALO;
        context.fillRect(x, y - 20, textWidth + 12, 20);
        context.fillStyle = "#ffffff";
        context.fillText(box.label, x + 6, y - 6);
      }

      context.font = "500 11px -apple-system, system-ui, sans-serif";
      context.fillStyle = "rgba(255, 255, 255, 0.75)";
      context.fillText("iRecon overlay, click-through", 10, height - 10);

      frame = requestAnimationFrame(draw);
    };

    resize();
    window.addEventListener("resize", resize);
    frame = requestAnimationFrame(draw);

    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("resize", resize);
    };
  }, []);

  return <canvas ref={canvasRef} />;
}

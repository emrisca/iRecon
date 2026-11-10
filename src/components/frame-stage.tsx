import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";
import { Minus, Plus } from "lucide-react";

import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { frameFullImage } from "../lib/api";
import type { Box, FrameView } from "../lib/types";

const MIN_SCALE = 1;
const MAX_SCALE = 12;

type FrameStageProps = {
  frame: FrameView;
  box: Box | null;
  onBox: (box: Box | null) => void;
  overlay?: Array<{ box: Box; color?: string }>;
  action?: ReactNode;
  fitToFrame?: boolean;
  className?: string;
};

function clamp01(value: number) {
  return Math.min(1, Math.max(0, value));
}

function clampScale(value: number) {
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, value));
}

/**
 * A frame-sized image viewer: scroll zooms around the pointer, Shift-drag,
 * Space-drag, right-drag or middle-drag pans, drag draws a box, and double
 * click fits the frame again. The picture is transformed with CSS rather than
 * resized, so zoom cannot feed back into layout and the box is drawn in screen
 * space so its border stays one pixel at any zoom.
 */
export function FrameStage({
  frame,
  box,
  onBox,
  overlay = [],
  action,
  fitToFrame = true,
  className = "",
}: FrameStageProps) {
  const viewport = useRef<HTMLDivElement | null>(null);
  const picture = useRef<HTMLImageElement | null>(null);
  const [viewportSize, setViewportSize] = useState({ width: 0, height: 0 });
  const [scale, setScale] = useState(1);
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const [draft, setDraft] = useState<Box | null>(null);
  const [panning, setPanning] = useState(false);
  const drawStart = useRef<{ x: number; y: number } | null>(null);
  const panStart = useRef<{ x: number; y: number; ox: number; oy: number } | null>(null);
  const spaceHeld = useRef(false);

  const aspect = frame.width / Math.max(1, frame.height);
  const baseWidth =
    viewportSize.width === 0
      ? 0
      : fitToFrame
        ? viewportSize.width
        : viewportSize.height > 0
          ? Math.min(viewportSize.width, viewportSize.height * aspect)
          : viewportSize.width;
  const baseHeight = baseWidth > 0 ? baseWidth / aspect : 0;
  const viewWidth = baseWidth * scale;
  const viewHeight = baseHeight * scale;

  useEffect(() => {
    const element = viewport.current;
    if (!element) return;
    // The fit view follows the viewport width. The fullscreen view also
    // limits the frame to the available height so it cannot be clipped.
    const measure = () =>
      setViewportSize({ width: element.clientWidth, height: element.clientHeight });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    setScale(1);
    setOffset({ x: 0, y: 0 });
  }, [frame.id]);

  const limit = useCallback((value: number, content: number, view: number) => {
    if (content <= view) return (view - content) / 2;
    return Math.min(0, Math.max(view - content, value));
  }, []);

  const contain = useCallback(
    (x: number, y: number, nextScale: number) => {
      const element = viewport.current;
      if (!element || baseWidth === 0) return { x, y };
      return {
        x: limit(x, baseWidth * nextScale, element.clientWidth),
        y: limit(y, baseHeight * nextScale, element.clientHeight),
      };
    },
    [baseWidth, baseHeight, limit]
  );

  useEffect(() => {
    setOffset((current) => contain(current.x, current.y, scale));
  }, [scale, baseWidth, contain]);

  useEffect(() => {
    const element = viewport.current;
    if (!element) return;
    const onWheel = (event: WheelEvent) => {
      if (baseWidth === 0) return;
      event.preventDefault();
      const next = clampScale(scale * Math.exp(-event.deltaY * 0.0022));
      if (next === scale) return;
      const rect = element.getBoundingClientRect();
      const px = event.clientX - rect.left;
      const py = event.clientY - rect.top;
      const factor = next / scale;
      setOffset(contain(px - (px - offset.x) * factor, py - (py - offset.y) * factor, next));
      setScale(next);
    };
    element.addEventListener("wheel", onWheel, { passive: false });
    return () => element.removeEventListener("wheel", onWheel);
  }, [baseWidth, scale, offset, contain]);

  useEffect(() => {
    const onDown = (event: KeyboardEvent) => {
      if (event.code === "Space") spaceHeld.current = true;
    };
    const onUp = (event: KeyboardEvent) => {
      if (event.code === "Space") spaceHeld.current = false;
    };
    window.addEventListener("keydown", onDown);
    window.addEventListener("keyup", onUp);
    return () => {
      window.removeEventListener("keydown", onDown);
      window.removeEventListener("keyup", onUp);
    };
  }, []);

  function normalized(clientX: number, clientY: number) {
    const element = picture.current;
    if (!element) return { x: 0, y: 0 };
    const rect = element.getBoundingClientRect();
    return {
      x: clamp01((clientX - rect.left) / rect.width),
      y: clamp01((clientY - rect.top) / rect.height),
    };
  }

  function onPointerDown(event: ReactPointerEvent<HTMLDivElement>) {
    try {
      event.currentTarget.setPointerCapture(event.pointerId);
    } catch {
      // A pointer that already left the window cannot be captured; the drag
      // still works from the events that do arrive.
    }
    const pan =
      event.button === 1 ||
      event.button === 2 ||
      event.shiftKey ||
      spaceHeld.current;
    if (pan) {
      panStart.current = { x: event.clientX, y: event.clientY, ox: offset.x, oy: offset.y };
      setPanning(true);
      return;
    }
    if (event.button !== 0) return;
    const at = normalized(event.clientX, event.clientY);
    drawStart.current = at;
    onBox(null);
    setDraft({ x: at.x, y: at.y, w: 0, h: 0 });
  }

  function onPointerMove(event: ReactPointerEvent<HTMLDivElement>) {
    if (panStart.current) {
      const start = panStart.current;
      setOffset(
        contain(start.ox + (event.clientX - start.x), start.oy + (event.clientY - start.y), scale)
      );
      return;
    }
    if (!drawStart.current) return;
    const at = normalized(event.clientX, event.clientY);
    const from = drawStart.current;
    setDraft({
      x: Math.min(from.x, at.x),
      y: Math.min(from.y, at.y),
      w: Math.abs(at.x - from.x),
      h: Math.abs(at.y - from.y),
    });
  }

  function onPointerUp(event: ReactPointerEvent<HTMLDivElement>) {
    if (panStart.current) {
      panStart.current = null;
      setPanning(false);
      return;
    }
    const from = drawStart.current;
    drawStart.current = null;
    setDraft(null);
    if (!from) return;
    const at = normalized(event.clientX, event.clientY);
    const next = {
      x: Math.min(from.x, at.x),
      y: Math.min(from.y, at.y),
      w: Math.abs(at.x - from.x),
      h: Math.abs(at.y - from.y),
    };
    if (next.w > 0.004 && next.h > 0.004) {
      onBox(next);
    }
  }

  function zoomBy(factor: number) {
    const element = viewport.current;
    if (!element) return;
    const next = clampScale(scale * factor);
    if (next === scale) return;
    const cx = element.clientWidth / 2;
    const cy = element.clientHeight / 2;
    const k = next / scale;
    setOffset(contain(cx - (cx - offset.x) * k, cy - (cy - offset.y) * k, next));
    setScale(next);
  }

  function fit() {
    setScale(1);
    setOffset({ x: 0, y: 0 });
  }

  const active = draft ?? box;

  return (
    <div className="flex flex-col gap-1.5">
      <div
        ref={viewport}
        className={`relative overflow-hidden rounded-md border border-border bg-muted ${
          fitToFrame ? "w-full" : ""
        } ${className}`}
        style={{
          aspectRatio: fitToFrame ? `${frame.width} / ${frame.height}` : undefined,
          cursor: panning ? "grabbing" : "crosshair",
        }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={() => {
          drawStart.current = null;
          panStart.current = null;
          setDraft(null);
          setPanning(false);
        }}
        onDoubleClick={fit}
        onContextMenu={(event) => event.preventDefault()}
      >
        <div
          className="relative origin-top-left"
          style={{
            width: baseWidth || 1,
            height: baseHeight || 1,
            transform: `translate(${offset.x}px, ${offset.y}px) scale(${scale})`,
          }}
        >
          <img
            ref={picture}
            src={frameFullImage(frame)}
            alt="Grabbed frame"
            draggable={false}
            className="pointer-events-none block h-full w-full select-none"
          />
        </div>
        {overlay.map((entry, index) => (
          <div
            key={`${index}-${entry.box.x}-${entry.box.y}`}
            className="pointer-events-none absolute border-2 ring-1 ring-black/60"
            style={{
              left: offset.x + entry.box.x * viewWidth,
              top: offset.y + entry.box.y * viewHeight,
              width: entry.box.w * viewWidth,
              height: entry.box.h * viewHeight,
              borderColor: entry.color ?? "white",
            }}
          />
        ))}
        {active ? (
          <div
            className="pointer-events-none absolute border border-white ring-1 ring-black/60"
            style={{
              left: offset.x + active.x * viewWidth,
              top: offset.y + active.y * viewHeight,
              width: active.w * viewWidth,
              height: active.h * viewHeight,
            }}
          />
        ) : null}
      </div>

      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-1.5">
          <Button
            size="sm"
            variant="outline"
            aria-label="Zoom out"
            disabled={scale <= MIN_SCALE}
            onClick={() => zoomBy(1 / 1.4)}
          >
            <Minus className="size-4" aria-hidden="true" />
          </Button>
          <Badge variant="outline" className="min-w-14 justify-center tabular-nums">
            {Math.round(scale * 100)}%
          </Badge>
          <Button
            size="sm"
            variant="outline"
            aria-label="Zoom in"
            disabled={scale >= MAX_SCALE}
            onClick={() => zoomBy(1.4)}
          >
            <Plus className="size-4" aria-hidden="true" />
          </Button>
          <Button size="sm" variant="ghost" onClick={fit}>
            Fit
          </Button>
        </div>
        {action}
      </div>
    </div>
  );
}

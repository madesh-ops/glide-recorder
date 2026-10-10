// Pick a region of a display on a live thumbnail. Coordinates are physical
// display pixels; the engine crops the display capture to this rectangle.
import { useEffect, useRef, useState } from "react";
import { api, type DisplayInfo } from "../lib/api";

export interface Area { x: number; y: number; w: number; h: number }

const MIN = 64;

function clampArea(a: Area, d: DisplayInfo): Area {
  const w = Math.round(Math.min(Math.max(a.w, MIN), d.width));
  const h = Math.round(Math.min(Math.max(a.h, MIN), d.height));
  const x = Math.round(Math.min(Math.max(a.x, 0), d.width - w));
  const y = Math.round(Math.min(Math.max(a.y, 0), d.height - h));
  return { x, y, w, h };
}

export function centeredArea(d: DisplayInfo, w: number, h: number): Area {
  const s = Math.min(1, d.width / w, d.height / h);
  const aw = Math.round(w * s);
  const ah = Math.round(h * s);
  return clampArea({ x: (d.width - aw) / 2, y: (d.height - ah) / 2, w: aw, h: ah }, d);
}

const PRESETS: { label: string; w: number; h: number }[] = [
  { label: "1280 × 720", w: 1280, h: 720 },
  { label: "1920 × 1080", w: 1920, h: 1080 },
  { label: "Square", w: 1080, h: 1080 },
  { label: "Vertical 9:16", w: 608, h: 1080 },
];

type Drag =
  | { mode: "new"; x0: number; y0: number }
  | { mode: "move"; dx: number; dy: number }
  | { mode: "resize"; corner: "nw" | "ne" | "sw" | "se"; start: Area };

export function AreaPicker({ display, area, onChange }: { display: DisplayInfo; area: Area; onChange: (a: Area) => void }) {
  const [thumb, setThumb] = useState<string | null>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  const drag = useRef<Drag | null>(null);

  useEffect(() => {
    let alive = true;
    setThumb(null);
    api.sourceThumbnail({ kind: "display", id: display.id }, 960).then((u) => alive && setThumb(u)).catch(() => {});
    return () => {
      alive = false;
    };
  }, [display.id]);

  /** Pointer position in display pixels. */
  const toDisplay = (e: React.PointerEvent) => {
    const r = boxRef.current!.getBoundingClientRect();
    return {
      x: ((e.clientX - r.left) / r.width) * display.width,
      y: ((e.clientY - r.top) / r.height) * display.height,
    };
  };

  const onDown = (e: React.PointerEvent) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    const p = toDisplay(e);
    const target = (e.target as HTMLElement).dataset;
    if (target.corner) {
      drag.current = { mode: "resize", corner: target.corner as "nw", start: area };
    } else if (target.area) {
      drag.current = { mode: "move", dx: p.x - area.x, dy: p.y - area.y };
    } else {
      drag.current = { mode: "new", x0: p.x, y0: p.y };
    }
  };

  const onMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    const p = toDisplay(e);
    if (d.mode === "new") {
      onChange(clampArea({ x: Math.min(d.x0, p.x), y: Math.min(d.y0, p.y), w: Math.abs(p.x - d.x0), h: Math.abs(p.y - d.y0) }, display));
    } else if (d.mode === "move") {
      onChange(clampArea({ ...area, x: p.x - d.dx, y: p.y - d.dy }, display));
    } else {
      const s = d.start;
      const left = d.corner.includes("w") ? Math.min(p.x, s.x + s.w - MIN) : s.x;
      const top = d.corner.includes("n") ? Math.min(p.y, s.y + s.h - MIN) : s.y;
      const right = d.corner.includes("e") ? Math.max(p.x, s.x + MIN) : s.x + s.w;
      const bottom = d.corner.includes("s") ? Math.max(p.y, s.y + MIN) : s.y + s.h;
      onChange(clampArea({ x: Math.max(0, left), y: Math.max(0, top), w: Math.min(right, display.width) - Math.max(0, left), h: Math.min(bottom, display.height) - Math.max(0, top) }, display));
    }
  };

  const pct = (v: number, of: number) => `${(v / of) * 100}%`;

  return (
    <div className="area-picker">
      <div
        className="area-stage"
        ref={boxRef}
        style={{
          aspectRatio: `${display.width} / ${display.height}`,
          // Fit the viewport height so the presets stay visible above the record bar.
          width: `min(100%, calc((100vh - 400px) * ${display.width / display.height}))`,
          backgroundImage: thumb ? `url("${thumb}")` : undefined,
        }}
        onPointerDown={onDown}
        onPointerMove={onMove}
        onPointerUp={() => (drag.current = null)}
      >
        {!thumb && <div className="thumb-loading" />}
        <div
          className="area-rect"
          data-area="1"
          style={{ left: pct(area.x, display.width), top: pct(area.y, display.height), width: pct(area.w, display.width), height: pct(area.h, display.height) }}
        >
          {(["nw", "ne", "sw", "se"] as const).map((c) => (
            <span key={c} className={`area-handle ${c}`} data-corner={c} />
          ))}
          <span className="area-size">{area.w} × {area.h}</span>
        </div>
      </div>
      <div className="row area-presets">
        <span className="faint">Drag on the screen to select. Presets:</span>
        {PRESETS.map((p) => (
          <button key={p.label} className="btn sm" onClick={() => onChange(centeredArea(display, p.w, p.h))}>{p.label}</button>
        ))}
      </div>
    </div>
  );
}

// Sampling of the Rust-computed preview tracks and CSS helpers that mirror the
// export compositor (crates/studio-core/src/render.rs).
import type { Background, PreviewModel } from "./api";

export const REF_HEIGHT = 1080;
/** Mirrors render::FALLBACK_CURSOR_SCALE (built-in arrow, 34 px design at 1080p). */
export const FALLBACK_CURSOR_SCALE = 0.6;

export interface CameraSample { x: number; y: number; zoom: number }
export interface CursorSample { x: number; y: number; shape: number; opacity: number }

function frameIndex(p: PreviewModel, t: number, stride: number, arr: number[]) {
  const n = arr.length / stride;
  const f = Math.max(0, t * p.rate);
  const i = Math.min(Math.floor(f), n - 1);
  const j = Math.min(i + 1, n - 1);
  return { i, j, k: Math.min(Math.max(f - i, 0), 1) };
}

export function sampleCamera(p: PreviewModel, t: number): CameraSample {
  const a = p.camera;
  if (a.length < 3) return { x: 0.5, y: 0.5, zoom: 1 };
  const { i, j, k } = frameIndex(p, t, 3, a);
  return {
    x: a[i * 3] + (a[j * 3] - a[i * 3]) * k,
    y: a[i * 3 + 1] + (a[j * 3 + 1] - a[i * 3 + 1]) * k,
    zoom: a[i * 3 + 2] + (a[j * 3 + 2] - a[i * 3 + 2]) * k,
  };
}

export function sampleCursor(p: PreviewModel, t: number): CursorSample | null {
  const a = p.cursor;
  if (a.length < 4) return null;
  const { i, j, k } = frameIndex(p, t, 4, a);
  return {
    x: a[i * 4] + (a[j * 4] - a[i * 4]) * k,
    y: a[i * 4 + 1] + (a[j * 4 + 1] - a[i * 4 + 1]) * k,
    shape: k < 0.5 ? a[i * 4 + 2] : a[j * 4 + 2],
    opacity: a[i * 4 + 3] + (a[j * 4 + 3] - a[i * 4 + 3]) * k,
  };
}

/** Mirrors render::click_ripple_scale. */
export function rippleScale(progress: number): number {
  return 0.35 + 0.65 * (1 - Math.pow(1 - progress, 3));
}

/** CSS for a background, matching render_background (CSS gradient semantics). */
export function backgroundCss(bg: Background, imageUrl?: (path: string) => string): string {
  switch (bg.kind) {
    case "solid":
      return bg.color;
    case "gradient": {
      const stops = [...bg.stops].sort((a, b) => a.at - b.at).map((s) => `${s.color} ${(s.at * 100).toFixed(2)}%`);
      return `linear-gradient(${bg.angle}deg, ${stops.join(", ")})`;
    }
    case "image":
      return imageUrl ? `center / cover no-repeat url("${imageUrl(bg.path)}")` : "#222";
  }
}

/** Scene transform for camera (normalized x, y, zoom) on a W×H canvas. */
export function sceneTransform(cam: CameraSample, w: number, h: number): string {
  const tx = w * (0.5 - cam.x * cam.zoom);
  const ty = h * (0.5 - cam.y * cam.zoom);
  return `translate(${tx}px, ${ty}px) scale(${cam.zoom})`;
}

/** Original, royalty-free background presets. */
export const BACKGROUND_PRESETS: { name: string; bg: Background }[] = [
  { name: "Violet", bg: { kind: "gradient", angle: 135, stops: [{ color: "#3B2A8F", at: 0 }, { color: "#7B5CF0", at: 0.55 }, { color: "#C59BF7", at: 1 }] } },
  { name: "Dusk", bg: { kind: "gradient", angle: 160, stops: [{ color: "#1F1C4D", at: 0 }, { color: "#6B3FA0", at: 0.5 }, { color: "#F08A7E", at: 1 }] } },
  { name: "Ocean", bg: { kind: "gradient", angle: 145, stops: [{ color: "#0B3A5B", at: 0 }, { color: "#1C7FA6", at: 0.55 }, { color: "#7FD3D9", at: 1 }] } },
  { name: "Forest", bg: { kind: "gradient", angle: 150, stops: [{ color: "#0E2E25", at: 0 }, { color: "#2F7A5B", at: 0.6 }, { color: "#B7E3A8", at: 1 }] } },
  { name: "Sunset", bg: { kind: "gradient", angle: 120, stops: [{ color: "#FF8A5B", at: 0 }, { color: "#F25F7A", at: 0.5 }, { color: "#8E44AD", at: 1 }] } },
  { name: "Peach", bg: { kind: "gradient", angle: 135, stops: [{ color: "#FFD3A5", at: 0 }, { color: "#FD9F9F", at: 1 }] } },
  { name: "Mint", bg: { kind: "gradient", angle: 135, stops: [{ color: "#D4FC79", at: 0 }, { color: "#96E6A1", at: 1 }] } },
  { name: "Sky", bg: { kind: "gradient", angle: 180, stops: [{ color: "#A1C4FD", at: 0 }, { color: "#C2E9FB", at: 1 }] } },
  { name: "Graphite", bg: { kind: "gradient", angle: 160, stops: [{ color: "#2B2B30", at: 0 }, { color: "#121214", at: 1 }] } },
  { name: "Midnight", bg: { kind: "gradient", angle: 135, stops: [{ color: "#0F2027", at: 0 }, { color: "#203A43", at: 0.5 }, { color: "#2C5364", at: 1 }] } },
  { name: "Rose", bg: { kind: "gradient", angle: 135, stops: [{ color: "#F4C4F3", at: 0 }, { color: "#FC67FA", at: 1 }] } },
  { name: "Sand", bg: { kind: "gradient", angle: 135, stops: [{ color: "#E6DADA", at: 0 }, { color: "#B9A89A", at: 1 }] } },
];

export const SOLID_PRESETS = ["#0F0F11", "#1F1F24", "#FFFFFF", "#F2F0EB", "#6B52F5", "#2563EB", "#10B981", "#F59E0B", "#EF4444", "#EC4899"];

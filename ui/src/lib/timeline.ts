// Time mapping between source (recording) and output (edited video) time.
// Mirrors crates/studio-core/src/timeline.rs so the editor and export agree.
import type { Clip, Timeline, ZoomSegment } from "./api";

export const MIN_CLIP = 0.1;

export function outputDuration(tl: Timeline): number {
  return tl.clips.reduce((acc, c) => acc + (c.end - c.start) / c.speed, 0);
}

export function outputToSource(tl: Timeline, t: number): number {
  let acc = 0;
  for (const c of tl.clips) {
    const len = (c.end - c.start) / c.speed;
    if (t < acc + len) return c.start + Math.max(0, t - acc) * c.speed;
    acc += len;
  }
  const last = tl.clips[tl.clips.length - 1];
  return last ? last.end : 0;
}

export function sourceToOutput(tl: Timeline, t: number): number | null {
  let acc = 0;
  for (const c of tl.clips) {
    if (t >= c.start && t <= c.end) return acc + (t - c.start) / c.speed;
    acc += (c.end - c.start) / c.speed;
  }
  return null;
}

/** Like sourceToOutput, but times inside a cut snap to the next kept moment. */
export function sourceToOutputClamped(tl: Timeline, t: number): number {
  let acc = 0;
  for (const c of tl.clips) {
    if (t < c.start) return acc;
    if (t <= c.end) return acc + (t - c.start) / c.speed;
    acc += (c.end - c.start) / c.speed;
  }
  return acc;
}

export function clipAt(tl: Timeline, sourceT: number): Clip | undefined {
  return tl.clips.find((c) => sourceT >= c.start && sourceT < c.end);
}

/** Next playable source time at or after `t` (skipping cuts), or null at the end. */
export function nextPlayable(tl: Timeline, t: number): number | null {
  for (const c of tl.clips) {
    if (t < c.start) return c.start;
    if (t < c.end) return t;
  }
  return null;
}

function newId(): string {
  return Math.random().toString(16).slice(2, 14).padEnd(12, "0");
}

export function splitAt(tl: Timeline, t: number): Timeline | null {
  const i = tl.clips.findIndex((c) => t > c.start + MIN_CLIP && t < c.end - MIN_CLIP);
  if (i < 0) return null;
  const c = tl.clips[i];
  const clips = [...tl.clips];
  clips.splice(i, 1, { ...c, end: t }, { id: newId(), start: t, end: c.end, speed: c.speed });
  return { ...tl, clips };
}

export function removeClip(tl: Timeline, id: string): Timeline | null {
  if (tl.clips.length <= 1) return null;
  return { ...tl, clips: tl.clips.filter((c) => c.id !== id) };
}

/** Zoom segments in output time for display (segments fully inside cuts are hidden). */
export function segmentOutputRange(tl: Timeline, s: ZoomSegment): [number, number] | null {
  const a = sourceToOutputClamped(tl, s.start);
  const b = sourceToOutputClamped(tl, s.end);
  return b - a > 1e-3 ? [a, b] : null;
}

export function newSegmentId(): string {
  return newId();
}

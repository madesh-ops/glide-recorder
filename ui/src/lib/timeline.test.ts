import { describe, expect, it } from "vitest";
import type { Timeline } from "./api";
import { nextPlayable, outputDuration, outputToSource, sourceToOutput, sourceToOutputClamped, splitAt, removeClip } from "./timeline";

const tl = (): Timeline => ({
  sourceDuration: 10,
  clips: [
    { id: "a", start: 1, end: 3, speed: 1 },
    { id: "b", start: 5, end: 9, speed: 2 },
  ],
});

describe("timeline mapping (mirrors Rust)", () => {
  it("computes output duration with speed", () => {
    expect(outputDuration(tl())).toBeCloseTo(4);
  });
  it("maps output to source across cuts", () => {
    expect(outputToSource(tl(), 0)).toBeCloseTo(1);
    expect(outputToSource(tl(), 2)).toBeCloseTo(5);
    expect(outputToSource(tl(), 3)).toBeCloseTo(7);
    expect(outputToSource(tl(), 99)).toBeCloseTo(9);
  });
  it("maps source to output and reports cuts", () => {
    expect(sourceToOutput(tl(), 7)).toBeCloseTo(3);
    expect(sourceToOutput(tl(), 4)).toBeNull();
    expect(sourceToOutputClamped(tl(), 4)).toBeCloseTo(2);
    expect(sourceToOutputClamped(tl(), 0)).toBeCloseTo(0);
  });
  it("round-trips", () => {
    for (let o = 0; o < 4; o += 0.1) {
      expect(sourceToOutput(tl(), outputToSource(tl(), o))!).toBeCloseTo(o, 6);
    }
  });
  it("finds the next playable time", () => {
    expect(nextPlayable(tl(), 0)).toBe(1);
    expect(nextPlayable(tl(), 4)).toBe(5);
    expect(nextPlayable(tl(), 6)).toBe(6);
    expect(nextPlayable(tl(), 9.5)).toBeNull();
  });
  it("splits and removes clips", () => {
    const s = splitAt(tl(), 2)!;
    expect(s.clips.map((c) => [c.start, c.end])).toEqual([[1, 2], [2, 3], [5, 9]]);
    expect(splitAt(tl(), 4)).toBeNull();
    expect(removeClip(s, s.clips[1].id)!.clips.length).toBe(2);
    expect(removeClip({ sourceDuration: 1, clips: [{ id: "x", start: 0, end: 1, speed: 1 }] }, "x")).toBeNull();
  });
});

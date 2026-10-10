import { beforeEach, describe, expect, it } from "vitest";
import { useEditor } from "./editor";
import type { OpenedProject } from "../lib/api";

function fixture(): OpenedProject {
  return {
    dir: "C:\\p",
    videoPath: "C:\\p\\screen.mp4",
    hasCursor: true,
    preview: { layout: { aspect: 16 / 9, screen: { x: 0, y: 0, w: 1, h: 1 }, sourceWidth: 1920, sourceHeight: 1080 }, rate: 60, camera: [], cursor: [], clicks: [], shapes: [] },
    project: {
      version: 1, id: "x", name: "Demo", createdAt: 0,
      source: { kind: "display", label: "D", width: 1920, height: 1080, fps: 60, scaleFactor: 1 },
      media: { video: "screen.mp4" },
      recording: { duration: 10, framesWritten: 600, framesDropped: 0, encoder: "libx264", pauseCount: 0 },
      export: { height: 1080, fps: 60, encoder: "auto", quality: "high" },
      edit: {
        timeline: { sourceDuration: 10, clips: [{ id: "c", start: 0, end: 10, speed: 1 }] },
        zoom: { settings: {} as never, segments: [] },
        cursor: { visible: true, scale: 1.5, smoothing: 0.1, hideWhenIdle: 0, clickEffect: true, clickColor: "#fff", clickSize: 36, clickDuration: 0.45 },
        style: { aspect: "auto", background: { kind: "solid", color: "#000" }, padding: 0.1, cornerRadius: 12, shadowOpacity: 0.4, shadowBlur: 40, shadowOffsetY: 10 },
        audio: { micVolume: 1, systemVolume: 1, micMuted: false, systemMuted: false },
      },
    },
  };
}

describe("editor undo/redo", () => {
  beforeEach(() => useEditor.getState().load(fixture()));

  it("undoes and redoes discrete edits", () => {
    const s = useEditor.getState();
    s.update((e) => ({ ...e, cursor: { ...e.cursor, scale: 2 } }));
    s.update((e) => ({ ...e, cursor: { ...e.cursor, scale: 3 } }));
    expect(useEditor.getState().edit!.cursor.scale).toBe(3);
    useEditor.getState().undo();
    expect(useEditor.getState().edit!.cursor.scale).toBe(2);
    useEditor.getState().undo();
    expect(useEditor.getState().edit!.cursor.scale).toBe(1.5);
    useEditor.getState().undo(); // no-op at the beginning
    useEditor.getState().redo();
    expect(useEditor.getState().edit!.cursor.scale).toBe(2);
  });

  it("collapses a drag gesture into one undo step", () => {
    const s = useEditor.getState();
    s.beginGesture();
    for (let i = 1; i <= 20; i++) {
      useEditor.getState().update((e) => ({ ...e, style: { ...e.style, padding: i / 100 } }), { history: false });
    }
    useEditor.getState().endGesture();
    expect(useEditor.getState().past.length).toBe(1);
    useEditor.getState().undo();
    expect(useEditor.getState().edit!.style.padding).toBe(0.1);
  });

  it("a new edit clears the redo stack", () => {
    const s = useEditor.getState();
    s.update((e) => ({ ...e, audio: { ...e.audio, micMuted: true } }));
    useEditor.getState().undo();
    expect(useEditor.getState().future.length).toBe(1);
    useEditor.getState().update((e) => ({ ...e, audio: { ...e.audio, micVolume: 0.5 } }));
    expect(useEditor.getState().future.length).toBe(0);
  });

  it("gestures without changes add no history", () => {
    useEditor.getState().beginGesture();
    useEditor.getState().endGesture();
    expect(useEditor.getState().past.length).toBe(0);
  });
});

// Editor state: the non-destructive EditState with undo/redo, selection and playback.
import { create } from "zustand";
import type { EditState, ExportSettings, OpenedProject, PreviewModel, Project } from "../lib/api";

export type Selection = { kind: "zoom"; id: string } | { kind: "clip"; id: string } | null;
export type Panel = "background" | "cursor" | "zoom" | "audio" | "clip";

const HISTORY_LIMIT = 200;

interface EditorStore {
  dir: string;
  videoPath: string;
  project: Project | null;
  hasCursor: boolean;
  edit: EditState | null;
  exportSettings: ExportSettings | null;
  preview: PreviewModel | null;
  past: EditState[];
  future: EditState[];
  /** Snapshot taken when a drag gesture starts; committed as one undo step. */
  gestureBase: EditState | null;
  selection: Selection;
  panel: Panel;
  /** Current playhead in *source* seconds. */
  time: number;
  playing: boolean;
  saveState: "saved" | "saving" | "dirty" | "error";
  revision: number;

  load(p: OpenedProject): void;
  unload(): void;
  /** Apply an edit. `history: false` is used while a gesture is in progress. */
  update(fn: (e: EditState) => EditState, opts?: { history?: boolean }): void;
  beginGesture(): void;
  endGesture(): void;
  undo(): void;
  redo(): void;
  setPreview(p: PreviewModel): void;
  select(s: Selection): void;
  setPanel(p: Panel): void;
  setTime(t: number): void;
  setPlaying(p: boolean): void;
  setSaveState(s: EditorStore["saveState"]): void;
  setExportSettings(s: ExportSettings): void;
  setName(name: string): void;
}

const clone = <T,>(v: T): T => structuredClone(v);

export const useEditor = create<EditorStore>((set, get) => ({
  dir: "",
  videoPath: "",
  project: null,
  hasCursor: false,
  edit: null,
  exportSettings: null,
  preview: null,
  past: [],
  future: [],
  gestureBase: null,
  selection: null,
  panel: "background",
  time: 0,
  playing: false,
  saveState: "saved",
  revision: 0,

  load: (p) =>
    set({
      dir: p.dir,
      videoPath: p.videoPath,
      project: p.project,
      hasCursor: p.hasCursor,
      edit: clone(p.project.edit),
      exportSettings: clone(p.project.export),
      preview: p.preview,
      past: [],
      future: [],
      gestureBase: null,
      selection: null,
      panel: "background",
      time: p.project.edit.timeline.clips[0]?.start ?? 0,
      playing: false,
      saveState: "saved",
      revision: 0,
    }),

  unload: () => set({ project: null, edit: null, preview: null, past: [], future: [], dir: "", playing: false }),

  update: (fn, opts) => {
    const cur = get().edit;
    if (!cur) return;
    const next = fn(clone(cur));
    const history = opts?.history ?? true;
    if (history && !get().gestureBase) {
      const past = [...get().past, cur].slice(-HISTORY_LIMIT);
      set({ edit: next, past, future: [], saveState: "dirty", revision: get().revision + 1 });
    } else {
      set({ edit: next, saveState: "dirty", revision: get().revision + 1 });
    }
  },

  beginGesture: () => {
    const cur = get().edit;
    if (cur && !get().gestureBase) set({ gestureBase: cur });
  },

  endGesture: () => {
    const base = get().gestureBase;
    if (!base) return;
    const changed = JSON.stringify(base) !== JSON.stringify(get().edit);
    set({
      gestureBase: null,
      ...(changed ? { past: [...get().past, base].slice(-HISTORY_LIMIT), future: [] } : {}),
    });
  },

  undo: () => {
    const { past, edit, future } = get();
    if (!past.length || !edit) return;
    const prev = past[past.length - 1];
    set({ edit: prev, past: past.slice(0, -1), future: [edit, ...future], saveState: "dirty", revision: get().revision + 1 });
  },

  redo: () => {
    const { past, edit, future } = get();
    if (!future.length || !edit) return;
    const next = future[0];
    set({ edit: next, past: [...past, edit], future: future.slice(1), saveState: "dirty", revision: get().revision + 1 });
  },

  setPreview: (p) => set({ preview: p }),
  select: (s) =>
    set((st) => ({
      selection: s,
      panel: s?.kind === "zoom" ? "zoom" : s?.kind === "clip" ? "clip" : st.panel,
    })),
  setPanel: (panel) => set({ panel }),
  setTime: (time) => set({ time }),
  setPlaying: (playing) => set({ playing }),
  setSaveState: (saveState) => set({ saveState }),
  setExportSettings: (exportSettings) => set({ exportSettings, saveState: "dirty", revision: get().revision + 1 }),
  setName: (name) => {
    const p = get().project;
    if (p) set({ project: { ...p, name } });
  },
}));

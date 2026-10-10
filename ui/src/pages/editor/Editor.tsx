import { useEffect, useRef, useState } from "react";
import { ChevronLeft, Download, Check, Loader2, AlertCircle, Keyboard } from "lucide-react";
import { api, errorText } from "../../lib/api";
import { useEditor } from "../../store/editor";
import { TitleBar } from "../../components/TitleBar";
import { Modal } from "../../components/Modal";
import { toast } from "../../components/toast";
import { Preview } from "./Preview";
import { Timeline } from "./Timeline";
import { Rail, SidePanel } from "./SidePanel";
import { ExportDialog } from "./ExportDialog";
import { player } from "./player";
import { newSegmentId, outputToSource, removeClip, sourceToOutputClamped, splitAt } from "../../lib/timeline";
import "../../styles/editor.css";

const SHORTCUTS: [string, string][] = [
  ["Space", "Play / pause"],
  ["← / →", "Previous / next frame (Shift: 1 second)"],
  ["Home / End", "Jump to start / end"],
  ["S", "Split clip at playhead"],
  ["Z", "Add zoom at playhead"],
  ["Delete", "Remove selected zoom or clip"],
  ["Ctrl+Z / Ctrl+Shift+Z", "Undo / redo"],
  ["Ctrl+E", "Export"],
  ["?", "Show shortcuts"],
];

function NameField() {
  const name = useEditor((s) => s.project?.name ?? "");
  const dir = useEditor((s) => s.dir);
  const [value, setValue] = useState(name);
  useEffect(() => setValue(name), [name]);
  const commit = async () => {
    const v = value.trim();
    if (!v || v === name) return setValue(name);
    try {
      useEditor.getState().setName(await api.renameProject(dir, v));
    } catch (e) {
      toast.error("Rename failed", errorText(e));
      setValue(name);
    }
  };
  return (
    <input
      className="name-field"
      value={value}
      aria-label="Project name"
      onChange={(e) => setValue(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") (e.target as HTMLInputElement).blur();
        if (e.key === "Escape") {
          setValue(name);
          (e.target as HTMLInputElement).blur();
        }
        e.stopPropagation();
      }}
      spellCheck={false}
    />
  );
}

function SaveIndicator() {
  const s = useEditor((st) => st.saveState);
  return (
    <span className={`save-indicator ${s}`}>
      {s === "saving" || s === "dirty" ? <Loader2 size={12} className="spin" /> : s === "error" ? <AlertCircle size={12} /> : <Check size={12} />}
      {s === "saved" ? "Saved" : s === "error" ? "Not saved" : "Saving…"}
    </span>
  );
}

export function Editor({ dir, onClose }: { dir: string; onClose: () => void }) {
  // The backend canonicalizes the folder path, so track readiness explicitly
  // rather than comparing `dir` strings.
  const [ready, setReady] = useState(false);
  const loaded = useEditor((s) => ready && !!s.edit);
  const [error, setError] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);
  const [help, setHelp] = useState(false);

  // Load the project.
  useEffect(() => {
    let alive = true;
    setError(null);
    setReady(false);
    api
      .openProject(dir)
      .then((p) => {
        if (!alive) return;
        useEditor.getState().load(p);
        setReady(true);
      })
      .catch((e) => alive && setError(errorText(e)));
    return () => {
      alive = false;
      player.pause();
    };
  }, [dir]);

  // Autosave (debounced) and preview recomputation when inputs to the camera/cursor change.
  const revision = useEditor((s) => s.revision);
  const previewKey = useEditor((s) =>
    s.edit ? JSON.stringify([s.edit.zoom, s.edit.cursor, s.edit.style.aspect, s.edit.style.padding]) : "",
  );
  const saveTimer = useRef<number | undefined>(undefined);
  useEffect(() => {
    if (!loaded || revision === 0) return;
    window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(async () => {
      const st = useEditor.getState();
      if (!st.edit) return;
      st.setSaveState("saving");
      try {
        await api.saveProject(st.dir, st.edit, st.exportSettings ?? undefined);
        if (useEditor.getState().revision === revision) useEditor.getState().setSaveState("saved");
      } catch (e) {
        useEditor.getState().setSaveState("error");
        toast.error("Could not save the project", errorText(e));
      }
    }, 700);
  }, [revision, loaded]);

  const previewSeq = useRef(0);
  useEffect(() => {
    if (!loaded || !previewKey) return;
    const seq = ++previewSeq.current;
    const id = window.setTimeout(async () => {
      const st = useEditor.getState();
      try {
        const p = await api.computePreview(st.dir, st.edit!);
        if (seq === previewSeq.current) useEditor.getState().setPreview(p);
      } catch (e) {
        toast.error("Preview update failed", errorText(e));
      }
    }, 90);
    return () => window.clearTimeout(id);
  }, [previewKey, loaded]);

  // Keyboard shortcuts.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      if (target.closest("input:not([type=range]), textarea, select")) return;
      const st = useEditor.getState();
      if (!st.edit || exporting) return;
      const k = e.key.toLowerCase();
      const fps = st.project?.source.fps ?? 60;
      if (e.ctrlKey && k === "z") {
        e.preventDefault();
        if (e.shiftKey) st.redo();
        else st.undo();
      } else if (e.ctrlKey && k === "y") {
        e.preventDefault();
        st.redo();
      } else if (e.ctrlKey && k === "e") {
        e.preventDefault();
        setExporting(true);
      } else if (e.key === " ") {
        e.preventDefault();
        player.toggle();
      } else if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
        e.preventDefault();
        const d = (e.shiftKey ? 1 : 1 / fps) * (e.key === "ArrowLeft" ? -1 : 1);
        player.step(d);
      } else if (e.key === "Home") {
        player.seek(st.edit.timeline.clips[0]?.start ?? 0);
      } else if (e.key === "End") {
        const last = st.edit.timeline.clips[st.edit.timeline.clips.length - 1];
        if (last) player.seek(last.end - 0.01);
      } else if (k === "s" && !e.ctrlKey) {
        const n = splitAt(st.edit.timeline, st.time);
        if (n) st.update((ed) => ({ ...ed, timeline: n }));
      } else if (k === "z" && !e.ctrlKey) {
        const tl = st.edit.timeline;
        const start = Math.max(0, outputToSource(tl, Math.max(0, sourceToOutputClamped(tl, st.time) - 0.6)));
        const end = Math.min(tl.sourceDuration, start + 2.5);
        if (!st.edit.zoom.segments.some((s) => s.start < end && start < s.end)) {
          const id = newSegmentId();
          st.update((ed) => ({
            ...ed,
            zoom: {
              ...ed.zoom,
              segments: [...ed.zoom.segments, { id, start, end, level: ed.zoom.settings.defaultLevel, focus: { mode: "auto" as const }, enabled: true, auto: false }].sort((a, b) => a.start - b.start),
            },
          }));
          st.select({ kind: "zoom", id });
        }
      } else if (e.key === "Delete" || e.key === "Backspace") {
        const sel = st.selection;
        if (sel?.kind === "zoom") {
          st.update((ed) => ({ ...ed, zoom: { ...ed.zoom, segments: ed.zoom.segments.filter((s) => s.id !== sel.id) } }));
          st.select(null);
        } else if (sel?.kind === "clip") {
          const n = removeClip(st.edit.timeline, sel.id);
          if (n) {
            st.update((ed) => ({ ...ed, timeline: n }));
            st.select(null);
          }
        }
      } else if (e.key === "Escape") {
        st.select(null);
      } else if (e.key === "?") {
        setHelp((h) => !h);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [exporting]);

  const back = async () => {
    player.pause();
    const st = useEditor.getState();
    if (st.edit && st.saveState !== "saved") {
      try {
        await api.saveProject(st.dir, st.edit, st.exportSettings ?? undefined);
      } catch (e) {
        toast.error("Could not save before closing", errorText(e));
        return;
      }
    }
    st.unload();
    onClose();
  };

  return (
    <>
      <TitleBar
        left={
          <div className="row">
            <button className="btn ghost sm" onClick={back} title="Back to projects">
              <ChevronLeft size={16} /> Projects
            </button>
          </div>
        }
        center={loaded ? <><NameField /><SaveIndicator /></> : null}
        right={
          <div className="row" style={{ marginRight: 8 }}>
            <button className="btn ghost icon" title="Keyboard shortcuts (?)" onClick={() => setHelp(true)}><Keyboard size={16} /></button>
            <button className="btn primary" disabled={!loaded} onClick={() => setExporting(true)}>
              <Download size={15} /> Export
            </button>
          </div>
        }
      />
      {error ? (
        <div className="empty" style={{ flex: 1 }}>
          <div className="empty-icon" style={{ background: "var(--record-soft)", color: "var(--danger)" }}><AlertCircle size={26} /></div>
          <h3>This project could not be opened</h3>
          <p style={{ margin: 0, maxWidth: 480 }}>{error}</p>
          <button className="btn" onClick={onClose}>Back to projects</button>
        </div>
      ) : !loaded ? (
        <div className="empty" style={{ flex: 1 }}><Loader2 className="spin" /> Opening project…</div>
      ) : (
        <div className="editor">
          <div className="editor-top">
            <Preview />
            <SidePanel />
            <Rail />
          </div>
          <Timeline />
        </div>
      )}
      {exporting && <ExportDialog onClose={() => setExporting(false)} />}
      {help && (
        <Modal onClose={() => setHelp(false)}>
          <h2>Keyboard shortcuts</h2>
          <div className="shortcut-list">
            {SHORTCUTS.map(([k, d]) => (
              <div key={k} className="field-row"><span className="muted">{d}</span><span className="kbd">{k}</span></div>
            ))}
          </div>
        </Modal>
      )}
    </>
  );
}

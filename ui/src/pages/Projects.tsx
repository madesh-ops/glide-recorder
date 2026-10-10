import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Film, Import, MoreHorizontal, Play, Trash2, FolderOpen, Video, LifeBuoy } from "lucide-react";
import { api, errorText, fileUrl, type ProjectSummary, type Unfinished } from "../lib/api";
import { formatBytes, formatDate, formatTime } from "../lib/format";
import { toast } from "../components/toast";

export function Projects({ onOpen, onNew }: { onOpen: (dir: string) => void; onNew: () => void }) {
  const [items, setItems] = useState<ProjectSummary[] | null>(null);
  const [importing, setImporting] = useState(false);
  const [menu, setMenu] = useState<{ p: ProjectSummary; x: number; y: number } | null>(null);
  const [unfinished, setUnfinished] = useState<Unfinished[]>([]);
  const [recovering, setRecovering] = useState<string | null>(null);

  const refresh = useCallback(() => {
    api.listUnfinished().then(setUnfinished).catch(() => setUnfinished([]));
    api.listProjects().then(setItems).catch((e) => {
      setItems([]);
      toast.error("Could not list projects", errorText(e));
    });
  }, []);

  useEffect(() => {
    refresh();
    const onFocus = () => refresh();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [refresh]);

  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [menu]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key.toLowerCase() === "n") {
        e.preventDefault();
        onNew();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onNew]);

  const importVideo = async () => {
    const path = await open({ multiple: false, filters: [{ name: "Video", extensions: ["mp4", "mov", "mkv", "webm", "avi"] }] });
    if (!path || Array.isArray(path)) return;
    setImporting(true);
    try {
      const dir = await api.importVideo(path);
      toast.success("Video imported");
      onOpen(dir);
    } catch (e) {
      toast.error("Import failed", errorText(e));
    } finally {
      setImporting(false);
    }
  };

  const remove = async (p: ProjectSummary) => {
    try {
      await api.deleteProject(p.dir);
      toast.success(`Moved “${p.name}” to the Recycle Bin`);
      refresh();
    } catch (e) {
      toast.error("Could not delete", errorText(e));
    }
  };

  const recover = async (u: Unfinished) => {
    setRecovering(u.dir);
    try {
      const dir = await api.recoverRecording(u.dir);
      toast.success("Recording recovered");
      onOpen(dir);
    } catch (e) {
      toast.error("Could not recover the recording", errorText(e));
      refresh();
    } finally {
      setRecovering(null);
    }
  };

  const discardUnfinished = async (u: Unfinished) => {
    try {
      await api.discardUnfinished(u.dir);
      refresh();
    } catch (e) {
      toast.error("Could not discard", errorText(e));
    }
  };

  return (
    <div className="page">
      <div className="page-header">
        <div>
          <h1>Projects</h1>
          <p className="muted">Recordings open in the editor with zooms, cursor and background applied automatically.</p>
        </div>
        <div className="spacer" />
        <button className="btn" onClick={importVideo} disabled={importing}>
          <Import size={15} /> {importing ? "Importing…" : "Import video"}
        </button>
        <button className="btn primary" onClick={onNew}>
          <Video size={15} /> New recording <span className="kbd" style={{ color: "rgba(255,255,255,.7)", borderColor: "rgba(255,255,255,.3)" }}>Ctrl N</span>
        </button>
      </div>

      {unfinished.map((u) => (
        <div key={u.dir} className="recover-banner" role="alert">
          <LifeBuoy size={18} />
          <div className="col" style={{ gap: 1, flex: 1, minWidth: 0 }}>
            <strong>“{u.name}” was interrupted before it finished saving</strong>
            <span className="muted" style={{ fontSize: 12 }}>
              {formatDate(u.createdAt)} · {formatBytes(u.bytes)} on disk. Glide can rebuild the video, audio and cursor data recorded up to that point.
            </span>
          </div>
          <button className="btn ghost" onClick={() => discardUnfinished(u)} disabled={!!recovering}>Discard</button>
          <button className="btn primary" onClick={() => recover(u)} disabled={!!recovering}>
            {recovering === u.dir ? "Recovering…" : "Recover"}
          </button>
        </div>
      ))}

      {items === null ? (
        <div className="grid">
          {Array.from({ length: 6 }).map((_, i) => (
            <div key={i} className="project-card skeleton">
              <div className="thumb" />
              <div className="meta"><div className="sk-line" /><div className="sk-line short" /></div>
            </div>
          ))}
        </div>
      ) : items.length === 0 ? (
        <div className="empty" style={{ flex: 1 }}>
          <div className="empty-icon"><Film size={26} /></div>
          <h3>No recordings yet</h3>
          <p style={{ maxWidth: 360, margin: 0 }}>
            Record your screen or a single window. Glide tracks your cursor and clicks so it can zoom in on what matters.
          </p>
          <div className="row" style={{ marginTop: 8 }}>
            <button className="btn record lg" onClick={onNew}><span className="rec-dot" /> Start your first recording</button>
            <button className="btn lg" onClick={importVideo}><Import size={15} /> Import a video</button>
          </div>
        </div>
      ) : (
        <div className="grid">
          {items.map((p) => (
            <div
              key={p.dir}
              className="project-card"
              tabIndex={0}
              onDoubleClick={() => onOpen(p.dir)}
              onKeyDown={(e) => e.key === "Enter" && onOpen(p.dir)}
              onContextMenu={(e) => setMenu({ p, x: e.clientX, y: e.clientY })}
            >
              <div className="thumb" onClick={() => onOpen(p.dir)}>
                {p.thumbnail && <img src={fileUrl(p.thumbnail)} alt="" draggable={false} onError={(e) => (e.currentTarget.style.display = "none")} />}
                <Film size={28} className="faint thumb-fallback" />
                <span className="duration">{formatTime(p.duration)}</span>
                <span className="play"><Play size={18} fill="currentColor" /></span>
              </div>
              <div className="meta">
                <div className="col" style={{ gap: 2, minWidth: 0 }}>
                  <span className="name" title={p.name}>{p.name}</span>
                  <span className="faint">{formatDate(p.createdAt)} · {p.width}×{p.height}</span>
                </div>
                <button
                  className="btn ghost icon sm"
                  aria-label="More"
                  onMouseDown={(e) => e.stopPropagation()}
                  onClick={(e) => setMenu({ p, x: e.clientX, y: e.clientY })}
                >
                  <MoreHorizontal size={16} />
                </button>
              </div>
            </div>
          ))}
        </div>
      )}

      {menu && (
        <div className="context-menu" style={{ left: menu.x, top: menu.y }} onMouseDown={(e) => e.stopPropagation()}>
          <button onClick={() => { onOpen(menu.p.dir); setMenu(null); }}><Play size={14} /> Open in editor</button>
          <button onClick={() => { api.revealPath(menu.p.dir); setMenu(null); }}><FolderOpen size={14} /> Show in Explorer</button>
          <div className="sep" />
          <button onClick={() => { remove(menu.p); setMenu(null); }}><Trash2 size={14} /> Move to Recycle Bin</button>
        </div>
      )}
    </div>
  );
}

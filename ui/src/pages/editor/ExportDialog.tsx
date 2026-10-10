import { useEffect, useMemo, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { CheckCircle2, FolderOpen, Play, XCircle, Loader2 } from "lucide-react";
import { api, errorText, on, type EncoderInfo, type ExportProgress, type ExportResult, type ExportSettings } from "../../lib/api";
import { formatBytes, formatTime } from "../../lib/format";
import { outputDuration } from "../../lib/timeline";
import { useEditor } from "../../store/editor";
import { Modal } from "../../components/Modal";
import { Segmented } from "../../components/controls";

type Phase =
  | { kind: "settings" }
  | { kind: "running"; job: string; progress: ExportProgress | null }
  | { kind: "done"; result: ExportResult }
  | { kind: "error"; message: string };

export function ExportDialog({ onClose }: { onClose: () => void }) {
  const project = useEditor((s) => s.project!);
  const dir = useEditor((s) => s.dir);
  const preview = useEditor((s) => s.preview);
  const timeline = useEditor((s) => s.edit!.timeline);
  const stored = useEditor((s) => s.exportSettings!);
  const setExportSettings = useEditor((s) => s.setExportSettings);
  const [settings, setSettings] = useState<ExportSettings>(stored);
  const [encoders, setEncoders] = useState<EncoderInfo[]>([]);
  const [output, setOutput] = useState<string>("");
  const [phase, setPhase] = useState<Phase>({ kind: "settings" });

  useEffect(() => {
    api.listEncoders().then(setEncoders).catch(() => {});
    api.appInfo().then((i) => {
      const safe = project.name.replace(/[<>:"/\\|?*]+/g, " ").trim() || "Recording";
      setOutput(`${i.outputDir}\\${safe}.mp4`);
    });
  }, [project.name]);

  useEffect(() => {
    const p1 = on<{ job: string; progress: ExportProgress }>("export-progress", (e) =>
      setPhase((ph) => (ph.kind === "running" && ph.job === e.job ? { ...ph, progress: e.progress } : ph)),
    );
    const p2 = on<{ job: string; result?: ExportResult; error?: string; cancelled: boolean }>("export-finished", (e) =>
      setPhase((ph) => {
        if (ph.kind !== "running" || ph.job !== e.job) return ph;
        if (e.cancelled) return { kind: "settings" };
        if (e.result) return { kind: "done", result: e.result };
        return { kind: "error", message: e.error ?? "Export failed" };
      }),
    );
    return () => {
      p1.then((f) => f());
      p2.then((f) => f());
    };
  }, []);

  const aspect = preview?.layout.aspect ?? 16 / 9;
  const outSize = useMemo(() => {
    const h = Math.floor(settings.height / 2) * 2;
    const w = Math.max(2, Math.round((h * aspect) / 2) * 2);
    return `${w} × ${h}`;
  }, [settings.height, aspect]);
  const duration = outputDuration(timeline);

  const browse = async () => {
    const p = await save({ defaultPath: output, filters: [{ name: "MP4 video", extensions: ["mp4"] }] });
    if (p) setOutput(p.toLowerCase().endsWith(".mp4") ? p : `${p}.mp4`);
  };

  const start = async () => {
    setExportSettings(settings);
    try {
      // Persist the latest edits first so the export renders exactly what is shown.
      const st = useEditor.getState();
      await api.saveProject(dir, st.edit!, settings);
      st.setSaveState("saved");
      const job = await api.startExport(dir, output, settings);
      setPhase({ kind: "running", job, progress: null });
    } catch (e) {
      setPhase({ kind: "error", message: errorText(e) });
    }
  };

  const running = phase.kind === "running";
  return (
    <Modal onClose={running ? undefined : onClose} width={480}>
      {phase.kind === "settings" && (
        <>
          <h2>Export video</h2>
          <p className="muted" style={{ margin: "0 0 18px" }}>
            MP4 (H.264) · {outSize} · {formatTime(duration)} · zooms, cursor and background included
          </p>
          <div className="col" style={{ gap: 14 }}>
            <div className="field">
              <span className="label">Resolution</span>
              <Segmented<number>
                value={settings.height}
                onChange={(height) => setSettings({ ...settings, height })}
                options={[
                  { value: 720, label: "720p" },
                  { value: 1080, label: "1080p" },
                  { value: 1440, label: "1440p" },
                  { value: 2160, label: "4K" },
                ]}
              />
            </div>
            <div className="row" style={{ gap: 14 }}>
              <div className="field" style={{ flex: 1 }}>
                <span className="label">Frame rate</span>
                <Segmented<number> value={settings.fps} onChange={(fps) => setSettings({ ...settings, fps })} options={[{ value: 30, label: "30 fps" }, { value: 60, label: "60 fps" }]} />
              </div>
              <div className="field" style={{ flex: 1 }}>
                <span className="label">Quality</span>
                <Segmented
                  value={settings.quality}
                  onChange={(quality) => setSettings({ ...settings, quality })}
                  options={[{ value: "low", label: "Small" }, { value: "high", label: "High" }, { value: "best", label: "Best" }]}
                />
              </div>
            </div>
            <div className="field">
              <span className="label">Encoder</span>
              <select className="select" value={settings.encoder} onChange={(e) => setSettings({ ...settings, encoder: e.target.value })}>
                <option value="auto">Automatic (hardware when available)</option>
                {encoders.map((e) => (
                  <option key={e.name} value={e.name}>{e.label}</option>
                ))}
              </select>
            </div>
            <div className="field">
              <span className="label">Save to</span>
              <div className="row">
                <input className="input" style={{ flex: 1 }} value={output} onChange={(e) => setOutput(e.target.value)} spellCheck={false} />
                <button className="btn" onClick={browse}>Browse…</button>
              </div>
            </div>
          </div>
          <div className="row" style={{ marginTop: 22, justifyContent: "flex-end" }}>
            <button className="btn ghost" onClick={onClose}>Cancel</button>
            <button className="btn primary" disabled={!output} onClick={start}>Export</button>
          </div>
        </>
      )}

      {phase.kind === "running" && (
        <>
          <h2 className="row"><Loader2 size={18} className="spin" /> Exporting…</h2>
          <p className="muted" style={{ margin: "0 0 16px", wordBreak: "break-all" }}>{output}</p>
          <div className="progress">
            <div style={{ width: `${phase.progress ? (phase.progress.frame / phase.progress.totalFrames) * 100 : 0}%` }} />
          </div>
          <div className="row faint" style={{ marginTop: 8, fontSize: 12 }}>
            <span>{phase.progress ? `Frame ${phase.progress.frame} of ${phase.progress.totalFrames}` : "Preparing…"}</span>
            <span className="spacer" />
            {phase.progress && <span>{phase.progress.fps.toFixed(0)} fps · {formatTime(phase.progress.etaSecs)} left</span>}
          </div>
          <div className="row" style={{ marginTop: 20, justifyContent: "flex-end" }}>
            <button className="btn" onClick={() => api.cancelExport(phase.job)}>Cancel export</button>
          </div>
        </>
      )}

      {phase.kind === "done" && (
        <>
          <h2 className="row"><CheckCircle2 size={20} color="var(--success)" /> Export complete</h2>
          <p className="muted" style={{ margin: "0 0 6px", wordBreak: "break-all" }}>{phase.result.output}</p>
          <p className="faint" style={{ margin: 0, fontSize: 12 }}>
            {phase.result.width}×{phase.result.height} · {formatTime(phase.result.duration)} · {formatBytes(phase.result.sizeBytes)} · {phase.result.encoder} ·
            rendered in {phase.result.elapsedSecs.toFixed(1)} s
          </p>
          <div className="row" style={{ marginTop: 22, justifyContent: "flex-end" }}>
            <button className="btn" onClick={() => api.revealPath(phase.result.output)}><FolderOpen size={14} /> Show in folder</button>
            <button className="btn" onClick={() => api.openPath(phase.result.output)}><Play size={14} /> Play</button>
            <button className="btn primary" onClick={onClose}>Done</button>
          </div>
        </>
      )}

      {phase.kind === "error" && (
        <>
          <h2 className="row"><XCircle size={20} color="var(--danger)" /> Export failed</h2>
          <pre className="error-box">{phase.message}</pre>
          <div className="row" style={{ marginTop: 18, justifyContent: "flex-end" }}>
            <button className="btn ghost" onClick={onClose}>Close</button>
            <button className="btn primary" onClick={() => setPhase({ kind: "settings" })}>Try again</button>
          </div>
        </>
      )}
    </Modal>
  );
}

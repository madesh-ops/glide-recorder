import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { CheckCircle2, FolderOpen, XCircle } from "lucide-react";
import { api, errorText, type AppInfo, type AppSettings, type EncoderInfo, type Easing } from "../lib/api";
import { Segmented, SliderField, ToggleRow } from "../components/controls";
import { toast } from "../components/toast";

export function Settings({ info }: { info: AppInfo | null }) {
  const [s, setS] = useState<AppSettings | null>(null);
  const [encoders, setEncoders] = useState<EncoderInfo[] | null>(null);

  useEffect(() => {
    api.getSettings().then(setS);
    api.listEncoders().then(setEncoders).catch(() => setEncoders([]));
  }, []);

  const save = async (next: AppSettings) => {
    setS(next);
    try {
      setS(await api.saveSettings(next));
    } catch (e) {
      toast.error("Could not save settings", errorText(e));
    }
  };

  if (!s) return <div className="page" />;
  const z = s.defaultZoom;
  const setZoom = (patch: Partial<typeof z>) => save({ ...s, defaultZoom: { ...z, ...patch } });

  const chooseFolder = async () => {
    const dir = await open({ directory: true, multiple: false, defaultPath: s.outputDir ?? info?.outputDir });
    if (dir && !Array.isArray(dir)) save({ ...s, outputDir: dir });
  };

  return (
    <div className="page settings-page">
      <div className="page-header">
        <div>
          <h1>Settings</h1>
          <p className="muted">Defaults for new recordings. Each project keeps its own editing choices.</p>
        </div>
      </div>

      <section className="settings-section">
        <h2>Recording</h2>
        <div className="settings-card">
          <div className="field-row">
            <div className="col" style={{ gap: 2 }}>
              <span>Recordings folder</span>
              <span className="faint mono" style={{ fontSize: 11.5 }}>{s.outputDir ?? info?.outputDir}</span>
            </div>
            <button className="btn" onClick={chooseFolder}><FolderOpen size={14} /> Change…</button>
          </div>
          <div className="divider" />
          <div className="field-row">
            <span>Frame rate</span>
            <div style={{ width: 200 }}>
              <Segmented<number> value={s.fps} onChange={(fps) => save({ ...s, fps })} options={[{ value: 24, label: "24" }, { value: 30, label: "30" }, { value: 60, label: "60" }]} />
            </div>
          </div>
          <div className="field-row">
            <span>Countdown</span>
            <div style={{ width: 200 }}>
              <Segmented<number> value={s.countdownSecs} onChange={(countdownSecs) => save({ ...s, countdownSecs })} options={[{ value: 0, label: "Off" }, { value: 3, label: "3 s" }, { value: 5, label: "5 s" }]} />
            </div>
          </div>
          <ToggleRow label="Hide Glide while recording" hint="The floating controls are never captured." value={s.hideWindowWhileRecording} onChange={(v) => save({ ...s, hideWindowWhileRecording: v })} />
          <div className="divider" />
          <div className="field-row">
            <div className="col" style={{ gap: 2 }}>
              <span>Video encoder</span>
              <span className="faint" style={{ fontSize: 11.5 }}>Only encoders that passed a test encode on this PC are listed.</span>
            </div>
            <select className="select" style={{ width: 240 }} value={s.encoder} onChange={(e) => save({ ...s, encoder: e.target.value })}>
              <option value="auto">Automatic (best available)</option>
              {(encoders ?? []).map((e) => (
                <option key={e.name} value={e.name}>{e.label}</option>
              ))}
            </select>
          </div>
        </div>
      </section>

      <section className="settings-section">
        <h2>Automatic zoom defaults</h2>
        <div className="settings-card">
          <ToggleRow label="Zoom in on clicks automatically" value={z.autoZoom} onChange={(autoZoom) => setZoom({ autoZoom })} />
          <SliderField label="Zoom level" value={z.defaultLevel} display={`${z.defaultLevel.toFixed(1)}×`} min={1.25} max={4} step={0.05} onChange={(defaultLevel) => setZoom({ defaultLevel })} />
          <SliderField label="Animation duration" value={z.transition} display={`${z.transition.toFixed(2)} s`} min={0.2} max={2} step={0.05} onChange={(transition) => setZoom({ transition })} />
          <SliderField label="Camera follow smoothness" value={z.followSmoothing} display={`${z.followSmoothing.toFixed(2)} s`} min={0.05} max={1.5} step={0.05} onChange={(followSmoothing) => setZoom({ followSmoothing })} />
          <div className="field-row">
            <span className="label">Easing</span>
            <select className="select" value={z.easing} onChange={(e) => setZoom({ easing: e.target.value as Easing })}>
              <option value="ease-in-out-cubic">Smooth (cubic)</option>
              <option value="ease-in-out-quint">Snappy (quint)</option>
              <option value="smoother">Gentle (smootherstep)</option>
              <option value="ease-out-cubic">Ease out</option>
              <option value="linear">Linear</option>
            </select>
          </div>
        </div>
      </section>

      <section className="settings-section">
        <h2>System</h2>
        <div className="settings-card">
          <div className="field-row">
            <span>FFmpeg</span>
            {info?.ffmpeg ? (
              <span className="row faint mono" style={{ fontSize: 11.5 }}><CheckCircle2 size={14} color="var(--success)" /> {info.ffmpeg}</span>
            ) : (
              <span className="row" style={{ color: "var(--danger)" }}><XCircle size={14} /> {info?.ffmpegError ?? "Checking…"}</span>
            )}
          </div>
          <div className="field-row">
            <span>Screen capture</span>
            <span className="row faint">
              {info?.captureSupported ? <><CheckCircle2 size={14} color="var(--success)" /> Windows Graphics Capture</> : <><XCircle size={14} color="var(--danger)" /> Unsupported</>}
            </span>
          </div>
          <div className="field-row">
            <span>Working encoders</span>
            <span className="faint">{encoders === null ? "Testing…" : encoders.map((e) => e.name).join(", ") || "None"}</span>
          </div>
          <div className="field-row">
            <span>Privacy</span>
            <span className="faint">Everything stays on this PC. Glide has no telemetry and never uploads recordings.</span>
          </div>
        </div>
      </section>
    </div>
  );
}

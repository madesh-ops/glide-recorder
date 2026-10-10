import { useEffect, useMemo, useState } from "react";
import { AppWindow, Check, Mic, MicOff, Monitor, RefreshCw, Scan, Volume2, VolumeX } from "lucide-react";
import { AreaPicker, centeredArea, type Area } from "./AreaPicker";
import { api, errorText, type AppInfo, type AppSettings, type AudioDevice, type DisplayInfo, type SourceSelection, type WindowInfo } from "../lib/api";
import { Segmented } from "../components/controls";
import { toast } from "../components/toast";

type Kind = "display" | "window" | "area";

function keyOf(s: SourceSelection) {
  return s.kind === "area" ? `area:${s.id}:${s.x},${s.y},${s.w},${s.h}` : `${s.kind}:${s.id}`;
}

/** Lazily grabs a still frame for a source card. */
function useThumbnail(source: SourceSelection, enabled: boolean) {
  const [src, setSrc] = useState<string | null>(null);
  const k = keyOf(source);
  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    api.sourceThumbnail(source, source.kind === "display" ? 560 : 400).then((u) => alive && setSrc(u)).catch(() => {});
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [k, enabled]);
  return src;
}

function SourceCard({
  source,
  title,
  subtitle,
  selected,
  onSelect,
  onStart,
  aspect,
}: {
  source: SourceSelection;
  title: string;
  subtitle: string;
  selected: boolean;
  onSelect: () => void;
  onStart: () => void;
  aspect: number;
}) {
  const thumb = useThumbnail(source, true);
  return (
    <button className={`source-card ${selected ? "selected" : ""}`} onClick={onSelect} onDoubleClick={onStart}>
      <div className="source-thumb" style={{ aspectRatio: String(Math.min(Math.max(aspect, 0.8), 2.4)) }}>
        {thumb ? <img src={thumb} alt="" draggable={false} /> : <div className="thumb-loading" />}
        {selected && (
          <span className="check">
            <Check size={14} strokeWidth={3} />
          </span>
        )}
      </div>
      <div className="source-meta">
        <span className="name" title={title}>{title}</span>
        <span className="faint">{subtitle}</span>
      </div>
    </button>
  );
}

export function NewRecording({ info, onCancel }: { info: AppInfo | null; onCancel: () => void }) {
  const [kind, setKind] = useState<Kind>(() =>
    import.meta.env.DEV && new URLSearchParams(location.search).get("kind") === "area" ? "area" : "display",
  );
  const [displays, setDisplays] = useState<DisplayInfo[]>([]);
  const [windows, setWindows] = useState<WindowInfo[]>([]);
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [selected, setSelected] = useState<SourceSelection | null>(null);
  const [mic, setMic] = useState<string>("none");
  const [systemAudio, setSystemAudio] = useState(true);
  const [fps, setFps] = useState(60);
  const [starting, setStarting] = useState(false);
  const [loading, setLoading] = useState(true);
  const [areaDisplay, setAreaDisplay] = useState<string | null>(null);
  const [area, setArea] = useState<Area | null>(null);

  const refresh = () => {
    setLoading(true);
    api
      .listSources()
      .then((s) => {
        setDisplays(s.displays);
        setWindows(s.windows);
        setSelected((cur) => cur ?? (s.displays[0] ? { kind: "display", id: s.displays[0].id } : null));
      })
      .catch((e) => toast.error("Could not list sources", errorText(e)))
      .finally(() => setLoading(false));
  };

  useEffect(() => {
    refresh();
    api.listAudioDevices().then(setDevices).catch(() => {});
    api.getSettings().then((s) => {
      setSettings(s);
      setFps(s.fps);
      setSystemAudio(s.recordSystemAudio);
      setMic(s.recordMic ? s.micDevice ?? "default" : "none");
    });
  }, []);

  const disabledReason = !info
    ? "Loading…"
    : info.ffmpegError
      ? "FFmpeg is required to record"
      : !info.captureSupported
        ? "Screen capture is not supported on this version of Windows"
        : !selected
          ? "Choose what to record"
          : null;

  const start = async () => {
    if (!selected || disabledReason || starting) return;
    setStarting(true);
    try {
      // Remember audio choices for next time.
      if (settings) {
        const next = { ...settings, fps, recordSystemAudio: systemAudio, recordMic: mic !== "none", micDevice: mic !== "none" && mic !== "default" ? mic : null };
        api.saveSettings(next).then(setSettings).catch(() => {});
      }
      await api.prepareRecording({ source: selected, mic, systemAudio, fps });
    } catch (e) {
      toast.error("Could not start recording", errorText(e));
    } finally {
      setStarting(false);
    }
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Enter" && !(e.target as HTMLElement).closest("select")) start();
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const list = useMemo(() => (kind === "window" ? windows : displays), [kind, displays, windows]);
  const areaDisp = displays.find((d) => d.id === areaDisplay) ?? displays[0];

  const switchKind = (k: Kind) => {
    setKind(k);
    if (k === "area" && areaDisp) {
      const a = area ?? centeredArea(areaDisp, 1280, 720);
      setAreaDisplay(areaDisp.id);
      setArea(a);
      setSelected({ kind: "area", id: areaDisp.id, ...a });
    } else if (k === "display" && displays[0]) {
      setSelected({ kind: "display", id: displays[0].id });
    } else if (k === "window") {
      setSelected(windows[0] ? { kind: "window", id: windows[0].id } : null);
    }
  };
  // Initialize the area once displays are known (also covers deep links in dev).
  useEffect(() => {
    if (kind === "area" && !area && displays.length) switchKind("area");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kind, displays.length]);

  const updateArea = (a: Area) => {
    setArea(a);
    if (areaDisp) setSelected({ kind: "area", id: areaDisp.id, ...a });
  };

  return (
    <div className="page record-page">
      <div className="page-header">
        <div>
          <h1>New recording</h1>
          <p className="muted">
            Pick a display or a window. Your cursor is recorded separately, so you can restyle it later. Press{" "}
            <span className="kbd">Ctrl+Alt+R</span> anywhere to stop, <span className="kbd">Ctrl+Alt+P</span> to pause.
          </p>
        </div>
        <div className="spacer" />
        <button className="btn ghost" onClick={refresh} title="Refresh sources">
          <RefreshCw size={15} className={loading ? "spin" : ""} /> Refresh
        </button>
      </div>

      <div className="row" style={{ marginBottom: 16 }}>
        <div style={{ width: 360 }}>
          <Segmented<Kind>
            value={kind}
            onChange={switchKind}
            options={[
              { value: "display", label: <><Monitor size={14} /> Display</> },
              { value: "window", label: <><AppWindow size={14} /> Window</> },
              { value: "area", label: <><Scan size={14} /> Area</> },
            ]}
          />
        </div>
        {kind === "area" && displays.length > 1 ? (
          <select className="select" value={areaDisp?.id} onChange={(e) => {
            const d = displays.find((x) => x.id === e.target.value);
            if (!d) return;
            setAreaDisplay(d.id);
            const a = centeredArea(d, 1280, 720);
            setArea(a);
            setSelected({ kind: "area", id: d.id, ...a });
          }}>
            {displays.map((d) => <option key={d.id} value={d.id}>{d.name}</option>)}
          </select>
        ) : (
          <span className="faint">{kind === "area" ? "Record part of a display" : `${list.length} available`}</span>
        )}
      </div>

      {kind === "area" && areaDisp && area && (
        <div style={{ flex: 1, paddingBottom: 24 }}>
          <AreaPicker display={areaDisp} area={area} onChange={updateArea} />
        </div>
      )}
      <div className={`source-grid ${kind}`} style={kind === "area" ? { display: "none" } : undefined}>
        {kind === "display" &&
          displays.map((d) => {
            const s: SourceSelection = { kind: "display", id: d.id };
            return (
              <SourceCard
                key={d.id}
                source={s}
                title={d.name}
                subtitle={`${d.width} × ${d.height}${d.scale !== 1 ? ` · ${Math.round(d.scale * 100)}%` : ""}`}
                selected={!!selected && keyOf(selected) === keyOf(s)}
                onSelect={() => setSelected(s)}
                onStart={start}
                aspect={d.width / d.height}
              />
            );
          })}
        {kind === "window" &&
          windows.map((w) => {
            const s: SourceSelection = { kind: "window", id: w.id };
            return (
              <SourceCard
                key={w.id}
                source={s}
                title={w.title}
                subtitle={`${w.app || "App"} · ${w.width} × ${w.height}`}
                selected={!!selected && keyOf(selected) === keyOf(s)}
                onSelect={() => setSelected(s)}
                onStart={start}
                aspect={w.width / w.height}
              />
            );
          })}
        {!loading && list.length === 0 && (
          <div className="empty" style={{ gridColumn: "1 / -1" }}>
            <h3>No {kind === "display" ? "displays" : "windows"} found</h3>
            <p style={{ margin: 0 }}>Open the app you want to record, then refresh.</p>
          </div>
        )}
      </div>

      <div className="record-bar">
        <div className="record-option">
          {mic === "none" ? <MicOff size={16} className="faint" /> : <Mic size={16} />}
          <select className="select" value={mic} onChange={(e) => setMic(e.target.value)} aria-label="Microphone">
            <option value="none">No microphone</option>
            <option value="default">Default microphone</option>
            {devices.map((d) => (
              <option key={d.id} value={d.id}>{d.name}{d.isDefault ? " (default)" : ""}</option>
            ))}
          </select>
        </div>
        <button className={`btn ${systemAudio ? "" : "ghost"}`} onClick={() => setSystemAudio(!systemAudio)} aria-pressed={systemAudio}>
          {systemAudio ? <Volume2 size={15} /> : <VolumeX size={15} />} System audio {systemAudio ? "on" : "off"}
        </button>
        <div style={{ width: 150 }}>
          <Segmented<number> value={fps} onChange={setFps} options={[{ value: 30, label: "30 fps" }, { value: 60, label: "60 fps" }]} />
        </div>
        <div className="spacer" />
        {disabledReason && <span className="faint">{disabledReason}</span>}
        <button className="btn ghost lg" onClick={onCancel}>Cancel</button>
        <button className="btn record lg" disabled={!!disabledReason || starting} onClick={start}>
          <span className="rec-dot" /> {starting ? "Starting…" : "Start recording"}
        </button>
      </div>
    </div>
  );
}

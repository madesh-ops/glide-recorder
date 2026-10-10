import { open } from "@tauri-apps/plugin-dialog";
import {
  Image as ImageIcon, MousePointer2, ZoomIn, Volume2, Film, RefreshCw, Trash2, Scissors, Crosshair, Upload, VolumeX,
} from "lucide-react";
import { api, errorText, fileUrl, joinPath, type Aspect, type Easing, type EditState, type ZoomSegment } from "../../lib/api";
import { BACKGROUND_PRESETS, SOLID_PRESETS, backgroundCss } from "../../lib/preview";
import { removeClip, splitAt } from "../../lib/timeline";
import { useEditor, type Panel } from "../../store/editor";
import { Segmented, SliderField, Toggle, ToggleRow } from "../../components/controls";
import { toast } from "../../components/toast";

const RAIL: { id: Panel; icon: typeof ImageIcon; label: string }[] = [
  { id: "background", icon: ImageIcon, label: "Background" },
  { id: "cursor", icon: MousePointer2, label: "Cursor" },
  { id: "zoom", icon: ZoomIn, label: "Zoom" },
  { id: "audio", icon: Volume2, label: "Audio" },
  { id: "clip", icon: Film, label: "Clip" },
];

/** Bind a slider so a drag becomes one undo step. */
function useSliderBinding() {
  const { beginGesture, endGesture, update } = useEditor.getState();
  return (apply: (e: EditState, v: number) => EditState) => ({
    onStart: beginGesture,
    onEnd: endGesture,
    // While a gesture is open the store folds every change into one undo step.
    onChange: (v: number) => update((e) => apply(e, v)),
  });
}

export function Rail() {
  const panel = useEditor((s) => s.panel);
  const setPanel = useEditor((s) => s.setPanel);
  return (
    <nav className="rail" aria-label="Editor panels">
      {RAIL.map(({ id, icon: Icon, label }) => (
        <button key={id} className={panel === id ? "on" : ""} title={label} aria-label={label} onClick={() => setPanel(id)}>
          <Icon size={18} strokeWidth={1.8} />
        </button>
      ))}
    </nav>
  );
}

export function SidePanel() {
  const panel = useEditor((s) => s.panel);
  return (
    <aside className="side-panel">
      {panel === "background" && <BackgroundPanel />}
      {panel === "cursor" && <CursorPanel />}
      {panel === "zoom" && <ZoomPanel />}
      {panel === "audio" && <AudioPanel />}
      {panel === "clip" && <ClipPanel />}
    </aside>
  );
}

function PanelHeader({ title, children }: { title: string; children?: React.ReactNode }) {
  return (
    <div className="panel-header">
      <h3>{title}</h3>
      {children}
    </div>
  );
}

function BackgroundPanel() {
  const style = useEditor((s) => s.edit!.style);
  const dir = useEditor((s) => s.dir);
  const update = useEditor((s) => s.update);
  const bind = useSliderBinding();
  const bgKey = JSON.stringify(style.background);

  const chooseImage = async () => {
    const path = await open({ multiple: false, filters: [{ name: "Image", extensions: ["png", "jpg", "jpeg", "webp", "bmp"] }] });
    if (!path || Array.isArray(path)) return;
    try {
      const rel = await api.setBackgroundImage(dir, path);
      update((e) => ({ ...e, style: { ...e.style, background: { kind: "image", path: rel } } }));
    } catch (e) {
      toast.error("Could not use that image", errorText(e));
    }
  };

  return (
    <div className="panel-body">
      <PanelHeader title="Background" />
      <div className="section-title">Wallpaper</div>
      <div className="swatches">
        {BACKGROUND_PRESETS.map((p) => (
          <button
            key={p.name}
            title={p.name}
            className={`swatch ${JSON.stringify(p.bg) === bgKey ? "on" : ""}`}
            style={{ background: backgroundCss(p.bg) }}
            onClick={() => update((e) => ({ ...e, style: { ...e.style, background: p.bg } }))}
          />
        ))}
        <button className={`swatch upload ${style.background.kind === "image" ? "on" : ""}`} title="Custom image" onClick={chooseImage}
          style={style.background.kind === "image" ? { background: backgroundCss(style.background, (p) => fileUrl(joinPath(dir, p))) } : undefined}>
          {style.background.kind !== "image" && <Upload size={14} />}
        </button>
      </div>
      <div className="section-title">Color</div>
      <div className="swatches small">
        {SOLID_PRESETS.map((c) => (
          <button
            key={c}
            title={c}
            className={`swatch ${style.background.kind === "solid" && style.background.color.toLowerCase() === c.toLowerCase() ? "on" : ""}`}
            style={{ background: c }}
            onClick={() => update((e) => ({ ...e, style: { ...e.style, background: { kind: "solid", color: c } } }))}
          />
        ))}
        <label className="swatch picker" title="Custom color">
          <input
            type="color"
            value={style.background.kind === "solid" ? style.background.color : "#6b52f5"}
            onFocus={() => useEditor.getState().beginGesture()}
            onBlur={() => useEditor.getState().endGesture()}
            onChange={(ev) => update((e) => ({ ...e, style: { ...e.style, background: { kind: "solid", color: ev.target.value } } }))}
          />
        </label>
      </div>
      <div className="divider" />
      <div className="field">
        <span className="label">Aspect ratio</span>
        <select className="select" value={style.aspect} onChange={(ev) => update((e) => ({ ...e, style: { ...e.style, aspect: ev.target.value as Aspect } }))}>
          <option value="auto">Auto (match recording)</option>
          <option value="wide16x9">Wide 16:9</option>
          <option value="standard4x3">Standard 4:3</option>
          <option value="square">Square 1:1</option>
          <option value="vertical9x16">Vertical 9:16</option>
        </select>
      </div>
      <SliderField label="Padding" value={style.padding} display={`${Math.round(style.padding * 100)}%`} min={0} max={0.3} step={0.005}
        {...bind((e, v) => ({ ...e, style: { ...e.style, padding: v } }))} />
      <SliderField label="Roundness" value={style.cornerRadius} display={`${Math.round(style.cornerRadius)} px`} min={0} max={60} step={1}
        {...bind((e, v) => ({ ...e, style: { ...e.style, cornerRadius: v } }))} />
      <SliderField label="Shadow" value={style.shadowOpacity} display={`${Math.round(style.shadowOpacity * 100)}%`} min={0} max={1} step={0.01}
        {...bind((e, v) => ({ ...e, style: { ...e.style, shadowOpacity: v } }))} />
      <SliderField label="Shadow softness" value={style.shadowBlur} display={`${Math.round(style.shadowBlur)} px`} min={0} max={160} step={1}
        {...bind((e, v) => ({ ...e, style: { ...e.style, shadowBlur: v } }))} />
    </div>
  );
}

function CursorPanel() {
  const c = useEditor((s) => s.edit!.cursor);
  const hasCursor = useEditor((s) => s.hasCursor);
  const update = useEditor((s) => s.update);
  const bind = useSliderBinding();
  const set = (patch: Partial<typeof c>) => update((e) => ({ ...e, cursor: { ...e.cursor, ...patch } }));
  if (!hasCursor) {
    return (
      <div className="panel-body">
        <PanelHeader title="Cursor" />
        <p className="muted">This video was imported, so there is no cursor data to restyle. Cursor effects are available for recordings made with Glide.</p>
      </div>
    );
  }
  return (
    <div className="panel-body">
      <PanelHeader title="Cursor" />
      <ToggleRow label="Show cursor" value={c.visible} onChange={(visible) => set({ visible })} />
      <SliderField label="Size" value={c.scale} display={`${c.scale.toFixed(1)}×`} min={0.5} max={4} step={0.05}
        {...bind((e, v) => ({ ...e, cursor: { ...e.cursor, scale: v } }))} />
      <SliderField label="Smooth movement" value={c.smoothing} display={c.smoothing === 0 ? "Off" : `${c.smoothing.toFixed(2)} s`} min={0} max={0.5} step={0.01}
        {...bind((e, v) => ({ ...e, cursor: { ...e.cursor, smoothing: v } }))} />
      <ToggleRow label="Hide when idle" hint="Fade out after the cursor rests for a while." value={c.hideWhenIdle > 0} onChange={(on) => set({ hideWhenIdle: on ? 2 : 0 })} />
      {c.hideWhenIdle > 0 && (
        <SliderField label="Idle delay" value={c.hideWhenIdle} display={`${c.hideWhenIdle.toFixed(1)} s`} min={0.5} max={8} step={0.1}
          {...bind((e, v) => ({ ...e, cursor: { ...e.cursor, hideWhenIdle: v } }))} />
      )}
      <div className="divider" />
      <ToggleRow label="Click effect" hint="A soft ripple wherever you clicked." value={c.clickEffect} onChange={(clickEffect) => set({ clickEffect })} />
      {c.clickEffect && (
        <>
          <SliderField label="Ripple size" value={c.clickSize} display={`${Math.round(c.clickSize)} px`} min={10} max={100} step={1}
            {...bind((e, v) => ({ ...e, cursor: { ...e.cursor, clickSize: v } }))} />
          <div className="field-row">
            <span className="label">Ripple color</span>
            <div className="row">
              {["#FFFFFF", "#6B52F5", "#FACC15", "#38BDF8"].map((col) => (
                <button key={col} className={`swatch tiny ${c.clickColor.toUpperCase() === col ? "on" : ""}`} style={{ background: col }} onClick={() => set({ clickColor: col })} aria-label={col} />
              ))}
            </div>
          </div>
        </>
      )}
    </div>
  );
}

function FocusPicker({ seg }: { seg: ZoomSegment }) {
  const dir = useEditor((s) => s.dir);
  const thumb = useEditor((s) => s.project?.media.thumbnail);
  const aspect = useEditor((s) => (s.project ? s.project.source.width / s.project.source.height : 16 / 9));
  const { update, beginGesture, endGesture } = useEditor.getState();
  if (seg.focus.mode !== "manual") return null;
  const { x, y } = seg.focus;
  const setFrom = (e: React.PointerEvent<HTMLDivElement>, history: boolean) => {
    const r = e.currentTarget.getBoundingClientRect();
    const nx = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
    const ny = Math.min(1, Math.max(0, (e.clientY - r.top) / r.height));
    update((ed) => ({ ...ed, zoom: { ...ed.zoom, segments: ed.zoom.segments.map((s) => (s.id === seg.id ? { ...s, focus: { mode: "manual", x: nx, y: ny } } : s)) } }), { history });
  };
  const viewW = 100 / seg.level;
  return (
    <div className="field">
      <span className="label">Focus point — drag to aim the zoom</span>
      <div
        className="focus-picker"
        style={{ aspectRatio: String(aspect), backgroundImage: thumb ? `url("${fileUrl(joinPath(dir, thumb))}")` : undefined }}
        onPointerDown={(e) => {
          e.currentTarget.setPointerCapture(e.pointerId);
          beginGesture();
          setFrom(e, false);
        }}
        onPointerMove={(e) => e.buttons & 1 && setFrom(e, false)}
        onPointerUp={() => endGesture()}
      >
        <div
          className="focus-view"
          style={{
            width: `${viewW}%`,
            height: `${viewW}%`,
            left: `${Math.min(Math.max(x * 100 - viewW / 2, 0), 100 - viewW)}%`,
            top: `${Math.min(Math.max(y * 100 - viewW / 2, 0), 100 - viewW)}%`,
          }}
        />
        <div className="focus-dot" style={{ left: `${x * 100}%`, top: `${y * 100}%` }} />
      </div>
    </div>
  );
}

function ZoomPanel() {
  const zoom = useEditor((s) => s.edit!.zoom);
  const dir = useEditor((s) => s.dir);
  const hasCursor = useEditor((s) => s.hasCursor);
  const selection = useEditor((s) => s.selection);
  const update = useEditor((s) => s.update);
  const bind = useSliderBinding();
  const seg = selection?.kind === "zoom" ? zoom.segments.find((s) => s.id === selection.id) : undefined;
  const st = zoom.settings;

  const regenerate = async (settings = st) => {
    const edit = useEditor.getState().edit!;
    try {
      const segments = await api.regenerateZoom(dir, { ...edit, zoom: { ...edit.zoom, settings } });
      update((e) => ({ ...e, zoom: { settings, segments } }));
      toast.info(settings.autoZoom ? `${segments.filter((s) => s.auto).length} automatic zooms` : "Automatic zooms removed");
    } catch (e) {
      toast.error("Could not generate zooms", errorText(e));
    }
  };

  const setSeg = (patch: Partial<ZoomSegment>) =>
    update((e) => ({ ...e, zoom: { ...e.zoom, segments: e.zoom.segments.map((s) => (s.id === seg!.id ? { ...s, ...patch, auto: false } : s)) } }));

  return (
    <div className="panel-body">
      <PanelHeader title="Zoom" />
      {seg ? (
        <div className="selected-block">
          <div className="row">
            <span className="seg-chip"><ZoomIn size={12} /> Selected zoom</span>
            <span className="spacer" />
            <Toggle value={seg.enabled} onChange={(enabled) => setSeg({ enabled })} label="Enabled" />
          </div>
          <SliderField label="Zoom level" value={seg.level} display={`${seg.level.toFixed(2)}×`} min={1.1} max={st.maxLevel} step={0.05}
            {...bind((e, v) => ({ ...e, zoom: { ...e.zoom, segments: e.zoom.segments.map((s) => (s.id === seg.id ? { ...s, level: v, auto: false } : s)) } }))} />
          <div className="field">
            <span className="label">Focus</span>
            <Segmented<"auto" | "manual">
              value={seg.focus.mode}
              onChange={(m) => setSeg({ focus: m === "auto" ? { mode: "auto" } : { mode: "manual", x: seg.anchor?.x ?? 0.5, y: seg.anchor?.y ?? 0.5 } })}
              options={[
                { value: "auto", label: <><MousePointer2 size={13} /> Follow cursor</> },
                { value: "manual", label: <><Crosshair size={13} /> Fixed point</> },
              ]}
            />
          </div>
          <FocusPicker seg={seg} />
          <button className="btn danger" onClick={() => { update((e) => ({ ...e, zoom: { ...e.zoom, segments: e.zoom.segments.filter((s) => s.id !== seg.id) } })); useEditor.getState().select(null); }}>
            <Trash2 size={14} /> Remove zoom
          </button>
        </div>
      ) : (
        <p className="muted hint-box">Select a zoom on the timeline to edit it, or click an empty spot on the zoom track to add one.</p>
      )}
      <div className="divider" />
      <div className="section-title">Automatic zoom</div>
      <ToggleRow
        label="Zoom in on clicks"
        hint={hasCursor ? "Generated from the clicks you made while recording." : "Imported videos have no click data."}
        value={st.autoZoom}
        onChange={(autoZoom) => regenerate({ ...st, autoZoom })}
      />
      <SliderField label="Default level" value={st.defaultLevel} display={`${st.defaultLevel.toFixed(1)}×`} min={1.25} max={st.maxLevel} step={0.05}
        {...bind((e, v) => ({ ...e, zoom: { ...e.zoom, settings: { ...e.zoom.settings, defaultLevel: v } } }))} />
      <SliderField label="Animation duration" value={st.transition} display={`${st.transition.toFixed(2)} s`} min={0.2} max={2} step={0.05}
        {...bind((e, v) => ({ ...e, zoom: { ...e.zoom, settings: { ...e.zoom.settings, transition: v } } }))} />
      <SliderField label="Follow smoothness" value={st.followSmoothing} display={`${st.followSmoothing.toFixed(2)} s`} min={0.05} max={1.5} step={0.05}
        {...bind((e, v) => ({ ...e, zoom: { ...e.zoom, settings: { ...e.zoom.settings, followSmoothing: v } } }))} />
      <div className="field">
        <span className="label">Easing</span>
        <select className="select" value={st.easing} onChange={(ev) => update((e) => ({ ...e, zoom: { ...e.zoom, settings: { ...e.zoom.settings, easing: ev.target.value as Easing } } }))}>
          <option value="ease-in-out-cubic">Smooth (cubic)</option>
          <option value="ease-in-out-quint">Snappy (quint)</option>
          <option value="smoother">Gentle (smootherstep)</option>
          <option value="ease-out-cubic">Ease out</option>
          <option value="linear">Linear</option>
        </select>
      </div>
      <button className="btn" disabled={!hasCursor || !st.autoZoom} onClick={() => regenerate()} title="Re-create automatic zooms; zooms you edited are kept">
        <RefreshCw size={14} /> Regenerate automatic zooms
      </button>
    </div>
  );
}

function AudioPanel() {
  const a = useEditor((s) => s.edit!.audio);
  const media = useEditor((s) => s.project!.media);
  const update = useEditor((s) => s.update);
  const bind = useSliderBinding();
  const tracks = [
    media.systemAudio && { key: "system" as const, label: "System audio", vol: a.systemVolume, muted: a.systemMuted },
    media.mic && { key: "mic" as const, label: "Microphone", vol: a.micVolume, muted: a.micMuted },
  ].filter(Boolean) as { key: "mic" | "system"; label: string; vol: number; muted: boolean }[];
  return (
    <div className="panel-body">
      <PanelHeader title="Audio" />
      {tracks.length === 0 && <p className="muted">This recording has no audio. Enable the microphone or system audio before recording to capture sound.</p>}
      {tracks.map((t) => (
        <div key={t.key} className="audio-track">
          <div className="field-row">
            <span style={{ fontWeight: 500 }}>{t.label}</span>
            <button
              className={`btn ghost icon sm ${t.muted ? "danger" : ""}`}
              title={t.muted ? "Unmute" : "Mute"}
              onClick={() => update((e) => ({ ...e, audio: { ...e.audio, ...(t.key === "mic" ? { micMuted: !t.muted } : { systemMuted: !t.muted }) } }))}
            >
              {t.muted ? <VolumeX size={15} /> : <Volume2 size={15} />}
            </button>
          </div>
          <SliderField label="Volume" value={t.vol} display={`${Math.round(t.vol * 100)}%`} min={0} max={2} step={0.01}
            {...bind((e, v) => ({ ...e, audio: { ...e.audio, ...(t.key === "mic" ? { micVolume: v } : { systemVolume: v }) } }))} />
        </div>
      ))}
      {tracks.length > 0 && <p className="faint" style={{ fontSize: 11.5 }}>Preview playback caps volume at 100%; exports apply the full gain.</p>}
    </div>
  );
}

function ClipPanel() {
  const tl = useEditor((s) => s.edit!.timeline);
  const selection = useEditor((s) => s.selection);
  const time = useEditor((s) => s.time);
  const update = useEditor((s) => s.update);
  const clip = selection?.kind === "clip" ? tl.clips.find((c) => c.id === selection.id) : undefined;
  return (
    <div className="panel-body">
      <PanelHeader title="Clip" />
      <p className="muted hint-box">Drag the edges of a clip to trim it. Split at the playhead to cut out a section or change its speed.</p>
      <button className="btn" onClick={() => { const n = splitAt(tl, time); if (n) update((e) => ({ ...e, timeline: n })); else toast.info("Move the playhead inside a clip to split it"); }}>
        <Scissors size={14} /> Split at playhead <span className="kbd">S</span>
      </button>
      {clip ? (
        <>
          <div className="divider" />
          <div className="field">
            <span className="label">Speed</span>
            <Segmented<number>
              value={clip.speed}
              onChange={(speed) => update((e) => ({ ...e, timeline: { ...e.timeline, clips: e.timeline.clips.map((c) => (c.id === clip.id ? { ...c, speed } : c)) } }))}
              options={[0.5, 1, 1.5, 2, 4].map((v) => ({ value: v, label: `${v}×` }))}
            />
          </div>
          <div className="faint" style={{ fontSize: 12 }}>
            Source {clip.start.toFixed(2)}s – {clip.end.toFixed(2)}s
          </div>
          <button
            className="btn danger"
            disabled={tl.clips.length <= 1}
            onClick={() => {
              const n = removeClip(tl, clip.id);
              if (n) {
                update((e) => ({ ...e, timeline: n }));
                useEditor.getState().select(null);
              }
            }}
          >
            <Trash2 size={14} /> Delete clip
          </button>
        </>
      ) : (
        <p className="faint">Select a clip on the timeline to change its speed or delete it.</p>
      )}
    </div>
  );
}

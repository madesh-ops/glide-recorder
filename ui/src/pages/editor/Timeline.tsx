import { useEffect, useMemo, useRef, useState } from "react";
import {
  Film, Pause, Play, Plus, Redo2, Scissors, SkipBack, SkipForward, Trash2, Undo2, ZoomIn, Eye, EyeOff, Crosshair, MousePointer2,
} from "lucide-react";
import type { ZoomSegment } from "../../lib/api";
import { formatTime } from "../../lib/format";
import {
  MIN_CLIP, newSegmentId, outputDuration, outputToSource, segmentOutputRange, sourceToOutputClamped, splitAt,
} from "../../lib/timeline";
import { useEditor } from "../../store/editor";
import { player } from "./player";

const PAD = 16; // px before time 0
const MIN_SEG = 0.4;

type Drag =
  | { kind: "scrub" }
  | { kind: "seg-move"; id: string; x0: number; start: number; end: number; moved: boolean }
  | { kind: "seg-start" | "seg-end"; id: string; x0: number; start: number; end: number }
  | { kind: "clip-start" | "clip-end"; id: string; x0: number; start: number; end: number; speed: number };

function niceStep(pps: number): number {
  const steps = [0.1, 0.25, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300];
  return steps.find((s) => s * pps >= 70) ?? 600;
}

export function Timeline() {
  const edit = useEditor((s) => s.edit);
  const time = useEditor((s) => s.time);
  const playing = useEditor((s) => s.playing);
  const selection = useEditor((s) => s.selection);
  const canUndo = useEditor((s) => s.past.length > 0);
  const canRedo = useEditor((s) => s.future.length > 0);
  const fps = useEditor((s) => s.project?.source.fps ?? 60);
  const { update, beginGesture, endGesture, select, undo, redo } = useEditor.getState();

  const scrollRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(800);
  const [zoomFactor, setZoomFactor] = useState(1);
  const [drag, setDrag] = useState<Drag | null>(null);
  const [ghost, setGhost] = useState<number | null>(null);
  const [menu, setMenu] = useState<{ seg: ZoomSegment; x: number; y: number } | null>(null);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setWidth(el.clientWidth));
    ro.observe(el);
    setWidth(el.clientWidth);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [menu]);

  const tl = edit?.timeline;
  const duration = tl ? outputDuration(tl) : 0;
  const pps = Math.max(4, ((width - PAD * 2) / Math.max(duration, 0.5)) * zoomFactor);
  const xOf = (outT: number) => PAD + outT * pps;
  const tOf = (x: number) => Math.max(0, Math.min(duration, (x - PAD) / pps));
  const playheadOut = tl ? sourceToOutputClamped(tl, time) : 0;

  // Keep the playhead visible while playing.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el || !playing) return;
    const x = xOf(playheadOut);
    if (x > el.scrollLeft + el.clientWidth - 40) el.scrollLeft = x - 80;
    if (x < el.scrollLeft) el.scrollLeft = Math.max(0, x - 80);
  });

  const step = niceStep(pps);
  const ticks = useMemo(() => {
    const out: number[] = [];
    for (let t = 0; t <= duration + 1e-6; t += step) out.push(t);
    return out;
  }, [duration, step]);

  if (!edit || !tl) return <div className="timeline" />;

  const segments = edit.zoom.segments;

  const localX = (e: { clientX: number }) => {
    const el = scrollRef.current!;
    return e.clientX - el.getBoundingClientRect().left + el.scrollLeft;
  };

  const onPointerMove = (e: React.PointerEvent) => {
    if (!drag) return;
    const x = localX(e);
    if (drag.kind === "scrub") {
      player.seekOutput(tOf(x));
      return;
    }
    const dOut = (x - drag.x0) / pps;
    if (drag.kind === "seg-move" || drag.kind === "seg-start" || drag.kind === "seg-end") {
      const others = segments.filter((s) => s.id !== drag.id);
      const prevEnd = Math.max(0, ...others.filter((s) => s.end <= drag.start + 1e-6).map((s) => s.end));
      const nextStart = Math.min(tl.sourceDuration, ...others.filter((s) => s.start >= drag.end - 1e-6).map((s) => s.start));
      const outStart = sourceToOutputClamped(tl, drag.start);
      const outEnd = sourceToOutputClamped(tl, drag.end);
      let start = drag.start;
      let end = drag.end;
      if (drag.kind === "seg-move") {
        const len = drag.end - drag.start;
        start = outputToSource(tl, Math.max(0, outStart + dOut));
        start = Math.min(Math.max(start, prevEnd), nextStart - len);
        end = start + len;
        if (Math.abs(x - drag.x0) > 3 && !drag.moved) setDrag({ ...drag, moved: true });
      } else if (drag.kind === "seg-start") {
        start = Math.min(Math.max(outputToSource(tl, Math.max(0, outStart + dOut)), prevEnd), end - MIN_SEG);
      } else {
        end = Math.max(Math.min(outputToSource(tl, outEnd + dOut), nextStart), start + MIN_SEG);
      }
      update(
        (ed) => ({
          ...ed,
          zoom: {
            ...ed.zoom,
            segments: ed.zoom.segments.map((s) => (s.id === drag.id ? { ...s, start, end, auto: false } : s)),
          },
        }),
        { history: false },
      );
      return;
    }
    if (drag.kind === "clip-start" || drag.kind === "clip-end") {
      const idx = tl.clips.findIndex((c) => c.id === drag.id);
      const prevEnd = idx > 0 ? tl.clips[idx - 1].end : 0;
      const nextStart = idx < tl.clips.length - 1 ? tl.clips[idx + 1].start : tl.sourceDuration;
      const dSrc = dOut * drag.speed;
      update(
        (ed) => ({
          ...ed,
          timeline: {
            ...ed.timeline,
            clips: ed.timeline.clips.map((c) => {
              if (c.id !== drag.id) return c;
              if (drag.kind === "clip-start") return { ...c, start: Math.min(Math.max(drag.start + dSrc, prevEnd), c.end - MIN_CLIP) };
              return { ...c, end: Math.max(Math.min(drag.end + dSrc, nextStart), c.start + MIN_CLIP) };
            }),
          },
        }),
        { history: false },
      );
    }
  };

  const endDrag = () => {
    if (!drag) return;
    if (drag.kind === "seg-move" && !drag.moved) select({ kind: "zoom", id: drag.id });
    if (drag.kind !== "scrub") endGesture();
    setDrag(null);
  };

  const startDrag = (e: React.PointerEvent, d: Drag) => {
    e.stopPropagation();
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
    scrollRef.current?.setPointerCapture(e.pointerId);
    if (d.kind !== "scrub") beginGesture();
    setDrag(d);
  };

  const addZoomAt = (outT: number) => {
    const src = outputToSource(tl, outT);
    const others = segments;
    const len = 2.5;
    let start = Math.max(0, src - 0.6);
    let end = Math.min(tl.sourceDuration, start + len);
    const blocking = others.find((s) => s.start < end && start < s.end);
    if (blocking) return;
    const next = others.filter((s) => s.start >= start).sort((a, b) => a.start - b.start)[0];
    if (next && next.start < end) end = next.start;
    if (end - start < MIN_SEG) return;
    const seg: ZoomSegment = {
      id: newSegmentId(),
      start,
      end,
      level: edit.zoom.settings.defaultLevel,
      focus: { mode: "auto" },
      enabled: true,
      auto: false,
    };
    update((ed) => ({ ...ed, zoom: { ...ed.zoom, segments: [...ed.zoom.segments, seg].sort((a, b) => a.start - b.start) } }));
    select({ kind: "zoom", id: seg.id });
  };

  const removeSegment = (id: string) => {
    update((ed) => ({ ...ed, zoom: { ...ed.zoom, segments: ed.zoom.segments.filter((s) => s.id !== id) } }));
    select(null);
  };

  const splitAtPlayhead = () => {
    const next = splitAt(tl, time);
    if (next) update((ed) => ({ ...ed, timeline: next }));
  };

  const ghostFits = (outT: number) => {
    const src = outputToSource(tl, outT);
    return !segments.some((s) => src >= s.start - 0.2 && src <= s.end + 0.2);
  };

  return (
    <div className="timeline">
      <div className="tl-toolbar">
        <div className="row">
          <button className="btn ghost icon" title="Undo (Ctrl+Z)" disabled={!canUndo} onClick={undo}><Undo2 size={16} /></button>
          <button className="btn ghost icon" title="Redo (Ctrl+Shift+Z)" disabled={!canRedo} onClick={redo}><Redo2 size={16} /></button>
        </div>
        <div className="transport">
          <span className="mono time-now">{formatTime(playheadOut, true)}</span>
          <button className="btn ghost icon" title="Previous frame (←)" onClick={() => player.step(-1 / fps)}><SkipBack size={15} /></button>
          <button className="btn play-btn" title="Play / pause (Space)" onClick={() => player.toggle()}>
            {playing ? <Pause size={16} fill="currentColor" /> : <Play size={16} fill="currentColor" />}
          </button>
          <button className="btn ghost icon" title="Next frame (→)" onClick={() => player.step(1 / fps)}><SkipForward size={15} /></button>
          <span className="mono faint">{formatTime(duration, true)}</span>
        </div>
        <div className="row">
          <button className="btn ghost sm" title="Add zoom at playhead (Z)" onClick={() => addZoomAt(playheadOut)}><Plus size={14} /> Zoom</button>
          <button className="btn ghost sm" title="Split clip at playhead (S)" onClick={splitAtPlayhead}><Scissors size={14} /> Split</button>
          <ZoomIn size={14} className="faint" />
          <input
            type="range"
            className="slider"
            style={{ width: 110, ["--fill" as string]: `${((zoomFactor - 1) / 19) * 100}%` }}
            min={1}
            max={20}
            step={0.1}
            value={zoomFactor}
            aria-label="Timeline zoom"
            onChange={(e) => setZoomFactor(Number(e.target.value))}
          />
        </div>
      </div>

      <div className="tl-main">
        <div className="tl-heads">
          <div className="tl-head ruler-head" />
          <div className="tl-head" title="Clips"><Film size={14} /></div>
          <div className="tl-head" title="Zooms"><ZoomIn size={14} /></div>
        </div>
        <div
          className="tl-scroll"
          ref={scrollRef}
          onPointerMove={onPointerMove}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
          onWheel={(e) => {
            if (e.ctrlKey) {
              setZoomFactor((z) => Math.min(20, Math.max(1, z * (e.deltaY < 0 ? 1.15 : 1 / 1.15))));
            } else if (scrollRef.current && Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
              scrollRef.current.scrollLeft += e.deltaY;
            }
          }}
        >
          <div className="tl-content" style={{ width: xOf(duration) + PAD }}>
            <div
              className="ruler"
              onPointerDown={(e) => {
                player.pause();
                player.seekOutput(tOf(localX(e)));
                startDrag(e, { kind: "scrub" });
              }}
            >
              {ticks.map((t) => (
                <div key={t} className="tick" style={{ left: xOf(t) }}>
                  <span>{formatTime(t)}</span>
                </div>
              ))}
            </div>

            <div
              className="track clips-track"
              onPointerDown={(e) => {
                if (e.target === e.currentTarget) {
                  select(null);
                  player.pause();
                  player.seekOutput(tOf(localX(e)));
                  startDrag(e, { kind: "scrub" });
                }
              }}
            >
              {(() => {
                let acc = 0;
                return tl.clips.map((c) => {
                  const len = (c.end - c.start) / c.speed;
                  const left = xOf(acc);
                  acc += len;
                  const sel = selection?.kind === "clip" && selection.id === c.id;
                  return (
                    <div
                      key={c.id}
                      className={`clip ${sel ? "selected" : ""}`}
                      style={{ left, width: Math.max(4, len * pps - 2) }}
                      onPointerDown={(e) => {
                        e.stopPropagation();
                        select({ kind: "clip", id: c.id });
                        player.pause();
                        player.seekOutput(tOf(localX(e)));
                      }}
                    >
                      <div className="handle l" onPointerDown={(e) => { select({ kind: "clip", id: c.id }); startDrag(e, { kind: "clip-start", id: c.id, x0: localX(e), start: c.start, end: c.end, speed: c.speed }); }} />
                      <span className="clip-label">
                        <Film size={12} /> Clip <span className="pill">{c.speed}×</span>
                        <span className="faint-dark">{formatTime(len)}</span>
                      </span>
                      <div className="handle r" onPointerDown={(e) => { select({ kind: "clip", id: c.id }); startDrag(e, { kind: "clip-end", id: c.id, x0: localX(e), start: c.start, end: c.end, speed: c.speed }); }} />
                    </div>
                  );
                });
              })()}
            </div>

            <div
              className="track zoom-track"
              onPointerMove={(e) => {
                if (drag || e.target !== e.currentTarget) return setGhost(null);
                const t = tOf(localX(e));
                setGhost(ghostFits(t) ? t : null);
              }}
              onPointerLeave={() => setGhost(null)}
              onPointerDown={(e) => {
                if (e.target === e.currentTarget && e.button === 0) {
                  addZoomAt(tOf(localX(e)));
                  setGhost(null);
                }
              }}
            >
              {segments.length === 0 && ghost === null && (
                <span className="track-hint">Click to add a zoom · zooms are created from your clicks automatically</span>
              )}
              {ghost !== null && (
                <div className="seg ghost" style={{ left: xOf(Math.max(0, ghost - 0.6)), width: 2.5 * pps }}>
                  <Plus size={12} /> Add zoom
                </div>
              )}
              {segments.map((s) => {
                const r = segmentOutputRange(tl, s);
                if (!r) return null;
                const sel = selection?.kind === "zoom" && selection.id === s.id;
                return (
                  <div
                    key={s.id}
                    className={`seg ${sel ? "selected" : ""} ${s.enabled ? "" : "disabled"}`}
                    style={{ left: xOf(r[0]), width: Math.max(6, (r[1] - r[0]) * pps - 2) }}
                    onPointerDown={(e) => {
                      if (e.button !== 0) return;
                      startDrag(e, { kind: "seg-move", id: s.id, x0: localX(e), start: s.start, end: s.end, moved: false });
                    }}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      select({ kind: "zoom", id: s.id });
                      setMenu({ seg: s, x: e.clientX, y: e.clientY });
                    }}
                  >
                    <div className="handle l" onPointerDown={(e) => startDrag(e, { kind: "seg-start", id: s.id, x0: localX(e), start: s.start, end: s.end })} />
                    <span className="seg-label">
                      <span className="seg-title"><ZoomIn size={11} /> Zoom</span>
                      <span className="seg-sub">
                        {s.level.toFixed(1)}× · {s.focus.mode === "auto" ? <><MousePointer2 size={10} /> Auto</> : <><Crosshair size={10} /> Manual</>}
                      </span>
                    </span>
                    <div className="handle r" onPointerDown={(e) => startDrag(e, { kind: "seg-end", id: s.id, x0: localX(e), start: s.start, end: s.end })} />
                  </div>
                );
              })}
            </div>

            <div className="playhead" style={{ left: xOf(playheadOut) }}>
              <div className="playhead-knob" onPointerDown={(e) => { player.pause(); startDrag(e, { kind: "scrub" }); }} />
            </div>
          </div>
        </div>
      </div>

      {menu && (
        <div className="context-menu" style={{ left: menu.x, top: menu.y - 90 }} onMouseDown={(e) => e.stopPropagation()}>
          <button
            onClick={() => {
              update((ed) => ({ ...ed, zoom: { ...ed.zoom, segments: ed.zoom.segments.map((s) => (s.id === menu.seg.id ? { ...s, enabled: !s.enabled, auto: false } : s)) } }));
              setMenu(null);
            }}
          >
            {menu.seg.enabled ? <><EyeOff size={14} /> Disable</> : <><Eye size={14} /> Enable</>}
          </button>
          <div className="sep" />
          <button onClick={() => { removeSegment(menu.seg.id); setMenu(null); }}><Trash2 size={14} /> Remove</button>
        </div>
      )}
    </div>
  );
}


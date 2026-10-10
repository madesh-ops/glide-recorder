// Live preview: the recorded video inside the styled scene, transformed by the
// Rust-computed camera, with the cursor and click ripples re-rendered on top.
// Per-frame updates write styles directly (no React re-render per frame).
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { fileUrl, joinPath, type PreviewModel } from "../../lib/api";
import { backgroundCss, FALLBACK_CURSOR_SCALE, REF_HEIGHT, rippleScale, sampleCamera, sampleCursor, sceneTransform } from "../../lib/preview";
import { useEditor } from "../../store/editor";
import { player } from "./player";

const ARROW_SVG =
  "data:image/svg+xml;utf8," +
  encodeURIComponent(
    `<svg xmlns="http://www.w3.org/2000/svg" width="22" height="34" viewBox="0 0 22 34"><path d="M1.5 1.5 L1.5 27.9 L7.9 21.98 L12.54 32.22 L16.38 30.62 L11.9 20.54 L20.06 20.54 Z" fill="#000" stroke="#fff" stroke-width="1.6" stroke-linejoin="round"/></svg>`,
  );
const ARROW = { w: 22, h: 34, hx: 1.5, hy: 1.5 };
const MAX_RIPPLES = 4;

export function Preview() {
  const dir = useEditor((s) => s.dir);
  const videoPath = useEditor((s) => s.videoPath);
  const project = useEditor((s) => s.project);
  const preview = useEditor((s) => s.preview);
  const style = useEditor((s) => s.edit?.style);
  const cursorSettings = useEditor((s) => s.edit?.cursor);
  const audio = useEditor((s) => s.edit?.audio);

  const wrapRef = useRef<HTMLDivElement>(null);
  const sceneRef = useRef<HTMLDivElement>(null);
  const videoRef = useRef<HTMLVideoElement>(null);
  const cursorRef = useRef<HTMLImageElement>(null);
  const rippleRefs = useRef<(HTMLDivElement | null)[]>([]);
  const micRef = useRef<HTMLAudioElement>(null);
  const sysRef = useRef<HTMLAudioElement>(null);
  const [box, setBox] = useState({ w: 0, h: 0 });
  const [loadError, setLoadError] = useState<string | null>(null);

  // Fit the canvas into the available area.
  useLayoutEffect(() => {
    const el = wrapRef.current;
    if (!el || !preview) return;
    const fit = () => {
      const pad = 40;
      const aw = Math.max(0, el.clientWidth - pad * 2);
      const ah = Math.max(0, el.clientHeight - pad * 2);
      const a = preview.layout.aspect;
      const w = Math.min(aw, ah * a);
      setBox({ w: Math.floor(w), h: Math.floor(w / a) });
    };
    fit();
    const ro = new ResizeObserver(fit);
    ro.observe(el);
    return () => ro.disconnect();
  }, [preview?.layout.aspect, preview]);

  useEffect(() => {
    player.attach(videoRef.current);
    const list: { el: HTMLAudioElement; track: "mic" | "system" }[] = [];
    if (micRef.current) list.push({ el: micRef.current, track: "mic" });
    if (sysRef.current) list.push({ el: sysRef.current, track: "system" });
    player.setAudio(list);
    return () => player.attach(null);
  }, [videoPath, project?.media.mic, project?.media.systemAudio]);

  useEffect(() => player.syncAudioParams(), [audio]);

  const shapeUrls = useMemo(() => (preview?.shapes ?? []).map((s) => fileUrl(joinPath(dir, s.file))), [preview?.shapes, dir]);

  // Per-frame render loop.
  const latest = useRef<{ preview: PreviewModel | null; box: typeof box }>({ preview: null, box });
  latest.current = { preview, box };
  useEffect(() => {
    let raf = 0;
    const frame = () => {
      raf = requestAnimationFrame(frame);
      player.tick();
      const { preview: p, box: b } = latest.current;
      const st = useEditor.getState();
      const edit = st.edit;
      if (!p || !edit || !b.w || !sceneRef.current) return;
      const t = st.playing ? player.time : st.time;
      const cam = sampleCamera(p, t);
      sceneRef.current.style.transform = sceneTransform(cam, b.w, b.h);

      const layout = p.layout;
      const sx = (layout.screen.w * b.w) / layout.sourceWidth;
      const sy = (layout.screen.h * b.h) / layout.sourceHeight;
      const ref = b.h / REF_HEIGHT;
      const cs = edit.cursor;
      const cur = cursorRef.current;
      const c = sampleCursor(p, t);
      if (cur) {
        if (c && cs.visible && c.opacity > 0.001) {
          const shape = c.shape >= 0 ? p.shapes[c.shape] : undefined;
          const px = layout.screen.x * b.w + c.x * sx;
          const py = layout.screen.y * b.h + c.y * sy;
          let w: number, h: number, hx: number, hy: number, src: string;
          if (shape) {
            const k = sx * cs.scale;
            w = shape.width * k;
            h = shape.height * k;
            hx = shape.hotspotX * k;
            hy = shape.hotspotY * k;
            src = shapeUrls[c.shape];
          } else {
            const k = ref * cs.scale * FALLBACK_CURSOR_SCALE;
            w = ARROW.w * k;
            h = ARROW.h * k;
            hx = ARROW.hx * k;
            hy = ARROW.hy * k;
            src = ARROW_SVG;
          }
          if (cur.getAttribute("src") !== src) cur.setAttribute("src", src);
          cur.style.width = `${w}px`;
          cur.style.height = `${h}px`;
          cur.style.transform = `translate(${px - hx}px, ${py - hy}px)`;
          cur.style.opacity = String(c.opacity);
        } else {
          cur.style.opacity = "0";
        }
      }

      // Click ripples (newest last).
      const active = cs.clickEffect
        ? p.clicks.filter((k) => (k.button === "left" || k.button === "right") && t >= k.t && t <= k.t + cs.clickDuration).slice(-MAX_RIPPLES)
        : [];
      for (let i = 0; i < MAX_RIPPLES; i++) {
        const el = rippleRefs.current[i];
        if (!el) continue;
        const k = active[i];
        if (!k) {
          el.style.opacity = "0";
          continue;
        }
        const prog = (t - k.t) / cs.clickDuration;
        const r = cs.clickSize * ref * rippleScale(prog);
        const cx = layout.screen.x * b.w + k.x * sx;
        const cy = layout.screen.y * b.h + k.y * sy;
        el.style.width = el.style.height = `${r * 2}px`;
        el.style.transform = `translate(${cx - r}px, ${cy - r}px)`;
        el.style.opacity = String(1 - prog);
        el.style.borderColor = cs.clickColor;
        el.style.background = cs.clickColor + "47";
      }
    };
    raf = requestAnimationFrame(frame);
    return () => cancelAnimationFrame(raf);
  }, [shapeUrls]);

  if (!preview || !style || !cursorSettings || !project) return <div className="preview" ref={wrapRef} />;

  const layout = preview.layout;
  const ref = box.h / REF_HEIGHT;
  const radius = Math.min(style.cornerRadius * ref, (layout.screen.w * box.w) / 2, (layout.screen.h * box.h) / 2);
  const bg = backgroundCss(style.background, (p) => fileUrl(joinPath(dir, p)));

  return (
    <div className="preview" ref={wrapRef} onClick={() => player.toggle()}>
      <div className="canvas" style={{ width: box.w, height: box.h }}>
        <div className="scene" ref={sceneRef} style={{ width: box.w, height: box.h }}>
          <div className="scene-bg" style={{ background: bg }} />
          <div
            className="screen"
            style={{
              left: layout.screen.x * box.w,
              top: layout.screen.y * box.h,
              width: layout.screen.w * box.w,
              height: layout.screen.h * box.h,
              borderRadius: radius,
              boxShadow:
                style.shadowOpacity > 0
                  ? `0 ${style.shadowOffsetY * ref}px ${style.shadowBlur * ref}px rgba(0,0,0,${style.shadowOpacity})`
                  : "none",
            }}
          >
            <video
              ref={videoRef}
              src={fileUrl(videoPath)}
              preload="auto"
              muted
              playsInline
              disablePictureInPicture
              onLoadedData={() => {
                setLoadError(null);
                player.seek(useEditor.getState().time);
              }}
              onError={() => setLoadError("The recording could not be loaded for preview.")}
            />
          </div>
          {Array.from({ length: MAX_RIPPLES }).map((_, i) => (
            <div key={i} className="ripple" ref={(el) => { rippleRefs.current[i] = el; }} />
          ))}
          <img className="cursor" ref={cursorRef} alt="" draggable={false} />
        </div>
      </div>
      {project.media.mic && <audio ref={micRef} src={fileUrl(joinPath(dir, project.media.mic))} preload="auto" />}
      {project.media.systemAudio && <audio ref={sysRef} src={fileUrl(joinPath(dir, project.media.systemAudio))} preload="auto" />}
      {loadError && <div className="preview-error">{loadError}</div>}
    </div>
  );
}

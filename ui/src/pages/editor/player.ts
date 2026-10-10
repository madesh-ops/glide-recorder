// Playback controller shared by the preview, timeline and keyboard shortcuts.
// The <video> is the clock; separate <audio> elements (mic / system WAVs) are
// kept in sync with it. Clip boundaries (cuts, speed) are enforced here.
import { useEditor } from "../../store/editor";
import { clipAt, nextPlayable, outputToSource, sourceToOutputClamped } from "../../lib/timeline";

class Player {
  video: HTMLVideoElement | null = null;
  audios: { el: HTMLAudioElement; track: "mic" | "system" }[] = [];
  /** Source time requested by the last seek (shown until the video catches up). */
  pendingSeek: number | null = null;

  attach(video: HTMLVideoElement | null) {
    this.video = video;
  }

  setAudio(list: { el: HTMLAudioElement; track: "mic" | "system" }[]) {
    this.audios = list;
    this.syncAudioParams();
  }

  syncAudioParams() {
    const edit = useEditor.getState().edit;
    if (!edit) return;
    for (const a of this.audios) {
      const vol = a.track === "mic" ? edit.audio.micVolume : edit.audio.systemVolume;
      const muted = a.track === "mic" ? edit.audio.micMuted : edit.audio.systemMuted;
      a.el.volume = Math.min(1, Math.max(0, vol));
      a.el.muted = muted;
    }
  }

  get time(): number {
    return this.pendingSeek ?? this.video?.currentTime ?? 0;
  }

  seek(sourceT: number) {
    const v = this.video;
    const st = useEditor.getState();
    const dur = st.edit?.timeline.sourceDuration ?? 0;
    const t = Math.min(Math.max(0, sourceT), Math.max(0, dur - 0.001));
    st.setTime(t);
    if (!v) return;
    this.pendingSeek = t;
    v.currentTime = t;
    for (const a of this.audios) a.el.currentTime = t;
  }

  seekOutput(outT: number) {
    const tl = useEditor.getState().edit?.timeline;
    if (tl) this.seek(outputToSource(tl, outT));
  }

  async play() {
    const v = this.video;
    const tl = useEditor.getState().edit?.timeline;
    if (!v || !tl) return;
    let t = nextPlayable(tl, v.currentTime);
    if (t === null) t = tl.clips[0]?.start ?? 0; // at the end: restart
    if (Math.abs(t - v.currentTime) > 0.01) this.seek(t);
    const clip = clipAt(tl, t);
    this.setRate(clip?.speed ?? 1);
    useEditor.getState().setPlaying(true);
    try {
      await v.play();
      for (const a of this.audios) {
        a.el.currentTime = v.currentTime;
        a.el.play().catch(() => {});
      }
    } catch {
      useEditor.getState().setPlaying(false);
    }
  }

  pause() {
    this.video?.pause();
    for (const a of this.audios) a.el.pause();
    useEditor.getState().setPlaying(false);
  }

  toggle() {
    if (useEditor.getState().playing) this.pause();
    else this.play();
  }

  setRate(r: number) {
    if (this.video && this.video.playbackRate !== r) this.video.playbackRate = r;
    for (const a of this.audios) if (a.el.playbackRate !== r) a.el.playbackRate = r;
  }

  /** Called every animation frame while playing: enforce cuts/speed and keep audio aligned. */
  tick() {
    const v = this.video;
    const st = useEditor.getState();
    const tl = st.edit?.timeline;
    if (!v || !tl) return;
    if (this.pendingSeek !== null && !v.seeking) this.pendingSeek = null;
    if (!st.playing) return;
    const t = v.currentTime;
    const clip = clipAt(tl, t);
    if (!clip) {
      const next = nextPlayable(tl, t);
      if (next === null) {
        this.pause();
        const last = tl.clips[tl.clips.length - 1];
        if (last) this.seek(Math.max(last.start, last.end - 0.02));
        return;
      }
      this.seek(next);
      const c = clipAt(tl, next);
      this.setRate(c?.speed ?? 1);
      return;
    }
    this.setRate(clip.speed);
    for (const a of this.audios) {
      if (!a.el.paused && Math.abs(a.el.currentTime - t) > 0.08) a.el.currentTime = t;
      if (a.el.paused && !v.paused) a.el.play().catch(() => {});
    }
    if (Math.abs(st.time - t) > 1e-4) st.setTime(t);
  }

  /** Step by output-time seconds (frame stepping uses 1/fps). */
  step(deltaOut: number) {
    const tl = useEditor.getState().edit?.timeline;
    if (!tl) return;
    const out = sourceToOutputClamped(tl, this.time);
    this.pause();
    this.seekOutput(Math.max(0, out + deltaOut));
  }
}

export const player = new Player();

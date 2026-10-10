// Typed wrappers around the Rust commands. Types mirror the serde models in
// crates/studio-core (camelCase / kebab-case enums).
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Easing = "linear" | "ease-in-out-cubic" | "ease-in-out-quint" | "ease-out-cubic" | "smoother";
export type Aspect = "auto" | "wide16x9" | "standard4x3" | "square" | "vertical9x16";
export type Quality = "low" | "high" | "best";

export interface Rect { x: number; y: number; w: number; h: number }
export interface Vec2 { x: number; y: number }

export interface Clip { id: string; start: number; end: number; speed: number }
export interface Timeline { sourceDuration: number; clips: Clip[] }

export type ZoomFocus = { mode: "auto" } | { mode: "manual"; x: number; y: number };
export interface ZoomSegment {
  id: string;
  start: number;
  end: number;
  level: number;
  focus: ZoomFocus;
  enabled: boolean;
  auto: boolean;
  anchor?: Vec2;
  anchorTime?: number;
}
export interface ZoomSettings {
  autoZoom: boolean;
  defaultLevel: number;
  minLevel: number;
  maxLevel: number;
  transition: number;
  easing: Easing;
  followSmoothing: number;
  deadZone: number;
  mergeGap: number;
  leadIn: number;
  hold: number;
  minDuration: number;
  minGap: number;
}
export interface CursorSettings {
  visible: boolean;
  scale: number;
  smoothing: number;
  hideWhenIdle: number;
  clickEffect: boolean;
  clickColor: string;
  clickSize: number;
  clickDuration: number;
}
export interface GradientStop { color: string; at: number }
export type Background =
  | { kind: "solid"; color: string }
  | { kind: "gradient"; angle: number; stops: GradientStop[] }
  | { kind: "image"; path: string };
export interface StyleSettings {
  aspect: Aspect;
  background: Background;
  padding: number;
  cornerRadius: number;
  shadowOpacity: number;
  shadowBlur: number;
  shadowOffsetY: number;
}
export interface AudioMix { micVolume: number; systemVolume: number; micMuted: boolean; systemMuted: boolean }
export interface EditState {
  timeline: Timeline;
  zoom: { settings: ZoomSettings; segments: ZoomSegment[] };
  cursor: CursorSettings;
  style: StyleSettings;
  audio: AudioMix;
}
export interface ExportSettings { height: number; fps: number; encoder: string; quality: Quality }
export interface Project {
  version: number;
  id: string;
  name: string;
  createdAt: number;
  source: { kind: "display" | "window" | "imported"; label: string; width: number; height: number; fps: number; scaleFactor: number };
  media: { video: string; mic?: string | null; systemAudio?: string | null; cursor?: string | null; thumbnail?: string | null };
  recording: { duration: number; framesWritten: number; framesDropped: number; encoder: string; pauseCount: number };
  edit: EditState;
  export: ExportSettings;
}
export interface Layout { aspect: number; screen: Rect; sourceWidth: number; sourceHeight: number }
export interface ClickEvent { t: number; x: number; y: number; button: "left" | "right" | "middle"; up?: number }
export interface CursorShape { file: string; width: number; height: number; hotspotX: number; hotspotY: number; kind?: string | null }
export interface PreviewModel {
  layout: Layout;
  rate: number;
  camera: number[];
  cursor: number[];
  clicks: ClickEvent[];
  shapes: CursorShape[];
}
export interface OpenedProject { dir: string; project: Project; preview: PreviewModel; videoPath: string; hasCursor: boolean }
export interface ProjectSummary {
  id: string; name: string; dir: string; createdAt: number; duration: number; width: number; height: number; thumbnail?: string | null;
}
export interface AppSettings {
  outputDir: string | null;
  fps: number;
  encoder: string;
  recordMic: boolean;
  micDevice: string | null;
  recordSystemAudio: boolean;
  countdownSecs: number;
  hideWindowWhileRecording: boolean;
  defaultZoom: ZoomSettings;
}
export interface AppInfo { version: string; ffmpeg?: string | null; ffmpegError?: string | null; captureSupported: boolean; outputDir: string }
export interface EncoderInfo { name: string; label: string; hardware: boolean }
export interface DisplayInfo { id: string; name: string; x: number; y: number; width: number; height: number; scale: number; primary: boolean }
export interface WindowInfo { id: number; title: string; app: string; x: number; y: number; width: number; height: number }
export interface AudioDevice { id: string; name: string; isDefault: boolean }
export type SourceSelection =
  | { kind: "display"; id: string }
  | { kind: "window"; id: number }
  | { kind: "area"; id: string; x: number; y: number; w: number; h: number };
export type RecordingState =
  | { state: "idle" } | { state: "preparing" } | { state: "recording" } | { state: "paused" }
  | { state: "finalizing" } | { state: "completed" } | { state: "failed"; message: string };
export interface SessionStatus {
  state: RecordingState;
  elapsed: number;
  framesWritten: number;
  framesCaptured: number;
  lag: number;
  bytes: number;
  micLevel?: number | null;
  systemLevel?: number | null;
  warnings: string[];
  needsStop: boolean;
  dir: string;
}
export interface Unfinished { dir: string; name: string; createdAt: number; bytes: number }
export interface ExportProgress { frame: number; totalFrames: number; fps: number; etaSecs: number }
export interface ExportResult {
  output: string; duration: number; frames: number; width: number; height: number; encoder: string; sizeBytes: number; elapsedSecs: number;
}

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  listEncoders: () => invoke<EncoderInfo[]>("list_encoders"),
  getSettings: () => invoke<AppSettings>("get_settings"),
  saveSettings: (settings: AppSettings) => invoke<AppSettings>("save_settings", { settings }),
  listSources: () => invoke<{ displays: DisplayInfo[]; windows: WindowInfo[] }>("list_sources"),
  sourceThumbnail: (source: SourceSelection, maxWidth = 480) => invoke<string>("source_thumbnail", { source, maxWidth }),
  listAudioDevices: () => invoke<AudioDevice[]>("list_audio_devices"),
  prepareRecording: (request: { source: SourceSelection; mic: string; systemAudio: boolean; fps?: number }) =>
    invoke<void>("prepare_recording", { request }),
  pendingRecording: () => invoke<{ countdown: number } | null>("pending_recording"),
  cancelPending: () => invoke<void>("cancel_pending"),
  startRecording: () => invoke<void>("start_recording"),
  pauseRecording: () => invoke<void>("pause_recording"),
  resumeRecording: () => invoke<void>("resume_recording"),
  recordingStatus: () => invoke<SessionStatus | null>("recording_status"),
  stopRecording: () => invoke<string>("stop_recording"),
  discardRecording: () => invoke<void>("discard_recording"),
  listProjects: () => invoke<ProjectSummary[]>("list_projects"),
  openProject: (dir: string) => invoke<OpenedProject>("open_project", { dir }),
  computePreview: (dir: string, edit: EditState) => invoke<PreviewModel>("compute_preview", { dir, edit }),
  regenerateZoom: (dir: string, edit: EditState) => invoke<ZoomSegment[]>("regenerate_zoom", { dir, edit }),
  saveProject: (dir: string, edit: EditState, exportSettings?: ExportSettings) =>
    invoke<EditState>("save_project", { dir, edit, export: exportSettings ?? null }),
  renameProject: (dir: string, name: string) => invoke<string>("rename_project", { dir, name }),
  deleteProject: (dir: string) => invoke<void>("delete_project", { dir }),
  importVideo: (path: string) => invoke<string>("import_video", { path }),
  setBackgroundImage: (dir: string, path: string) => invoke<string>("set_background_image", { dir, path }),
  listUnfinished: () => invoke<Unfinished[]>("list_unfinished"),
  recoverRecording: (dir: string) => invoke<string>("recover_recording", { dir }),
  discardUnfinished: (dir: string) => invoke<void>("discard_unfinished", { dir }),
  startExport: (dir: string, output: string, settings: ExportSettings) =>
    invoke<string>("start_export", { dir, output, settings }),
  cancelExport: (job: string) => invoke<void>("cancel_export", { job }),
  revealPath: (path: string) => invoke<void>("reveal_path", { path }),
  openPath: (path: string) => invoke<void>("open_path", { path }),
};

export function on<T>(event: string, cb: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(event, (e) => cb(e.payload));
}

export function fileUrl(path: string): string {
  return convertFileSrc(path);
}

export function joinPath(dir: string, rel: string): string {
  const sep = dir.includes("\\") ? "\\" : "/";
  return dir.replace(/[\\/]+$/, "") + sep + rel.replace(/[\\/]/g, sep);
}

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}

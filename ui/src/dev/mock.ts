// Development-only IPC mocks so the UI can be rendered and screenshotted in a
// plain browser (`/?mock&page=...`). Never included in production builds.
// Fixtures live in ui/dev-fixtures (git-ignored, dev-server only; generated with
// `studio-cli preview-json <project>` from a real recording).
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

export async function installMocks() {
  const opened = await fetch("/__mock/opened.json").then((r) => r.json());
  const dir: string = opened.dir;
  const project = opened.project;
  mockWindows("main");
  (window as unknown as { __TAURI_INTERNALS__: Record<string, unknown> }).__TAURI_INTERNALS__.convertFileSrc = (p: string) => {
    const norm = p.replace(/\\/g, "/");
    const base = dir.replace(/\\/g, "/");
    if (norm.startsWith(base)) return "/__mock" + norm.slice(base.length);
    return "/__mock/" + norm.split("/").pop();
  };
  const settings = {
    outputDir: null, fps: 60, encoder: "auto", recordMic: false, micDevice: null, recordSystemAudio: true,
    countdownSecs: 3, hideWindowWhileRecording: true, defaultZoom: project.edit.zoom.settings,
  };
  const projects = [0, 1, 2, 3, 4].map((i) => ({
    id: `p${i}`,
    name: ["Product demo", "Onboarding walkthrough", "Bug repro: export dialog", "Weekly update", "Feature tour"][i],
    dir: i === 0 ? dir : `${dir}-${i}`,
    createdAt: Date.now() - i * 26 * 3600 * 1000,
    duration: [12.1, 95.4, 34.0, 241.2, 63.5][i],
    width: 1920,
    height: 1080,
    thumbnail: `${dir}\\thumbnail.jpg`,
  }));
  mockIPC(
    (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      switch (cmd) {
        case "app_info":
          return { version: "0.1.0", ffmpeg: "C:\\ffmpeg\\bin\\ffmpeg.exe", ffmpegError: null, captureSupported: true, outputDir: "C:\\Users\\you\\Videos\\Glide" };
        case "list_encoders":
          return [
            { name: "h264_amf", label: "AMD AMF (hardware)", hardware: true },
            { name: "h264_mf", label: "Windows Media Foundation", hardware: false },
            { name: "libx264", label: "x264 (software)", hardware: false },
          ];
        case "get_settings":
        case "save_settings":
          return a.settings ?? settings;
        case "list_projects":
          return new URLSearchParams(location.search).has("empty") ? [] : projects;
        case "list_sources":
          return {
            displays: [{ id: "\\\\.\\DISPLAY1", name: "Primary Display", x: 0, y: 0, width: 1920, height: 1080, scale: 1, primary: true }],
            windows: [
              { id: 101, title: "Glide – README.md", app: "Code", x: 0, y: 0, width: 1600, height: 900, primary: false },
              { id: 102, title: "Inbox", app: "Outlook", x: 0, y: 0, width: 1400, height: 950 },
              { id: 103, title: "Figma – Landing page", app: "Figma", x: 0, y: 0, width: 1800, height: 1000 },
            ],
          };
        case "source_thumbnail":
          return "/__mock/thumbnail.jpg";
        case "list_audio_devices":
          return [{ id: "mic1", name: "Microphone (USB Audio)", isDefault: true }];
        case "open_project":
          return opened;
        case "compute_preview":
          return opened.preview;
        case "regenerate_zoom":
          return project.edit.zoom.segments;
        case "save_project":
          return a.edit;
        case "rename_project":
          return a.name;
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );
}

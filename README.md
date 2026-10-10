# Glide

A free, open-source screen recorder for **Windows 10/11** with automatic zoom,
re-rendered cursor, and a non-destructive editor. It follows the workflow of
Screen Studio for macOS. Glide is an independent project that uses none of
Screen Studio's code, assets, or branding. "Glide" is a working name.

- **Record** a display, a single window or an area (Windows Graphics Capture), plus
  microphone and system audio (WASAPI). The cursor and clicks are recorded as
  metadata, not burned into the video.
- **Automatic zoom**: clicks are grouped into interactions, and the camera
  eases in, follows the cursor with a dead zone and critically damped
  smoothing, then eases back out. Every zoom is editable on the timeline.
- **Edit**: trim, cut, change clip speed, add, move, or resize zooms, pin a
  zoom to a fixed point, restyle the cursor (size, smoothing, idle hiding,
  click ripples), and set the background, padding, corners, shadow, and
  aspect ratio. Undo/redo works throughout, and projects autosave.
- **Shortcuts that work anywhere**: Ctrl+Alt+R stops a recording, Ctrl+Alt+P
  pauses or resumes it.
- **Crash-safe**: if Glide or Windows dies mid-recording, the dashboard
  offers to recover everything captured up to that point.
- **Export** H.264 MP4 at 720p to 4K and 30/60 fps. Hardware encoding (AMD
  AMF, NVIDIA NVENC, Intel QSV) is used when it actually works, with a
  software fallback. You get a progress bar, can cancel, and every output is
  validated.

Everything runs locally. There is no telemetry and nothing is uploaded.

## Requirements

| | |
|---|---|
| OS | Windows 10 version 2004 or later (Windows 11 recommended) |
| FFmpeg | `ffmpeg.exe` and `ffprobe.exe`, either on `PATH`, next to `glide.exe`, or in a folder named by the `STUDIO_FFMPEG_DIR` environment variable. FFmpeg 6 or later; tested with 9.0.2. `winget install Gyan.FFmpeg` works. |
| GPU | Any DirectX 11 GPU. Hardware encoders are optional. |

## Building

Prerequisites: Rust (stable, MSVC toolchain), Node.js 20 or later, Visual
Studio Build Tools with the "Desktop development with C++" workload, and
WebView2 (preinstalled on Windows 11).

```bash
npm install
```

Run the app in development mode (Vite + Rust, with hot reload for the UI):

```bash
npx tauri dev
```

Build a release installer (NSIS) into `target/release/bundle/nsis/`:

```bash
npx tauri build
```

### Tests and checks

```bash
cargo test --workspace
```

```bash
cargo clippy --workspace --all-targets
```

```bash
npm test
```

```bash
npm run typecheck
```

### Headless CLI

`studio-cli` drives the same engine without the UI. It's useful for smoke
tests and for checking a machine's capabilities.

```bash
cargo run --release -p studio-engine --bin studio-cli -- probe
```

```bash
cargo run --release -p studio-engine --bin studio-cli -- record --seconds 10 --mic
```

```bash
cargo run --release -p studio-engine --bin studio-cli -- export "<project folder>" --height 1080 --fps 60
```

### UI development without the backend

In dev mode, `http://localhost:5173/?mock` renders the UI in an ordinary
browser with mocked IPC (`ui/src/dev/mock.ts`). The `&page=editor`,
`&page=record`, `&page=settings` and `&empty` parameters select a screen.
Fixtures go in `ui/dev-fixtures/`, which is git-ignored and served only by
the dev server, never bundled. Generate them with
`studio-cli preview-json <project>`.

## Repository layout

```
crates/studio-core     platform-independent engine (camera, timeline, project, render, export) + unit tests
crates/studio-engine   Windows capture, input hook, audio, recording session, studio-cli
src-tauri              Tauri 2 shell: commands, windows, permissions
ui                     React + TypeScript UI
docs                   architecture, UI reference checklist, status & limitations
```

Further reading:

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): design, threading, timing, and the zoom algorithm.
- [docs/STATUS.md](docs/STATUS.md): what's done, what isn't, and known limitations.
- [docs/UI_REFERENCE.md](docs/UI_REFERENCE.md): the UI reference checklist.

## Permissions and privacy

- **Screen capture** uses Windows Graphics Capture. On Windows 11, Glide asks
  Windows to hide the yellow capture border where that's allowed. The
  floating recording controls are excluded from capture
  (`WDA_EXCLUDEFROMCAPTURE`).
- **Input tracking** uses a low-level mouse hook (`WH_MOUSE_LL`) that is
  active only while recording. It records cursor position and mouse-button
  presses only, never keystrokes. Clicks on Glide's own windows are ignored.
- **Audio** comes from the selected microphone and the default output device
  in loopback mode.
- **Files**: recordings stay in the folder you choose (default
  `Videos\Glide`). Deleting a project moves it to the Recycle Bin.

## Licence

Glide is dual-licensed under MIT or Apache-2.0, at your option. See
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for dependency licences.
FFmpeg is not bundled. It's invoked as a separate process, and its licence
depends on the build you install.

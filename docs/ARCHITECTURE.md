# Architecture

Glide is a native Windows 10/11 screen recorder and editor inspired by the
Screen Studio workflow. Rust owns capture, timing, input tracking, the camera
engine, rendering and export. A Tauri 2 shell hosts a React/TypeScript UI that
only talks to Rust through typed commands and events.

## Repository layout

```
crates/
  studio-core/      Platform-independent engine. No Windows APIs. Fully unit tested.
    camera/         Auto-zoom segment generation + camera trajectory (springs, easing, clamping)
    cursor/         Cursor event model, smoothing, click effects
    timeline/       Non-destructive edit model: trim, speed segments, time mapping
    layout          Canvas/background/padding geometry shared by preview and export
    project/        Project schema (project.json, cursor.json), load/save, path validation
    settings        App settings model + persistence
    recording       Recording state machine (Idle → Preparing → Recording ⇄ Paused → Finalizing → Completed | Failed)
    render/         CPU compositor (background, shadow, rounded frame, zoom, cursor, click ripples)
    media/          FFmpeg discovery, encoder probing, encoder/decoder subprocess wrappers
    export          Export job: decode → time-map → render → encode + audio graph, progress & cancel
  studio-engine/    Windows-specific engine (windows crate, cpal)
    capture/        Windows Graphics Capture (monitor & window), D3D11 readback, frame pacing
    input/          WH_MOUSE_LL hook, cursor shape/visibility polling, cursor bitmap extraction
    audio/          WASAPI mic + loopback via cpal, clock-aligned WAV writers
    sources         Display & window enumeration
    session         Recording session orchestration (threads, channels, finalize, recovery)
    bin/studio-cli  Headless CLI (record / export / probe) used for smoke tests and debugging
src-tauri/          Tauri 2 app shell: commands, events, window management, permissions
ui/                 React + TypeScript UI (Vite)
docs/               Architecture, UI reference checklist, limitations
```

## Process & threading model

```
UI (WebView2) ──invoke──▶ Tauri commands ──▶ studio-engine::Session (owned by app state)
                ◀─events── progress / state / levels

Recording session threads:
  WGC FrameArrived (free-threaded pool) ─▶ latest-frame slot (Mutex+Condvar, buffer pool)
  Video pacer thread  ── pulls latest frame at fixed fps on the recording clock ─▶ ffmpeg stdin (bounded by pipe)
  Input hook thread   ── WH_MOUSE_LL + message loop ─▶ bounded channel ─▶ input writer
  Cursor poll thread  ── GetCursorInfo @ 60 Hz (shape/visibility/window bounds)
  cpal callbacks (mic, loopback) ─▶ bounded channel ─▶ WAV writer threads (gap fill / drift correction)
```

No capture, encoding or file I/O ever runs on the UI thread. All queues are
bounded; the video path keeps only the *latest* frame, so a slow encoder causes
dropped (counted) frames rather than unbounded memory growth.

## Timing model

A single monotonic clock (`Instant`, QPC on Windows) is the master. *Recording
time* = wall time since start − accumulated pause time.

* **Video** is written as constant frame rate: frame *n* represents recording
  time *n / fps*. The pacer emits the most recent captured frame at each tick
  (WGC only delivers frames when content changes; duplicates are cheap for
  H.264). If the writer falls behind it skips ticks, so video never drifts from
  the clock.
* **Audio** callbacks are aligned to the same clock. Each writer tracks
  `samples_written` vs `recording_time × rate`; gaps (WASAPI loopback delivers
  nothing during silence) are filled with silence and over-runs are dropped
  beyond a 40 ms tolerance. This also corrects device clock drift on long takes.
* **Input** events are stamped with recording time when the hook fires.
* **Pause**: the pacer stops emitting, audio is discarded, input is ignored, and
  pause duration is excluded from recording time — so all tracks stay aligned
  with no post-processing.

## Media files

A project is a folder:

```
<Recordings>/<name>/
  project.json     schema-versioned edit state (non-destructive)
  screen.mp4       CFR H.264, cursor NOT burned in, GOP = 1 s for fast scrubbing
  mic.wav          optional, 48 kHz, aligned to recording time 0
  system.wav       optional, 48 kHz, aligned to recording time 0
  cursor.json      cursor samples, clicks, shapes, capture bounds
  cursors/*.png    extracted cursor bitmaps (+ hotspot in cursor.json)
  thumbnail.jpg
```

During recording video goes to `screen.partial.mp4` as fragmented MP4 so a crash
leaves a playable file; finalize remuxes (no re-encode) to `screen.mp4` with
`+faststart`. Paths in `project.json` are relative and validated to stay inside
the project folder.

## Camera / auto-zoom engine (studio-core::camera)

Coordinates are **normalized canvas space** (0..1 × 0..1 of the output frame),
so preview and export at any resolution use the same trajectory.

1. **Collect** cursor samples and clicks (source pixels, recording time).
2. **Filter**: drop clicks during drags/double-click bursts (< 250 ms apart and
   < 8 px), ignore clicks outside the captured area.
3. **Cluster** clicks into interaction groups: consecutive clicks join a group if
   they occur within `merge_gap` seconds *and* the group's bounding box still fits
   a viewport at the configured zoom.
4. **Segments**: each group → `ZoomSegment{start = first − lead_in,
   end = last + hold}`; segments closer than `2 × transition + min_gap` merge; short
   segments are extended to `min_duration`; all clamped to the recording.
   Segments are stored in the project and are fully editable (move, resize, level,
   auto-follow vs. manual focus, disable, delete, add).
5. **Zoom curve**: per segment, zoom eases in over `transition` seconds (cubic
   in-out by default, configurable), holds, and eases out. Overlapping
   contributions take the max, so it is continuous.
6. **Focus**: auto segments follow the cursor using a *dead-zone* (the camera
   only moves when the cursor leaves the inner 55 % of the viewport); manual
   segments target a fixed point. The cursor is low-passed (0.12 s) before the
   dead zone, so sensor jitter and the small corrective movements people make
   when landing on a target don't nudge the camera after the main movement.
   The camera then follows its target through three critically damped springs
   in series (`SmoothFollow`, a 6th-order critically damped filter, total lag
   `follow_smoothing`). This gives continuous acceleration and jerk, a
   near-symmetric bell-shaped velocity profile (gentle start, natural
   deceleration), and monotonic motion with no overshoot or bounce. Each step
   is solved exactly for a target ramping linearly across the step
   (`smooth_damp_ramp`), so the 60 Hz preview and 120 Hz export trace the same
   path. Once the remaining motion is below ~0.01 output px, the camera comes
   to rest *exactly*, with no endless sub-pixel creep that would shimmer
   through the resampler.
7. **Bounds**: the visible window (1/zoom of the canvas) is clamped inside a
   region that blends from the full canvas (zoom 1) to the screen rectangle
   (once the viewport fits inside it), so content is never stretched, the
   background never shows at high zoom, and zoom-out returns to centre smoothly.
8. The trajectory is simulated once (60 Hz for the preview, 120 Hz for
   export) and cached as keyframes `(t, cx, cy, zoom)`, sampled with linear
   interpolation. The preview draws at a smoothed playback clock rather than the
   raw `video.currentTime`, which advances in uneven steps.

Tests cover segment generation, merging, bounds, continuity (bounded velocity and
acceleration → no jitter), smooth deceleration and exact settling for left↔right
pans, preview/export agreement, determinism and the disabled case.
`cargo run --release -p studio-core --example camera_motion` prints pan metrics
(see `docs/QUALITY.md`).

## Rendering

* **Preview** (UI): the browser decodes `screen.mp4` (hardware H.264 in
  WebView2). The scene (background, shadowed rounded frame, video, cursor sprite,
  click ripples) is a DOM subtree transformed with a CSS matrix computed from the
  Rust-generated camera keyframes. Layout values come from Rust, so preview and
  export share geometry.
* **Export** (Rust): `ffmpeg` decodes to RGBA on stdout → the CPU compositor
  (rayon, bilinear sampling, anti-aliased rounded corners, pre-rendered
  background+shadow layer) → `ffmpeg` encodes from stdin. Frames stream one at a
  time; memory is O(1 frame). Timeline mapping (trim/speed) is applied by
  reading/skipping/repeating decoded frames; audio uses an equivalent
  `atrim`/`atempo`/`concat` filter graph plus per-track volume and `amix`.
* A GPU (wgpu) compositor can replace the CPU one behind the same `Compositor`
  API; CPU was chosen first for determinism and zero driver risk.

## Encoders

`media::probe_encoders` *runs* each candidate (`h264_amf`, `h264_nvenc`,
`h264_qsv`, `h264_mf`, `libx264`) on a 1-frame test clip and keeps only the ones
that succeed. A listed encoder is never assumed to work (e.g. `h264_nvenc` is
compiled into common FFmpeg builds but fails without an NVIDIA driver).
Preference: user choice → first working hardware encoder → `libx264`.

## Security

* No shell is ever invoked; FFmpeg is spawned with an argument vector.
* Imported/opened paths are canonicalized; project media paths must resolve
  inside the project folder.
* The Tauri asset protocol scope is granted at runtime only for open project
  folders. Capabilities only enable the commands/plugins the UI uses.
* Nothing is uploaded; there is no telemetry.

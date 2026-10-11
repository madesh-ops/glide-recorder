# Status

Status of Glide v0.1.0 as of 2026-10-10. "Verified" means it was exercised
for real on the development machine (Windows 11, Ryzen 5 5500GT, Radeon
iGPU, FFmpeg 9.0.2), not just compiled.

## Definition of done

| Requirement | Status | Evidence |
|---|---|---|
| App builds and launches on Windows | ✅ | `npx tauri dev` launches the app; `npx tauri build` produces the NSIS installer |
| Select a display or window and record it | ✅ Engine verified · UI built | `studio-cli record` (display); UI source picker with live WGC thumbnails |
| Audio and video synchronized | ✅ with caveat | All track lengths equal to the ms (3 min test); a flash-and-beep test measured +87 ms, which includes the test player's own output latency (see Limitations) |
| Cursor and click metadata captured | ✅ Verified | Harness run: 255 samples, 3 clicks, 2 cursor shapes extracted with hotspots |
| Automatic zooms generated and adjustable | ✅ Verified | 2 segments generated from 3 clicks; timeline move, resize, add, remove; level and focus editing |
| Effects appear in the exported video | ✅ Verified | Exported frames show the 2× zoom on the clicked target, the rendered cursor, and click ripples |
| Trim recordings and save projects | ✅ | Clip trim, cut, and speed; integration test exports a trimmed, cut, 2× project |
| Reopen projects without losing editing state | ✅ Tested | `save_and_reload_preserves_edit_state` (exact float round-trip) |
| MP4 export works reliably | ✅ Tested | H.264 + AAC, validated with ffprobe; cancellation cleans up; AMF and x264 verified |
| No unbounded memory growth on long recordings | ✅ Verified | 3 min at 1080p60: private memory flat at 41–42 MB, 10,814/10,814 frames |
| Automated tests for core camera and recording logic | ✅ | 54 unit + 2 integration tests (Rust), 10 tests (UI) |
| README, build instructions, dependency notices | ✅ | README.md, THIRD_PARTY_NOTICES.md, docs/ |

## Completed features

- **Crash recovery** (verified by killing the recorder mid-take): a manifest
  is written at start, cursor data streams to an append-only `cursor.log`,
  and recovery remuxes the fragmented MP4, repairs WAV headers left
  unfinalized, and replays the cursor log. The dashboard shows a
  Recover / Discard banner for interrupted recordings.
- **Global shortcuts**: Ctrl+Alt+R starts a recording with the last-used source
  or stops the current one from anywhere (or opens
  New recording) and Ctrl+Alt+P pauses/resumes.
- **Area recording**: drag-select or pick a preset on a live display
  thumbnail. The region is cropped during GPU readback, and its alignment was
  verified against a cropped full-display recording (43 dB PSNR vs 16 dB
  for a 2 px offset).

- **Recording engine**: WGC display and window capture with the cursor
  excluded and the capture border disabled where allowed. GPU→CPU readback
  into a single-slot mailbox with buffer reuse. A CFR pacer drives the
  recording clock. FFmpeg writes fragmented MP4 (crash-tolerant), then remuxes
  with faststart. The state machine handles pause/resume and discard,
  surfaces encoder errors and source loss, and auto-stops.
- **Encoder detection**: each candidate is test-encoded in parallel
  (0.2 s). On this machine `h264_amf`, `h264_mf` and `libx264` work;
  `h264_nvenc` and `h264_qsv` are correctly rejected.
- **Audio**: WASAPI mic and loopback through cpal. Samples are placed by
  device capture timestamps, gaps are filled, drift is corrected, and each
  file is padded to the exact duration. Live level meters.
- **Input**: `WH_MOUSE_LL` click hook and a 120 Hz cursor poll. Cursor
  bitmaps are extracted (colour, legacy-mask, and monochrome cursors), and
  standard cursor kinds are identified. Clicks on Glide's own windows are
  ignored. Window capture tracks window movement.
- **Camera engine**: click filtering, time grouping, fit-to-interaction zoom
  level, segment merging and minimum durations, eased zoom curves, a
  dead-zone follow on a low-passed cursor through a 6th-order critically
  damped filter that comes to exact rest, zoom-toward-point
  interpolation, and bounds that keep content unstretched with no background
  at full zoom. Tests check jitter, continuity, bounds, and determinism.
- **Editor**: live preview that matches the export (Rust-computed tracks,
  CSS transform), synced mic/system audio preview, timeline (scrub, trim,
  split, cut, speed, zoom add/move/resize/disable/remove, hover ghost,
  context menu), background/cursor/zoom/audio/clip panels, manual focus
  picker, undo/redo with gesture coalescing, autosave, rename, keyboard
  shortcuts.
- **Export**: streaming decode → CPU compositor (rayon) → encode, about
  53 fps at 1080p on this machine. Audio follows the timeline through an
  `atrim`/`atempo`/`concat`/`amix` graph. Progress, cancellation, and output
  validation (duration, size, audio), with a temporary file renamed only on
  success.
- **Dashboard**: project grid with thumbnails, import video (transcoded for
  scrubbing), move to the Recycle Bin, empty state.
- **Settings**: output folder, fps, countdown, hide-while-recording, encoder,
  and zoom defaults. The settings file is written atomically and corrupt
  files are recovered.
- **Security**: no shell invocation, path validation (project containment,
  export target checks, `.mp4`-only open), minimal Tauri capabilities,
  asset-protocol scope granted per folder, no telemetry.

## Incomplete or not yet implemented

- Webcam and iOS-device recording.
- Fullscreen drag-to-select overlay for area recording. Today the area is
  chosen on a thumbnail inside the main window.
- Configurable shortcut keys (they are fixed to Ctrl+Alt+R / Ctrl+Alt+P).
- GPU (wgpu) compositor; export uses the CPU compositor.
- Motion blur, captions/transcripts, masks, GIF export, presets, command
  menu.
- High-resolution replacement of standard cursors when scaled up. Recorded
  bitmaps are scaled bilinearly; the kind is stored so this can be added.
- Audio gain above 100% is heard only in the export; preview caps at 100%.

## Blocked or unverified

- **Interactive end-to-end run of the GUI.** Screen-control access to the
  running Glide window was declined, so UI flows were verified two other
  ways: (a) rendering every screen in headless Chrome with Tauri's IPC mocks
  and real recording data, and (b) exercising the same Rust engine through
  `studio-cli`. Clicking through record → edit → export in the real window
  still needs a manual pass.
- **NVENC and QSV** paths are untested: there's no NVIDIA or Intel GPU here.
  They are probed and rejected cleanly when unavailable.
- **Multi-monitor and mixed-DPI** setups are untested: one 1920×1080 display
  at 100%. Coordinates are per-monitor-v2 physical pixels throughout.

## Known limitations

- **A/V offset**: in the flash-and-beep test, the tone landed 87 ms after the
  flash. The test played its tone through `System.Media.SoundPlayer`, whose
  output latency is part of that figure, so the true pipeline offset is
  smaller but not isolated. No drift accumulates (verified over 3 min).
- **Window capture with resizing**: output size is fixed at the window's
  initial size. A larger window is cropped and a smaller one leaves black
  padding at the bottom/right.
- **Monochrome cursors** that invert screen pixels (some I-beams) are drawn
  black, since inversion can't be reproduced over a re-rendered background.
- **Hidden cursor**: when Windows hides the pointer (for example while
  typing), it stays hidden in the recording.
- **Export speed** depends on the CPU at high resolutions. 4K60 export runs
  slower than real time on a 6-core CPU.

# Video quality audit

Investigation of "1080p recordings look a little blurry". Measured on
2026-10-10 on a Ryzen 5 5500GT with Radeon iGPU and FFmpeg 9.0.2.

## Method

`tools/quality/` reproduces every stage with real code paths:

| Script | Purpose |
|---|---|
| `make_testclip.py <dir>` | Lossless 1920×1080@60 reference: Segoe UI text at 12–16 px, coloured Consolas "code" on a dark panel, 1 px rules. Static for 2 s, scrolls for 2 s, static for 2 s. Synthetic, so it contains no private screen content. |
| `bench_encode.py <dir> [configs…]` | Recording stage: raw BGRA → FFmpeg with the app's arguments. Reports SSIM and PSNR in RGB (chroma losses count), Y-SSIM, bitrate, and speed. |
| `bench_export.py <dir> <repo> <recording.mp4> <tag>` | Export stage: runs the real export (`cargo run --release -p studio-core --example export_quality`) and compares it with an ideal Lanczos resample of the lossless reference. Also reports a sharpness index (mean Laplacian of luma relative to the ideal). |

Frames are paired by index (`setpts=N/60/TB`). Pairing by timestamp
mismatched MKV/MP4 rounding during motion and produced a false 23 dB score
for every configuration.

## Findings, by pipeline stage

| # | Stage | Finding | Impact |
|---|---|---|---|
| 1 | Capture (WGC, BGRA, native pixels) | Lossless. No scaling, and physical pixels at any DPI. | none |
| 2 | RGB → YUV 4:2:0 conversion | Default swscale settings, and 4:2:0 halves chroma resolution (coloured text fringes). | small; 4:2:0 is a hard ceiling (see Limits) |
| 3 | Recording encode | AMF *speed* preset at **QP 22**. Text edges ring and blur slightly: 37.5 dB. | moderate |
| 4 | Export decode (YUV → RGB) | Default swscale chroma upsampling. | small |
| 5 | **Export compositor** | **Bilinear sampling.** Zoomed text reached only **72 %** of ideal sharpness (2× zoom: 30.7 dB). Padding (downscale to 84 %) reached 86 %. | **large, the main cause** |
| 6 | **Export size** | A 1080p export of a 1080p recording with the default 8 % padding shrinks the screen to 84 %, so text *cannot* be pixel-sharp at that size. | **large** |
| 7 | Export encode | AMF QP 22 / x264 CRF 21 for the final file. | moderate |
| 8 | Pixel output | Float → u8 truncated instead of rounded (−0.5 level bias). | tiny |

## Changes

1. **Separable Catmull-Rom resampler** (`render.rs`). The camera only scales
   and translates, so resampling is separable, with per-frame precomputed
   taps for rows and columns. Upscaling uses Catmull-Rom. Downscaling widens
   the kernel by the scale factor, which low-pass filters it properly
   (no aliasing). It is also slightly *faster* than the old per-pixel
   bilinear path (5.2 s vs 6.0 s for the 6 s test export).
2. **Recording encode**: AMF *transcoding/quality* at QP 16 when the pixel
   rate is ≤ 1080p60 (measured at 130 fps, a 2× real-time margin), and
   *low-latency/speed* at QP 16 above that. x264 uses veryfast/superfast at
   CRF 16. Keeping `zerolatency` measured *sharper* on static text than
   mb-tree lookahead (38.5 vs 37.9 dB). NVENC/QSV get CQ 16; Media
   Foundation gets 0.15 bpp.
3. **Export encode ladder**: High = x264 slow CRF 17 / hardware QP 18;
   Best = CRF 14 / QP 15; Low = CRF 23 / QP 24.
4. **swscale `lanczos+accurate_rnd+full_chroma_int`** for both conversions.
5. **Sharp export size**: new projects default to the smallest standard
   height at which the recording isn't shrunk
   (`Layout::recommended_export_height`, 1440p for 1080p with padding). The
   export dialog warns when the chosen size shrinks text and names the
   resolution that keeps it pixel-sharp.
6. Rounding instead of truncation in the compositor.

## Results (same 6 s reference clip)

Recording stage (RGB PSNR / bitrate / encode speed at 1080p60):

| Configuration | PSNR | Mbit/s | fps |
|---|---|---|---|
| Before: AMF speed QP 22 | 37.50 dB | 3.9 | 211 |
| **After: AMF quality QP 16 + Lanczos** | **38.54 dB** | 5.2 | 129 |
| x264 veryfast CRF 16 (fallback) | 38.49 dB | 4.6 | 196 |

End to end (recording → export, frame at t = 1 s):

| Case | Before | After |
|---|---|---|
| 1:1 (no padding, no zoom) | 38.32 dB | **39.51 dB** |
| 2× zoom | 30.65 dB, sharpness 72 % | **36.71 dB, sharpness 96 %** |
| 8 % padding at 1080p | sharpness 86 % | **92 %** |
| 8 % padding at 1440p (new default) | n/a | text at native size, 31.4 dB* |

\* The padded cases have fractional-pixel offsets, so their PSNR against
the ideal is limited by sub-pixel alignment rather than blur. Use the
sharpness figure for those.

A live 1080p60 desktop recording with the new settings kept 0 ms encoder
lag (610/610 frames in 10.17 s) at about 7 Mbit/s.

## Camera motion

The follow had a "rough, jerky" arrival when panning left or right. `cargo run
--release -p studio-core --example camera_motion` simulates a 2× zoomed pan
with human-like strokes. Each stroke is a minimum-jerk reach that overshoots by
4 % and is followed by a corrective sub-movement, with ±1 px jitter at 125 Hz.
Motion is measured in output pixels of a 1440p export.

| Metric (left→right / right→left) | Before | After |
|---|---|---|
| Peak jerk (px/s³) | 151 k / 151 k | 104 k / 109 k |
| Time to rest after the cursor stops | 1.92 s / 1.91 s | **1.30 s / 1.28 s** |
| Sub-pixel crawl (0.01–0.5 px/frame) | 0.99 s / 0.99 s | 0.42 s / 0.43 s |
| Final step into rest | exponential, never exactly 0 | 0.01 px, then exactly still |
| Direction reversals | 0 | 0 |
| Preview (60 Hz) vs export (120 Hz) path | ≈ 1 frame apart (58 px mid-pan) | < 2 px |

Causes and changes:

1. **Single spring, exponential tail.** One critically damped spring makes
   a lopsided velocity curve: a quick kick, then a long creeping tail. The
   camera crawled for about a second at sub-pixel speeds, which shimmers text
   through the resampler and reads as "not quite stopping". It now uses three
   springs in series with the same total lag. The velocity curve is a
   near-symmetric bell (close to minimum-jerk), acceleration and jerk are
   continuous, and motion is monotonic with no overshoot.
2. **No defined rest.** A rest threshold (≈0.01 output px) stops the camera
   exactly. The final step is invisible.
3. **Overshoot clamp.** The old `smooth_damp` zeroed velocity whenever a step
   would pass the target, which was a hard stop. It is now the exact analytic
   solution, with velocity always continuous.
4. **Raw cursor into the dead zone.** Jitter and the small corrections at the
   end of a reach moved the target in late little nudges. The cursor is now
   low-passed (0.12 s) before the dead zone.
5. **Rate dependence.** Holding a moving target for each step leads by half a
   step per filter stage, so the 60 Hz preview ran a frame ahead of the
   export. Each stage is now solved exactly for a linearly ramping target. The
   displayed cursor uses the same spring.
6. **Preview clock.** The editor drew at `video.currentTime`, which advances
   in uneven steps. It now draws at a smoothed clock that extrapolates from
   the frame timestamp and eases towards the media clock.

Peak speed rises (2.7 → 3.6 k px/s) because the same distance is covered in
a more compact, symmetric curve instead of a long tail. Raise
**Follow smoothness** in the zoom panel for slower, softer pans.

## Limits and trade-offs

- **4:2:0 chroma.** H.264 as played by browsers and most players stores
  colour at half resolution. Small coloured text (syntax highlighting,
  ClearType fringes) loses some colour crispness, and every tested setting
  levels off near 38.7 dB RGB PSNR for this reason. 4:4:4 H.264 or HEVC
  would fix it but would not play in WebView2 or many players.
- **File size.** Recordings are about 30–60 % larger (masters at QP 16).
  Exports with fast *sub-pixel* motion (scrolling at a non-1:1 scale) cost
  more bits, especially with AMF, whose motion search is weaker than x264's.
- **Editor preview.** The preview is drawn at window size by the browser,
  so it is softer than the export. Judge sharpness from the exported file.

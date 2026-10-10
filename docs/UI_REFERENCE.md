# UI reference checklist

Sources: the public Screen Studio website (screen.studio) and its user guide
(screen.studio/guide), especially the "Adding & Editing zooms" pages and
their editor screenshots. Glide copies no assets. Every layout and visual
choice below was rebuilt from scratch and adapted to Windows conventions.

## Observed in the reference → Glide

| Reference behaviour or element | Glide | Status |
|---|---|---|
| Dark editor shell with low-contrast surfaces and a single violet accent (≈ `#6046F5`) | Tokens in `ui/src/styles/theme.css`, accent `#6B52F5` | Done |
| Centre preview: recording framed on a wallpaper with padding, rounded corners and a soft shadow | `Preview.tsx` (CSS) and `render.rs` (export) share the same geometry | Done |
| Vertical icon rail at the far right that switches the settings panel | `Rail` + `SidePanel` (Background, Cursor, Zoom, Audio, Clip) | Done |
| Timeline toolbar: track selector · transport (time / skip / play / skip / duration) · scissors · timeline scale slider | `Timeline.tsx` toolbar: undo/redo · transport · + Zoom / Split · scale slider | Done (undo/redo replaces the track selector) |
| Amber clip track with a "Clip" label and speed | `.clip` gradient `#D99A2B → #B47C1A`, label "Clip · 1× · duration" | Done |
| Violet zoom blocks labelled "Zoom", level and "Auto" | `.seg` violet gradient, "Zoom / 2.0× · Auto" (or "Manual") | Done |
| Click on an empty spot of the zoom track to add a zoom | Hover ghost "+ Add zoom", then click | Done |
| Drag zoom edges to change duration | Edge handles; dragging the body moves the zoom | Done |
| Click a zoom to open its settings (level, auto vs manual) | Selection opens the Zoom panel: level, Follow cursor / Fixed point, focus picker | Done |
| Right-click a zoom to Disable or Remove it | Context menu: Disable/Enable, Remove | Done |
| Automatic zooms generated from clicks | `camera::generate_segments`, regenerated on demand, edited zooms kept | Done |
| Recording bar: Display / Window / Area / Device · camera · mic · system audio · record | New-recording page: Display/Window tabs with live thumbnails, mic picker, system-audio toggle, fps, Start | Partial (no Area, Device, or webcam) |
| Countdown and recording controls (pause/resume, stop, restart) | Floating, capture-excluded bar: countdown, timer, size, level meters, pause, stop, discard | Done (no "restart") |
| Smooth cursor, cursor size, click effects, hide when idle | Cursor panel | Done |
| Background presets, solid colours, custom image | 12 original gradients, 10 colours plus a picker, image import (converted to PNG) | Done |
| Aspect ratio (wide, square, vertical) | Auto, 16:9, 4:3, 1:1, 9:16 | Done |
| Export: MP4 up to 4K at 60 fps, presets | Resolution, fps, quality, encoder, path, progress, cancel, open/reveal | Done |
| Command menu, captions, transcripts, masks, camera layouts, motion blur, shareable links | Not implemented | Out of scope for v0.1 |

## Window and layout metrics used

- Main window 1320×840 by default (minimum 1040×680), with a frameless
  custom title bar 44 px tall. Windows-style caption buttons sit on the
  right.
- Home: 232 px sidebar holding the primary "New recording" button, nav, and
  recordings folder link. The content area has 28/32 px padding.
- Editor: preview fills the remaining space with a 40 px safe margin. The
  settings panel is 300 px wide and the icon rail 56 px.
- Timeline is 236 px tall: a 46 px toolbar, a 28 px ruler, and two 56 px
  tracks holding 40 px blocks with 8 px corner radius.
- Type: Segoe UI Variable (Text/Display). Body 13 px; labels 12 px at weight
  500; headings 15–22 px at weight 600.
- Controls are 30 px tall (26 px small, 38 px large) with 6–8 px radii.
  Toggles are 32×18.

## Approximations

- Screen Studio's wallpapers are macOS system images. Glide ships its own
  gradients instead.
- The zoom easing feel was approximated (cubic in/out at 0.8 s by default)
  and can be configured. The real curve could not be measured from public
  material.
- The macOS recording bar floats over the desktop. On Windows, source
  selection happens in the main window and a small floating bar handles live
  control.

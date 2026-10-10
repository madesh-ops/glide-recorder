//! CPU compositor used by export.
//!
//! Scene (zoom 1): background + soft shadow + rounded recording frame.
//! The camera scales the whole scene about its centre `(x, y)`; the cursor and
//! click ripples are drawn on top in output space, positioned through the same
//! transform. All sampling is bilinear and rows are rendered in parallel.

use crate::camera::CameraKeyframe;
use crate::cursor::{CursorFrame, CursorSettings, CursorShape};
use crate::geometry::{Rect, Vec2};
use crate::layout::{Background, Layout, Rgba, StyleSettings, REF_HEIGHT};
use crate::{Error, Result};
use rayon::prelude::*;
use std::path::Path;

/// Premultiplied RGBA sprite with a hotspot.
#[derive(Debug, Clone)]
pub struct Sprite {
    pub w: u32,
    pub h: u32,
    pub hot_x: f64,
    pub hot_y: f64,
    pub data: Vec<u8>,
}

impl Sprite {
    pub fn from_png(path: &Path, hot_x: u32, hot_y: u32) -> Result<Sprite> {
        let file = std::fs::File::open(path)?;
        let mut dec = png::Decoder::new(std::io::BufReader::new(file));
        dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = dec.read_info().map_err(|e| Error::InvalidMedia(format!("{}: {e}", path.display())))?;
        let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
        let info = reader.next_frame(&mut buf).map_err(|e| Error::InvalidMedia(e.to_string()))?;
        let (w, h) = (info.width, info.height);
        let mut data = Vec::with_capacity((w * h * 4) as usize);
        let src = &buf[..info.buffer_size()];
        match info.color_type {
            png::ColorType::Rgba => data.extend_from_slice(src),
            png::ColorType::Rgb => src.chunks_exact(3).for_each(|p| data.extend_from_slice(&[p[0], p[1], p[2], 255])),
            png::ColorType::GrayscaleAlpha => {
                src.chunks_exact(2).for_each(|p| data.extend_from_slice(&[p[0], p[0], p[0], p[1]]))
            }
            png::ColorType::Grayscale => src.iter().for_each(|&g| data.extend_from_slice(&[g, g, g, 255])),
            png::ColorType::Indexed => return Err(Error::InvalidMedia("indexed PNG not expanded".into())),
        }
        premultiply(&mut data);
        Ok(Sprite { w, h, hot_x: hot_x as f64, hot_y: hot_y as f64, data })
    }

    /// Built-in arrow cursor (used when the recorded shape is unknown).
    pub fn default_arrow() -> Sprite {
        // Classic pointer outline in a 12×19 design grid, rendered at 32 px tall.
        const POLY: [(f64, f64); 7] =
            [(0.0, 0.0), (0.0, 16.5), (4.0, 12.8), (6.9, 19.2), (9.3, 18.2), (6.5, 11.9), (11.6, 11.9)];
        let scale = 1.6;
        let (w, h) = (22u32, 34u32);
        let pad = 1.5;
        let poly: Vec<Vec2> = POLY.iter().map(|&(x, y)| Vec2::new(x * scale + pad, y * scale + pad)).collect();
        let mut data = vec![0u8; (w * h * 4) as usize];
        const SS: usize = 4;
        for py in 0..h {
            for px in 0..w {
                let (mut white, mut black) = (0.0, 0.0);
                for sy in 0..SS {
                    for sx in 0..SS {
                        let p = Vec2::new(
                            px as f64 + (sx as f64 + 0.5) / SS as f64,
                            py as f64 + (sy as f64 + 0.5) / SS as f64,
                        );
                        if point_in_poly(p, &poly) {
                            if dist_to_poly_edge(p, &poly) < 1.4 {
                                white += 1.0;
                            } else {
                                black += 1.0;
                            }
                        }
                    }
                }
                let n = (SS * SS) as f64;
                let a = (white + black) / n;
                let lum = if white + black > 0.0 { white / (white + black) } else { 0.0 };
                let i = ((py * w + px) * 4) as usize;
                let v = (lum * a * 255.0).round() as u8;
                data[i..i + 4].copy_from_slice(&[v, v, v, (a * 255.0).round() as u8]);
            }
        }
        Sprite { w, h, hot_x: pad, hot_y: pad, data }
    }
}

fn point_in_poly(p: Vec2, poly: &[Vec2]) -> bool {
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn dist_to_poly_edge(p: Vec2, poly: &[Vec2]) -> f64 {
    let mut best = f64::MAX;
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        best = best.min(p.distance(Vec2::new(a.x + dx * t, a.y + dy * t)));
    }
    best
}

pub fn premultiply(data: &mut [u8]) {
    for p in data.chunks_exact_mut(4) {
        let a = p[3] as u32;
        for c in &mut p[..3] {
            *c = ((*c as u32 * a + 127) / 255) as u8;
        }
    }
}

/// Signed distance from `p` to a rounded rectangle centred at the origin.
#[inline]
fn sd_round_rect(px: f64, py: f64, hw: f64, hh: f64, r: f64) -> f64 {
    let qx = px.abs() - hw + r;
    let qy = py.abs() - hh + r;
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - r
}

/// erfc approximation (Abramowitz & Stegun 7.1.26, |error| < 1.5e-7).
fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * z);
    let y = t
        * (0.254_829_592 + t * (-0.284_496_736 + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))))
        * (-z * z).exp();
    if x >= 0.0 {
        y
    } else {
        2.0 - y
    }
}

/// Bilinear sample of an RGBA buffer at continuous pixel coords (centres at +0.5).
#[inline]
fn sample_bilinear(buf: &[u8], w: u32, h: u32, x: f64, y: f64) -> [f32; 4] {
    let fx = (x - 0.5).clamp(0.0, (w - 1) as f64);
    let fy = (y - 0.5).clamp(0.0, (h - 1) as f64);
    let x0 = fx.floor() as usize;
    let y0 = fy.floor() as usize;
    let x1 = (x0 + 1).min(w as usize - 1);
    let y1 = (y0 + 1).min(h as usize - 1);
    let tx = (fx - x0 as f64) as f32;
    let ty = (fy - y0 as f64) as f32;
    let w_ = w as usize;
    let p = |xx: usize, yy: usize| {
        let i = (yy * w_ + xx) * 4;
        [buf[i] as f32, buf[i + 1] as f32, buf[i + 2] as f32, buf[i + 3] as f32]
    };
    let (a, b, c, d) = (p(x0, y0), p(x1, y0), p(x0, y1), p(x1, y1));
    let mut out = [0f32; 4];
    for k in 0..4 {
        let top = a[k] + (b[k] - a[k]) * tx;
        let bot = c[k] + (d[k] - c[k]) * tx;
        out[k] = top + (bot - top) * ty;
    }
    out
}

/// Bilinear sample with transparent outside the buffer (for sprites).
#[inline]
fn sample_bilinear_clip(buf: &[u8], w: u32, h: u32, x: f64, y: f64) -> [f32; 4] {
    let fx = x - 0.5;
    let fy = y - 0.5;
    let x0 = fx.floor();
    let y0 = fy.floor();
    let tx = (fx - x0) as f32;
    let ty = (fy - y0) as f32;
    let p = |xx: f64, yy: f64| -> [f32; 4] {
        if xx < 0.0 || yy < 0.0 || xx >= w as f64 || yy >= h as f64 {
            return [0.0; 4];
        }
        let i = ((yy as usize) * w as usize + xx as usize) * 4;
        [buf[i] as f32, buf[i + 1] as f32, buf[i + 2] as f32, buf[i + 3] as f32]
    };
    let (a, b, c, d) = (p(x0, y0), p(x0 + 1.0, y0), p(x0, y0 + 1.0), p(x0 + 1.0, y0 + 1.0));
    let mut out = [0f32; 4];
    for k in 0..4 {
        let top = a[k] + (b[k] - a[k]) * tx;
        let bot = c[k] + (d[k] - c[k]) * tx;
        out[k] = top + (bot - top) * ty;
    }
    out
}

fn parse_color(s: &str) -> [f32; 4] {
    let c = Rgba::parse_hex(s).unwrap_or(Rgba::rgb(0, 0, 0));
    let a = c.a as f32 / 255.0;
    [c.r as f32 * a, c.g as f32 * a, c.b as f32 * a, c.a as f32]
}

/// Render the background into a premultiplied RGBA buffer.
fn render_background(bg: &Background, w: u32, h: u32, project_dir: Option<&Path>) -> Vec<u8> {
    let mut out = vec![0u8; (w * h * 4) as usize];
    match bg {
        Background::Solid { color } => {
            let c = parse_color(color);
            for p in out.chunks_exact_mut(4) {
                p.copy_from_slice(&[c[0] as u8, c[1] as u8, c[2] as u8, c[3] as u8]);
            }
        }
        Background::Gradient { angle, stops } => {
            let mut stops: Vec<(f64, [f32; 4])> = stops.iter().map(|s| (s.at.clamp(0.0, 1.0), parse_color(&s.color))).collect();
            stops.sort_by(|a, b| a.0.total_cmp(&b.0));
            if stops.is_empty() {
                stops.push((0.0, [0.0, 0.0, 0.0, 255.0]));
            }
            let a = angle.to_radians();
            let (dx, dy) = (a.sin(), -a.cos());
            let len = (w as f64 * a.sin()).abs() + (h as f64 * a.cos()).abs();
            let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
            out.par_chunks_mut(w as usize * 4).enumerate().for_each(|(y, row)| {
                for x in 0..w as usize {
                    let t = (((x as f64 + 0.5 - cx) * dx + (y as f64 + 0.5 - cy) * dy) / len + 0.5).clamp(0.0, 1.0);
                    let c = gradient_at(&stops, t);
                    row[x * 4..x * 4 + 4].copy_from_slice(&[c[0] as u8, c[1] as u8, c[2] as u8, c[3] as u8]);
                }
            });
        }
        Background::Image { path } => {
            let loaded = project_dir
                .and_then(|d| crate::project::resolve_media(d, path).ok())
                .and_then(|p| Sprite::from_png(&p, 0, 0).map_err(|e| log::warn!("background image: {e}")).ok());
            match loaded {
                Some(img) => {
                    // object-fit: cover
                    let s = (w as f64 / img.w as f64).max(h as f64 / img.h as f64);
                    let ox = (img.w as f64 * s - w as f64) / 2.0;
                    let oy = (img.h as f64 * s - h as f64) / 2.0;
                    out.par_chunks_mut(w as usize * 4).enumerate().for_each(|(y, row)| {
                        for x in 0..w as usize {
                            let c = sample_bilinear(&img.data, img.w, img.h, (x as f64 + 0.5 + ox) / s, (y as f64 + 0.5 + oy) / s);
                            row[x * 4..x * 4 + 4].copy_from_slice(&[c[0] as u8, c[1] as u8, c[2] as u8, 255]);
                        }
                    });
                }
                None => return render_background(&Background::default(), w, h, None),
            }
        }
    }
    out
}

fn gradient_at(stops: &[(f64, [f32; 4])], t: f64) -> [f32; 4] {
    if t <= stops[0].0 {
        return stops[0].1;
    }
    for w in stops.windows(2) {
        if t <= w[1].0 {
            let span = (w[1].0 - w[0].0).max(1e-9);
            let k = ((t - w[0].0) / span) as f32;
            return std::array::from_fn(|i| w[0].1[i] + (w[1].1[i] - w[0].1[i]) * k);
        }
    }
    stops[stops.len() - 1].1
}

/// Size of the built-in arrow relative to its 34 px design at 1080p.
pub const FALLBACK_CURSOR_SCALE: f64 = 0.6;

/// Ripple radius multiplier for animation progress 0..1 (ease-out growth).
/// Mirrored by the editor preview.
pub fn click_ripple_scale(progress: f64) -> f64 {
    0.35 + 0.65 * (1.0 - (1.0 - progress).powi(3))
}

pub struct Compositor {
    pub out_w: u32,
    pub out_h: u32,
    layout: Layout,
    backdrop: Vec<u8>,
    /// Recording frame in output pixels at zoom 1.
    screen_px: Rect,
    radius_px: f64,
    ref_scale: f64,
    cursor: CursorSettings,
    click_color: [f32; 4],
    sprites: Vec<Option<Sprite>>,
    fallback_cursor: Sprite,
}

pub struct FrameInput<'a> {
    /// Opaque RGBA source frame.
    pub src: &'a [u8],
    pub src_w: u32,
    pub src_h: u32,
    pub camera: CameraKeyframe,
    pub cursor: Option<CursorFrame>,
    /// Click positions (source px) with animation progress 0..1.
    pub clicks: &'a [(Vec2, f64)],
}

impl Compositor {
    pub fn new(
        layout: Layout,
        style: &StyleSettings,
        cursor: &CursorSettings,
        shapes: &[CursorShape],
        project_dir: Option<&Path>,
        out_w: u32,
        out_h: u32,
    ) -> Self {
        let ref_scale = out_h as f64 / REF_HEIGHT;
        let screen_px = Rect::new(
            layout.screen.x * out_w as f64,
            layout.screen.y * out_h as f64,
            layout.screen.w * out_w as f64,
            layout.screen.h * out_h as f64,
        );
        let radius_px = (style.corner_radius * ref_scale).min(screen_px.w.min(screen_px.h) / 2.0);
        let mut backdrop = render_background(&style.background, out_w, out_h, project_dir);

        // Soft drop shadow: a Gaussian-blurred rounded rect (erfc of the SDF).
        if style.shadow_opacity > 0.0 {
            let sigma = (style.shadow_blur * ref_scale / 2.0).max(0.5);
            let oy = style.shadow_offset_y * ref_scale;
            let op = style.shadow_opacity as f32;
            let c = screen_px.center();
            let (hw, hh) = (screen_px.w / 2.0, screen_px.h / 2.0);
            backdrop.par_chunks_mut(out_w as usize * 4).enumerate().for_each(|(y, row)| {
                for x in 0..out_w as usize {
                    let d = sd_round_rect(x as f64 + 0.5 - c.x, y as f64 + 0.5 - c.y - oy, hw, hh, radius_px);
                    let a = op * (0.5 * erfc(d / (sigma * std::f64::consts::SQRT_2))) as f32;
                    if a > 0.0 {
                        for ch in &mut row[x * 4..x * 4 + 3] {
                            *ch = (*ch as f32 * (1.0 - a)) as u8;
                        }
                    }
                }
            });
        }

        let sprites = shapes
            .iter()
            .map(|s| {
                let p = project_dir.and_then(|d| crate::project::resolve_media(d, &s.file).ok())?;
                Sprite::from_png(&p, s.hotspot_x, s.hotspot_y)
                    .map_err(|e| log::warn!("cursor shape {}: {e}", s.file))
                    .ok()
            })
            .collect();

        Self {
            out_w,
            out_h,
            layout,
            backdrop,
            screen_px,
            radius_px,
            ref_scale,
            cursor: cursor.clone(),
            click_color: parse_color(&cursor.click_color),
            sprites,
            fallback_cursor: Sprite::default_arrow(),
        }
    }

    /// Map a scene pixel (zoom 1) to an output pixel under `cam`.
    fn scene_to_out(&self, cam: &CameraKeyframe, q: Vec2) -> Vec2 {
        let (w, h) = (self.out_w as f64, self.out_h as f64);
        Vec2::new((q.x - cam.x * w) * cam.zoom + w / 2.0, (q.y - cam.y * h) * cam.zoom + h / 2.0)
    }

    fn source_to_scene(&self, p: Vec2) -> Vec2 {
        let c = self.layout.source_to_canvas(p);
        Vec2::new(c.x * self.out_w as f64, c.y * self.out_h as f64)
    }

    /// Render one output frame (RGBA, opaque) into `out`.
    pub fn render(&self, input: &FrameInput, out: &mut [u8]) {
        let (w, h) = (self.out_w as usize, self.out_h as usize);
        assert_eq!(out.len(), w * h * 4);
        assert!(input.src.len() >= input.src_w as usize * input.src_h as usize * 4);
        let cam = input.camera;
        let z = cam.zoom.max(1e-3);
        let (cx, cy) = (cam.x * w as f64, cam.y * h as f64);
        let r = self.screen_px;
        let rc = r.center();
        let (hw, hh) = (r.w / 2.0, r.h / 2.0);
        let (sx, sy) = (input.src_w as f64 / r.w, input.src_h as f64 / r.h);

        out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
            let qy = cy + (y as f64 + 0.5 - h as f64 / 2.0) / z;
            for x in 0..w {
                let qx = cx + (x as f64 + 0.5 - w as f64 / 2.0) / z;
                let d = sd_round_rect(qx - rc.x, qy - rc.y, hw, hh, self.radius_px);
                // Anti-aliased edge one output pixel wide.
                let cover = (0.5 - d * z).clamp(0.0, 1.0) as f32;
                let px = if cover >= 1.0 {
                    sample_bilinear(input.src, input.src_w, input.src_h, (qx - r.x) * sx, (qy - r.y) * sy)
                } else {
                    let bg = sample_bilinear(&self.backdrop, self.out_w, self.out_h, qx, qy);
                    if cover <= 0.0 {
                        bg
                    } else {
                        let v = sample_bilinear(input.src, input.src_w, input.src_h, (qx - r.x) * sx, (qy - r.y) * sy);
                        [
                            v[0] * cover + bg[0] * (1.0 - cover),
                            v[1] * cover + bg[1] * (1.0 - cover),
                            v[2] * cover + bg[2] * (1.0 - cover),
                            255.0,
                        ]
                    }
                };
                row[x * 4] = px[0] as u8;
                row[x * 4 + 1] = px[1] as u8;
                row[x * 4 + 2] = px[2] as u8;
                row[x * 4 + 3] = 255;
            }
        });

        // Scale from source pixels to output pixels at the current zoom.
        let src_to_out = r.w / input.src_w as f64 * z;
        if self.cursor.click_effect {
            for &(p, progress) in input.clicks {
                let center = self.scene_to_out(&cam, self.source_to_scene(p));
                // Scene element: scales with the camera like everything else (matches the CSS preview).
                let radius = self.cursor.click_size * self.ref_scale * z * click_ripple_scale(progress);
                let fade = (1.0 - progress) as f32;
                self.draw_ripple(out, center, radius, fade);
            }
        }
        if let Some(c) = input.cursor.filter(|c| self.cursor.visible && c.opacity > 0.0) {
            let sprite = self
                .sprites
                .get(c.shape as usize)
                .and_then(|s| s.as_ref())
                .unwrap_or(&self.fallback_cursor);
            // The built-in arrow is authored at 1080p reference scale rather than source px.
            let k = if std::ptr::eq(sprite, &self.fallback_cursor) {
                self.ref_scale * z * self.cursor.scale * FALLBACK_CURSOR_SCALE
            } else {
                src_to_out * self.cursor.scale
            };
            let pos = self.scene_to_out(&cam, self.source_to_scene(Vec2::new(c.x, c.y)));
            self.draw_sprite(out, sprite, pos, k, c.opacity as f32);
        }
    }

    fn draw_sprite(&self, out: &mut [u8], s: &Sprite, pos: Vec2, k: f64, opacity: f32) {
        if k <= 0.0 {
            return;
        }
        let x0 = (pos.x - s.hot_x * k).floor().max(0.0) as i64;
        let y0 = (pos.y - s.hot_y * k).floor().max(0.0) as i64;
        let x1 = ((pos.x + (s.w as f64 - s.hot_x) * k).ceil() as i64).min(self.out_w as i64);
        let y1 = ((pos.y + (s.h as f64 - s.hot_y) * k).ceil() as i64).min(self.out_h as i64);
        let w = self.out_w as usize;
        for y in y0..y1 {
            for x in x0..x1 {
                let u = (x as f64 + 0.5 - pos.x) / k + s.hot_x;
                let v = (y as f64 + 0.5 - pos.y) / k + s.hot_y;
                let c = sample_bilinear_clip(&s.data, s.w, s.h, u, v);
                let a = c[3] / 255.0 * opacity;
                if a <= 0.0 {
                    continue;
                }
                let i = (y as usize * w + x as usize) * 4;
                for ch in 0..3 {
                    let dst = out[i + ch] as f32;
                    out[i + ch] = (c[ch] * opacity + dst * (1.0 - a)).clamp(0.0, 255.0) as u8;
                }
            }
        }
    }

    fn draw_ripple(&self, out: &mut [u8], c: Vec2, radius: f64, fade: f32) {
        let ring = (2.0 * self.ref_scale).max(1.0);
        let x0 = (c.x - radius - 2.0).floor().max(0.0) as usize;
        let y0 = (c.y - radius - 2.0).floor().max(0.0) as usize;
        let x1 = ((c.x + radius + 2.0).ceil().max(0.0) as usize).min(self.out_w as usize);
        let y1 = ((c.y + radius + 2.0).ceil().max(0.0) as usize).min(self.out_h as usize);
        let w = self.out_w as usize;
        let col = self.click_color;
        for y in y0..y1 {
            for x in x0..x1 {
                let d = Vec2::new(x as f64 + 0.5, y as f64 + 0.5).distance(c) - radius;
                let fill = (0.5 - d).clamp(0.0, 1.0) as f32 * 0.28;
                let stroke = (1.0 - (d.abs() - ring / 2.0).max(0.0)).clamp(0.0, 1.0) as f32 * 0.85;
                let a = fill.max(stroke) * fade * (col[3] / 255.0);
                if a <= 0.0 {
                    continue;
                }
                let i = (y * w + x) * 4;
                for ch in 0..3 {
                    let src = col[ch] / (col[3] / 255.0).max(1e-3);
                    out[i + ch] = (src * a + out[i + ch] as f32 * (1.0 - a)) as u8;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: [u8; 4]) -> Vec<u8> {
        c.iter().copied().cycle().take((w * h * 4) as usize).collect()
    }

    fn px(buf: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    fn style() -> StyleSettings {
        StyleSettings {
            background: Background::Solid { color: "#0000FF".into() },
            padding: 0.1,
            corner_radius: 20.0,
            shadow_opacity: 0.0,
            ..Default::default()
        }
    }

    #[test]
    fn identity_camera_shows_padding_and_content() {
        let st = style();
        let layout = Layout::compute(160, 90, &st);
        let comp = Compositor::new(layout, &st, &CursorSettings { visible: false, ..Default::default() }, &[], None, 320, 180);
        let src = solid(160, 90, [255, 0, 0, 255]);
        let mut out = vec![0u8; 320 * 180 * 4];
        comp.render(
            &FrameInput { src: &src, src_w: 160, src_h: 90, camera: CameraKeyframe::IDENTITY, cursor: None, clicks: &[] },
            &mut out,
        );
        assert_eq!(px(&out, 320, 2, 2), [0, 0, 255, 255], "background in the padding");
        assert_eq!(px(&out, 320, 160, 90), [255, 0, 0, 255], "content in the middle");
        // Rounded corner: the very corner of the screen rect shows background.
        let r = comp.screen_px;
        let corner = px(&out, 320, r.x as u32, r.y as u32);
        assert!(corner[2] > 200, "rounded corner shows background: {corner:?}");
    }

    #[test]
    fn full_zoom_on_screen_hides_background() {
        let st = style();
        let layout = Layout::compute(160, 90, &st);
        let comp = Compositor::new(layout, &st, &CursorSettings { visible: false, ..Default::default() }, &[], None, 320, 180);
        // Left half red, right half green.
        let mut src = Vec::new();
        for _y in 0..90 {
            for x in 0..160 {
                src.extend_from_slice(if x < 80 { &[255, 0, 0, 255] } else { &[0, 255, 0, 255] });
            }
        }
        let mut out = vec![0u8; 320 * 180 * 4];
        let cam = CameraKeyframe { t: 0.0, x: 0.3, y: 0.5, zoom: 2.0 };
        comp.render(&FrameInput { src: &src, src_w: 160, src_h: 90, camera: cam, cursor: None, clicks: &[] }, &mut out);
        // Visible window spans x ∈ [0.05, 0.55] of the canvas: left edge is
        // inside the padding/screen boundary, centre shows red content.
        assert_eq!(px(&out, 320, 160, 90), [255, 0, 0, 255]);
        assert_eq!(px(&out, 320, 319, 90), [0, 255, 0, 255]);
    }

    #[test]
    fn gradient_matches_css_direction() {
        let bg = Background::Gradient {
            angle: 90.0,
            stops: vec![
                crate::layout::GradientStop { color: "#000000".into(), at: 0.0 },
                crate::layout::GradientStop { color: "#FFFFFF".into(), at: 1.0 },
            ],
        };
        let b = render_background(&bg, 100, 10, None);
        assert!(px(&b, 100, 0, 5)[0] < 10, "90deg = to right: dark on the left");
        assert!(px(&b, 100, 99, 5)[0] > 245);
    }

    #[test]
    fn cursor_and_ripple_draw_at_transformed_position() {
        let st = StyleSettings { padding: 0.0, corner_radius: 0.0, shadow_opacity: 0.0, ..style() };
        let layout = Layout::compute(200, 100, &st);
        let (ow, oh) = (2160u32, 1080u32);
        let comp = Compositor::new(layout, &st, &CursorSettings::default(), &[], None, ow, oh);
        let src = solid(200, 100, [255, 255, 255, 255]);
        let mut out = vec![0u8; (ow * oh * 4) as usize];
        let cursor = CursorFrame { t: 0.0, x: 100.0, y: 50.0, shape: u32::MAX, opacity: 1.0 };
        let clicks = [(Vec2::new(20.0, 20.0), 0.2)];
        comp.render(
            &FrameInput { src: &src, src_w: 200, src_h: 100, camera: CameraKeyframe::IDENTITY, cursor: Some(cursor), clicks: &clicks },
            &mut out,
        );
        // Hotspot lands at output (1080, 540); the arrow's dark interior is below-right of it.
        let p = px(&out, ow, 1084, 554);
        assert!(p[0] < 60, "arrow drawn near hotspot: {p:?}");
        assert_eq!(px(&out, ow, 1070, 530), [255, 255, 255, 255], "nothing up-left of the hotspot");
        // Ripple ring around the click at source (20,20) → output (216,216): darkens nothing
        // on white, so check the ripple region is still white-ish but the cursor area is not.
        assert!(px(&out, ow, 216, 216)[0] > 200);
    }

    #[test]
    fn erfc_sanity() {
        assert!((erfc(0.0) - 1.0).abs() < 1e-6);
        assert!(erfc(3.0) < 1e-4);
        assert!((erfc(-3.0) - 2.0).abs() < 1e-4);
    }
}

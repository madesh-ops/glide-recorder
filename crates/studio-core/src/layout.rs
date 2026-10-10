//! Canvas geometry and visual style shared by the editor preview and export.
//!
//! All lengths that depend on output size are expressed relative to a 1080 px
//! tall reference canvas ("ref px"), so a 4K export looks identical to the
//! 1080p preview, just sharper.

use crate::geometry::{Rect, Vec2};
use serde::{Deserialize, Serialize};

pub const REF_HEIGHT: f64 = 1080.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AspectRatio {
    /// Match the recorded source.
    #[default]
    Auto,
    Wide16x9,
    Standard4x3,
    Square,
    Vertical9x16,
}

impl AspectRatio {
    pub fn value(self, source_w: f64, source_h: f64) -> f64 {
        match self {
            AspectRatio::Auto => {
                if source_h > 0.0 {
                    source_w / source_h
                } else {
                    16.0 / 9.0
                }
            }
            AspectRatio::Wide16x9 => 16.0 / 9.0,
            AspectRatio::Standard4x3 => 4.0 / 3.0,
            AspectRatio::Square => 1.0,
            AspectRatio::Vertical9x16 => 9.0 / 16.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
    pub fn parse_hex(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches('#');
        let v = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok();
        match s.len() {
            6 => Some(Self::rgb(v(0)?, v(2)?, v(4)?)),
            8 => Some(Self { r: v(0)?, g: v(2)?, b: v(4)?, a: v(6)? }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GradientStop {
    pub color: String,
    /// 0..1 along the gradient line.
    pub at: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Background {
    Solid { color: String },
    /// CSS `linear-gradient(<angle>deg, ...)` semantics: 0° points up, 90° right.
    Gradient { angle: f64, stops: Vec<GradientStop> },
    /// Image file inside the project folder (copied on import).
    Image { path: String },
}

impl Default for Background {
    fn default() -> Self {
        // A calm violet wallpaper-like gradient (original, not a copied asset).
        Background::Gradient {
            angle: 135.0,
            stops: vec![
                GradientStop { color: "#3B2A8F".into(), at: 0.0 },
                GradientStop { color: "#7B5CF0".into(), at: 0.55 },
                GradientStop { color: "#C59BF7".into(), at: 1.0 },
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StyleSettings {
    pub aspect: AspectRatio,
    pub background: Background,
    /// Padding around the recording as a fraction of the canvas' smaller side.
    pub padding: f64,
    /// Corner radius in ref px.
    pub corner_radius: f64,
    /// Shadow opacity 0..1.
    pub shadow_opacity: f64,
    /// Shadow blur radius (CSS semantics) in ref px.
    pub shadow_blur: f64,
    /// Vertical shadow offset in ref px.
    pub shadow_offset_y: f64,
}

impl Default for StyleSettings {
    fn default() -> Self {
        Self {
            aspect: AspectRatio::Auto,
            background: Background::default(),
            padding: 0.08,
            corner_radius: 14.0,
            shadow_opacity: 0.45,
            shadow_blur: 60.0,
            shadow_offset_y: 18.0,
        }
    }
}

impl StyleSettings {
    pub fn sanitize(&mut self) {
        self.padding = finite_or(self.padding, 0.08).clamp(0.0, 0.35);
        self.corner_radius = finite_or(self.corner_radius, 14.0).clamp(0.0, 200.0);
        self.shadow_opacity = finite_or(self.shadow_opacity, 0.45).clamp(0.0, 1.0);
        self.shadow_blur = finite_or(self.shadow_blur, 60.0).clamp(0.0, 300.0);
        self.shadow_offset_y = finite_or(self.shadow_offset_y, 18.0).clamp(-200.0, 200.0);
    }
}

pub(crate) fn finite_or(v: f64, d: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        d
    }
}

/// Resolved geometry in normalized canvas coordinates (0..1 on both axes).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    /// Canvas width / height.
    pub aspect: f64,
    /// Where the recording sits on the canvas (normalized).
    pub screen: Rect,
    pub source_width: f64,
    pub source_height: f64,
}

impl Layout {
    pub fn compute(source_w: u32, source_h: u32, style: &StyleSettings) -> Self {
        let (sw, sh) = (source_w.max(1) as f64, source_h.max(1) as f64);
        let aspect = style.aspect.value(sw, sh);
        // Work in a space where the canvas is `aspect` × 1.
        let pad = style.padding.clamp(0.0, 0.35) * aspect.min(1.0);
        let avail_w = (aspect - 2.0 * pad).max(1e-3);
        let avail_h = (1.0 - 2.0 * pad).max(1e-3);
        let src_aspect = sw / sh;
        let (w, h) = if avail_w / avail_h > src_aspect {
            (avail_h * src_aspect, avail_h)
        } else {
            (avail_w, avail_w / src_aspect)
        };
        let x = (aspect - w) / 2.0;
        let y = (1.0 - h) / 2.0;
        Layout {
            aspect,
            screen: Rect::new(x / aspect, y, w / aspect, h),
            source_width: sw,
            source_height: sh,
        }
    }

    /// Source pixel → normalized canvas point.
    pub fn source_to_canvas(&self, p: Vec2) -> Vec2 {
        Vec2::new(
            self.screen.x + p.x / self.source_width * self.screen.w,
            self.screen.y + p.y / self.source_height * self.screen.h,
        )
    }

    /// Normalized source point (0..1 of the recording) → canvas point.
    pub fn source_norm_to_canvas(&self, p: Vec2) -> Vec2 {
        Vec2::new(self.screen.x + p.x * self.screen.w, self.screen.y + p.y * self.screen.h)
    }

    /// Output pixel size for a given output height (both even, for yuv420p).
    pub fn output_size(&self, height: u32) -> (u32, u32) {
        let h = (height.max(2) / 2) * 2;
        let w = ((h as f64 * self.aspect / 2.0).round() as u32).max(1) * 2;
        (w, h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_aspect_with_padding_is_centered_and_preserves_source_aspect() {
        let style = StyleSettings { padding: 0.1, ..Default::default() };
        let l = Layout::compute(1920, 1080, &style);
        assert!((l.aspect - 16.0 / 9.0).abs() < 1e-9);
        let c = l.screen.center();
        assert!((c.x - 0.5).abs() < 1e-9 && (c.y - 0.5).abs() < 1e-9);
        // Pixel aspect of the screen rect equals the source aspect.
        let px_aspect = (l.screen.w * l.aspect) / l.screen.h;
        assert!((px_aspect - 16.0 / 9.0).abs() < 1e-9);
        assert!((l.screen.h - 0.8).abs() < 1e-9);
    }

    #[test]
    fn vertical_canvas_fits_wide_source() {
        let style = StyleSettings { aspect: AspectRatio::Vertical9x16, padding: 0.05, ..Default::default() };
        let l = Layout::compute(2560, 1440, &style);
        assert!(l.screen.w <= 1.0 && l.screen.h < 1.0);
        let px_aspect = (l.screen.w * l.aspect) / l.screen.h;
        assert!((px_aspect - 2560.0 / 1440.0).abs() < 1e-9);
        assert_eq!(l.output_size(1920), (1080, 1920));
    }

    #[test]
    fn no_padding_fills_canvas() {
        let style = StyleSettings { padding: 0.0, ..Default::default() };
        let l = Layout::compute(1280, 720, &style);
        assert!((l.screen.w - 1.0).abs() < 1e-9 && (l.screen.h - 1.0).abs() < 1e-9);
        let p = l.source_to_canvas(Vec2::new(640.0, 360.0));
        assert!((p.x - 0.5).abs() < 1e-9 && (p.y - 0.5).abs() < 1e-9);
    }

    #[test]
    fn output_sizes_are_even() {
        let l = Layout::compute(1366, 768, &StyleSettings::default());
        let (w, h) = l.output_size(1081);
        assert_eq!(w % 2, 0);
        assert_eq!(h % 2, 0);
    }

    #[test]
    fn hex_parse() {
        assert_eq!(Rgba::parse_hex("#ff0080"), Some(Rgba::rgb(255, 0, 128)));
        assert_eq!(Rgba::parse_hex("00000080").unwrap().a, 128);
        assert_eq!(Rgba::parse_hex("nope"), None);
    }
}

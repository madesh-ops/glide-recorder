//! Cursor metadata recorded alongside the video, and the smoothing / effects
//! applied at edit time. The cursor is never burned into `screen.mp4`, so all
//! of this stays adjustable.

use crate::easing::Spring;
use crate::geometry::Vec2;
use crate::layout::finite_or;
use serde::{Deserialize, Serialize};

pub const CURSOR_TRACK_VERSION: u32 = 1;

/// A cursor sample in source pixels (relative to the captured area).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CursorSample {
    /// Recording time in seconds.
    pub t: f64,
    pub x: f32,
    pub y: f32,
    /// Index into `CursorTrack::shapes`, or `u32::MAX` if unknown.
    #[serde(rename = "s")]
    pub shape: u32,
    /// False when the system cursor is hidden (e.g. while typing, games).
    #[serde(rename = "v")]
    pub visible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClickEvent {
    /// Time of button-down.
    pub t: f64,
    pub x: f32,
    pub y: f32,
    pub button: MouseButton,
    /// Time of button-up, if seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub up: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorShape {
    /// PNG file relative to the project folder (straight RGBA).
    pub file: String,
    pub width: u32,
    pub height: u32,
    pub hotspot_x: u32,
    pub hotspot_y: u32,
    /// Human readable kind when it maps to a standard system cursor ("arrow", "ibeam", ...).
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorTrack {
    pub version: u32,
    pub source_width: u32,
    pub source_height: u32,
    pub samples: Vec<CursorSample>,
    pub clicks: Vec<ClickEvent>,
    pub shapes: Vec<CursorShape>,
}

impl CursorTrack {
    /// Raw cursor position at time `t` (source px). Interpolates between samples
    /// that are close in time; holds the last position across long gaps (a mouse
    /// at rest produces no events).
    pub fn raw_at(&self, t: f64) -> Option<(Vec2, &CursorSample)> {
        if self.samples.is_empty() {
            return None;
        }
        let i = self.samples.partition_point(|s| s.t <= t);
        if i == 0 {
            let s = &self.samples[0];
            return Some((Vec2::new(s.x as f64, s.y as f64), s));
        }
        let a = &self.samples[i - 1];
        let pa = Vec2::new(a.x as f64, a.y as f64);
        if let Some(b) = self.samples.get(i) {
            let gap = b.t - a.t;
            if gap > 1e-6 && gap < 0.1 {
                let pb = Vec2::new(b.x as f64, b.y as f64);
                return Some((pa.lerp(pb, (t - a.t) / gap), a));
            }
        }
        Some((pa, a))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CursorSettings {
    pub visible: bool,
    /// Multiplier on the recorded cursor size.
    pub scale: f64,
    /// Smoothing time constant in seconds (0 = raw movement).
    pub smoothing: f64,
    /// Hide the cursor after this many seconds without movement (0 = never).
    pub hide_when_idle: f64,
    pub click_effect: bool,
    /// Ripple color (hex).
    pub click_color: String,
    /// Ripple radius in ref px.
    pub click_size: f64,
    /// Ripple duration in seconds.
    pub click_duration: f64,
}

impl Default for CursorSettings {
    fn default() -> Self {
        Self {
            visible: true,
            scale: 1.5,
            smoothing: 0.12,
            hide_when_idle: 0.0,
            click_effect: true,
            click_color: "#FFFFFF".into(),
            click_size: 36.0,
            click_duration: 0.45,
        }
    }
}

impl CursorSettings {
    pub fn sanitize(&mut self) {
        self.scale = finite_or(self.scale, 1.5).clamp(0.25, 6.0);
        self.smoothing = finite_or(self.smoothing, 0.12).clamp(0.0, 1.0);
        self.hide_when_idle = finite_or(self.hide_when_idle, 0.0).clamp(0.0, 60.0);
        self.click_size = finite_or(self.click_size, 36.0).clamp(4.0, 200.0);
        self.click_duration = finite_or(self.click_duration, 0.45).clamp(0.05, 3.0);
    }
}

/// Resolved cursor state at a point in time, in source pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CursorFrame {
    pub t: f64,
    pub x: f64,
    pub y: f64,
    pub shape: u32,
    /// 0..1 (fades for idle hiding; 0 when the system cursor was hidden).
    pub opacity: f64,
}

/// Cursor path sampled at a fixed rate after smoothing and idle hiding.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CursorPath {
    pub rate: f64,
    pub frames: Vec<CursorFrame>,
}

const IDLE_FADE: f64 = 0.25;
const MOVE_EPS: f64 = 1.5;

impl CursorPath {
    pub fn compute(track: &CursorTrack, settings: &CursorSettings, duration: f64, rate: f64) -> Self {
        let mut frames = Vec::new();
        if track.samples.is_empty() || duration <= 0.0 {
            return Self { rate, frames };
        }
        let dt = 1.0 / rate;
        let n = (duration * rate).ceil() as usize + 1;
        frames.reserve(n);
        let (first, _) = track.raw_at(0.0).unwrap();
        let (mut sx, mut sy) = (Spring::at(first.x), Spring::at(first.y));
        let mut last_raw = first;
        let mut last_move_t = 0.0f64;
        for i in 0..n {
            let t = i as f64 * dt;
            let (raw, sample) = track.raw_at(t).unwrap();
            if raw.distance(last_raw) > MOVE_EPS {
                last_move_t = t;
                last_raw = raw;
            }
            let (x, y) = if i == 0 {
                (raw.x, raw.y)
            } else {
                (sx.step(raw.x, settings.smoothing, dt), sy.step(raw.y, settings.smoothing, dt))
            };
            let mut opacity = if sample.visible { 1.0 } else { 0.0 };
            if settings.hide_when_idle > 0.0 {
                let idle = t - last_move_t - settings.hide_when_idle;
                if idle > 0.0 {
                    opacity *= (1.0 - idle / IDLE_FADE).clamp(0.0, 1.0);
                }
            }
            frames.push(CursorFrame { t, x, y, shape: sample.shape, opacity });
        }
        Self { rate, frames }
    }

    pub fn sample(&self, t: f64) -> Option<CursorFrame> {
        if self.frames.is_empty() {
            return None;
        }
        let f = (t * self.rate).max(0.0);
        let i = (f.floor() as usize).min(self.frames.len() - 1);
        let j = (i + 1).min(self.frames.len() - 1);
        let a = self.frames[i];
        let b = self.frames[j];
        let k = (f - i as f64).clamp(0.0, 1.0);
        Some(CursorFrame {
            t,
            x: a.x + (b.x - a.x) * k,
            y: a.y + (b.y - a.y) * k,
            shape: if k < 0.5 { a.shape } else { b.shape },
            opacity: a.opacity + (b.opacity - a.opacity) * k,
        })
    }
}

/// One line of the append-only `cursor.log` written while recording. If the
/// app crashes, the log is replayed to rebuild the cursor track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k")]
pub enum CursorLogEntry {
    #[serde(rename = "s")]
    Sample(CursorSample),
    #[serde(rename = "c")]
    Click(ClickEvent),
    #[serde(rename = "u")]
    ClickUp { button: MouseButton, t: f64 },
    #[serde(rename = "h")]
    Shape(CursorShape),
}

impl CursorTrack {
    /// Rebuild a track from `cursor.log` lines. Malformed lines (e.g. a line
    /// cut off by a crash) are skipped.
    pub fn from_log(text: &str, source_width: u32, source_height: u32) -> Self {
        let mut t = CursorTrack { version: CURSOR_TRACK_VERSION, source_width, source_height, ..Default::default() };
        for line in text.lines() {
            match serde_json::from_str::<CursorLogEntry>(line) {
                Ok(CursorLogEntry::Sample(s)) => t.samples.push(s),
                Ok(CursorLogEntry::Click(c)) => t.clicks.push(c),
                Ok(CursorLogEntry::ClickUp { button, t: up }) => {
                    if let Some(c) = t.clicks.iter_mut().rev().find(|c| c.button == button && c.up.is_none()) {
                        c.up = Some(up);
                    }
                }
                Ok(CursorLogEntry::Shape(s)) => t.shapes.push(s),
                Err(_) => {}
            }
        }
        t
    }
}

/// Click ripple progress (0..1) for clicks active at time `t`.
pub fn active_clicks<'a>(
    clicks: &'a [ClickEvent],
    t: f64,
    duration: f64,
) -> impl Iterator<Item = (&'a ClickEvent, f64)> + 'a {
    let start = clicks.partition_point(|c| c.t < t - duration);
    clicks[start..]
        .iter()
        .take_while(move |c| c.t <= t)
        .filter(|c| c.button == MouseButton::Left || c.button == MouseButton::Right)
        .map(move |c| (c, ((t - c.t) / duration).clamp(0.0, 1.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(t: f64, x: f32, y: f32) -> CursorSample {
        CursorSample { t, x, y, shape: 0, visible: true }
    }

    fn track(samples: Vec<CursorSample>) -> CursorTrack {
        CursorTrack { version: 1, source_width: 1000, source_height: 1000, samples, ..Default::default() }
    }

    #[test]
    fn raw_interpolates_short_gaps_and_holds_long_gaps() {
        let t = track(vec![sample(0.0, 0.0, 0.0), sample(0.05, 100.0, 0.0), sample(5.0, 500.0, 0.0)]);
        let (p, _) = t.raw_at(0.025).unwrap();
        assert!((p.x - 50.0).abs() < 1e-6);
        let (p, _) = t.raw_at(2.0).unwrap();
        assert!((p.x - 100.0).abs() < 1e-6, "held during rest");
    }

    #[test]
    fn smoothing_reduces_jitter() {
        // Alternate ±4 px around 500 at 120 Hz.
        let samples: Vec<_> = (0..240)
            .map(|i| sample(i as f64 / 120.0, 500.0 + if i % 2 == 0 { 4.0 } else { -4.0 }, 500.0))
            .collect();
        let tr = track(samples);
        let raw = CursorPath::compute(&tr, &CursorSettings { smoothing: 0.0, ..Default::default() }, 2.0, 60.0);
        let smooth = CursorPath::compute(&tr, &CursorSettings { smoothing: 0.15, ..Default::default() }, 2.0, 60.0);
        let jitter = |p: &CursorPath| {
            p.frames.windows(2).skip(30).map(|w| (w[1].x - w[0].x).abs()).fold(0.0f64, f64::max)
        };
        assert!(jitter(&smooth) < jitter(&raw) * 0.5, "{} vs {}", jitter(&smooth), jitter(&raw));
    }

    #[test]
    fn idle_hiding_fades_and_returns() {
        let tr = track(vec![sample(0.0, 0.0, 0.0), sample(4.0, 300.0, 300.0)]);
        let s = CursorSettings { hide_when_idle: 1.0, smoothing: 0.0, ..Default::default() };
        let p = CursorPath::compute(&tr, &s, 5.0, 60.0);
        assert_eq!(p.sample(0.5).unwrap().opacity, 1.0);
        assert_eq!(p.sample(3.0).unwrap().opacity, 0.0);
        assert_eq!(p.sample(4.2).unwrap().opacity, 1.0);
    }

    #[test]
    fn log_replay_rebuilds_track_and_tolerates_truncation() {
        let entries = [
            CursorLogEntry::Shape(CursorShape { file: "cursors/cursor_0.png".into(), width: 32, height: 32, hotspot_x: 0, hotspot_y: 0, kind: Some("arrow".into()) }),
            CursorLogEntry::Sample(sample(0.0, 1.0, 2.0)),
            CursorLogEntry::Click(ClickEvent { t: 0.5, x: 3.0, y: 4.0, button: MouseButton::Left, up: None }),
            CursorLogEntry::ClickUp { button: MouseButton::Left, t: 0.6 },
            CursorLogEntry::Sample(sample(0.7, 5.0, 6.0)),
        ];
        let mut text: String = entries.iter().map(|e| serde_json::to_string(e).unwrap() + "
").collect();
        text.push_str("{\"k\":\"s\",\"t\":0.8,\"x\":"); // crash mid-line
        let t = CursorTrack::from_log(&text, 100, 50);
        assert_eq!(t.samples.len(), 2);
        assert_eq!(t.clicks.len(), 1);
        assert_eq!(t.clicks[0].up, Some(0.6));
        assert_eq!(t.shapes.len(), 1);
        assert_eq!((t.source_width, t.source_height), (100, 50));
    }

    #[test]
    fn active_click_progress() {
        let clicks = vec![
            ClickEvent { t: 1.0, x: 0.0, y: 0.0, button: MouseButton::Left, up: None },
            ClickEvent { t: 2.0, x: 0.0, y: 0.0, button: MouseButton::Left, up: None },
        ];
        let v: Vec<_> = active_clicks(&clicks, 1.2, 0.4).collect();
        assert_eq!(v.len(), 1);
        assert!((v[0].1 - 0.5).abs() < 1e-9);
        assert_eq!(active_clicks(&clicks, 1.9, 0.4).count(), 0);
    }
}

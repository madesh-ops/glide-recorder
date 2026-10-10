//! Automatic zoom and camera trajectory.
//!
//! The camera is an explicit state `(t, x, y, zoom)`: `(x, y)` is the centre of
//! the visible window in normalized canvas coordinates and `zoom` scales the
//! whole composed scene. See `docs/ARCHITECTURE.md` for the algorithm overview.

use crate::cursor::{ClickEvent, CursorTrack, MouseButton};
use crate::easing::{smooth_damp, smoothstep, Easing};
use crate::geometry::{Rect, Vec2};
use crate::layout::{finite_or, Layout};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ZoomSettings {
    /// Generate zoom segments from clicks automatically.
    pub auto_zoom: bool,
    /// Zoom level for new segments.
    pub default_level: f64,
    pub min_level: f64,
    pub max_level: f64,
    /// Zoom in/out animation duration in seconds.
    pub transition: f64,
    pub easing: Easing,
    /// Camera follow time constant in seconds (critically damped spring).
    pub follow_smoothing: f64,
    /// Fraction of the visible window the cursor may move in before the camera pans.
    pub dead_zone: f64,
    /// Clicks closer than this (seconds) belong to the same zoom.
    pub merge_gap: f64,
    /// Start zooming this long before the first click.
    pub lead_in: f64,
    /// Stay zoomed this long after the last click (before zooming out).
    pub hold: f64,
    pub min_duration: f64,
    /// Segments separated by less than this are merged (avoids zoom-out/in bounce).
    pub min_gap: f64,
}

impl Default for ZoomSettings {
    fn default() -> Self {
        Self {
            auto_zoom: true,
            default_level: 2.0,
            min_level: 1.25,
            max_level: 4.0,
            transition: 0.8,
            easing: Easing::EaseInOutCubic,
            follow_smoothing: 0.45,
            dead_zone: 0.55,
            merge_gap: 3.0,
            lead_in: 0.9,
            hold: 1.5,
            min_duration: 2.0,
            min_gap: 1.5,
        }
    }
}

impl ZoomSettings {
    pub fn sanitize(&mut self) {
        self.min_level = finite_or(self.min_level, 1.25).clamp(1.0, 8.0);
        self.max_level = finite_or(self.max_level, 4.0).clamp(self.min_level, 8.0);
        self.default_level = finite_or(self.default_level, 2.0).clamp(self.min_level, self.max_level);
        self.transition = finite_or(self.transition, 0.8).clamp(0.05, 3.0);
        self.follow_smoothing = finite_or(self.follow_smoothing, 0.45).clamp(0.0, 3.0);
        self.dead_zone = finite_or(self.dead_zone, 0.55).clamp(0.0, 0.95);
        self.merge_gap = finite_or(self.merge_gap, 3.0).clamp(0.0, 30.0);
        self.lead_in = finite_or(self.lead_in, 0.9).clamp(0.0, 5.0);
        self.hold = finite_or(self.hold, 1.5).clamp(0.0, 10.0);
        self.min_duration = finite_or(self.min_duration, 2.0).clamp(0.2, 30.0);
        self.min_gap = finite_or(self.min_gap, 1.5).clamp(0.0, 30.0);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case")]
pub enum ZoomFocus {
    /// Follow the cursor (starting at the anchor click, if any).
    #[default]
    Auto,
    /// Fixed point in normalized source coordinates (0..1).
    Manual { x: f64, y: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoomSegment {
    pub id: String,
    /// Source time (s) where the zoom starts easing in.
    pub start: f64,
    /// Source time (s) where the zoom has fully eased out.
    pub end: f64,
    pub level: f64,
    #[serde(default)]
    pub focus: ZoomFocus,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Generated from clicks (re-generated when auto-zoom settings change).
    #[serde(default)]
    pub auto: bool,
    /// First click of the interaction (normalized source coords) and its time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Vec2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_time: Option<f64>,
}

fn yes() -> bool {
    true
}

pub fn new_segment_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

const DOUBLE_CLICK_TIME: f64 = 0.35;
const DOUBLE_CLICK_DIST: f64 = 12.0;
/// Fraction of the visible window an interaction's clicks may span.
const GROUP_FILL: f64 = 0.7;

/// Generate zoom segments from the recorded clicks.
pub fn generate_segments(
    track: &CursorTrack,
    duration: f64,
    layout: &Layout,
    settings: &ZoomSettings,
) -> Vec<ZoomSegment> {
    if !settings.auto_zoom || duration <= 0.0 {
        return Vec::new();
    }
    let (sw, sh) = (layout.source_width, layout.source_height);

    // 1–2. Filter: primary/secondary buttons inside the capture, no double-click bursts.
    let mut clicks: Vec<&ClickEvent> = Vec::new();
    for c in &track.clicks {
        if !matches!(c.button, MouseButton::Left | MouseButton::Right) {
            continue;
        }
        if c.t < 0.0 || c.t > duration {
            continue;
        }
        let (x, y) = (c.x as f64, c.y as f64);
        if x < 0.0 || y < 0.0 || x > sw || y > sh {
            continue;
        }
        if let Some(p) = clicks.last() {
            let d = Vec2::new(p.x as f64, p.y as f64).distance(Vec2::new(x, y));
            if c.t - p.t < DOUBLE_CLICK_TIME && d < DOUBLE_CLICK_DIST {
                continue;
            }
        }
        clicks.push(c);
    }

    // 3. Group by time.
    let mut groups: Vec<Vec<&ClickEvent>> = Vec::new();
    for c in clicks {
        match groups.last_mut() {
            Some(g) if c.t - g.last().unwrap().t <= settings.merge_gap => g.push(c),
            _ => groups.push(vec![c]),
        }
    }

    // 4. One segment per group, with a level that fits the group's clicks.
    let mut segs: Vec<ZoomSegment> = groups
        .iter()
        .map(|g| {
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for c in g {
                x0 = x0.min(c.x as f64);
                x1 = x1.max(c.x as f64);
                y0 = y0.min(c.y as f64);
                y1 = y1.max(c.y as f64);
            }
            let span_w = (x1 - x0) / sw * layout.screen.w;
            let span_h = (y1 - y0) / sh * layout.screen.h;
            let span = span_w.max(span_h);
            let fit = if span > 1e-9 { GROUP_FILL / span } else { f64::MAX };
            let level = settings.default_level.min(fit).clamp(settings.min_level, settings.max_level);
            let first = g[0];
            let last = g[g.len() - 1];
            ZoomSegment {
                id: new_segment_id(),
                start: (first.t - settings.lead_in).max(0.0),
                end: (last.t + settings.hold + settings.transition).min(duration),
                level,
                focus: ZoomFocus::Auto,
                enabled: true,
                auto: true,
                anchor: Some(Vec2::new(first.x as f64 / sw, first.y as f64 / sh)),
                anchor_time: Some(first.t),
            }
        })
        .collect();

    // 5. Merge segments that are too close to each other.
    let mut merged: Vec<ZoomSegment> = Vec::with_capacity(segs.len());
    for s in segs.drain(..) {
        match merged.last_mut() {
            Some(m) if s.start - m.end < settings.min_gap => {
                m.end = m.end.max(s.end);
                m.level = m.level.min(s.level);
            }
            _ => merged.push(s),
        }
    }

    // 6. Enforce minimum duration within the recording.
    for s in &mut merged {
        if s.end - s.start < settings.min_duration {
            s.end = (s.start + settings.min_duration).min(duration);
            s.start = (s.end - settings.min_duration).max(0.0);
        }
    }
    // Extending may create overlaps again; resolve by merging.
    let mut out: Vec<ZoomSegment> = Vec::with_capacity(merged.len());
    for s in merged {
        match out.last_mut() {
            Some(m) if s.start <= m.end => {
                m.end = m.end.max(s.end);
                m.level = m.level.min(s.level);
            }
            _ => out.push(s),
        }
    }
    out.retain(|s| s.end - s.start >= 0.3);
    out
}

/// Zoom contributed by one segment at time `t`.
pub fn segment_zoom(seg: &ZoomSegment, t: f64, transition: f64, easing: Easing) -> f64 {
    if !seg.enabled || t <= seg.start || t >= seg.end || seg.level <= 1.0 {
        return 1.0;
    }
    let tr = transition.min((seg.end - seg.start) / 2.0).max(1e-3);
    let k = if t < seg.start + tr {
        easing.apply((t - seg.start) / tr)
    } else if t > seg.end - tr {
        easing.apply((seg.end - t) / tr)
    } else {
        1.0
    };
    1.0 + (seg.level - 1.0) * k
}

/// Overall zoom and the index of the dominating segment.
pub fn zoom_at(segments: &[ZoomSegment], t: f64, settings: &ZoomSettings) -> (f64, Option<usize>) {
    let mut best = (1.0, None);
    for (i, s) in segments.iter().enumerate() {
        let z = segment_zoom(s, t, settings.transition, settings.easing);
        if z > best.0 + 1e-12 {
            best = (z, Some(i));
        }
    }
    best
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraKeyframe {
    pub t: f64,
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

impl CameraKeyframe {
    pub const IDENTITY: CameraKeyframe = CameraKeyframe { t: 0.0, x: 0.5, y: 0.5, zoom: 1.0 };

    /// Visible window in normalized canvas coordinates.
    pub fn viewport(&self) -> Rect {
        let half = 0.5 / self.zoom;
        Rect::new(self.x - half, self.y - half, 1.0 / self.zoom, 1.0 / self.zoom)
    }
}

/// Region the visible window must stay inside at zoom `z`: the whole canvas at
/// zoom 1, blending to the recording itself once the window fits inside it.
pub fn camera_bounds(layout: &Layout, z: f64) -> Rect {
    let s = layout.screen;
    let fit = (1.0 / s.w).max(1.0 / s.h);
    let b = if fit <= 1.0 + 1e-6 { 1.0 } else { smoothstep(1.0, fit, z) };
    Rect::UNIT.lerp(&s, b)
}

pub fn clamp_center(c: Vec2, z: f64, r: Rect) -> Vec2 {
    let half = 0.5 / z;
    let axis = |v: f64, lo: f64, hi: f64| if lo <= hi { v.clamp(lo, hi) } else { (lo + hi) * 0.5 };
    Vec2::new(
        axis(c.x, r.x + half, r.right() - half),
        axis(c.y, r.y + half, r.bottom() - half),
    )
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CameraPath {
    pub rate: f64,
    pub frames: Vec<CameraKeyframe>,
}

impl CameraPath {
    /// Simulate the camera over `[0, duration]` at `rate` Hz.
    pub fn compute(
        segments: &[ZoomSegment],
        track: &CursorTrack,
        layout: &Layout,
        settings: &ZoomSettings,
        duration: f64,
        rate: f64,
    ) -> Self {
        let dt = 1.0 / rate;
        let n = (duration.max(0.0) * rate).ceil() as usize + 1;
        let mut frames = Vec::with_capacity(n);
        let center = Vec2::new(0.5, 0.5);
        let cursor_at = |t: f64| -> Option<Vec2> {
            track.raw_at(t).map(|(p, _)| layout.source_to_canvas(p))
        };

        let mut raw = center;
        let mut vel = (0.0, 0.0);
        let mut target = center;
        let mut active: Option<usize> = None;
        let mut ref_level = 1.0f64;
        let mut ref_vel = 0.0;

        for i in 0..n {
            let t = i as f64 * dt;
            let (z, dom) = zoom_at(segments, t, settings);
            let c = match dom {
                Some(si) => {
                    let seg = &segments[si];
                    let level = seg.level.max(1.0 + 1e-6);
                    let target_bounds = camera_bounds(layout, level);
                    let entering = active != Some(si);
                    if entering {
                        active = Some(si);
                        target = match seg.focus {
                            ZoomFocus::Manual { x, y } => layout.source_norm_to_canvas(Vec2::new(x, y)),
                            ZoomFocus::Auto => match seg.anchor {
                                Some(a) => layout.source_norm_to_canvas(a),
                                None => cursor_at(t).unwrap_or(center),
                            },
                        };
                        target = clamp_center(target, level, target_bounds);
                        if ref_level <= 1.0 + 1e-3 || frames.is_empty() {
                            ref_level = level;
                            ref_vel = 0.0;
                        }
                    }
                    match seg.focus {
                        ZoomFocus::Manual { x, y } => {
                            target = layout.source_norm_to_canvas(Vec2::new(x, y));
                        }
                        ZoomFocus::Auto => {
                            let before_anchor = seg.anchor_time.is_some_and(|at| t < at);
                            if !before_anchor {
                                if let Some(p) = cursor_at(t) {
                                    let half = 0.5 / level * settings.dead_zone;
                                    target.x = follow_axis(target.x, p.x, half);
                                    target.y = follow_axis(target.y, p.y, half);
                                }
                            }
                        }
                    }
                    target = clamp_center(target, level, target_bounds);

                    ref_level = smooth_damp(ref_level, level, &mut ref_vel, 0.2, dt).max(1.0 + 1e-6);
                    // How far along the zoom we are; 0 at zoom 1, 1 at the segment level.
                    let s = ((1.0 - 1.0 / z) / (1.0 - 1.0 / ref_level)).clamp(0.0, 1.0);
                    if entering && s < 0.02 {
                        // Invisible at this zoom: snap instead of sweeping across the screen.
                        raw = target;
                        vel = (0.0, 0.0);
                    } else {
                        raw.x = smooth_damp(raw.x, target.x, &mut vel.0, settings.follow_smoothing, dt);
                        raw.y = smooth_damp(raw.y, target.y, &mut vel.1, settings.follow_smoothing, dt);
                    }
                    center.lerp(raw, s)
                }
                None => {
                    active = None;
                    ref_level = 1.0;
                    ref_vel = 0.0;
                    center
                }
            };
            // Safety net only: with the target inside the level-L bounds, the
            // zoom-toward-point blend already keeps the window on the canvas.
            let c = clamp_center(c, z, Rect::UNIT);
            frames.push(CameraKeyframe { t, x: c.x, y: c.y, zoom: z });
        }
        Self { rate, frames }
    }

    pub fn sample(&self, t: f64) -> CameraKeyframe {
        if self.frames.is_empty() {
            return CameraKeyframe { t, ..CameraKeyframe::IDENTITY };
        }
        let f = (t * self.rate).max(0.0);
        let i = (f.floor() as usize).min(self.frames.len() - 1);
        let j = (i + 1).min(self.frames.len() - 1);
        let (a, b) = (self.frames[i], self.frames[j]);
        let k = (f - i as f64).clamp(0.0, 1.0);
        CameraKeyframe {
            t,
            x: a.x + (b.x - a.x) * k,
            y: a.y + (b.y - a.y) * k,
            zoom: a.zoom + (b.zoom - a.zoom) * k,
        }
    }
}

/// Move `target` only as far as needed to keep `p` within `half` of it.
fn follow_axis(target: f64, p: f64, half: f64) -> f64 {
    if p > target + half {
        p - half
    } else if p < target - half {
        p + half
    } else {
        target
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::CursorSample;
    use crate::layout::StyleSettings;

    const W: u32 = 1920;
    const H: u32 = 1080;

    fn layout() -> Layout {
        Layout::compute(W, H, &StyleSettings::default())
    }

    fn click(t: f64, x: f32, y: f32) -> ClickEvent {
        ClickEvent { t, x, y, button: MouseButton::Left, up: Some(t + 0.08) }
    }

    /// Cursor that moves linearly between waypoints, sampled at 120 Hz, with optional jitter.
    fn track(waypoints: &[(f64, f32, f32)], clicks: Vec<ClickEvent>, jitter: f32) -> CursorTrack {
        let mut samples = Vec::new();
        let end = waypoints.last().unwrap().0;
        let mut i = 0usize;
        let mut k = 0u32;
        let mut t = 0.0;
        while t <= end {
            while i + 1 < waypoints.len() && waypoints[i + 1].0 < t {
                i += 1;
            }
            let a = waypoints[i];
            let b = waypoints[(i + 1).min(waypoints.len() - 1)];
            let f = if b.0 > a.0 { ((t - a.0) / (b.0 - a.0)).clamp(0.0, 1.0) as f32 } else { 0.0 };
            let j = if k % 2 == 0 { jitter } else { -jitter };
            samples.push(CursorSample {
                t,
                x: a.1 + (b.1 - a.1) * f + j,
                y: a.2 + (b.2 - a.2) * f - j,
                shape: 0,
                visible: true,
            });
            k += 1;
            t += 1.0 / 120.0;
        }
        CursorTrack { version: 1, source_width: W, source_height: H, samples, clicks, shapes: vec![] }
    }

    #[test]
    fn no_clicks_no_zoom() {
        let tr = track(&[(0.0, 100.0, 100.0), (10.0, 1800.0, 900.0)], vec![], 0.0);
        let s = ZoomSettings::default();
        let segs = generate_segments(&tr, 10.0, &layout(), &s);
        assert!(segs.is_empty());
        let path = CameraPath::compute(&segs, &tr, &layout(), &s, 10.0, 60.0);
        for f in &path.frames {
            assert_eq!(f.zoom, 1.0);
            assert!((f.x - 0.5).abs() < 1e-12 && (f.y - 0.5).abs() < 1e-12);
        }
    }

    #[test]
    fn auto_zoom_disabled_generates_nothing() {
        let tr = track(&[(0.0, 100.0, 100.0), (10.0, 100.0, 100.0)], vec![click(3.0, 100.0, 100.0)], 0.0);
        let s = ZoomSettings { auto_zoom: false, ..Default::default() };
        assert!(generate_segments(&tr, 10.0, &layout(), &s).is_empty());
    }

    #[test]
    fn clicks_group_by_time_and_far_apart_clicks_split() {
        let clicks = vec![
            click(2.0, 400.0, 300.0),
            click(3.0, 450.0, 320.0),
            click(4.5, 500.0, 330.0),
            click(20.0, 1500.0, 800.0),
        ];
        let tr = track(&[(0.0, 400.0, 300.0), (30.0, 1500.0, 800.0)], clicks, 0.0);
        let s = ZoomSettings::default();
        let segs = generate_segments(&tr, 30.0, &layout(), &s);
        assert_eq!(segs.len(), 2, "{segs:#?}");
        assert!((segs[0].start - (2.0 - s.lead_in)).abs() < 1e-9);
        assert!((segs[0].end - (4.5 + s.hold + s.transition)).abs() < 1e-9);
        assert_eq!(segs[0].anchor_time, Some(2.0));
        assert!(segs.iter().all(|s| s.auto && s.enabled));
    }

    #[test]
    fn double_clicks_are_one_interaction_and_close_segments_merge() {
        let clicks = vec![
            click(2.0, 400.0, 300.0),
            click(2.2, 401.0, 300.0), // double click
            click(6.5, 420.0, 310.0), // > merge_gap after the last click, but segments nearly touch
        ];
        let tr = track(&[(0.0, 400.0, 300.0), (12.0, 400.0, 300.0)], clicks, 0.0);
        let s = ZoomSettings::default();
        let segs = generate_segments(&tr, 12.0, &layout(), &s);
        assert_eq!(segs.len(), 1, "{segs:#?}");
    }

    #[test]
    fn wide_interactions_get_a_lower_zoom_level() {
        let clicks = vec![click(2.0, 100.0, 500.0), click(3.0, 1800.0, 500.0)];
        let tr = track(&[(0.0, 100.0, 500.0), (10.0, 1800.0, 500.0)], clicks, 0.0);
        let s = ZoomSettings::default();
        let segs = generate_segments(&tr, 10.0, &layout(), &s);
        assert_eq!(segs.len(), 1);
        assert!(segs[0].level < s.default_level);
        assert!(segs[0].level >= s.min_level);
    }

    #[test]
    fn segments_are_clamped_to_recording() {
        let clicks = vec![click(0.2, 400.0, 300.0), click(9.9, 400.0, 300.0)];
        let tr = track(&[(0.0, 400.0, 300.0), (10.0, 400.0, 300.0)], clicks, 0.0);
        let segs = generate_segments(&tr, 10.0, &layout(), &ZoomSettings { merge_gap: 1.0, ..Default::default() });
        for s in &segs {
            assert!(s.start >= 0.0 && s.end <= 10.0 && s.end > s.start);
        }
    }

    fn scenario() -> (Vec<ZoomSegment>, CursorTrack, ZoomSettings, f64) {
        let clicks = vec![click(3.0, 300.0, 200.0), click(4.0, 600.0, 400.0), click(12.0, 1700.0, 950.0)];
        let tr = track(
            &[(0.0, 960.0, 540.0), (2.8, 300.0, 200.0), (4.0, 600.0, 400.0), (11.5, 1700.0, 950.0), (16.0, 1000.0, 500.0)],
            clicks,
            3.0,
        );
        let s = ZoomSettings::default();
        let segs = generate_segments(&tr, 16.0, &layout(), &s);
        (segs, tr, s, 16.0)
    }

    #[test]
    fn camera_stays_in_bounds_and_frames_are_continuous() {
        let (segs, tr, s, dur) = scenario();
        assert!(!segs.is_empty());
        let l = layout();
        let path = CameraPath::compute(&segs, &tr, &l, &s, dur, 60.0);
        let fit = (1.0 / l.screen.w).max(1.0 / l.screen.h);
        for f in &path.frames {
            let v = f.viewport();
            assert!(v.x >= -1e-9 && v.y >= -1e-9 && v.right() <= 1.0 + 1e-9 && v.bottom() <= 1.0 + 1e-9, "{f:?}");
            // While holding a zoom that fits inside the recording, no background shows.
            let holding = segs.iter().any(|g| (f.zoom - g.level).abs() < 1e-9 && g.level >= fit);
            if holding {
                let r = l.screen;
                assert!(v.x >= r.x - 1e-6 && v.right() <= r.right() + 1e-6, "background visible at full zoom {f:?}");
                assert!(v.y >= r.y - 1e-6 && v.bottom() <= r.bottom() + 1e-6, "background visible at full zoom {f:?}");
            }
        }
        // Continuity: per-frame zoom change is bounded by the easing's maximum
        // slope (cubic in-out peaks at 3 × level change / transition) — no jumps.
        let max_dz = segs.iter().map(|g| (g.level - 1.0) * 3.0 / s.transition / 60.0).fold(0.0, f64::max) * 1.05;
        for w in path.frames.windows(2) {
            assert!((w[1].zoom - w[0].zoom).abs() <= max_dz, "zoom jump {:?}", w);
            assert!((w[1].x - w[0].x).abs() < 0.03, "pan jump {:?}", w);
            assert!((w[1].y - w[0].y).abs() < 0.03, "pan jump {:?}", w);
        }
    }

    #[test]
    fn camera_has_no_jitter_from_noisy_cursor() {
        let (segs, tr, s, dur) = scenario();
        let path = CameraPath::compute(&segs, &tr, &layout(), &s, dur, 60.0);
        // Cursor jitter of ±3 px at 120 Hz must not leak into the camera: compare
        // against the same scenario with a perfectly clean cursor.
        let mut clean_tr = tr.clone();
        for (i, smp) in clean_tr.samples.iter_mut().enumerate() {
            let j = if i % 2 == 0 { 3.0 } else { -3.0 };
            smp.x -= j;
            smp.y += j;
        }
        let clean = CameraPath::compute(&segs, &clean_tr, &layout(), &s, dur, 60.0);
        let max_diff = path
            .frames
            .iter()
            .zip(&clean.frames)
            .map(|(a, b)| (a.x - b.x).abs().max((a.y - b.y).abs()))
            .fold(0.0, f64::max);
        assert!(max_diff < 0.002, "noise leaked into camera: {max_diff}");
        // Acceleration stays bounded (smooth motion, no spikes).
        let mut max_acc = 0.0f64;
        for w in path.frames.windows(3) {
            let ax = w[2].x - 2.0 * w[1].x + w[0].x;
            let ay = w[2].y - 2.0 * w[1].y + w[0].y;
            max_acc = max_acc.max(ax.abs()).max(ay.abs());
        }
        assert!(max_acc < 0.004, "max acceleration {max_acc}");
        // And no sign flips of velocity at high frequency while holding.
        let mut flips = 0;
        for w in path.frames.windows(3) {
            let v1 = w[1].x - w[0].x;
            let v2 = w[2].x - w[1].x;
            if v1.abs() > 1e-5 && v2.abs() > 1e-5 && v1.signum() != v2.signum() {
                flips += 1;
            }
        }
        assert!(flips < 6, "velocity sign flips: {flips}");
    }

    #[test]
    fn zoom_reaches_level_and_returns_to_identity() {
        let (segs, tr, s, dur) = scenario();
        let path = CameraPath::compute(&segs, &tr, &layout(), &s, dur, 60.0);
        let first = &segs[0];
        let mid = (first.start + first.end) / 2.0;
        let k = path.sample(mid);
        assert!((k.zoom - first.level).abs() < 1e-6);
        // The first click is visible when the zoom completes.
        let anchor = layout().source_norm_to_canvas(first.anchor.unwrap());
        let at = path.sample(first.anchor_time.unwrap() + 0.3);
        assert!(at.viewport().contains(anchor), "{at:?} {anchor:?}");
        // Outside all segments: identity.
        let last = path.frames.last().unwrap();
        if segs.iter().all(|s| s.end < dur - 0.1) {
            assert_eq!(last.zoom, 1.0);
            assert!((last.x - 0.5).abs() < 1e-9);
        }
        let before = path.sample(first.start - 0.05);
        assert_eq!(before.zoom, 1.0);
    }

    #[test]
    fn manual_focus_targets_fixed_point() {
        let tr = track(&[(0.0, 960.0, 540.0), (10.0, 960.0, 540.0)], vec![], 0.0);
        let seg = ZoomSegment {
            id: "m".into(),
            start: 1.0,
            end: 8.0,
            level: 3.0,
            focus: ZoomFocus::Manual { x: 0.8, y: 0.25 },
            enabled: true,
            auto: false,
            anchor: None,
            anchor_time: None,
        };
        let l = layout();
        let s = ZoomSettings::default();
        let path = CameraPath::compute(&[seg], &tr, &l, &s, 10.0, 60.0);
        let k = path.sample(5.0);
        let p = l.source_norm_to_canvas(Vec2::new(0.8, 0.25));
        assert!(k.viewport().contains(p));
        assert!((k.zoom - 3.0).abs() < 1e-9);
    }

    #[test]
    fn disabled_segments_are_ignored_and_output_is_deterministic() {
        let (mut segs, tr, s, dur) = scenario();
        let a = CameraPath::compute(&segs, &tr, &layout(), &s, dur, 60.0);
        let b = CameraPath::compute(&segs, &tr, &layout(), &s, dur, 60.0);
        assert_eq!(a, b);
        for seg in &mut segs {
            seg.enabled = false;
        }
        let c = CameraPath::compute(&segs, &tr, &layout(), &s, dur, 60.0);
        assert!(c.frames.iter().all(|f| f.zoom == 1.0));
    }

    #[test]
    fn short_segment_transitions_shrink() {
        let seg = ZoomSegment {
            id: "x".into(),
            start: 1.0,
            end: 1.6,
            level: 2.0,
            focus: ZoomFocus::Auto,
            enabled: true,
            auto: false,
            anchor: None,
            anchor_time: None,
        };
        let z = segment_zoom(&seg, 1.3, 0.8, Easing::EaseInOutCubic);
        assert!((z - 2.0).abs() < 1e-9, "peaks at the middle");
        assert_eq!(segment_zoom(&seg, 1.0, 0.8, Easing::Linear), 1.0);
        assert_eq!(segment_zoom(&seg, 1.6, 0.8, Easing::Linear), 1.0);
    }
}

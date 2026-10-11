//! Camera motion metrics for a realistic left↔right pan (see docs/QUALITY.md).
//!
//! A synthetic cursor makes human-like strokes: a minimum-jerk reach with a
//! small overshoot, a corrective sub-movement, and ±1 px sensor jitter at
//! 125 Hz. The camera is simulated at the export rate (120 Hz) and measured in
//! output pixels of a 1440p export.
//!
//! cargo run --release -p studio-core --example camera_motion

use studio_core::camera::{CameraPath, ZoomFocus, ZoomSegment, ZoomSettings};
use studio_core::cursor::{CursorSample, CursorTrack};
use studio_core::geometry::Vec2;
use studio_core::layout::{Layout, StyleSettings};

const W: u32 = 1920;
const H: u32 = 1080;
const OUT_W: f64 = 2560.0;
const RATE: f64 = 120.0;

fn min_jerk(f: f64) -> f64 {
    let f = f.clamp(0.0, 1.0);
    f * f * f * (10.0 + f * (-15.0 + 6.0 * f))
}

/// Human-like reach from `a` to `b` starting at `t0`: main stroke overshoots by
/// 4 %, then a short corrective stroke lands on `b`.
fn reach(t: f64, t0: f64, a: f64, b: f64) -> Option<f64> {
    let over = b + (b - a) * 0.04;
    let (d1, d2) = (0.7, 0.25);
    if t < t0 {
        None
    } else if t < t0 + d1 {
        Some(a + (over - a) * min_jerk((t - t0) / d1))
    } else if t < t0 + d1 + 0.08 {
        Some(over)
    } else if t < t0 + d1 + 0.08 + d2 {
        Some(over + (b - over) * min_jerk((t - t0 - d1 - 0.08) / d2))
    } else {
        Some(b)
    }
}

fn cursor_x(t: f64) -> f64 {
    // Rest at left, pan right at 3 s, pan back left at 7.5 s.
    if let Some(x) = reach(t, 7.5, 1650.0, 250.0) {
        return x;
    }
    reach(t, 3.0, 250.0, 1650.0).unwrap_or(250.0)
}

fn track() -> CursorTrack {
    let mut samples = Vec::new();
    let mut k = 0u64;
    let mut t = 0.0;
    while t <= 13.0 {
        // Deterministic pseudo-random jitter in [-1, 1] px.
        let h = (k.wrapping_mul(2654435761) % 1000) as f32 / 500.0 - 1.0;
        samples.push(CursorSample { t, x: cursor_x(t) as f32 + h, y: 540.0 - h * 0.7, shape: 0, visible: true });
        k += 1;
        t += 1.0 / 125.0;
    }
    CursorTrack { version: 1, source_width: W, source_height: H, samples, clicks: vec![], shapes: vec![] }
}

struct Metrics {
    peak_speed: f64,
    peak_acc: f64,
    peak_jerk: f64,
    reversals: usize,
    settle: f64,
    creep: f64,
    stop_jump: f64,
}

/// Metrics of x over [t0, t1) in output px. `stroke_end` is when the cursor stops.
fn measure(xs: &[f64], t0: f64, t1: f64, stroke_end: f64) -> Metrics {
    let (i0, i1) = ((t0 * RATE) as usize, (t1 * RATE) as usize);
    let dt = 1.0 / RATE;
    let v: Vec<f64> = (i0..i1).map(|i| (xs[i + 1] - xs[i]) / dt).collect();
    let a: Vec<f64> = v.windows(2).map(|w| (w[1] - w[0]) / dt).collect();
    let j: Vec<f64> = a.windows(2).map(|w| (w[1] - w[0]) / dt).collect();
    let peak_speed = v.iter().fold(0.0f64, |m, x| m.max(x.abs()));
    let mut reversals = 0;
    let mut sign = 0.0;
    for &x in &v {
        if x.abs() > 1.0 {
            // > 1 px/s
            if sign != 0.0 && x.signum() != sign {
                reversals += 1;
            }
            sign = x.signum();
        }
    }
    // Settle: last frame moving more than 0.01 px/frame, measured from the cursor stop.
    let last_moving = v.iter().rposition(|x| x.abs() * dt > 0.01).unwrap_or(0);
    let settle = (i0 + last_moving) as f64 * dt - stroke_end;
    // Creep: time spent crawling at 0.01–0.5 px per frame after the cursor stopped.
    let si = ((stroke_end * RATE) as usize).saturating_sub(i0);
    let creep = v[si.min(v.len())..].iter().filter(|x| (0.01..0.5).contains(&(x.abs() * dt))).count() as f64 * dt;
    // Largest per-frame velocity drop in the last 10 frames before rest (sudden stop).
    let stop_jump = (last_moving.saturating_sub(10)..=last_moving.min(v.len() - 2))
        .map(|i| (v[i + 1] - v[i]).abs() * dt)
        .fold(0.0, f64::max);
    Metrics {
        peak_speed,
        peak_acc: a.iter().fold(0.0f64, |m, x| m.max(x.abs())),
        peak_jerk: j.iter().fold(0.0f64, |m, x| m.max(x.abs())),
        reversals,
        settle,
        creep,
        stop_jump,
    }
}

fn main() {
    let layout = Layout::compute(W, H, &StyleSettings::default());
    let settings = ZoomSettings::default();
    let seg = ZoomSegment {
        id: "pan".into(),
        start: 0.5,
        end: 12.5,
        level: 2.0,
        focus: ZoomFocus::Auto,
        enabled: true,
        auto: false,
        anchor: Some(Vec2::new(250.0 / W as f64, 0.5)),
        anchor_time: Some(1.0),
    };
    let tr = track();
    let path = CameraPath::compute(&[seg], &tr, &layout, &settings, 13.0, RATE);
    let xs: Vec<f64> = path.frames.iter().map(|f| f.x * OUT_W * f.zoom).collect();
    let stroke = 0.7 + 0.08 + 0.25;
    println!("{:<14}{:>10}{:>12}{:>14}{:>10}{:>10}{:>10}{:>12}", "pan", "speed", "accel", "jerk", "revs", "settle", "creep", "stop-jump");
    println!("{:<14}{:>10}{:>12}{:>14}{:>10}{:>10}{:>10}{:>12}", "", "px/s", "px/s²", "px/s³", "", "s", "s", "px/frame");
    for (name, t0, t1) in [("left→right", 3.0, 7.4), ("right→left", 7.5, 11.6)] {
        let m = measure(&xs, t0, t1, t0 + stroke);
        println!(
            "{:<14}{:>10.0}{:>12.0}{:>14.0}{:>10}{:>10.2}{:>10.2}{:>12.4}",
            name, m.peak_speed, m.peak_acc, m.peak_jerk, m.reversals, m.settle, m.creep, m.stop_jump
        );
    }
    if std::env::args().any(|a| a == "--csv") {
        println!("t,cursor,camera");
        for (i, x) in xs.iter().enumerate() {
            let t = i as f64 / RATE;
            println!("{t:.4},{:.3},{x:.4}", cursor_x(t));
        }
    }
}

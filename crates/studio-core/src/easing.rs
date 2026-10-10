//! Easing curves and critically damped smoothing.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Easing {
    Linear,
    #[default]
    EaseInOutCubic,
    EaseInOutQuint,
    EaseOutCubic,
    /// Ken Perlin's smootherstep (zero 1st and 2nd derivative at both ends).
    Smoother,
}

impl Easing {
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::EaseInOutCubic => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            Easing::EaseInOutQuint => {
                if t < 0.5 {
                    16.0 * t.powi(5)
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(5) / 2.0
                }
            }
            Easing::EaseOutCubic => 1.0 - (1.0 - t).powi(3),
            Easing::Smoother => t * t * t * (t * (t * 6.0 - 15.0) + 10.0),
        }
    }
}

pub fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    if edge1 <= edge0 {
        return if x >= edge1 { 1.0 } else { 0.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Critically damped spring step (Game Programming Gems 4, "Critically Damped
/// Ease-In/Ease-Out Smoothing"). Unconditionally stable for any `dt`, never
/// overshoots a fixed target, and is deterministic.
///
/// `smooth_time` is approximately the time to reach the target. Returns the new
/// value and updates `velocity` in place.
pub fn smooth_damp(current: f64, target: f64, velocity: &mut f64, smooth_time: f64, dt: f64) -> f64 {
    if smooth_time <= 1e-6 {
        *velocity = 0.0;
        return target;
    }
    let omega = 2.0 / smooth_time;
    let x = omega * dt;
    let exp = 1.0 / (1.0 + x + 0.48 * x * x + 0.235 * x * x * x);
    let change = current - target;
    let temp = (*velocity + omega * change) * dt;
    *velocity = (*velocity - omega * temp) * exp;
    let mut out = target + (change + temp) * exp;
    // Prevent overshoot.
    if (target - current > 0.0) == (out > target) {
        out = target;
        *velocity = (out - target) / dt.max(1e-9);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easings_hit_endpoints_and_are_monotonic() {
        for e in [
            Easing::Linear,
            Easing::EaseInOutCubic,
            Easing::EaseInOutQuint,
            Easing::EaseOutCubic,
            Easing::Smoother,
        ] {
            assert!((e.apply(0.0)).abs() < 1e-12, "{e:?}");
            assert!((e.apply(1.0) - 1.0).abs() < 1e-12, "{e:?}");
            let mut prev = 0.0;
            for i in 1..=1000 {
                let v = e.apply(i as f64 / 1000.0);
                assert!(v + 1e-12 >= prev, "{e:?} not monotonic");
                prev = v;
            }
        }
    }

    #[test]
    fn smooth_damp_converges_without_overshoot() {
        let mut v = 0.0;
        let mut x = 0.0;
        let mut max = 0.0f64;
        for _ in 0..600 {
            x = smooth_damp(x, 1.0, &mut v, 0.3, 1.0 / 60.0);
            max = max.max(x);
        }
        assert!(max <= 1.0 + 1e-9);
        assert!((x - 1.0).abs() < 1e-4);
    }

    #[test]
    fn smooth_damp_is_stable_with_large_steps() {
        let mut v = 0.0;
        let mut x = 0.0;
        for _ in 0..50 {
            x = smooth_damp(x, 10.0, &mut v, 0.05, 1.0);
            assert!(x.is_finite() && x <= 10.0 + 1e-9);
        }
    }
}

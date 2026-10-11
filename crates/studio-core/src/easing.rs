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

/// Critically damped spring step, solved exactly for a target that is constant
/// over the step: `x(t) = (x0 + (v0 + ω·x0)·t)·e^(−ωt)`. Unconditionally
/// stable for any `dt`, deterministic, and velocity stays continuous (there is
/// no overshoot clamp that would zero it and cause a sudden stop). From rest it
/// never overshoots a fixed target.
///
/// `smooth_time` is the lag behind a target moving at constant speed (and
/// roughly the time to cover most of a step). Returns the new value and
/// updates `velocity` in place.
pub fn smooth_damp(current: f64, target: f64, velocity: &mut f64, smooth_time: f64, dt: f64) -> f64 {
    if smooth_time <= 1e-6 {
        *velocity = 0.0;
        return target;
    }
    if dt <= 0.0 {
        return current;
    }
    let omega = 2.0 / smooth_time;
    let x0 = current - target;
    let b = *velocity + omega * x0;
    let e = (-omega * dt).exp();
    *velocity = (*velocity - omega * b * dt) * e;
    target + (x0 + b * dt) * e
}

/// [`smooth_damp`] for a target that moves linearly from `from` to `to` during
/// the step, solved exactly. Sampling a moving target and holding it for the
/// step (as `smooth_damp` must) makes the result lead by half a step, so paths
/// computed at different rates (60 Hz preview, 120 Hz export) drift apart;
/// with a linear ramp they agree to second order.
pub fn smooth_damp_ramp(current: f64, from: f64, to: f64, velocity: &mut f64, smooth_time: f64, dt: f64) -> f64 {
    if smooth_time <= 1e-6 {
        *velocity = 0.0;
        return to;
    }
    if dt <= 0.0 {
        return current;
    }
    let omega = 2.0 / smooth_time;
    let k = (to - from) / dt;
    // Steady-state distance behind a target moving at speed `k`.
    let lag = 2.0 * k / omega;
    // The offset from that steady state decays like a spring with a fixed target.
    let mut du = *velocity - k;
    let u = smooth_damp(current - from + lag, 0.0, &mut du, smooth_time, dt);
    *velocity = du + k;
    to - lag + u
}

/// Position + velocity of a critically damped spring. Targets given to
/// successive steps are joined linearly ([`smooth_damp_ramp`]).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Spring {
    pub pos: f64,
    pub vel: f64,
    target: f64,
}

impl Spring {
    pub fn at(pos: f64) -> Self {
        Self { pos, vel: 0.0, target: pos }
    }

    pub fn step(&mut self, target: f64, smooth_time: f64, dt: f64) -> f64 {
        self.pos = smooth_damp_ramp(self.pos, self.target, target, &mut self.vel, smooth_time, dt);
        self.target = target;
        self.pos
    }

    /// Come to rest exactly on `target` once both the remaining distance and
    /// the motion per step are below `eps`. Removes the endless sub-pixel
    /// exponential tail (which shimmers through the resampler) with a jump far
    /// below one pixel.
    fn settle(&mut self, target: f64, eps: f64, dt: f64) {
        if (self.pos - target).abs() < eps && (self.vel * dt).abs() < eps {
            self.pos = target;
            self.vel = 0.0;
        }
    }
}

/// Three critically damped springs in series (a 6th-order critically damped
/// filter) that settles to rest.
///
/// Compared with a single spring of the same lag, acceleration and jerk stay
/// continuous even when the target starts or stops abruptly. The velocity
/// profile is a near-symmetric bell, close to the minimum-jerk motion people
/// read as natural: a gentle start, natural deceleration, and an arrival
/// without the long creeping exponential tail. The impulse response is
/// non-negative, so a monotonic target gives monotonic motion: no overshoot,
/// no bounce back.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SmoothFollow {
    inner: Spring,
    mid: Spring,
    outer: Spring,
}

impl SmoothFollow {
    pub fn at(pos: f64) -> Self {
        Self { inner: Spring::at(pos), mid: Spring::at(pos), outer: Spring::at(pos) }
    }

    pub fn pos(&self) -> f64 {
        self.outer.pos
    }

    pub fn vel(&self) -> f64 {
        self.outer.vel
    }

    pub fn at_rest(&self) -> bool {
        self.inner.vel == 0.0 && self.mid.vel == 0.0 && self.outer.vel == 0.0 && self.inner.pos == self.outer.pos && self.mid.pos == self.outer.pos
    }

    /// Advance by `dt` towards `target`. `smooth_time` is the total lag behind a
    /// target moving at constant speed (split between the stages);
    /// `rest_eps` is the distance below which the motion snaps to rest.
    pub fn step(&mut self, target: f64, smooth_time: f64, dt: f64, rest_eps: f64) -> f64 {
        let part = smooth_time / 3.0;
        self.inner.step(target, part, dt);
        self.inner.settle(target, rest_eps, dt);
        self.mid.step(self.inner.pos, part, dt);
        self.mid.settle(self.inner.pos, rest_eps, dt);
        self.outer.step(self.mid.pos, part, dt);
        self.outer.settle(self.mid.pos, rest_eps, dt);
        self.outer.pos
    }
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
    fn smooth_damp_is_exact_and_rate_independent() {
        // One big step equals many small ones for a fixed target.
        let (mut v1, mut v2) = (0.3, 0.3);
        let a = smooth_damp(0.0, 1.0, &mut v1, 0.4, 0.5);
        let mut b = 0.0;
        for _ in 0..500 {
            b = smooth_damp(b, 1.0, &mut v2, 0.4, 0.001);
        }
        assert!((a - b).abs() < 1e-12 && (v1 - v2).abs() < 1e-12, "{a} {b} {v1} {v2}");
    }

    #[test]
    fn smooth_damp_keeps_velocity_continuous_on_arrival() {
        // Tracking a target that moves then stops: no sudden velocity drop.
        let dt = 1.0 / 120.0;
        let (mut x, mut v) = (0.0, 0.0);
        let mut prev_v = 0.0f64;
        for i in 0..600 {
            let t = i as f64 * dt;
            let target = t.min(1.0);
            x = smooth_damp(x, target, &mut v, 0.3, dt);
            assert!((v - prev_v).abs() * dt < 0.002, "velocity jump at {t}");
            assert!(x <= 1.0 + 1e-12, "overshoot at {t}");
            prev_v = v;
        }
    }

    #[test]
    fn ramp_steps_are_rate_independent() {
        // Target moving at constant speed: the ramp solution is exact at any
        // rate; holding the target per step leads by half a step.
        let target = |t: f64| 2.0 * t;
        let ramp = |rate: f64| {
            let mut s = Spring::at(0.0);
            for i in 1..=(rate / 2.0) as usize {
                s.step(target(i as f64 / rate), 0.3, 1.0 / rate);
            }
            s.pos
        };
        let held = |rate: f64| {
            let (mut x, mut v) = (0.0, 0.0);
            for i in 1..=(rate / 2.0) as usize {
                x = smooth_damp(x, target(i as f64 / rate), &mut v, 0.3, 1.0 / rate);
            }
            x
        };
        let ramp_err = (ramp(60.0) - ramp(240.0)).abs();
        let held_err = (held(60.0) - held(240.0)).abs();
        assert!(ramp_err < 1e-9, "ramp {ramp_err}");
        assert!(held_err > 1e-3, "held {held_err}");
        // A constant target matches smooth_damp exactly.
        let mut v = 0.5;
        let mut s = Spring { pos: 0.2, vel: 0.5, target: 1.0 };
        let x = smooth_damp(0.2, 1.0, &mut v, 0.3, 0.01);
        assert!((s.step(1.0, 0.3, 0.01) - x).abs() < 1e-12 && (s.vel - v).abs() < 1e-12);
    }

    #[test]
    fn smooth_follow_is_monotonic_smooth_and_comes_to_rest() {
        let dt = 1.0 / 120.0;
        let mut f = SmoothFollow::at(0.0);
        let (mut prev, mut prev_v, mut prev_a) = (0.0f64, 0.0f64, 0.0f64);
        let mut rest_at = None;
        for i in 0..480 {
            let t = i as f64 * dt;
            // Target jumps (worst case for a single spring) then holds.
            let x = f.step(1.0, 0.4, dt, 1e-5);
            let v = (x - prev) / dt;
            let a = (v - prev_v) / dt;
            assert!(x >= prev - 1e-15 && x <= 1.0, "not monotonic at {t}");
            if i > 1 {
                // Acceleration changes gradually (continuous), even at the jump.
                assert!((a - prev_a).abs() * dt < 2.5, "acceleration step at {t}: {prev_a} -> {a}");
            }
            if rest_at.is_none() && f.at_rest() {
                rest_at = Some(t);
            }
            (prev, prev_v, prev_a) = (x, v, a);
        }
        let rest = rest_at.expect("never came to rest");
        assert!(rest < 2.0, "rest after {rest}s");
        assert_eq!(f.pos(), 1.0);
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

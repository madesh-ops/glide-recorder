//! Non-destructive timeline: an ordered list of clips over the source recording.
//!
//! Trimming adjusts the first/last clip, cutting splits a clip and removes the
//! middle part, and each clip has its own playback speed. Time mapping between
//! source time (the recording) and output time (the exported video) is exact
//! and is shared by preview and export.

use serde::{Deserialize, Serialize};

pub const MIN_CLIP: f64 = 0.1;
pub const MIN_SPEED: f64 = 0.25;
pub const MAX_SPEED: f64 = 16.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: String,
    /// Source start time (seconds into the recording).
    pub start: f64,
    /// Source end time.
    pub end: f64,
    pub speed: f64,
}

impl Clip {
    pub fn source_len(&self) -> f64 {
        self.end - self.start
    }
    pub fn output_len(&self) -> f64 {
        self.source_len() / self.speed
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeline {
    /// Duration of the source recording in seconds.
    pub source_duration: f64,
    pub clips: Vec<Clip>,
}

fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

impl Timeline {
    pub fn new(source_duration: f64) -> Self {
        Self {
            source_duration,
            clips: vec![Clip { id: new_id(), start: 0.0, end: source_duration, speed: 1.0 }],
        }
    }

    /// Repair invariants after deserialization or UI edits: clips sorted,
    /// inside the source, non-overlapping, minimum length, speed bounded.
    pub fn normalize(&mut self) {
        let dur = self.source_duration.max(0.0);
        for c in &mut self.clips {
            c.start = c.start.clamp(0.0, dur);
            c.end = c.end.clamp(0.0, dur);
            if !c.speed.is_finite() {
                c.speed = 1.0;
            }
            c.speed = c.speed.clamp(MIN_SPEED, MAX_SPEED);
            if c.id.is_empty() {
                c.id = new_id();
            }
        }
        self.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
        let mut prev_end = 0.0f64;
        for c in &mut self.clips {
            if c.start < prev_end {
                c.start = prev_end;
            }
            prev_end = prev_end.max(c.end);
        }
        self.clips.retain(|c| c.end - c.start >= MIN_CLIP * 0.5);
        if self.clips.is_empty() && dur > 0.0 {
            self.clips.push(Clip { id: new_id(), start: 0.0, end: dur, speed: 1.0 });
        }
    }

    pub fn output_duration(&self) -> f64 {
        self.clips.iter().map(Clip::output_len).sum()
    }

    /// Output time → source time. Clamped to the timeline.
    pub fn output_to_source(&self, t: f64) -> f64 {
        let mut acc = 0.0;
        for c in &self.clips {
            let len = c.output_len();
            if t < acc + len {
                return c.start + (t - acc).max(0.0) * c.speed;
            }
            acc += len;
        }
        self.clips.last().map(|c| c.end).unwrap_or(0.0)
    }

    /// Source time → output time, or `None` if that moment was cut.
    pub fn source_to_output(&self, t: f64) -> Option<f64> {
        let mut acc = 0.0;
        for c in &self.clips {
            if t >= c.start && t <= c.end {
                return Some(acc + (t - c.start) / c.speed);
            }
            acc += c.output_len();
        }
        None
    }

    /// Split the clip containing source time `t`. Returns false if `t` is not
    /// inside a clip or would create a too-short piece.
    pub fn split(&mut self, t: f64) -> bool {
        let Some(i) = self.clips.iter().position(|c| t > c.start + MIN_CLIP && t < c.end - MIN_CLIP) else {
            return false;
        };
        let c = self.clips[i].clone();
        self.clips[i].end = t;
        self.clips.insert(i + 1, Clip { id: new_id(), start: t, end: c.end, speed: c.speed });
        true
    }

    /// Remove the source range `[a, b]` from the timeline.
    pub fn cut(&mut self, a: f64, b: f64) {
        let (a, b) = (a.min(b), a.max(b));
        let mut out = Vec::with_capacity(self.clips.len() + 1);
        for c in self.clips.drain(..) {
            if c.end <= a || c.start >= b {
                out.push(c);
                continue;
            }
            if c.start < a {
                out.push(Clip { id: c.id.clone(), start: c.start, end: a, speed: c.speed });
            }
            if c.end > b {
                out.push(Clip { id: new_id(), start: b, end: c.end, speed: c.speed });
            }
        }
        self.clips = out;
        self.normalize();
    }

    pub fn trim_start(&self) -> f64 {
        self.clips.first().map(|c| c.start).unwrap_or(0.0)
    }
    pub fn trim_end(&self) -> f64 {
        self.clips.last().map(|c| c.end).unwrap_or(self.source_duration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn identity_mapping() {
        let t = Timeline::new(10.0);
        assert!(approx(t.output_duration(), 10.0));
        assert!(approx(t.output_to_source(3.5), 3.5));
        assert_eq!(t.source_to_output(3.5), Some(3.5));
    }

    #[test]
    fn trim_cut_and_speed() {
        let mut t = Timeline::new(10.0);
        t.clips[0].start = 1.0; // trim start
        t.clips[0].end = 9.0; // trim end
        t.cut(3.0, 5.0); // remove 2 s
        assert_eq!(t.clips.len(), 2);
        assert!(approx(t.output_duration(), 6.0));
        assert!(approx(t.output_to_source(0.0), 1.0));
        assert!(approx(t.output_to_source(2.0), 5.0), "jumps over the cut");
        assert_eq!(t.source_to_output(4.0), None);
        t.clips[1].speed = 2.0; // 5..9 at 2x → 2 s
        assert!(approx(t.output_duration(), 4.0));
        assert!(approx(t.output_to_source(3.0), 7.0));
        assert!(approx(t.source_to_output(7.0).unwrap(), 3.0));
    }

    #[test]
    fn mapping_roundtrip_and_monotonic() {
        let mut t = Timeline::new(20.0);
        t.split(5.0);
        t.split(12.0);
        t.clips[1].speed = 3.0;
        t.clips[2].speed = 0.5;
        let n = 2000;
        let mut prev = -1.0;
        for i in 0..n {
            let o = t.output_duration() * i as f64 / n as f64;
            let s = t.output_to_source(o);
            assert!(s >= prev - 1e-9, "monotonic");
            prev = s;
            let back = t.source_to_output(s).unwrap();
            assert!((back - o).abs() < 1e-6);
        }
    }

    #[test]
    fn normalize_repairs_bad_input() {
        let mut t = Timeline {
            source_duration: 5.0,
            clips: vec![
                Clip { id: "".into(), start: 3.0, end: 9.0, speed: 100.0 },
                Clip { id: "b".into(), start: -1.0, end: 4.0, speed: f64::NAN },
            ],
        };
        t.normalize();
        assert_eq!(t.clips.len(), 2);
        assert!(approx(t.clips[0].start, 0.0));
        assert!(approx(t.clips[0].speed, 1.0));
        assert!(approx(t.clips[1].start, 4.0), "overlap removed");
        assert!(approx(t.clips[1].end, 5.0));
        assert!(approx(t.clips[1].speed, MAX_SPEED));
        assert!(!t.clips[1].id.is_empty());
    }

    #[test]
    fn cutting_everything_restores_a_clip() {
        let mut t = Timeline::new(5.0);
        t.cut(0.0, 5.0);
        assert_eq!(t.clips.len(), 1);
    }
}

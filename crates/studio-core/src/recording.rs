//! Recording state machine and the pause-aware recording clock.

use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum RecordingState {
    Idle,
    Preparing,
    Recording,
    Paused,
    Finalizing,
    Completed,
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordingEvent {
    Prepare,
    Started,
    Pause,
    Resume,
    Stop,
    Finalized,
    Fail(String),
    /// Return to idle after completion/failure, or abort preparation.
    Reset,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid recording transition from {from:?} on {event:?}")]
pub struct InvalidTransition {
    pub from: RecordingState,
    pub event: RecordingEvent,
}

impl RecordingState {
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Preparing | Self::Recording | Self::Paused | Self::Finalizing)
    }

    pub fn apply(&self, event: RecordingEvent) -> Result<RecordingState, InvalidTransition> {
        use RecordingEvent as E;
        use RecordingState as S;
        let next = match (self, &event) {
            (S::Idle | S::Completed | S::Failed { .. }, E::Prepare) => S::Preparing,
            (S::Preparing, E::Started) => S::Recording,
            (S::Recording, E::Pause) => S::Paused,
            (S::Paused, E::Resume) => S::Recording,
            (S::Recording | S::Paused, E::Stop) => S::Finalizing,
            (S::Finalizing, E::Finalized) => S::Completed,
            // Any active state can fail; failing twice keeps the first message.
            (S::Preparing | S::Recording | S::Paused | S::Finalizing, E::Fail(m)) => {
                S::Failed { message: m.clone() }
            }
            (S::Preparing | S::Completed | S::Failed { .. } | S::Idle, E::Reset) => S::Idle,
            _ => {
                return Err(InvalidTransition { from: self.clone(), event });
            }
        };
        Ok(next)
    }
}

/// Pause-aware clock. Times are passed in explicitly (as offsets from an
/// arbitrary epoch) so the logic is testable; the engine feeds `Instant`
/// differences.
#[derive(Debug, Clone, Default)]
pub struct RecordingClock {
    started: Option<Duration>,
    paused_at: Option<Duration>,
    paused_total: Duration,
}

impl RecordingClock {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn start(&mut self, now: Duration) {
        self.started = Some(now);
        self.paused_at = None;
        self.paused_total = Duration::ZERO;
    }
    pub fn pause(&mut self, now: Duration) {
        if self.started.is_some() && self.paused_at.is_none() {
            self.paused_at = Some(now);
        }
    }
    pub fn resume(&mut self, now: Duration) {
        if let Some(p) = self.paused_at.take() {
            self.paused_total += now.saturating_sub(p);
        }
    }
    pub fn is_paused(&self) -> bool {
        self.paused_at.is_some()
    }
    /// Recording time in seconds at wall time `now` (frozen while paused).
    pub fn elapsed(&self, now: Duration) -> f64 {
        let Some(start) = self.started else { return 0.0 };
        let effective_now = self.paused_at.unwrap_or(now);
        effective_now
            .saturating_sub(start)
            .saturating_sub(self.paused_total)
            .as_secs_f64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use RecordingEvent as E;
    use RecordingState as S;

    #[test]
    fn happy_path() {
        let s = S::Idle.apply(E::Prepare).unwrap();
        let s = s.apply(E::Started).unwrap();
        let s = s.apply(E::Pause).unwrap();
        assert_eq!(s, S::Paused);
        let s = s.apply(E::Resume).unwrap();
        let s = s.apply(E::Stop).unwrap();
        assert_eq!(s, S::Finalizing);
        let s = s.apply(E::Finalized).unwrap();
        assert_eq!(s, S::Completed);
        assert_eq!(s.apply(E::Prepare).unwrap(), S::Preparing);
    }

    #[test]
    fn invalid_transitions_rejected() {
        assert!(S::Idle.apply(E::Stop).is_err());
        assert!(S::Idle.apply(E::Pause).is_err());
        assert!(S::Recording.apply(E::Prepare).is_err());
        assert!(S::Paused.apply(E::Pause).is_err());
        assert!(S::Finalizing.apply(E::Pause).is_err());
        assert!(S::Completed.apply(E::Fail("x".into())).is_err());
    }

    #[test]
    fn failures_and_recovery() {
        let f = S::Recording.apply(E::Fail("disk full".into())).unwrap();
        assert_eq!(f, S::Failed { message: "disk full".into() });
        assert!(!f.is_active());
        assert_eq!(f.apply(E::Reset).unwrap(), S::Idle);
        assert_eq!(S::Finalizing.apply(E::Fail("x".into())).unwrap(), S::Failed { message: "x".into() });
    }

    #[test]
    fn clock_excludes_pauses() {
        let ms = Duration::from_millis;
        let mut c = RecordingClock::new();
        assert_eq!(c.elapsed(ms(5)), 0.0);
        c.start(ms(1000));
        assert!((c.elapsed(ms(3000)) - 2.0).abs() < 1e-9);
        c.pause(ms(3000));
        assert!((c.elapsed(ms(9000)) - 2.0).abs() < 1e-9, "frozen while paused");
        c.pause(ms(9500)); // double pause is a no-op
        c.resume(ms(10000));
        assert!((c.elapsed(ms(11000)) - 3.0).abs() < 1e-9);
        c.resume(ms(12000)); // resume while running is a no-op
        assert!((c.elapsed(ms(12000)) - 4.0).abs() < 1e-9);
    }
}

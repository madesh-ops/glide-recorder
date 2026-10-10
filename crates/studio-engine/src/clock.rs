//! Shared, thread-safe recording clock (QPC via `Instant`).

use parking_lot::Mutex;
use std::time::Instant;
use studio_core::recording::RecordingClock;

pub struct SessionClock {
    epoch: Instant,
    inner: Mutex<RecordingClock>,
}

impl Default for SessionClock {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionClock {
    pub fn new() -> Self {
        Self { epoch: Instant::now(), inner: Mutex::new(RecordingClock::new()) }
    }
    pub fn start(&self) {
        self.inner.lock().start(self.epoch.elapsed());
    }
    pub fn pause(&self) {
        self.inner.lock().pause(self.epoch.elapsed());
    }
    pub fn resume(&self) {
        self.inner.lock().resume(self.epoch.elapsed());
    }
    pub fn is_paused(&self) -> bool {
        self.inner.lock().is_paused()
    }
    /// Recording time now (frozen while paused).
    pub fn now(&self) -> f64 {
        self.inner.lock().elapsed(self.epoch.elapsed())
    }
    /// Recording time at `at`, or `None` while paused (events then are dropped).
    pub fn at(&self, at: Instant) -> Option<f64> {
        let c = self.inner.lock();
        if c.is_paused() {
            return None;
        }
        Some(c.elapsed(at.saturating_duration_since(self.epoch)))
    }
}

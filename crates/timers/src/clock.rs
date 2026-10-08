use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Where the timers read "now": a monotonic time since the clock's own start.
pub trait Clock: Send + Sync {
    fn now(&self) -> Duration;
}

/// The real clock: `std::time::Instant` (on Windows Rust reads QueryPerformanceCounter). Monotonic — changing the
/// Windows clock or time zone never moves a running timer.
#[derive(Debug, Clone, Copy)]
pub struct MonoClock {
    origin: Instant,
}

impl MonoClock {
    pub fn new() -> Self {
        MonoClock { origin: Instant::now() }
    }
    /// The `Instant` of a time on this clock (for [`crate::alarm::Alarm`]); `None` if it is too far to represent.
    pub fn instant_at(&self, t: Duration) -> Option<Instant> {
        self.origin.checked_add(t)
    }
}

impl Default for MonoClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for MonoClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

/// A clock tests move by hand. Clones share the same time.
#[derive(Debug, Clone, Default)]
pub struct FakeClock {
    t: Arc<Mutex<Duration>>,
}

impl FakeClock {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn advance(&self, d: Duration) {
        let mut t = self.t.lock().unwrap_or_else(|p| p.into_inner());
        *t += d;
    }
    pub fn advance_ms(&self, ms: u64) {
        self.advance(Duration::from_millis(ms));
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Duration {
        *self.t.lock().unwrap_or_else(|p| p.into_inner())
    }
}

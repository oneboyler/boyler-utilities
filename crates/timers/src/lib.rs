//! `bu-timers` — the Timers tab's features (DESIGN.md §3.17), no UI.
//!
//! * [`stopwatch`] — Start / Stop / Resume, Lap, Reset; laps newest first (lap + total), best lap marked.
//! * [`countdown`] — typed times ("5" = 5 min, "1:30", "90s", "1h 20m"), Start / Pause / Resume / Again, Reset, the
//!   draining line, the end event (toast text + chime if on).
//! * [`bars`]      — the on-screen timer bars' STATE (the card switch, each bar, start / start over, what the screen shows,
//!   the "On screen" look settings). Drawing them is a later UI order; keys are mapped by a later app-layer order.
//! * [`sound`]     — the soft end chime, made in memory (the drawing's three-tone recipe), played through a [`SoundOs`].
//! * [`zones`]     — the World clock: your time + places, from Windows' own time zone rules (read-only).
//! * [`alarm`]     — a thread that SLEEPS until the next deadline (no polling) and wakes the app once.
//!
//! The clock is [`MonoClock`] = `std::time::Instant` (Windows: QueryPerformanceCounter — monotonic, not moved by clock
//! changes; its smallest step on the build PC was 100 ns, measured by `timers-show`). Every logic test uses [`FakeClock`] (no real
//! time, no PC load); only `tests/alarm_real_thread.rs` uses the real clock and a real thread, with generous bounds.

pub mod alarm;
pub mod bars;
mod clock;
pub mod countdown;
mod error;
pub mod parse;
pub mod sound;
pub mod stopwatch;
pub mod zones;

pub use clock::{Clock, FakeClock, MonoClock};
pub use error::{Result, TimerError};
pub use sound::{FakeSound, SoundOs};
#[cfg(windows)]
pub use sound::RealSound;

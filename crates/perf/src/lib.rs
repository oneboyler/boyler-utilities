//! `bu-perf` — the Performance tab's features (DESIGN.md §3.8), no UI.
//!
//! * [`live`]      — the tiles: CPU usage + clock, GPU usage / VRAM / temperature / fan, RAM, disk, network. A
//!   [`live::Sampler`] the menu starts when the page opens and stops when it closes — no thread, no cost while stopped.
//!   No FPS (rejected). No CPU temperature (needs a risky driver — reported as unavailable).
//! * [`specs`]     — "Your PC": CPU, GPU, RAM + speed, motherboard, drives, displays, Windows. Read once.
//! * [`processes`] — the process list (one row per app, helpers counted), Windows' own processes locked, End task /
//!   End process tree with the "asks first" data, priority with undo.
//!
//! Every Windows call goes through [`PerfOs`]: [`RealOs`] (Windows) and [`FakeOs`] (tests).

mod error;
pub mod fake;
pub mod live;
mod os;
pub mod processes;
#[cfg(windows)]
pub mod real;
pub mod specs;

pub use error::{PerfError, Result};
pub use fake::FakeOs;
pub use os::*;
#[cfg(windows)]
pub use real::RealOs;

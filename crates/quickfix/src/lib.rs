//! `bu-quickfix` — the Quick fixes card (DESIGN.md §3.5), no UI. Nothing runs in the background (the card has no
//! switch); each fix runs only when its button is pressed.
//!
//! * [`gfx`]     — Reset graphics driver: the Win + Ctrl + Shift + B chord (only while our menu is in front), or the
//!   display-adapter restart (`pnputil /restart-device`, admin).
//! * [`repair`]  — Repair Windows files (admin): DISM RestoreHealth then sfc /scannow, progress, Cancel, result line.
//! * [`cache`]   — Rebuild icon & thumbnail cache: Explorer stopped → cache files deleted → Explorer started.
//! * [`restore`] — Make a restore point (admin), only on demand, with Windows' 24-hour rule said plainly.
//!
//! Undo: none of these can be undone by nature (a reset, a repair, a rebuilt cache, a new restore point) — each is
//! a one-shot action that changes no setting; nothing is stored to switch back.
//!
//! Every Windows call goes through [`FixOs`]: `real::RealOs` (Windows) and [`fake::FakeFixOs`] (tests).

pub mod cache;
mod error;
pub mod fake;
pub mod gfx;
mod os;
#[cfg(windows)]
pub mod real;
pub mod repair;
pub mod restore;

pub use error::{FixError, Result};
pub use os::*;

/// The four rows, for the card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fix {
    ResetGraphicsDriver,
    RepairWindowsFiles,
    RebuildIconCache,
    MakeRestorePoint,
}

impl Fix {
    pub const ALL: [Fix; 4] = [Fix::ResetGraphicsDriver, Fix::RepairWindowsFiles, Fix::RebuildIconCache, Fix::MakeRestorePoint];

    /// The shield "Needs admin — Windows asks once" (DESIGN marks rows 2 and 4 ADM). Row 1's chord needs none; its
    /// adapter-restart fallback does ([`gfx::restart_adapters`]).
    pub fn needs_admin(self) -> bool {
        matches!(self, Fix::RepairWindowsFiles | Fix::MakeRestorePoint)
    }
}

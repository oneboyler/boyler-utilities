//! Typed errors. Nothing in this crate panics on bad input or a failing Windows call.

use crate::types::{ChangeKind, GpuVendor, MonitorId, VideoMode};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DisplayError {
    #[error("no monitor with id {0}")]
    MonitorNotFound(MonitorId),

    /// Apply was asked for a W × H the monitor does not report. `nearest` is the closest size it does report.
    #[error("{width} × {height} is not a mode this monitor reports")]
    ModeNotSupported { width: u32, height: u32, nearest: Option<VideoMode> },

    #[error("the monitor reports no modes")]
    NoModes,

    #[error("{0}% is not a Windows scaling value this monitor offers")]
    DpiNotOffered(u32),

    #[error("the monitor doesn't answer DDC/CI")]
    DdcNoAnswer,

    #[error("DDC/CI is switched off for this monitor after a crash during a DDC call")]
    DdcBlockedAfterCrash,

    #[error("DDC/CI is never used on this monitor model (known to crash Windows; PowerToys' exclusion list)")]
    DdcExcludedModel,

    #[error("vibrance is not available on this graphics card ({0:?})")]
    VibranceUnsupported(Option<GpuVendor>),

    /// The change needs administrator rights. The crate never elevates; a later helper does.
    #[error("this change needs administrator rights ({0:?})")]
    NeedsAdmin(ChangeKind),

    /// The read-only OS layer (`WinDisplayOs::read_only`, proof programs) refuses every change.
    #[error("read-only: {0} refused (proof program; nothing is changed)")]
    ReadOnly(&'static str),

    #[error("nothing to undo")]
    NothingToUndo,

    #[error("no change is waiting for Keep / Revert")]
    NoPendingChange,

    /// A preset with the same W, H, scaling and (snapped) Hz already exists — the index of that chip.
    #[error("already a preset")]
    DuplicatePreset(usize),

    #[error("no preset with that id")]
    PresetNotFound,

    #[error("the app watcher could not start: {0}")]
    Watcher(String),

    /// A Windows call failed: which call and its error code / text.
    #[error("{call} failed: {detail}")]
    Os { call: &'static str, detail: String },
}

pub type Result<T> = std::result::Result<T, DisplayError>;

impl DisplayError {
    pub fn os(call: &'static str, detail: impl ToString) -> Self {
        DisplayError::Os { call, detail: detail.to_string() }
    }
}

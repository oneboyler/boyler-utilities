//! Typed errors. Nothing in this crate panics on a Windows failure; every failure comes back as one of these.

use std::path::PathBuf;

/// Everything that can go wrong capturing, copying, saving or managing shots.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// No monitor with this number (numbers start at 1).
    #[error("no monitor {0}")]
    NoMonitor(usize),

    /// The region does not touch any monitor, or is smaller than the 4 px minimum.
    #[error("region {0}")]
    BadRegion(&'static str),

    /// Windows gave no frame in time (e.g. the secure desktop / UAC prompt was up, or a full-screen exclusive game owns the output).
    #[error("no frame from monitor {monitor} within {ms} ms")]
    NoFrame { monitor: usize, ms: u32 },

    /// This capture method is not available on this PC (e.g. Windows.Graphics.Capture before Windows 10 1903).
    #[error("capture method {0} is not supported here")]
    MethodUnsupported(&'static str),

    /// The shot id is not in the gallery index (deleted, or the file vanished and was pruned).
    #[error("no shot with id {0} in the gallery")]
    UnknownShot(u64),

    /// The chosen save folder does not exist or is not a folder.
    #[error("not a folder: {0}")]
    NotAFolder(PathBuf),

    /// The file lives on a drive without a Recycle Bin (network share, some USB sticks). The engine never deletes
    /// permanently in that case — the menu decides what to say.
    #[error("no Recycle Bin on the drive of {0}")]
    NoRecycleBin(PathBuf),

    /// A file or index could not be read / written. `what` names the file, `why` is the OS text.
    #[error("{what}: {why}")]
    Io { what: String, why: String },

    /// A stored file is not what we wrote (a broken index line, a PNG we can't decode).
    #[error("bad data in {0}")]
    BadData(String),

    /// The OS layer was built read-only (examples/show) or without capture rights and refused.
    #[error("read-only OS layer refused: {0}")]
    ReadOnly(&'static str),

    /// The user closed the folder picker without choosing.
    #[error("cancelled")]
    Cancelled,

    /// A Windows call failed. `code` is the HRESULT / Win32 value.
    #[error("{op} failed (code {code:#x})")]
    Os { op: String, code: i64 },
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn os(op: impl Into<String>, code: impl Into<i64>) -> Self {
        Error::Os { op: op.into(), code: code.into() }
    }

    pub(crate) fn io(what: impl std::fmt::Display, e: &std::io::Error) -> Self {
        Error::Io { what: what.to_string(), why: e.to_string() }
    }
}

#[cfg(windows)]
impl From<windows_core::Error> for Error {
    fn from(e: windows_core::Error) -> Self {
        Error::Os { op: e.message(), code: e.code().0 as i64 }
    }
}

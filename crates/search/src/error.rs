//! Typed errors. Nothing in this crate panics on a Windows failure; every failure is one of these.

use crate::os::WsStatus;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SearchError {
    /// Windows Search (the index service) is not running / turned off, and Everything is not available:
    /// files and folders cannot be searched. Apps still can.
    #[error("Windows Search is not running ({0:?})")]
    WindowsSearchOff(WsStatus),
    /// Everything is there but did not answer (database still loading, closed meanwhile).
    #[error("Everything did not answer: {0}")]
    Everything(String),
    /// Everything is not on this PC: files and folders cannot be searched (apps still can). The page offers the install.
    #[error("Everything is not installed")]
    EverythingNotInstalled,
    /// Everything answers but its index is still loading ("catching up").
    #[error("Everything is still loading its index")]
    EverythingLoading,
    /// The install was cancelled (Windows' admin prompt answered No).
    #[error("the install was cancelled")]
    InstallCancelled,
    /// The install failed: the download, its check or the installer itself.
    #[error("{0}")]
    Install(String),
    /// The search was stopped by the caller (a newer keystroke).
    #[error("cancelled")]
    Cancelled,
    /// The item cannot do this (an app without a file location has no folder to open).
    #[error("not supported: {0}")]
    Unsupported(String),
    /// A read-only layer refused a change (open, clipboard).
    #[error("refused by the read-only layer: {0}")]
    Refused(String),
    /// A Windows call failed: what was called, the code, and what it said.
    #[error("{call} failed ({code:#010x}): {text}")]
    Os { call: String, code: u32, text: String },
}

pub type Result<T> = std::result::Result<T, SearchError>;

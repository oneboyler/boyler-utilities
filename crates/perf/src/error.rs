/// Every error the Performance features return. Nothing in this crate panics on purpose.
#[derive(Debug, thiserror::Error)]
pub enum PerfError {
    /// Needs administrator rights (elevated apps, services, other users' processes). This crate never elevates.
    #[error("needs admin: {0}")]
    NeedsAdmin(String),
    /// Windows' own process — it is locked and can't be ended (DESIGN: no End).
    #[error("{0} is part of Windows and is locked")]
    Locked(String),
    /// Refused on purpose (Realtime priority, a priority change on an anti-cheat game …).
    #[error("refused: {0}")]
    Refused(String),
    /// The process is gone (it closed by itself).
    #[error("not found: {0}")]
    NotFound(String),
    /// A value this PC can't give without a driver / vendor library (CPU temperature …).
    #[error("unavailable: {0}")]
    Unavailable(String),
    /// A Windows call failed; `code` is the Win32 / HRESULT / NTSTATUS / PDH value.
    #[error("{context} failed (0x{code:08X})")]
    Os { context: String, code: u32 },
}

pub type Result<T> = std::result::Result<T, PerfError>;

use std::path::PathBuf;

/// Every error the Storage features return. Nothing in this crate panics on purpose.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// The step needs administrator rights. This crate never elevates; a later helper will.
    #[error("needs admin: {0}")]
    NeedsAdmin(String),
    /// The drive is locked (BitLocker) — nothing on it can be read.
    #[error("drive {0}: is locked")]
    Locked(char),
    /// One of Windows' own places (Windows, WindowsApps, System Volume Information, $Recycle.Bin) — can't be opened.
    #[error("Windows' own place, it manages this itself")]
    WindowsOwn,
    /// Clean was asked before the sizes were measured (DESIGN: nothing can be cleaned before it is measured).
    #[error("sizes are not measured yet")]
    NotMeasured,
    /// A path the cleaner refuses to touch (outside its own target folders, or too shallow to be safe).
    #[error("refused unsafe path {0}")]
    UnsafePath(PathBuf),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("cancelled")]
    Cancelled,
    #[error("{0} is not supported here")]
    Unsupported(String),
    /// A Windows call failed; `code` is the Win32 / HRESULT / NTSTATUS value.
    #[error("{context} failed (0x{code:08X})")]
    Os { context: String, code: u32 },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, StorageError>;

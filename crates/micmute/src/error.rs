/// Every error the Mic mute features return. Nothing in this crate panics on purpose.
#[derive(Debug, thiserror::Error)]
pub enum MicError {
    /// No microphone to work on (none plugged in / the picked one is gone).
    #[error("no microphone: {0}")]
    NoMic(String),
    /// Refused on purpose by a read-only OS layer (`examples/show`), or a change this crate never makes.
    #[error("read-only: {0}")]
    ReadOnly(String),
    /// Something this PC doesn't have (…).
    #[error("unavailable: {0}")]
    Unavailable(String),
    /// A Windows call failed; `code` is the Win32 / HRESULT value.
    #[error("{context} failed (0x{code:08X})")]
    Os { context: String, code: u32 },
}

pub type Result<T> = std::result::Result<T, MicError>;

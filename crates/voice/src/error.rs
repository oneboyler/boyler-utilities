/// Every error Voice to text returns. Nothing in this crate panics on purpose.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VoiceError {
    /// Refused on purpose (a test / read-only layer).
    #[error("read-only: {0}")]
    ReadOnly(String),
    /// A Windows call failed; `code` is the HRESULT / Win32 value.
    #[error("{context} failed (0x{code:08X})")]
    Os { context: String, code: u32 },
}

pub type Result<T> = std::result::Result<T, VoiceError>;

#[cfg(windows)]
pub(crate) fn os(context: &str, e: windows_core::Error) -> VoiceError {
    VoiceError::Os { context: context.to_string(), code: e.code().0 as u32 }
}

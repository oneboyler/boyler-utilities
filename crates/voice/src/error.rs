/// Every error Voice to text returns. Nothing in this crate panics on purpose.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VoiceError {
    /// Windows has no speech recognizer for this language (or none at all).
    #[error("no speech recognizer for {0}")]
    NoLanguage(String),
    /// No microphone Windows can record from.
    #[error("no microphone")]
    NoMic,
    /// Windows' "Online speech recognition" switch (Settings › Privacy & security › Speech) is off - the dictation service
    /// (the one Windows' voice typing uses) is refused (0x80045509).
    #[error("online speech recognition is off")]
    OnlineOff,
    /// Windows' microphone privacy switch blocks this app (Settings › Privacy & security › Microphone).
    #[error("microphone access is blocked")]
    MicDenied,
    /// The online dictation service could not be reached (no internet).
    #[error("no internet connection")]
    Offline,
    /// A dictation is already running.
    #[error("already listening")]
    Busy,
    /// Refused on purpose (a test / read-only layer).
    #[error("read-only: {0}")]
    ReadOnly(String),
    /// A file (a WAV in tests) could not be read.
    #[error("file: {0}")]
    File(String),
    /// A Windows call failed; `code` is the HRESULT / Win32 value.
    #[error("{context} failed (0x{code:08X})")]
    Os { context: String, code: u32 },
}

impl VoiceError {
    /// A Windows switch / a missing microphone stops every dictation until the user changes it (the page shows it in the
    /// words box, not as a passing toast).
    pub fn blocks(&self) -> bool {
        matches!(self, VoiceError::OnlineOff | VoiceError::MicDenied | VoiceError::NoMic)
    }
}

pub type Result<T> = std::result::Result<T, VoiceError>;

/// The speech privacy policy was not accepted (= Online speech recognition off; sperror.h SPERR_SPEECH_PRIVACY_POLICY_NOT_ACCEPTED).
pub const HR_PRIVACY_DECLINED: u32 = 0x8004_5509;
/// E_ACCESSDENIED: the microphone privacy switch blocks the app.
pub const HR_ACCESS_DENIED: u32 = 0x8007_0005;
/// MF_E_NO_CAPTURE_DEVICES_AVAILABLE: no microphone.
pub const HR_NO_CAPTURE_DEVICE: u32 = 0xC00D_ABE0;

/// A Windows error code -> the error the page explains (the three blockers), else `Os`.
pub fn from_code(context: &str, code: u32) -> VoiceError {
    match code {
        HR_PRIVACY_DECLINED => VoiceError::OnlineOff,
        HR_ACCESS_DENIED => VoiceError::MicDenied,
        HR_NO_CAPTURE_DEVICE => VoiceError::NoMic,
        _ => VoiceError::Os { context: context.to_string(), code },
    }
}

#[cfg(windows)]
pub(crate) fn os(context: &str, e: windows_core::Error) -> VoiceError {
    from_code(context, e.code().0 as u32)
}

/// Every error the Audio features return. Nothing in this crate panics on purpose.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AudioError {
    /// Windows answered "access denied" — needs administrator rights. This crate never elevates.
    #[error("needs admin: {0}")]
    NeedsAdmin(String),
    /// No device / app with this id (unplugged, closed).
    #[error("not found: {0}")]
    NotFound(String),
    /// The device is switched off: clicking its row doesn't select it (DESIGN: the switch only nudges).
    #[error("device is switched off: {0}")]
    DeviceOff(String),
    /// "One device always stays on": the last device still on can't be switched off.
    #[error("one device always stays on")]
    LastDeviceOn,
    /// A read-only OS layer (examples) refused a change.
    #[error("read-only: refused {0}")]
    ReadOnly(String),
    /// Core Audio isn't there (no audio service) or a value can't be read.
    #[error("unavailable: {0}")]
    Unavailable(String),
    /// A Windows call failed; `code` is the HRESULT.
    #[error("{context} failed (0x{code:08X})")]
    Os { context: String, code: u32 },
}

pub type Result<T> = std::result::Result<T, AudioError>;

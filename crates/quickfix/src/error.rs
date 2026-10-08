/// Every error the Quick fixes return. Nothing in this crate panics on purpose.
#[derive(Debug, thiserror::Error)]
pub enum FixError {
    /// Needs administrator rights ("Needs admin — Windows asks once"). This crate never elevates; a later helper does.
    #[error("needs admin: {0}")]
    NeedsAdmin(String),
    /// Refused on purpose (a read-only OS layer, or a safety rule — e.g. no key chord while another app is in front).
    #[error("refused: {0}")]
    Refused(String),
    /// Something this PC doesn't have / Windows can't do here.
    #[error("unavailable: {0}")]
    Unavailable(String),
    /// A Windows call failed; `code` is the Win32 / HRESULT value.
    #[error("{context} failed (0x{code:08X})")]
    Os { context: String, code: u32 },
}

pub type Result<T> = std::result::Result<T, FixError>;

//! What can go wrong. Every text is a plain sentence the page can show as it is.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddonError {
    /// No connection / the server stopped answering.
    Network(String),
    /// The download was not the pinned release (size or SHA-256), or the driver's signature did not check out.
    Verify(String),
    /// Unpacking or saving the files failed.
    Files(String),
    /// Windows' admin prompt was answered No.
    Declined,
    /// The elevated helper or Raw Accel's own tool reported a failure.
    Tool(String),
    /// The user pressed Cancel.
    Cancelled,
}

pub type Result<T> = std::result::Result<T, AddonError>;

impl fmt::Display for AddonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AddonError::Network(s) => write!(f, "The download failed: {s}"),
            AddonError::Verify(s) => write!(f, "{s} - nothing was installed"),
            AddonError::Files(s) => write!(f, "Could not save the files: {s}"),
            AddonError::Declined => write!(f, "The admin prompt was declined - nothing was changed"),
            AddonError::Tool(s) => write!(f, "{s}"),
            AddonError::Cancelled => write!(f, "Download cancelled"),
        }
    }
}

impl std::error::Error for AddonError {}

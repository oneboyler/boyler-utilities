/// Every error the updater returns. Nothing in this crate panics on purpose. Details are plain strings so the type stays
/// `Clone + Eq` (the menu can keep the last one to show).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpdateError {
    /// The repo setting is not `owner/name` (letters, digits, `.`, `_`, `-`).
    #[error("the update source {0:?} is not written as owner/name")]
    InvalidRepo(String),
    /// A version string (the app's own, or a release tag) can't be read.
    #[error("{0:?} is not a version number")]
    BadVersion(String),
    /// No connection / DNS / TLS / timeout. `context` says which step.
    #[error("no connection ({context}): {detail}")]
    Network { context: String, detail: String },
    /// GitHub refused because too many questions were asked from this address (HTTP 403 / 429).
    #[error("GitHub says too many requests - try again later")]
    RateLimited,
    /// Any other HTTP answer that is not "ok".
    #[error("the server answered {status} ({what})")]
    HttpStatus { status: u16, what: String },
    /// The answer arrived but isn't what the GitHub releases API sends.
    #[error("unreadable answer: {0}")]
    BadResponse(String),
    /// A newer release exists but has no Windows `.exe` file attached.
    #[error("release {tag} has no Windows file attached")]
    NoWindowsAsset { tag: String },
    /// The download link isn't https while the source is https (never downgrade).
    #[error("refusing a download over an insecure link: {0}")]
    InsecureDownload(String),
    /// The release's file is bigger than the safety limit.
    #[error("the file is {size} bytes, over the {limit}-byte limit")]
    TooLarge { size: u64, limit: u64 },
    /// What arrived is not the size the release lists.
    #[error("the download is {got} bytes, the release lists {expected}")]
    SizeMismatch { expected: u64, got: u64 },
    /// SHA-256 of the download differs from the one GitHub lists.
    #[error("the download's SHA-256 does not match (expected {expected}, got {got})")]
    HashMismatch { expected: String, got: String },
    /// The download isn't a Windows program (no `MZ` header) - e.g. an error page served with status 200.
    #[error("the download is not a Windows program")]
    NotAnExecutable,
    /// The app's folder can't be written (Program Files without admin, read-only...). The update is not attempted.
    #[error("the app's folder can't be written: {0}")]
    InstallDirNotWritable(String),
    /// A file operation failed.
    #[error("{context}: {detail}")]
    Io { context: String, detail: String },
    /// `cancel()` was called while the update was still downloading or checking; everything it had written is removed.
    #[error("the update was cancelled")]
    Cancelled,
    /// `update()` was called while another `update()` of the same `Updater` is still running.
    #[error("an update is already in progress")]
    AlreadyRunning,
    /// The small step that replaces the files could not be started.
    #[error("could not start the install step: {0}")]
    HelperStart(String),
}

pub type Result<T> = std::result::Result<T, UpdateError>;

impl UpdateError {
    pub(crate) fn io(context: impl Into<String>, e: &std::io::Error) -> Self {
        UpdateError::Io { context: context.into(), detail: e.to_string() }
    }
}

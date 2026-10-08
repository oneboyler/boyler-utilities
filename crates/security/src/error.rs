//! Typed errors. Nothing in this crate panics on a Windows failure; every failure is one of these.

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecurityError {
    /// The action needs admin rights and this process is not elevated (or Windows answered "access denied").
    /// Nothing was changed. (A later order adds the small elevated helper.)
    #[error("needs admin")]
    NeedsAdmin,
    /// Another antivirus is the active one; Defender stands by and cannot scan or remediate.
    #[error("{0} protects this PC; Defender is standing by")]
    OtherAntivirus(String),
    /// Defender's service is not running / not available on this PC.
    #[error("Microsoft Defender is not running")]
    DefenderNotRunning,
    /// A scan is already running; only one at a time.
    #[error("a scan is already running")]
    ScanRunning,
    /// There is no scan to cancel.
    #[error("no scan is running")]
    NoScanRunning,
    /// The offline scan restarts the PC; it only starts with the explicit confirmation.
    #[error("the offline scan restarts the PC and needs your confirmation first")]
    RestartNotConfirmed,
    /// The file or folder to scan is not there.
    #[error("not found: {0}")]
    PathMissing(String),
    /// The threat / quarantine item is not (or no longer) in Defender's list.
    #[error("no such threat: {0}")]
    NoSuchThreat(i64),
    /// Windows has no supported way to do this.
    #[error("not supported: {0}")]
    Unsupported(String),
    /// The change id was already undone or never existed / cannot be undone.
    #[error("nothing to undo")]
    NothingToUndo,
    /// A Windows program or call failed: what was called, the code, and what it said.
    #[error("{call} failed ({code:#010x}): {text}")]
    Os { call: String, code: u32, text: String },
    /// The app's elevated copy (Order 039) could not make the change: its own words.
    #[error("{0}")]
    Admin(String),
}

pub type Result<T> = std::result::Result<T, SecurityError>;

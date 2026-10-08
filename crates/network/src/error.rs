//! Typed errors. Nothing in this crate panics on a Windows failure; every failure is one of these.

/// Everything that can go wrong in the Network features.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NetError {
    /// The change needs admin rights and this process is not elevated. Nothing was changed.
    /// (A later order adds the small elevated helper; until then the menu shows the shield and stops here.)
    #[error("needs admin")]
    NeedsAdmin,
    /// No adapter carries a route to the internet.
    #[error("you're offline")]
    Offline,
    /// The adapter id is not (or no longer) on this PC.
    #[error("no such adapter: {0}")]
    NoSuchAdapter(String),
    /// The PC has no Wi-Fi radio (desktop without Wi-Fi).
    #[error("no Wi-Fi radio on this PC")]
    NoWifiRadio,
    /// Windows refused the change (e.g. radio access denied by privacy settings, or a policy).
    #[error("Windows denied access: {0}")]
    AccessDenied(String),
    /// No answer in time (ping, connect, download).
    #[error("timed out")]
    Timeout,
    /// The target answered "unreachable" or refused the connection.
    #[error("unreachable: {0}")]
    Unreachable(String),
    /// A host name could not be looked up.
    #[error("cannot look up {0}")]
    Resolve(String),
    /// A secured Wi-Fi network without a saved profile: the password field must be filled first.
    #[error("password needed")]
    PasswordNeeded,
    /// The caller stopped the job (speed test cancel, sampler stop).
    #[error("cancelled")]
    Cancelled,
    /// The test server answered with an HTTP error.
    #[error("server answered HTTP {0}")]
    Http(u32),
    /// The change id was already undone or never existed.
    #[error("nothing to undo")]
    NothingToUndo,
    /// Not available on this Windows version / this PC.
    #[error("not supported: {0}")]
    Unsupported(String),
    /// Any other Windows error: the failing call and its code.
    #[error("Windows error {code:#010x} in {call}")]
    Os { call: &'static str, code: u32 },
    /// The app's elevated copy (Order 039) could not make the change: its own words.
    #[error("{0}")]
    Admin(String),
}

pub type Result<T> = std::result::Result<T, NetError>;

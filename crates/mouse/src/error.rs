//! Typed errors. Nothing in this crate panics on a Windows failure; every failure comes back as one of these.

/// Everything that can go wrong reading or changing a mouse setting.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The change needs admin and this process is not elevated (or Windows answered "access denied").
    /// The app does NOT elevate itself; a later order adds the small elevated helper.
    #[error("`{what}` needs admin")]
    NeedsAdmin { what: String },

    /// A value outside what the control allows (e.g. pointer speed 0, scroll lines 101).
    #[error("`{what}`: {why}")]
    OutOfRange { what: String, why: String },

    /// Nothing to undo for this setting.
    #[error("nothing to undo for `{0}`")]
    NothingToUndo(String),

    /// The connected mouse is not one whose settings this app can read / change.
    #[error("this mouse is not supported: {0}")]
    UnsupportedMouse(String),

    /// No mouse with this id is connected (it was unplugged, or the dongle is asleep).
    #[error("mouse not connected: {0}")]
    MouseGone(String),

    /// The mouse answered something we don't understand (wrong length, wrong command echo, bad checksum).
    #[error("the mouse answered something unexpected: {0}")]
    BadAnswer(String),

    /// Raw Accel (driver or its folder) is not installed / not found.
    #[error("Raw Accel not found: {0}")]
    RawAccelMissing(String),

    /// Raw Accel's settings could not be read or understood.
    #[error("Raw Accel settings: {0}")]
    RawAccelSettings(String),

    /// Raw Accel's writer refused the settings (its message is kept word for word).
    #[error("Raw Accel refused the settings: {0}")]
    RawAccelRefused(String),

    /// No preset / app rule / cursor set with this name or id.
    #[error("not found: {0}")]
    NotFound(String),

    /// A name that can't be used (empty, too long, already taken).
    #[error("bad name `{name}`: {why}")]
    BadName { name: String, why: &'static str },

    /// A cursor file that is not a .cur / .ani (or is broken).
    #[error("not a cursor file: {0}")]
    NotACursor(String),

    /// The OS layer was built read-only (examples/show, proofs) and refused a change.
    #[error("read-only OS layer refused: {0}")]
    ReadOnly(String),

    /// The app start / stop watcher could not start (every source refused).
    #[error("app watcher: {0}")]
    Watcher(String),

    /// A file operation failed (copying a cursor pack, reading settings.json).
    #[error("{op}: {detail}")]
    Io { op: String, detail: String },

    /// A Windows call failed. `code` is the HRESULT / Win32 / NTSTATUS value.
    #[error("{}", os_words(.op, *.code))]
    Os { op: String, code: i64 },
}

/// Order 066: what the user reads when a Windows call fails - plain words, never the API's name ("SPI_SETCURSORS failed").
/// The call's name and the code stay in `op` / `code` for logs and tests.
fn os_words(op: &str, code: i64) -> String {
    let what = if op.starts_with("SPI_SETCURSORS") || op.starts_with("SetSystemCursor") {
        "Windows didn’t switch the cursors"
    } else if op.starts_with("LoadCursorFromFile") || op.starts_with("LoadImage") {
        "Windows couldn’t load that cursor file"
    } else if op.starts_with("SPI_") || op.starts_with("SystemParametersInfo") {
        "Windows didn’t take that setting"
    } else if op.starts_with("Reg") {
        "Windows couldn’t save that setting"
    } else {
        "Windows refused that change"
    };
    let _ = code; // (kept in the error for logs; the person reads the words)
    what.to_string()
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn os(op: impl Into<String>, code: impl Into<i64>) -> Self {
        Error::Os { op: op.into(), code: code.into() }
    }

    pub(crate) fn io(op: impl Into<String>, e: impl std::fmt::Display) -> Self {
        Error::Io { op: op.into(), detail: e.to_string() }
    }

    pub(crate) fn range(what: impl Into<String>, why: impl Into<String>) -> Self {
        Error::OutOfRange { what: what.into(), why: why.into() }
    }
}

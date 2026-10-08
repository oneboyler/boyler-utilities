//! Typed errors. Nothing in this crate panics on a Windows failure; every failure comes back as one of these.

/// Everything that can go wrong reading or changing a toggle.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// No row has this id.
    #[error("no toggle row with id `{0}`")]
    UnknownRow(String),

    /// The row needs admin and this process is not elevated (or Windows answered "access denied").
    /// The app does NOT elevate itself; a later order adds the small elevated helper.
    #[error("`{row}` needs admin")]
    NeedsAdmin { row: String },

    /// Windows accepted the write but the value read back differs (e.g. the UCPD driver silently blocks it).
    /// `settings_uri` is the Settings page to open instead, when one is known.
    #[error("Windows blocked the change of `{row}`")]
    BlockedByWindows { row: String, settings_uri: Option<&'static str> },

    /// The row can't be changed right now (e.g. Fast Startup while Hibernate is off). `reason` is the grey line the menu shows.
    #[error("`{row}` is unavailable: {reason}")]
    Disabled { row: String, reason: &'static str },

    /// The PC doesn't have the thing (e.g. no Bluetooth radio).
    #[error("`{row}` is not available on this PC: {reason}")]
    NotAvailable { row: String, reason: &'static str },

    /// The call doesn't fit the row (e.g. a timeout value for a plain switch, or a timeout not in the choice list).
    #[error("`{row}`: {why}")]
    WrongKind { row: String, why: &'static str },

    /// Nothing to undo for this row.
    #[error("nothing to undo for `{0}`")]
    NothingToUndo(String),

    /// The OS layer was built read-only (examples/show, the scratch-registry tests) and refused a change.
    #[error("read-only OS layer refused: {0}")]
    ReadOnly(String),

    /// A Windows call failed. `code` is the HRESULT / Win32 / NTSTATUS value.
    #[error("{op} failed (code {code:#x})")]
    Os { op: String, code: i64 },

    /// The app's elevated copy (Order 039) could not make the change: its own words.
    #[error("{0}")]
    Admin(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn os(op: impl Into<String>, code: impl Into<i64>) -> Self {
        Error::Os { op: op.into(), code: code.into() }
    }
}

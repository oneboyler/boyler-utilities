//! Typed errors. Nothing in this crate panics on bad input or a failing Windows call. Plain-string details so the type
//! stays `Clone + Eq` (the card keeps the last one to show).

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DriverError {
    /// No connection / DNS / TLS / timeout (the updater's own line: "no connection (<step>): <why>").
    #[error("{0}")]
    Network(String),
    /// NVIDIA answered something other than "ok".
    #[error("NVIDIA's site answered {0}")]
    HttpStatus(u16),
    /// NVIDIA's answer isn't what its driver lookup sends.
    #[error("unreadable answer from NVIDIA: {0}")]
    BadResponse(String),
    /// NVIDIA's product list has no card of this name.
    #[error("NVIDIA's driver list has no {0}")]
    UnknownCard(String),
    /// NVIDIA lists no driver for the card.
    #[error("NVIDIA lists no driver for this card")]
    NoDrivers,
    /// A download link that is not NVIDIA's own https download server.
    #[error("not an NVIDIA download link: {0}")]
    NotNvidiaLink(String),
    /// The downloaded file is not a Windows program, or not the driver package.
    #[error("the download is not an NVIDIA driver package ({0})")]
    NotAPackage(String),
    /// Windows doesn't trust the file's signature, or it is not signed by NVIDIA.
    #[error("not signed by NVIDIA ({0})")]
    NotSigned(String),
    /// Reading / writing a file failed.
    #[error("{context}: {detail}")]
    Io { context: String, detail: String },
    /// NVIDIA's installer ended with an error and the driver did not change.
    #[error("NVIDIA's installer ended with code {0}")]
    SetupFailed(u32),
    /// A Windows call failed.
    #[error("{0}")]
    Os(String),
}

pub type Result<T> = std::result::Result<T, DriverError>;

pub(crate) fn io(context: &str, e: impl std::fmt::Display) -> DriverError {
    DriverError::Io { context: context.to_string(), detail: e.to_string() }
}

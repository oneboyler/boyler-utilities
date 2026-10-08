/// Every error the Activity features return. Nothing in this crate panics on purpose.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActivityError {
    /// Reading or writing the local data files failed.
    #[error("file {path}: {msg}")]
    File { path: String, msg: String },
    /// A data file that can't be read (it is left exactly as it is — never deleted, never written over).
    #[error("bad data file {path}: {msg}")]
    BadData { path: String, msg: String },
    /// The folder is outside where it may write (`FileStore::in_scratch`: proof runs only inside the scratch folder).
    #[error("refused: {0}")]
    Refused(String),
    /// A Windows call failed.
    #[error("{context} failed (0x{code:08X})")]
    Os { context: String, code: u32 },
}

pub type Result<T> = std::result::Result<T, ActivityError>;

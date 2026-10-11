//! Typed errors. Nothing in this crate panics on bad input: a file Steam wrote oddly is refused, never half-written.

use crate::layout::LayoutError;
use crate::vdf::{EditError, ParseError};
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Steam is not installed (no Steam folder found)")]
    NoSteam,
    #[error("no Steam account has controller settings on this PC")]
    NoAccount,
    #[error("{0}: this game has no layout file Steam reads yet — change it once in Steam, then it can be changed here")]
    NoLayoutFile(String),
    #[error("Steam keeps the Desktop layout inside Steam (not in a file): change it in Steam › Settings › Controller")]
    DesktopNotAFile,
    #[error("the layout Steam uses for this game is missing on disk: {0}")]
    LayoutMissing(PathBuf),
    #[error("Steam's own copy of this layout is not on this PC, so there is nothing to go back to")]
    NoSteamDefault,
    #[error("this controller has no {0}")]
    NotOnThisPad(&'static str),
    #[error("the file was changed outside the app since our last change (by Steam?) — undo would overwrite that: {0}")]
    ChangedOutside(PathBuf),
    #[error("nothing to undo")]
    NothingToUndo,
    #[error("no backup of this file was made by the app, so there is nothing to put back")]
    NoBackup,
    #[error("refused: this OS layer is read-only ({0})")]
    ReadOnly(String),
    #[error("refused: {0} is outside the allowed folder")]
    OutsideScratch(PathBuf),
    #[error("{0} is not plain UTF-8 text, so it is not changed (writing it back could alter bytes)")]
    NotUtf8(PathBuf),
    #[error("the kept backup {0} belongs to another game, so nothing was put back")]
    BackupMismatch(PathBuf),
    #[error("Steam is closed — open Steam yourself first (the app never starts Steam)")]
    SteamClosed,
    #[error("a game is running through Steam — close it first, then restart Steam")]
    GameRunning,
    #[error("Steam did not close by itself within the time given — it was left running, nothing was forced")]
    SteamWontClose,
    #[error("controller not found (unplugged?): {0}")]
    PadGone(String),
    #[error("Steam file {path}: {err}")]
    Parse { path: PathBuf, err: ParseError },
    #[error("Steam file {path}: {err}")]
    Layout { path: PathBuf, err: LayoutError },
    #[error("Steam file {path}: {err}")]
    Edit { path: PathBuf, err: EditError },
    #[error("{op} failed: {err}")]
    Io { op: String, err: std::io::Error },
    #[error("{op} failed (Windows error {code})")]
    Os { op: String, code: i64 },
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn io(op: impl Into<String>, err: std::io::Error) -> Error {
        Error::Io { op: op.into(), err }
    }
    pub fn os(op: impl Into<String>, code: i64) -> Error {
        Error::Os { op: op.into(), code }
    }
}

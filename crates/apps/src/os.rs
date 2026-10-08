//! The OS layer behind a trait: the real Windows one (`real/`) and a fake for tests (`fake.rs`).
//! Only plain reads/actions live here; every rule (what is listed, locked, the confirm data, outcomes) is in `lib.rs`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hive {
    CurrentUser,
    LocalMachine,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RegView {
    Bits64,
    Bits32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegValue {
    Str(String),
    Dword(u32),
}

/// One subkey of an `…\CurrentVersion\Uninstall` key, with its values (names lower-case).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawEntry {
    pub hive: Hive,
    pub view: RegView,
    pub key_name: String,
    pub values: HashMap<String, RegValue>,
    /// The key's last-write time (FILETIME ticks) — our fallback for a missing InstallDate (that Windows does the same is a guess).
    pub last_write: Option<u64>,
}

impl RawEntry {
    pub fn text(&self, name: &str) -> Option<&str> {
        match self.values.get(&name.to_lowercase()) {
            Some(RegValue::Str(s)) if !s.trim().is_empty() => Some(s.trim()),
            _ => None,
        }
    }
    pub fn dword(&self, name: &str) -> Option<u32> {
        match self.values.get(&name.to_lowercase()) {
            Some(RegValue::Dword(d)) => Some(*d),
            // Some installers write numbers as strings.
            Some(RegValue::Str(s)) => s.trim().parse().ok(),
            None => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signature {
    None,
    Developer,
    Enterprise,
    Store,
    /// Part of Windows.
    System,
}

/// One Store (MSIX) package for the current user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawPackage {
    pub full_name: String,
    pub family_name: String,
    /// Identity name, e.g. `Microsoft.WindowsTerminal`.
    pub name: String,
    pub display_name: Option<String>,
    pub publisher: Option<String>,
    pub version: String,
    /// Install time as FILETIME ticks.
    pub installed: Option<u64>,
    pub logo: Option<PathBuf>,
    pub installed_path: Option<PathBuf>,
    pub is_framework: bool,
    pub is_resource: bool,
    pub is_bundle: bool,
    pub is_optional: bool,
    pub signature: Signature,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OsError {
    #[error("access denied")]
    AccessDenied,
    #[error("not found")]
    NotFound,
    /// The user said No to Windows' admin prompt.
    #[error("cancelled at Windows' prompt")]
    Cancelled,
    #[error("Windows error {code:#x}: {message}")]
    Other { code: i32, message: String },
}

pub trait AppsOs {
    /// Every subkey of HKLM Uninstall (64 + 32-bit views) and HKCU Uninstall (both views).
    fn uninstall_entries(&self) -> Result<Vec<RawEntry>, OsError>;
    /// Every package installed for the current user (filtering happens in lib.rs).
    fn store_packages(&self) -> Result<Vec<RawPackage>, OsError>;
    fn entry_exists(&self, hive: Hive, view: RegView, key_name: &str) -> bool;
    fn package_installed(&self, full_name: &str) -> bool;
    /// Start an uninstall command with its own window, and wait (blocking on Windows' process events, no polling timer) until it
    /// AND every process it started have ended. Returns the first process's exit code. The user saying No at the admin prompt =
    /// `Err(Cancelled)`.
    fn run_uninstaller(&self, command: &str) -> Result<u32, OsError>;
    /// Remove a Store package for the current user (PackageManager.RemovePackageAsync), waiting for the result.
    fn remove_package(&self, full_name: &str) -> Result<(), OsError>;
    /// Total size of the files under a folder (None if it can't be read).
    fn folder_size(&self, path: &Path) -> Option<u64>;
    fn expand_env(&self, s: &str) -> String;
    /// Start an app's own Modify / Repair setup with its own window and wait until it (and what it started) ended, like
    /// [`AppsOs::run_uninstaller`]. Returns its exit code. The user saying No at the admin prompt = `Err(Cancelled)`.
    fn run_setup(&self, command: &str) -> Result<u32, OsError>;
    /// Is this path an existing folder? ("Open install folder" opens folders only: an InstallLocation that names a file must
    /// never be started.)
    fn is_dir(&self, path: &Path) -> bool;
    /// Show a FOLDER in Explorer (`explorer.exe "<folder>"`), without waiting. The real layer refuses anything that is not an
    /// existing folder.
    fn open_folder(&self, dir: &Path) -> Result<(), OsError>;
    /// Open a Windows Settings page (`ms-settings:…` only; anything else is refused), without waiting.
    fn open_settings(&self, uri: &str) -> Result<(), OsError>;
}

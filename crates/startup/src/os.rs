//! The OS layer behind a trait: the real Windows one (`real.rs`) and a fake for tests (`fake.rs`).
//! Only plain reads/writes live here; every rule (what is listed, what may switch, undo) is in `lib.rs`.

use std::path::{Path, PathBuf};

/// Registry root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hive {
    CurrentUser,
    LocalMachine,
}

/// Which registry view: the 64-bit one, or the 32-bit one (`WOW6432Node`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RegView {
    Bits64,
    Bits32,
}

/// One string value of a registry key (REG_SZ / REG_EXPAND_SZ, not expanded).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegString {
    pub name: String,
    pub data: String,
}

/// One file in a Startup folder (a shortcut is resolved to its target).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderItem {
    /// File name as Windows stores it in `StartupApproved\StartupFolder` (e.g. `Foo.lnk`).
    pub file_name: String,
    pub path: PathBuf,
    /// Shortcut target (None for a plain file).
    pub target: Option<String>,
    pub arguments: Option<String>,
    /// Shortcut icon location, if the shortcut sets one.
    pub icon: Option<String>,
}

/// A Store (MSIX) app's StartupTask, with Windows' StartupTaskState number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreStartupTask {
    pub package_family: String,
    pub task_id: String,
    /// 0 Disabled, 1 DisabledByUser, 2 Enabled, 3 DisabledByPolicy, 4 EnabledByPolicy (Windows.ApplicationModel.StartupTaskState).
    pub state: u32,
    pub display_name: Option<String>,
    pub publisher: Option<String>,
    pub logo: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskTrigger {
    /// Runs when a user signs in.
    Logon,
    /// Runs when Windows boots.
    Boot,
}

/// A scheduled task that has a logon or boot trigger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawTask {
    /// Full task path, e.g. `\Microsoft\Windows\Foo\Bar`.
    pub path: String,
    pub name: String,
    pub enabled: bool,
    pub triggers: Vec<TaskTrigger>,
    /// First "start a program" action.
    pub command: Option<String>,
    pub arguments: Option<String>,
    pub author: Option<String>,
}

/// A service's start type (the SCM numbers: 0 Boot, 1 System, 2 Automatic, 3 Manual, 4 Disabled).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceStart {
    Boot,
    System,
    Automatic,
    Manual,
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawService {
    pub name: String,
    pub display_name: String,
    pub start: ServiceStart,
    /// "Automatic (Delayed Start)".
    pub delayed: bool,
    /// The service's binary path command line (may be unquoted, may hold %vars%).
    pub image_path: Option<String>,
}

/// Version-resource strings of an exe.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileInfo {
    pub company: Option<String>,
    pub description: Option<String>,
}

/// What a Windows call failed with.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OsError {
    #[error("access denied")]
    AccessDenied,
    #[error("not found")]
    NotFound,
    #[error("Windows error {code:#x}: {message}")]
    Other { code: i32, message: String },
    /// The admin prompt was answered No (the app's elevated copy, Order 039): nothing changed.
    #[error("needs admin")]
    NeedsAdmin,
    /// The elevated copy could not make the change: its own words.
    #[error("{0}")]
    Admin(String),
}

/// Everything the startup logic needs from Windows.
pub trait StartupOs {
    /// The current process runs elevated (admin).
    fn is_admin(&self) -> bool;

    /// String values of a key. A missing key is `Ok(vec![])`.
    fn reg_strings(&self, hive: Hive, view: RegView, path: &str) -> Result<Vec<RegString>, OsError>;
    /// A binary value (64-bit view). Missing value or key = `Ok(None)`.
    fn reg_binary(&self, hive: Hive, path: &str, name: &str) -> Result<Option<Vec<u8>>, OsError>;
    /// Write a REG_BINARY value (64-bit view), creating the key if needed.
    fn reg_set_binary(&self, hive: Hive, path: &str, name: &str, data: &[u8]) -> Result<(), OsError>;
    /// Delete a value (64-bit view). Missing = Ok.
    fn reg_delete_value(&self, hive: Hive, path: &str, name: &str) -> Result<(), OsError>;

    /// Files in the per-user (`all_users = false`) or all-users Startup folder.
    fn startup_folder(&self, all_users: bool) -> Result<Vec<FolderItem>, OsError>;
    /// Store apps' StartupTasks for the current user.
    fn store_startup_tasks(&self) -> Result<Vec<StoreStartupTask>, OsError>;

    /// All scheduled tasks with a logon or boot trigger (hidden ones included).
    fn logon_tasks(&self) -> Result<Vec<RawTask>, OsError>;
    fn set_task_enabled(&self, path: &str, enabled: bool) -> Result<(), OsError>;
    /// One task's current enabled state (for read-back).
    fn task_enabled(&self, path: &str) -> Result<bool, OsError>;

    /// Win32 services whose start type is Automatic, plus the services named in `also` whatever their start type.
    fn services(&self, also: &[String]) -> Result<Vec<RawService>, OsError>;
    fn set_service_start(&self, name: &str, start: ServiceStart, delayed: bool) -> Result<(), OsError>;
    /// One service's current start type + delayed flag (for read-back).
    fn service_start(&self, name: &str) -> Result<(ServiceStart, bool), OsError>;

    /// Services this app turned from Automatic to Manual (so they stay listed), with their old delayed flag.
    fn remembered_services(&self) -> Result<Vec<(String, bool)>, OsError>;
    fn remember_service(&self, name: &str, delayed: bool) -> Result<(), OsError>;
    fn forget_service(&self, name: &str) -> Result<(), OsError>;

    fn file_info(&self, path: &Path) -> FileInfo;
    /// Expand `%VAR%` names.
    fn expand_env(&self, s: &str) -> String;
    fn file_exists(&self, path: &Path) -> bool;
    /// Sub-folders of a folder (empty when missing).
    fn subdirs(&self, dir: &Path) -> Vec<PathBuf>;

    /// Windows' own boot measurements (`%windir%\System32\wdi\LogFiles\StartupInfo\<SID>_StartupInfo*.xml`) for the current
    /// user, newest first, as XML text. The folder is admin-only: `Err(AccessDenied)` without admin.
    fn impact_reports(&self) -> Result<Vec<String>, OsError>;

    /// Now as a Windows FILETIME (100 ns since 1601), for the "disabled at" stamp Task Manager writes.
    fn now_filetime(&self) -> u64;
}

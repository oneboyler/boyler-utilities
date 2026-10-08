//! The one door to Windows. [`crate::real::RealOs`] talks to Windows; [`crate::FakeOs`] is what every test uses.

use crate::error::Result;
use crate::model::{OpenTarget, Stamp};

/// An app from `shell:AppsFolder` (desktop programs and Store apps).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEntry {
    pub name: String,
    /// What `shell:AppsFolder\<this>` opens: an AUMID or a path.
    pub parsing_name: String,
    /// The program's own path, when Windows gives one (desktop apps).
    pub program_path: Option<String>,
}

/// Is Everything (voidtools) usable? The Search tab's ONE engine (the owner, Oct 8): files and folders come only from it,
/// through its IPC window messages (no DLL shipped).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EverythingStatus {
    /// Everything.exe is not on this PC (the page offers "Install Everything").
    NotInstalled,
    /// Installed, but no Everything answers (ours is started when the Search tab opens).
    NotRunning,
    /// It answers but its index is not ready yet. `building` = OUR instance started without an index file: it is making
    /// its file list (measured on a test PC: 75 - 80 s for five drives); else it is loading a saved one (9.3 s
    /// for that index). A copy the user runs is always `false` (we cannot tell).
    Loading { building: bool },
    Running { version: u32 },
}

/// The Windows Search service (`WSearch`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsStatus {
    Running,
    /// Installed, start type allows it, not running right now.
    Stopped,
    /// Start type Disabled: nothing is indexed (on the test PC).
    Disabled,
    /// The service does not exist.
    Missing,
}

/// Which folders Windows Search covers (from its crawl-scope rules).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexScope {
    /// Included roots, e.g. `C:\Users\`, `C:\ProgramData\Microsoft\Windows\Start Menu\`.
    pub included: Vec<String>,
    /// Excluded folders inside them, e.g. `C:\Users\*\AppData\`.
    pub excluded: Vec<String>,
}

/// What a file query asks of a backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileQuery {
    /// Lower-case words; every one must be in the NAME.
    pub words: Vec<String>,
    /// Folders only, or files only.
    pub folders: bool,
    /// Only these extensions (lower case, no dot); empty = any.
    pub extensions: Vec<String>,
    /// Ask for at most this many candidates (the service ranks them).
    pub max: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub name: String,
    pub path: String,
    pub is_folder: bool,
    pub size: Option<u64>,
    pub modified: Option<Stamp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hits {
    pub items: Vec<Hit>,
    /// How many matched in all, when the backend knows (Everything does).
    pub total: Option<usize>,
}

/// Everything the Search features need from Windows. All of it works without admin. The methods may block
/// (the index, a DLL call): the page calls them off its UI thread.
pub trait SearchOs: Send + Sync {
    /// `shell:AppsFolder`, all apps. Takes a moment: loaded on the first search, never on tab open.
    fn list_apps(&self) -> Result<Vec<AppEntry>>;

    fn everything_status(&self) -> EverythingStatus;
    fn everything_query(&self, q: &FileQuery) -> Result<Hits>;
    /// Make Everything answer: a copy the user runs is used as it is (and never stopped); else OUR own instance is started
    /// hidden (no window, no tray icon, no autostart). `Err(EverythingNotInstalled)` when it is not on the PC.
    fn everything_start(&self) -> Result<()>;
    /// Quit OUR instance (the menu closed / the app quits); a copy the user runs is left alone. No-op when none is ours.
    fn everything_stop(&self);
    /// Download the official installer, check its SHA-256, run it silently (Windows asks for admin once). Blocks.
    fn everything_install(&self) -> Result<()>;

    fn windows_search_status(&self) -> WsStatus;
    fn windows_search_scope(&self) -> IndexScope;
    fn windows_search_query(&self, q: &FileQuery) -> Result<Hits>;

    /// How many entries a folder has (the "<n> items" on a folder row). `None` if it cannot be read.
    fn dir_item_count(&self, path: &str) -> Option<u64>;
    /// Open an app / file / folder (ShellExecute).
    fn open(&self, target: &OpenTarget) -> Result<()>;
    /// Open Explorer with the path selected (SHOpenFolderAndSelectItems).
    fn reveal(&self, path: &str) -> Result<()>;
    /// Windows' "Open with" dialog (SHOpenWithDialog).
    fn open_with(&self, path: &str) -> Result<()>;
    /// Put text on the clipboard.
    fn copy_text(&self, text: &str) -> Result<()>;
}

impl FileQuery {
    /// The Everything search text: `folder: "w1" "w2"` / `file: "w1" ext:png;jpg` (names only, which is Everything's default).
    pub fn everything_text(&self) -> String {
        let mut s = String::from(if self.folders { "folder:" } else { "file:" });
        for w in &self.words {
            s.push(' ');
            s.push_str(&crate::model::everything_word(w));
        }
        if !self.extensions.is_empty() {
            s.push_str(" ext:");
            s.push_str(&self.extensions.join(";"));
        }
        s
    }

    /// The Windows Search SQL (`SystemIndex`). `LIKE '%word%'` on the display name; `[`, `_` and `%` typed by the user are made literal
    /// (`sql_like_word`); the service re-checks every name anyway.
    pub fn windows_search_sql(&self) -> String {
        let mut s = format!(
            "SELECT TOP {} System.ItemNameDisplay, System.ItemPathDisplay, System.Size, System.DateModified \
             FROM SystemIndex WHERE SCOPE='file:' AND System.ItemType {} 'Directory'",
            self.max.max(1),
            if self.folders { "=" } else { "<>" }
        );
        for w in &self.words {
            s.push_str(&format!(" AND System.ItemNameDisplay LIKE '%{}%'", crate::model::sql_like_word(w)));
        }
        if !self.extensions.is_empty() {
            let alts: Vec<String> = self.extensions.iter().map(|e| format!("System.FileExtension = '.{}'", crate::model::sql_string(e))).collect();
            s.push_str(&format!(" AND ({})", alts.join(" OR ")));
        }
        s
    }
}

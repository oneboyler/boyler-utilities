//! How a file is classed for the "File types" bar (DESIGN §3.12 item 2).
//!
//! The rule, first match wins:
//! 1. **Games** — anything inside a game folder: Steam libraries (`steamapps`), Epic install folders (its manifests),
//!    Riot Games, GOG (registry), Xbox app games (`X:\XboxGames`).
//! 2. **Windows & other** — anything inside the Windows folder (`C:\Windows`).
//! 3. **Apps** — anything inside `Program Files`, `Program Files (x86)`, `%LocalAppData%\Programs`, or any file with
//!    a program extension (.exe .dll .msi .msix …; not .sys — pagefile.sys and hiberfil.sys are Windows').
//! 4. By extension: **Videos**, **Pictures**, **Documents** (lists below).
//! 5. Everything else (music, archives, caches, unknown) → **Windows & other**.

use std::path::{Path, PathBuf};

/// The six parts of the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FileType {
    Games,
    Videos,
    Apps,
    Pictures,
    Documents,
    WindowsOther,
}

impl FileType {
    pub const ALL: [FileType; 6] =
        [FileType::Games, FileType::Videos, FileType::Apps, FileType::Pictures, FileType::Documents, FileType::WindowsOther];

    pub fn index(self) -> usize {
        self as usize
    }

    /// The row name.
    pub fn name(self) -> &'static str {
        match self {
            FileType::Games => "Games",
            FileType::Videos => "Videos",
            FileType::Apps => "Apps",
            FileType::Pictures => "Pictures",
            FileType::Documents => "Documents",
            FileType::WindowsOther => "Windows & other",
        }
    }
}

const VIDEO: &[&str] = &[
    "mp4", "mkv", "mov", "avi", "wmv", "webm", "flv", "m4v", "mpg", "mpeg", "ts", "m2ts", "mts", "3gp", "vob", "ogv",
];
const PICTURE: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "bmp", "webp", "tif", "tiff", "heic", "heif", "avif", "raw", "cr2", "cr3", "nef", "arw",
    "dng", "psd", "svg", "ico", "jxl",
];
const DOCUMENT: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "txt", "md", "csv", "epub", "pages",
    "numbers", "key",
];
const PROGRAM: &[&str] = &["exe", "dll", "msi", "msix", "appx", "ocx", "cpl", "scr"];

/// The folders the rule looks at. Comparisons are case-insensitive.
#[derive(Debug, Clone, Default)]
pub struct ClassRules {
    game_roots: Vec<String>,
    windows_roots: Vec<String>,
    app_roots: Vec<String>,
}

/// What a folder's contents are, from its place on the disk (rules 1–3); `Plain` = decide per file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderClass {
    Plain,
    Games,
    Windows,
    Apps,
}

fn norm(p: &Path) -> String {
    let mut s = p.to_string_lossy().replace('/', "\\").to_lowercase();
    while s.ends_with('\\') && s.len() > 3 {
        s.pop();
    }
    s
}

impl ClassRules {
    pub fn new(game_roots: &[PathBuf], windows: Option<&Path>, app_roots: &[PathBuf]) -> Self {
        ClassRules {
            game_roots: game_roots.iter().map(|p| norm(p)).collect(),
            windows_roots: windows.map(norm).into_iter().collect(),
            app_roots: app_roots.iter().map(|p| norm(p)).collect(),
        }
    }

    /// From what the OS layer knows (game roots, Windows folder, Program Files, `%LocalAppData%\Programs`).
    pub fn from_os(os: &dyn crate::StorageOs) -> Self {
        let dirs = os.known_dirs();
        let mut apps = dirs.program_files.clone();
        if let Some(local) = &dirs.local_appdata {
            apps.push(local.join("Programs"));
        }
        ClassRules::new(&os.game_roots(), dirs.windows.as_deref(), &apps)
    }

    /// The class a folder starts (rules 1–3, games first). Folders below it keep that class unless they are a rule
    /// folder themselves.
    pub fn folder_class(&self, folder: &Path) -> FolderClass {
        let f = norm(folder);
        if self.game_roots.contains(&f) {
            FolderClass::Games
        } else if self.windows_roots.contains(&f) {
            FolderClass::Windows
        } else if self.app_roots.contains(&f) {
            FolderClass::Apps
        } else {
            FolderClass::Plain
        }
    }

    /// True when one of the rule folders lies *below* `folder` — the scan stops comparing paths once nothing can
    /// match any more (cheap, case-insensitive prefix test).
    pub fn may_contain_root(&self, folder: &Path) -> bool {
        let f = norm(folder);
        let prefix = if f.ends_with('\\') { f.clone() } else { format!("{f}\\") };
        self.game_roots
            .iter()
            .chain(&self.windows_roots)
            .chain(&self.app_roots)
            .any(|r| r.starts_with(&prefix))
    }
}

/// The class of one file inside a folder of class `folder`.
pub fn classify_file(folder: FolderClass, file_name: &str) -> FileType {
    match folder {
        FolderClass::Games => FileType::Games,
        FolderClass::Windows => FileType::WindowsOther,
        FolderClass::Apps => FileType::Apps,
        FolderClass::Plain => by_extension(file_name),
    }
}

/// Rules 3 (program extensions) and 4–5.
pub fn by_extension(file_name: &str) -> FileType {
    let ext = match file_name.rsplit_once('.') {
        Some((_, e)) if !e.is_empty() && e.len() <= 8 => e.to_ascii_lowercase(),
        _ => return FileType::WindowsOther,
    };
    let ext = ext.as_str();
    if VIDEO.contains(&ext) {
        FileType::Videos
    } else if PICTURE.contains(&ext) {
        FileType::Pictures
    } else if DOCUMENT.contains(&ext) {
        FileType::Documents
    } else if PROGRAM.contains(&ext) {
        FileType::Apps
    } else {
        FileType::WindowsOther
    }
}

/// Steam library folders from `steamapps\libraryfolders.vdf` (its `"path"` lines), each as `<path>\steamapps`.
pub fn parse_steam_libraries(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|line| {
            let t = line.trim();
            let rest = t.strip_prefix("\"path\"")?.trim();
            let inner = rest.strip_prefix('"')?.strip_suffix('"')?;
            Some(PathBuf::from(inner.replace("\\\\", "\\")).join("steamapps"))
        })
        .collect()
}

/// `InstallLocation` from one Epic manifest (`%ProgramData%\Epic\EpicGamesLauncher\Data\Manifests\*.item`, JSON).
pub fn parse_epic_manifest(json: &str) -> Option<PathBuf> {
    let at = json.find("\"InstallLocation\"")?;
    let rest = &json[at + "\"InstallLocation\"".len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(PathBuf::from(out)).filter(|p| !p.as_os_str().is_empty()),
            '\\' => match chars.next() {
                Some('\\') => out.push('\\'),
                Some('/') => out.push('/'),
                Some('"') => out.push('"'),
                Some(other) => {
                    out.push('\\');
                    out.push(other)
                }
                None => return None,
            },
            c => out.push(c),
        }
    }
    None
}

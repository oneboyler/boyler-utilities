//! Plain data and pure rules of the Search page (DESIGN.md §3.15): filters, groups, ranking, marking, size / date text.
//! Every Windows call is behind [`crate::SearchOs`].

use std::fmt;

/// A local date and time (what the rows print).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Stamp {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

impl Stamp {
    pub fn new(year: u16, month: u8, day: u8, hour: u8, minute: u8) -> Stamp {
        Stamp { year, month, day, hour, minute }
    }
    /// "6 Oct 2026" (the row's date).
    pub fn date_label(&self) -> String {
        format!("{} {} {}", self.day, MONTHS[(self.month.clamp(1, 12) - 1) as usize], self.year)
    }
}

impl fmt::Display for Stamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02} {:02}:{:02}", self.year, self.month, self.day, self.hour, self.minute)
    }
}

/// The chips: All · Apps · Folders · Files · Pictures · Videos · Documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Apps,
    Folders,
    Files,
    Pictures,
    Videos,
    Documents,
}

impl Filter {
    pub const CHIPS: [Filter; 7] =
        [Filter::All, Filter::Apps, Filter::Folders, Filter::Files, Filter::Pictures, Filter::Videos, Filter::Documents];
    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Apps => "Apps",
            Filter::Folders => "Folders",
            Filter::Files => "Files",
            Filter::Pictures => "Pictures",
            Filter::Videos => "Videos",
            Filter::Documents => "Documents",
        }
    }
    /// The file type a typed filter keeps.
    fn types(self) -> Option<&'static [FileType]> {
        match self {
            Filter::Pictures => Some(&[FileType::Picture]),
            Filter::Videos => Some(&[FileType::Video]),
            Filter::Documents => Some(&[FileType::Document, FileType::Text]),
            _ => None,
        }
    }
    /// The extensions (lower case, no dot) a typed filter keeps; empty for the others.
    pub fn extensions(self) -> Vec<&'static str> {
        match self.types() {
            Some(ts) => ts.iter().flat_map(|t| t.extensions().iter().copied()).collect(),
            None => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemKind {
    App,
    Folder,
    File,
}

impl ItemKind {
    /// The group title.
    pub fn group_title(self) -> &'static str {
        match self {
            ItemKind::App => "Apps",
            ItemKind::Folder => "Folders",
            ItemKind::File => "Files",
        }
    }
}

/// The tile colour of a file row: picture, video, document, text, archive, program, config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Picture,
    Video,
    Document,
    Text,
    Archive,
    Program,
    Config,
    Other,
}

impl FileType {
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            FileType::Picture => &["png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "heic", "avif", "svg", "ico"],
            FileType::Video => &["mp4", "mkv", "avi", "mov", "wmv", "webm", "flv", "m4v", "mpg", "mpeg"],
            FileType::Document => &["pdf", "doc", "docx", "ppt", "pptx", "xls", "xlsx", "odt", "ods", "odp", "rtf"],
            FileType::Text => &["txt", "md", "log", "csv"],
            FileType::Archive => &["zip", "rar", "7z", "tar", "gz"],
            FileType::Program => &["exe", "msi", "bat", "cmd", "lnk"],
            FileType::Config => &["ini", "cfg", "conf", "toml", "yaml", "yml", "json", "xml"],
            FileType::Other => &[],
        }
    }
    pub fn of_file_name(name: &str) -> FileType {
        let ext = match name.rsplit_once('.') {
            Some((stem, e)) if !stem.is_empty() => e.to_ascii_lowercase(),
            _ => return FileType::Other,
        };
        for t in [FileType::Picture, FileType::Video, FileType::Document, FileType::Text, FileType::Archive, FileType::Program, FileType::Config] {
            if t.extensions().contains(&ext.as_str()) {
                return t;
            }
        }
        FileType::Other
    }
}

/// What opening an item means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenTarget {
    /// An app from `shell:AppsFolder`: its parsing name (an AUMID or a path).
    App(String),
    /// A file or folder path.
    Path(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub kind: ItemKind,
    pub name: String,
    /// The path shown under the name: a file / folder path, an app's program path (empty when Windows gives none).
    pub path: String,
    pub open: OpenTarget,
    pub file_type: Option<FileType>,
    pub modified: Option<Stamp>,
    pub size: Option<u64>,
}

impl Item {
    /// The full path for the right-click header and "Copy path". For files and folders: folder + name.
    pub fn full_path(&self) -> String {
        self.path.clone() // already the full path for files and folders; an app's program path (may be empty)
    }
    /// The folder a file / folder lives in ("Open file location" opens it with the item selected).
    pub fn parent_dir(&self) -> Option<String> {
        let p = self.path.trim_end_matches('\\');
        p.rfind('\\').map(|i| p[..i].to_string())
    }
    /// "142.6 MB" for files, none for folders / apps (the page shows "App" / "<n> items").
    pub fn size_text(&self) -> Option<String> {
        self.size.filter(|_| self.kind == ItemKind::File).map(format_size)
    }
    pub fn date_text(&self) -> Option<String> {
        self.modified.map(|m| m.date_label())
    }
}

/// "3 KB", "212 KB", "1.4 MB", "142.6 MB", "1.8 GB" (KB rounded, at least 1; MB / GB one decimal; under 1 KB: bytes).
pub fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{} KB", ((b / KB).round() as u64).max(1))
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else {
        format!("{:.1} GB", b / (KB * KB * KB))
    }
}

/// "<n> items" for a folder row.
pub fn items_text(n: u64) -> String {
    if n == 1 {
        "1 item".to_string()
    } else {
        format!("{n} items")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub filter: Filter,
    /// The page's type picker: only files with this extension (lower case, no dot, e.g. "txt"); then no apps / folders.
    pub ext: Option<String>,
}

impl Query {
    pub fn new(text: &str, filter: Filter) -> Query {
        Query { text: text.to_string(), filter, ext: None }
    }
    /// Only files of one extension (the type picker: ".txt" -> "txt").
    pub fn with_ext(mut self, ext: Option<&str>) -> Query {
        self.ext = ext.map(|e| e.trim_start_matches('.').to_ascii_lowercase()).filter(|e| !e.is_empty());
        self
    }
    /// Lower-case words; none for an empty / blank query.
    pub fn words(&self) -> Vec<String> {
        self.text.split_whitespace().map(|w| w.to_lowercase()).collect()
    }
}

// ----------------------------------------------------------------------------------------------- ranking / marking

fn is_word_start_char(c: char) -> bool {
    c.is_whitespace() || matches!(c, '.' | '_' | '-')
}

/// Does `w` occur in `name` (both lower case) at the start of a word (start of the name or after space . _ -)?
fn word_starts_with(name: &str, w: &str) -> bool {
    if w.is_empty() {
        return true;
    }
    let mut prev: Option<char> = None;
    for (i, c) in name.char_indices() {
        if prev.is_none_or(is_word_start_char) && name[i..].starts_with(w) {
            return true;
        }
        prev = Some(c);
    }
    false
}

/// Only the NAME counts (like the old Windows search): every word must be in it. The order:
/// 0 = the name starts with the whole text, 1 = every word starts a word, 2 = the words are somewhere in the name.
/// `None` = no match.
pub fn score(name: &str, words: &[String], text: &str) -> Option<u8> {
    let n = name.to_lowercase();
    if !words.iter().all(|w| n.contains(w.as_str())) {
        return None;
    }
    if n.starts_with(&text.trim().to_lowercase()) {
        return Some(0);
    }
    if words.iter().all(|w| word_starts_with(&n, w)) {
        return Some(1);
    }
    Some(2)
}

/// Sort key order of two ranked names: score, then shorter names, then alphabetical.
pub fn rank_cmp(a: (u8, &str), b: (u8, &str)) -> std::cmp::Ordering {
    a.0.cmp(&b.0).then_with(|| a.1.chars().count().cmp(&b.1.chars().count())).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
}

/// Where the typed words appear in `name`, as character ranges `(start, end)` (end exclusive), merged and sorted —
/// the page marks them ("matches marked like Toggles").
pub fn match_ranges(name: &str, words: &[String]) -> Vec<(usize, usize)> {
    let lower: Vec<char> = name.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect();
    let mut found: Vec<(usize, usize)> = Vec::new();
    for w in words {
        let wc: Vec<char> = w.chars().collect();
        if wc.is_empty() || wc.len() > lower.len() {
            continue;
        }
        let mut i = 0;
        while i + wc.len() <= lower.len() {
            if lower[i..i + wc.len()] == wc[..] {
                found.push((i, i + wc.len()));
                i += wc.len();
            } else {
                i += 1;
            }
        }
    }
    found.sort();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (s, e) in found {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    merged
}

// ----------------------------------------------------------------------------------------------- results

/// One group of the results ("Apps  3").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub kind: ItemKind,
    /// "Apps" / "Folders" / "Files" / "Pictures" / "Videos" / "Documents"
    pub title: &'static str,
    /// How many matched (the count in the group header and "Show all N").
    pub total: usize,
    /// The capped list (4 / 4 / 6 under All, 40 under one chip), best first.
    pub items: Vec<Item>,
    /// `total` is only what the index returned at most (more may exist).
    pub total_is_lower_bound: bool,
}

impl Group {
    /// "Show all N" is offered when more matched than are shown.
    pub fn show_all(&self) -> Option<usize> {
        (self.total > self.items.len()).then_some(self.total)
    }
}

/// How many rows each group shows under All, and under one chip.
pub const CAP_ALL: [(ItemKind, usize); 3] = [(ItemKind::App, 4), (ItemKind::Folder, 4), (ItemKind::File, 6)];
pub const CAP_ONE: usize = 40;

/// Where files and folders came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilesFrom {
    Everything { version: u32 },
    WindowsSearch,
    /// Neither: files and folders cannot be searched (apps still can). The reason is in `SearchResults::note`.
    Nothing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResults {
    pub query: Query,
    pub groups: Vec<Group>,
    pub files_from: FilesFrom,
    /// Why files / folders are missing (the page says it instead of "Nothing on this PC matches"), if they are.
    pub note: Option<crate::SearchError>,
}

impl SearchResults {
    pub fn is_empty(&self) -> bool {
        self.groups.iter().all(|g| g.items.is_empty())
    }
    /// All shown items in the order the page lists them (↑ ↓ and Enter use this; the first is selected).
    pub fn flat(&self) -> Vec<&Item> {
        self.groups.iter().flat_map(|g| g.items.iter()).collect()
    }
}

/// The page-4 line "Nothing on this PC matches “<text>”".
pub fn none_text(text: &str) -> String {
    format!("Nothing on this PC matches “{}”", text.trim())
}

/// What the page says in an empty field.
pub const EMPTY_HINT: (&str, &str) = ("Type to find apps, folders and files on this PC.", "Only what is on this PC — no web results, no ads.");

/// The right-click menu of a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    OpenFileLocation,
    CopyPath,
    /// Files only.
    OpenWith,
}

impl MenuAction {
    pub fn label(self) -> &'static str {
        match self {
            MenuAction::OpenFileLocation => "Open file location",
            MenuAction::CopyPath => "Copy path",
            MenuAction::OpenWith => "Open with…",
        }
    }
}

// ----------------------------------------------------------------------------------------------- query text for the backends

/// Quote a word for Everything: `"word"` (a `"` inside is dropped).
pub fn everything_word(w: &str) -> String {
    format!("\"{}\"", w.replace('"', ""))
}

/// Quote a word for a Windows Search SQL string: `'` doubled.
pub fn sql_string(w: &str) -> String {
    w.replace('\'', "''")
}

/// A typed word as the inside of `LIKE '%…%'` in Windows Search SQL: `'` doubled for the string, and `[`, `%`, `_` made literal
/// (`[` opens a set there, `%` and `_` are wildcards: a lone `[` can fail the whole query, `[1080p]` would match any of its letters,
/// and `%` / `_` would flood the candidate list).
pub fn sql_like_word(w: &str) -> String {
    let mut out = String::with_capacity(w.len());
    for c in w.chars() {
        match c {
            '\'' => out.push_str("''"),
            '[' => out.push_str("[[]"),
            '%' => out.push_str("[%]"),
            '_' => out.push_str("[_]"),
            _ => out.push(c),
        }
    }
    out
}

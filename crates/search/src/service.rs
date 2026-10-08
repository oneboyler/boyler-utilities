//! The commands of the Search tab (DESIGN.md §3.15): search as you type, the right-click menu, open. No threads, no timers:
//! every call runs when the page asks, on the caller's thread; nothing is loaded until the first word is typed and
//! [`SearchService::release`] drops it all when the menu closes.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::error::{Result, SearchError};
use crate::model::*;
use crate::os::*;

/// Candidates asked of a backend per group; the service ranks them and shows the best.
pub const MAX_CANDIDATES: usize = 300;

/// A newer keystroke cancels the older search (checked between the apps, folders and files steps).
#[derive(Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Cancel {
        Cancel::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Which source answers files and folders right now, and what the Windows Search index covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendReport {
    pub files_from: FilesFrom,
    pub everything: EverythingStatus,
    pub windows_search: WsStatus,
    /// Only filled when Windows Search is the source: the folders its index covers.
    pub scope: Option<IndexScope>,
}

#[derive(Default)]
struct Cache {
    apps: Option<Arc<Vec<AppEntry>>>,
}

pub struct SearchService {
    os: Arc<dyn SearchOs>,
    cache: Mutex<Cache>,
}

impl SearchService {
    pub fn new(os: Arc<dyn SearchOs>) -> SearchService {
        SearchService { os, cache: Mutex::new(Cache::default()) }
    }

    /// The service on the real PC.
    #[cfg(windows)]
    pub fn real() -> SearchService {
        SearchService::new(Arc::new(crate::real::RealOs::new()))
    }

    pub fn os(&self) -> Arc<dyn SearchOs> {
        self.os.clone()
    }

    /// The menu closed: forget the app list and quit OUR Everything (a copy the user runs is left alone). Nothing of the
    /// Search tab stays running (the owner: no RAM / CPU unless the user is using it).
    pub fn release(&self) {
        self.cache.lock().unwrap().apps = None;
        self.os.everything_stop();
    }

    /// The Search tab opened: make Everything answer (the user's own copy, or ours started hidden). Cheap when it already
    /// runs. `Err(EverythingNotInstalled)` = offer the install.
    pub fn start_engine(&self) -> Result<()> {
        self.os.everything_start()
    }

    /// Everything right now: not installed / not running / loading its index ("catching up") / ready.
    pub fn engine(&self) -> EverythingStatus {
        self.os.everything_status()
    }

    /// The page's "Install Everything": the official installer, checked, run silently (one admin prompt). Blocks for the
    /// download + install: call it off the UI thread. Afterwards `start_engine` starts it.
    pub fn install_engine(&self) -> Result<()> {
        self.os.everything_install()
    }

    fn apps(&self) -> Result<Arc<Vec<AppEntry>>> {
        if let Some(a) = self.cache.lock().unwrap().apps.clone() {
            return Ok(a);
        }
        let list = Arc::new(self.os.list_apps()?);
        self.cache.lock().unwrap().apps = Some(list.clone());
        Ok(list)
    }

    /// Which source answers files / folders: Everything when it runs and its index is loaded, else nothing (no other
    /// engine: the owner, Oct 8). Cheap (one window message).
    pub fn backend_report(&self) -> BackendReport {
        let everything = self.os.everything_status();
        let files_from = match everything {
            EverythingStatus::Running { version } => FilesFrom::Everything { version },
            _ => FilesFrom::Nothing,
        };
        BackendReport { files_from, everything, windows_search: WsStatus::Missing, scope: None }
    }

    /// Search as you type. An empty query returns no groups and asks no backend. The first non-empty search loads the
    /// app list (about a second the first time on a real PC); files and folders come from one query each.
    pub fn search(&self, q: &Query, cancel: &Cancel) -> Result<SearchResults> {
        let words = q.words();
        let empty = SearchResults { query: q.clone(), groups: Vec::new(), files_from: FilesFrom::Nothing, note: None };
        if words.is_empty() {
            return Ok(empty);
        }
        let text = words.join(" ");
        let mut groups = Vec::new();
        // the type picker: files of one extension only
        let typed = q.ext.is_some();

        // apps
        if !typed && matches!(q.filter, Filter::All | Filter::Apps) {
            let apps = self.apps()?;
            let mut ranked: Vec<(u8, Item)> = apps
                .iter()
                .filter_map(|a| score(&a.name, &words, &text).map(|s| (s, app_item(a))))
                .collect();
            ranked.sort_by(|a, b| rank_cmp((a.0, &a.1.name), (b.0, &b.1.name)));
            groups.push(group(ItemKind::App, "Apps", ranked.into_iter().map(|r| r.1).collect(), false, cap_for(q.filter, ItemKind::App)));
        }
        check(cancel)?;

        // files and folders
        let wants_folders = !typed && matches!(q.filter, Filter::All | Filter::Folders);
        let wants_files = typed || matches!(q.filter, Filter::All | Filter::Files | Filter::Pictures | Filter::Videos | Filter::Documents);
        let mut files_from = FilesFrom::Nothing;
        let mut note = None;
        if wants_folders || wants_files {
            let report = self.backend_report();
            files_from = report.files_from.clone();
            if files_from == FilesFrom::Nothing {
                note = Some(match report.everything {
                    EverythingStatus::NotInstalled => SearchError::EverythingNotInstalled,
                    EverythingStatus::Loading { .. } => SearchError::EverythingLoading,
                    _ => SearchError::Everything("Everything is not running".into()),
                });
            } else {
                let ext: Vec<String> = match &q.ext {
                    Some(e) => vec![e.clone()],
                    None => q.filter.extensions().iter().map(|e| e.to_string()).collect(),
                };
                let cap = if typed { CAP_ONE } else { cap_for(q.filter, ItemKind::File) };
                let run = |folders: bool, extensions: Vec<String>| -> Result<Hits> {
                    let fq = FileQuery { words: words.clone(), folders, extensions, max: MAX_CANDIDATES };
                    self.os.everything_query(&fq)
                };
                let result = (|| -> Result<()> {
                    if wants_folders {
                        let hits = run(true, Vec::new())?;
                        check(cancel)?;
                        groups.push(file_group(ItemKind::Folder, "Folders", hits, &words, &text, &[], cap_for(q.filter, ItemKind::Folder)));
                    }
                    if wants_files {
                        let hits = run(false, ext.clone())?;
                        check(cancel)?;
                        let title = if typed || q.filter == Filter::All || q.filter == Filter::Files { "Files" } else { q.filter.label() };
                        groups.push(file_group(ItemKind::File, title, hits, &words, &text, &ext, cap));
                    }
                    Ok(())
                })();
                match result {
                    Ok(()) => {}
                    Err(SearchError::Cancelled) => return Err(SearchError::Cancelled),
                    Err(e) => {
                        // the source failed mid-query: apps still show, the page says why files are missing
                        files_from = FilesFrom::Nothing;
                        note = Some(e);
                        groups.retain(|g| g.kind == ItemKind::App);
                    }
                }
            }
        }
        groups.retain(|g| g.total > 0);
        Ok(SearchResults { query: q.clone(), groups, files_from, note })
    }

    // ---------------------------------------------------------------------------------------- actions

    /// Click or Enter on a result.
    pub fn open(&self, item: &Item) -> Result<()> {
        self.os.open(&item.open)
    }

    /// The right-click menu: "Open file location", "Copy path", and "Open with…" for files. An app Windows gives no path
    /// for has no menu entries.
    pub fn menu_for(&self, item: &Item) -> Vec<MenuAction> {
        if item.path.is_empty() {
            return Vec::new();
        }
        let mut v = vec![MenuAction::OpenFileLocation, MenuAction::CopyPath];
        if item.kind == ItemKind::File {
            v.push(MenuAction::OpenWith);
        }
        v
    }

    /// The full path on top of the right-click menu.
    pub fn menu_header(&self, item: &Item) -> String {
        item.full_path()
    }

    /// Run a menu entry.
    pub fn run_menu(&self, item: &Item, action: MenuAction) -> Result<()> {
        if !self.menu_for(item).contains(&action) {
            return Err(SearchError::Unsupported(format!("{} for {}", action.label(), item.name)));
        }
        match action {
            MenuAction::OpenFileLocation => self.os.reveal(&item.path),
            MenuAction::CopyPath => self.os.copy_text(&item.full_path()),
            MenuAction::OpenWith => self.os.open_with(&item.path),
        }
    }

    /// "<n> items" for a folder row (reads the folder; ask for the rows that are shown, not for all candidates).
    pub fn folder_item_count(&self, item: &Item) -> Option<u64> {
        (item.kind == ItemKind::Folder).then(|| self.os.dir_item_count(&item.path)).flatten()
    }
}

fn check(c: &Cancel) -> Result<()> {
    if c.is_cancelled() {
        Err(SearchError::Cancelled)
    } else {
        Ok(())
    }
}

fn cap_for(filter: Filter, kind: ItemKind) -> usize {
    if filter == Filter::All {
        CAP_ALL.iter().find(|(k, _)| *k == kind).map(|(_, n)| *n).unwrap_or(CAP_ONE)
    } else {
        CAP_ONE
    }
}

fn app_item(a: &AppEntry) -> Item {
    Item {
        kind: ItemKind::App,
        name: a.name.clone(),
        path: a.program_path.clone().unwrap_or_default(),
        open: OpenTarget::App(a.parsing_name.clone()),
        file_type: None,
        modified: None,
        size: None,
    }
}

fn group(kind: ItemKind, title: &'static str, ranked: Vec<Item>, lower_bound: bool, cap: usize) -> Group {
    let total = ranked.len();
    Group { kind, title, total, items: ranked.into_iter().take(cap).collect(), total_is_lower_bound: lower_bound }
}

/// Turn a backend's candidates into a ranked, capped group. Every word must really be in the name (the index may
/// return a superset), duplicates by path are dropped, and typed filters keep only their extensions.
fn file_group(kind: ItemKind, title: &'static str, hits: Hits, words: &[String], text: &str, ext: &[String], cap: usize) -> Group {
    let truncated = hits.items.len() >= MAX_CANDIDATES;
    let mut seen = HashSet::new();
    let mut ranked: Vec<(u8, Item)> = Vec::new();
    for h in hits.items {
        if h.is_folder != (kind == ItemKind::Folder) {
            continue;
        }
        let file_type = (kind == ItemKind::File).then(|| FileType::of_file_name(&h.name));
        if kind == ItemKind::File && !ext.is_empty() {
            let e = h.name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
            if !ext.contains(&e) {
                continue;
            }
        }
        let Some(s) = score(&h.name, words, text) else { continue };
        if !seen.insert(h.path.to_lowercase()) {
            continue;
        }
        ranked.push((
            s,
            Item { kind, name: h.name, path: h.path.clone(), open: OpenTarget::Path(h.path), file_type, modified: h.modified, size: h.size },
        ));
    }
    ranked.sort_by(|a, b| rank_cmp((a.0, &a.1.name), (b.0, &b.1.name)));
    let n = ranked.len();
    let mut g = group(kind, title, ranked.into_iter().map(|r| r.1).collect(), false, cap);
    if truncated {
        g.total = hits.total.unwrap_or(n).max(n);
        g.total_is_lower_bound = hits.total.is_none();
    }
    g
}

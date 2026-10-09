//! The pretend Windows every test runs on: an app list, an "Everything" and a "Windows Search" with canned files, and a log of
//! every query and every action, so tests can prove what was (not) asked.

use std::sync::{Mutex, MutexGuard};

use crate::error::{Result, SearchError};
use crate::model::OpenTarget;
use crate::os::*;

pub struct FakeState {
    pub apps: Vec<AppEntry>,
    pub everything: EverythingStatus,
    pub everything_files: Vec<Hit>,
    pub ws: WsStatus,
    pub ws_files: Vec<Hit>,
    pub ws_scope: IndexScope,
    /// Folder item counts by path.
    pub counts: Vec<(String, u64)>,
    /// Every backend query, as text, in order.
    pub queries: Vec<String>,
    /// Every action that would change something or open something, in order.
    pub actions: Vec<String>,
    pub apps_loads: usize,
    pub everything_released: usize,
    /// What `everything_start` turns a NotRunning Everything into (Loading = its index not ready yet; a test flips it on).
    pub everything_starts_as: EverythingStatus,
    /// Our instance runs (started by `everything_start` from NotRunning; `everything_stop` quits only that one).
    pub everything_ours: bool,
    /// `everything_install`: Ok = installed (NotRunning after), Err = this error.
    pub install_result: Result<()>,
    /// The next Everything query fails with this.
    pub everything_error: Option<SearchError>,
    pub ws_error: Option<SearchError>,
    pub action_error: Option<SearchError>,
    /// Order 049: the NTFS drives + the Windows drive, the drives our Everything covers, and v1.0.0's Everything still there
    pub drives: (Vec<char>, char),
    pub covered: Vec<char>,
    pub old_everything: bool,
    /// the running Everything is a copy the user runs (not ours)
    pub user_copy: bool,
}

pub struct FakeOs(Mutex<FakeState>);

impl FakeOs {
    /// No apps, no files, Everything absent, Windows Search running with an empty index.
    pub fn new() -> FakeOs {
        FakeOs(Mutex::new(FakeState {
            apps: Vec::new(),
            everything: EverythingStatus::NotInstalled,
            everything_files: Vec::new(),
            ws: WsStatus::Running,
            ws_files: Vec::new(),
            ws_scope: IndexScope { included: vec![r"C:\Users\".into()], excluded: vec![r"C:\Users\*\AppData\".into()] },
            counts: Vec::new(),
            queries: Vec::new(),
            actions: Vec::new(),
            apps_loads: 0,
            everything_released: 0,
            everything_starts_as: EverythingStatus::Running { version: 1 },
            everything_ours: false,
            install_result: Ok(()),
            everything_error: None,
            ws_error: None,
            action_error: None,
            drives: (vec!['C', 'D'], 'C'),
            covered: vec!['C'],
            old_everything: false,
            user_copy: false,
        }))
    }
    pub fn state(&self) -> MutexGuard<'_, FakeState> {
        self.0.lock().unwrap()
    }
    pub fn app(&self, name: &str, parsing: &str, path: Option<&str>) {
        self.state().apps.push(AppEntry { name: name.into(), parsing_name: parsing.into(), program_path: path.map(String::from) });
    }
}

impl Default for FakeOs {
    fn default() -> Self {
        FakeOs::new()
    }
}

pub fn hit(name: &str, dir: &str, is_folder: bool, size: Option<u64>) -> Hit {
    Hit { name: name.into(), path: format!("{dir}\\{name}"), is_folder, size, modified: Some(crate::model::Stamp::new(2026, 10, 6, 21, 45)) }
}

/// What a real index does: names that contain every word, folders or files, the extensions, at most `max`.
fn run(files: &[Hit], q: &FileQuery) -> Hits {
    let all: Vec<Hit> = files
        .iter()
        .filter(|h| h.is_folder == q.folders)
        .filter(|h| {
            let n = h.name.to_lowercase();
            q.words.iter().all(|w| n.contains(w.as_str()))
        })
        .filter(|h| {
            q.extensions.is_empty() || {
                let e = h.name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
                q.extensions.contains(&e)
            }
        })
        .cloned()
        .collect();
    let total = all.len();
    Hits { items: all.into_iter().take(q.max).collect(), total: Some(total) }
}

fn describe(source: &str, q: &FileQuery) -> String {
    format!("{source} {} {:?} ext={:?} max={}", if q.folders { "folders" } else { "files" }, q.words, q.extensions, q.max)
}

impl SearchOs for FakeOs {
    fn list_apps(&self) -> Result<Vec<AppEntry>> {
        let mut s = self.state();
        s.apps_loads += 1;
        Ok(s.apps.clone())
    }
    fn everything_status(&self) -> EverythingStatus {
        self.state().everything
    }
    fn everything_query(&self, q: &FileQuery) -> Result<Hits> {
        let mut s = self.state();
        s.queries.push(describe("everything", q));
        if let Some(e) = s.everything_error.take() {
            return Err(e);
        }
        Ok(run(&s.everything_files, q))
    }
    fn everything_start(&self) -> Result<()> {
        let mut s = self.state();
        match s.everything {
            EverythingStatus::NotInstalled => Err(SearchError::EverythingNotInstalled),
            EverythingStatus::NotRunning => {
                s.actions.push("start everything".into());
                s.everything = s.everything_starts_as;
                s.everything_ours = true;
                Ok(())
            }
            // the user's own copy (or ours, already started): used as it is
            _ => Ok(()),
        }
    }
    fn everything_stop(&self) {
        let mut s = self.state();
        s.everything_released += 1;
        if s.everything_ours {
            s.actions.push("stop everything".into());
            s.everything_ours = false;
            s.everything = EverythingStatus::NotRunning;
        }
    }
    fn everything_install(&self, tidy: bool) -> Result<()> {
        let mut s = self.state();
        s.actions.push(if tidy { "install everything + tidy v1.0.0's".into() } else { "install everything".into() });
        let r = s.install_result.clone();
        if r.is_ok() {
            if s.everything == EverythingStatus::NotInstalled {
                s.everything = EverythingStatus::NotRunning;
            }
            if tidy {
                s.old_everything = false;
            }
        }
        r
    }
    fn everything_update(&self) -> Result<()> {
        let mut s = self.state();
        s.actions.push("update everything".into());
        if s.everything_ours && matches!(s.everything, EverythingStatus::Running { .. }) {
            s.everything = EverythingStatus::Loading { building: true };
        }
        Ok(())
    }
    fn drives(&self) -> (Vec<char>, char) {
        self.state().drives.clone()
    }
    fn set_drives(&self, letters: &[char]) {
        let mut s = self.state();
        if s.covered != letters {
            s.actions.push(format!("drives {}", letters.iter().collect::<String>()));
            s.covered = letters.to_vec();
        }
    }
    fn old_everything(&self) -> bool {
        self.state().old_everything
    }
    fn everything_mine(&self) -> bool {
        !self.state().user_copy
    }
    fn windows_search_status(&self) -> WsStatus {
        self.state().ws
    }
    fn windows_search_scope(&self) -> IndexScope {
        self.state().ws_scope.clone()
    }
    fn windows_search_query(&self, q: &FileQuery) -> Result<Hits> {
        let mut s = self.state();
        s.queries.push(describe("windows-search", q));
        if let Some(e) = s.ws_error.take() {
            return Err(e);
        }
        let mut hits = run(&s.ws_files, q);
        hits.total = None; // the OLE DB side does not know the full count
        Ok(hits)
    }
    fn dir_item_count(&self, path: &str) -> Option<u64> {
        self.state().counts.iter().find(|(p, _)| p == path).map(|(_, n)| *n)
    }
    fn open(&self, target: &OpenTarget) -> Result<()> {
        let mut s = self.state();
        if let Some(e) = s.action_error.take() {
            return Err(e);
        }
        s.actions.push(match target {
            OpenTarget::App(a) => format!("open app {a}"),
            OpenTarget::Path(p) => format!("open {p}"),
        });
        Ok(())
    }
    fn reveal(&self, path: &str) -> Result<()> {
        self.state().actions.push(format!("reveal {path}"));
        Ok(())
    }
    fn open_with(&self, path: &str) -> Result<()> {
        self.state().actions.push(format!("open with {path}"));
        Ok(())
    }
    fn copy_text(&self, text: &str) -> Result<()> {
        self.state().actions.push(format!("copy {text}"));
        Ok(())
    }
}

//! In-memory fake of the OS layer. Every behaviour is tested against this — nothing real is ever changed in a test.
//! Rules it mimics: HKLM writes, task writes and service writes need admin (else `AccessDenied`); names in `refused` are refused
//! even with admin (Windows' locked tasks); names in `ignored` accept the write but keep the old value (like UCPD silently
//! blocking a registry write).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use crate::os::*;

#[derive(Default)]
pub struct FakeState {
    pub admin: bool,
    /// (hive, view, lower-case key path) → string values.
    pub strings: HashMap<(Hive, RegView, String), Vec<RegString>>,
    /// (hive, lower-case key path, lower-case value name) → binary value.
    pub binary: HashMap<(Hive, String, String), Vec<u8>>,
    pub user_folder: Vec<FolderItem>,
    pub common_folder: Vec<FolderItem>,
    pub store: Vec<StoreStartupTask>,
    pub tasks: Vec<RawTask>,
    pub services: Vec<RawService>,
    pub memory: Vec<(String, bool)>,
    /// Existing files (lower-case path) with their version info.
    pub files: HashMap<String, FileInfo>,
    pub env: Vec<(String, String)>,
    pub impact: Option<Result<Vec<String>, OsError>>,
    pub now: u64,
    pub refused: HashSet<String>,
    pub ignored: HashSet<String>,
    /// Source name ("tasks", "services", "store", "folder", "run") → read error.
    pub fail_read: HashMap<&'static str, OsError>,
    /// Every write, in order (for tests).
    pub writes: Vec<String>,
}

#[derive(Default)]
pub struct FakeOs {
    state: Mutex<FakeState>,
}

impl FakeOs {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn state(&self) -> MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
    pub fn admin(self, admin: bool) -> Self {
        self.state().admin = admin;
        self
    }
    pub fn run(self, hive: Hive, view: RegView, key: &str, name: &str, data: &str) -> Self {
        self.state()
            .strings
            .entry((hive, view, key.to_lowercase()))
            .or_default()
            .push(RegString { name: name.into(), data: data.into() });
        self
    }
    pub fn binary(self, hive: Hive, path: &str, name: &str, data: &[u8]) -> Self {
        self.state().binary.insert((hive, path.to_lowercase(), name.to_lowercase()), data.to_vec());
        self
    }
    pub fn file(self, path: &str, company: &str, description: &str) -> Self {
        let opt = |s: &str| if s.is_empty() { None } else { Some(s.to_string()) };
        self.state().files.insert(path.to_lowercase(), FileInfo { company: opt(company), description: opt(description) });
        self
    }
    pub fn folder_item(self, all_users: bool, dir: &str, file_name: &str, target: Option<&str>, args: Option<&str>) -> Self {
        let item = FolderItem {
            file_name: file_name.into(),
            path: PathBuf::from(dir).join(file_name),
            target: target.map(Into::into),
            arguments: args.map(Into::into),
            icon: None,
        };
        let mut s = self.state();
        if all_users {
            s.common_folder.push(item)
        } else {
            s.user_folder.push(item)
        }
        drop(s);
        self
    }
    pub fn task(self, t: RawTask) -> Self {
        self.state().tasks.push(t);
        self
    }
    pub fn service(self, s: RawService) -> Self {
        self.state().services.push(s);
        self
    }
    pub fn store(self, t: StoreStartupTask) -> Self {
        self.state().store.push(t);
        self
    }
    pub fn impact(self, r: Result<Vec<String>, OsError>) -> Self {
        self.state().impact = Some(r);
        self
    }
    pub fn env(self, name: &str, value: &str) -> Self {
        self.state().env.push((name.into(), value.into()));
        self
    }
    pub fn get_binary(&self, hive: Hive, path: &str, name: &str) -> Option<Vec<u8>> {
        self.state().binary.get(&(hive, path.to_lowercase(), name.to_lowercase())).cloned()
    }
}

fn need_admin(s: &FakeState) -> Result<(), OsError> {
    if s.admin {
        Ok(())
    } else {
        Err(OsError::AccessDenied)
    }
}

impl StartupOs for FakeOs {
    fn is_admin(&self) -> bool {
        self.state().admin
    }

    fn reg_strings(&self, hive: Hive, view: RegView, path: &str) -> Result<Vec<RegString>, OsError> {
        let s = self.state();
        if let Some(e) = s.fail_read.get("run") {
            return Err(e.clone());
        }
        Ok(s.strings.get(&(hive, view, path.to_lowercase())).cloned().unwrap_or_default())
    }

    fn reg_binary(&self, hive: Hive, path: &str, name: &str) -> Result<Option<Vec<u8>>, OsError> {
        Ok(self.state().binary.get(&(hive, path.to_lowercase(), name.to_lowercase())).cloned())
    }

    fn reg_set_binary(&self, hive: Hive, path: &str, name: &str, data: &[u8]) -> Result<(), OsError> {
        let mut s = self.state();
        if hive == Hive::LocalMachine {
            need_admin(&s)?;
        }
        s.writes.push(format!("set {hive:?} {path}\\{name} = {data:02x?}"));
        if s.ignored.contains(name) {
            return Ok(());
        }
        s.binary.insert((hive, path.to_lowercase(), name.to_lowercase()), data.to_vec());
        Ok(())
    }

    fn reg_delete_value(&self, hive: Hive, path: &str, name: &str) -> Result<(), OsError> {
        let mut s = self.state();
        if hive == Hive::LocalMachine {
            need_admin(&s)?;
        }
        s.writes.push(format!("delete {hive:?} {path}\\{name}"));
        if s.ignored.contains(name) {
            return Ok(());
        }
        s.binary.remove(&(hive, path.to_lowercase(), name.to_lowercase()));
        Ok(())
    }

    fn startup_folder(&self, all_users: bool) -> Result<Vec<FolderItem>, OsError> {
        let s = self.state();
        if let Some(e) = s.fail_read.get("folder") {
            return Err(e.clone());
        }
        Ok(if all_users { s.common_folder.clone() } else { s.user_folder.clone() })
    }

    fn store_startup_tasks(&self) -> Result<Vec<StoreStartupTask>, OsError> {
        let s = self.state();
        if let Some(e) = s.fail_read.get("store") {
            return Err(e.clone());
        }
        Ok(s.store.clone())
    }

    fn logon_tasks(&self) -> Result<Vec<RawTask>, OsError> {
        let s = self.state();
        if let Some(e) = s.fail_read.get("tasks") {
            return Err(e.clone());
        }
        Ok(s.tasks.clone())
    }

    fn set_task_enabled(&self, path: &str, enabled: bool) -> Result<(), OsError> {
        let mut s = self.state();
        need_admin(&s)?;
        if s.refused.contains(path) {
            return Err(OsError::AccessDenied);
        }
        s.writes.push(format!("task {path} enabled={enabled}"));
        let ignore = s.ignored.contains(path);
        let t = s.tasks.iter_mut().find(|t| t.path.eq_ignore_ascii_case(path)).ok_or(OsError::NotFound)?;
        if !ignore {
            t.enabled = enabled;
        }
        Ok(())
    }

    fn task_enabled(&self, path: &str) -> Result<bool, OsError> {
        self.state().tasks.iter().find(|t| t.path.eq_ignore_ascii_case(path)).map(|t| t.enabled).ok_or(OsError::NotFound)
    }

    fn services(&self, also: &[String]) -> Result<Vec<RawService>, OsError> {
        let s = self.state();
        if let Some(e) = s.fail_read.get("services") {
            return Err(e.clone());
        }
        Ok(s.services
            .iter()
            .filter(|x| x.start == ServiceStart::Automatic || also.iter().any(|n| n.eq_ignore_ascii_case(&x.name)))
            .cloned()
            .collect())
    }

    fn set_service_start(&self, name: &str, start: ServiceStart, delayed: bool) -> Result<(), OsError> {
        let mut s = self.state();
        need_admin(&s)?;
        if s.refused.contains(name) {
            return Err(OsError::AccessDenied);
        }
        s.writes.push(format!("service {name} start={start:?} delayed={delayed}"));
        let ignore = s.ignored.contains(name);
        let x = s.services.iter_mut().find(|x| x.name.eq_ignore_ascii_case(name)).ok_or(OsError::NotFound)?;
        if !ignore {
            x.start = start;
            x.delayed = delayed;
        }
        Ok(())
    }

    fn service_start(&self, name: &str) -> Result<(ServiceStart, bool), OsError> {
        self.state()
            .services
            .iter()
            .find(|x| x.name.eq_ignore_ascii_case(name))
            .map(|x| (x.start, x.delayed))
            .ok_or(OsError::NotFound)
    }

    fn remembered_services(&self) -> Result<Vec<(String, bool)>, OsError> {
        Ok(self.state().memory.clone())
    }

    fn remember_service(&self, name: &str, delayed: bool) -> Result<(), OsError> {
        let mut s = self.state();
        s.memory.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        s.memory.push((name.to_string(), delayed));
        Ok(())
    }

    fn forget_service(&self, name: &str) -> Result<(), OsError> {
        self.state().memory.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        Ok(())
    }

    fn file_info(&self, path: &Path) -> FileInfo {
        self.state().files.get(&path.to_string_lossy().to_lowercase()).cloned().unwrap_or_default()
    }

    fn expand_env(&self, s: &str) -> String {
        // One pass per variable, searching only after the last replacement (a value holding its own %NAME% can't loop).
        let mut out = s.to_string();
        for (k, v) in &self.state().env {
            let pat = format!("%{}%", k.to_lowercase());
            let mut from = 0;
            while let Some(i) = out.get(from..).and_then(|rest| rest.to_lowercase().find(&pat)).map(|i| i + from) {
                if out.len() != out.to_lowercase().len() {
                    break; // lower-casing changed byte lengths: give up rather than cut a character
                }
                out.replace_range(i..i + pat.len(), v);
                from = i + v.len();
            }
        }
        out
    }

    fn file_exists(&self, path: &Path) -> bool {
        self.state().files.contains_key(&path.to_string_lossy().to_lowercase())
    }

    fn subdirs(&self, dir: &Path) -> Vec<PathBuf> {
        // Folders are implied by the fake's files: every direct child folder of `dir` that holds a file.
        let d = format!("{}\\", dir.to_string_lossy().to_lowercase().trim_end_matches('\\'));
        let mut out: Vec<PathBuf> = Vec::new();
        for f in self.state().files.keys() {
            if let Some(rest) = f.strip_prefix(&d) {
                if let Some((sub, _)) = rest.split_once('\\') {
                    let p = PathBuf::from(format!("{d}{sub}"));
                    if !out.contains(&p) {
                        out.push(p);
                    }
                }
            }
        }
        out
    }

    fn impact_reports(&self) -> Result<Vec<String>, OsError> {
        self.state().impact.clone().unwrap_or(Ok(Vec::new()))
    }

    fn now_filetime(&self) -> u64 {
        self.state().now
    }
}

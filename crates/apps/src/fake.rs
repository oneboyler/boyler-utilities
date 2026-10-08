//! In-memory fake of the OS layer — every uninstall in the tests happens here, never on the real PC.
//! An "uninstaller" is scripted per command: what exit code it returns and whether the entry is gone afterwards.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use crate::os::*;

/// What a scripted uninstaller does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Script {
    pub exit_code: u32,
    /// Removes its Uninstall entry (= really uninstalled).
    pub removes_entry: bool,
    /// The user says No at Windows' admin prompt.
    pub prompt_cancelled: bool,
}

#[derive(Default)]
pub struct FakeState {
    pub entries: Vec<RawEntry>,
    pub packages: Vec<RawPackage>,
    /// command → script. A command without a script exits 0 and removes nothing.
    pub scripts: HashMap<String, Script>,
    /// Store packages that refuse removal (error) / stay installed.
    pub package_errors: HashMap<String, OsError>,
    pub folder_sizes: HashMap<PathBuf, u64>,
    /// Paths that are folders (also every path given to `folder`).
    pub dirs: std::collections::HashSet<PathBuf>,
    pub fail_entries: Option<OsError>,
    pub fail_packages: Option<OsError>,
    /// Every action, in order.
    pub ran: Vec<String>,
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
    /// Add an Uninstall entry: `values` as (name, value) pairs.
    pub fn entry(self, hive: Hive, view: RegView, key: &str, values: &[(&str, RegValue)], last_write: Option<u64>) -> Self {
        let values = values.iter().map(|(n, v)| (n.to_lowercase(), v.clone())).collect();
        self.state().entries.push(RawEntry { hive, view, key_name: key.into(), values, last_write });
        self
    }
    pub fn package(self, p: RawPackage) -> Self {
        self.state().packages.push(p);
        self
    }
    pub fn script(self, command: &str, exit_code: u32, removes_entry: bool) -> Self {
        self.state().scripts.insert(command.into(), Script { exit_code, removes_entry, prompt_cancelled: false });
        self
    }
    pub fn script_prompt_cancelled(self, command: &str) -> Self {
        self.state().scripts.insert(command.into(), Script { exit_code: 0, removes_entry: false, prompt_cancelled: true });
        self
    }
    /// A path that exists as a folder (for "Open install folder").
    pub fn dir(self, path: &str) -> Self {
        self.state().dirs.insert(PathBuf::from(path));
        self
    }
    pub fn folder(self, path: &str, size: u64) -> Self {
        self.state().dirs.insert(PathBuf::from(path));
        self.state().folder_sizes.insert(PathBuf::from(path), size);
        self
    }
}

pub fn s(v: &str) -> RegValue {
    RegValue::Str(v.into())
}
pub fn d(v: u32) -> RegValue {
    RegValue::Dword(v)
}

impl AppsOs for FakeOs {
    fn uninstall_entries(&self) -> Result<Vec<RawEntry>, OsError> {
        let st = self.state();
        match &st.fail_entries {
            Some(e) => Err(e.clone()),
            None => Ok(st.entries.clone()),
        }
    }

    fn store_packages(&self) -> Result<Vec<RawPackage>, OsError> {
        let st = self.state();
        match &st.fail_packages {
            Some(e) => Err(e.clone()),
            None => Ok(st.packages.clone()),
        }
    }

    fn entry_exists(&self, hive: Hive, view: RegView, key_name: &str) -> bool {
        self.state().entries.iter().any(|e| e.hive == hive && e.view == view && e.key_name.eq_ignore_ascii_case(key_name))
    }

    fn package_installed(&self, full_name: &str) -> bool {
        self.state().packages.iter().any(|p| p.full_name == full_name)
    }

    fn run_uninstaller(&self, command: &str) -> Result<u32, OsError> {
        let mut st = self.state();
        st.ran.push(format!("run {command}"));
        let script = st.scripts.get(command).cloned().unwrap_or(Script { exit_code: 0, removes_entry: false, prompt_cancelled: false });
        if script.prompt_cancelled {
            return Err(OsError::Cancelled);
        }
        if script.removes_entry {
            // The entry whose command this is goes away.
            st.entries.retain(|e| {
                let cmds = ["quietuninstallstring", "uninstallstring"];
                !cmds.iter().any(|c| matches!(e.values.get(*c), Some(RegValue::Str(x)) if x == command))
                    && format!("MsiExec.exe /X{}", e.key_name) != command
            });
        }
        Ok(script.exit_code)
    }

    fn remove_package(&self, full_name: &str) -> Result<(), OsError> {
        let mut st = self.state();
        st.ran.push(format!("remove {full_name}"));
        if let Some(e) = st.package_errors.get(full_name) {
            return Err(e.clone());
        }
        st.packages.retain(|p| p.full_name != full_name);
        Ok(())
    }

    fn folder_size(&self, path: &Path) -> Option<u64> {
        self.state().folder_sizes.get(path).copied()
    }

    fn expand_env(&self, s: &str) -> String {
        s.replace("%ProgramFiles%", r"C:\Program Files")
    }

    fn run_setup(&self, command: &str) -> Result<u32, OsError> {
        let mut st = self.state();
        st.ran.push(format!("setup {command}"));
        match st.scripts.get(command) {
            Some(s) if s.prompt_cancelled => Err(OsError::Cancelled),
            Some(s) => Ok(s.exit_code),
            None => Ok(0),
        }
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.state().dirs.contains(path)
    }

    fn open_folder(&self, dir: &Path) -> Result<(), OsError> {
        if !self.is_dir(dir) {
            return Err(OsError::NotFound);
        }
        self.state().ran.push(format!("explore {}", dir.display()));
        Ok(())
    }

    fn open_settings(&self, uri: &str) -> Result<(), OsError> {
        if !uri.starts_with("ms-settings:") {
            return Err(OsError::NotFound);
        }
        self.state().ran.push(format!("settings {uri}"));
        Ok(())
    }
}

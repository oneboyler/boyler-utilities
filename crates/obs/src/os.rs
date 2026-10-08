//! Everything the engine asks of Windows, behind one trait: the REAL layer (real.rs) and a FAKE one (fake.rs) for tests -
//! a test never touches the real OBS, its files, the Startup folder or the speakers.

use std::path::{Path, PathBuf};

/// How OBS's exe was found (obsstart.c OBSP_*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    Running,
    Known,
    Install,
    Steam,
}

pub trait ObsOs: Send {
    /// %APPDATA%\obs-studio (tests: a scratch copy)
    fn obs_dir(&self) -> PathBuf;
    /// OBS holds its named mutex while it runs (checking it touches no process)
    fn obs_running(&self) -> bool;
    /// OBS's process: (pid, full exe path - empty if unreadable)
    fn find_obs(&self) -> Option<(u32, PathBuf)>;
    /// Ask OBS to close normally (WM_CLOSE to its main window, like its X). False = no window found.
    fn close_obs(&mut self, pid: u32) -> bool;
    /// Block until the process ends (true) or the time is up (false). Runs on a worker thread.
    fn wait_exit(&self, pid: u32, timeout_ms: u32) -> bool;
    /// the same wait, usable from another thread
    fn waiter(&self) -> Waiter;
    /// Start OBS from its own folder; `quiet` = minimised without focus (a fullscreen game stays in front).
    fn start_obs(&mut self, path: &Path, args: Option<&str>, quiet: bool) -> bool;
    /// OBS's install path from its installer's registry key, else the default folder.
    fn default_path(&self) -> PathBuf;
    /// %ProgramFiles%\obs-studio\bin\64bit\obs64.exe
    fn program_files_path(&self) -> Option<PathBuf>;
    /// OBS's Steam install (Steam's folder + every library)
    fn steam_obs(&self) -> Option<PathBuf>;
    fn file_exists(&self, p: &Path) -> bool;
    /// Free bytes for the user on the drive of `dir` (None = unknown)
    fn free_bytes(&self, dir: &Path) -> Option<u64>;
    /// OBS starts with Windows through something the user made (their own shortcut / Run entry)
    fn user_autostart(&self) -> bool;
    /// the app's own "Start OBS with Windows" shortcut is in the Startup folder
    fn shortcut_exists(&self) -> bool;
    fn shortcut_create(&mut self, obs: &Path) -> bool;
    /// removes ONLY the app's own shortcut; true = it's gone
    fn shortcut_delete(&mut self) -> bool;
    /// letters and digits from Windows' secure random generator
    fn random_password(&self, n: usize) -> Option<String>;
    /// write a whole file: a temp file first, then swapped in (OBS's profile, obs-websocket's config)
    fn write_file(&mut self, p: &Path, text: &str) -> bool;
    fn read_file(&self, p: &Path) -> Option<String>;
    /// play a .wav from memory, asynchronously, on the shared mixer
    fn play(&mut self, wav: Vec<u8>);
    /// NotificationsForOBS.exe running on its own: (pid, exe path)
    fn other_app(&self) -> Option<(u32, PathBuf)>;
    /// close it exactly like its own tray menu's Quit
    fn close_other_app(&mut self, pid: u32) -> bool;
    /// end it (it did not quit): only while that process still runs from `exe` (Order 043)
    fn end_other_app(&mut self, pid: u32, exe: &Path) -> bool;
    /// the exe NotificationsForOBS's "Start with Windows" entry starts
    fn other_app_run_entry(&self) -> Option<PathBuf>;
}

/// Where obs64.exe is: the running OBS first, then a path it was seen running from, then the normal install places, then
/// Steam (obsstart.c `obs_find_exe`).
pub fn find_exe(os: &dyn ObsOs, known: Option<&Path>) -> Option<(PathBuf, Found)> {
    if let Some((_, p)) = os.find_obs() {
        if !p.as_os_str().is_empty() && os.file_exists(&p) {
            return Some((p, Found::Running));
        }
    }
    if let Some(k) = known.filter(|k| !k.as_os_str().is_empty() && os.file_exists(k)) {
        return Some((k.to_path_buf(), Found::Known));
    }
    let d = os.default_path();
    if os.file_exists(&d) {
        return Some((d, Found::Install));
    }
    if let Some(p) = os.program_files_path().filter(|p| os.file_exists(p)) {
        return Some((p, Found::Install));
    }
    os.steam_obs().map(|p| (p, Found::Steam))
}

/// Something that waits for a process to end on another thread (the OS layer itself stays on the engine thread).
pub type Waiter = Box<dyn Fn(u32, u32) -> bool + Send>;

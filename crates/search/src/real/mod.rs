//! The real Windows layer. Apps: `shell:AppsFolder`. Files and folders: Everything through its IPC window messages (the copy
//! the user runs, else our own hidden instance - `host`); Windows Search's index code stays for `search-show` only (no
//! fallback engine: the owner, Oct 8). Open / reveal / open with / clipboard through the shell.
//! `RealOs::read_only()` (what `search-show` and the tests run on) refuses every open and clipboard write on its first line.

mod apps;
pub mod everything;
pub mod host;
mod shell;
pub mod wsearch;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

use crate::error::{Result, SearchError};
use crate::model::{OpenTarget, Stamp};
use crate::os::*;

pub struct RealOs {
    read_only: bool,
    /// The IPC window classes to look for, in order: a copy the user runs (None in tests), then ours.
    user_class: Option<String>,
    our_class: String,
    /// Our instance, when we started it (quit with `everything_stop`).
    ours: Mutex<Option<host::Ours>>,
    /// Ours was started without an index file: while it loads, it is building its file list for the first time.
    ours_building: AtomicBool,
    /// May this layer start / stop / install Everything (the app: yes; `read_only`: never).
    host: bool,
}

impl RealOs {
    /// The app's layer: reads and actions.
    pub fn new() -> RealOs {
        RealOs { read_only: false, user_class: Some(everything::class_of(None)), our_class: everything::class_of(Some(host::INSTANCE)), ours: Mutex::new(None), ours_building: AtomicBool::new(false), host: true }
    }
    /// Reads only; every open / reveal / open-with / clipboard call is refused before Windows is asked, and Everything is
    /// never started, stopped or installed.
    pub fn read_only() -> RealOs {
        let mut o = RealOs::new();
        o.host = false;
        o.read_only = true;
        o
    }

    fn refuse(&self, what: &str) -> Result<()> {
        if self.read_only {
            return Err(SearchError::Refused(what.to_string()));
        }
        Ok(())
    }

    /// Test hook: talk only to the Everything window of this class (a stand-in the test runs), never a real one; nothing
    /// is started, stopped or installed.
    #[doc(hidden)]
    pub fn with_everything_class(mut self, class: &str) -> RealOs {
        self.user_class = None;
        self.our_class = class.to_string();
        self.host = false;
        self
    }

    /// The Everything window that answers: the user's own copy first, then ours.
    fn ev_window(&self) -> Option<windows::Win32::Foundation::HWND> {
        self.ev_window_whose().map(|w| w.0)
    }

    /// Quit ours (`host::stop_ours`): a loaded index - or a saved one still loading - gets the time to be saved, and a clean
    /// quit then marks it whole; one still being built is ended (it leaves no index, and the next start builds again).
    /// (A saved index still loading was ended after 3 s too: its mark was gone, so the next start threw the whole index away
    /// and built again - the tab is mostly left within the 9 s a load takes.)
    fn quit_ours(&self, c: host::Ours) {
        let loaded = everything::find(&self.our_class).map(everything::db_loaded).unwrap_or(false);
        let keep = loaded || !self.ours_building.load(Ordering::SeqCst);
        let clean = host::stop_ours(host::exe().as_deref(), c, if keep { host::SAVE_WAIT } else { std::time::Duration::from_secs(3) });
        if clean && keep {
            if let Some(d) = host::data_dir() {
                host::mark_index(&d);
            }
        }
    }

    /// The same + whether it is ours.
    fn ev_window_whose(&self) -> Option<(windows::Win32::Foundation::HWND, bool)> {
        self.user_class.as_deref().and_then(everything::find).map(|h| (h, false)).or_else(|| everything::find(&self.our_class).map(|h| (h, true)))
    }
}

impl Default for RealOs {
    fn default() -> Self {
        RealOs::new()
    }
}

impl Drop for RealOs {
    fn drop(&mut self) {
        // the app quits: our instance goes with it
        let c = self.ours.get_mut().ok().and_then(|o| o.take());
        if let Some(c) = c {
            self.quit_ours(c);
        }
    }
}

/// COM for the shell and OLE DB calls on this thread; balanced on drop.
pub(crate) struct Com(bool);

impl Com {
    pub fn sta() -> Com {
        // S_OK / S_FALSE must be balanced with CoUninitialize; RPC_E_CHANGED_MODE (already MTA) must not be.
        Com(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok() })
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub(crate) fn os_err(call: &str, e: windows::core::Error) -> SearchError {
    SearchError::Os { call: call.to_string(), code: e.code().0 as u32, text: e.message() }
}

pub(crate) fn stamp_from_systemtime_utc(utc: &SYSTEMTIME) -> Option<Stamp> {
    let mut local = SYSTEMTIME::default();
    unsafe { SystemTimeToTzSpecificLocalTime(None, utc, &mut local).ok()? };
    Some(Stamp::new(local.wYear, local.wMonth as u8, local.wDay as u8, local.wHour as u8, local.wMinute as u8))
}

pub(crate) fn stamp_from_filetime(ft: FILETIME) -> Option<Stamp> {
    if ft.dwLowDateTime == 0 && ft.dwHighDateTime == 0 {
        return None;
    }
    let mut st = SYSTEMTIME::default();
    unsafe { FileTimeToSystemTime(&ft, &mut st).ok()? };
    stamp_from_systemtime_utc(&st)
}

impl SearchOs for RealOs {
    fn list_apps(&self) -> Result<Vec<AppEntry>> {
        apps::list()
    }

    fn everything_status(&self) -> EverythingStatus {
        match self.ev_window_whose() {
            Some((h, mine)) => match everything::major_version(h) {
                Some(v) if everything::db_loaded(h) => EverythingStatus::Running { version: v },
                Some(_) => EverythingStatus::Loading { building: mine && self.ours_building.load(Ordering::SeqCst) },
                None => EverythingStatus::NotRunning,
            },
            None if self.host && host::exe().is_none() => EverythingStatus::NotInstalled,
            None => EverythingStatus::NotRunning,
        }
    }

    fn everything_query(&self, q: &FileQuery) -> Result<Hits> {
        match self.ev_window() {
            Some(h) => everything::query(h, q, std::time::Duration::from_secs(5)),
            None => Err(SearchError::Everything("Everything is not running".into())),
        }
    }

    fn everything_start(&self) -> Result<()> {
        // start and stop are serialised by the `ours` lock (a stop in progress finishes first, then this looks again)
        let mut ours = self.ours.lock().unwrap();
        if self.ev_window().is_some() {
            return Ok(());
        }
        if !self.host {
            return Err(SearchError::Refused("start Everything".into()));
        }
        // ours was started and still runs (its window not up yet): wait for it, never a second one
        if let Some(c) = ours.as_mut() {
            if let Ok(None) = c.child.try_wait() {
                return Ok(());
            }
            *ours = None;
        }
        let exe = host::exe().ok_or(SearchError::EverythingNotInstalled)?;
        let dir = host::data_dir().ok_or_else(|| SearchError::Everything("no LOCALAPPDATA".into()))?;
        // no whole index of today's settings: it makes its file list (the page says "building"; later starts load the saved one)
        self.ours_building.store(host::prepare_index(&dir), Ordering::SeqCst);
        *ours = Some(host::Ours::new(host::start_ours(&exe, &dir)?));
        Ok(())
    }

    fn everything_stop(&self) {
        // the lock is held until ours is gone: a start meanwhile waits and then starts a fresh one
        let mut ours = self.ours.lock().unwrap();
        if let Some(c) = ours.take() {
            self.quit_ours(c);
        }
    }

    fn everything_install(&self) -> Result<()> {
        if !self.host {
            return Err(SearchError::Refused("install Everything".into()));
        }
        host::install()
    }

    fn windows_search_status(&self) -> WsStatus {
        wsearch::service_status()
    }

    fn windows_search_scope(&self) -> IndexScope {
        wsearch::scope()
    }

    fn windows_search_query(&self, q: &FileQuery) -> Result<Hits> {
        wsearch::query(q)
    }

    fn dir_item_count(&self, path: &str) -> Option<u64> {
        std::fs::read_dir(path).ok().map(|r| r.count() as u64)
    }

    fn open(&self, target: &OpenTarget) -> Result<()> {
        self.refuse("open")?;
        shell::open(target)
    }

    fn reveal(&self, path: &str) -> Result<()> {
        self.refuse("reveal")?;
        shell::reveal(path)
    }

    fn open_with(&self, path: &str) -> Result<()> {
        self.refuse("open with")?;
        shell::open_with(path)
    }

    fn copy_text(&self, text: &str) -> Result<()> {
        self.refuse("clipboard")?;
        shell::copy_text(text)
    }
}

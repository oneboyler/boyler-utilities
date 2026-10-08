//! The fake OS for tests and test copies of the app: Raw Accel's state in memory, the admin prompt's answer chosen by the
//! test, a log of every elevated run. Nothing on the PC is read or changed.

use crate::error::{AddonError, Result};
use crate::os::{AddonOs, Elevated, HelperAction};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The tool runs and works: install sets the filter, uninstall clears it (the running state changes only at a
    /// pretend restart, `restart()`).
    Works,
    Declined,
    Fails(String),
}

#[derive(Debug)]
struct State {
    filter: bool,
    running: bool,
    answer: Answer,
    signature_ok: bool,
    runs: Vec<(HelperAction, PathBuf)>,
}

#[derive(Debug)]
pub struct FakeOs {
    s: Mutex<State>,
}

impl FakeOs {
    /// Raw Accel absent.
    pub fn new() -> Self {
        FakeOs { s: Mutex::new(State { filter: false, running: false, answer: Answer::Works, signature_ok: true, runs: Vec::new() }) }
    }
    /// Raw Accel installed and running.
    pub fn installed() -> Self {
        let f = Self::new();
        f.set_installed(true, true);
        f
    }
    pub fn set_installed(&self, filter: bool, running: bool) {
        let mut s = self.s.lock().unwrap();
        s.filter = filter;
        s.running = running;
    }
    pub fn answer(&self, a: Answer) {
        self.s.lock().unwrap().answer = a;
    }
    pub fn signature_ok(&self, ok: bool) {
        self.s.lock().unwrap().signature_ok = ok;
    }
    /// A pretend restart: the driver runs exactly when the filter is set.
    pub fn restart(&self) {
        let mut s = self.s.lock().unwrap();
        s.running = s.filter;
    }
    pub fn runs(&self) -> Vec<(HelperAction, PathBuf)> {
        self.s.lock().unwrap().runs.clone()
    }
}

impl Default for FakeOs {
    fn default() -> Self {
        Self::new()
    }
}

impl AddonOs for FakeOs {
    fn rawaccel_filter_set(&self) -> bool {
        self.s.lock().unwrap().filter
    }
    fn rawaccel_running(&self) -> bool {
        self.s.lock().unwrap().running
    }
    fn verify_signature(&self, _file: &Path) -> Result<()> {
        if self.s.lock().unwrap().signature_ok {
            Ok(())
        } else {
            Err(AddonError::Verify("Windows did not trust the driver\u{2019}s signature".into()))
        }
    }
    fn run_elevated(&self, action: HelperAction, folder: &Path, _stop: &dyn Fn() -> bool) -> Elevated {
        let mut s = self.s.lock().unwrap();
        s.runs.push((action, folder.to_path_buf()));
        match s.answer.clone() {
            Answer::Works => {
                s.filter = action == HelperAction::RawAccelInstall;
                Elevated::Done
            }
            Answer::Declined => Elevated::Declined,
            Answer::Fails(m) => Elevated::Failed(m),
        }
    }
}

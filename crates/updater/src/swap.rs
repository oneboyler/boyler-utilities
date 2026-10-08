//! The tiny step that replaces the app after it has exited. It runs in a small separate process (a copy of the OLD, known-good
//! app started with `--bu-update-helper <plan.json>`, see [`crate::run_helper_if_requested`]) because a running exe can't
//! overwrite itself on Windows. Order of events, and what is left on disk if it stops at any point:
//!
//! 1. wait (no polling: a Windows wait on the process) until the old app has exited. Timeout -> nothing changed, the new file is
//!    removed, and NO second copy of the old app is started (the old one is still running).
//! 2. check the staged file again (size + SHA-256 when the release listed one).
//! 3. rename `BoylerUtilities.exe` -> `BoylerUtilities.exe.update-old` (the old version is KEPT).
//! 4. rename `BoylerUtilities.exe.update-new` -> `BoylerUtilities.exe`.
//! 5. start the new app with `--bu-updated <event> <old version>` and wait for it to call [`crate::confirm_started`] (a Windows
//!    event, no polling) - or to exit early, or to time out.
//! 6. confirmed -> delete the old file. Not confirmed / any step above failed -> ROLL BACK: stop the new one, put the old file
//!    back, start the old app again, and write the reason to the result file for the next start (`take_last_result`).
//!
//! If even the roll back fails the old file stays as `*.update-old` and the result says `RollbackFailed` with the path.

use crate::sha256::Sha256;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwapPlan {
    /// The installed app (`...\BoylerUtilities.exe`) - replaced.
    pub target: PathBuf,
    /// The verified new file, next to the target.
    pub new_file: PathBuf,
    /// Where the old file is kept until the new one has started.
    pub old_file: PathBuf,
    /// Written only when the update did NOT go through; read once by the app via `take_last_result`.
    pub result_file: PathBuf,
    /// The app that is exiting.
    pub old_pid: u32,
    pub expected_size: u64,
    pub expected_sha256: Option<String>,
    pub from_version: String,
    pub to_version: String,
    /// Name of the Windows event the new app signals when it is up.
    pub event_name: String,
    pub exit_wait_ms: u64,
    pub start_wait_ms: u64,
    pub rename_retries: u32,
    pub rename_retry_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartOutcome {
    Started,
    /// The new app ended before saying it was up.
    Exited,
    TimedOut,
}

/// The process side of the swap (waiting, starting, stopping). Real = `os_win::RealProcs`; tests use [`FakeProcs`].
pub trait Procs {
    /// True when the process is gone (or never existed), false on timeout.
    fn wait_for_exit(&mut self, pid: u32, timeout: Duration) -> bool;
    /// Must exist before the new app is started so it can open it.
    fn create_start_event(&mut self, name: &str) -> io::Result<()>;
    /// Starts `exe` with `args` (detached, no window of ours); returns a token for the next two calls.
    fn launch(&mut self, exe: &Path, args: &[String]) -> io::Result<u32>;
    fn wait_started(&mut self, child: u32, timeout: Duration) -> StartOutcome;
    fn kill(&mut self, child: u32);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SwapStatus {
    /// The new app started and confirmed; the old file is deleted.
    Updated,
    /// The new app did not start properly; the old app is back and was started again.
    RolledBack,
    /// Nothing was changed (old app didn't exit, bad file, couldn't move the old file).
    Aborted,
    /// The old file could not be put back. It is still on disk under `old_file`.
    RollbackFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwapResult {
    pub status: SwapStatus,
    pub from_version: String,
    pub to_version: String,
    /// Plain words, for the app to show.
    pub message: String,
}

fn ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

/// SHA-256 + length of a file, streamed.
pub(crate) fn hash_file(path: &Path, mut on_bytes: impl FnMut(u64)) -> io::Result<(u64, String)> {
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut total = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        total += n as u64;
        on_bytes(total);
    }
    Ok((total, crate::sha256::to_hex(&h.finalize())))
}

/// Retry a file operation a few times: antivirus and the search indexer briefly hold fresh exe files on Windows.
fn retry<T>(plan: &SwapPlan, mut op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut last = None;
    for i in 0..=plan.rename_retries {
        match op() {
            Ok(v) => return Ok(v),
            Err(e) => {
                last = Some(e);
                if i < plan.rename_retries {
                    std::thread::sleep(ms(plan.rename_retry_ms));
                }
            }
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("no attempt")))
}

fn remove_if_exists(plan: &SwapPlan, p: &Path) -> io::Result<()> {
    retry(plan, || match fs::remove_file(p) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        r => r,
    })
}

fn write_result(plan: &SwapPlan, r: &SwapResult) {
    if let Ok(text) = serde_json::to_string_pretty(r) {
        let _ = fs::write(&plan.result_file, text);
    }
}

fn finish(plan: &SwapPlan, status: SwapStatus, message: impl Into<String>) -> SwapResult {
    let r = SwapResult { status, from_version: plan.from_version.clone(), to_version: plan.to_version.clone(), message: message.into() };
    if status != SwapStatus::Updated {
        write_result(plan, &r);
    }
    r
}

/// Put the old file back (the new, bad one is deleted first). Returns an error text when it could not.
fn restore_old(plan: &SwapPlan) -> Result<(), String> {
    if plan.target.exists() {
        remove_if_exists(plan, &plan.target).map_err(|e| format!("could not remove the new file ({e})"))?;
    }
    retry(plan, || fs::rename(&plan.old_file, &plan.target)).map_err(|e| format!("could not move the old version back ({e})"))
}

/// What the install step is told on its command line, besides the plan file, so that it can still leave the user with a running
/// app when the plan file can't be read: `<target> <result file> <old pid> <exit wait ms>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fallback {
    pub target: PathBuf,
    pub result_file: PathBuf,
    pub old_pid: u32,
    pub exit_wait_ms: u64,
}

impl Fallback {
    pub fn to_args(&self) -> Vec<String> {
        vec![
            self.target.to_string_lossy().into_owned(),
            self.result_file.to_string_lossy().into_owned(),
            self.old_pid.to_string(),
            self.exit_wait_ms.to_string(),
        ]
    }

    /// `None` when the four values aren't all there (an older caller) or the numbers don't read.
    pub fn from_args(args: &[String]) -> Option<Fallback> {
        if args.len() < 4 || args[0].is_empty() || args[1].is_empty() {
            return None;
        }
        Some(Fallback { target: PathBuf::from(&args[0]), result_file: PathBuf::from(&args[1]), old_pid: args[2].parse().ok()?, exit_wait_ms: args[3].parse().ok()? })
    }
}

/// The plan file could not be read, so nothing is known about the staged files and NOTHING IS CHANGED. The app already left
/// (or is leaving): wait for it to be really gone, write the reason to the result file (the app shows it at its next start;
/// the half-done files are tidied by `cleanup_leftovers` then) and start the installed app again. When the old app does not
/// go away in time it is still running, so no second copy is started.
pub fn recover_without_plan(fb: &Fallback, procs: &mut dyn Procs) -> SwapResult {
    let gone = procs.wait_for_exit(fb.old_pid, ms(fb.exit_wait_ms));
    let message = if gone {
        "the update could not start (its instructions could not be read), so nothing was changed"
    } else {
        "the update could not start (its instructions could not be read) and the app did not close in time, so nothing was changed"
    };
    let r = SwapResult { status: SwapStatus::Aborted, from_version: String::new(), to_version: String::new(), message: message.to_string() };
    if let Ok(text) = serde_json::to_string_pretty(&r) {
        let _ = fs::write(&fb.result_file, text);
    }
    if gone {
        let _ = procs.launch(&fb.target, &[]);
    }
    r
}

/// Run the whole swap. Never panics; every outcome is a [`SwapResult`] (also written to `plan.result_file` unless `Updated`).
pub fn run_swap(plan: &SwapPlan, procs: &mut dyn Procs) -> SwapResult {
    // 1. the old app must be gone
    if !procs.wait_for_exit(plan.old_pid, ms(plan.exit_wait_ms)) {
        let _ = remove_if_exists(plan, &plan.new_file);
        return finish(plan, SwapStatus::Aborted, "the app did not close in time, so nothing was changed");
    }
    // from here on the old app is closed: every way out writes the result, then starts it again

    // 2. the staged file is still the verified one
    match hash_file(&plan.new_file, |_| {}) {
        Err(e) => {
            return abort_and_relaunch(plan, procs, format!("the downloaded file could not be read ({e}); nothing was changed"));
        }
        Ok((len, hash)) => {
            let size_bad = plan.expected_size != 0 && len != plan.expected_size;
            let hash_bad = plan.expected_sha256.as_deref().is_some_and(|h| !h.eq_ignore_ascii_case(&hash));
            if size_bad || hash_bad {
                let _ = remove_if_exists(plan, &plan.new_file);
                return abort_and_relaunch(plan, procs, "the downloaded file changed before it was installed; nothing was changed");
            }
        }
    }

    // 3. keep the old version
    if let Err(e) = remove_if_exists(plan, &plan.old_file) {
        let _ = remove_if_exists(plan, &plan.new_file);
        return abort_and_relaunch(plan, procs, format!("a leftover old file could not be removed ({e}); nothing was changed"));
    }
    if let Err(e) = retry(plan, || fs::rename(&plan.target, &plan.old_file)) {
        let _ = remove_if_exists(plan, &plan.new_file);
        return abort_and_relaunch(plan, procs, format!("the current version could not be moved aside ({e}); nothing was changed"));
    }

    // 4. put the new one in place
    if let Err(e) = retry(plan, || fs::rename(&plan.new_file, &plan.target)) {
        let why = format!("the new version could not be put in place ({e})");
        return rollback(plan, procs, None, &why);
    }

    // 5. start it and wait for "I am up"
    if let Err(e) = procs.create_start_event(&plan.event_name) {
        return rollback(plan, procs, None, &format!("could not prepare the start check ({e})"));
    }
    let args = vec!["--bu-updated".to_string(), plan.event_name.clone(), plan.from_version.clone()];
    let child = match procs.launch(&plan.target, &args) {
        Ok(c) => c,
        Err(e) => return rollback(plan, procs, None, &format!("the new version could not be started ({e})")),
    };
    match procs.wait_started(child, ms(plan.start_wait_ms)) {
        StartOutcome::Started => {
            // 6. all good - the old file is no longer needed (if it can't be removed now, cleanup_leftovers gets it next time)
            let _ = remove_if_exists(plan, &plan.old_file);
            finish(plan, SwapStatus::Updated, format!("updated from {} to {}", plan.from_version, plan.to_version))
        }
        StartOutcome::Exited => rollback(plan, procs, Some(child), "the new version closed right after starting"),
        StartOutcome::TimedOut => rollback(plan, procs, Some(child), "the new version did not finish starting in time"),
    }
}

/// Nothing was changed: record why, then start the old app again (it is closed; the user must not be left without it).
fn abort_and_relaunch(plan: &SwapPlan, procs: &mut dyn Procs, msg: impl Into<String>) -> SwapResult {
    let r = finish(plan, SwapStatus::Aborted, msg);
    let _ = procs.launch(&plan.target, &[]);
    r
}

fn rollback(plan: &SwapPlan, procs: &mut dyn Procs, child: Option<u32>, why: &str) -> SwapResult {
    if let Some(c) = child {
        procs.kill(c);
    }
    match restore_old(plan) {
        Ok(()) => {
            let _ = remove_if_exists(plan, &plan.new_file);
            // the result is written BEFORE the old app starts, so its first look finds it
            let r = finish(plan, SwapStatus::RolledBack, format!("{why}; the previous version is back"));
            let _ = procs.launch(&plan.target, &[]);
            r
        }
        Err(e) => finish(
            plan,
            SwapStatus::RollbackFailed,
            format!("{why}; and the previous version could not be restored: {e}. It is kept as {}", plan.old_file.display()),
        ),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// the fake, for tests

/// Scripted [`Procs`]: no processes. Records what was started (arguments + the bytes of the file at that moment, so a test can
/// tell "the new app" from "the old app").
pub struct FakeProcs {
    pub old_exits: bool,
    /// Outcome for each `wait_started`, in order (last one repeats). Default: Started.
    pub start_outcomes: Vec<StartOutcome>,
    /// Make the n-th launch (0-based) fail.
    pub fail_launch: Option<usize>,
    pub launches: Vec<(Vec<String>, Vec<u8>)>,
    pub killed: Vec<u32>,
    pub events: Vec<String>,
    /// Called at the start of every launch with its 0-based number (a test can lock a file at exactly that moment).
    pub on_launch: Option<Box<dyn FnMut(usize)>>,
    #[doc(hidden)]
    pub waits: usize,
}

impl Default for FakeProcs {
    fn default() -> Self {
        FakeProcs { old_exits: true, start_outcomes: vec![StartOutcome::Started], fail_launch: None, launches: vec![], killed: vec![], events: vec![], on_launch: None, waits: 0 }
    }
}

impl FakeProcs {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Procs for FakeProcs {
    fn wait_for_exit(&mut self, _pid: u32, _timeout: Duration) -> bool {
        self.old_exits
    }
    fn create_start_event(&mut self, name: &str) -> io::Result<()> {
        self.events.push(name.to_string());
        Ok(())
    }
    fn launch(&mut self, exe: &Path, args: &[String]) -> io::Result<u32> {
        let n = self.launches.len();
        if let Some(hook) = self.on_launch.as_mut() {
            hook(n);
        }
        self.launches.push((args.to_vec(), fs::read(exe).unwrap_or_default()));
        if self.fail_launch == Some(n) {
            return Err(io::Error::other("fake launch failure"));
        }
        Ok(n as u32 + 1)
    }
    fn wait_started(&mut self, _child: u32, _timeout: Duration) -> StartOutcome {
        let o = self.start_outcomes.get(self.waits).or(self.start_outcomes.last()).copied().unwrap_or(StartOutcome::Started);
        self.waits += 1;
        o
    }
    fn kill(&mut self, child: u32) {
        self.killed.push(child);
    }
}

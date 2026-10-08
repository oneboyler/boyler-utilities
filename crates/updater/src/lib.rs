//! `bu-updater` — the app updates ITSELF from a GitHub release, only when asked. No UI (the Settings page and the small
//! "Updating…" screen are later orders; this crate gives them the calls and the progress events).
//!
//! * [`Updater::check`] — the Settings button. Asks GitHub's releases API for the newest release of `owner/name` (the repo is a
//!   setting; empty = [`CheckResult::NotSetUp`], answered WITHOUT touching the network) and compares versions.
//!   **No other network use exists**: no timer, no background thread, nothing on start.
//! * [`Updater::update`] — downloads the release's Windows `.exe` next to the running app, verifies it (size, `MZ` header, and
//!   SHA-256 when the release lists one), then starts the small install step and returns [`UpdateReady`]. **The app must exit
//!   right after** (it should show "Restarting…"): the install step waits for it, swaps the files, starts the new version,
//!   waits for it to say it is up, and only then deletes the old file - or rolls back (see [`swap`]).
//! * The app's `main` must, as its very first lines: call [`run_helper_if_requested`] (it IS the install step when started with
//!   `--bu-update-helper`) and, once its window/tray is up, [`confirm_started`]. At start it may call [`cleanup_leftovers`] and
//!   [`take_last_result`] (to show "the update did not work: …").
//!
//! Tests never use the internet or the real app folder: logic against [`FakeHttp`] / [`FakeProcs`]; the real WinHTTP client and
//! the real swap with real processes against a local server and a fake app in a scratch folder.

mod error;
pub mod http;
#[cfg(windows)]
mod http_win;
#[cfg(windows)]
mod os_win;
pub mod release;
pub mod sha256;
pub mod swap;
pub mod version;

pub use error::{Result, UpdateError};
pub use http::{FakeHttp, FakeReply, Http, Request, Response};
#[cfg(windows)]
pub use http_win::WinHttp;
pub use release::{AssetInfo, ReleaseInfo};
pub use swap::{recover_without_plan, run_swap, Fallback, FakeProcs, Procs, StartOutcome, SwapPlan, SwapResult, SwapStatus};
pub use version::Version;

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The argument that makes the app exe act as the install step: `<exe> --bu-update-helper <plan.json>`.
pub const HELPER_ARG: &str = "--bu-update-helper";
/// The argument the install step starts the NEW app with: `<exe> --bu-updated <event name> <old version>`.
pub const UPDATED_ARG: &str = "--bu-updated";

/// Everything the updater needs to know. Nothing is read from disk or the network to build it.
#[derive(Debug, Clone)]
pub struct UpdaterConfig {
    /// `owner/name` on GitHub. Empty = updates are not set up yet (there is no repo yet).
    pub repo: String,
    /// `https://api.github.com`; tests point it at a local server.
    pub api_base: String,
    /// The running app's version (the app passes its own `CARGO_PKG_VERSION`).
    pub current_version: String,
    /// The running app's exe (what gets replaced).
    pub install_exe: PathBuf,
    /// Where the install step's own copy goes while it works (default: the system temp folder).
    pub temp_dir: PathBuf,
    pub user_agent: String,
    /// Safety limit for the downloaded file.
    pub max_download_bytes: u64,
    /// How long the install step waits for the app to exit.
    pub exit_wait: Duration,
    /// How long the install step waits for the new app to say it is up.
    pub start_wait: Duration,
}

impl UpdaterConfig {
    pub fn new(repo: &str, current_version: &str, install_exe: impl Into<PathBuf>) -> Self {
        UpdaterConfig {
            repo: repo.trim().to_string(),
            api_base: "https://api.github.com".to_string(),
            current_version: current_version.to_string(),
            install_exe: install_exe.into(),
            temp_dir: std::env::temp_dir(),
            user_agent: format!("BoylerUtilities/{current_version}"),
            max_download_bytes: 300 * 1024 * 1024,
            exit_wait: Duration::from_secs(60),
            start_wait: Duration::from_secs(20),
        }
    }

    fn exe_name(&self) -> String {
        self.install_exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// `<exe>.update-<suffix>` next to the exe.
    fn side_file(&self, suffix: &str) -> PathBuf {
        let mut n = self.install_exe.clone().into_os_string();
        n.push(format!(".update-{suffix}"));
        PathBuf::from(n)
    }
}

/// What `check()` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckResult {
    /// The repo setting is empty. Not an error, no network was used.
    NotSetUp,
    /// The repo has no published release yet.
    NoReleases,
    UpToDate { current: Version, latest: Version },
    Available(ReleaseInfo),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Downloading,
    Verifying,
    /// Putting the file in place and starting the install step.
    Installing,
    /// Done on our side: the app must exit now so the install step can swap the files.
    Restarting,
}

/// One progress event for the "Updating…" screen. `percent` is of the current phase (None when its total isn't known).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub phase: Phase,
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    pub percent: Option<u8>,
}

/// How far the downloaded file was checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verification {
    /// Size and SHA-256 both matched what the release lists.
    Sha256,
    /// The release listed no SHA-256: only the size (and the `MZ` header) could be checked.
    SizeOnly,
    /// The release listed neither: only the `MZ` header was checked.
    HeaderOnly,
}

/// `update()` succeeded: the app must exit now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateReady {
    pub from: String,
    pub to: String,
    pub bytes: u64,
    pub verification: Verification,
}

/// Starts the install step. Real: copies the running exe into the temp folder and starts it with [`HELPER_ARG`].
pub trait Installer: Send + Sync {
    fn start_swap(&self, config: &UpdaterConfig, plan: &SwapPlan) -> Result<()>;
}

/// Records the plan instead of starting anything (tests).
#[derive(Default)]
pub struct FakeInstaller {
    pub plans: std::sync::Mutex<Vec<SwapPlan>>,
    pub fail_with: Option<String>,
}

impl Installer for FakeInstaller {
    fn start_swap(&self, _config: &UpdaterConfig, plan: &SwapPlan) -> Result<()> {
        if let Some(why) = &self.fail_with {
            return Err(UpdateError::HelperStart(why.clone()));
        }
        self.plans.lock().unwrap().push(plan.clone());
        Ok(())
    }
}

impl<T: Installer + ?Sized> Installer for std::sync::Arc<T> {
    fn start_swap(&self, config: &UpdaterConfig, plan: &SwapPlan) -> Result<()> {
        (**self).start_swap(config, plan)
    }
}

#[cfg(windows)]
pub struct RealInstaller;

#[cfg(windows)]
impl Installer for RealInstaller {
    fn start_swap(&self, config: &UpdaterConfig, plan: &SwapPlan) -> Result<()> {
        let dir = config.temp_dir.join(format!("BoylerUtilities-update-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).map_err(|e| UpdateError::HelperStart(format!("temp folder: {e}")))?;
        // The install step is a copy of the OLD app: it is known to start, and it does not depend on the new file being healthy.
        let helper = dir.join("install-step.exe");
        fs::copy(&config.install_exe, &helper).map_err(|e| UpdateError::HelperStart(format!("copying the app: {e}")))?;
        let plan_file = dir.join("plan.json");
        let text = serde_json::to_string_pretty(plan).map_err(|e| UpdateError::HelperStart(e.to_string()))?;
        fs::write(&plan_file, text).map_err(|e| UpdateError::HelperStart(format!("writing the plan: {e}")))?;
        // the plan file first; then what the step needs to leave the user with a running app even if it can't read the plan
        let fallback = swap::Fallback { target: plan.target.clone(), result_file: plan.result_file.clone(), old_pid: plan.old_pid, exit_wait_ms: plan.exit_wait_ms };
        let mut args = vec![HELPER_ARG.to_string(), plan_file.to_string_lossy().into_owned()];
        args.extend(fallback.to_args());
        os_win::spawn_detached(&helper, &args)
            .map_err(|e| UpdateError::HelperStart(e.to_string()))?;
        Ok(())
    }
}

pub struct Updater {
    config: UpdaterConfig,
    http: Box<dyn Http>,
    installer: Box<dyn Installer>,
    /// True while an `update()` runs (only one at a time).
    busy: AtomicBool,
    /// Set by `cancel()` while `busy`; read by the running `update()`.
    cancel: AtomicBool,
}

/// Clears `busy` when `update()` ends by any way (return, error, panic).
struct BusyGuard<'a>(&'a AtomicBool);

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl Updater {
    pub fn new(config: UpdaterConfig, http: Box<dyn Http>, installer: Box<dyn Installer>) -> Self {
        Updater { config, http, installer, busy: AtomicBool::new(false), cancel: AtomicBool::new(false) }
    }

    /// The real thing: WinHTTP + the real install step.
    #[cfg(windows)]
    pub fn real(config: UpdaterConfig) -> Self {
        let http = WinHttp::new(&config.user_agent);
        Updater::new(config, Box::new(http), Box::new(RealInstaller))
    }

    pub fn config(&self) -> &UpdaterConfig {
        &self.config
    }

    /// The Settings button. The ONLY place that asks the network "is there something new?".
    pub fn check(&self) -> Result<CheckResult> {
        if self.config.repo.trim().is_empty() {
            return Ok(CheckResult::NotSetUp);
        }
        let repo = validate_repo(&self.config.repo)?;
        let current = Version::parse(&self.config.current_version)?;
        let url = format!("{}/repos/{}/releases/latest", self.config.api_base.trim_end_matches('/'), repo);
        let mut sink = http::LimitedVec { buf: Vec::new(), max: 4 * 1024 * 1024 };
        let resp = self.http.get(&Request { url: &url, accept: "application/vnd.github+json" }, &mut sink, &mut |_, _| {})?;
        match resp.status {
            200..=299 => {}
            404 => return Ok(CheckResult::NoReleases),
            403 | 429 => return Err(UpdateError::RateLimited),
            status => return Err(UpdateError::HttpStatus { status, what: "looking for the newest release".into() }),
        }
        let parsed = release::parse_release(&sink.buf)?;
        if parsed.draft || parsed.prerelease || parsed.version <= current {
            let latest = if parsed.draft || parsed.prerelease { current.clone() } else { parsed.version };
            return Ok(CheckResult::UpToDate { current, latest });
        }
        Ok(CheckResult::Available(parsed.into_info(&self.config.exe_name())?))
    }

    /// True while an `update()` call is running.
    pub fn is_updating(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    /// The Cancel button of the "Updating…" screen; callable from any thread (`Updater` is `Sync`). Asks the running `update()`
    /// to stop: it ends with [`UpdateError::Cancelled`] and removes everything it wrote. Takes effect at the next received
    /// chunk or the next phase edge (a connection that sends nothing ends by its own 15-30 s timeout), and only until the
    /// install step has been started - after that the app is on its way out and the update can no longer be stopped.
    /// Does nothing when no `update()` is running (a stale click can never cancel the NEXT update).
    pub fn cancel(&self) {
        if self.busy.load(Ordering::SeqCst) {
            self.cancel.store(true, Ordering::SeqCst);
        }
    }

    /// Download, verify, stage, start the install step. Returns when the app must exit. Only one `update()` runs at a time
    /// ([`UpdateError::AlreadyRunning`] for a second call); [`Updater::cancel`] stops it ([`UpdateError::Cancelled`]).
    pub fn update(&self, release: &ReleaseInfo, progress: &mut dyn FnMut(&Progress)) -> Result<UpdateReady> {
        if self.busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            return Err(UpdateError::AlreadyRunning);
        }
        let _busy = BusyGuard(&self.busy);
        self.cancel.store(false, Ordering::SeqCst);
        let part = self.config.side_file("part");
        let staged = self.config.side_file("new");
        let result = self.update_inner(release, &part, &staged, progress);
        if result.is_err() {
            let _ = fs::remove_file(&part);
            let _ = fs::remove_file(&staged);
        }
        result
    }

    fn update_inner(&self, release: &ReleaseInfo, part: &Path, staged: &Path, progress: &mut dyn FnMut(&Progress)) -> Result<UpdateReady> {
        let asset = &release.asset;
        // never downgrade the transport, never fetch something too big
        if self.config.api_base.starts_with("https://") && !asset.url.starts_with("https://") {
            return Err(UpdateError::InsecureDownload(asset.url.clone()));
        }
        if asset.size > self.config.max_download_bytes {
            return Err(UpdateError::TooLarge { size: asset.size, limit: self.config.max_download_bytes });
        }
        if !self.config.install_exe.is_file() {
            return Err(UpdateError::Io { context: "finding the running app".into(), detail: format!("{} is not a file", self.config.install_exe.display()) });
        }
        // creating the file proves the folder can be written, before a single byte is downloaded
        let _ = fs::remove_file(staged);
        let file = fs::File::create(part).map_err(|e| UpdateError::InstallDirNotWritable(format!("{}: {e}", part.display())))?;

        let mut rep = Reporter::new(progress);
        rep.emit(Phase::Downloading, 0, if asset.size > 0 { Some(asset.size) } else { None }, true);
        let limit = if asset.size > 0 { asset.size } else { self.config.max_download_bytes };
        let mut sink = CountingFile { file, written: 0, limit, exceeded: false, cancel: &self.cancel, cancelled: false };
        let get = self.http.get(&Request { url: &asset.url, accept: "application/octet-stream" }, &mut sink, &mut |done, total| {
            rep.emit(Phase::Downloading, done, total.or(if asset.size > 0 { Some(asset.size) } else { None }), false);
        });
        let (written, exceeded, cancelled) = (sink.written, sink.exceeded, sink.cancelled);
        sink.file.flush().map_err(|e| UpdateError::io("saving the download", &e))?;
        drop(sink);
        if cancelled || self.cancel.load(Ordering::SeqCst) {
            return Err(UpdateError::Cancelled);
        }
        let resp = match get {
            Ok(r) => r,
            Err(_) if exceeded => {
                return Err(if asset.size > 0 {
                    UpdateError::SizeMismatch { expected: asset.size, got: written + 1 }
                } else {
                    UpdateError::TooLarge { size: written, limit: self.config.max_download_bytes }
                })
            }
            Err(e) => return Err(e),
        };
        if !(200..300).contains(&resp.status) {
            return Err(UpdateError::HttpStatus { status: resp.status, what: "downloading the update".into() });
        }
        if asset.size > 0 && written != asset.size {
            return Err(UpdateError::SizeMismatch { expected: asset.size, got: written });
        }
        if let Some(announced) = resp.content_length {
            if announced != written {
                return Err(UpdateError::SizeMismatch { expected: announced, got: written });
            }
        }
        rep.emit(Phase::Downloading, written, Some(written), true);

        // MZ header: a Windows program, not an error page that came with status 200
        let mut head = [0u8; 2];
        let is_exe = fs::File::open(part).and_then(|mut f| io::Read::read_exact(&mut f, &mut head)).is_ok() && &head == b"MZ";
        if !is_exe {
            return Err(UpdateError::NotAnExecutable);
        }

        rep.emit(Phase::Verifying, 0, Some(written), true);
        let (_, hash) = swap::hash_file(part, |done| rep.emit(Phase::Verifying, done, Some(written), false)).map_err(|e| UpdateError::io("checking the download", &e))?;
        rep.emit(Phase::Verifying, written, Some(written), true);
        let verification = match &asset.sha256 {
            Some(expected) => {
                if !expected.eq_ignore_ascii_case(&hash) {
                    return Err(UpdateError::HashMismatch { expected: expected.clone(), got: hash });
                }
                Verification::Sha256
            }
            None if asset.size > 0 => Verification::SizeOnly,
            None => Verification::HeaderOnly,
        };

        // last point where Cancel still works: the install step is started below and the app has to leave
        if self.cancel.load(Ordering::SeqCst) {
            return Err(UpdateError::Cancelled);
        }
        rep.emit(Phase::Installing, 0, None, true);
        fs::rename(part, staged).map_err(|e| UpdateError::io("putting the new version next to the app", &e))?;
        let plan = SwapPlan {
            target: self.config.install_exe.clone(),
            new_file: staged.to_path_buf(),
            old_file: self.config.side_file("old"),
            result_file: self.config.side_file("result"),
            old_pid: std::process::id(),
            expected_size: written,
            expected_sha256: asset.sha256.clone().or(Some(hash)),
            from_version: Version::parse(&self.config.current_version)?.to_string(),
            to_version: release.version.to_string(),
            event_name: new_event_name(),
            exit_wait_ms: self.config.exit_wait.as_millis() as u64,
            start_wait_ms: self.config.start_wait.as_millis() as u64,
            rename_retries: 25,
            rename_retry_ms: 200,
        };
        self.installer.start_swap(&self.config, &plan)?;
        rep.emit(Phase::Restarting, written, Some(written), true);
        Ok(UpdateReady { from: plan.from_version, to: plan.to_version, bytes: written, verification })
    }
}

fn new_event_name() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("Local\\BoylerUtilities.Update.{}.{:x}", std::process::id(), nanos)
}

/// `owner/name`: both parts letters, digits, `.`, `_`, `-` (what GitHub allows); returned trimmed.
pub fn validate_repo(repo: &str) -> Result<String> {
    let r = repo.trim();
    let ok_part = |p: &str| !p.is_empty() && p != "." && p != ".." && p.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    match r.split_once('/') {
        Some((o, n)) if ok_part(o) && ok_part(n) => Ok(r.to_string()),
        _ => Err(UpdateError::InvalidRepo(repo.to_string())),
    }
}

/// A file that counts what it was given and refuses more than `limit`.
struct CountingFile<'a> {
    file: fs::File,
    written: u64,
    limit: u64,
    exceeded: bool,
    cancel: &'a AtomicBool,
    cancelled: bool,
}

impl Write for CountingFile<'_> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::SeqCst) {
            self.cancelled = true;
            return Err(io::Error::other("cancelled"));
        }
        if self.written + data.len() as u64 > self.limit {
            self.exceeded = true;
            return Err(io::Error::other("more data than the release lists"));
        }
        self.file.write_all(data)?;
        self.written += data.len() as u64;
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Throttles progress events: on every percent change, at most one more per 100 ms otherwise, always at phase edges.
struct Reporter<'a> {
    out: &'a mut dyn FnMut(&Progress),
    last_percent: Option<u8>,
    last_phase: Option<Phase>,
    last_at: Instant,
}

impl<'a> Reporter<'a> {
    fn new(out: &'a mut dyn FnMut(&Progress)) -> Self {
        Reporter { out, last_percent: None, last_phase: None, last_at: Instant::now() }
    }

    fn emit(&mut self, phase: Phase, done: u64, total: Option<u64>, force: bool) {
        let percent = total.filter(|t| *t > 0).map(|t| ((done.min(t) * 100) / t) as u8);
        let changed = self.last_phase != Some(phase) || percent != self.last_percent;
        if force || (changed && percent.is_some()) || self.last_at.elapsed() >= Duration::from_millis(100) {
            self.last_phase = Some(phase);
            self.last_percent = percent;
            self.last_at = Instant::now();
            (self.out)(&Progress { phase, bytes_done: done, bytes_total: total, percent });
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// the app's `main` hooks

/// FIRST line of the app's `main`: when the process was started as the install step it does the swap and returns
/// `Some(exit code)` - the app must then return at once without opening anything. Otherwise `None`.
/// `args` = `std::env::args()` collected.
#[cfg(windows)]
pub fn run_helper_if_requested(args: &[String]) -> Option<i32> {
    if args.get(1).map(String::as_str) != Some(HELPER_ARG) {
        return None;
    }
    let Some(plan_path) = args.get(2) else { return Some(2) };
    let plan: SwapPlan = match fs::read_to_string(plan_path).ok().and_then(|t| serde_json::from_str(&t).ok()) {
        Some(p) => p,
        None => {
            // the plan is unreadable: nothing is known about the files, so nothing is changed - but the app already exited, so
            // start it again (once the old process is really gone) and leave a note for its first look
            if let Some(fb) = swap::Fallback::from_args(args.get(3..).unwrap_or(&[])) {
                let mut procs = os_win::RealProcs::new();
                swap::recover_without_plan(&fb, &mut procs);
            }
            return Some(2);
        }
    };
    let mut procs = os_win::RealProcs::new();
    let r = run_swap(&plan, &mut procs);
    Some(if matches!(r.status, SwapStatus::Updated) { 0 } else { 1 })
}

/// What the new app learns from being started by the install step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatedFrom {
    pub previous_version: String,
}

/// Call once the new app is really up (window/tray created). When it was started by the install step (`--bu-updated`) this
/// tells the install step so the old version can be deleted; without that call the install step rolls back after
/// `start_wait`. Returns `None` for a normal start (nothing happens).
#[cfg(windows)]
pub fn confirm_started(args: &[String]) -> Option<UpdatedFrom> {
    let i = args.iter().position(|a| a == UPDATED_ARG)?;
    let event = args.get(i + 1)?;
    let previous = args.get(i + 2).cloned().unwrap_or_default();
    os_win::signal_started(event);
    Some(UpdatedFrom { previous_version: previous })
}

/// An update that did NOT go through (the install step wrote why), read once and deleted. Call at start; show it as a note.
pub fn take_last_result(config: &UpdaterConfig) -> Option<SwapResult> {
    let p = config.side_file("result");
    let text = fs::read_to_string(&p).ok()?;
    let _ = fs::remove_file(&p);
    serde_json::from_str(&text).ok()
}

/// Removes what an interrupted update could leave: half downloads, an unused staged file, an old copy kept for rollback, and the
/// install step's temp folders. Does nothing when this start is the one the install step just launched (`--bu-updated`):
/// the install step is still using those files then.
pub fn cleanup_leftovers(config: &UpdaterConfig, args: &[String]) {
    if args.iter().any(|a| a == UPDATED_ARG) {
        return;
    }
    let _ = fs::remove_file(config.side_file("part"));
    let _ = fs::remove_file(config.side_file("new"));
    if config.install_exe.is_file() {
        let _ = fs::remove_file(config.side_file("old"));
    }
    if let Ok(rd) = fs::read_dir(&config.temp_dir) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with("BoylerUtilities-update-") {
                let _ = fs::remove_dir_all(e.path());
            }
        }
    }
}

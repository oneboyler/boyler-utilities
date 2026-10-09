//! Settings › About › Check for updates: the page's side of `bu-updater` (Order 017). `check()` and `update()` block (network,
//! disk), so each runs on its own short-lived thread started by the user's click (TEMP until the job runner of Order 014 item 2
//! is merged: `cx.start_job`); what it finds comes back through a channel the page reads in its `tick` - each message wakes
//! the menu (Order 047: no frames while it only waits).
//!
//! Real: the app's own repo on GitHub (`REPO`, Order 044); with an empty repo `check()` answers `NotSetUp` WITHOUT touching the network.
//! Test copies: a FAKE release server in memory (`FakeHttp`: a v0.2.0 release of 31.8 MB, paced like a real download) and a
//! FAKE install step (`FakeInstaller`: records the plan, starts nothing); the files it writes go to ONE scratch folder under
//! %TEMP% per test copy, removed as soon as the driver and its last worker thread are gone. Never the internet.
//!
//! ONE driver for the whole app run (the Settings page keeps it across close): one `Updater`, so a second `update()` can never
//! run beside the first (017 remarks). Once `update()` has returned Ok the driver is SPENT - the install step waits for the app to
//! exit, so nothing is checked or updated again in this run (the worker thread sets the latch itself, even with the tab closed).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use bu_updater::{CheckResult, FakeHttp, FakeInstaller, Http, Progress, ReleaseInfo, Request, Response, UpdateError, UpdateReady, Updater, UpdaterConfig};

/// The GitHub repo the app updates from (`owner/name`, Order 044: the public repo). Empty = "updates not set up yet".
pub const REPO: &str = "oneboyler/boyler-utilities";

/// The pages Settings › About links to: "What's new" = the releases, "GitHub" = the repo.
pub fn page_url(releases: bool) -> String {
    if releases {
        format!("https://github.com/{REPO}/releases")
    } else {
        format!("https://github.com/{REPO}")
    }
}

/// The fake server's repo and release (the drawing's sample: v0.2.0, 31.8 MB).
pub const FAKE_REPO: &str = "boyler/utilities";
pub const FAKE_NEW: &str = "0.2.0";
pub const FAKE_SIZE: u64 = 31_800_000;

/// What a worker thread found.
#[derive(Debug)]
pub enum Msg {
    Checked(Result<CheckResult, UpdateError>),
    Progress(Progress),
    Done(Result<UpdateReady, UpdateError>),
}

pub struct Driver {
    up: Arc<Updater>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    /// fake only: the server (to add the download when an update starts) and the scratch folder (removed with its last owner)
    fake: Option<(Arc<FakeHttp>, Arc<ScratchDir>)>,
    /// fake only: the plans the fake install step was given (tests)
    pub installer: Option<Arc<FakeInstaller>>,
    /// a worker thread is running
    pub busy: bool,
    /// `update()` returned Ok: the install step is staged and waits for the app to exit (set by the worker thread)
    spent: Arc<AtomicBool>,
}

/// The fake's scratch folder: removed when the driver AND every worker thread holding it are gone (a test copy that leaves
/// the tab or exits mid-update leaves nothing behind once the thread ends).
pub struct ScratchDir(pub PathBuf);
impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The fake server, paced: every 256 KB chunk of the download waits `delay` (31.8 MB ≈ 3.1 s, the drawing's 3.2 s).
struct ArcHttp(Arc<FakeHttp>, Duration);
impl Http for ArcHttp {
    fn get(&self, req: &Request, sink: &mut dyn std::io::Write, progress: &mut dyn FnMut(u64, Option<u64>)) -> bu_updater::Result<Response> {
        let d = self.1;
        self.0.get(req, sink, &mut |done, total| {
            progress(done, total);
            if total.is_some_and(|t| t > 1_000_000) && !d.is_zero() {
                std::thread::sleep(d);
            }
        })
    }
}

impl Driver {
    /// The real updater (WinHTTP + the real install step) for the running exe.
    pub fn real(version: &str) -> Driver {
        Self::real_for(REPO, version)
    }

    /// The real updater for another repo (tests: an empty repo answers before any request).
    pub fn real_for(repo: &str, version: &str) -> Driver {
        let exe = std::env::current_exe().unwrap_or_default();
        let up = Updater::real(UpdaterConfig::new(repo, version, exe));
        let (tx, rx) = channel();
        Driver { up: Arc::new(up), tx, rx, fake: None, installer: None, busy: false, spent: Arc::new(AtomicBool::new(false)) }
    }

    /// The fake release server: `version` = the version the app says it has; `delay` = the pause per 256 KB chunk.
    pub fn fake(version: &str, delay: Duration) -> Driver {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("BoylerUtilities-test-updater-{}-{n}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        // the "running app" the fake update replaces: a stand-in file (the real exe is never touched)
        let _ = std::fs::write(dir.join("BoylerUtilities.exe"), b"MZ stand-in");
        let mut cfg = UpdaterConfig::new(FAKE_REPO, version, dir.join("BoylerUtilities.exe"));
        cfg.temp_dir = dir.clone();
        let http = Arc::new(FakeHttp::new().with_chunk(256 * 1024));
        let tag = format!("v{FAKE_NEW}");
        let json = format!(
            r#"{{"tag_name":"{tag}","name":"Boyler Utilities {FAKE_NEW}","body":"","html_url":"https://github.com/{FAKE_REPO}/releases/tag/{tag}","draft":false,"prerelease":false,"assets":[{{"name":"BoylerUtilities.exe","size":{FAKE_SIZE},"browser_download_url":"https://github.com/{FAKE_REPO}/releases/download/{tag}/BoylerUtilities.exe"}}]}}"#
        );
        http.ok(&format!("https://api.github.com/repos/{FAKE_REPO}/releases/latest"), json);
        let inst = Arc::new(FakeInstaller::default());
        let up = Updater::new(cfg, Box::new(ArcHttp(http.clone(), delay)), Box::new(inst.clone()));
        let (tx, rx) = channel();
        let fake = Some((http, Arc::new(ScratchDir(dir))));
        Driver { up: Arc::new(up), tx, rx, fake, installer: Some(inst), busy: false, spent: Arc::new(AtomicBool::new(false)) }
    }

    /// The button: ask for the newest release (the ONLY network use; real with no repo = no network at all). Returns whether
    /// it started: not while a worker runs (e.g. a cancelled update still ending - up to ~30 s on a silent connection) and
    /// never once the driver is spent.
    pub fn check(&mut self) -> bool {
        if self.busy || self.is_spent() {
            return false;
        }
        self.busy = true;
        let (up, tx) = (self.up.clone(), self.tx.clone());
        let hold = self.fake.as_ref().map(|f| f.1.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Checked(up.check()));
            // Order 047: the answer wakes the menu (the page no longer asks every frame while it waits)
            crate::services::Waker.wake();
            drop(hold);
        });
        true
    }

    /// Download, verify, stage, start the install step. Returns whether it started (same rules as `check`).
    pub fn update(&mut self, rel: ReleaseInfo) -> bool {
        if self.busy || self.is_spent() {
            return false;
        }
        if let Some((http, _)) = &self.fake {
            // the download exists only while a fake update runs (31.8 MB in memory)
            let mut body = vec![0u8; FAKE_SIZE as usize];
            body[0] = b'M';
            body[1] = b'Z';
            http.ok(&rel.asset.url, body);
        }
        self.busy = true;
        let (up, tx, spent) = (self.up.clone(), self.tx.clone(), self.spent.clone());
        let hold = self.fake.as_ref().map(|f| f.1.clone());
        let is_fake = self.fake.is_some();
        std::thread::spawn(move || {
            let tx2 = tx.clone();
            // Order 055: every sample goes into the channel (the page keeps the latest), but the menu is woken at most every
            // 33 ms - the download reports once per chunk (hundreds a second on a fast line) and each wake is a frame; a
            // new phase and the last percent wake at once, the end (`Done`) always does
            let mut last_wake: Option<std::time::Instant> = None;
            let mut last_phase = None;
            let r = up.update(&rel, &mut |p: &Progress| {
                let _ = tx2.send(Msg::Progress(p.clone()));
                let phase = std::mem::discriminant(&p.phase);
                if wake_due(last_wake.map(|t| t.elapsed()), last_phase != Some(phase) || p.percent == Some(100)) {
                    last_wake = Some(std::time::Instant::now());
                    last_phase = Some(phase);
                    crate::services::Waker.wake();
                }
            });
            let ok = r.is_ok();
            if ok {
                // latched here, not in the page: it holds even while the tab is closed and nobody polls
                spent.store(true, Ordering::SeqCst);
            }
            let _ = tx.send(Msg::Done(r));
            crate::services::Waker.wake();
            drop(hold);
            // the install step waits for this app to end: it ends even if the menu (or the Settings tab) was closed
            // meanwhile - after the page's own "Installing… / Restarting…" (1.6 + 1.3 s) when it is shown. A fake never
            // ends the app.
            if ok && !is_fake {
                std::thread::sleep(std::time::Duration::from_millis(3200));
                crate::services::request_exit();
            }
        });
        true
    }

    /// `update()` returned Ok: the install step waits for this app to exit; nothing more may run in this app run.
    pub fn is_spent(&self) -> bool {
        self.spent.load(Ordering::SeqCst)
    }

    /// The Updating window's Cancel: asks the running update to stop. It gives no answer - `update()`'s result says what
    /// happened (Cancelled, or Ok when the cancel came after the point of no return).
    pub fn cancel(&self) {
        self.up.cancel();
    }

    /// What the threads found since the last call.
    pub fn poll(&mut self) -> Vec<Msg> {
        let mut out = Vec::new();
        while let Ok(m) = self.rx.try_recv() {
            if matches!(m, Msg::Checked(_) | Msg::Done(_)) {
                self.busy = false;
            }
            if let (Msg::Done(_), Some((http, _))) = (&m, &self.fake) {
                // fake: the 31.8 MB download leaves memory once the update has ended
                http.ok(&format!("https://github.com/{FAKE_REPO}/releases/download/v{FAKE_NEW}/BoylerUtilities.exe"), Vec::<u8>::new());
            }
            out.push(m);
        }
        out
    }
}

/// Order 055: does a progress sample wake the menu now? `since` = the time since the last wake (None = never), `urgent` = a
/// new phase / the last percent. At most one wake per 33 ms otherwise.
pub(super) fn wake_due(since: Option<Duration>, urgent: bool) -> bool {
    urgent || since.is_none_or(|s| s >= Duration::from_millis(33))
}

/// A short reason for the About line ("Could not check · …").
pub fn reason(e: &UpdateError) -> String {
    match e {
        UpdateError::Network { .. } => "no connection to GitHub".into(),
        UpdateError::RateLimited => "GitHub is busy, try again in a while".into(),
        UpdateError::HttpStatus { status, .. } => format!("GitHub answered {status}"),
        UpdateError::NoWindowsAsset { .. } => "the release has no Windows file".into(),
        UpdateError::SizeMismatch { .. } | UpdateError::HashMismatch { .. } | UpdateError::NotAnExecutable => "the download was damaged".into(),
        UpdateError::InstallDirNotWritable(_) => "the app's folder can't be written".into(),
        UpdateError::Cancelled => "cancelled".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Order 055: progress wakes the menu at most every 33 ms, but a new phase / the last percent at once.
    #[test]
    fn progress_wakes_are_throttled_to_33_ms() {
        assert!(wake_due(None, false), "the first sample wakes");
        assert!(!wake_due(Some(Duration::from_millis(5)), false));
        assert!(wake_due(Some(Duration::from_millis(33)), false));
        assert!(wake_due(Some(Duration::from_millis(1)), true), "a new phase does not wait");
    }
}

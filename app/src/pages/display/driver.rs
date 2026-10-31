//! The Display tab's "Graphics driver" card (Order 125; the owner Oct 30: "only for users it detects have nvidia gpu. and it
//! tells you your version, newest version, and you get to either update or pick a version you want"). Shown ONLY when
//! Windows lists an NVIDIA card. Order 129 (the owner Oct 31: "it can be more compact for sure ... one row for current
//! version. then one that is for select the version you wanna download ... it opens up a new window where you select the
//! version, and maybe it has release date next to each too? and then you click the download button next to the one you
//! want"): two rows - Your version, and Driver versions ("New driver available - 621.01" / "You have the newest", and
//! Choose). Choose opens the small popup window (the app's `dialog`, like Audio's Mute settings): NVIDIA's Game Ready
//! drivers for this card, newest first, each with its date and a Download button (the installed one marked Installed,
//! the newest marked Newest); the running download / install and its result show on its line (and on the card's line).
//!
//! Everything is `bu-gpudriver`: the card reads the card + driver and NVIDIA's list once when the tab opens (on an
//! off-UI thread; nothing in the background). A Download downloads the package into the user's temp folder
//! (`BoylerUtilities-gpudriver`, progress on its line), checks it is NVIDIA's, and hands it to the app's elevated copy (one
//! admin prompt) which installs the DRIVER ONLY. The download is deleted when the install ends, whatever the end (a "No"
//! at the admin prompt too). The job outlives the tab and the menu (its state is the app's, not the page's); one job at a
//! time - the other Download buttons are greyed while it runs. Test copies: a fake card, NVIDIA's recorded answers in
//! memory, a fake install - never the internet, never an installer.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use taffy::style::AlignItems;

use bu_gpudriver::{DriverVersion, GpuOs, Installed, NvidiaGpu, Release, Signatures};
use bu_updater::Http;

use crate::gfx::{Align, Font};
use crate::pages::Env;
use crate::ui::cx::Cx;
use crate::ui::el::{idx, key, lh, sub, El, Key};
use crate::ui::pieces::badge::{self, Tone};
use crate::ui::pieces::button::{self, Kind};
use crate::ui::pieces::{dialog, group, progress};
use crate::ui::{ACC, FG, FG2};

pub(super) const K_CHOOSE: Key = key("dsp.drv.choose");
/// the version window (`dialog`: its × = `sub(K_WIN, "x")`, beside it = `sub(K_WIN, "out")`)
pub(super) const K_WIN: Key = key("dsp.drv.win");
/// a line's Download button: `idx(K_DL, i)` (i = the line, newest first)
pub(super) const K_DL: Key = key("dsp.drv.dl");
const K_BAR: Key = key("dsp.drv.bar");

/// The window's width (`.dlg` 388) and its list's height before it scrolls (the 468 px window less its title and padding).
const WIN_W: f32 = 388.0;
const LIST_H: f32 = 401.0;

/// The install step: (version, downloaded package) -> how it ended, or the line the card shows.
type InstallFn = dyn Fn(DriverVersion, &Path) -> Result<Installed, String> + Send + Sync;

/// What the card works through (real: Windows, NVIDIA's site, the elevated copy; tests: fakes).
pub struct Backend {
    pub gpu: Arc<dyn GpuOs>,
    pub http: Arc<dyn Http>,
    pub sig: Arc<dyn Signatures>,
    /// where packages are downloaded (removed when a job ends)
    pub dir: PathBuf,
    pub install: Box<InstallFn>,
}

impl Backend {
    /// The real one. `read_only` (a measuring copy): reads only, every install refused.
    #[cfg(windows)]
    pub fn real(read_only: bool) -> Backend {
        use bu_gpudriver::real::{RealGpuOs, RealSignatures};
        let install: Box<InstallFn> = if read_only {
            Box::new(|_, _| Err("A test copy installs nothing".into()))
        } else {
            Box::new(|v, file| {
                use crate::admin::{client::admin, Op, Purpose};
                match admin().call(Purpose::GpuDriver, Op::GpuDriver { version: v, file: file.to_string_lossy().into_owned() }) {
                    Ok(f) if f.first().map(String::as_str) == Some("restart") => Ok(Installed::NeedsRestart),
                    Ok(_) => Ok(Installed::Done),
                    Err(e) => Err(e.to_string()),
                }
            })
        };
        Backend {
            gpu: Arc::new(RealGpuOs),
            http: Arc::new(bu_updater::WinHttp::new(&format!("BoylerUtilities/{}", env!("CARGO_PKG_VERSION")))),
            sig: Arc::new(RealSignatures),
            dir: std::env::temp_dir().join("BoylerUtilities-gpudriver"),
            install,
        }
    }
}

/// A test card: Windows' driver version string, changed by a fake install to the installed version.
pub struct FakeGpu {
    pub name: String,
    pub windows_version: Mutex<String>,
    pub vendor: &'static str,
}

impl GpuOs for FakeGpu {
    fn adapters(&self) -> bu_gpudriver::Result<Vec<bu_gpudriver::Adapter>> {
        Ok(vec![bu_gpudriver::Adapter {
            name: self.name.clone(),
            hardware_id: format!(r"PCI\VEN_{}&DEV_2684", self.vendor),
            driver_version: self.windows_version.lock().unwrap().clone(),
        }])
    }
    fn os_build(&self) -> u32 {
        22631
    }
    fn is_laptop(&self) -> bool {
        false
    }
}

/// Windows' version string of an NVIDIA version ("621.01" -> "32.0.16.2101").
pub fn windows_version(v: DriverVersion) -> String {
    let n = v.major * 100 + v.minor;
    format!("32.0.1{}.{:04}", n / 10000, n % 10000)
}

/// What the test copies' fake install did.
#[derive(Default)]
pub struct FakeInstalls {
    pub calls: Mutex<Vec<(String, PathBuf, bool)>>,
    /// the next install's answer: None = installs it; Some(line) = fails with that line
    pub fail: Mutex<Option<String>>,
    /// held while locked: the install waits (a test can look at the card mid-install)
    pub gate: Mutex<()>,
    /// the next install panics (a job thread that dies)
    pub panic: std::sync::atomic::AtomicBool,
    /// the next install asks for a restart (Windows still reports the old driver until then)
    pub restart: std::sync::atomic::AtomicBool,
}

impl Backend {
    /// A test copy: an RTX 4090 with `installed` ("610.88"), NVIDIA's recorded answers, packages of `package_len` bytes,
    /// a fake install that sets the card's version (and records each call, with whether the package was there).
    pub fn fake(installed: &str, nvidia: bool, package_len: usize, dir: PathBuf, log: Arc<FakeInstalls>) -> Backend {
        let v = DriverVersion::parse(installed).map(windows_version).unwrap_or_default();
        let gpu = Arc::new(FakeGpu {
            name: if nvidia { "NVIDIA GeForce RTX 4090".into() } else { "AMD Radeon RX 7900 XTX".into() },
            windows_version: Mutex::new(v),
            vendor: if nvidia { "10DE" } else { "1002" },
        });
        let g = gpu.clone();
        Backend {
            gpu,
            http: Arc::new(bu_gpudriver::fake::sample_site(package_len)),
            sig: Arc::new(bu_gpudriver::fake::FakeSignatures::nvidia()),
            dir,
            install: Box::new(move |v, file| {
                log.calls.lock().unwrap().push((v.to_string(), file.to_path_buf(), file.is_file()));
                if log.panic.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    panic!("test: the install thread dies");
                }
                let _held = log.gate.lock().unwrap();
                if let Some(e) = log.fail.lock().unwrap().take() {
                    return Err(e);
                }
                if log.restart.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    return Ok(Installed::NeedsRestart);
                }
                *g.windows_version.lock().unwrap() = windows_version(v);
                Ok(Installed::Done)
            }),
        }
    }
}

/// What the card read when the tab opened.
#[derive(Clone, Debug, PartialEq)]
pub enum Look {
    /// no NVIDIA card (or Windows' list unreadable): no card on the tab
    NoNvidia,
    /// the card, and NVIDIA's list (None = still asking; Err = the line why it failed)
    Gpu { gpu: NvidiaGpu, list: Option<Result<Vec<Release>, String>> },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Downloading { done: u64, total: Option<u64> },
    Installing,
    Done { result: Result<Installed, String>, after: Option<DriverVersion> },
}

/// One download + install (the app's, not the page's: it goes on with the menu closed).
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub release: Release,
    pub phase: Phase,
    /// bumped on every change (the page rebuilds when it moves)
    pub seq: u64,
}

impl Job {
    pub fn busy(&self) -> bool {
        !matches!(self.phase, Phase::Done { .. })
    }
}

pub type Shared = Arc<Mutex<Option<Job>>>;

/// The job's lock (end review: a thread that died holding it doesn't take the menu with it).
fn lock(j: &Shared) -> std::sync::MutexGuard<'_, Option<Job>> {
    j.lock().unwrap_or_else(|e| e.into_inner())
}

/// The real app's one job.
fn app_job() -> Shared {
    static J: OnceLock<Shared> = OnceLock::new();
    J.get_or_init(Shared::default).clone()
}

#[derive(Default)]
pub struct Card {
    be: Option<Arc<Backend>>,
    look: Option<Look>,
    rx: Option<std::sync::mpsc::Receiver<Look>>,
    job: Shared,
    seen: u64,
    /// the driver Windows reported after a job that ended while this tab was open (a read that started before the end
    /// would put the old one back)
    after: Option<DriverVersion>,
    /// the version window is open (since then, ms: its open motion)
    win: Option<f64>,
}

impl Card {
    /// The tab opened: read the card + driver and NVIDIA's list off the menu's thread.
    pub fn open(&mut self, env: &Env) {
        #[cfg(windows)]
        if !env.fake() {
            self.open_with(Arc::new(Backend::real(env.real_read)), app_job());
            return;
        }
        let dir = std::env::temp_dir().join(format!("bu-gpudriver-card-{}-{:?}", std::process::id(), std::thread::current().id()));
        self.open_with(Arc::new(Backend::fake("610.88", true, 64 * 1024, dir, Arc::default())), Shared::default());
    }

    pub fn open_with(&mut self, be: Arc<Backend>, job: Shared) {
        let (tx, rx) = std::sync::mpsc::channel();
        let b = be.clone();
        let j = job.clone();
        crate::offui::spawn("gpudriver-read", move || {
            // a download left by an app that quit mid-job (~1 GB) goes now, unless a job is running
            if !j.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|j| j.busy()) {
                let _ = std::fs::remove_dir_all(&b.dir);
            }
            let look = match bu_gpudriver::detect(&*b.gpu) {
                Ok(Some(gpu)) => {
                    // the card at once, NVIDIA's list when it is in
                    let _ = tx.send(Look::Gpu { gpu: gpu.clone(), list: None });
                    crate::services::Waker.wake();
                    let list = bu_gpudriver::lookup(&*b.http, &*b.gpu, &gpu.name).map_err(|e| e.to_string());
                    Look::Gpu { gpu, list: Some(list) }
                }
                _ => Look::NoNvidia,
            };
            let _ = tx.send(look);
        });
        // a job that already ended: this read comes after it (and is newer)
        self.seen = lock(&job).as_ref().map_or(0, |j| j.seq);
        self.after = None;
        self.be = Some(be);
        self.rx = Some(rx);
        self.job = job;
        self.win = None;
    }

    /// New reads / job steps came in (the page builds again).
    pub fn tick(&mut self) -> bool {
        let mut moved = false;
        while let Some(l) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.look = Some(l);
            moved = true;
        }
        let job = lock(&self.job).clone();
        if let Some(j) = job {
            if j.seq != self.seen {
                self.seen = j.seq;
                moved = true;
                // the driver Windows reports after the install
                if let Phase::Done { after: Some(v), .. } = j.phase {
                    self.after = Some(v);
                }
            }
        }
        if let (Some(v), Some(Look::Gpu { gpu, .. })) = (self.after, self.look.as_mut()) {
            gpu.installed = Some(v);
        }
        moved
    }

    /// Tests: wait for the open read (and a running job) to finish (at most 10 s).
    #[cfg(test)]
    pub fn settle(&mut self) {
        let t0 = std::time::Instant::now();
        while self.rx.is_some() && t0.elapsed().as_secs() < 10 {
            self.tick();
            let reading = !matches!(&self.look, Some(Look::NoNvidia) | Some(Look::Gpu { list: Some(_), .. }));
            let busy = lock(&self.job).as_ref().is_some_and(|j| j.busy());
            if !reading && !busy {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn gpu(&self) -> Option<&NvidiaGpu> {
        match &self.look {
            Some(Look::Gpu { gpu, .. }) => Some(gpu),
            _ => None,
        }
    }

    fn releases(&self) -> &[Release] {
        match &self.look {
            Some(Look::Gpu { list: Some(Ok(l)), .. }) => l,
            _ => &[],
        }
    }

    fn busy(&self) -> bool {
        lock(&self.job).as_ref().is_some_and(|j| j.busy())
    }

    /// NVIDIA's newest, when it is newer than the installed one (or the installed one is unknown).
    fn newer(&self) -> Option<&Release> {
        match (self.releases().first(), self.gpu()) {
            (Some(n), Some(g)) if g.installed.is_none_or(|i| n.version > i) => Some(n),
            _ => None,
        }
    }

    /// The version installed but waiting for a restart (Windows reports the old one until then): no Download for it.
    fn pending_restart(&self) -> Option<DriverVersion> {
        lock(&self.job).as_ref().filter(|j| matches!(j.phase, Phase::Done { result: Ok(Installed::NeedsRestart), .. })).map(|j| j.release.version)
    }

    /// Download + install `rel` on its own thread. Nothing when a job is running.
    fn start(&mut self, rel: Release) {
        let Some(be) = self.be.clone() else { return };
        {
            let mut j = lock(&self.job);
            if j.as_ref().is_some_and(|j| j.busy()) {
                return;
            }
            let seq = j.as_ref().map(|j| j.seq + 1).unwrap_or(1);
            *j = Some(Job { release: rel.clone(), phase: Phase::Downloading { done: 0, total: None }, seq });
        }
        let job = self.job.clone();
        let started = std::thread::Builder::new().name("bu-gpudriver-job".into()).spawn(move || {
            // however the thread ends (a panic too), the job ends: the card never stays busy
            let guard = EndGuard(job.clone());
            run_job(&be, &job, &rel);
            std::mem::forget(guard);
        });
        if started.is_err() {
            finish(&self.job, Err("Couldn\u{2019}t start".into()), None);
        }
    }

    /// The frame closed the popup (Esc): the version window closes.
    pub fn dismiss(&mut self) {
        self.win = None;
    }

    /// A click on one of the card's parts or its window's (true = it was the card's). `now` = when (the window's open
    /// motion starts then).
    pub fn click(&mut self, k: Key, now: f64) -> bool {
        if k == K_CHOOSE {
            if !self.releases().is_empty() {
                self.win = Some(now);
            }
            return true;
        }
        if k == sub(K_WIN, "x") || k == sub(K_WIN, "out") {
            self.win = None;
            return true;
        }
        if k == sub(K_WIN, "win") {
            return true;
        }
        if let Some(i) = (0..self.releases().len()).find(|&i| k == idx(K_DL, i)) {
            let r = self.releases()[i].clone();
            // the installed one (and one waiting for a restart) has no Download; one job at a time (`start` refuses a second)
            if self.win.is_some() && self.gpu().and_then(|g| g.installed) != Some(r.version) && self.pending_restart() != Some(r.version) {
                self.start(r);
            }
            return true;
        }
        false
    }

    /// The card (None = no NVIDIA card: nothing on the tab).
    pub fn build(&mut self, cx: &mut Cx) -> Option<El> {
        let gpu = self.gpu()?;
        let job = lock(&self.job).clone();
        let value = |s: &str| El::text(s, Font::new(13.0, 400), FG2(), lh(13.0, 1.35)).align(Align::Right);
        let mine = gpu.installed.map(|v| v.to_string()).unwrap_or_else(|| "Unknown".into());
        let row = |first: bool, kids: Vec<El>| group::row_ex(first, 14.0, kids).pad(7.0, 14.0, 7.0, 14.0).min_h(46.0);
        let mut rows = vec![row(true, vec![group::lbl("Your version", Some(&gpu.name)), group::ctl(vec![value(&mine)])])];
        // the Driver versions line: a running job (or one waiting for a restart), else whether a newer one is out (how a
        // job failed stays on its line in the window - end review: not on the card for the rest of the session)
        let (line, accent) = match (&job, &self.look) {
            (Some(j), _) if j.busy() || matches!(j.phase, Phase::Done { result: Ok(Installed::NeedsRestart), .. }) => (job_line(j, true), false),
            (_, Some(Look::Gpu { list: Some(Err(e)), .. })) => (format!("Couldn\u{2019}t check: {e}"), false),
            (_, Some(Look::Gpu { list: Some(Ok(_)), .. })) => match self.newer() {
                Some(n) => (format!("New driver available - {}", n.version), true),
                None => ("You have the newest".to_string(), false),
            },
            _ => ("Checking\u{2026}".to_string(), false),
        };
        let lbl = El::col()
            .flex1()
            .child(El::text("Driver versions", Font::new(13.0, 400), FG(), lh(13.0, 1.35)).ellipsis())
            .child(El::text(&line, Font::new(11.0, 400), if accent { ACC() } else { FG2() }, lh(11.0, 1.35)).ellipsis().margin(1.0, 0.0, 0.0, 0.0));
        let choose = button::cbtn(cx, K_CHOOSE, "Choose", Kind::Ghost, true, self.releases().is_empty(), 0.0);
        rows.push(row(false, vec![lbl, group::ctl(vec![choose])]));
        Some(El::block().child(group::gh("Graphics driver")).child(group::grp(rows)))
    }

    /// The version window (Choose opened it): NVIDIA's list for this card, newest first, a Download on each line.
    pub fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        let at = self.win?;
        let mine = self.gpu()?.installed;
        let job = lock(&self.job).clone();
        let busy = job.as_ref().is_some_and(|j| j.busy());
        let mut rows = Vec::new();
        for (i, rel) in self.releases().iter().enumerate() {
            let mut name = El::row().center().gap(6.0).child(El::text(&rel.version.to_string(), Font::new(13.0, 400), FG(), lh(13.0, 1.35)));
            if i == 0 {
                name = name.child(badge::tag("Newest", Tone::Teal));
            }
            // (a finished install: the line is as before, its Installed tag says it)
            let this_job = job.as_ref().filter(|j| j.release.version == rel.version && !matches!(j.phase, Phase::Done { result: Ok(Installed::Done), .. }));
            let small = match this_job {
                Some(j) => job_line(j, false),
                None if rel.date.is_empty() => "Game Ready".to_string(),
                None => format!("{} \u{b7} Game Ready", rel.date),
            };
            let lbl = El::col().flex1().child(name).child(El::text(&small, Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).ellipsis().margin(1.0, 0.0, 0.0, 0.0));
            let right = match this_job {
                Some(j) if j.busy() => {
                    let share = match j.phase {
                        Phase::Downloading { done, total } => total.filter(|&t| t > 0).map(|t| done as f32 / t as f32),
                        _ => None,
                    };
                    progress::bar(cx, K_BAR, share).key(K_BAR).w(96.0).none()
                }
                Some(Job { phase: Phase::Done { result: Ok(Installed::NeedsRestart), .. }, .. }) => badge::tag("Restart", Tone::Plain),
                _ if mine == Some(rel.version) => badge::tag("Installed", Tone::Green),
                _ => button::cbtn_sized(cx, idx(K_DL, i), "Download", Kind::Ghost, button::MCFB, busy, 0.0),
            };
            rows.push(group::row_ex(i == 0, 14.0, vec![lbl, group::ctl(vec![right])]).pad(7.0, 14.0, 7.0, 14.0).min_h(46.0));
        }
        let list = group::grp(rows);
        // the Mute settings window's list (`.mdb`): it scrolls when taller than the window, with the slim thumb in its own
        // 10 px lane (end review: its height from the 46 px lines, not a second layout of up to 50 lines every frame)
        let full = self.releases().len() as f32 * 47.0 + 2.0;
        let lane = if full > LIST_H { 10.0 } else { 0.0 };
        let body = cx
            .scroll_box(sub(K_WIN, "list"), vec![list])
            .items(AlignItems::STRETCH)
            .pad(0.0, 18.0 + lane, 2.0, 18.0)
            .margin(0.0, -18.0, 0.0, -18.0)
            .slim_thumb(crate::ui::el::SlimThumb::GLASS)
            .style(|st| {
                st.flex_shrink = 1.0;
                st.min_size.height = taffy::style::LengthPercentageAuto::length(0.0);
            });
        Some(dialog::dialog(cx, K_WIN, WIN_W, "Driver versions", vec![body], vec![], true, at))
    }

    #[cfg(test)]
    pub fn job(&self) -> Option<Job> {
        lock(&self.job).clone()
    }

    /// Pictures: NVIDIA's list made longer with older made-up versions (the real one has 50: the window scrolls).
    #[cfg(test)]
    pub fn add_old_releases(&mut self, n: usize) {
        if let Some(Look::Gpu { list: Some(Ok(l)), .. }) = &mut self.look {
            let last = l.last().cloned().expect("a list");
            for k in 1..=n {
                let v = DriverVersion { major: last.version.major - (k as u32 * 3) / 2, minor: (k as u32 * 37) % 90 + 5 };
                let date = format!("{} {} 2026", 28 - (k * 5) % 27, ["Jul", "Jun", "May", "Apr", "Mar", "Feb", "Jan"][(k / 2).min(6)]);
                l.push(Release { version: v, date, url: last.url.clone(), size: last.size.clone() });
            }
        }
    }
}

/// A job's line: running ("Downloading · 42 % of 1.04 GB") or how it ended. `card` = the card's line (names the version;
/// the window's line is under it already).
fn job_line(j: &Job, card: bool) -> String {
    let v = if card { format!(" {}", j.release.version) } else { String::new() };
    match &j.phase {
        Phase::Downloading { done, total } => {
            let pct = total.filter(|&t| t > 0).map(|t| format!("{} % of {}", done * 100 / t, j.release.size)).unwrap_or_else(|| j.release.size.clone());
            format!("Downloading{v} \u{b7} {pct}")
        }
        Phase::Installing => format!("Installing{v} \u{b7} the screen may go black for a moment"),
        Phase::Done { result: Ok(Installed::Done), .. } => format!("Installed{v}"),
        Phase::Done { result: Ok(Installed::NeedsRestart), .. } => format!("Installed{v} \u{b7} restart the PC to finish"),
        Phase::Done { result: Err(e), .. } => format!("Not installed{v} \u{b7} {e}"),
    }
}

/// Ends a job (its result line), waking the menu.
fn finish(job: &Shared, result: Result<Installed, String>, after: Option<DriverVersion>) {
    {
        let mut j = lock(job);
        if let Some(j) = j.as_mut() {
            j.phase = Phase::Done { result, after };
            j.seq += 1;
        }
    }
    crate::services::Waker.wake();
}

/// Ends a job whose thread stopped early (a panic): "not installed".
struct EndGuard(Shared);

impl Drop for EndGuard {
    fn drop(&mut self) {
        finish(&self.0, Err("Stopped".into()), None);
    }
}

/// The job's thread: download (progress on its line), install through the backend, delete the download, read the
/// installed driver again.
fn run_job(be: &Backend, job: &Shared, rel: &Release) {
    let set = |p: Phase| {
        if let Some(j) = lock(job).as_mut() {
            j.phase = p;
            j.seq += 1;
        }
        crate::services::Waker.wake();
    };
    let mut last = u64::MAX;
    let got = bu_gpudriver::download(&*be.http, &*be.sig, rel, &be.dir, &mut |done, total| {
        // a step per whole percent (a 1 GB download: 100 wakes, not 60 000)
        let pct = total.filter(|&t| t > 0).map(|t| done * 100 / t).unwrap_or(done >> 24);
        if pct != last {
            last = pct;
            set(Phase::Downloading { done, total });
        }
    });
    let result = match got {
        Err(e) => Err(format!("Download failed: {e}")),
        Ok(path) => {
            set(Phase::Installing);
            (be.install)(rel.version, &path)
        }
    };
    let _ = std::fs::remove_dir_all(&be.dir);
    let after = bu_gpudriver::detect(&*be.gpu).ok().flatten().and_then(|g| g.installed);
    set(Phase::Done { result, after });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::Gfx;
    use crate::ui::cx::State;

    struct T {
        c: Card,
        g: Gfx,
        st: State,
        now: f64,
        log: Arc<FakeInstalls>,
        dir: PathBuf,
    }

    impl T {
        fn new(installed: &str, nvidia: bool) -> T {
            let dir = std::env::temp_dir().join(format!("bu-drvcard-{}-{installed}-{nvidia}-{:?}", std::process::id(), std::thread::current().id()));
            let log = Arc::new(FakeInstalls::default());
            let mut c = Card::default();
            c.open_with(Arc::new(Backend::fake(installed, nvidia, 300_000, dir.clone(), log.clone())), Shared::default());
            c.settle();
            T { c, g: Gfx::new(1.0), st: State::default(), now: 0.0, log, dir }
        }
        /// the card's texts, then the window's (when open)
        fn text(&mut self) -> String {
            self.now += 16.0;
            let mut cx = Cx::new(self.now, false, &self.g, &mut self.st);
            let mut v = Vec::new();
            if let Some(e) = self.c.build(&mut cx) {
                texts(&e, &mut v);
            }
            if let Some(p) = self.c.popup(&mut cx) {
                texts(&p, &mut v);
            }
            v.join("|")
        }
        fn window(&mut self) -> Option<crate::ui::lay::Laid> {
            self.now += 16.0;
            let mut cx = Cx::new(self.now, false, &self.g, &mut self.st);
            let p = self.c.popup(&mut cx)?;
            let root = El::block().w(crate::ui::WIN_W).h(crate::ui::WIN_H).child(p);
            Some(crate::ui::lay::Laid::new(&self.g, root, crate::ui::WIN_W, Some(crate::ui::WIN_H)))
        }
        fn choose(&mut self) {
            self.c.click(K_CHOOSE, self.now);
        }
    }

    fn texts(e: &El, out: &mut Vec<String>) {
        if let crate::ui::el::Content::Text(t) = &e.content {
            out.push(t.s.clone());
        }
        for c in &e.children {
            texts(c, out);
        }
    }

    #[test]
    fn no_nvidia_card_means_no_card() {
        let mut t = T::new("610.88", false);
        assert_eq!(t.text(), "");
        t.choose();
        assert!(t.c.win.is_none());
        assert!(t.c.job().is_none());
    }

    #[test]
    fn two_rows_your_version_and_a_newer_driver_with_choose() {
        let mut t = T::new("610.88", true);
        let s = t.text();
        assert_eq!(s, "Graphics driver|Your version|NVIDIA GeForce RTX 4090|610.88|Driver versions|New driver available - 621.01|Choose");
    }

    #[test]
    fn on_the_newest_it_says_so() {
        let mut t = T::new("621.01", true);
        assert_eq!(t.text(), "Graphics driver|Your version|NVIDIA GeForce RTX 4090|621.01|Driver versions|You have the newest|Choose");
    }

    #[test]
    fn choose_opens_the_window_newest_first_with_dates_and_downloads() {
        let mut t = T::new("617.42", true);
        t.choose();
        let s = t.text();
        let win = s.split("|Choose|").nth(1).expect("the window");
        assert_eq!(
            win,
            "Driver versions|621.01|Newest|21 Oct 2026 \u{b7} Game Ready|Download|617.42|6 Oct 2026 \u{b7} Game Ready|Installed|610.88|28 Jul 2026 \u{b7} Game Ready|Download"
        );
        // the × and a click beside it close it
        let l = t.window().unwrap();
        assert!(l.rect_of(sub(K_WIN, "x")).is_some());
        assert!(l.rect_of(idx(K_DL, 0)).is_some() && l.rect_of(idx(K_DL, 2)).is_some());
        assert!(l.rect_of(idx(K_DL, 1)).is_none(), "the installed one has no Download");
        t.c.click(sub(K_WIN, "x"), t.now);
        assert!(t.c.popup(&mut Cx::new(t.now, false, &t.g, &mut t.st)).is_none());
        t.choose();
        t.c.click(sub(K_WIN, "out"), t.now);
        assert!(t.c.win.is_none());
        t.choose();
        t.c.dismiss();
        assert!(t.c.win.is_none(), "Esc closes it");
    }

    #[test]
    fn a_download_installs_that_one_and_deletes_the_download() {
        let mut t = T::new("610.88", true);
        t.choose();
        assert!(t.c.click(idx(K_DL, 0), t.now));
        t.c.settle();
        let calls = t.log.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "621.01");
        assert_eq!(calls[0].1, t.dir.join("621.01.exe"));
        assert!(calls[0].2, "the package was there for the install");
        assert!(!t.dir.exists(), "the download is deleted after the install");
        let s = t.text();
        assert!(s.starts_with("Graphics driver|Your version|NVIDIA GeForce RTX 4090|621.01|Driver versions|You have the newest|Choose|"), "{s}");
        assert!(s.contains("|621.01|Newest|21 Oct 2026 \u{b7} Game Ready|Installed|617.42|"), "it is the installed one now: {s}");
        assert!(t.c.newer().is_none());
    }

    #[test]
    fn an_older_one_can_be_picked_and_then_the_newer_is_offered() {
        let mut t = T::new("621.01", true);
        t.choose();
        assert!(t.c.click(idx(K_DL, 2), t.now));
        t.c.settle();
        assert_eq!(t.log.calls.lock().unwrap()[0].0, "610.88");
        let s = t.text();
        assert!(s.contains("Your version|NVIDIA GeForce RTX 4090|610.88|Driver versions|New driver available - 621.01|"), "{s}");
    }

    #[test]
    fn the_installed_one_and_a_closed_window_start_nothing() {
        let mut t = T::new("617.42", true);
        t.choose();
        t.c.click(idx(K_DL, 1), t.now);
        t.c.click(sub(K_WIN, "x"), t.now);
        t.c.click(idx(K_DL, 0), t.now);
        assert!(t.c.job().is_none());
        assert!(t.log.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn while_one_runs_the_other_downloads_are_greyed_and_do_nothing() {
        let mut t = T::new("610.88", true);
        let log = t.log.clone();
        let held = log.gate.lock().unwrap();
        t.choose();
        t.c.click(idx(K_DL, 0), t.now);
        // wait until it is installing (held at the gate)
        let t0 = std::time::Instant::now();
        while !matches!(t.c.job().map(|j| j.phase), Some(Phase::Installing)) {
            assert!(t0.elapsed().as_secs() < 10);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        t.c.tick();
        let s = t.text();
        assert!(s.contains("|Driver versions|Installing 621.01 \u{b7} the screen may go black for a moment|Choose|"), "the card's line: {s}");
        assert!(s.contains("|621.01|Newest|Installing \u{b7} the screen may go black for a moment|617.42|"), "the window's line: {s}");
        let l = t.window().unwrap();
        assert!(l.rect_of(K_BAR).is_some(), "its progress on its line");
        assert!(l.focusables().iter().all(|k| *k != idx(K_DL, 1) && *k != idx(K_DL, 2)), "the other Downloads are greyed");
        t.c.click(idx(K_DL, 1), t.now);
        drop(held);
        t.c.settle();
        assert_eq!(t.log.calls.lock().unwrap().len(), 1, "one install only");
    }

    #[test]
    fn a_job_whose_thread_dies_ends_and_can_be_tried_again() {
        let mut t = T::new("610.88", true);
        t.log.panic.store(true, std::sync::atomic::Ordering::SeqCst);
        t.choose();
        t.c.click(idx(K_DL, 0), t.now);
        t.c.settle();
        let s = t.text();
        assert!(s.contains("|Driver versions|New driver available - 621.01|Choose|"), "the card is back to normal: {s}");
        assert!(s.contains("|621.01|Newest|Not installed \u{b7} Stopped|Download|"), "how it ended, on its line: {s}");
        t.c.click(idx(K_DL, 0), t.now);
        t.c.settle();
        assert!(t.text().contains("|621.01|Newest|21 Oct 2026 \u{b7} Game Ready|Installed|"));
    }

    #[test]
    fn a_download_left_by_a_quit_mid_job_goes_when_the_tab_opens() {
        let dir = std::env::temp_dir().join(format!("bu-drvcard-left-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("621.01.exe.part"), b"MZ half").unwrap();
        let mut c = Card::default();
        c.open_with(Arc::new(Backend::fake("610.88", true, 10, dir.clone(), Arc::default())), Shared::default());
        c.settle();
        assert!(!dir.exists());
    }

    #[test]
    fn a_no_at_the_admin_prompt_says_so_and_deletes_the_download() {
        let mut t = T::new("610.88", true);
        *t.log.fail.lock().unwrap() = Some(crate::admin::NOT_CHANGED.to_string());
        t.choose();
        t.c.click(idx(K_DL, 0), t.now);
        t.c.settle();
        assert!(!t.dir.exists());
        let s = t.text();
        assert!(s.contains("|610.88|Driver versions|New driver available - 621.01|Choose|"), "not kept on the card: {s}");
        assert!(s.contains("|621.01|Newest|Not installed \u{b7} Needs admin \u{2014} not changed|Download|"), "it can be tried again: {s}");
    }

    #[test]
    fn a_restart_to_finish_offers_no_second_download_of_it() {
        let mut t = T::new("610.88", true);
        t.log.restart.store(true, std::sync::atomic::Ordering::SeqCst);
        t.choose();
        t.c.click(idx(K_DL, 0), t.now);
        t.c.settle();
        let s = t.text();
        assert!(s.contains("|Driver versions|Installed 621.01 \u{b7} restart the PC to finish|Choose|"), "the card says it: {s}");
        assert!(s.contains("|621.01|Newest|Installed \u{b7} restart the PC to finish|Restart|617.42|"), "no Download on it: {s}");
        assert!(t.window().unwrap().rect_of(idx(K_DL, 0)).is_none());
        t.c.click(idx(K_DL, 0), t.now);
        assert!(!t.c.busy());
        assert_eq!(t.log.calls.lock().unwrap().len(), 1, "not downloaded and installed again");
    }

    #[test]
    fn a_read_that_started_before_the_install_ended_doesnt_put_the_old_driver_back() {
        let mut t = T::new("610.88", true);
        let old = t.c.look.clone().unwrap();
        let rel = t.c.releases()[0].clone();
        // the tab opened while a job ran: its read (the old driver) comes in after the job ended
        let (tx, rx) = std::sync::mpsc::channel();
        t.c.rx = Some(rx);
        t.c.look = None;
        *t.c.job.lock().unwrap() = Some(Job { release: rel, phase: Phase::Done { result: Ok(Installed::Done), after: DriverVersion::parse("621.01") }, seq: 9 });
        t.c.tick();
        tx.send(old.clone()).unwrap();
        t.c.tick();
        assert_eq!(t.c.gpu().unwrap().installed, DriverVersion::parse("621.01"), "a read before any tick");
        tx.send(old).unwrap();
        t.c.tick();
        assert_eq!(t.c.gpu().unwrap().installed, DriverVersion::parse("621.01"), "a late read after the tick");
        assert!(t.text().contains("|Your version|NVIDIA GeForce RTX 4090|621.01|Driver versions|You have the newest|"));
    }

    #[test]
    fn the_download_shows_its_progress() {
        let mut t = T::new("610.88", true);
        let release = t.c.releases()[0].clone();
        *t.c.job.lock().unwrap() = Some(Job {
            release,
            phase: Phase::Downloading { done: 446_000_000, total: Some(1_040_000_000) },
            seq: 1,
        });
        t.choose();
        let s = t.text();
        assert!(s.contains("|Driver versions|Downloading 621.01 \u{b7} 42 % of 1.04 GB|Choose|"), "{s}");
        assert!(s.contains("|621.01|Newest|Downloading \u{b7} 42 % of 1.04 GB|617.42|"), "{s}");
    }

    #[test]
    fn nvidia_unreachable_says_so_and_offers_nothing() {
        let dir = std::env::temp_dir().join(format!("bu-drvcard-off-{}", std::process::id()));
        let mut be = Backend::fake("610.88", true, 10, dir, Arc::default());
        let h = bu_updater::FakeHttp::new();
        h.route(bu_gpudriver::lookup::PRODUCTS_URL, bu_updater::FakeReply::Offline("no network".into()));
        be.http = Arc::new(h);
        let mut c = Card::default();
        c.open_with(Arc::new(be), Shared::default());
        c.settle();
        let mut t = T { c, g: Gfx::new(1.0), st: State::default(), now: 0.0, log: Arc::default(), dir: PathBuf::new() };
        let s = t.text();
        assert_eq!(s, "Graphics driver|Your version|NVIDIA GeForce RTX 4090|610.88|Driver versions|Couldn\u{2019}t check: no connection (connecting): no network|Choose");
        t.choose();
        assert!(t.c.win.is_none(), "nothing to choose from");
    }

    #[test]
    fn a_long_list_scrolls_inside_the_window() {
        let mut t = T::new("610.88", true);
        t.c.add_old_releases(47);
        t.choose();
        let l = t.window().unwrap();
        let (_, _, _, wh) = l.rect_of(sub(K_WIN, "win")).unwrap();
        assert!(wh <= 468.0, "the window keeps its size: {wh}");
        let (_, _, _, list_h) = l.rect_of(sub(K_WIN, "list")).unwrap();
        let (_, _, _, inner) = l.rect_of(sub(sub(K_WIN, "list"), "in")).unwrap();
        assert!(inner > list_h * 3.0, "the list is taller than its box: {inner} > {list_h}");
        // the slim thumb's own 10 px lane: the lines end 10 px sooner than in a short list
        let long = l.rect_of(idx(K_DL, 0)).unwrap();
        let mut short = T::new("610.88", true);
        short.choose();
        let s = short.window().unwrap().rect_of(idx(K_DL, 0)).unwrap();
        assert_eq!(s.0 + s.2 - (long.0 + long.2), 10.0, "the thumb's lane");
    }

    #[test]
    fn windows_versions_of_nvidia_versions() {
        for v in ["610.88", "621.01", "560.70", "552.22"] {
            let d = DriverVersion::parse(v).unwrap();
            assert_eq!(DriverVersion::from_windows(&windows_version(d)), Some(d), "{v}");
        }
    }
}

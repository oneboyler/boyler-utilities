//! The Storage tab (menu-v22 page `sto`, v18b + v22): drive tiles; What's using <drive> (File types / Folders) measured ONLY
//! when Measure is pressed (a full C: walk took 90 s / 145 CPU-s in Order 007: the page shows its progress and can stop
//! it); Clean up (sizes measured only on Measure, cleaning only after the sizes are shown and Clean is pressed); Drive
//! health (read-only). Wired to crates/storage: the FAKE PC (`FakeOs::drawing`) in every test copy, the real one otherwise.

mod view;

#[cfg(test)]
mod tests;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bu_storage::cleanup::{self, CleanKind, CleanPlan, CleanReport, PartState};
use bu_storage::drives::{self, DriveTile};
use bu_storage::health::{self, HealthRow};
use bu_storage::bigfiles::BigFile;
use bu_storage::scan::{self, FolderId, ScanControl, ScanResult};
use bu_storage::{FakeOs, StorageError, StorageOs};

use crate::pages::{Env, Page};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, sub, El, Key};

const K_DRV: Key = key("sto.drv");
const K_SEG: Key = key("sto.seg");
const K_MEASURE: Key = key("sto.measure");
const K_STOP: Key = key("sto.stop");
const K_AGAIN: Key = key("sto.again");
const K_TYR: Key = key("sto.tyr");
const K_BACK: Key = key("sto.back");
const K_CRUMB: Key = key("sto.crumb");
const K_FDR: Key = key("sto.fdr");
const K_FOP: Key = key("sto.fop");
const K_CN: Key = key("sto.cn");
const K_CLN: Key = key("sto.cln");
/// Clean up's "Measure again" link (Order 069)
const K_CAGAIN: Key = key("sto.cagain");
const K_TOAST: Key = key("sto.toast");
/// The Folders view's small Folders | Files switch (Order 069)
const K_FSEG: Key = key("sto.fseg");
/// Files view: a row (right-click = its menu), its Show in folder button, the menu, the delete confirm
const K_BIG: Key = key("sto.big");
const K_BOP: Key = key("sto.bop");
const K_FMENU: Key = key("sto.fmenu");
const K_FDLG: Key = key("sto.fdlg");
/// Drive health's "Read with admin" link (Order 039, A_039_01)
const K_HADM: Key = key("sto.hadm");

/// GBf (the drawing's size format, 1 GB = 1024³ bytes): 1000 GB and over in TB (2 decimals), 100 GB and over whole, 1 GB
/// and over one decimal, under that MB.
pub fn gbf(bytes: u64) -> String {
    let v = bytes as f64 / 1_073_741_824.0;
    if v >= 1000.0 {
        format!("{:.2} TB", v / 1024.0)
    } else if v >= 100.0 {
        format!("{} GB", v.round() as i64)
    } else if v >= 1.0 {
        format!("{v:.1} GB")
    } else if v > 0.0 {
        format!("{} MB", ((v * 1024.0).round() as i64).max(1))
    } else {
        "0 B".into()
    }
}

/// One drive's "What's using" state.
#[derive(Clone)]
enum Scan {
    /// "Not measured yet" + Measure
    Idle,
    /// the walk runs on its own thread; `ctl` = its progress and Stop
    Running { ctl: Arc<ScanControl>, started: f64 },
    /// `when` = the clock time it finished ("21:37"), `at` = the same moment ("Measured 5 min ago")
    Done { result: Arc<ScanResult>, when: String, at: std::time::SystemTime },
}

enum Msg {
    Drives(Result<Vec<DriveTile>, StorageError>),
    Health(Vec<HealthRow>),
    /// a walk's end; the `ScanControl` says WHICH walk (a stopped walk's late answer must not touch a newer one)
    Scanned(char, Arc<ScanControl>, Result<Box<ScanResult>, StorageError>, String, std::time::SystemTime),
    /// a folder of a kept (pruned) walk scanned again: drive, folder, the walk it belongs to, its control, the result
    SubScanned(char, FolderId, Arc<ScanResult>, Arc<ScanControl>, Result<Box<ScanResult>, StorageError>),
    Measured(Result<CleanPlan, StorageError>),
    Cleaned(Result<CleanReport, StorageError>),
    /// "Read with admin" (Order 039): the health rows read again with the elevated copy's admin reads
    HealthAdmin(Result<Vec<HealthRow>, crate::admin::AdminError>),
    Toast(String),
    /// a file moved to the Recycle Bin (or not): drive, the file, the answer
    Recycled(char, BigFile, Result<(), StorageError>),
}

/// Windows' own Temp folder in a clean report: emptied by the app's elevated copy (Order 039, one admin prompt, as part of
/// the same Clean press). Declined / failed: its part stays skipped (`NeedsAdmin`) and the toast says so.
fn clean_windows_temp(admin: &crate::admin::client::Admin, rep: &mut CleanReport) {
    let Some(row) = rep.rows.iter_mut().find(|r| r.kind == Some(CleanKind::TempFiles)) else { return };
    if let Ok(f) = admin.call(crate::admin::Purpose::Storage, crate::admin::Op::CleanWindowsTemp) {
        let n: Vec<u64> = f.iter().filter_map(|x| x.parse().ok()).collect();
        if let [fb, ff, ub, uf] = n[..] {
            row.freed_bytes += fb;
            row.freed_files += ff;
            row.in_use_bytes += ub;
            row.in_use_files += uf;
            row.skipped.retain(|(_, st)| *st != PartState::NeedsAdmin);
        }
    }
}

/// The health rows with the admin-only fields read by the app's elevated copy (one prompt for every drive): each disk
/// whose own read missed something is read again there. A "No" ends it (nothing changed on screen).
fn read_health_admin(os: &dyn StorageOs, admin: &Arc<crate::admin::client::Admin>) -> Result<Vec<HealthRow>, crate::admin::AdminError> {
    use crate::admin::{AdminError, Op, Purpose};
    let _one = admin.scope(Purpose::Storage);
    let drives = os.drives().unwrap_or_default();
    let disks = os.physical_disks().map_err(|e| AdminError::Failed(e.to_string()))?;
    let mut rows = Vec::new();
    for d in disks {
        let mut raw = os.disk_health(d.number).unwrap_or_default();
        if !raw.needs_admin.is_empty() {
            match admin.call(Purpose::Storage, Op::DiskHealth(d.number)) {
                Ok(f) => raw = crate::admin::exec::parse_health(&f).ok_or_else(|| AdminError::Failed("bad answer".into()))?,
                Err(AdminError::Declined) => return Err(AdminError::Declined),
                Err(_) => {}
            }
        }
        rows.push(health::build_row(&d, &raw, &drives));
    }
    Ok(rows)
}

type Inbox = Arc<Mutex<Vec<Msg>>>;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum View {
    #[default]
    Types,
    Folders,
}

/// What the Folders view lists (Order 069): the folders, or the drive's biggest single files.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum FMode {
    #[default]
    Folders,
    Files,
}

/// The drawing's timings (menu-v22 stoRender / clnMeasure / the Clean click).
/// The list's slide-in when the view, the drive or the folder changes: `translateX(dir*10px) -> 0`, opacity 0 -> 1, 220 ms EASE_OUT.
pub(super) const LIST_MS: f64 = 220.0;
/// After Measure: each size arrives at 300 + i*180 ms; the rows are measured (ticks, "Clean …") at 1000 ms.
pub(super) const SIZE_FIRST_MS: f64 = 300.0;
pub(super) const SIZE_STEP_MS: f64 = 180.0;
pub(super) const SIZES_DONE_MS: f64 = 1000.0;
/// Clean: the ticked rows count down one after another (380 ms apart), each in 520 ms, then show ✓.
pub(super) const CLEAN_STEP_MS: f64 = 380.0;
pub(super) const CLEAN_ROW_MS: f64 = 520.0;

/// A finished clean while its rows still count down.
#[derive(Clone)]
pub(super) struct Countdown {
    /// when the first row starts
    pub t0: f64,
    /// the cleaned rows in the order they count down, with their size before
    pub rows: Vec<(CleanKind, u64)>,
    pub report: CleanReport,
}

impl Countdown {
    /// How far row `k` is (None = not one of the cleaned rows; 0 = not started; 1 = done).
    pub fn progress(&self, k: CleanKind, now: f64, rm: bool) -> Option<f64> {
        let j = self.rows.iter().position(|r| r.0 == k)?;
        if rm {
            return Some(1.0);
        }
        Some(((now - self.t0 - j as f64 * CLEAN_STEP_MS) / CLEAN_ROW_MS).clamp(0.0, 1.0))
    }
    fn end(&self) -> f64 {
        self.t0 + self.rows.len().saturating_sub(1) as f64 * CLEAN_STEP_MS + CLEAN_ROW_MS
    }
}

/// What the page keeps in the app's store when it closes (the owner Oct 8, F2: closing the window must not stop a measure or
/// wipe its result - "no rescan on every refresh"): the walks (a running one goes on: its thread answers into the kept
/// inbox, read when the tab opens again), the results with their time, the clean-up sizes. Key [`KEEP_KEY`].
#[derive(Clone)]
struct Kept {
    inbox: Inbox,
    scans: HashMap<char, Scan>,
    drv: char,
    view: View,
    fmode: FMode,
    recycled: Vec<(char, BigFile)>,
    plan: Option<CleanPlan>,
    measuring: bool,
    cleaning: bool,
    ticked: HashSet<CleanKind>,
    report: Option<CleanReport>,
    staged: Option<CleanPlan>,
    size_t0: f64,
    measure_at: f64,
    countdown: Option<Countdown>,
    /// false once the menu closed: a walk that ends then keeps only its pruned tree
    menu_open: Arc<AtomicBool>,
}

const KEEP_KEY: &str = "sto.kept";

#[derive(Default)]
pub struct Storage {
    env: Env,
    os: Option<Arc<dyn StorageOs>>,
    fake: Option<Arc<FakeOs>>,
    inbox: Inbox,
    drives: Vec<DriveTile>,
    drv: char,
    view: View,
    fmode: FMode,
    /// Files moved to the Recycle Bin from the Files list since the drive was measured (they stay out of the list)
    recycled: Vec<(char, BigFile)>,
    scans: HashMap<char, Scan>,
    /// the folder looked at in the Folders view (a chain from the root)
    path: Vec<FolderId>,
    plan: Option<CleanPlan>,
    measuring: bool,
    cleaning: bool,
    ticked: HashSet<CleanKind>,
    report: Option<CleanReport>,
    health: Vec<HealthRow>,
    /// Drive health was read with admin this visit (the rows stay filled until the tab closes) / is being read now
    health_admin: bool,
    health_reading: bool,
    toast: Option<(String, f64)>,
    now: f64,
    /// Order 055: the frame's time for the sweep / the shimmers (live boxes read it when they are painted; `tick` sets it
    /// every frame) - they move at the monitor's rate without the page being built again
    clock: Rc<Cell<f64>>,
    /// the last build drew a walk's sweep (its progress line, bytes and seconds, is data: built again 4 times a second) /
    /// a Clean up shimmer
    sweep_on: Cell<bool>,
    shim_on: Cell<bool>,
    /// when the sweep's progress line was last built again
    text_at: f64,
    /// the last `tick`'s true moved only the live boxes
    live_only: bool,
    /// test copies only (`BU_TEST_STATE scroll=<px>`): the page moved up (see network's)
    shift: f32,
    /// reduced motion (Windows' "show animations" off, from the last build): no staging, no slides
    rm: bool,
    /// the list's slide-in: (start, direction -1 / 0 / +1)
    list_anim: Option<(f64, f32)>,
    /// Clean up measured, its sizes still arriving one by one (`size_t0` = when the first one shows minus 300 ms)
    staged: Option<CleanPlan>,
    size_t0: f64,
    /// when Measure (Clean up) was pressed
    measure_at: f64,
    /// a finished clean whose rows still count down
    countdown: Option<Countdown>,
    /// Files view: the right-click menu (file, pointer x, y, since) and the delete confirm (file, since; closing since)
    fmenu: Option<(BigFile, f32, f32)>,
    fdlg: Option<(BigFile, f64)>,
    fdlg_closing: Option<f64>,
    /// a folder of a pruned walk being scanned again: (drive, folder, its control, since)
    sub: Option<(char, FolderId, Arc<ScanControl>, f64)>,
    /// the menu is open (shared with the walks: one that ends after the menu closed keeps only its pruned tree)
    menu_open: Arc<AtomicBool>,
    /// set on the first open; the page object lives as long as the menu window, so this guard's `Drop` = the menu closed
    guard: MenuGuard,
}

impl Storage {
    /// A test copy's preset states (`BU_TEST_STATE`, read only in a FAKE test copy: `env.fake()`): `scroll=<px>`, `measured` (C: walked, File
    /// types), `folders` (+ the Folders view), `files` (+ the Files list), `filemenu` / `filedlg` (+ a Files row's right-click menu / its delete confirm), `scanning` (a walk 41 s in), `cleanmeasured` (Clean up measured), `cleaned`.
    fn apply_test_states(&mut self) {
        let states: Vec<String> =
            std::env::var("BU_TEST_STATE").map(|s| s.split(',').map(|t| t.trim().to_string()).collect()).unwrap_or_default();
        let Some(os) = self.os.clone() else { return };
        for t in states {
            match t.as_str() {
                "measured" | "folders" | "files" | "filemenu" | "filedlg" => {
                    if let Ok(mut r) = scan::scan_drive(os.as_ref(), 'C', &ScanControl::new()) {
                        if self.env.frozen {
                            // the drawing's sample sizes (`types:{games:386,videos:268,apps:152,pics:48,docs:19}`); its folder tree
                            // and its types are not one consistent walk, so the fake's walk can't give both
                            let g = |v: f64| (v * 1_073_741_824.0) as u64;
                            r.types.walked = [g(386.0), g(268.0), g(152.0), g(48.0), g(19.0), g(378.0)];
                        }
                        self.scans.insert('C', Scan::Done { result: Arc::new(r), when: "21:37".into(), at: std::time::SystemTime::now() });
                    }
                    if t != "measured" {
                        self.view = View::Folders;
                    }
                    if matches!(t.as_str(), "files" | "filemenu" | "filedlg") {
                        self.fmode = FMode::Files;
                    }
                    // the first file that can be deleted (the drive's top ones are Windows' pagefile / hiberfil)
                    if let Some(f) = self.big_files().into_iter().find(|b| b.can_recycle()) {
                        if t == "filemenu" {
                            self.fmenu = Some((f, 330.0, 330.0));
                        } else if t == "filedlg" {
                            self.fdlg = Some((f, self.now - 1000.0));
                        }
                    }
                }
                // Order 039: the hard drive read without admin (no SMART) - Drive health shows "Read with admin"
                "healthadmin" => {
                    for h in self.health.iter_mut().filter(|h| h.media == bu_storage::MediaKind::Hdd) {
                        h.temperature_c = None;
                        h.power_on_hours = None;
                        h.admin_would_add = vec!["SMART attributes (temperature, power-on hours, moved sectors)".into()];
                    }
                }
                "scanning" => {
                    let ctl = Arc::new(ScanControl::new());
                    self.scans.insert('C', Scan::Running { ctl, started: self.now - 41_000.0 });
                }
                "cleanmeasured" | "cleaned" => {
                    if let Ok(p) = cleanup::measure(os.as_ref()) {
                        if t == "cleaned" {
                            let ticked: Vec<CleanKind> = p.default_ticked();
                            self.report = p.clean(os.as_ref(), &ticked).ok();
                        }
                        self.plan = Some(p);
                    }
                }
                s => {
                    if let Some(px) = s.strip_prefix("scroll=").and_then(|v| v.parse().ok()) {
                        self.shift = px;
                    }
                }
            }
        }
    }

    fn post(inbox: &Inbox, m: Msg) {
        if let Ok(mut q) = inbox.lock() {
            q.push(m);
        }
        // the menu reads it on its next tick (an open menu repaints; a closed one finds it when the tab opens)
        crate::services::Waker.wake();
    }

    fn toast(&mut self, t: impl Into<String>) {
        self.toast = Some((t.into(), self.now));
    }

    /// A job on its own thread (the fake answers in place in a test copy, so its first picture is complete).
    fn run(&mut self, inline: bool, f: impl FnOnce(&dyn StorageOs, &Inbox) + Send + 'static) {
        let (Some(os), inbox) = (self.os.clone(), self.inbox.clone()) else { return };
        if inline {
            f(os.as_ref(), &inbox);
            self.drain();
        } else {
            let _ = std::thread::Builder::new().name("bu-sto-job".into()).spawn(move || f(os.as_ref(), &inbox));
        }
    }

    fn drain(&mut self) -> bool {
        let msgs: Vec<Msg> = match self.inbox.lock() {
            Ok(mut q) => std::mem::take(&mut *q),
            Err(_) => return false,
        };
        let any = !msgs.is_empty();
        for m in msgs {
            self.apply(m);
        }
        any
    }

    fn apply(&mut self, m: Msg) {
        match m {
            Msg::Drives(d) => match d {
                Ok(d) => {
                    if !d.iter().any(|t| t.info.letter == self.drv) {
                        self.drv = drives::default_choice(&d).unwrap_or('C');
                    }
                    self.drives = d;
                }
                Err(e) => self.toast(format!("Can't read the drives · {e}")),
            },
            Msg::Health(h) => self.health = h,
            Msg::Scanned(l, from, r, when, at) => {
                // only the walk running now counts: Stop + Measure starts a new walk while the old one may still answer
                // (Cancelled, or even a result) - that answer is dropped
                let current = matches!(self.scans.get(&l), Some(Scan::Running { ctl, .. }) if Arc::ptr_eq(ctl, &from));
                if !current {
                    return;
                }
                match r {
                    Ok(res) => {
                        self.recycled.retain(|(d, _)| *d != l);
                        self.scans.insert(l, Scan::Done { result: Arc::from(res), when, at });
                        if l == self.drv {
                            self.path.clear();
                            self.slide(0.0);
                        }
                    }
                    Err(StorageError::Cancelled) => {
                        self.scans.insert(l, Scan::Idle);
                    }
                    Err(e) => {
                        self.scans.insert(l, Scan::Idle);
                        self.toast(format!("Couldn't measure {l}: · {e}"));
                    }
                }
            }
            Msg::SubScanned(l, id, base, from, r) => {
                if !matches!(&self.sub, Some((_, _, c, _)) if Arc::ptr_eq(c, &from)) {
                    return;
                }
                self.sub = None;
                // only into the walk it was started from (a new Measure replaced it meanwhile: dropped)
                let Some(Scan::Done { result, when, at }) = self.scans.get(&l).cloned() else { return };
                if !Arc::ptr_eq(&result, &base) {
                    return;
                }
                match r {
                    Ok(sub) => {
                        let mut full = (*result).clone();
                        if full.tree.graft(id, &sub.tree).is_ok() {
                            self.scans.insert(l, Scan::Done { result: Arc::new(full), when, at });
                            if l == self.drv {
                                self.path.push(id);
                                self.slide(1.0);
                            }
                        }
                    }
                    Err(StorageError::Cancelled) => {}
                    Err(e) => self.toast(format!("Couldn't measure that folder · {e}")),
                }
            }
            Msg::Measured(r) => match r {
                // Measure again (after a clean, or by its link): the new sizes replace the old at once, the ticks stay as they are
                Ok(p) if self.plan.is_some() => {
                    self.plan = Some(p);
                    self.measuring = false;
                }
                Ok(p) => {
                    // the sizes arrive one by one (the drawing: 300 + i*180 ms after the press); a slower real measure
                    // starts that run when its answer is here
                    self.size_t0 = self.measure_at.max(self.now - SIZE_FIRST_MS);
                    self.staged = Some(p);
                    self.settle();
                }
                Err(e) => {
                    self.measuring = false;
                    self.toast(format!("Couldn't measure · {e}"));
                }
            },
            Msg::Cleaned(r) => match r {
                Ok(rep) => {
                    // the cleaned rows count down one after another; the toast and the tiles follow the last one
                    let plan = self.plan.clone();
                    let rows = CleanKind::ALL
                        .iter()
                        .filter(|k| rep.rows.iter().any(|c| c.kind == Some(**k)))
                        .map(|k| (*k, plan.as_ref().and_then(|p| p.row(*k)).map(|r| r.bytes).unwrap_or(0)))
                        .collect();
                    self.countdown = Some(Countdown { t0: self.now, rows, report: rep });
                    self.settle();
                }
                Err(e) => {
                    self.cleaning = false;
                    self.toast(format!("Couldn't clean · {e}"));
                }
            },
            Msg::HealthAdmin(r) => {
                self.health_reading = false;
                match r {
                    Ok(rows) => {
                        self.health = rows;
                        self.health_admin = true;
                    }
                    Err(e) => self.toast(e.to_string()),
                }
            }
            Msg::Toast(t) => self.toast(t),
            Msg::Recycled(l, file, r) => match r {
                Ok(()) => {
                    // it leaves the list (the folder sizes and the drive's free space stay as measured: the bin still holds it)
                    self.recycled.push((l, file.clone()));
                    self.toast(format!("Moved to the Recycle Bin · {}", gbf(file.bytes)));
                    // the Recycle bin row of Clean up holds one more file now: its size is measured again (else Clean would
                    // empty the bin with this file in it without the page having said so)
                    if self.plan.is_some() {
                        self.measure_clean();
                    }
                }
                Err(StorageError::UnsafePath(_)) => self.toast("Windows manages that file"),
                Err(StorageError::NotFound(_)) => {
                    self.recycled.push((l, file));
                    self.toast("That file is already gone");
                }
                Err(e) => self.toast(format!("Couldn’t delete {} · {e}", file.name)),
            },
        }
    }


    /// "Read with admin": the drives' admin-only details, read once by the app's elevated copy (read-only; only on the click).
    fn health_with_admin(&mut self) {
        if self.health_reading || self.health_admin {
            return;
        }
        self.health_reading = true;
        let admin = crate::admin::client::admin();
        self.run(false, move |os, inbox| Self::post(inbox, Msg::HealthAdmin(read_health_admin(os, &admin))));
    }

    /// Show the link? (a drive whose own read missed something, not read with admin yet)
    pub(super) fn health_link(&self) -> bool {
        !self.health_admin && self.health.iter().any(|h| !h.admin_would_add.is_empty())
    }

    /// The staged parts that are over: the measured sizes are all shown -> measured; the countdown ended -> the result,
    /// its toast and the tiles' new free space. True when something changed.
    fn settle(&mut self) -> bool {
        let mut changed = false;
        if self.staged.is_some() && (self.rm || self.now >= self.size_t0 + SIZES_DONE_MS) {
            self.plan = self.staged.take();
            self.measuring = false;
            changed = true;
        }
        if self.countdown.as_ref().is_some_and(|c| self.rm || self.now >= c.end()) {
            let rep = self.countdown.take().map(|c| c.report).unwrap_or_default();
            self.cleaning = false;
            let (freed, left) = (rep.freed_bytes(), rep.in_use_bytes());
            let mut t = format!("Freed {}", gbf(freed));
            if left > 0 {
                t.push_str(&format!(" · {} of temp files are in use and stay", gbf(left)));
            }
            // Windows' Temp folder: the admin prompt answered No (or the copy failed)
            if rep.rows.iter().any(|r| r.skipped.iter().any(|(_, st)| *st == PartState::NeedsAdmin)) {
                t.push_str(" · Windows temp folder: needs admin, not changed");
            }
            // rows of an earlier clean that this one did not touch keep their "Cleaned"
            let mut all = self.report.take().unwrap_or_default();
            all.rows.retain(|old| !rep.rows.iter().any(|n| n.kind == old.kind));
            all.rows.extend(rep.rows);
            self.report = Some(all);
            self.toast(t);
            // Order 069: after ANY clean (one row or all) the sizes are measured again by themselves - whatever was left unticked
            self.measure_clean();
            // the tiles show the new free space
            self.run(self.env.fake(), |os, inbox| Self::post(inbox, Msg::Drives(drives::list(os))));
            changed = true;
        }
        changed
    }

    /// The list slides in (`stoRender(true, dir)`): -1 = back up, +1 = one folder deeper, 0 = another view / drive / new result.
    fn slide(&mut self, dir: f32) {
        self.list_anim = if self.rm { None } else { Some((self.now, dir)) };
    }

    fn scan_of(&self, l: char) -> &Scan {
        self.scans.get(&l).unwrap_or(&Scan::Idle)
    }

    // ---- actions (only from a click)

    /// Measure (or Measure again): the walk of one drive on its own thread, with progress and Stop.
    fn measure_drive(&mut self, l: char) {
        if matches!(self.scan_of(l), Scan::Running { .. }) {
            return;
        }
        let ctl = Arc::new(ScanControl::new());
        self.scans.insert(l, Scan::Running { ctl: ctl.clone(), started: self.now });
        let inbox = self.inbox.clone();
        let Some(os) = self.os.clone() else { return };
        let frozen = self.env.frozen;
        let menu_open = self.menu_open.clone();
        // the walk owns its OS layer and answers into the inbox the app keeps (`Kept`): it goes on with the tab left or the
        // window closed, and its result waits there
        let _ = std::thread::Builder::new().name("bu-sto-scan".into()).spawn(move || {
            let mut r = scan::scan_drive(os.as_ref(), l, &ctl).map(Box::new);
            // the menu closed meanwhile: only what the page shows first is kept (no RAM while nobody looks)
            if !menu_open.load(Ordering::Relaxed) {
                if let Ok(res) = &mut r {
                    res.tree = res.tree.pruned();
                }
            }
            let when = if frozen { "21:37".to_string() } else { bu_network::real::WindowsNet::local_hhmm() };
            Self::post(&inbox, Msg::Scanned(l, ctl, r, when, std::time::SystemTime::now()));
        });
    }

    /// Scan one folder of the drive's kept (pruned) walk again; its answer is grafted back into that walk.
    fn scan_sub(&mut self, id: FolderId, base: Arc<ScanResult>) {
        if self.sub.is_some() {
            return;
        }
        let (Some(os), Ok(path)) = (self.os.clone(), base.tree.path(id)) else { return };
        let l = self.drv;
        let ctl = Arc::new(ScanControl::new());
        self.sub = Some((l, id, ctl.clone(), self.now));
        let inbox = self.inbox.clone();
        let _ = std::thread::Builder::new().name("bu-sto-sub".into()).spawn(move || {
            let r = scan::scan_subfolder(os.as_ref(), l, &path, &ctl).map(Box::new);
            Self::post(&inbox, Msg::SubScanned(l, id, base, ctl, r));
        });
    }

    fn stop_drive(&mut self, l: char) {
        if let Some(Scan::Running { ctl, .. }) = self.scans.get(&l) {
            ctl.cancel();
        }
        self.scans.insert(l, Scan::Idle);
    }

    /// Measure the clean-up rows: the first time (the sizes arrive one by one), and again whenever asked or after a clean
    /// (Order 069: never blocked by what was ticked or cleaned - only by a measure or a clean already running).
    /// The Files list of the shown drive (empty before it is measured).
    fn big_files(&self) -> Vec<BigFile> {
        match self.scan_of(self.drv) {
            Scan::Done { result, .. } => result.biggest.iter().filter(|b| !self.recycled.iter().any(|(d, r)| *d == self.drv && r == *b)).cloned().collect(),
            _ => Vec::new(),
        }
    }

    /// Right-click on a Files row: its menu at the pointer.
    fn file_menu(&mut self, i: usize, x: f32, y: f32) {
        if self.fdlg.is_some() {
            return;
        }
        if let Some(f) = self.big_files().get(i) {
            self.fmenu = Some((f.clone(), x, y));
        }
    }

    /// Delete in the menu: the confirm first (nothing moves before it).
    fn file_delete_ask(&mut self, f: BigFile) {
        if f.can_recycle() {
            self.fmenu = None;
            self.fdlg = Some((f, self.now));
            self.fdlg_closing = None;
        }
    }

    fn close_file_dialog(&mut self) {
        if self.fdlg.is_some() && self.fdlg_closing.is_none() {
            self.fdlg_closing = Some(self.now);
        }
    }

    /// The confirm's Delete: the file goes to the Recycle Bin (it can be restored from there), on its own thread.
    fn file_delete(&mut self) {
        let Some((f, _)) = self.fdlg.take() else { return };
        self.fdlg_closing = None;
        // a --real-read test copy reads the real PC and changes nothing
        if self.env.real_read {
            self.toast("Test copy: nothing is deleted");
            return;
        }
        let l = self.drv;
        self.run(false, move |os, inbox| {
            let r = bu_storage::bigfiles::recycle(os, &f.path());
            Self::post(inbox, Msg::Recycled(l, f, r))
        });
    }

    fn measure_clean(&mut self) {

        if self.measuring || self.cleaning {
            return;
        }
        self.measuring = true;
        self.measure_at = self.now;
        if self.plan.is_none() {
            self.report = None;
        }
        self.run(false, |os, inbox| Self::post(inbox, Msg::Measured(cleanup::measure(os))));
    }

    /// Clean: only the ticked rows, only from the measured plan (the crate refuses anything not measured).
    fn clean(&mut self) {
        let Some(plan) = self.plan.clone() else { return };
        if self.cleaning {
            return;
        }
        let ticked: Vec<CleanKind> = CleanKind::ALL.iter().copied().filter(|k| self.ticked.contains(k) && plan.row(*k).is_some_and(|r| !r.is_empty())).collect();
        if ticked.is_empty() {
            return;
        }
        self.cleaning = true;
        // Windows' own Temp folder needs admin: the elevated copy empties it in the same Clean press (one prompt)
        let win_temp = ticked.contains(&CleanKind::TempFiles) && plan.row(CleanKind::TempFiles).is_some_and(|r| r.parts.iter().any(|p| p.state == PartState::NeedsAdmin));
        let admin = crate::admin::client::admin();
        self.run(false, move |os, inbox| {
            let mut r = plan.clean(os, &ticked);
            if let (true, Ok(rep)) = (win_temp, &mut r) {
                clean_windows_temp(&admin, rep);
            }
            Self::post(inbox, Msg::Cleaned(r))
        });
    }

    fn open_in_explorer(&mut self, path: std::path::PathBuf, file: bool) {
        if self.env.test {
            // a test copy never opens anything on the screen
            self.toast(if file { format!("Opens Explorer with {} selected", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()) } else { format!("Opens {} in Explorer", path.display()) });
            return;
        }
        let arg = if file { format!("/select,\"{}\"", path.display()) } else { format!("\"{}\"", path.display()) };
        // Order 047: starting a process holds the thread 10-40 ms (more with a busy disk): off the menu's thread
        crate::offui::spawn("sto-explorer", move || {
            let _ = std::process::Command::new("explorer.exe").raw_arg_compat(&arg).spawn();
        });
    }
}

/// `Command::raw_arg` (Windows) so Explorer gets its `/select,"path"` unquoted as one piece.
trait RawArg {
    fn raw_arg_compat(&mut self, a: &str) -> &mut Self;
}

impl RawArg for std::process::Command {
    fn raw_arg_compat(&mut self, a: &str) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.raw_arg(a)
    }
}

impl Page for Storage {
    fn id(&self) -> &'static str {
        "sto"
    }
    fn name(&self) -> &'static str {
        "Storage"
    }
    fn icon(&self) -> &'static str {
        "drive"
    }
    fn ready(&self) -> bool {
        self.os.is_none() || !self.drives.is_empty()
    }

    fn open(&mut self, env: &Env, now: f64) {
        self.close();
        self.env = env.clone();
        self.now = now;
        self.drv = 'C';
        let os: Arc<dyn StorageOs> = if env.fake() {
            let f = Arc::new(FakeOs::drawing());
            self.fake = Some(f.clone());
            f
        } else {
            Arc::new(bu_storage::RealOs::new())
        };
        self.os = Some(os);
        // Recycle bin + Temp files start ticked; shader and launcher caches do not (Order 069: clearing them makes games
        // recompile shaders). Empty rows show greyed whatever their tick.
        self.ticked = CleanKind::ALL.iter().copied().filter(|k| k.ticked_by_default()).collect();
        // what the tab had when it closed (a walk that ran on, the results, the clean-up sizes): shown at once
        if self.guard.keep.is_none() {
            self.guard.keep = Some(env.keep.clone());
        }
        self.menu_open = Arc::new(AtomicBool::new(true));
        if let Some(k) = env.keep.get::<Kept>(KEEP_KEY) {
            // results that came in while away are applied below; their plain toasts are stale by now
            if let Ok(mut q) = k.inbox.lock() {
                q.retain(|m| !matches!(m, Msg::Toast(_)));
            }
            self.inbox = k.inbox;
            self.scans = k.scans;
            self.drv = k.drv;
            self.view = k.view;
            self.fmode = k.fmode;
            self.recycled = k.recycled;
            self.plan = k.plan;
            self.measuring = k.measuring;
            self.cleaning = k.cleaning;
            self.ticked = k.ticked;
            self.report = k.report;
            self.staged = k.staged;
            self.size_t0 = k.size_t0;
            self.measure_at = k.measure_at;
            self.countdown = k.countdown;
            k.menu_open.store(true, Ordering::Relaxed);
            self.menu_open = k.menu_open;
        }
        // light reads only (drive sizes, the drives' own health records); nothing is measured on open (v22)
        let inline = env.fake();
        self.run(inline, |os, inbox| {
            Self::post(inbox, Msg::Drives(drives::list(os)));
            Self::post(inbox, Msg::Health(health::read_all(os).unwrap_or_default()));
        });
        // FAKE copies only: a --real-read test copy runs the real storage layer, and `cleaned` cleans
        if env.fake() {
            self.apply_test_states();
        }
    }

    fn close(&mut self) {
        // leaving the tab / closing the window keeps the work (F2): a running walk goes on, results stay (small: totals + the
        // folder tree the walk made); only the page's own pictures and services go
        if self.os.is_some() {
            self.env.keep.put(
                KEEP_KEY,
                Kept {
                    inbox: self.inbox.clone(),
                    scans: std::mem::take(&mut self.scans),
                    drv: self.drv,
                    view: self.view,
                    fmode: self.fmode,
                    recycled: std::mem::take(&mut self.recycled),
                    plan: self.plan.take(),
                    measuring: self.measuring,
                    cleaning: self.cleaning,
                    ticked: std::mem::take(&mut self.ticked),
                    report: self.report.take(),
                    staged: self.staged.take(),
                    size_t0: self.size_t0,
                    measure_at: self.measure_at,
                    countdown: self.countdown.take(),
                    menu_open: self.menu_open.clone(),
                },
            );
        }
        if let Some((_, _, c, _)) = self.sub.take() {
            c.cancel();
        }
        // the page object stays with the menu window: it keeps knowing it was opened and where the app's store is (its Drop)
        *self = Storage { inbox: Arc::new(Mutex::new(Vec::new())), guard: std::mem::take(&mut self.guard), ..Storage::default() };
    }

    /// Order 055: the sweep and the shimmers are live boxes (motion: true every frame while one shows, the live pass only);
    /// the page is built again for data only - a message, a staged size arriving, the end of the sizes / the countdown, and
    /// the walk's progress line 4 times a second.
    fn tick(&mut self, now: f64) -> bool {
        let prev = self.now;
        self.now = now;
        self.clock.set(now);
        let a = self.drain();
        let mut data = self.settle() || a;
        // a staged size shows up at its time (its shimmer is replaced by the number)
        if self.staged.is_some() && CleanKind::ALL.iter().enumerate().any(|(i, _)| {
            let at = self.size_t0 + SIZE_FIRST_MS + i as f64 * SIZE_STEP_MS;
            prev < at && at <= now
        }) {
            data = true;
        }
        if self.sweep_on.get() && now - self.text_at >= 250.0 {
            self.text_at = now;
            data = true;
        }
        self.live_only = !data;
        data || self.sweep_on.get() || self.shim_on.get()
    }
    fn live_only(&self) -> bool {
        self.live_only
    }

    /// Order 047: the staged parts end at known moments - the measured sizes become the plan at `size_t0 + 1000 ms`, the
    /// countdown's toast and result come at its end. The last size arrives (and its shimmer stops) before that, so with
    /// nothing moving the menu sleeps until then and `settle` runs at it. A walk's end wakes the menu by itself (`post`).
    fn wake_at(&self, now: f64) -> Option<f64> {
        [self.staged.as_ref().map(|_| self.size_t0 + SIZES_DONE_MS), self.countdown.as_ref().map(|c| c.end())]
            .into_iter()
            .flatten()
            .reduce(f64::min)
            .map(|t| t.max(now + 1.0))
    }

    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        self.now = cx.now;
        self.rm = cx.rm;
        // Order 047: no frames just because a walk runs - the shown drive's walk draws its sweep (`view::scan_state`, real
        // motion, asks for its own frames); a walk of a drive not shown has nothing on screen, and its end wakes the menu
        view::page(self, cx)
    }

    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        self.now = cx.now;
        match ev {
            Ev::Click(k) => self.click(*k),
            // a right-click on a Files row (or on a part of it) opens its menu at the pointer
            Ev::Context(k, x, y) if self.view == View::Folders && self.fmode == FMode::Files => {
                let n = self.big_files().len();
                if let Some(i) = (0..n).find(|i| *k == idx(K_BIG, *i) || *k == idx(K_BOP, *i)) {
                    self.file_menu(i, *x, *y);
                }
            }
            _ => {}
        }
    }


    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        let mut kids = Vec::new();
        if let Some(m) = view::file_menu_el(self, cx) {
            kids.push(m.z(20));
        }
        if let Some(t) = self.fdlg_closing {
            if cx.now - t > crate::ui::pieces::udlg::close_ms(cx.rm) {
                self.fdlg = None;
                self.fdlg_closing = None;
            }
        }
        if let Some(d) = view::file_dialog_el(self, cx) {
            kids.push(d);
        }
        if let Some((t, at)) = self.toast.clone() {
            if cx.now - at <= crate::ui::pieces::toast::SHOW_MS + 300.0 {
                kids.push(crate::ui::pieces::toast::toast(cx, K_TOAST, &t, at, false));
            }
        }
        if kids.is_empty() {
            return None;
        }
        Some(El::block().abs(0.0, 0.0, f32::NAN, f32::NAN).size(crate::ui::WIN_W, crate::ui::WIN_H).no_hit().children(kids))
    }

    fn popup_dismiss(&mut self) {
        // a click beside the menu closes it (the confirm's own dim takes the clicks beside it: "out"); Esc closes the confirm
        self.fmenu = None;
        self.close_file_dialog();
    }


    fn describe(&self) -> String {
        let s = match self.scan_of(self.drv) {
            Scan::Idle => "idle",
            Scan::Running { .. } => "running",
            Scan::Done { .. } => "done",
        };
        format!(
            "drives={} drv={} scan={s} view={:?} clean={} health={}",
            self.drives.len(),
            self.drv,
            self.view,
            if self.cleaning {
                "cleaning"
            } else if self.measuring {
                "measuring"
            } else if self.plan.is_some() {
                "measured"
            } else {
                "idle"
            },
            self.health.len()
        )
    }
}

impl Storage {
    fn click(&mut self, k: Key) {
        let n = self.drives.len();
        if let Some(i) = (0..n).find(|i| k == idx(K_DRV, *i)) {
            let l = self.drives[i].info.letter;
            if l != self.drv {
                self.drv = l;
                self.path.clear();
                self.slide(0.0);
            }
            return;
        }
        if let Some(i) = (0..2).find(|i| k == idx(K_SEG, *i)) {
            self.view = if i == 0 { View::Types } else { View::Folders };
            self.slide(0.0);
            return;
        }
        // Order 069: the Folders | Files switch, the Files rows' Show in folder, the right-click menu, the delete confirm
        if let Some(i) = (0..2).find(|i| k == idx(K_FSEG, *i)) {
            let m = if i == 0 { FMode::Folders } else { FMode::Files };
            if m != self.fmode {
                self.fmode = m;
                self.path.clear();
                self.fmenu = None;
                self.slide(0.0);
            }
            return;
        }
        if k == sub(K_FDLG, "no") || k == sub(K_FDLG, "out") {
            self.close_file_dialog();
            return;
        }
        if k == sub(K_FDLG, "go") {
            if self.fdlg_closing.is_none() {
                self.file_delete();
            }
            return;
        }
        if let Some((f, ..)) = self.fmenu.clone() {
            if let Some(i) = (0..4).find(|i| k == idx(K_FMENU, *i)) {
                self.fmenu = None;
                match i {
                    1 => self.open_in_explorer(f.path(), true),
                    2 => self.file_delete_ask(f),
                    _ => {}
                }
                return;
            }
        }
        if self.fmode == FMode::Files {
            let files = self.big_files();
            if let Some((i, f)) = files.iter().enumerate().find(|(i, _)| k == idx(K_BOP, *i)) {
                let _ = i;
                self.open_in_explorer(f.path(), true);
                return;
            }
        }
        if let Some(i) = (0..CleanKind::ALL.len()).find(|i| k == idx(K_CN, *i)) {

            let kind = CleanKind::ALL[i];
            let ok = !self.cleaning && !self.measuring && self.plan.as_ref().and_then(|p| p.row(kind)).is_some_and(|r| !r.is_empty());
            if ok && !self.ticked.remove(&kind) {
                self.ticked.insert(kind);
            }
            return;
        }
        // the Folders view: rows (go deeper), the path (go back up), Open in Explorer
        if let Some(Scan::Done { result, .. }) = self.scans.get(&self.drv) {
            let result = result.clone();
            let cur = self.path.last().copied().unwrap_or(result.tree.root());
            let rows = result.tree.rows(cur).unwrap_or_default();
            for (i, r) in rows.iter().enumerate() {
                if k == idx(K_FDR, i) {
                    if let scan::RowKind::Folder { id, has_subfolders: true, windows_own: false, .. } = r.kind {
                        if result.tree.is_pruned(id) {
                            // kept from an earlier menu without its inside: scan just this folder, then go in
                            self.scan_sub(id, result.clone());
                        } else {
                            self.path.push(id);
                            self.slide(1.0);
                        }
                    }
                    return;
                }
                if k == idx(K_FOP, i) {
                    let p = match r.kind {
                        scan::RowKind::Folder { id, .. } => result.tree.path(id).ok().map(|p| (p, false)),
                        scan::RowKind::File => result.tree.path(cur).ok().map(|p| (p.join(&r.name), true)),
                        scan::RowKind::OtherFiles { .. } => result.tree.path(cur).ok().map(|p| (p, false)),
                    };
                    if let Some((p, file)) = p {
                        self.open_in_explorer(p, file);
                    }
                    return;
                }
            }
            for i in 0..=self.path.len() {
                if k == idx(K_CRUMB, i) && i < self.path.len() {
                    self.path.truncate(i);
                    self.slide(-1.0);
                    return;
                }
            }
        }
        match k {
            _ if k == K_MEASURE || k == K_AGAIN => self.measure_drive(self.drv),
            _ if k == K_STOP => self.stop_drive(self.drv),
            _ if k == K_BACK => {
                if self.path.pop().is_some() {
                    self.slide(-1.0);
                }
            }
            _ if k == K_HADM => self.health_with_admin(),
            _ if k == K_CAGAIN => self.measure_clean(),
            _ if k == K_CLN => {
                if self.plan.is_none() {
                    self.measure_clean();
                } else {
                    self.clean();
                }
            }
            _ => {}
        }
    }

    /// The row was cleaned and nothing is left in it (its "Cleaned" shows instead of its size); while the sizes are measured
    /// again after a clean, a cleaned row waits for its new size.
    pub(super) fn done(&self, kind: CleanKind) -> bool {
        let cleaned = self.report.as_ref().is_some_and(|r| r.rows.iter().any(|c| c.kind == Some(kind)));
        cleaned && (self.measuring || self.plan.as_ref().and_then(|p| p.row(kind)).is_none_or(|r| r.is_empty()))
    }
}

/// The menu window closed (its pages go with it): the kept walks drop their folder trees down to what the page shows
/// first (the root's rows) - the owner's rule: no RAM unless you are doing something in it. Measured on a scratch tree:
/// 2,041 folders / 10,000 files = 512 KB full, 4 KB pruned (crates/storage tests/scratch_real.rs `tree_ram_on_a_scratch_tree`).
#[derive(Default)]
struct MenuGuard {
    keep: Option<crate::keep::Keep>,
}

impl Drop for MenuGuard {
    fn drop(&mut self) {
        let Some(keep) = self.keep.take() else { return };
        keep.update::<Kept>(KEEP_KEY, |k| {
            k.menu_open.store(false, Ordering::Relaxed);
            for s in k.scans.values_mut() {
                if let Scan::Done { result, .. } = s {
                    let mut r = (**result).clone();
                    r.tree = r.tree.pruned();
                    *result = Arc::new(r);
                }
            }
        });
    }
}

//! The Security tab (menu-v22 page `sec`, Order 023): a simple front for Microsoft Defender - the status card, the three
//! scans (Offline restarts the PC: it asks first), the drop zone, then the two fold cards Threats found (Remove / Allow) and
//! Quarantine (Restore / Delete). Scans start ONLY from their buttons; opening the tab reads Defender's state and nothing
//! else. Delete opens Windows Security's own Protection history (A_016_01: Windows has no command that deletes one item).
//! Wired to `bu-security` (Defender on this PC; its FAKE layer with the drawing's sample in every test copy). the owner's F2:
//! the service lives in `env.keep` for the app's whole life, so a scan goes on with the tab left or the window closed and
//! the page shows it (running, or its answer) the moment it opens again; the last read of Defender shows at once too.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::{mpsc, Arc};

use bu_security as sec;
use taffy::style::{AlignItems, JustifyContent};

use crate::anim::{Bezier, EASE};
use crate::gfx::{sh, Align, Font, Gfx, Rgba};
use crate::pages::{Env, Page};
use crate::undo::{DefaultItem, Resettable, Val};
use crate::ui::pieces::mitems::{self, Place};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, lh, sub, Cursor, El, Key, RADIUS_PILL};
use crate::ui::pieces::button::{cbtn, Kind};
use crate::ui::pieces::card::{card, card_icon};
use crate::ui::pieces::tip::{self, Rq};
use crate::ui::pieces::{self, badge, group, link, reset, toast};
use crate::ui::{cmix, ACC, ACC_S, AMBER, CTL, CTL_H, DASH, FG, FG2, FG3, GREEN, GRP, HAIR, HOV, RED, SEL, TRK, VZ1, VZ2, WIN_W};

const K_REVIEW: Key = key("sec.review");
const K_SCAN: Key = key("sec.scan");
const K_CANCEL: Key = key("sec.cancel");
const K_DZ: Key = key("sec.dz");
const K_TH: Key = key("sec.threats");
const K_Q: Key = key("sec.quar");
const K_ASK: Key = key("sec.ask");
const K_TOAST: Key = key("sec.toast");
const K_RS: Key = key("sec.reset");

/// The page's content width (600 - 2 x 26).
const W: f32 = 548.0;

// the drawing's inline SVGs (menu-v22 Security page constants, word for word)
const SHOK: &str = r#"<svg viewBox="0 0 24 24"><path d="M12 3l7 2.6v5.3c0 4.4-2.9 7.7-7 9.1-4.1-1.4-7-4.7-7-9.1V5.6z"/><path d="M8.7 11.9l2.3 2.3 4.3-4.6"/></svg>"#;
const SHWN: &str = r#"<svg viewBox="0 0 24 24"><path d="M12 3l7 2.6v5.3c0 4.4-2.9 7.7-7 9.1-4.1-1.4-7-4.7-7-9.1V5.6z"/><path d="M12 8.2v4.4"/><path d="M12 15.6h.01" stroke-width="2.2"/></svg>"#;
const IC_QUICK: &str = r#"<svg viewBox="0 0 16 16"><path d="M9 1.8L3.8 9h3.7L7 14.2 12.2 7H8.5z"/></svg>"#;
const IC_FULL: &str = r#"<svg viewBox="0 0 16 16"><rect x="2" y="3.2" width="12" height="9.6" rx="1.8"/><path d="M2 9.4h12M11.2 11.1h.01"/></svg>"#;
const IC_OFF: &str = r#"<svg viewBox="0 0 16 16"><path d="M12.9 8.6A5 5 0 1 1 11.2 4.3"/><path d="M11.5 1.8v2.8H8.7"/></svg>"#;
const DZI: &str = r#"<svg viewBox="0 0 20 20"><path d="M10 3v9M6.5 8.6L10 12.1l3.5-3.5"/><path d="M3.5 12.5v2a2 2 0 0 0 2 2h9a2 2 0 0 0 2-2v-2"/></svg>"#;
const OKI: &str = r#"<svg viewBox="0 0 20 20"><path d="M5 10.4l3.3 3.3L15.2 6.6"/></svg>"#;
const BADI: &str = r#"<svg viewBox="0 0 20 20"><path d="M10 3.2l7.2 12.6H2.8z"/><path d="M10 8.2v3.6"/><path d="M10 14.1h.01" stroke-width="2.2"/></svg>"#;

/// The three scans (menu-v22 `SCANS`): title, line, icon, admin.
const SCANS: [(&str, &str, &str, bool); 3] =
    [("Quick scan", "About 5 minutes", IC_QUICK, false), ("Full scan", "Up to an hour", IC_FULL, false), ("Offline scan", "Restarts the PC", IC_OFF, true)];

const FOLD: Bezier = Bezier::new(0.3, 0.7, 0.2, 1.0);
const POP: Bezier = Bezier::new(0.3, 1.3, 0.5, 1.0);

/// A short question in the popup list, by its button (the drawing's `askIn` / `askOffline`).
#[derive(Clone, Debug, PartialEq)]
enum Ask {
    Offline,
    Allow(sec::ThreatRow),
    Restore(sec::ThreatRow),
}

/// The drop zone (`.sdz`).
#[derive(Clone, Debug, PartialEq)]
enum Dz {
    Idle,
    /// found, and Defender already dealt with it (quarantined / removed / cleaned): the "bad" look, no choice to make
    Handled { path: String, text: String },
    Busy { path: String, since: f64 },
    Ok { path: String, text: String },
    Bad { path: String, row: sec::ThreatRow },
}

enum Msg {
    Page(Box<sec::Result<sec::SecurityPage>>),
    /// a scan ended: its id (the report waits in env.keep)
    ScanDone(u64),
    Toast(String),
    /// an action finished: its toast (or error) and the page is read again
    Did(sec::Result<String>, Option<String>),
    /// a scan could not start (another one runs, Defender is off, the path is gone)
    StartFailed(sec::SecurityError),
}

/// What the tab keeps in `env.keep` for the app's life (F2): the service (its scan thread and "is a scan running" outlive the
/// page) and, in a test copy, the fake Windows behind it.
#[derive(Clone)]
struct Held {
    svc: Arc<sec::SecurityService>,
    fake: Option<sec::FakeOs>,
}

const KEEP_SVC: &str = "sec.svc";
/// the last read of Defender (`SecurityPage`): shown at once when the tab opens, then read again
const KEEP_PAGE: &str = "sec.page";
/// the last scan's report + whether a page has shown it yet (a scan that ended with the window closed shows its answer on open)
const KEEP_REPORT: &str = "sec.report";
/// The kept report: (report, shown yet, the scan's id).
type Rep = (sec::ScanReport, bool, u64);
/// Every scan's id (its answer is shown exactly once: by the open page's channel or from `env.keep`, whichever is first).
static SCAN_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Take the kept report if no page has shown it yet (and, with `id`, only that scan's): marks it shown in the same lock.
fn claim(keep: &crate::keep::Keep, id: Option<u64>) -> Option<sec::ScanReport> {
    let mut got = None;
    keep.update::<Rep>(KEEP_REPORT, |r| {
        if !r.1 && id.is_none_or(|i| i == r.2) {
            r.1 = true;
            got = Some(r.0.clone());
        }
    });
    got
}
/// the drop zone's last answer
const KEEP_DZ: &str = "sec.dz";

/// The drawing's sample Defender (menu-v22 SEC) for a test copy: protected, last quick scan today 09:12, definitions
/// 1.421.733.0 from 06:40, nothing waiting, two files in Quarantine; the drawing's clock 6 Oct 2026 21:37; not elevated
/// (Allow / Restore / Offline say "Needs admin").
fn sample_fake() -> sec::FakeOs {
    let f = sec::FakeOs::protected();
    {
        let mut s = f.state();
        s.elevated = false;
        s.now = sec::Stamp::new(2026, 10, 6, 21, 37);
        if let Some(st) = s.status.as_mut() {
            st.definitions_version = "1.421.733.0".into();
            st.definitions_updated = Some(sec::Stamp::new(2026, 10, 6, 6, 40));
            st.quick_scan_end = Some(sec::Stamp::new(2026, 10, 6, 9, 12));
        }
    }
    let q = |id: &str, tid: i64, name: &str, sev: sec::Severity, path: &str, at: sec::Stamp| (sec::fake::threat(tid, name, sev, false), sec::fake::detection(id, tid, 3, at, path));
    for (t, d) in [
        q("{d-1}", 2147735503, "HackTool:Win32/AutoKMS", sec::Severity::High, r"C:\Users\User\Downloads\kms_activator.exe", sec::Stamp::new(2026, 10, 3, 18, 2)),
        q("{d-2}", 2147745002, "PUA:Win32/Presenoker", sec::Severity::Moderate, r"C:\Users\User\Downloads\driver_updater_setup.exe", sec::Stamp::new(2026, 9, 28, 14, 30)),
    ] {
        f.add_threat(t, d);
    }
    f
}

#[derive(Default)]
pub struct Security {
    env: Env,
    svc: Option<Arc<sec::SecurityService>>,
    /// the fake Windows of a test copy (tests and the drawing's drop rule use it)
    fake: Option<sec::FakeOs>,
    page: Option<sec::SecurityPage>,
    read_err: Option<sec::SecurityError>,
    now_stamp: Option<sec::Stamp>,
    scanning: Option<(sec::ScanKind, f64)>,
    th_open: bool,
    q_open: bool,
    ask: Option<(Ask, (f32, f32, f32, f32), f64)>,
    dz: Option<Dz>,
    /// Order 045: files from Explorer are dragged over the drop zone (`.sdz.over`)
    drag_over: bool,
    toast: Option<(String, f64)>,
    press_box: std::collections::HashMap<Key, (f32, f32, f32, f32)>,
    tx: Option<mpsc::Sender<Msg>>,
    rx: Option<mpsc::Receiver<Msg>>,
    now: f64,
    /// Order 055: the frame's time for the sweep bar and the ring (live boxes read it when they are painted; `tick` sets it
    /// every frame) - they move at the monitor's rate without the page being built again
    clock: Rc<Cell<f64>>,
    /// the whole seconds of the card's "running 0:05" line as last built (a new second is the one rebuild a scan needs)
    secs_shown: Cell<u64>,
    /// the last `tick`'s true moved only the live boxes (the sweep, the ring)
    live_only: bool,
    /// when `tick` last asked whether a scan from an earlier open has ended
    polled: f64,
    /// Order 036: a page never opened (Settings › Reset, the uninstaller) resets through a service made on first use: the
    /// sample fake in a test copy, Defender otherwise
    rs_svc: std::cell::OnceCell<Arc<sec::SecurityService>>,
}

fn row_key(base: Key, r: &sec::ThreatRow) -> Key {
    sub(sub(base, &r.threat_id.to_string()), &r.path())
}

impl Security {
    fn svc(&self) -> Option<Arc<sec::SecurityService>> {
        self.svc.clone()
    }
    fn send(&self) -> mpsc::Sender<Msg> {
        self.tx.clone().expect("open")
    }
    /// Run a blocking crate call off the UI thread (WMI takes 1-2 s on a real PC); the fake runs inline.
    fn run(&self, job: impl FnOnce() + Send + 'static) {
        if self.env.fake() {
            job();
        } else {
            // its answer repaints an idle menu (no frames run while nothing moves)
            let wake = self.env.waker();
            std::thread::spawn(move || {
                job();
                wake.wake();
            });
        }
    }
    /// Read Defender's state (reading starts no scan).
    fn read(&self) {
        let Some(svc) = self.svc() else { return };
        let tx = self.send();
        self.run(move || {
            let _ = tx.send(Msg::Page(Box::new(svc.page())));
        });
    }

    fn show_toast(&mut self, t: impl Into<String>) {
        self.toast = Some((t.into(), self.now));
    }

    fn err_toast(e: &sec::SecurityError) -> String {
        match e {
            // the admin prompt answered No (or a test copy, which never asks): nothing changed
            sec::SecurityError::NeedsAdmin => crate::admin::NOT_CHANGED.to_string(),
            sec::SecurityError::Unsupported(t) => {
                let mut c = t.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            }
            other => {
                let t = other.to_string();
                let mut c = t.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            }
        }
    }

    /// Used by `open` and `build`: the messages, the OS clock's stamp and the end of a scan.
    fn pump(&mut self) -> bool {
        let mut any = self.drain();
        if let Some(svc) = self.svc() {
            self.now_stamp = Some(svc.now());
            any |= self.scan_ended();
        }
        any
    }

    /// The messages that came (a fake answers inline and a handled message may ask again - read after an action: until quiet).
    fn drain(&mut self) -> bool {
        let mut any = false;
        for _ in 0..8 {
            if !self.pump_once() {
                break;
            }
            any = true;
        }
        any
    }

    /// A scan started by an earlier open answers that page's channel (gone): its answer is read from env.keep.
    fn scan_ended(&mut self) -> bool {
        let Some(svc) = self.svc() else { return false };
        if self.scanning.is_none() || svc.is_scanning() {
            return false;
        }
        let Some(r) = claim(&self.env.keep, None) else { return false };
        self.scan_done(r);
        // its read of Defender (inline on the fake) is waiting in the channel
        for _ in 0..8 {
            if !self.pump_once() {
                break;
            }
        }
        true
    }

    /// The whole seconds the card's "running 0:05" line shows (Quick and Full scans only).
    fn scan_secs(&self, now: f64) -> Option<u64> {
        match &self.scanning {
            Some((sec::ScanKind::Quick, since)) | Some((sec::ScanKind::Full, since)) => Some(((now - since) / 1000.0).max(0.0) as u64),
            _ => None,
        }
    }

    fn pump_once(&mut self) -> bool {
        let mut msgs = Vec::new();
        if let Some(rx) = &self.rx {
            while let Ok(m) = rx.try_recv() {
                msgs.push(m);
            }
        }
        let any = !msgs.is_empty();
        for m in msgs {
            match m {
                Msg::Page(r) => match *r {
                    Ok(p) => {
                        self.read_err = None;
                        self.env.keep.put(KEEP_PAGE, p.clone());
                        self.page = Some(p);
                    }
                    Err(e) => self.read_err = Some(e),
                },
                // this scan's answer, unless `pump` already took it from env.keep (shown once, never as another scan's)
                Msg::ScanDone(id) => {
                    if let Some(r) = claim(&self.env.keep, Some(id)) {
                        self.scan_done(r);
                    }
                }
                Msg::Toast(t) => self.show_toast(t),
                Msg::StartFailed(e) => {
                    self.scanning = None;
                    if matches!(self.dz, Some(Dz::Busy { .. })) {
                        self.dz = Some(Dz::Idle);
                    }
                    self.show_toast(Self::err_toast(&e));
                }
                Msg::Did(r, after) => {
                    match r {
                        Ok(t) => {
                            self.show_toast(t);
                            if let Some(a) = after {
                                self.dz_after(&a);
                            }
                        }
                        Err(e) => self.show_toast(Self::err_toast(&e)),
                    }
                    self.read();
                }
            }
        }
        any
    }

    // ---- scans

    fn start(&mut self, kind: sec::ScanKind) {
        let Some(svc) = self.svc() else { return };
        if self.scanning.is_some() {
            return;
        }
        if let sec::ScanKind::Path(p) = &kind {
            self.fake_path(p);
        }
        let tx = self.send();
        let k2 = kind.clone();
        // the answer is kept for the app's life (F2): with the tab left or the window closed it waits in `env.keep` (not shown
        // yet) and the page shows it on its next open; an open page gets it right away
        let keep = self.env.keep.clone();
        let wake = self.env.waker();
        let id = SCAN_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // shown as running at once; the start itself (Defender's checks: WMI, 1-2 s on a real PC) runs off the UI thread and
        // a refusal comes back as StartFailed (the end review, Order 030: it froze the window)
        self.scanning = Some((k2, self.now));
        if let sec::ScanKind::Path(p) = &kind {
            self.dz = Some(Dz::Busy { path: p.clone(), since: self.now });
        }
        let tx2 = tx.clone();
        self.run(move || {
            let r = svc.start_scan(kind, move |r| {
                keep.put::<Rep>(KEEP_REPORT, (r, false, id));
                let _ = tx.send(Msg::ScanDone(id));
                wake.wake();
            });
            if let Err(e) = r {
                let _ = tx2.send(Msg::StartFailed(e));
            }
        });
        self.fake_timer();
        self.pump();
        self.read();
    }

    /// The drop zone: scan one dropped / picked file or folder (the page API's file drop and Windows' file picker call this
    /// once they land - PIECES_WANTED; tests call it directly).
    pub fn scan_path(&mut self, path: &str) {
        if matches!(self.dz, Some(Dz::Busy { .. })) {
            return;
        }
        self.start(sec::ScanKind::Path(path.to_string()));
    }

    /// Defender on this PC (a test copy never gets here: `env.fake()` = the fake).
    fn real_service() -> sec::SecurityService {
        // Allow / Restore / Remove / the offline scan go to the app's elevated copy behind Windows' admin prompt (Order 039)
        #[cfg(windows)]
        {
            let os: Arc<dyn sec::SecurityOs> = Arc::new(sec::RealOs::new());
            sec::SecurityService::new(Arc::new(crate::admin::proxy::SecurityOs::new(os, crate::admin::client::admin())))
        }
        #[cfg(not(windows))]
        {
            sec::SecurityService::new(Arc::new(sample_fake()))
        }
    }

    /// The fake only (a test copy): the dropped path "exists", and the drawing's rule decides what the scan finds - a name
    /// with crack / keygen / unlock / hack / cheat = a threat waiting for a choice, "handled" = one Defender quarantined
    /// itself. In the app's test copy a scan ends after 9 s (the drawing's scan); unit tests end it themselves.
    fn fake_path(&self, p: &str) {
        let Some(f) = &self.fake else { return };
        let low = p.to_lowercase();
        let now = f.state().now;
        {
            let mut s = f.state();
            s.existing.insert(p.to_string());
            if low.contains("handled") {
                s.scan_adds.push((sec::fake::detection("{d-h}", 2147844002, 3, now, p), sec::fake::threat(2147844002, "Trojan:Win32/Handled.A", sec::Severity::High, false)));
            } else if ["crack", "keygen", "unlock", "hack", "cheat"].iter().any(|w| low.contains(w)) {
                s.scan_adds.push((sec::fake::detection("{d-w}", 2147844001, 1, now, p), sec::fake::threat(2147844001, "Trojan:Win32/Wacatac.B!ml", sec::Severity::Severe, true)));
            }
        }
    }

    /// The app's test copy: a fake scan ends on its own after the drawing's 9 s (unit tests release it themselves).
    fn fake_timer(&self) {
        let Some(f) = self.fake.clone() else { return };
        if !crate::testmode::on() {
            return;
        }
        std::thread::spawn(move || {
            if f.wait_scan_started(std::time::Duration::from_secs(5)) {
                std::thread::sleep(std::time::Duration::from_millis(9000));
                f.release_scan();
            }
        });
    }

    fn scan_done(&mut self, r: sec::ScanReport) {
        self.scanning = None;
        let title = r.kind.title();
        match &r.kind {
            sec::ScanKind::Path(p) => {
                let p = p.clone();
                match (&r.outcome, r.waiting().first()) {
                    (sec::ScanOutcome::Finished, Some(row)) => {
                        let row = (*row).clone();
                        self.show_toast(format!("Threat found \u{00b7} {}", row.file));
                        self.dz = Some(Dz::Bad { path: p, row });
                    }
                    (sec::ScanOutcome::Finished, None) if r.new_threats.is_empty() => self.dz = Some(Dz::Ok { path: p, text: "No threats found".into() }),
                    (sec::ScanOutcome::Finished, None) => {
                        let name = r.new_threats[0].name.clone();
                        self.show_toast(format!("Threat found \u{00b7} Defender quarantined {}", r.new_threats[0].file));
                        self.dz = Some(Dz::Handled { path: p, text: format!("Threat found: {name} \u{00b7} Defender quarantined it") });
                    }
                    (sec::ScanOutcome::Cancelled, _) => self.dz = Some(Dz::Idle),
                    (sec::ScanOutcome::Failed(e), _) => {
                        self.show_toast(Self::err_toast(e));
                        self.dz = Some(Dz::Idle);
                    }
                }
            }
            _ => match &r.outcome {
                sec::ScanOutcome::Finished if r.is_clean() => self.show_toast(format!("{title} done \u{00b7} no threats found")),
                sec::ScanOutcome::Finished => {
                    let n = r.waiting().len();
                    self.show_toast(if n > 0 { format!("{title} done \u{00b7} {n} found") } else { format!("{title} done \u{00b7} Defender dealt with what it found") });
                }
                sec::ScanOutcome::Cancelled => self.show_toast("Scan stopped"),
                sec::ScanOutcome::Failed(e) => self.show_toast(Self::err_toast(e)),
            },
        }
        self.read();
    }

    fn cancel(&mut self) {
        if let Some(svc) = self.svc() {
            let _ = svc.cancel_scan();
        }
        self.pump();
    }

    // ---- threats

    fn remove(&mut self, r: &sec::ThreatRow) {
        let Some(svc) = self.svc() else { return };
        let tx = self.send();
        let id = r.threat_id;
        let path = r.path();
        self.run(move || {
            let res = svc.remove_threat(id).map(|c| c.toast());
            let _ = tx.send(Msg::Did(res, Some(format!("removed|{path}"))));
        });
        self.pump();
    }

    fn allow(&mut self, r: &sec::ThreatRow) {
        let Some(svc) = self.svc() else { return };
        let tx = self.send();
        let id = r.threat_id;
        let path = r.path();
        let (file, log) = (r.file.clone(), can_log());
        self.run(move || {
            let res = svc.allow_threat(id).map(|c| c.toast());
            // Order 036: a waiting threat was not allowed before
            if log && res.is_ok() {
                crate::undo::note("sec", &allow_item(id), &allow_label(&file), &allow_val(false), &allow_val(true));
            }
            let _ = tx.send(Msg::Did(res, Some(format!("allowed|{path}"))));
        });
        self.pump();
    }

    fn restore(&mut self, r: &sec::ThreatRow) {
        let Some(svc) = self.svc() else { return };
        let tx = self.send();
        let id = r.threat_id;
        let path = r.path();
        let (file, log) = (r.file.clone(), can_log());
        self.run(move || {
            // Order 036: Restore also allows the threat (the restored file itself is a one-time action: not logged)
            let was = svc.os().allowed_threat_ids().map(|ids| ids.contains(&id)).unwrap_or(false);
            // the Allow and the file in ONE admin prompt
            let _admin = crate::admin::client::admin().scope(crate::admin::Purpose::Security);
            let res = svc.restore_quarantined(id, &path);
            if log && res.is_ok() && !was {
                crate::undo::note("sec", &allow_item(id), &allow_label(&file), &allow_val(false), &allow_val(true));
            }
            let _ = tx.send(Msg::Did(res, None));
        });
        self.pump();
    }

    /// A_016_01: Quarantine's Delete opens Windows Security's Protection history (Windows' own Remove is there).
    fn delete(&mut self) {
        let Some(svc) = self.svc() else { return };
        let tx = self.send();
        self.run(move || {
            if let Err(e) = svc.open_protection_history() {
                let _ = tx.send(Msg::Toast(Self::err_toast(&e)));
            }
        });
        self.pump();
    }

    fn offline(&mut self) {
        let Some(svc) = self.svc() else { return };
        let tx = self.send();
        self.run(move || {
            let res = svc.offline_scan(true).map(|_| "Your PC restarts in a moment \u{00b7} Defender scans before Windows starts".to_string());
            let _ = tx.send(Msg::Did(res, None));
        });
        self.pump();
    }

    /// The drop zone's own threat was handled from the list or from the zone: say so in its place (the drawing's dzAfter).
    fn dz_after(&mut self, what: &str) {
        let Some((kind, path)) = what.split_once('|') else { return };
        if let Some(Dz::Bad { path: p, .. }) = &self.dz {
            if p.eq_ignore_ascii_case(path) {
                let text = if kind == "removed" { "Removed \u{00b7} it waits in Quarantine if you need it back" } else { "Allowed \u{00b7} Defender leaves it alone now" };
                self.dz = Some(Dz::Ok { path: p.clone(), text: text.into() });
            }
        }
    }

    fn threats(&self) -> Vec<sec::ThreatRow> {
        self.page.as_ref().map(|p| p.threats.clone()).unwrap_or_default()
    }
    fn quarantine(&self) -> Vec<sec::ThreatRow> {
        self.page.as_ref().map(|p| p.quarantine.clone()).unwrap_or_default()
    }

    // ------------------------------------------------------------------------------------------------- building

    /// `.sst`: the status card's top (shield, three lines, Review).
    fn status(&self, cx: &mut Cx) -> El {
        let now = self.now_stamp.unwrap_or(sec::Stamp::new(2026, 1, 1, 0, 0));
        let n = self.threats().len();
        let busy = self.scanning.is_some();
        let p = self.page.as_ref();
        let defs = p.map(|p| {
            let up = p.status.definitions_updated.map(|s| if s.label(&now).starts_with("Today") { format!("updated today {}", s.short(&now)) } else { format!("updated {}", s.label(&now)) });
            match up {
                Some(u) => format!("Definitions: {} \u{00b7} {}", p.status.definitions_version, u),
                None => format!("Definitions: {}", p.status.definitions_version),
            }
        });
        let last = p.and_then(|p| p.last_scan.clone()).map(|(s, k)| {
            let clean = self.threats().is_empty() && self.page.as_ref().map(|p| p.unreadable.is_none()).unwrap_or(false);
            format!("Last scan: {} \u{00b7} {}{}", s.label(&now), k.title(), if clean { " \u{00b7} no threats" } else { "" })
        });
        let last = last.unwrap_or_else(|| "Last scan: never".into());
        // the banner (bu-security `Banner`); the three drawn ones + the crate's three undrawn ones (page's wording)
        let banner = p.map(|p| p.banner.clone());
        let (warn, t, s1, s2) = if let Some(e) = &self.read_err {
            (true, "Can\u{2019}t read Defender".to_string(), Self::err_toast(e), String::new())
        } else if let Some((k, _)) = &self.scanning {
            (false, format!("{} running", k.title()), "You can keep using your PC".to_string(), last.clone())
        } else {
            match banner {
                Some(sec::Banner::NeedsAttention { .. }) if n > 0 => (
                    true,
                    "Needs attention".to_string(),
                    if n == 1 { "1 threat found \u{00b7} Remove or Allow it below".to_string() } else { format!("{n} threats found \u{00b7} Remove or Allow them below") },
                    defs.clone().unwrap_or_default(),
                ),
                Some(sec::Banner::OtherAntivirus { name }) => (false, format!("{name} protects this PC"), "Defender is standing by".to_string(), last.clone()),
                Some(sec::Banner::ProtectionOff) => (true, "Protection is off".to_string(), "Real-time protection is off in Windows Security".to_string(), last.clone()),
                Some(sec::Banner::CannotRead) => {
                    let why = self.page.as_ref().and_then(|p| p.unreadable.as_ref()).map(Self::err_toast).unwrap_or_default();
                    (true, "Can\u{2019}t read Defender\u{2019}s list".to_string(), why, String::new())
                }
                Some(sec::Banner::Scanning { title }) => (false, format!("{title} running"), "You can keep using your PC".to_string(), last.clone()),
                None => (false, String::new(), String::new(), String::new()),
                _ => (false, "You\u{2019}re protected".to_string(), last.clone(), defs.clone().unwrap_or_default()),
            }
        };
        // `.ssi{width:46px;height:46px;border-radius:50%;background:rgba(48,209,88,.15);box-shadow:inset 0 0 0 1px rgba(48,209,88,.35)}`
        // `.ssi svg{width:24px;height:24px;stroke:var(--green);stroke-width:1.6}` `.sst.warn .ssi{background:rgba(255,69,58,.14);
        // box-shadow:inset 0 0 0 1px rgba(255,69,58,.38)}` `svg{stroke:var(--red)}` `.sst.busy .ssi{background:var(--sel);
        // box-shadow:inset 0 0 0 1px var(--acc-s)}` `svg{stroke:var(--acc)}`
        let (bg, ring, col) = if busy {
            (SEL(), ACC_S(), ACC())
        } else if warn {
            (Rgba::rgba(255, 69, 58, 0.14), Rgba::rgba(255, 69, 58, 0.38), RED())
        } else {
            (Rgba::rgba(48, 209, 88, 0.15), Rgba::rgba(48, 209, 88, 0.35), GREEN())
        };
        let ssi = El::block()
            .size(46.0, 46.0)
            .none()
            .radius(RADIUS_PILL)
            .bg(bg)
            .inset(&[sh(0.0, 0.0, 0.0, 1.0, ring)])
            .place_center()
            .child(El::icon_svg(if warn && !busy { SHWN } else { SHOK }, 24.0, 1.6, col));
        // `.sst .lbl b{display:block;font:600 15px/20px "Segoe UI Variable Display";letter-spacing:-.01em}`
        // `.lbl small{display:block;font-size:11px;color:var(--fg2);margin-top:1px}` `.sst .lbl small{font-variant-numeric:tabular-nums}`
        let sm = |t: &str| El::text(t, Font::new(11.0, 400).tnum(), FG2(), lh(11.0, 1.35)).ellipsis().margin(1.0, 0.0, 0.0, 0.0);
        let mut lbl = El::col().flex1().child(El::text(t, Font::display(15.0, 600).ls(-150), FG(), 20.0).ellipsis()).child(sm(&s1));
        if !s2.is_empty() {
            lbl = lbl.child(sm(&s2));
        }
        let mut ctl = group::ctl(vec![]);
        if n > 0 && !busy {
            ctl = group::ctl(vec![cbtn(cx, K_REVIEW, "Review", Kind::Ghost, true, false, 0.0)]);
        }
        // `.sst{display:flex;align-items:center;gap:14px;padding:14px}`
        El::row().center().gap(14.0).pad_all(14.0).child(ssi).child(lbl).child(ctl)
    }

    /// `.scg3` with the three `.scb` buttons.
    fn scans(&self, cx: &mut Cx) -> El {
        let mut kids = Vec::new();
        for (i, (t, s, ic, adm)) in SCANS.iter().enumerate() {
            let k = idx(K_SCAN, i);
            let now_one = matches!((&self.scanning, i), (Some((sec::ScanKind::Quick, _)), 0) | (Some((sec::ScanKind::Full, _)), 1));
            let disabled = self.scanning.is_some() && !now_one;
            let hv = if disabled { 0.0 } else { cx.hover_t(k, 150.0, EASE) };
            let pr = if disabled { 0.0 } else { cx.active_t(k, 120.0, EASE) };
            let op = cx.tr(k, 3, if disabled { 0.4 } else { 1.0 }, 200.0, EASE);
            // `.scb b{display:flex;align-items:center;gap:3px;font-size:12.5px;font-weight:600;white-space:nowrap}`
            // `.scb b .rq{width:14px;height:14px}` `.scb b .rq svg{width:11px;height:11px}` (`.rq` colour --fg3, stroke 1.4)
            let mut b = El::row().center().gap(3.0).child(El::text(*t, crate::ui::pieces::btn_font(12.5, 600), FG(), lh(12.5, 1.35)).none());
            if *adm {
                // `c.adm?tipIc('adm',TIP.adm)` - the shared tip icon (its hover + the "Needs admin" tip)
                b = b.child(tip::rq(cx, sub(k, "rq"), Rq::Adm, 14.0, tip::texts::ADM, false));
            }
            // `.scb small{display:block;font-size:10.5px;color:var(--fg2);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}`
            let small = El::text(*s, crate::ui::pieces::btn_font(10.5, 400), FG2(), lh(10.5, 1.35)).ellipsis();
            // `#sw .scb{display:flex;align-items:center;gap:10px;height:52px;min-width:0;padding:0 10px;border-radius:9px;
            //   background:var(--ctl);box-shadow:inset 0 0 0 .5px var(--hair)}` `:hover{background:var(--ctl-h)}` `:active{scale(.97)}`
            //   `:disabled{opacity:.4}` `.now{background:var(--sel);box-shadow:inset 0 0 0 1px var(--acc-s)}`
            // `.scb>i{width:28px;height:28px;border-radius:7px;background:var(--sel)}` `.scb>i svg{15px;stroke:var(--acc);stroke-width:1.6}`
            let (bg, ins) = if now_one { (SEL(), sh(0.0, 0.0, 0.0, 1.0, ACC_S())) } else { (cmix(CTL(), CTL_H(), hv), sh(0.0, 0.0, 0.0, 0.5, HAIR())) };
            let mut btn = El::row()
                .center()
                .gap(10.0)
                .h(52.0)
                .min_w(0.0)
                .pad(0.0, 10.0, 0.0, 10.0)
                .radius(9.0)
                .bg(bg)
                .inset(&[ins])
                .opacity(op)
                .scale(1.0 - 0.03 * pr)
                .child(El::block().size(28.0, 28.0).none().radius(7.0).bg(SEL()).place_center().child(El::icon_svg(ic, 15.0, 1.6, ACC())))
                .child(El::col().min_w(0.0).child(b).child(small));
            btn = if disabled { btn.key(k) } else { btn.on_click(k).cursor(Cursor::Hand) };
            kids.push(btn);
        }
        // `.scg3{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:8px;padding:10px;box-shadow:inset 0 1px 0 var(--hair)}`
        El::grid().cols(3).gap(8.0).pad_all(10.0).inset(&[sh(0.0, 1.0, 0.0, 0.0, HAIR())]).children(kids)
    }

    /// The scan progress that grows out under the buttons while a scan runs (`.xp` > `.sprg`).
    fn progress(&self, cx: &mut Cx) -> El {
        let open = matches!(&self.scanning, Some((sec::ScanKind::Quick, _)) | Some((sec::ScanKind::Full, _)));
        let (title, since) = match &self.scanning {
            Some((k, t)) => (k.title(), *t),
            None => (String::new(), self.now),
        };
        let secs = ((self.now - since) / 1000.0).max(0.0) as u64;
        // bu-security reports only the elapsed time (Defender prints no progress): the bar glides, the line says how long
        let em = format!("running {}:{:02}", secs / 60, secs % 60);
        if open {
            self.secs_shown.set(secs);
        }
        // Order 055: no `st.busy` (that built the whole page every frame for the whole scan): the glide is a live box that
        // reads the frame's time when it is painted
        let clock = self.clock.clone();
        // `.sbar{position:relative;height:4px;margin:7px 0 6px;border-radius:2px;background:var(--trk);overflow:hidden}`
        // `.sbar i{background:linear-gradient(90deg,var(--vz1),var(--vz2))}` (an indeterminate glide, like `.updbar.ind`)
        let bar = El::block().h(4.0).margin(7.0, 0.0, 6.0, 0.0).radius(2.0).bg(TRK()).clip().child(
            El::paint(move |gx: &Gfx, (x, y, w, h)| {
                let t = (((clock.get() % 1100.0) / 1100.0) as f32).clamp(0.0, 1.0);
                let g = Bezier::new(0.45, 0.0, 0.55, 1.0).ease(t as f64) as f32;
                let pw = w * 0.38;
                let px = x + pw * (-1.0 + 3.5 * g);
                gx.fill_rr_shader(px, y, pw, h, 2.0, &gx.hgrad(px, 0.0, px + pw, 0.0, &[(0.0, VZ1()), (1.0, VZ2())]), 1.0);
            })
            .abs(0.0, 0.0, 0.0, 0.0)
            .live(),
        );
        // `.sprg{padding:2px 14px 12px}` `.sprh{display:flex;align-items:baseline;gap:8px;font-size:12px}` `.sprh b{font-weight:600}`
        // `.sprh em{font-style:normal;color:var(--fg2);font-variant-numeric:tabular-nums}` `.sprh .lnk{margin-left:auto}`
        let head = El::row()
            .items(AlignItems::BASELINE)
            .gap(8.0)
            .child(El::text(title, Font::new(12.0, 600), FG(), lh(12.0, 1.35)).none())
            .child(El::text(em, Font::new(12.0, 400).tnum(), FG2(), lh(12.0, 1.35)).none())
            .child(link::link(cx, K_CANCEL, "Cancel", 12.0).ml_auto());
        let body = El::col().pad(2.0, 14.0, 12.0, 14.0).child(head).child(bar);
        let ft = crate::ui::pieces::card::fold_t(cx, sub(K_SCAN, "xp"), open);
        crate::ui::pieces::card::drop_out(cx, body, W, ft)
    }

    /// `.sdz`: the drop zone (idle / scanning / its answer).
    fn drop_zone(&self, cx: &mut Cx) -> El {
        let st = self.dz.clone().unwrap_or(Dz::Idle);
        let hv = cx.hover_t(K_DZ, 200.0, EASE);
        // Order 045: a file dragged over it from Explorer (`dragenter`: `if(dzSt==='idle'||dzSt==='ok'||dzSt==='bad')
        // dz.classList.add('over')`): `.sdz.over{background:var(--sel);transform:scale(1.01)}` (transitions .2s ease /
        // .2s cubic-bezier(.3,1.3,.5,1)), `.sdz.over::after{border-color:var(--acc);border-style:solid}` (.2s ease),
        // `.sdz.over .sdzi{transform:translateY(-3px) scale(1.06)}` (.3s cubic-bezier(.3,1.3,.5,1))
        let over = if self.drag_over && !matches!(st, Dz::Busy { .. }) { 1.0 } else { 0.0 };
        let bounce = crate::anim::Bezier::new(0.3, 1.3, 0.5, 1.0);
        let ov = cx.tr(K_DZ, 10, over, 200.0, EASE);
        let ov_s = cx.tr(K_DZ, 11, over, 200.0, bounce);
        let ov_i = cx.tr(K_DZ, 12, over, 300.0, bounce);
        let inner = match &st {
            Dz::Idle => {
                // `.sdzc{display:flex;flex-direction:column;align-items:center;gap:3px;text-align:center}` `.sdzi{width:40px;height:40px;
                // margin-bottom:5px;border-radius:10px;background:var(--sel)}` `svg{22px;stroke:var(--acc);stroke-width:1.6}`
                // `.sdzc b{font-size:13.5px;font-weight:600}` `.sdzc small{font-size:12px;color:var(--fg2)}` `u{color:var(--acc)}`
                El::col()
                    .items(AlignItems::CENTER)
                    .gap(3.0)
                    .no_hit()
                    .child(
                        El::block()
                            .size(40.0, 40.0)
                            .none()
                            .margin(0.0, 0.0, 5.0, 0.0)
                            .radius(10.0)
                            .bg(SEL())
                            .translate(0.0, -3.0 * ov_i)
                            .scale(1.0 + 0.06 * ov_i)
                            .place_center()
                            .child(El::icon_svg(DZI, 22.0, 1.6, ACC())),
                    )
                    .child(El::text("Drop a file or folder here to scan", Font::new(13.5, 600), FG(), lh(13.5, 1.35)))
                    .child(
                        El::row()
                            .child(El::text("or ", Font::new(12.0, 400), FG2(), lh(12.0, 1.35)))
                            .child(El::text("click to pick one", Font::new(12.0, 400), ACC(), lh(12.0, 1.35))),
                    )
            }
            Dz::Busy { path, since } => {
                let (dir, name) = split(path);
                // `.sdzr .ring{width:44px;height:44px}` `circle{stroke-width:3}` `.rb{stroke:var(--trk)}` `.rf{stroke:var(--acc)}`: no
                // progress from Defender for one file - the arc turns (the drawing fills it with a made-up %)
                // Order 055: a live box, the turn is read from the frame's time when it is painted (no `st.busy`)
                let (clock, since) = (self.clock.clone(), *since);
                let ring = El::paint(move |g: &Gfx, (x, y, w, _)| {
                    let deg = (((clock.get() - since) % 1100.0) / 1100.0 * 360.0) as f32;
                    let r = skia_safe::Rect::new(x + 3.0 + 1.5, y + 3.0 + 1.5, x + w - 4.5, y + w - 4.5);
                    g.stroke_oval(r, 3.0, TRK(), 1.0);
                    let mut b = skia_safe::PathBuilder::new();
                    b.add_arc(r, -90.0 + deg, 100.0);
                    g.stroke_geom(&b.detach(), 3.0, ACC());
                })
                .size(44.0, 44.0)
                .none()
                .live();
                sdzr(ring, &name, &dir, "Scanning\u{2026}", FG2(), 500, vec![])
            }
            Dz::Ok { path, text } => {
                let (dir, name) = split(path);
                let again = cbtn(cx, sub(K_DZ, "again"), "Scan another", Kind::Ghost, true, false, 0.0);
                sdzr(sri2(false), &name, &dir, text, GREEN(), 600, vec![again])
            }
            Dz::Handled { path, text } => {
                let (dir, name) = split(path);
                let again = cbtn(cx, sub(K_DZ, "again"), "Scan another", Kind::Ghost, true, false, 0.0);
                sdzr(sri2(true), &name, &dir, text, RED(), 600, vec![again])
            }
            Dz::Bad { path, row } => {
                let (dir, name) = split(path);
                let rm = cbtn(cx, sub(K_DZ, "rm"), "Remove", Kind::RedText, true, false, 0.0);
                let al = cbtn(cx, sub(K_DZ, "al"), "Allow", Kind::Ghost, true, false, 0.0);
                sdzr(sri2(true), &name, &dir, &format!("Threat found: {}", row.name), RED(), 600, vec![rm, al])
            }
        };
        // `.sdz::after{border:1.5px dashed var(--dash)}` `.sdz:hover::after{border-color:var(--acc-s)}` `.sdz.res::after{border-style:solid;
        // border-color:var(--hair)}` `.sdz.res.bad::after{rgba(255,69,58,.45)}` `.sdz.res.ok::after{rgba(48,209,88,.4)}`
        let (dashed, bc) = match &st {
            Dz::Idle => (true, cmix(DASH(), ACC_S(), hv)),
            Dz::Busy { .. } => (false, HAIR()),
            Dz::Ok { .. } => (false, Rgba::rgba(48, 209, 88, 0.4)),
            Dz::Bad { .. } | Dz::Handled { .. } => (false, Rgba::rgba(255, 69, 58, 0.45)),
        };
        let (dashed, bc) = (dashed && over == 0.0, cmix(bc, ACC(), ov));
        let border = El::paint(move |g: &Gfx, (x, y, w, h)| {
            if dashed {
                dashed_rr(g, x, y, w, h, 12.0, 1.5, bc);
            } else {
                g.stroke_rr(x + 0.75, y + 0.75, w - 1.5, h - 1.5, 11.25, 1.5, bc);
            }
        })
        .abs(0.0, 0.0, 0.0, 0.0)
        .no_hit();
        // `.sdz{position:relative;display:flex;align-items:center;justify-content:center;height:132px;margin-top:12px;border-radius:12px;
        //   background:var(--grp);cursor:pointer}` `.sdz.res{cursor:default}`
        let mut z = El::row()
            .center()
            .justify(JustifyContent::CENTER)
            .h(132.0)
            .margin(12.0, 0.0, 0.0, 0.0)
            .radius(12.0)
            .bg(cmix(GRP(), SEL(), ov))
            .scale(1.0 + 0.01 * ov_s)
            .child(inner)
            .child(border);
        z = if matches!(st, Dz::Idle) { z.on_click(K_DZ).cursor(Cursor::Hand) } else { z.key(K_DZ) };
        z
    }

    /// The two fold cards' small line + count pill: ((Threats found), (Quarantine)). When Defender's lists could not be read the
    /// page knows neither: "Can't be read right now" and a "—" pill (None), never "Empty" / "0". Before the first read: "Reading…"
    /// (a first read that failed: "Can't be read right now").
    fn card_facts(&self, n: usize, m: usize) -> ((String, Option<usize>), (String, Option<usize>)) {
        let unknown = match &self.page {
            None if self.read_err.is_some() => Some("Can\u{2019}t be read right now"),
            None => Some("Reading\u{2026}"),
            Some(p) if p.unreadable.is_some() => Some("Can\u{2019}t be read right now"),
            Some(_) => None,
        };
        if let Some(s) = unknown {
            return ((s.to_string(), None), (s.to_string(), None));
        }
        let th = if n == 0 { "None \u{00b7} every scan came back clean".to_string() } else if n == 1 { "1 needs your choice".to_string() } else { format!("{n} need your choice") };
        let q = if m == 0 { "Empty".to_string() } else { "Removed threats, locked away \u{00b7} restore or delete".to_string() };
        ((th, Some(n)), (q, Some(m)))
    }

    /// A fold card (`.grp.card.secc`): the head row looks clickable at rest (icon, title, line, count pill, chevron button).
    /// `count` None = not known: the pill shows "—".
    #[allow(clippy::too_many_arguments)]
    fn fold_card(&self, cx: &mut Cx, k: Key, icon: &str, title: &str, adm: bool, small: &str, count: Option<usize>, red: bool, open: bool, body: El) -> El {
        let fk = sub(k, "fold");
        let hv = cx.hover_t(fk, 150.0, EASE);
        let xh = cx.hover_t(sub(k, "cx"), 150.0, EASE);
        let rot = cx.tr(k, 12, if open { 1.0 } else { 0.0 }, 300.0, FOLD);
        // `.card.secc.bad .ci{background:rgba(255,69,58,.16);box-shadow:inset 0 0 0 .5px rgba(255,69,58,.4)}` `svg{stroke:#ff6b61}`
        let ci = if red {
            El::block()
                .size(32.0, 32.0)
                .none()
                .radius(8.0)
                .bg(Rgba::rgba(255, 69, 58, 0.16))
                .inset(&[sh(0.0, 0.0, 0.0, 0.5, Rgba::rgba(255, 69, 58, 0.4))])
                .place_center()
                .child(El::icon(icon, 18.0, 1.5, Rgba::hex(0xff6b61)))
        } else {
            card_icon(icon)
        };
        // `.card.secc .ct{display:flex;align-items:center;gap:6px}` + `.rq` (18 x 18, svg 12, --fg3) for the admin shield
        let mut ct = El::row().center().gap(6.0).child(El::text(title, Font::new(13.0, 600), FG(), lh(13.0, 1.35)).none());
        if adm {
            // `tipIc('adm','Remove, Allow, Restore and Delete need admin — Windows asks once')`
            ct = ct.child(tip::rq(cx, sub(k, "rq"), Rq::Adm, 18.0, "Remove, Allow, Restore and Delete need admin \u{2014} Windows asks once", false));
        }
        let lbl = El::row()
            .center()
            .gap(12.0)
            .flex1()
            .child(ci)
            .child(El::col().min_w(0.0).child(ct).child(El::text(small, Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).ellipsis().margin(1.0, 0.0, 0.0, 0.0)));
        // `.fcnt{min-width:22px;height:20px;padding:0 7px;border-radius:10px;background:var(--ctl);box-shadow:inset 0 0 0 .5px var(--hair);
        //   color:var(--fg2);font-size:11.5px;font-weight:600;line-height:20px;text-align:center}` `.fcnt.red{background:rgba(255,69,58,.18);
        //   color:#ff6b61;box-shadow:inset 0 0 0 .5px rgba(255,69,58,.4)}`
        let (cb, cc, cs) = if red {
            (Rgba::rgba(255, 69, 58, 0.18), Rgba::hex(0xff6b61), Rgba::rgba(255, 69, 58, 0.4))
        } else {
            (CTL(), FG2(), HAIR())
        };
        let cnt = El::row()
            .center()
            .justify(JustifyContent::CENTER)
            .min_w(22.0)
            .h(20.0)
            .none()
            .pad(0.0, 7.0, 0.0, 7.0)
            .radius(10.0)
            .bg(cb)
            .inset(&[sh(0.0, 0.0, 0.0, 0.5, cs)])
            .child(El::text(count.map_or("\u{2014}".to_string(), |c| c.to_string()), Font::new(11.5, 600), cc, 20.0).align(Align::Center));
        // `.cx{width:28px;height:28px;margin:0 -6px 0 2px;border-radius:6px;color:var(--fg2)}` `.cx:hover{background:var(--ctl-h);color:var(--fg)}`
        // `.cx svg{width:12px;height:12px;stroke-width:1.5;transition:transform .3s cubic-bezier(.3,.7,.2,1)}` `.cx.open svg{rotate(180deg)}`
        let cxb = El::block()
            .size(28.0, 28.0)
            .none()
            .margin(0.0, -6.0, 0.0, 2.0)
            .radius(6.0)
            .bg(CTL_H().mul_a(xh))
            .place_center()
            .key(sub(k, "cx"))
            .child(El::icon("chevDw", 12.0, 1.5, FG()).rotate(180.0 * rot).no_hit());
        // `.row.ch.ex.first` with `.card.secc .ch{min-height:58px}` `.ch{padding:10px 12px}` `.ch.ex{cursor:pointer}` `:hover{background:var(--hov)}`
        let head = El::row()
            .center()
            .gap(12.0)
            .min_h(58.0)
            .pad(10.0, 12.0, 10.0, 12.0)
            .bg(HOV().mul_a(hv))
            .on_click(fk)
            .cursor(Cursor::Hand)
            .child(lbl)
            .child(group::ctl(vec![cnt, cxb]));
        card(cx, k, head, Some(body), open, W)
    }

    /// One row of Threats found / Quarantine (`.row.thr` / `.row.thr.q`).
    fn threat_row(&self, cx: &mut Cx, r: &sec::ThreatRow, q: bool, now: &sec::Stamp) -> El {
        let rk = row_key(if q { K_Q } else { K_TH }, r);
        // `.thi{width:26px;height:26px;border-radius:7px;background:rgba(255,69,58,.14)}` `svg{14px;stroke:var(--red);stroke-width:1.6}`
        // `.thr.q .thi{background:var(--ctl)}` `svg{stroke:var(--fg2)}`
        let thi = if q {
            El::block().size(26.0, 26.0).none().radius(7.0).bg(CTL()).place_center().child(El::icon("lock", 14.0, 1.6, FG2()))
        } else {
            El::block().size(26.0, 26.0).none().radius(7.0).bg(Rgba::rgba(255, 69, 58, 0.14)).place_center().child(El::icon_svg(BADI, 14.0, 1.6, RED()))
        };
        // `.sev{height:16px;padding:0 6px;border-radius:5px;font-size:10px;font-weight:600;line-height:16px;background:rgba(255,69,58,.16);
        //   color:var(--red)}` `.sev.mid{background:rgba(255,214,10,.16);color:var(--amber)}`
        let mid = !r.severity.is_red();
        let sev = El::row()
            .center()
            .h(16.0)
            .none()
            .pad(0.0, 6.0, 0.0, 6.0)
            .radius(5.0)
            .bg(if mid { Rgba::rgba(255, 214, 10, 0.16) } else { Rgba::rgba(255, 69, 58, 0.16) })
            .child(El::text(r.severity.label(), Font::new(10.0, 600), if mid { mid_col() } else { RED() }, 16.0));
        // `.thr .snm .ttl{display:flex;align-items:center;gap:6px;min-width:0}` + `.tti` (13 px, ellipsis, padding-bottom 2 / margin -2)
        let ttl = El::row()
            .center()
            .gap(6.0)
            .min_w(0.0)
            // `h('span',{class:'tti',text:t.name,title:t.name})`
            .child(
                El::text(r.name.clone(), Font::new(13.0, 400), FG(), lh(13.0, 1.35))
                    .ellipsis()
                    .shrink(1.0)
                    .pad(0.0, 0.0, 2.0, 0.0)
                    .margin(0.0, 0.0, -2.0, 0.0)
                    .key(sub(rk, "name"))
                    .title(&r.name),
            )
            .child(sev);
        let when = if q { r.changed.or(r.found) } else { r.found };
        let line = format!(
            "{} \u{00b7} {} \u{00b7} {}{}",
            r.file,
            r.folder_name(),
            if q { "quarantined " } else { "found " },
            when.map(|w| w.short(now)).unwrap_or_default()
        );
        // the small line: `title:t.dir+'\\'+t.file`
        let snm = El::col()
            .min_w(0.0)
            .child(ttl)
            .child(El::text(line, Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).ellipsis().margin(1.0, 0.0, 0.0, 0.0).key(sub(rk, "path")).title(&r.path()));
        // `.thr .lbl.ap{gap:11px}` `.thr{gap:10px;min-height:52px}` `.card.secc .xin .row{padding-left:12px}` `.thr .ctl{gap:6px}`
        let lbl = El::row().center().gap(11.0).flex1().child(thi).child(snm);
        let btns = if q {
            vec![
                cbtn(cx, sub(rk, "restore"), "Restore", Kind::Ghost, true, false, 0.0),
                cbtn(cx, sub(rk, "delete"), "Delete", Kind::RedText, true, false, 0.0),
            ]
        } else {
            vec![
                cbtn(cx, sub(rk, "remove"), "Remove", Kind::RedText, true, false, 0.0),
                cbtn(cx, sub(rk, "allow"), "Allow", Kind::Ghost, true, false, 0.0),
            ]
        };
        // the first row also has its hairline (`.card.secc .fcin>.row.first::before{display:block}`) at left 12
        El::row()
            .center()
            .gap(10.0)
            .min_h(52.0)
            .pad(7.0, 12.0, 7.0, 12.0)
            .child(El::block().abs(12.0, 0.0, 0.0, f32::NAN).h(1.0).bg(HAIR()).no_hit())
            .child(lbl)
            .child(El::row().none().center().gap(6.0).children(btns))
    }

    fn rows_body(&self, cx: &mut Cx, rows: &[sec::ThreatRow], q: bool) -> El {
        let now = self.now_stamp.unwrap_or(sec::Stamp::new(2026, 1, 1, 0, 0));
        if rows.is_empty() {
            // `.card.secc .thnone{position:relative;padding:11px 14px 12px 56px}` + hairline at left 12; `.thnone{font-size:12px;color:var(--fg2)}`
            let t = if self.page.as_ref().map(|p| p.unreadable.is_some()).unwrap_or(false) { "Can\u{2019}t be read right now." } else if q { "Empty." } else { "Nothing found." };
            return El::block()
                .pad(11.0, 14.0, 12.0, 56.0)
                .child(El::block().abs(12.0, 0.0, 0.0, f32::NAN).h(1.0).bg(HAIR()).no_hit())
                .child(El::text(t, Font::new(12.0, 400), FG2(), lh(12.0, 1.35)));
        }
        El::block().children(rows.iter().map(|r| self.threat_row(cx, r, q, &now)).collect::<Vec<_>>())
    }

    fn ask_el(&self, cx: &mut Cx) -> Option<El> {
        let (a, bx, _) = self.ask.clone()?;
        let (t, p, go) = match &a {
            Ask::Offline => (
                "Restart and scan?".to_string(),
                "Your PC restarts and Defender scans it before Windows starts (about 15 minutes). Save your work first.".to_string(),
                "Restart and scan",
            ),
            Ask::Allow(r) => (format!("Allow {}?", r.file), "Defender stops warning about it on this PC. Only if you trust where it came from.".to_string(), "Allow"),
            Ask::Restore(r) => (format!("Restore {}?", r.file), format!("It goes back to {} and Defender allows it from now on.", r.folder_name()), "Restore"),
        };
        Some(mitems::confirm(cx, K_ASK, &t, &p, "Cancel", go, Kind::Primary, Place::Under(bx.0, bx.1, bx.2, bx.3), 262.0))
    }
}

fn split(path: &str) -> (String, String) {
    match path.trim_end_matches('\\').rsplit_once('\\') {
        Some((d, n)) => (d.to_string(), n.to_string()),
        None => (String::new(), path.to_string()),
    }
}

/// `.sri2{width:44px;height:44px;border-radius:50%;background:rgba(48,209,88,.15);box-shadow:inset 0 0 0 1px rgba(48,209,88,.35)}`
/// `svg{22px;stroke:var(--green);stroke-width:1.8}` `.sdzr.bad .sri2{background:rgba(255,69,58,.14);box-shadow:inset 0 0 0 1px
/// rgba(255,69,58,.38)}` `svg{stroke:var(--red)}`
fn sri2(bad: bool) -> El {
    let (bg, ring, c, ic) = if bad {
        (Rgba::rgba(255, 69, 58, 0.14), Rgba::rgba(255, 69, 58, 0.38), RED(), BADI)
    } else {
        (Rgba::rgba(48, 209, 88, 0.15), Rgba::rgba(48, 209, 88, 0.35), GREEN(), OKI)
    };
    El::block().size(44.0, 44.0).none().radius(RADIUS_PILL).bg(bg).inset(&[sh(0.0, 0.0, 0.0, 1.0, ring)]).place_center().child(El::icon_svg(ic, 22.0, 1.8, c))
}

/// `.sdzr{display:flex;align-items:center;gap:14px;width:100%;padding:0 18px}` `.sdzr .lbl b{display:block;font-size:13.5px;
/// font-weight:600;line-height:18px;ellipsis}` `.sdzr .lbl small{ellipsis}` `.sdzr .lbl .vr{margin-top:4px;font-size:12px;font-weight:600;
/// color:var(--green)}` (`.bad` red, `.busy` --fg2 500) `.sdzr .ctl{gap:6px}`
fn sdzr(icon: El, name: &str, dir: &str, verdict: &str, vc: Rgba, vw: u16, btns: Vec<El>) -> El {
    let lbl = El::col()
        .flex1()
        // `h('b',{text:f.name,title:f.name})`
        .child(El::text(name, Font::new(13.5, 600), FG(), 18.0).ellipsis().key(sub(K_DZ, "name")).title(name))
        .child(El::text(dir, Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).ellipsis().margin(1.0, 0.0, 0.0, 0.0))
        .child(El::text(verdict, Font::new(12.0, vw).tnum(), vc, lh(12.0, 1.35)).margin(4.0, 0.0, 0.0, 0.0));
    let mut r = El::row().center().gap(14.0).w_pct(100.0).pad(0.0, 18.0, 0.0, 18.0).child(icon).child(lbl);
    if !btns.is_empty() {
        r = r.child(El::row().none().center().gap(6.0).children(btns));
    }
    r
}

/// A dashed rounded border inside the box (CSS `border:1.5px dashed`): the width snaps to 1 device pixel (Blink floors a
/// border width to whole device pixels) and the line sits on the snapped box's pixel centres. the owner (test build 1): the zone's
/// edges "look terrible" - the old copy dashed only the straight sides and left each 12 px corner as an empty arc with a 2 px
/// speck. Now the WHOLE outline is dashed, corner middle to corner middle: each of the four runs (half a corner arc, a side,
/// half the next corner arc) holds a whole number of dash + gap periods (dash 3 x width, gap about 3 x width, stretched to
/// fit), starting and ending in the middle of a dash - so every corner carries one dash centred on its 45 degree point and
/// the pattern is the same at all four corners.
#[allow(clippy::too_many_arguments)]
fn dashed_rr(g: &Gfx, x: f32, y: f32, w: f32, h: f32, r: f32, _lw: f32, c: Rgba) {
    let lw = 1.0;
    let (sx, sy, sw, sh) = (x.round(), y.round(), w.round(), h.round());
    let (x0, y0, x1, y1) = (sx + lw / 2.0, sy + lw / 2.0, sx + sw - lw / 2.0, sy + sh - lw / 2.0);
    let rr = (r - lw / 2.0).max(0.5);
    let mut p = skia_safe::Paint::new(c.c4(), None);
    p.set_anti_alias(true);
    p.set_style(skia_safe::PaintStyle::Stroke).set_stroke_width(lw);
    let quarter = std::f32::consts::FRAC_PI_2 * rr;
    let dash = 3.0 * lw;
    // (corner oval, start angle) of the four runs, clockwise from the top-left corner's middle; each run = 45° of its
    // corner, the side, 45° of the next corner
    let ov = |cx_: f32, cy_: f32| skia_safe::Rect::new(cx_ - rr, cy_ - rr, cx_ + rr, cy_ + rr);
    let (tl, tr, br, bl) = (ov(x0 + rr, y0 + rr), ov(x1 - rr, y0 + rr), ov(x1 - rr, y1 - rr), ov(x0 + rr, y1 - rr));
    let runs = [(tl, 225.0f32, tr, (x1 - x0) - 2.0 * rr), (tr, 315.0, br, (y1 - y0) - 2.0 * rr), (br, 45.0, bl, (x1 - x0) - 2.0 * rr), (bl, 135.0, tl, (y1 - y0) - 2.0 * rr)];
    for (a, start, b, side) in runs {
        let mut pb = skia_safe::PathBuilder::new();
        pb.arc_to(a, start, 45.0, true);
        pb.arc_to(b, start + 45.0, 45.0, false);
        let len = side.max(0.0) + quarter;
        let n = (len / (2.0 * dash)).round().max(1.0);
        let gap = len / n - dash;
        let mut q = p.clone();
        // phase dash / 2: the run starts (and, a whole number of periods later, ends) in the middle of a dash
        q.set_path_effect(skia_safe::PathEffect::dash(&[dash, gap.max(0.5)], dash / 2.0));
        g.cv().draw_path(&pb.detach(), &q);
    }
}

impl Page for Security {
    fn id(&self) -> &'static str {
        "sec"
    }
    fn name(&self) -> &'static str {
        "Security"
    }
    fn icon(&self) -> &'static str {
        "secu"
    }
    fn open(&mut self, env: &Env, now: f64) {
        self.env = env.clone();
        self.now = now;
        // the service is made once and kept for the app's life (F2): a scan started before keeps running and is found here
        let held = match env.keep.get::<Held>(KEEP_SVC) {
            Some(h) => h,
            None => {
                let h = if env.fake() {
                    let f = sample_fake();
                    Held { svc: Arc::new(sec::SecurityService::new(Arc::new(f.clone()))), fake: Some(f) }
                } else {
                    Held { svc: Arc::new(Self::real_service()), fake: None }
                };
                env.keep.put(KEEP_SVC, h.clone());
                h
            }
        };
        self.svc = Some(held.svc.clone());
        self.fake = held.fake;
        let (tx, rx) = mpsc::channel();
        self.tx = Some(tx);
        self.rx = Some(rx);
        // what was known last shows at once (then Defender is read again in the background)
        self.page = env.keep.get::<sec::SecurityPage>(KEEP_PAGE);
        self.dz = Some(env.keep.get::<Dz>(KEEP_DZ).unwrap_or(Dz::Idle));
        if let sec::ScanState::Running { kind, elapsed } = held.svc.scan_state() {
            // a scan from an earlier open is still running: its card / drop zone shows it
            let since = now - elapsed.as_secs_f64() * 1000.0;
            if let sec::ScanKind::Path(p) = &kind {
                self.dz = Some(Dz::Busy { path: p.clone(), since });
            }
            self.scanning = Some((kind, since));
        } else {
            if matches!(self.dz, Some(Dz::Busy { .. })) {
                self.dz = Some(Dz::Idle);
            }
            // a scan that ended while the tab was away: its answer now
            if let Some(r) = claim(&env.keep, None) {
                self.scan_done(r);
            }
        }
        self.read();
        self.pump();
    }
    fn close(&mut self) {
        // the page goes; the service (and a running scan) stays in env.keep, so does the drop zone's answer
        if let Some(dz) = self.dz.clone() {
            self.env.keep.put(KEEP_DZ, dz);
        }
        let env = std::mem::take(&mut self.env);
        *self = Security { env, ..Security::default() };
    }
    fn ready(&self) -> bool {
        // Defender's first answer (or the kept one) is in: no "Reading…" flash on a real PC
        self.page.is_some() || self.read_err.is_some()
    }
    /// Order 055: a scan's sweep / the ring are live boxes (motion, the monitor's rate: true every frame while they show);
    /// the page is built again only for data - a message, the end of a scan, a new second in "running 0:05".
    fn tick(&mut self, now: f64) -> bool {
        self.now = now;
        self.clock.set(now);
        let mut data = self.drain();
        // a scan of an earlier open has no message: its end is asked for 4 times a second (not the OS clock, not every frame)
        if self.scanning.is_some() && (data || now - self.polled >= 250.0) {
            self.polled = now;
            data |= self.scan_ended();
        }
        let second = self.scan_secs(now).is_some_and(|s| s != self.secs_shown.get());
        let moving = self.scanning.is_some() || matches!(self.dz, Some(Dz::Busy { .. }));
        self.live_only = !(data || second);
        data || second || moving
    }
    fn live_only(&self) -> bool {
        self.live_only
    }
    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        self.now = cx.now;
        self.clock.set(cx.now);
        self.pump();
        let th = self.threats();
        let qr = self.quarantine();
        let n = th.len();
        let m = qr.len();
        let top = group::grp(vec![self.status(cx), self.scans(cx), self.progress(cx)]);
        let dz = self.drop_zone(cx);
        let th_body = self.rows_body(cx, &th, false);
        let q_body = self.rows_body(cx, &qr, true);
        let ((th_small, th_cnt), (q_small, q_cnt)) = self.card_facts(n, m);
        let thc = self.fold_card(cx, K_TH, "secu", "Threats found", true, &th_small, th_cnt, n > 0, self.th_open, th_body).margin(12.0, 0.0, 0.0, 0.0);
        let qc = self.fold_card(cx, K_Q, "lock", "Quarantine", false, &q_small, q_cnt, false, self.q_open, q_body).margin(10.0, 0.0, 0.0, 0.0);
        // Order 036 (the drawing's RS.sec): the reset line - Allow is a change to Windows (Defender's allow list)
        let rs = reset::reset_line(cx, K_RS, Some("Windows defaults"));
        vec![pieces::header(self.name(), Some(badge::via("shd16", "Microsoft Defender"))), top, dz, thc, qc, rs]
    }
    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        match ev {
            Ev::Press(k, _, _, bx) => {
                self.press_box.insert(*k, *bx);
            }
            // Order 045: files from Explorer over the drop zone (or one of its buttons / its name) light it up
            Ev::DragOver(k) => {
                let parts = [K_DZ, sub(K_DZ, "again"), sub(K_DZ, "rm"), sub(K_DZ, "al"), sub(K_DZ, "name")];
                self.drag_over = k.is_some_and(|k| parts.contains(&k));
            }
            // a file / folder dropped anywhere on the page scans the first one (the drop zone is the page's target)
            Ev::Drop(_, paths) => {
                self.drag_over = false;
                if let Some(p) = paths.first() {
                    if matches!(self.dz, None | Some(Dz::Idle) | Some(Dz::Ok { .. }) | Some(Dz::Handled { .. })) {
                        self.dz = Some(Dz::Idle);
                        self.scan_path(p);
                    }
                }
            }
            // "click to pick one": Windows' file picker (a folder: drop it - the picker opens on files)
            Ev::Click(k) if *k == K_DZ => {
                if let Some(p) = cx.pick_file("Pick a file to scan", &[("All files", "*.*")]) {
                    self.scan_path(&p);
                }
            }
            Ev::Click(k) if *k == K_REVIEW => {
                self.th_open = true;
                cx.scroll_to(K_TH);
            }
            // Order 036: the frame's ONE review over the change log, applied through `Resettable` below
            Ev::Click(k) if *k == sub(K_RS, "pc") => cx.open_reset(crate::undo::Kind::HowItWas, self.bx(*k)),
            Ev::Click(k) if *k == sub(K_RS, "win") => cx.open_reset(crate::undo::Kind::WindowsDefaults, self.bx(*k)),
            Ev::Click(k) => self.click(*k),
            _ => {}
        }
    }
    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        let mut kids = Vec::new();
        if let Some(a) = self.ask_el(cx) {
            kids.push(a.z(20));
        }
        if let Some((t, at)) = self.toast.clone() {
            if cx.now - at < toast::SHOW_MS + 400.0 {
                kids.push(toast::toast(cx, K_TOAST, &t, at, false));
            } else {
                self.toast = None;
            }
        }
        if kids.is_empty() {
            return None;
        }
        Some(El::block().abs(0.0, 0.0, f32::NAN, f32::NAN).size(WIN_W, crate::ui::WIN_H).no_hit().children(kids))
    }
    fn popup_dismiss(&mut self) {
        self.ask = None;
    }
    fn describe(&self) -> String {
        format!(
            "threats={} quarantine={} scanning={} dz={} ask={} toast={:?}",
            self.threats().len(),
            self.quarantine().len(),
            self.scanning.as_ref().map(|s| s.0.title()).unwrap_or_default(),
            match &self.dz {
                Some(Dz::Idle) | None => "idle",
                Some(Dz::Busy { .. }) => "busy",
                Some(Dz::Ok { .. }) => "ok",
                Some(Dz::Bad { .. }) => "bad",
                Some(Dz::Handled { .. }) => "handled",
            },
            self.ask.is_some(),
            self.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default()
        )
    }
    /// Order 036: cheap (nothing is opened here); a page never opened makes its service on the first reset call.
    fn resettable(&mut self) -> Option<&mut dyn Resettable> {
        Some(self)
    }
}

impl Security {
    fn bx(&self, k: Key) -> (f32, f32, f32, f32) {
        self.press_box.get(&k).copied().unwrap_or((300.0, 200.0, 80.0, 26.0))
    }

    /// An admin-only question (Offline scan, Allow, Restore) is asked only when this copy can do it; else the admin toast
    /// right away (no question that ends in "needs admin").
    fn ask_admin(&mut self, a: Ask, k: Key) {
        let elevated = self.svc().map(|s| s.os().is_elevated()).unwrap_or(false);
        if elevated {
            self.ask = Some((a, self.bx(k), self.now));
        } else {
            self.show_toast(Self::err_toast(&sec::SecurityError::NeedsAdmin));
        }
    }

    fn click(&mut self, k: Key) {
        if k == sub(K_ASK, "out") || k == sub(K_ASK, "no") {
            self.ask = None;
            return;
        }
        if k == sub(K_ASK, "go") {
            if let Some((a, _, _)) = self.ask.take() {
                match a {
                    Ask::Offline => self.offline(),
                    Ask::Allow(r) => self.allow(&r),
                    Ask::Restore(r) => self.restore(&r),
                }
            }
            return;
        }
        if k == idx(K_SCAN, 0) {
            self.start(sec::ScanKind::Quick);
        } else if k == idx(K_SCAN, 1) {
            self.start(sec::ScanKind::Full);
        } else if k == idx(K_SCAN, 2) {
            if self.scanning.is_none() {
                self.ask_admin(Ask::Offline, k);
            }
        } else if k == K_CANCEL {
            self.cancel();
        } else if k == K_REVIEW {
            // the drawing also scrolls to the card (page scrolling = PIECES_WANTED cx.scroll_to)
            self.th_open = true;
        } else if k == sub(K_TH, "fold") {
            self.th_open = !self.th_open;
        } else if k == sub(K_Q, "fold") {
            self.q_open = !self.q_open;
        } else if k == K_DZ {
            // (Windows' file picker opens from `event`: it needs the click's Cx)
        } else if k == sub(K_DZ, "again") {
            self.dz = Some(Dz::Idle);
        } else if k == sub(K_DZ, "rm") {
            if let Some(Dz::Bad { row, .. }) = self.dz.clone() {
                self.remove(&row);
            }
        } else if k == sub(K_DZ, "al") {
            if let Some(Dz::Bad { row, .. }) = self.dz.clone() {
                self.ask_admin(Ask::Allow(row), k);
            }
        } else {
            for r in self.threats() {
                let rk = row_key(K_TH, &r);
                if k == sub(rk, "remove") {
                    self.remove(&r);
                    return;
                }
                if k == sub(rk, "allow") {
                    self.ask_admin(Ask::Allow(r.clone()), k);
                    return;
                }
            }
            for r in self.quarantine() {
                let rk = row_key(K_Q, &r);
                if k == sub(rk, "restore") {
                    self.ask_admin(Ask::Restore(r.clone()), k);
                    return;
                }
                if k == sub(rk, "delete") {
                    self.delete();
                    return;
                }
            }
        }
    }
}

#[allow(dead_code)]
const _UNUSED: (Bezier, f32) = (POP, 0.0);

// =================================================================================================== Order 036: reset
// Security's change-log items (the drawing's RS.sec): an Allow in Defender ("allow:<threat id>": Allow, and the Allow a
// Restore adds). Scans, Update, Offline scan, Remove (to Quarantine), the restored file itself and opening Protection
// history are one-time actions: nothing is logged.

/// The app is running (it has the services, maybe busy further up the stack): a unit test without them logs nothing.
fn can_log() -> bool {
    crate::services::in_use() || crate::services::with(|_| ()).is_some()
}

fn allow_item(id: i64) -> String {
    format!("allow:{id}")
}

fn allow_label(file: &str) -> String {
    format!("Allowed in Defender \u{00b7} {file}")
}

fn allow_val(on: bool) -> Val {
    if on {
        Val::new("on", "Allowed")
    } else {
        Val::new("off", "Not allowed")
    }
}

impl Security {
    /// The open page's service, the one kept in `env.keep` from an earlier open, or (never opened: Settings › Reset, the
    /// uninstaller) one made now - the sample fake in a test copy (and in unit tests), Defender otherwise.
    fn rs_service(&self) -> Arc<sec::SecurityService> {
        if let Some(s) = self.svc() {
            return s;
        }
        if let Some(h) = self.env.keep.get::<Held>(KEEP_SVC) {
            return h.svc;
        }
        self.rs_svc
            .get_or_init(|| {
                if crate::testmode::on() || cfg!(test) {
                    Arc::new(sec::SecurityService::new(Arc::new(sample_fake())))
                } else {
                    Arc::new(Self::real_service())
                }
            })
            .clone()
    }
}

// The reset's reads and writes, for the page itself and its detached copy (Order 047: `SecReset`, on the review's worker
// thread) alike: the service + the open page's last read of Defender (None = closed).

/// The open page's last read of Defender (a new read takes 1-2 s: closed = the last recorded value).
fn rs_current(page: Option<&sec::SecurityPage>, item: &str) -> Option<Val> {
    let id: i64 = item.strip_prefix("allow:")?.parse().ok()?;
    let p = page?;
    Some(allow_val(p.allowed.iter().any(|a| a.threat_id == id)))
}

/// Windows' own value: nothing allowed - every Allow Defender holds (also ones made before the app, the drawing's
/// "Allowed in Defender · 1 file → none").
fn rs_defaults(svc: impl FnOnce() -> Arc<sec::SecurityService>, page: Option<&sec::SecurityPage>) -> Vec<DefaultItem> {
    let list = match page {
        Some(p) if p.unreadable.is_none() => p.allowed.clone(),
        _ => svc().allowed_in_defender().unwrap_or_default(),
    };
    list.into_iter()
        .map(|a| {
            let file = a.files.first().map(|f| split(f).1).unwrap_or_else(|| a.name.clone());
            DefaultItem { item: allow_item(a.threat_id), label: allow_label(&file), now: allow_val(true), default: allow_val(false) }
        })
        .collect()
}

fn rs_apply(svc: &sec::SecurityService, item: &str, to: &Val) -> Result<(), String> {
    if crate::testmode::real_read() {
        return Err("A read-only test copy changes nothing".into());
    }
    let id: i64 = item.strip_prefix("allow:").and_then(|s| s.parse().ok()).ok_or("Unknown setting")?;
    let r = if to.raw == "on" {
        svc.add_allow(id)
    } else {
        match svc.remove_allow(id) {
            // already not allowed: nothing to take away
            Err(sec::SecurityError::NoSuchThreat(_)) => Ok(()),
            r => r,
        }
    };
    r.map_err(|e| Security::err_toast(&e))
}

impl Resettable for Security {
    fn page_id(&self) -> &str {
        "sec"
    }
    fn page_title(&self) -> &str {
        "Security"
    }
    fn current(&self, item: &str) -> Option<Val> {
        rs_current(self.page.as_ref(), item)
    }
    fn windows_defaults(&self) -> Vec<DefaultItem> {
        rs_defaults(|| self.rs_service(), self.page.as_ref())
    }
    fn apply(&mut self, item: &str, to: &Val) -> Result<(), String> {
        rs_apply(&self.rs_service(), item, to)?;
        // the open page shows Defender's new list
        self.reset_done();
        Ok(())
    }
    /// Order 047: Defender's list (WMI, 1-2 s) and the Allow changes (an admin prompt) on the review's worker thread.
    fn detach(&mut self) -> Option<crate::undo::Detached> {
        Some(Box::new(SecReset { svc: self.rs_service(), page: self.page.clone() }))
    }
    fn reset_done(&mut self) {
        if self.tx.is_some() {
            self.read();
        }
    }
}

/// Order 047: the Security page's reset as a copy for the review's worker thread.
struct SecReset {
    svc: Arc<sec::SecurityService>,
    page: Option<sec::SecurityPage>,
}

impl Resettable for SecReset {
    fn page_id(&self) -> &str {
        "sec"
    }
    fn page_title(&self) -> &str {
        "Security"
    }
    fn current(&self, item: &str) -> Option<Val> {
        rs_current(self.page.as_ref(), item)
    }
    fn windows_defaults(&self) -> Vec<DefaultItem> {
        rs_defaults(|| self.svc.clone(), self.page.as_ref())
    }
    fn apply(&mut self, item: &str, to: &Val) -> Result<(), String> {
        rs_apply(&self.svc, item, to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::cx::State;
    use crate::ui::lay::Laid;
    use std::time::Duration;

    fn page() -> Security {
        page_in(&Env { test: true, ..Env::default() })
    }
    fn page_in(env: &Env) -> Security {
        let mut p = Security::default();
        p.open(env, 0.0);
        p
    }
    fn fake(p: &Security) -> sec::FakeOs {
        p.fake.clone().unwrap()
    }
    fn log(p: &Security) -> Vec<String> {
        fake(p).log()
    }
    fn elevate(p: &Security) {
        fake(p).set_elevated(true);
    }
    /// Wait until the fake's scan thread runs.
    fn started(p: &Security) {
        assert!(fake(p).wait_scan_started(Duration::from_secs(2)));
    }
    /// Let the fake's scan end and wait for the page to get its answer.
    fn finish(p: &mut Security) {
        started(p);
        fake(p).release_scan();
        settle(p);
    }
    fn settle(p: &mut Security) {
        for _ in 0..400 {
            p.pump();
            if p.scanning.is_none() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the scan did not end");
    }

    #[test]
    fn opening_reads_and_starts_no_scan() {
        let p = page();
        assert!(log(&p).is_empty(), "no scan on open: {:?}", log(&p));
        assert_eq!(p.quarantine().len(), 2);
        assert!(p.threats().is_empty());
        assert!(p.scanning.is_none());
        assert_eq!(p.page.as_ref().unwrap().banner, sec::Banner::Protected);
    }

    #[test]
    fn scans_only_on_their_buttons() {
        let mut p = page();
        p.click(idx(K_SCAN, 0));
        started(&p);
        assert_eq!(log(&p), vec!["scan Quick scan".to_string()]);
        assert!(p.scanning.is_some());
        p.click(idx(K_SCAN, 1));
        assert_eq!(log(&p).len(), 1, "one scan at a time");
        finish(&mut p);
        assert_eq!(p.toast.as_ref().unwrap().0, "Quick scan done \u{00b7} no threats found");
        p.click(idx(K_SCAN, 1));
        p.click(K_CANCEL);
        settle(&mut p);
        assert_eq!(p.toast.as_ref().unwrap().0, "Scan stopped");
    }

    /// Order 055: a running scan keeps frames coming (the sweep is motion) but builds the page only for data / a new second.
    #[test]
    fn a_scan_moves_live_boxes_without_rebuilding_the_page() {
        let mut p = page();
        p.click(idx(K_SCAN, 0));
        started(&p);
        let since = p.scanning.as_ref().unwrap().1;
        assert!(p.tick(since + 10.0), "the sweep moves");
        p.secs_shown.set(0);
        assert!(p.tick(since + 20.0) && p.live_only(), "same second, no message: live pass only");
        assert!(p.tick(since + 1500.0) && !p.live_only(), "a new second in \"running 0:01\": the page is built");
        fake(&p).release_scan();
        let mut t = since + 2000.0;
        for _ in 0..400 {
            t += 300.0;
            p.tick(t);
            if p.scanning.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(p.scanning.is_none(), "the end of the scan is data");
        assert!(!p.tick(t + 10.0), "at rest: no frames");
    }

    /// the owner's F2: a scan goes on with the window closed; the next open shows it running, then its answer (once).
    #[test]
    fn a_scan_outlives_the_page_and_its_answer_waits() {
        let env = Env { test: true, ..Env::default() };
        let mut p = page_in(&env);
        p.click(idx(K_SCAN, 1));
        started(&p);
        let f = fake(&p);
        p.close();
        // reopened while it runs: the same service, the scan shows
        let mut p = page_in(&env);
        assert!(matches!(p.scanning, Some((sec::ScanKind::Full, _))), "running scan found again");
        p.close();
        // reopened while it runs and it ends with this page open: the answer comes through env.keep
        let mut p = page_in(&env);
        p.click(idx(K_SCAN, 0));
        assert_eq!(f.state().scans_started, 1, "no second scan meanwhile");
        f.release_scan();
        settle(&mut p);
        assert_eq!(p.toast.as_ref().unwrap().0, "Full scan done \u{00b7} no threats found");
        p.close();
        // a second scan; it ends with the window closed
        let mut p = page_in(&env);
        p.click(idx(K_SCAN, 1));
        for _ in 0..400 {
            if f.state().scans_started == 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        p.close();
        // it ends with the window closed: the answer waits in env.keep
        f.release_scan();
        for _ in 0..400 {
            if matches!(env.keep.get::<Rep>(KEEP_REPORT), Some((_, false, _))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut p = page_in(&env);
        assert!(p.scanning.is_none());
        assert_eq!(p.toast.as_ref().unwrap().0, "Full scan done \u{00b7} no threats found");
        // shown once: the next open does not show it again; the last read shows at once
        p.close();
        let p = page_in(&env);
        assert!(p.toast.is_none());
        assert_eq!(p.quarantine().len(), 2);
    }

    #[test]
    fn a_dropped_file_scans_and_its_answer_is_kept() {
        let env = Env { test: true, ..Env::default() };
        let mut p = page_in(&env);
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        p.event(&Ev::Drop(K_DZ, vec![r"C:\x\notes.txt".into()]), &mut cx);
        assert!(matches!(p.dz, Some(Dz::Busy { .. })));
        finish(&mut p);
        assert!(matches!(&p.dz, Some(Dz::Ok { text, .. }) if text == "No threats found"), "{:?}", p.dz);
        p.close();
        let p = page_in(&env);
        assert!(matches!(&p.dz, Some(Dz::Ok { .. })), "the zone's answer is remembered");
    }

    #[test]
    fn offline_scan_asks_first_and_needs_admin() {
        let mut p = page();
        // not elevated: no question, the admin toast at once
        p.click(idx(K_SCAN, 2));
        assert!(p.ask.is_none());
        assert_eq!(p.toast.as_ref().unwrap().0, crate::admin::NOT_CHANGED);
        elevate(&p);
        p.click(idx(K_SCAN, 2));
        assert!(matches!(p.ask, Some((Ask::Offline, _, _))));
        assert!(log(&p).is_empty());
        p.click(sub(K_ASK, "no"));
        assert!(p.ask.is_none() && log(&p).is_empty());
        p.click(idx(K_SCAN, 2));
        p.click(sub(K_ASK, "go"));
        assert_eq!(log(&p), vec!["offline scan (restart)".to_string()]);
    }

    /// REVIEW_023 d1c8dd5 HOLD 2: lists that could not be read show no "Empty" and no count; read lists show theirs.
    #[test]
    fn unread_lists_claim_no_count() {
        let mut p = page();
        let (n, m) = (p.threats().len(), p.quarantine().len());
        let ((_, thc), (_, qc)) = p.card_facts(n, m);
        assert_eq!((thc, qc), (Some(n), Some(m)));
        p.page.as_mut().unwrap().unreadable = Some(sec::SecurityError::NeedsAdmin);
        let ((th, thc), (q, qc)) = p.card_facts(0, 0);
        assert_eq!((th.as_str(), thc, q.as_str(), qc), ("Can\u{2019}t be read right now", None, "Can\u{2019}t be read right now", None));
        // a first read that failed (no page at all): not "Reading…" forever
        p.page = None;
        p.read_err = Some(sec::SecurityError::NeedsAdmin);
        let ((th, thc), (_, qc)) = p.card_facts(0, 0);
        assert_eq!((th.as_str(), thc, qc), ("Can\u{2019}t be read right now", None, None));
    }

    /// The 016 review's case: Defender quarantined the dropped file on its own - the zone says a threat was found (bad look),
    /// never "No threats found".
    #[test]
    fn a_threat_defender_handled_itself_is_not_clean() {
        let mut p = page();
        p.scan_path(r"C:\x\handled_sample.exe");
        finish(&mut p);
        assert!(matches!(p.dz, Some(Dz::Handled { .. })), "{:?}", p.dz);
        assert!(p.toast.as_ref().unwrap().0.starts_with("Threat found"));
        assert_eq!(p.quarantine().len(), 3);
    }

    #[test]
    fn a_dropped_threat_then_remove_and_allow() {
        let mut p = page();
        p.scan_path(r"C:\Users\User\Downloads\free_skins_unlocker.zip");
        assert!(matches!(p.dz, Some(Dz::Busy { .. })));
        finish(&mut p);
        assert!(matches!(p.dz, Some(Dz::Bad { .. })), "{:?}", p.dz);
        assert_eq!(p.threats().len(), 1);
        // without admin: says so, changes nothing
        p.click(sub(K_DZ, "rm"));
        assert_eq!(p.toast.as_ref().unwrap().0, crate::admin::NOT_CHANGED);
        assert_eq!(p.threats().len(), 1);
        elevate(&p);
        p.click(sub(K_DZ, "rm"));
        assert_eq!(p.toast.as_ref().unwrap().0, "free_skins_unlocker.zip removed \u{00b7} in Quarantine now");
        assert!(p.threats().is_empty());
        assert_eq!(p.quarantine().len(), 3);
        assert!(matches!(&p.dz, Some(Dz::Ok { text, .. }) if text.starts_with("Removed")));
    }

    #[test]
    fn allow_and_restore_ask_first() {
        let mut p = page();
        elevate(&p);
        p.scan_path(r"C:\x\keygen.exe");
        finish(&mut p);
        let r = p.threats()[0].clone();
        p.click(sub(row_key(K_TH, &r), "allow"));
        assert!(matches!(p.ask, Some((Ask::Allow(_), _, _))));
        assert!(!log(&p).iter().any(|l| l.starts_with("allow")));
        p.click(sub(K_ASK, "go"));
        assert!(log(&p).contains(&format!("allow {}", r.threat_id)));
        assert_eq!(p.toast.as_ref().unwrap().0, "keygen.exe allowed");
        let q = p.quarantine().iter().find(|q| q.file == "kms_activator.exe").unwrap().clone();
        p.click(sub(row_key(K_Q, &q), "restore"));
        p.click(sub(K_ASK, "go"));
        assert_eq!(p.toast.as_ref().unwrap().0, "kms_activator.exe restored to Downloads");
        assert_eq!(p.quarantine().len(), 1);
    }

    #[test]
    fn delete_opens_windows_security_history() {
        let mut p = page();
        let q = p.quarantine()[0].clone();
        p.click(sub(row_key(K_Q, &q), "delete"));
        assert_eq!(log(&p), vec!["open protection history".to_string()]);
        assert_eq!(p.quarantine().len(), 2, "nothing deleted by the app");
    }

    #[test]
    fn closing_drops_the_page_but_keeps_the_service() {
        let env = Env { test: true, ..Env::default() };
        let mut p = page_in(&env);
        p.close();
        assert!(p.svc.is_none() && p.page.is_none() && p.rx.is_none());
        assert!(env.keep.get::<Held>(KEEP_SVC).is_some());
    }

    /// Boxes = Chromium's (dom_dump on menu-v22, page sec): the status group (26, 98) 548 x 151.69, the first scan button
    /// (36, 187.69) 170.66 x 52, the drop zone (26, 261.69) 548 x 132, Threats found (26, 405.69) 548 x 58, Quarantine
    /// (26, 473.69) 548 x 58.
    #[test]
    fn boxes_match_the_drawing() {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut p = page();
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        let kids = p.build(&mut cx);
        let root = El::block().w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids);
        let laid = Laid::new(&g, root, 600.0, None);
        let r = |k: Key| laid.rect_of(k).map(|(x, y, w, h)| (x, y + 56.0, w, h)).unwrap();
        let near = |a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)| (a.0 - b.0).abs() < 0.02 && (a.1 - b.1).abs() < 0.02 && (a.2 - b.2).abs() < 0.02 && (a.3 - b.3).abs() < 0.02;
        let b0 = r(idx(K_SCAN, 0));
        assert!(near(b0, (36.0, 187.6875, 170.6563, 52.0)), "scan 0 {:?}", b0);
        let dz = r(K_DZ);
        assert!(near(dz, (26.0, 261.6875, 548.0, 132.0)), "drop zone {:?}", dz);
        let th = r(sub(K_TH, "fold"));
        assert!(near(th, (26.0, 405.6875, 548.0, 58.0)), "threats {:?}", th);
        let q = r(sub(K_Q, "fold"));
        assert!(near(q, (26.0, 473.6875, 548.0, 58.0)), "quarantine {:?}", q);
    }

    // ------------------------------------------------------------------ Order 036: the change log + the shared reset
    fn review(p: &Security, kind: crate::undo::Kind) -> crate::undo::Review {
        crate::services::with(|s| {
            crate::undo::flush(&mut s.store);
            crate::undo::Review::for_page(kind, p, &s.store)
        })
        .unwrap()
    }

    /// As the frame does it: the page applies outside the services, each ok line is noted.
    fn reset(p: &mut Security, rv: &crate::undo::Review) -> Vec<crate::undo::LineResult> {
        let res = rv.apply_each(&mut [p as &mut dyn Resettable], &mut |l| {
            crate::undo::note(&l.page, &l.item, &l.label, &l.from, &l.to);
            Ok(())
        });
        crate::services::with(|s| crate::undo::flush(&mut s.store));
        res
    }

    /// Allow = ONE entry (not allowed before); untick = kept; the reset takes the Allow away again. Remove (Quarantine) is a
    /// one-time action: no entry.
    #[test]
    fn an_allow_goes_into_the_change_log_and_back() {
        use crate::undo::{read_record, Kind};
        crate::services::init(windows::Win32::Foundation::HWND::default(), true);
        let mut p = page();
        elevate(&p);
        p.scan_path(r"C:\x\keygen.exe");
        finish(&mut p);
        let r = p.threats()[0].clone();
        p.click(sub(row_key(K_TH, &r), "allow"));
        p.click(sub(K_ASK, "go"));
        p.pump();
        p.pump();
        let item = format!("allow:{}", r.threat_id);
        let rec = crate::services::with(|s| read_record(&s.store, "sec", &item)).flatten().expect("an entry");
        assert_eq!((rec.label.as_str(), rec.was.text.as_str(), rec.now.text.as_str()), ("Allowed in Defender \u{00b7} keygen.exe", "Not allowed", "Allowed"));
        let mut rv = review(&p, Kind::HowItWas);
        assert_eq!(rv.lines.len(), 1, "Allow only (the scan wrote nothing)");
        assert_eq!(rv.lines[0].change_text(), "Allowed  →  Not allowed");
        rv.toggle(0);
        assert!(reset(&mut p, &rv).is_empty());
        assert!(fake(&p).state().allowed.contains(&r.threat_id), "an unticked line stays as it is");
        rv.toggle(0);
        assert_eq!(reset(&mut p, &rv)[0].outcome, crate::undo::Outcome::Ok);
        assert!(!fake(&p).state().allowed.contains(&r.threat_id), "the Allow is gone again");
        p.pump();
        assert!(review(&p, Kind::HowItWas).is_empty(), "nothing left to reset");
        crate::services::shutdown();
    }

    /// Restore puts the file back AND allows it: the Allow is an entry (the restore itself is one-time). "Windows defaults" =
    /// nothing allowed (every Allow Defender holds), applied through the page.
    #[test]
    fn restore_logs_its_allow_and_windows_defaults_allow_nothing() {
        use crate::undo::Kind;
        crate::services::init(windows::Win32::Foundation::HWND::default(), true);
        let mut p = page();
        elevate(&p);
        fake(&p).state().allowed.push(42);
        let q = p.quarantine().iter().find(|q| q.file == "kms_activator.exe").unwrap().clone();
        p.click(sub(row_key(K_Q, &q), "restore"));
        p.click(sub(K_ASK, "go"));
        p.pump();
        p.pump();
        let how = review(&p, Kind::HowItWas);
        assert_eq!(how.lines.iter().map(|l| l.label.as_str()).collect::<Vec<_>>(), ["Allowed in Defender \u{00b7} kms_activator.exe"]);
        let rv = review(&p, Kind::WindowsDefaults);
        assert_eq!(rv.lines.len(), 2, "the restored one + one allowed before the app: {:?}", rv.lines);
        let res = reset(&mut p, &rv);
        assert!(res.iter().all(|r| r.outcome == crate::undo::Outcome::Ok), "{res:?}");
        assert!(fake(&p).state().allowed.is_empty());
        p.pump();
        assert!(review(&p, Kind::WindowsDefaults).is_empty());
        crate::services::shutdown();
    }

    /// The reset line opens the frame's shared review; a page never opened resets through its own service (the sample fake
    /// in tests - never Defender), and without admin says so.
    #[test]
    fn the_reset_line_and_a_never_opened_page() {
        use crate::undo::Kind;
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut p = page();
        let mut cx = Cx::new(0.0, false, &g, &mut st).for_page("sec");
        let b = (200.0, 700.0, 140.0, 16.0);
        p.event(&Ev::Press(sub(K_RS, "pc"), 210.0, 705.0, b), &mut cx);
        p.event(&Ev::Click(sub(K_RS, "pc")), &mut cx);
        assert!(matches!(cx.reqs.as_slice(), [crate::ui::cx::Req::Reset(Kind::HowItWas, r)] if *r == b), "{:?}", cx.reqs);
        drop(cx);
        crate::services::init(windows::Win32::Foundation::HWND::default(), true);
        let mut c = Security::default();
        assert!(c.resettable().is_some());
        assert!(c.rs_svc.get().is_none(), "resettable() is cheap: nothing made");
        crate::undo::note("sec", "allow:7", "Allowed in Defender \u{00b7} x.exe", &allow_val(false), &allow_val(true));
        let rv = review(&c, Kind::HowItWas);
        let res = reset(&mut c, &rv);
        assert_eq!(res[0].outcome, crate::undo::Outcome::Failed(crate::admin::NOT_CHANGED.into()));
        assert!(c.rs_svc.get().is_some(), "made on first use");
        crate::services::shutdown();
    }

    /// Order 047: the frame's reset through the page's detached copy - the review opened and the Reset pressed each inside
    /// one frame (16 ms), the reads and the put-backs on the review's worker thread; then the page re-reads (`reset_done`).
    /// (The page's fake has no slow mode for these calls: the proof is that both run on the worker - `Reading` / `Running`.)
    fn reset_off_the_menu(p: &mut dyn Resettable, kind: crate::undo::Kind) -> (crate::undo::Review, Vec<crate::undo::LineResult>) {
        fn wait<T>(mut f: impl FnMut() -> Option<T>) -> T {
            let t0 = std::time::Instant::now();
            loop {
                if let Some(v) = f() {
                    return v;
                }
                assert!(t0.elapsed().as_secs() < 10, "the review's worker never answered");
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        let opened = crate::services::with(|s| {
            crate::undo::flush(&mut s.store);
            crate::offui::assert_quick("opening the review", || crate::undo::Review::open(kind, false, &mut [&mut *p], &s.store))
        })
        .unwrap();
        let crate::undo::Opened::Reading(mut job) = opened else { panic!("the review is read on a worker thread") };
        let rv = wait(|| job.take());
        let applied = crate::offui::assert_quick("Reset", || rv.start_apply(&mut [&mut *p]));
        let crate::undo::Applied::Running(mut job) = applied else { panic!("the reset is put back on a worker thread") };
        let res = wait(|| job.take());
        p.reset_done();
        (rv, res)
    }

    /// Order 047: Security's reset (Windows defaults: nothing allowed) is read and put back on the review's worker thread -
    /// Defender's list (WMI, 1-2 s) and the admin-proxied Allow change never hold the menu; same lines, same results.
    #[test]
    fn the_reset_review_reads_and_puts_back_off_the_menus_thread() {
        use crate::undo::Kind;
        crate::services::init(windows::Win32::Foundation::HWND::default(), true);
        let mut p = page();
        elevate(&p);
        fake(&p).state().allowed.push(42);
        p.read();
        p.pump();
        let (rv, res) = reset_off_the_menu(&mut p, Kind::WindowsDefaults);
        assert_eq!(rv.lines.len(), 1, "{:?}", rv.lines);
        assert_eq!(rv.lines[0].change_text(), "Allowed  →  Not allowed");
        assert!(res.iter().all(|r| r.outcome == crate::undo::Outcome::Ok), "{res:?}");
        assert!(fake(&p).state().allowed.is_empty(), "put back by the worker");
        p.pump();
        assert!(review(&p, Kind::WindowsDefaults).is_empty(), "the open page read Defender again");
        crate::services::shutdown();
    }
}

/// `.sev.mid` words: var(--amber); light (Order 033) `#sw.light .sev.mid{color:#b07d00}`.
fn mid_col() -> Rgba {
    if crate::ui::is_light() {
        Rgba::hex(0xb07d00)
    } else {
        AMBER()
    }
}

//! The Mouse tab's service (Order 019): ONE background thread owns bu-mouse's `Mouse<O>` - the REAL OS layer in normal
//! runs, the FAKE one (`sample_fake`, the drawing's sample PC) in every test copy - so nothing the page does can stall the
//! window: reading / writing the mouse itself (HID, up to ~1 s per request when it sleeps) and handing settings to Raw
//! Accel's driver (its own ~1 s anti-abuse delay per write) run there. The page sends `Cmd`s and gets `Reply`s (a fresh
//! `View` of everything it shows + a toast / an error line), polled from `Page::tick`.
//!
//! Opening the tab reads only light things first (Windows' mouse settings, cursors, Raw Accel's status - registry /
//! SystemParametersInfo / one ioctl), then the mouse itself (`mice()` from the device list + `read_on_mouse`, read
//! requests only). Nothing slow starts on its own after that: every change comes from a click.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::JoinHandle;

use bu_mouse::accel::args::Profile;
use bu_mouse::accel::panel::{Curve, Panel};
use bu_mouse::accel::service::RawAccelStatus;
use bu_mouse::accel::switch::{PerApp, Target};
use bu_mouse::cursors::{CursorsState, Pack, Role, SetId};
use bu_mouse::device::{OnMouse, ReadOptions, YourMouse};
use bu_mouse::fake::{CmouseModel, FakeOs};
use bu_mouse::os::{DriverVersion, HidInfo, MouseOs, WinRaw, WinSetting};
use bu_mouse::pulsar::Link;
use bu_mouse::settings::WindowsMouse;
use bu_mouse::{AppDirs, Mouse};

use crate::undo::{DefaultItem, Val};

/// Everything the page shows, as read by the worker.
#[derive(Clone, Debug, Default)]
pub struct View {
    pub win: Option<WindowsMouse>,
    /// the connected mice, best first (`[0]` = "Your mouse"); `None` = not read yet
    pub mice: Option<Vec<YourMouse>>,
    pub on_mouse: Option<OnMouse>,
    pub ra: Option<RawAccelStatus>,
    pub panel: Panel,
    pub per_app: PerApp,
    /// the user's own Raw Accel profile 0 (the graph is drawn on it, as the driver would run it)
    pub base: Profile,
    /// Order 063: "another program also writes the driver" - the card's warning line (None = nothing to say)
    pub other_writer: Option<String>,
    /// Order 077: that line comes with the one-click "Use ours again" (Raw Accel set the driver after this app)
    pub use_ours: bool,
    /// Order 063: rows whose game runs now and was switched to
    pub active: Vec<bu_mouse::accel::switch::RowId>,
    pub cursors: Option<CursorsState>,
    pub packs: Vec<Pack>,
    pub glass: bool,
    /// Order 042: the cursor schemes Windows has installed (and the user's saved ones) with the bubbles each has a cursor
    /// for - listed in the role pickers after the app's own sets
    pub schemes: Vec<(String, Vec<Role>)>,
    /// per bubble (Role::ALL order): the "Matches your other cursors" set, if any
    pub suggest: Vec<Option<SetId>>,
    /// Order 066: the cursor FILE of each of the 7 bubbles (Role::ALL order) for every set the pickers list - the pickers
    /// draw the real pictures from these
    pub files: Vec<(SetId, Vec<Option<String>>)>,
    /// Order 066: the files picked before with "Choose your own file…" (full paths), newest first
    pub own_files: Vec<String>,
    pub elevated: bool,
    /// Order 036 - the change log: every item's value now (item id -> value), the reset review's "now" side
    pub vals: Vec<(String, Val)>,
    /// "Windows defaults": every item with its value now and Windows' own value
    pub win_def: Vec<DefaultItem>,
}

/// The page id the change log keeps the Mouse tab's items under.
pub const PAGE: &str = "cur";

/// The change log's items of the Mouse tab: (item id, label). Ids are stable (kept across restarts). The mouse's own
/// DPI / polling / lift-off are not here: they are saved on the mouse, not in Windows (the drawing's `RS.cur` lists none).
pub const ITEMS: [(&str, &str); 8] = [
    ("speed", "Pointer speed"),
    ("epp", "Enhance pointer precision"),
    ("scroll", "Scroll lines"),
    ("dblclick", "Double-click speed"),
    ("swap", "Swap primary button"),
    ("cursors", "Cursors"),
    ("cursor_size", "Cursor size"),
    ("accel", "Mouse acceleration"),
];

/// One change the worker made on the PC: (item id, label, value before, value after) - written into the change log by
/// the page (`Mouse::take`).
pub type Change = (String, String, Val, Val);

/// What the page asks for.
#[derive(Clone, Debug)]
pub enum Cmd {
    Speed(u32),
    Precision(bool),
    Lines(u32),
    DoubleClick(usize),
    Swap(bool),
    Dpi(u32),
    Polling(u32),
    LiftOff(u32),
    /// the card's state as the page edited it (curve, values, presets, per app); handed to the driver if it changed
    Accel(Box<(Panel, PerApp)>),
    AccelOn(bool),
    /// "Use ours again" on the amber line: Raw Accel set the driver after this app - the card is set again (a click, so it writes)
    UseOurs,
    CopyCurve,
    CursorRole(Role, SetId),
    CursorSize(u32),
    DeletePack(String),
    /// "Choose your own file…" in a role's picker: that .cur / .ani for this role
    RoleFile(Role, PathBuf),
    /// "Import cursors…": .cur / .ani files and / or a pack's folder
    Import(Vec<PathBuf>),
    /// hover a row of a role's picker = try that cursor (Windows shows it until `EndPreview`)
    Preview(Role, SetId),
    /// "Get more cursors": the downloaded .zip of a pack (name, file in the app's folder) - unpacked, added as a scheme, the
    /// .zip deleted
    InstallStore(String, PathBuf),
    EndPreview,
    /// the change log's reset (the frame's review, a ticked line): put one item to this value; the answer comes back on
    /// the sender (the frame waits for it)
    Restore(String, Val, Sender<Result<(), String>>),
    /// Order 047: the same, asked by the review's worker thread (a detached reset copy): its answer is not one the page
    /// counts as pending
    RestoreAway(String, Val, Sender<Result<(), String>>),
    /// a reset was put back on the UI thread (Order 036 review): only read again, so the tab shows it
    Reread,
}

/// One answer: the new view, maybe a toast, maybe an error (shown as a toast too).
#[derive(Clone, Debug)]
pub struct Reply {
    pub view: View,
    pub toast: Option<String>,
    /// the mouse is still being read (the first reply comes before the HID read)
    pub reading_mouse: bool,
    /// sent on its own (a mouse plugged in / unplugged), not the answer to a command
    pub unasked: bool,
    /// what the command changed on the PC (the change log)
    pub changes: Vec<Change>,
}

/// The page's handle on the worker. Dropping it ends the thread (the tab closed: nothing stays running).
pub struct Svc {
    tx: Option<Sender<Cmd>>,
    rx: Receiver<Reply>,
    th: Option<JoinHandle<()>>,
    /// commands sent and not answered yet
    pub pending: usize,
    /// the first view has not arrived yet
    pub opening: bool,
}

impl Svc {
    /// `fake` = the test copies' sample PC (the drawing's values); else the real Windows layer.
    pub fn start(fake: bool) -> Svc {
        Svc::start_with(fake, Sample::default())
    }

    /// `sample` = which of the drawing's sample PCs the FAKE is (test copies only).
    pub fn start_with(fake: bool, sample: Sample) -> Svc {
        Svc::start_ex(fake, false, sample)
    }

    /// `read_only` = a `--real-read` test copy: the real Windows layer that refuses every change (`RealOs::read_only`).
    pub fn start_ex(fake: bool, read_only: bool, sample: Sample) -> Svc {
        Svc::start_full(fake, read_only, sample, false)
    }

    /// `accel_logged` = the change log already holds "Mouse acceleration" (its "how your PC was" is kept): the worker then
    /// keeps no copy of the driver's settings before its first write.
    pub fn start_full(fake: bool, read_only: bool, sample: Sample, accel_logged: bool) -> Svc {
        let (tx, crx) = channel::<Cmd>();
        let (rtx, rx) = channel::<Reply>();
        let th = std::thread::Builder::new()
            .name("bu-mouse".into())
            .spawn(move || {
                if fake {
                    DRAWING_SETS.with(|d| d.set(true));
                    run(sample_fake_with(sample), crx, rtx, accel_logged, false);
                } else {
                    #[cfg(windows)]
                    run(real(read_only), crx, rtx, accel_logged, true);
                }
            })
            .ok();
        // the open read answers twice (light things, then the mouse itself)
        Svc { tx: Some(tx), rx, th, pending: 2, opening: true }
    }

    pub fn send(&mut self, c: Cmd) {
        if let Some(tx) = &self.tx {
            if tx.send(c).is_ok() {
                self.pending += 1;
            }
        }
    }

    /// Order 047: the worker's command line for another thread (a detached reset copy: `Cmd::RestoreAway`).
    pub fn sender(&self) -> Option<Sender<Cmd>> {
        self.tx.clone()
    }

    /// Replies that arrived (never waits).
    pub fn poll(&mut self) -> Vec<Reply> {
        let mut out = Vec::new();
        while let Ok(r) = self.rx.try_recv() {
            if !r.unasked {
                self.pending = self.pending.saturating_sub(1);
            }
            self.opening = false;
            out.push(r);
        }
        out
    }

    /// Tests: wait until every command is answered (at most `ms`).
    pub fn settle(&mut self, ms: u64) -> Vec<Reply> {
        let t0 = std::time::Instant::now();
        let mut out = Vec::new();
        while self.pending > 0 && t0.elapsed().as_millis() < ms as u128 {
            match self.rx.recv_timeout(std::time::Duration::from_millis(20)) {
                Ok(r) => {
                    if !r.unasked {
                        self.pending = self.pending.saturating_sub(1);
                    }
                    self.opening = false;
                    out.push(r);
                }
                Err(_) => continue,
            }
        }
        out
    }
}

impl Drop for Svc {
    fn drop(&mut self) {
        // closing the channel ends the worker's loop; it is not joined (a mouse read may still be waiting on Windows -
        // the thread ends by itself when that returns)
        self.tx.take();
        self.th.take();
    }
}

/// The app's own folder for the Mouse tab (packs, the copy of Raw Accel's earlier settings).
pub(super) fn data_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Boyler Utilities").join("mouse")).unwrap_or_else(|| PathBuf::from("."))
}

/// The real service with nothing read yet (a closed tab's reset: no Raw Accel folder search, no mouse, no thread).
#[cfg(windows)]
fn real_bare(read_only: bool) -> Mouse<bu_mouse::win::RealOs> {
    let os = if read_only { bu_mouse::win::RealOs::read_only() } else { bu_mouse::win::RealOs::new() };
    Mouse::new(os, AppDirs::new(data_dir()))
}

#[cfg(windows)]
fn real(read_only: bool) -> Mouse<bu_mouse::win::RealOs> {
    let mut m = real_bare(read_only);
    // Order 040: the app's own Glass cursors (built into the exe) go into its folder when the tab opens - only the files
    // that are missing or differ, so an update replaces older ones. A read-only test copy writes nothing; test copies on
    // the fake never get here (their folder is never written).
    // (never without APPDATA: the folder would fall back to the current folder)
    if !read_only && std::env::var_os("APPDATA").is_some() {
        let _ = m.install_glass();
    }
    m.accel_mut().rawaccel_dir = find_rawaccel_folder();
    m
}

/// Raw Accel's folder (for its settings.json + writer.exe): the user's Desktop / Downloads / Documents, names only
/// (Order 037: first the add-ons folder - its RawAccel is the one the Add-ons page installed).
#[cfg(windows)]
pub fn find_rawaccel_folder() -> Option<PathBuf> {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let roots: Vec<PathBuf> = std::iter::once(crate::addons::dir()).chain(["Desktop", "Downloads", "Documents"].iter().map(|d| PathBuf::from(&home).join(d))).collect();
    bu_mouse::accel::service::find_rawaccel_dir(&roots)
}

/// The real service for the always-on engine (nothing read, no cursor files written).
#[cfg(windows)]
pub fn real_bare_pub() -> Mouse<bu_mouse::win::RealOs> {
    real_bare(false)
}

/// The VALORANT row's exe (the per-app sample row; the app picker's games are in `mod.rs`).
pub const VAL_EXE: &str = "VALORANT-Win64-Shipping.exe";

/// The drawing's sample PC as a FAKE: Windows' mouse settings as drawn (speed 10, Enhance pointer precision off, 3 lines,
/// 500 ms, not swapped), a Pulsar X2 CrazyLight on its 8K dongle (1600 DPI on the active stage, 1000 Hz, 1 mm, 78 %), Raw
/// Accel 1.7.0 with the card as drawn (presets Valorant = Linear 2.8 / 55 / cap output 2.6 and Default = Natural, Valorant
/// loaded; VALORANT -> Valorant, everywhere else Off; switched on, folded), every cursor Windows default at size 1.
pub fn sample_fake() -> Mouse<FakeOs> {
    sample_fake_with(Sample::default())
}

/// The drawing's other sample PCs (its keys K and R): a mouse the app can't talk to (Lamzu Maya X), Raw Accel not
/// installed.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sample {
    pub unsupported_mouse: bool,
    pub no_raw_accel: bool,
}

pub fn sample_fake_with(sample: Sample) -> Mouse<FakeOs> {
    let mut os = FakeOs::new();
    os.win.insert(WinSetting::Precision, WinRaw::Mouse([0, 0, 0]));
    const CFG: &str = r"\\?\hid#vid_3710&pid_5406&mi_01&col05#sample";
    let hid = |path: &str, page: u16, inl: u16, outl: u16| HidInfo {
        path: path.into(),
        vid: 0x3710,
        pid: 0x5406,
        version: 0x0305,
        usage_page: page,
        usage: 0x02,
        input_len: inl,
        output_len: outl,
        feature_len: 0,
        interface: Some(1),
        product: Some("Pulsar 8K Dongle".into()),
        manufacturer: None,
    };
    os.hid = vec![hid(r"\\?\hid#vid_3710&pid_5406&mi_00#sample", 0x01, 8, 0), hid(CFG, 0xFF02, 17, 17)];
    let mut model = CmouseModel::new();
    // the drawing's mouse: stage 2 (1600 DPI) active
    model.mem[4..6].copy_from_slice(&[2, 0x55 - 2]);
    os.mice.insert(CFG.into(), Box::new(move |req| Ok(model.answer(req))));
    if sample.unsupported_mouse {
        os.hid = vec![HidInfo {
            path: r"\\?\hid#vid_373e&pid_001e&mi_00#sample".into(),
            vid: 0x373E,
            pid: 0x001E,
            version: 0x0100,
            usage_page: 0x01,
            usage: 0x02,
            input_len: 8,
            output_len: 0,
            feature_len: 0,
            interface: Some(0),
            product: Some("Lamzu Maya X".into()),
            manufacturer: None,
        }];
    }
    os.rawaccel_version = if sample.no_raw_accel { None } else { Some(DriverVersion { major: 1, minor: 7, patch: 0 }) };
    let mut m = Mouse::new(os, AppDirs::new(r"C:\BoylerUtilities-test-no-such-folder\mouse"));
    let p = &mut m.accel_mut().panel;
    p.set_curve(Curve::Natural);
    let (def, _) = p.save_as_preset();
    let _ = p.rename_preset(def, "Default");
    p.set_curve(Curve::Linear);
    // the drawing's numbers (Linear's own defaults are his real Raw Accel since Order 077: 2.6 / cap output 2.0)
    p.set_value(bu_mouse::accel::panel::Field::Acceleration, 2.8);
    p.set_value(bu_mouse::accel::panel::Field::CapOutput, 2.6);
    let (val, _) = p.save_as_preset();
    let _ = p.rename_preset(val, "Valorant");
    // chips in the drawing's order: Valorant, Default
    p.presets.reverse();
    p.on = true;
    p.expanded = false;
    let a = m.accel_mut();
    let row = a.per_app.add_row(VAL_EXE, Target::Preset(val));
    a.per_app.set_row_label(row, "VALORANT");
    a.per_app.set_everywhere_else(Target::Off);
    // the sample driver already runs what the card says (no "another program wrote it" line in the drawing)
    if let Ok(cfg) = m.config_for(&m.accel_target()) {
        m.os_mut().rawaccel_driver = bu_mouse::accel::bytes::to_bytes(&cfg);
    }
    m
}

fn on_off(b: bool) -> String {
    if b { "On" } else { "Off" }.into()
}

fn lines_text(n: u32) -> String {
    match n {
        u32::MAX => "One screen".into(),
        1 => "1 line".into(),
        n => format!("{n} lines"),
    }
}

// ------------------------------------------------------------------------------------------------ the change log (Order 036)

fn win_setting(item: &str) -> Option<WinSetting> {
    Some(match item {
        "speed" => WinSetting::PointerSpeed,
        "epp" => WinSetting::Precision,
        "scroll" => WinSetting::ScrollLines,
        "dblclick" => WinSetting::DoubleClick,
        "swap" => WinSetting::SwapButtons,
        _ => return None,
    })
}

/// An item's label (the drawing's words).
pub fn label(item: &str) -> &'static str {
    ITEMS.iter().find(|(i, _)| *i == item).map(|(_, l)| *l).unwrap_or("")
}

/// A Windows mouse setting as the change log keeps it: raw = `WinRaw::to_text`, text = what the page shows.
fn win_val(s: WinSetting, v: WinRaw) -> Val {
    let text = match (s, v) {
        (WinSetting::Precision, WinRaw::Mouse(a)) => on_off(a[2] != 0),
        (WinSetting::ScrollLines, WinRaw::Num(n)) => lines_text(n),
        (WinSetting::DoubleClick, WinRaw::Num(n)) => format!("{n} ms"),
        (WinSetting::SwapButtons, WinRaw::Bool(b)) => on_off(b),
        (_, v) => v.to_text(),
    };
    Val::new(&v.to_text(), &text)
}

/// The cursors' words: one set for every bubble = its name ("Windows default", "Neon Pack"), else "your own".
fn cursors_words<O: MouseOs>(m: &Mouse<O>) -> String {
    match m.cursors() {
        Ok(c) => match c.roles.first() {
            Some(f) if c.roles.iter().all(|r| r.set == f.set) => f.set.label(),
            _ => "your own".into(),
        },
        Err(_) => "your own".into(),
    }
}

/// One item's value on the PC now (None = it can't be read: no Raw Accel 1.7 driver, a failed read).
pub fn item_val<O: MouseOs>(m: &Mouse<O>, item: &str) -> Option<Val> {
    if let Some(s) = win_setting(item) {
        return m.os().win_get(s).ok().map(|v| win_val(s, v));
    }
    match item {
        "cursors" => Some(Val::new(&m.cursor_look_text().ok()?, &cursors_words(m))),
        "cursor_size" => {
            let raw = m.cursor_size_text().ok()?;
            let px = raw.split(',').next().and_then(|p| p.parse().ok()).unwrap_or(32);
            Some(Val::new(&raw, &bu_mouse::cursors::size_step(px).clamp(1, 15).to_string()))
        }
        "accel" => {
            let b = m.driver_state().ok()??;
            Some(Val::new(&m.driver_state_file(&b).to_string_lossy(), &on_off(bu_mouse::accel::service::driver_state_on(&b))))
        }
        _ => None,
    }
}

/// Every item's value now (the view's `vals`).
fn item_vals<O: MouseOs>(m: &Mouse<O>) -> Vec<(String, Val)> {
    ITEMS.iter().filter_map(|(i, _)| item_val(m, i).map(|v| (i.to_string(), v))).collect()
}

/// "Windows defaults": Windows' own values (Control Panel's) - speed 10, precision on, 3 lines, 500 ms, not swapped, the
/// Windows cursors at size 1 (the frame leaves out lines already at that value). Raw Accel has no Windows default.
pub fn windows_defaults<O: MouseOs>(m: &Mouse<O>) -> Vec<DefaultItem> {
    let mut out = Vec::new();
    let mut push = |item: &str, now: Option<Val>, default: Val| {
        if let Some(now) = now {
            out.push(DefaultItem { item: item.into(), label: label(item).into(), now, default });
        }
    };
    let epp = item_val(m, "epp");
    // precision on with other numbers than Windows' 6,10,1 is still "On": left as it is
    let epp_def = match &epp {
        Some(v) if v.text == "On" => v.clone(),
        _ => win_val(WinSetting::Precision, WinRaw::Mouse(bu_mouse::settings::PRECISION_ON)),
    };
    push("speed", item_val(m, "speed"), win_val(WinSetting::PointerSpeed, WinRaw::Num(10)));
    push("epp", epp, epp_def);
    push("scroll", item_val(m, "scroll"), win_val(WinSetting::ScrollLines, WinRaw::Num(3)));
    push("dblclick", item_val(m, "dblclick"), win_val(WinSetting::DoubleClick, WinRaw::Num(500)));
    push("swap", item_val(m, "swap"), win_val(WinSetting::SwapButtons, WinRaw::Bool(false)));
    let cur = item_val(m, "cursors");
    // every bubble already Windows' own: no line (the other roles are never changed by a pick)
    let all_default = m.cursors().map(|c| c.roles.iter().all(|r| r.set == SetId::WindowsDefault)).unwrap_or(false);
    let cur_def = match (&cur, all_default, m.windows_default_look_text()) {
        (Some(v), true, _) => Some(v.clone()),
        (_, false, Ok(t)) => Some(Val::new(&t, "Windows default")),
        _ => None,
    };
    if let Some(d) = cur_def {
        push("cursors", cur, d);
    }
    push("cursor_size", item_val(m, "cursor_size"), Val::new("32,1", "1"));
    out
}

/// Puts one item to a value from the change log (`Val::raw`), with no earlier state (a fresh service works the same).
pub fn restore<O: MouseOs>(m: &mut Mouse<O>, item: &str, raw: &str) -> Result<(), String> {
    let r = if let Some(s) = win_setting(item) {
        match WinRaw::from_text(s, raw) {
            Some(v) => m.restore_windows(s, v),
            None => return Err(format!("{} · unreadable value", label(item))),
        }
    } else {
        match item {
            "cursors" => m.restore_cursor_look(raw),
            "cursor_size" => m.restore_cursor_size(raw),
            "accel" => {
                let r = m.restore_driver_state(std::path::Path::new(raw));
                // the always-on engine reads the card again (it is saved OFF now): a game start must not set the old curve back
                #[cfg(windows)]
                if r.is_ok() {
                    super::rt::reload_soon();
                }
                r
            }
            _ => return Err("Unknown setting".into()),
        }
    };
    r.map_err(|e| e.to_string())
}

/// The items a command may change on the PC (their values are compared before / after it).
fn watched(c: &Cmd) -> &'static [&'static str] {
    match c {
        Cmd::Speed(_) => &["speed"],
        Cmd::Precision(_) => &["epp"],
        Cmd::Lines(_) => &["scroll"],
        Cmd::DoubleClick(_) => &["dblclick"],
        Cmd::Swap(_) => &["swap"],
        Cmd::CursorRole(..) | Cmd::RoleFile(..) | Cmd::DeletePack(_) => &["cursors"],
        Cmd::CursorSize(_) => &["cursor_size"],
        Cmd::Accel(_) | Cmd::AccelOn(_) | Cmd::UseOurs | Cmd::CopyCurve => &["accel"],
        // the mouse's own memory (not Windows), hover previews (Windows reloads them), imports (the app's own folder),
        // the change log's own resets (the frame records those)
        _ => &[],
    }
}

/// A closed tab's service for the change log (Settings › Reset, the uninstaller's undo): the FAKE sample PC in test
/// copies, else the real Windows layer (read-only in a `--real-read` copy). Nothing is read until asked.
pub trait Restorer {
    fn val(&self, item: &str) -> Option<Val>;
    fn defaults(&self) -> Vec<DefaultItem>;
    fn restore(&mut self, item: &str, raw: &str) -> Result<(), String>;
}

impl<O: MouseOs> Restorer for Mouse<O> {
    fn val(&self, item: &str) -> Option<Val> {
        item_val(self, item)
    }
    fn defaults(&self) -> Vec<DefaultItem> {
        windows_defaults(self)
    }
    fn restore(&mut self, item: &str, raw: &str) -> Result<(), String> {
        restore(self, item, raw)
    }
}

pub fn restorer() -> Box<dyn Restorer + Send> {
    // (unit tests: always the fake - a closed tab is asked by tests of other parts too)
    if crate::testmode::on() || cfg!(test) {
        return Box::new(SendFake(sample_fake()));
    }
    #[cfg(windows)]
    let m: Box<dyn Restorer + Send> = Box::new(real_bare(crate::testmode::real_read()));
    #[cfg(not(windows))]
    let m: Box<dyn Restorer + Send> = Box::new(SendFake(sample_fake()));
    m
}

/// Order 047: the closed tab's FAKE service may be used on the review's worker thread (a detached reset copy).
struct SendFake(Mouse<FakeOs>);
// SAFETY: `FakeOs` is not `Send` only because its fake mice are `Box<dyn FnMut>` with no `Send` bound; the sample PC's
// one mouse (`sample_fake_with`) captures only its `CmouseModel` (plain data). One thread uses it at a time (the page
// keeps it behind a `Mutex`).
unsafe impl Send for SendFake {}

impl Restorer for SendFake {
    fn val(&self, item: &str) -> Option<Val> {
        Restorer::val(&self.0, item)
    }
    fn defaults(&self) -> Vec<DefaultItem> {
        Restorer::defaults(&self.0)
    }
    fn restore(&mut self, item: &str, raw: &str) -> Result<(), String> {
        Restorer::restore(&mut self.0, item, raw)
    }
}

thread_local! {
    /// the FAKE worker (test copies): the cursor picker lists the drawing's sets - Glass and an imported "Neon Pack" - without
    /// any cursor files (a test copy never writes the Glass files into a folder; a fake has no folder of packs)
    static DRAWING_SETS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The drawing's imported pack (its `CSETS` "Neon Pack": a cursor for every role).
fn neon_pack() -> bu_mouse::cursors::Pack {
    let roles = bu_mouse::cursors::WinRole::ALL.iter().map(|r| (*r, format!("{}.cur", r.reg_name().to_ascii_lowercase()))).collect();
    bu_mouse::cursors::Pack { name: "Neon Pack".into(), roles, files: Vec::new() }
}

fn view<O: MouseOs>(m: &mut Mouse<O>, mice: Option<Vec<YourMouse>>, on: Option<OnMouse>) -> View {
    let drawing_sets = DRAWING_SETS.with(|d| d.get());
    let active = games();
    // the games that run now are the engine's to know; the tab's copy of the card counts them too (header, "another program wrote it")
    m.accel_mut().per_app.set_active_rows(&active);
    let m = &*m;
    let other = m.other_writer().ok().flatten();
    let base = m.rawaccel_settings().ok().flatten().and_then(|c| c.profiles.first().cloned()).unwrap_or_default();
    View {
        win: m.windows_mouse().ok(),
        mice,
        on_mouse: on,
        ra: m.rawaccel_status().ok(),
        panel: m.accel().panel.clone(),
        per_app: m.accel().per_app.clone(),
        base,
        other_writer: other.as_ref().map(|w| w.line.clone()),
        use_ours: other.as_ref().is_some_and(|w| w.use_ours),
        active,
        cursors: m.cursors().ok(),
        packs: if drawing_sets { vec![neon_pack()] } else { m.packs().unwrap_or_default() },
        glass: drawing_sets || !m.glass_set().is_empty(),
        // (a test copy shows the drawing's sets only)
        schemes: if drawing_sets { Vec::new() } else { m.installed_schemes().unwrap_or_default() },
        suggest: Role::ALL.iter().map(|r| m.suggestion(*r).ok().flatten()).collect(),
        files: if drawing_sets { Vec::new() } else { m.set_preview_files() },
        own_files: if drawing_sets { Vec::new() } else { m.own_files().into_iter().map(|p| p.to_string_lossy().into_owned()).collect() },
        elevated: m.os().is_elevated(),
        vals: item_vals(m),
        win_def: windows_defaults(m),
    }
}

/// How often the open tab looks at Windows' device list for a mouse plugged in / unplugged.
pub const RELIST_MS: u64 = 1500;

/// The same mice (by USB id and interface) - the list did not change.
pub fn same_mice(a: &[YourMouse], b: &[YourMouse]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x.vid, x.pid, &x.config_path) == (y.vid, y.pid, &y.config_path))
}

/// What the first ("Your mouse") supported mouse reports; a mouse that just woke gets ~3 s to link (ReadOptions default).
fn read_first<O: MouseOs>(m: &mut Mouse<O>, mice: &[YourMouse]) -> Option<OnMouse> {
    match mice.first() {
        Some(y) if y.protocol.is_some() => m.read_on_mouse(y, ReadOptions::default()).ok(),
        _ => None,
    }
}

/// `accel_logged`: the change log already keeps "Mouse acceleration" (no copy of the driver's settings is needed);
/// `note_if_gone`: a change whose answer finds the page gone is noted into the change log from here (the real PC only).
fn run<O: MouseOs>(mut m: Mouse<O>, rx: Receiver<Cmd>, tx: Sender<Reply>, mut accel_logged: bool, note_if_gone: bool) {
    #[cfg(windows)]
    TAB_HAS_ENGINE.with(|e| e.set(note_if_gone && super::rt::running()));
    // the saved card comes back (Order 063: the switch, presets and per-game rows are kept in a file); only when nothing is
    // saved yet (the first run) the card mirrors what the installed Raw Accel runs - it changes nothing on the driver
    if let Ok(false) = m.load_accel() {
        // (the mirror is not saved: the card is the user's only once they change it - a card they never turned on must not
        // write the driver at the next start)
        let _ = m.start_from_rawaccel();
    }
    // every answer wakes an idle menu (the page shows it at once, not at the next mouse move)
    let send = |r: Reply| {
        let ok = tx.send(r).is_ok();
        crate::services::Waker.wake();
        ok
    };
    if !send(Reply { view: view(&mut m, None, None), toast: None, reading_mouse: true, unasked: false, changes: Vec::new() }) {
        return;
    }
    let mut list = m.mice().unwrap_or_default();
    let mut on = read_first(&mut m, &list);
    let mut mice = Some(list.clone());
    if !send(Reply { view: view(&mut m, mice.clone(), on.clone()), toast: None, reading_mouse: false, unasked: false, changes: Vec::new() }) {
        return;
    }
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(RELIST_MS)) {
            Ok(c) => {
                // the change log: the values the command may change, before and after it
                let items = watched(&c);
                let before: Vec<(&str, Option<Val>)> = items.iter().map(|i| (*i, item_val(&m, i))).collect();
                let driver_before = if items.contains(&"accel") && !accel_logged { m.driver_state().ok().flatten() } else { None };
                // (a detached reset copy asked: the page did not count it as pending)
                let away = matches!(c, Cmd::RestoreAway(..));
                let toast = apply(&mut m, &list, &mut on, c);
                let mut changes = Vec::new();
                for (i, old) in before {
                    let (Some(old), Some(new)) = (old, item_val(&m, i)) else { continue };
                    if old.raw == new.raw {
                        continue;
                    }
                    if i == "accel" {
                        // the driver's settings before the app's first change: kept as a file (the old value names it)
                        if let Some(b) = &driver_before {
                            if m.keep_driver_state(b).is_ok() {
                                accel_logged = true;
                            }
                        }
                    }
                    changes.push((i.to_string(), label(i).to_string(), old, new));
                }
                if !send(Reply { view: view(&mut m, mice.clone(), on.clone()), toast, reading_mouse: false, unasked: away, changes: changes.clone() }) {
                    if note_if_gone {
                        for (i, l, o, n) in &changes {
                            crate::undo::note(PAGE, i, l, o, n);
                        }
                    }
                    return;
                }
            }
            // while the tab is open: a mouse plugged in / unplugged (or its cable / dongle swapped) shows within RELIST_MS.
            // Only Windows' device list is read (nothing is sent to any mouse unless the list changed).
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let now = m.mice().unwrap_or_default();
                if !same_mice(&now, &list) {
                    list = now;
                    on = read_first(&mut m, &list);
                    mice = Some(list.clone());
                    if !send(Reply { view: view(&mut m, mice.clone(), on.clone()), toast: None, reading_mouse: false, unasked: true, changes: Vec::new() }) {
                        return;
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = m.end_preview();
}

/// Does one command; returns its toast (the drawing's words) or the error as a toast.
fn apply<O: MouseOs>(m: &mut Mouse<O>, mice: &[YourMouse], on: &mut Option<OnMouse>, c: Cmd) -> Option<String> {
    let err = |e: bu_mouse::Error| Some(e.to_string());
    match c {
        Cmd::Speed(v) => m.set_pointer_speed(v).err().and_then(err),
        Cmd::Precision(v) => m.set_precision(v).err().and_then(err),
        Cmd::Lines(v) => m.set_scroll_lines(v).err().and_then(err),
        Cmd::DoubleClick(s) => m.set_double_click_step(s).err().and_then(err),
        Cmd::Swap(v) => match m.set_buttons_swapped(v) {
            Ok(t) => Some(t.to_string()),
            Err(e) => err(e),
        },
        Cmd::Dpi(v) | Cmd::Polling(v) | Cmd::LiftOff(v) => {
            let Some(y) = mice.first() else { return Some("No mouse found".into()) };
            let link = on.as_ref().map(|o| o.link).unwrap_or(Link::from_code(5));
            let r = match c {
                Cmd::Dpi(_) => m.set_dpi(y, v).map(Some),
                Cmd::Polling(_) => m.set_polling(y, v, link).map(|_| Some(format!("Polling {v} Hz · saved on the mouse"))),
                _ => m.set_lift_off(y, v).map(|_| None),
            };
            match r {
                Ok(t) => {
                    if let Ok(o) = m.read_on_mouse(y, ReadOptions::default()) {
                        *on = Some(o);
                    }
                    t
                }
                Err(e) => err(e),
            }
        }
        Cmd::Accel(b) => {
            let (panel, per_app) = *b;
            let a = m.accel_mut();
            a.panel = panel;
            a.per_app = per_app;
            persist_apply(m, false)
        }
        Cmd::AccelOn(v) if engine_on() => {
            let a = m.accel_mut();
            a.panel.on = v;
            a.panel.expanded = v;
            persist_apply(m, true).or_else(|| Some(m.accel_on_toast(v)))
        }
        Cmd::AccelOn(v) => match m.set_accel_on(v) {
            Ok(t) => Some(t),
            Err(e) => err(e),
        },
        Cmd::UseOurs => persist_apply(m, true).or_else(|| Some("Set again \u{b7} the driver runs this card".to_string())),
        Cmd::CopyCurve => match m.copy_its_curve() {
            Ok(t) => {
                let _ = persist_apply(m, false);
                Some(t)
            }
            Err(e) => err(e),
        },
        Cmd::CursorRole(r, s) => m.set_role(r, s).err().and_then(err),
        Cmd::CursorSize(n) => m.set_cursor_size(n).err().and_then(err),
        Cmd::DeletePack(n) => match m.delete_pack(&n) {
            Ok(was) => Some(format!("{n} deleted{}", if was.is_empty() { "" } else { " · back to Windows default" })),
            Err(e) => err(e),
        },
        Cmd::RoleFile(r, f) => match m.set_role_file(r, &f) {
            Ok(_) => Some(format!("{} · your file", r.name())),
            Err(e) => err(e),
        },
        Cmd::Import(ps) => match m.import_cursors(&ps) {
            Ok((_, t, skipped)) if skipped.is_empty() => Some(t),
            Ok((_, t, skipped)) => Some(format!("{t} · skipped {}", skipped.join(", "))),
            Err(e) => err(e),
        },
        Cmd::Preview(r, s) => m.preview_cursor(r, &s).err().and_then(err),
        Cmd::InstallStore(name, path) => {
            let r = std::fs::read(&path).map_err(|e| format!("couldn’t read the download: {e}")).and_then(|b| m.install_store_zip(&name, &b).map_err(|e| e.to_string()));
            // the .zip was only the way here
            let _ = std::fs::remove_file(&path);
            match r {
                Ok(_) => Some(format!("{name} installed · pick it in any cursor's list")),
                Err(e) => Some(format!("{name} was not installed: {e}")),
            }
        }
        Cmd::EndPreview => m.end_preview().err().and_then(err),
        Cmd::Reread => {
            // a reset (it ran on another service) may have switched the saved card off: read it again
            if engine_on() {
                let _ = m.load_accel();
            }
            None
        }
        Cmd::Restore(item, to, back) | Cmd::RestoreAway(item, to, back) => {
            // the frame shows the outcome (its own toast); the page's view follows from this answer
            let _ = back.send(restore(m, &item, &to.raw));
            None
        }
    }
}

/// Is the always-on engine running (the real app)? Then it owns the driver: the tab only saves the card and asks it.
fn engine_on() -> bool {
    TAB_HAS_ENGINE.with(|e| e.get())
}

thread_local! {
    /// this worker belongs to the real app with the engine running (set once in `run`; the FAKE workers of the test copies never
    /// have it, whatever else runs in the same process)
    static TAB_HAS_ENGINE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A change of the card: with the engine, saved to `accel.json` and handed to the engine (it knows which games run now); else
/// (test copies on the fake, no APPDATA) straight to the driver. `switched` = the header switch itself was clicked
/// (only then a card that is OFF writes the driver: tuning a card that is off never touches what Raw Accel runs).
fn persist_apply<O: MouseOs>(m: &mut Mouse<O>, switched: bool) -> Option<String> {
    #[cfg(windows)]
    if engine_on() {
        let saved = m.save_accel().err().map(|e| format!("Couldn\u{2019}t save the acceleration settings: {e}"));
        return super::rt::apply_now(switched).or(saved);
    }
    sync(m)
}

/// Hands the card's state to the driver (only when Raw Accel runs and something changed).
fn sync<O: MouseOs>(m: &mut Mouse<O>) -> Option<String> {
    if !matches!(m.rawaccel_status(), Ok(RawAccelStatus::Installed { .. })) {
        return None;
    }
    m.sync_driver().err().map(|e| e.to_string())
}

/// The always-on engine's view of the games (rows whose game runs now and was switched to).
fn games() -> Vec<bu_mouse::accel::switch::RowId> {
    #[cfg(windows)]
    {
        super::rt::status().active
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

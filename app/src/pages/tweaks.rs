//! The Tweaks tab (menu-v22 page `tgl`, "Toggles" renamed in v21), Order 021: the switch groups (bu-toggles), the
//! Fullscreen optimizations games window (v22), Quick fixes as the last switch group (bu-quickfix), Default apps at the
//! very bottom, the search field, the folding group headers and the reset line. Every switch shows what Windows is set to
//! NOW (read on open: registry / power / SPI reads, no scan), every change is read back by the crate.

mod qf;
mod rows;
mod svc;
pub mod temp;

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};

use bu_toggles::defaults::DefaultApps;
use bu_toggles::{Applied, Badge, Error as TErr, RowState, Timeout, Value, TIMEOUT_CHOICES};
use taffy::style::JustifyContent;

use crate::anim::{Bezier, EASE};
use crate::gfx::{Align, Font, Rgba};
use crate::pages::{Env, Page};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, lh, sub, Cursor, El, Key};
use crate::ui::pieces::mitems::{self, It, Place, Row};
use crate::ui::pieces::tip::{self, Rq};
use crate::ui::pieces::{self, bits, card, dialog, dropdown, group, inote, link, reset, rowbits, search, toast, toggle};
use crate::ui::{cmix, ACC, FG, FG2, FG3, GRP, HAIR, HOV};
use crate::undo::{DefaultItem, Resettable, Val};

use rows::{GROUPS, ROWS};
use svc::Svc;

/// the sub-line words of a row that still restarts Explorer (Order 043)
const RESTARTS: &str = "Explorer restarts for a moment";

const K_SEARCH: Key = key("tgl.search");
const K_GH: Key = key("tgl.gh");
const K_TG: Key = key("tgl.tg");
const K_TIP: Key = key("tgl.tip");
const K_TO: Key = key("tgl.to");
const K_TMENU: Key = key("tgl.tmenu");
const K_FSO: Key = key("tgl.fso");
const K_FDLG: Key = key("tgl.fdlg");
const K_FROW: Key = key("tgl.frow");
const K_FTG: Key = key("tgl.ftg");
const K_FDEL: Key = key("tgl.fdel");
const K_FADD: Key = key("tgl.fadd");
const K_DPK: Key = key("tgl.dpk");
const K_BMENU: Key = key("tgl.bmenu");
const K_ASK: Key = key("tgl.ask");
const K_RS: Key = key("tgl.rs");
const K_TOAST: Key = key("tgl.toast");

/// The page's content width (`.pg` 600 - 2 × 26 padding).
const W: f32 = 548.0;
/// Groups: the 8 switch groups, then Quick fixes (8), Default apps (9).
const G_QF: usize = 8;
const G_DEF: usize = 9;
const FOLD: Bezier = Bezier::new(0.3, 0.7, 0.2, 1.0);

/// The drawing's time list (`TIMES`): seconds, label; the `['-']` separator sits before Never.
/// The menu row of TIMES[n]: the separator sits before the last one ("Never").
fn time_row(n: usize) -> usize {
    if n + 1 == TIMES.len() {
        n + 1
    } else {
        n
    }
}

/// The browser list under "Change" (`openMenu` rows with the app tile `.mitem .at{margin-right:2px}` after the ✓ column).
/// Page-local: the shared `mitems` rows have no `.at` lead. Item i = `Ev::Click(idx(K_BMENU, i))`, placed like `placeMenu`.
fn browser_menu(cx: &mut Cx, bs: &[bu_toggles::defaults::Browser], x: f32, y: f32) -> El {
    let mut rows = Vec::new();
    for (n, b) in bs.iter().enumerate() {
        let k = idx(K_BMENU, n);
        let on = cx.hovered(k);
        let col = if on { crate::ui::WHITE } else { FG() };
        let ck = El::text(if b.is_current { "\u{2713}" } else { "" }, Font::new(12.0, 400), col, lh(12.0, 1.35)).w(14.0).none().align(Align::Center);
        let mut r = El::row()
            .center()
            .gap(6.0)
            .h(26.0)
            .pad(0.0, 14.0, 0.0, 6.0)
            .radius(5.0)
            .child(ck)
            .child(app_tile(&b.name).margin(0.0, 2.0, 0.0, 0.0))
            .child(El::text(b.name.clone(), Font::new(13.0, 400), col, lh(13.0, 1.35)))
            .on_click(k)
            .cursor(Cursor::Hand);
        if on {
            r = r.bg(ACC());
        }
        rows.push(r);
    }
    let est = 10.0 + 26.0 * bs.len() as f32;
    pieces::dropdown::menu_box(cx, K_BMENU, x, y, 150.0, est, 300.0, rows)
}

const TIMES: [(u32, &str); 10] = [
    (60, "1 min"),
    (120, "2 min"),
    (300, "5 min"),
    (600, "10 min"),
    (900, "15 min"),
    (1800, "30 min"),
    (3600, "1 hour"),
    (7200, "2 hours"),
    (18000, "5 hours"),
    (0, "Never"),
];

/// What the drawing toasts after a switch flips (`TGT`: [off, on]) where the crate has no toast of its own.
const TGT: [(&str, &str, &str); 6] = [
    ("altsh", "Alt + Shift no longer switches your keyboard layout", "Alt + Shift switches your keyboard layout again"),
    ("sleep", "The PC never sleeps by itself now", "Sleep is on again"),
    ("fast", "Fast Startup off · every Shut down is a clean start", "Fast Startup on"),
    ("widgets", "Widgets are gone from the taskbar", "Widgets are back on the taskbar"),
    ("copilot", "Copilot is off", "Copilot is on"),
    ("togk", "Holding Num Lock no longer asks about Toggle Keys", "The Toggle Keys pop-up is back"),
];

/// Windows' own value per row for "Windows defaults" (Windows 11 24H2 out of the box, desktop PC). GUESSED from Microsoft's
/// documentation / a fresh install where noted nowhere else; None = no single factory value (hardware, the app's own
/// feature, or unclear) — such rows are never in the Windows defaults list.
const WIN_DEFAULT: [(&str, bool); 37] = [
    ("ext", false), ("hid", false), ("ctx", false), ("thispc", false), ("copy", false), ("odads", true),
    ("endtask", false), ("secs", false), ("left", false), ("tview", true), ("recs", true), ("recsec", true), ("flash", true),
    ("gmode", true), ("xbtn", true), ("ahdr", false),
    ("altsh", false), ("sticky", true), ("filter", true), ("togk", true), ("clip", false), ("scroll", true), ("prtsnip", true),
    ("usbss", true), ("sleep", true), ("fast", true),
    ("duck", true), ("mono", false), ("micacc", true), ("camacc", true), ("bing", true), ("widgets", true),
    ("lock", true), ("setads", true), ("tips", true), ("transp", true), ("anim", true),
];

/// One game of the Fullscreen optimizations window (the window keeps the games switched back on, Windows forgets them).
#[derive(Clone, Debug, PartialEq)]
struct Game {
    exe: String,
    off: bool,
}

enum Pop {
    /// a time list under its button (row index, the button's box)
    Times(usize, (f32, f32, f32, f32)),
    /// the browser list under "Change"
    Browsers(f32, f32),
    /// the games window, opened at
    Games(f64),
    /// "Rebuild icon & thumbnail cache?" by its button
    Ask(f32, f32),
}

/// Order 047: one change-log line a worker job made (written by the page when its answer lands, or by the worker itself
/// when the page has closed meanwhile).
struct Log {
    item: String,
    label: String,
    old: Val,
    new: Val,
}

/// The games window's change, as clicked (the exe; the list is matched by it when the answer lands).
enum GameAct {
    /// "1 game" opened the window: Windows' list read again
    List,
    Add(String),
    /// the game's switch: optimizations off = true
    Flip(String, bool),
    Remove(String),
}

/// Order 047: what the page's worker thread answers (`Tweaks::run`). Every crate call of the open page runs there - an
/// Explorer restart (1-6 s), a settings broadcast to every window, Settings / "Open with" opening, the open's read of
/// ~50 rows + the games + the default apps - never on the menu's thread.
enum Done {
    /// every row, the games Windows has, the default apps (`fresh`: the tab's open read - the games list starts over)
    All { st: Vec<Option<RowState>>, games: Vec<Game>, defaults: Option<Box<DefaultApps>>, fresh: bool },
    /// a switch / a time on row i: its answer, its group read back, the change-log lines, the time picked
    Row { i: usize, a: Result<Applied, TErr>, group: Vec<(usize, Option<RowState>)>, logs: Vec<Log>, secs: Option<u32> },
    /// the games window: its answer, Windows' games list after it (when read)
    Games { act: GameAct, r: Result<(), TErr>, real: Option<Vec<Game>>, logs: Vec<Log> },
    /// Windows' Settings / "Open with" was opened (nothing to show)
    Opened,
}

type Job = Box<dyn FnOnce(&mut Svc) -> Done + Send>;

/// The last read the tab showed (`env.keep`, Order 047): shown at once when the tab opens again, while the fresh read runs.
#[derive(Clone)]
struct Snap {
    st: Vec<Option<RowState>>,
    games: Vec<Game>,
    defaults: Option<DefaultApps>,
}
const KEEP: &str = "tgl.read";

fn lock(s: &Mutex<Svc>) -> MutexGuard<'_, Svc> {
    s.lock().unwrap_or_else(|p| p.into_inner())
}

/// Write a job's change-log lines (the ONE change log, Order 036) under this page.
fn note_logs(logs: &[Log]) {
    for l in logs {
        crate::undo::note("tgl", &l.item, &l.label, &l.old, &l.new);
    }
}

/// The page's worker: one thread for the open page (COM ready, as the shell / WinRT calls want), its jobs one after
/// another in click order on the page's ONE service (it remembers e.g. Sleep's time for switching Sleep on again). It
/// ends when the page drops its sender (close); an answer the closed page can't take still writes its change-log lines.
fn start_worker(svc: Arc<Mutex<Svc>>, slow: u64) -> (Sender<Job>, Receiver<Done>) {
    let (jtx, jrx) = channel::<Job>();
    let (dtx, drx) = channel::<Done>();
    let _ = std::thread::Builder::new().name("bu-tweaks".into()).spawn(move || {
        #[cfg(windows)]
        let com = unsafe {
            windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED | windows::Win32::System::Com::COINIT_DISABLE_OLE1DDE).is_ok()
        };
        for job in jrx {
            if slow > 0 {
                // test copies only: a stand-in for a slow Windows call
                std::thread::sleep(std::time::Duration::from_millis(slow));
            }
            let d = job(&mut lock(&svc));
            if let Err(e) = dtx.send(d) {
                if let Done::Row { logs, .. } | Done::Games { logs, .. } = &e.0 {
                    note_logs(logs);
                }
            }
            crate::services::Waker.wake();
        }
        #[cfg(windows)]
        if com {
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    });
    (jtx, drx)
}

/// Row i's value for the change log (None: a row the log doesn't keep, `NO_LOG`).
fn log_of(s: &Svc, i: usize) -> Option<Val> {
    if NO_LOG.contains(&ROWS[i].id) {
        return None;
    }
    row_val(s, i)
}

/// Row i was changed: its change-log line, old -> its value read back now (none when nothing changed).
fn push_log(logs: &mut Vec<Log>, s: &Svc, i: usize, old: Option<Val>) {
    if let (Some(old), Some(new)) = (old, log_of(s, i)) {
        if old != new {
            logs.push(Log { item: ROWS[i].id.into(), label: ROWS[i].title.into(), old, new });
        }
    }
}

/// A switch's / a time's end (worker): row i's group read back, then the Settings page the crate asks for opened.
fn finish(s: &mut Svc, i: usize, a: Result<Applied, TErr>, logs: Vec<Log>, secs: Option<u32>) -> Done {
    let g = ROWS[i].group;
    let group = ROWS.iter().enumerate().filter(|(_, r)| r.group == g).map(|(j, r)| (j, s.read(r.crate_id).ok())).collect();
    if let Ok(Some(act)) = a.as_ref().map(|a| a.open.clone()) {
        let _ = s.open(&act);
    }
    Done::Row { i, a, group, logs, secs }
}

/// The games Windows has (worker).
fn games_of(s: &Svc) -> Vec<Game> {
    s.fso_games().unwrap_or_default().into_iter().map(|g| Game { exe: g.exe, off: g.fso_off }).collect()
}

/// A game's Fullscreen optimizations flag set (`off` = optimizations off) / removed (worker); the change-log line is
/// `fso:<exe>` with the state before.
fn fso_change(s: &mut Svc, exe: &str, off: bool, logs: &mut Vec<Log>) -> Result<(), TErr> {
    let old = s.fso_state(exe).ok();
    let r = s.fso_set(exe, off).map(|_| ());
    if let (Ok(_), Some(old)) = (&r, old) {
        if old != off {
            logs.push(Log { item: format!("{FSO_ITEM}{exe}"), label: fso_label(exe), old: fso_val(old), new: fso_val(off) });
        }
    }
    r
}

pub struct Tweaks {
    query: String,
    /// the page's ONE crate service; the worker (`work`) uses it, the menu's thread only for the reset review
    svc: Option<Arc<Mutex<Svc>>>,
    /// Order 047: the worker's job line and its answers; jobs sent and not answered yet
    work: Option<Sender<Job>>,
    done: Option<Receiver<Done>>,
    pending: usize,
    /// rows whose switch / time waits for the worker (their clicks wait too)
    wait: Vec<usize>,
    /// the admin row whose switch is dimmed until its answer has landed (DESIGN "Admin flip")
    dim: Option<usize>,
    /// a games window change waits for the worker (its clicks wait too)
    games_wait: usize,
    /// the real Windows (not a test copy's fake)
    real: bool,
    keep: crate::keep::Keep,
    /// test copies only: every worker job first sleeps this long (ms) - a stand-in for a slow Windows call
    slow: u64,
    test: bool,
    st: Vec<Option<RowState>>,
    games: Vec<Game>,
    picks: Vec<String>,
    defaults: Option<DefaultApps>,
    qf: Option<qf::Qf>,
    /// folded groups (`G.open = false`)
    shut: [bool; 10],
    pop: Option<Pop>,
    toast: Option<(String, f64)>,
    /// the box of the last pressed element (popups open under it)
    pressed: (f32, f32, f32, f32),
    /// a test copy that reads the real Windows but changes nothing (measure.ps1)
    real_read: bool,
    /// the crate service for the reset line while the page is CLOSED (Settings › Reset, the uninstaller): built on the
    /// first `current` / `apply` / `windows_defaults` that needs it (Order 036 addendum: `resettable()` stays cheap)
    rs: std::cell::RefCell<Option<Arc<Mutex<Svc>>>>,
    /// an admin row's switch waiting for Windows' admin prompt / the elevated copy (Order 039): row, its answer
    admin_wait: Option<(usize, std::sync::mpsc::Receiver<Result<Applied, TErr>>)>,
}

impl Default for Tweaks {
    fn default() -> Self {
        Tweaks {
            query: String::new(),
            svc: None,
            work: None,
            done: None,
            pending: 0,
            wait: Vec::new(),
            dim: None,
            games_wait: 0,
            real: false,
            keep: crate::keep::Keep::default(),
            slow: 0,
            test: false,
            st: Vec::new(),
            games: Vec::new(),
            picks: Vec::new(),
            defaults: None,
            qf: None,
            shut: [false; 10],
            pop: None,
            toast: None,
            pressed: (0.0, 0.0, 0.0, 0.0),
            real_read: false,
            rs: std::cell::RefCell::new(None),
            admin_wait: None,
        }
    }
}

fn row_ix(id: &str) -> Option<usize> {
    ROWS.iter().position(|r| r.id == id)
}

/// "photo.exe" from a full path.
fn file_name(exe: &str) -> &str {
    exe.rsplit(['\\', '/']).next().unwrap_or(exe)
}

/// A game's name and tile. The drawing's games (FSO, FSO_PICK) by exe; any other: the exe's name and a grey tile.
/// TEMP: the exe's own description + icon need `appinfo` (PIECES_WANTED "appinfo::icon_pixels").
fn game_look(exe: &str) -> (String, &'static str, Rgba, Rgba) {
    let f = file_name(exe).to_lowercase();
    match f.as_str() {
        "rocketleague.exe" => ("Rocket League".into(), "pad", Rgba::hex(0x5ab4ff), Rgba::hex(0x2a5fd6)),
        "cs2.exe" => ("Counter-Strike 2".into(), "aim", Rgba::hex(0xffb04a), Rgba::hex(0xe0701c)),
        "fortniteclient-win64-shipping.exe" => ("Fortnite".into(), "pad", Rgba::hex(0x9f8cff), Rgba::hex(0x5a3fe0)),
        _ => {
            let n = file_name(exe);
            let stem = n.strip_suffix(".exe").or_else(|| n.strip_suffix(".EXE")).unwrap_or(n);
            (stem.to_string(), "pad", Rgba::hex(0x8a8f99), Rgba::hex(0x3d4149))
        }
    }
}

/// The drawing's app tiles (`DAPPS`) for the Default apps list, by the app's name; others: a grey document tile.
fn app_tile(name: &str) -> El {
    let (g, a, b) = match name {
        "Chrome" | "Google Chrome" => ("globe", 0xffd35a, 0xe6493b),
        "Firefox" => ("globe", 0xffb04a, 0xe8541c),
        "Edge" | "Microsoft Edge" => ("globe", 0x4fd3a8, 0x2a76e8),
        "Brave" => ("globe", 0xff8a5a, 0xd9401c),
        "Opera GX" => ("globe", 0xff6b8a, 0xc4134f),
        "Photos" => ("img", 0x5ab4ff, 0x2a74e6),
        "Paint" => ("dpen", 0xffc56b, 0xe0861c),
        "Media Player" => ("mplay", 0xff8fb6, 0xd9467e),
        "VLC" | "VLC media player" => ("mplay", 0xffb04a, 0xe8701c),
        "Acrobat Reader" => ("doc", 0xff7a76, 0xc4313f),
        "Notepad" => ("mtxt", 0x7fb2ff, 0x3b6fd6),
        "VS Code" | "Visual Studio Code" => ("doc", 0x5ab4ff, 0x1f6fd0),
        "Notepad++" => ("mtxt", 0x9be15d, 0x4a9a1c),
        "File Explorer" | "Windows Explorer" => ("fold", 0xffd35a, 0xe8a33a),
        "7-Zip" => ("zip", 0xa2abbd, 0x6c7487),
        "WinRAR" => ("zip", 0xb58cff, 0x6f4ae0),
        _ => ("doc", 0xa2abbd, 0x6c7487),
    };
    bits::at(g, Rgba::hex(a), Rgba::hex(b), false)
}

fn on_text(v: &Value) -> String {
    match v {
        Value::Switch(true) => "On".into(),
        Value::Switch(false) => "Off".into(),
        Value::Timeout(t) => time_label(*t).into(),
        Value::Games(g) => format!("{} games", g.len()),
    }
}

fn time_label(t: Timeout) -> &'static str {
    TIMES.iter().find(|(s, _)| *s == t.seconds()).map(|x| x.1).unwrap_or("—")
}

// ---------------------------------------------------------------- the reset line (Order 036)

/// Rows the change log doesn't keep: Copilot (off = the app is uninstalled, a one-time action; on = Windows'
/// Store page, nothing changes by itself), Fullscreen optimizations (one entry per game instead, `fso:<exe>`).
const NO_LOG: [&str; 2] = ["copilot", "fso"];
/// The item of one game's Fullscreen optimizations flag: `fso:<the exe's full path>`.
const FSO_ITEM: &str = "fso:";

fn sw_val(on: bool) -> Val {
    if on {
        Val::new("on", "On")
    } else {
        Val::new("off", "Off")
    }
}

/// A game's flag: raw "off" = Fullscreen optimizations off for it (our flag set), "on" = Windows' normal.
fn fso_val(off: bool) -> Val {
    sw_val(!off)
}

fn fso_label(exe: &str) -> String {
    format!("Fullscreen optimizations · {}", game_look(exe).0)
}

/// "30 min", "Never"; a time not in the list (set elsewhere): "45 min", "1 h".
fn secs_label(s: u32) -> String {
    TIMES.iter().find(|(x, _)| *x == s).map(|x| x.1.to_string()).unwrap_or_else(|| Timeout::from_seconds(s).label())
}

/// Row i's value as the change log keeps it: a switch "on" / "off" ("On" / "Off"), a time its seconds ("10 min"). (Sleep
/// off also sets "Sleep after" to never: the page logs that row too, so a reset brings the time back with it.)
fn row_val(s: &Svc, i: usize) -> Option<Val> {
    Some(match s.read(ROWS[i].crate_id).ok()?.value {
        Value::Switch(on) => sw_val(on),
        // a time: plugged in AND on battery ("600,300"), so a laptop's own battery time comes back too
        Value::Timeout(t) => match s.power_values(ROWS[i].crate_id) {
            Ok(p) => Val::new(&format!("{},{}", p.ac, p.dc), &secs_label(p.ac)),
            Err(_) => Val::new(&t.seconds().to_string(), &secs_label(t.seconds())),
        },
        _ => return None,
    })
}

/// One item's value now (row id or `fso:<exe>`).
fn item_val(s: &Svc, item: &str) -> Option<Val> {
    match item.strip_prefix(FSO_ITEM) {
        Some(exe) => s.fso_state(exe).ok().map(fso_val),
        None => row_ix(item).filter(|i| !NO_LOG.contains(&ROWS[*i].id)).and_then(|i| row_val(s, i)),
    }
}

/// Put one item to a value through the crate (each change read back by it). Err = the reason, as the toast says it.
/// The reset applies items in name order: "sleep" comes back before its "sleepafter" time.
fn put(s: &mut Svc, item: &str, to: &Val) -> Result<(), String> {
    let e = |e: TErr| Tweaks::err_toast(&e);
    if let Some(exe) = item.strip_prefix(FSO_ITEM) {
        return s.fso_set(exe, to.raw == "off").map(|_| ()).map_err(e);
    }
    let i = row_ix(item).filter(|i| !NO_LOG.contains(&ROWS[*i].id)).ok_or("This setting isn’t known any more")?;
    let id = ROWS[i].crate_id;
    // already there (e.g. "Sleep after" = never once Sleep itself went back off): nothing to do
    if item_val(s, item).is_some_and(|v| v.raw == to.raw) {
        return Ok(());
    }
    if let Some((ac, dc)) = to.raw.split_once(',') {
        let (Ok(ac), Ok(dc)) = (ac.parse::<u32>(), dc.parse::<u32>()) else { return Err("This value isn’t known".into()) };
        return s.set_power_values(id, bu_toggles::os::PowerValues { ac, dc }).map(|_| ()).map_err(e);
    }
    match to.raw.as_str() {
        "on" => s.set(id, true).map(|_| ()).map_err(e),
        "off" => s.set(id, false).map(|_| ()).map_err(e),
        raw => {
            let secs = raw.parse::<u32>().map_err(|_| "This value isn’t known".to_string())?;
            s.set_seconds(id, secs).map(|_| ()).map_err(e)
        }
    }
}

/// The crate service the reset line uses while the page is closed: the FAKE one in a test copy (and in unit tests), the
/// read-only real one in a --real-read copy (its apply refuses first), else the real one.
fn fresh_svc() -> Option<Svc> {
    if cfg!(test) || (crate::testmode::on() && !crate::testmode::real_read()) {
        return Some(Svc::sample());
    }
    #[cfg(windows)]
    {
        Some(Svc::real(crate::testmode::real_read()))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

impl Tweaks {
    fn words(&self) -> Vec<String> {
        self.query.trim().to_lowercase().split_whitespace().map(String::from).collect()
    }

    fn show_toast(&mut self, t: impl Into<String>, now: f64) {
        self.toast = Some((t.into(), now));
    }

    /// Order 047: hand a job to the page's worker (its answer lands in `tick`).
    fn run(&mut self, job: impl FnOnce(&mut Svc) -> Done + Send + 'static) -> bool {
        let sent = self.work.as_ref().is_some_and(|w| w.send(Box::new(job)).is_ok());
        if sent {
            self.pending += 1;
        }
        sent
    }

    /// Read every row, the games and the default apps again (on the worker; `fresh` = the tab's open read).
    fn refresh(&mut self, fresh: bool) {
        self.run(move |s| Done::All { st: ROWS.iter().map(|r| s.read(r.crate_id).ok()).collect(), games: games_of(s), defaults: s.defaults().ok().map(Box::new), fresh });
    }

    /// Remember what the tab shows for its next open (`env.keep`): the rows, the games Windows has, the default apps.
    fn remember(&self) {
        if self.st.is_empty() {
            return;
        }
        let games = self.games.iter().filter(|g| g.off).cloned().collect();
        self.keep.put(KEEP, Snap { st: self.st.clone(), games, defaults: self.defaults.clone() });
    }

    /// A worker's answer: the page shows it (and says how it went, as it did when the call ran here).
    fn landed(&mut self, d: Done, now: f64) {
        match d {
            Done::All { st, games, defaults, fresh } => {
                self.st = st;
                if fresh {
                    self.games.clear();
                }
                self.merge_games(games);
                self.defaults = defaults.map(|d| *d);
            }
            Done::Row { i, a, group, logs, secs } => {
                self.wait.retain(|&j| j != i);
                if self.dim == Some(i) {
                    self.dim = None;
                }
                for (j, s) in group {
                    if let Some(x) = self.st.get_mut(j) {
                        *x = s;
                    }
                }
                note_logs(&logs);
                let ok = a.is_ok();
                self.after(i, a, now);
                if let (true, Some(secs)) = (ok, secs) {
                    // the drawing's own words (`cPopup` set): "The screen turns off after 10 min" / "The PC sleeps after 30 min"
                    let l = time_label(Timeout::from_seconds(secs));
                    let txt = match (ROWS[i].id, secs) {
                        ("scroff", 0) => "The screen stays on".to_string(),
                        ("scroff", _) => format!("The screen turns off after {l}"),
                        (_, 0) => "The PC never sleeps by itself now".to_string(),
                        _ => format!("The PC sleeps after {l}"),
                    };
                    self.show_toast(txt, now);
                }
            }
            Done::Games { act, r, real, logs } => {
                self.games_wait = self.games_wait.saturating_sub(1);
                note_logs(&logs);
                if let Some(v) = real {
                    self.merge_games(v);
                }
                match (act, r) {
                    (GameAct::List, _) => {}
                    (GameAct::Add(exe), Ok(())) => {
                        let (name, ..) = game_look(&exe);
                        self.show_toast(format!("{name} added · off from its next start"), now);
                    }
                    (GameAct::Flip(exe, off), Ok(())) => {
                        if let Some(g) = self.games.iter_mut().find(|g| g.exe == exe) {
                            g.off = off;
                        }
                        let (name, ..) = game_look(&exe);
                        let t = if off { format!("Off for {name} · from its next start") } else { format!("Back on for {name} · from its next start") };
                        self.show_toast(t, now);
                    }
                    (GameAct::Remove(exe), Ok(())) => {
                        self.games.retain(|g| g.exe != exe);
                        let (name, ..) = game_look(&exe);
                        self.show_toast(format!("{name} removed · back to normal"), now);
                    }
                    (_, Err(e)) => self.show_toast(Self::err_toast(&e), now),
                }
            }
            Done::Opened => {}
        }
        self.remember();
    }

    /// Test copies (and proof pictures): wait (5 s at most) until every worker job has answered and landed.
    #[cfg(test)]
    pub fn settle(&mut self) {
        let t0 = std::time::Instant::now();
        while (self.pending > 0 || self.admin_wait.is_some()) && t0.elapsed().as_secs() < 5 {
            self.tick(1000.0);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(self.pending == 0, "a Tweaks job never answered");
    }

    fn value(&self, i: usize) -> Option<&Value> {
        self.st.get(i).and_then(|s| s.as_ref()).map(|s| &s.value)
    }

    fn on(&self, i: usize) -> bool {
        matches!(self.value(i), Some(Value::Switch(true)))
    }

    /// The crate service for the reset line: the open page's own one, else one built on first use (`rs`).
    /// (The reset review runs on the menu's thread; the open page's worker may hold the service a moment.)
    fn with_svc<R>(&self, f: impl FnOnce(&Svc) -> R) -> Option<R> {
        self.shared_svc().map(|s| f(&lock(&s)))
    }

    fn with_svc_mut<R>(&mut self, f: impl FnOnce(&mut Svc) -> R) -> Option<R> {
        self.shared_svc().map(|s| f(&mut lock(&s)))
    }

    /// The open page's service, else the closed page's (made on first need) - shared, so the reset's worker copy uses
    /// the very same one (Order 047).
    fn shared_svc(&self) -> Option<Arc<Mutex<Svc>>> {
        if let Some(s) = &self.svc {
            return Some(s.clone());
        }
        let mut c = self.rs.borrow_mut();
        if c.is_none() {
            *c = fresh_svc().map(|s| Arc::new(Mutex::new(s)));
        }
        c.clone()
    }


    /// A crate error as the toast says it.
    fn err_toast(e: &TErr) -> String {
        match e {
            TErr::NeedsAdmin { .. } => crate::admin::NOT_CHANGED.into(),
            TErr::BlockedByWindows { .. } => "Windows blocked this change".into(),
            TErr::Disabled { reason, .. } | TErr::NotAvailable { reason, .. } => (*reason).into(),
            other => other.to_string(),
        }
    }

    /// A switch's / a time's answer, its group already read back (`landed`): the toast.
    fn after(&mut self, i: usize, a: Result<Applied, TErr>, now: f64) {
        let r = &ROWS[i];
        match a {
            Ok(a) => {
                let on = self.on(i);
                let t = a.toast.or_else(|| TGT.iter().find(|x| x.0 == r.id).map(|x| (if on { x.2 } else { x.1 }).to_string()));
                if let Some(t) = t {
                    self.show_toast(t, now);
                }
            }
            Err(e) => self.show_toast(Self::err_toast(&e), now),
        }
    }

    fn flip(&mut self, i: usize, _cx: &mut Cx) {
        let r = &ROWS[i];
        if self.st.get(i).and_then(|s| s.as_ref()).is_some_and(|s| !s.enabled) {
            return;
        }
        if self.admin_wait.is_some() || self.wait.contains(&i) || self.svc.is_none() {
            return;
        }
        let to = !self.on(i);
        // (the same kind of service the page has: a --real-read test copy's is read-only and changes nothing)
        #[cfg(windows)]
        let read_only = self.real_read;
        // an admin row on Windows: the change goes to the app's elevated copy behind Windows' admin prompt - off the UI
        // thread (the menu keeps painting; the switch dims until the answer). The worker logs the change itself: the menu
        // may have closed meanwhile.
        #[cfg(windows)]
        if self.real && bu_toggles::rows::find(r.crate_id).is_some_and(|c| c.needs_admin()) {
            let (tx, rx) = std::sync::mpsc::channel();
            let w = crate::services::Waker;
            let _ = std::thread::Builder::new().name("bu-tweaks-admin".into()).spawn(move || {
                let mut s = Svc::real(read_only);
                let old = if NO_LOG.contains(&ROWS[i].id) { None } else { row_val(&s, i) };
                let a = s.set(ROWS[i].crate_id, to);
                if let (true, Some(o), Some(n)) = (a.is_ok(), old, row_val(&s, i)) {
                    if o != n {
                        crate::undo::note("tgl", ROWS[i].id, ROWS[i].title, &o, &n);
                    }
                }
                let _ = tx.send(a);
                w.wake();
            });
            self.admin_wait = Some((i, rx));
            self.dim = Some(i);
            self.wait.push(i);
            return;
        }
        // Order 047: the switch itself on the worker (an Explorer restart, a settings broadcast to every window); the
        // switch flips when its answer lands, as it did when the menu waited for it
        let id = r.crate_id;
        // Sleep off sets "Sleep after" to never (on: back to a time): that row is logged too
        let also = if r.id == "sleep" { row_ix("sleepafter") } else { None };
        let sent = self.run(move |s| {
            let old = log_of(s, i);
            let also = also.map(|j| (j, log_of(s, j)));
            let a = s.set(id, to);
            let mut logs = Vec::new();
            if a.is_ok() {
                push_log(&mut logs, s, i, old);
                if let Some((j, o)) = also {
                    push_log(&mut logs, s, j, o);
                }
            }
            finish(s, i, a, logs, None)
        });
        if sent {
            self.wait.push(i);
        }
    }

    fn set_time(&mut self, i: usize, secs: u32, _cx: &mut Cx) {
        let t = Timeout::from_seconds(secs);
        if self.value(i) == Some(&Value::Timeout(t)) || self.wait.contains(&i) || self.svc.is_none() {
            return;
        }
        // Order 047: on the worker (power settings); the list shows the time when its answer lands
        let id = ROWS[i].crate_id;
        let sent = self.run(move |s| {
            let old = log_of(s, i);
            let a = s.set_timeout(id, t);
            let mut logs = Vec::new();
            if a.is_ok() {
                push_log(&mut logs, s, i, old);
            }
            finish(s, i, a, logs, Some(secs))
        });
        if sent {
            self.wait.push(i);
        }
    }

    /// Windows' games list merged in: keep the ones switched back on (Windows forgets them), add new ones Windows has.
    fn merge_games(&mut self, real: Vec<Game>) {
        let mut out: Vec<Game> = self.games.iter().filter(|g| !real.iter().any(|r| r.exe.eq_ignore_ascii_case(&g.exe))).map(|g| Game { off: false, ..g.clone() }).collect();
        out.extend(real);
        out.sort_by_key(|g| g.exe.to_lowercase());
        self.games = out;
    }

    /// A games window change on the worker (Order 047): the flag set, the change-log line, Windows' list read after an add.
    fn game_job(&mut self, act: GameAct) {
        let sent = self.run(move |s| {
            let mut logs = Vec::new();
            let (r, real) = match &act {
                GameAct::List => (Ok(()), Some(games_of(s))),
                GameAct::Add(exe) => {
                    let r = fso_change(s, exe, true, &mut logs);
                    let real = r.is_ok().then(|| games_of(s));
                    (r, real)
                }
                GameAct::Flip(exe, off) => (fso_change(s, exe, *off, &mut logs), None),
                GameAct::Remove(exe) => (fso_change(s, exe, false, &mut logs), None),
            };
            Done::Games { act, r, real, logs }
        });
        if sent {
            self.games_wait += 1;
        }
    }

    // ------------------------------------------------------------------ building

    fn tip_icons(&self, cx: &mut Cx, i: usize) -> Vec<El> {
        let row = bu_toggles::rows::find(ROWS[i].crate_id);
        let Some(row) = row else { return Vec::new() };
        let mut v = Vec::new();
        let adm = row.badges.contains(&Badge::Admin);
        if adm {
            v.push(tip::rq(cx, idx(sub(K_TIP, "a"), i), Rq::Adm, 18.0, tip::texts::ADM, false));
        }
        // the drawing's `TIP` text of the row's restart kind (exp / out / boot / game)
        let rst = row.badges.iter().find_map(|b| match b {
            Badge::Admin => None,
            Badge::Explorer => Some(tip::texts::EXP),
            Badge::SignOut => Some(tip::texts::OUT),
            Badge::Restart => Some(tip::texts::BOOT),
            Badge::NextGame => Some(tip::texts::GAME),
        });
        if let Some(t) = rst {
            v.push(tip::rq(cx, idx(sub(K_TIP, "r"), i), Rq::Rst, 18.0, t, adm));
        }
        v
    }

    /// `.ttl{display:flex;align-items:center;gap:4px;min-width:0}` with `.tti` (the title, marked by the search) + tip icons.
    fn ttl(&self, _cx: &mut Cx, title: &str, tips: Vec<El>, words: &[String]) -> El {
        El::row()
            .center()
            .gap(4.0)
            .min_w(0.0)
            .child(temp::marked(title, words, Font::new(13.0, 400), FG(), lh(13.0, 1.35)))
            .children(tips)
    }

    fn small(t: &str) -> El {
        El::text(t, Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).ellipsis().margin(1.0, 0.0, 0.0, 0.0)
    }

    fn hits(&self, hay: &str, words: &[String]) -> bool {
        let h = hay.to_lowercase();
        words.iter().all(|w| h.contains(w.as_str()))
    }

    fn row_hit(&self, i: usize, words: &[String]) -> bool {
        let r = &ROWS[i];
        self.hits(&format!("{} {} {} {}", r.title, r.sub, r.k, GROUPS[r.group]), words)
    }

    fn switch_row(&mut self, cx: &mut Cx, i: usize, first: bool, words: &[String]) -> El {
        let r = &ROWS[i];
        let st = self.st.get(i).cloned().flatten();
        let tips = self.tip_icons(cx, i);
        let ttl = self.ttl(cx, r.title, tips, words);
        let enabled = st.as_ref().is_none_or(|s| s.enabled);
        let sub_text = match (&st, r.id) {
            (Some(s), _) if !s.enabled && s.disabled_reason.is_some() => s.disabled_reason.unwrap_or("").to_string(),
            (None, _) => "Can’t read it right now".into(),
            // Order 043: a row that still restarts Explorer says so BEFORE it is switched
            _ if bu_toggles::rows::find(r.crate_id).is_some_and(|c| c.restarts_explorer()) => {
                if r.sub.is_empty() { RESTARTS.into() } else { format!("{} · {RESTARTS}", r.sub) }
            }
            _ => r.sub.to_string(),
        };
        let mut lbl = El::col().flex1().child(ttl);
        if !sub_text.is_empty() {
            lbl = lbl.child(Self::small(&sub_text));
        }
        let ctl = match st.as_ref().map(|s| &s.value) {
            Some(Value::Timeout(t)) => {
                // `cPopup(TIMES, …, 'w')` = `.pu.w{width:128px}`
                dropdown::dropdown(cx, idx(K_TO, i), time_label(*t), Some(128.0))
            }
            // waiting for Windows' admin prompt (DESIGN "Admin flip": the switch dims, no clicks, then flips)
            Some(Value::Switch(on)) if self.dim == Some(i) => {
                toggle::toggle(cx, idx(K_TG, i), *on, false).opacity(0.55).no_hit()
            }
            Some(Value::Switch(on)) => toggle::toggle(cx, idx(K_TG, i), *on, false),
            _ => toggle::toggle(cx, idx(K_TG, i), false, true),
        };
        let mut row = group::row(first, vec![lbl, group::ctl(vec![ctl])]);
        if !enabled {
            // `.row.dim{opacity:.4;pointer-events:none}`
            row = row.opacity(0.4).no_hit();
        }
        row
    }

    /// `.row.trow.fsor`: the title + line, and the blue count link ("1 game" / "Add games") that opens the games window.
    fn fso_row(&mut self, cx: &mut Cx, i: usize, first: bool, words: &[String]) -> El {
        let r = &ROWS[i];
        let n = self.games.len();
        let cnt = if n == 0 { "Add games".to_string() } else if n == 1 { "1 game".into() } else { format!("{n} games") };
        let ttl = self.ttl(cx, r.title, Vec::new(), words);
        let lbl = El::col().flex1().child(ttl).child(Self::small(r.sub));
        // `cnt.title='Games with fullscreen optimizations off'`
        let cnt = link::link(cx, K_FSO, &cnt, 12.0).title("Games with fullscreen optimizations off");
        group::row(first, vec![lbl, group::ctl(vec![cnt])])
    }

    /// The group header `.gh.gfold`: title (fg2, fg on hover), the count `.gcnt` (shown while folded), `.ghr` with the
    /// chevron `.gchv` (turned -90° folded). Click = fold (not while searching).
    fn fold_head(&self, cx: &mut Cx, g: usize, title: &str, count: usize, shut: bool) -> El {
        let k = idx(K_GH, g);
        let hv = cx.hover_t(k, 120.0, EASE);
        let cnt_op = cx.tr(k, 1, if shut { 1.0 } else { 0.0 }, 150.0, EASE);
        let rot = cx.tr(k, 2, if shut { 1.0 } else { 0.0 }, 280.0, FOLD);
        let chev = El::block()
            .size(16.0, 16.0)
            .none()
            .place_center()
            .child(El::icon("chevDw", 10.0, 1.6, cmix(FG3(), FG2(), hv)).rotate(-90.0 * rot));
        El::row()
            .center()
            .gap(6.0)
            .margin(20.0, 12.0, 7.0, 12.0)
            .radius(5.0)
            .on_click(k)
            .cursor(Cursor::Hand)
            .child(El::text(title, Font::new(11.0, 500), cmix(FG2(), FG(), hv), lh(11.0, 1.35)).none())
            .child(El::text(count.to_string(), Font::new(11.0, 400), FG3(), lh(11.0, 1.35)).none().opacity(cnt_op))
            .child(El::row().center().gap(10.0).ml_auto().child(chev))
    }

    /// A group: header + the folding box (`.xp` / `.xin`). None when the search hides all of it.
    fn group(&self, cx: &mut Cx, g: usize, title: &str, count: usize, rows: Vec<El>, searching: bool) -> Option<El> {
        if rows.is_empty() {
            return None;
        }
        let open = searching || !self.shut[g];
        let head = self.fold_head(cx, g, title, count, !open);
        let ft = card::fold_t(cx, idx(sub(K_GH, "xp"), g), open);
        let body = group::grp(rows);
        // at rest (fully open) no clip: Chromium snaps `.xin{overflow:hidden}` to whole pixels, so its clip never cuts the
        // box's anti-aliased bottom rim; `card::drop_out` clips at the fractional height (418.70 px on Files & Explorer)
        let xp = if ft.0 >= 0.9995 && ft.1 >= 0.9995 { body } else { card::drop_out(cx, body, W, ft) };
        Some(El::block().child(head).child(xp))
    }

    /// Default apps: `.row.dln{gap:10px;min-height:34px;padding-top:4px;padding-bottom:4px}` = `.dext` (58 px, 12.5 px 600; the
    /// browser 500), `.dnow` (tile + name, 12.5 px) and `.dpk` "Change".
    fn default_rows(&self, cx: &mut Cx, words: &[String], hits: &mut usize) -> Vec<El> {
        let Some(d) = &self.defaults else { return Vec::new() };
        let mut list: Vec<(String, Option<String>, String)> = vec![(d.browser.label.clone(), d.browser.app.as_ref().map(|a| a.name.clone()), "default web internet links http Links, .html and .htm".into())];
        for f in &d.file_types {
            list.push((f.label.clone(), f.app.as_ref().map(|a| a.name.clone()), "Opens with open with default program file type extension".into()));
        }
        let mut out = Vec::new();
        for (i, (label, app, k)) in list.into_iter().enumerate() {
            if !words.is_empty() && !self.hits(&format!("{label} {k} Default apps"), words) {
                continue;
            }
            *hits += 1;
            let ext = temp::marked(&label, words, Font::new(12.5, if i == 0 { 500 } else { 600 }).tnum(), FG(), lh(12.5, 1.35)).w(58.0).none();
            let name = app.clone().unwrap_or_else(|| "Not set".into());
            let now = El::row()
                .center()
                .gap(7.0)
                .flex1()
                .child(El::row().none().child(app_tile(&name)))
                .child(El::text(name, Font::new(12.5, 400), FG(), lh(12.5, 1.35)).ellipsis());
            let k = idx(K_DPK, i);
            let hv = cx.hover_t(k, 120.0, EASE);
            let dpk = El::row()
                .center()
                .h(22.0)
                .none()
                .pad(0.0, 9.0, 0.0, 9.0)
                .radius(6.0)
                .bg(HOV().mul_a(hv))
                .on_click(k)
                .cursor(Cursor::Hand)
                .child(El::text("Change", pieces::btn_font(11.5, 400), ACC(), lh(11.5, 1.35)));
            // `ch.title='Pick another browser'` / `ch.title='Opens Windows’ “Open with” list for '+d.t+' files'`
            let dpk = if i == 0 { dpk.title("Pick another browser") } else { dpk.title(&format!("Opens Windows’ “Open with” list for {label} files")) };
            let first = out.is_empty();
            out.push(group::row(first, vec![ext, now, dpk]).gap(10.0).min_h(34.0).pad(4.0, 12.0, 4.0, 12.0));
        }
        out
    }

    fn games_window(&mut self, cx: &mut Cx, at: f64) -> El {
        // `.fsol{border-radius:9px;background:var(--grp);box-shadow:inset 0 0 0 .5px var(--hair);overflow:hidden}`
        let mut list = El::block().radius(9.0).bg(GRP()).inset(&[crate::gfx::sh(0.0, 0.0, 0.0, 0.5, HAIR())]).clip();
        if self.games.is_empty() {
            // `.fsoe{padding:11px 12px;font-size:11.5px;color:var(--fg3)}`
            list = list.child(El::text("No games yet · Add game picks a game’s .exe", Font::new(11.5, 400), FG3(), lh(11.5, 1.35)).pad(11.0, 12.0, 11.0, 12.0));
        }
        let games = self.games.clone();
        for (i, g) in games.iter().enumerate() {
            let (name, glyph, a, b) = game_look(&g.exe);
            let rk = idx(K_FROW, i);
            let row_hover = cx.hovered(rk);
            // `.fsog{position:relative;display:flex;align-items:center;gap:10px;min-height:42px;padding:5px 10px}`
            // `.fsog+.fsog::before{left:44px;right:0;top:0;height:1px;background:var(--hair)}` `.fsog .lbl small{color:var(--fg3)}`
            let mut r = El::row().center().gap(10.0).min_h(42.0).pad(5.0, 10.0, 5.0, 10.0).key(rk);
            if i > 0 {
                r = r.child(El::block().abs(44.0, 0.0, 0.0, f32::NAN).h(1.0).bg(HAIR()).no_hit());
            }
            let lbl = El::col()
                .flex1()
                .child(El::text(name, Font::new(13.0, 400), FG(), lh(13.0, 1.35)).ellipsis())
                .child(El::text(file_name(&g.exe), Font::new(11.0, 400), FG3(), lh(11.0, 1.35)).ellipsis().margin(1.0, 0.0, 0.0, 0.0));
            r = r
                .child(pieces::listrow::tile(&pieces::listrow::Tile::Glyph { glyph, a, b }, 24.0))
                .child(lbl)
                // `title:'Remove from the list (its setting goes back to normal)'`
                .child(rowbits::rdel(cx, idx(K_FDEL, i), row_hover).title("Remove from the list (its setting goes back to normal)"))
                .child(toggle::toggle(cx, idx(K_FTG, i), g.off, false));
            list = list.child(r);
        }
        // `.fsodlg .inote.fson{margin-top:10px;padding:0 2px}` + `.inote.fson{padding:0 0 1px;white-space:normal}` (calm: fg3 icon)
        let note = inote::inote("Set it before the game starts — Windows reads it at launch.", true, &inote::FSODLG);
        // `#sw .dft .cbtn.ic{display:inline-flex;align-items:center;gap:7px;padding:0 14px 0 11px}` + `.cbtn.ic svg{15px;stroke-width:1.5}`
        // (the `.acc` look; the `+` icon is put in front of the shared button's text)
        // `title:'Pick the game’s .exe'`
        let add = pieces::button::cbtn_sized(cx, K_FADD, "Add game", pieces::button::Kind::Primary, pieces::button::DFT, false, 76.0).title("Pick the game’s .exe");
        let add = El::row()
            .center()
            .gap(7.0)
            .none()
            .child(add.pad(0.0, 14.0, 0.0, 33.0))
            .child(El::icon("plus12", 15.0, 1.5, crate::ui::WHITE).abs(11.0, 7.5, f32::NAN, f32::NAN).no_hit());
        // `.fsodlg{width:380px}` `.fsodlg .dft{margin-top:14px}`
        let body = vec![list, note, El::row().justify(JustifyContent::FLEX_END).gap(8.0).margin(14.0, 0.0, 0.0, 0.0).child(add)];
        dialog::dialog(cx, K_FDLG, 380.0, "Fullscreen optimizations off", body, Vec::new(), true, at)
    }
}

impl Page for Tweaks {
    fn id(&self) -> &'static str {
        "tgl"
    }
    fn name(&self) -> &'static str {
        "Tweaks"
    }
    fn icon(&self) -> &'static str {
        "tgl"
    }
    fn open(&mut self, env: &Env, _now: f64) {
        self.test = env.fake();
        self.real_read = env.real_read;
        self.real = cfg!(windows) && !env.fake();
        self.keep = env.keep.clone();
        #[cfg(windows)]
        let s = if env.fake() { Svc::sample() } else { Svc::real(env.real_read) };
        #[cfg(not(windows))]
        let s = Svc::sample();
        let s = Arc::new(Mutex::new(s));
        let (work, done) = start_worker(s.clone(), self.slow);
        self.svc = Some(s);
        self.work = Some(work);
        self.done = Some(done);
        self.picks = if self.test { svc::SAMPLE_PICKS.iter().map(|s| s.to_string()).collect() } else { Vec::new() };
        self.games.clear();
        // Order 047: the tab shows its last read at once (registry, power and SPI reads of ~50 rows, the Copilot package
        // query, every file type's default app: 50-400 ms), the fresh one comes from the worker
        if let Some(k) = env.keep.get::<Snap>(KEEP) {
            self.st = k.st;
            self.games = k.games;
            self.defaults = k.defaults;
        }
        self.refresh(true);
        let fix: Arc<dyn bu_quickfix::FixOs> = if env.fake() {
            let f = bu_quickfix::fake::FakeFixOs::new();
            // the drawing's sample: "last one 2 Oct 2026, 18:04"
            f.add_point(bu_quickfix::fake::stamp(2026, 10, 2, 18, 4), "Boyler Utilities · 2 Oct 2026");
            Arc::new(f)
        } else {
            #[cfg(windows)]
            {
                if env.real_read {
                    Arc::new(bu_quickfix::real::RealOs::read_only())
                } else {
                    // DISM / sfc and the restore point run in the app's elevated copy (Order 039): each fix gets its own
                    // layer whose scope makes all of its admin calls one prompt
                    let real: Arc<dyn bu_quickfix::FixOs> = Arc::new(bu_quickfix::real::RealOs::new());
                    let p = Arc::new(crate::admin::proxy::FixOs::new(real, crate::admin::client::admin()));
                    let one = p.clone();
                    self.qf = Some(qf::Qf::with_admin(p, Box::new(move |purpose| Arc::new(one.scoped(purpose)) as Arc<dyn bu_quickfix::FixOs>)));
                    return;
                }
            }
            #[cfg(not(windows))]
            {
                Arc::new(bu_quickfix::fake::FakeFixOs::new())
            }
        };
        self.qf = Some(qf::Qf::new(fix));
    }
    fn close(&mut self) {
        // a running fix keeps going on its own thread (the drawing: "keeps running when the menu closes"); its row
        // state is dropped with the page
        // Order 047: answers already in but not taken still write their change-log lines; a job still running writes its
        // own (its worker ends after it)
        if let Some(rx) = &self.done {
            while let Ok(d) = rx.try_recv() {
                if let Done::Row { logs, .. } | Done::Games { logs, .. } = &d {
                    note_logs(logs);
                }
            }
        }
        *self = Tweaks { shut: self.shut, ..Tweaks::default() };
    }
    fn ready(&self) -> bool {
        // Order 047: the last read (`env.keep`) or the fresh one is in
        !self.st.is_empty()
    }
    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        if let Some(q) = &mut self.qf {
            for t in q.poll() {
                self.toast = Some((t, cx.now));
            }
        }
        let words = self.words();
        let searching = !words.is_empty();
        let mut out = vec![pieces::header(self.name(), Some(search::search(cx, K_SEARCH, &self.query, "Search tweaks", false)))];
        let mut any = false;
        for (g, gname) in GROUPS.iter().enumerate() {
            let ids: Vec<usize> = (0..ROWS.len()).filter(|&i| ROWS[i].group == g).collect();
            let shown: Vec<usize> = ids.iter().copied().filter(|&i| !searching || self.row_hit(i, &words)).collect();
            let mut rows = Vec::new();
            for (n, &i) in shown.iter().enumerate() {
                let first = n == 0;
                rows.push(if ROWS[i].id == "fso" { self.fso_row(cx, i, first, &words) } else { self.switch_row(cx, i, first, &words) });
            }
            any |= !rows.is_empty();
            if let Some(e) = self.group(cx, g, gname, ids.len(), rows, searching) {
                out.push(e);
            }
        }
        let mut dh = 0;
        let drows = self.default_rows(cx, &words, &mut dh);
        any |= dh > 0;
        let dn = self.defaults.as_ref().map(|d| 1 + d.file_types.len()).unwrap_or(0);
        if let Some(e) = self.group(cx, G_DEF, "Default apps", dn, drows, searching) {
            out.push(e);
        }
        // Quick fixes: the very last group (the drawing inserts it before `.tnone`, i.e. after Default apps)
        let mut qrows = Vec::new();
        if let Some(q) = &self.qf {
            for (i, (t, s, _)) in qf::QF.iter().enumerate() {
                if searching && !self.hits(&format!("{t} {s} {} Quick fixes", qf::QF_K), &words) {
                    continue;
                }
                let title = temp::marked(t, &words, Font::new(13.0, 400), FG(), lh(13.0, 1.35));
                let first = qrows.is_empty();
                qrows.push(q.row(cx, i, first, title));
            }
        }
        any |= !qrows.is_empty();
        if let Some(e) = self.group(cx, G_QF, "Quick fixes", qf::QF.len(), qrows, searching) {
            out.push(e);
        }
        if searching && !any {
            // `.tnone{padding:34px 0 10px;text-align:center;font-size:12.5px;color:var(--fg2)}`
            out.push(
                El::text(format!("Nothing matches “{}”", self.query.trim()), Font::new(12.5, 400), FG2(), lh(12.5, 1.35))
                    .align(Align::Center)
                    .pad(34.0, 0.0, 10.0, 0.0),
            );
        }
        out.push(reset::reset_line(cx, K_RS, Some("Windows defaults")));
        out
    }
    fn tick(&mut self, now: f64) -> bool {
        if let Some((i, rx)) = &self.admin_wait {
            let got = match rx.try_recv() {
                Ok(a) => Some(a),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(TErr::Admin("The admin helper stopped".into()))),
                Err(_) => None,
            };
            if let Some(a) = got {
                let i = *i;
                self.admin_wait = None;
                // its group read back (and a Settings page it asks for opened) on the worker; the switch stays dimmed
                // until that lands
                if !self.run(move |s| finish(s, i, a, Vec::new(), None)) {
                    self.wait.retain(|&j| j != i);
                    self.dim = None;
                }
            }
        }
        // Order 047: the worker's answers (it woke the menu)
        let mut changed = false;
        while let Some(d) = self.done.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.pending = self.pending.saturating_sub(1);
            self.landed(d, now);
            changed = true;
        }
        // Quick fixes: only a real change of a row draws (a run's progress moved, a run ended, the restore point line came)
        if let Some(q) = &mut self.qf {
            let before = q.picture();
            let mut toasts = q.poll();
            changed |= !toasts.is_empty() || q.picture() != before;
            if let Some(t) = toasts.pop() {
                self.toast = Some((t, now));
            }
        }
        // Order 047: a toast needs no frames of the page's own - it fades on its transitions and wakes the menu itself
        // (`toast::toast`); here it is only forgotten once it has gone (`wake_at`)
        if self.toast.as_ref().is_some_and(|(_, t)| now - t >= toast::SHOW_MS + 300.0) {
            self.toast = None;
        }
        changed
    }
    fn wake_at(&self, now: f64) -> Option<f64> {
        let t = self.toast.as_ref().map(|(_, t)| (t + toast::SHOW_MS + 300.0).max(now + 1.0));
        let q = self.qf.as_ref().and_then(|q| q.wake_at(now));
        match (t, q) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        let now = cx.now;
        if let Ev::Press(_, _, _, r) = ev {
            self.pressed = *r;
        }
        let Ev::Click(k) = ev else {
            match ev {
                // Order 045: Ctrl+F = the search (`if(menuShown()&&curPane==='tgl'&&(e.ctrlKey||e.metaKey)&&!e.altKey&&
                // e.code==='KeyF'){e.preventDefault();tsIn.focus();tsIn.select();return;}`, L7992)
                Ev::Key(_, 0x46) if cx.mods.ctrl && !cx.mods.alt => {
                    cx.focus(Some(K_SEARCH));
                    cx.used = true;
                }
                Ev::Char(k, c) if *k == K_SEARCH => search::edit_char(&mut self.query, *c),
                // the drawing's `tsIn` keydown: Esc clears (an empty field: Esc leaves it), Enter leaves the field
                Ev::Key(k, 0x1B) if *k == K_SEARCH && self.query.is_empty() => cx.focus(None),
                Ev::Key(k, 0x0D) if *k == K_SEARCH => cx.focus(None),
                Ev::Key(k, vk) if *k == K_SEARCH => search::edit_key(&mut self.query, *vk),
                _ => {}
            }
            return;
        };
        let k = *k;
        let (bx, by, bw, bh) = self.pressed;
        if k == sub(K_SEARCH, "x") {
            self.query.clear();
            return;
        }
        if let Some(g) = (0..10).find(|&g| idx(K_GH, g) == k) {
            if self.query.trim().is_empty() {
                self.shut[g] = !self.shut[g];
            }
            return;
        }
        if let Some(i) = (0..ROWS.len()).find(|&i| idx(K_TG, i) == k) {
            self.flip(i, cx);
            return;
        }
        if let Some(i) = (0..ROWS.len()).find(|&i| idx(K_TO, i) == k) {
            self.pop = Some(Pop::Times(i, (bx, by, bw, bh)));
            return;
        }
        if k == K_FSO {
            // Order 047: Windows' list read again on the worker; the window shows the list it has meanwhile
            self.game_job(GameAct::List);
            self.pop = Some(Pop::Games(now));
            return;
        }
        if let Some(i) = (0..4).find(|&i| idx(qf::K_QF, i) == k) {
            if i == 2 && !self.qf.as_ref().is_some_and(|q| q.running(2)) {
                // asks first, by the button (placeMenu-like at the button's bottom right: menuAt(btn, r.right, r.bottom, 260))
                self.pop = Some(Pop::Ask(bx + bw, by + bh));
                return;
            }
            if let Some(t) = self.qf.as_mut().and_then(|q| q.press(i, false)) {
                self.show_toast(t, now);
            }
            return;
        }
        if let Some(i) = (0..10).find(|&i| idx(K_DPK, i) == k) {
            let Some(d) = self.defaults.clone() else { return };
            if i == 0 {
                self.pop = Some(Pop::Browsers(bx, by + bh + 4.0));
            } else if let Some(f) = d.file_types.get(i - 1) {
                // Order 047: Windows' "Open with" opens from the worker (the shell can take seconds)
                if let Some(a) = f.change.clone() {
                    self.run(move |s| {
                        let _ = s.open(&a);
                        Done::Opened
                    });
                }
                self.show_toast(bu_toggles::defaults::file_type_toast(&f.label), now);
            }
            return;
        }
        if k == sub(K_RS, "pc") || k == sub(K_RS, "win") {
            // the frame's shared review over the ONE change log / this page's Windows defaults (Order 036)
            let kind = if k == sub(K_RS, "win") { crate::undo::Kind::WindowsDefaults } else { crate::undo::Kind::HowItWas };
            cx.open_reset(kind, self.pressed);
            return;
        }
        // ---- popups
        match self.pop.take() {
            Some(Pop::Times(i, r)) => {
                if let Some(n) = (0..TIMES.len()).find(|&n| idx(K_TMENU, time_row(n)) == k) {
                    self.set_time(i, TIMES[n].0, cx);
                } else {
                    self.pop = Some(Pop::Times(i, r));
                }
            }
            Some(Pop::Browsers(x, y)) => {
                let bs = self.defaults.as_ref().map(|d| d.browsers.clone()).unwrap_or_default();
                if let Some(n) = (0..bs.len()).find(|&n| idx(K_BMENU, n) == k) {
                    if !bs[n].is_current {
                        // Order 047: Settings opens from the worker (the shell can take seconds)
                        let a = bs[n].change.clone();
                        self.run(move |s| {
                            let _ = s.open(&a);
                            Done::Opened
                        });
                        self.show_toast(bu_toggles::defaults::browser_pick_toast(&bs[n].name), now);
                    }
                } else {
                    self.pop = Some(Pop::Browsers(x, y));
                }
            }
            Some(Pop::Ask(x, y)) => {
                if k == sub(K_ASK, "go") {
                    if let Some(t) = self.qf.as_mut().and_then(|q| q.press(2, true)) {
                        self.show_toast(t, now);
                    }
                } else if k != sub(K_ASK, "no") {
                    self.pop = Some(Pop::Ask(x, y));
                }
            }
            Some(Pop::Games(at)) => {
                self.pop = Some(Pop::Games(at));
                if k == sub(K_FDLG, "x") || k == sub(K_FDLG, "out") {
                    self.pop = None;
                } else if k == K_FADD {
                    // Windows' file picker (a test copy: the drawing's FSO_PICK games come back instead)
                    let picked = if self.test && !self.picks.is_empty() { Some(self.picks.remove(0)) } else { cx.pick_file("Pick the game", &[("Programs", "*.exe")]) };
                    // Order 047: the flag is set on the worker; the list and the toast come when its answer lands
                    if let Some(exe) = picked {
                        self.game_job(GameAct::Add(exe));
                    }
                } else if self.games_wait > 0 {
                    // a change of the list is on its way: a row's click waits for it (the rows may move)
                } else if let Some(i) = (0..self.games.len()).find(|&i| idx(K_FTG, i) == k) {
                    let g = self.games[i].clone();
                    self.game_job(GameAct::Flip(g.exe, !g.off));
                } else if let Some(i) = (0..self.games.len()).find(|&i| idx(K_FDEL, i) == k) {
                    let g = self.games[i].clone();
                    self.game_job(GameAct::Remove(g.exe));
                }
            }
            None => {}
        }
    }
    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        let now = cx.now;
        let pop = match self.pop.take() {
            Some(Pop::Times(i, r)) => {
                let cur = match self.value(i) {
                    Some(Value::Timeout(t)) => t.seconds(),
                    _ => u32::MAX,
                };
                // the drawing's list: …5 hours, `['-']` (.msep), Never
                let mut list: Vec<Row> = Vec::new();
                for (n, (s, l)) in TIMES.iter().enumerate() {
                    if n == TIMES.len() - 1 {
                        list.push(Row::Sep);
                    }
                    list.push(Row::Item(It::tick(l, *s == cur)));
                }
                let e = mitems::menu(cx, K_TMENU, &list, Place::Under(r.0, r.1, r.2, r.3), 150.0);
                self.pop = Some(Pop::Times(i, r));
                Some(e)
            }
            Some(Pop::Browsers(x, y)) => {
                let bs = self.defaults.as_ref().map(|d| d.browsers.clone()).unwrap_or_default();
                let e = browser_menu(cx, &bs, x, y);
                self.pop = Some(Pop::Browsers(x, y));
                Some(e)
            }
            Some(Pop::Games(at)) => {
                let e = self.games_window(cx, at);
                self.pop = Some(Pop::Games(at));
                Some(e)
            }
            Some(Pop::Ask(x, y)) => {
                // menuAt(btn, r.right, r.bottom, 260)
                let e = mitems::confirm(
                    cx,
                    K_ASK,
                    "Rebuild icon & thumbnail cache?",
                    "File Explorer restarts: the taskbar blinks once and open Explorer windows close.",
                    "Cancel",
                    "Rebuild",
                    pieces::button::Kind::Primary,
                    Place::At(x, y),
                    260.0,
                );
                self.pop = Some(Pop::Ask(x, y));
                Some(e)
            }
            None => None,
        };
        let t = self.toast.clone().map(|(t, at)| toast::toast(cx, K_TOAST, &t, at, false));
        let _ = now;
        match (pop, t) {
            (None, None) => None,
            (p, t) => Some(El::block().abs(0.0, 0.0, 0.0, 0.0).no_hit().children(p).children(t)),
        }
    }
    fn popup_dismiss(&mut self) {
        self.pop = None;
    }
    fn resettable(&mut self) -> Option<&mut dyn Resettable> {
        Some(self)
    }
    fn describe(&self) -> String {
        let on: Vec<&str> = (0..ROWS.len()).filter(|&i| self.on(i)).map(|i| ROWS[i].id).collect();
        format!("search={} on={} games={} pop={}", self.query, on.join(","), self.games.len(), self.pop.is_some())
    }
}

#[cfg(test)]
mod tests;

/// Windows' own values of the rows that differ from them now (the reset's "Windows defaults").
fn win_defaults(s: &Svc) -> Vec<DefaultItem> {
    WIN_DEFAULT
        .iter()
        .filter_map(|(id, d)| {
            let i = row_ix(id)?;
            let Value::Switch(now) = s.read(ROWS[i].crate_id).ok()?.value else { return None };
            Some(DefaultItem { item: id.to_string(), label: ROWS[i].title.into(), now: sw_val(now), default: sw_val(*d) })
        })
        .collect()
}

/// Order 047: Tweaks' reset for a worker thread (`Resettable::detach`): the page's ONE service (shared), so an Explorer
/// restart, a settings broadcast or an admin prompt of a reset never holds the menu.
struct Away {
    svc: Arc<Mutex<Svc>>,
    real_read: bool,
    /// test copies only: every read / put-back first sleeps this long (ms)
    slow: u64,
}

impl Away {
    fn wait(&self) {
        if self.slow > 0 {
            std::thread::sleep(std::time::Duration::from_millis(self.slow));
        }
    }
}

impl Resettable for Away {
    fn page_id(&self) -> &str {
        "tgl"
    }
    fn page_title(&self) -> &str {
        "Tweaks"
    }
    fn current(&self, item: &str) -> Option<Val> {
        self.wait();
        item_val(&lock(&self.svc), item)
    }
    fn has_item(&self, item: &str) -> bool {
        item.starts_with(FSO_ITEM) || row_ix(item).is_some()
    }
    fn windows_defaults(&self) -> Vec<DefaultItem> {
        self.wait();
        win_defaults(&lock(&self.svc))
    }
    fn apply(&mut self, item: &str, to: &Val) -> Result<(), String> {
        if self.real_read || crate::testmode::real_read() {
            return Err("A read-only test copy changes nothing".into());
        }
        self.wait();
        put(&mut lock(&self.svc), item, to)
    }
}

/// The reset line (Order 036): "Back to how your PC was" from the ONE change log, "Windows defaults" from `WIN_DEFAULT`.
/// Items: the row ids (`ext`, `scroff`, `sleep`…) and `fso:<exe>` per game.
impl Resettable for Tweaks {
    fn page_id(&self) -> &str {
        "tgl"
    }
    fn page_title(&self) -> &str {
        "Tweaks"
    }
    fn current(&self, item: &str) -> Option<Val> {
        self.with_svc(|s| item_val(s, item)).flatten()
    }
    fn has_item(&self, item: &str) -> bool {
        item.starts_with(FSO_ITEM) || row_ix(item).is_some()
    }
    fn windows_defaults(&self) -> Vec<DefaultItem> {
        self.with_svc(win_defaults).unwrap_or_default()
    }
    fn apply(&mut self, item: &str, to: &Val) -> Result<(), String> {
        if self.real_read || crate::testmode::real_read() {
            return Err("A read-only test copy changes nothing".into());
        }
        let r = self.with_svc_mut(|s| put(s, item, to)).ok_or("Tweaks can’t be read on this PC")?;
        if self.svc.is_some() {
            // the open page shows the value it was put back to
            // (Order 047: read again on the worker; the rows show it when it lands)
            self.refresh(false);
        }
        r
    }
    /// Order 047: the review's reads and put-backs on a worker thread, on this page's own service.
    fn detach(&mut self) -> Option<crate::undo::Detached> {
        let svc = self.shared_svc()?;
        Some(Box::new(Away { svc, real_read: self.real_read, slow: self.slow }))
    }
    fn reset_done(&mut self) {
        if self.svc.is_some() {
            self.refresh(false);
        }
    }
}

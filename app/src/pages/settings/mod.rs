//! The Settings tab (menu-v22 page `set`, Order 024): App (Start with Windows · Theme · Glass style), All shortcuts (every key
//! in the app; a row opens its feature, Search's key is set right here), Reset (Back to how your PC was · Windows defaults ·
//! Reset the app's own settings) and the cleaner About (the app, its version, Check for updates - ONLY on click; a new version
//! updates the app itself in the small "Updating…" window; no repo yet = "updates not set up yet").
//!
//! Every box is the drawing's CSS, quoted on each builder. Keys are the keys manager's (All shortcuts lists its actions;
//! Search's field is its `srch.open`); the app-wide asks go through the page API (`TempReq` only records them for the test
//! hook). Theme = Dark glass · Light glass · Match Windows (Order 033). About › Licences opens the third-party parts and
//! their licence texts inside this tab (Order 040, `licences.rs`).

mod autostart;
pub mod licences;
pub mod update;

use taffy::style::{AlignItems, JustifyContent};

use crate::anim::EASE;
use crate::gfx::{sh, Font, Gfx, Rgba};
use crate::pages::{Env, Page};
use crate::settings::GlassStyle;
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, lh, sub, Cursor, El, Key};
use crate::ui::pieces::button::{self, Kind};
use crate::ui::pieces::{self, dialog, group, keyfield, link, mitems, progress, reset, seg, toggle};
use crate::ui::{cmix, CTL, CTL_H, FG, FG2, FG3, GREEN, HAIR, HOV, KEY, WIN_W};

use autostart::AutoStart;
use bu_updater::{CheckResult, Phase, ReleaseInfo, UpdateError};
use update::{Driver, Msg};

const K_START: Key = key("set.start");
const K_THEME: Key = key("set.theme");
const K_GLASS: Key = key("set.glass");
const K_SC: Key = key("set.sc");
const K_SRCH: Key = key("set.srchkey");
const K_RS_WAS: Key = key("set.rs.was");
const K_RS_WIN: Key = key("set.rs.win");
const K_RS_APP: Key = key("set.rs.app");
const K_CHECK: Key = key("set.check");
const K_NEWS: Key = key("set.news");
const K_GH: Key = key("set.github");
const K_LIC: Key = key("set.lic");
const K_UPD: Key = key("set.upd");
const K_UPD_NO: Key = key("set.upd.cancel");
const K_ASK: Key = key("set.ask");
/// the shared confirm's buttons (`mitems::confirm`: `sub(key, "no")` / `sub(key, "go")`)
fn k_ask_no() -> Key {
    sub(K_ASK, "no")
}
fn k_ask_go() -> Key {
    sub(K_ASK, "go")
}
const K_RV: Key = key("set.rv");
const K_RV_NO: Key = key("set.rv.no");
const K_RV_GO: Key = key("set.rv.go");

const VK_ESCAPE: u16 = 0x1B;

/// The app's own version (Cargo) - "Version 1.0.0".
const VERSION: &str = env!("CARGO_PKG_VERSION");

const THEMES: [&str; 3] = ["Dark glass", "Light glass", "Match Windows"];
const GLASS: [&str; 3] = ["Liquid", "Frosted", "Windows look"];
/// What the page asked of the app, as the test hook shows it (`describe`). The asks themselves go through the page API
/// (`cx.set_glass`, `cx.show_tab`, `cx.open_reset_all`, `cx.reset_app_settings`, `cx.exit_for_update`).
/// Order 014 item 2); the test hook shows them (`describe`).
#[derive(Clone, Debug, PartialEq)]
pub enum TempReq {
    /// Settings › Glass style: the frame paints the window with it (`cx.set_glass`)
    Glass(GlassStyle),
    /// Settings › Theme (Dark glass / Light glass / Match Windows): `cx.set_theme`
    Theme(&'static str),
    /// All shortcuts: open that tab and light its key row (the drawing's `jump(id, el)`) - done with `cx.show_tab`; kept
    /// here for the test hook
    ShowTab(String, String),
    /// the update is staged: the app must exit now (the install step swaps the exe and starts the new one)
    ExitForUpdate,
    /// "Reset the app's own settings" (`cx.reset_app_settings`)
    ResetApp,
    /// Settings › Reset › Review…: the frame's review of every tab's changes (`kind` = "was" / "win")
    ReviewAll(&'static str),
}

/// One row of All shortcuts: the keys manager's action (its id, name, owner tab), what it shows.
#[derive(Clone, Debug)]
struct Shortcut {
    id: String,
    name: String,
    tab: String,
    /// the key ("PrtSc", "Ctrl + M") or None
    key: Option<String>,
    /// the feature is switched off ("Off" instead of "Not set"; keys shown dimmed)
    off: bool,
}

fn sc(id: &str, name: &str, tab: &str, key: Option<&str>, off: bool) -> Shortcut {
    Shortcut { id: id.into(), name: name.into(), tab: tab.into(), key: key.map(str::to_string), off }
}

/// The About line under "Boyler Utilities".
#[derive(Clone, Debug, PartialEq)]
enum AbLine {
    NotChecked,
    Asking,
    Newest,
    UpToDate,
    NotSetUp,
    NoRelease,
    Failed(String),
    Updating(String),
    /// Cancel was clicked; `update()` has not answered yet (it may still end Ok past the point of no return)
    Cancelling(String),
    Cancelled(String),
    UpdateFailed(String, String),
}

/// The Updating window's state.
#[derive(Clone, Debug)]
struct UpdUi {
    ver: String,
    opened_at: f64,
    /// 0 downloading, 1 installing, 2 restarting
    stage: u8,
    stage_at: f64,
    done: u64,
    total: u64,
    dl_from: f64,
    /// update() returned Ok (the app may exit once "Restarting…" has shown)
    ready: bool,
    /// Cancel was clicked: the window is closed, the update is asked to stop, its result decides the About line
    cancel_asked: bool,
}

impl UpdUi {
    /// the window is on screen (not after Cancel)
    fn shown(&self) -> bool {
        !self.cancel_asked
    }
}

#[derive(Clone, Debug)]
enum Pop {
    None,
    /// "Reset the app's own settings?" by its button
    Ask((f32, f32, f32, f32)),
    /// the review popup: kind ("was" / "win"), the button's box, the lines
    Review(&'static str, (f32, f32, f32, f32), Vec<reset::Line>),
}

pub struct Settings {
    env: Env,
    auto: Option<Box<dyn AutoStart>>,
    start_on: bool,
    // the app's own choices: the glass style and the theme are saved by the frame (`cx.set_glass`, `cx.set_theme`)
    theme: usize,
    glass: GlassStyle,
    version: String,
    // the updater: ONE for the whole app run (kept in `KEPT` while the page object is gone); a check or an update that is
    // running is kept too, so its result is never lost (017 notes: always act on update()'s result)
    driver: Option<Driver>,
    ab: AbLine,
    checking: bool,
    upd: Option<UpdUi>,
    // the page's state (dropped on close)
    pop: Pop,
    boxes: std::collections::HashMap<Key, (f32, f32, f32, f32)>,
    /// the toast of `say` went to the frame (`cx.toast`)
    toast_sent: bool,
    toast: Option<(String, f64)>,
    pub reqs: Vec<TempReq>,
    /// `open` ran and `close` / drop has not stashed yet (a page the menu made but never showed stashes nothing)
    opened: bool,
    /// About › Licences is showing (its list or one part) instead of the settings - read when it opens, dropped with it
    lic: Option<licences::View>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            env: Env::default(),
            auto: None,
            start_on: false,
            theme: 0,
            glass: GlassStyle::Liquid,
            version: VERSION.to_string(),
            driver: None,
            ab: AbLine::NotChecked,
            checking: false,
            upd: None,
            pop: Pop::None,
            boxes: Default::default(),
            toast_sent: false,
            toast: None,
            reqs: Vec::new(),
            opened: false,
            lic: None,
        }
    }
}

/// What outlives the page object. The menu builds its pages anew on every open and drops them when it closes (menu.rs /
/// ui.rs: `pages::all()`), so the ONE updater of the app run, its running check / update and window, and the app's own
/// choices live here between two shows of the tab. The UI thread only (pages are made, shown and dropped there).
struct Kept {
    theme: usize,
    glass: GlassStyle,
    version: String,
    driver: Option<Driver>,
    ab: AbLine,
    checking: bool,
    upd: Option<UpdUi>,
}

thread_local! {
    static KEPT: std::cell::RefCell<Option<Kept>> = const { std::cell::RefCell::new(None) };
}

impl Drop for Settings {
    /// A menu close drops the page without `close()`: it must behave like leaving the tab.
    fn drop(&mut self) {
        self.stash();
    }
}

impl Settings {
    /// Leaving the tab or closing the menu: while it downloads the download is cancelled (nobody watches it; the answer is
    /// read on the next show); from "Installing…" on nothing is cancelled. The updater, the running check / update, its window
    /// and the app's choices go to `KEPT`; the page's own state goes.
    fn stash(&mut self) {
        if !std::mem::take(&mut self.opened) {
            return;
        }
        self.cancel_update();
        let busy = self.checking || self.upd.is_some();
        let k = Kept {
            theme: self.theme,
            glass: self.glass,
            version: std::mem::take(&mut self.version),
            driver: self.driver.take(),
            ab: if busy { std::mem::replace(&mut self.ab, AbLine::NotChecked) } else { AbLine::NotChecked },
            checking: self.checking,
            upd: self.upd.take(),
        };
        KEPT.with(|c| *c.borrow_mut() = Some(k));
        let env = std::mem::take(&mut self.env);
        let mut fresh = Settings::default();
        fresh.env = env;
        *self = fresh;
    }

    /// The tab is shown again: take back what was kept.
    fn unstash(&mut self) {
        if let Some(k) = KEPT.with(|c| c.borrow_mut().take()) {
            self.theme = k.theme;
            self.glass = k.glass;
            self.version = k.version;
            self.driver = k.driver;
            self.ab = k.ab;
            self.checking = k.checking;
            self.upd = k.upd;
        }
    }

    fn say(&mut self, t: &str, now: f64) {
        self.toast = Some((t.to_string(), now));
        self.toast_sent = false;
    }

    /// Every key in the app: the keys manager's actions (each page's, in the order they were added), Search's own action
    /// left out (its row holds its key field). The comparison pictures show the drawing's sample.
    fn shortcuts(&self, cx: &Cx) -> Vec<Shortcut> {
        if self.env.frozen {
            return vec![
                sc("mic.toggle", "Mic mute", "aud", None, true),
                sc("shot.key", "Screenshot", "shot", Some("PrtSc"), false),
                sc("vtt.listen", "Voice to text", "vtt", None, false),
                sc("tmr.k.1", "Timer \u{b7} Stopwatch", "tmr", None, false),
                sc("tmr.k.2", "Timer \u{b7} Pizza", "tmr", None, false),
                sc("tmr.k.3", "Timer \u{b7} Ultimate", "tmr", Some("F8"), false),
            ];
        }
        let mic_on = crate::pages::audio::mic_mute_on();
        cx.actions()
            .into_iter()
            .filter(|a| a.page != "srch")
            // Mic mute's separate keys show only in that mode, its one key only in the other
            .filter(|a| crate::pages::audio::mic_action_shown(&a.id))
            .map(|a| {
                let name = if a.page == "tmr" { format!("Timer \u{b7} {}", a.name) } else { a.name.clone() };
                let off = a.page == "aud" && !mic_on;
                Shortcut { id: a.id.clone(), name, tab: a.page.clone(), key: a.key.clone(), off }
            })
            .collect()
    }

    /// Search's key: the keys manager's action of the Search tab (its key field sits in its All shortcuts row).
    fn search_action(cx: &Cx) -> Option<String> {
        cx.actions().into_iter().find(|a| a.page == "srch").map(|a| a.id)
    }

    fn check(&mut self) {
        if self.checking || self.upd.is_some() {
            return;
        }
        // the driver refuses while a worker still runs or once an update is staged: then nothing changes on the page
        if self.driver.as_mut().is_some_and(|d| d.check()) {
            self.checking = true;
            self.ab = AbLine::Asking;
        }
    }

    fn start_update(&mut self, rel: ReleaseInfo, now: f64) {
        let ver = rel.version.to_string();
        let total = rel.asset.size;
        if self.driver.as_mut().is_some_and(|d| d.update(rel)) {
            self.ab = AbLine::Updating(ver.clone());
            self.upd = Some(UpdUi { ver, opened_at: now, stage: 0, stage_at: now, done: 0, total, dl_from: now, ready: false, cancel_asked: false });
        }
    }

    /// The thread's answers.
    fn take(&mut self, now: f64) -> bool {
        let Some(d) = self.driver.as_mut() else { return false };
        // a Cancel clicked before the worker entered update() is dropped by the updater (a stale click never cancels the
        // NEXT update), so it is asked again until update() answers
        if self.upd.as_ref().is_some_and(|u| u.cancel_asked) {
            d.cancel();
        }
        let msgs = d.poll();
        let mut any = !msgs.is_empty();
        for m in msgs {
            match m {
                Msg::Checked(r) => {
                    self.checking = false;
                    match r {
                        Ok(CheckResult::NotSetUp) => self.ab = AbLine::NotSetUp,
                        Ok(CheckResult::NoReleases) => self.ab = AbLine::NoRelease,
                        Ok(CheckResult::UpToDate { .. }) => self.ab = AbLine::Newest,
                        Ok(CheckResult::Available(rel)) => self.start_update(rel, now),
                        Err(e) => self.ab = AbLine::Failed(update::reason(&e)),
                    }
                }
                Msg::Progress(p) => {
                    if let Some(u) = self.upd.as_mut() {
                        match p.phase {
                            Phase::Downloading => {
                                u.done = p.bytes_done;
                                if let Some(t) = p.bytes_total {
                                    u.total = t;
                                }
                            }
                            Phase::Verifying | Phase::Installing | Phase::Restarting => {
                                if u.stage == 0 {
                                    u.stage = 1;
                                    u.stage_at = now;
                                }
                            }
                        }
                    }
                }
                // update()'s result decides, never the Cancel click: Ok = staged (even after a Cancel that came too late), so
                // the window comes back with "Installing…" / "Restarting…" and the app exits for the install step
                Msg::Done(r) => match r {
                    Ok(_) => {
                        if let Some(u) = self.upd.as_mut() {
                            if u.cancel_asked {
                                u.cancel_asked = false;
                                u.opened_at = now;
                            }
                            if u.stage == 0 {
                                u.stage = 1;
                                u.stage_at = now;
                            }
                            u.ready = true;
                            self.ab = AbLine::Updating(u.ver.clone());
                        }
                    }
                    Err(UpdateError::Cancelled) => {
                        if let Some(u) = self.upd.take() {
                            self.ab = AbLine::Cancelled(u.ver);
                        }
                    }
                    Err(e) => {
                        if let Some(u) = self.upd.take() {
                            self.ab = AbLine::UpdateFailed(u.ver, update::reason(&e));
                        }
                    }
                },
            }
        }
        // the drawing's timing: "Installing…" at least 1.6 s, then "Restarting Boyler Utilities…" 1.3 s
        let mut finish = None;
        if let Some(u) = self.upd.as_mut() {
            if u.ready && u.stage == 1 && now - u.stage_at >= 1600.0 {
                u.stage = 2;
                u.stage_at = now;
                any = true;
            }
            if u.ready && u.stage == 2 && now - u.stage_at >= 1300.0 && !self.reqs.contains(&TempReq::ExitForUpdate) {
                finish = Some(u.ver.clone());
            }
        }
        if let Some(ver) = finish {
            // (Order 047: the page is built again with it - the window closes / the app is asked to exit)
            any = true;
            if self.env.fake() {
                // the fake can't restart anything: it ends like the drawing (the window closes, About says up to date)
                self.upd = None;
                self.version = ver.clone();
                self.ab = AbLine::UpToDate;
                self.say(&format!("Updated to {ver}"), now);
                // the "new app": a fresh fake at the new version (the old one's scratch folder goes with it)
                self.driver = Some(Driver::fake(&self.version, std::time::Duration::from_millis(26)));
            } else if !self.reqs.contains(&TempReq::ExitForUpdate) {
                self.reqs.push(TempReq::ExitForUpdate);
            }
        }
        any
    }

    /// Cancel (only while downloading - the drawing hides it after): the window closes and the update is asked to stop; the
    /// About line says "cancelled" only when update() answers Cancelled.
    fn cancel_update(&mut self) {
        if let Some(u) = self.upd.as_mut().filter(|u| u.stage == 0 && !u.cancel_asked && !u.ready) {
            if let Some(d) = self.driver.as_ref() {
                d.cancel();
            }
            u.cancel_asked = true;
            self.ab = AbLine::Cancelling(u.ver.clone());
        }
    }

    /// The drawing's sample change log (RS) for the page's own review popup in test copies (nothing on the PC changes); a
    /// real copy opens the frame's all-tabs review instead (`cx.open_reset_all`, the recorded changes of every tab).
    fn review_lines(&self, kind: &str) -> Vec<reset::Line> {
        if !self.env.fake() {
            return Vec::new();
        }
        let was: &[(&str, &[(&str, &str, &str)])] = &[
            ("Audio", &[("Default output", "Headphones (Arctis Nova)", "Speakers (Realtek)"), ("Keep my devices", "On", "Off"), ("Discord volume", "80 %", "100 %")]),
            ("Display", &[("DELL 27 \u{b7} vibrance", "60 %", "50 %"), ("Switch automatically", "2 apps", "none")]),
            ("Screenshots", &[("Print Screen key", "Boyler capture", "Snipping Tool"), ("Screenshots folder", "D:\\Clips\\Screens", "Pictures\\Screenshots")]),
            ("Mouse", &[("Pointer speed", "10", "8"), ("Enhance pointer precision", "Off", "On"), ("Double-click speed", "faster", "as before")]),
            ("Controller", &[("Rocket League \u{b7} DualSense Edge", "your edits", "as on 7 Oct (backup)")]),
            ("Tweaks", &[("Mouse acceleration", "Off", "On"), ("Game Mode", "On", "Off"), ("Bing in Start search", "Off", "On"), ("Sticky Keys shortcut", "Off", "On"), ("File extensions", "Shown", "Hidden")]),
            ("Startup", &[("OBS Studio", "Off", "Starts with Windows"), ("Adobe Acrobat Update Task", "Off", "On")]),
            ("Network", &[("DNS (Ethernet)", "Custom \u{b7} 9.9.9.9", "Automatic"), ("VirtualBox Host-Only", "Off", "On")]),
            ("Security", &[("Allowed in Defender", "1 file", "none")]),
        ];
        let win: &[(&str, &[(&str, &str, &str)])] = &[
            ("Audio", &[("Keep my devices", "On", "Off"), ("App volumes", "5 changed", "100 %"), ("New apps volume", "On \u{b7} 50 %", "Off")]),
            ("Display", &[("Vibrance (both monitors)", "60 % \u{b7} 50 %", "50 %"), ("Switch automatically", "2 apps", "none")]),
            ("Screenshots", &[("Print Screen key", "Boyler capture", "Snipping Tool"), ("Screenshots folder", "D:\\Clips\\Screens", "Pictures\\Screenshots")]),
            ("Mouse", &[("Enhance pointer precision", "Off", "On"), ("Cursors", "your own", "Windows default"), ("Cursor size", "2", "1")]),
            ("Controller", &[("Rocket League \u{b7} DualSense Edge", "your layout", "Steam\u{2019}s layout")]),
            ("Tweaks", &[("Mouse acceleration", "Off", "On"), ("Bing in Start search", "Off", "On"), ("Sticky Keys shortcut", "Off", "On"), ("File extensions", "Shown", "Hidden"), ("Widgets", "Off", "On")]),
            ("Network", &[("DNS (all adapters)", "Custom", "Automatic"), ("VirtualBox Host-Only", "Off", "On"), ("Bluetooth Network", "Off", "On")]),
            ("Security", &[("Allowed in Defender", "1 file", "none")]),
        ];
        let src = if kind == "was" { was } else { win };
        let mut out = Vec::new();
        for (tab, rows) in src {
            for (i, (what, from, to)) in rows.iter().enumerate() {
                out.push(reset::Line { title: what.to_string(), from: from.to_string(), to: to.to_string(), ticked: true, heading: (i == 0).then(|| tab.to_string()) });
            }
        }
        out
    }

    fn box_of(&self, k: Key) -> (f32, f32, f32, f32) {
        self.boxes.get(&k).copied().unwrap_or((0.0, 0.0, 0.0, 0.0))
    }

    fn about_line(&self, cx: &mut Cx) -> El {
        // `.lbl small` = 11 px --fg2, `.abs .abnew{color:var(--green);font-weight:600}`
        let f = Font::new(11.0, 400);
        let t = |s: String| El::text(s, f, FG2(), lh(11.0, 1.35)).none();
        let v = &self.version;
        let line = match &self.ab {
            AbLine::NotChecked => t(format!("Version {v} \u{b7} not checked yet")),
            AbLine::Asking => t("Asking GitHub for the newest release\u{2026}".into()),
            AbLine::Newest => t(format!("Version {v} \u{b7} you have the newest \u{b7} checked just now")),
            AbLine::UpToDate => t(format!("Version {v} \u{b7} up to date \u{b7} checked just now")),
            AbLine::NotSetUp => t(format!("Version {v} \u{b7} updates not set up yet")),
            AbLine::NoRelease => t(format!("Version {v} \u{b7} no release published yet")),
            AbLine::Failed(e) => t(format!("Version {v} \u{b7} could not check \u{b7} {e}")),
            AbLine::Updating(n) | AbLine::Cancelling(n) | AbLine::Cancelled(n) | AbLine::UpdateFailed(n, _) => {
                let rest = match &self.ab {
                    AbLine::Updating(_) => " \u{b7} updating now".to_string(),
                    AbLine::Cancelling(_) => " \u{b7} cancelling\u{2026}".to_string(),
                    AbLine::Cancelled(_) => " \u{b7} update cancelled".to_string(),
                    AbLine::UpdateFailed(_, e) => format!(" \u{b7} the update did not work \u{b7} {e}"),
                    _ => String::new(),
                };
                El::row().child(El::text(format!("v{n} is out"), Font::new(11.0, 600), GREEN(), lh(11.0, 1.35)).none()).child(t(rest))
            }
        };
        let _ = cx;
        line.margin(1.0, 0.0, 0.0, 0.0)
    }
}

impl Page for Settings {
    fn id(&self) -> &'static str {
        "set"
    }
    fn name(&self) -> &'static str {
        "Settings"
    }
    fn icon(&self) -> &'static str {
        "gear"
    }

    fn open(&mut self, env: &Env, _now: f64) {
        self.env = env.clone();
        // one registry read (real) / the drawing's sample (frozen) / off (other test copies)
        let a: Box<dyn AutoStart> = if env.fake() { Box::new(autostart::Fake(env.frozen)) } else { Box::new(autostart::Real) };
        self.start_on = a.get();
        self.auto = Some(a);
        // what outlived the last menu / tab (the menu builds its pages anew on every open)
        self.unstash();
        // the saved glass style, shown from the first frame (not the default, then a snap)
        self.glass = crate::services::with(|s| s.glass()).unwrap_or(self.glass);
        // the saved theme, the same way
        if let Some(t) = crate::services::with(|s| s.theme()) {
            self.theme = crate::settings::Theme::ALL.iter().position(|x| *x == t).unwrap_or(0);
        }
        self.opened = true;
        // nothing goes online here: the updater only answers the button. ONE driver per app run (made on the first open).
        if self.driver.is_none() {
            self.driver = Some(if env.fake() { Driver::fake(&self.version, std::time::Duration::from_millis(26)) } else { Driver::real(&self.version) });
        }
    }

    fn close(&mut self) {
        self.stash();
    }

    /// Order 047: true only when the updater's thread said something (each message wakes the menu) or a timed step of
    /// the Updating window came; the moving parts (the Checking… spinner, the indeterminate bar) run their own frames.
    fn tick(&mut self, now: f64) -> bool {
        self.take(now)
    }

    /// Order 047: the Updating window's timed steps ("Installing…" 1.6 s, then "Restarting…" 1.3 s), and while a Cancel
    /// waits for update()'s answer it is asked again every 100 ms (an early Cancel is dropped by the updater).
    fn wake_at(&self, now: f64) -> Option<f64> {
        let u = self.upd.as_ref()?;
        if u.cancel_asked {
            return Some(now + 100.0);
        }
        match (u.ready, u.stage) {
            (true, 1) => Some((u.stage_at + 1600.0).max(now + 1.0)),
            (true, 2) if !self.reqs.contains(&TempReq::ExitForUpdate) => Some((u.stage_at + 1300.0).max(now + 1.0)),
            _ => None,
        }
    }

    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        let now = cx.now;
        // (Order 047: no `st.busy` while checking / updating - the spinner and the bar ask for frames only while they move)
        // what the page said (a check's answer, "Nothing to reset"): the frame's toast, once
        if let Some((t, _)) = self.toast.as_ref().filter(|_| !self.toast_sent) {
            cx.toast(&t.clone());
            self.toast_sent = true;
        }
        // the update is staged: the app ends now (the install step swaps the exe and starts the new one)
        if self.reqs.contains(&TempReq::ExitForUpdate) {
            cx.exit_for_update();
        }
        if let Some(v) = self.lic.as_mut() {
            return v.build(cx);
        }
        let mut out = vec![pieces::header(self.name(), None)];

        // ---- App: group('App', [row(Start with Windows, toggle), row(Theme, seg), row(Glass style, seg, 'How clear the window is')])
        let glass_i = GlassStyle::ALL.iter().position(|g| *g == self.glass).unwrap_or(0);
        out.push(group::gh("App"));
        out.push(group::grp(vec![
            group::row(true, vec![group::lbl("Start with Windows", None), group::ctl(vec![toggle::toggle(cx, K_START, self.start_on, false)])]),
            group::row(false, vec![group::lbl("Theme", None), group::ctl(vec![seg::seg(cx, K_THEME, &THEMES, self.theme, false)])]),
            group::row(false, vec![group::lbl("Glass style", Some("How clear the window is")), group::ctl(vec![seg::seg(cx, K_GLASS, &GLASS, glass_i, false)])]),
        ]));

        // ---- All shortcuts: `.grp.scg{overflow:hidden}`, rows `.row.sc` (+ Search's `.row.sc.scin` with its inline key field)
        out.push(group::gh("All shortcuts"));
        let mut rows = Vec::new();
        let list = self.shortcuts(cx);
        let srch = if self.env.frozen { None } else { Self::search_action(cx) };
        let mut srch_err = None;
        // Search's key is set right here (v21: its page no longer has it) - the keys manager's field; it goes third
        let srch_f = match &srch {
            Some(a) => {
                let (set, listening, err) = cx.key_field(a);
                srch_err = err;
                let show = match (&listening, &set) {
                    (Some((held, _)), _) => keyfield::Show::Listening(held.as_deref()),
                    (None, Some(k)) => keyfield::Show::Set(k),
                    (None, None) => keyfield::Show::Empty,
                };
                // Order 045: `title:'Click to change'` (Search's key field, menu-v22 L5863 - shown here since v21)
                keyfield::keyfield(cx, K_SRCH, show, listening.as_ref().map(|l| l.1).unwrap_or(0.0), true).title("Click to change")
            }
            None if self.env.frozen => keyfield::keyfield(cx, K_SRCH, keyfield::Show::Empty, 0.0, true),
            None => El::text("Not set", Font::new(12.0, 400), FG3(), lh(12.0, 1.35)),
        };
        let mut srch_row = Some(sc_row(cx, idx(K_SC, 99), list.is_empty(), "Search", vec![srch_f], false));
        let mut first = true;
        for (i, s) in list.iter().enumerate() {
            if i == 2 {
                rows.extend(srch_row.take());
            }
            let val = match (&s.key, s.off) {
                (Some(k), dim) => keycaps(k, dim),
                (None, true) => vec![El::text("Off", Font::new(12.0, 400), FG3(), lh(12.0, 1.35))],
                (None, false) => vec![El::text("Not set", Font::new(12.0, 400), FG3(), lh(12.0, 1.35))],
            };
            rows.push(sc_row(cx, idx(K_SC, i), first, &s.name, val, true));
            first = false;
        }
        rows.extend(srch_row.take());
        out.push(group::grp(rows).clip());
        // the keys manager's refusal of Search's key ("Already used by Mic mute") under the list
        if let Some(e) = srch_err {
            out.push(El::text(e, Font::new(11.5, 400), crate::ui::RED(), 15.0).margin(6.0, 12.0, 0.0, 12.0));
        }

        // ---- Reset
        out.push(group::gh("Reset"));
        out.push(group::grp(vec![
            group::row(true, vec![group::lbl("Back to how your PC was", Some("Undoes every change this app made to Windows")), group::ctl(vec![button::cbtn(cx, K_RS_WAS, "Review\u{2026}", Kind::Ghost, true, false, 0.0)])]),
            group::row(false, vec![group::lbl("Windows defaults", Some("Windows\u{2019} own values for everything this app can change")), group::ctl(vec![button::cbtn(cx, K_RS_WIN, "Review\u{2026}", Kind::Ghost, true, false, 0.0)])]),
            group::row(false, vec![group::lbl("Reset the app\u{2019}s own settings", Some("Keys, glass, presets, timers \u{b7} Windows is not touched")), group::ctl(vec![button::cbtn(cx, K_RS_APP, "Reset\u{2026}", Kind::Ghost, true, false, 0.0)])]),
        ]));

        // ---- About: `.row.abr{gap:12px;min-height:60px;padding-top:10px;padding-bottom:10px}` = the app icon `.abi` (36 px),
        // `.lbl` (`.abn{font-size:13.5px;font-weight:600}` + the line), `.ctl` (the `.cbtn.sm.abb` button)
        out.push(group::gh("About"));
        let busy = self.checking;
        let label = if busy { "Checking\u{2026}" } else if self.upd.is_some() { "Updating\u{2026}" } else { "Check for updates" };
        let abb = abb(cx, K_CHECK, label, busy);
        let lbl = El::col().flex1().child(El::text("Boyler Utilities", Font::new(13.5, 600), FG(), lh(13.5, 1.35)).ellipsis()).child(self.about_line(cx));
        let abr = El::row().center().gap(12.0).min_h(60.0).pad(10.0, 12.0, 10.0, 12.0).child(app_icon(36.0)).child(lbl).child(group::ctl(vec![abb]));
        out.push(group::grp(vec![abr]));
        // `.abl{display:flex;align-items:center;gap:7px;margin:8px 12px 0;font-size:12px;color:var(--fg3)}` `.abl .lnk{font-size:12px}`
        let dot = || El::text("\u{b7}", Font::new(12.0, 400), FG3(), lh(12.0, 1.35)).none();
        out.push(
            El::row()
                .center()
                .gap(7.0)
                .margin(8.0, 12.0, 0.0, 12.0)
                .child(link::link(cx, K_NEWS, "What\u{2019}s new", 12.0))
                .child(dot())
                .child(link::link(cx, K_GH, "GitHub", 12.0))
                .child(dot())
                .child(link::link(cx, K_LIC, "Licences", 12.0)),
        );
        let _ = now;
        out
    }

    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        let now = cx.now;
        if let Ev::Press(k, _, _, r) = ev {
            self.boxes.insert(*k, *r);
        }
        // Licences' own input; the rest (the Updating window's Cancel) goes on as always
        if self.licences_event(ev, cx) {
            cx.dirty = true;
            return;
        }
        match ev {
            Ev::Click(k) if *k == K_START => {
                let want = !self.start_on;
                if let Some(a) = self.auto.as_mut() {
                    match a.set(want) {
                        Ok(()) => {
                            // Order 036: into the change log (its first old value = how the PC was)
                            let old = self.start_on;
                            self.start_on = a.get();
                            cx.record(AUTO_ITEM, AUTO_LABEL, &auto_val(old), &auto_val(self.start_on));
                        }
                        Err(e) => self.say(&format!("Start with Windows: {e}"), now),
                    }
                }
            }
            Ev::Click(k) if (0..3).any(|i| *k == idx(K_THEME, i)) => {
                let i = (0..3).find(|i| *k == idx(K_THEME, *i)).unwrap_or(0);
                self.theme = i;
                // Order 033: saved, and the frame switches the whole window on its next frame (Match Windows follows Windows' app
                // theme live)
                cx.set_theme(crate::settings::Theme::ALL[i]);
                self.reqs.push(TempReq::Theme(crate::settings::Theme::ALL[i].id()));
            }
            Ev::Click(k) if (0..3).any(|i| *k == idx(K_GLASS, i)) => {
                let i = (0..3).find(|i| *k == idx(K_GLASS, *i)).unwrap_or(0);
                self.glass = GlassStyle::ALL[i];
                // feedback F5: the window takes the new glass at once (the frame restyles it on its next frame) and the
                // choice is saved
                cx.set_glass(self.glass);
                self.reqs.push(TempReq::Glass(self.glass));
            }
            // All shortcuts: a row opens its feature (the drawing's jump); Search's field listens right here
            Ev::Click(k) if (0..64).any(|i| *k == idx(K_SC, i)) => {
                let i = (0..64).find(|i| *k == idx(K_SC, *i)).unwrap_or(0);
                if let Some(s) = self.shortcuts(cx).get(i) {
                    // the drawing's jump(id, el): that tab, which then shows the key's place (its `Page::jump`)
                    cx.show_tab(&s.tab, Some(&s.id));
                    self.reqs.push(TempReq::ShowTab(s.tab.clone(), s.name.clone()));
                }
            }
            // Search's key field: the keys manager listens (it takes the key messages; Esc cancels) / the × clears it
            Ev::Click(k) if *k == K_SRCH => {
                if let Some(a) = Self::search_action(cx) {
                    if cx.key_field(&a).1.is_none() {
                        cx.listen_key(&a);
                    }
                }
            }
            // the field lost the focus (a click elsewhere): it stops listening
            Ev::Blur(k) if *k == K_SRCH => cx.stop_listening(),
            Ev::Click(k) if *k == sub(K_SRCH, "clr") => {
                if let Some(a) = Self::search_action(cx) {
                    cx.stop_listening();
                    cx.clear_key(&a);
                }
            }
            // Reset
            Ev::Click(k) if *k == K_RS_WAS || *k == K_RS_WIN => {
                let kind = if *k == K_RS_WAS { "was" } else { "win" };
                if self.env.frozen {
                    // test pictures: the drawing's sample log in the page's own review (nothing on the PC changes)
                    let lines = self.review_lines(kind);
                    self.pop = Pop::Review(kind, self.box_of(*k), lines);
                } else {
                    // the frame's ONE review over every tab's recorded changes, applied through each page
                    let k2 = if kind == "was" { crate::undo::Kind::HowItWas } else { crate::undo::Kind::WindowsDefaults };
                    cx.open_reset_all(k2, self.box_of(*k));
                    self.reqs.push(TempReq::ReviewAll(kind));
                }
            }
            Ev::Click(k) if *k == K_RS_APP => self.pop = Pop::Ask(self.box_of(*k)),
            Ev::Click(k) if *k == k_ask_no() || *k == K_RV_NO => self.pop = Pop::None,
            Ev::Click(k) if *k == k_ask_go() => {
                self.pop = Pop::None;
                self.glass = GlassStyle::Liquid;
                self.theme = 0;
                cx.reset_app_settings();
                cx.set_glass(GlassStyle::Liquid);
                // the pages' live copies of what the store just forgot (else their next change saves the old values back)
                crate::pages::audio::reset_app_settings(self.env.fake());
                self.reqs.push(TempReq::ResetApp);
                self.reqs.push(TempReq::Glass(GlassStyle::Liquid));
                // (the frame's toast says it: `Cx::reset_app_settings`)
            }
            Ev::Click(k) if *k == K_RV_GO => {
                if let Pop::Review(kind, _, lines) = std::mem::replace(&mut self.pop, Pop::None) {
                    let n = lines.iter().filter(|l| l.ticked).count();
                    // test copies only (the drawing's sample log): nothing on the PC changes
                    let what = if kind == "was" { "Back to how it was" } else { "Windows defaults" };
                    let t = if n == 0 { "Nothing to reset".to_string() } else { format!("{what} \u{b7} {n} {} reset", if n == 1 { "setting" } else { "settings" }) };
                    self.say(&t, now);
                }
            }
            Ev::Click(k) if matches!(self.pop, Pop::Review(..)) => {
                if let Pop::Review(_, _, lines) = &mut self.pop {
                    if let Some(i) = (0..lines.len()).find(|i| *k == idx(K_RV, *i)) {
                        lines[i].ticked = !lines[i].ticked;
                    }
                }
            }
            // About
            Ev::Click(k) if *k == K_CHECK => self.check(),
            Ev::Click(k) if *k == K_UPD_NO => self.cancel_update(),
            Ev::Click(k) if *k == K_NEWS || *k == K_GH => {
                // the releases page / the repo in the user's browser - never from a test copy (no browser in tests)
                if !self.env.test {
                    // Order 047: the program start (10-40 ms) runs off the menu's thread
                    let url = update::page_url(*k == K_NEWS);
                    crate::offui::spawn("set-link", move || {
                        let _ = std::process::Command::new("explorer").arg(url).spawn();
                    });
                }
            }
            Ev::Click(k) if *k == K_LIC => match licences::View::new() {
                Ok(v) => {
                    self.lic = Some(v);
                    cx.scroll_y(0.0);
                }
                // (the file is checked by the tests; a broken one never ships)
                Err(e) => self.say(&format!("Licences could not be read \u{b7} {e}"), now),
            },
            _ => {}
        }
        cx.dirty = true;
    }

    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        let now = cx.now;
        // the Updating window (`miniDlg('Updating…', …, {cls:'upddlg', noX:true, locked})`)
        if let Some(u) = self.upd.clone().filter(UpdUi::shown) {
            return Some(self.updating(cx, &u, now));
        }
        match self.pop.clone() {
            Pop::Ask(b) => {
                // askIn(b, …) = the shared popup confirm `.mcf` under its button (placeMenu(b, 262)); appReset's go is red
                Some(mitems::confirm(
                    cx,
                    K_ASK,
                    "Reset the app\u{2019}s own settings?",
                    "Keys, glass style, presets, timers and places go back to how the app came. Your Windows settings stay as they are.",
                    "Cancel",
                    "Reset",
                    Kind::Red,
                    mitems::Place::Under(b.0, b.1, b.2, b.3),
                    262.0,
                ))
            }
            Pop::Review(kind, (bx, by, bw, bh), lines) => {
                let n = lines.iter().filter(|l| l.ticked).count();
                let (title, text) = if kind == "was" {
                    ("Back to how your PC was?", "Each one goes back to the value it had before this app changed it.")
                } else {
                    ("Windows defaults for everything?", "Each one goes to Windows\u{2019} own value.")
                };
                let go = button::cbtn_sized(cx, K_RV_GO, &if n > 0 { format!("Reset {n}") } else { "Reset".into() }, Kind::Red, button::MCFB, n == 0, 0.0);
                let no = button::cbtn_sized(cx, K_RV_NO, "Cancel", Kind::Ghost, button::MCFB, false, 0.0);
                // placeMenu(btn, 316): under the button, or above it when it would leave the window (its height measured)
                let probe = reset::review(cx, K_RV, title, text, &lines, vec![no.clone(), go.clone()]);
                let mh = measure(cx.g, &probe, 316.0) + 10.0;
                let (left, top) = place((bx, by, bw, bh), 326.0, mh);
                Some(reset::review_popup(cx, K_RV, left, top, title, text, &lines, vec![no, go]))
            }
            // (a toast is the frame's own: `say` hands it over in `build`)
            Pop::None => None,
        }
    }

    fn popup_dismiss(&mut self) {
        // the Updating window is locked while it works (the drawing's `locked: () => upLock`)
        if !self.upd.as_ref().is_some_and(UpdUi::shown) {
            self.pop = Pop::None;
            self.toast = None;
        }
    }

    fn describe(&self) -> String {
        let pop = match &self.pop {
            Pop::None => "none".to_string(),
            Pop::Ask(_) => "ask".to_string(),
            Pop::Review(k, _, l) => format!("review:{k}:{}", l.iter().filter(|x| x.ticked).count()),
        };
        let upd = self.upd.as_ref().map(|u| format!("{}:{}:{}/{}{}", u.ver, u.stage, u.done, u.total, if u.cancel_asked { ":cancel" } else { "" })).unwrap_or_else(|| "-".into());
        format!(
            "start={} theme={} glass={} version={} about={:?} checking={} upd={} pop={} toast={:?} reqs={:?} {}",
            self.start_on,
            THEMES[self.theme],
            self.glass.id(),
            self.version,
            self.ab,
            self.checking,
            upd,
            pop,
            self.toast.as_ref().map(|t| t.0.as_str()).unwrap_or(""),
            self.reqs,
            self.lic.as_ref().map(|v| v.describe()).unwrap_or_else(|| "lic=-".into())
        )
    }
    fn resettable(&mut self) -> Option<&mut dyn crate::undo::Resettable> {
        Some(self)
    }
}

/// Order 036: Start with Windows in the change log (Settings has no reset line of its own: its lines show in Settings ›
/// Reset and the uninstaller's undo; Windows has no default for it - the drawing lists none).
const AUTO_ITEM: &str = "autostart";
const AUTO_LABEL: &str = "Start with Windows";

fn auto_val(on: bool) -> crate::undo::Val {
    if on {
        crate::undo::Val::new("on", "On")
    } else {
        crate::undo::Val::new("off", "Off")
    }
}

impl Settings {
    /// Input while Licences shows: the back arrow / Esc go back one level (a part -> the list -> the settings, About in view
    /// again), a row opens its part (from the top). True = it was Licences' input.
    fn licences_event(&mut self, ev: &Ev, cx: &mut Cx) -> bool {
        let Some(v) = self.lic.as_mut() else { return false };
        match ev {
            Ev::Click(k) if *k == licences::K_BACK => {}
            // Esc with no element focused, or right after a click (the clicked row / arrow holds the focus)
            Ev::Key(_, VK_ESCAPE) => cx.used = true,
            Ev::Click(k) if v.open_row(*k) => {
                cx.scroll_y(0.0);
                return true;
            }
            _ => return false,
        }
        // back one level
        let row = v.open;
        if v.back() {
            if let Some(i) = row {
                cx.scroll_to(idx(licences::K_ROW, i));
            }
        } else {
            self.lic = None;
            cx.scroll_to(K_LIC);
        }
        true
    }

    /// The page's switch, or (a closed page: Settings › Reset, the uninstaller) one made now - the fake in test copies.
    fn auto_now(&mut self) -> &mut Box<dyn AutoStart> {
        self.auto.get_or_insert_with(|| if crate::testmode::on() || cfg!(test) { Box::new(autostart::Fake(false)) } else { Box::new(autostart::Real) })
    }
}

impl crate::undo::Resettable for Settings {
    fn page_id(&self) -> &str {
        "set"
    }
    fn page_title(&self) -> &str {
        "Settings"
    }
    fn current(&self, item: &str) -> Option<crate::undo::Val> {
        if item != AUTO_ITEM {
            return None;
        }
        match &self.auto {
            Some(a) => Some(auto_val(a.get())),
            // one registry read; a closed test copy knows nothing (the record's last value is used)
            None if !crate::testmode::on() && !cfg!(test) => Some(auto_val(autostart::Real.get())),
            None => None,
        }
    }
    fn has_windows_defaults(&self) -> bool {
        false
    }
    fn apply(&mut self, item: &str, to: &crate::undo::Val) -> Result<(), String> {
        if item != AUTO_ITEM {
            return Err("Unknown setting".into());
        }
        if crate::testmode::real_read() {
            return Err("A read-only test copy changes nothing".into());
        }
        let on = to.raw == "on";
        let a = self.auto_now();
        a.set(on)?;
        let now = a.get();
        self.start_on = now;
        Ok(())
    }
}

impl Settings {
    /// The Updating window: `.upddlg{width:340px}` `.upd{display:flex;align-items:center;gap:12px;margin:2px 0 14px}`
    /// `.upd .abi{width:40px;height:40px}` `.upd b{display:block;font-size:13.5px;font-weight:600}` `.upd small{display:block;
    /// margin-top:1px;font-size:11.5px;color:var(--fg2)}`, the bar, the status line, `#sw .upddlg .dft{margin-top:14px}`
    /// (Cancel only while downloading; `.dft.gone{visibility:hidden}` keeps its place).
    fn updating(&self, cx: &mut Cx, u: &UpdUi, now: f64) -> El {
        let frozen = self.env.frozen;
        let share = if u.total > 0 { u.done as f32 / u.total as f32 } else { 0.0 };
        // the comparison picture: the drawing's sample moment (41.5 %, 13.2 of 31.8 MB, 5 s left)
        let (share, mb_done, left_s) = if frozen && u.stage == 0 {
            (0.415, 13.2, 5)
        } else {
            let el = ((now - u.dl_from) / 1000.0).max(0.05);
            let rate = u.done as f64 / el;
            let left = if rate > 0.0 { ((u.total.saturating_sub(u.done)) as f64 / rate).round().max(1.0) as u32 } else { 0 };
            (share, u.done as f64 / 1e6, left)
        };
        let mb_total = u.total as f64 / 1e6;
        let (status, right) = match u.stage {
            0 if u.done == 0 && !frozen => ("Downloading\u{2026}".to_string(), String::new()),
            0 => (format!("Downloading \u{b7} {mb_done:.1} of {mb_total:.1} MB"), format!("{left_s} s left")),
            1 => ("Installing\u{2026}".to_string(), String::new()),
            _ => ("Restarting Boyler Utilities\u{2026}".to_string(), String::new()),
        };
        let head = El::row()
            .center()
            .gap(12.0)
            .margin(2.0, 0.0, 14.0, 0.0)
            .child(app_icon(40.0))
            .child(
                El::col()
                    .child(El::text(format!("Boyler Utilities {}", u.ver), Font::new(13.5, 600), FG(), lh(13.5, 1.35)))
                    .child(El::text(format!("You have {}", self.version), Font::new(11.5, 400), FG2(), lh(11.5, 1.35)).margin(1.0, 0.0, 0.0, 0.0)),
            );
        let bar = progress::bar(cx, sub(K_UPD, "bar"), if u.stage == 0 { Some(share) } else { None });
        let cancel = button::cbtn_sized(cx, K_UPD_NO, "Cancel", Kind::Ghost, button::DFT, false, 76.0);
        let dft = El::row().justify(JustifyContent::FLEX_END).gap(8.0).margin(14.0, 0.0, 0.0, 0.0).child(if u.stage == 0 { cancel } else { cancel.opacity(0.0).no_hit() });
        dialog::dialog(cx, K_UPD, 340.0, "Updating\u{2026}", vec![head, bar, progress::status(&status, &right), dft], vec![], false, u.opened_at)
    }
}

/// A shortcuts row: `.row.sc{cursor:pointer;transition:background-color .12s ease}` `.sc:hover{background:var(--hov)}`
/// `.sc .ctl{gap:6px}` `.sc .chv{margin-left:6px}` `.sc .chv svg{width:7px;height:12px;stroke:var(--fg3);stroke-width:1.5}`.
/// `jump` = the row opens its feature (the chevron shows); Search's row (`.scin`) holds its own key field.
fn sc_row(cx: &mut Cx, k: Key, first: bool, name: &str, mut val: Vec<El>, jump: bool) -> El {
    let hv = cx.hover_t(k, 120.0, EASE);
    if jump {
        val.push(El::icon("chevR", 7.0, 1.5, FG3()).h(12.0).margin(0.0, 0.0, 0.0, 6.0).no_hit());
    }
    let ctl = El::row().none().center().gap(6.0).children(val);
    let r = group::row(first, vec![group::lbl(name, None), ctl]).bg(HOV().mul_a(hv)).key(k).cursor(Cursor::Hand);
    if jump {
        r.on_click(k)
    } else {
        r
    }
}

/// Keys as caps (`keycaps`): `.kc{display:inline-flex;align-items:center;height:18px;padding:0 6px;border-radius:4px;
/// background:var(--key);box-shadow:inset 0 0 0 .5px var(--hair),0 1px 0 rgba(0,0,0,.22);font:600 11px/1 var(--font);
/// color:var(--fg)}` `.kc.dim{opacity:.55}` - the shared `ibtn::keycap` (CAP).
fn keycaps(s: &str, dim: bool) -> Vec<El> {
    let mut out = Vec::new();
    for (i, p) in s.split(" + ").enumerate() {
        if i > 0 {
            out.push(El::text("+", Font::new(11.0, 400), FG3(), lh(11.0, 1.35)).none());
        }
        out.push(pieces::ibtn::keycap(p, &pieces::ibtn::CAP, dim));
    }
    out
}

/// "Check for updates": `.cbtn.sm.abb` = the small `.cbtn` (26 px, padding 0 12px, 12 px text) with an icon:
/// `#sw .abb{display:inline-flex;align-items:center;gap:6px}` `.abb svg{width:13px;height:13px;stroke-width:1.5}`
/// `.abb.busy svg{animation:spin .8s linear infinite}`. (Settings only: the shared `ibtn::icbtn` is the 30 px dialog button.)
fn abb(cx: &mut Cx, k: Key, label: &str, busy: bool) -> El {
    let hv = cx.hover_t(k, 150.0, EASE);
    let pr = cx.active_t(k, 120.0, EASE);
    let icon = if busy { progress::spinner(cx, "upd", 13.0, FG()) } else { El::icon("upd", 13.0, 1.5, FG()) };
    El::row()
        .center()
        .justify(JustifyContent::CENTER)
        .gap(6.0)
        .h(26.0)
        .pad(0.0, 12.0, 0.0, 12.0)
        .radius(7.0)
        .bg(cmix(CTL(), CTL_H(), hv))
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
        .none()
        .scale(1.0 - 0.03 * pr)
        .on_click(k)
        .cursor(Cursor::Hand)
        .child(icon.no_hit())
        .child(El::text(label, pieces::btn_font(12.0, 400), FG(), lh(12.0, 1.35)))
}

/// The app icon (`PANE[32].dark` drawn at `size`): `<rect x=2 y=2 width=28 height=28 rx=7.2 fill=#1d2434/>
/// <rect x=2.5 y=2.5 width=27 height=27 rx=6.7 stroke=rgba(255,255,255,.20) stroke-width=1/>
/// <path d="M8 12H24M8 21H24" stroke=#fff stroke-opacity=.5 stroke-width=2 stroke-linecap=round/>
/// <circle cx=19.2 cy=12 r=3.5 fill=#3395ff/><circle cx=12.8 cy=21 r=3.5 fill=#fff/>` (viewBox 0 0 32 32).
fn app_icon(size: f32) -> El {
    El::paint(move |g: &Gfx, (x, y, _w, _h)| {
        let k = size / 32.0;
        g.fill_rr(x + 2.0 * k, y + 2.0 * k, 28.0 * k, 28.0 * k, 7.2 * k, Rgba::hex(0x1d2434));
        g.stroke_rr(x + 2.5 * k, y + 2.5 * k, 27.0 * k, 27.0 * k, 6.7 * k, k, Rgba(1.0, 1.0, 1.0, 0.2));
        g.line(x + 8.0 * k, y + 12.0 * k, x + 24.0 * k, y + 12.0 * k, 2.0 * k, Rgba(1.0, 1.0, 1.0, 0.5), true);
        g.line(x + 8.0 * k, y + 21.0 * k, x + 24.0 * k, y + 21.0 * k, 2.0 * k, Rgba(1.0, 1.0, 1.0, 0.5), true);
        g.fill_circle(x + 19.2 * k, y + 12.0 * k, 3.5 * k, Rgba::hex(0x3395ff));
        g.fill_circle(x + 12.8 * k, y + 21.0 * k, 3.5 * k, Rgba(1.0, 1.0, 1.0, 1.0));
    })
    .size(size, size)
    .none()
}

#[cfg(test)]
mod tests;

/// The drawing's placeMenu(btn, minW): under the button (+4), kept 8 px inside the window on the right (then right-aligned to
/// the button) and at the bottom (then above the button), rounded to whole pixels (`Math.round`).
fn place((bx, by, bw, bh): (f32, f32, f32, f32), mw: f32, mh: f32) -> (f32, f32) {
    let mut left = bx;
    let mut top = by + bh + 4.0;
    if left + mw > WIN_W - 8.0 {
        left = (bx + bw - mw).max(8.0);
    }
    if top + mh > crate::ui::WIN_H - 8.0 {
        top = (by - mh - 4.0).max(8.0);
    }
    (left.round(), top.round())
}

/// The laid-out height of a popup's content at width `w` (what `offsetHeight` gives the drawing).
fn measure(g: &Gfx, el: &El, w: f32) -> f32 {
    crate::ui::lay::Laid::new(g, El::block().w(w).child(el.clone()), w, None).nodes[0].rect.3
}

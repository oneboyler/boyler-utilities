//! The Startup tab (menu-v22 page `sup`), Order 021: everything that starts with Windows in one list (bu-startup):
//! the All / Normal / Hidden switch, "Starts with Windows" + "<on> of <shown> on", one row per entry (tile, name + the
//! v21 "Windows" badge, publisher, where-tag, impact dot, lock for Windows' own tasks / services, switch), the
//! "This is part of Windows" question before a Windows entry is switched off, the right-click menu, the reset line.
//! The list is read when the tab opens (a read: registry, Startup folders, tasks, services, the boot report) - off the UI
//! thread in a real copy.

mod svc;

use std::sync::mpsc::{channel, Receiver};

use bu_startup::saved::{State as SState, Target};
use bu_startup::{ImpactState, Impact, Kind as SKind, LockReason, StartupEntry, StartupError, StartupList, Switch, View};

use crate::anim::{Bezier, EASE};
use crate::gfx::{sh, Font, Rgba};
use crate::pages::tweaks::temp;
use crate::pages::{Env, Page};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, lh, sub, Cursor, El, IconPaint, Key, RADIUS_PILL};
use crate::ui::pieces::button::Kind;
use crate::ui::pieces::listrow::{tile, Tile};
use crate::ui::pieces::mitems::{self, It, Place, Row};
use crate::ui::pieces::{self, group, reset, seg, tip, toast, toggle};
use crate::ui::{AMBER, CTL, FG, FG2, FG3, GREEN, HOV, ICO_ON, SEL};
use crate::undo::{Resettable, Val};

use svc::Svc;

const K_SEG: Key = key("sup.seg");
const K_TG: Key = key("sup.tg");
const K_ROW: Key = key("sup.row");
const K_TIP: Key = key("sup.tip");
const K_ASK: Key = key("sup.ask");
const K_MENU: Key = key("sup.menu");
const K_RS: Key = key("sup.rs");
const K_TOAST: Key = key("sup.toast");
const LABELS: [&str; 3] = ["All", "Normal", "Hidden"];
const VIEWS: [View; 3] = [View::All, View::Normal, View::Hidden];
const EASE_OUT: Bezier = Bezier::new(0.0, 0.0, 0.58, 1.0);

/// The where-tag (`WHERE`): text and its tip.
fn where_tag(k: SKind) -> (&'static str, &'static str) {
    match k {
        SKind::Normal => ("Startup", "Task Manager shows this one too"),
        SKind::HiddenTask => ("Hidden: task", "A scheduled task · Task Manager doesn’t show it"),
        SKind::HiddenService => ("Hidden: service", "A service · Task Manager doesn’t show it"),
    }
}

/// What switching a Windows entry off does (the drawing's `warn`), by its program; others: the generic warning (the
/// drawing's badge tip "switching it off can break a Windows feature").
fn warn(e: &StartupEntry) -> &'static str {
    let f = e.path.as_ref().and_then(|p| p.file_name()).map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
    match f.as_str() {
        "securityhealthsystray.exe" => "Hides Windows Security’s tray icon and its alerts (e.g. a threat found).",
        _ => "Switching it off can break a Windows feature.",
    }
}

/// The drawing's tiles (`SUP[].g`, `bg`) by name; any other entry: a grey app tile. Shown until the entry's own icon is
/// read, and for an entry without one (`entry_icon`).
fn look(name: &str) -> Tile {
    let (glyph, a, b): (&'static str, u32, u32) = match name {
        "Discord" => ("chat", 0x8f95ff, 0x5a5fe0),
        "Steam" => ("pad", 0x6f8fb8, 0x2b3f5c),
        "OneDrive" | "Microsoft OneDrive" => ("cloud", 0x5ab4ff, 0x2a74e6),
        "Windows Security notification icon" | "Microsoft Defender Antivirus Service" => ("secu", 0x5ab4ff, 0x2a74e6),
        "Spotify" => ("note", 0x46d989, 0x1c9a5a),
        "NVIDIA App" | "NVIDIA LocalSystem Container" => ("chip", 0x9be15d, 0x4a9a1c),
        "OBS Studio" => ("rec", 0x7a808c, 0x3a3e47),
        "Wootility" => ("kb16", 0xffb86b, 0xe0661c),
        "GoogleUpdateTaskMachineUA" => ("upd", 0x7fb2ff, 0x3b6fd6),
        "Adobe Acrobat Update Task" => ("upd", 0xff7a76, 0xc4313f),
        "MicrosoftEdgeUpdateTaskMachineCore" => ("upd", 0x4fd3a8, 0x2a76e8),
        "EpicOnlineServices" => ("pad", 0x8a8f99, 0x3d4149),
        "Windows Audio" => ("spk", 0xa2abbd, 0x6c7487),
        _ => ("appw", 0xa2abbd, 0x6c7487),
    };
    Tile::Glyph { glyph, a: Rgba::hex(a), b: Rgba::hex(b) }
}

/// `.wbdg svg{width:9px;height:9px;fill:currentColor;stroke:none}` with ICON.win16 (the shared filled-icon paint).
fn win16_filled(c: Rgba) -> El {
    El::icon("win16", 9.0, 0.0, c).icon_paint(IconPaint { fill_all: true, classes: vec![] }).none()
}

enum Pop {
    /// "This is part of Windows": entry index, at (x, y) = the switch's bottom-right
    Ask(usize, f32, f32),
    /// right-click menu: entry index, at the pointer
    Menu(usize, f32, f32),
}

#[derive(Default)]
pub struct Startup {
    on: usize,
    svc: Option<Svc>,
    list: Option<StartupList>,
    loading: Option<Receiver<StartupList>>,
    real_read: bool,
    test: bool,
    pop: Option<Pop>,
    toast: Option<(String, f64)>,
    pressed: (f32, f32, f32, f32),
    /// rows that just appeared (the filter's slide-in): entry id -> when
    appear: Vec<(String, f64)>,
    /// the crate service for the reset line while the page is CLOSED (Settings › Reset, the uninstaller): built on the
    /// first `current` / `apply` that needs it (Order 036 addendum: `resettable()` stays cheap)
    rs: std::cell::RefCell<Option<Svc>>,
    /// what "Open file location" / "Search online" opened (test copies log instead of opening)
    pub opened: Vec<String>,
    /// the rows' order (entry ids), fixed when the list first arrives this visit (`set_list`)
    order: Vec<String>,
    /// Order 042 (the owner's test 2: "none of the startup apps seem to have icons"): entry id -> the app's own icon as
    /// Windows shows it, read on the list's helper thread right after the list (a row without one keeps its tile)
    icons: std::collections::HashMap<String, std::sync::Arc<crate::png::Pixels>>,
    icon_rx: Option<Receiver<Vec<(String, crate::png::Pixels)>>>,
    /// an admin row's switch waiting for Windows' admin prompt / the elevated copy: entry id, switched to, its answer
    admin_wait: Option<(String, bool, Receiver<AdminAnswer>)>,
}

/// The app's own icon for a row (helper thread only: it asks the shell): the entry's icon file, else its program - a
/// task's .png logo as it is, anything else (exe, dll, ico, shortcut) the way Explorer shows it.
#[cfg(windows)]
fn entry_icon(e: &StartupEntry) -> Option<crate::png::Pixels> {
    let p = e.icon_path.as_ref().or(e.path.as_ref())?;
    let s = p.to_string_lossy();
    if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("png")) {
        return crate::png::load_png(&s).ok();
    }
    crate::appinfo::icon_pixels(&s, 48)
}

type AdminAnswer = Result<(bu_startup::Change, Option<SState>), StartupError>;

/// A row's state as the change log keeps it: raw = the crate's text (`on`, `off`, `auto`, `manual`…), shown = the
/// drawing's words (`RS.sup`: a Task Manager row "Starts with Windows" / "Off", a task or service "On" / "Off").
fn val(t: &Target, s: SState) -> Val {
    let text = match (s.is_on(), t.is_normal()) {
        (true, true) => "Starts with Windows",
        (true, false) => "On",
        (false, _) => "Off",
    };
    Val::new(&s.to_text(), text)
}

/// The crate service the reset line uses while the page is closed: the FAKE one in a test copy (and in unit tests),
/// else the real one (a --real-read copy refuses every apply before it is used).
fn fresh_svc() -> Option<Svc> {
    if cfg!(test) || (crate::testmode::on() && !crate::testmode::real_read()) {
        return Some(Svc::sample());
    }
    #[cfg(windows)]
    {
        Some(Svc::real())
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// The change log's line for a switch: item = where the entry lives, old = its state before, new = read back (or what it
/// was switched to).
fn log_vals(c: &bu_startup::Change, st: Option<SState>, on: bool) -> (String, Val, Val) {
    let t = Target::of_change(c);
    let old = SState::before(c);
    let new = st.unwrap_or(match old {
        SState::Enabled(_) => SState::Enabled(on),
        SState::Service { delayed, .. } => {
            SState::Service { start: if on { bu_startup::ServiceStart::Automatic } else { bu_startup::ServiceStart::Manual }, delayed: on && delayed }
        }
    });
    (t.to_text(), val(&t, old), val(&t, new))
}

/// A short name for a target in an error ("OBS Studio is gone"): the value / task / service name.
fn target_name(t: &Target) -> String {
    match t {
        Target::Flag(s) => s.value_name.clone(),
        Target::Task(p) => p.rsplit('\\').next().unwrap_or(p).to_string(),
        Target::Service(n) => n.clone(),
    }
}

/// The sort group of a row: 0 = on, not Windows' own · 1 = on, Windows' own · 2 = off.
fn rank(e: &StartupEntry) -> u8 {
    match (e.enabled, e.windows_own) {
        (true, false) => 0,
        (true, true) => 1,
        (false, _) => 2,
    }
}

impl Startup {
    /// The crate service for the reset line: the open page's own one, else one built on first use (`rs`).
    fn with_svc<R>(&self, f: impl FnOnce(&Svc) -> R) -> Option<R> {
        if let Some(s) = &self.svc {
            return Some(f(s));
        }
        let mut c = self.rs.borrow_mut();
        if c.is_none() {
            *c = fresh_svc();
        }
        c.as_ref().map(f)
    }

    fn entries(&self) -> Vec<(usize, &StartupEntry)> {
        let Some(l) = &self.list else { return Vec::new() };
        let v = VIEWS[self.on];
        l.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| match v {
                View::All => true,
                View::Normal => e.kind == SKind::Normal,
                View::Hidden => e.kind != SKind::Normal,
            })
            .collect()
    }

    fn show_toast(&mut self, t: impl Into<String>, now: f64) {
        self.toast = Some((t.into(), now));
    }

    fn reload(&mut self) {
        if let Some(s) = &self.svc {
            let l = s.list();
            self.set_list(l);
        }
    }

    /// The list in the owner's order (Oct 8): on and not Windows' own first, then Windows' own that are on, then everything
    /// that is off (the crate's order inside each group). The order is fixed when the tab opens: a row switched while the
    /// tab is shown stays where it is (it does not jump away from the pointer); new rows go to their group's end.
    fn set_list(&mut self, mut l: StartupList) {
        if self.order.is_empty() {
            l.entries.sort_by_key(rank);
            self.order = l.entries.iter().map(|e| e.id.clone()).collect();
        } else {
            // a known row keeps its place; a new one goes right after the last known row of its group (or of an earlier group)
            let known = |e: &StartupEntry| self.order.iter().position(|id| *id == e.id);
            let last_of: Vec<Option<usize>> =
                (0..3u8).map(|g| l.entries.iter().filter(|e| rank(e) <= g).filter_map(known).max()).collect();
            let pos = |e: &StartupEntry| match known(e) {
                Some(p) => (p, 0),
                None => last_of[rank(e) as usize].map_or((0, 0), |p| (p, 1)),
            };
            l.entries.sort_by_key(pos);
        }
        self.list = Some(l);
    }

    fn err_text(e: &StartupError, name: &str) -> String {
        match e {
            StartupError::NeedsAdmin => crate::admin::NOT_CHANGED.into(),
            StartupError::Locked(_) => format!("Locked · {name} is part of Windows"),
            StartupError::UseSettings(_) => "Only Windows Settings can switch this one · Settings › Apps › Startup".into(),
            StartupError::Gone => format!("{name} is gone · the list is read again"),
            other => {
                let s = other.to_string();
                let mut c = s.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            }
        }
    }

    /// Switch entry i to `on` through the crate; the change goes into the ONE change log (Order 036): item = where the
    /// entry lives (`Target`), old = its state before (from the crate's undo value), new = its state read back.
    fn apply(&mut self, i: usize, on: bool, cx: &mut Cx) {
        let now = cx.now;
        let Some(e) = self.list.as_ref().and_then(|l| l.entries.get(i)).cloned() else { return };
        if self.real_read {
            self.show_toast("A read-only test copy changes nothing", now);
            return;
        }
        let Some(s) = &self.svc else { return };
        // an admin row on Windows: the change goes to the app's elevated copy behind Windows' admin prompt - off the UI
        // thread (the menu keeps painting; the row's switch dims until the answer, Order 039)
        #[cfg(windows)]
        if e.needs_admin() && matches!(s, Svc::Real(_)) {
            if self.admin_wait.is_some() {
                return;
            }
            let (tx, rx) = channel();
            let w = crate::services::Waker;
            let ec = e.clone();
            let _ = std::thread::Builder::new().name("bu-startup-admin".into()).spawn(move || {
                let s = Svc::real();
                let r = s.set(&ec, on).map(|c| {
                    let st = s.state_of(&Target::of_change(&c)).ok();
                    (c, st)
                });
                // logged here, not by the page: the menu may have closed meanwhile
                if let Ok((c, st)) = &r {
                    let (item, old, new) = log_vals(c, *st, on);
                    crate::undo::note("sup", &item, &ec.name, &old, &new);
                }
                let _ = tx.send(r);
                w.wake();
            });
            self.admin_wait = Some((e.id.clone(), on, rx));
            return;
        }
        let r = s.set(&e, on).map(|c| {
            let st = s.state_of(&Target::of_change(&c)).ok();
            (c, st)
        });
        self.done(&e, on, r, now, &mut |item, label, old, new| cx.record(item, label, old, new));
    }

    /// A switch's answer (now, or from the elevated copy): log it in the change log, read the list again, say how it went.
    fn done(&mut self, e: &StartupEntry, on: bool, r: Result<(bu_startup::Change, Option<SState>), StartupError>, now: f64, rec: &mut dyn FnMut(&str, &str, &Val, &Val)) {
        match r {
            Ok((c, st)) => {
                let (item, old, new) = log_vals(&c, st, on);
                rec(&item, &e.name, &old, &new);
                self.reload();
                let t = if on { format!("{} starts with Windows again", e.name) } else { format!("{} won’t start with Windows · switch it back any time", e.name) };
                self.show_toast(t, now);
            }
            Err(err) => {
                let t = Self::err_text(&err, &e.name);
                if matches!(err, StartupError::Gone) {
                    self.reload();
                }
                self.show_toast(t, now);
            }
        }
    }

    fn flip(&mut self, i: usize, cx: &mut Cx) {
        let now = cx.now;
        let Some(e) = self.list.as_ref().and_then(|l| l.entries.get(i)).cloned() else { return };
        if let Switch::Locked(r) = e.switch {
            let t = match r {
                LockReason::AntiCheat => format!("Locked · {} is anti-cheat", e.name),
                LockReason::RunOnce => format!("Locked · {} runs once at the next sign-in", e.name),
                _ => format!("Locked · {} is part of Windows", e.name),
            };
            self.show_toast(t, now);
            return;
        }
        if self.admin_wait.as_ref().is_some_and(|(id, ..)| *id == e.id) {
            return;
        }
        let to = !e.enabled;
        if e.windows_own && !to {
            // v21: a Windows entry asks first, by its switch (menuAt(tg, r.right, r.bottom, 260))
            let (x, y, w, h) = self.pressed;
            self.pop = Some(Pop::Ask(i, x + w, y + h));
            return;
        }
        self.apply(i, to, cx);
    }

    /// The right-click menu of entry i at the pointer (the frame's `Ev::Context` on a row).
    pub fn context(&mut self, i: usize, x: f32, y: f32) {
        self.pop = Some(Pop::Menu(i, x, y));
    }

    fn open_location(&mut self, e: &StartupEntry, now: f64) {
        let Some(p) = &e.path else {
            self.show_toast("Windows doesn’t say where this one is", now);
            return;
        };
        let f = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        self.opened.push(format!("select:{}", p.display()));
        if !self.test && !self.real_read {
            // Explorer opens with the file selected (the user's own click)
            let _ = std::process::Command::new("explorer.exe").arg(format!("/select,{}", p.display())).spawn();
        }
        self.show_toast(format!("Opens Explorer with {f} selected"), now);
    }

    fn search_online(&mut self, e: &StartupEntry, now: f64) {
        let q: String = e.name.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
        let url = format!("https://www.bing.com/search?q={q}");
        self.opened.push(url.clone());
        if !self.test && !self.real_read {
            let _ = std::process::Command::new("explorer.exe").arg(&url).spawn();
        }
        self.show_toast(format!("Searches the web for “{}”", e.name), now);
    }

    // ------------------------------------------------------------------ building

    /// `.row.srow{gap:10px;min-height:48px}`: `.lbl.ap{gap:11px}` = tile + `.snm` (name `.ttl` with the Windows badge / admin
    /// shield, publisher `small`), `.ctl{gap:6px}` = `.stgw` tag, `.impw` dot, `.wlk` lock, switch. `.soff` greys it.
    fn row(&self, cx: &mut Cx, i: usize, e: &StartupEntry, first: bool) -> El {
        let soff = cx.tr(idx(sub(K_ROW, "off"), i), 1, if e.enabled { 0.0 } else { 1.0 }, 250.0, EASE);
        let lock = matches!(e.switch, Switch::Locked(_));
        let hidden = e.kind != SKind::Normal;
        // `.srow.soff .ait{filter:grayscale(1);opacity:.42}`
        let mut t = match self.icons.get(&e.id) {
            // the app's own icon (greyed with the row: `filter:grayscale(1)`)
            Some(px) => {
                let t = tile(&Tile::Icon(px.clone()), 24.0).opacity(1.0 - 0.58 * soff);
                if soff > 0.0 {
                    t.color_filter(crate::gfx::CssColor::Grayscale(soff))
                } else {
                    t
                }
            }
            None => tile(&look(&e.name), 24.0).opacity(1.0 - 0.58 * soff),
        };
        if soff > 0.5 && !self.icons.contains_key(&e.id) {
            if let Tile::Glyph { glyph, a, b } = look(&e.name) {
                t = tile(&Tile::Glyph { glyph, a: a.gray(), b: b.gray() }, 24.0).opacity(1.0 - 0.58 * soff);
            }
        }
        let name_col = crate::ui::cmix(FG(), FG2(), soff);
        // `.ttl{display:flex;align-items:center;gap:4px;min-width:0}` (+ `.srow .ttl{min-width:0}`)
        // Order 045: `h('span',{class:'tti',text:a.name,title:a.name})`
        let mut ttl = El::row()
            .center()
            .gap(4.0)
            .min_w(0.0)
            .child(El::text(e.name.clone(), Font::new(13.0, 400), name_col, lh(13.0, 1.35)).ellipsis().key(idx(sub(K_TIP, "n"), i)).title(&e.name));
        if e.windows_own {
            // `.wbdg{display:inline-flex;align-items:center;gap:4px;height:17px;padding:0 6px 0 5px;margin-left:7px;border-radius:5px;
            //   background:rgba(255,214,10,.13);color:#ffd60a;box-shadow:inset 0 0 0 .5px rgba(255,214,10,.3);font-size:10.5px;
            //   font-weight:600;line-height:17px;letter-spacing:0}` `.wbdg svg{width:9px;height:9px;fill:currentColor}` (win16)
            // light (Order 033): `#sw.light .wbdg{background:rgba(196,140,0,.12);color:#9a6b00;box-shadow:inset 0 0 0 .5px rgba(196,140,0,.35)}`
            let (c, wbg, wrim) = if crate::ui::is_light() {
                (Rgba::hex(0x9a6b00), Rgba::rgba(196, 140, 0, 0.12), Rgba::rgba(196, 140, 0, 0.35))
            } else {
                (Rgba::hex(0xffd60a), Rgba::rgba(255, 214, 10, 0.13), Rgba::rgba(255, 214, 10, 0.3))
            };
            ttl = ttl.child(
                El::row()
                    .center()
                    .gap(4.0)
                    .h(17.0)
                    .none()
                    .pad(0.0, 6.0, 0.0, 5.0)
                    .margin(0.0, 0.0, 0.0, 7.0)
                    .radius(5.0)
                    .bg(wbg)
                    .inset(&[sh(0.0, 0.0, 0.0, 0.5, wrim)])
                    .key(idx(sub(K_TIP, "w"), i))
                    // Order 045: the `.wbdg` data-tip (locked / not locked)
                    .tip(if lock {
                        "Part of Windows \u{b7} locked, so nothing Windows needs can be switched off"
                    } else {
                        "Part of Windows \u{b7} switching it off can break a Windows feature"
                    })
                    .child(win16_filled(c))
                    .child(El::text("Windows", Font::new(10.5, 600).ls(0), c, 17.0)),
            );
        }
        // the drawing shields the hidden rows (`hid&&!a.lock`); a machine-wide Run key / Startup folder needs admin too
        // (HKLM StartupApproved), so the shield follows what the crate says
        if !lock && e.needs_admin() {
            ttl = ttl.child(tip::rq(cx, idx(sub(K_TIP, "a"), i), tip::Rq::Adm, 18.0, tip::texts::ADM, false));
        }
        let mut snm = El::col().min_w(0.0).child(ttl);
        if let Some(p) = &e.publisher {
            snm = snm.child(El::text(p.clone(), Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).ellipsis().margin(1.0, 0.0, 0.0, 0.0));
        }
        let lbl = El::row().center().gap(11.0).flex1().child(t).child(snm);
        // `.stgw{display:flex;justify-content:flex-end;width:104px}` `.stag{height:18px;padding:0 7px;border-radius:5px;
        //   background:var(--ctl);color:var(--fg2);font-size:10.5px;font-weight:600;line-height:18px}` `.stag.hid{background:var(--sel);
        //   color:var(--ico-on)}`; `.srow.soff .stag,.srow.soff .impw{opacity:.45}`
        let (tag, where_tip) = where_tag(e.kind);
        let (tbg, tfg) = if hidden { (SEL(), ICO_ON()) } else { (CTL(), FG2()) };
        let stag = El::row()
            .h(18.0)
            .none()
            .pad(0.0, 7.0, 0.0, 7.0)
            .radius(5.0)
            .bg(tbg)
            .opacity(1.0 - 0.55 * soff)
            // Order 045: `'data-tip':WHERE[a.w][1]`
            .key(idx(sub(K_TIP, "t"), i))
            .tip(where_tip)
            .child(El::text(tag, Font::new(10.5, 600), tfg, 18.0));
        let stgw = El::row().w(104.0).none().justify(taffy::style::JustifyContent::FLEX_END).child(stag);
        // `.impw{display:grid;place-items:center;width:22px;height:22px}` `i{width:8px;height:8px;border-radius:50%}`
        // `.high i{background:#ff6b5e}` `.med i{background:var(--amber)}` `.low i{background:var(--green)}`
        let dot = match e.impact {
            ImpactState::Measured(Impact::High, _) => Some(Rgba::hex(0xff6b5e)),
            ImpactState::Measured(Impact::Medium, _) => Some(AMBER()),
            ImpactState::Measured(Impact::Low, _) => Some(GREEN()),
            // not in the boot report / no report: no dot (unclear in the drawing - every sample row has one)
            _ => None,
        };
        let mut impw = El::block().size(22.0, 22.0).none().place_center().opacity(1.0 - 0.55 * soff);
        if let Some(c) = dot {
            impw = impw.child(El::block().size(8.0, 8.0).radius(RADIUS_PILL).bg(c));
        }
        // Order 045: `'data-tip':'Startup impact: '+IMPN[a.imp]` (IMPN = High / Medium / Low; no measured impact = no dot, no tip)
        let imp_n = match e.impact {
            ImpactState::Measured(Impact::High, _) => Some("High"),
            ImpactState::Measured(Impact::Medium, _) => Some("Medium"),
            ImpactState::Measured(Impact::Low, _) => Some("Low"),
            _ => None,
        };
        if let Some(n) = imp_n {
            impw = impw.key(idx(sub(K_TIP, "i"), i)).tip(&format!("Startup impact: {n}"));
        }
        let mut ctl = vec![stgw, impw];
        if lock {
            // `.srow .wlk{display:grid;place-items:center;width:14px;height:14px;margin-right:-2px;color:var(--fg3)}` svg 12 / 1.4
            // Order 045: `'data-tip':'Locked · part of Windows'`
            ctl.push(
                El::block()
                    .size(14.0, 14.0)
                    .none()
                    .margin(0.0, -2.0, 0.0, 0.0)
                    .place_center()
                    .key(idx(sub(K_TIP, "l"), i))
                    .tip("Locked \u{b7} part of Windows")
                    .child(El::icon("lock", 12.0, 1.4, FG3())),
            );
        }
        // `.srow.wlock .tg{opacity:.38;cursor:default}` - still clickable (it says why it's locked)
        let mut tg = toggle::toggle(cx, idx(K_TG, i), e.enabled, false);
        if lock {
            tg = tg.opacity(0.38).cursor(Cursor::Default);
        }
        // waiting for Windows' admin prompt (DESIGN "Admin flip": the switch dims, no clicks, then flips)
        if self.admin_wait.as_ref().is_some_and(|(id, ..)| *id == e.id) {
            tg = tg.opacity(0.55).cursor(Cursor::Default);
        }
        ctl.push(tg);
        let mut r = group::row(first, vec![lbl, El::row().none().center().gap(6.0).children(ctl)]).gap(10.0).min_h(48.0).key(idx(K_ROW, i));
        // `.srow.ctx{background:var(--hov)}` while its right-click menu is open
        if matches!(self.pop, Some(Pop::Menu(j, ..)) if j == i) {
            r = r.bg(HOV());
        }
        // the filter's slide-in: opacity 0 -> 1, translateY(-4px) -> 0, 240 ms ease-out
        if let Some((_, at)) = self.appear.iter().find(|(id, _)| *id == e.id) {
            let k = ((cx.now - at) / 240.0).clamp(0.0, 1.0);
            if k < 1.0 && !cx.rm {
                let v = EASE_OUT.ease(k) as f32;
                cx.st.busy = true;
                r = r.opacity(v).translate(0.0, -4.0 * (1.0 - v));
            }
        }
        r
    }
}

impl Page for Startup {
    fn id(&self) -> &'static str {
        "sup"
    }
    fn name(&self) -> &'static str {
        "Startup"
    }
    fn icon(&self) -> &'static str {
        "rocket"
    }
    fn open(&mut self, env: &Env, _now: f64) {
        self.test = env.fake();
        self.real_read = env.real_read;
        if env.fake() {
            let s = Svc::sample();
            let l = s.list();
            self.set_list(l);
            self.svc = Some(s);
            return;
        }
        #[cfg(windows)]
        {
            // the list is read off the UI thread (tasks / services / version info / the boot report can take a moment)
            self.svc = Some(Svc::real());
            let (tx, rx) = channel();
            let (itx, irx) = channel();
            let w = env.waker();
            let _ = std::thread::Builder::new().name("bu-startup-list".into()).spawn(move || {
                let l = bu_startup::Startup::new(bu_startup::real::RealOs::new()).list();
                let entries = l.entries.clone();
                let _ = tx.send(l);
                w.wake();
                // then the rows' icons (the shell's icon reading can take a moment: the list shows first)
                unsafe {
                    let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
                }
                let icons: Vec<_> = entries.iter().filter_map(|e| entry_icon(e).map(|px| (e.id.clone(), px))).collect();
                unsafe {
                    windows::Win32::System::Com::CoUninitialize();
                }
                let _ = itx.send(icons);
                w.wake();
            });
            self.loading = Some(rx);
            self.icon_rx = Some(irx);
        }
    }
    fn close(&mut self) {
        *self = Startup { on: self.on, ..Startup::default() };
    }
    fn ready(&self) -> bool {
        self.loading.is_none()
    }
    fn tick(&mut self, now: f64) -> bool {
        if let Some(rx) = &self.loading {
            if let Ok(l) = rx.try_recv() {
                self.loading = None;
                self.set_list(l);
                return true;
            }
        }
        if let Some(rx) = &self.icon_rx {
            if let Ok(v) = rx.try_recv() {
                self.icon_rx = None;
                self.icons = v.into_iter().map(|(id, px)| (id, std::sync::Arc::new(px))).collect();
                return true;
            }
        }
        if let Some((id, on, rx)) = &self.admin_wait {
            let got = match rx.try_recv() {
                Ok(r) => Some(r),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(StartupError::Os(bu_startup::OsError::Admin("The admin helper stopped".into())))),
                Err(_) => None,
            };
            if let Some(r) = got {
                let (id, on) = (id.clone(), *on);
                self.admin_wait = None;
                if let Some(e) = self.list.as_ref().and_then(|l| l.entries.iter().find(|e| e.id == id)).cloned() {
                    // (the worker thread logged the change already)
                    self.done(&e, on, r, now, &mut |_, _, _, _| {});
                }
                return true;
            }
        }
        let toast = self.toast.as_ref().is_some_and(|(_, t)| now - t < toast::SHOW_MS + 300.0);
        if !toast {
            self.toast = None;
        }
        toast
    }
    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        let rows: Vec<(usize, StartupEntry)> = self.entries().into_iter().map(|(i, e)| (i, e.clone())).collect();
        let on = rows.iter().filter(|(_, e)| e.enabled).count();
        let mut els = Vec::new();
        for (n, (i, e)) in rows.iter().enumerate() {
            els.push(self.row(cx, *i, e, n == 0));
        }
        // `.gh` + `.ghs{font-weight:400;color:var(--fg3)}`: "Starts with Windows  <on> of <shown> on"
        let gh = group::gh("Starts with Windows").child(El::text(format!("{on} of {} on", rows.len()), Font::new(11.0, 400), FG3(), lh(11.0, 1.35)));
        let mut out = vec![pieces::header(self.name(), Some(seg::seg(cx, K_SEG, &LABELS, self.on, true))), El::block().child(gh).child(group::grp(els))];
        out.push(reset::reset_line(cx, K_RS, None));
        out
    }
    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        let now = cx.now;
        if let Ev::Press(_, _, _, r) = ev {
            self.pressed = *r;
        }
        // the right-click menu (`contextmenu` on a row; its switch counts as the row)
        if let Ev::Context(k, x, y) = ev {
            let n = self.list.as_ref().map(|l| l.entries.len()).unwrap_or(0);
            if let Some(i) = (0..n).find(|&i| idx(K_ROW, i) == *k || idx(K_TG, i) == *k) {
                self.context(i, *x, *y);
            }
            return;
        }
        let Ev::Click(k) = ev else { return };
        let k = *k;
        if let Some(i) = (0..LABELS.len()).find(|&i| idx(K_SEG, i) == k) {
            if i != self.on {
                let before: Vec<String> = self.entries().iter().map(|(_, e)| e.id.clone()).collect();
                self.on = i;
                let new: Vec<String> = self.entries().iter().map(|(_, e)| e.id.clone()).filter(|id| !before.contains(id)).collect();
                self.appear = new.into_iter().map(|id| (id, now)).collect();
            }
            return;
        }
        let n = self.list.as_ref().map(|l| l.entries.len()).unwrap_or(0);
        if let Some(i) = (0..n).find(|&i| idx(K_TG, i) == k) {
            self.flip(i, cx);
            return;
        }
        if k == sub(K_RS, "pc") {
            // the frame's shared review over the ONE change log (Order 036)
            cx.open_reset(crate::undo::Kind::HowItWas, self.pressed);
            return;
        }
        match self.pop.take() {
            Some(Pop::Ask(i, x, y)) => {
                if k == sub(K_ASK, "go") {
                    self.apply(i, false, cx);
                } else if k != sub(K_ASK, "no") {
                    self.pop = Some(Pop::Ask(i, x, y));
                }
            }
            Some(Pop::Menu(i, x, y)) => {
                let e = self.list.as_ref().and_then(|l| l.entries.get(i)).cloned();
                match (e, k) {
                    // row 0 is the header line
                    (Some(e), k) if k == idx(K_MENU, 1) => self.open_location(&e, now),
                    (Some(e), k) if k == idx(K_MENU, 2) => self.search_online(&e, now),
                    _ => self.pop = Some(Pop::Menu(i, x, y)),
                }
            }
            None => {}
        }
    }
    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        let pop = match self.pop.take() {
            Some(Pop::Ask(i, x, y)) => {
                let e = self.list.as_ref().and_then(|l| l.entries.get(i)).cloned();
                let w = e.as_ref().map(warn).unwrap_or("");
                let el = mitems::confirm(cx, K_ASK, "This is part of Windows", &format!("{w} You can switch it back on any time."), "Keep on", "Turn off", Kind::Red, Place::At(x, y), 260.0);
                self.pop = Some(Pop::Ask(i, x, y));
                Some(el)
            }
            Some(Pop::Menu(i, x, y)) => {
                let path = self.list.as_ref().and_then(|l| l.entries.get(i)).map(|e| e.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| e.command.clone())).unwrap_or_default();
                // Order 045: `h('div',{class:'mhead',text:a.path,title:a.path})`
                let list = [Row::HeadTitled(&path, &path), Row::Item(It::icon("fold", "Open file location")), Row::Item(It::icon("globe", "Search online"))];
                let el = mitems::menu(cx, K_MENU, &list, Place::At(x, y), 190.0);
                self.pop = Some(Pop::Menu(i, x, y));
                Some(el)
            }
            None => None,
        };
        let t = self.toast.clone().map(|(t, at)| toast::toast(cx, K_TOAST, &t, at, false));
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
        let on: Vec<String> = self.entries().iter().filter(|(_, e)| e.enabled).map(|(_, e)| e.name.clone()).collect();
        format!("filter={} shown={} on={} pop={}", LABELS[self.on], self.entries().len(), on.join("|"), self.pop.is_some())
    }
}

#[cfg(test)]
mod tests;

/// The reset line (Order 036): "Back to how your PC was" from the ONE change log; Startup has no Windows defaults
/// (Windows has no default startup list). Items are `Target` texts, values `SState` texts.
impl Resettable for Startup {
    fn page_id(&self) -> &str {
        "sup"
    }
    fn page_title(&self) -> &str {
        "Startup"
    }
    fn has_windows_defaults(&self) -> bool {
        false
    }
    fn current(&self, item: &str) -> Option<Val> {
        let t = Target::from_text(item)?;
        self.with_svc(|s| s.state_of(&t).ok()).flatten().map(|st| val(&t, st))
    }
    fn apply(&mut self, item: &str, to: &Val) -> Result<(), String> {
        if self.real_read || crate::testmode::real_read() {
            return Err("A read-only test copy changes nothing".into());
        }
        let t = Target::from_text(item).ok_or("This entry isn’t known any more")?;
        let st = SState::from_text(&to.raw).ok_or("This value isn’t known")?;
        let r = self.with_svc(|s| s.put_back(&t, st)).ok_or("Startup can’t be read on this PC")?;
        r.map_err(|e| Self::err_text(&e, &target_name(&t)))?;
        if self.list.is_some() {
            // the open page shows the value it was put back to
            self.reload();
        }
        Ok(())
    }
}

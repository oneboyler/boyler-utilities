//! Display's part of the app's ONE change log (Order 036; the drawing's `RS.dsp`, menu-v21 `resetPop`). Every change this
//! tab makes on the PC is noted with the value it had before ([`rec`]); the frame's shared review puts them back through
//! the page's [`Resettable`]:
//! - `mode:<monitor>` "DELL 27″ · resolution" - a resolution / rate / scaling change once KEPT (one the 10 s countdown or
//!   Revert took back is never noted; a preset chip is the same Apply + Keep);
//! - `main` "Main display" - the monitor that was main;
//! - `bri:<monitor>` / `con:<monitor>` "DELL 27″ · brightness / contrast" - the monitor's own DDC/CI value;
//! - `vib:<monitor>` "DELL 27″ · vibrance" - the driver's level;
//! - `rules` "Switch automatically" - the app's own rules (the drawing lists them): "2 apps → none".
//!   Rule switches at a game's start go back by themselves when it closes: they are not changes.
//!   "Windows defaults" = vibrance 50 % (the driver's normal level) on each monitor + no rules.
//!   It works with the page closed (Settings › Reset, the uninstaller): the runtime is made on first need, never in
//!   `resettable()`.

use std::sync::Arc;

use bu_display::picture::vibrance_percent;
use bu_display::store::Store;
use bu_display::{DisplayOs, MonitorId, MonitorInfo, Mode, Vcp, VcpValue, VibranceRaw};

use crate::undo::{DefaultItem, Resettable, Val};

use super::rt::{self, Rt, Svc};
use super::Display;

pub const PAGE: &str = "dsp";
pub const MAIN: &str = "main";
pub const RULES: &str = "rules";
pub const RULES_LABEL: &str = "Switch automatically";
const NO_RULES: &str = "[]";

/// Note one change into the change log, from any thread: written at once where the services are free (notes queued
/// before it first), else queued for the main loop (`undo::note`). A unit test without the app's services keeps nothing
/// (its note would wait in the shared queue and land in another test's store).
pub fn rec(item: &str, label: &str, old: &Val, new: &Val) {
    let done = crate::services::try_with(|s| {
        crate::undo::flush(&mut s.store);
        let _ = crate::undo::record(&mut s.store, PAGE, item, label, old, new);
    });
    if done.is_none() && !cfg!(test) {
        crate::undo::note(PAGE, item, label, old, new);
    }
}

/// "DELL 27″" (the selector's name for a monitor).
pub fn mon_name(m: &MonitorInfo) -> String {
    let brand = m.name.split_whitespace().next().unwrap_or("Monitor");
    match m.diagonal_inches {
        Some(d) => format!("{brand} {}″", d.round() as u32),
        None => brand.to_string(),
    }
}

/// One item of the log.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Mode(MonitorId),
    Main,
    Ddc(MonitorId, Vcp),
    Vib(MonitorId),
    Rules,
}

impl Item {
    pub fn id(&self) -> String {
        match self {
            Item::Mode(m) => format!("mode:{m}"),
            Item::Main => MAIN.into(),
            Item::Ddc(m, Vcp::Brightness) => format!("bri:{m}"),
            Item::Ddc(m, Vcp::Contrast) => format!("con:{m}"),
            Item::Vib(m) => format!("vib:{m}"),
            Item::Rules => RULES.into(),
        }
    }

    pub fn parse(s: &str) -> Option<Item> {
        if s == MAIN {
            return Some(Item::Main);
        }
        if s == RULES {
            return Some(Item::Rules);
        }
        let (k, m) = s.split_once(':')?;
        let m = MonitorId(m.to_string());
        Some(match k {
            "mode" => Item::Mode(m),
            "bri" => Item::Ddc(m, Vcp::Brightness),
            "con" => Item::Ddc(m, Vcp::Contrast),
            "vib" => Item::Vib(m),
            _ => return None,
        })
    }

    /// The popup's words for it ("DELL 27″ · vibrance").
    pub fn label(&self, name: &str) -> String {
        match self {
            Item::Mode(_) => format!("{name} · resolution"),
            Item::Main => "Main display".into(),
            Item::Ddc(_, Vcp::Brightness) => format!("{name} · brightness"),
            Item::Ddc(_, Vcp::Contrast) => format!("{name} · contrast"),
            Item::Vib(_) => format!("{name} · vibrance"),
            Item::Rules => RULES_LABEL.into(),
        }
    }
}

/// A mode: raw = bu-display's text of it, shown "1920 × 1080 · 165 Hz · Keep aspect".
pub fn mode_val(svc: &Svc, id: &MonitorId, m: &Mode) -> Val {
    Val::new(&m.to_raw(), &format!("{} · {}", rt::mode_text(svc, id, m), m.scaling.label()))
}

/// The main display: raw = its id, shown by its name.
pub fn main_val(mons: &[MonitorInfo], id: &MonitorId) -> Val {
    Val::new(&id.0, &mons.iter().find(|m| &m.id == id).map(mon_name).unwrap_or_else(|| "Monitor".into()))
}

/// A DDC/CI value: raw = the monitor's own value, shown in %.
pub fn ddc_val(v: VcpValue) -> Val {
    Val::new(&v.current.to_string(), &format!("{} %", v.percent()))
}

/// Vibrance: raw = the driver's level, shown in % (50 = normal).
pub fn vib_val(v: &VibranceRaw) -> Val {
    Val::new(&v.current.to_string(), &format!("{} %", vibrance_percent(v)))
}

/// The rules: raw = all rows, shown "2 apps" (the ones switched on) / "none".
pub fn rules_val(st: &Store) -> Val {
    let all = st.rules.rules();
    let on = all.iter().filter(|r| r.enabled).count();
    let apps = |n: usize| if n == 1 { "1 app".to_string() } else { format!("{n} apps") };
    let text = match (all.len(), on) {
        (0, _) => "none".to_string(),
        (n, 0) => format!("{}, off", apps(n)),
        (_, n) => apps(n),
    };
    Val::new(&st.rules.rules_raw(), &text)
}

/// Rules the app had before Order 036 kept no log line: the first time the page opens with rules and no line, it notes
/// "none → today's rules" (the app's rules did not exist before the app).
pub fn seed_rules(rt: &Rt) {
    let has = crate::services::try_with(|s| crate::undo::read_record(&s.store, PAGE, RULES).is_some());
    if has != Some(false) {
        return;
    }
    let Some(now) = rt.store.lock().ok().map(|s| rules_val(&s)) else { return };
    if now.raw != NO_RULES {
        rec(RULES, RULES_LABEL, &Val::new(NO_RULES, "none"), &now);
    }
}

/// The runtime for a CLOSED page (Settings › Reset, the uninstaller): the app run's own (real) one; the fake in test
/// copies and unit tests; a --real-read copy's read-only one (its `apply` refuses before it).
fn build_rt() -> Arc<Rt> {
    #[cfg(windows)]
    if !(cfg!(test) || (crate::testmode::on() && !crate::testmode::real_read())) {
        return Rt::shared(crate::testmode::real_read());
    }
    Rt::fake_sample()
}

impl Display {
    /// The page's runtime when open, else one made on first need (never in `resettable()`: it must stay cheap).
    pub(super) fn any_rt(&self) -> Arc<Rt> {
        match &self.rt {
            Some(r) => r.clone(),
            None => self.lazy_rt.get_or_init(build_rt).clone(),
        }
    }
}

impl Resettable for Display {
    fn page_id(&self) -> &str {
        PAGE
    }
    fn page_title(&self) -> &str {
        "Display"
    }

    /// Mode, main display, vibrance and the rules are read live (cheap); DDC/CI answers are slow, so those lines use the
    /// last noted value.
    fn current(&self, item: &str) -> Option<Val> {
        let rt = self.any_rt();
        match Item::parse(item)? {
            Item::Rules => rt.store.lock().ok().map(|s| rules_val(&s)),
            Item::Mode(id) => {
                let s = rt.svc.lock().ok()?;
                let m = s.monitor(&id).ok()?.current;
                Some(mode_val(&s, &id, &m))
            }
            Item::Main => {
                let s = rt.svc.lock().ok()?;
                let mons = s.monitors().ok()?;
                let main = mons.iter().find(|m| m.is_main)?.id.clone();
                Some(main_val(&mons, &main))
            }
            Item::Vib(id) => rt.svc.lock().ok()?.os_mut().vibrance_get(&id).ok().map(|v| vib_val(&v)),
            Item::Ddc(..) => None,
        }
    }

    /// Vibrance 50 % (the driver's normal level) on every monitor that has it; no "Switch automatically" rules.
    fn windows_defaults(&self) -> Vec<DefaultItem> {
        let rt = self.any_rt();
        let mut out = Vec::new();
        if let Ok(mut s) = rt.svc.lock() {
            let mons = s.monitors().unwrap_or_default();
            for m in &mons {
                if let Ok(v) = s.os_mut().vibrance_get(&m.id) {
                    let it = Item::Vib(m.id.clone());
                    out.push(DefaultItem { item: it.id(), label: it.label(&mon_name(m)), now: vib_val(&v), default: Val::new(&v.default.to_string(), "50 %") });
                }
            }
        }
        if let Ok(st) = rt.store.lock() {
            out.push(DefaultItem { item: RULES.into(), label: RULES_LABEL.into(), now: rules_val(&st), default: Val::new(NO_RULES, "none") });
        }
        out
    }

    fn apply(&mut self, item: &str, to: &Val) -> Result<(), String> {
        if crate::testmode::real_read() {
            return Err("A read-only test copy changes nothing".into());
        }
        let it = Item::parse(item).ok_or("Not a Display setting")?;
        let rt = self.any_rt();
        match it {
            Item::Rules => {
                {
                    let mut st = rt.store.lock().map_err(|e| e.to_string())?;
                    let Store { presets, rules } = &mut *st;
                    rules.set_rules_raw(&to.raw, presets).map_err(|e| e.to_string())?;
                }
                rt.save();
                // rules may have gone: the watcher follows (no rules = no watcher)
                rt.sync_watcher();
            }
            it => {
                let mut s = rt.svc.lock().map_err(|e| e.to_string())?;
                let r = match it {
                    Item::Mode(id) => {
                        if s.pending().is_some() {
                            return Err("Keep or revert the new resolution first".into());
                        }
                        let m = Mode::from_raw(&to.raw).ok_or("Not a resolution")?;
                        // stored as Windows' saved setting too (it is the user's own old mode)
                        s.os_mut().apply_mode(&id, &m, true)
                    }
                    Item::Main => s.set_main(&MonitorId(to.raw.clone())).map(|_| ()),
                    Item::Ddc(id, vcp) => {
                        if bu_display::picture::ddc_excluded(&id) {
                            return Err(bu_display::DisplayError::DdcExcludedModel.to_string());
                        }
                        let v = to.raw.parse::<u32>().map_err(|_| "Not a monitor value")?;
                        s.os_mut().ddc_set(&id, vcp, v)
                    }
                    Item::Vib(id) => {
                        let v = to.raw.parse::<i32>().map_err(|_| "Not a vibrance level")?;
                        s.os_mut().vibrance_set(&id, v)
                    }
                    Item::Rules => Ok(()),
                };
                r.map_err(|e| e.to_string())?;
            }
        }
        rt.bump();
        // an open page shows what is on the PC now (its picture values read again)
        if self.rt.is_some() {
            self.pic = vec![None; self.mons.len()];
            self.reload();
            self.read_picture(self.sel);
        }
        Ok(())
    }
}

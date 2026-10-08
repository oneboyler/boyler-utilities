//! The picked part's settings (v21/v22 `btnBody`, `stickBody`, `trigBody`, `gyroBody`, `tpadBody`) and the Controller
//! settings popup's rows (`devIn` + the Light bar, A_015_02). Every control reads the game's layout (`PadView`) and writes
//! one `bu_controller::Change` (or a per-controller value) through the page's registry.
//!
//! Settings Steam's files don't have (Lane L's report, gap 4: gyro smoothing, Calibrate, touchpad "one button", turn off when
//! idle) are NOT shown (Order 045 item 13: "Nothing on screen may do nothing"); the Edge's Fn buttons (no slot of their own)
//! are shown as the drawing draws them but dimmed and not clickable.
//!
//! Order 042 item 9: every row ends in a small slot (`look::TAIL_W`) - the "Undo" link while it holds the last change,
//! and a reset (↺) while its value differs from Steam's (the game's Steam layout; Steam's default for the controller's own
//! file). Item 11: the stick's "Check stick drift".

use bu_controller::layout::Press;
use bu_controller::prefs::NOISE_STEPS;
use bu_controller::settings::{radius_to_pct, CURVES, DZ_SHAPES, DZ_SOURCES, FLICK_SNAPS, GYRO_AXES, GYRO_BUTTONS, HAPTICS, HAPTICS_GROUP};
use bu_controller::{Action, ButtonId, GyroSetting, PrefSetting, PressSetting, Side, StickMode, StickSetting, TouchMode, TouchSetting, TriggerSetting};
use taffy::style::AlignItems;

use super::look::{self, ActShow};
use super::pic::Pid;
use super::{fmt, from_raw, gyro_mode_v, show, stick_dz, sv, touch_mode_v, Back, Conv, Ctl, Drift, Fmt, Open, AW, DRIFT_MS, LW, PANEL_IN, W};
use crate::gfx::Font;
use crate::ui::cx::Cx;
use crate::ui::el::{idx, lh, sub, El, Key};
use crate::ui::pieces::segx::{self, Label};
use crate::ui::pieces::{button, dropdown, group, link, progress, toggle};
use crate::ui::{FG2, FG3};

/// What the row builders need besides the page: the label column, the row's width, whose settings they are.
#[derive(Clone, Copy)]
struct Cols {
    lw: f32,
    w: f32,
    /// the Controller settings popup (the controller's own file), not a part's window
    dlg: bool,
}

const PANEL: Cols = Cols { lw: LW, w: PANEL_IN, dlg: false };
/// `.pdvd .prw>span:first-child{width:150px}`; the dialog's content 480 - 2 x 18 (as wide as a part's window since the
/// rows got their undo / reset slot, Order 042), the group's padding 2 x 12
const DLG: Cols = Cols { lw: 150.0, w: super::DLG_W - 36.0 - 24.0, dlg: true };
use super::PANEL_LEFT;

/// A switch that is off when the file has no value / on when it holds anything but 0.
fn on_nz(r: Option<i64>) -> bool {
    r.unwrap_or(0) != 0
}

/// A switch that is on when the file has no value (Steam's default on), off only at 0.
fn on_unless0(r: Option<i64>) -> bool {
    r.unwrap_or(1) != 0
}

impl Open {
    /// A control's key (the test hook's `el:pad.c.<id>`).
    pub(super) fn k(id: &str) -> Key {
        crate::ui::el::key(&format!("pad.c.{id}"))
    }

    /// The words of a change made in this row ("Left stick · Dead zone", "Controller settings · Rumble").
    fn row_words(&self, c: Cols, label: &str) -> String {
        let area = if c.dlg { "Controller settings".to_string() } else { self.sel.map(|p| self.pname(p)).unwrap_or_else(|| "Controller".into()) };
        format!("{area} \u{b7} {label}")
    }

    /// Registers a row: its controls lead to it (the undo list's "made here" and words).
    fn row(&mut self, c: Cols, row: Key, controls: &[Key], label: &str) {
        let words = self.row_words(c, label);
        for k in controls {
            self.rows.insert(*k, (row, words.clone()));
        }
    }

    /// The row's end (`look::tail`): "Undo" while the newest change was made in this row, the reset while the value
    /// differs from Steam's (`back` = what it writes + its tip).
    fn tail(&mut self, cx: &mut Cx, row: Key, back: Option<(Back, String)>) -> El {
        let undo = (self.undo_at() == Some(row)).then(|| {
            let k = sub(row, "undo");
            self.reg(k, Ctl::Undo);
            k
        });
        let back = back.map(|(b, tip)| {
            let k = sub(row, "back");
            if let Some(r) = self.rows.get(&row).cloned() {
                self.rows.insert(k, r);
            }
            self.reg(k, Ctl::Back(b));
            (k, tip)
        });
        look::tail(cx, undo, back.as_ref().map(|(k, t)| (*k, t.as_str())))
    }

    /// Steam's value of a control, when it differs from what the row shows (`same` = shown alike).
    fn steam_back(&self, w: W, raw: Option<i64>, same: impl Fn(Option<i64>, Option<i64>) -> bool, tip: impl Fn(Option<i64>) -> String) -> Option<(Back, String)> {
        let st = self.steam_raw(w)?;
        if same(st, raw) {
            return None;
        }
        let tip = if matches!(w, W::Pref(_) | W::Noise) { "Back to Steam\u{2019}s default".to_string() } else { format!("Back to Steam\u{2019}s setting: {}", tip(st)) };
        Some((Self::back_of(w, st)?, tip))
    }

    #[allow(clippy::too_many_arguments)]
    fn r_slider(&mut self, cx: &mut Cx, c: Cols, id: &str, label: &str, (lo, hi, step): (f64, f64, f64), def: f64, conv: Conv, f: Fmt, w: W, raw: Option<i64>, dead: bool) -> El {
        self.r_slider_in(cx, c, id, label, (lo, hi, step), def, conv, f, w, raw, dead, (lo, hi))
    }

    /// A slider whose value stays inside `keep` (the drawing's setters: dead zone <= "full at" - 5, "full at" >= dead zone + 5).
    #[allow(clippy::too_many_arguments)]
    fn r_slider_in(&mut self, cx: &mut Cx, c: Cols, id: &str, label: &str, (lo, hi, step): (f64, f64, f64), def: f64, conv: Conv, f: Fmt, w: W, raw: Option<i64>, dead: bool, keep: (f64, f64)) -> El {
        let k = Self::k(id);
        let shown = |r: Option<i64>| r.map(|r| from_raw(r, conv)).unwrap_or(def).clamp(lo, hi);
        let mut v = shown(raw);
        if let Some((dk, dv)) = self.drag {
            if dk == k {
                v = dv;
            }
        }
        let v = v.clamp(lo, hi);
        self.reg(k, if dead { Ctl::Dead } else { Ctl::Slider { lo, hi, step, def, conv, fmt: f, w, keep } });
        self.row(c, k, &[k], label);
        let back = if dead { None } else { self.steam_back(w, raw, |a, b| (shown(a) - shown(b)).abs() < step / 2.0, |s| fmt(shown(s), f)) };
        let tail = self.tail(cx, k, back);
        let v01 = ((v - lo) / (hi - lo)) as f32;
        look::asr(cx, k, label, c.lw, c.w, v01, (step / (hi - lo).max(f64::EPSILON)) as f32, &fmt(v, f), dead, Some(tail))
    }

    /// A switch: `on_of` reads the file's value the way the switch shows it.
    #[allow(clippy::too_many_arguments)]
    fn r_toggle(&mut self, cx: &mut Cx, c: Cols, id: &str, label: &str, raw: Option<i64>, on_of: fn(Option<i64>) -> bool, w: W, on_v: Option<i64>, off_v: Option<i64>, dead: bool) -> El {
        let k = Self::k(id);
        let on = on_of(raw);
        self.reg(k, if dead { Ctl::Dead } else { Ctl::Toggle { on, w, on_v, off_v } });
        self.row(c, k, &[k], label);
        let back = if dead { None } else { self.steam_back(w, raw, |a, b| on_of(a) == on_of(b), |s| (if on_of(s) { "On" } else { "Off" }).into()) };
        let tail = self.tail(cx, k, back);
        look::prw(label, c.lw, vec![toggle::toggle(cx, k, on, false)], dead, Some(tail))
    }

    /// A segmented switch row (`pSeg`): segment i writes `vals[i]`. `cur_of` = the segment of a file value; None = a value
    /// the segment doesn't list (then no segment is on, and each one writes).
    #[allow(clippy::too_many_arguments)]
    fn r_seg(&mut self, cx: &mut Cx, c: Cols, id: &str, label: &str, labels: &[&str], vals: &[Option<i64>], raw: Option<i64>, cur_of: &dyn Fn(Option<i64>) -> Option<usize>, w: W, dead: bool) -> El {
        let k = Self::k(id);
        let cur = cur_of(raw);
        let keys: Vec<Key> = (0..vals.len()).map(|i| idx(k, i)).collect();
        for (i, v) in vals.iter().enumerate() {
            self.reg(keys[i], if dead { Ctl::Dead } else { Ctl::Val { v: *v, w, on: Some(i) == cur } });
        }
        self.row(c, k, &keys, label);
        let back = if dead { None } else { self.steam_back(w, raw, |a, b| cur_of(a) == cur_of(b), |s| cur_of(s).and_then(|i| labels.get(i)).map(|l| l.to_string()).unwrap_or_else(|| "its own".into())) };
        let tail = self.tail(cx, k, back);
        // `.prw .seg button{padding:0 9px}`: `segx::PRW` (fit; segment i = `Ev::Click(idx(k, i))`, as before)
        let labels: Vec<Label> = labels.iter().map(|l| Label::Text(l)).collect();
        look::prw(label, c.lw, vec![segx::seg_ex(cx, k, &labels, cur, &segx::PRW)], dead, Some(tail))
    }

    /// A popup button row (`puBtn`, `.pu` at a fixed width): shows `raw`, or `dflt` when the file has no value.
    #[allow(clippy::too_many_arguments)]
    fn r_menu(&mut self, cx: &mut Cx, c: Cols, id: &str, label: &str, items: Vec<(Option<i64>, String)>, raw: Option<i64>, dflt: Option<i64>, w: W, width: f32, dead: bool) -> El {
        let k = Self::k(id);
        let cur = raw.or(dflt);
        let mut items = items;
        // a value the list doesn't have (Steam wrote it): shown as its own item, never replaced by a listed one
        if !items.iter().any(|(v, _)| *v == cur) {
            items.push((cur, cur.map(|v| format!("Other ({v})")).unwrap_or_else(|| "Steam\u{2019}s own".into())));
        }
        let shown = items.iter().find(|(v, _)| *v == cur).map(|(_, l)| l.clone()).unwrap_or_default();
        let name_of = |v: Option<i64>| items.iter().find(|(x, _)| *x == v).map(|(_, l)| l.clone()).unwrap_or_else(|| v.map(|v| format!("Other ({v})")).unwrap_or_else(|| "Steam\u{2019}s own".into()));
        let back = if dead { None } else { self.steam_back(w, raw, |a, b| a.or(dflt) == b.or(dflt), |s| name_of(s.or(dflt))) };
        self.reg(k, if dead { Ctl::Dead } else { Ctl::Menu { items, cur, w, width } });
        self.row(c, k, &[k], label);
        let tail = self.tail(cx, k, back);
        look::prw(label, c.lw, vec![dropdown::dropdown(cx, k, &shown, Some(width))], dead, Some(tail))
    }

    /// A "does" chip row (`acChip`); `same` = unchanged from Steam's layout (quieter).
    fn r_act(&mut self, cx: &mut Cx, id: &str, label: &str, cur: &Action, w: AW, steam: Option<&Action>) -> El {
        let k = Self::k(id);
        self.reg(k, Ctl::Act { cur: cur.clone(), w, title: label.into() });
        self.row(PANEL, k, &[k], label);
        let same = steam.map(|s| s == cur).unwrap_or(false);
        let back = self.steam_act(w).filter(|s| s != cur).map(|s| (Back::Layout(vec![Self::act_change(w, s.clone())]), format!("Back to Steam\u{2019}s setting: {}", s.label(self.xbox()))));
        let tail = self.tail(cx, k, back);
        look::prw(label, PANEL.lw, vec![look::ach(cx, k, &show(cur, self.xbox()), same, false)], false, Some(tail))
    }

    /// A group of settings, always open (the owner, test build 2: "outer ring, haptics, etc, are closed down menus for some
    /// reason rather than being open inside of left stick" - the drawing's closed `.pfold`s are open sections now).
    fn r_sec(title: &str, rows: Vec<El>) -> El {
        look::psec(title, 10.0, rows)
    }

    fn choice(list: &[(i64, &str)]) -> Vec<(Option<i64>, String)> {
        list.iter().map(|(v, l)| (Some(*v), l.to_string())).collect()
    }

    /// "Check stick drift" (Order 042 item 11, the owner test build 2: "calculate stick drift or something"): hands off the
    /// stick for 5 s while a line fills; then how far it wandered from the centre on its own and a dead zone just above
    /// that, with one "Use it" (a normal, undoable dead-zone change). Reads the live stick only; nothing is written before
    /// "Use it".
    fn r_drift(&mut self, cx: &mut Cx, side: Side, now_dz: f64) -> El {
        let n = super::stick_id(side);
        let go = Self::k(&format!("{n}.drift"));
        let f12 = Font::new(12.0, 400);
        let can = self.connected().is_some() && (self.fake || self.live.is_some());
        let kids = match self.drift.filter(|d| matches!(d, Drift::Run { side: s, .. } | Drift::Done { side: s, .. } if *s == side)) {
            Some(Drift::Run { at, .. }) => {
                let left = (DRIFT_MS - (cx.now - at)).max(0.0);
                let share = (1.0 - left / DRIFT_MS) as f32;
                // the bar and the count move every frame while it runs (the live pass)
                vec![
                    progress::bar(cx, sub(go, "bar"), Some(share)).flex1().live(),
                    El::text(format!("Hands off \u{b7} {} s", (left / 1000.0).ceil() as i64), f12.tnum(), FG2(), lh(12.0, 1.35)).none().live(),
                ]
            }
            Some(Drift::Done { max, dz, .. }) => {
                let again = sub(go, "again");
                self.reg(again, Ctl::DriftGo(side));
                let pct = max as f64 * 100.0;
                let tip = format!("On its own the stick moved up to {pct:.1} % from the centre; a {dz} % dead zone hides that");
                let mut v = Vec::new();
                if (dz - now_dz).abs() < 0.5 {
                    v.push(El::text(format!("Drift {pct:.1} % \u{b7} your {dz} % covers it"), f12, FG2(), lh(12.0, 1.35)).none().key(sub(go, "res")).tip(&tip));
                } else {
                    let use_k = Self::k(&format!("{n}.drift.use"));
                    self.reg(use_k, Ctl::DriftUse(side, dz));
                    v.push(El::text(format!("Drift {pct:.1} % \u{2192} {dz} %"), f12, FG2(), lh(12.0, 1.35)).none().key(sub(go, "res")).tip(&tip));
                    v.push(button::cbtn(cx, use_k, "Use it", button::Kind::Primary, true, false, 0.0));
                }
                v.push(link::link(cx, again, "Again", 11.5));
                v
            }
            _ => {
                if can {
                    self.reg(go, Ctl::DriftGo(side));
                }
                let hint = if can { "leave the stick alone for 5 s" } else { "plug in the controller" };
                vec![button::cbtn(cx, go, "Check", button::Kind::Ghost, true, !can, 0.0), El::text(hint, Font::new(11.5, 400), FG3(), lh(11.5, 1.35)).none()]
            }
        };
        look::prw("Stick drift", PANEL.lw, kids, false, Some(El::block().w(look::TAIL_W).none()))
    }

    // ============================================================================================ the bodies
    pub(super) fn panel_body(&mut self, cx: &mut Cx, id: Pid) -> El {
        let kids = match id {
            Pid::Stick(s) => self.stick_body(cx, s),
            Pid::Trig(s) => self.trig_body(cx, s),
            Pid::Gyro => self.gyro_body(cx),
            Pid::Touch => self.tpad_body(cx),
            Pid::B(ButtonId::Home) => vec![
                look::prw("Does", PANEL.lw, vec![group::fixed("Opens Steam")], false, None),
                look::pnote("Steam keeps this button for its own menu."),
            ],
            Pid::Fn(_) => vec![
                look::prw("Does", PANEL.lw, vec![look::ach(cx, Self::k("fn"), &ActShow::Nothing, false, true)], true, None),
                look::pnote("Steam has no setting of its own for the Fn buttons."),
            ],
            Pid::B(b) => self.btn_body(cx, b),
            Pid::Light => vec![],
        };
        // `.pdx` (a column of rows)
        El::col().items(AlignItems::STRETCH).w(PANEL_IN).children(kids)
    }

    fn btn_body(&mut self, cx: &mut Cx, b: ButtonId) -> Vec<El> {
        let Some(v) = self.view.as_ref().and_then(|v| v.buttons.iter().find(|x| x.id == b)).cloned() else {
            return vec![look::pnote("This layout has no setting for this button.")];
        };
        let steam = self.steam.as_ref().and_then(|s| s.buttons.iter().find(|x| x.id == b)).cloned();
        let act = |p: Press| v.presses.iter().find(|(q, _)| *q == p).map(|(_, a)| a.clone()).unwrap_or(Action::Nothing);
        let st_act = |p: Press| steam.as_ref().and_then(|s| s.presses.iter().find(|(q, _)| *q == p).map(|(_, a)| a.clone()));
        let n = format!("{b:?}").to_lowercase();
        let does = self.r_act(cx, &format!("{n}.full"), "Does", &act(Press::Full), AW::Btn(b, Press::Full), st_act(Press::Full).as_ref());
        let long = self.r_act(cx, &format!("{n}.long"), "Long press", &act(Press::Long), AW::Btn(b, Press::Long), None);
        let dbl = self.r_act(cx, &format!("{n}.double"), "Double press", &act(Press::Double), AW::Btn(b, Press::Double), None);
        let press = look::psec_first("Press", vec![does, long, dbl]);
        // "More ways to press": start, release, together with
        let start = self.r_act(cx, &format!("{n}.start"), "Start press", &act(Press::Start), AW::Btn(b, Press::Start), None);
        let rel = self.r_act(cx, &format!("{n}.release"), "Release press", &act(Press::Release), AW::Btn(b, Press::Release), None);
        let chord_b = sv(&v.settings, PressSetting::ChordButton);
        let ck = Self::k(&format!("{n}.chordb"));
        let items = Self::choice(bu_controller::settings::GYRO_BUTTONS);
        let name_of = |x: Option<i64>| items.iter().find(|(y, _)| *y == x).map(|(_, l)| l.clone()).unwrap_or_else(|| "L1".into());
        let shown = name_of(chord_b);
        self.reg(ck, Ctl::Menu { items: items.clone(), cur: chord_b, w: W::Btn(b, PressSetting::ChordButton), width: 110.0 });
        let cak = Self::k(&format!("{n}.chord"));
        let chord_a = act(Press::Chord);
        self.reg(cak, Ctl::Act { cur: chord_a.clone(), w: AW::Btn(b, Press::Chord), title: "Chorded press".into() });
        self.row(PANEL, ck, &[ck, cak], "Together with");
        // its reset: both values back to Steam's (one write)
        let st_b = steam.as_ref().map(|s| sv(&s.settings, PressSetting::ChordButton));
        let st_a = st_act(Press::Chord);
        let back = match (st_b, st_a) {
            (Some(sb), Some(sa)) if sb != chord_b || sa != chord_a => {
                let mut c = Vec::new();
                if sb != chord_b {
                    c.extend(Self::layout_change(W::Btn(b, PressSetting::ChordButton), sb));
                }
                if sa != chord_a {
                    c.push(Self::act_change(AW::Btn(b, Press::Chord), sa.clone()));
                }
                Some((super::Back::Layout(c), format!("Back to Steam\u{2019}s setting: {} \u{b7} {}", name_of(sb), sa.label(self.xbox()))))
            }
            _ => None,
        };
        let tail = self.tail(cx, ck, back);
        let together = look::prw("Together with", PANEL.lw, vec![dropdown::dropdown(cx, ck, &shown, Some(110.0)), look::ach(cx, cak, &show(&chord_a, self.xbox()), false, false)], false, Some(tail));
        let more = Self::r_sec("More ways to press", vec![start, rel, together]);
        // "Press settings" (Steam's Regular Press Settings)
        let s = |p: PressSetting| sv(&v.settings, p);
        let turbo = on_nz(s(PressSetting::HoldToRepeat));
        let rows = vec![
            self.r_toggle(cx, PANEL, &format!("{n}.turbo"), "Hold to repeat (turbo)", s(PressSetting::HoldToRepeat), on_nz, W::Btn(b, PressSetting::HoldToRepeat), Some(1), None, false),
            self.r_slider(cx, PANEL, &format!("{n}.rate"), "Repeat every", (0.0, 1000.0, 1.0), 100.0, Conv::Raw, Fmt::Ms, W::Btn(b, PressSetting::RepeatRate), s(PressSetting::RepeatRate), !turbo),
            self.r_toggle(cx, PANEL, &format!("{n}.tog"), "Toggle", s(PressSetting::Toggle), on_nz, W::Btn(b, PressSetting::Toggle), Some(1), None, false),
            self.r_toggle(cx, PANEL, &format!("{n}.cyc"), "Cycle commands", s(PressSetting::CycleCommands), on_nz, W::Btn(b, PressSetting::CycleCommands), Some(1), None, false),
            self.r_toggle(cx, PANEL, &format!("{n}.intr"), "Interruptible", s(PressSetting::Interruptible), on_unless0, W::Btn(b, PressSetting::Interruptible), None, Some(0), false),
            self.r_toggle(cx, PANEL, &format!("{n}.inv"), "Invert input", s(PressSetting::InvertInput), on_nz, W::Btn(b, PressSetting::InvertInput), Some(1), None, false),
            self.r_slider(cx, PANEL, &format!("{n}.fs"), "Fire start delay", (0.0, 1000.0, 1.0), 0.0, Conv::Raw, Fmt::Ms, W::Btn(b, PressSetting::FireStartDelay), s(PressSetting::FireStartDelay), false),
            self.r_slider(cx, PANEL, &format!("{n}.fe"), "Fire end delay", (0.0, 1000.0, 1.0), 0.0, Conv::Raw, Fmt::Ms, W::Btn(b, PressSetting::FireEndDelay), s(PressSetting::FireEndDelay), false),
            self.r_menu(cx, PANEL, &format!("{n}.hap"), "Haptics", Self::choice(HAPTICS), s(PressSetting::Haptics), Some(0), W::Btn(b, PressSetting::Haptics), 112.0, false),
        ];
        let ps = Self::r_sec("Press settings", rows);
        vec![press, more, ps]
    }

    fn stick_body(&mut self, cx: &mut Cx, side: Side) -> Vec<El> {
        let Some(v) = self.view.as_ref().and_then(|v| v.sticks.iter().find(|s| s.side == side)).cloned() else { return vec![] };
        let n = super::stick_id(side);
        let s = |x: StickSetting| sv(&v.settings, x);
        let mode_i = StickMode::LISTED.iter().position(|m| *m == v.mode).map(|i| i as i64);
        let mut modes: Vec<(Option<i64>, String)> = StickMode::LISTED.iter().enumerate().map(|(i, m)| (Some(i as i64), m.label().to_string())).collect();
        if mode_i.is_none() {
            modes.push((None, v.mode.label().to_string()));
        }
        let acts = self.r_menu(cx, PANEL, &format!("{n}.mode"), "Acts as", modes, mode_i, None, W::StickMode(side), 150.0, false);
        // the dead zone circle + the response curve (live dot = the real stick)
        let (inner, outer) = stick_dz(&v);
        let (ik, ok) = (Self::k(&format!("{n}.dz")), Self::k(&format!("{n}.full")));
        let inner = self.drag.filter(|d| d.0 == ik).map(|d| d.1).unwrap_or(inner);
        let outer = self.drag.filter(|d| d.0 == ok).map(|d| d.1).unwrap_or(outer);
        let curve = s(StickSetting::Curve).unwrap_or(0);
        let shape = s(StickSetting::CurveShape).map(|r| r as f64 / 100.0).unwrap_or(1.0);
        let sens = s(StickSetting::Sensitivity).map(|r| r as f64).unwrap_or(100.0);
        let anti = s(StickSetting::AntiDeadZone).map(radius_to_pct).unwrap_or(0.0);
        let shp = s(StickSetting::DeadZoneShape).unwrap_or(1);
        let lp = if side == Side::Left { self.lv.ls } else { self.lv.rs };
        let live = !self.frozen && (self.fake || self.live.is_some());
        let wk = Self::k(&format!("{n}.well"));
        self.reg(wk, Ctl::Well { inner: ik, outer: ok, at: (inner, outer) });
        let wells = wells(cx, wk, Wells { inner, outer, curve, shape, sens, anti, shp, lp }, live);
        let dz = self.r_slider_in(cx, PANEL, &format!("{n}.dz"), "Dead zone", (0.0, 60.0, 1.0), 8.0, Conv::Radius, Fmt::Pct, W::Stick(side, StickSetting::DeadZone), s(StickSetting::DeadZone), false, (0.0, outer - 5.0));
        let full = self.r_slider_in(cx, PANEL, &format!("{n}.full"), "Full at", (40.0, 100.0, 1.0), 100.0, Conv::Radius, Fmt::Pct, W::Stick(side, StickSetting::FullAt), s(StickSetting::FullAt), false, (inner + 5.0, 100.0));
        let shape_of = |r: Option<i64>| DZ_SHAPES.iter().position(|(v, _)| *v == r.unwrap_or(1));
        let shp_row = self.r_seg(cx, PANEL, &format!("{n}.shape"), "Shape", &["Circle", "Cross", "Square"], &[Some(1), Some(0), Some(2)], s(StickSetting::DeadZoneShape), &shape_of, W::Stick(side, StickSetting::DeadZoneShape), false);
        let src = self.r_menu(cx, PANEL, &format!("{n}.src"), "Source", Self::choice(DZ_SOURCES), s(StickSetting::DeadZoneSource), Some(2), W::Stick(side, StickSetting::DeadZoneSource), 170.0, false);
        let drift = self.r_drift(cx, side, stick_dz(&v).0.round());
        let dz_sec = look::psec("Dead zone", 0.0, vec![wells, dz, full, shp_row, src, drift]);
        let adz = vec![
            self.r_slider(cx, PANEL, &format!("{n}.anti"), "Anti-dead zone", (0.0, 40.0, 1.0), 0.0, Conv::Radius, Fmt::Pct, W::Stick(side, StickSetting::AntiDeadZone), s(StickSetting::AntiDeadZone), false),
            self.r_slider(cx, PANEL, &format!("{n}.antib"), "Anti-dead zone buffer", (0.0, 40.0, 1.0), 0.0, Conv::Radius, Fmt::Pct, W::Stick(side, StickSetting::AntiDeadZoneBuffer), s(StickSetting::AntiDeadZoneBuffer), false),
        ];
        let adz = Self::r_sec("Advanced dead zone", adz);
        // Response
        let bid = if side == Side::Left { ButtonId::L3 } else { ButtonId::R3 };
        let curve_row = self.r_menu(cx, PANEL, &format!("{n}.curve"), "Curve", Self::choice(CURVES), s(StickSetting::Curve), Some(0), W::Stick(side, StickSetting::Curve), 120.0, false);
        let mut resp = vec![curve_row];
        if curve == 5 {
            resp.push(self.r_slider(cx, PANEL, &format!("{n}.cshape"), "Curve shape", (0.3, 3.0, 0.05), 1.0, Conv::Div100, Fmt::Shape, W::Stick(side, StickSetting::CurveShape), s(StickSetting::CurveShape), false));
        }
        resp.push(self.r_slider(cx, PANEL, &format!("{n}.sens"), "Sensitivity", (10.0, 300.0, 5.0), 100.0, Conv::Raw, Fmt::Pct, W::Stick(side, StickSetting::Sensitivity), s(StickSetting::Sensitivity), false));
        let st_press = self.steam.as_ref().and_then(|x| x.sticks.iter().find(|q| q.side == side)).map(|q| q.press.clone());
        let pname = self.pname(Pid::B(bid)).replace(" press", "");
        resp.push(self.r_act(cx, &format!("{n}.press"), &format!("Press ({pname})"), &v.press, AW::Btn(bid, Press::Full), st_press.as_ref()));
        let resp = look::psec("Response", 10.0, resp);
        // Output
        let own = usize::from(side == Side::Right);
        let out_of = move |r: Option<i64>| match r {
            Some(1) => Some(1),
            Some(0) => Some(0),
            None => Some(own),
            _ => None, // e.g. 2 = mouse: not one of the two segments
        };
        let outp = vec![
            self.r_seg(cx, PANEL, &format!("{n}.out"), "Sends to", &["Left stick", "Right stick"], &[Some(0), Some(1)], s(StickSetting::SendsTo), &out_of, W::Stick(side, StickSetting::SendsTo), false),
            self.r_slider(cx, PANEL, &format!("{n}.hs"), "Left-right speed", (10.0, 300.0, 5.0), 100.0, Conv::Raw, Fmt::Pct, W::Stick(side, StickSetting::LeftRightSpeed), s(StickSetting::LeftRightSpeed), false),
            self.r_slider(cx, PANEL, &format!("{n}.vs"), "Up-down speed", (10.0, 300.0, 5.0), 100.0, Conv::Raw, Fmt::Pct, W::Stick(side, StickSetting::UpDownSpeed), s(StickSetting::UpDownSpeed), false),
            self.r_toggle(cx, PANEL, &format!("{n}.ix"), "Invert left-right", s(StickSetting::InvertLeftRight), on_nz, W::Stick(side, StickSetting::InvertLeftRight), Some(1), None, false),
            self.r_toggle(cx, PANEL, &format!("{n}.iy"), "Invert up-down", s(StickSetting::InvertUpDown), on_nz, W::Stick(side, StickSetting::InvertUpDown), Some(1), None, false),
            self.r_toggle(cx, PANEL, &format!("{n}.smooth"), "Smoothing", s(StickSetting::Smoothing), on_nz, W::Stick(side, StickSetting::Smoothing), Some(1), None, false),
        ];
        let outp = Self::r_sec("Output", outp);
        // Outer ring
        let ring = vec![
            self.r_act(cx, &format!("{n}.ring"), "At the edge does", &v.ring_action, AW::Ring(side), None),
            self.r_slider(cx, PANEL, &format!("{n}.rr"), "Ring starts at", (50.0, 100.0, 1.0), 90.0, Conv::Radius, Fmt::Pct, W::Stick(side, StickSetting::RingStartsAt), s(StickSetting::RingStartsAt), false),
            self.r_toggle(cx, PANEL, &format!("{n}.rinv"), "Inside instead", s(StickSetting::RingInsideInstead), on_nz, W::Stick(side, StickSetting::RingInsideInstead), Some(1), None, false),
        ];
        let ring = Self::r_sec("Outer ring", ring);
        let mut out = vec![acts, dz_sec, adz, resp, outp, ring];
        if v.mode == StickMode::FlickStick {
            let fl = vec![
                self.r_slider(cx, PANEL, &format!("{n}.fls"), "Turn speed", (10.0, 300.0, 5.0), 100.0, Conv::Raw, Fmt::Pct, W::Stick(side, StickSetting::FlickTurnSpeed), s(StickSetting::FlickTurnSpeed), false),
                self.r_menu(cx, PANEL, &format!("{n}.snap"), "Snap", Self::choice(FLICK_SNAPS), s(StickSetting::FlickSnap), Some(0), W::Stick(side, StickSetting::FlickSnap), 126.0, false),
                self.r_slider(cx, PANEL, &format!("{n}.fwd"), "Forward zone", (0.0, 45.0, 1.0), 7.0, Conv::Raw, Fmt::Deg, W::Stick(side, StickSetting::FlickForwardZone), s(StickSetting::FlickForwardZone), false),
                self.r_slider(cx, PANEL, &format!("{n}.sweep"), "Sweep speed", (10.0, 300.0, 5.0), 100.0, Conv::Raw, Fmt::Pct, W::Stick(side, StickSetting::FlickSweepSpeed), s(StickSetting::FlickSweepSpeed), false),
            ];
            out.push(Self::r_sec("Flick stick", fl));
        }
        let hap = vec![self.r_menu(cx, PANEL, &format!("{n}.hap"), "Haptic intensity", Self::choice(HAPTICS), s(StickSetting::Haptics), Some(0), W::Stick(side, StickSetting::Haptics), 112.0, false)];
        out.push(Self::r_sec("Haptics", hap));
        out
    }

    fn trig_body(&mut self, cx: &mut Cx, side: Side) -> Vec<El> {
        let Some(t) = self.view.as_ref().and_then(|v| v.triggers.iter().find(|x| x.side == side)).cloned() else { return vec![] };
        let steam = self.steam.as_ref().and_then(|v| v.triggers.iter().find(|x| x.side == side)).cloned();
        let n = if side == Side::Left { "l2" } else { "r2" };
        let s = |x: TriggerSetting| sv(&t.settings, x);
        let at_k = Self::k(&format!("{n}.at"));
        let at = self.drag.filter(|d| d.0 == at_k).map(|d| d.1).unwrap_or_else(|| s(TriggerSetting::ClicksAt).map(radius_to_pct).unwrap_or(100.0));
        let pull = if side == Side::Left { self.lv.l2 } else { self.lv.r2 };
        // Analog = 0, Click only = 1 (`W::TrigAnalog`)
        let analog_of = |r: Option<i64>| r.map(|r| r as usize);
        let mode = self.r_seg(cx, PANEL, &format!("{n}.mode"), "Mode", &["Analog", "Click only"], &[Some(0), Some(1)], Some(i64::from(!t.analog)), &analog_of, W::TrigAnalog(side), false);
        let at_row = self.r_slider(cx, PANEL, &format!("{n}.at"), "Clicks at", (5.0, 100.0, 1.0), 100.0, Conv::Radius, Fmt::FullPull, W::Trig(side, TriggerSetting::ClicksAt), s(TriggerSetting::ClicksAt), false);
        let live = !self.frozen && (self.fake || self.live.is_some());
        // Order 045: `h('div',{class:'pdtr','data-tip':'Where the pull counts as a click'})`
        let bar = look::pdtr((at / 100.0) as f32, pull, live).key(Self::k(&format!("{n}.bar"))).tip("Where the pull counts as a click");
        let click = self.r_act(cx, &format!("{n}.click"), "Click does", &t.click, AW::Trig(side, false), steam.as_ref().map(|x| &x.click));
        let soft = self.r_act(cx, &format!("{n}.soft"), "Soft pull does", &t.soft_pull, AW::Trig(side, true), None);
        let pull_sec = look::psec_first("Pull", vec![mode, at_row, bar, click, soft]);
        let curves: Vec<(i64, &str)> = CURVES.iter().filter(|(v, _)| *v != 5).cloned().collect();
        let dz = vec![
            self.r_slider(cx, PANEL, &format!("{n}.dz"), "Dead zone", (0.0, 40.0, 1.0), 0.0, Conv::Radius, Fmt::Pct, W::Trig(side, TriggerSetting::DeadZone), s(TriggerSetting::DeadZone), false),
            self.r_slider(cx, PANEL, &format!("{n}.full"), "Full at", (50.0, 100.0, 1.0), 100.0, Conv::Radius, Fmt::Pct, W::Trig(side, TriggerSetting::FullAt), s(TriggerSetting::FullAt), false),
            self.r_menu(cx, PANEL, &format!("{n}.curve"), "Curve", Self::choice(&curves), s(TriggerSetting::Curve), Some(0), W::Trig(side, TriggerSetting::Curve), 120.0, false),
        ];
        let dz = Self::r_sec("Dead zone & curve", dz);
        let own = if side == Side::Left { 1 } else { 2 };
        let out_of = move |r: Option<i64>| match r.unwrap_or(own) {
            1 => Some(0),
            2 => Some(1),
            0 => Some(2),
            _ => None,
        };
        let l2 = self.pname(Pid::Trig(Side::Left)).replace(" trigger", "");
        let r2 = self.pname(Pid::Trig(Side::Right)).replace(" trigger", "");
        // the drawing's four (Off / Low / Medium / High) + Steam's "each press's own" (5) when the file holds it or says nothing
        let hap_v = s(TriggerSetting::Haptics);
        let mut haps: Vec<(Option<i64>, String)> = HAPTICS_GROUP.iter().filter(|(v, _)| *v != 5).map(|(v, l)| (Some(*v), l.to_string())).collect();
        if matches!(hap_v, None | Some(5)) {
            haps.push((hap_v, "Each press\u{2019}s own".to_string())); // other odd values: r_menu's "Other (n)"
        }
        let outp = vec![
            self.r_seg(cx, PANEL, &format!("{n}.out"), "Sends to", &[&l2, &r2, "Nothing"], &[Some(1), Some(2), Some(0)], s(TriggerSetting::SendsTo), &out_of, W::Trig(side, TriggerSetting::SendsTo), false),
            self.r_menu(cx, PANEL, &format!("{n}.hap"), "Haptics", haps, hap_v, None, W::Trig(side, TriggerSetting::Haptics), 112.0, false),
        ];
        let outp = Self::r_sec("Output & haptics", outp);
        vec![pull_sec, dz, outp]
    }

    fn gyro_body(&mut self, cx: &mut Cx) -> Vec<El> {
        let Some(g) = self.view.as_ref().and_then(|v| v.gyro.clone()) else { return vec![look::pnote("This controller has no gyro.")] };
        let s = |x: GyroSetting| sv(&g.settings, x);
        let off = g.mode == bu_controller::GyroMode::Off;
        let modes = vec![(Some(0), "Off".to_string()), (Some(1), "As mouse".into()), (Some(2), "As joystick".into()), (Some(3), "As joystick \u{b7} camera".into())];
        let held_off = |r: Option<i64>| r == Some(0);
        let rows = vec![
            self.r_menu(cx, PANEL, "gy.mode", "Mode", modes, gyro_mode_v(&g.mode), None, W::GyroMode, 170.0, false),
            self.r_menu(cx, PANEL, "gy.hold", "On while held", Self::choice(GYRO_BUTTONS), s(GyroSetting::OnWhileHeld), Some(0), W::Gyro(GyroSetting::OnWhileHeld), 132.0, off),
            self.r_toggle(cx, PANEL, "gy.inv", "Held = off", s(GyroSetting::HeldBehaviour), held_off, W::Gyro(GyroSetting::HeldBehaviour), Some(0), None, off),
            self.r_slider(cx, PANEL, "gy.sens", "Sensitivity", (10.0, 400.0, 5.0), 100.0, Conv::Raw, Fmt::Pct, W::Gyro(GyroSetting::Sensitivity), s(GyroSetting::Sensitivity), off),
        ];
        let motion = look::psec_first("Motion", rows);
        let axis_of = |r: Option<i64>| GYRO_AXES.iter().position(|(v, _)| *v == r.unwrap_or(0));
        let fine = vec![
            self.r_slider(cx, PANEL, "gy.dz", "Speed dead zone", (0.0, 50.0, 1.0), 10.0, Conv::Raw, Fmt::Pct, W::Gyro(GyroSetting::SpeedDeadZone), s(GyroSetting::SpeedDeadZone), off),
            self.r_slider(cx, PANEL, "gy.prec", "Precision speed", (0.0, 100.0, 1.0), 0.0, Conv::Raw, Fmt::Pct, W::Gyro(GyroSetting::PrecisionSpeed), s(GyroSetting::PrecisionSpeed), off),
            self.r_slider(cx, PANEL, "gy.ratio", "Up-down speed", (25.0, 200.0, 5.0), 100.0, Conv::Raw, Fmt::Pct, W::Gyro(GyroSetting::UpDownSpeed), s(GyroSetting::UpDownSpeed), off),
            self.r_seg(cx, PANEL, "gy.axis", "Turn with", &["Left-right", "Tilt", "Both"], &[Some(0), Some(1), Some(2)], s(GyroSetting::TurnWith), &axis_of, W::Gyro(GyroSetting::TurnWith), off),
            // (the drawing's "Smoothing" is not in Steam's files - Lane L gap 4 - so it is left out: Order 045 item 13)
            self.r_toggle(cx, PANEL, "gy.ix", "Invert left-right", s(GyroSetting::InvertLeftRight), on_nz, W::Gyro(GyroSetting::InvertLeftRight), Some(1), None, off),
            self.r_toggle(cx, PANEL, "gy.iy", "Invert up-down", s(GyroSetting::InvertUpDown), on_nz, W::Gyro(GyroSetting::InvertUpDown), Some(1), None, off),
        ];
        let fine = Self::r_sec("Fine-tuning", fine);
        // (the drawing's "Calibrate" row: Steam calibrates in its own screen, no file - left out, Order 045 item 13)
        vec![motion, fine]
    }

    fn tpad_body(&mut self, cx: &mut Cx) -> Vec<El> {
        let Some(t) = self.view.as_ref().and_then(|v| v.touchpad.clone()) else { return vec![look::pnote("This controller has no touchpad.")] };
        let st = self.steam.as_ref().and_then(|v| v.touchpad.clone());
        let s = |x: TouchSetting| sv(&t.settings, x);
        // Nothing / As mouse / Scroll = the segments 0 / 1 / 2; a touch menu, d-pad, radial menu …: no segment is on
        let touch_of = |r: Option<i64>| r.map(|r| r as usize);
        let touch = vec![
            self.r_seg(cx, PANEL, "tp.touch", "Touch", &["Nothing", "As mouse", "Scroll"], &[Some(0), Some(1), Some(2)], touch_mode_v(&t.touch), &touch_of, W::TouchMode, false),
            self.r_slider(cx, PANEL, "tp.spd", "Mouse speed", (10.0, 300.0, 5.0), 100.0, Conv::Raw, Fmt::Pct, W::Touch(TouchSetting::MouseSpeed), s(TouchSetting::MouseSpeed), t.touch != TouchMode::Mouse),
        ];
        let touch = look::psec_first("Touch", touch);
        let click = vec![
            self.r_act(cx, "tp.lc", "Left half click", &t.left_click, AW::TouchClick(Side::Left), st.as_ref().map(|x| &x.left_click)),
            self.r_act(cx, "tp.rc", "Right half click", &t.right_click, AW::TouchClick(Side::Right), st.as_ref().map(|x| &x.right_click)),
        ];
        let click = look::psec("Click", 10.0, click);
        // (the drawing's "One button" is not in Steam's files - left out, Order 045 item 13)
        let more = vec![
            self.r_toggle(cx, PANEL, "tp.req", "Click needs a press", s(TouchSetting::ClickNeedsPress), on_unless0, W::Touch(TouchSetting::ClickNeedsPress), None, Some(0), false),
            self.r_menu(cx, PANEL, "tp.hap", "Haptics", Self::choice(HAPTICS), s(TouchSetting::Haptics), Some(0), W::Touch(TouchSetting::Haptics), 112.0, false),
        ];
        let more = Self::r_sec("More", more);
        vec![touch, click, more]
    }
}

/// The Controller settings popup's rows (`devIn`) + the Light bar (colour + brightness, A_015_02) for pads that have one.
pub(super) fn prefs_rows(o: &mut Open, cx: &mut Cx) -> Vec<El> {
    let p = o.pref().cloned();
    let dead = p.is_none();
    let get = |s: PrefSetting| p.as_ref().and_then(|p| p.get(s)).map(str::to_string);
    let num = |s: PrefSetting| get(s).and_then(|v| v.trim().parse::<i64>().ok());
    let dz = |s: PrefSetting| p.as_ref().and_then(|p| p.stick_deadzone_pct(s)).map(bu_controller::settings::pct_to_radius);
    let mut rows = vec![
        o.r_slider(cx, DLG, "pf.lsdz", "Left stick dead zone", (0.0, 40.0, 1.0), 8.0, Conv::Radius, Fmt::Pct, W::Pref(PrefSetting::LeftStickDeadZone), dz(PrefSetting::LeftStickDeadZone), dead),
        o.r_slider(cx, DLG, "pf.rsdz", "Right stick dead zone", (0.0, 40.0, 1.0), 8.0, Conv::Radius, Fmt::Pct, W::Pref(PrefSetting::RightStickDeadZone), dz(PrefSetting::RightStickDeadZone), dead),
        o.r_toggle(cx, DLG, "pf.anti", "Anti-drift", num(PrefSetting::AntiDrift), |r| r == Some(1), W::Pref(PrefSetting::AntiDrift), Some(1), Some(0), dead),
    ];
    if o.kind.has_gyro() {
        // nothing in the file = Steam's default 0.5 = Medium; a value the three steps don't have = its own item (never rewritten)
        let raw = get(PrefSetting::GyroNoiseFilter);
        let noise = match &raw {
            None => Some(1),
            Some(v) => NOISE_STEPS.iter().position(|(_, n)| v.trim().parse::<f64>().ok() == n.parse::<f64>().ok()).map(|i| i as i64),
        };
        let mut items: Vec<(Option<i64>, String)> = NOISE_STEPS.iter().enumerate().map(|(i, (l, _))| (Some(i as i64), l.to_string())).collect();
        if noise.is_none() {
            items.push((None, raw.unwrap_or_default().trim().to_string()));
        }
        rows.push(o.r_menu(cx, DLG, "pf.noise", "Gyro noise filter", items, noise, None, W::Noise, 112.0, dead));
    }
    rows.push(o.r_toggle(cx, DLG, "pf.rumble", "Rumble", num(PrefSetting::Rumble), |r| r != Some(0), W::Pref(PrefSetting::Rumble), Some(1), Some(0), dead));
    // (the drawing's "Turn off when idle" is a Steam-wide setting, not in this file - Lane L gap 4 - left out, Order 045
    // item 13)
    if o.kind.has_light_bar() {
        let led = p.as_ref().and_then(|p| p.led());
        let on = led.and_then(|c| look::LCOL.iter().position(|(_, x)| x.unwrap_or((0, 0, 0)) == c));
        let k = Open::k("pf.lc");
        let keys: Vec<Key> = (0..look::LCOL.len()).map(|i| idx(k, i)).collect();
        for (i, sk) in keys.iter().enumerate() {
            o.reg(*sk, if dead { Ctl::Dead } else { Ctl::Val { v: Some(i as i64), w: W::Light, on: on == Some(i) } });
        }
        o.row(DLG, k, &keys, "Light bar");
        // no reset: Steam's own colour for a controller is not known (its file only has what was picked)
        let tail = o.tail(cx, k, None);
        rows.push(look::prw("Light bar", DLG.lw, vec![look::lsw(cx, k, on)], dead, Some(tail)));
        let bri = get(PrefSetting::LedBrightness).and_then(|v| v.trim().parse::<f64>().ok()).map(|v| (v * 100.0).round() as i64);
        rows.push(o.r_slider(cx, DLG, "pf.bri", "Brightness", (0.0, 100.0, 5.0), 100.0, Conv::Unit, Fmt::Pct, W::Pref(PrefSetting::LedBrightness), bri, dead));
    }
    rows
}

/// The two wells of a stick (`stickWells`): the dead zone circle (live dot = the stick) and the response curve.
pub(super) struct Wells {
    pub inner: f64,
    pub outer: f64,
    pub curve: i64,
    pub shape: f64,
    pub sens: f64,
    pub anti: f64,
    pub shp: i64,
    pub lp: (f32, f32),
}

/// `curveF(s)`: the output % for a push % (Steam's curve ids: Linear 0, Aggressive 1, Relaxed 2, Wide 3, Extra wide 4, Custom 5
/// with the drawing's exponents `CEXP{linear:1,relaxed:1.5,aggr:.6,wide:2,xwide:2.6}`).
fn curve_f(w: &Wells) -> impl Fn(f64) -> f64 + '_ {
    let e = match w.curve {
        1 => 0.6,
        2 => 1.5,
        3 => 2.0,
        4 => 2.6,
        5 => w.shape,
        _ => 1.0,
    };
    move |x: f64| {
        if x <= w.inner {
            return 0.0;
        }
        let t = ((x - w.inner) / (w.outer - w.inner).max(1.0)).clamp(0.0, 1.0);
        (w.anti + (100.0 - w.anti) * t.powf(e) * w.sens / 100.0).min(100.0)
    }
}

/// `.pwls{display:grid;grid-template-columns:1fr 1fr;gap:10px}` `.cpn .pwls{max-width:340px;margin:2px 0 8px}`
/// `.pwl{aspect-ratio:136/118;border-radius:9px;background:var(--well);box-shadow:inset 0 0 0 .5px var(--hair);overflow:hidden}`
/// `.pwcap{display:flex;flex-wrap:wrap;justify-content:space-between;column-gap:6px;margin-top:4px;font-size:10.5px;color:var(--fg3)}`
pub(super) fn wells(cx: &mut Cx, key: Key, w: Wells, live: bool) -> El {
    use crate::gfx::{sh, Font, Rgba};
    use crate::ui::el::lh;
    use crate::ui::{ACC, DASH, FG2, FG3, HAIR, WELL};
    let col_w = (PANEL_IN - 10.0) / 2.0;
    let h = col_w * 118.0 / 136.0;
    let s = col_w / 136.0;
    let mag = (w.lp.0 as f64).hypot(w.lp.1 as f64);
    let inside = mag * 100.0 <= w.inner;
    let f = curve_f(&w);
    let out_pct = f((mag * 100.0).min(100.0)).round();
    let pts: Vec<(f32, f32)> = (0..=100).map(|i| (i as f32, f(i as f64) as f32)).collect();
    let (inner, outer, shp, lp) = (w.inner as f32, w.outer as f32, w.shp, w.lp);
    let lx = (mag * 100.0).min(100.0) as f32;
    let ly = f(lx as f64) as f32;
    let well = |e: El| e.size(col_w, h).radius(9.0).bg(WELL()).inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())]).clip();
    // the dead zone circle (W 136, H 118, centre 68 / 59, R 48)
    let mut dz = El::paint(move |g, (x, y, _, _)| {
        let t0 = g.transform();
        g.set_transform(&(windows_numerics::Matrix3x2 { M11: s, M12: 0.0, M21: 0.0, M22: s, M31: x, M32: y } * t0));
        let (cx0, cy0, rr) = (68.0f32, 59.0f32, 48.0f32);
        g.line(cx0 - rr, cy0, cx0 + rr, cy0, 1.0, HAIR(), false);
        g.line(cx0, cy0 - rr, cx0, cy0 + rr, 1.0, HAIR(), false);
        g.stroke_geom_ex(&skia_safe::Path::circle((cx0, cy0), rr, None), 1.0, DASH(), false, true, 1.0);
        // the outer ring (dashed --acc)
        let ro = rr * outer / 100.0;
        {
            use skia_safe as sk;
            let mut p = sk::Paint::new(ACC().c4(), None);
            p.set_anti_alias(true).set_style(sk::PaintStyle::Stroke).set_stroke_width(1.2);
            p.set_path_effect(sk::PathEffect::dash(&[3.0, 3.0], 0.0));
            g.cv().draw_circle((cx0, cy0), ro, &p);
        }
        // the inner shape (.dzi: fill --ctl-h, stroke --fg3)
        let ri = (rr * inner / 100.0).max(0.01);
        let path = match shp {
            2 => g.rr_path(cx0 - ri, cy0 - ri, 2.0 * ri, 2.0 * ri, 2.0),
            0 => {
                let d = format!("M{} {}H{}V{}H{}ZM{} {}H{}V{}H{}Z", cx0 - rr, cy0 - ri, cx0 + rr, cy0 + ri, cx0 - rr, cx0 - ri, cy0 - rr, cx0 + ri, cy0 + rr, cx0 - ri);
                g.path(&d)
            }
            _ => skia_safe::Path::circle((cx0, cy0), ri, None),
        };
        g.fill_geom(&path, crate::ui::CTL_H());
        g.stroke_geom_ex(&path, 1.0, FG3(), false, true, 1.0);
        // the live stick: line + dot (green outside the dead zone, a ring inside)
        let (dx, dy) = (cx0 + lp.0 * rr, cy0 + lp.1 * rr);
        g.line(cx0, cy0, dx, dy, 1.0, ACC().mul_a(0.45), false);
        if inside {
            g.stroke_geom_ex(&skia_safe::Path::circle((dx, dy), 3.4, None), 1.2, FG2(), false, true, 1.0);
        } else {
            g.fill_circle(dx, dy, 3.4, Rgba::hex(0x30d158));
        }
        // no handles on the rings (the owner, test build 2: "these big ass bubbles ... isn't needed at all"): the rings
        // themselves are dragged (`Ctl::Well`, the grab cursor)
        g.set_transform(&t0);
    })
    .abs(0.0, 0.0, f32::NAN, f32::NAN)
    .size(col_w, h)
    .no_hit();
    if live {
        dz = dz.live();
    }
    // .pwlive{left:6px;top:5px;gap:4px;font-size:9.5px;font-weight:700;color:#4cd964;letter-spacing:.02em} i{6px;#30d158}
    let live_tag = El::row()
        .abs(6.0, 5.0, f32::NAN, f32::NAN)
        .center()
        .gap(4.0)
        .no_hit()
        .child(El::block().size(6.0, 6.0).radius(3.0).bg(Rgba::hex(0x30d158)))
        .child(El::text("Live", Font::new(9.5, 700).ls(190), Rgba::hex(0x4cd964), lh(9.5, 1.35)));
    // `.pwl.dz{cursor:grab}`: a press / drag on it moves the nearer ring (the page's `Ctl::Well`)
    // `.pwl.dz{cursor:grab}` `.drag{cursor:grabbing}`
    // Order 045: `h('div',{class:'pwl dz','data-tip':'The dot is your stick, live · drag the inner ring = dead zone · …'})`
    let dz_well = well(El::block())
        .key(key)
        .tip("The dot is your stick, live \u{b7} drag the inner ring = dead zone \u{b7} the dashed ring = where it reaches full")
        .cursor(crate::ui::el::Cursor::Grab)
        .child(dz)
        .child(live_tag);
    let cap = |l: El, r: El| El::row().wrap().justify(taffy::style::JustifyContent::SPACE_BETWEEN).gap2(0.0, 6.0).margin(4.0, 0.0, 0.0, 0.0).child(l).child(r);
    let f105 = Font::new(10.5, 400);
    let dz_cap = cap(
        El::row().child(El::text("Your stick: ", f105, FG3(), lh(10.5, 1.35))).child(El::text(format!("{} %", (mag.min(1.0) * 100.0).round()), Font::new(10.5, 600).tnum(), FG2(), lh(10.5, 1.35))),
        El::text(if inside { "in the dead zone".to_string() } else { format!("out \u{2192} {out_pct} %") }, f105, FG3(), lh(10.5, 1.35)),
    );
    // the response curve (GL 20, GR 6, GT 7, GB 15)
    // the curve's hover readout (`cvW` mousemove: hx = the pointer's push %; `.pwl.hov .hv{opacity:1}` `.pwrd` "x → y %"); the
    // well's left edge in page coordinates = the panel's content box + the first column + the grid gap
    let cvk = sub(key, "cv");
    let hv_t = cx.hover_t(cvk, 120.0, crate::anim::EASE);
    let hx = cx.hover_point(cvk).map(|(px, _)| {
        let left = PANEL_LEFT + col_w + 10.0;
        (((px - left) / col_w * 136.0 - 20.0) / (136.0 - 20.0 - 6.0) * 100.0).clamp(0.0, 100.0)
    });
    let hy = hx.map(|v| f(v as f64) as f32);
    let mut cv = El::paint(move |g, (x, y, _, _)| {
        let t0 = g.transform();
        g.set_transform(&(windows_numerics::Matrix3x2 { M11: s, M12: 0.0, M21: 0.0, M22: s, M31: x, M32: y } * t0));
        let (gl, gr, gt, gb, ww, hh) = (20.0f32, 6.0f32, 7.0f32, 15.0f32, 136.0f32, 118.0f32);
        let px = |v: f32| gl + v / 100.0 * (ww - gl - gr);
        let py = |v: f32| gt + (1.0 - v / 100.0) * (hh - gt - gb);
        let bot = hh - gb;
        for v in [25.0, 50.0, 75.0] {
            g.line(gl, py(v), ww - gr, py(v), 1.0, HAIR(), false);
        }
        g.line(gl, bot, ww - gr, bot, 1.0, DASH(), false);
        {
            use skia_safe as sk;
            let mut p = sk::Paint::new(FG3().c4(), None);
            p.set_anti_alias(true).set_style(sk::PaintStyle::Stroke).set_stroke_width(1.0);
            p.set_path_effect(sk::PathEffect::dash(&[2.0, 3.0], 0.0));
            g.cv().draw_line((px(0.0), py(0.0)), (px(100.0), py(100.0)), &p);
        }
        let mut d = String::new();
        for (i, (a, b)) in pts.iter().enumerate() {
            d += &format!("{}{:.1} {:.1}", if i == 0 { "M" } else { "L" }, px(*a), py(*b));
        }
        let area = format!("{d}L{} {}L{} {}Z", px(100.0), bot, px(0.0), bot);
        let sh_ = skia_safe::gradient_shader::linear(
            ((0.0, gt), (0.0, bot)),
            skia_safe::gradient_shader::GradientShaderColors::Colors(&[ACC().a(0.26).c4().to_color(), ACC().a(0.0).c4().to_color()]),
            None,
            skia_safe::TileMode::Clamp,
            None,
            None,
        );
        if let Some(shd) = sh_ {
            let mut p = skia_safe::Paint::default();
            p.set_anti_alias(true).set_shader(shd);
            g.cv().draw_path(&g.path(&area), &p);
        }
        g.stroke_geom(&g.path(&d), 2.0, ACC());
        // the axis labels (9 px --fg3)
        let f9 = Font::new(9.0, 400);
        let tl = |s: &str, x: f32, base: f32, al: crate::gfx::Align| g.text(s, f9, x, base - 9.0, 11.0, FG3(), al, 0.0);
        tl("100", gl - 4.0, py(100.0) + 3.0, crate::gfx::Align::Right);
        tl("50", gl - 4.0, py(50.0) + 3.0, crate::gfx::Align::Right);
        tl("0", gl, hh - 4.0, crate::gfx::Align::Left);
        tl("100", ww - gr, hh - 4.0, crate::gfx::Align::Right);
        // the live dot on the curve
        g.fill_circle(px(lx), py(ly), 3.0, Rgba::hex(0x30d158));
        g.stroke_geom_ex(&skia_safe::Path::circle((px(lx), py(ly)), 3.0, None), 1.0, crate::ui::WHITE, false, true, 1.0);
        // `.pwl .hl{stroke:var(--fg2);stroke-dasharray:2 2}` + `.hdot{fill:var(--acc);stroke:#fff;stroke-width:1.5}` r 3.2
        if let (Some(hx), Some(hy)) = (hx, hy) {
            if hv_t > 0.0 {
                use skia_safe as sk;
                let mut p = sk::Paint::new(FG2().mul_a(hv_t).c4(), None);
                p.set_anti_alias(true).set_style(sk::PaintStyle::Stroke).set_stroke_width(1.0);
                p.set_path_effect(sk::PathEffect::dash(&[2.0, 2.0], 0.0));
                g.cv().draw_line((px(hx), gt), (px(hx), bot), &p);
                g.fill_circle(px(hx), py(hy), 3.2, ACC().mul_a(hv_t));
                g.stroke_geom_ex(&skia_safe::Path::circle((px(hx), py(hy)), 3.2, None), 1.5, crate::ui::WHITE.mul_a(hv_t), false, true, 1.0);
            }
        }
        g.set_transform(&t0);
    })
    .abs(0.0, 0.0, f32::NAN, f32::NAN)
    .size(col_w, h)
    .no_hit();
    if live {
        cv = cv.live();
    }
    let mut cv_well = well(El::block()).key(cvk).cursor(crate::ui::el::Cursor::Default).child(cv);
    // `.pwrd{position:absolute;right:6px;top:5px;padding:2px 6px;border-radius:5px;background:var(--pop);box-shadow:inset 0 0 0 .5px
    // var(--hl),0 2px 6px rgba(0,0,0,.18);font-size:10px;font-weight:600;tabular-nums;color:var(--fg)}` (opacity .12 s)
    if let (Some(hx), Some(hy)) = (hx, hy) {
        cv_well = cv_well.child(
            El::block()
                .abs(f32::NAN, 5.0, 6.0, f32::NAN)
                .pad(2.0, 6.0, 2.0, 6.0)
                .radius(5.0)
                .bg(crate::ui::POP())
                .shadow(&[sh(0.0, 2.0, 6.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.18))])
                .inset(&[sh(0.0, 0.0, 0.0, 0.5, crate::ui::HL_V19())])
                .opacity(hv_t)
                .no_hit()
                .child(El::text(format!("{} \u{2192} {} %", hx.round(), hy.round()), Font::new(10.0, 600).tnum(), crate::ui::FG(), lh(10.0, 1.35))),
        );
    }
    let cv_cap = cap(El::text("Response curve", f105, FG3(), lh(10.5, 1.35)), El::text("push \u{2192}", f105, FG3(), lh(10.5, 1.35)));
    let _ = cx;
    El::grid()
        .cols(2)
        .gap(10.0)
        // margin-top 2 collapses into the heading's 3 px bottom margin in the drawing's block flow
        .margin(0.0, 0.0, 8.0, 0.0)
        .child(El::col().min_w(0.0).child(dz_well).child(dz_cap))
        .child(El::col().min_w(0.0).child(cv_well).child(cv_cap))
}

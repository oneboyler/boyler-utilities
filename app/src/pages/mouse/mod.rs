//! The Mouse tab (menu-v22 page `cur`, Order 019), top to bottom as drawn: YOUR MOUSE (the mouse's own DPI / polling /
//! lift-off / battery for mice the app knows, else its name + web-settings link), MOUSE ACCELERATION (one card: Raw
//! Accel's curve, presets, graph, per app - or the install card), MOUSE SETTINGS (Windows' own pointer settings), CURSORS
//! (a bubble per role + its picker, Import cursors…, Size), the reset line. Wired to bu-mouse through `svc` (a worker
//! thread: the real OS layer in normal runs, the drawing's sample PC as a FAKE in test copies). Every box is the drawing's
//! CSS; rules are quoted where a part is not a shared piece.

mod art;
mod pic;
mod store;
#[cfg(windows)]
pub mod rt;
pub mod svc;

use std::path::PathBuf;
use std::rc::Rc;

use taffy::style::{AlignItems, JustifyContent};

use bu_mouse::accel::curves::{init_data, sensitivity};
use bu_mouse::accel::panel::{rows, value_text, visible_rows, CapType, Curve, Field, Panel, RowSpec, SENS};
use bu_mouse::accel::service::{header_line, RawAccelStatus, INSTALL_TEXT, INSTALL_TITLE};
use bu_mouse::accel::switch::{PerApp, PresetId, RowId, Target};
use bu_mouse::cursors::{Role, SetId};
use bu_mouse::device::{DPI_CHIPS, LIFT_OFF_CHIPS, POLLING_CHIPS};
use bu_mouse::settings::{double_click_ms_for_step, double_click_step_for_ms, ScrollLines};

use crate::anim::EASE;
use crate::gfx::{sh, Align, Font, Rgba};
use crate::pages::{Env, Page};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, lh, sub, Cursor, El, Key, RADIUS_PILL};
use crate::ui::pieces::mitems::{self, It, Lead, Place, Row};
use crate::ui::pieces::{self, bits, button, card, dropdown, fold, group, link, nbox, reset, rowbits, seg, segx, slider, toast, toggle};
use crate::ui::{cmix, ACC, ACC_S, AMBER, CTL, CTL_H, DASH, FG, FG2, FG3, HAIR, HL_V19, HOV, ICO_ON, POP, SEL, WELL, WHITE};

use svc::{Cmd, Svc, View};

/// One line of a popup list, owned (a `mitems::Row` only borrows its words). Item i (counting separators) = `idx(K_MENU, i)`.
enum MItem {
    /// `game` = the app tile before the label (the list of apps)
    Item { label: String, game: Option<&'static Game>, checked: bool },
    Sep,
}

// ---- keys ("cur.<name>")
const K_WEB: Key = key("cur.web");
const K_DPI: Key = key("cur.dpi");
const K_DPIN: Key = key("cur.dpin");
const K_HZ: Key = key("cur.hz");
const K_LOD: Key = key("cur.lod");
const K_AC: Key = key("cur.ac");
const K_ACH: Key = key("cur.ach");
const K_ACT: Key = key("cur.act");
const K_ACX: Key = key("cur.acx");
const K_CHIP: Key = key("cur.chip");
const K_SAVE: Key = key("cur.save");
const K_UPD: Key = key("cur.upd");
const K_MODE: Key = key("cur.mode");
const K_GAIN: Key = key("cur.gain");
const K_ASL: Key = key("cur.asl");
const K_CAP: Key = key("cur.cap");
const K_GRAPH: Key = key("cur.graph");
const K_ACDPI: Key = key("cur.acdpi");
const K_EPPOFF: Key = key("cur.eppoff");
const K_ROW: Key = key("cur.row");
const K_ADD: Key = key("cur.add");
const K_ELSE: Key = key("cur.else");
const K_COPY: Key = key("cur.copy");
const K_OPEN: Key = key("cur.openra");
const K_GIT: Key = key("cur.git");
/// addons-v1: the card's Get (the feature add-on "Mouse acceleration", the same job as the Add-ons page's)
const K_RAGET: Key = key("cur.raget");
const K_SPEED: Key = key("cur.speed");
const K_EPP: Key = key("cur.epp");
const K_LINES: Key = key("cur.lines");
const K_DCT: Key = key("cur.dct");
const K_DBL: Key = key("cur.dbl");
const K_SWAP: Key = key("cur.swap");
const K_IMP: Key = key("cur.imp");
const K_SGET: Key = key("cur.sget");
const K_SIZE: Key = key("cur.size");
const K_ROLE: Key = key("cur.role");
const K_RS: Key = key("cur.rs");
const K_RSP: Key = key("cur.rsp");
const K_RSC: Key = key("cur.rsc");
const K_RSG: Key = key("cur.rsg");
const K_MENU: Key = key("cur.menu");
const K_TOAST: Key = key("cur.toast");

/// The drawing's AMODES (the curve popup).
const CURVES: [Curve; 6] = Curve::ALL;
/// The roles' names in the bubbles (`CROLES`) and the picker title (`CLONG`).
const ROLE_TITLE: [&str; 7] = ["Normal", "Link", "Text", "Busy", "Working in background", "Move", "Resize"];

/// The app picker's games (`RAPPS`): id, name, glyph, tile gradient, the exe the per-app switch watches.
struct Game {
    name: &'static str,
    icon: &'static str,
    c1: u32,
    c2: u32,
    exe: &'static str,
}
const GAMES: [Game; 6] = [
    Game { name: "VALORANT", icon: "pad", c1: 0xff7a76, c2: 0xd83f4c, exe: svc::VAL_EXE },
    Game { name: "Counter-Strike 2", icon: "aim", c1: 0xffc56b, c2: 0xe0861c, exe: "cs2.exe" },
    Game { name: "Fortnite", icon: "pad", c1: 0xb58cff, c2: 0x6f4ae0, exe: "FortniteClient-Win64-Shipping.exe" },
    Game { name: "Apex Legends", icon: "tri", c1: 0xff8f6b, c2: 0xc4422f, exe: "r5apex.exe" },
    Game { name: "Rocket League", icon: "globe", c1: 0x5ab4ff, c2: 0x2a74e6, exe: "RocketLeague.exe" },
    Game { name: "Minecraft", icon: "cube", c1: 0x7ed67a, c2: 0x3f9a3b, exe: "Minecraft.Windows.exe" },
];

fn rgb(h: u32) -> Rgba {
    Rgba::rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
}

/// A game's app tile (`bits::at`: its 135° gradient and glyph); inside a click target, so it takes no clicks itself.
fn game_tile(g: &Game) -> El {
    bits::at(g.icon, rgb(g.c1), rgb(g.c2), false).no_hit()
}

fn game_of(exe: &str) -> Option<&'static Game> {
    let f = exe.rsplit(['\\', '/']).next().unwrap_or(exe);
    GAMES.iter().find(|g| g.exe.eq_ignore_ascii_case(f))
}

/// One row of a role's picker.
struct PickRow {
    id: SetId,
    name: String,
    note: String,
    /// an imported set (× deletes it)
    imp: bool,
    /// Glass / Windows default / the matching set (above the separator)
    top: bool,
    matches: bool,
    /// the set's cursor file for this bubble: the row draws the real picture from it
    file: Option<String>,
}

/// Which popup list is open.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Menu {
    Curve,
    RowApp(RowId),
    RowPreset(RowId),
    Else,
    Cursor(usize),
}

/// One line of the drawing's sample review (test pictures only): title, now, back.
#[derive(Clone, Debug)]
struct SampleLine {
    title: &'static str,
    now: &'static str,
    to: &'static str,
}

#[derive(Default)]
pub struct Mouse {
    svc: Option<Svc>,
    v: View,
    fake: bool,
    /// a test copy (fake, or real but read-only): never opens a browser / Raw Accel
    test: bool,
    frozen: bool,
    reading: bool,
    /// the card as the page edits it (handed to the worker after each change)
    panel: Panel,
    per_app: PerApp,
    panel_from_worker: bool,
    dpi_text: String,
    acdpi: u32,
    acdpi_text: String,
    dct_open: bool,
    dct_last: f64,
    menu: Option<Menu>,
    /// the list the frame closed on a press beside it (`popup_dismiss`), and that press's copy for its click
    dismissed: Option<Menu>,
    press_dismissed: Option<Menu>,
    anchor: (f32, f32, f32, f32),
    toast: Option<(String, f64)>,
    graph_hover: Option<f64>,
    /// a slider being dragged (its key): replies don't overwrite its value meanwhile
    drag: Option<Key>,
    /// a row being dragged on the accel card, by field (sent on release)
    accel_dirty: bool,
    /// a chip being renamed: (preset, text)
    rename: Option<(PresetId, String)>,
    /// hovered picker row being previewed (role, set)
    preview: Option<(usize, SetId)>,
    /// TEST COPIES ONLY (`review=` test state): the page's own review with the drawing's sample lines (`RS.cur`) for the
    /// pixel proof - (Windows defaults? else back to how it was, the ticks). Every other copy opens the frame's review over
    /// the change log (`cx.open_reset`).
    reset: Option<(bool, Vec<bool>)>,
    test_rs: Option<Vec<SampleLine>>,
    /// a closed tab's service for the change log (Settings › Reset, the uninstaller's undo): made at its first use
    /// (Order 047: shared with a detached reset copy, which uses it on the review's worker thread)
    cold: Cold,
    /// TEST COPIES ONLY (`rename` test state): rename the first chip once the presets are there, and focus its field at the
    /// next build, as the pen click does
    test_rename: bool,
    test_focus: bool,
    /// Order 066: the "Get more cursors" window
    store_open: bool,
    store_msg: Option<String>,
    store_at: f64,
    store_done: Option<crate::jobs::JobId>,
    /// the pack being downloaded (its name), for the install after the download job
    store_getting: Option<String>,
}

impl Mouse {
    fn send(&mut self, c: Cmd) {
        if let Some(s) = &mut self.svc {
            s.send(c);
        }
    }

    fn say(&mut self, t: impl Into<String>, now: f64) {
        self.toast = Some((t.into(), now));
    }

    fn push_accel(&mut self) {
        self.send(Cmd::Accel(Box::new((self.panel.clone(), self.per_app.clone()))));
    }

    fn take(&mut self, r: svc::Reply, now: f64) {
        let keep_win = self.drag.is_some() && self.v.win.is_some();
        let win = self.v.win;
        self.v = r.view;
        if keep_win {
            self.v.win = win;
        }
        self.reading = r.reading_mouse;
        if !self.panel_from_worker {
            self.panel = self.v.panel.clone();
            self.per_app = self.v.per_app.clone();
            self.panel_from_worker = true;
        }
        self.apply_test_rename();
        // the change log (Order 036): what the command changed on the PC, its value before first
        for (i, l, o, n) in &r.changes {
            record_change(i, l, o, n);
        }
        if let Some(t) = r.toast {
            self.say(t, now);
        }
        if let Some(d) = self.mouse_dpi() {
            if self.acdpi == 0 {
                self.acdpi = d;
            }
        }
    }

    fn supported(&self) -> bool {
        self.v.mice.as_ref().and_then(|m| m.first()).map(|y| y.protocol.is_some()).unwrap_or(true)
    }

    fn mouse_dpi(&self) -> Option<u32> {
        self.v.on_mouse.as_ref().and_then(|o| o.dpi).map(|d| d.0)
    }

    /// The DPI chips: the drawing's four, plus the mouse's own DPI in its place when it is none of them (Order 042:
    /// a mouse at 2400 - read from the mouse, measured - showed only in the Custom box, "not in the list").
    fn dpi_chips(&self) -> Vec<u32> {
        let mut v = DPI_CHIPS.to_vec();
        if let Some(d) = self.mouse_dpi().filter(|d| !v.contains(d)) {
            v.push(d);
            v.sort_unstable();
        }
        v
    }

    fn ra_installed(&self) -> bool {
        !matches!(self.v.ra, Some(RawAccelStatus::NotInstalled))
    }

    fn vals(&self) -> bu_mouse::accel::panel::CurveValues {
        self.panel.current_values()
    }

    /// The graph's samples (speed 0..120, sensitivity): Raw Accel's own modifier on the user's profile with the card's
    /// curve (`aDraw` samples n = 240).
    fn graph_points(&self) -> Vec<(f64, f64)> {
        let p = self.panel.current_setting().apply_to(&self.v.base);
        let d = init_data(&p);
        (0..=240).map(|i| {
            let x = i as f64 / 240.0 * 120.0;
            // speed 0 has no sensitivity (output / input): the first point uses a speed just above 0, as the drawing does
            (x, sensitivity(&p, &d, x.max(1e-3)))
        }).collect()
    }

    // ================================================================== YOUR MOUSE
    fn your_mouse(&mut self, cx: &mut Cx) -> El {
        let sup = self.supported();
        let pct = self.v.on_mouse.as_ref().and_then(|o| o.battery_percent);
        // .gh .ghr{margin-left:auto;display:flex;align-items:center;gap:10px;font-weight:400;color:var(--fg2)}
        // .bat{display:inline-flex;align-items:center;gap:6px;font-variant-numeric:tabular-nums} .bat svg{width:21px;height:12px;color:var(--fg2)}
        let mut gh = group::gh("Your mouse");
        if sup {
            if let Some(p) = pct {
                let icon = El::block()
                    .size(21.0, 12.0)
                    .none()
                    .child(El::icon("bat", 21.0, 1.2, FG2()).h(12.0).class_op("lv", 0.0))
                    .child(El::paint(move |g, (x, y, _, _)| art::battery_level(g, x, y, p as f32)).abs(0.0, 0.0, 0.0, 0.0));
                let bat = El::row()
                    .center()
                    .gap(6.0)
                    .none()
                    .child(icon)
                    .child(El::text(format!("{p} %"), Font::new(11.0, 400).tnum(), FG2(), lh(11.0, 1.35)));
                gh = gh.child(El::row().ml_auto().center().gap(10.0).none().child(bat));
            }
        }
        let y = self.v.mice.as_ref().and_then(|m| m.first().cloned());
        let name = y.as_ref().map(|y| y.name.clone()).unwrap_or_default();
        let mut rows = Vec::new();
        if sup {
            // .ymrow{min-height:58px}: the mouse's icon tile, name, line + its web settings link
            let sub_line = y.as_ref().map(|y| format!("{} · {}", y.sub_line(), y.ids())).unwrap_or_else(|| "Wireless · saved on the mouse itself".to_string());
            let lnk = y.as_ref().and_then(|y| y.link()).map(|(t, _)| t);
            let mut head = group::row(
                true,
                vec![El::row()
                    .center()
                    .gap(10.0)
                    .flex1()
                    .child(card::card_icon("mouse"))
                    .child(
                        El::col()
                            .child(El::text(name.clone(), Font::new(13.0, 600), FG(), lh(13.0, 1.35)).ellipsis())
                            .child(El::text(sub_line, Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).margin(1.0, 0.0, 0.0, 0.0)),
                    )],
            )
            .min_h(58.0);
            if let Some(t) = lnk {
                head = head.child(group::ctl(vec![link::link(cx, K_WEB, &t, 12.0)]));
            }
            rows.push(head);
            // DPI: the chips (the mouse's own DPI always among them, `dpi_chips`) + the Custom field to type any other one
            // (`.seg.nopick` = no pill while the DPI isn't read yet)
            let dpi = self.mouse_dpi();
            let dchips = self.dpi_chips();
            let chips: Vec<String> = dchips.iter().map(|d| d.to_string()).collect();
            let labels: Vec<&str> = chips.iter().map(|s| s.as_str()).collect();
            let lit = dpi.and_then(|d| dchips.iter().position(|c| *c == d));
            let mut s = seg::seg(cx, K_DPI, &labels, lit.unwrap_or(0), false);
            if lit.is_none() {
                nopick(&mut s, 0);
            }
            let txt = if cx.focused(K_DPIN) { self.dpi_text.clone() } else { String::new() };
            // Order 045: `h('div',{class:'nbox sm',title:'Type any DPI, 50 – 26000'})` - the mouse's real range (Order 042)
            let (dlo, dhi) = bu_mouse::device::DPI_RANGE;
            let field = nbox::nbox(cx, K_DPIN, &txt, "Custom", &nbox::SM, &nbox::Cue::NONE).wheel_steps().title(&format!("Type any DPI, {dlo} \u{2013} {dhi}"));
            rows.push(group::row(false, vec![group::lbl("DPI", None), group::ctl(vec![s, field])]));
            // Polling rate (+ the unit) and Lift-off distance
            let hz = self.v.on_mouse.as_ref().and_then(|o| o.polling_hz);
            let hl: Vec<String> = POLLING_CHIPS.iter().map(|d| d.to_string()).collect();
            let hlab: Vec<&str> = hl.iter().map(|s| s.as_str()).collect();
            let hi = hz.and_then(|h| POLLING_CHIPS.iter().position(|c| *c == h));
            let mut hs = seg::seg(cx, K_HZ, &hlab, hi.unwrap_or(0), false);
            if hi.is_none() {
                nopick(&mut hs, 0);
            }
            // .unit{font-size:12px;color:var(--fg3);margin-left:-2px}
            let unit = El::text("Hz", Font::new(12.0, 400), FG3(), lh(12.0, 1.35)).none().margin(0.0, 0.0, 0.0, -2.0);
            rows.push(group::row(false, vec![group::lbl("Polling rate", Some("Higher is smoother · uses more battery")), group::ctl(vec![hs, unit])]));
            let lo = self.v.on_mouse.as_ref().and_then(|o| o.lift_off);
            let li = lo.and_then(|l| LIFT_OFF_CHIPS.iter().position(|c| *c == l));
            let mut ls = seg::seg(cx, K_LOD, &["1 mm", "2 mm"], li.unwrap_or(0), false);
            if li.is_none() {
                nopick(&mut ls, 0);
            }
            rows.push(group::row(false, vec![group::lbl("Lift-off distance", Some("How high you lift it before it stops")), group::ctl(vec![ls])]));
        } else {
            // a mouse the app can't talk to yet: `.ci.dim{background:var(--ctl);box-shadow:inset 0 0 0 .5px var(--hair)}`
            // `.ci.dim svg{stroke:var(--fg2)}`; `.ct{gap:7px}` name · `.ymd{color:var(--fg3);font-weight:400}` "—" · the link
            // (`.ct .lnk{font-size:13px;line-height:17px;font-weight:400}`)
            let ci = El::block().size(32.0, 32.0).none().radius(8.0).bg(CTL()).inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())]).place_center().child(El::icon("mouse", 18.0, 1.5, FG2()));
            let mut ct = El::row().center().gap(7.0).child(El::text(if name.is_empty() { "No mouse found".to_string() } else { name }, Font::new(13.0, 600), FG(), lh(13.0, 1.35)).none());
            if let Some((t, _)) = y.as_ref().and_then(|y| y.link()) {
                ct = ct
                    .child(El::text("\u{2014}", Font::new(13.0, 400), FG3(), lh(13.0, 1.35)).none())
                    // `.ct .lnk{13px/17px}` loses to `#sw .lnk{12px/16px}` (an id rule) and `#sw button{font:inherit}` keeps
                    // `.ct`'s weight: Chromium's computed 600 12px/16px
                    .child(bits::lnk(cx, K_WEB, &t, 12.0, 600, 16.0));
            }
            let sub_line = y.as_ref().map(|y| format!("{} \u{b7} {}", y.sub_line(), y.ids())).unwrap_or_else(|| "DPI and polling for this mouse aren\u{2019}t supported yet".to_string());
            rows.push(
                group::row(
                    true,
                    vec![El::row().center().gap(10.0).flex1().child(ci).child(El::col().child(ct).child(El::text(sub_line.replace('\'', "\u{2019}"), Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).margin(1.0, 0.0, 0.0, 0.0)))],
                )
                .min_h(58.0),
            );
        }
        // Order 061: every other mouse Windows lists, named, with its VID:PID (so an unknown mouse can be reported)
        for o in self.v.mice.as_ref().map(|m| m.iter().skip(1)).into_iter().flatten() {
            rows.push(group::row(false, vec![group::lbl(&o.name, Some(&format!("Also connected \u{b7} {}", o.ids())))]));
        }
        El::block().child(gh).child(group::grp(rows))
    }

    // ================================================================== ACCELERATION
    fn accel(&mut self, cx: &mut Cx) -> El {
        if !self.ra_installed() {
            return self.install_card(cx);
        }
        let on = self.panel.on;
        let open = self.panel.expanded;
        let hv = cx.hover_t(K_ACH, 150.0, EASE);
        let line = header_line(&self.panel, &self.per_app);
        let right = group::ctl(vec![toggle::toggle(cx, K_ACT, on, false), fold::chev(cx, K_ACX, open, false)]);
        let head = card::card_head("accel", "Mouse acceleration", Some(&line), vec![right]).bg(HOV().mul_a(hv)).on_click(K_ACH).cursor(Cursor::Hand);
        let body = self.accel_body(cx);
        // .lockb: switched off = .36 (the drawing also greys it: filter grayscale(1) - not in the painter, see the report)
        let lt = cx.tr(K_AC, 20, if on { 1.0 } else { 0.36 }, 250.0, EASE);
        let body = body.opacity(lt);
        card::card(cx, K_AC, head, Some(body), open, 548.0).margin(12.0, 0.0, 0.0, 0.0)
    }

    fn install_card(&mut self, cx: &mut Cx) -> El {
        // .racard{display:flex;align-items:flex-start;gap:14px;padding:16px 16px 15px}
        // .rai{40x40;border-radius:10px;background:linear-gradient(135deg,#5ab4ff,#2a74e6);box-shadow:inset 0 0 0 .5px rgba(255,255,255,.24),
        //   inset 0 1px 0 rgba(255,255,255,.18)} .rai svg{22px;stroke:#fff;stroke-width:1.6}
        let rai = El::block()
            .size(40.0, 40.0)
            .none()
            .radius(10.0)
            .bg_linear(135.0, &[(0.0, rgb(0x5ab4ff)), (1.0, rgb(0x2a74e6))])
            .inset(&[sh(0.0, 0.0, 0.0, 0.5, Rgba(1.0, 1.0, 1.0, 0.24)), sh(0.0, 1.0, 0.0, 0.0, Rgba(1.0, 1.0, 1.0, 0.18))])
            .place_center()
            .child(El::icon("accel", 22.0, 1.6, WHITE));
        // .rat b{13.5px 600 18px} small{margin-top:3px;11.5px/16px --fg2} #sw .lnk.ragl{inline-flex;gap:5px;margin-top:9px;12.5px 600}
        let hv = cx.hovered(K_GIT);
        let git = El::row()
            .center()
            .gap(5.0)
            .none()
            .margin(9.0, 0.0, 0.0, 0.0)
            .on_click(K_GIT)
            // Order 045: `title:'https://'+RA.url+'/releases'`
            .title(bu_mouse::accel::service::RAWACCEL_RELEASES)
            .cursor(Cursor::Hand)
            .child(El::text("Get it from its official GitHub", Font::new(12.5, 600).ls(0), ACC(), 16.0).underline(hv))
            .child(El::icon("open", 12.0, 1.6, ACC()));
        let rat = El::col()
            .flex1()
            .items(AlignItems::FLEX_START)
            .child(El::text(INSTALL_TITLE, Font::new(13.5, 600), FG(), 18.0).wrapping())
            .child(El::text(INSTALL_TEXT, Font::new(11.5, 400), FG2(), 16.0).wrapping().margin(3.0, 0.0, 0.0, 0.0))
            // the inline-flex link sits on a line box whose strut (13 px / 17.55) reaches 0.55 px under it (Chromium's
            // computed .rat height 78.55)
            .child(
                // #sw .racard .raget{margin:9px 14px 0 0;vertical-align:top} - before the link, on its line (addons-v1)
                El::row().items(AlignItems::FLEX_START).child(self.raget(cx).margin(9.0, 14.0, 0.0, 0.0)).child(git.margin(9.0, 0.0, 0.55, 0.0)),
            );
        group::grp(vec![El::row().items(AlignItems::FLEX_START).gap(14.0).pad(16.0, 16.0, 15.0, 16.0).child(rai).child(rat)]).clip().margin(12.0, 0.0, 0.0, 0.0)
    }

    /// The card's Get (`.cbtn.sm.acc.raget`): "Get · 1.5 MB"; while the add-on job runs "Getting… 44 %" (disabled); after the
    /// install, until the restart, "Restart to finish" (disabled).
    fn raget(&mut self, cx: &mut Cx) -> El {
        let v = crate::addons::view("acc");
        let (label, off) = match v.busy {
            Some(crate::addons::Phase::Download { got, total, .. }) if total > 0 => (format!("Getting… {} %", got * 100 / total), true),
            Some(_) => ("Getting…".to_string(), true),
            None if v.got && v.restart => ("Restart to finish".to_string(), true),
            None => (format!("Get · {}", crate::addons::mb(crate::addons::size("acc"))), false),
        };
        button::cbtn(cx, K_RAGET, &label, button::Kind::Primary, true, off, 0.0)
    }

    fn accel_body(&mut self, cx: &mut Cx) -> El {
        let mut kids = vec![self.presets_row(cx), self.curve_body(cx)];
        let epp = self.v.win.map(|w| w.precision).unwrap_or(false);
        if self.panel.on && epp {
            kids.push(self.epp_warn(cx));
        }
        if let Some(line) = self.v.other_writer.clone() {
            kids.push(self.writer_warn(&line));
        }
        // .accc .acsub{display:flex;align-items:baseline;gap:8px;padding:11px 12px 3px 56px;font-size:12px;font-weight:600}
        // align-items:baseline: the layout engine's boxes have no text baselines, so the 11 px line sits where Chromium's
        // baseline alignment puts it (2 px under the 12 px one: its computed box in the dom dump)
        // Order 063 (the owner: the main switch turns the preset on for everything; a different preset per game is optional):
        // no game listed = one quiet line with the add button; the title and the "when none are open" row appear with the first game
        if !self.per_app.rows().is_empty() {
            kids.push(
                El::row()
                    .items(AlignItems::FLEX_START)
                    .gap(8.0)
                    .pad(11.0, 12.0, 3.0, 56.0)
                    .child(hair56())
                    .child(El::text("Per game", Font::new(12.0, 600), FG(), lh(12.0, 1.35)).none())
                    .child(El::text("Switches when the game starts or stops \u{b7} never mid-game", Font::new(11.0, 400), FG3(), lh(11.0, 1.35)).ellipsis().margin(2.0, 0.0, 0.0, 0.0)),
            );
        }
        kids.push(self.per_app_rows(cx));
        // .accc .acft{display:flex;align-items:center;gap:10px;padding:9px 12px 10px 56px;font-size:11px;color:var(--fg3)}
        // #sw .accc .acft .lnk{font-size:11px;line-height:15px}
        let (ver, ra_dir) = match &self.v.ra {
            Some(RawAccelStatus::Installed { version, dir }) => (version.to_string(), dir.clone()),
            _ => (String::new(), None),
        };
        // Order 045: `acCopy.title='Reads C:\\…\\RawAccel\\settings.json'` (the Raw Accel folder the app found; none known = no name)
        let mut copy = small_link(cx, K_COPY, "Copy its curve", 11.0);
        if let Some(d) = ra_dir {
            copy = copy.title(&format!("Reads {}", d.join("settings.json").display()));
        }
        kids.push(
            El::row()
                .center()
                .gap(10.0)
                .pad(9.0, 12.0, 10.0, 56.0)
                .child(hair56())
                .child(El::text(format!("Runs on your Raw Accel {ver}"), Font::new(11.0, 400), FG3(), lh(11.0, 1.35)).none())
                .child(copy)
                .child(small_link(cx, K_OPEN, "Open Raw Accel", 11.0)),
        );
        El::block().children(kids)
    }

    fn presets_row(&mut self, cx: &mut Cx) -> El {
        // .row.apre{gap:10px;min-height:46px;padding-top:8px;padding-bottom:8px} (+ .card .xin .row{padding-left:56px})
        let mut chips = El::row().center().wrap().gap(6.0).flex1();
        let changed = self.panel.changed_since_loaded();
        let presets = self.panel.presets.clone();
        for (i, p) in presets.iter().enumerate() {
            chips = chips.child(self.chip(cx, i, p.id, &p.name, self.panel.loaded == Some(p.id), changed && self.panel.loaded == Some(p.id)));
        }
        chips = chips.child(bits::addb(cx, K_SAVE, "Save as preset", true));
        let mut r = El::row()
            .center()
            .gap(10.0)
            .min_h(46.0)
            .pad(8.0, 12.0, 8.0, 56.0)
            .child(hair56())
            .child(El::text("Presets", Font::new(12.0, 400), FG2(), lh(12.0, 1.35)).none())
            .child(chips);
        if changed {
            if let Some(n) = self.panel.loaded.and_then(|id| self.panel.preset(id)).map(|p| p.name.clone()) {
                // .apre>.lnk{flex:none;font-size:11.5px} - `#sw .lnk` (an id rule) wins: 12 px
                r = r.child(link::link(cx, K_UPD, &format!("Update {n}"), 12.0));
            }
        }
        r
    }

    /// One preset chip: `.apc{position:relative;display:inline-flex;align-items:center;height:26px;padding:0 10px;
    ///   border-radius:13px;background:var(--ctl);box-shadow:inset 0 0 0 .5px var(--hair);font-size:12px}` `:hover{background:
    ///   var(--ctl-h)}` `:active{transform:scale(.97)}` `.on{background:var(--sel);box-shadow:inset 0 0 0 1.5px var(--acc);
    ///   font-weight:600}` + the amber dot `.apd` (changed) + the tools (rename / delete) sliding out on hover (`.apt` grid
    ///   0fr -> 1fr .18 s after .12 s).
    fn chip(&mut self, cx: &mut Cx, i: usize, id: PresetId, name: &str, on: bool, modified: bool) -> El {
        let k = idx(K_CHIP, i);
        let hv = cx.hover_t(k, 150.0, EASE);
        let pr = cx.active_t(k, 120.0, EASE);
        let open = cx.hovered(k) && self.rename.is_none();
        let tw = cx.tr_delayed(k, 30, if open { 37.0 } else { 0.0 }, 180.0, if open { 120.0 } else { 0.0 }, EASE);
        let w = if on { 600 } else { 400 };
        let mut c = El::row()
            .center()
            .h(26.0)
            .none()
            .pad(0.0, 10.0, 0.0, 10.0)
            .radius(13.0)
            .bg(if on { SEL() } else { cmix(CTL(), CTL_H(), hv) })
            .inset(&[if on { sh(0.0, 0.0, 0.0, 1.5, ACC()) } else { sh(0.0, 0.0, 0.0, 0.5, HAIR()) }])
            .scale(1.0 - 0.03 * pr)
            .on_click(k)
            .cursor(Cursor::Hand);
        if modified {
            // .apd{width:6px;height:6px;margin-right:6px;border-radius:50%;background:var(--amber)}
            // Order 045: `h('i',{class:'apd','data-tip':'Changed since you loaded it · click to go back'})`
            c = c.child(
                El::block()
                    .size(6.0, 6.0)
                    .none()
                    .margin(0.0, 6.0, 0.0, 0.0)
                    .radius(RADIUS_PILL)
                    .bg(AMBER())
                    .key(sub(k, "apd"))
                    .tip("Changed since you loaded it \u{b7} click to go back"),
            );
        }
        match &self.rename {
            Some((rid, t)) if *rid == id => {
                if std::mem::take(&mut self.test_focus) {
                    cx.focus(Some(sub(k, "in")));
                }
                // .apin{height:20px;margin:0 -4px;padding:0 4px;border-radius:4px;background:var(--well);font-weight:600}
                // the drawing's fit(): width max(3, letters × .62 + 1.2) em (border box). Selection / caret: TEMP until
                // PIECES_WANTED "Text input: Mouse preset name (.apin)" (Lane W)
                let fit = (t.chars().count() as f32 * 0.62 + 1.2).max(3.0) * 12.0;
                c = c.child(El::text(t.clone(), Font::new(12.0, 600), FG(), 20.0).w(fit).clip().h(20.0).margin(0.0, -4.0, 0.0, -4.0).pad(0.0, 4.0, 0.0, 4.0).radius(4.0).bg(WELL()).key(sub(k, "in")).cursor(Cursor::Text));
            }
            _ => c = c.child(El::text(name, Font::new(12.0, w), FG(), lh(12.0, 1.35)).none()),
        }
        // the tools: margin-left 5, 18 px round buttons (pen 11 px / × 7 px), margin-right -5
        let tool = |cx: &mut Cx, kk: Key, icon: &str, size: f32, red: bool| {
            let h = cx.hover_t(kk, 120.0, EASE);
            let bg = if red { Rgba::rgba(255, 69, 58, 0.16) } else { CTL_H() };
            let col = if red { cmix(FG2(), crate::ui::RED(), h) } else { cmix(FG2(), FG(), h) };
            El::block().size(18.0, 18.0).none().radius(RADIUS_PILL).bg(bg.mul_a(h)).place_center().on_click(kk).cursor(Cursor::Hand).child(El::icon(icon, size, if red { 1.5 } else { 1.4 }, col).no_hit())
        };
        // `.apt{display:grid;grid-template-columns:0fr}` (-> 1fr = 37 px open) holds `.apti{min-width:0;overflow:hidden;
        // display:flex;gap:1px;margin-right:-5px}`: the tools' box is the column + 5 px, clipped (Chromium: apt 37, apti 42)
        let apti = El::row()
            .center()
            .gap(1.0)
            .w(tw + 5.0)
            .h(18.0)
            .clip()
            // Order 045: `title:'Rename'` / `title:'Delete'` (`.apx` / `.apx.del`)
            .child(tool(cx, sub(k, "ren"), "dpen", 11.0, false).margin(0.0, 0.0, 0.0, 5.0).title("Rename"))
            .child(tool(cx, sub(k, "del"), "x", 7.0, true).title("Delete"));
        c.child(El::block().w(tw).h(18.0).none().child(apti))
    }

    fn curve_body(&mut self, cx: &mut Cx) -> El {
        let curve = self.panel.curve;
        let vals = self.vals();
        // .actop{display:flex;align-items:center;gap:12px;height:28px;margin-bottom:3px}
        let mode = dropdown::dropdown(cx, K_MODE, curve.name(), Some(122.0));
        let ghv = cx.hover_t(K_GAIN, 120.0, EASE);
        let g_on = cx.tr(K_GAIN, 2, if vals.gain { 1.0 } else { 0.0 }, 150.0, EASE);
        let g_ck = cx.tr(K_GAIN, 3, if vals.gain { 1.0 } else { 0.0 }, 180.0, crate::anim::Bezier::new(0.3, 1.3, 0.5, 1.0));
        // #sw .acgn{inline-flex;gap:6px;height:24px;padding:0 7px 0 4px;border-radius:6px;font-size:12px} :hover{background:var(--hov)}
        // .acgn i{15x15;border-radius:4px;background:var(--ctl);box-shadow:inset 0 0 0 1px var(--dash)} .on i{background:var(--acc);box-shadow:none}
        let box_ = El::block()
            .size(15.0, 15.0)
            .none()
            .radius(4.0)
            .bg(cmix(CTL(), ACC(), g_on))
            .inset(&[sh(0.0, 0.0, 0.0, 1.0, DASH().mul_a(1.0 - g_on))])
            .place_center()
            .child(El::paint(move |g, (x, y, _, _)| {
                if g_ck > 0.001 {
                    // an <svg> paints at its pixel-snapped place
                    let (x, y, _, _) = g.snap(x, y, 10.0, 10.0);
                    let s = 0.6 + 0.4 * g_ck;
                    let cv = g.cv();
                    cv.save();
                    cv.translate((x + 5.0, y + 5.0));
                    cv.scale((s, s));
                    cv.translate((-5.0, -5.0));
                    g.stroke_geom_ex(&g.path("M2.2 5.3l1.9 1.9 3.8-4.3"), 1.7, WHITE, true, true, g_ck.min(1.0));
                    cv.restore();
                }
            }).size(10.0, 10.0));
        let gain = El::row()
            .center()
            .gap(6.0)
            .h(24.0)
            .none()
            .pad(0.0, 7.0, 0.0, 4.0)
            .radius(6.0)
            .bg(HOV().mul_a(ghv))
            .on_click(K_GAIN)
            // Order 045: the `.acgn` button's data-tip
            .tip("On: the curve shapes the gain, so speed-ups feel smooth (Raw Accel\u{2019}s default). Off: it shapes the sensitivity (legacy)")
            .cursor(Cursor::Hand)
            .child(box_)
            .child(El::text("Gain", Font::new(12.0, 400).ls(0), FG(), lh(12.0, 1.35)));
        let top = El::row().center().gap(12.0).h(28.0).margin(0.0, 0.0, 3.0, 0.0).child(mode).child(gain);
        // the rows: Raw Accel's name (84 px, 12 px --fg2), the slider (flex 1), the value (.sv min 36)
        let mut acrs = El::block();
        let vis = visible_rows(curve, &vals);
        let mut cap_done = false;
        for spec in rows(curve) {
            if curve.has_cap() && !cap_done && matches!(spec.field, Field::CapInput) {
                acrs = acrs.child(self.cap_row(cx, &vals));
                cap_done = true;
            }
            if !vis.iter().any(|v| v.field == spec.field) {
                continue;
            }
            acrs = acrs.child(asr(cx, spec.label, idx(K_ASL, field_ix(spec.field)), frac(&spec, vals.get(spec.field)), &value_text(&spec, vals.get(spec.field)), false));
        }
        acrs = acrs.child(asr(cx, SENS.label, idx(K_ASL, 99), frac(&SENS, self.panel.sens), &value_text(&SENS, self.panel.sens), true));
        let acl = El::col().flex1().child(top).child(acrs);
        // the graph + its DPI line
        let pts = Rc::new(self.graph_points());
        let hover = self.graph_hover.map(|hx| {
            let i = ((hx / 120.0) * 240.0).round().clamp(0.0, 240.0) as usize;
            (hx, pts[i].1)
        });
        let pts2 = pts.clone();
        let graph = El::block()
            .size(art::GW, art::GH)
            .none()
            .radius(9.0)
            .bg(WELL())
            .inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
            .clip()
            .key(K_GRAPH)
            .child(El::paint(move |g, (x, y, _, _)| art::graph(g, x, y, &pts2, hover)).abs(0.0, 0.0, 0.0, 0.0).no_hit());
        let graph = match hover {
            // .acrd{position:absolute;right:8px;top:7px;padding:3px 7px;border-radius:6px;background:var(--pop);
            //   box-shadow:inset 0 0 0 .5px var(--hl),0 2px 6px rgba(0,0,0,.18);font:600 11px/14px;tabular-nums}
            Some((hx, hy)) => graph.child(
                El::block()
                    .abs(f32::NAN, 7.0, 8.0, f32::NAN)
                    .pad(3.0, 7.0, 3.0, 7.0)
                    .radius(6.0)
                    .bg(crate::ui::POP())
                    .inset(&[sh(0.0, 0.0, 0.0, 0.5, crate::ui::HL_V19())])
                    .shadow(&[sh(0.0, 2.0, 6.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.18))])
                    .no_hit()
                    .child(El::text(bu_mouse::accel::service::readout(hx, hy), Font::new(11.0, 600).tnum(), FG(), 14.0)),
            ),
            None => graph,
        };
        // .acdpi{display:flex;justify-content:flex-end;gap:5px;height:20px;margin-top:4px;font-size:10.5px;color:var(--fg3)}
        // Order 045: `acDpi.setAttribute('data-tip','Raw Accel measures speed in mouse dots, so the same curve feels different at
        // another DPI'+(YM.sup?'':' · type your mouse’s DPI'))`
        const ACDPI_TIP: &str = "Raw Accel measures speed in mouse dots, so the same curve feels different at another DPI";
        let mut dpi = El::row().center().justify(JustifyContent::FLEX_END).gap(5.0).h(20.0).margin(4.0, 0.0, 0.0, 0.0).key(sub(K_ACDPI, "line"));
        let f = Font::new(10.5, 400);
        if let (true, Some(d)) = (self.supported(), self.mouse_dpi()) {
            dpi = dpi.tip(ACDPI_TIP).child(El::text(format!("at {d} DPI · from Your mouse"), f, FG3(), lh(10.5, 1.35)).none());
        } else {
            dpi = dpi.tip(&format!("{ACDPI_TIP} \u{b7} type your mouse\u{2019}s DPI"));
            let t = if cx.focused(K_ACDPI) { self.acdpi_text.clone() } else { self.acdpi.max(50).to_string() };
            dpi = dpi
                .child(El::text("at", f, FG3(), lh(10.5, 1.35)).none())
                .child(nbox::nbox(cx, K_ACDPI, &t, "", &nbox::XS, &nbox::Cue::NONE))
                .child(El::text("DPI", f, FG3(), lh(10.5, 1.35)).none());
        }
        let acr = El::col().w(214.0).none().child(graph).child(dpi);
        // .accc .acb{display:flex;align-items:flex-start;gap:14px;padding:10px 12px 10px 56px} + its hairline from 56
        El::row().items(AlignItems::FLEX_START).gap(14.0).pad(10.0, 12.0, 10.0, 56.0).child(hair56()).child(acl).child(acr)
    }

    fn cap_row(&mut self, cx: &mut Cx, vals: &bu_mouse::accel::panel::CurveValues) -> El {
        let on = match vals.cap_type {
            CapType::Input => 0,
            CapType::Output => 1,
            CapType::Both => 2,
        };
        let s = segx::seg_ex(cx, K_CAP, &[segx::Label::Text("Input"), segx::Label::Text("Output"), segx::Label::Text("Both")], Some(on), &segx::ACCT);
        El::row()
            .center()
            .gap(8.0)
            .h(27.0)
            .child(El::text("Cap type", Font::new(12.0, 400), FG2(), lh(12.0, 1.35)).w(84.0).none())
            .child(s)
    }

    fn epp_warn(&mut self, cx: &mut Cx) -> El {
        // .acwarn{display:flex;align-items:center;gap:7px;padding:9px 12px 10px 56px;font-size:11px;line-height:15px;color:var(--fg2)}
        // .acwarn svg{width:14px;height:14px;stroke:var(--amber);stroke-width:1.5} #sw .acwarn .lnk{font-size:11.5px}
        El::row()
            .center()
            .gap(7.0)
            .pad(9.0, 12.0, 10.0, 56.0)
            .child(hair56())
            .child(El::icon("tri", 14.0, 1.5, AMBER()))
            .child(El::text("Windows\u{2019} Enhance pointer precision is on too, so the two stack.", Font::new(11.0, 400), FG2(), 15.0).none())
            .child(link::link(cx, K_EPPOFF, "Turn it off", 11.5))
    }

    /// "Another program also writes the driver" (Order 063): the card's amber line, same look as the Enhance-pointer-precision one.
    fn writer_warn(&mut self, line: &str) -> El {
        El::row()
            .items(AlignItems::FLEX_START)
            .gap(7.0)
            .pad(9.0, 12.0, 10.0, 56.0)
            .child(hair56())
            .child(El::icon("tri", 14.0, 1.5, AMBER()).margin(1.0, 0.0, 0.0, 0.0))
            .child(El::text(line, Font::new(11.0, 400), FG2(), 15.0).wrapping().flex1())
    }

    fn per_app_rows(&mut self, cx: &mut Cx) -> El {
        let mut rows = Vec::new();
        let list: Vec<_> = self.per_app.rows().to_vec();
        if list.is_empty() {
            // collapsed until used (the owner: "optional to have it on different preset per game")
            return El::row()
                .center()
                .gap(12.0)
                .min_h(40.0)
                .pad(7.0, 12.0, 7.0, 56.0)
                .child(hair56())
                .child(bits::addb(cx, K_ADD, "Add a game", false))
                .child(El::text("A different preset while it is open", Font::new(11.0, 400), FG3(), lh(11.0, 1.35)).ellipsis().flex1_auto());
        }
        for (i, r) in list.iter().enumerate() {
            let rk = idx(K_ROW, i);
            let rh = cx.hovered(rk);
            let game = game_of(&r.exe);
            // .accc .aapp .pu.app{width:172px} .pu.app{max-width:none;padding-left:4px}
            let mut kids = Vec::new();
            if let Some(g) = game {
                kids.push(game_tile(g));
            }
            let lab = if r.label.is_empty() { "Choose a game".to_string() } else { r.label.clone() };
            kids.push(El::text(lab, pieces::btn_font(13.0, 400), if r.label.is_empty() { FG3() } else { FG() }, lh(13.0, 1.35)).ellipsis().flex1_auto());
            let app = dropdown::dropdown_with(cx, sub(rk, "app"), kids, 4.0, 6.0).w(172.0);
            let pre = self.preset_btn(cx, sub(rk, "pre"), r.target);
            // the game was already open when the app noticed it: not switched, and the row says so (Order 063)
            let late = self.v.late.contains(&r.id);
            let note = if late {
                El::text("Already open \u{b7} next launch", Font::new(11.0, 400), AMBER(), lh(11.0, 1.35)).ellipsis().flex1().title("This game was already running when the app noticed it, so it keeps the settings it started with. The next launch switches.")
            } else {
                El::block().flex1()
            };
            rows.push(
                El::row()
                    .center()
                    .gap(8.0)
                    .min_h(40.0)
                    .pad(7.0, 12.0, 7.0, 56.0)
                    .key(rk)
                    .children(if i == 0 { None } else { Some(hair56()) })
                    .child(app)
                    .child(arrow())
                    .child(pre)
                    .child(note)
                    // Order 045: `h('button',{class:'rdel',title:'Remove',…})`
                    .child(rowbits::rdel(cx, sub(rk, "x"), rh).title("Remove")),
            );
        }
        // .accc .row.addr{min-height:36px}
        rows.push(El::row().center().gap(12.0).min_h(36.0).pad(7.0, 12.0, 7.0, 56.0).child(hair56()).child(bits::addb(cx, K_ADD, "Add a game", false)));
        // "When none of these games are open" -> the main preset (default) / another preset / Off
        let eb = self.preset_btn(cx, K_ELSE, self.per_app.everywhere_else());
        rows.push(
            El::row()
                .center()
                .gap(10.0)
                .min_h(40.0)
                .pad(7.0, 12.0, 7.0, 56.0)
                .child(hair56())
                .child(El::text("When none of these games are open", Font::new(12.5, 400), FG2(), lh(12.5, 1.35)).none().pad(0.0, 0.0, 0.0, 2.0))
                .child(eb),
        );
        El::block().children(rows)
    }

    /// `.pu.apu{width:128px;font-size:12.5px}` `.pu.apu.offp span{color:var(--fg2)}` (the button's own font rule wins: 13 px)
    fn preset_btn(&mut self, cx: &mut Cx, k: Key, t: Target) -> El {
        let (name, off) = match t {
            Target::Main => ("Main preset".to_string(), false),
            Target::Off => ("Off".to_string(), true),
            Target::Preset(id) => match self.panel.preset(id) {
                Some(p) => (p.name.clone(), false),
                None => ("Off".to_string(), true),
            },
        };
        let mut b = dropdown::dropdown(cx, k, &name, Some(128.0));
        if off {
            if let Some(crate::ui::el::Content::Text(t)) = b.children.first_mut().map(|c| &mut c.content) {
                t.color = FG2();
            }
        }
        b
    }

    // ================================================================== MOUSE SETTINGS
    fn settings(&mut self, cx: &mut Cx) -> El {
        let w = self.v.win;
        let speed = w.map(|w| w.pointer_speed).unwrap_or(10);
        let epp = w.map(|w| w.precision).unwrap_or(false);
        let (lines_v, lines_t) = match w.map(|w| w.scroll_lines) {
            Some(ScrollLines::Lines(n)) => (n, if n == 1 { "1 line".to_string() } else { format!("{n} lines") }),
            Some(ScrollLines::OneScreen) => (100, "One screen".to_string()),
            None => (3, "3 lines".to_string()),
        };
        let dbl = w.map(|w| w.double_click_ms).unwrap_or(500);
        let swap = w.map(|w| w.buttons_swapped).unwrap_or(false);
        let msl = |cx: &mut Cx, k: Key, v: f32, t: &str| {
            // .msl{display:flex;align-items:center;gap:10px} .msl .rng{width:168px} .msl .sv{min-width:50px}
            // Order 045: an arrow key = one stop (pointer speed 20, scroll lines 100, double-click 15 - `slider_event`)
            let n = match k {
                K_SPEED => 20,
                K_LINES => 100,
                K_DBL => 15,
                _ => 101,
            };
            El::row().center().gap(10.0).none().child(slider::stops(slider::slider(cx, k, v, 168.0, 20.0, slider::default()), n)).child(slider::value_label(t).min_w(50.0))
        };
        let r_speed = group::row(true, vec![group::lbl("Pointer speed", None), group::ctl(vec![msl(cx, K_SPEED, (speed as f32 - 1.0) / 19.0, &speed.to_string())])]);
        let r_epp = group::row(false, vec![group::lbl("Enhance pointer precision", Some("Acceleration: quick moves go further")), group::ctl(vec![toggle::toggle(cx, K_EPP, epp, false)])]);
        let r_lines = group::row(false, vec![group::lbl("Scroll lines", None), group::ctl(vec![msl(cx, K_LINES, (lines_v.clamp(1, 100) as f32 - 1.0) / 99.0, &lines_t)])]);
        // #sw .dct{30x30;border-radius:8px;background:var(--well);box-shadow:inset 0 0 0 .5px var(--hair)} :hover{background:var(--ctl)}
        let dhv = cx.hover_t(K_DCT, 150.0, EASE);
        let open = self.dct_open;
        let pop = cx.tr(K_DCT, 4, if open { 1.0 } else { 0.0 }, 280.0, EASE);
        let _ = pop;
        let dct = El::block()
            .size(30.0, 30.0)
            .none()
            .radius(8.0)
            .bg(cmix(WELL(), CTL(), dhv))
            .inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
            .place_center()
            .on_click(K_DCT)
            // Order 045: `h('button',{class:'dct',title:'Double-click to test',…})`
            .title("Double-click to test")
            .cursor(Cursor::Hand)
            .child(El::paint(move |g, (x, y, _, _)| art::folder(g, x, y, open)).size(20.0, 20.0).no_hit());
        let step = double_click_step_for_ms(dbl);
        let r_dbl = group::row(
            false,
            vec![group::lbl("Double-click speed", Some("Double-click the folder to test it")), group::ctl(vec![dct, msl(cx, K_DBL, step as f32 / 14.0, &format!("{dbl} ms"))])],
        );
        let r_swap = group::row(false, vec![group::lbl("Swap primary button", None), group::ctl(vec![toggle::toggle(cx, K_SWAP, swap, false)])]);
        El::block().child(group::gh("Mouse settings")).child(group::grp(vec![r_speed, r_epp, r_lines, r_dbl, r_swap]))
    }

    // ================================================================== CURSORS
    fn cursors(&mut self, cx: &mut Cx) -> El {
        let size = self.v.cursors.as_ref().map(|c| c.size).unwrap_or(1);
        // .gh .ghr .lnk{font-size:11.5px;line-height:15px} .gxs{margin:0 4px} .sl{display:flex;align-items:center;gap:10px}
        // .ghr .rng{width:132px} .sv.cszv{min-width:18px}
        let ghr = El::row()
            .ml_auto()
            .center()
            .gap(10.0)
            .none()
            // `.gh .ghr .lnk{11.5px/15px}` loses to `#sw .lnk{12px/16px}` (an id rule): Chromium's computed 12 / 16
            // Order 045: `impLnk.title='Pick .cur / .ani files or a downloaded cursor pack folder'`
            .child(link::link(cx, K_SGET, "Get more cursors\u{2026}", 12.0).title("Cursor packs with an open licence - one click adds one"))
            .child(pieces::separator(4.0))
            .child(link::link(cx, K_IMP, "Import cursors\u{2026}", 12.0).title("Pick .cur / .ani files or a downloaded cursor pack folder"))
            .child(pieces::separator(4.0))
            .child(El::text("Size", Font::new(11.0, 400), FG2(), lh(11.0, 1.35)).none())
            .child(
                El::row()
                    .center()
                    .gap(10.0)
                    .none()
                    // Order 045: `szIn.title=curPx()+' px'` (curPx = 32 + (size - 1) × 16)
                    .child(slider::stops(slider::slider(cx, K_SIZE, (size as f32 - 1.0) / 14.0, 132.0, 20.0, slider::default()), 15).title(&format!("{} px", bu_mouse::cursors::size_px(size))))
                    .child(slider::value_label(&size.to_string()).min_w(18.0)),
            );
        let gh = group::gh("Cursors").child(ghr);
        // .crow1{display:grid;grid-template-columns:repeat(7,minmax(0,1fr));align-items:start;padding:12px 6px 10px}
        let ck = 1.0 + (size as f32 - 1.0) * 0.04;
        let mut row = El::grid().cols(7).items(AlignItems::START).pad(12.0, 6.0, 10.0, 6.0);
        for (i, r) in Role::ALL.iter().enumerate() {
            let k = idx(K_ROLE, i);
            let hv = cx.hover_t(k, 150.0, EASE);
            let pr = cx.active_t(k, 120.0, EASE);
            let ctx = self.menu == Some(Menu::Cursor(i));
            let look = self.look_of_role(*r);
            // Order 066: his REAL cursor of this role (its file), the stand-in drawing only when there is no file to read
            let real = self.v.cursors.as_ref().and_then(|c| c.roles.iter().find(|x| x.role == *r)).and_then(|x| pic::load(&x.file));
            let sc = cx.tr(k, 9, ck, 260.0, crate::anim::Bezier::new(0.3, 0.7, 0.2, 1.0));
            let spin = self.spin();
            // .crb .cpv{44x44;border-radius:12px;background:var(--ctl);box-shadow:inset 0 0 0 .5px var(--hair),0 1px 2px rgba(0,0,0,.12)}
            // :hover .cpv{background:var(--ctl-h);box-shadow:inset 0 0 0 1px var(--acc-s)} .ctx .cpv{background:var(--sel);inset 1.5px acc}
            // :active .cpv{transform:scale(.95)}
            let (bg, ins) = if ctx { (SEL(), sh(0.0, 0.0, 0.0, 1.5, ACC())) } else { (cmix(CTL(), CTL_H(), hv), if hv > 0.5 { sh(0.0, 0.0, 0.0, 1.0, ACC_S()) } else { sh(0.0, 0.0, 0.0, 0.5, HAIR()) }) };
            // light (Order 033): `#sw.light .crb .cpv{background:rgba(60,60,67,.13)}` `#sw.light .crb:hover .cpv{...18}` (white cursors need
            // a slightly deeper cell; it outweighs :hover / .ctx)
            let bg = if crate::ui::is_light() { cmix(Rgba::rgba(60, 60, 67, 0.13), Rgba::rgba(60, 60, 67, 0.18), hv) } else { bg };
            let cpv = El::block()
                .size(44.0, 44.0)
                .none()
                .radius(12.0)
                .bg(bg)
                .inset(&[ins])
                .shadow(&[sh(0.0, 1.0, 2.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.12))])
                .scale(1.0 - 0.05 * pr)
                .child(
                    El::paint(move |g, (x, y, _, _)| match &real {
                        Some(p) => pic::paint(g, p, x + 7.0, y + 7.0, 30.0, sc),
                        None => art::cursor(g, &look, i, x + 7.0, y + 7.0, 30.0, sc, spin),
                    })
                    .abs(0.0, 0.0, 0.0, 0.0)
                    .no_hit(),
                );
            // .crn{font-size:10.5px;line-height:13px;color:var(--fg2)} :hover/.ctx .crn{color:var(--fg)}
            let crn = El::text(r.name(), Font::new(10.5, 400), cmix(FG2(), FG(), if ctx { 1.0 } else { hv }), 13.0).none();
            // Order 045: `const t=roleTitle(r)+': '+st.name;b.title=t+' · click to change'`
            let set = self.v.cursors.as_ref().and_then(|c| c.roles.iter().find(|x| x.role == *r)).map(|x| x.set.clone()).unwrap_or(SetId::WindowsDefault);
            let title = format!("{}: {} \u{b7} click to change", ROLE_TITLE[i], set.label());
            row = row.child(
                El::col()
                    .center()
                    .gap(5.0)
                    .self_align(taffy::style::AlignSelf::START)
                    .style(|s| s.justify_self = Some(AlignItems::CENTER))
                    .on_click(k)
                    .title(&title)
                    .cursor(Cursor::Hand)
                    .child(cpv)
                    .child(crn),
            );
        }
        El::block().child(gh).child(group::grp(vec![row]).clip())
    }

    fn look_of_set(&self, s: &SetId) -> art::Look {
        match s {
            SetId::Glass => art::GLASS,
            // an imported pack: the drawing's stand-in look (its real .cur / .ani pictures are not read yet - see the
            // report); Windows default and anything set elsewhere: Windows' own look
            SetId::Pack(_) | SetId::Own | SetId::OwnFile(_) => art::NEON,
            SetId::WindowsDefault | SetId::Other(_) | SetId::Scheme(_) => art::WIN,
        }
    }

    fn look_of_role(&self, r: Role) -> art::Look {
        let set = self.v.cursors.as_ref().and_then(|c| c.roles.iter().find(|x| x.role == r)).map(|x| x.set.clone()).unwrap_or(SetId::WindowsDefault);
        self.look_of_set(&set)
    }

    /// The spinners' turn (`cspin 1.1s linear infinite`): frozen test pictures keep 0.
    fn spin(&self) -> f32 {
        0.0
    }

    /// The "when none of these games are open" list (None = a separator): the main preset (default), the presets, Off.
    fn else_options(&self) -> Vec<Option<Target>> {
        let mut v = vec![Some(Target::Main)];
        if !self.panel.presets.is_empty() {
            v.push(None);
            v.extend(self.panel.presets.iter().map(|p| Some(Target::Preset(p.id))));
        }
        v.push(None);
        v.push(Some(Target::Off));
        v
    }

    fn else_label(&self, t: Target) -> String {
        match t {
            Target::Main => "Main preset".into(),
            Target::Off => "Off".into(),
            Target::Preset(id) => self.panel.preset(id).map(|p| p.name.clone()).unwrap_or_else(|| "Off".into()),
        }
    }

    // ================================================================== popups
    fn menu_items(&self, m: Menu) -> Vec<MItem> {
        let it = |l: &str, c: bool| MItem::Item { label: l.to_string(), game: None, checked: c };
        match m {
            Menu::Curve => CURVES.iter().map(|c| it(c.name(), *c == self.panel.curve)).collect(),
            Menu::RowApp(id) => {
                let cur = self.per_app.rows().iter().find(|r| r.id == id).map(|r| r.exe.clone()).unwrap_or_default();
                let mut v: Vec<MItem> = GAMES
                    .iter()
                    .map(|g| MItem::Item { label: g.name.to_string(), game: Some(g), checked: g.exe.eq_ignore_ascii_case(&cur) })
                    .collect();
                v.push(MItem::Sep);
                v.push(it("Browse for an app\u{2026}", false));
                v
            }
            Menu::RowPreset(id) => {
                let cur = self.per_app.rows().iter().find(|r| r.id == id).map(|r| r.target).unwrap_or(Target::Off);
                let mut v: Vec<MItem> = self.panel.presets.iter().map(|p| it(&p.name, cur == Target::Preset(p.id))).collect();
                v.push(MItem::Sep);
                v.push(it("Off", cur == Target::Off));
                v
            }
            Menu::Else => {
                let cur = self.per_app.everywhere_else();
                self.else_options()
                    .into_iter()
                    .map(|o| match o {
                        None => MItem::Sep,
                        Some(t) => it(&self.else_label(t), t == cur),
                    })
                    .collect()
            }
            Menu::Cursor(_) => Vec::new(),
        }
    }

    /// The open list as `mitems::menu` rows' box, under the button's box (`self.anchor`). A click on row i = `Ev::Click(idx(K_MENU,
    /// i))`, separators counted (`pick` takes that row index).
    fn menu_box(&self, cx: &mut Cx, m: Menu) -> El {
        let items = self.menu_items(m);
        let rows: Vec<Row> = items
            .iter()
            .map(|it| match it {
                MItem::Sep => Row::Sep,
                // an app tile is 18 px wide: `Lead::Gt` holds its place (16 px) until the real tile replaces it below
                MItem::Item { label, game, checked } => {
                    let r = It::tick(label, *checked);
                    Row::Item(if game.is_some() { r.lead(Lead::Gt("", Rgba(0.0, 0.0, 0.0, 0.0))) } else { r })
                }
            })
            .collect();
        let (ax, ay, aw, ah) = self.anchor;
        let mut menu = mitems::menu(cx, K_MENU, &rows, Place::Under(ax, ay, aw, ah), 150.0);
        // `.mitem .at{margin-right:2px}`: the game's tile in place of the placeholder (child 0 = the tick)
        for (it, row) in items.iter().zip(menu.children.iter_mut()) {
            if let (MItem::Item { game: Some(g), .. }, Some(slot)) = (it, row.children.get_mut(1)) {
                *slot = game_tile(g).margin(0.0, 2.0, 0.0, 0.0);
            }
        }
        menu
    }

    fn pick(&mut self, m: Menu, i: usize, now: f64) {
        match m {
            Menu::Curve => {
                if let Some(c) = CURVES.get(i) {
                    self.panel.set_curve(*c);
                    self.push_accel();
                }
            }
            Menu::RowApp(id) => {
                if let Some(g) = GAMES.get(i) {
                    self.per_app.set_row_exe(id, g.exe);
                    self.per_app.set_row_label(id, g.name);
                    self.push_accel();
                } else if i == GAMES.len() + 1 {
                    self.say("Picks any .exe in the real app", now);
                }
            }
            Menu::RowPreset(id) => {
                let n = self.panel.presets.len();
                let t = if i < n { Target::Preset(self.panel.presets[i].id) } else { Target::Off };
                self.per_app.set_row_target(id, t);
                self.push_accel();
            }
            Menu::Else => {
                if let Some(Some(t)) = self.else_options().get(i).copied() {
                    self.per_app.set_everywhere_else(t);
                    self.push_accel();
                }
            }
            Menu::Cursor(_) => {}
        }
    }

    fn cursor_menu(&mut self, cx: &mut Cx, ri: usize) -> El {
        let role = Role::ALL[ri];
        let cur = self.v.cursors.as_ref().and_then(|c| c.roles.iter().find(|x| x.role == role)).map(|x| x.set.clone()).unwrap_or(SetId::WindowsDefault);
        let sets = self.picker_sets(ri);
        // hover a row = try that cursor (Windows shows it until the pointer leaves the row / the list closes)
        let hovered = (0..sets.len()).find(|j| cx.hovered(idx(K_MENU, *j))).map(|j| (ri, sets[j].id.clone()));
        if hovered != self.preview {
            match &hovered {
                Some((r, s)) => self.send(Cmd::Preview(Role::ALL[*r], s.clone())),
                None => self.send(Cmd::EndPreview),
            }
            self.preview = hovered;
        }
        let mut kids = Vec::new();
        // .cmh{display:flex;align-items:baseline;gap:8px;margin:4px 7px 7px;font-size:13px} b{600} span{11px --fg3}
        // (baseline: the 11 px line sits 3 px lower - Chromium's computed boxes; the layout engine has no text baselines)
        kids.push(
            El::row()
                .items(AlignItems::FLEX_START)
                .gap(8.0)
                .margin(4.0, 7.0, 7.0, 7.0)
                .child(El::text(ROLE_TITLE[ri], Font::new(13.0, 600), FG(), lh(13.0, 1.35)).none())
                .child(El::text("Hover one to try it", Font::new(11.0, 400), FG3(), lh(11.0, 1.35)).none().margin(3.0, 0.0, 0.0, 0.0)),
        );
        let mut list = El::col().gap(1.0);
        let mut n = 0;
        let first_rest = sets.iter().position(|s| !s.top);
        for (j, st) in sets.iter().enumerate() {
            if Some(j) == first_rest {
                list = list.child(El::block().h(1.0).margin(4.0, 6.0, 4.0, 6.0).bg(HAIR()));
            }
            let look = self.look_of_set(&st.id);
            list = list.child(self.cmi(cx, idx(K_MENU, n), ri, &look, st.file.as_deref(), &st.name, &st.note, st.id == cur, st.imp, st.matches));
            n += 1;
        }
        list = list.child(El::block().h(1.0).margin(4.0, 6.0, 4.0, 6.0).bg(HAIR()));
        // .cmi.own: a dashed tile with plus12 (13 px, --ico-on 1.6), the name in --ico-on 600
        let k = idx(K_MENU, 900);
        let hv = cx.hover_t(k, 120.0, EASE);
        let own_tile = El::block()
            .size(36.0, 36.0)
            .none()
            .place_center()
            .child(El::paint(|g, (x, y, w, h)| crate::pages::display::dashed_rr(g, x, y, w, h, 8.0, DASH())).abs(0.0, 0.0, 0.0, 0.0).no_hit())
            .child(El::icon("plus12", 13.0, 1.6, ICO_ON()));
        list = list.child(
            El::row()
                .center()
                .gap(10.0)
                .h(46.0)
                .pad(0.0, 9.0, 0.0, 5.0)
                .radius(8.0)
                .bg(HOV().mul_a(hv))
                .on_click(k)
                .cursor(Cursor::Hand)
                .child(own_tile)
                .child(
                    El::col()
                        .flex1()
                        .child(El::text("Choose your own file\u{2026}", Font::new(13.0, 600), ICO_ON(), 17.0).ellipsis())
                        .child(El::text("A .cur or .ani file", Font::new(11.0, 400), FG3(), 14.0)),
                ),
        );
        // Order 066: "Get more cursors" - the same dashed tile, opens the window of downloadable packs
        let gk = idx(K_MENU, 901);
        let ghv = cx.hover_t(gk, 120.0, EASE);
        let get_tile = El::block()
            .size(36.0, 36.0)
            .none()
            .place_center()
            .child(El::paint(|g, (x, y, w, h)| crate::pages::display::dashed_rr(g, x, y, w, h, 8.0, DASH())).abs(0.0, 0.0, 0.0, 0.0).no_hit())
            .child(El::icon("plus12", 13.0, 1.6, ICO_ON()));
        list = list.child(
            El::row()
                .center()
                .gap(10.0)
                .h(46.0)
                .pad(0.0, 9.0, 0.0, 5.0)
                .radius(8.0)
                .bg(HOV().mul_a(ghv))
                .on_click(gk)
                .cursor(Cursor::Hand)
                .child(get_tile)
                .child(
                    El::col()
                        .flex1()
                        .child(El::text("Get more cursors…", Font::new(13.0, 600), ICO_ON(), 17.0).ellipsis())
                        .child(El::text("Packs with an open licence", Font::new(11.0, 400), FG3(), 14.0)),
                ),
        );
        // .menu.curm{width:272px;padding:6px}
        let (ax, ay, aw, ah) = self.anchor;
        let mut est = 6.0 + 30.0 + 47.0 * (n as f32 + 2.0) + 18.0 + 6.0;
        // Order 042: with Windows' cursor schemes the list can be taller than the window - it scrolls inside the menu
        let max_h = crate::ui::WIN_H - 16.0;
        if est > max_h {
            let list_h = 47.0 * (n as f32 + 2.0) + 18.0 - (est - max_h);
            list = cx.scroll_box(sub(K_MENU, "sc"), vec![list]).h(list_h);
            est = max_h;
        }
        kids.push(list);
        let mut y = ay + ah + 4.0;
        if y + est > crate::ui::WIN_H - 8.0 {
            y = (ay - est - 4.0).max(8.0);
        }
        let x = if ax + 272.0 > crate::ui::WIN_W - 8.0 { (ax + aw - 272.0).max(8.0) } else { ax };
        let t = cx.tr(K_MENU, 1, 1.0, 120.0, EASE);
        El::col()
            .abs(x.round(), y.round(), f32::NAN, f32::NAN)
            .w(272.0)
            .pad_all(6.0)
            .radius(10.0)
            .bg(POP())
            .backdrop(30.0, 1.8)
            .shadow(&[sh(0.0, 0.0, 0.0, 0.5, Rgba(0.0, 0.0, 0.0, 0.35)), sh(0.0, 12.0, 32.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.35))])
            .inset(&[sh(0.0, 0.0, 0.0, 0.5, HL_V19())])
            .opacity(t)
            .scale(0.97 + 0.03 * t)
            .key(K_MENU)
            .children(kids)
    }

    /// `.cmi{display:flex;align-items:center;gap:10px;height:46px;padding:0 9px 0 5px;border-radius:8px}` `:hover{background:var(--hov)}`
    /// `.on{background:var(--sel)}` `.cmi .cpv{36x36;border-radius:8px}` (well + hairline) `svg{26px}` `.cmt b{13px 500 17px}`
    /// `small{11px 14px --fg3}` `.cck{14px, --ico-on 1.8}` shown when on; × on an imported set on hover (`.ichx`).
    /// The rows of a role's picker (`openCurPick`): Glass - the app's own set - first (Order 040; it says "Matches your other
    /// cursors" when it is that set), Windows default, then another set that matches your other cursors, then the rest
    /// (imported packs that have this role), each once.
    fn picker_sets(&self, ri: usize) -> Vec<PickRow> {
        let role = Role::ALL[ri];
        let m = self.v.suggest.get(ri).cloned().flatten();
        // Order 066: every row draws the REAL cursor file of its set for this bubble
        let file_of = |id: &SetId| -> Option<String> { self.v.files.iter().find(|(s, _)| s == id).and_then(|(_, f)| f.get(ri).cloned().flatten()) };
        let mut v = Vec::new();
        // what he has right now, when it is none of the sets below (a file he picked, a mix, another tool's cursors)
        if let Some(c) = self.v.cursors.as_ref().and_then(|c| c.roles.iter().find(|x| x.role == role)).filter(|c| matches!(c.set, SetId::Own | SetId::Other(_))) {
            let note = if c.file.is_empty() { "Windows' built-in cursor".to_string() } else { c.file.rsplit(['\\', '/']).next().unwrap_or("").to_string() };
            v.push(PickRow { id: c.set.clone(), name: "Your current cursors".into(), note, imp: false, top: true, matches: false, file: Some(c.file.clone()).filter(|f| !f.is_empty()) });
        }
        if self.v.glass {
            let matches = m.as_ref() == Some(&SetId::Glass);
            let note = if matches { "Matches your other cursors" } else { "Frosted glass" };
            v.push(PickRow { id: SetId::Glass, name: "Glass".into(), note: note.into(), imp: false, top: true, matches, file: file_of(&SetId::Glass) });
        }
        v.push(PickRow {
            id: SetId::WindowsDefault,
            name: "Windows default".into(),
            note: "The cursors Windows came with".into(),
            imp: false,
            top: true,
            matches: false,
            file: file_of(&SetId::WindowsDefault),
        });
        if let Some(id) = m.as_ref().filter(|id| **id != SetId::Glass) {
            let imp = matches!(id, SetId::Pack(_));
            v.push(PickRow { id: id.clone(), name: id.label(), note: "Matches your other cursors".into(), imp, top: true, matches: true, file: file_of(id) });
        }
        for p in &self.v.packs {
            let id = SetId::Pack(p.name.clone());
            if p.has(role) && m.as_ref() != Some(&id) {
                // a pack from "Get more cursors" says so; the others were imported by hand
                let note = if bu_mouse::store::LIST.iter().any(|l| l.name == p.name) { "Made by the community" } else { "Imported" };
                v.push(PickRow { file: file_of(&id), id, name: p.name.clone(), note: note.into(), imp: true, top: false, matches: false });
            }
        }
        // the files he picked before with "Choose your own file…" - any of them can be put on this bubble again
        for f in &self.v.own_files {
            let name = f.rsplit(['\\', '/']).next().unwrap_or(f).to_string();
            v.push(PickRow { id: SetId::OwnFile(f.clone()), name, note: "A file you picked".into(), imp: false, top: false, matches: false, file: Some(f.clone()) });
        }
        // Order 042: every cursor scheme Windows has installed (the owner: "is there more windows presets for cursors?")
        for (name, roles) in &self.v.schemes {
            let id = SetId::Scheme(name.clone());
            if roles.contains(&role) && m.as_ref() != Some(&id) {
                v.push(PickRow { file: file_of(&id), id, name: name.clone(), note: "Windows cursor scheme".into(), imp: false, top: false, matches: false });
            }
        }
        v
    }

    #[allow(clippy::too_many_arguments)]
    fn cmi(&mut self, cx: &mut Cx, k: Key, ri: usize, look: &art::Look, file: Option<&str>, name: &str, note: &str, on: bool, imp: bool, matches: bool) -> El {
        let hv = cx.hover_t(k, 120.0, EASE);
        let look = *look;
        // Order 066: this set's REAL cursor for the bubble (its .cur / .ani file); the stand-in only when it can't be read
        let real = file.and_then(pic::load);
        let tile = El::block()
            .size(36.0, 36.0)
            .none()
            .radius(8.0)
            // (light, Order 033: `#sw.light .cmi .cpv{background:rgba(60,60,67,.13)}`)
            .bg(if crate::ui::is_light() { Rgba::rgba(60, 60, 67, 0.13) } else { WELL() })
            .inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
            .child(
                El::paint(move |g, (x, y, _, _)| match &real {
                    Some(p) => pic::paint(g, p, x + 5.0, y + 5.0, 26.0, 1.0),
                    None => art::cursor(g, &look, ri, x + 5.0, y + 5.0, 26.0, 1.0, 0.0),
                })
                .abs(0.0, 0.0, 0.0, 0.0)
                .no_hit(),
            );
        let mut r = El::row()
            .center()
            .gap(10.0)
            .h(46.0)
            .pad(0.0, 9.0, 0.0, 5.0)
            .radius(8.0)
            .bg(if on { SEL() } else { HOV().mul_a(hv) })
            .on_click(k)
            .cursor(Cursor::Hand)
            .child(tile)
            // `.cmi.match .cmt small{color:var(--ico-on);font-weight:600}`
            .child(
                El::col()
                    .flex1()
                    .child(El::text(name, Font::new(13.0, 500), FG(), 17.0).ellipsis())
                    .child(El::text(note, Font::new(11.0, if matches { 600 } else { 400 }), if matches { ICO_ON() } else { FG3() }, 14.0)),
            );
        if imp {
            let xk = sub(k, "x");
            let xh = cx.hover_t(xk, 120.0, EASE);
            r = r.child(
                El::block()
                    .size(20.0, 20.0)
                    .none()
                    .radius(6.0)
                    .bg(Rgba::rgba(255, 69, 58, 0.14).mul_a(xh))
                    .place_center()
                    .opacity(hv)
                    .on_click(xk)
                    // Order 045: `h('span',{class:'ichx',title:'Delete',…})`
                    .title("Delete")
                    .child(El::icon("x", 8.0, 1.5, cmix(FG3(), crate::ui::RED(), xh)).no_hit()),
            );
        }
        r.child(El::icon("dcheck", 14.0, 1.8, ICO_ON()).opacity(if on { 1.0 } else { 0.0 }))
    }

    fn slider_event(&mut self, k: Key, x: f32, r: (f32, f32, f32, f32), release: bool) {
        let t = ((x - r.0 - 8.0) / (r.2 - 16.0)).clamp(0.0, 1.0);
        let step = |n: u32| (t * (n - 1) as f32).round() as u32;
        if let Some(w) = &mut self.v.win {
            if k == K_SPEED {
                let v = 1 + step(20);
                if v != w.pointer_speed {
                    w.pointer_speed = v;
                    self.send(Cmd::Speed(v));
                }
                return;
            }
            if k == K_LINES {
                let v = 1 + step(100);
                if w.scroll_lines != ScrollLines::Lines(v) {
                    w.scroll_lines = ScrollLines::Lines(v);
                    self.send(Cmd::Lines(v));
                }
                return;
            }
            if k == K_DBL {
                let s = step(15) as usize;
                let ms = double_click_ms_for_step(s);
                if ms != w.double_click_ms {
                    w.double_click_ms = ms;
                    self.dct_last = 0.0;
                    self.send(Cmd::DoubleClick(s));
                }
                return;
            }
        }
        if k == K_SIZE {
            let n = 1 + step(15);
            if let Some(c) = &mut self.v.cursors {
                if c.size != n {
                    c.size = n;
                    if release {
                        self.send(Cmd::CursorSize(n));
                    }
                } else if release {
                    self.send(Cmd::CursorSize(n));
                }
            }
            return;
        }
        // the accel rows (sent to the driver when let go: its writes take ~1 s each)
        for f in all_fields() {
            if k == idx(K_ASL, field_ix(f)) {
                if let Some(spec) = rows(self.panel.curve).into_iter().find(|s| s.field == f) {
                    let lo = vis_min(&spec);
                    let v = lo + (spec.max - lo) * t as f64;
                    self.panel.set_value(f, v);
                    self.accel_dirty = true;
                }
            }
        }
        if k == idx(K_ASL, 99) {
            self.panel.set_sens(SENS.min + (SENS.max - SENS.min) * t as f64);
            self.accel_dirty = true;
        }
        if release && self.accel_dirty {
            self.accel_dirty = false;
            self.push_accel();
        }
    }
}

/// The card's rows' hairline from 56 px (`.card .xin .row::before{left:56px}`).
/// Windows' own cursors folder (`%SystemRoot%\Cursors`): where the cursor pickers open (Order 042, the owner's test 2: "thats
/// the folder it should open to when you press open on the cursors, not some random document folder").
fn cursors_dir() -> String {
    format!(r"{}\Cursors", std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()))
}

fn hair56() -> El {
    El::block().abs(56.0, 0.0, 0.0, f32::NAN).h(1.0).bg(HAIR()).no_hit()
}

/// `.aar{flex:none;width:12px;text-align:center;font-size:12px;color:var(--fg3)}` "→"
fn arrow() -> El {
    El::text("\u{2192}", Font::new(12.0, 400), FG3(), lh(12.0, 1.35)).w(12.0).none().align(Align::Center)
}

/// A `.lnk` with its own size / line-height 15 (`.gh .ghr .lnk`, `.acft .lnk`).
fn small_link(cx: &mut Cx, k: Key, t: &str, size: f32) -> El {
    let hv = cx.hovered(k);
    El::text(t, Font::new(size, 400).ls(0), ACC(), 15.0).underline(hv).none().on_click(k).cursor(Cursor::Hand)
}

/// `.seg.nopick .pill{opacity:0}` (and no segment is `.on`): the DPI is your own.
fn nopick(s: &mut El, on: usize) {
    if let Some(p) = s.children.get_mut(0) {
        p.opacity = 0.0;
    }
    if let Some(b) = s.children.get_mut(on + 1) {
        b.opacity = 0.78;
    }
}

/// One accel row (`.asr{display:flex;align-items:center;gap:8px;height:27px}` `>span:first-child{width:84px;flex:none;
/// font-size:12px;color:var(--fg2)}` `.rng{flex:1}` `.sv{min-width:36px}`; `.sens` = 32 px, margin-top 4, padding-top 4,
/// the hairline above, its label in --fg).
fn asr(cx: &mut Cx, label: &str, k: Key, v: f32, val: &str, sens: bool) -> El {
    let mut r = El::row().center().gap(8.0).h(if sens { 32.0 } else { 27.0 });
    if sens {
        r = r.margin(4.0, 0.0, 0.0, 0.0).pad(4.0, 0.0, 0.0, 0.0).inset(&[sh(0.0, 1.0, 0.0, 0.0, HAIR())]);
    }
    r.child(El::text(label, Font::new(12.0, 400), if sens { FG() } else { FG2() }, lh(12.0, 1.35)).w(84.0).none())
        .child(slider::slider(cx, k, v, 116.0, 20.0, slider::default()))
        .child(slider::value_label(val).min_w(36.0))
}

/// The slider's own range = the drawing's (`APAR`): Acceleration from 0 and Motivity from 1 - the crate starts those one
/// step above (Raw Accel refuses 0 / 1), so the lowest place of the thumb is that one step.
fn vis_min(s: &RowSpec) -> f64 {
    if matches!(s.field, Field::Acceleration | Field::Motivity) {
        s.min - s.step
    } else {
        s.min
    }
}

fn frac(s: &RowSpec, v: f64) -> f32 {
    let lo = vis_min(s);
    ((v - lo) / (s.max - lo)).clamp(0.0, 1.0) as f32
}

fn all_fields() -> [Field; 15] {
    use Field::*;
    [Acceleration, Exponent, InputOffset, CapInput, CapOutput, DecayRate, Limit, JumpInput, JumpOutput, Smooth, SyncSpeed, Motivity, Gamma, Scale, OutputOffset]
}

fn field_ix(f: Field) -> usize {
    all_fields().iter().position(|x| *x == f).unwrap_or(0)
}

impl Page for Mouse {
    fn id(&self) -> &'static str {
        "cur"
    }
    fn name(&self) -> &'static str {
        "Mouse"
    }
    fn icon(&self) -> &'static str {
        "mouse"
    }
    fn open(&mut self, env: &Env, _now: f64) {
        *self = Mouse::default();
        self.fake = env.fake();
        self.frozen = env.frozen;
        // test copies: which of the drawing's sample PCs (BU_TEST_PAGE_STATE "cur:mouse=other" / "cur:raw=none")
        let mut sample = svc::Sample::default();
        if env.test {
            if let Some(v) = std::env::var("BU_TEST_PAGE_STATE").ok().and_then(|v| v.strip_prefix("cur:").map(str::to_string)) {
                sample.unsupported_mouse = v.split(';').any(|p| p == "mouse=other");
                sample.no_raw_accel = v.split(';').any(|p| p == "raw=none");
            }
        }
        self.test = env.test;
        // the add-on's state (Raw Accel installed / restart pending) read again: it may have changed outside the app
        crate::addons::refresh();
        // (the change log already keeps Raw Accel's earlier settings: no copy of them is kept again)
        let accel_logged = crate::services::try_with(|s| crate::undo::read_record(&s.store, svc::PAGE, "accel").is_some()).unwrap_or(false);
        self.svc = Some(Svc::start_full(self.fake, env.test, sample, accel_logged));
        // opening never waits on Windows (PAGES.md): a real copy shows the page at once and fills it from the worker's first
        // answer (tick). Test copies (the fake answers at once) wait for it so the proofs start from the drawing's values.
        let t0 = std::time::Instant::now();
        while env.test && self.svc.as_ref().map(|s| s.opening).unwrap_or(false) && t0.elapsed().as_millis() < 150 {
            std::thread::sleep(std::time::Duration::from_millis(2));
            self.poll_now(0.0);
        }
        if env.test {
            self.test_state();
        }
        self.poll_now(0.0);
    }
    fn close(&mut self) {
        // drops the worker (its channel closes; the thread ends) and every value
        *self = Mouse::default();
    }
    /// Order 063: the acceleration card runs without the tab: at app start the saved card comes back (the driver is written only
    /// when the user had it on and it differs) and the games in the per-game rows are listened for. Real runs only.
    fn background(&self, env: &Env) -> Option<Box<dyn crate::pages::Background>> {
        #[cfg(windows)]
        if !env.test && !crate::undo::headless() {
            return rt::start().map(|g| Box::new(g) as Box<dyn crate::pages::Background>);
        }
        let _ = env;
        None
    }
    /// Windows' settings, the cursors and the mouse's own values are in (the frame holds the tab up to 0.4 s for them)
    fn ready(&self) -> bool {
        self.svc.as_ref().is_none_or(|s| !s.opening) && !self.reading
    }
    fn tick(&mut self, now: f64) -> bool {
        self.poll_now(now)
    }
    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        // a dismiss is for the press that follows it at once (no build in between); one by Esc / a scroll / a right-click
        // is over here - kept, it made the next click on that list's button do nothing
        self.dismissed = None;
        // Order 047: answers on their way no longer keep frames coming - the worker wakes the menu with each one
        // (`svc::run`'s `send`) and `tick` takes it
        if let Some(n) = crate::addons::take_notice() {
            cx.toast(&n);
        }
        self.follow_store(cx);
        let mut kids = vec![pieces::header(self.name(), None)];
        kids.push(self.your_mouse(cx));
        kids.push(self.accel(cx));
        kids.push(self.settings(cx));
        kids.push(self.cursors(cx));
        kids.push(reset::reset_line(cx, K_RS, Some("Windows defaults")));
        kids
    }
    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        let mut layer = El::block().abs(0.0, 0.0, f32::NAN, f32::NAN).size(crate::ui::WIN_W, crate::ui::WIN_H).no_hit();
        let mut any = false;
        if let Some(m) = self.menu {
            any = true;
            // a click beside the popup is the frame's: it calls `popup_dismiss` (no scrim needed)
            let el = match m {
                Menu::Cursor(i) => self.cursor_menu(cx, i),
                _ => self.menu_box(cx, m),
            };
            layer = layer.child(el);
        }
        if let Some(d) = self.store_window(cx) {
            any = true;
            layer = layer.child(d);
        }
        if let Some((win, ticks)) = self.reset.clone() {
            any = true;
            layer = layer.child(self.reset_popup(cx, win, &ticks));
        }
        if let Some((t, at)) = self.toast.clone() {
            if cx.now - at < toast::SHOW_MS + 300.0 {
                any = true;
                layer = layer.child(toast::toast(cx, K_TOAST, &t, at, false));
            } else {
                self.toast = None;
            }
        }
        if any {
            Some(layer)
        } else {
            None
        }
    }
    fn popup_dismiss(&mut self) {
        if self.store_open {
            // (no job to stop here: the window's own Done / close does that; a press beside it just closes the list)
            self.store_open = false;
            store::OPEN.store(false, std::sync::atomic::Ordering::SeqCst);
            pic::forget();
            return;
        }
        self.dismissed = self.menu;
        self.close_menu();
        self.reset = None;
    }
    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        let now = cx.now;
        match ev {
            Ev::Press(k, x, _, r) => {
                // the list the frame closed on this very press (it comes before the press)
                self.press_dismissed = self.dismissed.take();
                // the box a popup list opens under - never a press INSIDE an open list / window (that moved the open
                // cursor list onto the row that was pressed, test build 1)
                if self.menu.is_none() && self.reset.is_none() {
                    self.anchor = *r;
                }
                let k = *k;
                if [K_SPEED, K_LINES, K_DBL, K_SIZE].contains(&k) || (0..100).any(|i| k == idx(K_ASL, i)) {
                    self.drag = Some(k);
                    self.slider_event(k, *x, *r, false);
                }
                if k == K_DPIN {
                    self.dpi_text = String::new();
                }
                if k == K_ACDPI {
                    self.acdpi_text = String::new();
                }
            }
            Ev::Drag(k, x, _, r) => {
                if self.drag == Some(*k) {
                    self.slider_event(*k, *x, *r, false);
                }
            }
            Ev::Release(k) => {
                if self.drag == Some(*k) {
                    let r = (0.0, 0.0, 0.0, 0.0);
                    let _ = r;
                    self.drag = None;
                    if *k == K_SIZE {
                        if let Some(n) = self.v.cursors.as_ref().map(|c| c.size) {
                            self.send(Cmd::CursorSize(n));
                        }
                    }
                    if self.accel_dirty {
                        self.accel_dirty = false;
                        self.push_accel();
                    }
                }
            }
            Ev::Click(k) => {
                self.click(*k, now, cx);
                self.press_dismissed = None;
            }
            Ev::Char(k, c) => {
                // (no select-on-focus here: a press empties the text, so `selected` stays off)
                if *k == K_DPIN {
                    nbox::type_char(&mut self.dpi_text, &mut false, *c, 5, nbox::Filter::Digits);
                }
                if *k == K_ACDPI {
                    nbox::type_char(&mut self.acdpi_text, &mut false, *c, 5, nbox::Filter::Digits);
                }
                if let Some((_, t)) = &mut self.rename {
                    if !c.is_control() && t.chars().count() < bu_mouse::accel::panel::PRESET_NAME_MAX {
                        t.push(*c);
                    }
                }
            }
            Ev::Key(k, vk) => self.key(*k, *vk, cx),
            Ev::Blur(k) => {
                if *k == K_DPIN {
                    self.commit_dpi();
                }
                if *k == K_ACDPI {
                    self.commit_acdpi();
                }
                if self.rename.is_some() {
                    self.end_rename(true, now);
                }
            }
            // Order 045: the wheel over the custom DPI box = ±50 like its arrows (`dpiBox.addEventListener('wheel',e=>{
            // e.preventDefault();setDpi(YM.dpi+(e.deltaY<0?50:-50),true);})`, L4338)
            Ev::Wheel(k, d) => {
                if *k == K_DPIN {
                    if let Some(v) = self.mouse_dpi() {
                        self.send(Cmd::Dpi(if *d > 0 { v + 50 } else { v.saturating_sub(50) }));
                    }
                }
            }
            // no right-click menu or file drop on the drawn Mouse page
            Ev::Context(..) | Ev::Drop(..) | Ev::DragOver(..) => {}
        }
    }
    fn describe(&self) -> String {
        let w = self.v.win;
        format!(
            "speed={} epp={} dpi={} accel_on={} open={} curve={} presets={} menu={:?} toast={:?} rename={:?}",
            w.map(|w| w.pointer_speed).unwrap_or(0),
            w.map(|w| w.precision).unwrap_or(false),
            self.mouse_dpi().unwrap_or(0),
            self.panel.on,
            self.panel.expanded,
            self.panel.curve.name(),
            self.panel.presets.len(),
            self.menu,
            self.toast.as_ref().map(|t| t.0.clone()),
            self.rename.as_ref().map(|r| r.1.clone())
        )
    }
    fn resettable(&mut self) -> Option<&mut dyn crate::undo::Resettable> {
        Some(self)
    }
}

/// Writes one change into the change log (Order 036): at once when the services are free, else queued (`undo::note`).
/// With no services at all (a unit test of the page alone) nothing is written.
fn record_change(item: &str, label: &str, old: &crate::undo::Val, new: &crate::undo::Val) {
    let done = crate::services::try_with(|s| {
        crate::undo::flush(&mut s.store);
        let _ = crate::undo::record(&mut s.store, svc::PAGE, item, label, old, new);
    });
    if done.is_none() && crate::services::in_use() {
        crate::undo::note(svc::PAGE, item, label, old, new);
    }
}

/// The change log's side of the Mouse tab (Order 036): Windows' mouse settings, the cursors, the cursor size, Raw Accel's
/// driver settings. The open tab answers through its worker (its view stays right); a closed one (Settings › Reset, the
/// uninstaller) through a service made at the first use - nothing is made by `resettable()` itself.
impl crate::undo::Resettable for Mouse {
    fn page_id(&self) -> &str {
        svc::PAGE
    }
    fn page_title(&self) -> &str {
        "Mouse"
    }
    fn current(&self, item: &str) -> Option<crate::undo::Val> {
        if self.svc.is_some() {
            return self.v.vals.iter().find(|(i, _)| i == item).map(|(_, v)| v.clone());
        }
        with_cold(&self.cold, |c| c.val(item))
    }
    fn windows_defaults(&self) -> Vec<crate::undo::DefaultItem> {
        if self.svc.is_some() {
            return self.v.win_def.clone();
        }
        with_cold(&self.cold, |c| c.defaults())
    }
    fn apply(&mut self, item: &str, to: &crate::undo::Val) -> Result<(), String> {
        if crate::testmode::real_read() || (self.test && !self.fake && self.svc.is_some()) {
            return Err("A read-only test copy changes nothing".into());
        }
        if self.svc.is_some() && !self.fake {
            // a real change ends with SPIF_SENDCHANGE, a broadcast to every window - this (UI) thread's too: waiting here
            // for the worker would stall both. So it is put back right here, and the open tab's worker reads it again.
            let r = with_cold(&self.cold, |c| c.restore(item, &to.raw));
            self.send(Cmd::Reread);
            self.panel_from_worker = false; // the card is read again from the worker (a reset may have switched it off)
            return r;
        }
        if self.svc.is_some() {
            // (the fake: no broadcast) the open tab's worker does it (one command after the ones it already has); the frame waits for the answer
            let (tx, rx) = std::sync::mpsc::channel();
            self.send(Cmd::Restore(item.to_string(), to.clone(), tx));
            return rx.recv_timeout(std::time::Duration::from_secs(20)).unwrap_or_else(|_| Err("The mouse settings didn\u{2019}t answer in time".into()));
        }
        with_cold(&self.cold, |c| c.restore(item, &to.raw))
    }
    fn detach(&mut self) -> Option<crate::undo::Detached> {
        let open = self.svc.is_some();
        Some(Box::new(MouseReset {
            seen: open.then(|| (self.v.vals.clone(), self.v.win_def.clone())),
            worker: if open && self.fake { self.svc.as_ref().and_then(|s| s.sender()) } else { None },
            cold: self.cold.clone(),
            read_only: self.test && !self.fake && open,
        }))
    }
    fn reset_done(&mut self) {
        // the open tab's worker reads the put-back values again (the fake's worker made the change itself: its answer
        // carries the new view)
        if self.svc.is_some() && !self.fake {
            self.send(Cmd::Reread);
            self.panel_from_worker = false; // the card is read again from the worker (a reset may have switched it off)
        }
    }
}

/// A closed tab's service for the change log, made at its first use (Order 047: shared, a detached copy uses it on the
/// review's worker thread).
type Cold = std::sync::Arc<std::sync::Mutex<Option<Box<dyn svc::Restorer + Send>>>>;

fn with_cold<R>(cold: &Cold, f: impl FnOnce(&mut (dyn svc::Restorer + Send)) -> R) -> R {
    let mut g = cold.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(svc::restorer).as_mut())
}

/// Order 047: the Mouse tab's reset as a copy for the review's worker thread (`Resettable::detach`): the open tab's last
/// read, the open FAKE tab's worker (its fake PC is the one the tab shows), the closed tab's service. A real put-back runs
/// on the worker thread (it ends with SPIF_SENDCHANGE, a broadcast the menu's thread answers - it keeps pumping now).
struct MouseReset {
    seen: Option<(Vec<(String, crate::undo::Val)>, Vec<crate::undo::DefaultItem>)>,
    worker: Option<std::sync::mpsc::Sender<Cmd>>,
    cold: Cold,
    read_only: bool,
}

impl crate::undo::Resettable for MouseReset {
    fn page_id(&self) -> &str {
        svc::PAGE
    }
    fn page_title(&self) -> &str {
        "Mouse"
    }
    fn current(&self, item: &str) -> Option<crate::undo::Val> {
        match &self.seen {
            Some((vals, _)) => vals.iter().find(|(i, _)| i == item).map(|(_, v)| v.clone()),
            None => with_cold(&self.cold, |c| c.val(item)),
        }
    }
    fn windows_defaults(&self) -> Vec<crate::undo::DefaultItem> {
        match &self.seen {
            Some((_, d)) => d.clone(),
            None => with_cold(&self.cold, |c| c.defaults()),
        }
    }
    fn apply(&mut self, item: &str, to: &crate::undo::Val) -> Result<(), String> {
        if crate::testmode::real_read() || self.read_only {
            return Err("A read-only test copy changes nothing".into());
        }
        if let Some(tx) = &self.worker {
            let (btx, rx) = std::sync::mpsc::channel();
            if tx.send(Cmd::RestoreAway(item.to_string(), to.clone(), btx)).is_err() {
                return Err("The mouse settings didn\u{2019}t answer in time".into());
            }
            return rx.recv_timeout(std::time::Duration::from_secs(20)).unwrap_or_else(|_| Err("The mouse settings didn\u{2019}t answer in time".into()));
        }
        with_cold(&self.cold, |c| c.restore(item, &to.raw))
    }
}

impl Mouse {
    /// TEST COPIES ONLY (`env.test`): the test hook's `click` cannot reach page elements yet (see the report), so the
    /// pixel proofs of the open / popup states start the page in that state: `BU_TEST_PAGE_STATE` = `cur:` + `;`-separated
    /// `acopen` (the card open), `menu=<curve|app|pre|else|cursor:N>@x,y,w,h` (that list open under the box x,y,w,h -
    /// the button's window box from the drawing's dom dump), `rename` (the first chip being renamed), `dct` (the double-click
    /// test folder open), `review=<was|win>@x,y,w,h` (that reset review under the link's box, with the drawing's sample
    /// lines). Below the first screen the proofs scroll with the frame's test hook `scroll:<px>`.
    fn test_state(&mut self) {
        if let Ok(v) = std::env::var("BU_TEST_PAGE_STATE") {
            self.test_state_from(&v);
        }
    }

    fn apply_test_rename(&mut self) {
        if self.test_rename {
            if let Some(p) = self.panel.presets.first() {
                self.rename = Some((p.id, p.name.clone()));
                self.test_rename = false;
                self.test_focus = true;
            }
        }
    }

    fn test_state_from(&mut self, v: &str) {
        let Some(v) = v.strip_prefix("cur:") else { return };
        for part in v.split(';') {
            let (k, arg) = part.split_once('=').unwrap_or((part, ""));
            match k {
                "acopen" => self.panel.expanded = true,
                "dct" => self.dct_open = true,
                "menu" => {
                    let (m, at) = arg.split_once('@').unwrap_or((arg, "0,0,0,0"));
                    let n: Vec<f32> = at.split(',').filter_map(|x| x.parse().ok()).collect();
                    if n.len() == 4 {
                        self.anchor = (n[0], n[1], n[2], n[3]);
                    }
                    let row = self.per_app.rows().first().map(|r| r.id);
                    self.menu = match m {
                        "curve" => Some(Menu::Curve),
                        "app" => row.map(Menu::RowApp),
                        "pre" => row.map(Menu::RowPreset),
                        "else" => Some(Menu::Else),
                        c if c.starts_with("cursor:") => c[7..].parse().ok().map(Menu::Cursor),
                        _ => None,
                    };
                }
                // now, or once the presets are there
                "rename" => {
                    self.test_rename = true;
                    self.apply_test_rename();
                }
                "review" => {
                    // `review=<was|win>@x,y,w,h` (the link's window box): the reset review with the drawing's sample lines
                    let (w, at) = arg.split_once('@').unwrap_or((arg, "0,0,0,0"));
                    let n: Vec<f32> = at.split(',').filter_map(|x| x.parse().ok()).collect();
                    if n.len() == 4 {
                        self.anchor = (n[0], n[1], n[2], n[3]);
                    }
                    let win = w == "win";
                    let l = |title, now, to| SampleLine { title, now, to };
                    let lines = if win {
                        vec![l("Enhance pointer precision", "Off", "On"), l("Cursors", "your own", "Windows default"), l("Cursor size", "2", "1")]
                    } else {
                        vec![l("Pointer speed", "10", "8"), l("Enhance pointer precision", "Off", "On"), l("Double-click speed", "faster", "as before")]
                    };
                    // with a box: open now; without: the lines wait for a click on the link (scrolled proofs)
                    if n.len() == 4 {
                        self.reset = Some((win, vec![true; lines.len()]));
                    }
                    self.test_rs = Some(lines);
                }
                _ => {}
            }
        }
    }

    fn take_now(&mut self, r: svc::Reply) {
        self.take(r, 0.0);
    }

    fn poll_now(&mut self, now: f64) -> bool {
        let rs = self.svc.as_mut().map(|s| s.poll()).unwrap_or_default();
        let got = !rs.is_empty();
        for r in rs {
            self.take(r, now);
        }
        got
    }

    /// The reset review (the drawing's `resetPop`): title, line, the ticked list, Cancel / Reset n, under the link (placeMenu
    /// with min width 316: above it when it doesn't fit below).
    fn reset_popup(&mut self, cx: &mut Cx, win: bool, ticks: &[bool]) -> El {
        let lines: Vec<reset::Line> = self
            .test_rs
            .iter()
            .flatten()
            .zip(ticks.iter())
            .map(|(l, t)| reset::Line { title: l.title.into(), from: l.now.into(), to: l.to.into(), ticked: *t, heading: None })
            .collect();
        let n = ticks.iter().filter(|t| **t).count();
        let (title, text) = if win {
            ("Mouse · Windows defaults?", "Each one goes to Windows’ own value.")
        } else {
            ("Mouse · back to how it was?", "Each one goes back to the value it had before this app changed it.")
        };
        let go = if n > 0 { format!("Reset {n}") } else { "Reset".to_string() };
        let buttons = vec![
            button::cbtn_sized(cx, K_RSC, "Cancel", button::Kind::Ghost, button::MCFB, false, 0.0),
            button::cbtn_sized(cx, K_RSG, &go, button::Kind::Red, button::MCFB, n == 0, 0.0),
        ];
        let (ax, ay, aw, ah) = self.anchor;
        // the review's height: 5+17+3+(text: 15 a line at 304 px)+8 + lines (min(196, 26 each + 14)) + 10+26 + 4, + the box's 10
        let text_lines = (cx.g.text_width(text, Font::new(11.5, 400)) / 304.0).ceil().max(1.0);
        let est = 10.0 + 5.0 + 17.0 + 3.0 + 15.0 * text_lines + 8.0 + (lines.len() as f32 * 40.0).min(196.0) + 10.0 + 26.0 + 4.0;
        let mut y = ay + ah + 4.0;
        if y + est > crate::ui::WIN_H - 8.0 {
            y = (ay - est - 4.0).max(8.0);
        }
        // placeMenu: right-aligned with the link when it doesn't fit
        let mw = 316.0f32.max(aw) + 10.0;
        let x = if ax + mw > crate::ui::WIN_W - 8.0 { (ax + aw - mw).max(8.0) } else { ax };
        reset::review_popup(cx, K_RSP, x.round(), y.round(), title, text, &lines, buttons)
    }

    fn close_menu(&mut self) {
        if self.menu.is_some() {
            // the pictures of a picker's rows are read again the next time one opens
            pic::forget();
        }
        self.menu = None;
        if self.preview.take().is_some() {
            self.send(Cmd::EndPreview);
        }
    }

    fn toggle_menu(&mut self, m: Menu) {
        // the list's own button pressed while it was open: the frame already closed it on that press - stays closed
        if self.press_dismissed.take() == Some(m) {
            return;
        }
        if self.menu == Some(m) {
            self.close_menu();
        } else {
            self.close_menu();
            self.menu = Some(m);
        }
    }

    fn commit_dpi(&mut self) {
        if let Ok(v) = self.dpi_text.parse::<u32>() {
            if Some(bu_mouse::device::custom_dpi(v)) != self.mouse_dpi() {
                self.send(Cmd::Dpi(v));
            }
        }
        self.dpi_text.clear();
    }

    fn commit_acdpi(&mut self) {
        if let Ok(v) = self.acdpi_text.parse::<u32>() {
            self.acdpi = bu_mouse::device::custom_dpi(v);
        }
        self.acdpi_text.clear();
    }

    fn end_rename(&mut self, save: bool, now: f64) {
        if let Some((id, t)) = self.rename.take() {
            if save {
                let old = self.panel.preset(id).map(|p| p.name.clone()).unwrap_or_default();
                if t.trim() != old {
                    match self.panel.rename_preset(id, &t) {
                        Ok(()) => self.say(format!("Renamed · {}", t.trim()), now),
                        Err(e) => self.say(e, now),
                    }
                    self.push_accel();
                }
            }
        }
    }

    fn key(&mut self, k: Key, vk: u16, cx: &mut Cx) {
        let now = cx.now;
        const ENTER: u16 = 0x0D;
        const ESC: u16 = 0x1B;
        const BACK: u16 = 0x08;
        const UP: u16 = 0x26;
        const DOWN: u16 = 0x28;
        if vk == ESC && self.menu.is_some() {
            self.close_menu();
            return;
        }
        if k == K_DPIN {
            match vk {
                BACK => {
                    self.dpi_text.pop();
                }
                ENTER => {
                    self.commit_dpi();
                    cx.focus(None);
                }
                ESC => {
                    self.dpi_text.clear();
                    cx.focus(None);
                }
                UP | DOWN => {
                    if let Some(d) = self.mouse_dpi() {
                        let v = if vk == UP { d + 50 } else { d.saturating_sub(50) };
                        self.send(Cmd::Dpi(v));
                    }
                }
                _ => {}
            }
            return;
        }
        if k == K_ACDPI {
            match vk {
                BACK => {
                    self.acdpi_text.pop();
                }
                ENTER => {
                    self.commit_acdpi();
                    cx.focus(None);
                }
                ESC => {
                    self.acdpi_text.clear();
                    cx.focus(None);
                }
                _ => {}
            }
            return;
        }
        if self.rename.is_some() {
            match vk {
                BACK => {
                    if let Some((_, t)) = &mut self.rename {
                        t.pop();
                    }
                }
                ENTER => self.end_rename(true, now),
                ESC => self.end_rename(false, now),
                _ => {}
            }
        }
    }

    fn click(&mut self, k: Key, now: f64, cx: &mut Cx) {
        if self.store_clicked(k, cx) {
            return;
        }
        if let Some((_, ticks)) = &mut self.reset {
            for (i, t) in ticks.iter_mut().enumerate() {
                if k == idx(K_RSP, i) {
                    *t = !*t;
                    return;
                }
            }
        }
        // a popup list's item
        if let Some(m) = self.menu {
            if let Menu::Cursor(ri) = m {
                let sets: Vec<SetId> = self.picker_sets(ri).into_iter().map(|r| r.id).collect();
                for j in 0..300 {
                    if k == idx(K_MENU, j) {
                        let role = Role::ALL[ri];
                        if let Some(s) = sets.get(j).cloned() {
                            self.preview = None;
                            self.menu = None;
                            // the set it already has: nothing to write (the drawing: `if(S.cur[ro]!==st.id)pickRoleCur`)
                            let cur = self.v.cursors.as_ref().and_then(|c| c.roles.iter().find(|x| x.role == role)).map(|x| x.set.clone());
                            if cur.as_ref() != Some(&s) {
                                self.send(Cmd::CursorRole(role, s));
                            }
                        }
                        return;
                    }
                    if k == sub(idx(K_MENU, j), "x") {
                        // × on an imported set deletes it (the list stays open, without it)
                        if let Some(SetId::Pack(n)) = sets.get(j).cloned() {
                            self.send(Cmd::DeletePack(n));
                        }
                        return;
                    }
                }
                if k == idx(K_MENU, 901) {
                    self.close_menu();
                    self.open_store(cx);
                    return;
                }
                if k == idx(K_MENU, 900) {
                    self.close_menu();
                    // Windows' file picker (modal over the menu, inside this click); a test copy gets BU_PICK
                    if let Some(f) = cx.pick_file_in("Choose a cursor", &[("Cursors", "*.cur;*.ani")], &cursors_dir()) {
                        self.send(Cmd::RoleFile(Role::ALL[ri], PathBuf::from(f)));
                    }
                    return;
                }
            } else {
                for i in 0..40 {
                    if k == idx(K_MENU, i) {
                        self.close_menu();
                        self.pick(m, i, now);
                        return;
                    }
                }
            }
        }
        let fake = self.test;
        match k {
            _ if k == K_SGET => self.open_store(cx),
            _ if k == K_WEB => {
                if let Some((_, url)) = self.v.mice.as_ref().and_then(|m| m.first()).and_then(|y| y.link()) {
                    open_url(url, fake);
                }
            }
            _ if (0..self.dpi_chips().len()).any(|i| k == idx(K_DPI, i)) => {
                let chips = self.dpi_chips();
                let i = (0..chips.len()).find(|i| k == idx(K_DPI, *i)).unwrap_or(0);
                if self.mouse_dpi() != Some(chips[i]) {
                    self.send(Cmd::Dpi(chips[i]));
                }
            }
            _ if (0..4).any(|i| k == idx(K_HZ, i)) => {
                let i = (0..4).find(|i| k == idx(K_HZ, *i)).unwrap_or(0);
                self.send(Cmd::Polling(POLLING_CHIPS[i]));
            }
            _ if (0..2).any(|i| k == idx(K_LOD, i)) => {
                let i = (0..2).find(|i| k == idx(K_LOD, *i)).unwrap_or(0);
                self.send(Cmd::LiftOff(LIFT_OFF_CHIPS[i]));
            }
            _ if k == K_ACT => {
                let on = !self.panel.on;
                self.panel.on = on;
                self.panel.expanded = on;
                self.send(Cmd::AccelOn(on));
            }
            _ if k == K_ACH || k == K_ACX => {
                self.panel.expanded = !self.panel.expanded;
            }
            _ if k == K_SAVE => {
                let (id, _) = self.panel.save_as_preset();
                let name = self.panel.preset(id).map(|p| p.name.clone()).unwrap_or_default();
                self.rename = Some((id, name));
                self.push_accel();
            }
            _ if k == K_UPD => {
                if let Some(t) = self.panel.update_loaded() {
                    self.say(t, now);
                    self.push_accel();
                }
            }
            _ if k == K_MODE => self.toggle_menu(Menu::Curve),
            _ if k == K_GAIN => {
                let g = !self.vals().gain;
                self.panel.set_gain(g);
                self.push_accel();
            }
            _ if (0..3).any(|i| k == idx(K_CAP, i)) => {
                let i = (0..3).find(|i| k == idx(K_CAP, *i)).unwrap_or(1);
                self.panel.set_cap_type([CapType::Input, CapType::Output, CapType::Both][i]);
                self.push_accel();
            }
            _ if k == K_EPPOFF => self.send(Cmd::Precision(false)),
            _ if k == K_ADD => {
                let t = self.panel.loaded.or_else(|| self.panel.presets.first().map(|p| p.id)).map(Target::Preset).unwrap_or(Target::Off);
                let id = self.per_app.add_row("", t);
                self.per_app.set_row_label(id, "");
                self.menu = Some(Menu::RowApp(id));
            }
            _ if k == K_ELSE => self.toggle_menu(Menu::Else),
            _ if k == K_COPY => self.send(Cmd::CopyCurve),
            _ if k == K_OPEN => match &self.v.ra {
                // its own window (rawaccel.exe in the folder found on the Desktop / Downloads / Documents); never from a test copy
                Some(RawAccelStatus::Installed { dir: Some(d), .. }) if !fake => {
                    // Order 047: the file check and the process start (10-40 ms) off the menu's thread
                    let (exe, dir) = (d.join("rawaccel.exe"), d.to_path_buf());
                    crate::offui::spawn("mouse-rawaccel", move || {
                        if exe.is_file() {
                            let _ = std::process::Command::new(exe).current_dir(dir).spawn();
                        }
                    });
                }
                Some(RawAccelStatus::Installed { dir: None, .. }) => self.say("Raw Accel\u{2019}s folder wasn\u{2019}t found on the Desktop, in Downloads or Documents", now),
                _ => {}
            },
            _ if k == K_GIT => open_url(bu_mouse::accel::service::RAWACCEL_RELEASES, fake),
            _ if k == K_RAGET => {
                if let Err(e) = crate::addons::start_acc(cx, crate::addons::Op::Get) {
                    cx.toast(&e);
                }
            }
            _ if k == K_EPP => {
                let v = !self.v.win.map(|w| w.precision).unwrap_or(false);
                if let Some(w) = &mut self.v.win {
                    w.precision = v;
                }
                self.send(Cmd::Precision(v));
            }
            _ if k == K_SWAP => {
                let v = !self.v.win.map(|w| w.buttons_swapped).unwrap_or(false);
                if let Some(w) = &mut self.v.win {
                    w.buttons_swapped = v;
                }
                self.send(Cmd::Swap(v));
            }
            _ if k == K_DCT => {
                // the folder opens when it is double-clicked fast enough (the Windows double-click time)
                let ms = self.v.win.map(|w| w.double_click_ms).unwrap_or(500) as f64;
                if self.dct_last > 0.0 && now - self.dct_last <= ms {
                    self.dct_last = 0.0;
                    self.dct_open = !self.dct_open;
                } else {
                    self.dct_last = now;
                }
            }
            // a .cur / .ani, or a pack's install.inf = its whole folder
            _ if k == K_IMP => {
                if let Some(f) = cx.pick_file_in("Import cursors", &[("Cursors or a pack's install.inf", "*.cur;*.ani;*.inf")], &cursors_dir()) {
                    let p = PathBuf::from(f);
                    let is_inf = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("inf"));
                    let pick = if is_inf { p.parent().map(|d| d.to_path_buf()).unwrap_or(p) } else { p };
                    self.send(Cmd::Import(vec![pick]));
                }
            }
            _ if k == sub(K_RS, "pc") || k == sub(K_RS, "win") => {
                let win = k == sub(K_RS, "win");
                self.close_menu();
                match &self.test_rs {
                    // test pictures: the drawing's sample review (nothing on the PC changes)
                    Some(t) => self.reset = Some((win, vec![true; t.len()])),
                    // the frame's review over the change log, under the link (its box: the press that came before)
                    None => cx.open_reset(if win { crate::undo::Kind::WindowsDefaults } else { crate::undo::Kind::HowItWas }, self.anchor),
                }
            }
            _ if k == K_RSC || k == K_RSG => self.reset = None,
            _ => {
                for i in 0..7 {
                    if k == idx(K_ROLE, i) {
                        self.toggle_menu(Menu::Cursor(i));
                        return;
                    }
                }
                let presets: Vec<PresetId> = self.panel.presets.iter().map(|p| p.id).collect();
                for (i, id) in presets.iter().enumerate() {
                    let ck = idx(K_CHIP, i);
                    if k == ck {
                        if self.rename.is_none() {
                            if let Some(t) = self.panel.load_preset(*id) {
                                self.say(t, now);
                                self.push_accel();
                            }
                        }
                        return;
                    }
                    if k == sub(ck, "ren") {
                        let n = self.panel.preset(*id).map(|p| p.name.clone()).unwrap_or_default();
                        self.rename = Some((*id, n));
                        cx.focus(Some(sub(ck, "in")));
                        return;
                    }
                    if k == sub(ck, "del") {
                        let name = self.panel.preset(*id).map(|p| p.name.clone()).unwrap_or_default();
                        self.panel.delete_preset(*id);
                        let (apps, ee) = self.per_app.forget_preset(*id);
                        let used: Vec<String> = self.per_app.rows().iter().filter(|r| apps.iter().any(|a| r.exe.ends_with(a.as_str()))).map(|r| r.label.clone()).collect();
                        let mut parts = Vec::new();
                        if !used.is_empty() {
                            parts.push(format!("{} now Off", used.join(", ")));
                        }
                        if ee {
                            parts.push("the other games use the main preset".to_string());
                        }
                        self.say(if parts.is_empty() { format!("Deleted {name}") } else { format!("Deleted {name} · {}", parts.join(", ")) }, now);
                        self.push_accel();
                        return;
                    }
                }
                let rows: Vec<RowId> = self.per_app.rows().iter().map(|r| r.id).collect();
                for (i, id) in rows.iter().enumerate() {
                    let rk = idx(K_ROW, i);
                    if k == sub(rk, "app") {
                        self.toggle_menu(Menu::RowApp(*id));
                        return;
                    }
                    if k == sub(rk, "pre") {
                        self.toggle_menu(Menu::RowPreset(*id));
                        return;
                    }
                    if k == sub(rk, "x") {
                        self.per_app.remove_row(*id);
                        self.push_accel();
                        return;
                    }
                }
            }
        }
    }
}

/// Opens a web page in the user's browser - never from a test copy (the project rules: never a browser in tests).
fn open_url(url: &str, test: bool) {
    if test {
        return;
    }
    // Order 047: starting a process holds the thread 10-40 ms: off the menu's thread
    let u = url.to_string();
    crate::offui::spawn("mouse-url", move || {
        let _ = std::process::Command::new("explorer").arg(u).spawn();
    });
}

#[cfg(test)]
mod tests;

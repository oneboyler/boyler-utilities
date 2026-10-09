//! The Timers tab (menu-v22 page `tmr`, Order 018). The v21 design ("one clean timer with no blobby backgrounds"):
//! the header switch Stopwatch / Countdown / World clock; ONE timer on the page itself (its name, the big time, the
//! countdown's line, its buttons, then its options: On screen + Move, Sound at the end + ▶, its key); under it "Your timers"
//! (Order 078: only the ones added with "New timer", hidden while there are none; World clock: the places, added by a search
//! over an offline list of cities), each with play / pause and its own On-screen switch; the timers on the screen
//! itself (overlay.rs). The timers live in `model` (they keep running with the menu closed); this page only shows them.

mod model;
pub mod overlay;

use skia_safe as sk;
use taffy::prelude::*;
use taffy::style::{AlignItems, JustifyContent};

use crate::anim::{Bezier, EASE, EASE_OUT};
use crate::gfx::{sh, Align, Font, Gfx, Rgba};
use crate::pages::{Env, Page};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, lh, sub, Cursor, El, Key};
use crate::ui::pieces::keyfield::{self, Show};
use crate::ui::pieces::{self, btn_font, dropdown, group, link, mbtn, rowbits, search, seg, tinput};
use crate::ui::{cmix, ACC, ACC_S, CTL, CTL_H, FG, FG2, FG3, GREEN, HAIR, HOV, TRK, WHITE};
use model::{Kind, Mode};

const K_SEG: Key = key("tmr.seg");
const K_NAME: Key = key("tmr.name");
const K_TIME: Key = key("tmr.time");
const K_GO: Key = key("tmr.go");
const K_LAP: Key = key("tmr.lap");
const K_RST: Key = key("tmr.rst");
const K_SCR: Key = key("tmr.scr");
const K_MOVE: Key = key("tmr.move");
const K_SND: Key = key("tmr.snd");
const K_PV: Key = key("tmr.pv");
const K_KEY: Key = key("tmr.key");
const K_ADD: Key = key("tmr.add");
const K_MENU: Key = key("tmr.menu");
const K_ROW: Key = key("tmr.row");
const K_PLACE: Key = key("tmr.place");
/// Order 078: the search box inside the "Add a place" list (its results are `idx(K_MENU, i)`)
const K_CQ: Key = key("tmr.cq");
const K_HERO: Key = key("tmr.hero");
const LABELS: [&str; 3] = ["Stopwatch", "Countdown", "World clock"];

/// Order 078: the "Add a place" list's width
const POP_W: f32 = 300.0;

/// `cdend`: .55 s ease-in-out, 3 times
const END_MS: f64 = 550.0;
const EASE_IN_OUT: Bezier = Bezier::new(0.42, 0.0, 0.58, 1.0);

#[derive(Default)]
pub struct Timers {
    env: Env,
    shown: bool,
    /// the name being typed (`.thn` focused) and whether it is all selected (focus selects it)
    name_edit: Option<(String, bool)>,
    /// the countdown's time being typed (`.thd` focused)
    time_edit: Option<(String, bool)>,
    /// the red line under the key field: text, since
    kerr: Option<(String, f64)>,
    /// the "Add a place" list is open: the button's box (window coordinates)
    menu: Option<(f32, f32, f32, f32)>,
    /// Order 078: what is typed in that list's search box, and the result the arrow keys stand on
    cq: String,
    cq_sel: usize,
    toast: Option<(String, f64)>,
    /// the mode switch's fade-up started
    hero_at: Option<f64>,
    /// a countdown ended: which, when (`.thero.end`)
    end_at: Option<(u32, f64)>,
    /// the ▶ preview was clicked (`.pb.play` for 300 ms)
    pv_at: Option<f64>,
    /// a new row slides in: its id, when
    new_row: Option<(u32, f64)>,
    /// laps count when the newest lap row slid in
    lap_at: Option<(usize, f64)>,
    /// the world clock's UTC minute last shown (it repaints once a minute; None = not built yet)
    shown_minute: Option<u64>,
    /// Order 047: what the running timers showed at the last build / true tick (the digits of each) - a change repaints.
    /// None = no timer runs (or the world clock is shown)
    shown_sig: Option<u64>,
    /// Order 055: the same for the DATA that may only repaint at 30 Hz - the selected stopwatch's hundredths, the
    /// countdowns' line / ring steps - and when that last repainted (ms)
    shown_fine: u64,
    last_fire: f64,
}

/// Order 055: a stopwatch's hundredths and a countdown's ring are data: shown at most every 33 ms (30 Hz), whatever the
/// monitor's rate.
const FINE_MS: f64 = 33.0;

/// Order 055: what the page shows of the running timers, as two signatures: (every running timer's digits - these repaint at
/// once; the selected stopwatch's hundredths + every running countdown's ring step - these at most every `FINE_MS`).
/// (None, 0) when nothing runs or the world clock is shown: no strings are built then.
fn page_sig(m: &model::Model) -> (Option<u64>, u64) {
    use std::hash::{Hash, Hasher};
    if m.mode == Mode::Clk || !m.any_running() {
        return (None, 0);
    }
    let mut coarse = std::collections::hash_map::DefaultHasher::new();
    let mut fine = std::collections::hash_map::DefaultHasher::new();
    for t in m.timers.iter().filter(|t| t.running()) {
        t.id.hash(&mut coarse);
        t.short_text().hash(&mut coarse);
        match t.kind {
            Kind::Sw if t.id == m.sel => t.big_text().hash(&mut fine),
            Kind::Sw => {}
            Kind::Cd => ((t.share_left() * model::RING_STEPS as f32) as i32).hash(&mut fine),
        }
    }
    (Some(coarse.finish()), fine.finish())
}

/// Order 055: is a running stopwatch's hundredths on the page (the big time of the selected stopwatch)?
fn hundredths_shown(m: &model::Model) -> bool {
    m.mode != Mode::Clk && m.selected().is_some_and(|t| t.kind == Kind::Sw && t.running())
}

/// A colour with the hover mix of `.tmrw`'s transition.
fn mixc(a: Rgba, b: Rgba, t: f32) -> Rgba {
    cmix(a, b, t)
}

impl Timers {
    fn test(&self) -> bool {
        self.env.test
    }
    fn with<R>(&self, f: impl FnOnce(&mut model::Model) -> R) -> R {
        model::with(self.env.test, self.env.frozen, f)
    }
    fn show_toast(&mut self, t: String, now: f64) {
        self.toast = Some((t, now));
    }

    /// Typing done in the big time: apply it (`cdApply`).
    fn apply_time(&mut self) {
        if let Some((text, _)) = self.time_edit.take() {
            self.with(|m| {
                let id = m.sel;
                m.type_time(id, &text);
            });
        }
    }
    fn apply_name(&mut self) {
        if let Some((text, _)) = self.name_edit.take() {
            self.with(|m| {
                let id = m.sel;
                m.rename(id, &text);
            });
        }
    }

    // ------------------------------------------------------------------ the timer
    fn hero(&mut self, cx: &mut Cx) -> El {
        let (mode, sel) = self.with(|m| (m.mode, m.sel));
        let clk = mode == Mode::Clk;
        let info = self.with(|m| {
            m.selected().map(|t| {
                (t.kind, t.name.clone(), t.big_text(), t.share_left(), t.running(), t.cd.at_rest(), t.sw.button(), t.cd.button(), t.sw.lap_enabled(), t.sw.reset_enabled(), t.cd.reset_enabled(), t.screen, t.cd.sound_on, t.key.clone(), t.rgba())
            })
        });
        let world = if clk { Some(self.with(|m| m.world())) } else { None };
        // `.thero{--c:var(--acc);display:flex;flex-direction:column;align-items:center;padding:4px 0 6px}`
        let mut hero = El::col().items(AlignItems::CENTER).pad(4.0, 0.0, 6.0, 0.0);
        let Some((kind, name, big, share, running, at_rest, swb, cdb, lap_on, sw_rst, cd_rst, screen, sound, tkey, c)) = info else { return hero };
        let cd = !clk && kind == Kind::Cd;
        let sw = !clk && kind == Kind::Sw;

        // `.thn` (240 x 24): the shared piece (tinput::thn) - hover / focus / the selection when focus selects it
        let name_text = if clk { world.as_ref().map(|w| w.home_name.clone()).unwrap_or_default() } else { self.name_edit.as_ref().map(|e| e.0.clone()).unwrap_or(name.clone()) };
        // `title:'Click to rename'`
        let name_el = tinput::thn(cx, K_NAME, &name_text, self.name_edit.as_ref().is_some_and(|e| e.1), clk).title("Click to rename");

        // `.thd` (56 px digits, as wide as the hero): the shared piece (tinput::thd) - editable while a countdown is not
        // running; the end blink (`.thero.end .thd{animation:cdend .55s ease-in-out 3}`)
        let big_text = if clk {
            world.as_ref().map(|w| w.home_time.clone()).unwrap_or_default()
        } else {
            self.time_edit.as_ref().map(|e| e.0.clone()).unwrap_or(big)
        };
        let end_since = self.end_at.filter(|(id, _)| *id == sel && !clk).map(|(_, at)| at);
        let time_el = tinput::thd(cx, K_TIME, &big_text, crate::ui::WIN_W - 52.0, cd && !running, self.time_edit.as_ref().is_some_and(|e| e.1), end_since);
        hero = hero.child(name_el).child(time_el);

        if clk {
            // `.thsub{height:18px;margin-top:-2px;font-size:12.5px;line-height:18px;color:var(--fg2)}`
            let line = world.as_ref().map(|w| w.home_line.clone()).unwrap_or_default();
            hero = hero.child(El::text(line, Font::new(12.5, 400), FG2(), 18.0).h(18.0).margin(-2.0, 0.0, 0.0, 0.0));
        } else {
            // `.thpr{width:260px;height:3px;margin-top:4px;border-radius:2px;background:var(--trk);overflow:hidden}`
            // `.thpr i{border-radius:2px;background:var(--c);transform:scaleX(left/set)}` `.thero.idle .thpr i{opacity:0}` (.2 s)
            // `.thero.sw .thpr{visibility:hidden}`
            let idle = cd && at_rest;
            let iop = cx.tr(K_HERO, 1, if idle { 0.0 } else { 1.0 }, 200.0, EASE);
            let mut pr = El::block().size(260.0, 3.0).none().margin(4.0, 0.0, 0.0, 0.0);
            if !sw {
                pr = pr.radius(2.0).bg(TRK()).clip().child(El::block().h(3.0).w(260.0 * share).radius(2.0).bg(c).opacity(iop));
            }
            hero = hero.child(pr);
            // `.thb{display:flex;gap:8px;margin-top:14px}` `.thb .tbn:first-child{min-width:96px}` (cd: no Lap)
            let (go_icon, go_label) = match kind {
                Kind::Sw => (if running { "tpause" } else { "tplay" }, match swb {
                    bu_timers::stopwatch::SwButton::Start => "Start",
                    bu_timers::stopwatch::SwButton::Stop => "Stop",
                    bu_timers::stopwatch::SwButton::Resume => "Resume",
                }),
                Kind::Cd => (if running { "tpause" } else { "tplay" }, match cdb {
                    bu_timers::countdown::CdButton::Start => "Start",
                    bu_timers::countdown::CdButton::Pause => "Pause",
                    bu_timers::countdown::CdButton::Resume => "Resume",
                    bu_timers::countdown::CdButton::Again => "Again",
                }),
            };
            let mut thb = El::row().gap(8.0).margin(14.0, 0.0, 0.0, 0.0).child(tbn(cx, K_GO, go_icon, go_label, !running, false, 96.0));
            if sw {
                thb = thb.child(tbn(cx, K_LAP, "tflag", "Lap", false, !lap_on, 0.0));
            }
            thb = thb.child(tbn(cx, K_RST, "treset", "Reset", false, !(if sw { sw_rst } else { cd_rst }), 0.0));
            hero = hero.child(thb);
            // `.tho{display:flex;align-items:center;justify-content:center;gap:18px;height:26px;margin-top:12px;font-size:12px}`
            // `.thsc,.thsn{display:flex;align-items:center;gap:6px}` `.thero.sw .thsn{display:none}` `.thmv.off{display:none}`
            let moving = self.with(|m| m.moving);
            let mut thsc = El::row().center().gap(6.0).child(rowbits::wps(cx, K_SCR, "On screen", screen, rowbits::WpsAt::Timers));
            if screen {
                thsc = thsc.child(link::link(cx, K_MOVE, if moving { "Done" } else { "Move" }, 12.0));
            }
            let mut tho = El::row().center().justify(JustifyContent::CENTER).gap(18.0).h(26.0).margin(12.0, 0.0, 0.0, 0.0).child(thsc);
            if cd {
                tho = tho.child(El::row().center().gap(6.0).child(rowbits::wps(cx, K_SND, "Sound at the end", sound, rowbits::WpsAt::Timers)).child(rowbits::pb(cx, K_PV, self.pv_at, 1, !sound)));
            }
            // `.shk.thk{gap:7px}` with the inline key field ("Bind" until a key is set) - the keys manager's field for this
            // timer's action
            let _ = tkey;
            ensure_action(sel, &name);
            let (set, listening, err) = cx.key_field(&key_action(sel));
            let show = match (&listening, &set) {
                (Some((held, _)), _) => Show::Listening(held.as_deref()),
                (None, Some(k)) => Show::Set(k.as_str()),
                (None, None) => Show::Empty,
            };
            let since = listening.as_ref().map(|l| l.1).unwrap_or(0.0);
            // `hKeyBox=h('div',{class:'shk thk',title:'This timer’s key'})`
            tho = tho.child(
                El::row().center().gap(7.0).none().key(sub(K_KEY, "box")).title("This timer\u{2019}s key").child(keyfield::keyfield(cx, K_KEY, show, since, true)),
            );
            hero = hero.child(tho);
            // the manager's refusal ("Already used by Mic mute") as the red line under the field, faded in once
            match (err, &self.kerr) {
                (Some(e), Some((old, _))) if *old == e => {}
                (Some(e), _) => self.kerr = Some((e, cx.now)),
                (None, _) => self.kerr = None,
            }
            if let Some((t, at)) = self.kerr.clone() {
                hero = hero.child(kerr(cx, K_KEY, &t, at));
            }
            if sw {
                // `.thero .tlps{width:280px;height:auto;margin-top:8px}` + `.tlp` rows (newest 3; the best lap green)
                let laps = self.with(|m| m.laps(sel));
                let n_all = self.with(|m| m.get(sel).map(|t| t.sw.laps().len()).unwrap_or(0));
                let mut tl = El::col().w(280.0).margin(8.0, 0.0, 0.0, 0.0);
                for (i, l) in laps.iter().enumerate() {
                    let mut r = lap_row(&l.label, &l.lap_text, &l.total_text, l.best);
                    if i == 0 {
                        if let Some((n, at)) = self.lap_at {
                            if n == n_all {
                                let t = ((cx.now - at) / 220.0).clamp(0.0, 1.0);
                                if t < 1.0 {
                                    cx.st.busy = true;
                                }
                                let e = EASE_OUT.ease(t) as f32;
                                r = r.opacity(e).translate(0.0, -4.0 * (1.0 - e));
                            }
                        }
                    }
                    tl = tl.child(r);
                }
                hero = hero.child(tl);
            }
        }
        // the switch's fade-up: opacity 0 -> 1, translateY 4px -> 0, 240 ms EASE_OUT
        if let Some(at) = self.hero_at {
            let t = if cx.rm { 1.0 } else { ((cx.now - at) / 240.0).clamp(0.0, 1.0) };
            if t < 1.0 {
                cx.st.busy = true;
                let e = EASE_OUT.ease(t) as f32;
                hero = hero.opacity(e).translate(0.0, 4.0 * (1.0 - e));
            }
        }
        hero
    }

    // ------------------------------------------------------------------ the list
    fn list(&mut self, cx: &mut Cx) -> El {
        let mode = self.with(|m| m.mode);
        let clk = mode == Mode::Clk;
        let mut rows = Vec::new();
        let count;
        if clk {
            let w = self.with(|m| m.world());
            let places: Vec<model::PlaceItem> = self.with(|m| m.places.clone());
            count = places.len();
            for (i, (p, (city, time, line))) in places.iter().zip(w.places).enumerate() {
                rows.push(place_row(cx, i, i == 0, &city, &time, &line, p.screen));
            }
            if rows.is_empty() {
                // `.thnone{padding:13px 12px;font-size:12px;color:var(--fg2)}`
                rows.push(El::text("No places yet.", Font::new(12.0, 400), FG2(), lh(12.0, 1.35)).pad(13.0, 12.0, 13.0, 12.0));
            }
        } else {
            // Order 078: only the timers that were added; the big Stopwatch / Countdown on top is the tab's own
            let items = self.with(|m| {
                m.listed()
                    .map(|t| (t.id, t.kind, t.name.clone(), t.kind_text(), t.short_text(), t.share_left(), t.running(), t.screen, t.rgba(), t.id == m.sel))
                    .collect::<Vec<_>>()
            });
            count = items.len();
            if count == 0 {
                // nothing added: no "Your timers" title, no empty box - just the button to add one
                let add = mbtn::mbtn(cx, K_ADD, mbtn::Mb::Icon("plus12", "New timer"), false);
                return El::block().child(El::row().center().margin(20.0, 12.0, 7.0, 12.0).child(El::row().center().ml_auto().child(add)));
            }
            for (i, it) in items.into_iter().enumerate() {
                let tid = it.0;
                let mut r = timer_row(cx, i == 0, it);
                if let Some((id, at)) = self.new_row {
                    if id == tid {
                        let t = ((cx.now - at) / 260.0).clamp(0.0, 1.0);
                        if t < 1.0 {
                            cx.st.busy = true;
                            let e = EASE_OUT.ease(t) as f32;
                            r = r.opacity(e).translate(0.0, -6.0 * (1.0 - e));
                        }
                    }
                }
                rows.push(r);
            }
        }
        // `.gh` + `.ghs{font-weight:400;color:var(--fg3)}` + `.gh .ghr{margin-left:auto;display:flex;align-items:center;gap:10px}`
        let gh = El::row()
            .center()
            .gap(6.0)
            .margin(20.0, 12.0, 7.0, 12.0)
            .child(El::text(if clk { "Places" } else { "Your timers" }, Font::new(11.0, 500), FG2(), lh(11.0, 1.35)))
            .child(El::text(count.to_string(), Font::new(11.0, 400), FG3(), lh(11.0, 1.35)))
            .child(El::row().center().gap(10.0).ml_auto().child(mbtn::mbtn(cx, K_ADD, mbtn::Mb::Icon("plus12", if clk { "Add a place" } else { "New timer" }), false)));
        // `.grp.tml{overflow:hidden}`
        El::block().child(gh).child(group::grp(rows).clip())
    }
}


/// `#sw .tbn{display:inline-flex;align-items:center;justify-content:center;gap:5px;height:28px;padding:0 11px 0 9px;border-radius:7px;
///   background:var(--ctl);box-shadow:inset 0 0 0 .5px var(--hair);font-size:12.5px;transition:background-color .15s ease,
///   opacity .2s ease,transform .12s ease,filter .15s ease}` `:hover{background:var(--ctl-h)}` `:active{transform:scale(.97)}`
/// `:disabled{opacity:.38}` `.tbn svg{width:12px;height:12px;stroke-width:1.6}` `#sw .tbn.acc{background:var(--acc);color:#fff;
///   font-weight:600;box-shadow:inset 0 0 0 .5px rgba(255,255,255,.2),0 1px 3px rgba(0,0,0,.22)}` `.acc:hover{filter:brightness(1.08)}`
fn tbn(cx: &mut Cx, k: Key, icon: &str, label: &str, acc: bool, disabled: bool, min_w: f32) -> El {
    let hv = if disabled { 0.0 } else { cx.hover_t(k, 150.0, EASE) };
    let pr = if disabled { 0.0 } else { cx.active_t(k, 120.0, EASE) };
    let op = cx.tr(k, 3, if disabled { 0.38 } else { 1.0 }, 200.0, EASE);
    let (bg, fg) = if acc {
        let b = 1.0 + 0.08 * hv;
        (Rgba((ACC().0 * b).min(1.0), (ACC().1 * b).min(1.0), (ACC().2 * b).min(1.0), 1.0), WHITE)
    } else {
        (cmix(CTL(), CTL_H(), hv), FG())
    };
    let mut b = El::row()
        .center()
        .justify(JustifyContent::CENTER)
        .gap(5.0)
        .h(28.0)
        .min_w(min_w)
        .none()
        .pad(0.0, 11.0, 0.0, 9.0)
        .radius(7.0)
        .bg(bg)
        .scale(1.0 - 0.03 * pr)
        .opacity(op)
        .child(icon_f(icon, 12.0, 1.6, fg).no_hit())
        .child(El::text(label, btn_font(12.5, if acc { 600 } else { 400 }), fg, lh(12.5, 1.35)).no_hit());
    b = if acc {
        b.inset(&[sh(0.0, 0.0, 0.0, 0.5, Rgba(1.0, 1.0, 1.0, 0.2))]).shadow(&[sh(0.0, 1.0, 3.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.22))])
    } else {
        b.inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
    };
    if disabled {
        b.key(k)
    } else {
        b.on_click(k).cursor(Cursor::Hand)
    }
}

/// `.tlp{display:grid;grid-template-columns:52px minmax(0,1fr) auto;align-items:center;height:20px;font-size:11.5px;color:var(--fg2);
///   font-variant-numeric:tabular-nums}` `.tlp b{font-weight:600;color:var(--fg)}` `.tlp.best b{color:var(--green)}`
fn lap_row(label: &str, lap: &str, total: &str, best: bool) -> El {
    let f = Font::new(11.5, 400).tnum();
    El::grid()
        .h(20.0)
        .items(AlignItems::CENTER)
        .style(|s| s.grid_template_columns = vec![length(52.0), minmax(length(0.0), fr(1.0)), auto()])
        .child(El::text(label, f, FG2(), lh(11.5, 1.35)))
        .child(El::text(lap, Font::new(11.5, 600).tnum(), if best { GREEN() } else { FG() }, lh(11.5, 1.35)))
        .child(El::text(total, f, FG2(), lh(11.5, 1.35)))
}

/// The countdown ring: `.tmr{width:24px;height:24px;border-radius:50%;background:conic-gradient(var(--c) calc(var(--p)*1%),
///   rgba(255,255,255,.13) 0);mask:radial-gradient(circle,transparent 8.4px,#000 9px)}`
fn ring(c: Rgba, share: f32) -> El {
    El::paint(move |g: &Gfx, (x, y, w, h)| {
        let (sx, sy, sw, shh) = g.snap(x, y, w, h);
        let (cx_, cy_) = (sx + sw / 2.0, sy + shh / 2.0);
        let rest = Rgba(1.0, 1.0, 1.0, 0.13);
        let p = share.clamp(0.0, 1.0);
        let cols = [c.c4(), c.c4(), rest.c4(), rest.c4()];
        let pos = [0.0, p, p, 1.0];
        // CSS conic gradients start at 12 o'clock; Skia's sweep at 3 o'clock
        let m = sk::Matrix::rotate_deg_pivot(-90.0, (cx_, cy_));
        let interp = sk::gradient::Interpolation {
            in_premul: sk::gradient::interpolation::InPremul::Yes,
            color_space: sk::gradient::interpolation::ColorSpace::Destination,
            hue_method: sk::gradient::interpolation::HueMethod::Shorter,
        };
        let srgb = Some(sk::ColorSpace::new_srgb());
        let Some(cone) = sk::gradient_shader::sweep_with_interpolation((cx_, cy_), (&cols[..], srgb.clone()), &pos[..], sk::TileMode::Clamp, None, interp, Some(&m)) else { return };
        // the mask: transparent up to 8.4 px from the centre, opaque from 9 px (a circle sized to the farthest corner)
        let far = (sw * sw + shh * shh).sqrt() / 2.0;
        let clear = Rgba(0.0, 0.0, 0.0, 0.0).c4();
        let black = Rgba(0.0, 0.0, 0.0, 1.0).c4();
        let mc = [clear, clear, black, black];
        let mp = [0.0, 8.4 / far, 9.0 / far, 1.0];
        let Some(mask) = sk::gradient_shader::radial_with_interpolation(((cx_, cy_), far), (&mc[..], srgb), &mp[..], sk::TileMode::Clamp, interp, None) else { return };
        let shd = sk::shaders::blend(sk::BlendMode::DstIn, cone, mask);
        g.fill_rr_shader(x, y, w, h, w.min(h) / 2.0, &shd, 1.0);
    })
    .size(24.0, 24.0)
    .none()
    .no_hit()
}

/// `#sw .tmpb,#sw .tmsb{width:28px;height:28px;border-radius:50%;background:var(--ctl);color:var(--fg2);box-shadow:inset 0 0 0 .5px
///   var(--hair);transition:background-color .12s ease,color .12s ease,transform .12s ease}` `:hover{background:var(--ctl-h);color:var(--fg)}`
/// `:active{transform:scale(.92)}` `.tmpb svg{12px;stroke-width:1.6}` `.tmsb svg{15px;stroke-width:1.3}` `#sw .tmpb.on{background:var(--c);
///   color:#fff;box-shadow:inset 0 0 0 .5px rgba(255,255,255,.2)}` `#sw .tmsb.on{background:var(--acc);color:#fff;...}`
fn round_btn(cx: &mut Cx, k: Key, icon: &str, on: bool, on_bg: Rgba) -> El {
    let hv = cx.hover_t(k, 120.0, EASE);
    let pr = cx.active_t(k, 120.0, EASE);
    let onv = cx.tr(k, 3, if on { 1.0 } else { 0.0 }, 120.0, EASE);
    let (size, stroke) = if icon == "tscr" { (15.0, 1.3) } else { (12.0, 1.6) };
    let fg = cmix(cmix(FG2(), FG(), hv), WHITE, onv);
    El::block()
        .size(28.0, 28.0)
        .none()
        .radius(14.0)
        .bg(cmix(cmix(CTL(), CTL_H(), hv), on_bg, onv))
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, cmix(HAIR(), Rgba(1.0, 1.0, 1.0, 0.2), onv))])
        .place_center()
        .scale(1.0 - 0.08 * pr)
        .on_click(k)
        .cursor(Cursor::Hand)
        .child(icon_f(icon, size, stroke, fg).no_hit())
}

/// `.tmtm{flex:none;min-width:66px;text-align:right;font:600 15px/1 "Segoe UI Variable Display";font-variant-numeric:tabular-nums;
///   color:var(--fg2)}` `.tmrw.run .tmtm,.tmrw.clr .tmtm{color:var(--fg)}`
fn tmtm(t: &str, bright: bool) -> El {
    El::text(t, Font::display(15.0, 600).ls(-78).tnum(), if bright { FG() } else { FG2() }, 15.0).align(Align::Right).min_w(66.0).none()
}

/// `.row.tmrw{gap:11px;min-height:50px;cursor:pointer;transition:background-color .12s ease}` `:hover{background:var(--hov)}`
/// `.tmrw.sel{background:rgba(10,132,255,.12)}`
fn row_box(cx: &mut Cx, k: Key, first: bool, sel: bool, kids: Vec<El>) -> El {
    let hv = cx.hover_t(k, 120.0, EASE);
    let sv = cx.tr(k, 4, if sel { 1.0 } else { 0.0 }, 120.0, EASE);
    let bg = mixc(HOV().mul_a(hv), Rgba::rgba(10, 132, 255, 0.12), sv);
    group::row(first, kids).gap(11.0).min_h(50.0).bg(bg).on_click(k).cursor(Cursor::Hand)
}

/// The row's On-screen button name: `t.screen?'On screen · click to hide':'Show on screen'`.
fn screen_title(on: bool) -> &'static str {
    if on {
        "On screen \u{b7} click to hide"
    } else {
        "Show on screen"
    }
}

type Item = (u32, Kind, String, String, String, f32, bool, bool, Rgba, bool);

fn timer_row(cx: &mut Cx, first: bool, (id, kind, name, kind_text, time, share, run, screen, c, sel): Item) -> El {
    let k = idx(K_ROW, id as usize);
    let hovered = cx.hovered(k);
    let glyph = match kind {
        // `.tmr.sw{background:none;mask:none;color:var(--c)}` + the stopwatch glyph (20 px, stroke 1.4)
        Kind::Sw => El::block().size(24.0, 24.0).none().place_center().no_hit().child(El::icon("tsw", 20.0, 1.4, c)),
        Kind::Cd => ring(c, share),
    };
    let kids = vec![
        glyph,
        group::lbl(&name, Some(&kind_text)),
        tmtm(&time, run),
        // `t.pb.title=(t.run?'Pause ':'Start ')+t.n` · `t.sb.title=t.screen?'On screen · click to hide':'Show on screen'` · `title:'Remove'`
        round_btn(cx, sub(k, "pb"), if run { "tpause" } else { "tplay" }, run, c).title(&format!("{}{name}", if run { "Pause " } else { "Start " })),
        round_btn(cx, sub(k, "sb"), "tscr", screen, ACC()).title(screen_title(screen)),
        rowbits::rdel(cx, sub(k, "del"), hovered).title("Remove"),
    ];
    row_box(cx, k, first, sel, kids)
}

fn place_row(cx: &mut Cx, i: usize, first: bool, city: &str, time: &str, line: &str, screen: bool) -> El {
    let k = idx(K_PLACE, i);
    let hovered = cx.hovered(k);
    let kids = vec![
        // `.tmrw.clr .tmr.sw{color:var(--fg2)}` + the clock glyph
        El::block().size(24.0, 24.0).none().place_center().no_hit().child(El::icon("tclk", 20.0, 1.4, FG2())),
        group::lbl(city, Some(line)),
        tmtm(time, true),
        round_btn(cx, sub(k, "sb"), "tscr", screen, ACC()).title(screen_title(screen)),
        rowbits::rdel(cx, sub(k, "del"), hovered).title("Remove"),
    ];
    // place rows are not picked (no click of their own) but keep the row hover
    let hv = cx.hover_t(k, 120.0, EASE);
    group::row(first, kids).gap(11.0).min_h(50.0).bg(HOV().mul_a(hv)).key(k).cursor(Cursor::Hand)
}

/// A timer's own key = a keys-manager action (it starts / pauses that timer from anywhere, menu open or closed; the manager
/// refuses a key another feature uses and registers it with Windows).
fn key_action(id: u32) -> String {
    format!("tmr.k.{id}")
}

/// Make sure the timer `id` has its action (added when the timer is first shown; not inside a key handler).
fn ensure_action(id: u32, name: &str) {
    let aid = key_action(id);
    crate::services::try_with(|s| {
        if !s.has_action(&aid) {
            s.add_action(crate::keys::Action::new(&aid, name, "tmr"), move |down| {
                if down {
                    model::with_existing(|m| m.toggle(id));
                    overlay::sync();
                }
            });
        }
    });
}

/// The timer went: its key with it.
fn drop_action(id: u32) {
    crate::services::try_with(|s| s.remove_action(&key_action(id)));
}

impl Page for Timers {
    fn id(&self) -> &'static str {
        "tmr"
    }
    fn name(&self) -> &'static str {
        "Timers"
    }
    fn icon(&self) -> &'static str {
        "swatch"
    }
    /// At app start: the timers of the last run are gone (they live in memory), so are their keys - a saved
    /// `tmr.k.<id>` would otherwise land on a new timer with the same number.
    fn start(&self, s: &mut crate::services::Services) {
        let scope = crate::settings::Scope::App;
        if let Some(list) = s.store.get_list(scope, crate::keys::SETTING).map(|l| l.to_vec()) {
            let keep: Vec<String> = list.iter().filter(|l| !l.starts_with("tmr.k.")).cloned().collect();
            if keep.len() != list.len() {
                let _ = s.store.set_list(scope, crate::keys::SETTING, &keep);
                s.keys.reload(&s.store);
            }
        }
    }
    fn open(&mut self, env: &Env, _now: f64) {
        self.env = env.clone();
        self.shown = true;
        // the model is made on first use (nothing slow: no scan, no wait on Windows)
        self.with(|m| {
            m.check();
        });
        overlay::set_preview(true);
        // test copies only (pictures of open states): BU_TMR_CLICKS = "tmr.seg#1;tmr.add;tmr.row#2/sb" replayed as clicks
        if let Some(list) = crate::testmode::env("BU_TMR_CLICKS") {
            self.replay(&list, _now);
        }
    }
    fn close(&mut self) {
        if self.shown {
            self.apply_name();
            self.apply_time();
            self.with(|m| m.moving = false);
        }
        self.shown = false;
        self.name_edit = None;
        self.time_edit = None;
        crate::services::try_with(|s| {
            if s.listening.as_ref().is_some_and(|l| l.action.starts_with("tmr.k.")) {
                s.stop_listening();
            }
        });
        self.kerr = None;
        self.menu = None;
        self.cq.clear();
        self.toast = None;
        self.hero_at = None;
        self.end_at = None;
        self.pv_at = None;
        self.new_row = None;
        self.lap_at = None;
        self.shown_minute = None;
        overlay::set_preview(false);
    }
    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        let on = self.with(|m| match m.mode {
            Mode::Sw => 0,
            Mode::Cd => 1,
            Mode::Clk => 2,
        });
        let header = pieces::header(self.name(), Some(seg::seg(cx, K_SEG, &LABELS, on, true)));
        let hero = self.hero(cx);
        let list = self.list(cx);
        // Order 055: what this build shows is what `tick` compares against (no extra rebuild right after a hover's build)
        let (minute, (coarse, fine)) = self.with(|m| (m.utc_minute(), page_sig(m)));
        self.shown_minute = Some(minute);
        self.shown_sig = coarse;
        self.shown_fine = fine;
        self.last_fire = cx.now;
        // a toast (a countdown ended, a refusal): the frame's own (frosted, above everything, not blocking the page)
        if let Some((t, _)) = self.toast.take() {
            cx.toast(&t);
        }
        vec![header, hero, list]
    }
    fn tick(&mut self, now: f64) -> bool {
        // (a countdown that reached zero finishes here; with nothing running this is a few comparisons)
        let (ended, (coarse, fine), minute, clock) = self.with(|m| {
            m.check();
            (std::mem::take(&mut m.ended), page_sig(m), m.utc_minute(), m.mode == Mode::Clk)
        });
        // Order 047 / 055: a running timer repaints when a digit changed (at once), and its hundredths / ring step at most
        // every 33 ms - never every frame of a 360 Hz screen. `wake_at` asks again exactly when that can next happen, so
        // the digits keep moving with a still mouse and an unfocused menu. (`page_sig` builds nothing when no timer runs.)
        let mut dirty = coarse != self.shown_sig;
        if !dirty && fine != self.shown_fine && now - self.last_fire >= FINE_MS - 1.0 {
            dirty = true;
        }
        if dirty {
            self.shown_sig = coarse;
            self.shown_fine = fine;
            self.last_fire = now;
        }
        if let Some((id, text)) = ended.into_iter().last() {
            self.show_toast(text, now);
            self.end_at = Some((id, now));
            dirty = true;
            overlay::sync();
        }
        // the world clock repaints once a minute (the minute number, no strings)
        if clock && self.shown_minute != Some(minute) {
            dirty = true;
        }
        dirty
    }
    /// (Order 047: not opened ahead on hover - opening shows the bars preview on the screen)
    fn preopen(&self) -> bool {
        false
    }
    fn wake_at(&self, now: f64) -> Option<f64> {
        // Order 055: exactly when the page will look different - the next whole second of a running timer, a stopwatch's
        // hundredths 33 ms after the last repaint (30 Hz), a countdown's ring step (never faster than 33 ms), the world
        // clock's next minute. Nothing running and no clock = None (no frames, no wake-ups). 1 ms past the edge so the
        // tick never lands just before it.
        let (running, clock, hundredths, (digits, ring), minute, end) =
            self.with(|m| (m.any_running(), m.mode == Mode::Clk, hundredths_shown(m), m.page_next_change(), m.to_next_minute(), m.next_end()));
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0 + 1.0;
        let mut at: Option<f64> = None;
        let mut take = |t: f64| at = Some(at.map_or(t, |a| a.min(t)));
        if clock {
            take(now + ms(minute));
            // a countdown that ends meanwhile (its toast)
            if let Some(e) = end {
                take(now + ms(e));
            }
        } else if running {
            if let Some(d) = digits {
                take(now + ms(d));
            }
            if let Some(r) = ring {
                take(now + ms(r).max(FINE_MS));
            }
            if hundredths {
                take((self.last_fire + FINE_MS).max(now + 1.0));
            }
        }
        at
    }
    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        let now = cx.now;
        let sel = self.with(|m| m.sel);
        match ev {
            Ev::Press(k, ..) if *k == K_NAME => {
                if self.name_edit.is_none() && self.with(|m| m.mode) != Mode::Clk {
                    let n = self.with(|m| m.selected().map(|t| t.name.clone()).unwrap_or_default());
                    self.name_edit = Some((n, true));
                }
            }
            Ev::Press(k, ..) if *k == K_TIME => {
                let ok = self.with(|m| m.mode != Mode::Clk && m.selected().map(|t| t.kind == Kind::Cd && !t.cd.is_running()).unwrap_or(false));
                if ok {
                    if self.time_edit.is_none() {
                        let t = self.with(|m| m.selected().map(|t| t.cd.text()).unwrap_or_default());
                        self.time_edit = Some((t, true));
                    }
                } else {
                    cx.focus(None);
                }
            }
            Ev::Press(k, x, y, b) if *k == K_ADD => {
                let _ = (x, y);
                // (only the World clock's button opens a list; in Stopwatch / Countdown it adds a timer on the click)
                if self.with(|m| m.mode) == Mode::Clk {
                    self.menu = if self.menu.is_some() { None } else { Some(*b) };
                    // it opens with the search box ready to type in
                    self.cq.clear();
                    self.cq_sel = 0;
                    if self.menu.is_some() {
                        cx.focus(Some(K_CQ));
                    }
                }
            }
            // (the list is gone - Esc closed it - but the field kept the focus: it hears nothing and lets go)
            Ev::Char(k, _) | Ev::Key(k, _) if *k == K_CQ && self.menu.is_none() => cx.focus(None),
            Ev::Char(k, c) if *k == K_CQ => {
                if self.cq.chars().count() < 40 {
                    search::edit_char(&mut self.cq, *c);
                    self.cq_sel = 0;
                }
            }
            Ev::Key(k, vk) if *k == K_CQ => match *vk {
                0x0D => {
                    // the row under the pointer is the highlighted one; else the arrowed one
                    let at = (0..model::FOUND_MAX).find(|&i| cx.hovered(idx(K_MENU, i))).unwrap_or(self.cq_sel);
                    if self.pick_found(at) {
                        cx.focus(None);
                    }
                }
                0x26 => self.cq_sel = self.cq_sel.saturating_sub(1),
                0x28 => {
                    let q = self.cq.clone();
                    let n = self.with(|m| m.search_places(&q).len());
                    self.cq_sel = (self.cq_sel + 1).min(n.saturating_sub(1));
                }
                0x08 => {
                    self.cq.pop();
                    self.cq_sel = 0;
                }
                _ => {}
            },
            Ev::Click(k) if *k == sub(K_CQ, "x") => {
                self.cq.clear();
                self.cq_sel = 0;
                cx.focus(Some(K_CQ));
            }
            Ev::Char(k, c) if *k == K_NAME => {
                if let Some((s, all)) = self.name_edit.as_mut() {
                    if *all {
                        s.clear();
                        *all = false;
                    }
                    if !c.is_control() && s.chars().count() < model::NAME_MAX {
                        s.push(*c);
                    }
                }
            }
            Ev::Char(k, c) if *k == K_TIME => {
                if let Some((s, all)) = self.time_edit.as_mut() {
                    if *all {
                        s.clear();
                        *all = false;
                    }
                    if !c.is_control() {
                        s.push(*c);
                    }
                }
            }
            Ev::Key(k, vk) if *k == K_NAME => match *vk {
                0x0D => {
                    self.apply_name();
                    cx.focus(None);
                }
                0x1B => self.name_edit = None,
                0x08 => {
                    if let Some((s, all)) = self.name_edit.as_mut() {
                        if *all {
                            s.clear();
                            *all = false;
                        } else {
                            s.pop();
                        }
                    }
                }
                _ => {}
            },
            Ev::Key(k, vk) if *k == K_TIME => match *vk {
                0x0D => {
                    // Enter: apply the typed time and start
                    self.apply_time();
                    cx.focus(None);
                    let start = self.with(|m| m.selected().map(|t| !t.cd.is_running()).unwrap_or(false));
                    if start {
                        self.with(|m| m.toggle(sel));
                        overlay::sync();
                    }
                }
                0x1B => self.time_edit = None,
                0x08 => {
                    if let Some((s, all)) = self.time_edit.as_mut() {
                        if *all {
                            s.clear();
                            *all = false;
                        } else {
                            s.pop();
                        }
                    }
                }
                _ => {}
            },
            Ev::Blur(k) if *k == K_NAME => self.apply_name(),
            Ev::Blur(k) if *k == K_TIME => self.apply_time(),
            // the key field: the keys manager listens (it takes the key messages itself, Esc cancels)
            Ev::Click(k) if *k == K_KEY => {
                let aid = key_action(sel);
                if cx.key_field(&aid).1.is_none() {
                    cx.listen_key(&aid);
                }
            }
            // the field lost the focus (a click elsewhere): it stops listening
            Ev::Blur(k) if *k == K_KEY => cx.stop_listening(),
            Ev::Click(k) if *k == sub(K_KEY, "clr") => {
                let aid = key_action(sel);
                cx.stop_listening();
                cx.clear_key(&aid);
                self.kerr = None;
            }
            Ev::Click(k) => self.click(*k, now),
            _ => {}
        }
        // names, keys, switches: the screen follows (nothing in test copies)
        overlay::sync();
    }
    /// Order 078: "Add a place" = a search box over every city of the offline list (type 2+ letters): each result shows its
    /// country and the time there now; a click (or Enter on the first / the arrowed one) adds it.
    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        let (bx, by, bw, bh) = self.menu?;
        let found = self.with(|m| m.search_places(&self.cq));
        self.cq_sel = self.cq_sel.min(found.len().saturating_sub(1));
        // the box: 300 px wide, 5 px padding; the search field fills it
        let inner = POP_W - 10.0;
        let mut kids = vec![search::search(cx, K_CQ, &self.cq, "Search any city or country", false).w(inner)];
        let mut h = 10.0 + 28.0;
        if found.is_empty() {
            // `.mhead`-like hint line
            let typed = self.cq.chars().filter(|c| c.is_alphanumeric()).count() >= 2;
            let text = if typed { "No place found" } else { "Type a city, a town or a country" };
            kids.push(El::text(text, Font::new(12.0, 400), FG3(), lh(12.0, 1.35)).pad(8.0, 6.0, 6.0, 6.0));
            h += 8.0 + 16.0 + 6.0;
        } else {
            let any_hover = (0..found.len()).any(|i| cx.hovered(idx(K_MENU, i)));
            for (i, f) in found.iter().enumerate() {
                let k = idx(K_MENU, i);
                let on = if any_hover { cx.hovered(k) } else { i == self.cq_sel };
                let (c1, c2) = if on { (WHITE, WHITE) } else { (FG(), FG2()) };
                let day = match f.day {
                    d if d > 0 => " · tomorrow",
                    d if d < 0 => " · yesterday",
                    _ => "",
                };
                let mut r = El::row()
                    .center()
                    .gap(8.0)
                    .h(28.0)
                    .pad(0.0, 10.0, 0.0, 10.0)
                    .radius(5.0)
                    .child(El::text(f.place.city, Font::new(13.0, 400), c1, lh(13.0, 1.35)).ellipsis().flex1_auto())
                    .child(El::text(f.place.land, Font::new(12.0, 400), c2, lh(12.0, 1.35)).none())
                    .child(El::text(format!("{}{day}", f.time), Font::new(12.5, 600).tnum(), c1, lh(12.5, 1.35)).align(Align::Right).min_w(44.0).none())
                    .on_click(k)
                    .cursor(Cursor::Hand);
                if on {
                    r = r.bg(ACC());
                }
                kids.push(r);
            }
            h += 4.0 + 28.0 * found.len() as f32;
        }
        // under the button, its left edge on the button's; too wide -> its right edge on the button's
        let mut left = bx;
        let mut top = by + bh + 4.0;
        if left + POP_W > crate::ui::WIN_W - 8.0 {
            left = 8f32.max(bx + bw - POP_W);
        }
        if top + h > crate::ui::WIN_H - 8.0 {
            top = 8f32.max(by - h - 4.0);
        }
        // a click beside the list closes it (the frame's popup layer takes every click while a popup is open)
        Some(dropdown::menu_box(cx, K_MENU, left.round(), top.round(), POP_W, h, 300.0, kids))
    }
    fn popup_dismiss(&mut self) {
        self.menu = None;
        self.cq.clear();
    }
    fn describe(&self) -> String {
        model::with_existing(|m| {
            let sel = m.selected().map(|t| format!("{}:{}:{}", t.name, t.big_text(), if t.running() { "run" } else { "stop" })).unwrap_or_default();
            format!(
                "mode={:?} sel={} timers={} places={} pills={} window={} timer={} menu={}",
                m.mode,
                sel,
                m.timers.iter().map(|t| format!("{}{}", t.name, if t.screen { "*" } else { "" })).collect::<Vec<_>>().join(","),
                m.places.iter().map(|p| p.place.city).collect::<Vec<_>>().join(","),
                m.pills(self.shown).len(),
                overlay::window_exists(),
                overlay::timer_armed(),
                self.menu.is_some()
            )
        })
        .unwrap_or_default()
    }
}

impl Timers {
    /// Test copies: press + click named elements through `event`, each at its laid-out box (the test hook's click does not
    /// reach pages yet - the frame drops it). Names: "tmr.add", "tmr.seg#1" (= idx), "tmr.row#2/sb" (= sub of idx).
    fn replay(&mut self, list: &str, now: f64) {
        let g = Gfx::new(1.0);
        for name in list.split(';').filter(|n| !n.is_empty()) {
            let (base, part) = name.split_once('/').unwrap_or((name, ""));
            let mut k = match base.split_once('#') {
                Some((b, i)) => idx(key(b), i.parse().unwrap_or(0)),
                None => key(base),
            };
            if !part.is_empty() {
                k = sub(k, part);
            }
            let mut st = crate::ui::cx::State::default();
            let mut cx = Cx::new(now, true, &g, &mut st);
            let kids = self.build(&mut cx);
            let root = El::block().w(crate::ui::WIN_W).pad(2.0, 26.0, 18.0, 26.0).children(kids);
            let laid = crate::ui::lay::Laid::new(&g, root, crate::ui::WIN_W, None);
            let r = laid.rect_of(k).map(|(x, y, w, h)| (x, y + crate::ui::PAGE_TOP, w, h)).unwrap_or((0.0, 0.0, 0.0, 0.0));
            self.event(&Ev::Press(k, r.0 + r.2 / 2.0, r.1 + r.3 / 2.0, r), &mut cx);
            self.event(&Ev::Click(k), &mut cx);
        }
    }

    fn click(&mut self, k: Key, now: f64) {
        let sel = self.with(|m| m.sel);
        if let Some(i) = (0..LABELS.len()).find(|&i| idx(K_SEG, i) == k) {
            let mode = [Mode::Sw, Mode::Cd, Mode::Clk][i];
            self.apply_name();
            self.apply_time();
            if self.with(|m| m.set_mode(mode)) {
                self.hero_at = Some(now);
                crate::services::try_with(|s| {
                    if s.listening.as_ref().is_some_and(|l| l.action.starts_with("tmr.k.")) {
                        s.stop_listening();
                    }
                });
                self.kerr = None;
            }
            return;
        }
        match k {
            K_GO => self.with(|m| m.toggle(sel)),
            K_RST => self.with(|m| m.reset(sel)),
            K_LAP => {
                self.with(|m| m.lap(sel));
                let n = self.with(|m| m.get(sel).map(|t| t.sw.laps().len()).unwrap_or(0));
                self.lap_at = Some((n, now));
            }
            K_SCR => {
                let on = !self.with(|m| m.get(sel).map(|t| t.screen).unwrap_or(false));
                if let Some(t) = self.with(|m| m.set_screen(sel, on)) {
                    self.show_toast(t, now);
                }
            }
            K_SND => self.with(|m| {
                if let Some(t) = m.get_mut(sel) {
                    t.cd.sound_on = !t.cd.sound_on;
                }
            }),
            K_PV => {
                if self.with(|m| m.preview(sel)) {
                    self.pv_at = Some(now);
                }
            }
            K_MOVE => self.with(|m| m.moving = !m.moving),
            K_ADD => {
                if self.with(|m| m.mode) != Mode::Clk {
                    self.menu = None;
                    let id = self.with(|m| m.add_new());
                    self.new_row = Some((id, now));
                    // the new timer's name is ready to type (the drawing focuses it)
                    let n = self.with(|m| m.get(id).map(|t| t.name.clone()).unwrap_or_default());
                    self.name_edit = Some((n, true));
                }
            }
            _ => {
                if let Some(i) = (0..model::FOUND_MAX).find(|&i| idx(K_MENU, i) == k) {
                    let _ = self.pick_found(i);
                } else if let Some(id) = self.row_part(k, "") {
                    self.apply_name();
                    self.apply_time();
                    self.with(|m| m.pick(id));
                    // a timer key field that listened belongs to the timer left: it stops (else the key lands there)
                    crate::services::try_with(|s| {
                        if s.listening.as_ref().is_some_and(|l| l.action.starts_with("tmr.k.")) {
                            s.stop_listening();
                        }
                    });
                    self.kerr = None;
                } else if let Some(id) = self.row_part(k, "pb") {
                    self.with(|m| m.toggle(id));
                } else if let Some(id) = self.row_part(k, "sb") {
                    self.with(|m| {
                        let on = !m.get(id).map(|t| t.screen).unwrap_or(false);
                        m.set_screen(id, on);
                    });
                } else if let Some(id) = self.row_part(k, "del") {
                    self.name_edit = None;
                    self.time_edit = None;
                    self.with(|m| m.remove(id));
                    drop_action(id);
                } else if let Some(i) = self.place_part(k, "sb") {
                    self.with(|m| m.toggle_place_screen(i));
                } else if let Some(i) = self.place_part(k, "del") {
                    self.with(|m| m.remove_place(i));
                }
            }
        }
        overlay::sync();
    }

    /// The result `i` of the "Add a place" search is added to the list; the box closes.
    fn pick_found(&mut self, i: usize) -> bool {
        let q = self.cq.clone();
        let Some(f) = self.with(|m| m.search_places(&q).get(i).cloned()) else { return false };
        self.with(|m| m.add_place(f.place));
        self.menu = None;
        self.cq.clear();
        self.cq_sel = 0;
        true
    }

    /// Which timer a row key (or one of its parts) belongs to.
    fn row_part(&self, k: Key, part: &str) -> Option<u32> {
        let ids: Vec<u32> = self.with(|m| m.timers.iter().map(|t| t.id).collect());
        ids.into_iter().find(|&id| {
            let r = idx(K_ROW, id as usize);
            if part.is_empty() {
                r == k
            } else {
                sub(r, part) == k
            }
        })
    }
    fn place_part(&self, k: Key, part: &str) -> Option<usize> {
        let n = self.with(|m| m.places.len());
        (0..n).find(|&i| sub(idx(K_PLACE, i), part) == k)
    }
}

impl Drop for Timers {
    fn drop(&mut self) {
        if self.shown {
            overlay::set_preview(false);
        }
    }
}

#[cfg(test)]
mod tests;

/// An icon whose `.f` parts are filled with its colour (`svg .f{fill:currentColor;stroke:none}`: the play triangle of
/// `tplay`, the screen bar of `tscr`) - the shared icon painter's `IconPaint`.
fn icon_f(name: &str, size: f32, stroke: f32, c: Rgba) -> El {
    El::icon(name, size, stroke, c).icon_paint(crate::ui::el::IconPaint { fill_all: false, classes: vec![("f".to_string(), crate::ui::el::ClassPaint::Fill(c))] })
}
/// The red line under a key field: `.kerr{font-size:11.5px;line-height:15px;color:var(--red)}` `.kerr.on{display:block;
/// animation:kin .2s ease-out}` (`kin`: opacity 0 + translateY(-3px) -> none). `#sw .tbErr{padding:4px 0 0;text-align:center}`.
fn kerr(cx: &mut Cx, key: Key, text: &str, shown_at: f64) -> El {
    let t = if cx.rm { 1.0 } else { ((cx.now - shown_at) / 200.0).clamp(0.0, 1.0) };
    if t < 1.0 {
        cx.st.busy = true;
    }
    let e = crate::anim::EASE_OUT_CSS.ease(t) as f32;
    let _ = key;
    El::text(text, Font::new(11.5, 400), crate::ui::RED(), 15.0).align(crate::gfx::Align::Center).pad(4.0, 0.0, 0.0, 0.0).w_pct(100.0).opacity(e).translate(0.0, -3.0 * (1.0 - e))
}

/// Test copy only (Order 079 flicker probe, command `tmrtick`): the fake timer clock moves `ms` on.
pub(crate) fn test_advance_clock(ms: u64) {
    model::with_existing(|m| {
        if let Some(fc) = &m.fake_clock {
            fc.advance_ms(ms);
        }
    });
}

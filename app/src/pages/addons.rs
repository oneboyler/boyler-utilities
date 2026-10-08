//! The Add-ons tab (Order 037; drawing addons-v1, page `add`, icon `puzzle`, just before Settings): only things you
//! download, in two sections of tiles - Pages (a whole tab, got only here) and Features (also inside their tab). A tile =
//! icon, name, one line, size, state + its one button (Get / Remove; while downloading: bar + % + ×). A click on the tile
//! opens its small window (built like the Updating window) with ONE button: Get / Cancel / Remove.
//! The state and the work live in `crate::addons` (one app-wide job, shared with the Mouse tab's Get).

use taffy::style::{AlignItems, JustifyContent};

use crate::addons::{self, Addon, Kind as AdKind, Op, Phase, View};
use crate::gfx::{sh, Font, Rgba};
use crate::pages::{Env, Page};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, lh, sub, Cursor, El, Key};
use crate::ui::pieces::button::{self, Kind};
use crate::ui::pieces::{self, bits, dialog, group, progress};
use crate::ui::{ACC, ACC_S, CTL, CTL_H, FG, FG2, FG3, GREEN, GRP, HAIR, HOV, SEL, TRK};

const K_TILE: Key = key("add.tile");
const K_BTN: Key = key("add.btn");
const K_X: Key = key("add.x");
const K_DLG: Key = key("add.dlg");
const K_DBTN: Key = key("add.dbtn");
const K_DLNK: Key = key("add.open");
const K_BAR: Key = key("add.bar");
const K_DBAR: Key = key("add.dbar");

/// The small window's width (`.addlg{width:350px}`).
const DLG_W: f32 = 350.0;

#[derive(Default)]
pub struct Addons {
    /// the small window: which add-on, opened when
    dlg: Option<(usize, f64)>,
    test: bool,
}

fn view_of(a: &Addon) -> View {
    addons::view(a.id)
}

fn pos(id: &str) -> usize {
    addons::CATALOGUE.iter().position(|a| a.id == id).unwrap_or(0)
}

/// The header's right line (`.ph .adsum{font-size:12px;color:var(--fg2);tabular-nums}`), the drawing's adSync rule.
pub fn summary() -> String {
    let got: Vec<&Addon> = addons::CATALOGUE.iter().filter(|a| view_of(a).got).collect();
    if !got.is_empty() {
        let mb: u64 = got.iter().map(|a| addons::size(a.id)).sum();
        return format!("{} of {} on this PC \u{b7} {}", got.len(), addons::CATALOGUE.len(), addons::mb(mb));
    }
    if addons::CATALOGUE.iter().any(|a| view_of(a).busy.is_some()) {
        return "Downloading 1 \u{b7} nothing on this PC yet".into();
    }
    "Nothing downloaded yet".into()
}

/// The download's share 0..1 (the bar), when downloading.
fn share(v: &View) -> Option<f32> {
    match v.busy {
        Some(Phase::Download { got, total, .. }) if total > 0 => Some(got as f32 / total as f32),
        Some(Phase::Checking | Phase::Installing) => Some(1.0),
        _ => None,
    }
}

impl Addons {
    fn tile(&self, cx: &mut Cx, i: usize, a: &Addon) -> El {
        let v = view_of(a);
        let k = idx(K_TILE, i);
        // .adt::before{background:var(--hov);opacity:0 -> 1 on hover, .15s ease}
        let hv = cx.hover_t(k, 150.0, crate::anim::EASE);
        // .adt{box-shadow:inset 0 0 0 .5px var(--hair)} .adt.have{box-shadow:inset 0 0 0 1px var(--acc-s)}
        let rim = if v.got { sh(0.0, 0.0, 0.0, 1.0, ACC_S()) } else { sh(0.0, 0.0, 0.0, 0.5, HAIR()) };
        // .adi{34x34;border-radius:9px;background:var(--sel)} svg{19px;stroke:var(--acc);stroke-width:1.5}
        let adi = El::block().size(34.0, 34.0).none().radius(9.0).bg(SEL()).place_center().child(El::icon(a.icon, 19.0, 1.5, ACC()));
        // .adtx b{13px 600 17px ellipsis} span{2 lines, height 32px, margin-top 1px, 11.5px/16px --fg2}
        let tx = El::col()
            .flex1()
            .min_w(0.0)
            .child(El::text(a.name, Font::new(13.0, 600), FG(), 17.0).ellipsis())
            .child(El::block().h(32.0).margin(1.0, 0.0, 0.0, 0.0).clip().child(El::text(a.line, Font::new(11.5, 400), FG2(), 16.0).wrapping()));
        // .adsz{flex:none;margin-left:auto;11.5px/17px --fg3 tabular-nums}
        let sz = El::text(addons::mb(addons::size(a.id)), Font::new(11.5, 400).tnum(), FG3(), 17.0).none().ml_auto();
        let top = El::row().items(AlignItems::FLEX_START).gap(11.0).min_w(0.0).child(adi).child(tx).child(sz);
        El::col()
            .min_w(0.0)
            .pad(12.0, 12.0, 10.0, 12.0)
            .radius(10.0)
            .bg(GRP())
            .inset(&[rim])
            .on_click(k)
            .cursor(Cursor::Hand)
            .child(El::block().abs(0.0, 0.0, 0.0, 0.0).radius(10.0).bg(HOV().mul_a(hv)).no_hit())
            .child(top)
            .child(self.foot(cx, i, a, &v))
    }

    /// `.adft{display:flex;align-items:center;gap:8px;height:26px;margin-top:10px}`: state + button, or (busy) bar + % + ×.
    fn foot(&self, cx: &mut Cx, i: usize, a: &Addon, v: &View) -> El {
        let ft = El::row().center().gap(8.0).h(26.0).margin(10.0, 0.0, 0.0, 0.0);
        if let Some(p) = v.busy {
            if let Some(s) = share(v) {
                // .adbar{flex:1;height:4px;border-radius:2px;background:var(--trk);overflow:hidden} i{gradient acc -> #64c8ff}
                let w = cx.tr(idx(K_BAR, i), 1, s, 250.0, crate::anim::Bezier::new(0.0, 0.0, 1.0, 1.0));
                let bar = El::block().flex1().h(4.0).radius(2.0).bg(TRK()).clip().child(
                    El::paint(move |g, (x, y, bw, h)| {
                        g.fill_rr_shader(x, y, bw * w, h, 2.0, &g.hgrad(x, 0.0, x + bw * w, 0.0, &[(0.0, ACC()), (1.0, Rgba::hex(0x64c8ff))]), 1.0);
                    })
                    .abs(0.0, 0.0, 0.0, 0.0),
                );
                // .adpc{width:30px;text-align:right;11.5px --fg2 tabular-nums}
                let pc = El::text(format!("{} %", (s * 100.0).floor() as u32), Font::new(11.5, 400).tnum(), FG2(), lh(11.5, 1.35)).w(30.0).none().align(crate::gfx::Align::Right);
                let mut row = ft.child(bar).child(pc);
                if matches!(p, Phase::Download { .. }) {
                    row = row.child(self.xbtn(cx, idx(K_X, i)));
                }
                return row;
            }
            // removing: the state line says so, the button waits
            let meta = El::text("Removing\u{2026}", Font::new(11.5, 400).tnum(), FG3(), lh(11.5, 1.35)).ellipsis().flex1();
            return ft.child(meta).child(button::cbtn(cx, idx(K_BTN, i), "Remove", Kind::Quiet, true, true, 64.0));
        }
        let also = if a.kind == AdKind::Feature { " \u{b7} also in Mouse" } else { "" };
        let f = Font::new(11.5, 400).tnum();
        // .adm{flex:1;min-width:0;overflow:hidden;display:flex;align-items:center;gap:5px;11.5px --fg3 nowrap}
        let mut meta = El::row().flex1().min_w(0.0).clip().center().gap(5.0);
        if v.got {
            // .adm .ok{inline-flex;gap:4px;color:var(--green)} svg{11px;stroke-width:1.8}
            let ok = El::row().center().gap(4.0).none().child(El::icon("check", 11.0, 1.8, GREEN())).child(El::text("On this PC", f, GREEN(), lh(11.5, 1.35)).none());
            meta = meta.child(ok);
            let rest = if v.restart { format!("restart to finish{also}") } else { also.trim_start_matches(" \u{b7} ").to_string() };
            if !rest.is_empty() {
                meta = meta.child(El::text(format!("\u{b7} {rest}"), f, FG3(), lh(11.5, 1.35)).ellipsis());
            }
        } else {
            let t = if v.restart { format!("Not downloaded \u{b7} restart to finish{also}") } else { format!("Not downloaded{also}") };
            meta = meta.child(El::text(t, f, FG3(), lh(11.5, 1.35)).ellipsis());
        }
        // #sw .adft .cbtn.sm{min-width:64px}: Get = .acc, Remove = .qt
        let btn = if v.got { button::cbtn(cx, idx(K_BTN, i), "Remove", Kind::Quiet, true, false, 64.0) } else { button::cbtn(cx, idx(K_BTN, i), "Get", Kind::Primary, true, false, 64.0) };
        ft.child(meta).child(btn)
    }

    /// `#sw .adx{22x22;border-radius:6px;background:var(--ctl);color:var(--fg2)}` `:hover{background:var(--ctl-h);color:var(--fg)}`
    /// `.adx svg{8px;stroke-width:1.5}`
    fn xbtn(&self, cx: &mut Cx, k: Key) -> El {
        let hv = cx.hover_t(k, 150.0, crate::anim::EASE);
        El::block()
            .size(22.0, 22.0)
            .none()
            .radius(6.0)
            .bg(crate::ui::cmix(CTL(), CTL_H(), hv))
            .place_center()
            .on_click(k)
            .cursor(Cursor::Hand)
            // `title:'Cancel'` (a plain hover name, not a data-tip)
            .title("Cancel")
            .child(El::icon("x", 8.0, 1.5, crate::ui::cmix(FG2(), FG(), hv)))
    }

    fn section(&self, cx: &mut Cx, title: &str, sub_line: &str, kind: AdKind) -> El {
        // .gh .ghs{margin-left:auto;font-weight:400;color:var(--fg3)} .adg{grid 2 columns, gap 8px}
        let gh = group::gh(title).child(bits::ghs(sub_line).ml_auto());
        let tiles: Vec<El> = addons::CATALOGUE.iter().enumerate().filter(|(_, a)| a.kind == kind).map(|(i, a)| self.tile(cx, i, a)).collect();
        El::block().child(gh).child(El::grid().cols(2).gap(8.0).children(tiles))
    }

    /// The small window (`miniDlg('Add-on', …, {cls:'addlg'})`): icon + name + state, what it adds, a note, the download's
    /// progress (only while downloading), ONE button.
    fn window(&self, cx: &mut Cx, i: usize, at: f64) -> El {
        let a = &addons::CATALOGUE[i];
        let v = view_of(a);
        let size = addons::mb(addons::size(a.id));
        let state = match v.busy {
            Some(Phase::Download { .. } | Phase::Checking) => format!("Downloading \u{b7} {size}"),
            Some(Phase::Installing) => format!("Installing \u{b7} {size}"),
            Some(Phase::Removing) => format!("Removing \u{b7} {size}"),
            None if v.got && v.restart => format!("On this PC \u{b7} restart to finish \u{b7} {size}"),
            None if v.got => format!("On this PC \u{b7} {size}"),
            None if v.restart => format!("{size} \u{b7} not downloaded \u{b7} restart to finish"),
            None => format!("{size} \u{b7} not downloaded"),
        };
        // .add1{flex;align-items:center;gap:12px;margin:4px 0 12px} .add1 .adi{40x40;radius 10} svg{22px}
        // b{13.5px 600} small{margin-top:1px;11.5px --fg2 tabular-nums}
        let head = El::row()
            .center()
            .gap(12.0)
            .margin(4.0, 0.0, 12.0, 0.0)
            .child(El::block().size(40.0, 40.0).none().radius(10.0).bg(SEL()).place_center().child(El::icon(a.icon, 22.0, 1.5, ACC())))
            .child(
                El::col()
                    .min_w(0.0)
                    .child(El::text(a.name, Font::new(13.5, 600), FG(), lh(13.5, 1.35)).ellipsis())
                    .child(El::text(state, Font::new(11.5, 400).tnum(), FG2(), lh(11.5, 1.35)).ellipsis().margin(1.0, 0.0, 0.0, 0.0)),
            );
        // .adl{radius 9px;background:var(--grp);inset .5px hair} li{padding:8px 12px 8px 26px;12px/16px} li+li::before{left:12px
        // right:0;top:0;height:1px;--hair} li::after{left:13px;top:14px;4x4;radius 50%;--acc}
        let items: Vec<El> = a
            .adds
            .iter()
            .enumerate()
            .map(|(n, t)| {
                let mut li = El::block().pad(8.0, 12.0, 8.0, 26.0).child(El::text(*t, Font::new(12.0, 400), FG(), 16.0).wrapping());
                if n > 0 {
                    li = li.child(El::block().abs(12.0, 0.0, 0.0, f32::NAN).h(1.0).bg(HAIR()).no_hit());
                }
                li.child(El::block().abs(13.0, 14.0, f32::NAN, f32::NAN).size(4.0, 4.0).radius(2.0).bg(ACC()).no_hit())
            })
            .collect();
        let list = El::block().radius(9.0).bg(GRP()).inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())]).children(items);
        // .adn{margin:9px 2px 0;line-height:16px;--fg2} - Chromium computes it 12px (an id rule beats .adn's 11.5px; the dom
        // dump), its "Open Mouse" too (`#sw .lnk{12px/16px}`, weight inherited 400)
        let link = (a.kind == AdKind::Feature).then_some("Open Mouse");
        // the box Chromium lays out (dom dump, k3 detail): 3 px under the list, the window's full 314 px content width (an
        // id rule's `p` margin wins over .adn's `9px 2px 0`)
        let note = note_lines(cx, a.note, link, DLG_W - 36.0).margin(3.0, 0.0, 0.0, 0.0);
        let mut body = vec![head, list, note];
        // .adp{margin-top:14px}: .updbar + .updst (Downloading · 1.2 of 2.6 MB | 11 s left) - only while downloading
        if let Some(p) = v.busy.filter(|p| !matches!(p, Phase::Removing)) {
            let (l, r) = match p {
                Phase::Download { got, total, left_s } => {
                    (format!("Downloading \u{b7} {:.1} of {:.1} MB", got as f64 / 1e6, total as f64 / 1e6), left_s.map(|s| format!("{s} s left")).unwrap_or_default())
                }
                Phase::Checking => ("Checking the download".into(), String::new()),
                _ => ("Installing \u{b7} answer the admin prompt".into(), String::new()),
            };
            body.push(El::col().margin(14.0, 0.0, 0.0, 0.0).child(progress::bar(cx, K_DBAR, share(&v))).child(progress::status(&l, &r)));
        }
        // #sw .addlg .dft{margin-top:14px} .dft .cbtn{min-width:86px} (+ the footer's 30 px height)
        let (label, kind, off) = match v.busy {
            Some(Phase::Download { .. }) => ("Cancel", Kind::Ghost, false),
            Some(Phase::Checking | Phase::Installing) => ("Cancel", Kind::Ghost, true),
            Some(Phase::Removing) => ("Remove", Kind::RedText, true),
            None if v.got => ("Remove", Kind::RedText, false),
            None => ("Get", Kind::Primary, false),
        };
        body.push(El::row().justify(JustifyContent::FLEX_END).margin(14.0, 0.0, 0.0, 0.0).child(button::cbtn_sized(cx, K_DBTN, label, kind, button::DFT, off, 86.0)));
        let title = if a.kind == AdKind::Page { "Page add-on" } else { "Feature add-on" };
        dialog::dialog(cx, K_DLG, DLG_W, title, body, vec![], true, at)
    }

    /// Get / Remove / Cancel of add-on `i` (the tile's button, the window's button).
    fn act(&mut self, cx: &mut Cx, i: usize) {
        let a = &addons::CATALOGUE[i];
        let v = view_of(a);
        match (a.id, v.busy, v.got) {
            ("acc", Some(Phase::Download { .. }), _) => addons::cancel_acc(cx),
            (_, Some(_), _) => {}
            // Order 047: its take-over still runs (off the menu's thread): a second Get waits for it
            ("obs", None, false) if addons::taking_over() => {}
            ("obs", None, false) => {
                // (the original NotificationsForOBS taken over: that line instead)
                let t = addons::set("obs", true);
                // Order 047: a take-over runs off the menu's thread - its line comes as the page's notice when it ends
                if !addons::taking_over() {
                    cx.toast(&t.unwrap_or_else(|| format!("{} is in the top row", a.name)));
                }
            }
            ("obs", None, true) => {
                addons::set("obs", false);
                cx.toast(&format!("Removed {}", a.name));
            }
            (_, None, got) => {
                if let Err(e) = addons::start_acc(cx, if got { Op::Remove } else { Op::Get }) {
                    cx.toast(&e);
                }
            }
        }
    }
}

/// The note with an inline link at its end, wrapped word by word to `width` (the layout has no inline runs; the words are
/// measured with the painter's own shaping, so the break falls where the drawing's does).
fn note_lines(cx: &mut Cx, text: &str, link: Option<&str>, width: f32) -> El {
    let f = Font::new(12.0, 400);
    let space = cx.g.text_width(" ", f);
    let mut lines: Vec<String> = vec![String::new()];
    for w in text.split(' ') {
        let cur = lines.last().unwrap();
        let cand = if cur.is_empty() { w.to_string() } else { format!("{cur} {w}") };
        if !cur.is_empty() && cx.g.text_width(&cand, f) > width {
            lines.push(w.to_string());
        } else {
            *lines.last_mut().unwrap() = cand;
        }
    }
    let mut last_link_own_line = false;
    if let Some(l) = link {
        let lw = cx.g.text_width(l, f);
        if cx.g.text_width(lines.last().unwrap(), f) + space + lw > width {
            last_link_own_line = true;
        }
    }
    let n = lines.len();
    let mut col = El::col();
    for (i, t) in lines.into_iter().enumerate() {
        let mut row = El::row().child(El::text(t, f, FG2(), 16.0).none());
        if i + 1 == n && !last_link_own_line {
            if let Some(l) = link {
                row = row.child(El::text(" ", f, FG2(), 16.0).none()).child(bits::lnk(cx, K_DLNK, l, 12.0, 400, 16.0));
            }
        }
        col = col.child(row);
    }
    if let (true, Some(l)) = (last_link_own_line, link) {
        col = col.child(El::row().child(bits::lnk(cx, K_DLNK, l, 12.0, 400, 16.0)));
    }
    col
}

impl Page for Addons {
    fn id(&self) -> &'static str {
        "add"
    }
    fn name(&self) -> &'static str {
        "Add-ons"
    }
    fn icon(&self) -> &'static str {
        "puzzle"
    }

    fn open(&mut self, env: &Env, _now: f64) {
        self.test = env.fake();
        addons::refresh();
    }

    fn close(&mut self) {
        self.dlg = None;
    }

    /// Order 047: a Get's take-over ended (its worker woke the menu): the feature starts, the page shows it. Nothing
    /// else moves here by itself (a job's progress wakes the menu itself).
    fn tick(&mut self, _now: f64) -> bool {
        addons::poll_take_over()
    }

    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        if let Some(n) = addons::take_notice() {
            cx.toast(&n);
        }
        // a job runs: its progress is painted as it comes (the waker rebuilds the page)
        let right = El::text(summary(), Font::new(12.0, 400).tnum(), FG2(), lh(12.0, 1.35)).none();
        vec![
            pieces::header(self.name(), Some(right)),
            self.section(cx, "Pages", "A whole page, only here", AdKind::Page),
            self.section(cx, "Features", "Also inside their page", AdKind::Feature),
        ]
    }

    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        let Ev::Click(k) = ev else { return };
        let k = *k;
        for i in 0..addons::CATALOGUE.len() {
            if k == idx(K_BTN, i) || k == idx(K_X, i) {
                self.act(cx, i);
                return;
            }
            if k == idx(K_TILE, i) {
                self.dlg = Some((i, cx.now));
                return;
            }
        }
        if k == K_DBTN {
            if let Some((i, _)) = self.dlg {
                self.act(cx, i);
            }
        } else if k == K_DLNK {
            self.dlg = None;
            cx.show_tab("cur", None);
        } else if k == sub(K_DLG, "x") || k == sub(K_DLG, "out") {
            self.dlg = None;
        }
    }

    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        let (i, at) = self.dlg?;
        Some(self.window(cx, i, at))
    }

    fn popup_dismiss(&mut self) {
        self.dlg = None;
    }

    fn jump(&mut self, target: &str) {
        self.dlg = Some((pos(target), 0.0));
    }

    fn describe(&self) -> String {
        format!("{} dlg={:?} summary={:?}", addons::describe(), self.dlg.map(|d| addons::CATALOGUE[d.0].id), summary())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalogue_is_the_drawings() {
        let ids: Vec<&str> = addons::CATALOGUE.iter().map(|a| a.id).collect();
        assert_eq!(ids, ["obs", "acc"]);
        assert_eq!(addons::addon("obs").unwrap().kind, AdKind::Page);
        assert_eq!(addons::addon("acc").unwrap().page, "cur");
        assert_eq!(addons::mb(addons::size("acc")), "1.5 MB");
    }
}

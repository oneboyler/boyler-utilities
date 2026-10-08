//! The pieces gallery (Order 014, test only): every shared piece in its resting states at fixed places, painted by the
//! app's own painter - the same places as tools/ref/gallery.html, which builds the same pieces with the drawing's own
//! CSS and markup. `--cmd "gallery:<png>"` (a test copy) writes the picture; tools/ref/diff_gallery.py compares.

use crate::gfx::{Gfx, Rgba};
use crate::icons::Icons;
use crate::ui::cx::{Cx, State};
use crate::ui::el::{key, El};
use crate::ui::lay::Laid;
use crate::ui::pieces::{self, badge, button, card, dialog, dropdown, group, keyfield, link, listrow, progress, reset, search, seg, slider, toggle};

pub const W: f32 = 600.0;
pub const H: f32 = 1000.0;
/// the gallery's backdrop (both sides): an opaque colour, so glass pieces blur a flat colour
pub const BG: Rgba = Rgba::rgb(29, 32, 48);

fn at(x: f32, y: f32, e: El) -> El {
    e.abs(x, y, f32::NAN, f32::NAN)
}

/// Scene 2: the small popup window (title + ×, a line, Cancel / Save) over the dimmed window.
fn scene2(cx: &mut Cx) -> El {
    let body = vec![El::text("One key mutes your mic in every app, games included.", crate::gfx::Font::new(12.0, 400), crate::ui::FG2(), 16.0).wrapping().margin(0.0, 0.0, 12.0, 0.0)];
    let footer = vec![
        button::cbtn_sized(cx, key("d.no"), "Cancel", button::Kind::Ghost, button::DFT, false, 76.0),
        button::cbtn_sized(cx, key("d.go"), "Save", button::Kind::Primary, button::DFT, false, 76.0),
    ];
    let d = dialog::dialog(cx, key("dlg"), 388.0, "Mute settings", body, footer, true, -10000.0);
    El::block().size(W, H).bg(BG).child(d)
}

/// Scene 3: the reset review in its popup list box, and a toast.
fn scene3(cx: &mut Cx) -> El {
    let lines = vec![
        reset::Line { title: "Refresh rate".into(), from: "60 Hz".into(), to: "144 Hz".into(), ticked: true, heading: None },
        reset::Line { title: "Scaling".into(), from: "125 %".into(), to: "100 %".into(), ticked: false, heading: None },
    ];
    let buttons = vec![
        button::cbtn_sized(cx, key("r.no"), "Cancel", button::Kind::Ghost, button::MCFB, false, 0.0),
        button::cbtn_sized(cx, key("r.go"), "Reset 1", button::Kind::Red, button::MCFB, false, 0.0),
    ];
    let r = reset::review_popup(cx, key("rv"), 20.0, 40.0, "Display · back to how it was?", "Each one goes back to the value it had before this app changed it.", &lines, buttons);
    let t = crate::ui::pieces::toast::toast(cx, key("toast"), "Windows asks for admin once", cx.now, false);
    El::block().size(W, H).bg(BG).child(r).child(t)
}

/// Scene 4's hovered and pressed elements (CSS :hover / :active; gallery.js gives the same elements `.hv` / `.ac`,
/// copies of the drawing's own :hover / :active rules).
const HOVERED: [&str; 12] = ["h.b0", "h.b1", "h.b2", "h.t0", "h.t1", "h.x", "h.d0", "h.btn", "h.k0", "h.k1", "h.m0", "h.f0"];
const PRESSED: [&str; 4] = ["a.b0", "a.b1", "a.d0", "a.btn"];
/// Scene 4's clock: the looping animations (indeterminate bar, spinner) are shown 550 ms in; gallery.js pauses them there.
const SCENE4_NOW: f64 = 550.0;

/// Scene 4: hover + pressed states, the key field listening, the fold card hovered and open, the level meter, the
/// indeterminate bar and the spinner.
#[allow(clippy::vec_init_then_push)]
fn scene4(cx: &mut Cx) -> El {
    let mut kids: Vec<El> = Vec::new();
    // ---- hover
    kids.push(at(20.0, 20.0, button::cbtn(cx, key("h.b0"), "Cancel", button::Kind::Ghost, false, false, 0.0)));
    kids.push(at(110.0, 20.0, button::cbtn(cx, key("h.b1"), "Save", button::Kind::Primary, false, false, 0.0)));
    kids.push(at(190.0, 20.0, button::cbtn(cx, key("h.b2"), "End task", button::Kind::RedText, true, false, 0.0)));
    kids.push(at(290.0, 20.0, toggle::toggle(cx, key("h.t0"), false, false)));
    kids.push(at(350.0, 20.0, toggle::toggle(cx, key("h.t1"), true, false)));
    kids.push(at(410.0, 20.0, button::icon_btn(cx, key("h.x"), "x", 9.0, 1.5)));
    kids.push(at(20.0, 70.0, dropdown::dropdown(cx, key("h.d0"), "English", None)));
    kids.push(at(130.0, 70.0, button::btn(cx, key("h.btn"), "open", "Open folder", false)));
    kids.push(at(270.0, 70.0, keyfield::keyfield(cx, key("h.k0"), keyfield::Show::Empty, 0.0, false)));
    kids.push(at(370.0, 70.0, keyfield::keyfield(cx, key("h.k1"), keyfield::Show::Set("Ctrl + M"), 0.0, false)));
    // ---- pressed
    kids.push(at(20.0, 120.0, button::cbtn(cx, key("a.b0"), "Cancel", button::Kind::Ghost, false, false, 0.0)));
    kids.push(at(110.0, 120.0, button::cbtn(cx, key("a.b1"), "Save", button::Kind::Primary, false, false, 0.0)));
    kids.push(at(190.0, 120.0, dropdown::dropdown(cx, key("a.d0"), "English", None)));
    kids.push(at(300.0, 120.0, button::btn(cx, key("a.btn"), "open", "Open folder", false)));
    // ---- the key field listening (no keys yet / modifiers held), 625 ms after it started: the ring's breathe is
    // 425 ms in (after its .2 s delay), opacity .71 - gallery.js pauses the drawing's animation there
    let now = cx.now - 625.0;
    kids.push(at(20.0, 170.0, keyfield::keyfield(cx, key("l.k0"), keyfield::Show::Listening(None), now, false)));
    kids.push(at(200.0, 170.0, keyfield::keyfield(cx, key("l.k1"), keyfield::Show::Listening(Some("Ctrl + Alt")), now, false)));
    // ---- level meter, spinner, indeterminate bar
    kids.push(at(20.0, 226.0, slider::level_bar(200.0, 0.3, 0.6, 0.9, crate::ui::VZ1(), crate::ui::VZ2())));
    kids.push(at(260.0, 220.0, progress::spinner(cx, "rst", 14.0, crate::ui::FG2())));
    kids.push(at(300.0, 224.0, El::col().w(280.0).child(progress::bar(cx, key("p.ind"), None))));
    // ---- the popup list with its first item hovered
    let items = vec![
        dropdown::Item { label: "Headphones".into(), checked: true, disabled: false },
        dropdown::Item { label: "Speakers".into(), checked: false, disabled: false },
    ];
    // (the popup list keeps itself inside the 520 px window: it stays in the window's top part)
    kids.push(dropdown::menu(cx, key("h.m0"), &items, 400.0, 290.0, 150.0));
    // ---- fold card: hovered (closed), and open
    kids.push(at(26.0, 270.0, card::fold_card(cx, key("h.f0"), vec![group::lbl("More settings", None)], El::block().h(40.0), false, 340.0).w(340.0)));
    // ---- the search field focused, empty (the ring + placeholder; at 550 ms its caret is in its "off" half, gallery.js
    // hides Chromium's caret)
    kids.push(at(390.0, 400.0, search::search(cx, key("f.q0"), "", "Search apps", false)));
    let body = group::row(false, vec![group::lbl("Sound", Some("Plays when the mic mutes"))]);
    kids.push(at(26.0, 360.0, card::fold_card(cx, key("o.f0"), vec![group::lbl("More settings", None)], body, true, 340.0).w(340.0)));
    El::block().size(W, H).bg(BG).children(kids)
}

/// The gallery's boxes (scene 1 = the pieces at rest; 2 = the small popup window; 3 = the reset review + a toast;
/// 4 = hover / pressed / listening / open states and the moving pieces).
pub fn build_scene(cx: &mut Cx, scene: u32) -> El {
    match scene {
        2 => scene2(cx),
        3 => scene3(cx),
        4 => scene4(cx),
        _ => build(cx),
    }
}

/// The gallery's boxes.
#[allow(clippy::vec_init_then_push)]
pub fn build(cx: &mut Cx) -> El {
    let mut kids: Vec<El> = Vec::new();
    // ---- row 1: toggles, slider + value
    kids.push(at(20.0, 20.0, toggle::toggle(cx, key("t0"), false, false)));
    kids.push(at(80.0, 20.0, toggle::toggle(cx, key("t1"), true, false)));
    kids.push(at(140.0, 20.0, toggle::toggle(cx, key("t2"), false, true)));
    kids.push(at(200.0, 20.0, toggle::toggle(cx, key("t3"), true, true)));
    kids.push(at(260.0, 22.0, slider::slider(cx, key("s0"), 0.5, 150.0, 20.0, slider::default())));
    kids.push(at(420.0, 22.0, slider::value_label("74 %")));
    // ---- row 2: dropdowns, link, badges
    kids.push(at(20.0, 70.0, dropdown::dropdown(cx, key("d0"), "Headphones (Arctis Nova)", Some(196.0))));
    kids.push(at(230.0, 70.0, dropdown::dropdown(cx, key("d1"), "English", None)));
    kids.push(at(340.0, 74.0, link::link(cx, key("l0"), "Mute settings", 12.0)));
    kids.push(at(440.0, 74.0, badge::tag("Later", badge::Tone::Plain)));
    kids.push(at(490.0, 74.0, badge::tag("ON", badge::Tone::Green)));
    kids.push(at(530.0, 74.0, badge::tag("NEW", badge::Tone::Teal)));
    // ---- row 3: segmented switches
    kids.push(at(20.0, 120.0, seg::seg(cx, key("g0"), &["All", "Normal", "Hidden"], 1, false)));
    kids.push(at(260.0, 120.0, seg::seg(cx, key("g1"), &["Stopwatch", "Timer"], 0, true)));
    // ---- row 4: buttons
    kids.push(at(20.0, 170.0, button::cbtn(cx, key("b0"), "Cancel", button::Kind::Ghost, false, false, 0.0)));
    kids.push(at(110.0, 170.0, button::cbtn(cx, key("b1"), "Save", button::Kind::Primary, false, false, 0.0)));
    kids.push(at(190.0, 170.0, button::cbtn(cx, key("b2"), "Reset", button::Kind::Red, false, false, 0.0)));
    kids.push(at(270.0, 170.0, button::cbtn(cx, key("b3"), "End task", button::Kind::RedText, true, false, 0.0)));
    kids.push(at(370.0, 170.0, button::cbtn(cx, key("b4"), "Apply", button::Kind::Primary, false, true, 0.0)));
    kids.push(at(450.0, 170.0, button::icon_btn(cx, key("b5"), "x", 9.0, 1.5)));
    kids.push(at(20.0, 215.0, button::btn(cx, key("b6"), "open", "Open folder", false)));
    kids.push(at(150.0, 215.0, button::btn(cx, key("b7"), "", "Move icon", true)));
    // ---- row 5: search, key fields
    kids.push(at(20.0, 260.0, search::search(cx, key("q0"), "", "Search tweaks", false)));
    kids.push(at(230.0, 260.0, search::search(cx, key("q1"), "blue", "Search apps", false)));
    kids.push(at(20.0, 305.0, keyfield::keyfield(cx, key("k0"), keyfield::Show::Empty, 0.0, false)));
    kids.push(at(120.0, 305.0, keyfield::keyfield(cx, key("k1"), keyfield::Show::Set("Ctrl + Shift + M"), 0.0, false)));
    kids.push(at(320.0, 305.0, badge::live_note("Live only while this page is open")));
    kids.push(at(320.0, 330.0, badge::via("shd16", "Microsoft Defender")));
    // ---- row 6: a group with rows, its header and footer (548 wide like a page)
    let grp = El::col().w(548.0).child(group::gh("Devices")).child(group::grp(vec![
        group::row(true, vec![group::lbl("Keep my devices", Some("Stop Windows switching to newly plugged-in devices")), group::ctl(vec![toggle::toggle(cx, key("t4"), true, false)])]),
        group::row(false, vec![group::lbl("Sound", None), group::ctl(vec![dropdown::dropdown(cx, key("d2"), "Soft click", Some(128.0))])]),
    ]))
    .child(group::gf("Sets your Windows default devices."));
    kids.push(at(26.0, 350.0, grp));
    // ---- row 7: card head, fold card, list row
    let ch = card::card_head("mic", "Mic mute", Some("One key mutes your mic everywhere"), vec![toggle::toggle(cx, key("t5"), true, false)]);
    kids.push(at(26.0, 500.0, card::card(cx, key("c0"), ch, None, false, 548.0).w(548.0)));
    let fold = card::fold_card(cx, key("f0"), vec![group::lbl("More settings", None)], El::block().h(40.0), false, 548.0).w(548.0);
    kids.push(at(26.0, 552.0, fold));
    let tile = listrow::Tile::Glyph { glyph: "note", a: Rgba::hex(0x3ddc84), b: Rgba::hex(0x1db954) };
    kids.push(at(26.0, 640.0, group::grp(vec![listrow::list_row(true, &tile, "Spotifast", Some("Starts with Windows"), vec![toggle::toggle(cx, key("t6"), true, false)])]).w(548.0)));
    // ---- row 8: reset line, progress
    kids.push(at(26.0, 690.0, reset::reset_line(cx, key("r0"), Some("Windows defaults")).w(548.0)));
    let bar = progress::bar(cx, key("p0"), Some(0.4));
    kids.push(at(26.0, 750.0, El::col().w(300.0).child(bar).child(progress::status("Downloading…", "12.4 of 31 MB"))));
    // ---- row 9: the popup list, the review, a toast-like note, a small window
    let items = vec![
        dropdown::Item { label: "Headphones".into(), checked: true, disabled: false },
        dropdown::Item { label: "Speakers".into(), checked: false, disabled: false },
        dropdown::Item { label: "Monitor (off)".into(), checked: false, disabled: true },
    ];
    kids.push(dropdown::menu(cx, key("m0"), &items, 440.0, 210.0, 150.0));
    let _ = (dialog::dialog as fn(_, _, _, _, _, _, _, _) -> El, pieces::header as fn(&str, Option<El>) -> El);
    El::block().size(W, H).bg(BG).children(kids)
}

/// Paint the gallery into a picture at `scale` (DIPs x scale pixels).
pub fn render(scale: f32, scene: u32) -> Option<crate::png::Pixels> {
    let g = Gfx::new(scale);
    let icons = Icons::new();
    let mut st = State::default();
    let mut now = 0.0;
    if scene == 4 {
        // the keyed elements under the pointer: the piece itself (the popup list's item 0, the fold card's fold row)
        st.hover = HOVERED
            .iter()
            .map(|n| match *n {
                "h.m0" => crate::ui::el::idx(key(n), 0),
                "h.f0" => crate::ui::el::sub(key(n), "fold"),
                _ => key(n),
            })
            .collect();
        st.active = PRESSED.iter().map(|n| key(n)).collect();
        st.focus = Some(key("f.q0"));
        st.hover.extend(st.active.clone());
        now = SCENE4_NOW;
    }
    let mut cx = Cx::new(now, false, &g, &mut st);
    let root = build_scene(&mut cx, scene);
    let laid = Laid::new(&g, root, W, Some(H));
    let mut s = crate::gfx::new_surface((W * scale).round() as i32, (H * scale).round() as i32)?;
    // the backdrop first (what the glass pieces blur), then everything with it
    g.begin(s.canvas());
    g.fill_rect(0.0, 0.0, W, H, BG);
    g.end();
    let base = s.image_snapshot();
    g.begin(s.canvas());
    laid.paint(&g, &icons, 0.0, 0.0, Some(&base));
    g.end();
    Some(crate::png::from_surface(&mut s))
}

// ---- Order 025 (Lane W): the SECOND gallery - the pieces Order 025 adds (PIECES_WANTED.md), each in every state the
// drawing has (rest, hover, focus, open ...), at the same places as tools/ref/gallery_w.js (the drawing's CSS + markup,
// states forced with Chromium's own forcePseudoState). Kept apart from `build` so Lane K's gallery merges untouched.
// Picture: `BU_GALLERY_W=<png> cargo test -p bu-app gallery_w_png -- --ignored` (off-screen, nothing on the screen);
// compared with tools/ref/diff_gallery_w.py.

/// The keys the second gallery shows hovered / focused (Chromium forces the same states on the same entries).
pub mod w {
    use crate::ui::el::{key, Key};
    pub const NBOX_HOVER: Key = key("w.nbox.num.h");
    pub const NBOX_FOCUS: Key = key("w.nbox.num.f");
    pub const NBOX_SM_HOVER: Key = key("w.nbox.sm.h");
    pub const NBOX_SM_FOCUS: Key = key("w.nbox.sm.f");
    pub const MB_HOVER: Key = key("w.mb.h");
    pub const MB_PRESS: Key = key("w.mb.p");
    pub const RQ_HOVER: Key = key("w.rq.h");
    pub const RDEL_HOVER: Key = key("w.rd1");
    pub const IB_HOVER: Key = key("w.ib1");
    pub const PB_HOVER: Key = key("w.pb1");
    pub const WPS_HOVER: Key = key("w.wps2");
    pub const NBOX_SEL: Key = key("w.nbox.sel");
    pub const DNS_FOCUS: Key = key("w.dns.3");
    pub const IDB_HOVER: Key = key("w.idb.h");
    pub const DTL_HOVER: Key = key("w.dtl.h");
    pub const KDIN_F: Key = key("w.kd1");
    pub const THN_H: Key = key("w.thn1");
    pub const THN_F: Key = key("w.thn2");
    pub const THD_F: Key = key("w.thd1");
    pub const APIN_F: Key = key("w.ap0");
    pub const SBB_ALONE: Key = key("w.sbb.alone");
    pub const GS_START_H: Key = key("w.gs.h0");
    pub const GS_STOP_H: Key = key("w.gs.h1");
    pub const IB_OPEN_H: Key = key("w.ib.h0");
    pub const THD_SEL: Key = key("w.thd2");
    pub const ADDB_H: Key = key("w.addb1");
    pub const LNK_H: Key = key("w.lnk1");
    pub const FOLD_H: Key = key("w.fold1");
    pub const CX_H: Key = key("w.cx1");
    /// hovered elements (CSS :hover)
    pub fn hovered() -> Vec<Key> {
        use crate::ui::el::idx;
        // + the hovered menu rows (batch 3): Startup's first item, Apps' Uninstall, DNS Cloudflare, the pad row
        vec![NBOX_HOVER, NBOX_SM_HOVER, MB_HOVER, MB_PRESS, RQ_HOVER, IDB_HOVER, DTL_HOVER, idx(key("w.m1"), 1), idx(key("w.m3"), 1), idx(key("w.m5"), 2), idx(key("w.m6"), 4), idx(key("w.sb2"), 0), idx(key("w.sb1"), 2), idx(key("w.sb3"), 1), RDEL_HOVER, PB_HOVER, WPS_HOVER, IB_HOVER, THN_H, SBB_ALONE, GS_START_H, GS_STOP_H, IB_OPEN_H, ADDB_H, LNK_H, FOLD_H, CX_H]
    }
    /// pressed elements (CSS :active)
    pub fn active() -> Vec<Key> {
        vec![MB_PRESS, crate::ui::el::idx(key("w.sb3"), 1), SBB_ALONE]
    }
    /// focused elements (CSS :focus-within) - the gallery builds once per focused key
    pub fn focused() -> Vec<Key> {
        vec![NBOX_FOCUS, NBOX_SM_FOCUS, NBOX_SEL, DNS_FOCUS, KDIN_F, THN_F, THD_F, APIN_F, THD_SEL]
    }
}

/// The second gallery's boxes; `now` is the clock the pieces see (the gallery uses 600 ms: carets are in their "off" half).
pub fn build_w(cx: &mut Cx) -> El {
    use crate::ui::pieces::nbox;
    let mut kids: Vec<El> = Vec::new();
    // ---- batch 1: the text field .nbox
    let snap = Some(cx.now - 300.0);
    kids.push(at(20.0, 20.0, nbox::nbox(cx, key("w.nbox.num"), "1920", "", &nbox::NUM, &nbox::Cue::NONE)));
    kids.push(at(100.0, 20.0, nbox::nbox(cx, w::NBOX_HOVER, "1080", "", &nbox::NUM, &nbox::Cue::NONE)));
    kids.push(at(180.0, 20.0, nbox::nbox(cx, w::NBOX_FOCUS, "2560", "", &nbox::NUM, &nbox::Cue::NONE)));
    kids.push(at(260.0, 20.0, nbox::nbox(cx, key("w.nbox.hz"), "144", "", &nbox::HZ, &nbox::Cue::NONE)));
    kids.push(at(330.0, 20.0, nbox::nbox(cx, key("w.nbox.hz.s"), "59.94", "", &nbox::HZ, &nbox::Cue { snap_at: snap, ..nbox::Cue::NONE })));
    kids.push(at(20.0, 70.0, nbox::nbox(cx, key("w.nbox.sm.p"), "", "Custom", &nbox::SM, &nbox::Cue::NONE)));
    kids.push(at(100.0, 70.0, nbox::nbox(cx, key("w.nbox.sm"), "3200", "Custom", &nbox::SM, &nbox::Cue::NONE)));
    kids.push(at(180.0, 70.0, nbox::nbox(cx, w::NBOX_SM_HOVER, "800", "Custom", &nbox::SM, &nbox::Cue::NONE)));
    kids.push(at(264.0, 70.0, nbox::nbox(cx, w::NBOX_SM_FOCUS, "1600", "Custom", &nbox::SM, &nbox::Cue::NONE)));
    kids.push(at(340.0, 70.0, nbox::nbox(cx, key("w.nbox.tbdu"), "30", "", &nbox::SM_TBDU, &nbox::Cue::NONE)));
    kids.push(at(410.0, 73.0, nbox::nbox(cx, key("w.nbox.xs"), "1600", "", &nbox::XS, &nbox::Cue::NONE)));
    kids.push(at(20.0, 110.0, El::row().w(260.0).child(nbox::nbox(cx, key("w.nbox.f0"), "", "Password", &nbox::FORM, &nbox::Cue::NONE))));
    kids.push(at(20.0, 150.0, El::row().w(260.0).child(nbox::nbox(cx, key("w.nbox.f1"), "hunter22", "Password", &nbox::FORM, &nbox::Cue::NONE))));
    // ---- batch 1: the small header button .mbtn, the info note .inote
    use crate::ui::pieces::{group, inote, mbtn};
    kids.push(at(20.0, 200.0, mbtn::mbtn(cx, key("w.mb.0"), mbtn::Mb::Text("Flush DNS"), false)));
    kids.push(at(110.0, 200.0, mbtn::mbtn(cx, w::MB_HOVER, mbtn::Mb::Text("Flush DNS"), false)));
    kids.push(at(200.0, 200.0, mbtn::mbtn(cx, w::MB_PRESS, mbtn::Mb::Text("Copy all"), false)));
    kids.push(at(280.0, 200.0, mbtn::mbtn(cx, key("w.mb.d"), mbtn::Mb::Done("Flushed"), false)));
    kids.push(at(370.0, 200.0, mbtn::mbtn(cx, key("w.mb.i"), mbtn::Mb::Icon("plus12", "New timer"), false)));
    kids.push(at(20.0, 240.0, mbtn::mbtn(cx, key("w.mb.dns"), mbtn::Mb::Dns("Automatic"), false)));
    kids.push(at(280.0, 240.0, mbtn::mbtn(cx, key("w.mb.dnsw"), mbtn::Mb::Dns("Cloudflare"), true)));
    kids.push(at(420.0, 240.0, mbtn::mbtn(cx, key("w.mb.set"), mbtn::Mb::Set("Default"), false)));
    let ghb = mbtn::mbtn(cx, key("w.mb.gh"), mbtn::Mb::Text("Flush DNS"), false);
    kids.push(at(26.0, 270.0, El::block().w(548.0).child(mbtn::gh_with("Connection", vec![], vec![ghb]).margin(0.0, 12.0, 0.0, 12.0))));
    // the three .inote uses of v22: Audio's Mute settings window (`.mmd .isub` - wraps), Tweaks' fullscreen window
    // (`.fsodlg .inote.calm.fson`), Apps' uninstall window (`.inote.udw`)
    let isub = |n: El| El::block().w(420.0).radius(9.0).bg(crate::ui::GRP()).inset(&[crate::gfx::sh(0.0, 0.0, 0.0, 0.5, crate::ui::HAIR())]).child(n);
    let row_note = inote::inote("Can\u{2019}t show over games in exclusive fullscreen \u{2014} use borderless / windowed fullscreen.", false, &inote::ROW_WRAP);
    kids.push(at(26.0, 300.0, isub(row_note)));
    kids.push(at(26.0, 360.0, El::block().w(380.0).child(inote::inote("Set it before the game starts \u{2014} Windows reads it at launch.", true, &inote::FSODLG))));
    kids.push(at(26.0, 410.0, El::block().w(360.0).child(inote::inote("Spotifast: closes it first. Steam: keeps your games.", false, &inote::UDW))));
    let _ = group::grp;
    // ---- batch H: El::icon_svg (a page's own <svg>) and El::icon_fit (xMidYMid meet in another box shape)
    const SHWN: &str = r#"<svg viewBox="0 0 24 24"><path d="M12 3l7 2.6v5.3c0 4.4-2.9 7.7-7 9.1-4.1-1.4-7-4.7-7-9.1V5.6z"/><path d="M12 8.2v4.4"/><path d="M12 15.6h.01" stroke-width="2.2"/></svg>"#;
    const BADI: &str = r#"<svg viewBox="0 0 20 20"><path d="M10 3.2l7.2 12.6H2.8z"/><path d="M10 8.2v3.6"/><path d="M10 14.1h.01" stroke-width="2.2"/></svg>"#;
    kids.push(at(20.0, 470.0, El::icon_svg(SHWN, 24.0, 1.6, crate::ui::AMBER())));
    kids.push(at(60.0, 472.0, El::icon_svg(BADI, 20.0, 1.5, crate::ui::RED())));
    kids.push(at(100.0, 476.0, El::icon_fit("chev", 8.0, 12.0, 1.5, crate::ui::FG2())));
    kids.push(at(120.0, 476.5, El::icon_fit("chev", 7.0, 11.0, 1.5, crate::ui::FG2())));
    // ---- batch 2: tooltips (.rq tip icons in a .ttl title, the shared bubble .wtip, .idb + .htip)
    use crate::ui::pieces::{ptl, tip};
    let tti = El::text("Hide file extensions", crate::ui::F13, crate::ui::FG(), crate::ui::LH13).none().pad(0.0, 0.0, 2.0, 0.0).margin(0.0, 0.0, -2.0, 0.0);
    let (ra, rr) = (tip::rq(cx, key("w.rq.a"), tip::Rq::Adm, 18.0, tip::texts::ADM, false), tip::rq(cx, key("w.rq.r"), tip::Rq::Rst, 18.0, tip::texts::EXP, true));
    kids.push(at(20.0, 520.0, El::row().center().gap(4.0).child(tti).child(ra).child(rr)));
    kids.push(at(200.0, 520.0, tip::rq(cx, w::RQ_HOVER, tip::Rq::Adm, 18.0, tip::texts::ADM, false)));
    kids.push(at(240.0, 521.0, tip::rq(cx, key("w.rq.i"), tip::Rq::Info, 16.0, "x", false)));
    kids.push(at(380.0, 1730.0, tip::idb(cx, w::IDB_HOVER, "ident", "Identify monitors")));
    kids.push(at(420.0, 1900.0, tip::rq(cx, key("w.rq.ta"), tip::Rq::Adm, 18.0, tip::texts::ADM, false)));
    kids.push(tip::bubble(cx, key("w.rq.ta"), tip::texts::ADM, (420.0, 1900.0, 18.0, 18.0), W, true));
    kids.push(at(520.0, 40.0, tip::rq(cx, key("w.rq.tb"), tip::Rq::Rst, 18.0, "Locked", false)));
    kids.push(tip::bubble(cx, key("w.rq.tb"), "Locked", (520.0, 40.0, 18.0, 18.0), W, true));
    // ---- batch 2: tiles (.pcg grid: .ptl Performance, .dtl Storage drives, .atl Activity)
    let t = |label, name, right, value, extra| ptl::Tile { label, name, right, value, extra, below: vec![] };
    let disk_rq = tip::rq(cx, key("w.pt.rq"), tip::Rq::Info, 16.0, "x", false).margin(-2.0, 0.0, -2.0, 0.0);
    let perf = vec![
        ptl::ptl(cx, ptl::Span::Two, t("CPU", Some("Ryzen 7 7800X3D"), None, Some(("12", Some("%"))), Some(ptl::pcx(cx, &[("4.70", true), (" GHz", false)], false))), false),
        ptl::ptl(cx, ptl::Span::Four, t("GPU", Some("NVIDIA GeForce RTX 4070 SUPER"), Some(ptl::pcq("info", "Driver 581.42")), Some(("38", Some("%"))), Some(ptl::pcx(cx, &[("VRAM ", false), ("7.1", true), (" / 12 GB", false)], false))), false),
        ptl::ptl(cx, ptl::Span::Two, t("RAM", Some("32 GB DDR5"), None, None, Some(ptl::pcx(cx, &[("of 32 \u{b7} 41 %", false)], false))), false),
        ptl::ptl(cx, ptl::Span::Two, t("Disk", Some("C: Samsung 990 PRO"), Some(disk_rq), Some(("3", Some("%"))), Some(ptl::pcx(cx, &[("12 MB/s", false)], false))), false),
        ptl::ptl(cx, ptl::Span::Two, t("Network", Some("Ethernet \u{b7} 1 Gbps"), None, Some(("\u{2193} 4.2", Some("Mb/s"))), None), false),
        ptl::ptl(cx, ptl::Span::Three, t("FPS", Some("Half wide"), None, Some(("144", None)), None), false),
        ptl::ptl(cx, ptl::Span::Three, t("Frame time", None, None, Some(("6.9", Some("ms"))), None), false),
    ];
    kids.push(at(26.0, 600.0, ptl::grid(perf).margin(0.0, 0.0, 0.0, 0.0).w(548.0)));
    let mut drives = Vec::new();
    for (i, (l, n, ic, v, u, x, pct, on, low)) in [
        ("C:", "Windows", "ssd", "412", "GB", "free of 1.82 TB", 0.779, true, false),
        ("D:", "Games", "ssd", "1.21", "TB", "free of 3.64 TB", 0.668, false, false),
        ("E:", "Backup", "hdd", "92.4", "GB", "free of 1.82 TB", 0.95, false, true),
    ]
    .into_iter()
    .enumerate()
    {
        let k = if i == 1 { w::DTL_HOVER } else { crate::ui::el::idx(key("w.dtl"), i) };
        let bar = ptl::dbar(cx, k, pct, low);
        let tile = ptl::Tile { label: l, name: Some(n), right: Some(ptl::dtk(ic)), value: Some((v, Some(u))), extra: Some(ptl::pcx(cx, &[(x, false)], true)), below: vec![bar] };
        drives.push(ptl::dtl(cx, k, ptl::Span::Two, tile, on));
    }
    kids.push(at(26.0, 820.0, El::block().w(548.0).child(ptl::grid(drives))));
    // `.atl .pch>.rq{width:16px;height:16px;margin-top:-2px;margin-bottom:-2px}`
    let up_rq = tip::rq(cx, key("w.at.rq"), tip::Rq::Info, 16.0, "x", false).margin(-2.0, 0.0, -2.0, 0.0);
    let atl = |cx: &Cx, label, right, v, line: &str| {
        let below = vec![ptl::atl_line(line)];
        ptl::ptl(cx, ptl::Span::Two, ptl::Tile { label, name: None, right, value: Some((v, None)), extra: None, below }, true)
    };
    let acts = vec![atl(cx, "Screen time", None, "5 h 12 m", "today \u{b7} since 11:24"), atl(cx, "Games", None, "1 h 40 m", ""), atl(cx, "Uptime", Some(up_rq), "1 d 3 h", "since Mon 18:02")];
    kids.push(at(26.0, 916.0, ptl::grid(acts).margin(0.0, 0.0, 0.0, 0.0).w(548.0)));
    // ---- batch 1 fix (REVIEW_025_b1): the value selected on focus, the step cue at 60 ms, the Custom DNS well field;
    // batch H: one pad glyph PG (El::icon_svg)
    let sel = nbox::Cue { selected: true, ..nbox::Cue::NONE };
    kids.push(at(20.0, 1680.0, nbox::nbox(cx, w::NBOX_SEL, "1920", "", &nbox::NUM, &sel)));
    let step = nbox::Cue { step: Some((cx.now - 60.0, 1)), ..nbox::Cue::NONE };
    kids.push(at(100.0, 1680.0, nbox::nbox(cx, key("w.nbox.step"), "2560", "", &nbox::NUM, &step)));
    kids.push(at(180.0, 1686.0, nbox::nbox(cx, key("w.nbox.xs2"), "800", "", &nbox::XS, &nbox::Cue::NONE)));
    let lab = |t: &str| El::text(t, crate::gfx::Font::new(12.0, 400), crate::ui::FG2(), crate::ui::el::lh(12.0, 1.35)).none().w(62.0);
    let dfr = |l: El, f: El| El::row().center().gap(8.0).margin(0.0, 0.0, 6.0, 0.0).child(l).child(El::block().flex1().child(f));
    let dns = El::col()
        .w(270.0)
        .child(dfr(lab("Primary"), nbox::well(cx, key("w.dns.0"), "1.1.1.1", "Primary", false)))
        .child(dfr(lab("Secondary"), nbox::well(cx, key("w.dns.1"), "", "Secondary", false)))
        .child(dfr(lab("Primary"), nbox::well(cx, key("w.dns.2"), "1.1.1", "Primary", true)))
        .child(dfr(lab("Secondary"), nbox::well(cx, w::DNS_FOCUS, "2606:4700::1111", "Secondary", false)));
    kids.push(at(20.0, 1730.0, dns));
    const PG_MIC: &str = r#"<svg viewBox="0 0 12 12"><rect x="4.4" y="1.6" width="3.2" height="5.4" rx="1.6"/><path d="M2.8 5.8a3.2 3.2 0 0 0 6.4 0M6 9v1.5"/></svg>"#;
    const PG_TRI: &str = r#"<svg viewBox="0 0 12 12"><path d="M6 2.2l4 7H2z"/></svg>"#;
    kids.push(at(540.0, 1684.0, El::block().size(14.0, 14.0).place_center().child(El::icon_svg(PG_MIC, 12.0, 1.4, crate::ui::FG2()))));
    kids.push(at(560.0, 1684.0, El::block().size(14.0, 14.0).place_center().child(El::icon_svg(PG_TRI, 12.0, 1.4, crate::ui::FG2()))));
    // ---- batch 3: the popup menu's rows (.mhead .msep .mitem.cxi .mic .danger .dis .msub em .mr .gt .mi2 .pmh .kb)
    use crate::ui::el::idx;
    use crate::ui::pieces::mitems::{self, It, Lead, Place, Right, Row};
    // menus are placed in WINDOW coordinates (600 x 520): two stand-in windows at y 1030 and 1410
    let (mut w1, mut w2): (Vec<El>, Vec<El>) = (Vec::new(), Vec::new());
    let m1 = key("w.m1");
    let rows1 = [Row::Head("C:\\Program Files\\Spotifast\\Spotifast.exe"), Row::Item(It::icon("fold", "Open file location")), Row::Item(It::icon("globe", "Search online"))];
    w1.push(mitems::menu(cx, m1, &rows1, Place::At(18.0, 8.0), 190.0));
    let m2 = key("w.m2");
    let rows2 = [
        Row::Head("Part of Windows \u{b7} it can\u{2019}t be ended"),
        Row::Item(It::icon("xend", "End task").disabled(true)),
        Row::Item(It::icon("tree", "End process tree").disabled(true)),
        Row::Sep,
        Row::Item(It::icon("fold", "Open file location")),
        Row::Item(It::icon("prio", "Set priority").right(Right::Sub)),
    ];
    w1.push(mitems::menu(cx, m2, &rows2, Place::At(278.0, 8.0), 214.0));
    let m3 = key("w.m3");
    let rows3 = [Row::Head("Spotifast AB \u{b7} version 1.2.48"), Row::Item(It::icon("trash", "Uninstall").danger()), Row::Sep, Row::Item(It::icon("tool", "Repair")), Row::Item(It::icon("undo", "Reset"))];
    w1.push(mitems::menu(cx, m3, &rows3, Place::At(18.0, 198.0), 214.0));
    let m4 = key("w.m4");
    let rows4 = [Row::Item(It::icon("trash", "Delete 3 screenshots").danger()), Row::Item(It::icon("copy", "Copy path"))];
    w1.push(mitems::menu(cx, m4, &rows4, Place::At(278.0, 198.0), 200.0));
    let m5 = key("w.m5");
    let rows5 = [
        Row::Head("DNS for Ethernet"),
        Row::Item(It::tick("Automatic", true).right(Right::Em("from your router"))),
        Row::Item(It::tick("Cloudflare", false).right(Right::Em("1.1.1.1"))),
        Row::Sep,
        Row::Item(It::tick("Custom\u{2026}", false).right(Right::Em("8.8.8.8"))),
    ];
    w2.push(mitems::menu(cx, m5, &rows5, Place::At(18.0, 8.0), 214.0));
    let m6 = key("w.m6");
    let rows6 = [
        Row::Section("Mouse"),
        Row::Item(It::tick("Left click", false).kb(true)),
        Row::Item(It::tick("Right click", false)),
        Row::Section("Controllers"),
        Row::Item(It::tick("Pulsar pad", true).lead(Lead::Mi2("pad")).right(Right::Mr("Connected"))),
        Row::Item(It::tick("Old pad", false).lead(Lead::Mi2("pad")).right(Right::Mr("Off"))),
        Row::Item(It::tick("Elden Ring", false).lead(Lead::Gt("ER", Rgba::hex(0x8a6d3b)))),
        Row::Item(It::tick("Desktop", false).lead(Lead::Gt("D", Rgba::hex(0x3a6df0))).disabled(true)),
    ];
    w2.push(mitems::menu(cx, m6, &rows6, Place::At(278.0, 8.0), 214.0));
    kids.push(at(0.0, 1030.0, El::block().size(crate::ui::WIN_W, crate::ui::WIN_H).children(w1)));
    kids.push(at(0.0, 1410.0, El::block().size(crate::ui::WIN_W, crate::ui::WIN_H).children(w2)));
    let _ = idx;
    // ---- batch 4: the popup confirm .mcf (in the menu box) and the selection bar .selbar, in stand-in windows
    use crate::ui::pieces::{button, selbar};
    let cf1 = mitems::confirm(
        cx,
        key("w.cf1"),
        "This is part of Windows",
        "Windows uses it for sign-in and updates. You can switch it back on any time.",
        "Keep on",
        "Turn off",
        button::Kind::Red,
        Place::At(18.0, 8.0),
        260.0,
    );
    let cf2 = mitems::confirm(
        cx,
        key("w.cf2"),
        "Allow this file?",
        "Defender stops warning about it and restores it to where it was.",
        "Cancel",
        "Allow",
        button::Kind::Primary,
        Place::Under(300.0, -20.0, 90.0, 24.0),
        262.0,
    );
    kids.push(at(0.0, 2010.0, El::block().size(crate::ui::WIN_W, crate::ui::WIN_H).child(cf1).child(cf2)));
    let shots = [selbar::Sbb { icon: "copy", label: "Copy", danger: false }, selbar::Sbb { icon: "trash", label: "Delete", danger: true }];
    let sb1 = selbar::selbar(cx, key("w.sb1"), "3 selected", None, &shots, true);
    kids.push(at(0.0, 2050.0, El::block().size(crate::ui::WIN_W, crate::ui::WIN_H).child(sb1)));
    let apps = [selbar::Sbb { icon: "trash", label: "Uninstall", danger: true }];
    let sb2 = selbar::selbar(cx, key("w.sb2"), "2 selected", Some("4.2 GB"), &apps, true);
    kids.push(at(0.0, 2140.0, El::block().size(crate::ui::WIN_W, crate::ui::WIN_H).child(sb2)));
    // ---- batch 5: row remove x (.rdel, its row hovered), sound preview .pb, chip switch .wps + .mtg
    use crate::ui::pieces::rowbits;
    kids.push(at(20.0, 2710.0, rowbits::rdel(cx, key("w.rd0"), true)));
    kids.push(at(60.0, 2710.0, rowbits::rdel(cx, w::RDEL_HOVER, true)));
    kids.push(at(100.0, 2709.0, rowbits::pb(cx, key("w.pb0"), None, 1, false)));
    kids.push(at(140.0, 2709.0, rowbits::pb(cx, w::PB_HOVER, None, 1, false)));
    kids.push(at(180.0, 2709.0, rowbits::pb(cx, key("w.pb2"), Some(cx.now - 100.0), 1, false)));
    kids.push(at(220.0, 2709.0, rowbits::pb(cx, key("w.pb3"), None, 1, true)));
    kids.push(at(20.0, 2750.0, rowbits::wps(cx, key("w.wps0"), "Show Windows processes", false, rowbits::WpsAt::Header)));
    kids.push(at(220.0, 2750.0, rowbits::wps(cx, key("w.wps1"), "Show Windows processes", true, rowbits::WpsAt::Header)));
    // Timers' `.tho` row is 26 px high and centres the 24 px chip
    kids.push(at(420.0, 2750.0, El::row().h(26.0).center().child(rowbits::wps(cx, w::WPS_HOVER, "On screen", false, rowbits::WpsAt::Timers))));
    kids.push(at(20.0, 2790.0, El::row().h(26.0).center().child(rowbits::wps(cx, key("w.wps3"), "Sound at the end", true, rowbits::WpsAt::Timers))));
    // ---- batch 6: segmented-switch variants (.seg.sm in a .gh .ghr, .prw .seg fit, .seg.acct, .seg.monseg), keycaps,
    // the dialog icon button .cbtn.ic and the game-server .gsgo
    use crate::ui::pieces::{ibtn, segx};
    use segx::Label;
    kids.push(at(20.0, 2850.0, segx::seg_ex(cx, key("w.sx0"), &[Label::Text("Today"), Label::Text("7 days")], Some(0), &segx::SM)));
    kids.push(at(160.0, 2850.0, segx::seg_ex(cx, key("w.sx1"), &[Label::Text("Circle"), Label::Text("Cross"), Label::Text("Square")], Some(1), &segx::PRW)));
    kids.push(at(380.0, 2850.0, El::row().w(200.0).child(segx::seg_ex(cx, key("w.sx2"), &[Label::Text("Input"), Label::Text("Output"), Label::Text("Both")], Some(2), &segx::ACCT))));
    let mons = [Label::Mon("1", "DELL 27\u{2033}"), Label::Mon("2", "LG 24\u{2033}")];
    kids.push(at(20.0, 2890.0, segx::seg_ex(cx, key("w.sx3"), &mons, Some(0), &segx::MONSEG)));
    kids.push(at(300.0, 2890.0, segx::seg_ex(cx, key("w.sx4"), &[Label::Text("800"), Label::Text("1600"), Label::Text("3200")], None, &segx::PRW)));
    kids.push(at(20.0, 2945.0, ibtn::keycap("Ctrl", &ibtn::CAP, false)));
    kids.push(at(70.0, 2946.0, ibtn::keycap("Shift", &ibtn::CAP_SM, false)));
    kids.push(at(120.0, 2946.0, ibtn::keycap("F13", &ibtn::CAP_SM, true)));
    kids.push(at(170.0, 2940.0, ibtn::keycap("Alt", &ibtn::CAP_BIG, false)));
    kids.push(at(230.0, 2946.0, ibtn::keycap("Q", &ibtn::CAP_XS, false)));
    // `.menu.padm .tsrch{width:100%}` (240 px inside the 252 px popup): Lane K's search field with its width set
    kids.push(at(300.0, 2940.0, crate::ui::pieces::search::search(cx, key("w.srch240"), "", "Search a button, key or click", false).w(240.0)));
    kids.push(at(20.0, 2990.0, ibtn::icbtn(cx, key("w.ib0"), "open", "Open", false)));
    kids.push(at(130.0, 2990.0, ibtn::icbtn(cx, w::IB_HOVER, "fold", "Change", true)));
    kids.push(at(250.0, 2990.0, ibtn::icbtn(cx, key("w.ib2"), "plus12", "Add game", true)));
    kids.push(at(380.0, 2992.0, ibtn::gsgo(cx, key("w.gs0"), false)));
    kids.push(at(480.0, 2992.0, ibtn::gsgo(cx, key("w.gs1"), true)));
    // ---- batch 4 fix (REVIEW_025_b4 remark 5): the .sbb pressed state and a .mcf title cut by its ellipsis
    let sb3 = selbar::selbar(cx, key("w.sb3"), "3 selected", None, &shots, true);
    // (at the gallery's bottom: its 0 14px 36px shadow reaches ~50 px around - REVIEW_025_b6 remark 1)
    kids.push(at(0.0, 3594.0, El::block().size(crate::ui::WIN_W, crate::ui::WIN_H).child(sb3)));
    let cf3 = mitems::confirm(
        cx,
        key("w.cf3"),
        "Allow Trojan:Win32/Wacatac.B!ml in setup_installer_x64.exe?",
        "Defender stops warning about it and restores it to where it was.",
        "Cancel",
        "Allow",
        button::Kind::Primary,
        Place::Under(300.0, -20.0, 90.0, 24.0),
        262.0,
    );
    kids.push(at(0.0, 3160.0, El::block().size(crate::ui::WIN_W, crate::ui::WIN_H).child(cf3)));
    // ---- batch 6: CSS filter colour functions (El::color_filter) - Audio's muted app tile `.mxr.mu .ait{filter:grayscale(1);
    // opacity:.42}`: the tile plain, grayscale only, grayscale + opacity
    use crate::gfx::CssColor;
    use crate::ui::pieces::listrow::{tile, Tile};
    let ait = || tile(&Tile::Glyph { glyph: "note", a: Rgba::rgba(255, 95, 87, 1.0), b: Rgba::rgba(255, 45, 85, 1.0) }, 24.0);
    kids.push(at(20.0, 3300.0, ait()));
    kids.push(at(60.0, 3300.0, ait().color_filter(CssColor::Grayscale(1.0))));
    kids.push(at(100.0, 3300.0, ait().color_filter(CssColor::Grayscale(1.0)).opacity(0.42)));
    // ---- batch 7: the other text inputs (.kdin, .thn, .thd, .apin) and the slim inner scrollbar (.rsl2)
    use crate::ui::pieces::tinput;
    kids.push(at(20.0, 3340.0, tinput::kdin(cx, key("w.kd0"), "", "youtube.com", 300.0)));
    kids.push(at(20.0, 3385.0, tinput::kdin(cx, w::KDIN_F, "brb 5 min", "brb 5 min", 300.0)));
    kids.push(at(340.0, 3340.0, El::row().child(tinput::thn(cx, key("w.thn0"), "Pasta", false, false))));
    kids.push(at(340.0, 3372.0, El::row().child(tinput::thn(cx, w::THN_H, "Pasta", false, false))));
    kids.push(at(340.0, 3404.0, El::row().child(tinput::thn(cx, w::THN_F, "Pasta", true, false))));
    kids.push(at(20.0, 3440.0, tinput::thd(cx, key("w.thd0"), "5:00", 300.0, false, false, None)));
    kids.push(at(20.0, 3515.0, tinput::thd(cx, w::THD_F, "12:30", 300.0, true, false, None)));
    kids.push(at(360.0, 3460.0, El::row().child(tinput::apin(cx, w::APIN_F, "Gaming", true))));
    kids.push(at(460.0, 3460.0, El::row().child(tinput::apin(cx, key("w.ap1"), "Quake", false))));
    // `.rsl2` (max-height 196, margin 0 -6px, padding 0 6px) holding 20 rows of 20 px, scrolled 0 / 100 px: the lane takes 9 px
    // of the width while the list overflows (a classic scrollbar) - the rows end 9 px earlier
    let rsl2 = |off: f32| {
        let rows: Vec<El> = (0..20).map(|i| El::block().h(20.0).bg(if i % 2 == 1 { Rgba(1.0, 1.0, 1.0, 0.06) } else { Rgba(0.0, 0.0, 0.0, 0.0) })).collect();
        El::block().w(260.0).child(
            El::col().h(196.0).margin(0.0, -6.0, 0.0, -6.0).pad(0.0, 15.0, 0.0, 6.0).clip().slim_thumb(crate::ui::el::SlimThumb::GLASS).child(El::col().translate(0.0, -off).children(rows)),
        )
    };
    kids.push(at(20.0, 3600.0, rsl2(0.0)));
    kids.push(at(320.0, 3600.0, rsl2(100.0)));
    // REVIEW_025_b4 remark 3 (the cause of "selbar pressed"): the same pressed Delete drawn alone, outside the glass bar
    kids.push(at(20.0, 3830.0, selbar::sbb(cx, w::SBB_ALONE, Some("Delete"), "trash", true, false, true)));
    // ---- batch 6 fix (REVIEW_025_b6): Storage's seg in a .ghr, Mouse's DPI seg with nothing picked (base .seg), the keycap in
    // Controller's .ach chip, hover of gsgo Start / Stop and the plain .cbtn.ic, the .lockb grayscale subtree
    kids.push(at(20.0, 3900.0, segx::seg_ex(cx, key("w.sx5"), &[Label::Text("File types"), Label::Text("Folders")], Some(0), &segx::GHR)));
    kids.push(at(300.0, 3900.0, segx::seg_ex(cx, key("w.sx6"), &[Label::Text("800"), Label::Text("1600"), Label::Text("3200")], None, &segx::BASE)));
    kids.push(at(20.0, 3944.0, ibtn::keycap("Ctrl", &ibtn::CAP_SM_BTN, false)));
    kids.push(at(100.0, 3940.0, ibtn::gsgo(cx, w::GS_START_H, false)));
    kids.push(at(200.0, 3940.0, ibtn::gsgo(cx, w::GS_STOP_H, true)));
    kids.push(at(300.0, 3938.0, ibtn::icbtn(cx, w::IB_OPEN_H, "open", "Open", false)));
    let lockb = El::row().center().gap(8.0).child(ait()).child(El::text("Push to talk", crate::gfx::Font::new(13.0, 400), crate::ui::FG(), crate::ui::el::lh(13.0, 1.35)));
    kids.push(at(20.0, 3980.0, lockb.color_filter(CssColor::Grayscale(1.0)).opacity(0.36)));
    // ---- batch 8: spinner, add button, app tile, link with its parent's weight, the group header's grey run, the fold card
    // head (.ch.ex + .fcnt + .cx), Apps' uninstall confirm (.dlg.udlg); + the selected countdown digits (REVIEW_025_b7 HOLD 1)
    use crate::ui::pieces::{bits, fold, udlg};
    let now = cx.now;
    kids.push(at(20.0, 4200.0, bits::uspin(cx, now)));
    kids.push(at(66.0, 4196.0, El::row().child(bits::addb(cx, key("w.addb0"), "Add app", false))));
    kids.push(at(186.0, 4196.0, El::row().child(bits::addb(cx, w::ADDB_H, "Add a key", false))));
    kids.push(at(300.0, 4196.0, El::row().child(bits::addb(cx, key("w.addb2"), "Save as preset", true))));
    let (ta, tb) = (Rgba::rgba(90, 200, 250, 1.0), Rgba::rgba(0, 122, 255, 1.0));
    kids.push(at(20.0, 4240.0, bits::at("note", ta, tb, false)));
    kids.push(at(60.0, 4240.0, bits::at("note", ta, tb, true)));
    kids.push(at(100.0, 4240.0, bits::lnk(cx, key("w.lnk0"), "Open web settings", 12.0, 600, 16.0)));
    kids.push(at(240.0, 4240.0, bits::lnk(cx, w::LNK_H, "Open web settings", 12.0, 600, 16.0)));
    kids.push(at(20.0, 4280.0, group::gh("Switch automatically").child(bits::ghs("while an app is running")).margin(0.0, 0.0, 0.0, 0.0)));
    let fcard = |h: El| crate::ui::pieces::group::grp(vec![h]).w(548.0).clip();
    let c0 = fold::chev(cx, key("w.cx0"), false, false);
    let h0 = fold::head(cx, key("w.fold0"), "secu", "Threats found", vec![], "Nothing found in the last scan", vec![fold::fcnt("0", false), c0], true, false);
    kids.push(at(26.0, 4310.0, fcard(h0)));
    let c1 = fold::chev(cx, w::CX_H, false, false);
    let h1 = fold::head(cx, w::FOLD_H, "secu", "Threats found", vec![], "2 need a look", vec![fold::fcnt("2", true), c1], true, true);
    kids.push(at(26.0, 4378.0, fcard(h1)));
    let c2 = fold::chev(cx, key("w.cx2"), true, false);
    let h2 = fold::head(cx, key("w.fold2"), "lock", "Quarantine", vec![], "3 files kept safe", vec![fold::fcnt("3", false), c2], true, false);
    kids.push(at(26.0, 4446.0, fcard(h2)));
    kids.push(at(20.0, 4520.0, El::row().child(tinput::thd(cx, w::THD_SEL, "12:30", 300.0, true, true, None))));
    let rows = [
        udlg::UdRow { tile: Tile::Glyph { glyph: "note", a: Rgba::rgba(255, 95, 87, 1.0), b: Rgba::rgba(255, 45, 85, 1.0) }, name: "Spotifast", size: "1.2 GB" },
        udlg::UdRow { tile: Tile::Glyph { glyph: "note", a: ta, b: tb }, name: "Pasta Player", size: "240 MB" },
    ];
    let foot = vec![
        button::cbtn_sized(cx, key("w.ud.no"), "Cancel", button::Kind::Ghost, button::DFT, false, 76.0),
        button::cbtn_sized(cx, key("w.ud.go"), "Uninstall", button::Kind::Red, button::DFT, false, 76.0),
    ];
    let ud = udlg::udlg(cx, key("w.ud"), "Uninstall 3 apps?", "Frees about 1.4 GB. An app\u{2019}s own uninstaller may open: finish it there.", &rows, 1, Some("Spotifast: keeps your playlists."), foot, -10000.0, None);
    kids.push(at(0.0, 4600.0, El::block().size(crate::ui::WIN_W, crate::ui::WIN_H).child(ud)));
    El::block().size(W, WH).bg(BG).children(kids)
}

/// The second gallery's height (its own: more pieces than Lane K's gallery holds).
pub const WH: f32 = 5140.0;

/// Paint the second gallery at `scale`. Chromium can force :focus-within on many elements at once, the app has one
/// focus: the gallery is painted once per focused key and each focused entry's box is taken from its own pass.
pub fn render_w(scale: f32) -> Option<crate::png::Pixels> {
    let g = Gfx::new(scale);
    let icons = Icons::new();
    let mut s = crate::gfx::new_surface((W * scale).round() as i32, (WH * scale).round() as i32)?;
    g.begin(s.canvas());
    g.fill_rect(0.0, 0.0, W, WH, BG);
    g.end();
    let base = s.image_snapshot();
    // pass 1: everything, no focus
    let paint = |focus: Option<crate::ui::el::Key>, clip: Option<(f32, f32, f32, f32)>, s: &mut skia_safe::Surface| {
        let mut st = State::default();
        st.hover = w::hovered();
        st.focus = focus;
        st.active = w::active();
        let mut cx = Cx::new(600.0, false, &g, &mut st);
        let root = build_w(&mut cx);
        let laid = Laid::new(&g, root, W, Some(WH));
        g.begin(s.canvas());
        if let Some((x, y, w, h)) = clip {
            g.push_clip(x, y, w, h);
            g.fill_rect(x, y, w, h, BG);
        }
        laid.paint(&g, &icons, 0.0, 0.0, Some(&base));
        if clip.is_some() {
            g.pop_clip();
        }
        g.end();
        laid
    };
    let laid = paint(None, None, &mut s);
    // test proofs: every laid box (to compare with tools/ref/gallery_w.js --dump) - test builds only
    #[cfg(test)]
    if let Ok(p) = std::env::var("BU_GALLERY_W_DUMP") {
        let lines: Vec<String> = laid.debug_boxes().iter().map(|b| format!("{:.3}\t{:.3}\t{:.3}\t{:.3}\t{}", b.0, b.1, b.2, b.3, b.4)).collect();
        let _ = std::fs::write(p, lines.join("\n"));
    }
    // pass 2..: each focused entry repainted (its box + 6 px for the ring) with that key focused
    for k in w::focused() {
        if let Some((x, y, w, h)) = laid.rect_of(k) {
            paint(Some(k), Some((x - 6.0, y - 6.0, w + 12.0, h + 12.0)), &mut s);
        }
    }
    Some(crate::png::from_surface(&mut s))
}

#[cfg(test)]
mod w_tests {
    /// Writes the second gallery's picture to $BU_GALLERY_W (the pixel proof of Order 025's pieces).
    #[test]
    #[ignore]
    fn gallery_w_png() {
        let out = std::env::var("BU_GALLERY_W").expect("set BU_GALLERY_W=<png>");
        let sc: f32 = std::env::var("BU_GALLERY_W_SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
        let p = super::render_w(sc).expect("surface");
        // the PNG writer is WIC (COM)
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        crate::png::save_png(&p, &out).expect("save");
    }

    /// Writes Lane K's (first) gallery to $BU_GALLERY_K (scene 1; scenes 2-4 next to it as <name>_s<n>.png) - Order 025's
    /// proof that its additive helpers change nothing that existed: the same test on the base commit must give the same bytes.
    #[test]
    #[ignore]
    fn gallery_k_png() {
        let out = std::env::var("BU_GALLERY_K").expect("set BU_GALLERY_K=<png>");
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        for scene in 1..=4u32 {
            let p = super::render(1.0, scene).expect("surface");
            let name = if scene == 1 { out.clone() } else { out.replace(".png", &format!("_s{scene}.png")) };
            crate::png::save_png(&p, &name).expect("save");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene4_focused_search_is_laid_out() {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        st.focus = Some(key("f.q0"));
        let mut cx = Cx::new(SCENE4_NOW, false, &g, &mut st);
        let root = build_scene(&mut cx, 4);
        let l = Laid::new(&g, root, W, Some(H));
        let r = l.rect_of(key("f.q0"));
        assert_eq!(r, Some((390.0, 400.0, 196.0, 28.0)), "{r:?}");
    }
}

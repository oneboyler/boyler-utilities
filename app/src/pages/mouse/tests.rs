//! The Mouse page against the FAKE (the drawing's sample PC): what it shows and every action it sends.

use super::*;
use crate::gfx::Gfx;
use crate::ui::cx::State;
use crate::undo::{Kind, LineResult, Outcome, Record, Resettable, Review, Val};

/// The app's services for one test (a scratch settings store: the change log), stopped at the end.
struct Sv;
impl Sv {
    fn start() -> Sv {
        crate::services::init(windows::Win32::Foundation::HWND::default(), true);
        Sv
    }
}
impl Drop for Sv {
    fn drop(&mut self) {
        crate::services::shutdown();
    }
}

/// The Mouse tab's entry for an item.
fn rec(item: &str) -> Option<Record> {
    crate::services::with(|s| {
        crate::undo::flush(&mut s.store);
        crate::undo::read_record(&s.store, "cur", item)
    })
    .flatten()
}

/// The frame's review for this tab.
fn review(m: &Mouse, kind: Kind) -> Review {
    crate::services::with(|s| Review::for_page(kind, m, &s.store)).unwrap()
}

/// The frame's "Reset": the ticked lines through the page (the open tab's worker / a closed tab's own service), each ok
/// recorded; then the tab's view catches up.
fn reset(m: &mut Mouse, rv: &Review) -> Vec<LineResult> {
    let res = crate::services::with(|s| rv.apply(&mut s.store, &mut [&mut *m as &mut dyn Resettable])).unwrap();
    settle(m);
    res
}

fn opened() -> Mouse {
    let mut m = Mouse::default();
    m.open(&Env { test: true, frozen: true, ..Env::default() }, 0.0);
    settle(&mut m);
    m
}

fn settle(m: &mut Mouse) {
    let rs = m.svc.as_mut().map(|s| s.settle(5000)).unwrap_or_default();
    for r in rs {
        m.take(r, 0.0);
    }
}

fn with_cx(f: impl FnOnce(&mut Cx)) {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(1000.0, false, &g, &mut st).for_page("cur");
    f(&mut cx);
}

fn click(m: &mut Mouse, k: Key) {
    with_cx(|cx| m.event(&Ev::Click(k), cx));
    settle(m);
}

/// A drag on a slider of box `r` to `frac` of its way (the thumb runs from r.x + 8 to r.x + r.w - 8), then let go.
fn drag(m: &mut Mouse, k: Key, r: (f32, f32, f32, f32), frac: f32) {
    with_cx(|cx| {
        m.event(&Ev::Press(k, r.0 + 8.0 + (r.2 - 16.0) * frac, r.1 + 10.0, r), cx);
        m.event(&Ev::Release(k), cx);
    });
    settle(m);
}

fn typed(m: &mut Mouse, k: Key, text: &str, vk_after: u16) {
    with_cx(|cx| {
        for c in text.chars() {
            m.event(&Ev::Char(k, c), cx);
        }
        m.event(&Ev::Key(k, vk_after), cx);
    });
    settle(m);
}

fn toast(m: &Mouse) -> Option<&str> {
    m.toast.as_ref().map(|t| t.0.as_str())
}

/// The page laid out and painted off-screen with the app's painter (a debugging / layout check; no screen).
fn render_page(m: &mut Mouse, out: Option<&str>) -> crate::ui::lay::Laid {
    let g = Gfx::new(1.0);
    let icons = crate::icons::Icons::new();
    let mut st = State::default();
    let mut cx = Cx::new(0.0, false, &g, &mut st);
    let kids = m.build(&mut cx);
    let root = El::block().w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids);
    let laid = crate::ui::lay::Laid::new(&g, root, 600.0, None);
    if let Some(p) = out {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let h = laid.height.ceil() as i32;
        let mut s = crate::gfx::new_surface(600, h).unwrap();
        g.begin(s.canvas());
        g.fill_rect(0.0, 0.0, 600.0, h as f32, Rgba::rgb(20, 24, 40));
        laid.paint(&g, &icons, 0.0, 0.0, None);
        g.end();
        let px = crate::png::from_surface(&mut s);
        crate::png::save_png(&px, p).expect("save");
    }
    laid
}

const SPEED_BOX: (f32, f32, f32, f32) = (334.0, 451.0, 168.0, 20.0);
const SIZE_BOX: (f32, f32, f32, f32) = (402.0, 681.0, 132.0, 20.0);

#[test]
fn graph_matches_raw_accels_window() {
    let m = opened();
    let p = m.graph_points();
    for (x, want) in [(0.0, 1.0), (60.0, 1.13), (80.0, 1.50), (100.0, 1.72), (120.0, 1.86)] {
        let i = (x / 0.5) as usize;
        assert_eq!(format!("{:.2}", p[i].1), format!("{want:.2}"), "at {x}");
    }
}

#[test]
fn open_card_lays_out() {
    let mut m = opened();
    m.panel.expanded = true;
    let out = std::env::var("BU_Q_RENDER").ok();
    let laid = render_page(&mut m, out.as_deref());
    assert!(laid.height.is_finite() && laid.height > 800.0, "height {}", laid.height);
    for n in &laid.nodes {
        assert!(n.rect.0.is_finite() && n.rect.1.is_finite() && n.rect.2.is_finite() && n.rect.3.is_finite(), "NaN box");
    }
}

#[test]
fn opens_with_the_drawings_sample_and_changes_nothing() {
    let _s = Sv::start();
    let m = opened();
    let w = m.v.win.unwrap();
    assert_eq!((w.pointer_speed, w.precision, w.double_click_ms, w.buttons_swapped), (10, false, 500, false));
    assert_eq!(m.mouse_dpi(), Some(1600));
    let on = m.v.on_mouse.as_ref().unwrap();
    assert_eq!((on.polling_hz, on.lift_off, on.battery_percent), (Some(1000), Some(10), Some(78)));
    assert_eq!(header_line(&m.panel, &m.per_app), "VALORANT: Valorant · off everywhere else");
    assert_eq!(m.panel.presets.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Valorant", "Default"]);
    assert!(m.panel.on && !m.panel.expanded);
    // opening only reads: nothing in the change log; every item's value is read (Raw Accel's fake driver too)
    assert!(crate::services::with(|s| crate::undo::records(&s.store, Some("cur"))).unwrap().is_empty());
    assert_eq!(m.v.vals.iter().map(|(i, _)| i.as_str()).collect::<Vec<_>>(), svc::ITEMS.map(|(i, _)| i));
    assert_eq!(m.v.cursors.as_ref().unwrap().size, 1);
}

#[test]
fn dpi_polling_lift_off_go_to_the_mouse_and_stay_out_of_the_change_log() {
    let _s = Sv::start();
    let mut m = opened();
    click(&mut m, idx(K_DPI, 1));
    assert_eq!(m.mouse_dpi(), Some(800));
    assert_eq!(toast(&m), Some("DPI 800 · saved on the mouse"));
    click(&mut m, idx(K_HZ, 2));
    assert_eq!(m.v.on_mouse.as_ref().unwrap().polling_hz, Some(4000));
    click(&mut m, idx(K_LOD, 1));
    assert_eq!(m.v.on_mouse.as_ref().unwrap().lift_off, Some(20));
    // a typed DPI: the nearest value the mouse stores (10-DPI steps up to 10240; Order 042)
    typed(&mut m, K_DPIN, "1234", 0x0D);
    assert_eq!(m.mouse_dpi(), Some(1230));
    // Order 045 item 4: the wheel over the box = ±50 (as its arrows)
    with_cx(|cx| m.event(&Ev::Wheel(K_DPIN, 1), cx));
    settle(&mut m);
    assert_eq!(m.mouse_dpi(), Some(1280));
    with_cx(|cx| m.event(&Ev::Wheel(K_DPIN, -1), cx));
    settle(&mut m);
    assert_eq!(m.mouse_dpi(), Some(1230));
    // saved on the mouse itself, not in Windows (the drawing's RS.cur lists none): nothing in the change log
    assert!(crate::services::with(|s| crate::undo::records(&s.store, Some("cur"))).unwrap().is_empty());
}

/// Order 036: every Windows mouse setting writes ONE entry with its value before the FIRST change; the reset puts the
/// ticked ones back through the open tab's worker; an unticked one stays (the fake PC only).
#[test]
fn windows_settings_go_into_the_change_log_and_back() {
    let _s = Sv::start();
    let mut m = opened();
    drag(&mut m, K_SPEED, SPEED_BOX, 1.0);
    drag(&mut m, K_SPEED, SPEED_BOX, 0.0);
    assert_eq!(m.v.win.unwrap().pointer_speed, 1);
    click(&mut m, K_SWAP);
    drag(&mut m, K_DBL, SPEED_BOX, 1.0);
    drag(&mut m, K_LINES, SPEED_BOX, 0.0);
    let r = rec("speed").expect("one entry");
    assert_eq!((r.label.as_str(), r.was.raw.as_str(), r.now.raw.as_str()), ("Pointer speed", "10", "1"), "the value before the FIRST change");
    assert!(rec("epp").is_none(), "only what changed");
    let mut rv = review(&m, Kind::HowItWas);
    let mut got: Vec<String> = rv.lines.iter().map(|l| format!("{}: {}", l.label, l.change_text())).collect();
    got.sort();
    assert_eq!(
        got,
        [
            "Double-click speed: 200 ms  →  500 ms",
            "Pointer speed: 1  →  10",
            "Scroll lines: 1 line  →  3 lines",
            "Swap primary button: On  →  Off"
        ]
    );
    // untick the swap: kept
    let i = rv.lines.iter().position(|l| l.item == "swap").unwrap();
    rv.toggle(i);
    assert!(reset(&mut m, &rv).iter().all(|r| r.outcome == Outcome::Ok));
    let w = m.v.win.unwrap();
    assert_eq!((w.pointer_speed, w.double_click_ms, w.buttons_swapped), (10, 500, true));
    assert_eq!(w.scroll_lines, ScrollLines::Lines(3));
    let left: Vec<String> = review(&m, Kind::HowItWas).lines.iter().map(|l| l.item.clone()).collect();
    assert_eq!(left, ["swap"]);
}

#[test]
fn windows_defaults_come_from_the_tab() {
    let _s = Sv::start();
    let mut m = opened();
    drag(&mut m, K_SPEED, SPEED_BOX, 1.0);
    assert_eq!(m.v.win.unwrap().pointer_speed, 20);
    click(&mut m, K_SWAP);
    assert!(m.v.win.unwrap().buttons_swapped);
    assert_eq!(toast(&m), Some("Right button is now your main button"));
    drag(&mut m, K_DBL, SPEED_BOX, 1.0);
    assert_eq!(m.v.win.unwrap().double_click_ms, 200);
    drag(&mut m, K_LINES, SPEED_BOX, 0.0);
    assert_eq!(m.v.win.unwrap().scroll_lines, ScrollLines::Lines(1));
    // Windows defaults: speed 10, precision on (the sample has it off), 3 lines, 500 ms, swap off; cursors / size already
    let mut rv = review(&m, Kind::WindowsDefaults);
    let def: Vec<String> = rv.lines.iter().map(|l| format!("{}: {}", l.label, l.change_text())).collect();
    assert_eq!(
        def,
        [
            "Pointer speed: 20  →  10",
            "Enhance pointer precision: Off  →  On",
            "Scroll lines: 1 line  →  3 lines",
            "Double-click speed: 200 ms  →  500 ms",
            "Swap primary button: On  →  Off"
        ]
    );
    // untick precision, reset the rest
    rv.toggle(1);
    assert_eq!(reset(&mut m, &rv).len(), 4);
    let w = m.v.win.unwrap();
    assert_eq!((w.pointer_speed, w.precision, w.double_click_ms, w.buttons_swapped), (10, false, 500, false));
    assert_eq!(w.scroll_lines, ScrollLines::Lines(3));
    assert_eq!(review(&m, Kind::WindowsDefaults).lines.len(), 1, "only the unticked precision is left");
}

/// Raw Accel: the app's first write to the driver keeps a copy of what it ran (the entry's old value names it); the
/// reset hands that copy back to the (fake) driver.
#[test]
fn raw_accel_goes_into_the_change_log_and_back() {
    let _s = Sv::start();
    let mut m = opened();
    // "Everywhere else": Valorant - the driver now runs a curve
    click(&mut m, K_ELSE);
    click(&mut m, idx(K_MENU, 2));
    let r = rec("accel").expect("one entry");
    assert_eq!((r.label.as_str(), r.was.text.as_str(), r.now.text.as_str()), ("Mouse acceleration", "Off", "On"));
    assert!(r.was.raw.ends_with(".bin") && r.was.raw.contains(r"rawaccel\before"), "{}", r.was.raw);
    let rv = review(&m, Kind::HowItWas);
    assert_eq!(rv.lines.iter().map(|l| l.change_text()).collect::<Vec<_>>(), ["On  →  Off"]);
    assert!(reset(&mut m, &rv).iter().all(|r| r.outcome == Outcome::Ok));
    assert_eq!(m.v.vals.iter().find(|(i, _)| i == "accel").map(|(_, v)| v.raw.clone()), Some(r.was.raw.clone()), "the driver runs its copy again");
    assert!(review(&m, Kind::HowItWas).is_empty());
    // Raw Accel has no Windows default
    assert!(!review(&m, Kind::WindowsDefaults).lines.iter().any(|l| l.item == "accel"));
}

/// A tab that was never opened (Settings › Reset, the uninstaller): `resettable()` makes nothing; the first question
/// makes its own service (the fake in tests) and the reset goes through it.
#[test]
fn a_closed_tab_resets_through_its_own_service() {
    let _s = Sv::start();
    let mut m = Mouse::default();
    assert!(m.resettable().is_some());
    assert!(m.cold.lock().unwrap().is_none() && m.svc.is_none(), "nothing made by resettable()");
    crate::services::with(|s| crate::undo::record(&mut s.store, "cur", "speed", "Pointer speed", &Val::plain("8"), &Val::plain("12"))).unwrap().unwrap();
    let rv = review(&m, Kind::HowItWas);
    // its value now is read (the fake PC: 10), not the last recorded one
    assert_eq!(rv.lines.iter().map(|l| l.change_text()).collect::<Vec<_>>(), ["10  →  8"]);
    assert!(reset(&mut m, &rv).iter().all(|r| r.outcome == Outcome::Ok));
    assert!(review(&m, Kind::HowItWas).is_empty(), "back: the line is gone");
    let d: Vec<String> = review(&m, Kind::WindowsDefaults).lines.iter().map(|l| format!("{}: {}", l.label, l.change_text())).collect();
    assert_eq!(d, ["Pointer speed: 8  →  10", "Enhance pointer precision: Off  →  On"]);
}

/// The reset line's links open the frame's review (over the change log) under the link's box; the page shows none itself.
#[test]
fn the_reset_links_open_the_frames_review() {
    let mut m = opened();
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(1000.0, false, &g, &mut st).for_page("cur");
    let b = (220.0, 1267.0, 132.0, 16.0);
    for (k, kind) in [(sub(K_RS, "pc"), Kind::HowItWas), (sub(K_RS, "win"), Kind::WindowsDefaults)] {
        cx.reqs.clear();
        m.event(&Ev::Press(k, 230.0, 1270.0, b), &mut cx);
        m.event(&Ev::Click(k), &mut cx);
        assert!(matches!(cx.reqs.as_slice(), [crate::ui::cx::Req::Reset(k2, a)] if *k2 == kind && *a == b), "{kind:?}");
        assert!(m.reset.is_none());
    }
}
#[test]
fn precision_warning_shows_and_its_link_turns_it_off() {
    let mut m = opened();
    click(&mut m, K_EPP);
    assert!(m.v.win.unwrap().precision);
    m.panel.expanded = true;
    let laid = render_page(&mut m, None);
    assert!(laid.nodes.iter().any(|n| matches!(&n.el.content, crate::ui::el::Content::Text(t) if t.s.contains("so the two stack"))));
    click(&mut m, K_EPPOFF);
    assert!(!m.v.win.unwrap().precision);
    let laid = render_page(&mut m, None);
    assert!(!laid.nodes.iter().any(|n| matches!(&n.el.content, crate::ui::el::Content::Text(t) if t.s.contains("so the two stack"))));
}

#[test]
fn acceleration_card_switch_curve_presets_per_app() {
    let mut m = opened();
    // the head opens / closes the card; the switch turns it off (plain 1:1) and on again
    click(&mut m, K_ACH);
    assert!(m.panel.expanded);
    click(&mut m, K_ACT);
    assert!(!m.panel.on);
    assert_eq!(toast(&m), Some("Acceleration off · plain 1:1 everywhere"));
    click(&mut m, K_ACT);
    assert!(m.panel.on && m.panel.expanded);
    // the curve popup: Natural (Linear keeps its values)
    click(&mut m, K_MODE);
    assert_eq!(m.menu, Some(Menu::Curve));
    click(&mut m, idx(K_MENU, 2));
    assert_eq!(m.panel.curve, Curve::Natural);
    assert!(m.panel.changed_since_loaded(), "the amber dot + Update");
    // loading Valorant again: Linear 2.8 / 55 / 2.6
    click(&mut m, idx(K_CHIP, 0));
    assert_eq!(m.panel.curve, Curve::Linear);
    assert_eq!(m.vals().get(Field::Acceleration), 2.8);
    assert_eq!(toast(&m), Some("Loaded Valorant"));
    // a slider (sent when let go): Acceleration to the far left = 0.05 (Raw Accel refuses 0)
    drag(&mut m, idx(K_ASL, field_ix(Field::Acceleration)), (174.0, 489.0, 116.0, 20.0), 0.0);
    assert_eq!(m.vals().get(Field::Acceleration), 0.05);
    assert_eq!(m.v.panel.current_values().get(Field::Acceleration), 0.05, "the worker got it");
    // Save as preset: "Preset 3", renamed inline to "Fast"
    click(&mut m, K_SAVE);
    assert_eq!(m.rename.as_ref().map(|r| r.1.as_str()), Some("Preset 3"));
    with_cx(|cx| {
        for _ in 0..8 {
            m.event(&Ev::Key(0, 0x08), cx);
        }
        for c in "Fast".chars() {
            m.event(&Ev::Char(0, c), cx);
        }
        m.event(&Ev::Key(0, 0x0D), cx);
    });
    settle(&mut m);
    assert_eq!(m.panel.presets.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Valorant", "Default", "Fast"]);
    // the VALORANT row -> Fast; deleting Fast turns the row Off
    click(&mut m, sub(idx(K_ROW, 0), "pre"));
    assert!(matches!(m.menu, Some(Menu::RowPreset(_))));
    click(&mut m, idx(K_MENU, 2));
    assert_eq!(header_line(&m.panel, &m.per_app), "VALORANT: Fast · off everywhere else");
    click(&mut m, sub(idx(K_CHIP, 2), "del"));
    assert_eq!(header_line(&m.panel, &m.per_app), "VALORANT: Off · off everywhere else");
    assert_eq!(toast(&m), Some("Deleted Fast · VALORANT now Off"));
    // Add app -> the app picker opens at once; Rocket League
    click(&mut m, K_ADD);
    assert!(matches!(m.menu, Some(Menu::RowApp(_))));
    click(&mut m, idx(K_MENU, 4));
    assert_eq!((m.per_app.rows()[1].label.as_str(), m.per_app.rows()[1].exe.as_str()), ("Rocket League", "RocketLeague.exe"));
    // Everywhere else -> Default; the row's × removes it
    click(&mut m, K_ELSE);
    click(&mut m, idx(K_MENU, 3));
    assert!(header_line(&m.panel, &m.per_app).ends_with("everywhere else: Default"));
    click(&mut m, sub(idx(K_ROW, 1), "x"));
    assert_eq!(m.per_app.rows().len(), 1);
    // the worker has the card as the page left it (what it hands Raw Accel's driver)
    assert_eq!(m.v.per_app.rows().len(), 1);
    assert_eq!(m.v.panel.presets.len(), 2);
    // Esc / a click beside a list closes it
    click(&mut m, K_MODE);
    m.popup_dismiss();
    assert_eq!(m.menu, None);
}

#[test]
fn cursor_size_goes_into_the_change_log_and_back() {
    let _s = Sv::start();
    let mut m = opened();
    drag(&mut m, K_SIZE, SIZE_BOX, 1.0 / 14.0);
    assert_eq!(m.v.cursors.as_ref().unwrap().size, 2);
    let r = rec("cursor_size").expect("one entry");
    assert_eq!((r.was.raw.as_str(), r.was.text.as_str(), r.now.text.as_str()), ("32,1", "1", "2"));
    let d = review(&m, Kind::WindowsDefaults);
    assert!(d.lines.iter().any(|l| l.label == "Cursor size" && l.change_text() == "2  →  1"), "{:?}", d.lines);
    // the Link bubble's picker: its own set again (Windows default) writes nothing and logs nothing
    click(&mut m, idx(K_ROLE, 1));
    assert_eq!(m.menu, Some(Menu::Cursor(1)));
    // (Glass is the first row; Windows default the second)
    let wd = m.picker_sets(1).iter().position(|r| r.id == SetId::WindowsDefault).unwrap();
    click(&mut m, idx(K_MENU, wd));
    assert_eq!(m.menu, None);
    assert!(rec("cursors").is_none());
    // back to how it was: size 1 again, nothing left
    let rv = review(&m, Kind::HowItWas);
    assert_eq!(rv.lines.len(), 1);
    assert!(reset(&mut m, &rv).iter().all(|r| r.outcome == Outcome::Ok));
    assert_eq!(m.v.cursors.as_ref().unwrap().size, 1);
    assert!(review(&m, Kind::HowItWas).is_empty());
}

/// The cursors' look (all 17 roles + the scheme) is one item: read, Windows' own look, put back from its text alone.
#[test]
fn the_cursors_item_is_the_whole_look() {
    let mut f = svc::sample_fake();
    let was = svc::item_val(&f, "cursors").unwrap();
    assert_eq!(was.text, "Windows default");
    assert!(!svc::windows_defaults(&f).iter().any(|d| d.item == "cursors" && d.now.raw != d.default.raw), "already Windows' own: no line");
    f.set_scheme("Windows Black").unwrap();
    let now = svc::item_val(&f, "cursors").unwrap();
    assert_ne!(now.raw, was.raw);
    assert_eq!(now.text, "Windows Black");
    let d = svc::windows_defaults(&f).into_iter().find(|d| d.item == "cursors").unwrap();
    assert_eq!((d.label.as_str(), d.now.text.as_str(), d.default.text.as_str()), ("Cursors", "Windows Black", "Windows default"));
    svc::restore(&mut f, "cursors", &was.raw).unwrap();
    assert_eq!(svc::item_val(&f, "cursors").unwrap(), was);
    assert!(svc::restore(&mut f, "nothing", "x").is_err());
}
#[test]
fn file_pickers_and_links_never_open_anything_in_a_test() {
    let mut m = opened();
    // Import cursors… / Choose your own file… use Windows' file picker only inside a delivered click (never here)
    click(&mut m, K_IMP);
    assert!(toast(&m).is_none() && m.svc.as_ref().unwrap().pending == 0, "nothing picked, nothing sent");
    click(&mut m, K_WEB);
    click(&mut m, K_GIT);
}

/// the owner (test build 1): "every time i clicked the pick your own one it keeps moving the dropdown menu around the page" -
/// a press INSIDE the open cursor list moved the list's anchor onto the pressed row.
#[test]
fn a_press_inside_the_open_cursor_list_does_not_move_it() {
    let mut m = opened();
    let bubble = (40.0, 300.0, 60.0, 60.0);
    with_cx(|cx| m.event(&Ev::Press(idx(K_ROLE, 1), 50.0, 310.0, bubble), cx));
    click(&mut m, idx(K_ROLE, 1));
    assert_eq!((m.menu, m.anchor), (Some(Menu::Cursor(1)), bubble));
    with_cx(|cx| m.event(&Ev::Press(idx(K_MENU, 50), 120.0, 420.0, (46.0, 400.0, 260.0, 46.0)), cx));
    assert_eq!(m.anchor, bubble, "the list stays where it opened");
    // its "Choose your own file…" row closes it (the picker is Windows' own window)
    click(&mut m, idx(K_MENU, 50));
    assert_eq!(m.menu, None);
}

#[test]
fn closing_the_tab_drops_everything() {
    let mut m = opened();
    m.close();
    assert!(m.svc.is_none() && m.v.win.is_none() && m.panel.presets.is_empty());
}

/// Every control of the page (card open) lands where Chromium lays out the drawing (menu-v22, tools/ref/dom_dump.js with
/// the card opened; window coordinates = page + 56, page not scrolled) - also the parts below the window's fold, which the
/// pixel comparison can't reach yet.
#[test]
fn boxes_match_the_drawing_card_open() {
    let mut m = opened();
    m.panel.expanded = true;
    let laid = render_page(&mut m, None);
    let row0 = idx(K_ROW, 0);
    let want: Vec<(&str, Key, [f32; 4])> = vec![
        ("web link", K_WEB, [427.812, 152.844, 134.188, 16.0]),
        ("accel switch", K_ACT, [486.0, 356.625, 44.0, 24.0]),
        ("chip Valorant", idx(K_CHIP, 0), [129.156, 408.625, 64.812, 26.0]),
        ("chip Default", idx(K_CHIP, 1), [199.969, 408.625, 57.781, 26.0]),
        ("save as preset", K_SAVE, [263.75, 408.625, 113.234, 26.0]),
        ("curve", K_MODE, [82.0, 456.625, 122.0, 24.0]),
        ("gain", K_GAIN, [216.0, 456.625, 56.047, 24.0]),
        ("acceleration", idx(K_ASL, field_ix(Field::Acceleration)), [174.0, 489.125, 116.0, 20.0]),
        ("input offset", idx(K_ASL, field_ix(Field::InputOffset)), [174.0, 516.125, 116.0, 20.0]),
        ("cap: input seg", idx(K_CAP, 0), [176.0, 543.125, 52.0, 20.0]),
        ("cap: output", idx(K_ASL, field_ix(Field::CapOutput)), [174.0, 570.125, 116.0, 20.0]),
        ("sens", idx(K_ASL, 99), [174.0, 605.625, 116.0, 20.0]),
        ("graph", K_GRAPH, [348.0, 454.625, 214.0, 178.0]),
        ("row app", sub(row0, "app"), [82.0, 705.469, 172.0, 24.0]),
        ("row preset", sub(row0, "pre"), [282.0, 705.469, 128.0, 24.0]),
        ("row x", sub(row0, "x"), [540.0, 706.469, 22.0, 22.0]),
        ("add app", K_ADD, [76.0, 744.469, 84.484, 26.0]),
        ("everywhere else", K_ELSE, [282.0, 785.469, 128.0, 24.0]),
        ("copy its curve", K_COPY, [229.672, 826.469, 68.297, 15.0]),
        ("open raw accel", K_OPEN, [307.969, 826.469, 78.578, 15.0]),
        ("pointer speed", K_SPEED, [334.0, 904.312, 168.0, 20.0]),
        ("precision", K_EPP, [518.0, 947.0, 44.0, 24.0]),
        ("scroll lines", K_LINES, [334.0, 993.703, 168.0, 20.0]),
        ("folder", K_DCT, [296.0, 1033.391, 30.0, 30.0]),
        ("double-click", K_DBL, [334.0, 1038.391, 168.0, 20.0]),
        ("swap", K_SWAP, [518.0, 1081.094, 44.0, 24.0]),
        ("import", K_IMP, [258.234, 1136.094, 85.828, 16.0]),
        ("size", K_SIZE, [402.0, 1134.094, 132.0, 20.0]),
        ("back to how it was", sub(K_RS, "pc"), [220.922, 1267.188, 132.344, 16.0]),
        ("windows defaults", sub(K_RS, "win"), [372.266, 1267.188, 94.125, 16.0]),
    ];
    let bubbles = [48.281, 124.844, 201.422, 277.984, 354.562, 431.125, 507.703];
    let mut bad = Vec::new();
    let mut check = |name: &str, k: Key, w: [f32; 4]| match laid.rect_of(k) {
        Some(r) => {
            let r = [r.0, r.1 + 56.0, r.2, r.3];
            if r.iter().zip(w.iter()).any(|(a, b)| (a - b).abs() > 0.02) {
                bad.push(format!("{name}: app {r:?} drawing {w:?}"));
            }
        }
        None => bad.push(format!("{name}: missing")),
    };
    for (n, k, w) in want {
        check(n, k, w);
    }
    for (i, x) in bubbles.iter().enumerate() {
        check(&format!("bubble {i}"), idx(K_ROLE, i), [*x, 1173.094, 44.0, 62.0]);
    }
    assert!(bad.is_empty(), "\n{}", bad.join("\n"));
}

fn popup_laid(m: &mut Mouse) -> crate::ui::lay::Laid {
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(0.0, false, &g, &mut st);
    let p = m.popup(&mut cx).expect("a popup");
    crate::ui::lay::Laid::new(&g, El::block().w(600.0).h(520.0).child(p), 600.0, Some(520.0))
}

fn text_rect(l: &crate::ui::lay::Laid, s: &str) -> (f32, f32, f32, f32) {
    l.nodes.iter().find(|n| matches!(&n.el.content, crate::ui::el::Content::Text(t) if t.s == s)).map(|n| n.rect).unwrap_or_else(|| panic!("no text {s}"))
}

fn near(a: (f32, f32, f32, f32), b: [f32; 4]) -> bool {
    // within 2/64 px (LayoutUnit rounding of centred boxes differs by one unit)
    (a.0 - b[0]).abs() < 0.032 && (a.1 - b[1]).abs() < 0.032 && (a.2 - b[2]).abs() < 0.032 && (a.3 - b[3]).abs() < 0.032
}

/// The role picker (`.menu.curm`) and the popup lists land where Chromium puts the drawing's (dom dumps of the drawing
/// with the Link bubble / the row's pickers clicked; the same anchor box).
#[test]
fn popups_match_the_drawing() {
    // the Link picker over the bubble at (124.844, 720.25, 44, 62): Chromium 125, 468, 272 wide
    let mut m = opened();
    m.anchor = (124.844, 720.25, 44.0, 62.0);
    m.menu = Some(Menu::Cursor(1));
    let l = popup_laid(&mut m);
    let b = l.rect_of(K_MENU).unwrap();
    assert_eq!((b.0, b.2), (125.0, 272.0));
    let rel = |r: (f32, f32, f32, f32)| (r.0 - b.0, r.1 - b.1, r.2, r.3);
    assert!(near(rel(text_rect(&l, "Link")), [13.0, 10.0, 23.86, 17.55]), "{:?}", rel(text_rect(&l, "Link")));
    assert!(near(rel(text_rect(&l, "Hover one to try it")), [44.86, 13.0, 87.55, 14.84]), "{:?}", rel(text_rect(&l, "Hover one to try it")));
    let row = rel(l.rect_of(idx(K_MENU, 0)).unwrap());
    assert!(near(row, [6.0, 34.84, 260.0, 46.0]), "{row:?}");
    // (Order 040: Glass is the first row now, so "Windows default" - the drawing's first row - sits one row (46 + 1 gap)
    // lower; the first row's name sits where the drawing's did)
    assert!(near(rel(text_rect(&l, "Windows default")), [57.0, 42.34 + 47.0, 176.0, 17.0]), "{:?}", rel(text_rect(&l, "Windows default")));
    let glass = rel(text_rect(&l, "Glass"));
    assert!(near(glass, [57.0, 42.34, 176.0, 17.0]), "{glass:?}");
    // the curve list over its button (82, 456.625, 122, 24): no room under it, so above it (Chromium: 82, 287), 150 wide
    let mut m = opened();
    m.panel.expanded = true;
    m.anchor = (82.0, 456.625, 122.0, 24.0);
    m.menu = Some(Menu::Curve);
    let l = popup_laid(&mut m);
    let b = l.rect_of(K_MENU).unwrap();
    assert_eq!((b.0, b.1, b.2, b.3), (82.0, 287.0, 150.0, 166.0));
}

fn opened_with(sample: svc::Sample) -> Mouse {
    let mut m = Mouse::default();
    m.open(&Env { test: true, frozen: true, ..Env::default() }, 0.0);
    m.svc = Some(Svc::start_with(true, sample));
    m.panel_from_worker = false;
    settle(&mut m);
    m
}

/// The drawing's other samples: an unsupported mouse (name — link, the line under it) and Raw Accel not installed (the
/// install card): boxes as Chromium lays them out.
#[test]
fn unsupported_mouse_and_no_raw_accel_boxes() {
    let mut m = opened_with(svc::Sample { unsupported_mouse: true, no_raw_accel: true });
    assert!(!m.supported());
    let l = render_page(&mut m, None);
    let w = |s: &str| {
        let r = text_rect(&l, s);
        (r.0, r.1 + 56.0, r.2, r.3)
    };
    assert!(near(w("Lamzu Maya X"), [80.0, 144.14, 84.73, 17.55]), "{:?}", w("Lamzu Maya X"));
    assert!(near(w("\u{2014}"), [171.73, 144.14, 12.94, 17.55]), "{:?}", w("\u{2014}"));
    assert!(near(w("open its web settings"), [191.67, 144.91, 115.77, 16.0]), "{:?}", w("open its web settings"));
    assert!(near(w(bu_mouse::accel::service::INSTALL_TITLE), [96.0, 217.84, 379.66, 18.0]), "{:?}", w(bu_mouse::accel::service::INSTALL_TITLE));
}

/// The `rename` test state (pixel proof of the chip being renamed): the first chip shows its name field, focused.
#[test]
fn rename_test_state_shows_the_field() {
    let mut m = opened();
    m.test_state_from("cur:acopen;rename");
    settle(&mut m);
    assert!(m.rename.is_some(), "rename not set (presets {:?})", m.panel.presets.iter().map(|p| &p.name).collect::<Vec<_>>());
    let laid = render_page(&mut m, None);
    assert!(laid.rect_of(sub(idx(K_CHIP, 0), "in")).is_some());
}

/// A click on the button of the open list closes it: the frame closes the list on the press (`popup_dismiss`), so the
/// click that follows must not open it again.
#[test]
fn the_open_lists_own_button_closes_it() {
    let mut m = opened();
    click(&mut m, K_MODE);
    assert_eq!(m.menu, Some(Menu::Curve));
    // the real order: the frame's dismiss on the button's press, the press, the click
    m.popup_dismiss();
    with_cx(|cx| m.event(&Ev::Press(K_MODE, 10.0, 10.0, (0.0, 0.0, 122.0, 24.0)), cx));
    click(&mut m, K_MODE);
    assert_eq!(m.menu, None, "stays closed");
    click(&mut m, K_MODE);
    assert_eq!(m.menu, Some(Menu::Curve), "the next click opens it again");
}

/// Order 029 review: a dismiss by Esc / a scroll (no press follows it) must not swallow the next click on that list's
/// button.
#[test]
fn a_dismiss_by_esc_or_scroll_does_not_block_the_next_click() {
    let mut m = opened();
    click(&mut m, K_MODE);
    m.popup_dismiss();
    // the frame builds the page again before the next press
    let _ = render_page(&mut m, None);
    with_cx(|cx| m.event(&Ev::Press(K_MODE, 10.0, 10.0, (0.0, 0.0, 122.0, 24.0)), cx));
    click(&mut m, K_MODE);
    assert_eq!(m.menu, Some(Menu::Curve));
}

/// Order 040: the app's own Glass set is the first row of every role's picker, Windows default the second (the fake lists
/// the drawing's sets: Glass, then the imported "Neon Pack" below the line).
#[test]
fn glass_is_the_first_row_of_every_cursor_picker() {
    let m = opened();
    for ri in 0..Role::ALL.len() {
        let rows = m.picker_sets(ri);
        let ids: Vec<&SetId> = rows.iter().map(|r| &r.id).collect();
        assert_eq!(ids[..2], [&SetId::Glass, &SetId::WindowsDefault], "role {ri}");
        assert_eq!((rows[0].name.as_str(), rows[0].note.as_str(), rows[0].top), ("Glass", "Frosted glass", true));
        assert!(rows.iter().filter(|r| r.id == SetId::Glass).count() == 1);
        assert_eq!(rows.last().map(|r| (&r.id, r.top)), Some((&SetId::Pack("Neon Pack".into()), false)));
    }
}

/// Order 042 (the owner's test 2: "is there more windows presets for cursors?"): every cursor scheme Windows has installed is
/// listed in the role pickers after the app's own sets (only for the bubbles it has a cursor for); a long list scrolls.
#[test]
fn windows_cursor_schemes_are_listed_in_the_pickers() {
    let mut m = opened();
    m.v.schemes = vec![("Windows Black".into(), Role::ALL.to_vec()), ("Only arrows".into(), vec![Role::Normal])];
    let rows = m.picker_sets(0);
    let ids: Vec<&SetId> = rows.iter().map(|r| &r.id).collect();
    assert_eq!(ids[..2], [&SetId::Glass, &SetId::WindowsDefault]);
    assert_eq!(ids[ids.len() - 2..], [&SetId::Scheme("Windows Black".into()), &SetId::Scheme("Only arrows".into())]);
    assert_eq!(rows.last().map(|r| (r.note.as_str(), r.top)), Some(("Windows cursor scheme", false)));
    // the Text bubble: only the scheme that has a text cursor
    assert!(m.picker_sets(Role::ALL.iter().position(|r| *r == Role::Text).unwrap()).iter().all(|r| r.id != SetId::Scheme("Only arrows".into())));
    // twelve schemes: the list scrolls inside the menu, which stays inside the window
    m.v.schemes = (0..12).map(|i| (format!("Scheme {i}"), Role::ALL.to_vec())).collect();
    m.anchor = (60.0, 300.0, 40.0, 40.0);
    let mut el = None;
    with_cx(|cx| el = Some(m.cursor_menu(cx, 0)));
    let el = el.unwrap();
    let g = crate::gfx::Gfx::new(1.0);
    let l = crate::ui::lay::Laid::new(&g, El::block().w(crate::ui::WIN_W).h(crate::ui::WIN_H).child(el), crate::ui::WIN_W, Some(crate::ui::WIN_H));
    let r = l.rect_of(K_MENU).unwrap();
    assert!(r.1 >= 8.0 && r.1 + r.3 <= crate::ui::WIN_H - 8.0 + 0.5, "menu {r:?} inside the window");
    assert!(l.rect_of(sub(K_MENU, "sc")).is_some(), "the list scrolls");
}

/// Order 042 proof picture (`BU_PIC_OUT=<folder> cargo test -p bu-app proof_042 -- --ignored`): the Normal bubble's picker listing the cursor
/// schemes Windows has installed ON THIS PC (read only from the registry), scrolling inside the window.
#[test]
#[ignore]
fn proof_042_cursor_picker_with_windows_schemes() {
    if std::env::var("BU_PIC_OUT").is_err() {
        return;
    }
    let mut m = opened();
    let real = bu_mouse::Mouse::new(bu_mouse::win::RealOs::read_only(), bu_mouse::AppDirs::new(std::env::var("BU_PIC_OUT").unwrap_or_default()));
    m.v.schemes = real.installed_schemes().unwrap();
    println!("installed schemes: {:?}", m.v.schemes.iter().map(|s| s.0.as_str()).collect::<Vec<_>>());
    assert!(!m.v.schemes.is_empty());
    m.anchor = (60.0, 300.0, 40.0, 40.0);
    let mut el = None;
    with_cx(|cx| el = Some(m.cursor_menu(cx, 0)));
    let root = El::block().w(crate::ui::WIN_W).h(crate::ui::WIN_H).child(el.unwrap());
    crate::ui::lay::proof_png(root, crate::ui::WIN_W, crate::ui::WIN_H, 1.5, "cursor_picker.png");
}

/// Order 042 (test feedback: the polling rate showed but not the DPI, 2400, which was not in the list):
/// the mouse's own DPI - e.g. 2400 read from a real mouse (measured with mouse-read) - is a lit chip of its own, in its place.
#[test]
fn the_mouses_own_dpi_is_a_lit_chip_whatever_it_is() {
    let mut m = opened();
    typed(&mut m, K_DPIN, "2400", 0x0D);
    assert_eq!(m.mouse_dpi(), Some(2400));
    assert_eq!(m.dpi_chips(), vec![400, 800, 1600, 2400, 3200]);
    let l = render_page(&mut m, None);
    assert!(l.nodes.iter().any(|n| matches!(&n.el.content, crate::ui::el::Content::Text(t) if t.s == "2400")), "a 2400 chip");
    // a click on 400 sets it; the 2400 chip goes (it was only the mouse's own)
    click(&mut m, idx(K_DPI, 0));
    assert_eq!(m.mouse_dpi(), Some(400));
    assert_eq!(m.dpi_chips(), DPI_CHIPS.to_vec());
}

/// Order 042 proof picture (`BU_PIC_OUT=<folder> cargo test -p bu-app proof_042 -- --ignored`): "Your mouse" with the mouse at 2400 DPI.
#[test]
#[ignore]
fn proof_042_mouse_dpi_2400() {
    if std::env::var("BU_PIC_OUT").is_err() {
        return;
    }
    let mut m = opened();
    typed(&mut m, K_DPIN, "2400", 0x0D);
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(0.0, false, &g, &mut st);
    let kids = m.build(&mut cx);
    let root = El::block().w(600.0).h(330.0).pad(2.0, 26.0, 18.0, 26.0).children(kids);
    crate::ui::lay::proof_png(root, 600.0, 330.0, 2.0, "mouse_dpi_2400.png");
}

/// Order 047 (idle cost): with the worker's answers in and nothing moving the tab asks for no frames - `tick` is false and
/// a build does not ask for the next frame (an answer still on its way wakes the menu itself).
#[test]
fn nothing_moving_asks_for_no_frames() {
    let mut m = opened();
    assert!(!m.tick(10.0), "nothing new: no repaint");
    assert_eq!(m.wake_at(10.0), None);
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(20.0, false, &g, &mut st).for_page("cur");
    let _ = m.build(&mut cx);
    drop(cx);
    assert!(!st.busy, "an idle tab asks for no frames");
}

/// Order 047: the frame's reset through the page's detached copy - the review opened and the Reset pressed each inside
/// one frame (16 ms), the reads and the put-backs on the review's worker thread; then the page re-reads (`reset_done`).
/// (The page's fake has no slow mode for these calls: the proof is that both run on the worker - `Reading` / `Running`.)
fn reset_off_the_menu(p: &mut dyn Resettable, kind: Kind) -> (crate::undo::Review, Vec<crate::undo::LineResult>) {
    fn wait<T>(mut f: impl FnMut() -> Option<T>) -> T {
        let t0 = std::time::Instant::now();
        loop {
            if let Some(v) = f() {
                return v;
            }
            assert!(t0.elapsed().as_secs() < 10, "the review's worker never answered");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    let opened = crate::services::with(|s| {
        crate::undo::flush(&mut s.store);
        crate::offui::assert_quick("opening the review", || crate::undo::Review::open(kind, false, &mut [&mut *p], &s.store))
    })
    .unwrap();
    let crate::undo::Opened::Reading(mut job) = opened else { panic!("the review is read on a worker thread") };
    let rv = wait(|| job.take());
    let applied = crate::offui::assert_quick("Reset", || rv.start_apply(&mut [&mut *p]));
    let crate::undo::Applied::Running(mut job) = applied else { panic!("the reset is put back on a worker thread") };
    let res = wait(|| job.take());
    p.reset_done();
    (rv, res)
}

/// Order 047: the Mouse tab's reset (Windows defaults) is read from the tab's last view and put back by its worker, asked
/// from the review's worker thread - the menu's thread never waits for it; same lines, same results, the tab shows it.
#[test]
fn the_reset_review_reads_and_puts_back_off_the_menus_thread() {
    let _s = Sv::start();
    let mut m = opened();
    drag(&mut m, K_SPEED, SPEED_BOX, 1.0);
    assert_eq!(m.v.win.unwrap().pointer_speed, 20);
    let (rv, res) = reset_off_the_menu(&mut m, Kind::WindowsDefaults);
    let def: Vec<String> = rv.lines.iter().map(|l| format!("{}: {}", l.label, l.change_text())).collect();
    assert_eq!(def, ["Pointer speed: 20  →  10", "Enhance pointer precision: Off  →  On"]);
    assert!(res.iter().all(|r| r.outcome == Outcome::Ok), "{res:?}");
    let t0 = std::time::Instant::now();
    while !(m.v.win.unwrap().pointer_speed == 10 && m.v.win.unwrap().precision) && t0.elapsed().as_secs() < 5 {
        m.tick(0.0);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let w = m.v.win.unwrap();
    assert_eq!((w.pointer_speed, w.precision), (10, true), "the tab shows its worker's answer");
}

/// Order 047: a setting change (Swap primary button: SystemParametersInfo + its broadcast) never holds the menu - the
/// click only hands it to the tab's worker and returns within one frame; the tab shows the worker's answer.
/// (The fake mouse layer has no slow mode: the proof is the click's own time and the answer from the worker.)
#[test]
fn a_setting_change_never_holds_the_menu() {
    let mut m = opened();
    assert!(!m.v.win.unwrap().buttons_swapped);
    let g = Gfx::new(1.0);
    let mut st = State::default();
    let mut cx = Cx::new(1000.0, false, &g, &mut st).for_page("cur");
    crate::offui::assert_quick("Swap primary button", || m.event(&Ev::Click(K_SWAP), &mut cx));
    drop(cx);
    settle(&mut m);
    assert!(m.v.win.unwrap().buttons_swapped);
    assert_eq!(toast(&m), Some("Right button is now your main button"));
}

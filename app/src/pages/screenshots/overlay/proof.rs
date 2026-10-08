//! Test only: the overlay painted OFF-SCREEN by the app's own painter, in the same states the drawing was rendered in (Lane Q's
//! scratch ovl_ref.js + states.js: <state>_A.png = the drawing's overlay, <state>_B.png = the same moment without the overlay).
//! Each state is painted over its B picture (the frozen picture) and saved as <state>_app.png next to them - nothing on the
//! screen, no capture of the real screen. Run: `BU_OVL_PROOF=<dir> cargo test -p bu-app overlay_proof -- --ignored`.

use super::model::{Model, Tool};
use super::view;
use bu_screenshot::fake::mon;
use bu_screenshot::{geom, Monitor};

use crate::gfx::Gfx;
use crate::icons::Icons;
use crate::ui::cx::{Cx, State};

fn monitors() -> Vec<Monitor> {
    // the drawing's desktop: monitor 1 = its 1920 x 1080 screen, monitor 2 (1920 x 1080) to its right
    let mut v = vec![mon(0, 0, 1920, 1080, true, false), mon(1920, 0, 1920, 1080, false, false)];
    geom::number_monitors(&mut v);
    v
}

fn drag(m: &mut Model, pts: &[(f32, f32)], now: f64) {
    m.press(pts[0], None, now);
    for p in &pts[1..] {
        m.moved(*p, false);
    }
    m.release(now);
}

const T: f64 = 1000.0;

/// The states of states.js, built through the model's own calls.
pub fn states() -> Vec<(&'static str, Model)> {
    // after "ann" the drawing's co keeps the white pen and the fire stamp (the next states start with them)
    let fire = || Some(super::model::Keep { color: 0xffffff, emoji: "🔥".to_string(), recent: super::emoji::Recent::default() });
    let sel = |keep| {
        let mut m = Model::new(monitors(), (560.0, 300.0), 0.0, keep);
        drag(&mut m, &[(560.0, 300.0), (700.0, 420.0), (900.0, 560.0), (1180.0, 700.0)], 10.0);
        m.moved((1180.0, 700.0), false);
        m
    };
    let mut v = Vec::new();
    v.push(("idle", Model::new(monitors(), (700.0, 420.0), 0.0, None)));
    v.push(("sel", sel(None)));
    {
        let mut m = sel(None);
        m.pick_tool(Tool::Pen, 20.0);
        drag(&mut m, &[(620.0, 360.0), (640.0, 350.0), (665.0, 348.0), (690.0, 356.0), (712.0, 372.0), (730.0, 395.0)], 20.0);
        m.pick_tool(Tool::Arrow, 20.0);
        drag(&mut m, &[(760.0, 620.0), (820.0, 580.0), (880.0, 520.0)], 20.0);
        m.pick_color(0x30d158);
        m.pick_tool(Tool::Box, 20.0);
        drag(&mut m, &[(920.0, 340.0), (1000.0, 400.0), (1080.0, 460.0)], 20.0);
        m.pick_color(0xffd60a);
        m.pick_tool(Tool::Hl, 20.0);
        drag(&mut m, &[(600.0, 640.0), (650.0, 640.0), (700.0, 642.0), (750.0, 640.0)], 20.0);
        m.pick_color(0xffffff);
        m.pick_tool(Tool::Text, 20.0);
        m.press((620.0, 470.0), None, 20.0);
        for c in "Nice".chars() {
            m.char_input(c);
        }
        m.commit_text();
        m.pick_tool(Tool::Emoji, 20.0);
        m.emoji = "🔥".into();
        m.pick_tool(Tool::Emoji, 20.0);
        m.pick_tool(Tool::Emoji, 20.0);
        drag(&mut m, &[(1050.0, 600.0), (1050.0, 600.0)], 20.0);
        m.pick_tool(Tool::Box, 20.0);
        m.pick_tool(Tool::Box, 20.0);
        m.moved((1500.0, 200.0), false);
        v.push(("ann", m));
    }
    {
        let mut m = sel(fire());
        m.pick_tool(Tool::Emoji, 20.0);
        m.moved((1500.0, 200.0), false);
        v.push(("emo", m));
    }
    {
        let mut m = sel(fire());
        m.pick_tool(Tool::Emoji, 20.0);
        m.picker_big(true, 20.0);
        m.moved((1500.0, 200.0), false);
        v.push(("more", m));
    }
    {
        let mut m = Model::new(monitors(), (700.0, 420.0), 0.0, fire());
        m.set_live(true);
        v.push(("live", m));
    }
    {
        let mut m = sel(fire());
        m.size_start();
        // the reference window is off-screen and never focused: Chromium shows the inactive selection colours
        m.focused = false;
        v.push(("size", m));
    }
    {
        let mut m = Model::new(monitors(), (560.0, 300.0), 0.0, fire());
        m.pick_preset(0, 20.0);
        m.moved((1500.0, 200.0), false);
        v.push(("mon1", m));
    }
    {
        let mut m = sel(None);
        m.pick_tool(Tool::Pen, 20.0);
        m.moved((800.0, 500.0), false);
        v.push(("pen", m));
    }
    for (name, q) in [("search", "heart"), ("nores", "zzqq")] {
        let mut m = sel(None);
        m.pick_tool(Tool::Emoji, 20.0);
        m.picker_big(true, 20.0);
        for c in q.chars() {
            m.char_input(c);
        }
        m.moved((1500.0, 200.0), false);
        v.push((name, m));
    }
    {
        let mut m = sel(None);
        m.pick_tool(Tool::Text, 20.0);
        m.press((700.0, 500.0), None, 20.0);
        for c in "Hello".chars() {
            m.char_input(c);
        }
        m.moved((1500.0, 200.0), false);
        m.focused = false;
        v.push(("txt", m));
    }
    for (name, at) in [("tiptool", (647.0, 733.0)), ("tipmon", (652.0, 281.0))] {
        let mut m = sel(None);
        m.moved(at, true);
        v.push((name, m));
    }
    v
}

/// The keys under the pointer in the hover states (CSS :hover chains, innermost first).
fn hover_of(name: &str) -> Vec<crate::ui::el::Key> {
    use crate::ui::el::idx;
    match name {
        "tiptool" => vec![idx(view::K_TOOL, 0), view::K_TB],
        "tipmon" => vec![idx(view::K_MON, 0), view::K_BAR],
        _ => Vec::new(),
    }
}

/// The same states on a 3440 x 1440 screen (the second monitor 1920 x 1080 to its right, as the drawing has it).
pub fn states_3440() -> Vec<(&'static str, Model)> {
    let mons = || {
        let mut v = vec![mon(0, 0, 3440, 1440, true, false), mon(3440, 0, 1920, 1080, false, false)];
        geom::number_monitors(&mut v);
        v
    };
    states()
        .into_iter()
        .filter(|(n, _)| matches!(*n, "sel" | "ann" | "more"))
        .map(|(n, m)| {
            let mut m2 = Model::new(mons(), m.last, m.opened_at, Some(m.keep()));
            m2.mode = m.mode;
            m2.sel = m.sel;
            m2.anns = m.anns.clone();
            m2.tool = m.tool;
            m2.picker = m.picker.clone();
            m2.tb_at = m.tb_at;
            (n, m2)
        })
        .collect()
}

#[test]
#[ignore]
fn overlay_proof() {
    let Ok(dir) = std::env::var("BU_OVL_PROOF") else { return };
    // WIC (the PNG reader / writer) needs COM on this thread
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    }
    let g = Gfx::new(1.0);
    let icons = Icons::new();
    let wide = std::env::var("BU_OVL_3440").is_ok();
    let (list, sw, sh) = if wide { (states_3440(), 3440, 1440) } else { (states(), 1920, 1080) };
    for (name, m) in list {
        let b = format!("{dir}\\{name}_B.png");
        let Ok(bg) = crate::png::load_png(&b) else { panic!("cannot read {b}") };
        let bg = crate::png::to_image(&bg).expect("B picture");
        let mut st = State::default();
        st.hover = hover_of(name);
        for k in st.hover.clone() {
            st.hover_since.insert(k, 0.0);
        }
        // build twice: the first build seeds the transitions at their targets (CSS: no transition on the first style)
        for _ in 0..2 {
            let mut cx = Cx::new(T * 10.0, false, &g, &mut st);
            let _ = view::scene(&m, &mut cx, 0, T * 10.0);
        }
        let mut cx = Cx::new(T * 10.0, false, &g, &mut st);
        let sc = view::scene(&m, &mut cx, 0, T * 10.0);
        let mut surf = crate::gfx::new_surface(sw, sh).expect("surface");
        view::paint(&g, &icons, &mut surf, &sc, Some(&bg));
        let px = crate::png::from_surface(&mut surf);
        crate::png::save_png(&px, &format!("{dir}\\{name}_app.png")).expect("save");
    }
}

/// The capture toast (state "toast": Copy of the 620 x 400 box, ~650 ms after it showed) over its B picture. The drawing's
/// thumbnail is its own redrawing of the fake desktop, so the app's thumbnail is the real crop of the B picture (that 80 x 45
/// box is left out of the count).
#[test]
#[ignore]
fn overlay_proof_toast() {
    let Ok(dir) = std::env::var("BU_OVL_PROOF") else { return };
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    }
    let g = Gfx::new(1.0);
    let icons = Icons::new();
    let bg = crate::png::to_image(&crate::png::load_png(&format!("{dir}\\toast_B.png")).expect("toast_B")).expect("img");
    let crop = bg.make_subset(None, skia_safe::IRect::from_xywh(560, 300, 620, 400), Default::default()).expect("crop");
    let t = super::toast::toast(std::rc::Rc::new(crop), "Screenshot copied", "620×400", 800.0);
    let front = crate::ui::el::El::block().size(1920.0, 1080.0).child(t.abs(f32::NAN, f32::NAN, 12.0, 48.0 + 12.0));
    let sc = view::Scene {
        w: 1920.0,
        h: 1080.0,
        scale: 1.0,
        back: crate::ui::el::El::block().size(1920.0, 1080.0),
        front,
        glass: vec![view::Glass { key: super::toast::K_TOAST, blur: 30.0, sat: Some(1.6) }],
        busy: false,
    };
    let mut surf = crate::gfx::new_surface(1920, 1080).expect("surface");
    view::paint(&g, &icons, &mut surf, &sc, Some(&bg));
    let px = crate::png::from_surface(&mut surf);
    crate::png::save_png(&px, &format!("{dir}\\toast_app.png")).expect("save");
}

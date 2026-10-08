//! Order 045 proof pictures (tests only, run on purpose): `BU_PIC_OUT=<folder> cargo test -p bu-app proof_045 -- --ignored
//! --test-threads=1`. Each look is painted off-screen by the app's own painter, dark and light; nothing opens on screen and
//! only the pages' FAKE services are used.

use crate::gfx::Gfx;
use crate::pages::{Env, Page};
use crate::ui::cx::{Cx, Ev, State};
use crate::ui::el::{key, El, Key};
use crate::ui::lay::{proof_png, Laid};
use crate::ui::pieces::keyfield::{self, Show};

fn env() -> Env {
    Env { test: true, frozen: true, ..Env::default() }
}

/// A page's boxes as the frame lays them (`.pg`: 600 wide, padding 2 26 18 26) + the open popup over them.
fn page_root(p: &mut dyn Page, st: &mut State, now: f64) -> El {
    let g = Gfx::new(1.0);
    let mut cx = Cx::new(now, false, &g, st).for_page(p.id());
    let kids = p.build(&mut cx);
    El::block().w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids)
}

fn both(mut f: impl FnMut(&str)) {
    crate::testmode::set(true, false);
    for (light, name) in [(false, "dark"), (true, "light")] {
        crate::ui::set_light(light);
        f(name);
    }
    crate::ui::set_light(false);
}

fn on() -> bool {
    std::env::var("BU_PIC_OUT").is_ok()
}

/// Item 12: the keyboard focus ring on a Display button, a switch and a slider's thumb.
#[test]
#[ignore]
fn proof_045_focus_ring() {
    if !on() {
        return;
    }
    both(|theme| {
        let mut p = crate::pages::display::Display::default();
        p.open(&env(), 0.0);
        let mut st = State::default();
        let root = page_root(&mut p, &mut st, 5000.0);
        let g = Gfx::new(1.0);
        let laid = Laid::new(&g, root.clone(), 600.0, None);
        let list = laid.focusables();
        let info = |k: Key| laid.focus_info(k).unwrap();
        let slider = list.iter().copied().find(|k| info(*k).1.is_some());
        let button = list.iter().copied().find(|k| info(*k).2 && info(*k).1.is_none() && laid.rect_of(*k).is_some_and(|r| r.3 >= 26.0 && r.2 > 40.0));
        let mut out = root;
        for k in [slider, button].into_iter().flatten() {
            let (radius, range, _, _) = info(k);
            out = out.child(crate::ui::focus_ring(laid.rect_of(k).unwrap(), radius, range));
        }
        let h = laid.height.min(900.0);
        proof_png(out, 600.0, h, 1.5, &format!("12_focus_ring_{theme}.png"));
    });
}

/// Item 6: Security's drop zone while a file is dragged over it (`.sdz.over`), next to it at rest.
#[test]
#[ignore]
fn proof_045_drop_zone_over() {
    if !on() {
        return;
    }
    both(|theme| {
        for over in [false, true] {
            let mut p = crate::pages::security::Security::default();
            p.open(&env(), 0.0);
            let mut st = State::default();
            let _ = page_root(&mut p, &mut st, 100.0);
            if over {
                let g = Gfx::new(1.0);
                let mut cx = Cx::new(200.0, false, &g, &mut st).for_page("sec");
                p.event(&Ev::DragOver(Some(key("sec.dz"))), &mut cx);
            }
            // (the .2 s / .3 s transitions start at this build: the picture is taken after them)
            let _ = page_root(&mut p, &mut st, 5000.0);
            let root = page_root(&mut p, &mut st, 5600.0);
            let g = Gfx::new(1.0);
            let laid = Laid::new(&g, root.clone(), 600.0, None);
            let dz = laid.rect_of(key("sec.dz")).expect("the drop zone");
            // just the drop zone and a little around it
            let cut = El::block().w(600.0).h(dz.3 + 40.0).clip().child(root.translate(0.0, -(dz.1 - 20.0)));
            proof_png(cut, 600.0, dz.3 + 40.0, 1.5, &format!("06_drop_zone_{}_{theme}.png", if over { "over" } else { "rest" }));
        }
    });
}

/// Item 2: a key field's caps dropping in after a key was taken (0 / 80 / 160 / 240 / 400 ms) - one row per moment.
#[test]
#[ignore]
fn proof_045_keycaps_drop_in() {
    if !on() {
        return;
    }
    both(|theme| {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let k = key("proof.kf");
        let mut col = El::col().w(260.0).pad_all(12.0).gap(10.0);
        for t in [0.0, 80.0, 160.0, 240.0, 400.0] {
            st.kf_set.insert(k, 1000.0);
            let mut cx = Cx::new(1000.0 + t, false, &g, &mut st);
            col = col.child(El::row().child(keyfield::keyfield(&mut cx, k, Show::Set("Ctrl + Shift + M"), 0.0, false)));
        }
        proof_png(col, 260.0, 200.0, 2.0, &format!("02_keycaps_{theme}.png"));
    });
}

/// Item 3: the corner tag + both guide lines as they show over a desktop while the icon snaps to the bottom-right corner.
#[test]
#[ignore]
fn proof_045_guides() {
    if !on() {
        return;
    }
    crate::testmode::set(true, false);
    let blue = crate::gfx::Rgba(10.0 / 255.0, 132.0 / 255.0, 1.0, 0.8);
    let desk = El::block()
        .w(480.0)
        .h(300.0)
        .bg_linear(135.0, &[(0.0, crate::gfx::Rgba(0.18, 0.3, 0.45, 1.0)), (1.0, crate::gfx::Rgba(0.6, 0.45, 0.35, 1.0))])
        // .guide.v at the right edge line (W - EDGE), .guide.h at the bottom one (H - EDGE)
        .child(El::block().abs(480.0 - 24.0, 0.0, f32::NAN, f32::NAN).size(1.0, 300.0).bg(blue))
        .child(El::block().abs(0.0, 300.0 - 24.0, f32::NAN, f32::NAN).size(480.0, 1.0).bg(blue))
        .child(crate::guides::tag_el("Bottom right", 340.0, 250.0));
    proof_png(desk, 480.0, 300.0, 2.0, "03_guides.png");
}

/// Items 10 + 11: the shared tip bubble over a data-tip (Network's ping pill) and over a plain title (an Apps row's lock).
#[test]
#[ignore]
fn proof_045_tips() {
    if !on() {
        return;
    }
    both(|theme| {
        let pages: [(&str, Box<dyn Page>); 2] = [("net", Box::new(crate::pages::network::Network::default())), ("apps", Box::new(crate::pages::apps::Apps::default()))];
        for (name, mut p) in pages {
            p.open(&env(), 0.0);
            let mut st = State::default();
            let _ = page_root(p.as_mut(), &mut st, 100.0);
            let root = page_root(p.as_mut(), &mut st, 5000.0);
            let g = Gfx::new(1.0);
            let laid = Laid::new(&g, root.clone(), 600.0, None);
            // the first tipped element (and its keyed ancestors as the hover chain)
            let Some(i) = laid.nodes.iter().position(|n| n.el.tip.is_some() && n.el.key.is_some()) else { continue };
            let mut hover = Vec::new();
            let mut c = Some(i);
            while let Some(j) = c {
                if let Some(k) = laid.nodes[j].el.key {
                    hover.push(k);
                }
                c = laid.nodes[j].parent;
            }
            let mut tips = crate::ui::pieces::tip::Tips::default();
            tips.update(&g, &[(&laid, (0.0, 0.0))], &hover, 0.0, 600.0);
            tips.update(&g, &[(&laid, (0.0, 0.0))], &hover, 700.0, 600.0);
            let Some(b) = tips.el(900.0) else { continue };
            let r = laid.nodes[i].rect;
            let h = (r.1 + 120.0).min(laid.height);
            proof_png(El::block().w(600.0).h(h).child(root).child(b), 600.0, h, 1.5, &format!("10_11_tip_{name}_{theme}.png"));
        }
    });
}

/// Item 8: a click on a locked Apps row pulses its lock (`nudge`: scale 1 -> 1.14 at 35 % -> 1, 340 ms) - at rest and at
/// the pulse's peak.
#[test]
#[ignore]
fn proof_045_apps_lock_pulse() {
    if !on() {
        return;
    }
    both(|theme| {
        let mut p = crate::pages::apps::Apps::default();
        p.open(&env(), 0.0);
        let mut st = State::default();
        let root = page_root(&mut p, &mut st, 100.0);
        let g = Gfx::new(1.0);
        let laid = Laid::new(&g, root, 600.0, None);
        let Some(i) = laid.nodes.iter().position(|n| n.el.tip.as_deref().is_some_and(|t| t.starts_with("Windows keeps this one"))) else { panic!("no locked row") };
        let mut c = laid.nodes[i].parent;
        let mut row = None;
        while let Some(j) = c {
            if laid.nodes[j].el.click {
                row = laid.nodes[j].el.key;
                break;
            }
            c = laid.nodes[j].parent;
        }
        let row = row.expect("the row");
        let r = laid.nodes[i].rect;
        {
            let mut cx = Cx::new(1000.0, false, &g, &mut st).for_page("apps");
            p.event(&Ev::Press(row, r.0, r.1, r), &mut cx);
            p.event(&Ev::Release(row), &mut cx);
            p.event(&Ev::Click(row), &mut cx);
        }
        // (EASE_OUT reaches the 35 % keyframe ~25 ms in)
        for (t, name) in [(1000.0 + 25.0, "peak"), (5000.0, "rest")] {
            let root = page_root(&mut p, &mut st, t);
            let cut = El::block().w(600.0).h(80.0).clip().child(root.translate(0.0, -(r.1 - 30.0)));
            proof_png(cut, 600.0, 80.0, 2.0, &format!("08_lock_{name}_{theme}.png"));
        }
    });
}

/// Item 15: Tweaks' Sound group with "Calls turn other sounds down" (Windows' own Communications choice).
#[test]
#[ignore]
fn proof_045_tweaks_sound() {
    if !on() {
        return;
    }
    both(|theme| {
        let mut p = crate::pages::tweaks::Tweaks::default();
        p.open(&env(), 0.0);
        let mut st = State::default();
        let _ = page_root(&mut p, &mut st, 100.0);
        let root = page_root(&mut p, &mut st, 5000.0);
        let g = Gfx::new(1.0);
        let laid = Laid::new(&g, root.clone(), 600.0, None);
        let Some(n) = laid.nodes.iter().find(|n| matches!(&n.el.content, crate::ui::el::Content::Text(t) if t.s == "Calls turn other sounds down")) else { panic!("no ducking row") };
        let y = n.rect.1;
        let cut = El::block().w(600.0).h(220.0).clip().child(root.translate(0.0, -(y - 50.0)));
        proof_png(cut, 600.0, 220.0, 1.5, &format!("15_tweaks_sound_{theme}.png"));
    });
}

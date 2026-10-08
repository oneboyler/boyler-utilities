//! Apps' uninstall confirm (menu-v22 `.dlg.udlg`, Order 025 batch 8): the small glass sheet WITHOUT the title row - a plain
//! title, one line, the apps that go (when more than one), a word when it matters, Cancel / Uninstall. Lane K's
//! `dialog::dialog` is the window with the title row + ×; this is its other v22 form. A page returns it from `Page::popup`.

use crate::anim::{Bezier, EASE_IN, EASE_OUT_CSS};
use crate::gfx::{sh, Font, Rgba};
use crate::ui::cx::Cx;
use crate::ui::el::{lh, sub, El, Key};
use super::dialog::{dlg_bg, dlg_inset, dlg_shadow, scrim};
use crate::ui::{FG, FG2, FG3, HAIR, RADIUS, WELL, WIN_H, WIN_W};

use super::inote::{inote, UDW};
use super::listrow::{tile, Tile};

const POP_IN: Bezier = Bezier::new(0.3, 1.2, 0.5, 1.0);

/// One app in the list (`.udr`): its tile, name and size ("1.2 GB").
pub struct UdRow<'a> {
    pub tile: Tile,
    pub name: &'a str,
    pub size: &'a str,
}

/// The confirm. `title` = "Uninstall Spotifast?" / "Uninstall 3 apps?", `line` = "Frees about 1.4 GB. An app's own uninstaller
/// may open: finish it there.", `rows` = the first four apps (pass none for one app: the drawing hides the list then),
/// `more` = how many beyond those ("+ 2 more"), `warn` = the apps' own words that matter (`.inote.udw`), `footer` = the
/// buttons (`button::cbtn_sized(.., "Cancel", Kind::Ghost, button::DFT, false, 76.0)` + Uninstall `Kind::Red`; focus Cancel
/// on open, as the drawing does 40 ms later). `opened_at` = when it opened; `closing_at` = when Cancel / a click beside it
/// started closing it (the page drops it after `close_ms(rm)`; it ignores "out" and "win" meanwhile). A click beside it = `Ev::Click(sub(key, "out"))`.
///
/// `.dlg{width:388px;padding:18px 18px 16px;border-radius:14px;background:var(--menu);backdrop-filter:blur(30px) saturate(180%);
///   box-shadow:inset 0 0 0 .5px var(--hl),inset 0 1px 0 rgba(255,255,255,.06),0 0 0 .5px rgba(0,0,0,.35),0 24px 60px rgba(0,0,0,.45)}`
/// `.udlg{width:360px}` `.dlg h3{margin:0;font:600 15px/20px "Segoe UI Variable Display";letter-spacing:-.01em}`
/// `.dlg p{margin:3px 0 14px;font-size:12px;line-height:16px;color:var(--fg2)}` `.udl{margin:-4px 0 6px;padding:4px 0;
///   border-radius:9px;background:var(--well);box-shadow:inset 0 0 0 .5px var(--hair)}` `.udr{display:flex;align-items:center;gap:9px;
///   height:32px;padding:0 10px;font-size:12.5px}` `.udr .ait{width:20px;height:20px;border-radius:5px}` `.udr .ait svg{12px}`
/// `.udr .tti{flex:1}` `.udv{font-size:12px;color:var(--fg2);tabular-nums}` `.udm{padding:4px 10px 4px 39px;font-size:11.5px;
///   color:var(--fg3)}` `#sw .udlg .dft{margin-top:14px}`; open: the dim .18 s ease-out, the sheet .34 s from 8 px lower at
///   scale .965 (cubic-bezier(.3,1.2,.5,1); reduced motion: opacity .16 s); close: the sheet to scale .98 + transparent .13 s
///   ease-in (none under reduced motion), the dim .15 s (.12 s) ease-in.
#[allow(clippy::too_many_arguments)]
pub fn udlg(cx: &mut Cx, key: Key, title: &str, line: &str, rows: &[UdRow], more: usize, warn: Option<&str>, footer: Vec<El>, opened_at: f64, closing_at: Option<f64>) -> El {
    let rm = cx.rm;
    let age = cx.now - opened_at;
    // opening
    let wo = if rm { (age / 160.0).clamp(0.0, 1.0) as f32 } else { EASE_OUT_CSS.ease((age / 180.0).clamp(0.0, 1.0)) as f32 };
    let bp = if rm { (age / 160.0).clamp(0.0, 1.0) } else { (age / 340.0).clamp(0.0, 1.0) };
    let be = if bp >= 1.0 { 1.0 } else { POP_IN.ease(bp) as f32 };
    let (mut op, mut dy, mut sc) = if rm { (be, 0.0, 1.0) } else { (be.min(1.0), 8.0 * (1.0 - be), if bp >= 1.0 { 1.0 } else { 0.965 + 0.035 * be }) };
    if age < 400.0 {
        cx.st.busy = true;
    }
    // closing: the WRAPPER (dim + sheet together) fades out over close_ms (the page drops it then); on top of that the sheet
    // itself goes to transparent / scale .98 in 130 ms (not under reduced motion)
    let mut wf = 1.0;
    if let Some(t) = closing_at {
        let a = cx.now - t;
        wf = 1.0 - EASE_IN.ease((a / close_ms(rm)).clamp(0.0, 1.0)) as f32;
        if !rm {
            let p = EASE_IN.ease((a / 130.0).clamp(0.0, 1.0)) as f32;
            op *= 1.0 - p;
            dy = 0.0;
            sc = 1.0 - 0.02 * p;
        }
        // busy up to AND past the drop time, so the frame that drops it always comes
        if a <= close_ms(rm) {
            cx.st.busy = true;
        }
    }
    let mut win = El::block()
        .w(360.0)
        .pad(18.0, 18.0, 16.0, 18.0)
        .radius(14.0)
        .bg(dlg_bg())
        .backdrop(30.0, 1.8)
        .shadow(&dlg_shadow())
        .inset(&dlg_inset())
        .opacity(op)
        .translate(0.0, dy)
        .scale(sc)
        // the sheet takes its own clicks (`Ev::Click(sub(key, "win"))`, pages ignore it): a click on its blank parts is never
        // "beside it" (as dialog.rs)
        .on_click(sub(key, "win"))
        .child(El::text(title, Font::display(15.0, 600).ls(-150), FG(), 20.0).wrapping())
        .child(El::text(line, Font::new(12.0, 400), FG2(), 16.0).wrapping().margin(3.0, 0.0, 14.0, 0.0));
    if !rows.is_empty() {
        let mut l = El::col().margin(-4.0, 0.0, 6.0, 0.0).pad(4.0, 0.0, 4.0, 0.0).radius(9.0).bg(WELL()).inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())]);
        for r in rows {
            l = l.child(
                El::row()
                    .center()
                    .gap(9.0)
                    .h(32.0)
                    .pad(0.0, 10.0, 0.0, 10.0)
                    .child(tile(&r.tile, 20.0))
                    .child(El::text(r.name, Font::new(12.5, 400), FG(), lh(12.5, 1.35)).ellipsis().flex1().min_w(0.0))
                    .child(El::text(r.size, Font::new(12.0, 400).tnum(), FG2(), lh(12.0, 1.35)).none()),
            );
        }
        if more > 0 {
            l = l.child(El::text(format!("+ {more} more"), Font::new(11.5, 400), FG3(), lh(11.5, 1.35)).pad(4.0, 10.0, 4.0, 39.0));
        }
        win = win.child(l);
    }
    if let Some(w) = warn {
        win = win.child(inote(w, false, &UDW));
    }
    if !footer.is_empty() {
        win = win.child(El::row().justify(taffy::style::JustifyContent::FLEX_END).gap(8.0).margin(14.0, 0.0, 0.0, 0.0).children(footer));
    }
    // the dim keeps catching clicks while it fades out (as `.dlgw` until it is removed): the page ignores "out" while closing
    El::grid().abs(0.0, 0.0, 0.0, 0.0).size(WIN_W, WIN_H).place_center().radius(RADIUS).bg(scrim(wo)).opacity(wf).z(9).on_click(sub(key, "out")).child(win)
}

/// How long the close takes = the wrapper's fade (150 ms ease-in, 120 ms under reduced motion); the page drops the confirm
/// once `now - closing_at > close_ms(rm)`.
pub fn close_ms(rm: bool) -> f64 {
    if rm {
        120.0
    } else {
        150.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::Gfx;
    use crate::ui::cx::State;
    use crate::ui::el::key;
    use crate::ui::lay::Laid;

    /// REVIEW_025_b8 HOLD 1: a click on the confirm's own text / list / padding is the sheet's (`win`), never `out`.
    #[test]
    fn a_click_inside_the_confirm_is_not_a_click_beside_it() {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(0.0, true, &g, &mut st);
        let d = udlg(&mut cx, key("ud"), "Uninstall Spotifast?", "Frees about 1.2 GB.", &[], 0, Some("Spotifast: keeps your playlists."), vec![], -10000.0, None);
        let l = Laid::new(&g, El::block().w(WIN_W).h(WIN_H).child(d), WIN_W, Some(WIN_H));
        let (wx, wy, ww, wh) = l.rect_of(sub(key("ud"), "win")).unwrap();
        let click_at = |x: f32, y: f32| l.hit(x, y).and_then(|(i, _)| l.clickable(i));
        for (x, y) in [(wx + 24.0, wy + 24.0), (wx + 30.0, wy + wh - 20.0), (wx + ww - 6.0, wy + wh - 6.0)] {
            assert_eq!(click_at(x, y), Some(sub(key("ud"), "win")), "inside at {x},{y}");
        }
        assert_eq!(click_at(6.0, 6.0), Some(sub(key("ud"), "out")));
    }

    /// REVIEW_025_b8 HOLD 3: the closing confirm asks for frames until (and past) its drop time, and fades as one.
    #[test]
    fn closing_keeps_frames_coming_until_it_is_dropped() {
        for rm in [false, true] {
            let g = Gfx::new(1.0);
            let mut st = State::default();
            let at = close_ms(rm);
            let mut cx = Cx::new(10_000.0 + at, rm, &g, &mut st);
            let _ = udlg(&mut cx, key("ud"), "t", "l", &[], 0, None, vec![], 0.0, Some(10_000.0));
            assert!(cx.st.busy, "rm {rm}: busy at the drop time");
        }
    }
}

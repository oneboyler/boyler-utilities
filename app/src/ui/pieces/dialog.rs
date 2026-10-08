//! The small popup window (menu-v22 `.dlgw` + `.dlg.mdlg`, "built like Screenshots' folder window": a title + ×, its
//! content, optional buttons at the bottom right; Esc or a click beside it closes it). Used for Mute settings, Controller
//! settings, Tweaks' games, the Updating window, the per-tab reset review. A page returns it from `Page::popup`.

use taffy::style::{AlignItems, JustifyContent};

use crate::anim::{Bezier, EASE};
use crate::gfx::{sh, Font, Rgba};
use crate::ui::cx::Cx;
use crate::ui::el::{sub, El, Key};
use crate::ui::{FG, HL_V19, RADIUS, WIN_H, WIN_W};

use super::button::icon_btn;

const POP_IN: Bezier = Bezier::new(0.3, 1.2, 0.5, 1.0);

/// The popup window's fill. The drawing's `var(--menu)` (rgba(42,42,48,.74)) relies on `backdrop-filter: blur(30px)` over
/// EVERYTHING behind it; here that blur only reaches the menu's own picture - the desktop under the window's glass is not
/// in it, so 26 % of the real desktop (only the window's 13 px blur) showed through and the text was hard to read (the owner
/// Oct 8: "mute settings window is hard to see ... very hard to see anything on that screen"). So the window uses
/// `var(--pop)` (rgba(36,37,43,.94)), the drawing's own opaque popup fill.
pub fn dlg_bg() -> Rgba {
    crate::ui::POP()
}
/// The dim behind a popup window (`.dlgw` rgba(4,6,12,.32) in the drawing): stronger for the same reason, so the page
/// behind steps back.
pub const SCRIM_A: f32 = 0.55;

/// The dim behind a popup: dark rgba(4,6,12) at `SCRIM_A`; light (Order 033) the drawing's `#sw.light .dlgw{background:rgba(30,32,40,.14)}`
/// (its fill is the opaque light `--pop` already). `wo` = the fade-in 0..1.
pub fn scrim(wo: f32) -> Rgba {
    if crate::ui::is_light() {
        Rgba::rgba(30, 32, 40, 0.14 * wo)
    } else {
        Rgba::rgba(4, 6, 12, SCRIM_A * wo)
    }
}

/// The popup window's outer shadow: `.dlg{box-shadow:...,0 0 0 .5px rgba(0,0,0,.35),0 24px 60px rgba(0,0,0,.45)}`; light:
/// `#sw.light .dlg{box-shadow:inset 0 0 0 .5px var(--hl),0 0 0 .5px rgba(0,0,0,.14),0 24px 60px rgba(0,0,0,.2)}`.
pub fn dlg_shadow() -> [crate::gfx::Shadow; 2] {
    let (a, b) = if crate::ui::is_light() { (0.14, 0.2) } else { (0.35, 0.45) };
    [sh(0.0, 0.0, 0.0, 0.5, Rgba(0.0, 0.0, 0.0, a)), sh(0.0, 24.0, 60.0, 0.0, Rgba(0.0, 0.0, 0.0, b))]
}

/// Its inner rim: `inset 0 0 0 .5px var(--hl), inset 0 1px 0 rgba(255,255,255,.06)`; light: the --hl ring only.
pub fn dlg_inset() -> Vec<crate::gfx::Shadow> {
    if crate::ui::is_light() {
        return vec![sh(0.0, 0.0, 0.0, 0.5, crate::ui::hl())];
    }
    vec![sh(0.0, 0.0, 0.0, 0.5, crate::ui::hl()), sh(0.0, 1.0, 0.0, 0.0, Rgba(1.0, 1.0, 1.0, 0.06))]
}

/// The window: `width` (`.dlg` 388, `.mmdlg` 430, `.upddlg` 340), `title`, `body` (its content boxes), `footer` (the
/// `.dft` buttons, right-aligned) and whether it has the × (`noX`). `opened_at` = when it opened (the open motion:
/// the dim behind fades in .18 s ease-out; the window rises 8 px from scale .965 in .34 s cubic-bezier(.3,1.2,.5,1)).
/// A click on the × = `Ev::Click(sub(key, "x"))`, beside the window = `Ev::Click(sub(key, "out"))` - the page closes it.
/// `.dlgw{position:absolute;inset:0;z-index:9;place-items:center;border-radius:inherit;background:rgba(4,6,12,.32)}`
/// `.dlg{padding:18px 18px 16px;border-radius:14px;background:var(--menu);backdrop-filter:blur(30px) saturate(180%);
///   box-shadow:inset 0 0 0 .5px var(--hl),inset 0 1px 0 rgba(255,255,255,.06),0 0 0 .5px rgba(0,0,0,.35),0 24px 60px rgba(0,0,0,.45)}`
/// `.dlg.mdlg{display:flex;flex-direction:column;max-height:468px;padding-bottom:14px}`
/// `.dlh{display:flex;align-items:center;gap:6px;margin:-5px -7px 12px 0;min-height:28px}` `.dlh h3{flex:1;min-width:0;
///   font:600 15px/20px "Segoe UI Variable Display";letter-spacing:-.01em}` `.dft{display:flex;justify-content:flex-end;gap:8px;margin-top:18px}`
#[allow(clippy::too_many_arguments)]
pub fn dialog(cx: &mut Cx, key: Key, width: f32, title: &str, body: Vec<El>, footer: Vec<El>, close_x: bool, opened_at: f64) -> El {
    let age = cx.now - opened_at;
    let rm = cx.rm;
    let wo = if rm { (age / 160.0).clamp(0.0, 1.0) as f32 } else { crate::anim::EASE_OUT_CSS.ease((age / 180.0).clamp(0.0, 1.0)) as f32 };
    let bp = if rm { (age / 160.0).clamp(0.0, 1.0) } else { (age / 340.0).clamp(0.0, 1.0) };
    // at rest exactly 1 (the ease ends at 0.99999994: the scale then moved the text 1 px)
    let be = if bp >= 1.0 { 1.0 } else { POP_IN.ease(bp) as f32 };
    if age < 400.0 {
        cx.st.busy = true;
    }
    let mut head = El::row().center().gap(6.0).margin(-5.0, -7.0, 12.0, 0.0).min_h(28.0).none().child(
        El::text(title, Font::display(15.0, 600).ls(-150), FG(), 20.0).flex1(),
    );
    if close_x {
        // miniDlg's `.dlx`: `'aria-label':'Close',title:'Close'`
        head = head.child(icon_btn(cx, sub(key, "x"), "x", 9.0, 1.5).title("Close"));
    }
    let mut win = El::col()
        .w(width)
        .max_h(468.0)
        .pad(18.0, 18.0, 14.0, 18.0)
        .radius(14.0)
        .bg(dlg_bg())
        .backdrop(30.0, 1.8)
        .shadow(&dlg_shadow())
        .inset(&dlg_inset())
        .items(AlignItems::STRETCH)
        .opacity(if rm { be } else { be.min(1.0) })
        // the window itself takes its clicks (`Ev::Click(sub(key, "win"))`, pages ignore it): a click on its blank parts
        // is not a click beside it
        .on_click(sub(key, "win"))
        .child(head)
        // `.mdb{min-height:0;overflow-y:auto;margin:0 -18px;padding:0 18px 2px}`
        .child(El::col().items(AlignItems::STRETCH).margin(0.0, -18.0, 0.0, -18.0).pad(0.0, 18.0, 2.0, 18.0).children(body).style(|s| {
            s.flex_shrink = 1.0;
            s.min_size.height = taffy::style::LengthPercentageAuto::length(0.0);
        }));
    if !rm {
        win = win.translate(0.0, 8.0 * (1.0 - be)).scale(0.965 + 0.035 * be);
    }
    if !footer.is_empty() {
        win = win.child(El::row().justify(JustifyContent::FLEX_END).gap(8.0).margin(18.0, 0.0, 0.0, 0.0).children(footer));
    }
    let _ = EASE;
    El::grid()
        .abs(0.0, 0.0, 0.0, 0.0)
        .size(WIN_W, WIN_H)
        .place_center()
        .radius(RADIUS)
        .bg(scrim(wo))
        .z(9)
        .on_click(sub(key, "out"))
        .child(win)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::Gfx;
    use crate::ui::cx::State;
    use crate::ui::el::key;
    use crate::ui::lay::Laid;

    /// REVIEW_014_item1 HOLD 3: a click on the window's own blank parts (its title, padding, text) is the window's
    /// (`win`, pages ignore it), never `out` (beside it) - which closed Mute settings & co. on an inside click.
    #[test]
    fn a_click_inside_the_window_is_not_a_click_beside_it() {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let mut cx = Cx::new(0.0, true, &g, &mut st);
        let body = vec![El::text("Some text of the window", Font::new(13.0, 400), FG(), 18.0)];
        let d = dialog(&mut cx, key("dlg"), 388.0, "Mute settings", body, vec![], true, -10000.0);
        let l = Laid::new(&g, El::block().w(WIN_W).h(WIN_H).child(d), WIN_W, Some(WIN_H));
        let (wx, wy, ww, wh) = l.rect_of(sub(key("dlg"), "win")).unwrap();
        let click_at = |x: f32, y: f32| l.hit(x, y).and_then(|(i, _)| l.clickable(i));
        // the title (top left inside the padding), the body text, a blank spot at the bottom right
        for (x, y) in [(wx + 24.0, wy + 22.0), (wx + 30.0, wy + wh - 24.0), (wx + ww - 6.0, wy + wh - 6.0)] {
            assert_eq!(click_at(x, y), Some(sub(key("dlg"), "win")), "inside at {x},{y}");
        }
        // beside it: the dim
        assert_eq!(click_at(6.0, 6.0), Some(sub(key("dlg"), "out")));
    }
}

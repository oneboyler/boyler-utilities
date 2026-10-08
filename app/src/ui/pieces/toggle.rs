//! The on/off switch, 44 x 24, one size everywhere (menu-v22 `.tg`).

use crate::anim::{Bezier, EASE};
use crate::gfx::{sh, Rgba};
use crate::ui::cx::Cx;
use crate::ui::el::{Cursor, El, Key, RADIUS_PILL};
use crate::ui::{cmix, ACC, TRK, WHITE};

const KNOB: Bezier = Bezier::new(0.3, 0.7, 0.2, 1.0);

/// `.tg{width:44px;height:24px;border-radius:12px;background:var(--trk);transition:background .2s ease}` `.tg.on{background:var(--acc)}`
/// `.tg::after{left:2px;top:2px;width:20px;height:20px;border-radius:50%;background:#fff;
///   box-shadow:0 1px 3px rgba(0,0,0,.3),0 0 0 .5px rgba(0,0,0,.06);transition:transform .24s cubic-bezier(.3,.7,.2,1)}`
/// `.tg.on::after{transform:translateX(20px)}` `.tg:disabled{opacity:.38}`. Click = `Ev::Click(key)`.
pub fn toggle(cx: &mut Cx, key: Key, on: bool, disabled: bool) -> El {
    let v = if on { 1.0 } else { 0.0 };
    let col = cx.tr(key, 1, v, 200.0, EASE);
    let k = cx.tr(key, 2, v, 240.0, KNOB);
    let knob = El::block()
        .abs(2.0, 2.0, f32::NAN, f32::NAN)
        .size(20.0, 20.0)
        .radius(RADIUS_PILL)
        .bg(WHITE)
        .shadow(&[sh(0.0, 1.0, 3.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.3)), sh(0.0, 0.0, 0.0, 0.5, Rgba(0.0, 0.0, 0.0, 0.06))])
        .translate(20.0 * k, 0.0)
        .no_hit();
    let mut t = El::block().size(44.0, 24.0).none().radius(12.0).bg(cmix(TRK(), ACC(), col)).child(knob);
    if disabled {
        t = t.opacity(0.38);
    } else {
        t = t.on_click(key).cursor(Cursor::Hand);
    }
    t
}

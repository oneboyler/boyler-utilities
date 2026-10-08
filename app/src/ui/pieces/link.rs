//! The small blue text link (menu-v22 `#sw .lnk`), e.g. "Mute settings", "Change path".

use crate::gfx::Font;
use crate::ui::cx::Cx;
use crate::ui::el::{Cursor, El, Key};
use crate::ui::ACC;

/// `#sw .lnk{border:0;padding:0;background:transparent;color:var(--acc);font-size:12px;line-height:16px}`
/// `.lnk:hover{text-decoration:underline;text-underline-offset:2px}`. `size` = 12 (11.5 in the Screenshots header).
pub fn link(cx: &mut Cx, key: Key, label: &str, size: f32) -> El {
    let hv = cx.hovered(key);
    El::text(label, Font::new(size, 400).ls(0), ACC(), 16.0).align(crate::gfx::Align::Center).underline(hv).none().on_click(key).cursor(Cursor::Hand)
}

//! Small labels (menu-v22): the badge `.tag` (and its green / teal kinds), the header's live note `.pclive`, the
//! header's "via" note `.svia`, the live status pill `.stp`.

use crate::gfx::{Font, Rgba};
use crate::ui::el::{lh, El, RADIUS_PILL};
use crate::ui::{CTL, FG3, GREEN, VZ2};

/// The badge's colour.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    /// `.tag`: background var(--ctl), colour var(--fg3)
    Plain,
    /// `.tag.gsb2`: rgba(48,209,88,.16), #4cd964
    Green,
    /// `.tag.gtag`: rgba(47,214,196,.15), var(--vz2)
    Teal,
}

/// `.tag{flex:none;height:16px;padding:0 6px;border-radius:8px;background:var(--ctl);color:var(--fg3);font-size:10px;
///   font-weight:600;line-height:16px;letter-spacing:.02em}`
pub fn tag(text: &str, tone: Tone) -> El {
    let (bg, fg) = match tone {
        Tone::Plain => (CTL(), FG3()),
        Tone::Green => (Rgba::rgba(48, 209, 88, 0.16), Rgba::hex(0x4cd964)),
        Tone::Teal => (Rgba::rgba(47, 214, 196, 0.15), VZ2()),
    };
    El::row().center().h(16.0).none().pad(0.0, 6.0, 0.0, 6.0).radius(8.0).bg(bg).child(El::text(text, Font::new(10.0, 600).ls(200), fg, 16.0))
}

/// `.pclive{display:flex;align-items:center;gap:6px;font-size:11px;color:var(--fg3);white-space:nowrap}`
/// `.pclive i{width:6px;height:6px;border-radius:50%;background:var(--green)}` - e.g. "Live only while this page is open".
pub fn live_note(text: &str) -> El {
    El::row()
        .center()
        .gap(6.0)
        .none()
        .child(El::block().size(6.0, 6.0).none().radius(RADIUS_PILL).bg(GREEN()))
        .child(El::text(text, Font::new(11.0, 400), FG3(), lh(11.0, 1.35)))
}

/// `.svia{display:flex;align-items:center;gap:6px;font-size:11px;color:var(--fg3);white-space:nowrap}`
/// `.svia svg{width:13px;height:13px;stroke:currentColor;stroke-width:1.4}` - e.g. the Defender shield + "Microsoft Defender".
pub fn via(icon: &str, text: &str) -> El {
    El::row().center().gap(6.0).none().child(El::icon(icon, 13.0, 1.4, FG3())).child(El::text(text, Font::new(11.0, 400), FG3(), lh(11.0, 1.35)))
}

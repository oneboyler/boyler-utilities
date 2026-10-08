//! The info note (menu-v22 `.inote`, Order 025): an amber (or calm grey) info icon + one line of text, under a group's
//! rows (Display overlay, Storage drive health), in a card's fold, or wrapped inside a small popup window.

use crate::gfx::Font;
use crate::ui::el::{El};
use crate::ui::{AMBER, FG2, FG3, HAIR};

/// Where the note sits (one class combination of the drawing each).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Opts {
    /// padding (top, right, bottom, left)
    pub pad: (f32, f32, f32, f32),
    /// the hairline on top (`::before`) starting this far from the left; None = `::before{display:none}`
    pub line_left: Option<f32>,
    /// `white-space:normal` (the text wraps) - else one line
    pub wrap: bool,
    /// margin-top
    pub mt: f32,
}

// The three uses v22 has (the plain one-line `.inote`, the card-fold `.card .xin>.lockb>.inote` and a bare `.fson` match no
// element of v22, so they are not offered).
/// `.mmd .inote{white-space:normal}` / `.inote.hlw` - the last line under rows, `padding:9px 12px 10px`, the hairline from
/// 12 px, the text wraps (Audio's Mute settings window inside its `.isub`; Storage's drive health)
pub const ROW_WRAP: Opts = Opts { pad: (9.0, 12.0, 10.0, 12.0), line_left: Some(12.0), wrap: true, mt: 0.0 };
/// `.fsodlg .inote.fson{margin-top:10px;padding:0 2px}` (`.inote.fson{white-space:normal}` `::before{display:none}`) -
/// Tweaks' "Fullscreen optimizations off" window
pub const FSODLG: Opts = Opts { pad: (0.0, 2.0, 0.0, 2.0), line_left: None, wrap: true, mt: 10.0 };
/// `.inote.udw` - Apps' uninstall window: `padding:4px 0 0`, wraps, no line
pub const UDW: Opts = Opts { pad: (4.0, 0.0, 0.0, 0.0), line_left: None, wrap: true, mt: 0.0 };

/// The info note.
///
/// `.inote{position:relative;display:flex;align-items:center;gap:7px;padding:9px 12px 10px;font-size:11px;line-height:15px;
///   color:var(--fg2);white-space:nowrap}` `::before{left:12px;right:0;top:0;height:1px;background:var(--hair)}`
/// `.inote i{display:block;flex:none}` `.inote svg{width:14px;height:14px;stroke:var(--amber);stroke-width:1.5}` (`ICON.info`)
/// `.inote.calm svg{stroke:var(--fg3)}`. `calm` = the grey icon. No states of its own (a locked card dims it with its
/// `.lockb` parent).
pub fn inote(text: &str, calm: bool, o: &Opts) -> El {
    let (t, r, b, l) = o.pad;
    let mut tx = El::text(text, Font::new(11.0, 400), FG2(), 15.0);
    if o.wrap {
        tx = tx.wrapping().shrink(1.0).min_w(0.0);
    } else {
        tx = tx.none();
    }
    let mut n = El::row()
        .center()
        .gap(7.0)
        .pad(t, r, b, l)
        .margin(o.mt, 0.0, 0.0, 0.0)
        .child(El::icon("info", 14.0, 1.5, if calm { FG3() } else { AMBER() }).no_hit())
        .child(tx);
    if let Some(x) = o.line_left {
        n = n.child(El::block().abs(x, 0.0, 0.0, f32::NAN).h(1.0).bg(HAIR()).no_hit());
    }
    n
}

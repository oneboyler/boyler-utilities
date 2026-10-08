//! The small header button (menu-v22 `.mbtn`, Order 025): sits in a group header's right part (`group::gh_right`) -
//! Network "Flush DNS" + "DNS Automatic ▾", Timers "+ New timer", Your PC "Copy all", Controller "Set … ▾".

use crate::anim::EASE;
use crate::gfx::sh;
use crate::ui::cx::Cx;
use crate::ui::el::{lh, sub, Cursor, El, Key};
use crate::ui::{cmix, CTL, CTL_H, FG, FG2, FG3, GREEN, HAIR};

use super::btn_font;

/// What the button shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mb<'a> {
    /// text only ("Flush DNS", "Copy all")
    Text(&'a str),
    /// `<i>` + svg 12 px (an ICON name) + text ("+ New timer": `plus12`)
    Icon(&'a str, &'a str),
    /// the short "done" state: `.done` (green) with `ICON.dcheck` + the word ("Flushed", "Copied"); the page shows it
    /// for `DONE_MS` after the click, then the normal text again
    Done(&'a str),
    /// `.mbtn.dnsb`: "DNS" (fg2) + the value + the admin shield `.rq.adm` + the chevron (Network)
    Dns(&'a str),
    /// `.mbtn.pdset`: "Set" (fg3) + the value + the chevron (Controller)
    Set(&'a str),
}

/// How long the drawing shows the done state (`setTimeout(…,1600)`).
pub const DONE_MS: f64 = 1600.0;

/// The small header button.
///
/// `#sw .mbtn{display:inline-flex;align-items:center;gap:5px;height:22px;padding:0 9px;border-radius:6px;background:var(--ctl);
///   box-shadow:inset 0 0 0 .5px var(--hair);font-size:11.5px;font-weight:500;color:var(--fg);white-space:nowrap;
///   transition:background-color .12s ease,color .2s ease,transform .12s ease}` `:hover{background:var(--ctl-h)}`
/// `.mbtn:active{transform:scale(.96)}` `#sw .mbtn.done{color:var(--green)}` `#sw .mbtn i{display:block}`
/// `.mbtn svg{width:12px;height:12px;stroke:currentColor;stroke-width:1.8}`
/// `#sw .mbtn.dnsb{gap:4px;padding:0 6px 0 8px}` `.dnsb .dnsl{color:var(--fg2);font-weight:500}`
/// `.dnsb .rq{width:14px;height:14px;margin:0 -1px;background:transparent!important}` `.dnsb .rq svg{width:11px;height:11px}`
/// (`.rq{color:var(--fg3)}` `.rq:hover{color:var(--fg)}`; its svg's stroke is `.mbtn svg{stroke-width:1.8}` - same
/// specificity as `.rq svg{stroke-width:1.4}`, later in the sheet) `.dnsb .dch svg{width:8px;height:12px;
/// stroke:var(--fg2);stroke-width:1.5}` `#sw .pdset{gap:4px;flex:none}` `.pdset .pdsl{color:var(--fg3);font-weight:500}`
/// `.pdset svg{width:7px;height:11px;stroke:var(--fg2);stroke-width:1.5}`.
/// `wait` = `#sw .mbtn.wait` while a change is being made (the drawing's inline `opacity:.55` wins over the class's .5;
/// no clicks). The drawing has no disabled or "menu open" look: a button that cannot act looks as at rest.
/// Clicks: `Ev::Click(key)`; the DNS button's shield is `sub(key, "rq")` (hover only).
pub fn mbtn(cx: &mut Cx, key: Key, what: Mb, wait: bool) -> El {
    // `.wait{pointer-events:none}`: no hover, no press while waiting
    let hv = if wait { 0.0 } else { cx.hover_t(key, 120.0, EASE) };
    let pr = if wait { 0.0 } else { cx.active_t(key, 120.0, EASE) };
    let done = cx.tr(key, 5, if matches!(what, Mb::Done(_)) { 1.0 } else { 0.0 }, 200.0, EASE);
    let fg = cmix(FG(), GREEN(), done);
    let font = btn_font(11.5, 500);
    let lhv = lh(11.5, 1.35);
    let txt = |s: &str, c| El::text(s, font, c, lhv).none();
    let (gap, pl, pr_) = match what {
        Mb::Dns(_) => (4.0, 8.0, 6.0),
        Mb::Set(_) => (4.0, 9.0, 9.0),
        _ => (5.0, 9.0, 9.0),
    };
    let mut kids: Vec<El> = Vec::new();
    match what {
        Mb::Text(s) => kids.push(txt(s, fg)),
        Mb::Icon(ic, s) => {
            kids.push(El::icon(ic, 12.0, 1.8, fg).no_hit());
            kids.push(txt(s, fg));
        }
        Mb::Done(s) => {
            kids.push(El::icon("dcheck", 12.0, 1.8, fg).no_hit());
            kids.push(txt(s, fg));
        }
        Mb::Dns(v) => {
            let rk = sub(key, "rq");
            let rh = cx.hover_t(rk, 120.0, EASE);
            kids.push(txt("DNS", FG2()));
            kids.push(txt(v, fg));
            kids.push(
                El::block()
                    .size(14.0, 14.0)
                    .none()
                    .margin(0.0, -1.0, 0.0, -1.0)
                    .radius(5.0)
                    .key(rk)
                    // `#sw .mbtn i{display:block}` beats `.rq{display:grid}`: the svg is an inline box centred by the
                    // button's `text-align:center`, at the top of the 14 px box (Chromium: svg at x + 1.5, y + 0)
                    .child(El::icon("shield", 11.0, 1.8, cmix(FG3(), FG(), rh)).abs(1.5, 0.0, f32::NAN, f32::NAN).no_hit()),
            );
            // `.dch` is `display:contents`: the svg (8 x 12) itself is the flex item
            kids.push(chev(8.0, 12.0));
        }
        Mb::Set(v) => {
            kids.push(txt("Set", FG3()));
            kids.push(txt(v, fg));
            kids.push(chev(7.0, 11.0));
        }
    }
    let mut b = El::row()
        .center()
        .gap(gap)
        .h(22.0)
        .pad(0.0, pr_, 0.0, pl)
        .radius(6.0)
        .bg(cmix(CTL(), CTL_H(), hv))
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
        .none()
        .scale(1.0 - 0.04 * pr)
        .children(kids);
    if wait {
        b = b.opacity(0.55).key(key).no_hit();
    } else {
        b = b.on_click(key).cursor(Cursor::Hand);
    }
    b
}

/// The chevron svg `ICON.chev` (viewBox 9 x 14) in a w x h box: SVG's default `xMidYMid meet` (`El::icon_fit`).
fn chev(w: f32, h: f32) -> El {
    El::icon_fit("chev", w, h, 1.5, FG2()).none().no_hit()
}

/// A group header with a right part (where `.mbtn`s sit): `.gh` (`group::gh`) + `.gh .ghr{margin-left:auto;display:flex;
/// align-items:center;gap:10px;font-weight:400;color:var(--fg2)}`. `left` = after the title (e.g. a count), `right` =
/// the `.ghr` children.
pub fn gh_with(title: &str, left: Vec<El>, right: Vec<El>) -> El {
    // Lane K's `group::gh` (the title row) + the `.ghr` part
    super::group::gh(title).children(left).child(El::row().center().gap(10.0).ml_auto().children(right))
}

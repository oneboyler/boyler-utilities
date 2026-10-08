//! The key field, menu-v22's small "Keybind" style (`.kf` + its keycaps `.kc`): empty it says "Keybind"; set it shows
//! the keys as caps with a small clear ×; listening it says "Press a key…" (or the held modifiers + "+ …") with a
//! breathing blue ring. The keys manager (Order 014 item 2) does the capture and refuses keys used elsewhere; the page
//! passes what to show.

use crate::anim::EASE;
use crate::gfx::{sh, Font, Rgba};
use crate::ui::cx::Cx;
use crate::ui::el::{sub, Cursor, El, Key};
use crate::ui::{cmix, ACC, ACC_S, CTL, CTL_H, FG, FG2, FG3, HAIR, KEY};

/// What the field shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Show<'a> {
    /// no key yet: "Bind" (`.kph.none`; the owner Oct 8: "rename keybind to just bind")
    Empty,
    /// a key: "Ctrl + Shift + M" shown as caps
    Set(&'a str),
    /// waiting for a key: the modifiers held so far ("Ctrl + Alt") or None
    Listening(Option<&'a str>),
}

/// `.kc{height:18px;padding:0 6px;border-radius:4px;background:var(--key);box-shadow:inset 0 0 0 .5px var(--hair),
///   0 1px 0 rgba(0,0,0,.22);font:600 11px/1 var(--font);color:var(--fg)}`
fn cap(text: &str) -> El {
    El::row()
        .center()
        .h(18.0)
        .none()
        .pad(0.0, 6.0, 0.0, 6.0)
        .radius(4.0)
        .bg(KEY())
        .shadow(&[sh(0.0, 1.0, 0.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.22))])
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
        .child(El::text(text, Font::new(11.0, 600), FG(), 11.0))
}

/// `.kf .plus{font-size:11px;color:var(--fg3)}`
fn plus(t: &str) -> El {
    El::text(t, Font::new(11.0, 400), FG3(), crate::ui::el::lh(11.0, 1.35)).none()
}

/// `.kf{display:flex;align-items:center;gap:4px;height:26px;min-width:0;padding:0 4px;border-radius:7px;background:var(--ctl);
///   box-shadow:inset 0 0 0 .5px var(--hair);transition:background .15s ease,transform .12s ease}` `:hover{background:var(--ctl-h)}`
/// `:active{transform:scale(.98)}` `::after{box-shadow:0 0 0 3px var(--acc-s),inset 0 0 0 1px var(--acc);opacity:0}`
/// `.listen::after{opacity:1;animation:breathe 1.7s ease-in-out .2s infinite}` (1 -> .42 -> 1)
/// `.keys{flex:1;gap:4px;padding-left:1px}` `.kph{font-size:12px;color:var(--fg2);padding:0 5px}` `.kph.none{color:var(--fg3)}`
/// `.clr{width:18px;height:18px;border-radius:50%;color:var(--fg3)}` (only when set; svg 8 px; `:hover{background:var(--ctl-h);
/// color:var(--fg)}`). A click on the field = `Ev::Click(key)` (start listening), on the × = `Ev::Click(sub(key, "clr"))`.
/// `inline` = `.kf.inl` (headers, rows: `.keys{padding-left:0}`, no clear ×).
pub fn keyfield(cx: &mut Cx, key: Key, show: Show, listen_since: f64, inline: bool) -> El {
    let hv = cx.hover_t(key, 150.0, EASE);
    let pr = cx.active_t(key, 120.0, EASE);
    let listening = matches!(show, Show::Listening(_));
    let ring = cx.tr(key, 3, if listening { 1.0 } else { 0.0 }, 180.0, EASE);
    let (drop_at, shake_x) = motion(cx, key, &show, listen_since);
    let now = cx.now;
    let rm = cx.rm;
    let mut keys = El::row().center().gap(4.0).flex1_auto().pad(0.0, 0.0, 0.0, if inline { 0.0 } else { 1.0 });
    let caps = |mut k: El, s: &str, drop: Option<f64>| {
        for (i, p) in s.split(" + ").enumerate() {
            if i > 0 {
                k = k.child(plus("+"));
            }
            let mut c = cap(p);
            if let Some(t0) = drop {
                let (op, dy, sc) = cap_drop(now - t0 - i as f64 * 40.0, rm);
                c = c.opacity(op).translate(0.0, dy).scale(sc);
            }
            k = k.child(c);
        }
        k
    };
    let kph = |t: &str, c: Rgba| El::text(t, Font::new(12.0, 400), c, crate::ui::el::lh(12.0, 1.35)).none().pad(0.0, 5.0, 0.0, 5.0);
    keys = match &show {
        Show::Empty => keys.child(kph("Bind", FG3())),
        Show::Set(s) => caps(keys, s, drop_at),
        Show::Listening(None) => keys.child(kph("Press a key\u{2026}", FG2())),
        Show::Listening(Some(h)) => caps(keys, h, None).child(plus("+ \u{2026}")),
    };
    let mut f = El::row()
        .center()
        .gap(4.0)
        .h(26.0)
        .none()
        .pad(0.0, 4.0, 0.0, 4.0)
        .radius(7.0)
        .bg(cmix(CTL(), CTL_H(), hv))
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, HAIR())])
        .scale(1.0 - 0.02 * pr)
        .translate(shake_x, 0.0)
        .on_click(key)
        .cursor(Cursor::Hand)
        .child(keys);
    if let (Show::Set(_), false) = (&show, inline) {
        let ck = sub(key, "clr");
        let ch = cx.hover_t(ck, 150.0, EASE);
        f = f.child(
            El::block()
                .size(18.0, 18.0)
                .none()
                .radius(9.0)
                .bg(CTL_H().mul_a(ch))
                .place_center()
                .on_click(ck)
                .cursor(Cursor::Hand)
                // `#sw button{color:inherit}` beats `.kf .clr{color:var(--fg3)}`: the × is --fg
                .child(El::icon("x", 8.0, 1.5, FG()).no_hit()),
        );
    }
    if ring > 0.001 {
        // breathe: after .2 s, 1.7 s ease-in-out, 1 -> .42 -> 1
        let mut op = ring;
        if listening {
            cx.st.busy = true;
            let t = (cx.now - listen_since - 200.0).max(0.0) % 1700.0 / 1700.0;
            let half = if t < 0.5 { t * 2.0 } else { (1.0 - t) * 2.0 };
            let e = crate::anim::Bezier::new(0.42, 0.0, 0.58, 1.0).ease(half) as f32;
            op *= 1.0 - 0.58 * e;
        }
        f = f.child(El::block().abs(0.0, 0.0, 0.0, 0.0).radius(7.0).shadow(&[sh(0.0, 0.0, 0.0, 3.0, ACC_S())]).inset(&[sh(0.0, 0.0, 0.0, 1.0, ACC())]).opacity(op).no_hit());
    }
    f
}

/// Order 045: the field's two motions (menu-v22 `commitCap`, L3371-3382): when its key is taken the caps drop in (the
/// drop-in's start, while it runs), and on "Already used by …" the field shakes (its x offset now). The keys manager
/// says when it took / refused a key (`Services::key_bound_at` / `key_refused_at`); the field remembers that it was
/// listening (`State::kf_listen`), so a key shown on its own (the page opening) never drops in.
fn motion(cx: &mut Cx, key: Key, show: &Show, listen_since: f64) -> (Option<f64>, f32) {
    let (bound, refused) = crate::services::with(|s| (s.key_bound_at, s.key_refused_at)).unwrap_or((None, None));
    let now = cx.now;
    let mut shake = 0.0;
    match show {
        Show::Listening(_) => {
            cx.st.kf_listen.insert(key, listen_since);
            cx.st.kf_set.remove(&key);
            // `anim(f.el,[translateX 0,-4,3,-2,0],{duration:300,easing:'ease-out'})`, not under reduced motion
            if let Some(t) = refused.filter(|t| *t >= listen_since && now - t < 300.0) {
                if !cx.rm {
                    cx.st.busy = true;
                    shake = shake_x(crate::anim::EASE_OUT_CSS.ease((now - t) / 300.0) as f32);
                }
            }
        }
        Show::Set(s) => {
            if let Some(since) = cx.st.kf_listen.remove(&key) {
                // (the keys manager's time and the frame's are one clock; a time ahead of the frame is not trusted)
                if let Some(t) = bound.filter(|t| *t >= since && *t <= now + 1.0) {
                    cx.st.kf_set.insert(key, t);
                }
            }
            if let Some(&t0) = cx.st.kf_set.get(&key) {
                let n = s.split(" + ").count() as f64;
                let dur = if cx.rm { 150.0 } else { 320.0 };
                if now - t0 < dur + (n - 1.0) * 40.0 {
                    cx.st.busy = true;
                    return (Some(t0), 0.0);
                }
                cx.st.kf_set.remove(&key);
            }
        }
        Show::Empty => {
            cx.st.kf_listen.remove(&key);
            cx.st.kf_set.remove(&key);
        }
    }
    (None, shake)
}

/// The shake's keyframes at eased progress `p`: translateX 0, -4, 3, -2, 0 px at 0, .25, .5, .75, 1.
fn shake_x(p: f32) -> f32 {
    const K: [f32; 5] = [0.0, -4.0, 3.0, -2.0, 0.0];
    let f = (p.clamp(0.0, 1.0) * 4.0).min(3.999);
    let i = f as usize;
    K[i] + (K[i + 1] - K[i]) * (f - i as f32)
}

/// One keycap's drop-in `t` ms after its own start (each cap 40 ms after the one before): `{opacity:0,translateY(-6px)
/// scale(.9)} -> {opacity:1,translateY(1px) scale(1.03), offset:.6} -> {opacity:1,none}`, 320 ms EASE_OUT, `fill:'backwards'`
/// (before its start it shows the first frame); reduced motion: opacity 0 -> 1 over 150 ms. Returns (opacity, dy, scale).
fn cap_drop(t: f64, rm: bool) -> (f32, f32, f32) {
    if rm {
        return ((t / 150.0).clamp(0.0, 1.0) as f32, 0.0, 1.0);
    }
    let p = crate::anim::EASE_OUT.ease((t / 320.0).clamp(0.0, 1.0)) as f32;
    if p < 0.6 {
        let q = p / 0.6;
        (q, -6.0 + 7.0 * q, 0.9 + 0.13 * q)
    } else {
        let q = (p - 0.6) / 0.4;
        (1.0, 1.0 - q, 1.03 - 0.03 * q)
    }
}

/// A key field bound to a keys-manager action (the keys manager does the capture and the refusals): what it shows comes
/// from `cx.key_field(action)`. Hand its events to `action_event`.
pub fn action_field(cx: &mut Cx, key: Key, action: &str, inline: bool) -> El {
    let (set, listening, _err) = cx.key_field(action);
    let (show, since) = match (&listening, &set) {
        (Some((held, since)), _) => (Show::Listening(held.as_deref()), *since),
        (None, Some(k)) => (Show::Set(k), 0.0),
        (None, None) => (Show::Empty, 0.0),
    };
    keyfield(cx, key, show, since, inline)
}

/// The field's events: a click listens, the × clears, Esc / focus lost stops listening. True = it was this field's.
pub fn action_event(ev: &crate::ui::cx::Ev, cx: &mut Cx, key: Key, action: &str) -> bool {
    use crate::ui::cx::Ev;
    match ev {
        Ev::Click(k) if *k == key => cx.listen_key(action),
        Ev::Click(k) if *k == sub(key, "clr") => cx.clear_key(action),
        Ev::Blur(k) if *k == key => cx.stop_listening(),
        _ => return false,
    }
    true
}

/// The red line under a key field (`.kerr{font-size:11.5px;line-height:15px;color:var(--red)}`), e.g. "Already used by Mic mute".
pub fn error_line(text: &str) -> El {
    El::text(text, Font::new(11.5, 400), crate::ui::RED(), 15.0)
}

/// A key row of a header (`.shk{display:flex;align-items:center;gap:8px;font-size:12px;color:var(--fg2);white-space:nowrap}`): an
/// optional label ("Screenshot key") and the inline key field.
pub fn key_row(cx: &mut Cx, key: Key, label: Option<&str>, show: Show, listen_since: f64) -> El {
    let mut r = El::row().center().gap(8.0).none();
    if let Some(l) = label {
        r = r.child(El::text(l, Font::new(12.0, 400), FG2(), crate::ui::el::lh(12.0, 1.35)).none());
    }
    r.child(keyfield(cx, key, show, listen_since, true))
}

//! The Voice to text tab (menu-v22 page `vtt`, Order 024; Order 060: Windows' own voice typing). One calm column: the header
//! (the small "Keybind" field), the words box that fills the page, and at the bottom trash (red) · mic · copy.
//!
//! The words box is a plain text box. The big mic focuses it and opens **Windows' voice typing (Win+H)** for it - the shortcut
//! is sent only while this window is the one in front (`bu-voice`, never another app or a game) - and the words land in the box as
//! typed characters. Win+H has its own permission in Windows, so the page asks about no privacy switch and has no language
//! picker (voice typing has its own, in its own panel). Copy / Clear / typing a fix by hand stay.
//!
//! Every box is the drawing's CSS (quoted on each builder). `.pg.vpg.on{display:flex;flex-direction:column}`: the frame lays
//! pages out as a block, so the words box gets the height flex would give it (the page's 444 px content height minus the
//! header and the controls) - the same boxes as Chromium's.

use taffy::style::{AlignItems, JustifyContent};

use crate::anim::{EASE, EASE_OUT, EASE_OUT_CSS};
use crate::gfx::{sh, Align, Font, Gfx, Rgba};
use crate::pages::{Env, Page};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{key, lh, Cursor, El, Key, RADIUS_PILL};
use crate::ui::pieces::{self, keyfield};
use crate::ui::{cmix, ACC, CTL, CTL_H, FG, FG2, FG3, GREEN, HAIR, PAGE_H, RED, WHITE};

use bu_voice::{Clipboard, VoiceTyping};

const K_KEY: Key = key("vtt.key");
const K_TXT: Key = key("vtt.text");
const K_MIC: Key = key("vtt.mic");
const K_CLR: Key = key("vtt.clear");
const K_CPY: Key = key("vtt.copy");
/// The keys-manager action of the small key field: opens voice typing in the box (the menu opens on this tab).
pub const ACTION: &str = "vtt.talk";

const VK_ESCAPE: u16 = 0x1B;
const VK_BACK: u16 = 0x08;
const VK_DELETE: u16 = 0x2E;
const VK_LEFT: u16 = 0x25;
const VK_RIGHT: u16 = 0x27;
const VK_HOME: u16 = 0x24;
const VK_END: u16 = 0x23;

/// `.vtx{font:400 19px/1.55 "Segoe UI Variable Text";letter-spacing:-.008em}` (-0.152 px)
const F_WORDS: Font = Font::new(19.0, 400).ls(-152);
/// `.vtx{padding:16px 20px 18px}`
const TX_PAD: (f32, f32, f32) = (16.0, 20.0, 18.0);
/// the page's content height: `.pg{position:absolute;inset:0;padding:2px 26px 18px}` in the 464 px page area
const CONTENT_H: f32 = PAGE_H - 20.0;
/// `.ph{min-height:32px;margin:0 2px 8px}`
const HEADER_H: f32 = 40.0;
/// `.vctl{padding:14px 0 6px}` around the 68 px mic
const CTL_H_PX: f32 = 14.0 + 68.0 + 6.0;

/// Tests only: the fake voice typing's "our window is in front" switch and its sent-shortcuts counter, the fake clipboard.
#[cfg(test)]
type Probe = (std::sync::Arc<std::sync::atomic::AtomicBool>, std::sync::Arc<std::sync::atomic::AtomicU32>, std::sync::Arc<std::sync::Mutex<String>>);

#[derive(Default)]
pub struct Voice {
    env: Env,
    /// Windows' voice typing (Win+H, guarded to our own window) and the clipboard: fakes in every test copy
    vt: Option<Box<dyn VoiceTyping>>,
    clip: Option<Box<dyn Clipboard>>,
    /// the words (typed, dictated by Windows' voice typing, or fixed by hand)
    text: String,
    /// the caret (a char index) while the box has the focus
    caret: Option<usize>,
    caret_at: f64,
    /// the mic was pressed on and not off again: Windows' voice typing is (probably) open for the box. The page cannot see
    /// Windows' own panel, so closing it there leaves this on until the next mic press (**unclear**: no read of its state).
    typing: bool,
    mic_at: Option<f64>,
    copied_at: Option<f64>,
    /// the last thing said (shown by the frame's toast: `fresh` = not handed to it yet)
    toast: Option<(String, f64)>,
    fresh: bool,
    /// the key opened the menu to open voice typing (the frame's `jump("listen")`): done on the next build
    start_on_build: bool,
    /// when the placeholder last appeared (its `kin` fade)
    ph_since: f64,
    ph_was: bool,
    #[cfg(test)]
    probe: Option<Probe>,
}

impl Voice {
    fn say(&mut self, text: &str, now: f64) {
        self.toast = Some((text.to_string(), now));
        self.fresh = true;
    }

    fn words(&self) -> usize {
        self.text.split_whitespace().count()
    }

    fn chars(&self) -> usize {
        self.text.chars().count()
    }

    /// The box takes the focus with its caret at the end (the mic, the key, Clear / Copy while voice typing is open: the words
    /// Windows types go to the focused box, so the focus must come back to it).
    fn focus_box(&mut self, cx: &mut Cx) {
        cx.focus(Some(K_TXT));
        if self.caret.is_none() {
            self.caret = Some(self.chars());
            self.caret_at = cx.now;
        }
    }

    /// The mic / the key: focus our own text box, then open (or close) Windows' voice typing for it. The shortcut is sent only
    /// when the box has the focus AND this window is in front (the real layer refuses otherwise) - never to another app.
    fn mic(&mut self, cx: &mut Cx) {
        let now = cx.now;
        self.focus_box(cx);
        if !cx.focused(K_TXT) {
            return;
        }
        let Some(vt) = self.vt.as_mut() else { return };
        match vt.toggle() {
            Ok(true) => {
                self.typing = !self.typing;
                if self.typing {
                    self.mic_at = Some(now);
                }
            }
            Ok(false) => self.say("Voice typing only opens while this window is in front", now),
            Err(e) => self.say(&format!("Voice typing could not open ({e})"), now),
        }
    }

    /// Insert `s` at the caret (typed, or Windows' voice typing).
    fn insert(&mut self, s: &str, now: f64) {
        let mut v: Vec<char> = self.text.chars().collect();
        let i = self.caret.unwrap_or(v.len()).min(v.len());
        let add: Vec<char> = s.chars().collect();
        let n = add.len();
        v.splice(i..i, add);
        self.text = v.into_iter().collect();
        self.caret = Some(i + n);
        self.caret_at = now;
    }
}

impl Page for Voice {
    fn id(&self) -> &'static str {
        "vtt"
    }
    fn name(&self) -> &'static str {
        "Voice to text"
    }
    fn icon(&self) -> &'static str {
        "wave"
    }

    fn open(&mut self, env: &Env, now: f64) {
        self.env = env.clone();
        // fake voice typing + fake clipboard in every test copy (no key is sent); Windows' otherwise
        if env.fake() {
            let (vt, ours, sent) = bu_voice::fake::FakeVoiceTyping::new();
            let (clip, got) = bu_voice::fake::FakeClipboard::new();
            #[cfg(test)]
            {
                self.probe = Some((ours, sent, got));
            }
            #[cfg(not(test))]
            let _ = (ours, sent, got);
            self.vt = Some(Box::new(vt));
            self.clip = Some(Box::new(clip));
        } else {
            self.vt = Some(Box::new(bu_voice::real::WinVoiceTyping));
            self.clip = Some(Box::new(bu_voice::real::WinClipboard));
        }
        self.ph_since = now;
        self.ph_was = true;
    }

    fn close(&mut self) {
        // words are not kept across a close (Windows' voice typing is not ours to close: see `typing`)
        let env = std::mem::take(&mut self.env);
        *self = Voice { env, ..Voice::default() };
    }

    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        let now = cx.now;
        let has_text = self.words() > 0;
        // the caret only blinks (530 ms on / off): built again at each flip, no frames between
        if self.caret.is_some() {
            cx.wake_every(530.0, self.caret_at);
        }
        // the key: voice typing opens in the box (the menu opened here) or, open already, closes - like the mic button
        if std::mem::take(&mut self.start_on_build) {
            self.mic(cx);
        }
        // what was said goes to the frame's toast (above the page, never blocking it)
        if std::mem::take(&mut self.fresh) {
            if let Some((t, _)) = &self.toast {
                cx.toast(t);
            }
        }

        // ---- header: `.vhr{display:flex;align-items:center;gap:10px}` = `.shk.vkey` (the small field, the keys manager's `vtt.talk`)
        let vhr = El::row().center().gap(10.0).none().child(El::row().center().gap(8.0).none().child(keyfield::action_field(cx, K_KEY, ACTION, true).title("Click to change")));
        let mut out = vec![pieces::header(self.name(), Some(vhr))];

        // ---- `.kerr.vErr` ("Already used by …" right under the key): `.kerr{font-size:11.5px;line-height:15px;color:var(--red)}`
        // `.vErr{margin:-2px 2px 6px auto;text-align:right}`
        let mut err_h = 0.0;
        if let Some(e) = &cx.key_field(ACTION).2 {
            err_h = 15.0 - 2.0 + 6.0;
            out.push(El::row().justify(JustifyContent::FLEX_END).margin(-2.0, 2.0, 6.0, 0.0).child(El::text(e.clone(), Font::new(11.5, 400), RED(), 15.0)));
        }

        // ---- the words box `.grp.vbox{position:relative;flex:1 1 auto;min-height:150px;overflow:hidden}`
        let box_h = (CONTENT_H - HEADER_H - err_h - CTL_H_PX).max(150.0);
        let caret = self.caret.filter(|_| cx.focused(K_TXT));
        let caret_on = caret.is_some() && ((now - self.caret_at) % 1060.0) < 530.0;
        let t2 = self.text.clone();
        let words_paint = El::paint(move |g, (x, y, w, h)| paint_words(g, &t2, (x, y, w, h), caret.filter(|_| caret_on))).abs(0.0, 0.0, 0.0, 0.0);
        // the box is always a text box: a click puts the caret, keys (and Windows' voice typing) type
        let vtx = El::block().abs(0.0, 0.0, 0.0, 0.0).child(words_paint.no_hit()).key(K_TXT).cursor(Cursor::Text);
        let mut vbox = pieces::group::grp(vec![]).h(box_h).clip().child(vtx);
        // `.vph` "Your words show up here": only with no words; `.vph.on{animation:kin .25s ease-out}`
        let ph = !has_text;
        if ph && !self.ph_was {
            self.ph_since = now;
        }
        self.ph_was = ph;
        if ph {
            let p = ((now - self.ph_since) / 250.0).clamp(0.0, 1.0);
            if p < 1.0 {
                cx.st.busy = true;
            }
            let e = EASE_OUT_CSS.ease(p) as f32;
            // `.vph{position:absolute;inset:0;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:6px}`
            // `.vph b{font:600 19px/1.3 "Segoe UI Variable Display";letter-spacing:-.01em;color:var(--fg3)}`
            let vph = El::col()
                .abs(0.0, 0.0, 0.0, 0.0)
                .items(AlignItems::CENTER)
                .justify(JustifyContent::CENTER)
                .gap(6.0)
                .opacity(e)
                .translate(0.0, -3.0 * (1.0 - e))
                .child(El::text("Your words show up here", Font::display(19.0, 600).ls(-190), FG3(), lh(19.0, 1.3)).align(Align::Center).no_hit())
                .no_hit();
            vbox = vbox.child(vph);
        }
        // `.vst{position:absolute;right:14px;bottom:10px;display:flex;align-items:center;gap:7px;height:16px;font-size:11.5px;
        //   color:var(--fg2);white-space:nowrap}` - the red `.vdot` + "Windows voice typing is open" / "n words"; nothing when idle
        let vst_text = if self.typing {
            Some("Windows voice typing is open \u{b7} the mic closes it".to_string())
        } else if has_text {
            Some(format!("{} words \u{b7} click the text to fix a word", self.words()))
        } else {
            None
        };
        if let Some(t) = vst_text {
            let mut row = El::row().abs(f32::NAN, f32::NAN, 14.0, 10.0).center().gap(7.0).h(16.0).no_hit();
            if self.typing {
                // `.vdot{width:7px;height:7px;border-radius:50%;background:var(--red)}` (still: nothing animates while it waits)
                row = row.child(El::block().size(7.0, 7.0).none().radius(RADIUS_PILL).bg(RED()));
            }
            vbox = vbox.child(row.child(El::text(t, Font::new(11.5, 400), FG2(), lh(11.5, 1.35))));
        }
        out.push(vbox);

        // ---- `.vctl{flex:none;display:flex;flex-direction:column;align-items:center;justify-content:flex-end;padding:14px 0 6px}`
        // `.vrow{display:flex;align-items:center;gap:30px}`: Clear (red trash) · mic · Copy
        let copied = self.copied_at.map(|t| now - t < 1400.0).unwrap_or(false);
        // the check mark goes back to the copy icon at a known moment (its fade is the buttons' transitions)
        if let Some(t) = self.copied_at.filter(|_| copied) {
            cx.wake_at(t + 1400.0);
        }
        let clr = vbt(cx, K_CLR, "trash", Tone::Red, !has_text).tip("Clear");
        let cpy = vbt(cx, K_CPY, if copied { "check2" } else { "copy" }, if copied { Tone::Done } else if has_text { Tone::Acc } else { Tone::Plain }, !has_text).tip("Copy");
        let mic = self.mic_button(cx, self.typing);
        out.push(
            El::col()
                .items(AlignItems::CENTER)
                .justify(JustifyContent::FLEX_END)
                .pad(14.0, 0.0, 6.0, 0.0)
                .child(El::row().center().gap(30.0).child(clr).child(mic).child(cpy)),
        );

        out
    }

    fn event(&mut self, ev: &Ev, cx: &mut Cx) {
        let now = cx.now;
        match ev {
            Ev::Click(k) if *k == K_MIC => self.mic(cx),
            Ev::Click(k) if *k == K_CLR => {
                self.text.clear();
                self.caret = None;
                // voice typing may still be open: the focus (and with it the words Windows types) goes back to the box
                if self.typing {
                    self.focus_box(cx);
                }
            }
            Ev::Click(k) if *k == K_CPY => {
                let t = self.text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !t.is_empty() {
                    match self.clip.as_mut().map(|c| c.set_text(&t)) {
                        Some(Ok(())) => {
                            self.copied_at = Some(now);
                            self.say("Copied to clipboard", now);
                        }
                        Some(Err(e)) => self.say(&format!("Could not copy ({e})"), now),
                        None => {}
                    }
                }
                if self.typing {
                    self.focus_box(cx);
                }
            }
            // the small key field: the keys manager listens, captures and refuses (Esc / focus lost stops it)
            e if keyfield::action_event(e, cx, K_KEY, ACTION) => {}
            Ev::Key(k, VK_ESCAPE) if *k == K_KEY => cx.stop_listening(),
            // the words box (contentEditable in the drawing): a click puts the caret, characters and keys edit
            Ev::Press(k, x, y, r) if *k == K_TXT => {
                self.caret = Some(hit_char(cx.g, &self.text, *r, *x, *y));
                self.caret_at = now;
            }
            Ev::Char(k, c) if *k == K_TXT => {
                // (Windows' voice typing types like a keyboard: a line break is Enter)
                let c = if *c == '\r' { '\n' } else { *c };
                if c == '\n' || !c.is_control() {
                    self.insert(&c.to_string(), now);
                }
            }
            Ev::Key(k, vk) if *k == K_TXT => {
                let mut s: Vec<char> = self.text.chars().collect();
                let i = self.caret.unwrap_or(s.len()).min(s.len());
                let n = match *vk {
                    VK_BACK if i > 0 => {
                        s.remove(i - 1);
                        i - 1
                    }
                    VK_DELETE if i < s.len() => {
                        s.remove(i);
                        i
                    }
                    VK_LEFT => i.saturating_sub(1),
                    VK_RIGHT => (i + 1).min(s.len()),
                    VK_HOME => 0,
                    VK_END => s.len(),
                    _ => i,
                };
                if matches!(*vk, VK_BACK | VK_DELETE) {
                    self.text = s.iter().collect();
                }
                self.caret = Some(n);
                self.caret_at = now;
            }
            Ev::Blur(k) if *k == K_TXT => self.caret = None,
            _ => {}
        }
        cx.dirty = true;
    }

    fn describe(&self) -> String {
        format!("typing={} words={} caret={:?} toast={:?} text={:?}", self.typing, self.words(), self.caret, self.toast.as_ref().map(|t| t.0.as_str()).unwrap_or(""), self.text)
    }

    fn start(&self, s: &mut crate::services::Services) {
        // the small key field's key: the menu opens on this tab and voice typing opens in the box (pressed again there: it closes)
        s.add_action(crate::keys::Action::new(ACTION, "Voice to text", "vtt"), |down| {
            if down {
                crate::services::show_menu("vtt", Some("listen"));
            }
        });
    }

    fn jump(&mut self, target: &str) {
        if target == "listen" {
            self.start_on_build = true;
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Tone {
    Plain,
    Red,
    Acc,
    Done,
}

/// The round icon buttons beside the mic: `#sw .vbt{display:inline-flex;align-items:center;justify-content:center;border-radius:8px;
///   background:var(--ctl);box-shadow:inset 0 0 0 .5px var(--hair);transition:background-color .15s ease,opacity .2s ease,
///   transform .12s ease,color .2s ease}` `#sw .vbt.vib{width:40px;height:40px;border-radius:50%}` `.vbt.vib svg{width:17px;height:17px}`
/// `.vbt svg{stroke:currentColor;stroke-width:1.5}` `#sw .vbt:hover{background:var(--ctl-h)}` `.vbt:active{transform:scale(.97)}`
/// `#sw .vbt:disabled{opacity:.38;pointer-events:none}` `#sw .vbt.vred{color:var(--red)}` `#sw .vbt.vred:hover{background:rgba(255,69,58,.16)}`
/// `#sw .vbt.acc{background:var(--acc);color:#fff;box-shadow:inset 0 0 0 .5px rgba(255,255,255,.2),0 1px 3px rgba(0,0,0,.22)}`
/// `.acc:hover{filter:brightness(1.08)}` `#sw .vbt.done{background:var(--ctl);color:var(--green);box-shadow:inset 0 0 0 .5px var(--hair)}`
/// Its tip (`data-tip`: "Clear" / "Copy") = the frame's shared tooltip (`El::tip`, set by the caller).
fn vbt(cx: &mut Cx, k: Key, icon: &str, tone: Tone, disabled: bool) -> El {
    let hv = if disabled { 0.0 } else { cx.hover_t(k, 150.0, EASE) };
    let pr = if disabled { 0.0 } else { cx.active_t(k, 120.0, EASE) };
    let op = cx.tr(k, 5, if disabled { 0.38 } else { 1.0 }, 200.0, EASE);
    let acc = cx.tr(k, 6, if tone == Tone::Acc { 1.0 } else { 0.0 }, 150.0, EASE);
    let bright = |c: Rgba, t: f32| Rgba((c.0 * (1.0 + 0.08 * t)).min(1.0), (c.1 * (1.0 + 0.08 * t)).min(1.0), (c.2 * (1.0 + 0.08 * t)).min(1.0), c.3);
    let plain_bg = match tone {
        Tone::Red => cmix(CTL(), Rgba::rgba(255, 69, 58, 0.16), hv),
        Tone::Done => CTL(),
        _ => cmix(CTL(), CTL_H(), hv),
    };
    let bg = cmix(plain_bg, bright(ACC(), hv), acc);
    let target = match tone {
        Tone::Red => RED(),
        Tone::Acc => WHITE,
        Tone::Done => GREEN(),
        Tone::Plain => FG(),
    };
    let fg = target;
    let mut b = El::block()
        .size(40.0, 40.0)
        .none()
        .radius(RADIUS_PILL)
        .bg(bg.mul_a(op))
        .inset(&[sh(0.0, 0.0, 0.0, 0.5, cmix(HAIR(), Rgba(1.0, 1.0, 1.0, 0.2), acc).mul_a(op))])
        .place_center()
        .scale(1.0 - 0.03 * pr)
        .child(El::icon(icon, 17.0, 1.5, fg).opacity(op).no_hit());
    if acc > 0.001 {
        b = b.shadow(&[sh(0.0, 1.0, 3.0, 0.0, Rgba(0.0, 0.0, 0.0, 0.22 * acc))]);
    }
    if disabled {
        b.key(k).no_hit()
    } else {
        b.on_click(k).cursor(Cursor::Hand)
    }
}

impl Voice {
    /// The mic: `.vmic{position:relative;width:68px;height:68px;border-radius:50%}` over `.vmb` (`position:absolute;inset:0;
    /// border-radius:50%;background:var(--ctl);box-shadow:inset 0 0 0 .5px var(--hair),0 2px 8px rgba(0,0,0,.16);transition:
    /// background-color .2s ease,box-shadow .2s ease`) `.vmic:hover .vmb{background:var(--ctl-h)}` `.vmb svg{width:28px;height:28px;
    /// stroke:var(--acc);stroke-width:1.5;transition:stroke .2s ease}` `.vmic.on .vmb{background:var(--acc);box-shadow:inset 0 0 0 .5px
    /// rgba(255,255,255,.25),0 4px 14px rgba(0,0,0,.22)}` `.vmic.on .vmb svg{stroke:#fff}`. vStart: the core .9 -> 1.06 (55 %) -> 1
    /// in 320 ms (EASE_OUT). (Order 060: no voice-level rings - Windows' voice typing does the listening and gives us no level.)
    fn mic_button(&mut self, cx: &mut Cx, on: bool) -> El {
        let hv = cx.hover_t(K_MIC, 200.0, EASE);
        let ont = cx.tr(K_MIC, 3, if on { 1.0 } else { 0.0 }, 200.0, EASE);
        let mut press = 1.0;
        if let Some(t0) = self.mic_at {
            let age = cx.now - t0;
            if age < 320.0 && !cx.rm {
                cx.st.busy = true;
                let p = EASE_OUT.ease(age / 320.0) as f32;
                press = if p < 0.55 { 0.9 + 0.16 * p / 0.55 } else { 1.06 - 0.06 * (p - 0.55) / 0.45 };
            } else {
                self.mic_at = None;
            }
        }
        // light (Order 033): `#sw.light .vmic:not(.on) .vmb{background:rgba(255,255,255,.72);box-shadow:inset 0 0 0 .5px var(--hair),
        // 0 2px 8px rgba(0,0,0,.08)}` `#sw.light .vmic:not(.on):hover .vmb{background:#fff}`
        let (off_bg, off_sh, on_sh) = if crate::ui::is_light() { (cmix(Rgba(1.0, 1.0, 1.0, 0.72), WHITE, hv), 0.08, 0.14) } else { (cmix(CTL(), CTL_H(), hv), 0.16, 0.06) };
        let core = El::block()
            .abs(0.0, 0.0, 0.0, 0.0)
            .radius(RADIUS_PILL)
            .bg(cmix(off_bg, ACC(), ont))
            .inset(&[sh(0.0, 0.0, 0.0, 0.5, cmix(HAIR(), Rgba(1.0, 1.0, 1.0, 0.25), ont))])
            .shadow(&[sh(0.0, 2.0 + 2.0 * ont, 8.0 + 6.0 * ont, 0.0, Rgba(0.0, 0.0, 0.0, off_sh + on_sh * ont))])
            .place_center()
            .scale(press)
            .no_hit()
            .child(El::icon("mic", 28.0, 1.5, cmix(ACC(), WHITE, ont)));
        El::block().size(68.0, 68.0).none().radius(RADIUS_PILL).on_click(K_MIC).cursor(Cursor::Hand).child(core)
    }
}


/// The words box's lines: (first char, chars) of the words, broken like Blink's pre-wrap at spaces.
fn lines(g: &Gfx, all: &[char], maxw: f32) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut last_space: Option<usize> = None;
    let mut i = 0usize;
    while i < all.len() {
        if all[i] == '\n' {
            out.push((start, i - start));
            start = i + 1;
            last_space = None;
            i += 1;
            continue;
        }
        if all[i] == ' ' {
            last_space = Some(i);
        } else {
            let s: String = all[start..=i].iter().collect();
            if g.text_width(&s, F_WORDS) > maxw + 0.001 {
                if let Some(sp) = last_space.filter(|sp| *sp > start) {
                    // the space hangs at the end of the line
                    out.push((start, sp + 1 - start));
                    start = sp + 1;
                    last_space = None;
                    continue;
                }
            }
        }
        i += 1;
    }
    out.push((start, all.len() - start));
    out
}

/// `.vtx` content: the words (--fg) and, while the box has the focus, its caret (a char index). The box scrolls to its end
/// like `vTx.scrollTop = scrollHeight`.
fn paint_words(g: &Gfx, text: &str, (x, y, w, h): (f32, f32, f32, f32), caret: Option<usize>) {
    let (pt, px, pb) = TX_PAD;
    let all: Vec<char> = text.chars().collect();
    if all.is_empty() && caret.is_none() {
        return;
    }
    let maxw = w - 2.0 * px;
    let lh_ = lh(19.0, 1.55);
    let ls = lines(g, &all, maxw);
    let total = ls.len() as f32 * lh_ + pt + pb;
    let sy = (total - h).max(0.0);
    g.push_clip(x, y, w, h);
    for (k, (s, n)) in ls.iter().enumerate() {
        let ly = y + pt + lh_ * k as f32 - sy;
        if ly > y + h || ly + lh_ < y {
            continue;
        }
        let a: String = all[*s..s + n].iter().collect();
        g.text(&a, F_WORDS, x + px, ly, lh_, FG(), Align::Left, 0.0);
    }
    // the line box's baseline: half-leading of 29.45 around Segoe's 19 px ascent/descent ≈ 21.6 from the line top (measured from
    // Blink's vertical-align:-3px caret: its bottom sits 3 px under the baseline)
    let base = |ly: f32| ly + (lh_ - 19.0 * 1.33) / 2.0 + 19.0 * 1.06;
    if let Some(ci) = caret {
        let ci = ci.min(all.len());
        let k = ls.iter().rposition(|(s, _)| *s <= ci).unwrap_or(0);
        let (s, _) = ls[k];
        let pre: String = all[s..ci].iter().collect();
        let (cxp, ly) = (x + px + g.text_width(&pre, F_WORDS), y + pt + lh_ * k as f32 - sy);
        g.fill_rect(cxp, base(ly) - 18.0, 1.0, 22.0, FG());
    }
    g.pop_clip();
}

/// The char index of the words nearest a click at (px, py) in the words box `r`.
fn hit_char(g: &Gfx, text: &str, r: (f32, f32, f32, f32), px: f32, py: f32) -> usize {
    let (pt, padx, pb) = TX_PAD;
    let all: Vec<char> = text.chars().collect();
    let lh_ = lh(19.0, 1.55);
    let ls = lines(g, &all, r.2 - 2.0 * padx);
    let total = ls.len() as f32 * lh_ + pt + pb;
    let sy = (total - r.3).max(0.0);
    let k = (((py - r.1 - pt + sy) / lh_).floor().max(0.0) as usize).min(ls.len() - 1);
    let (s, n) = ls[k];
    let mut best = (s, f32::MAX);
    for i in s..=s + n {
        let pre: String = all[s..i].iter().collect();
        let d = (r.0 + padx + g.text_width(&pre, F_WORDS) - px).abs();
        if d < best.1 {
            best = (i, d);
        }
    }
    best.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::cx::State as CssState;
    use std::sync::atomic::Ordering;

    fn page() -> (Voice, Gfx) {
        let mut v = Voice::default();
        v.open(&Env { test: true, ..Env::default() }, 0.0);
        (v, Gfx::new(1.0))
    }

    /// One event with the frame's state kept between events (the focus the page took stays).
    fn ev(v: &mut Voice, g: &Gfx, st: &mut CssState, e: Ev, now: f64) {
        let mut cx = Cx::new(now, false, g, st);
        v.event(&e, &mut cx);
    }

    fn sent(v: &Voice) -> u32 {
        v.probe.as_ref().unwrap().1.load(Ordering::SeqCst)
    }

    fn put_in_front(v: &Voice, ours: bool) {
        v.probe.as_ref().unwrap().0.store(ours, Ordering::SeqCst);
    }

    #[test]
    fn opens_on_the_fakes_with_an_empty_box() {
        let (v, _) = page();
        assert!(v.describe().contains("typing=false words=0"), "{}", v.describe());
        assert_eq!(sent(&v), 0, "opening the tab sends nothing");
    }

    /// Order 060: the mic focuses our own box first, then sends Win+H once; pressed again it sends it again (Windows' voice
    /// typing is a toggle) and the page shows it as off.
    #[test]
    fn the_mic_focuses_the_box_then_opens_voice_typing_and_again_closes_it() {
        let (mut v, g) = page();
        let mut st = CssState::default();
        st.focus = Some(K_MIC); // the press on the mic gave the mic the focus
        ev(&mut v, &g, &mut st, Ev::Click(K_MIC), 1.0);
        assert_eq!(st.focus, Some(K_TXT), "our own text box has the focus");
        assert_eq!(sent(&v), 1);
        assert!(v.typing && v.caret == Some(0), "{}", v.describe());
        ev(&mut v, &g, &mut st, Ev::Click(K_MIC), 2.0);
        assert_eq!(sent(&v), 2);
        assert!(!v.typing);
    }

    /// Order 060: the shortcut goes only to our own window - with another app (or a game) in front nothing is sent and the
    /// page says why.
    #[test]
    fn nothing_is_sent_while_another_window_is_in_front() {
        let (mut v, g) = page();
        put_in_front(&v, false);
        let mut st = CssState::default();
        ev(&mut v, &g, &mut st, Ev::Click(K_MIC), 1.0);
        assert_eq!(sent(&v), 0);
        assert!(!v.typing);
        assert!(v.describe().contains("only opens while this window is in front"), "{}", v.describe());
        put_in_front(&v, true);
        ev(&mut v, &g, &mut st, Ev::Click(K_MIC), 2.0);
        assert_eq!(sent(&v), 1);
        assert!(v.typing);
    }

    /// The words Windows' voice typing types arrive as characters for the box: they land at the caret; characters meant for
    /// another control never do; a line break is Enter.
    #[test]
    fn the_words_voice_typing_types_land_in_the_box() {
        let (mut v, g) = page();
        let mut st = CssState::default();
        ev(&mut v, &g, &mut st, Ev::Click(K_MIC), 1.0);
        for c in "Hey, are you on?\r".chars() {
            ev(&mut v, &g, &mut st, Ev::Char(K_TXT, c), 2.0);
        }
        for c in "nope".chars() {
            ev(&mut v, &g, &mut st, Ev::Char(K_CLR, c), 2.0);
        }
        assert!(v.describe().ends_with("text=\"Hey, are you on?\\n\""), "{}", v.describe());
        assert_eq!(v.words(), 4);
    }

    #[test]
    fn copy_takes_the_words_and_clear_empties_and_both_keep_voice_typing_pointed_at_the_box() {
        let (mut v, g) = page();
        let mut st = CssState::default();
        ev(&mut v, &g, &mut st, Ev::Click(K_MIC), 1.0);
        for c in "one  two".chars() {
            ev(&mut v, &g, &mut st, Ev::Char(K_TXT, c), 2.0);
        }
        st.focus = Some(K_CPY); // the press on Copy gave it the focus
        ev(&mut v, &g, &mut st, Ev::Click(K_CPY), 3.0);
        assert!(v.describe().contains("toast=\"Copied to clipboard\""), "{}", v.describe());
        assert_eq!(v.probe.as_ref().unwrap().2.lock().unwrap().as_str(), "one two");
        assert_eq!(st.focus, Some(K_TXT), "voice typing is still open: the box gets the focus back");
        st.focus = Some(K_CLR);
        ev(&mut v, &g, &mut st, Ev::Click(K_CLR), 4.0);
        assert!(v.describe().contains("words=0"), "{}", v.describe());
        assert_eq!(st.focus, Some(K_TXT));
        // nothing open: Clear leaves the focus where it is
        let (mut v, g) = page();
        let mut st = CssState::default();
        st.focus = Some(K_CLR);
        ev(&mut v, &g, &mut st, Ev::Click(K_CLR), 1.0);
        assert_eq!(st.focus, Some(K_CLR));
    }

    #[test]
    fn fixing_a_word_by_hand() {
        let (mut v, g) = page();
        let mut st = CssState::default();
        for c in "one two".chars() {
            ev(&mut v, &g, &mut st, Ev::Char(K_TXT, c), 0.5);
        }
        ev(&mut v, &g, &mut st, Ev::Press(K_TXT, 1000.0, 20.0, (0.0, 0.0, 544.0, 316.0)), 1.0);
        assert_eq!(v.caret, Some(7)); // a click right of the text: the end
        ev(&mut v, &g, &mut st, Ev::Key(K_TXT, VK_BACK), 2.0);
        ev(&mut v, &g, &mut st, Ev::Key(K_TXT, VK_BACK), 2.0);
        ev(&mut v, &g, &mut st, Ev::Key(K_TXT, VK_BACK), 2.0);
        for c in "three".chars() {
            ev(&mut v, &g, &mut st, Ev::Char(K_TXT, c), 3.0);
        }
        assert!(v.describe().ends_with("text=\"one three\""), "{}", v.describe());
        ev(&mut v, &g, &mut st, Ev::Blur(K_TXT), 4.0);
        assert_eq!(v.caret, None);
    }

    #[test]
    fn closing_the_tab_drops_the_words() {
        let (mut v, g) = page();
        let mut st = CssState::default();
        ev(&mut v, &g, &mut st, Ev::Click(K_MIC), 1.0);
        v.close();
        assert!(v.vt.is_none() && v.toast.is_none() && !v.typing && v.text.is_empty());
    }

    /// The key field's action is registered at app start (the keys manager captures it); the key opens the menu here and
    /// voice typing opens in the box, pressed again it closes.
    #[test]
    fn the_key_is_the_keys_managers_and_opens_voice_typing_in_the_box() {
        crate::services::init(windows::Win32::Foundation::HWND::default(), true);
        let names: Vec<String> = crate::services::with(|s| s.action_list().into_iter().map(|a| a.id).collect()).unwrap();
        assert!(names.iter().any(|n| n == ACTION), "{names:?}");
        let (mut v, g) = page();
        let mut st = CssState::default();
        v.jump("listen");
        {
            let mut cx = Cx::new(1.0, false, &g, &mut st).for_page("vtt");
            let _ = v.build(&mut cx);
        }
        assert!(v.typing && sent(&v) == 1 && st.focus == Some(K_TXT), "{}", v.describe());
        v.jump("listen");
        {
            let mut cx = Cx::new(2.0, false, &g, &mut st).for_page("vtt");
            let _ = v.build(&mut cx);
        }
        assert!(!v.typing && sent(&v) == 2, "{}", v.describe());
        crate::services::shutdown();
    }

    /// The page has no language picker and no "Online speech recognition" line or link any more.
    #[test]
    fn no_language_picker_and_no_online_speech_line() {
        let (mut v, g) = page();
        let mut st = CssState::default();
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        let kids = v.build(&mut cx);
        let root = El::block().w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids);
        let laid = crate::ui::lay::Laid::new(&g, root, 600.0, None);
        for k in ["vtt.lang", "vtt.langs", "vtt.fix"] {
            assert!(laid.rect_of(key(k)).is_none(), "{k} is gone");
        }
    }

    /// The page painted off-screen with the app's painter into the lane's scratch folder (looked at, Order 060): empty, with
    /// words and the caret, and with voice typing open.
    #[test]
    fn the_page_is_painted() {
        let dir = std::env::temp_dir().join("BoylerUtilities-test").join("voice-test");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, words, typing) in [("vtt-empty.png", "", false), ("vtt-words.png", "Hey, are you on tonight? Bring your headset and we can queue up after dinner.", false), ("vtt-typing.png", "Hey, are you on", true)] {
            let (mut v, g) = page();
            let mut st = CssState::default();
            for c in words.chars() {
                ev(&mut v, &g, &mut st, Ev::Char(K_TXT, c), 0.5);
            }
            if typing {
                ev(&mut v, &g, &mut st, Ev::Click(K_MIC), 1.0);
            }
            let icons = crate::icons::Icons::new();
            let mut cx = Cx::new(1000.0, false, &g, &mut st);
            let kids = v.build(&mut cx);
            let root = El::block().w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids);
            let laid = crate::ui::lay::Laid::new(&g, root, 600.0, None);
            unsafe {
                let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
            }
            let h = laid.height.ceil() as i32;
            let mut s = crate::gfx::new_surface(600, h).unwrap();
            g.begin(s.canvas());
            g.fill_rect(0.0, 0.0, 600.0, h as f32, Rgba::rgb(20, 24, 40));
            laid.paint(&g, &icons, 0.0, 0.0, None);
            g.end();
            let px = crate::png::from_surface(&mut s);
            crate::png::save_png(&px, &dir.join(name).to_string_lossy()).expect("save");
        }
    }

    #[test]
    fn the_words_box_gets_the_height_flex_gives_it() {
        let (mut v, g) = page();
        let mut st = CssState::default();
        let mut cx = Cx::new(0.0, false, &g, &mut st);
        let kids = v.build(&mut cx);
        let root = El::block().w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids);
        let laid = crate::ui::lay::Laid::new(&g, root, 600.0, None);
        // header 2..34 (+8), the box 42..358, the controls 358..446 = the page's 464 px exactly
        let vbox = laid.nodes.iter().find(|n| n.rect.3 == 316.0).expect("the 316 px words box");
        assert_eq!((vbox.rect.0, vbox.rect.1, vbox.rect.2), (26.0, 42.0, 548.0));
        assert_eq!(laid.nodes[0].rect.3, 464.0);
    }

    /// At rest the page asks for no frames; the box's caret wakes the menu only at its blink flips (530 ms) instead of a
    /// rebuild every frame - and with voice typing open nothing animates either (no level meter any more).
    #[test]
    fn at_rest_nothing_asks_for_frames_and_the_caret_only_wakes_at_its_flips() {
        let (mut v, g) = page();
        let mut st = CssState::default();
        for c in "one two".chars() {
            ev(&mut v, &g, &mut st, Ev::Char(K_TXT, c), 0.5);
        }
        ev(&mut v, &g, &mut st, Ev::Click(K_MIC), 100.0);
        st.wake = None;
        st.busy = false;
        v.mic_at = None;
        let mut cx = Cx::new(200.0, false, &g, &mut st);
        let _ = v.build(&mut cx);
        drop(cx);
        assert!(!st.busy, "no rebuild every frame for the caret or the open voice typing");
        assert_eq!(st.wake, Some(530.5), "built again at the caret's next flip: {:?}", st.wake);
    }
}

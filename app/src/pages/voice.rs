//! The Voice to text tab (menu-v22 page `vtt`, Order 024). One calm column: the header (the small "Keybind" field + the
//! language picker), the words box that fills the page, and at the bottom trash (red) · mic · copy. Listening… / Done show
//! small in the words box's bottom-right corner, nothing when idle. The engine is Windows' online dictation - the speech
//! service Windows' own voice typing (Win+H) uses (`bu-voice`, Order 043); the language list offers ONLY the dictation
//! languages Windows has on this PC (+ "More languages…" = Windows Settings). "Online speech recognition" off, the microphone
//! blocked or no microphone: one line in the words box (+ a link to that Windows setting) - the page never changes a setting.
//!
//! Every box is the drawing's CSS (quoted on each builder). `.pg.vpg.on{display:flex;flex-direction:column}`: the frame lays
//! pages out as a block, so the words box gets the height flex would give it (the page's 444 px content height minus the
//! header and the controls) - the same boxes as Chromium's.

use taffy::style::{AlignItems, JustifyContent};

use crate::anim::{Bezier, EASE, EASE_OUT, EASE_OUT_CSS};
use crate::gfx::{sh, Align, Font, Gfx, Rgba};
use crate::pages::{Env, Page};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, key, lh, Cursor, El, Key, RADIUS_PILL};
use crate::ui::pieces::mitems::{self, It, Place, Row};
use crate::ui::pieces::{self, group, keyfield, link};
use crate::ui::{cmix, ACC, CTL, CTL_H, FG, FG2, FG3, GREEN, HAIR, PAGE_H, RED, WHITE};

use bu_voice::{Dictation, Language, State, VoiceError};

const K_KEY: Key = key("vtt.key");
const K_LANG: Key = key("vtt.lang");
const K_MENU: Key = key("vtt.langs");
const K_TXT: Key = key("vtt.text");
const K_MIC: Key = key("vtt.mic");
const K_CLR: Key = key("vtt.clear");
const K_CPY: Key = key("vtt.copy");
/// the blocked line's link to the Windows setting
const K_FIX: Key = key("vtt.fix");
/// The keys-manager action of the small key field: starts / stops the dictation (the menu opens on this tab).
pub const ACTION: &str = "vtt.talk";

const VK_RETURN: u16 = 0x0D;
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

/// What the page asked Windows to open (a normal copy opens it through `bu-voice`; a test copy only records it, and the
/// test hook shows it in `describe`).
#[derive(Clone, Debug, PartialEq)]
pub enum TempReq {
    /// open Windows Settings › Speech ("More languages…")
    SpeechSettings,
    /// open Windows Settings › Privacy & security › Speech (the blocked line's link: "Online speech recognition" off)
    PrivacySpeech,
    /// open Windows Settings › Privacy & security › Microphone (the blocked line's link: the microphone blocked)
    PrivacyMic,
}

/// The words box's line for what stops every dictation, and its link (label, the Settings page, the test record).
fn blocked_line(e: &VoiceError) -> (&'static str, Option<(&'static str, &'static str, TempReq)>) {
    match e {
        VoiceError::OnlineOff => ("Online speech recognition is off in Windows", Some(("Turn it on in Windows Settings", bu_voice::SETTINGS_PRIVACY_SPEECH, TempReq::PrivacySpeech))),
        VoiceError::MicDenied => ("Windows blocks apps from using the microphone", Some(("Allow it in Windows Settings", bu_voice::SETTINGS_PRIVACY_MIC, TempReq::PrivacyMic))),
        _ => ("No microphone found", None),
    }
}

#[derive(Default)]
pub struct Voice {
    env: Env,
    dict: Option<Dictation>,
    langs: Vec<Language>,
    /// the chosen language (its tag; the settings store's `lang` - kept over a restart - read on the first build)
    lang: String,
    lang_loaded: bool,
    /// the language popup's anchor (the button's box) while it is open
    menu: Option<(f32, f32, f32, f32)>,
    lang_box: (f32, f32, f32, f32),
    /// the popup was just closed by this press (a click on the button closes it, never reopens it)
    just_closed: bool,
    /// the mic's smoothed voice level (the drawing's V.lv) and when it was pressed on (its little bounce)
    lv: f32,
    mic_at: Option<f64>,
    copied_at: Option<f64>,
    /// the last thing said (shown by the frame's toast: `fresh` = not handed to it yet)
    toast: Option<(String, f64)>,
    fresh: bool,
    /// what stops every dictation now ("Online speech recognition" off, the microphone blocked, no microphone): one line in
    /// the words box instead of the placeholder; read on open and again on each mic press
    blocked: Option<VoiceError>,
    /// the key opened the menu to start a dictation (the frame's `jump("listen")`): started on the next build
    start_on_build: bool,
    /// fixing a word: the caret (a char index in the firm words) while the text has focus
    caret: Option<usize>,
    caret_at: f64,
    /// when the placeholder last appeared (its `kin` fade)
    ph_since: f64,
    ph_was: bool,
    pub reqs: Vec<TempReq>,
}

impl Voice {
    fn lang(&self) -> Option<&Language> {
        self.langs.iter().find(|l| l.tag == self.lang).or(self.langs.first())
    }

    fn say(&mut self, text: &str, now: f64) {
        self.toast = Some((text.to_string(), now));
        self.fresh = true;
    }

    fn mic(&mut self, now: f64) {
        let Some(lang) = self.lang().cloned() else {
            self.say("Add a speech language in Windows Settings", now);
            return;
        };
        let Some(d) = self.dict.as_mut() else { return };
        let was = d.state();
        self.caret = None;
        if was != State::Listening {
            // the user may have changed the Windows setting since: read it again (read-only, quick)
            self.blocked = d.check().err().filter(|e| e.blocks());
            if self.blocked.is_some() {
                return;
            }
        }
        match d.toggle(&lang) {
            Ok(()) => {
                if was != State::Listening && d.state() == State::Listening {
                    self.mic_at = Some(now);
                }
            }
            Err(e) if e.blocks() => self.blocked = Some(e),
            Err(e) => self.say(&format!("Voice to text could not start ({e})"), now),
        }
    }

    fn state(&self) -> State {
        self.dict.as_ref().map(|d| d.state()).unwrap_or(State::Idle)
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
        // fake engine + fake clipboard in every test copy; Windows' speech + clipboard otherwise
        let dict = if env.fake() {
            let (clip, _) = bu_voice::fake::FakeClipboard::new();
            let mut eng = bu_voice::fake::FakeEngine::drawing();
            if env.frozen {
                // the comparison pictures: the drawing's sample list (VLANGS)
                let l = |t: &str, n: &str| Language { tag: t.into(), name: n.into(), engine: "fake".into() };
                eng = eng.with_languages(vec![l("en-US", "English (US)"), l("en-GB", "English (UK)"), l("de-DE", "Deutsch")]);
            }
            Dictation::new(Box::new(eng), Box::new(clip))
        } else {
            Dictation::new(Box::new(bu_voice::real::WinSpeech::new()), Box::new(bu_voice::real::WinClipboard))
        };
        // quick reads, no audio: Windows' dictation languages, and whether a Windows switch / no microphone stops it
        self.langs = dict.languages().unwrap_or_default();
        self.blocked = dict.check().err().filter(|e| e.blocks());
        if !self.langs.iter().any(|l| l.tag == self.lang) {
            self.lang = self.langs.first().map(|l| l.tag.clone()).unwrap_or_default();
        }
        self.dict = Some(dict);
        self.ph_since = now;
        self.ph_was = true;
    }

    fn close(&mut self) {
        // stops listening (the Dictation stops its session when dropped); words are not kept across a close
        let env = std::mem::take(&mut self.env);
        *self = Voice { env, ..Voice::default() };
    }

    fn tick(&mut self, now: f64) -> bool {
        let Some(d) = self.dict.as_mut() else { return false };
        let mut changed = d.poll();
        if let Some(e) = d.take_error() {
            let t = match e {
                e if e.blocks() => {
                    let t = blocked_line(&e).0.to_string();
                    self.blocked = Some(e);
                    t
                }
                VoiceError::Offline => "No internet connection \u{b7} Voice to text needs it".to_string(),
                e => format!("Voice to text stopped ({e})"),
            };
            self.toast = Some((t, now));
            self.fresh = true;
            changed = true;
        }
        // vFrame: lv += (t - lv) * (t > lv ? .45 : .14), t = env * (.9 + .1 sin(now / 41)) while listening
        let on = d.state() == State::Listening;
        let t = if on { d.level() * (0.9 + 0.1 * (now / 41.0).sin() as f32) } else { 0.0 };
        let k = if t > self.lv { 0.45 } else { 0.14 };
        self.lv += (t - self.lv) * k;
        if !on && self.lv < 0.004 {
            self.lv = 0.0;
        }
        changed || on || self.lv > 0.0 || d.state() == State::Finishing
    }

    fn build(&mut self, cx: &mut Cx) -> Vec<El> {
        let now = cx.now;
        let st = self.state();
        let (firm, guess, words) = match &self.dict {
            Some(d) => (d.firm().to_string(), d.guess().to_string(), d.words()),
            None => (String::new(), String::new(), 0),
        };
        let has_text = words > 0;
        if matches!(st, State::Listening | State::Finishing) || self.lv > 0.0 || self.caret.is_some() {
            cx.st.busy = true;
        }

        // the chosen language from the settings store (kept over a restart), once per open
        if !self.lang_loaded {
            self.lang_loaded = true;
            let kept = cx.get_str("lang", "");
            if self.langs.iter().any(|l| l.tag == kept) {
                self.lang = kept;
            }
        }
        // the key: the dictation starts (the menu opened here) or, while it listens, stops - like the mic button
        if std::mem::take(&mut self.start_on_build) {
            self.mic(now);
        }
        // what was said goes to the frame's toast (above the page, never blocking it)
        if std::mem::take(&mut self.fresh) {
            if let Some((t, _)) = &self.toast {
                cx.toast(t);
            }
        }

        // ---- header: `.vhr{display:flex;align-items:center;gap:10px}` = `.shk.vkey` (the small field, the keys manager's
        // `vtt.talk`) + `.pu.vlang{width:132px}`
        let label = self.lang().map(|l| l.name.clone()).unwrap_or_else(|| "None installed".into());
        let vhr = El::row()
            .center()
            .gap(10.0)
            .none()
            // `fVtt.el.title='Click to change'` / `vLang.title='Speech language'`
            .child(El::row().center().gap(8.0).none().child(keyfield::action_field(cx, K_KEY, ACTION, true).title("Click to change")))
            .child(pieces::dropdown::dropdown(cx, K_LANG, &label, Some(132.0)).title("Speech language"));
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
        let listening = st == State::Listening;
        let caret = self.caret;
        let caret_on = caret.is_some() && ((now - self.caret_at) % 1060.0) < 530.0;
        let car_on = listening && (now % 1000.0) < 500.0; // `.vcar{animation:vcar 1s steps(1) infinite}` (50 %: opacity 0)
        let (f2, g2) = (firm.clone(), guess.clone());
        let words_paint = El::paint(move |g, (x, y, w, h)| paint_words(g, &f2, &g2, (x, y, w, h), caret.filter(|_| caret_on), car_on))
            .abs(0.0, 0.0, 0.0, 0.0);
        let mut vtx = El::block().abs(0.0, 0.0, 0.0, 0.0).child(words_paint.no_hit());
        if st == State::Done || (st == State::Idle && has_text) {
            vtx = vtx.key(K_TXT).cursor(Cursor::Text);
        }
        let mut vbox = group::grp(vec![]).h(box_h).clip().child(vtx);
        // `.vph` "Your words show up here": only with no words and not listening; `.vph.on{animation:kin .25s ease-out}`
        let ph = !has_text && st != State::Listening && st != State::Finishing;
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
            // blocked (Order 043): the same line says what stops it, the small link under it opens that Windows setting
            let (line, fix) = match &self.blocked {
                Some(b) => blocked_line(b),
                None => ("Your words show up here", None),
            };
            let mut vph = El::col()
                .abs(0.0, 0.0, 0.0, 0.0)
                .items(AlignItems::CENTER)
                .justify(JustifyContent::CENTER)
                .gap(6.0)
                .opacity(e)
                .translate(0.0, -3.0 * (1.0 - e))
                .child(El::text(line, Font::display(19.0, 600).ls(-190), FG3(), lh(19.0, 1.3)).align(Align::Center).no_hit());
            match fix {
                Some((label, ..)) => vph = vph.child(link::link(cx, K_FIX, label, 12.0)),
                None => vph = vph.no_hit(),
            }
            vbox = vbox.child(vph);
        }
        // `.vst{position:absolute;right:14px;bottom:10px;display:flex;align-items:center;gap:7px;height:16px;font-size:11.5px;
        //   color:var(--fg2);white-space:nowrap}` - Listening… (+ the red `.vdot`) / Done · n words; nothing when idle
        let vst_text = match st {
            State::Listening | State::Finishing => Some("Listening\u{2026} \u{b7} Enter or the mic stops".to_string()),
            State::Done if has_text => Some(format!("Done \u{b7} {words} words \u{b7} click the text to fix a word")),
            _ => None,
        };
        if let Some(t) = vst_text {
            let mut row = El::row().abs(f32::NAN, f32::NAN, 14.0, 10.0).center().gap(7.0).h(16.0).no_hit();
            if matches!(st, State::Listening | State::Finishing) {
                // `.vdot{width:7px;height:7px;border-radius:50%;background:var(--red);animation:vdot 1.4s ease-in-out infinite}` (50 %: .3)
                let ph = (now % 1400.0) / 1400.0;
                let half = if ph < 0.5 { ph * 2.0 } else { (1.0 - ph) * 2.0 };
                let op = 1.0 - 0.7 * Bezier::new(0.42, 0.0, 0.58, 1.0).ease(half) as f32;
                row = row.child(El::block().size(7.0, 7.0).none().radius(RADIUS_PILL).bg(RED()).opacity(op));
            }
            vbox = vbox.child(row.child(El::text(t, Font::new(11.5, 400), FG2(), lh(11.5, 1.35))));
        }
        out.push(vbox);

        // ---- `.vctl{flex:none;display:flex;flex-direction:column;align-items:center;justify-content:flex-end;padding:14px 0 6px}`
        // `.vrow{display:flex;align-items:center;gap:30px}`: Clear (red trash) · mic · Copy
        let copied = self.copied_at.map(|t| now - t < 1400.0).unwrap_or(false);
        if copied {
            cx.st.busy = true;
        }
        let clr = vbt(cx, K_CLR, "trash", Tone::Red, !has_text).tip("Clear");
        let cpy = vbt(cx, K_CPY, if copied { "check2" } else { "copy" }, if copied { Tone::Done } else if st == State::Done && has_text { Tone::Acc } else { Tone::Plain }, !has_text).tip("Copy");
        let mic = self.mic_button(cx, listening);
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
        if let Ev::Press(k, ..) = ev {
            if *k != K_LANG {
                self.just_closed = false;
            }
        }
        match ev {
            Ev::Click(k) if *k == K_MIC => self.mic(now),
            Ev::Key(k, VK_RETURN) if *k == K_MIC => {
                if self.state() == State::Listening {
                    self.mic(now);
                    // (Order 045: Enter stopped it - the frame's Enter on a Tab-focused button must not start it again)
                    cx.used = true;
                }
            }
            Ev::Click(k) if *k == K_CLR => {
                if let Some(d) = self.dict.as_mut() {
                    d.clear();
                }
                self.caret = None;
            }
            Ev::Click(k) if *k == K_CPY => {
                let r = self.dict.as_mut().map(|d| d.copy());
                match r {
                    Some(Ok(true)) => {
                        self.copied_at = Some(now);
                        self.say("Copied to clipboard", now);
                    }
                    Some(Err(e)) => self.say(&format!("Could not copy ({e})"), now),
                    _ => {}
                }
            }
            // the language picker (the drawing's openMenu: a click on the open button closes it)
            Ev::Press(k, _, _, r) if *k == K_LANG => self.lang_box = *r,
            Ev::Click(k) if *k == K_LANG => {
                if std::mem::take(&mut self.just_closed) {
                    return;
                }
                self.menu = if self.menu.is_some() { None } else { Some(self.lang_box) };
            }
            // the list's rows: the languages, a line (row n), "More languages…" (row n + 1)
            Ev::Click(k) if (0..self.langs.len() + 2).any(|i| i != self.langs.len() && *k == idx(K_MENU, i)) => {
                let i = (0..self.langs.len() + 2).find(|i| *k == idx(K_MENU, *i)).unwrap_or(0);
                self.menu = None;
                if i < self.langs.len() {
                    if self.state() == State::Listening {
                        // a new language takes effect from the next start
                        if let Some(d) = self.dict.as_mut() {
                            d.stop();
                        }
                    }
                    self.lang = self.langs[i].tag.clone();
                    // kept at once in the settings store (over a restart too)
                    cx.set_str("lang", &self.lang);
                } else {
                    self.say("Windows Settings \u{203a} Speech opens \u{b7} add a language there", now);
                    self.reqs.push(TempReq::SpeechSettings);
                    if !self.env.test {
                        let _ = bu_voice::real::open_settings(bu_voice::SETTINGS_SPEECH);
                    }
                }
            }
            // the blocked line's link: that Windows setting opens (the user changes it there; the next mic press reads it again)
            Ev::Click(k) if *k == K_FIX => {
                if let Some((_, Some((_, uri, req)))) = self.blocked.as_ref().map(blocked_line) {
                    self.reqs.push(req);
                    if !self.env.test {
                        let _ = bu_voice::real::open_settings(uri);
                    }
                }
            }
            // the small key field: the keys manager listens, captures and refuses (Esc / focus lost stops it)
            e if keyfield::action_event(e, cx, K_KEY, ACTION) => {}
            Ev::Key(k, VK_ESCAPE) if *k == K_KEY => cx.stop_listening(),
            // fixing a word once stopped (contentEditable in the drawing): a click puts the caret, keys edit
            Ev::Press(k, x, y, r) if *k == K_TXT => {
                if let Some(d) = self.dict.as_ref() {
                    let firm = d.firm().to_string();
                    self.caret = Some(hit_char(cx.g, &firm, *r, *x, *y));
                    self.caret_at = now;
                }
            }
            Ev::Char(k, c) if *k == K_TXT => {
                if let (Some(d), Some(i)) = (self.dict.as_mut(), self.caret) {
                    if !c.is_control() {
                        let mut s: Vec<char> = d.firm().chars().collect();
                        let i = i.min(s.len());
                        s.insert(i, *c);
                        d.set_text(&s.iter().collect::<String>());
                        self.caret = Some(i + 1);
                        self.caret_at = now;
                    }
                }
            }
            Ev::Key(k, vk) if *k == K_TXT => {
                if let (Some(d), Some(i)) = (self.dict.as_mut(), self.caret) {
                    let mut s: Vec<char> = d.firm().chars().collect();
                    let i = i.min(s.len());
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
                        d.set_text(&s.iter().collect::<String>());
                    }
                    self.caret = Some(n);
                    self.caret_at = now;
                }
            }
            Ev::Blur(k) if *k == K_TXT => self.caret = None,
            _ => {}
        }
        cx.dirty = true;
    }

    fn popup(&mut self, cx: &mut Cx) -> Option<El> {
        let (bx, by, bw, bh) = self.menu?;
        // the drawing's VLANGS: the languages (a tick on the chosen one), a line, "More languages…"; placeMenu(btn, 150)
        let mut list: Vec<Row> = self.langs.iter().map(|l| Row::Item(It::tick(&l.name, l.tag == self.lang))).collect();
        list.push(Row::Sep);
        list.push(Row::Item(It::tick("More languages\u{2026}", false)));
        Some(mitems::menu(cx, K_MENU, &list, Place::Under(bx, by, bw, bh), 150.0))
    }

    fn popup_dismiss(&mut self) {
        if self.menu.take().is_some() {
            self.just_closed = true;
        }
    }

    fn describe(&self) -> String {
        let st = match self.state() {
            State::Idle => "idle",
            State::Listening => "listening",
            State::Finishing => "finishing",
            State::Done => "done",
        };
        let (words, text) = self.dict.as_ref().map(|d| (d.words(), d.text())).unwrap_or((0, String::new()));
        format!(
            "state={st} words={words} lang={} langs={} menu={} blocked={:?} toast={:?} reqs={:?} text={:?}",
            self.lang,
            self.langs.iter().map(|l| l.tag.as_str()).collect::<Vec<_>>().join(","),
            self.menu.is_some(),
            self.blocked,
            self.toast.as_ref().map(|t| t.0.as_str()).unwrap_or(""),
            self.reqs,
            text
        )
    }

    fn start(&self, s: &mut crate::services::Services) {
        // the small key field's key: the menu opens on this tab and the dictation starts (pressed again there: it stops)
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
    /// The mic: `.vmic{position:relative;width:68px;height:68px;border-radius:50%}` with two `.vring`s (`position:absolute;inset:0;
    /// border-radius:50%;background:var(--acc);opacity:0`) under `.vmb` (`position:absolute;inset:0;border-radius:50%;
    /// background:var(--ctl);box-shadow:inset 0 0 0 .5px var(--hair),0 2px 8px rgba(0,0,0,.16);transition:background-color .2s ease,
    /// box-shadow .2s ease`) `.vmic:hover .vmb{background:var(--ctl-h)}` `.vmb svg{width:28px;height:28px;stroke:var(--acc);
    /// stroke-width:1.5;transition:stroke .2s ease}` `.vmic.on .vmb{background:var(--acc);box-shadow:inset 0 0 0 .5px
    /// rgba(255,255,255,.25),0 4px 14px rgba(0,0,0,.22)}` `.vmic.on .vmb svg{stroke:#fff}`. vLevel: ring 1 scale 1 + l·.34, opacity
    /// (listening ? .22 + l·.5 : l·.5); ring 2 scale 1 + l·.62, opacity l·.32; the core scale 1 + l·.07. vStart: the core
    /// .9 -> 1.06 (55 %) -> 1 in 320 ms (EASE_OUT).
    fn mic_button(&mut self, cx: &mut Cx, on: bool) -> El {
        let l = self.lv;
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
        let ring = |s: f32, o: f32| El::block().abs(0.0, 0.0, 0.0, 0.0).radius(RADIUS_PILL).bg(ACC()).scale(s).opacity(o).no_hit();
        let r1 = ring(1.0 + l * 0.34, if on { 0.22 + l * 0.5 } else { l * 0.5 });
        let r2 = ring(1.0 + l * 0.62, l * 0.32);
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
            .scale((1.0 + l * 0.07) * press)
            .no_hit()
            .child(El::icon("mic", 28.0, 1.5, cmix(ACC(), WHITE, ont)));
        El::block().size(68.0, 68.0).none().radius(RADIUS_PILL).on_click(K_MIC).cursor(Cursor::Hand).child(r2).child(r1).child(core)
    }
}

/// The words box's lines: (first char, chars) of the firm + guessed words, broken like Blink's pre-wrap at spaces.
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

/// `.vtx` content: `.vfin` (the firm words, --fg) then `.vint` (the guessed ones, --fg2), then the listening caret `.vcar`
/// (`display:inline-block;width:2px;height:21px;margin-left:3px;vertical-align:-3px;border-radius:1px;background:var(--acc)`).
/// `edit` = the fixing caret (a char index in the firm words). The box scrolls to its end like `vTx.scrollTop = scrollHeight`.
fn paint_words(g: &Gfx, firm: &str, guess: &str, (x, y, w, h): (f32, f32, f32, f32), edit: Option<usize>, listen_caret: bool) {
    let (pt, px, pb) = TX_PAD;
    let mut all: Vec<char> = firm.chars().collect();
    let firm_n = all.len();
    if !guess.is_empty() {
        if !all.is_empty() && *all.last().unwrap() != ' ' {
            all.push(' ');
        }
        all.extend(guess.chars());
    }
    if all.is_empty() && edit.is_none() && !listen_caret {
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
        let fe = firm_n.clamp(*s, s + n);
        let a: String = all[*s..fe].iter().collect();
        let b: String = all[fe..s + n].iter().collect();
        g.text(&a, F_WORDS, x + px, ly, lh_, FG(), Align::Left, 0.0);
        if !b.is_empty() {
            let ax = g.text_width(&a, F_WORDS);
            g.text(&b, F_WORDS, x + px + ax, ly, lh_, FG2(), Align::Left, 0.0);
        }
    }
    let caret_at = |ci: usize| -> (f32, f32) {
        let k = ls.iter().rposition(|(s, _)| *s <= ci).unwrap_or(0);
        let (s, _) = ls[k];
        let pre: String = all[s..ci.min(all.len())].iter().collect();
        (x + px + g.text_width(&pre, F_WORDS), y + pt + lh_ * k as f32 - sy)
    };
    // the line box's baseline: half-leading of 29.45 around Segoe's 19 px ascent/descent ≈ 21.6 from the line top (measured from
    // Blink's vertical-align:-3px caret: its bottom sits 3 px under the baseline)
    let base = |ly: f32| ly + (lh_ - 19.0 * 1.33) / 2.0 + 19.0 * 1.06;
    if let Some(ci) = edit {
        let (cxp, ly) = caret_at(ci.min(firm_n));
        g.fill_rect(cxp, base(ly) - 18.0, 1.0, 22.0, FG());
    }
    if listen_caret {
        let (cxp, ly) = caret_at(all.len());
        g.fill_rr(cxp + 3.0, base(ly) + 3.0 - 21.0, 2.0, 21.0, 1.0, ACC());
    }
    g.pop_clip();
}

/// The char index of the firm words nearest a click at (px, py) in the words box `r`.
fn hit_char(g: &Gfx, firm: &str, r: (f32, f32, f32, f32), px: f32, py: f32) -> usize {
    let (pt, padx, pb) = TX_PAD;
    let all: Vec<char> = firm.chars().collect();
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

    fn page() -> (Voice, Gfx) {
        let mut v = Voice::default();
        v.open(&Env { test: true, ..Env::default() }, 0.0);
        (v, Gfx::new(1.0))
    }

    fn ev(v: &mut Voice, g: &Gfx, e: Ev, now: f64) {
        let mut st = CssState::default();
        let mut cx = Cx::new(now, false, g, &mut st);
        v.event(&e, &mut cx);
    }

    fn run_until(v: &mut Voice, want: State) {
        for i in 0..4000 {
            v.tick(i as f64);
            if v.state() == want {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        panic!("never reached {want:?}: {}", v.describe());
    }

    #[test]
    fn opens_on_the_fake_with_only_the_languages_windows_has() {
        let (v, _) = page();
        assert!(v.describe().contains("state=idle"));
        assert!(v.describe().contains("lang=en-US langs=en-US"));
    }

    #[test]
    fn the_mic_listens_words_arrive_stop_copies_and_clear_empties() {
        let (mut v, g) = page();
        ev(&mut v, &g, Ev::Click(K_MIC), 1.0);
        assert_eq!(v.state(), State::Listening);
        // the drawing's recording: wait for its first firm sentence
        for i in 0..3000 {
            v.tick(i as f64);
            if v.dict.as_ref().unwrap().firm().contains("tonight?") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(v.dict.as_ref().unwrap().firm().starts_with("Hey, are you on tonight?"), "{}", v.describe());
        // Enter on the focused mic stops (the drawing: "Enter or the mic stops")
        ev(&mut v, &g, Ev::Key(K_MIC, VK_RETURN), 2.0);
        run_until(&mut v, State::Done);
        ev(&mut v, &g, Ev::Click(K_CPY), 3.0);
        assert!(v.describe().contains("toast=\"Copied to clipboard\""), "{}", v.describe());
        ev(&mut v, &g, Ev::Click(K_CLR), 4.0);
        assert_eq!(v.state(), State::Idle);
        assert!(v.describe().contains("words=0"));
    }

    #[test]
    fn fixing_a_word_after_stopping() {
        let (mut v, g) = page();
        v.dict.as_mut().unwrap().set_text("one two");
        assert_eq!(v.state(), State::Done);
        ev(&mut v, &g, Ev::Press(K_TXT, 1000.0, 20.0, (0.0, 0.0, 544.0, 316.0)), 1.0);
        assert_eq!(v.caret, Some(7)); // a click right of the text: the end
        ev(&mut v, &g, Ev::Key(K_TXT, VK_BACK), 2.0);
        ev(&mut v, &g, Ev::Key(K_TXT, VK_BACK), 2.0);
        ev(&mut v, &g, Ev::Key(K_TXT, VK_BACK), 2.0);
        for c in "three".chars() {
            ev(&mut v, &g, Ev::Char(K_TXT, c), 3.0);
        }
        assert!(v.describe().ends_with("text=\"one three\""), "{}", v.describe());
        ev(&mut v, &g, Ev::Blur(K_TXT), 4.0);
        assert_eq!(v.caret, None);
    }

    #[test]
    fn the_language_list_more_languages_asks_for_windows_settings_never_opens_it_in_a_test() {
        let (mut v, g) = page();
        ev(&mut v, &g, Ev::Press(K_LANG, 500.0, 70.0, (440.0, 60.0, 132.0, 24.0)), 1.0);
        ev(&mut v, &g, Ev::Click(K_LANG), 1.0);
        assert!(v.describe().contains("menu=true"));
        let mut st = CssState::default();
        let mut cx = Cx::new(2.0, false, &g, &mut st);
        assert!(v.popup(&mut cx).is_some(), "the list: English (US), the line, More languages…");
        // a click on the button while open: the frame dismisses it, the click must not reopen it
        v.popup_dismiss();
        ev(&mut v, &g, Ev::Press(K_LANG, 500.0, 70.0, (440.0, 60.0, 132.0, 24.0)), 3.0);
        ev(&mut v, &g, Ev::Click(K_LANG), 3.0);
        assert!(v.describe().contains("menu=false"));
        ev(&mut v, &g, Ev::Click(K_LANG), 4.0);
        // row 1 is the line (no click), row 2 "More languages…"
        ev(&mut v, &g, Ev::Click(idx(K_MENU, 1)), 5.0);
        assert!(v.describe().contains("menu=true"), "the line is not a row to pick");
        ev(&mut v, &g, Ev::Click(idx(K_MENU, 2)), 5.0);
        assert!(v.describe().contains("reqs=[SpeechSettings]"), "{}", v.describe());
        assert!(v.describe().contains("menu=false"));
    }

    #[test]
    fn closing_the_tab_drops_the_engine_and_the_words() {
        let (mut v, g) = page();
        ev(&mut v, &g, Ev::Click(K_MIC), 1.0);
        v.close();
        assert!(v.dict.is_none() && v.toast.is_none());
    }

    /// The chosen language is the settings store's (the owner Oct 8: settings survive a menu close and a restart), and the key
    /// field's action is registered at app start (the keys manager captures it).
    #[test]
    fn the_chosen_language_survives_and_the_key_is_the_keys_managers() {
        crate::services::init(windows::Win32::Foundation::HWND::default(), true);
        let names: Vec<String> = crate::services::with(|s| s.action_list().into_iter().map(|a| a.id).collect()).unwrap();
        assert!(names.iter().any(|n| n == ACTION), "{names:?}");
        let g = Gfx::new(1.0);
        let pev = |v: &mut Voice, e: Ev, now: f64| {
            let mut st = CssState::default();
            let mut cx = Cx::new(now, false, &g, &mut st).for_page("vtt");
            v.event(&e, &mut cx);
        };
        let mut v = Voice::default();
        v.open(&Env { test: true, frozen: true, ..Env::default() }, 0.0);
        pev(&mut v, Ev::Press(K_LANG, 500.0, 70.0, (440.0, 60.0, 132.0, 24.0)), 1.0);
        pev(&mut v, Ev::Click(K_LANG), 1.0);
        pev(&mut v, Ev::Click(idx(K_MENU, 2)), 2.0);
        assert_eq!(v.lang, "de-DE");
        drop(v);
        let mut v = Voice::default();
        v.open(&Env { test: true, frozen: true, ..Env::default() }, 3.0);
        let mut st = CssState::default();
        let mut cx = Cx::new(3.0, false, &g, &mut st).for_page("vtt");
        let _ = v.build(&mut cx);
        assert_eq!(v.lang, "de-DE", "{}", v.describe());
        // the key opens the menu here and starts the dictation
        v.jump("listen");
        let _ = v.build(&mut cx);
        assert_eq!(v.state(), State::Listening);
        // pressed again while it listens: it stops
        v.jump("listen");
        let _ = v.build(&mut cx);
        assert_ne!(v.state(), State::Listening);
        crate::services::shutdown();
    }

    /// A test page on a fake engine that is blocked by `e` (the switch can be flipped through the returned handle).
    fn blocked_page(e: VoiceError) -> (Voice, Gfx, std::sync::Arc<std::sync::Mutex<Option<VoiceError>>>) {
        let (mut v, g) = page();
        let eng = bu_voice::fake::FakeEngine::drawing().with_blocker(Some(e));
        let h = eng.blocked.clone();
        let (clip, _) = bu_voice::fake::FakeClipboard::new();
        v.dict = Some(Dictation::new(Box::new(eng), Box::new(clip)));
        (v, g, h)
    }

    /// The link under the blocked line, laid out like the page (None = no link).
    fn fix_link(v: &mut Voice, g: &Gfx) -> Option<(f32, f32, f32, f32)> {
        let mut st = CssState::default();
        let mut cx = Cx::new(0.0, false, g, &mut st);
        let kids = v.build(&mut cx);
        let root = El::block().w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids);
        crate::ui::lay::Laid::new(g, root, 600.0, None).rect_of(K_FIX)
    }

    /// Order 043: "Online speech recognition" off in Windows - the mic does not listen, the words box says so in one line
    /// with the link to Settings › Privacy & security › Speech (never opened in a test, the setting never changed); once the
    /// user turned it on, the next mic press listens.
    #[test]
    fn online_speech_off_shows_one_line_and_the_privacy_speech_link() {
        let (mut v, g, sw) = blocked_page(VoiceError::OnlineOff);
        ev(&mut v, &g, Ev::Click(K_MIC), 1.0);
        assert_eq!(v.state(), State::Idle);
        assert!(v.describe().contains("blocked=Some(OnlineOff)"), "{}", v.describe());
        assert_eq!(blocked_line(&VoiceError::OnlineOff).0, "Online speech recognition is off in Windows");
        let r = fix_link(&mut v, &g).expect("the link under the line");
        assert!(r.2 > 0.0 && r.3 == 16.0, "{r:?}");
        ev(&mut v, &g, Ev::Click(K_FIX), 2.0);
        assert!(v.describe().contains("reqs=[PrivacySpeech]"), "{}", v.describe());
        *sw.lock().unwrap() = None;
        ev(&mut v, &g, Ev::Click(K_MIC), 3.0);
        assert_eq!(v.state(), State::Listening);
        assert!(v.describe().contains("blocked=None"), "{}", v.describe());
        assert_eq!(fix_link(&mut v, &g), None);
    }

    /// The microphone blocked for apps: its own line + the link to Settings › Privacy & security › Microphone; no microphone:
    /// the line alone (nothing to open).
    #[test]
    fn mic_blocked_and_no_mic_have_their_own_lines() {
        let (mut v, g, _) = blocked_page(VoiceError::MicDenied);
        ev(&mut v, &g, Ev::Click(K_MIC), 1.0);
        assert_eq!(v.state(), State::Idle);
        assert!(fix_link(&mut v, &g).is_some());
        ev(&mut v, &g, Ev::Click(K_FIX), 2.0);
        assert!(v.describe().contains("reqs=[PrivacyMic]"), "{}", v.describe());
        let (mut v, g, _) = blocked_page(VoiceError::NoMic);
        ev(&mut v, &g, Ev::Click(K_MIC), 1.0);
        assert!(v.describe().contains("blocked=Some(NoMic)"), "{}", v.describe());
        assert_eq!(fix_link(&mut v, &g), None);
        ev(&mut v, &g, Ev::Click(K_FIX), 2.0);
        assert!(v.describe().contains("reqs=[]"), "{}", v.describe());
    }

    /// The service refusing after the start (0x80045509 from Windows, the mic taken away): listening ends, the toast and the
    /// words box say why; no internet: a toast only (nothing to change in Windows).
    #[test]
    fn a_refusal_while_listening_becomes_the_blocked_line() {
        for (e, toast, blocked) in [
            (VoiceError::OnlineOff, "Online speech recognition is off in Windows", true),
            (VoiceError::Offline, "No internet connection \u{b7} Voice to text needs it", false),
        ] {
            let (mut v, g) = page();
            let eng = bu_voice::fake::FakeEngine::scripted(vec![bu_voice::Heard::Failed(e.clone())]);
            let (clip, _) = bu_voice::fake::FakeClipboard::new();
            v.dict = Some(Dictation::new(Box::new(eng), Box::new(clip)));
            ev(&mut v, &g, Ev::Click(K_MIC), 1.0);
            run_until(&mut v, State::Idle);
            assert!(v.describe().contains(&format!("toast={toast:?}")), "{}", v.describe());
            assert_eq!(v.blocked.is_some(), blocked, "{}", v.describe());
        }
    }

    /// The blocked lines painted off-screen with the app's painter into the lane's scratch folder (looked at, Order 043).
    #[test]
    fn the_blocked_lines_are_painted() {
        let dir = std::env::temp_dir().join("BoylerUtilities-test").join("voice-test");
        std::fs::create_dir_all(&dir).unwrap();
        for (e, name) in [(VoiceError::OnlineOff, "vtt-online-off.png"), (VoiceError::MicDenied, "vtt-mic-blocked.png"), (VoiceError::NoMic, "vtt-no-mic.png")] {
            let (mut v, g, _) = blocked_page(e);
            v.blocked = v.dict.as_ref().unwrap().check().err();
            let icons = crate::icons::Icons::new();
            let mut st = CssState::default();
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
}

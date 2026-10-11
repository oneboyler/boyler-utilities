//! The ONE button window (Order 090, drawing keyboard-v8.html): what a keyboard key, a mouse button or a controller button
//! plays and what it runs. Shared by the Keyboard, Mouse and Controller tabs; each page keeps its own model and window, and
//! builds these parts into it:
//! * [`SoundEd`] - the Sound part: "Pack's sound" (a switch: off = silent for this button only) and "Your sound" on top
//!   (Press: none / your file · Release: same as press / none / your file · Pitch · Loudness; drop an .mp3 / .wav / .ogg on
//!   Press or Release; ▶ plays both layers). For several buttons at once ("N keys") Pitch and Loudness are each Same / Random
//!   with their own slider (a keyboard's special keys - Space, Enter ... - keep their built-in shape).
//! * [`MacroEd`] - a macro's steps right in the window: Repeat, Record, the steps (Key down · Key up · Press · Type · Wait ·
//!   Click · Open, + Button in a controller's macro), Add.
//! * the "your sounds" folder ([`take_file`], [`load_clips`]) and the ▶ ([`hear`]).
//!
//! Stateless builders + small state structs: a page calls `view` while it builds its window and `event` with every event,
//! and gets back what changed (it saves, tells the engine, plays).

use std::path::PathBuf;

use bu_keysound::layers::{self, file_id_ok, Layer, Layers, Mode, Release, Vary};
use bu_keysound::macros::{self, Macro, Repeat, Step};
use bu_keysound::{Dev, Hear, Pack, PlayOn};
use taffy::style::AlignItems;

use crate::anim::EASE;
use crate::gfx::{Align, Font};
use crate::ui::cx::{Cx, Ev};
use crate::ui::el::{idx, lh, sub, Cursor, El, Key, RADIUS_PILL};
use crate::ui::pieces::button::{self, Kind as BKind};
use crate::ui::pieces::keyfield::{self, Show};
use crate::ui::pieces::mitems::{self, It, Place, Row};
use crate::ui::pieces::nbox::{self, Filter};
use crate::ui::pieces::{dropdown, group, link, seg, slider, tinput, toggle};
use crate::ui::{cmix, ACC, CTL, FG, FG2, FG3};

// ================================================================== small shared pieces

/// The ▶ button of a window ("Play it · both sounds"): a small filled triangle, a soft highlight on hover (Order 098).
pub fn play_btn(cx: &mut Cx, k: Key, tip: &str) -> El {
    button::play_icon_btn(cx, k).title(tip)
}

/// A window's small bold heading with things on its right ("Sound ▶", "Your sound · on top … Remove your sound").
fn heading(title: &str, small: Option<&str>, right: Vec<El>) -> El {
    let mut r = El::row().center().gap(8.0).margin(14.0, 2.0, 7.0, 2.0).min_h(28.0).child(El::text(title, Font::new(13.0, 600), FG(), lh(13.0, 1.35)).none());
    if let Some(s) = small {
        r = r.child(El::text(s, Font::new(11.5, 400), FG3(), lh(11.5, 1.35)).none());
    }
    r.child(El::block().flex1()).children(right)
}

/// A row that is greyed and takes no clicks.
fn dim(e: El, on: bool) -> El {
    if on {
        e
    } else {
        e.opacity(0.38).no_hit()
    }
}

/// "+1.5" / "-4.0" / "0.0" (semitones).
pub fn fmt_st(v: f32) -> String {
    if v.abs() < 0.05 {
        "0.0".into()
    } else if v > 0.0 {
        format!("+{v:.1}")
    } else {
        format!("\u{2212}{:.1}", -v)
    }
}

/// The slider value (0..1) of `v` in lo..hi.
fn frac(v: f32, lo: f32, hi: f32) -> f32 {
    ((v - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// `t` (0..1) of a slider -> a value in lo..hi on `step`.
fn stepped(t: f32, lo: f32, hi: f32, step: f32) -> f32 {
    let v = lo + t.clamp(0.0, 1.0) * (hi - lo);
    ((v / step).round() * step).clamp(lo, hi)
}

/// The text of every value when they are all the same, else None ("Mixed").
fn same<T: PartialEq + Clone>(v: impl IntoIterator<Item = T>) -> Option<T> {
    let mut it = v.into_iter();
    let first = it.next()?;
    it.all(|x| x == first).then_some(first)
}

// ================================================================== "your sounds": the folder, taking a file, loading

/// Where the files of "your sound" are kept: `<settings>\keysounds\yours` (a copy: moving the original breaks nothing).
pub fn yours_dir() -> Option<PathBuf> {
    crate::services::with(|s| super::keyboard::glue::packs_dir(s.store.folder()).join("yours"))
}

/// The file types "your sound" takes.
pub const SOUND_FILTER: (&str, &str) = ("Sounds (.mp3, .wav, .ogg)", "*.mp3;*.wav;*.ogg");

/// Takes a sound file the user picked or dropped: checks its type, copies it into [`yours_dir`] (the same file again = the
/// same copy), decodes it safely (a broken file says why, never crashes) and hands it to the engine. Ok = its id.
pub fn take_file(path: &str) -> Result<String, String> {
    let src = std::path::Path::new(path);
    let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = src.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if !["mp3", "wav", "ogg"].contains(&ext.as_str()) {
        return Err(format!("{name} · use an .mp3, .wav or .ogg file"));
    }
    let dir = yours_dir().ok_or("the settings folder isn't available")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("the sounds folder: {e}"))?;
    let len = std::fs::metadata(src).map_err(|_| format!("{name} can't be read"))?.len();
    // a plain, short file name: the original's, cleaned
    // (at most 90 BYTES: a long Cyrillic / CJK name stays a usable id - Order 090 review)
    let mut stem = String::new();
    for c in src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default().chars().map(|c| if c.is_alphanumeric() || " -_.()".contains(c) { c } else { '_' }) {
        if stem.len() + c.len_utf8() > 90 {
            break;
        }
        stem.push(c);
    }
    let stem = if stem.trim().is_empty() { "sound".to_string() } else { stem.trim().to_string() };
    let mut id = format!("{stem}.{ext}");
    let mut n = 2;
    // `created`: this call made the copy (only then may a failure remove it - an existing copy other buttons use stays)
    let created = loop {
        if !file_id_ok(&id) {
            return Err(format!("{name} \u{b7} rename the file (its name can't be used)"));
        }
        let p = dir.join(&id);
        match std::fs::metadata(&p) {
            Ok(m) if m.len() == len && std::fs::read(&p).ok() == std::fs::read(src).ok() => break false,
            Ok(_) => {
                id = format!("{stem} ({n}).{ext}");
                n += 1;
            }
            Err(_) => {
                std::fs::copy(src, &p).map_err(|e| format!("{name}: {e}"))?;
                break true;
            }
        }
        if n > 999 {
            return Err("too many files of that name".into());
        }
    };
    let dest = dir.join(&id);
    match bu_keysound::safe::read_sound(&dest, true) {
        Ok(c) => {
            super::keyboard::glue::engine().set_clip(&id, Some(c));
            Ok(id)
        }
        Err(e) => {
            if created {
                let _ = std::fs::remove_file(&dest);
            }
            Err(format!("{name} \u{b7} {e}"))
        }
    }
}

/// Loads every file in `ids` the engine doesn't hold yet (from their caches: fast; a missing / broken file stays silent).
pub fn load_clips(ids: &[String]) {
    let Some(dir) = yours_dir() else { return };
    let e = super::keyboard::glue::engine();
    let have = e.clip_ids();
    for id in ids {
        if have.contains(id) || !file_id_ok(id) {
            continue;
        }
        if let Ok(c) = bu_keysound::safe::read_sound(&dir.join(id), true) {
            e.set_clip(id, Some(c));
        }
    }
}

/// What a page's ▶ plays: for each button (at most 24, one after another) its down and its up as "Play on" says - the
/// pack's sound unless it is off for that button, and the button's own sound on top. `class` names what kind of mouse /
/// controller button a slot is. Works with the sounds switched off; nothing plays in a test copy.
pub struct HearOf<'a> {
    pub dev: Dev,
    pub pack: Option<&'a Pack>,
    pub volume: u8,
    pub play_on: PlayOn,
    pub mouse: &'a dyn Fn(u16) -> Option<bu_keysound::synth::MouseButtonClass>,
    pub pad: &'a dyn Fn(u16) -> Option<bu_rawin::PadSoundClass>,
}

pub fn hear(h: &HearOf, layers: &Layers, ids: &[u16]) {
    if crate::testmode::on() {
        return;
    }
    let e = super::keyboard::glue::engine();
    let g = bu_keysound::gain(h.volume);
    let gap = if ids.len() > 1 { 110 } else { 0 };
    let mut list: Vec<Hear> = Vec::new();
    for (i, &slot) in ids.iter().take(24).enumerate() {
        let l = layers.of(slot);
        for (down, at) in [(true, 0u32), (false, 90u32)] {
            if !h.play_on.plays(down) {
                continue;
            }
            let at_ms = i as u32 * gap + if h.play_on == PlayOn::Release { 0 } else { at };
            if l.pack_on {
                if let Some(p) = h.pack {
                    if let Some((s, rate, loud, st)) = e.pack_sound(h.dev, p, slot, down, (h.mouse)(slot), (h.pad)(slot)) {
                        list.push(Hear { sound: s, rate, gain: g * loud * if down { 1.0 } else { 0.9 }, pitch: st, at_ms });
                    }
                }
            }
            if let Some(f) = l.own_file(down) {
                if let Some(c) = e.clip(f) {
                    let up = if !down && l.release == Release::SameAsPress { layers::RELEASE_UP } else { 0.0 };
                    list.push(Hear { sound: c.mono.clone(), rate: c.rate, gain: g * f32::from(l.loud) / 100.0, pitch: l.pitch + up, at_ms });
                }
            }
        }
    }
    e.hear(list);
}

// ================================================================== the variation rows (N keys, Make a pack from one sound)

/// The width of the choice + slider column of a variation row inside a button window (pack-noise-v1: 292 px).
pub const SND_VARY_W: f32 = 292.0;

/// The amount written under a variation row's name ("+1.5", "100 %", "± 10 %"): Same = the one value of every normal
/// key, Random = how much it may differ (Order 098).
pub fn vary_label(pitch: bool, row: &layers::Row) -> String {
    let v = row.value();
    match (pitch, row.mode) {
        (true, Mode::Same) => fmt_st(v),
        (false, Mode::Same) => format!("{v:.0} %"),
        (true, Mode::Random) => format!("\u{b1} {v:.1}"),
        (false, Mode::Random) => format!("\u{b1} {v:.0} %"),
    }
}

/// One Pitch / Loudness row (pack-noise-v1 `.vr`): the name with the written amount under it on the left; on the right the
/// choice (Same / Random) with its one slider right under it, `col_w` wide (Same: the value of every normal key; Random: how
/// much they differ).
/// Choice = `sub(base, "pm" | "lm")`, slider = `sub(base, "pv" | "lv")`.
pub fn vary_row(cx: &mut Cx, base: Key, pitch: bool, row: &layers::Row, col_w: f32, first: bool) -> El {
    let labels = ["Same", "Random"];
    let at = Mode::ALL.iter().position(|m| *m == row.mode).unwrap_or(0);
    let sg = seg::seg_w(cx, sub(base, if pitch { "pm" } else { "lm" }), &labels, at, col_w);
    let (lo, hi, _) = layers::range(pitch, row.mode);
    let sl = slider::slider(cx, sub(base, if pitch { "pv" } else { "lv" }), frac(row.value(), lo, hi), col_w, 16.0, slider::default());
    let name = El::col()
        .flex1()
        .min_w(0.0)
        .pad(4.0, 0.0, 0.0, 0.0)
        .child(El::text(if pitch { "Pitch" } else { "Loudness" }, Font::new(13.0, 500), FG(), lh(13.0, 1.35)).ellipsis())
        .child(El::text(vary_label(pitch, row), Font::new(11.0, 400).tnum(), FG2(), lh(11.0, 1.35)).ellipsis().margin(3.0, 0.0, 0.0, 0.0));
    let right = El::col().none().w(col_w).gap(9.0).items(AlignItems::STRETCH).child(sg).child(sl);
    let mut r = El::row().items(AlignItems::FLEX_START).gap(12.0).pad(9.0, 12.0, 8.0, 12.0);
    if !first {
        r = r.child(El::block().abs(12.0, 0.0, 0.0, f32::NAN).h(1.0).bg(crate::ui::HAIR()).no_hit());
    }
    r.child(name).child(right)
}

/// Pitch + Loudness rows for the pack maker (keys under `base`).
pub fn vary_rows(cx: &mut Cx, base: Key, vary: &Vary, col_w: f32) -> Vec<El> {
    vec![vary_row(cx, base, true, &vary.pitch, col_w, true), vary_row(cx, base, false, &vary.loud, col_w, false)]
}

/// What a variation row event did: None = not one of its; Some(false) = changed (keep dragging); Some(true) = done (a mode
/// was picked or the slider let go: save / play now).
pub fn vary_event(ev: &Ev, base: Key, vary: &mut Vary) -> Option<bool> {
    let (kp, kl) = (sub(base, "pv"), sub(base, "lv"));
    let set = |vary: &mut Vary, k: Key, t: f32| {
        let pitch = k == kp;
        let row = if pitch { &mut vary.pitch } else { &mut vary.loud };
        let (lo, hi, _) = layers::range(pitch, row.mode);
        row.set_value(pitch, lo + t.clamp(0.0, 1.0) * (hi - lo));
    };
    match ev {
        Ev::Press(k, x, _, r) | Ev::Drag(k, x, _, r) if *k == kp || *k == kl => {
            set(vary, *k, slider::value_at(*r, *x));
            Some(false)
        }
        Ev::Release(k) if *k == kp || *k == kl => Some(true),
        Ev::Click(k) => {
            for (pitch, part) in [(true, "pm"), (false, "lm")] {
                for (i, m) in Mode::ALL.iter().enumerate() {
                    if *k == idx(sub(base, part), i) {
                        if pitch {
                            vary.pitch.mode = *m;
                        } else {
                            vary.loud.mode = *m;
                        }
                        return Some(true);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

// ================================================================== the Sound part

/// Which list of the Sound part is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SPop {
    Press,
    Release,
}

/// The Sound part's state (kept by the page while its window is open).
pub struct SoundEd {
    pop: Option<(SPop, (f32, f32, f32, f32))>,
    press: Option<(Key, (f32, f32, f32, f32))>,
    /// Several buttons: how their pitch + loudness are set (kept while the window is open).
    pub vary: Vary,
    /// A line under the part (a file that couldn't be taken).
    pub msg: Option<String>,
}

impl Default for SoundEd {
    fn default() -> Self {
        SoundEd { pop: None, press: None, vary: Vary::for_keys(), msg: None }
    }
}

/// How the part speaks of what it changes.
pub struct Words {
    /// "this key" / "this button"
    pub one: &'static str,
    /// "these keys" / "these buttons"
    pub many: &'static str,
}

pub const KEY_WORDS: Words = Words { one: "this key", many: "these keys" };
pub const BUTTON_WORDS: Words = Words { one: "this button", many: "these buttons" };

/// What an event did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Out {
    /// The layers changed (the page tells the engine).
    pub changed: bool,
    /// Save now (a slider is saved when it is let go).
    pub save: bool,
    /// Play the buttons now.
    pub play: bool,
    /// The event was the part's own (the page does nothing more with it).
    pub used: bool,
}

impl SoundEd {
    pub fn reset(&mut self) {
        *self = SoundEd::default();
    }

    /// The Sound part for the buttons `ids` (in reading order; one = the single window).
    pub fn view(&self, cx: &mut Cx, base: Key, layers: &Layers, ids: &[u16], w: &Words, file_name: &dyn Fn(&str) -> String) -> El {
        let n = ids.len();
        let recs: Vec<Layer> = ids.iter().map(|s| layers.of(*s)).collect();
        let play = play_btn(cx, sub(base, "play"), if n > 1 { "Play them one after another" } else { "Play it · both sounds" });
        // the pack's sound
        let offs = same(recs.iter().map(|r| !r.pack_on));
        let mine = recs.iter().any(|r| r.press.is_some());
        let one = if n > 1 { w.many } else { w.one };
        let hint = match offs {
            None => "On for some, off for others".to_string(),
            Some(true) if mine => "Off · only your sound plays".to_string(),
            Some(true) => format!("Off · {one} {} silent", if n > 1 { "are" } else { "is" }),
            Some(false) => format!("Off = silent for {one} only"),
        };
        let pk = group::row(true, vec![group::lbl("Pack\u{2019}s sound", Some(&hint)), group::ctl(vec![toggle::toggle(cx, sub(base, "pack"), offs == Some(false), false)])]).min_h(50.0);
        // your sound
        let head = heading("Sound", None, vec![play]);
        let pv = same(recs.iter().map(|r| r.press.clone()));
        let press_t = match &pv {
            None => "Mixed".to_string(),
            Some(None) => "None".to_string(),
            Some(Some(f)) => file_name(f),
        };
        let rv = same(recs.iter().map(|r| r.release.clone()));
        let rel_t = match &rv {
            None => "Mixed".to_string(),
            Some(Release::SameAsPress) => "Same as press".to_string(),
            Some(Release::None) => "None".to_string(),
            Some(Release::File(f)) => file_name(f),
        };
        let press = dropdown::dropdown(cx, sub(base, "press"), &press_t, Some(190.0));
        let rel = dropdown::dropdown(cx, sub(base, "rel"), &rel_t, Some(190.0));
        let mut rows = vec![
            pk,
            group::row(false, vec![group::lbl("Press", None), group::ctl(vec![press])]).key(sub(base, "pressrow")),
            dim(group::row(false, vec![group::lbl("Release", None), group::ctl(vec![rel])]).key(sub(base, "relrow")), mine),
        ];
        if n == 1 {
            let r = &recs[0];
            let ps = slider::slider(cx, sub(base, "pitch"), frac(r.pitch, layers::PITCH_MIN, layers::PITCH_MAX), 150.0, 20.0, slider::default());
            let ls = slider::slider(cx, sub(base, "loud"), frac(f32::from(r.loud), 0.0, f32::from(layers::LOUD_MAX)), 150.0, 20.0, slider::default());
            rows.push(dim(group::row(false, vec![group::lbl("Pitch", None), group::ctl(vec![ps, slider::value_label(&fmt_st(r.pitch))])]), mine));
            rows.push(dim(group::row(false, vec![group::lbl("Loudness", None), group::ctl(vec![ls, slider::value_label(&format!("{} %", r.loud))])]), mine));
        } else {
            for pitch in [true, false] {
                let row = if pitch { &self.vary.pitch } else { &self.vary.loud };
                rows.push(dim(vary_row(cx, base, pitch, row, SND_VARY_W, false), mine));
            }
        }
        let mut kids = vec![head, group::grp(rows)];
        if let Some(m) = &self.msg {
            kids.push(keyfield::error_line(m));
        }
        El::col().items(AlignItems::STRETCH).children(kids)
    }

    /// The part's open list (the page puts it on top of everything).
    pub fn popup(&self, cx: &mut Cx, base: Key, layers: &Layers, ids: &[u16]) -> Option<El> {
        let (p, a) = self.pop?;
        let recs: Vec<Layer> = ids.iter().map(|s| layers.of(*s)).collect();
        let rows: Vec<Row> = match p {
            SPop::Press => {
                let pv = same(recs.iter().map(|r| r.press.is_some()));
                vec![Row::Item(It::tick("None", pv == Some(false))), Row::Item(It::tick("Your file\u{2026}", pv == Some(true)))]
            }
            SPop::Release => {
                let rv = same(recs.iter().map(|r| match r.release {
                    Release::SameAsPress => 0,
                    Release::None => 1,
                    Release::File(_) => 2,
                }));
                vec![
                    Row::Item(It::tick("Same as press", rv == Some(0))),
                    Row::Item(It::tick("None", rv == Some(1))),
                    Row::Item(It::tick("Your file\u{2026}", rv == Some(2))),
                ]
            }
        };
        Some(mitems::menu(cx, sub(base, "menu"), &rows, Place::Under(a.0, a.1, a.2, a.3), a.2.max(170.0)).z(40))
    }

    /// Changes `ids`' layers with `f` (the i-th of n gets its place in a rise).
    fn edit(layers: &mut Layers, ids: &[u16], mut f: impl FnMut(&mut Layer, u16, usize)) {
        for (i, &s) in ids.iter().enumerate() {
            let mut l = layers.of(s);
            f(&mut l, s, i);
            if l.press.is_none() {
                l.remove_own();
            }
            let _ = layers.set(s, l);
        }
    }

    /// Several buttons: every one that has a sound of yours gets its pitch + loudness from the variation rows.
    fn apply_vary(&self, layers: &mut Layers, ids: &[u16], board: bool) {
        let n = ids.len();
        if n < 2 {
            return;
        }
        let v = self.vary;
        Self::edit(layers, ids, |l, s, _| {
            if l.press.is_some() {
                let (p, ld) = v.at(s, board);
                l.pitch = p;
                l.loud = ld;
            }
        });
    }

    /// A file (picked or dropped) for the press (`release` false) or the release of every button in `ids`.
    fn put_file(&mut self, layers: &mut Layers, ids: &[u16], path: &str, release: bool, test: bool, board: bool) -> bool {
        let ext = std::path::Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if !["mp3", "wav", "ogg"].contains(&ext.as_str()) {
            let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            self.msg = Some(format!("{name} \u{b7} use an .mp3, .wav or .ogg file"));
            return false;
        }
        let id = if test {
            // a test copy keeps nothing: the name is taken as it is
            std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        } else {
            match take_file(path) {
                Ok(id) => id,
                Err(e) => {
                    self.msg = Some(e);
                    return false;
                }
            }
        };
        self.msg = None;
        let n = ids.len();
        let v = self.vary;
        Self::edit(layers, ids, |l, s, _| {
            if release {
                if l.press.is_some() {
                    l.release = Release::File(id.clone());
                }
            } else {
                let had = l.press.is_some();
                l.press = Some(id.clone());
                if !had && n > 1 {
                    let (p, ld) = v.at(s, board);
                    l.pitch = p;
                    l.loud = ld;
                }
            }
        });
        true
    }

    /// One event. `test` = a test copy (no file dialog, nothing copied). `board` = the buttons are keyboard keys, so the special
    /// ones (Space, Enter ...) get their built-in shape ([`layers::shape`]) when several are set at once.
    pub fn event(&mut self, ev: &Ev, cx: &mut Cx, base: Key, layers: &mut Layers, ids: &[u16], test: bool, board: bool) -> Out {
        let mut o = Out::default();
        if ids.is_empty() {
            return o;
        }
        let n = ids.len();
        let k_pitch = sub(base, "pitch");
        let k_loud = sub(base, "loud");
        let k_pv = sub(base, "pv");
        let k_lv = sub(base, "lv");
        let slide = |k: Key| k == k_pitch || k == k_loud || k == k_pv || k == k_lv;
        match ev {
            Ev::Press(k, x, _, r) => {
                self.press = Some((*k, *r));
                if slide(*k) {
                    o = self.slid(*k, slider::value_at(*r, *x), base, layers, ids, board);
                    o.used = true;
                }
            }
            Ev::Drag(k, x, _, r) if slide(*k) => {
                o = self.slid(*k, slider::value_at(*r, *x), base, layers, ids, board);
                o.used = true;
            }
            Ev::Release(k) if slide(*k) => {
                o.save = true;
                o.play = true;
                o.used = true;
            }
            Ev::Drop(k, paths) if [sub(base, "press"), sub(base, "pressrow"), sub(base, "rel"), sub(base, "relrow")].contains(k) => {
                o.used = true;
                if let Some(p) = paths.first() {
                    let release = *k == sub(base, "rel") || *k == sub(base, "relrow");
                    if self.put_file(layers, ids, p, release, test, board) {
                        o.changed = true;
                        o.save = true;
                        o.play = true;
                    }
                }
            }
            Ev::Click(k) => {
                let k = *k;
                // the open list owns the next click
                if let Some((p, _)) = self.pop {
                    self.pop = None;
                    o.used = true;
                    let pick = (0..3).find(|i| k == idx(sub(base, "menu"), *i));
                    match (p, pick) {
                        (SPop::Press, Some(0)) => {
                            Self::edit(layers, ids, |l, _, _| l.remove_own());
                            o.changed = true;
                            o.save = true;
                            o.play = true;
                        }
                        (SPop::Press, Some(1)) | (SPop::Release, Some(2)) => {
                            let release = p == SPop::Release;
                            if test {
                                return o;
                            }
                            if let Some(path) = cx.pick_file("Pick a sound", &[SOUND_FILTER]) {
                                if self.put_file(layers, ids, &path, release, test, board) {
                                    o.changed = true;
                                    o.save = true;
                                    o.play = true;
                                }
                            }
                        }
                        (SPop::Release, Some(i)) => {
                            Self::edit(layers, ids, |l, _, _| {
                                if l.press.is_some() {
                                    l.release = if i == 0 { Release::SameAsPress } else { Release::None };
                                }
                            });
                            o.changed = true;
                            o.save = true;
                            o.play = true;
                        }
                        _ => {}
                    }
                    return o;
                }
                let rect = self.press.filter(|(pk, _)| *pk == k).map(|(_, r)| r).unwrap_or((0.0, 0.0, 0.0, 0.0));
                if k == sub(base, "play") {
                    o.play = true;
                    o.used = true;
                } else if k == sub(base, "pack") {
                    let recs: Vec<Layer> = ids.iter().map(|s| layers.of(*s)).collect();
                    let all_on = recs.iter().all(|r| r.pack_on);
                    let mine = recs.iter().any(|r| r.press.is_some());
                    Self::edit(layers, ids, |l, _, _| l.pack_on = !all_on);
                    o.changed = true;
                    o.save = true;
                    o.play = !all_on || mine;
                    o.used = true;
                } else if k == sub(base, "press") {
                    self.pop = Some((SPop::Press, rect));
                    o.used = true;
                } else if k == sub(base, "rel") {
                    self.pop = Some((SPop::Release, rect));
                    o.used = true;
                } else if n > 1 {
                    for (pitch, part) in [(true, "pm"), (false, "lm")] {
                        for (i, m) in Mode::ALL.iter().enumerate() {
                            if k == idx(sub(base, part), i) {
                                if pitch {
                                    self.vary.pitch.mode = *m;
                                } else {
                                    self.vary.loud.mode = *m;
                                }
                                self.apply_vary(layers, ids, board);
                                o.changed = true;
                                o.save = true;
                                o.play = true;
                                o.used = true;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        o
    }

    fn slid(&mut self, k: Key, t: f32, base: Key, layers: &mut Layers, ids: &[u16], board: bool) -> Out {
        let mut o = Out { changed: true, ..Out::default() };
        if k == sub(base, "pitch") {
            let v = stepped(t, layers::PITCH_MIN, layers::PITCH_MAX, 0.5);
            Self::edit(layers, ids, |l, _, _| {
                if l.press.is_some() {
                    l.pitch = v;
                }
            });
        } else if k == sub(base, "loud") {
            let v = stepped(t, 0.0, f32::from(layers::LOUD_MAX), 5.0) as u16;
            Self::edit(layers, ids, |l, _, _| {
                if l.press.is_some() {
                    l.loud = v;
                }
            });
        } else {
            let pitch = k == sub(base, "pv");
            let row = if pitch { &mut self.vary.pitch } else { &mut self.vary.loud };
            let (lo, hi, _) = layers::range(pitch, row.mode);
            row.set_value(pitch, lo + t.clamp(0.0, 1.0) * (hi - lo));
            self.apply_vary(layers, ids, board);
        }
        o.used = true;
        o
    }

    /// Esc: the open list closes first. True = it was open.
    pub fn escape(&mut self) -> bool {
        self.pop.take().is_some()
    }

    pub fn menu_open(&self) -> bool {
        self.pop.is_some()
    }
}

// ================================================================== the macro editor

/// Which list of the macro editor is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MPop {
    /// "Runs [macro]": the macros + New macro + Ready-made + Rename.
    List,
    Repeat,
    /// The Click / Button step `usize`'s button.
    StepBtn(usize),
}

/// The editor's state (kept by the page while its window is open).
#[derive(Default)]
pub struct MacroEd {
    pop: Option<(MPop, (f32, f32, f32, f32))>,
    press: Option<(Key, (f32, f32, f32, f32))>,
    /// The step that waits for its key.
    listen: Option<usize>,
    pub rec: bool,
    rec_at: Option<f64>,
    /// The name is being typed in place of the list.
    renaming: bool,
    since: f64,
}

/// Where the macro lives: a keyboard key / mouse button (the app runs it) or a controller button (Steam plays it: no Type,
/// no Open, no Record; a Button step instead).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacroFor {
    Keys,
    /// A mouse button: as a key, but Windows tells the app only that it went down - so no "While the key is held".
    Mouse,
    Pad,
}

/// The Repeat choices for a macro of `f` (a controller's has none: Steam's own turbo / toggle are in its window).
fn repeats(f: MacroFor) -> Vec<Repeat> {
    match f {
        MacroFor::Keys => Repeat::ALL.to_vec(),
        MacroFor::Mouse => Repeat::ALL.into_iter().filter(|r| *r != Repeat::Held).collect(),
        MacroFor::Pad => Vec::new(),
    }
}

/// The step kinds the Add row offers, in order: (kind index, name, icon).
const KINDS: [(&str, &str); 8] = [("Key down", "kbd"), ("Key up", "kbd"), ("Press", "kbd"), ("Type", "note"), ("Wait", "gauge"), ("Click", "mouse"), ("Open", "arrow"), ("Button", "pad")];

fn kind_ok(i: usize, f: MacroFor) -> bool {
    match f {
        MacroFor::Keys | MacroFor::Mouse => i != 7,
        // Steam can't type a text or open anything; a key held down is let go when the button is (no Key up)
        MacroFor::Pad => !matches!(i, 1 | 3 | 6),
    }
}

fn kind_of(s: &Step) -> usize {
    match s {
        Step::Down(_) => 0,
        Step::Up(_) => 1,
        Step::Keys(_) => 2,
        Step::Type(_) => 3,
        Step::Wait(_) => 4,
        Step::Click(_) => 5,
        Step::Open(_) => 6,
        Step::Pad(_) => 7,
    }
}

/// The controller buttons a Button step can press: (number, name). Names as the Controller tab shows them (PlayStation's).
pub const PAD_BUTTONS: [(u8, &str); 17] = [
    (bu_rawin::padbtn::SOUTH, "Cross"),
    (bu_rawin::padbtn::EAST, "Circle"),
    (bu_rawin::padbtn::WEST, "Square"),
    (bu_rawin::padbtn::NORTH, "Triangle"),
    (bu_rawin::padbtn::LB, "L1"),
    (bu_rawin::padbtn::RB, "R1"),
    (bu_rawin::padbtn::LT, "L2"),
    (bu_rawin::padbtn::RT, "R2"),
    (bu_rawin::padbtn::BACK, "Create"),
    (bu_rawin::padbtn::START, "Options"),
    (bu_rawin::padbtn::LS, "L3"),
    (bu_rawin::padbtn::RS, "R3"),
    (bu_rawin::padbtn::TOUCHPAD, "Touchpad"),
    (bu_rawin::padbtn::DPAD_UP, "D-pad up"),
    (bu_rawin::padbtn::DPAD_RIGHT, "D-pad right"),
    (bu_rawin::padbtn::DPAD_DOWN, "D-pad down"),
    (bu_rawin::padbtn::DPAD_LEFT, "D-pad left"),
];

pub fn pad_button_name(b: u8) -> &'static str {
    PAD_BUTTONS.iter().find(|(n, _)| *n == b).map(|(_, s)| *s).unwrap_or("Button")
}

/// The names that never depend on the keyboard layout: the modifiers, then the keys manager's fixed names.
fn fixed_key_name(vk: u16) -> Option<String> {
    use crate::keys::{VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT};
    let n = match vk {
        VK_LWIN => "Win",
        VK_RWIN => "Right Win",
        VK_CONTROL | VK_LCONTROL => "Ctrl",
        VK_RCONTROL => "Right Ctrl",
        VK_SHIFT | VK_LSHIFT => "Shift",
        VK_RSHIFT => "Right Shift",
        VK_MENU | VK_LMENU => "Alt",
        VK_RMENU => "Right Alt",
        _ => return crate::keys::fixed_name(vk),
    };
    Some(n.to_string())
}

/// A key's name when no keyboard layout is at hand (a test, a picture): letters and digits as typed, else "Key 123".
fn plain_key_name(vk: u16) -> String {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => char::from(vk as u8).to_string(),
        0x60..=0x69 => format!("Num {}", vk - 0x60),
        _ => format!("Key {vk}"),
    }
}

/// The name of a virtual key: the fixed names (Win, Ctrl, Enter, F5 ...), else the keyboard layout's own, never a hex number.
pub fn key_name(vk: u16) -> String {
    fixed_key_name(vk).or_else(|| crate::services::try_with(|s| s.keys.key_name(vk))).unwrap_or_else(|| plain_key_name(vk))
}

/// "Ctrl + Shift + K".
pub fn combo_text(vks: &[u16]) -> String {
    vks.iter().map(|v| key_name(*v)).collect::<Vec<_>>().join(" + ")
}

/// A step in one short line ("Press Ctrl + C", "Wait 200 ms").
pub fn step_text(s: &Step) -> String {
    match s {
        Step::Keys(k) => format!("Press {}", combo_text(k)),
        Step::Type(t) => format!("Type \u{201c}{t}\u{201d}"),
        Step::Wait(ms) => format!("Wait {ms} ms"),
        Step::Open(o) => format!("Open {o}"),
        Step::Down(k) => format!("{} down", key_name(*k)),
        Step::Up(k) => format!("{} up", key_name(*k)),
        Step::Click(b) => macros::click_name(*b).to_string(),
        Step::Pad(b) => pad_button_name(*b).to_string(),
    }
}

/// What the editor did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MOut {
    /// The macros changed (save them).
    pub save: bool,
    /// This macro was picked for the button (its id; "" = none).
    pub chose: Option<String>,
    /// The event was the editor's own.
    pub used: bool,
    pub toast: Option<String>,
}

impl MacroEd {
    pub fn reset(&mut self) {
        *self = MacroEd::default();
    }

    /// The editor's rows for the button whose macro is `cur` (None = none chosen yet). `runs` = show the "Runs [macro]" row
    /// (a controller button picks its macro in Does instead); `also` = the other buttons carrying it.
    #[allow(clippy::too_many_arguments)]
    pub fn view(&mut self, cx: &mut Cx, base: Key, macros: &[Macro], cur: Option<&str>, f: MacroFor, runs: bool, also: &[String]) -> Vec<El> {
        let m = cur.and_then(|id| macros.iter().find(|m| m.id == id));
        let mut out = Vec::new();
        if runs {
            let ctl = if self.renaming && m.is_some() {
                tinput::kdin(cx, sub(base, "name"), &m.map(|m| m.name.clone()).unwrap_or_default(), "Macro name", 220.0)
            } else {
                dropdown::dropdown(cx, sub(base, "list"), &m.map(|m| m.name.clone()).unwrap_or_else(|| "Choose a macro".into()), Some(220.0))
            };
            out.push(group::grp(vec![group::row(true, vec![group::lbl("Runs", None), group::ctl(vec![ctl])])]));
        }
        if let Some(m) = m {
            out.push(self.editor(cx, base, m, f, also));
        }
        out.push(group::gf(match f {
            MacroFor::Keys | MacroFor::Mouse => "Works right away \u{b7} not while a game is in front",
            MacroFor::Pad => "Steam plays it in your games \u{b7} keys, waits, clicks and buttons",
        }));
        out
    }

    fn editor(&mut self, cx: &mut Cx, base: Key, m: &Macro, f: MacroFor, also: &[String]) -> El {
        // Repeat [..]  · Also on F11 · Record
        let mut meta = El::row().center().gap(8.0);
        if f != MacroFor::Pad {
            let rep = dropdown::dropdown(cx, sub(base, "rep"), &m.rep.label(), Some(176.0));
            meta = meta.child(El::text("Repeat", Font::new(12.0, 400), FG2(), lh(12.0, 1.35)).none()).child(rep);
        }
        meta = meta.child(El::block().flex1());
        if !also.is_empty() {
            meta = meta.child(El::text(&format!("Also on {}", also.join(", ")), Font::new(11.5, 400), FG3(), lh(11.5, 1.35)).ellipsis().max_w(140.0));
        }
        if f != MacroFor::Pad {
            meta = meta.child(button::cbtn(cx, sub(base, "rec"), if self.rec { "Stop recording" } else { "Record" }, if self.rec { BKind::Red } else { BKind::Ghost }, true, false, 0.0));
        }
        let mut rows: Vec<El> = Vec::new();
        for (i, s) in m.steps.iter().enumerate() {
            rows.push(self.step_row(cx, base, i, s, m.steps.len()));
        }
        if rows.is_empty() {
            let t = if self.rec { "Press the keys now\u{2026} each press is kept, with the waits between them" } else { "No steps yet. Add one below, or press Record and do it once." };
            let t = if f == MacroFor::Pad { "No steps yet. Add one below." } else { t };
            rows.push(El::text(t, Font::new(12.0, 400), FG3(), lh(12.0, 1.35)).wrapping().align(Align::Center).pad(14.0, 14.0, 14.0, 14.0));
        }
        let (bg, rim) = group::glass();
        let list = El::col().items(AlignItems::STRETCH).radius(10.0).bg(bg).inset(&rim).margin(8.0, 0.0, 0.0, 0.0).children(rows);
        let mut add = El::row().center().gap(6.0).wrap().margin(8.0, 0.0, 0.0, 0.0).child(El::text("Add", Font::new(11.5, 600), FG2(), lh(11.5, 1.35)).none());
        for (i, (name, icon)) in KINDS.iter().enumerate() {
            if !kind_ok(i, f) {
                continue;
            }
            let k = idx(sub(base, "add"), i);
            let hv = cx.hover_t(k, 120.0, EASE);
            add = add.child(
                El::row()
                    .none()
                    .center()
                    .gap(4.0)
                    .h(24.0)
                    .pad(0.0, 9.0, 0.0, 7.0)
                    .radius(RADIUS_PILL)
                    .bg(cmix(CTL(), ACC(), hv * 0.35))
                    .on_click(k)
                    .cursor(Cursor::Hand)
                    .child(El::icon(icon, 11.0, 1.5, FG2()).no_hit())
                    .child(El::text(*name, Font::new(11.5, 400), FG(), lh(11.5, 1.35)).no_hit()),
            );
        }
        El::col().items(AlignItems::STRETCH).margin(10.0, 0.0, 0.0, 0.0).child(meta).child(list).child(add)
    }

    fn step_row(&mut self, cx: &mut Cx, base: Key, i: usize, s: &Step, n: usize) -> El {
        let k = idx(sub(base, "step"), i);
        let (name, icon) = KINDS[kind_of(s)];
        let ic = El::block().size(22.0, 22.0).none().radius(7.0).bg(CTL()).place_center().child(El::icon(icon, 13.0, 1.5, FG2()).no_hit());
        let val: El = match s {
            Step::Keys(v) => {
                let t = combo_text(v);
                let show = if self.listen == Some(i) { Show::Listening(None) } else if v.is_empty() { Show::Empty } else { Show::Set(&t) };
                keyfield::keyfield(cx, sub(k, "key"), show, self.since, true).w(140.0)
            }
            Step::Down(vk) | Step::Up(vk) => {
                let t = if *vk == 0 { String::new() } else { key_name(*vk) };
                let show = if self.listen == Some(i) { Show::Listening(None) } else if *vk == 0 { Show::Empty } else { Show::Set(&t) };
                keyfield::keyfield(cx, sub(k, "key"), show, self.since, true).w(140.0)
            }
            Step::Type(t) => tinput::kdin(cx, sub(k, "val"), t, "text", 140.0),
            Step::Wait(ms) => El::row().center().gap(5.0).child(tinput::kdin(cx, sub(k, "val"), &ms.to_string(), "100", 100.0)).child(El::text("ms", Font::new(11.5, 400), FG3(), lh(11.5, 1.35)).none()),
            Step::Open(t) => tinput::kdin(cx, sub(k, "val"), t, "https://\u{2026}  or  C:\\\u{2026}\\app.exe", 140.0),
            Step::Click(b) => dropdown::dropdown(cx, sub(k, "btn"), macros::click_name(*b), Some(140.0)),
            Step::Pad(b) => dropdown::dropdown(cx, sub(k, "btn"), pad_button_name(*b), Some(140.0)),
        };
        let small = |e: El| e.size(22.0, 22.0);
        let up = small(button::icon_btn(cx, sub(k, "up"), "chevDw", 11.0, 1.5).rotate(180.0).title("Move up"));
        let up = if i == 0 { up.opacity(0.3).no_hit() } else { up };
        let dn = small(button::icon_btn(cx, sub(k, "dn"), "chevDw", 11.0, 1.5).title("Move down"));
        let dn = if i + 1 >= n { dn.opacity(0.3).no_hit() } else { dn };
        let cp = small(button::icon_btn(cx, sub(k, "cp"), "copy", 11.0, 1.5).title("Copy step"));
        let rm = small(button::icon_btn(cx, sub(k, "rm"), "x", 9.0, 1.5).title("Delete step"));
        El::row()
            .center()
            .gap(6.0)
            .min_h(38.0)
            .pad(5.0, 6.0, 5.0, 8.0)
            .child(ic)
            .child(El::text(name, Font::new(11.5, 400), FG2(), lh(11.5, 1.35)).w(58.0).none())
            .child(El::block().flex1().child(val))
            .child(up)
            .child(dn)
            .child(cp)
            .child(rm)
    }

    /// The editor's open list.
    pub fn popup(&self, cx: &mut Cx, base: Key, macros: &[Macro], cur: Option<&str>, f: MacroFor) -> Option<El> {
        let (p, a) = self.pop?;
        let m = cur.and_then(|id| macros.iter().find(|m| m.id == id));
        let owned: Vec<String> = match p {
            MPop::List => macros.iter().map(|m| m.name.clone()).collect(),
            MPop::Repeat => repeats(f).iter().map(|r| r.label()).collect(),
            MPop::StepBtn(_) => Vec::new(),
        };
        let rows: Vec<Row> = match p {
            MPop::List => {
                let mut r: Vec<Row> = owned.iter().zip(macros).map(|(n, mm)| Row::Item(It::tick(n.as_str(), Some(mm.id.as_str()) == cur))).collect();
                if !r.is_empty() {
                    r.push(Row::Sep);
                }
                r.push(Row::Item(It::tick("New macro", false)));
                if m.is_some() {
                    r.push(Row::Item(It::tick("Rename this macro\u{2026}", false)));
                }
                if f != MacroFor::Pad {
                    r.push(Row::Section("Ready-made"));
                    for (name, _) in macros::templates() {
                        r.push(Row::Item(It::tick(name, false)));
                    }
                }
                r
            }
            MPop::Repeat => {
                let list = repeats(f);
                owned.iter().zip(list).map(|(l, r)| Row::Item(It::tick(l.as_str(), m.is_some_and(|m| m.rep == r)))).collect()
            }
            MPop::StepBtn(i) => match m.and_then(|m| m.steps.get(i)) {
                Some(Step::Click(b)) => {
                    (0..=macros::CLICK_X2).map(|c| Row::Item(It::tick(macros::click_name(c), c == *b))).collect()
                }
                Some(Step::Pad(b)) => {
                    PAD_BUTTONS.iter().map(|(n, name)| Row::Item(It::tick(name, n == b))).collect()
                }
                _ => return None,
            },
        };
        let menu = if rows.len() > 14 { mitems::menu_scroll(cx, sub(base, "menu"), &rows, Place::Under(a.0, a.1, a.2, a.3), a.2.max(170.0)) } else { mitems::menu(cx, sub(base, "menu"), &rows, Place::Under(a.0, a.1, a.2, a.3), a.2.max(170.0)) };
        Some(menu.z(40))
    }

    pub fn menu_open(&self) -> bool {
        self.pop.is_some()
    }

    /// Esc: an open list, a listening step or the recorder stop first. True = something stopped.
    pub fn escape(&mut self) -> bool {
        let was = self.pop.is_some() || self.listen.is_some() || self.rec || self.renaming;
        self.pop = None;
        self.listen = None;
        self.rec = false;
        self.renaming = false;
        was
    }

    /// "Delete macro" (the page shows it next to its reset link while the button runs a macro).
    pub fn delete_link(cx: &mut Cx, base: Key) -> El {
        link::link(cx, sub(base, "del"), "Delete macro", 12.0)
    }

    /// One event; `macros` is the shared list (the Keyboard tab's), `cur` the macro of this button.
    pub fn event(&mut self, ev: &Ev, cx: &mut Cx, base: Key, macros: &mut Vec<Macro>, cur: Option<&str>, f: MacroFor) -> MOut {
        let mut o = MOut::default();
        let at = cur.and_then(|id| macros.iter().position(|m| m.id == id));
        match ev {
            Ev::Press(k, _, _, r) => self.press = Some((*k, *r)),
            Ev::Click(k) => {
                let k = *k;
                if let Some((p, _)) = self.pop {
                    self.pop = None;
                    o.used = true;
                    let pick = (0..64).find(|i| k == idx(sub(base, "menu"), *i));
                    let Some(pick) = pick else { return o };
                    match p {
                        MPop::List => {
                            let n = macros.len();
                            let sep = usize::from(n > 0);
                            if pick < n {
                                o.chose = Some(macros[pick].id.clone());
                                self.listen = None;
                                self.rec = false;
                            } else if pick == n + sep {
                                // New macro: made, picked, its name typed in place
                                if macros.len() >= macros::MAX_MACROS {
                                    o.toast = Some(format!("At most {} macros", macros::MAX_MACROS));
                                    return o;
                                }
                                let id = macros::new_id(macros);
                                macros.push(Macro::new(&id, &format!("Macro {}", macros.len() + 1)));
                                o.chose = Some(id);
                                o.save = true;
                                self.renaming = true;
                                cx.focus(Some(sub(base, "name")));
                            } else if at.is_some() && pick == n + sep + 1 {
                                self.renaming = true;
                                cx.focus(Some(sub(base, "name")));
                            } else if f != MacroFor::Pad {
                                // Ready-made (after its heading)
                                let first = n + sep + 1 + usize::from(at.is_some()) + 1;
                                if let Some((name, steps)) = pick.checked_sub(first).and_then(|t| macros::templates().into_iter().nth(t)) {
                                    if macros.len() >= macros::MAX_MACROS {
                                        o.toast = Some(format!("At most {} macros", macros::MAX_MACROS));
                                        return o;
                                    }
                                    let id = macros::new_id(macros);
                                    let mut m = Macro::new(&id, name);
                                    m.steps = steps;
                                    macros.push(m);
                                    o.chose = Some(id);
                                    o.save = true;
                                }
                            }
                        }
                        MPop::Repeat => {
                            if let (Some(a), Some(r)) = (at, repeats(f).get(pick)) {
                                macros[a].rep = *r;
                                o.save = true;
                            }
                        }
                        MPop::StepBtn(i) => {
                            if let Some(a) = at {
                                match macros[a].steps.get_mut(i) {
                                    Some(Step::Click(b)) if pick <= usize::from(macros::CLICK_X2) => *b = pick as u8,
                                    Some(Step::Pad(b)) => {
                                        if let Some((n, _)) = PAD_BUTTONS.get(pick) {
                                            *b = *n;
                                        }
                                    }
                                    _ => {}
                                }
                                o.save = true;
                            }
                        }
                    }
                    return o;
                }
                let rect = self.press.filter(|(pk, _)| *pk == k).map(|(_, r)| r).unwrap_or((0.0, 0.0, 0.0, 0.0));
                if k == sub(base, "list") {
                    self.pop = Some((MPop::List, rect));
                    o.used = true;
                    return o;
                }
                if k == sub(base, "name") {
                    o.used = true;
                    return o;
                }
                if self.renaming {
                    // a click anywhere else ends the renaming
                    self.renaming = false;
                }
                if k == sub(base, "del") {
                    if let Some(a) = at {
                        macros.remove(a);
                        o.chose = Some(String::new());
                        o.save = true;
                        o.toast = Some("Macro deleted".into());
                    }
                    self.escape();
                    o.used = true;
                    return o;
                }
                let Some(a) = at else { return o };
                if k == sub(base, "rep") {
                    self.pop = Some((MPop::Repeat, rect));
                    o.used = true;
                    return o;
                }
                if k == sub(base, "rec") {
                    self.rec = !self.rec;
                    self.rec_at = None;
                    self.listen = None;
                    if self.rec {
                        cx.focus(Some(sub(base, "rec")));
                    }
                    o.used = true;
                    return o;
                }
                for (i, _) in KINDS.iter().enumerate() {
                    if k == idx(sub(base, "add"), i) && kind_ok(i, f) {
                        let m = &mut macros[a];
                        if m.steps.len() >= macros::MAX_STEPS {
                            o.toast = Some(format!("At most {} steps", macros::MAX_STEPS));
                            return o;
                        }
                        let s = match i {
                            0 => Step::Down(0),
                            1 => Step::Up(0),
                            2 => Step::Keys(Vec::new()),
                            3 => Step::Type(String::new()),
                            4 => Step::Wait(100),
                            5 => Step::Click(macros::CLICK_LEFT),
                            6 => Step::Open(String::new()),
                            _ => Step::Pad(bu_rawin::padbtn::SOUTH),
                        };
                        let listens = i <= 2;
                        m.steps.push(s);
                        self.rec = false;
                        let last = m.steps.len() - 1;
                        if listens {
                            self.listen = Some(last);
                            cx.focus(Some(sub(idx(sub(base, "step"), last), "key")));
                        } else if matches!(i, 3 | 4 | 6) {
                            cx.focus(Some(sub(idx(sub(base, "step"), last), "val")));
                        }
                        o.save = true;
                        o.used = true;
                        return o;
                    }
                }
                let n = macros[a].steps.len();
                for i in 0..n {
                    let s = idx(sub(base, "step"), i);
                    let steps = &mut macros[a].steps;
                    if k == sub(s, "up") && i > 0 {
                        steps.swap(i, i - 1);
                    } else if k == sub(s, "dn") && i + 1 < n {
                        steps.swap(i, i + 1);
                    } else if k == sub(s, "cp") {
                        if n < macros::MAX_STEPS {
                            let c = steps[i].clone();
                            steps.insert(i + 1, c);
                        }
                    } else if k == sub(s, "rm") {
                        steps.remove(i);
                        self.listen = None;
                    } else if k == sub(s, "key") {
                        self.listen = Some(i);
                        cx.focus(Some(sub(s, "key")));
                        o.used = true;
                        return o;
                    } else if k == sub(s, "btn") {
                        self.pop = Some((MPop::StepBtn(i), rect));
                        o.used = true;
                        return o;
                    } else {
                        continue;
                    }
                    o.save = true;
                    o.used = true;
                    return o;
                }
            }
            Ev::Char(k, c) => {
                let Some(a) = at else { return o };
                if *k == sub(base, "name") {
                    nbox::type_char(&mut macros[a].name, &mut false, *c, macros::MAX_NAME, Filter::Any);
                    o.save = true;
                    o.used = true;
                    return o;
                }
                if let Some(i) = (0..macros[a].steps.len()).find(|i| *k == sub(idx(sub(base, "step"), *i), "val")) {
                    match macros[a].steps.get_mut(i) {
                        Some(Step::Type(t)) => nbox::type_char(t, &mut false, *c, macros::MAX_TEXT, Filter::Any),
                        Some(Step::Open(t)) => nbox::type_char(t, &mut false, *c, bu_keysound::binds::MAX_TARGET, Filter::Any),
                        Some(Step::Wait(ms)) if c.is_ascii_digit() => {
                            let mut t = ms.to_string();
                            if t == "0" {
                                t.clear();
                            }
                            t.push(*c);
                            *ms = t.parse::<u32>().unwrap_or(*ms).min(macros::MAX_WAIT_MS);
                        }
                        _ => {}
                    }
                    o.save = true;
                    o.used = true;
                }
            }
            Ev::Key(k, vk) => {
                let (k, vk) = (*k, *vk);
                if k == sub(base, "name") {
                    if let Some(a) = at {
                        if vk == 0x08 {
                            macros[a].name.pop();
                            o.save = true;
                        } else if vk == 0x0D || vk == 0x1B {
                            if macros[a].name.trim().is_empty() {
                                macros[a].name = "Macro".into();
                            }
                            self.renaming = false;
                            cx.used = true;
                        }
                    }
                    o.used = true;
                    return o;
                }
                let Some(a) = at else { return o };
                if k == sub(base, "rec") && self.rec {
                    o.used = true;
                    if vk == 0x1B {
                        self.rec = false;
                        cx.used = true;
                        return o;
                    }
                    cx.used = true;
                    if let Some(c) = super::keyboard::combo_of(vk) {
                        let now = cx.now;
                        let gap = self.rec_at.map(|t| now - t).unwrap_or(0.0);
                        self.rec_at = Some(now);
                        // the waits between presses, to 10 ms (the drawing's recorder)
                        let ms = ((gap / 10.0).round() * 10.0) as u32;
                        let m = &mut macros[a];
                        if ms >= 30 && !m.steps.is_empty() && m.steps.len() < macros::MAX_STEPS {
                            m.steps.push(Step::Wait(ms.min(macros::MAX_WAIT_MS)));
                        }
                        if m.steps.len() < macros::MAX_STEPS {
                            m.steps.push(Step::Keys(c));
                        }
                        o.save = true;
                    }
                    return o;
                }
                let n = macros[a].steps.len();
                for i in 0..n {
                    let s = idx(sub(base, "step"), i);
                    if k == sub(s, "key") && self.listen == Some(i) {
                        o.used = true;
                        if vk == 0x1B {
                            self.listen = None;
                            cx.used = true;
                            return o;
                        }
                        let step = &mut macros[a].steps[i];
                        match step {
                            Step::Keys(v) => {
                                if let Some(c) = super::keyboard::combo_of(vk) {
                                    *v = c;
                                    self.listen = None;
                                    o.save = true;
                                }
                            }
                            Step::Down(v) | Step::Up(v) => {
                                *v = vk;
                                self.listen = None;
                                o.save = true;
                            }
                            _ => {}
                        }
                        cx.used = true;
                        return o;
                    }
                    if k == sub(s, "val") {
                        o.used = true;
                        match macros[a].steps.get_mut(i) {
                            Some(Step::Type(t)) | Some(Step::Open(t)) if vk == 0x08 => {
                                t.pop();
                            }
                            Some(Step::Type(t)) if vk == 0x0D => t.push('\n'),
                            Some(Step::Wait(ms)) if vk == 0x08 => *ms /= 10,
                            _ => return o,
                        }
                        o.save = true;
                        return o;
                    }
                }
            }
            Ev::Blur(k) if *k == sub(base, "name") => {
                self.renaming = false;
            }
            _ => {}
        }
        o
    }

    /// The window opened at `now` (the key fields' listening motion starts there).
    pub fn opened(&mut self, now: f64) {
        self.since = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semitones_and_sliders_read_as_drawn() {
        assert_eq!(fmt_st(0.0), "0.0");
        assert_eq!(fmt_st(1.5), "+1.5");
        assert_eq!(fmt_st(-4.0), "\u{2212}4.0");
        assert_eq!(stepped(0.5, -12.0, 12.0, 0.5), 0.0);
        assert_eq!(stepped(1.0, 0.0, 200.0, 5.0), 200.0);
        assert_eq!(same([1, 1, 1]), Some(1));
        assert_eq!(same([1, 2]), None);
    }

    #[test]
    fn controller_macros_offer_steam_steps_only() {
        let names: Vec<&str> = (0..KINDS.len()).filter(|i| kind_ok(*i, MacroFor::Pad)).map(|i| KINDS[i].0).collect();
        assert_eq!(names, vec!["Key down", "Press", "Wait", "Click", "Button"]);
        let keys: Vec<&str> = (0..KINDS.len()).filter(|i| kind_ok(*i, MacroFor::Keys)).map(|i| KINDS[i].0).collect();
        assert_eq!(keys, vec!["Key down", "Key up", "Press", "Type", "Wait", "Click", "Open"]);
    }
}

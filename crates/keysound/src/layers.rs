//! What ONE key, mouse button or controller button plays (Order 090, drawing keyboard-v8): two LAYERS, never a replacement.
//! * the PACK's sound - on by default; the "Pack's sound" switch in the button's window silences it for that button only;
//! * "YOUR sound" on top - a file of the user's own for the press and (same as press / none / another file) the release,
//!   with its own pitch (semitones) and loudness (%). It is kept apart from the packs, so it stays on any pack.
//!
//! Also the maths of the variation rows ("N keys" window and "Make a pack from one sound"): Pitch and Loudness each Same /
//! Random with its own slider, the special keys of a keyboard (Space, Enter ...) always with a built-in shape ([`shape`]) -
//! and the pack made from one sound ([`Made`]). Pure: the model, its text form for the settings file, the checks; the
//! engine plays it.

use std::collections::BTreeMap;

/// The pitch range of a key's own sound (semitones, the slider's ends).
pub const PITCH_MIN: f32 = -12.0;
pub const PITCH_MAX: f32 = 12.0;
/// The loudness range (% of the volume slider's level).
pub const LOUD_MAX: u16 = 200;
/// The most buttons with a layer of their own per device (a keyboard has ~110 keys).
pub const MAX_LAYERS: usize = 160;
/// The longest file name of a "your sound" file kept.
pub const MAX_FILE_ID: usize = 120;

/// What a key's own sound plays when the key comes up.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Release {
    /// The press file again (default).
    #[default]
    SameAsPress,
    /// Nothing.
    None,
    /// Another file (its id in the "your sounds" folder).
    File(String),
}

/// One key's / button's two layers.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    /// The pack's sound plays for this button (the window's "Pack's sound" switch).
    pub pack_on: bool,
    /// Your sound: the press file (its id in the "your sounds" folder). None = no sound of your own (then release, pitch and
    /// loudness wait for a file, as the window greys them).
    pub press: Option<String>,
    pub release: Release,
    /// Semitones, [`PITCH_MIN`] ..= [`PITCH_MAX`].
    pub pitch: f32,
    /// %, 0 ..= [`LOUD_MAX`].
    pub loud: u16,
}

impl Default for Layer {
    fn default() -> Self {
        Layer { pack_on: true, press: None, release: Release::SameAsPress, pitch: 0.0, loud: 100 }
    }
}

impl Layer {
    /// Plays exactly as if nothing was set (the pack's sound, nothing of your own).
    pub fn is_plain(&self) -> bool {
        self.pack_on && self.press.is_none()
    }

    /// "Remove your sound": the pack switch stays as it is.
    pub fn remove_own(&mut self) {
        let pack_on = self.pack_on;
        *self = Layer { pack_on, ..Layer::default() };
    }

    /// Your sound's file for a press (`down`) or a release; None = nothing of yours plays.
    pub fn own_file(&self, down: bool) -> Option<&str> {
        let press = self.press.as_deref()?;
        if down {
            return Some(press);
        }
        match &self.release {
            Release::SameAsPress => Some(press),
            Release::None => None,
            Release::File(f) => Some(f.as_str()),
        }
    }

    /// Every file this layer uses (press, release).
    pub fn files(&self) -> Vec<&str> {
        let mut v = Vec::new();
        if let Some(p) = &self.press {
            v.push(p.as_str());
            if let Release::File(f) = &self.release {
                v.push(f.as_str());
            }
        }
        v
    }

    fn clamp(mut self) -> Layer {
        self.pitch = if self.pitch.is_finite() { self.pitch.clamp(PITCH_MIN, PITCH_MAX) } else { 0.0 };
        self.loud = self.loud.min(LOUD_MAX);
        self
    }
}

/// A file id is a plain file name inside the "your sounds" folder (no folders, no `|`).
pub fn file_id_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_FILE_ID && !s.contains(['/', '\\', ':', '|', '\0', '"']) && s != "." && s != ".." && !s.chars().any(char::is_control)
}

/// The layers of one device, by button: a key's scan code (as [`crate::remap::Code`]), a mouse button's number
/// (`bu_rawin::MOUSE_LEFT` ..) or a controller button's (`bu_rawin::pad::SOUTH` ..). A button with no entry plays plainly.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layers(BTreeMap<u16, Layer>);

impl Layers {
    pub fn new() -> Layers {
        Layers(BTreeMap::new())
    }

    pub fn get(&self, slot: u16) -> Option<&Layer> {
        self.0.get(&slot)
    }

    /// The layer of `slot` (the plain one when nothing is set).
    pub fn of(&self, slot: u16) -> Layer {
        self.0.get(&slot).cloned().unwrap_or_default()
    }

    /// Sets `slot`'s layer; a plain one is removed. Err = too many buttons with their own layer.
    pub fn set(&mut self, slot: u16, l: Layer) -> Result<(), String> {
        let l = l.clamp();
        if l.is_plain() {
            self.0.remove(&slot);
            return Ok(());
        }
        if !self.0.contains_key(&slot) && self.0.len() >= MAX_LAYERS {
            return Err(format!("at most {MAX_LAYERS} buttons can have a sound of their own"));
        }
        if l.files().iter().any(|f| !file_id_ok(f)) {
            return Err("not a usable sound file name".into());
        }
        self.0.insert(slot, l);
        Ok(())
    }

    pub fn remove(&mut self, slot: u16) {
        self.0.remove(&slot);
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (u16, &Layer)> {
        self.0.iter().map(|(k, v)| (*k, v))
    }

    /// Buttons with a sound of their own (a file), for the picture's dots.
    pub fn with_own(&self) -> impl Iterator<Item = u16> + '_ {
        self.0.iter().filter(|(_, l)| l.press.is_some() || !l.pack_on).map(|(k, _)| *k)
    }

    /// Every file in use (to load them, and to tidy the folder).
    pub fn files(&self) -> Vec<String> {
        let mut v: Vec<String> = self.0.values().flat_map(|l| l.files()).map(str::to_string).collect();
        v.sort();
        v.dedup();
        v
    }

    /// The text kept in the settings file: one line per button, `slot|pack(1/0)|press|release|pitch|loud`; release `=` is
    /// the press file, `-` none; an empty press = no file.
    pub fn to_lines(&self) -> Vec<String> {
        self.0
            .iter()
            .map(|(s, l)| {
                let rel = match &l.release {
                    Release::SameAsPress => "=".to_string(),
                    Release::None => "-".to_string(),
                    Release::File(f) => f.clone(),
                };
                format!("{s}|{}|{}|{rel}|{:.1}|{}", u8::from(l.pack_on), l.press.as_deref().unwrap_or(""), l.pitch, l.loud)
            })
            .collect()
    }

    /// Reads [`Layers::to_lines`]; a line that doesn't parse is left out.
    pub fn from_lines(lines: &[String]) -> Layers {
        let mut out = Layers::new();
        for line in lines {
            let p: Vec<&str> = line.split('|').collect();
            if p.len() != 6 {
                continue;
            }
            let (Ok(slot), Ok(pitch), Ok(loud)) = (p[0].parse::<u16>(), p[4].parse::<f32>(), p[5].parse::<u16>()) else { continue };
            if !pitch.is_finite() {
                continue;
            }
            let press = (!p[2].is_empty()).then(|| p[2].to_string());
            let release = match p[3] {
                "=" => Release::SameAsPress,
                "-" => Release::None,
                f => Release::File(f.to_string()),
            };
            let _ = out.set(slot, Layer { pack_on: p[1] != "0", press, release, pitch, loud });
        }
        out
    }
}

// ------------------------------------------------------------------ variation (N keys, Make a pack from one sound)

/// One row of the variation: Same / Random (Order 098 - "Rising across keys" is gone).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Same,
    Random,
}

impl Mode {
    pub const ALL: [Mode; 2] = [Mode::Same, Mode::Random];
    pub fn key(self) -> &'static str {
        match self {
            Mode::Same => "same",
            Mode::Random => "rnd",
        }
    }
    /// An unknown word (an old save's "rise") reads as Same.
    pub fn from_key(s: &str) -> Mode {
        Mode::ALL.into_iter().find(|m| m.key() == s).unwrap_or_default()
    }
}

/// One row (Pitch or Loudness): the mode + the slider of each mode (kept apart, so flipping back finds your value).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Row {
    pub mode: Mode,
    /// Same: the value every normal key gets (pitch -12..=12 st; loudness 0..=200 %).
    pub same: f32,
    /// Random: ± how much (pitch 0..=3 st; loudness 0..=30 %).
    pub rnd: f32,
}

/// The slider ends of each mode: (min, max, step).
pub fn range(pitch: bool, m: Mode) -> (f32, f32, f32) {
    match (pitch, m) {
        (true, Mode::Same) => (-12.0, 12.0, 0.5),
        (true, Mode::Random) => (0.0, 3.0, 0.1),
        (false, Mode::Same) => (0.0, 200.0, 5.0),
        (false, Mode::Random) => (0.0, 30.0, 1.0),
    }
}

impl Row {
    /// The slider value of the current mode.
    pub fn value(&self) -> f32 {
        match self.mode {
            Mode::Same => self.same,
            Mode::Random => self.rnd,
        }
    }

    pub fn set_value(&mut self, pitch: bool, v: f32) {
        let (lo, hi, step) = range(pitch, self.mode);
        let v = if v.is_finite() { ((v.clamp(lo, hi) / step).round() * step).clamp(lo, hi) } else { lo };
        match self.mode {
            Mode::Same => self.same = v,
            Mode::Random => self.rnd = v,
        }
    }
}

/// The built-in shape of the special keys of a keyboard (Order 098, the owner: "the space bar enter etc ctrl whatever have their
/// own already built ... presets that makes them sound more like this keys in terms of pitch"): (pitch in semitones,
/// loudness in % points) ADDED to the key's value, in Same AND in Random (no random on these keys). None = a normal key.
/// Space deepest and a touch louder; Enter / Backspace / right Shift (and the numpad's Enter, + and 0) lower; left Shift /
/// Caps Lock / Tab a bit lower; Ctrl / Alt / Win / Menu slightly lower. (The numbers are our design call.)
pub fn shape(code: u16) -> Option<(f32, i16)> {
    match code {
        0x39 => Some((-5.0, 12)),
        0x1C | 0x0E | 0x36 | 0xE01C | 0x4E | 0x52 => Some((-3.0, 8)),
        0x2A | 0x3A | 0x0F => Some((-2.0, 5)),
        0x1D | 0xE01D | 0x38 | 0xE038 | 0xE05B | 0xE05C | 0xE05D => Some((-1.5, 3)),
        _ => None,
    }
}

/// Pitch + Loudness rows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vary {
    pub pitch: Row,
    pub loud: Row,
}

impl Vary {
    /// "Make a pack from one sound" starts here: Same, pitch 0, loudness 100 % (the special keys still have their shape).
    pub fn for_pack() -> Vary {
        Vary::for_keys()
    }

    /// The "N keys" window starts here: the same pitch 0, the same 100 %.
    pub fn for_keys() -> Vary {
        Vary { pitch: Row { mode: Mode::Same, same: 0.0, rnd: 1.5 }, loud: Row { mode: Mode::Same, same: 100.0, rnd: 10.0 } }
    }

    /// (pitch st, loudness %) of the key `slot`. `board` = `slot` is a keyboard scan code, so the special keys get their
    /// [`shape`] on top of the Same value (in Random: on top of 0 st / 100 %, no random). A normal key: Same = the one value;
    /// Random = a difference fixed per key (the same key always gets the same one).
    pub fn at(&self, slot: u16, board: bool) -> (f32, u16) {
        let special = if board { shape(slot) } else { None };
        let (sp, sl) = special.map_or((0.0, 0.0), |(p, l)| (p, f32::from(l)));
        let one = |r: &Row, neutral: f32, loud: bool| match (r.mode, special.is_some()) {
            (Mode::Same, _) => r.same,
            (Mode::Random, true) => neutral,
            (Mode::Random, false) => neutral + seed(slot, loud) * r.rnd,
        };
        let p = ((one(&self.pitch, 0.0, false) + sp) * 10.0).round() / 10.0;
        let l = (one(&self.loud, 100.0, true) + sl).round().clamp(0.0, f32::from(LOUD_MAX));
        (p.clamp(PITCH_MIN, PITCH_MAX), l as u16)
    }

    /// Text for the settings / a pack file: `pm,psame,prnd;lm,lsame,lrnd`.
    pub fn to_text(&self) -> String {
        let r = |r: &Row| format!("{},{},{}", r.mode.key(), r.same, r.rnd);
        format!("{};{}", r(&self.pitch), r(&self.loud))
    }

    /// Reads [`Vary::to_text`]; an older save (four numbers with a Rising spread in the middle, or the mode "rise") loads as
    /// Same.
    pub fn from_text(s: &str) -> Option<Vary> {
        let row = |t: &str| -> Option<Row> {
            let p: Vec<&str> = t.split(',').collect();
            if p.len() != 3 && p.len() != 4 {
                return None;
            }
            Some(Row { mode: Mode::from_key(p[0]), same: p[1].parse().ok()?, rnd: p[p.len() - 1].parse().ok()? })
        };
        let (a, b) = s.split_once(';')?;
        Some(Vary { pitch: row(a)?, loud: row(b)? })
    }
}

/// A number in -1 ..= 1 fixed for (slot, which row): the drawing's kSeed.
pub fn seed(slot: u16, loud: bool) -> f32 {
    let mut x: u64 = if loud { 7 } else { 3 };
    for c in format!("{slot:X}").bytes() {
        x = (x * 31 + u64::from(c)) % 100_003;
    }
    x = (x * 9301 + 49_297) % 233_280;
    x as f32 / 233_280.0 * 2.0 - 1.0
}

// ------------------------------------------------------------------ a pack made from one sound

/// "Make a pack from one sound": one press file (and maybe a release file) spread over every key with a pitch and loudness
/// per key. Kept as `<packs>\made\<name>\pack.txt` + the files; it is one of "Your packs".
#[derive(Debug, Clone, PartialEq)]
pub struct Made {
    pub name: String,
    /// The press file / the optional release file (their names inside the pack's folder).
    pub press: String,
    pub release: Option<String>,
    pub vary: Vary,
    /// (scan code, pitch st, loudness %) of every key.
    pub keys: Vec<(u16, f32, u16)>,
}

/// With no release file the press plays again on the release, this much higher (semitones).
pub const RELEASE_UP: f32 = 1.0;

impl Made {
    /// Gives every key of `codes` (the full keyboard) its pitch + loudness from `vary` (the special keys with their shape).
    pub fn spread(vary: &Vary, codes: &[u16]) -> Vec<(u16, f32, u16)> {
        codes
            .iter()
            .map(|&code| {
                let (p, l) = vary.at(code, true);
                (code, p, l)
            })
            .collect()
    }

    /// The pack file's text.
    pub fn to_text(&self) -> String {
        let mut s = format!("name={}\npress={}\nrelease={}\nvary={}\n", self.name, self.press, self.release.as_deref().unwrap_or(""), self.vary.to_text());
        for (c, p, l) in &self.keys {
            s.push_str(&format!("key={c:X},{p:.1},{l}\n"));
        }
        s
    }

    pub fn from_text(t: &str) -> Result<Made, String> {
        let mut m = Made { name: String::new(), press: String::new(), release: None, vary: Vary::for_pack(), keys: Vec::new() };
        for line in t.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            match k {
                "name" => m.name = v.chars().take(60).collect(),
                "press" => m.press = v.to_string(),
                "release" => m.release = (!v.is_empty()).then(|| v.to_string()),
                "vary" => m.vary = Vary::from_text(v).unwrap_or_else(Vary::for_pack),
                "key" => {
                    let p: Vec<&str> = v.split(',').collect();
                    if p.len() == 3 && m.keys.len() < 512 {
                        if let (Ok(c), Ok(st), Ok(l)) = (u16::from_str_radix(p[0], 16), p[1].parse::<f32>(), p[2].parse::<u16>()) {
                            if st.is_finite() {
                                m.keys.push((c, st.clamp(PITCH_MIN, PITCH_MAX), l.min(LOUD_MAX)));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if !file_id_ok(&m.press) || m.release.as_deref().is_some_and(|r| !file_id_ok(r)) {
            return Err("the pack names no usable sound file".into());
        }
        if m.name.trim().is_empty() {
            return Err("the pack has no name".into());
        }
        Ok(m)
    }
}

/// Semitones -> playback speed (1.0 = as recorded).
pub fn speed(st: f32) -> f32 {
    2f32.powf(st / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn own(press: &str) -> Layer {
        Layer { press: Some(press.into()), ..Layer::default() }
    }

    #[test]
    fn a_plain_layer_is_not_kept_and_the_pack_switch_alone_is() {
        let mut l = Layers::new();
        l.set(0x1E, Layer::default()).unwrap();
        assert!(l.is_empty(), "nothing set = nothing kept");
        l.set(0x1E, Layer { pack_on: false, ..Layer::default() }).unwrap();
        assert_eq!(l.len(), 1, "a key whose pack sound is off is kept (it is silent)");
        assert_eq!(l.with_own().collect::<Vec<_>>(), vec![0x1E]);
        l.set(0x1E, Layer::default()).unwrap();
        assert!(l.is_empty());
    }

    #[test]
    fn your_sound_plays_press_and_the_release_you_chose() {
        let mut a = own("boing.wav");
        assert_eq!(a.own_file(true), Some("boing.wav"));
        assert_eq!(a.own_file(false), Some("boing.wav"), "same as press by default");
        a.release = Release::None;
        assert_eq!(a.own_file(false), None);
        a.release = Release::File("pop.mp3".into());
        assert_eq!(a.own_file(false), Some("pop.mp3"));
        assert_eq!(a.files(), vec!["boing.wav", "pop.mp3"]);
        a.remove_own();
        assert_eq!(a.own_file(true), None);
        assert!(a.pack_on, "removing your sound keeps the pack switch");
    }

    #[test]
    fn layers_round_trip_through_the_settings_text_and_are_clamped() {
        let mut l = Layers::new();
        l.set(0x1C, Layer { pitch: -4.0, loud: 120, ..own("boing.wav") }).unwrap();
        l.set(0x0E, Layer { pack_on: false, release: Release::None, pitch: 1.0, ..own("pop.mp3") }).unwrap();
        l.set(0x10, Layer { release: Release::File("b.ogg".into()), pitch: 99.0, loud: 900, ..own("a.wav") }).unwrap();
        let back = Layers::from_lines(&l.to_lines());
        assert_eq!(back, l);
        assert_eq!(back.of(0x10).pitch, 12.0);
        assert_eq!(back.of(0x10).loud, 200);
        assert_eq!(back.files(), vec!["a.wav", "b.ogg", "boing.wav", "pop.mp3"]);
        // junk lines are skipped, never a panic
        let junk = Layers::from_lines(&["x".into(), "1|1|a|=|nan|5".into(), "2|1|../x|=|0|100".into(), "3|1|ok.wav|=|0.5|100".into()]);
        assert_eq!(junk.len(), 1);
        assert!(junk.get(3).is_some());
    }

    #[test]
    fn a_file_id_is_a_plain_name() {
        assert!(file_id_ok("boing.wav"));
        for bad in ["", "a/b.wav", "..", "c:x", "a|b", "a\"b"] {
            assert!(!file_id_ok(bad), "{bad}");
        }
        let mut l = Layers::new();
        assert!(l.set(1, own("a\\b.wav")).is_err());
    }

    #[test]
    fn too_many_layers_are_refused() {
        let mut l = Layers::new();
        for s in 0..MAX_LAYERS as u16 {
            l.set(s, own("a.wav")).unwrap();
        }
        assert!(l.set(9999, own("a.wav")).is_err());
        assert!(l.set(0, own("b.wav")).is_ok(), "changing one that is there is fine");
    }

    #[test]
    fn variation_same_and_random() {
        let mut v = Vary::for_keys();
        v.pitch.set_value(true, 2.0);
        assert_eq!(v.at(0x1E, true), (2.0, 100));
        assert_eq!(v.at(0x20, true), (2.0, 100), "Same: every normal key identical");
        assert_eq!(v.at(5, false), (2.0, 100), "a mouse / pad button has no special shape");
        v.loud.mode = Mode::Random;
        v.loud.set_value(false, 10.0);
        let a = v.at(0x1E, true).1;
        assert_eq!(a, v.at(0x1E, true).1, "random is fixed per key: the same key, the same sound every press");
        assert!((90..=110).contains(&a));
        let spread: std::collections::HashSet<u16> = (0..40u16).map(|s| v.at(s, false).1).collect();
        assert!(spread.len() > 5, "different keys differ");
        v.pitch.mode = Mode::Random;
        v.pitch.set_value(true, 1.5);
        let pitches: std::collections::HashSet<i32> = (0x10..0x30u16).map(|s| (v.at(s, false).0 * 10.0) as i32).collect();
        assert!(pitches.len() > 5, "random pitch differs per key");
        assert!((0x10..0x30u16).all(|s| v.at(s, false).0.abs() <= 1.5));
        // slider steps and ends
        v.pitch.mode = Mode::Same;
        v.pitch.set_value(true, 30.0);
        assert_eq!(v.pitch.same, 12.0);
        v.pitch.set_value(true, 1.26);
        assert_eq!(v.pitch.same, 1.5, "steps of 0.5");
        assert_eq!(v.pitch.rnd, 1.5, "each mode keeps its own slider value");
        assert_eq!(Vary::from_text(&v.to_text()), Some(v));
    }

    #[test]
    fn special_keys_keep_their_shape_in_same_and_in_random() {
        let space = 0x39;
        let board: Vec<u16> = vec![0x1E, 0x1F, 0x2C, 0x10, 0x02, 0x1C, 0x0E, 0x36, 0x2A, 0x3A, 0x0F, 0x1D, 0x38, 0xE05B, 0xE01C, 0x4E, 0x52];
        for random in [false, true] {
            let mut v = Vary::for_keys();
            if random {
                v.pitch.mode = Mode::Random;
                v.pitch.set_value(true, 3.0);
                v.loud.mode = Mode::Random;
                v.loud.set_value(false, 30.0);
            }
            let sp = v.at(space, true);
            for &c in board.iter() {
                let k = v.at(c, true);
                assert!(k.0 > sp.0, "Space is the deepest (random {random}): {c:X} {k:?} vs {sp:?}");
            }
            assert!(sp.1 > 100, "Space a touch louder");
            // Enter lower than a letter by its shape, Ctrl a little lower, left Shift between
            let (enter, ctrl, lshift) = (v.at(0x1C, true).0, v.at(0x1D, true).0, v.at(0x2A, true).0);
            assert!(enter < lshift && lshift < ctrl, "Enter {enter} < L-Shift {lshift} < Ctrl {ctrl}");
            assert!(v.at(0xE01C, true).0 == enter && v.at(0x4E, true).0 == enter && v.at(0x52, true).0 == enter, "numpad Enter / + / 0 like Enter");
            // the same key always sounds the same
            assert_eq!(v.at(0x1E, true), v.at(0x1E, true));
        }
        // Same: the special keys = the one value + their shape
        let mut v = Vary::for_keys();
        v.pitch.set_value(true, 2.0);
        assert_eq!(v.at(space, true).0, -3.0);
        assert_eq!(v.at(0x1E, true).0, 2.0);
        // Random: no random on a special key, its shape alone
        v.pitch.mode = Mode::Random;
        v.pitch.set_value(true, 3.0);
        assert_eq!(v.at(space, true).0, -5.0);
        assert_eq!(v.at(0x1C, true).0, -3.0);
    }

    #[test]
    fn an_old_save_with_rising_loads_as_same() {
        let v = Vary::from_text("rise,0,7,1.5;same,100,30,10").unwrap();
        assert_eq!(v.pitch.mode, Mode::Same);
        assert_eq!(v.pitch.rnd, 1.5);
        assert_eq!(v.loud.same, 100.0);
        assert_eq!(Vary::from_text("rnd,0,2;same,100,10").unwrap().pitch.mode, Mode::Random);
        assert!(Vary::from_text("same,0;same,100,10").is_none());
    }

    #[test]
    fn the_whole_pack_puts_space_lowest() {
        let codes = [0x39, 0x1C, 0x02, 0x1E, 0x2C, 0xE01D];
        let keys = Made::spread(&Vary::for_pack(), &codes);
        let by = |c: u16| keys.iter().find(|k| k.0 == c).unwrap();
        assert_eq!((by(0x39).1, by(0x39).2), (-5.0, 112), "Space: deepest and a touch louder");
        assert_eq!((by(0x1E).1, by(0x1E).2), (0.0, 100), "a letter: Same, pitch 0, 100 %");
        assert!(keys.iter().all(|k| k.0 == 0x39 || k.1 > by(0x39).1));
    }

    #[test]
    fn a_made_pack_round_trips_and_refuses_bad_files() {
        let m = Made { name: "My thock".into(), press: "thock.wav".into(), release: None, vary: Vary::for_pack(), keys: vec![(0x39, -3.5, 100), (0xE01C, 1.2, 90)] };
        assert_eq!(Made::from_text(&m.to_text()), Ok(m.clone()));
        let bad = m.to_text().replace("press=thock.wav", "press=..\\x.wav");
        assert!(Made::from_text(&bad).is_err());
        assert!(Made::from_text("garbage").is_err());
        assert!((speed(12.0) - 2.0).abs() < 1e-6);
        assert!((speed(0.0) - 1.0).abs() < 1e-6);
    }
}

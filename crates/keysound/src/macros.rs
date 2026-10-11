//! Macros (Order 058): a list of STEPS the user builds (press a key or combo, type a text, wait N ms, open an app / file /
//! website) and the plan of input events they become. Pure: the model, its JSON form for the settings file, the checks and
//! [`plan`]; sending the events is [`crate::send`] (SendInput on one worker thread).
//!
//! Safety is in [`run`] (also pure, tested with fakes): before every event the runner asks whether input may go out NOW — it
//! must not while a game / full-screen window is in front (no input into games: his Vanguard) nor while the window in front
//! is an administrator window (never type into those). If the answer turns to "no" mid-way the macro stops and lets go of any
//! key it holds. Nothing here records anything: the recorder in the page only keeps what the user saves.

use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};

pub const MAX_STEPS: usize = 64;
pub const MAX_TEXT: usize = 400;
pub const MAX_WAIT_MS: u32 = 30_000;
/// All the waits of one macro together (a macro is a quick helper, not a program).
pub const MAX_TOTAL_WAIT_MS: u32 = 120_000;
pub const MAX_MACROS: usize = 32;
pub const MAX_NAME: usize = 40;
pub const MAX_KEYS_IN_STEP: usize = 6;

const VK_RETURN: u16 = 0x0D;
const VK_TAB: u16 = 0x09;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// "Press": a key or combo - modifiers first, the main key last (virtual-key codes), pressed together.
    Keys(Vec<u16>),
    Type(String),
    Wait(u32),
    /// An app / file path or a web address.
    Open(String),
    /// Order 090 (his "hold in hold out on keys"): one key goes down and stays down ...
    Down(u16),
    /// ... until this lets it go (a macro that ends with a key still held lets go of it at the end).
    Up(u16),
    /// A mouse click: [`CLICK_LEFT`] .. [`CLICK_X2`].
    Click(u8),
    /// A controller button (`bu_rawin::padbtn` numbers): only in a controller button's macro, which Steam plays; a keyboard /
    /// mouse macro never offers it and its runner skips it.
    Pad(u8),
}

/// The mouse buttons a Click step can press.
pub const CLICK_LEFT: u8 = 0;
pub const CLICK_RIGHT: u8 = 1;
pub const CLICK_MIDDLE: u8 = 2;
pub const CLICK_X1: u8 = 3;
pub const CLICK_X2: u8 = 4;

/// A Click step's name.
pub fn click_name(b: u8) -> &'static str {
    match b {
        CLICK_LEFT => "Left click",
        CLICK_RIGHT => "Right click",
        CLICK_MIDDLE => "Wheel click",
        CLICK_X1 => "Back (side)",
        CLICK_X2 => "Forward (side)",
        _ => "Click",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Macro {
    pub id: String,
    pub name: String,
    pub steps: Vec<Step>,
    /// Order 090: how often a press runs it (the editor's "Repeat").
    pub rep: Repeat,
}

/// The editor's "Repeat": once, a few times, while the key is held, or until it is pressed again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Repeat {
    #[default]
    Once,
    Times(u8),
    /// Runs again and again while the key / button stays down; letting go stops it.
    Held,
    /// Runs again and again until the key / button is pressed a second time.
    Toggle,
}

impl Repeat {
    /// The choices in the editor's list, in order.
    pub const ALL: [Repeat; 7] = [Repeat::Once, Repeat::Times(2), Repeat::Times(3), Repeat::Times(5), Repeat::Times(10), Repeat::Held, Repeat::Toggle];

    pub fn label(self) -> String {
        match self {
            Repeat::Once | Repeat::Times(0 | 1) => "Once".into(),
            Repeat::Times(n) => format!("{n} times"),
            Repeat::Held => "While the key is held".into(),
            Repeat::Toggle => "Until pressed again".into(),
        }
    }

    fn key(self) -> String {
        match self {
            Repeat::Once | Repeat::Times(0 | 1) => "1".into(),
            Repeat::Times(n) => n.to_string(),
            Repeat::Held => "held".into(),
            Repeat::Toggle => "toggle".into(),
        }
    }

    fn from_key(s: &str) -> Repeat {
        match s {
            "held" => Repeat::Held,
            "toggle" => Repeat::Toggle,
            n => match n.parse::<u8>() {
                Ok(0 | 1) | Err(_) => Repeat::Once,
                Ok(n) => Repeat::Times(n.min(MAX_TIMES)),
            },
        }
    }

    /// How many times one press plays the steps (None = until stopped: held / toggle).
    pub fn times(self) -> Option<u32> {
        match self {
            Repeat::Once => Some(1),
            Repeat::Times(n) => Some(u32::from(n.clamp(1, MAX_TIMES))),
            Repeat::Held | Repeat::Toggle => None,
        }
    }
}

/// The most a "N times" macro repeats.
pub const MAX_TIMES: u8 = 10;

impl Macro {
    pub fn new(id: &str, name: &str) -> Macro {
        Macro { id: id.into(), name: name.into(), steps: Vec::new(), rep: Repeat::Once }
    }

    /// Fine to save and run?
    pub fn check(&self) -> Result<(), String> {
        if self.id.is_empty() || self.id.len() > 16 || !self.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err("bad macro id".into());
        }
        if self.name.chars().count() > MAX_NAME || self.name.chars().any(|c| c.is_control()) {
            return Err("the name is too long".into());
        }
        if self.steps.len() > MAX_STEPS {
            return Err(format!("at most {MAX_STEPS} steps"));
        }
        let mut waits = 0u32;
        for s in &self.steps {
            match s {
                Step::Keys(k) => {
                    if k.is_empty() || k.len() > MAX_KEYS_IN_STEP || k.iter().any(|v| *v == 0 || *v >= 0xFF) {
                        return Err("a key step needs 1 to 6 keys".into());
                    }
                }
                Step::Type(t) => {
                    if t.chars().count() > MAX_TEXT || t.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
                        return Err(format!("a text step is at most {MAX_TEXT} characters"));
                    }
                }
                Step::Wait(ms) => {
                    if *ms > MAX_WAIT_MS {
                        return Err(format!("a wait is at most {} s", MAX_WAIT_MS / 1000));
                    }
                    waits = waits.saturating_add(*ms);
                }
                Step::Open(t) => {
                    if t.is_empty() || t.len() > crate::binds::MAX_TARGET || t.chars().any(|c| c.is_control()) || t.contains('"') {
                        return Err("an open step needs an app, file or website".into());
                    }
                    if t.contains("://") && !crate::binds::is_web(t) {
                        return Err("a website must start with http:// or https://".into());
                    }
                }
                Step::Down(vk) | Step::Up(vk) => {
                    if *vk == 0 || *vk >= 0xFF {
                        return Err("a key down / up step needs a key".into());
                    }
                }
                Step::Click(b) => {
                    if *b > CLICK_X2 {
                        return Err("a click step needs a mouse button".into());
                    }
                }
                Step::Pad(b) => {
                    if *b > 18 {
                        return Err("a button step needs a controller button".into());
                    }
                }
            }
        }
        if waits > MAX_TOTAL_WAIT_MS {
            return Err(format!("the waits add up to more than {} s", MAX_TOTAL_WAIT_MS / 1000));
        }
        Ok(())
    }

    /// The macro as saved: one JSON object (`{"id":…,"name":…,"steps":[{"k":[17,75]},{"t":"text"},{"w":200},{"o":"https://…"}]}`).
    pub fn to_json(&self) -> String {
        let steps: Vec<Value> = self
            .steps
            .iter()
            .map(|s| match s {
                Step::Keys(k) => json!({ "k": k }),
                Step::Type(t) => json!({ "t": t }),
                Step::Wait(w) => json!({ "w": w }),
                Step::Open(o) => json!({ "o": o }),
                Step::Down(k) => json!({ "d": k }),
                Step::Up(k) => json!({ "u": k }),
                Step::Click(b) => json!({ "c": b }),
                Step::Pad(b) => json!({ "p": b }),
            })
            .collect();
        if self.rep == Repeat::Once {
            json!({ "id": self.id, "name": self.name, "steps": steps }).to_string()
        } else {
            json!({ "id": self.id, "name": self.name, "steps": steps, "r": self.rep.key() }).to_string()
        }
    }

    pub fn from_json(s: &str) -> Result<Macro, String> {
        let v: Value = serde_json::from_str(s).map_err(|e| format!("not a macro: {e}"))?;
        let id = v.get("id").and_then(Value::as_str).ok_or("no id")?.to_string();
        let name = v.get("name").and_then(Value::as_str).unwrap_or("Macro").to_string();
        let mut steps = Vec::new();
        for st in v.get("steps").and_then(Value::as_array).ok_or("no steps")? {
            let step = if let Some(k) = st.get("k").and_then(Value::as_array) {
                Step::Keys(k.iter().filter_map(|x| x.as_u64()).filter_map(|x| u16::try_from(x).ok()).collect())
            } else if let Some(t) = st.get("t").and_then(Value::as_str) {
                Step::Type(t.to_string())
            } else if let Some(w) = st.get("w").and_then(Value::as_u64) {
                Step::Wait(u32::try_from(w).unwrap_or(u32::MAX))
            } else if let Some(o) = st.get("o").and_then(Value::as_str) {
                Step::Open(o.to_string())
            } else if let Some(k) = st.get("d").and_then(Value::as_u64) {
                Step::Down(u16::try_from(k).unwrap_or(0))
            } else if let Some(k) = st.get("u").and_then(Value::as_u64) {
                Step::Up(u16::try_from(k).unwrap_or(0))
            } else if let Some(b) = st.get("c").and_then(Value::as_u64) {
                Step::Click(u8::try_from(b).unwrap_or(u8::MAX))
            } else if let Some(b) = st.get("p").and_then(Value::as_u64) {
                Step::Pad(u8::try_from(b).unwrap_or(u8::MAX))
            } else {
                return Err("an unknown step".into());
            };
            steps.push(step);
        }
        let rep = v.get("r").and_then(Value::as_str).map(Repeat::from_key).unwrap_or_default();
        let m = Macro { id, name, steps, rep };
        m.check()?;
        Ok(m)
    }
}

/// Ready-made macros (Order 059): (name, steps). Placeholders (an e-mail address, three sites) are for the user to edit in the
/// macro window; every one passes [`Macro::check`].
pub fn templates() -> Vec<(&'static str, Vec<Step>)> {
    const CTRL: u16 = 0x11;
    vec![
        ("Type my e-mail", vec![Step::Type("your.name@example.com".into())]),
        (
            "Open 3 sites",
            vec![
                Step::Open("https://www.youtube.com".into()),
                Step::Wait(500),
                Step::Open("https://www.twitch.tv".into()),
                Step::Wait(500),
                Step::Open("https://mail.google.com".into()),
            ],
        ),
        (
            "Copy + search Google",
            vec![Step::Keys(vec![CTRL, 0x43]), Step::Wait(150), Step::Open("https://www.google.com".into()), Step::Wait(1500), Step::Keys(vec![CTRL, 0x56]), Step::Keys(vec![VK_RETURN])],
        ),
    ]
}

/// A fresh macro id not used by `existing` (`m1`, `m2` …).
pub fn new_id(existing: &[Macro]) -> String {
    (1..).map(|n| format!("m{n}")).find(|id| !existing.iter().any(|m| &m.id == id)).unwrap_or_else(|| "m0".into())
}

/// The macros in the settings file's lines (a line that isn't a good macro is left out), at most [`MAX_MACROS`].
pub fn from_lines(lines: &[String]) -> Vec<Macro> {
    let mut out: Vec<Macro> = Vec::new();
    for l in lines {
        if let Ok(m) = Macro::from_json(l) {
            if out.len() < MAX_MACROS && !out.iter().any(|o| o.id == m.id) {
                out.push(m);
            }
        }
    }
    out
}

pub fn to_lines(ms: &[Macro]) -> Vec<String> {
    ms.iter().take(MAX_MACROS).map(Macro::to_json).collect()
}

/// One thing to send / do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ev {
    Key { vk: u16, down: bool },
    /// A mouse button ([`CLICK_LEFT`] ..).
    Mouse { button: u8, down: bool },
    /// One UTF-16 unit typed as a character (KEYEVENTF_UNICODE), so any text works on any keyboard layout.
    Char { unit: u16, down: bool },
    Sleep(u32),
    Open(String),
}

/// Typing pace: a short pause after each character so slow apps keep up.
const CHAR_GAP_MS: u32 = 4;

/// The events of a macro, in order. A combo presses its keys in order and lets go in reverse; typed text is sent as
/// characters (Enter and Tab as their keys).
pub fn plan(m: &Macro) -> Vec<Ev> {
    let mut v = Vec::new();
    for s in &m.steps {
        match s {
            Step::Keys(k) => {
                v.extend(k.iter().map(|&vk| Ev::Key { vk, down: true }));
                v.extend(k.iter().rev().map(|&vk| Ev::Key { vk, down: false }));
                v.push(Ev::Sleep(CHAR_GAP_MS));
            }
            Step::Type(t) => {
                for c in t.chars() {
                    match c {
                        '\n' | '\r' => {
                            v.push(Ev::Key { vk: VK_RETURN, down: true });
                            v.push(Ev::Key { vk: VK_RETURN, down: false });
                        }
                        '\t' => {
                            v.push(Ev::Key { vk: VK_TAB, down: true });
                            v.push(Ev::Key { vk: VK_TAB, down: false });
                        }
                        c => {
                            let mut b = [0u16; 2];
                            for &unit in c.encode_utf16(&mut b).iter() {
                                v.push(Ev::Char { unit, down: true });
                                v.push(Ev::Char { unit, down: false });
                            }
                        }
                    }
                    v.push(Ev::Sleep(CHAR_GAP_MS));
                }
            }
            Step::Wait(ms) => v.push(Ev::Sleep(*ms)),
            Step::Open(t) => v.push(Ev::Open(t.clone())),
            Step::Down(vk) => v.push(Ev::Key { vk: *vk, down: true }),
            Step::Up(vk) => v.push(Ev::Key { vk: *vk, down: false }),
            Step::Click(b) => {
                v.push(Ev::Mouse { button: *b, down: true });
                v.push(Ev::Sleep(CHAR_GAP_MS));
                v.push(Ev::Mouse { button: *b, down: false });
                v.push(Ev::Sleep(CHAR_GAP_MS));
            }
            // Steam plays a controller macro; nothing here can press a controller button
            Step::Pad(_) => {}
        }
    }
    v
}

/// Where the events go (the real one is SendInput + ShellExecute; tests record them).
pub trait Out {
    fn key(&mut self, vk: u16, down: bool);
    fn mouse(&mut self, button: u8, down: bool);
    fn unit(&mut self, unit: u16, down: bool);
    fn open(&mut self, target: &str) -> Result<(), String>;
    /// Sleep up to `ms`; the real one returns early when asked to stop.
    fn sleep(&mut self, ms: u32);
}

/// May input go out right now? None = yes; Some(reason) = no.
pub trait Safety {
    fn blocked(&mut self) -> Option<String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// It stopped before the end: why ("a game or full-screen window is in front", "an administrator window is in front", "stopped").
    Stopped(String),
}

/// Runs `events`: before each key / character / open the safety is asked; a "no" (or `cancel`) stops it and releases every
/// key the macro still holds. Sleeps are cut in slices of 25 ms so a "no" or a cancel is noticed quickly.
pub fn run(events: &[Ev], out: &mut dyn Out, safety: &mut dyn Safety, cancel: &AtomicBool) -> Outcome {
    let mut held: Vec<u16> = Vec::new();
    let mut held_mouse: Vec<u8> = Vec::new();
    // every way out lets go of what the macro holds: keys AND mouse buttons (a cancel inside a Click's down / up must
    // never leave the button down - Order 090 review)
    let stop = |held: &mut Vec<u16>, held_mouse: &mut Vec<u8>, out: &mut dyn Out, why: String| {
        for b in held_mouse.drain(..) {
            out.mouse(b, false);
        }
        for vk in held.drain(..).rev() {
            out.key(vk, false);
        }
        Outcome::Stopped(why)
    };
    for e in events {
        if cancel.load(Ordering::SeqCst) {
            return stop(&mut held, &mut held_mouse, out, "stopped".into());
        }
        match e {
            Ev::Sleep(ms) => {
                let mut left = *ms;
                while left > 0 {
                    let slice = left.min(25);
                    out.sleep(slice);
                    left -= slice;
                    if cancel.load(Ordering::SeqCst) {
                        return stop(&mut held, &mut held_mouse, out, "stopped".into());
                    }
                    if *ms > 100 {
                        if let Some(why) = safety.blocked() {
                            return stop(&mut held, &mut held_mouse, out, why);
                        }
                    }
                }
            }
            Ev::Key { vk, down } => {
                if let Some(why) = safety.blocked() {
                    return stop(&mut held, &mut held_mouse, out, why);
                }
                out.key(*vk, *down);
                if *down {
                    held.push(*vk);
                } else {
                    held.retain(|h| h != vk);
                }
            }
            Ev::Char { unit, down } => {
                if let Some(why) = safety.blocked() {
                    return stop(&mut held, &mut held_mouse, out, why);
                }
                out.unit(*unit, *down);
            }
            Ev::Mouse { button, down } => {
                if let Some(why) = safety.blocked() {
                    return stop(&mut held, &mut held_mouse, out, why);
                }
                out.mouse(*button, *down);
                if *down {
                    held_mouse.push(*button);
                } else {
                    held_mouse.retain(|b| b != button);
                }
            }
            Ev::Open(t) => {
                if let Some(why) = safety.blocked() {
                    return stop(&mut held, &mut held_mouse, out, why);
                }
                if let Err(e) = out.open(t) {
                    return stop(&mut held, &mut held_mouse, out, e);
                }
            }
        }
    }
    // a key put down by a Key down step and never let go: let go of it now (nothing stays stuck after a macro)
    for b in held_mouse.drain(..) {
        out.mouse(b, false);
    }
    for vk in held.drain(..).rev() {
        out.key(vk, false);
    }
    Outcome::Done
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mac(steps: Vec<Step>) -> Macro {
        Macro { id: "m1".into(), name: "Test".into(), steps, rep: Repeat::Once }
    }

    #[derive(Default)]
    struct Rec {
        log: Vec<String>,
    }
    impl Out for Rec {
        fn key(&mut self, vk: u16, down: bool) {
            self.log.push(format!("{}{vk:X}", if down { "v" } else { "^" }));
        }
        fn mouse(&mut self, b: u8, down: bool) {
            self.log.push(format!("{}M{b}", if down { "v" } else { "^" }));
        }
        fn unit(&mut self, unit: u16, down: bool) {
            self.log.push(format!("{}'{}'", if down { "v" } else { "^" }, char::from_u32(unit as u32).unwrap_or('?')));
        }
        fn open(&mut self, t: &str) -> Result<(), String> {
            self.log.push(format!("open {t}"));
            Ok(())
        }
        fn sleep(&mut self, ms: u32) {
            self.log.push(format!("zz{ms}"));
        }
    }

    /// Safe for the first `n` questions, then not.
    struct After(u32, &'static str);
    impl Safety for After {
        fn blocked(&mut self) -> Option<String> {
            if self.0 == 0 {
                Some(self.1.to_string())
            } else {
                self.0 -= 1;
                None
            }
        }
    }
    struct Always;
    impl Safety for Always {
        fn blocked(&mut self) -> Option<String> {
            None
        }
    }

    #[test]
    fn a_combo_presses_in_order_and_lets_go_in_reverse() {
        let m = mac(vec![Step::Keys(vec![0x11, 0x10, 0x4B])]);
        let ev = plan(&m);
        assert_eq!(
            ev[..6],
            [
                Ev::Key { vk: 0x11, down: true },
                Ev::Key { vk: 0x10, down: true },
                Ev::Key { vk: 0x4B, down: true },
                Ev::Key { vk: 0x4B, down: false },
                Ev::Key { vk: 0x10, down: false },
                Ev::Key { vk: 0x11, down: false },
            ]
        );
    }

    #[test]
    fn text_is_typed_as_characters_with_enter_and_tab_as_keys() {
        let m = mac(vec![Step::Type("a\nč\t".into())]);
        let mut rec = Rec::default();
        let out = run(&plan(&m), &mut rec, &mut Always, &AtomicBool::new(false));
        assert_eq!(out, Outcome::Done);
        let keys: Vec<&String> = rec.log.iter().filter(|l| !l.starts_with("zz")).collect();
        assert_eq!(keys, ["v'a'", "^'a'", "vD", "^D", "v'č'", "^'č'", "v9", "^9"]);
    }

    #[test]
    fn a_character_outside_the_bmp_is_two_units() {
        let m = mac(vec![Step::Type("😀".into())]);
        let n = plan(&m).iter().filter(|e| matches!(e, Ev::Char { .. })).count();
        assert_eq!(n, 4, "two UTF-16 units, each down and up");
    }

    #[test]
    fn waits_are_slept_in_slices_and_open_goes_through() {
        let m = mac(vec![Step::Wait(60), Step::Open("https://example.com".into())]);
        let mut rec = Rec::default();
        assert_eq!(run(&plan(&m), &mut rec, &mut Always, &AtomicBool::new(false)), Outcome::Done);
        assert_eq!(rec.log, ["zz25", "zz25", "zz10", "open https://example.com"]);
    }

    /// The safety rule of Order 058: never into a game / full-screen window, never into an admin window.
    #[test]
    fn a_game_or_admin_window_in_front_stops_the_macro_before_it_sends_anything() {
        let m = mac(vec![Step::Keys(vec![0x11, 0x43]), Step::Type("hi".into()), Step::Open("C:\\x.exe".into())]);
        let mut rec = Rec::default();
        let out = run(&plan(&m), &mut rec, &mut After(0, "a game or full-screen window is in front"), &AtomicBool::new(false));
        assert_eq!(out, Outcome::Stopped("a game or full-screen window is in front".into()));
        assert!(rec.log.is_empty(), "nothing was sent: {:?}", rec.log);
    }

    #[test]
    fn if_a_game_comes_to_the_front_midway_it_stops_and_lets_go_of_held_keys() {
        // Ctrl down is allowed (1 question), the next key down is not
        let m = mac(vec![Step::Keys(vec![0x11, 0x43])]);
        let mut rec = Rec::default();
        let out = run(&plan(&m), &mut rec, &mut After(1, "an administrator window is in front"), &AtomicBool::new(false));
        assert_eq!(out, Outcome::Stopped("an administrator window is in front".into()));
        assert_eq!(rec.log, ["v11", "^11"], "Ctrl is released, not left stuck");
    }

    #[test]
    fn cancel_stops_at_once_and_releases() {
        let m = mac(vec![Step::Keys(vec![0x10, 0x41]), Step::Wait(1000)]);
        let cancel = AtomicBool::new(false);
        struct StopAfterDown<'a>(&'a AtomicBool, Rec);
        impl Out for StopAfterDown<'_> {
            fn key(&mut self, vk: u16, down: bool) {
                self.1.key(vk, down);
                if down && vk == 0x10 {
                    self.0.store(true, Ordering::SeqCst);
                }
            }
            fn mouse(&mut self, b: u8, d: bool) {
                self.1.mouse(b, d)
            }
            fn unit(&mut self, u: u16, d: bool) {
                self.1.unit(u, d)
            }
            fn open(&mut self, t: &str) -> Result<(), String> {
                self.1.open(t)
            }
            fn sleep(&mut self, ms: u32) {
                self.1.sleep(ms)
            }
        }
        let mut out = StopAfterDown(&cancel, Rec::default());
        assert_eq!(run(&plan(&m), &mut out, &mut Always, &cancel), Outcome::Stopped("stopped".into()));
        assert_eq!(out.1.log, ["v10", "^10"]);
    }

    #[test]
    fn checks_refuse_bad_macros() {
        assert!(mac(vec![Step::Keys(vec![])]).check().is_err());
        assert!(mac(vec![Step::Keys(vec![0xFF])]).check().is_err());
        assert!(mac(vec![Step::Keys(vec![1, 2, 3, 4, 5, 6, 7])]).check().is_err());
        assert!(mac(vec![Step::Type("x".repeat(MAX_TEXT + 1))]).check().is_err());
        assert!(mac(vec![Step::Wait(MAX_WAIT_MS + 1)]).check().is_err());
        assert!(mac(vec![Step::Wait(30_000); 5]).check().is_err(), "waits add up");
        assert!(mac(vec![Step::Open(String::new())]).check().is_err());
        assert!(mac(vec![Step::Open("file:///C:/x".into())]).check().is_err());
        assert!(mac(vec![Step::Open("javascript://x".into())]).check().is_err());
        assert!(mac(vec![Step::Open("https://ok.example".into()), Step::Open("C:\\Windows\\notepad.exe".into())]).check().is_ok());
        assert!(mac(vec![Step::Wait(0); MAX_STEPS + 1]).check().is_err());
        let mut m = mac(vec![]);
        m.id = "bad id!".into();
        assert!(m.check().is_err());
    }

    #[test]
    fn json_round_trip_and_bad_json() {
        let m = Macro {
            id: "m7".into(),
            name: "Open my notes".into(),
            steps: vec![Step::Keys(vec![0x5B, 0x52]), Step::Wait(300), Step::Type("notepad \"x\"\n".into()), Step::Open("https://example.com".into())],
            rep: Repeat::Once,
        };
        let j = m.to_json();
        assert_eq!(Macro::from_json(&j), Ok(m.clone()));
        assert!(Macro::from_json("nope").is_err());
        assert!(Macro::from_json(r#"{"id":"m1","name":"x","steps":[{"z":1}]}"#).is_err());
        assert!(Macro::from_json(r#"{"id":"m1","name":"x","steps":[{"w":99999999}]}"#).is_err());
        let lines = vec![j.clone(), "junk".into(), j, Macro::new("m2", "Two").to_json()];
        let ms = from_lines(&lines);
        assert_eq!(ms.len(), 2, "junk and a repeated id are left out");
        assert_eq!(new_id(&ms), "m1");
        assert_eq!(new_id(&[Macro::new("m1", "a"), Macro::new("m2", "b")]), "m3");
        assert_eq!(new_id(&[]), "m1");
    }

    /// Order 090: Key down / Key up / Click / a controller button - saved, planned and run; a key left down is let go at the end.
    #[test]
    fn hold_release_click_and_button_steps() {
        let m = Macro {
            id: "m3".into(),
            name: "Hold".into(),
            steps: vec![Step::Down(0x10), Step::Click(CLICK_RIGHT), Step::Keys(vec![0x41]), Step::Up(0x10), Step::Down(0x11), Step::Pad(3)],
            rep: Repeat::Times(3),
        };
        assert_eq!(Repeat::from_key(&Repeat::Held.key()), Repeat::Held);
        assert_eq!(Repeat::from_key("99"), Repeat::Times(MAX_TIMES));
        assert_eq!(Repeat::Times(3).times(), Some(3));
        assert_eq!(Repeat::Toggle.times(), None);
        assert!(m.check().is_ok());
        assert_eq!(Macro::from_json(&m.to_json()), Ok(m.clone()));
        let mut out = Rec::default();
        let cancel = AtomicBool::new(false);
        assert_eq!(run(&plan(&m), &mut out, &mut Always, &cancel), Outcome::Done);
        out.log.retain(|l| !l.starts_with("zz"));
        assert_eq!(out.log, vec!["v10", "vM1", "^M1", "v41", "^41", "^10", "v11", "^11"], "shift held over the click and A; Ctrl let go at the end; the button step is Steam's");
        assert!(mac(vec![Step::Down(0)]).check().is_err());
        assert!(mac(vec![Step::Click(9)]).check().is_err());
        assert!(mac(vec![Step::Pad(200)]).check().is_err());
        assert_eq!(click_name(CLICK_X2), "Forward (side)");
    }

    #[test]
    fn the_ready_made_macros_are_valid_and_named() {
        let t = templates();
        assert_eq!(t.len(), 3);
        for (name, steps) in t {
            assert!(!name.is_empty() && name.chars().count() <= MAX_NAME);
            let m = mac(steps);
            assert_eq!(m.check(), Ok(()), "{name}");
            assert!(!m.steps.is_empty());
            assert!(!plan(&m).is_empty(), "{name} plans some input");
        }
    }
}

#[cfg(test)]
mod stop_tests {
    use super::*;

    /// Order 090 review: a cancel that lands between a Click's down and up lets go of the button.
    #[test]
    fn a_cancel_inside_a_click_lets_go_of_the_button() {
        struct CancelOnDown<'a>(&'a AtomicBool, Vec<String>);
        impl Out for CancelOnDown<'_> {
            fn key(&mut self, vk: u16, down: bool) {
                self.1.push(format!("{}{vk:X}", if down { "v" } else { "^" }));
            }
            fn mouse(&mut self, b: u8, down: bool) {
                self.1.push(format!("{}M{b}", if down { "v" } else { "^" }));
                if down {
                    self.0.store(true, Ordering::SeqCst);
                }
            }
            fn unit(&mut self, _: u16, _: bool) {}
            fn open(&mut self, _: &str) -> Result<(), String> {
                Ok(())
            }
            fn sleep(&mut self, _: u32) {}
        }
        struct Ok_;
        impl Safety for Ok_ {
            fn blocked(&mut self) -> Option<String> {
                None
            }
        }
        let cancel = AtomicBool::new(false);
        let mut m = Macro::new("m1", "Click");
        m.steps = vec![Step::Down(0x10), Step::Click(CLICK_LEFT)];
        let mut out = CancelOnDown(&cancel, Vec::new());
        assert_eq!(run(&plan(&m), &mut out, &mut Ok_, &cancel), Outcome::Stopped("stopped".into()));
        assert_eq!(out.1, vec!["v10", "vM0", "^M0", "^10"], "the button and the key are both let go");
    }
}

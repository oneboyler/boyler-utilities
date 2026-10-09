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
    /// A key or combo: modifiers first, the main key last (virtual-key codes), pressed together.
    Keys(Vec<u16>),
    Type(String),
    Wait(u32),
    /// An app / file path or a web address.
    Open(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Macro {
    pub id: String,
    pub name: String,
    pub steps: Vec<Step>,
}

impl Macro {
    pub fn new(id: &str, name: &str) -> Macro {
        Macro { id: id.into(), name: name.into(), steps: Vec::new() }
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
            })
            .collect();
        json!({ "id": self.id, "name": self.name, "steps": steps }).to_string()
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
            } else {
                return Err("an unknown step".into());
            };
            steps.push(step);
        }
        let m = Macro { id, name, steps };
        m.check()?;
        Ok(m)
    }
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
        }
    }
    v
}

/// Where the events go (the real one is SendInput + ShellExecute; tests record them).
pub trait Out {
    fn key(&mut self, vk: u16, down: bool);
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
    let stop = |held: &mut Vec<u16>, out: &mut dyn Out, why: String| {
        for vk in held.drain(..).rev() {
            out.key(vk, false);
        }
        Outcome::Stopped(why)
    };
    for e in events {
        if cancel.load(Ordering::SeqCst) {
            return stop(&mut held, out, "stopped".into());
        }
        match e {
            Ev::Sleep(ms) => {
                let mut left = *ms;
                while left > 0 {
                    let slice = left.min(25);
                    out.sleep(slice);
                    left -= slice;
                    if cancel.load(Ordering::SeqCst) {
                        return stop(&mut held, out, "stopped".into());
                    }
                    if *ms > 100 {
                        if let Some(why) = safety.blocked() {
                            return stop(&mut held, out, why);
                        }
                    }
                }
            }
            Ev::Key { vk, down } => {
                if let Some(why) = safety.blocked() {
                    return stop(&mut held, out, why);
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
                    return stop(&mut held, out, why);
                }
                out.unit(*unit, *down);
            }
            Ev::Open(t) => {
                if let Some(why) = safety.blocked() {
                    return stop(&mut held, out, why);
                }
                if let Err(e) = out.open(t) {
                    return stop(&mut held, out, e);
                }
            }
        }
    }
    Outcome::Done
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mac(steps: Vec<Step>) -> Macro {
        Macro { id: "m1".into(), name: "Test".into(), steps }
    }

    #[derive(Default)]
    struct Rec {
        log: Vec<String>,
    }
    impl Out for Rec {
        fn key(&mut self, vk: u16, down: bool) {
            self.log.push(format!("{}{vk:X}", if down { "v" } else { "^" }));
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
}

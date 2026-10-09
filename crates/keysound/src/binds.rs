//! What a key of the keyboard picture does besides being remapped (Order 058): a PRESET ACTION (media, volume, open an
//! app / folder / website), one of the APP's own actions (the keys manager's: mic mute, screenshot …), or a MACRO. Keyed by the
//! key's scancode (its place on the keyboard, whatever the layout calls it). Pure: the model, its text form for the settings
//! file, and the checks; running them is [`crate::send`], binding the key is the app's keys manager.

use crate::remap::Code;
use std::collections::BTreeMap;

/// The longest app path / folder / website kept.
pub const MAX_TARGET: usize = 500;
/// The most keys that can carry an action or a macro.
pub const MAX_BINDS: usize = 64;

/// Virtual keys the key-combo presets press (the left Ctrl / Alt / Shift / Win ones Windows' own shortcuts use).
const CTRL: u16 = 0x11;
const ALT: u16 = 0x12;
const SHIFT: u16 = 0x10;
const WIN: u16 = 0x5B;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preset {
    // Edit
    Copy,
    Paste,
    Cut,
    Undo,
    Redo,
    SelectAll,
    // Windows
    AltTab,
    TaskManager,
    LockPc,
    ShowDesktop,
    EmojiPanel,
    Snip,
    PrintScreen,
    // Browser
    BrowserBack,
    BrowserForward,
    BrowserRefresh,
    NewTab,
    CloseTab,
    ReopenTab,
    // Media
    PlayPause,
    NextTrack,
    PrevTrack,
    StopMedia,
    // Volume
    VolumeUp,
    VolumeDown,
    VolumeMute,
    /// The next sound output that is on becomes Windows' default (the app does this itself: Audio tab's devices).
    NextOutput,
    /// An app or file to open (its full path).
    OpenApp(String),
    OpenFolder(String),
    /// A web address (http / https only).
    OpenWeb(String),
}

impl Preset {
    /// The presets that need nothing more, in the page's order (grouped: Edit, Windows, Browser, Media, Volume).
    pub const SIMPLE: [Preset; 27] = [
        Preset::Copy,
        Preset::Paste,
        Preset::Cut,
        Preset::Undo,
        Preset::Redo,
        Preset::SelectAll,
        Preset::AltTab,
        Preset::TaskManager,
        Preset::LockPc,
        Preset::ShowDesktop,
        Preset::EmojiPanel,
        Preset::Snip,
        Preset::PrintScreen,
        Preset::BrowserBack,
        Preset::BrowserForward,
        Preset::BrowserRefresh,
        Preset::NewTab,
        Preset::CloseTab,
        Preset::ReopenTab,
        Preset::PlayPause,
        Preset::NextTrack,
        Preset::PrevTrack,
        Preset::StopMedia,
        Preset::VolumeUp,
        Preset::VolumeDown,
        Preset::VolumeMute,
        Preset::NextOutput,
    ];

    /// (saved id, what the user sees, group) of each preset.
    fn info(&self) -> (&'static str, &'static str, &'static str) {
        match self {
            Preset::Copy => ("edit.copy", "Copy", "Edit"),
            Preset::Paste => ("edit.paste", "Paste", "Edit"),
            Preset::Cut => ("edit.cut", "Cut", "Edit"),
            Preset::Undo => ("edit.undo", "Undo", "Edit"),
            Preset::Redo => ("edit.redo", "Redo", "Edit"),
            Preset::SelectAll => ("edit.selectall", "Select all", "Edit"),
            Preset::AltTab => ("win.alttab", "Switch window (Alt + Tab)", "Windows"),
            Preset::TaskManager => ("win.taskmgr", "Task Manager", "Windows"),
            Preset::LockPc => ("win.lock", "Lock the PC", "Windows"),
            Preset::ShowDesktop => ("win.desktop", "Show the desktop", "Windows"),
            Preset::EmojiPanel => ("win.emoji", "Emoji panel", "Windows"),
            Preset::Snip => ("win.snip", "Snipping tool (screenshot of an area)", "Windows"),
            Preset::PrintScreen => ("win.prtsc", "Print Screen", "Windows"),
            Preset::BrowserBack => ("web.back", "Back", "Browser"),
            Preset::BrowserForward => ("web.forward", "Forward", "Browser"),
            Preset::BrowserRefresh => ("web.refresh", "Refresh", "Browser"),
            Preset::NewTab => ("web.newtab", "New tab", "Browser"),
            Preset::CloseTab => ("web.closetab", "Close tab", "Browser"),
            Preset::ReopenTab => ("web.reopentab", "Reopen closed tab", "Browser"),
            Preset::PlayPause => ("media.playpause", "Play / pause", "Media"),
            Preset::NextTrack => ("media.next", "Next track", "Media"),
            Preset::PrevTrack => ("media.prev", "Previous track", "Media"),
            Preset::StopMedia => ("media.stop", "Stop", "Media"),
            Preset::VolumeUp => ("vol.up", "Volume up", "Volume"),
            Preset::VolumeDown => ("vol.down", "Volume down", "Volume"),
            Preset::VolumeMute => ("vol.mute", "Mute", "Volume"),
            Preset::NextOutput => ("vol.nextout", "Switch audio output", "Volume"),
            Preset::OpenApp(_) => ("open.app", "An app or file…", "Open"),
            Preset::OpenFolder(_) => ("open.folder", "A folder…", "Open"),
            Preset::OpenWeb(_) => ("open.web", "A website…", "Open"),
        }
    }

    /// Stable id (saved in the settings file).
    pub fn id(&self) -> &'static str {
        self.info().0
    }

    /// What the user sees.
    pub fn name(&self) -> &'static str {
        self.info().1
    }

    /// The group the page's list puts it in.
    pub fn group(&self) -> &'static str {
        self.info().2
    }

    /// The key combination the preset presses (modifiers first), for the ones that are a Windows / app shortcut. They go out as
    /// `SendInput` key presses into the window in front, so - like a macro - never over a game / full-screen window or an
    /// administrator window. The media and volume keys are not combos (they are for Windows itself).
    pub fn combo(&self) -> Option<&'static [u16]> {
        Some(match self {
            Preset::Copy => &[CTRL, 0x43],
            Preset::Paste => &[CTRL, 0x56],
            Preset::Cut => &[CTRL, 0x58],
            Preset::Undo => &[CTRL, 0x5A],
            Preset::Redo => &[CTRL, 0x59],
            Preset::SelectAll => &[CTRL, 0x41],
            Preset::AltTab => &[ALT, 0x09],
            Preset::TaskManager => &[CTRL, SHIFT, 0x1B],
            Preset::ShowDesktop => &[WIN, 0x44],
            Preset::EmojiPanel => &[WIN, 0xBE],
            Preset::Snip => &[WIN, SHIFT, 0x53],
            Preset::PrintScreen => &[0x2C],
            Preset::BrowserBack => &[ALT, 0x25],
            Preset::BrowserForward => &[ALT, 0x27],
            Preset::BrowserRefresh => &[0x74],
            Preset::NewTab => &[CTRL, 0x54],
            Preset::CloseTab => &[CTRL, 0x57],
            Preset::ReopenTab => &[CTRL, SHIFT, 0x54],
            _ => return None,
        })
    }

    /// The app path / folder / address, for the three that have one.
    pub fn target(&self) -> Option<&str> {
        match self {
            Preset::OpenApp(t) | Preset::OpenFolder(t) | Preset::OpenWeb(t) => Some(t),
            _ => None,
        }
    }

    /// Opens something on screen (so it never runs over a game / full-screen window); the media and volume keys don't.
    pub fn opens_something(&self) -> bool {
        self.target().is_some()
    }

    /// The preset with this id (and target, for the three that need one).
    pub fn from_parts(id: &str, target: &str) -> Option<Preset> {
        let p = match id {
            "open.app" => Preset::OpenApp(target.trim().to_string()),
            "open.folder" => Preset::OpenFolder(target.trim().to_string()),
            "open.web" => Preset::OpenWeb(target.trim().to_string()),
            _ => Preset::SIMPLE.iter().find(|p| p.id() == id)?.clone(),
        };
        p.check().is_ok().then_some(p)
    }

    /// Is the target usable? (Empty is allowed for editing: it just does nothing until one is typed — see [`Preset::ready`].)
    pub fn check(&self) -> Result<(), String> {
        let Some(t) = self.target() else { return Ok(()) };
        if t.len() > MAX_TARGET || t.chars().any(|c| c.is_control()) {
            return Err("that is too long or has odd characters".into());
        }
        if let Preset::OpenWeb(w) = self {
            if !w.is_empty() && !is_web(w) {
                return Err("a website must start with http:// or https://".into());
            }
        }
        if t.contains('"') {
            return Err("a path can't hold quotes".into());
        }
        Ok(())
    }

    /// Has everything it needs to run (an open-something preset needs its target).
    pub fn ready(&self) -> bool {
        self.check().is_ok() && self.target().is_none_or(|t| !t.is_empty())
    }
}

/// An http / https address with something after the scheme.
pub fn is_web(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    (l.starts_with("http://") && l.len() > 7 || l.starts_with("https://") && l.len() > 8) && !s.contains(char::is_whitespace)
}

/// What one key carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bind {
    Preset(Preset),
    /// One of the app's own actions: the keys manager's action id (`micmute.toggle` …).
    App(String),
    /// A macro, by its id.
    Macro(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Binds {
    map: BTreeMap<Code, Bind>,
}

impl Binds {
    pub const fn new() -> Binds {
        Binds { map: BTreeMap::new() }
    }

    pub fn get(&self, c: Code) -> Option<&Bind> {
        self.map.get(&c)
    }

    pub fn set(&mut self, c: Code, b: Bind) -> Result<(), String> {
        if !self.map.contains_key(&c) && self.map.len() >= MAX_BINDS {
            return Err(format!("at most {MAX_BINDS} keys with an action"));
        }
        self.map.insert(c, b);
        Ok(())
    }

    pub fn remove(&mut self, c: Code) -> Option<Bind> {
        self.map.remove(&c)
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (Code, &Bind)> {
        self.map.iter().map(|(c, b)| (*c, b))
    }

    /// The keys that run macro `id`.
    pub fn keys_of_macro(&self, id: &str) -> Vec<Code> {
        self.map.iter().filter(|(_, b)| matches!(b, Bind::Macro(m) if m == id)).map(|(c, _)| *c).collect()
    }

    /// Forget every key that runs macro `id` (the macro was deleted).
    pub fn drop_macro(&mut self, id: &str) {
        self.map.retain(|_, b| !matches!(b, Bind::Macro(m) if m == id));
    }

    /// The settings file's lines: `<code hex>|p|<preset id>|<target>`, `<code hex>|a|<action id>`, `<code hex>|m|<macro id>`.
    pub fn to_lines(&self) -> Vec<String> {
        self.map
            .iter()
            .map(|(c, b)| match b {
                Bind::Preset(p) => format!("{c:X}|p|{}|{}", p.id(), p.target().unwrap_or("")),
                Bind::App(a) => format!("{c:X}|a|{a}"),
                Bind::Macro(m) => format!("{c:X}|m|{m}"),
            })
            .collect()
    }

    /// Reads such lines back; a line that isn't right is left out (never a failure).
    pub fn from_lines(lines: &[String]) -> Binds {
        let mut b = Binds::new();
        for l in lines {
            let mut it = l.splitn(4, '|');
            let (Some(c), Some(k), Some(id)) = (it.next(), it.next(), it.next()) else { continue };
            let rest = it.next().unwrap_or("");
            let Ok(code) = u16::from_str_radix(c, 16) else { continue };
            if code == 0 || id.is_empty() {
                continue;
            }
            let bind = match k {
                "p" => Preset::from_parts(id, rest).map(Bind::Preset),
                "a" => Some(Bind::App(id.to_string())),
                "m" => Some(Bind::Macro(id.to_string())),
                _ => None,
            };
            if let Some(bind) = bind {
                let _ = b.set(code, bind);
            }
        }
        b
    }
}

/// Would running `bind` press `vk` BY ITSELF (no modifier held) - the very key it is bound to? A key bound through Windows'
/// hotkeys is taken from every app, input we send ourselves included: such a bind would run itself again and the key
/// would never type (the owner's "J → macro that presses J": "its like an infinite loop"). A TYPED text (`Step::Type`) goes out
/// as Unicode, never as that key, so a macro can still type its own letter.
pub fn presses_own_key(bind: &Bind, macros: &[crate::macros::Macro], vk: u16) -> bool {
    match bind {
        Bind::Preset(p) => p.combo() == Some(&[vk][..]),
        Bind::Macro(id) => macros.iter().find(|m| &m.id == id).is_some_and(|m| m.steps.iter().any(|s| matches!(s, crate::macros::Step::Keys(k) if k.as_slice() == [vk]))),
        Bind::App(_) => false,
    }
}

/// `binds` without the keys that would press themselves ([`presses_own_key`]; `vk_of` = the key's virtual key on the user's
/// layout) - those stay as Windows made them. Returns the kept binds and the keys that were dropped.
pub fn without_self_loops(binds: &Binds, macros: &[crate::macros::Macro], vk_of: &dyn Fn(Code) -> Option<u16>) -> (Binds, Vec<Code>) {
    let mut kept = Binds::new();
    let mut dropped = Vec::new();
    for (c, b) in binds.iter() {
        if vk_of(c).is_some_and(|vk| presses_own_key(b, macros, vk)) {
            dropped.push(c);
        } else {
            let _ = kept.set(c, b.clone());
        }
    }
    (kept, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_round_trip_through_their_lines() {
        let mut b = Binds::new();
        b.set(0x44, Bind::Preset(Preset::PlayPause)).unwrap();
        b.set(0xE038, Bind::Preset(Preset::OpenWeb("https://example.com/a?b=1|2".into()))).unwrap();
        b.set(0x57, Bind::Macro("m1".into())).unwrap();
        b.set(0x58, Bind::App("micmute.toggle".into())).unwrap();
        b.set(0x3B, Bind::Preset(Preset::OpenApp(r"C:\Program Files\App\app.exe".into()))).unwrap();
        let lines = b.to_lines();
        assert_eq!(Binds::from_lines(&lines), b);
        assert!(lines.contains(&"44|p|media.playpause|".to_string()));
        assert!(lines.contains(&"57|m|m1".to_string()));
    }

    #[test]
    fn bad_lines_are_dropped_not_fatal() {
        let lines: Vec<String> = ["", "zz|p|vol.up|", "0|p|vol.up|", "44|p|nonsense|", "44|x|a", "44|p", "45|p|open.web|javascript:alert(1)", "46|p|vol.up|", "47|m|"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let b = Binds::from_lines(&lines);
        assert_eq!(b.len(), 1, "only 46 is good: {:?}", b);
        assert_eq!(b.get(0x46), Some(&Bind::Preset(Preset::VolumeUp)));
    }

    #[test]
    fn websites_are_http_only_and_targets_are_checked() {
        assert!(is_web("https://boyler.example/x"));
        assert!(is_web("HTTP://a.b"));
        assert!(!is_web("javascript:alert(1)"));
        assert!(!is_web("file:///C:/x"));
        assert!(!is_web("https://"));
        assert!(!is_web("https://a b"));
        assert!(Preset::OpenWeb("ftp://x".into()).check().is_err());
        assert!(Preset::OpenApp("a\"b".into()).check().is_err());
        assert!(Preset::OpenApp("x".repeat(MAX_TARGET + 1)).check().is_err());
        assert!(Preset::OpenFolder("C:\\Users".into()).ready());
        assert!(!Preset::OpenFolder(String::new()).ready(), "a target is needed to run");
        assert!(Preset::OpenFolder(String::new()).check().is_ok(), "but an empty one can be edited");
        assert!(Preset::VolumeUp.ready());
    }

    #[test]
    fn only_the_three_open_presets_open_something() {
        for p in Preset::SIMPLE {
            assert!(!p.opens_something(), "{:?}", p);
        }
        assert!(Preset::OpenApp("a".into()).opens_something());
        assert!(Preset::OpenWeb("https://a".into()).opens_something());
    }

    #[test]
    fn a_macro_can_be_dropped_from_every_key_and_the_count_is_capped() {
        let mut b = Binds::new();
        b.set(0x44, Bind::Macro("m1".into())).unwrap();
        b.set(0x45, Bind::Macro("m1".into())).unwrap();
        b.set(0x46, Bind::Macro("m2".into())).unwrap();
        assert_eq!(b.keys_of_macro("m1"), vec![0x44, 0x45]);
        b.drop_macro("m1");
        assert_eq!(b.len(), 1);
        let mut full = Binds::new();
        for c in 1..=MAX_BINDS as u16 {
            full.set(c, Bind::Preset(Preset::VolumeUp)).unwrap();
        }
        assert!(full.set(0x200, Bind::Preset(Preset::VolumeUp)).is_err());
        assert!(full.set(1, Bind::Preset(Preset::VolumeDown)).is_ok(), "changing a key's own is fine");
    }

    #[test]
    fn preset_ids_round_trip() {
        for p in Preset::SIMPLE {
            assert_eq!(Preset::from_parts(p.id(), ""), Some(p.clone()));
        }
        assert_eq!(Preset::from_parts("open.folder", " C:\\x "), Some(Preset::OpenFolder("C:\\x".into())));
        assert_eq!(Preset::from_parts("nope", ""), None);
        assert_eq!(Preset::VolumeUp.group(), "Volume");
        assert_eq!(Preset::NextTrack.group(), "Media");
    }

    /// Order 059: every ready-made action has its own id and name, groups come in one block each, the shortcut ones carry a
    /// combo (modifiers first, then one key), the others (media / volume / lock / output / open) don't.
    #[test]
    fn the_ready_made_actions_are_complete_and_tidy() {
        let ids: std::collections::HashSet<&str> = Preset::SIMPLE.iter().map(|p| p.id()).collect();
        assert_eq!(ids.len(), Preset::SIMPLE.len(), "ids are unique");
        let names: std::collections::HashSet<&str> = Preset::SIMPLE.iter().map(|p| p.name()).collect();
        assert_eq!(names.len(), Preset::SIMPLE.len());
        // each group is one run in the list
        let mut seen: Vec<&str> = Vec::new();
        for p in Preset::SIMPLE.iter() {
            if seen.last() != Some(&p.group()) {
                assert!(!seen.contains(&p.group()), "{} is split", p.group());
                seen.push(p.group());
            }
        }
        assert_eq!(seen, vec!["Edit", "Windows", "Browser", "Media", "Volume"]);
        for p in Preset::SIMPLE.iter() {
            if let Some(c) = p.combo() {
                let (last, mods) = c.split_last().unwrap();
                assert!(mods.iter().all(|m| [0x11, 0x12, 0x10, 0x5B].contains(m)), "{:?}: modifiers first", p);
                assert!(![0x11, 0x12, 0x10, 0x5B].contains(last), "{:?}: ends in a real key", p);
            }
            assert_eq!(Preset::from_parts(p.id(), ""), Some(p.clone()));
        }
        assert_eq!(Preset::Copy.combo(), Some(&[0x11u16, 0x43][..]));
        assert_eq!(Preset::AltTab.combo(), Some(&[0x12u16, 0x09][..]));
        assert_eq!(Preset::TaskManager.combo(), Some(&[0x11u16, 0x10, 0x1B][..]));
        for p in [Preset::PlayPause, Preset::VolumeMute, Preset::LockPc, Preset::NextOutput, Preset::OpenWeb("https://a.b".into())] {
            assert_eq!(p.combo(), None, "{:?}", p);
        }
        assert!(Preset::LockPc.ready() && Preset::NextOutput.ready());
        // a few of the ones the owner named
        for want in ["Copy", "Paste", "Cut", "Undo", "Redo", "Select all", "Task Manager", "Lock the PC", "Show the desktop", "Emoji panel", "Back", "Forward", "Refresh", "New tab", "Close tab", "Reopen closed tab", "Switch audio output"] {
            assert!(names.contains(want), "{want}");
        }
    }

    /// Order 059 (the owner, v1.0.3: "i put 'j' in macros ... pressing 'j' doesnt work anymore at all, its like an infinite loop"): a
    /// key whose action presses that same key bare (J -> macro "press J", F5 -> "Refresh" = F5) is left as Windows made it;
    /// a macro that TYPES the letter, or presses it with a modifier, or another key, is fine.
    #[test]
    fn a_key_that_would_press_itself_is_left_alone() {
        use crate::macros::{Macro, Step};
        const J: u16 = 0x4A;
        let mut press_j = Macro::new("m1", "Press J");
        press_j.steps = vec![Step::Keys(vec![J])];
        let mut type_j = Macro::new("m2", "Type j");
        type_j.steps = vec![Step::Type("j".into())];
        let mut ctrl_j = Macro::new("m3", "Ctrl J");
        ctrl_j.steps = vec![Step::Keys(vec![0x11, J])];
        let mut other = Macro::new("m4", "Other");
        other.steps = vec![Step::Wait(100), Step::Keys(vec![0x4B])];
        let macros = vec![press_j, type_j, ctrl_j, other];
        assert!(presses_own_key(&Bind::Macro("m1".into()), &macros, J), "J -> a macro that presses J loops");
        for ok in ["m2", "m3", "m4"] {
            assert!(!presses_own_key(&Bind::Macro(ok.into()), &macros, J), "{ok} can run on J");
        }
        assert!(!presses_own_key(&Bind::Macro("gone".into()), &macros, J));
        // a preset whose shortcut is the key itself (F5 = Refresh on the F5 key, Print Screen on its own key)
        assert!(presses_own_key(&Bind::Preset(Preset::BrowserRefresh), &macros, 0x74));
        assert!(presses_own_key(&Bind::Preset(Preset::PrintScreen), &macros, 0x2C));
        assert!(!presses_own_key(&Bind::Preset(Preset::BrowserRefresh), &macros, J));
        assert!(!presses_own_key(&Bind::Preset(Preset::Copy), &macros, 0x43), "Ctrl + C is not the bare C key");
        assert!(!presses_own_key(&Bind::Preset(Preset::VolumeMute), &macros, 0xAD), "media keys go to Windows, not through the hotkey");
        assert!(!presses_own_key(&Bind::App("micmute.toggle".into()), &macros, J));
        // the whole set: the loops are dropped, the rest kept
        let mut b = Binds::new();
        b.set(0x24, Bind::Macro("m1".into())).unwrap(); // the J key (scancode 0x24) -> macro pressing J
        b.set(0x25, Bind::Macro("m2".into())).unwrap(); // the K key -> a macro that types
        b.set(0x3F, Bind::Preset(Preset::BrowserRefresh)).unwrap(); // F5
        let vk = |c: Code| match c {
            0x24 => Some(J),
            0x25 => Some(0x4B),
            0x3F => Some(0x74),
            _ => None,
        };
        let (kept, dropped) = without_self_loops(&b, &macros, &vk);
        assert_eq!(dropped, vec![0x24, 0x3F]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept.get(0x25), Some(&Bind::Macro("m2".into())));
    }
}

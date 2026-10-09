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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preset {
    PlayPause,
    NextTrack,
    PrevTrack,
    StopMedia,
    VolumeUp,
    VolumeDown,
    VolumeMute,
    /// An app or file to open (its full path).
    OpenApp(String),
    OpenFolder(String),
    /// A web address (http / https only).
    OpenWeb(String),
}

impl Preset {
    /// The presets that need nothing more, in the page's order.
    pub const SIMPLE: [Preset; 7] =
        [Preset::PlayPause, Preset::NextTrack, Preset::PrevTrack, Preset::StopMedia, Preset::VolumeUp, Preset::VolumeDown, Preset::VolumeMute];

    /// Stable id (saved in the settings file).
    pub fn id(&self) -> &'static str {
        match self {
            Preset::PlayPause => "media.playpause",
            Preset::NextTrack => "media.next",
            Preset::PrevTrack => "media.prev",
            Preset::StopMedia => "media.stop",
            Preset::VolumeUp => "vol.up",
            Preset::VolumeDown => "vol.down",
            Preset::VolumeMute => "vol.mute",
            Preset::OpenApp(_) => "open.app",
            Preset::OpenFolder(_) => "open.folder",
            Preset::OpenWeb(_) => "open.web",
        }
    }

    /// What the user sees.
    pub fn name(&self) -> &'static str {
        match self {
            Preset::PlayPause => "Play / pause",
            Preset::NextTrack => "Next track",
            Preset::PrevTrack => "Previous track",
            Preset::StopMedia => "Stop",
            Preset::VolumeUp => "Volume up",
            Preset::VolumeDown => "Volume down",
            Preset::VolumeMute => "Mute",
            Preset::OpenApp(_) => "An app or file…",
            Preset::OpenFolder(_) => "A folder…",
            Preset::OpenWeb(_) => "A website…",
        }
    }

    /// The group the page's list puts it in.
    pub fn group(&self) -> &'static str {
        match self {
            Preset::PlayPause | Preset::NextTrack | Preset::PrevTrack | Preset::StopMedia => "Media",
            Preset::VolumeUp | Preset::VolumeDown | Preset::VolumeMute => "Volume",
            _ => "Open",
        }
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
            "media.playpause" => Preset::PlayPause,
            "media.next" => Preset::NextTrack,
            "media.prev" => Preset::PrevTrack,
            "media.stop" => Preset::StopMedia,
            "vol.up" => Preset::VolumeUp,
            "vol.down" => Preset::VolumeDown,
            "vol.mute" => Preset::VolumeMute,
            "open.app" => Preset::OpenApp(target.trim().to_string()),
            "open.folder" => Preset::OpenFolder(target.trim().to_string()),
            "open.web" => Preset::OpenWeb(target.trim().to_string()),
            _ => return None,
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
}

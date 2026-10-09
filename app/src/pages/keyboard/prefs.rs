//! What the Keyboard tab remembers (the settings store, page scope `keyboard`). The pressed keys are never part of it.
//! `on` is OFF by default (while off nothing listens); the pack is Linear, the volume 5 % (the owner's sound rule), "off while a
//! game is in front" on.

use crate::settings::{Scope, SettingsStore};
use bu_keysound::binds::Binds;
use bu_keysound::macros::{self, Macro};
use bu_keysound::{Pack, Rule, Settings, DEFAULT_VOLUME};

pub const PAGE: &str = "keyboard";
const K_ON: &str = "on";
const K_PACK: &str = "pack";
const K_VOLUME: &str = "volume";
const K_GAME: &str = "off_in_game";
const K_REPEAT: &str = "repeat_ms";
const K_MOUSE: &str = "mouse_on";
const K_MOUSE_VOL: &str = "mouse_volume";
const K_RULES: &str = "rules";
const K_BINDS: &str = "binds";
const K_MACROS: &str = "macros";
const K_SIZE: &str = "size";
/// The most per-app rules kept (the page's list is short; the file never grows).
pub const MAX_RULES: usize = 32;

#[derive(Debug, Clone, PartialEq)]
pub struct Prefs {
    /// The key-sound switch.
    pub on: bool,
    pub s: Settings,
    /// Keys with a preset action (media, volume, open …) or a macro. A remap is NOT here: Windows holds it.
    pub binds: Binds,
    pub macros: Vec<Macro>,
    /// The keyboard picture's size.
    pub size: Size,
}

/// How much of a keyboard the picture shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Size {
    #[default]
    Full,
    Tkl,
    P75,
    P60,
}

impl Size {
    pub const ALL: [Size; 4] = [Size::Full, Size::Tkl, Size::P75, Size::P60];
    pub fn key(self) -> &'static str {
        match self {
            Size::Full => "full",
            Size::Tkl => "tkl",
            Size::P75 => "75",
            Size::P60 => "60",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Size::Full => "Full",
            Size::Tkl => "TKL",
            Size::P75 => "75 %",
            Size::P60 => "60 %",
        }
    }
    pub fn from_key(s: &str) -> Size {
        Size::ALL.into_iter().find(|z| z.key() == s).unwrap_or_default()
    }
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { on: false, s: Settings::default(), binds: Binds::new(), macros: Vec::new(), size: Size::Full }
    }
}

fn scope() -> Scope<'static> {
    Scope::Page(PAGE)
}

impl Prefs {
    pub fn load(store: &SettingsStore) -> Prefs {
        let d = Settings::default();
        let pack = store.get_str(scope(), K_PACK).and_then(Pack::from_key).unwrap_or(d.pack);
        let rules = store
            .get_list(scope(), K_RULES)
            .map(|l| l.iter().filter_map(|e| parse_rule(e)).take(MAX_RULES).collect())
            .unwrap_or_default();
        Prefs {
            on: store.bool_or(scope(), K_ON, false),
            s: Settings {
                pack,
                volume: store.i64_or(scope(), K_VOLUME, i64::from(DEFAULT_VOLUME)).clamp(0, 100) as u8,
                off_in_game: store.bool_or(scope(), K_GAME, false),
                repeat_ms: store.i64_or(scope(), K_REPEAT, 0).clamp(0, 80) as u8,
                mouse_on: store.bool_or(scope(), K_MOUSE, false),
                mouse_volume: store.i64_or(scope(), K_MOUSE_VOL, i64::from(DEFAULT_VOLUME)).clamp(0, 100) as u8,
                rules,
            },
            binds: Binds::from_lines(store.get_list(scope(), K_BINDS).unwrap_or_default()),
            macros: macros::from_lines(store.get_list(scope(), K_MACROS).unwrap_or_default()),
            size: Size::from_key(store.str_or(scope(), K_SIZE, "full")),
        }
    }

    pub fn save(&self, store: &mut SettingsStore) {
        let _ = store.set_bool(scope(), K_ON, self.on);
        let _ = store.set_str(scope(), K_PACK, &self.s.pack.to_key());
        let _ = store.set_i64(scope(), K_VOLUME, i64::from(self.s.volume));
        let _ = store.set_bool(scope(), K_GAME, self.s.off_in_game);
        let _ = store.set_i64(scope(), K_REPEAT, i64::from(self.s.repeat_ms));
        let _ = store.set_bool(scope(), K_MOUSE, self.s.mouse_on);
        let _ = store.set_i64(scope(), K_MOUSE_VOL, i64::from(self.s.mouse_volume));
        let rules: Vec<String> = self.s.rules.iter().take(MAX_RULES).map(rule_text).collect();
        let _ = store.set_list(scope(), K_RULES, &rules);
        let _ = store.set_list(scope(), K_BINDS, &self.binds.to_lines());
        let _ = store.set_list(scope(), K_MACROS, &macros::to_lines(&self.macros));
        let _ = store.set_str(scope(), K_SIZE, self.size.key());
    }
}

/// `exe|pack` (`discord.exe|off`, `notepad.exe|typewriter`).
fn rule_text(r: &Rule) -> String {
    format!("{}|{}", r.exe, r.pack.as_ref().map(Pack::to_key).unwrap_or_else(|| "off".into()))
}

fn parse_rule(e: &str) -> Option<Rule> {
    let (exe, pack) = e.split_once('|')?;
    let exe = exe.trim().to_ascii_lowercase();
    if exe.is_empty() || exe.contains(['\\', '/']) {
        return None;
    }
    let pack = if pack == "off" { None } else { Some(Pack::from_key(pack)?) };
    Some(Rule { exe, pack })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::scratch::Scratch;
    use bu_keysound::binds::{Bind, Preset};
    use bu_keysound::macros::Step;
    use bu_keysound::PackId;

    #[test]
    fn a_fresh_install_is_off_quiet_and_game_safe() {
        let sc = Scratch::new("kb-fresh");
        let store = SettingsStore::open(sc.dir());
        let p = Prefs::load(&store);
        assert!(!p.on, "off by default: nothing listens");
        assert_eq!(p.s.volume, 5);
        assert!(!p.s.off_in_game, "off by default: sounds play in games");
        assert_eq!(p.s.repeat_ms, 0);
        assert!(!p.s.mouse_on, "Order 064: mouse clicks are off by default");
        assert_eq!(p.s.mouse_volume, 5, "and quiet");
        assert_eq!(p.s.pack, Pack::Builtin(PackId::Linear));
        assert!(p.s.rules.is_empty());
        assert_eq!(p, Prefs::default());
    }

    #[test]
    fn prefs_round_trip_and_hold_no_keys() {
        let sc = Scratch::new("kb-round");
        let mut store = SettingsStore::open(sc.dir());
        let mut binds = Binds::new();
        binds.set(0x44, Bind::Preset(Preset::PlayPause)).unwrap();
        binds.set(0x57, Bind::Macro("m1".into())).unwrap();
        let mut m = Macro::new("m1", "Open my notes");
        m.steps = vec![Step::Keys(vec![0x5B, 0x52]), Step::Wait(300), Step::Type("notepad".into())];
        let p = Prefs {
            on: true,
            binds,
            macros: vec![m],
            size: Size::Tkl,
            s: Settings {
                pack: Pack::Imported("Holy Panda".into()),
                volume: 12,
                off_in_game: true,
                repeat_ms: 35,
                mouse_on: true,
                mouse_volume: 9,
                rules: vec![Rule { exe: "discord.exe".into(), pack: None }, Rule { exe: "notepad.exe".into(), pack: Some(Pack::Builtin(PackId::Typewriter)) }],
            },
        };
        p.save(&mut store);
        assert_eq!(Prefs::load(&store), p);
        drop(store);
        let again = SettingsStore::open(sc.dir());
        assert_eq!(Prefs::load(&again), p, "read back from the file");
        // what the file holds: switches, a pack name, a volume, program names, the keys that carry an action / macro (by their
        // place on the keyboard) and the macros the user built - never what was typed
        let text = std::fs::read_to_string(again.path()).unwrap();
        for line in text.lines().filter(|l| l.starts_with("page:keyboard")) {
            let key = line.split('\t').nth(1).unwrap();
            assert!(["on", "pack", "volume", "off_in_game", "repeat_ms", "mouse_on", "mouse_volume", "rules", "binds", "macros", "size"].contains(&key), "unexpected setting {key}");
        }
    }

    #[test]
    fn broken_values_fall_back_to_the_defaults() {
        let sc = Scratch::new("kb-broken");
        let mut store = SettingsStore::open(sc.dir());
        let _ = store.set_str(scope(), K_PACK, "no-such-pack");
        let _ = store.set_i64(scope(), K_VOLUME, 900);
        let _ = store.set_list(scope(), K_RULES, &["noseparator".to_string(), "|off".into(), r"C:\x.exe|off".into(), "a.exe|nope".into(), "Ok.EXE|glass-tap".into()]);
        let p = Prefs::load(&store);
        assert_eq!(p.s.pack, Pack::Builtin(PackId::Linear));
        assert_eq!(p.s.volume, 100, "clamped");
        assert_eq!(p.s.rules, vec![Rule { exe: "ok.exe".into(), pack: Some(Pack::Builtin(PackId::GlassTap)) }], "only the one good rule, lower case");
    }
}

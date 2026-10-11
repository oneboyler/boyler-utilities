//! What the Keyboard tab remembers (the settings store, page scope `keyboard`). The pressed keys are never part of it.
//! `on` is OFF by default (while off nothing listens); the pack is Linear, the volume 5 % (the owner's sound rule), "off while a
//! game is in front" on.

use crate::settings::{Scope, SettingsStore};
use bu_keysound::binds::Binds;
use bu_keysound::macros::{self, Macro};
use bu_keysound::{click_from_key, click_key, Layers, Pack, PlayOn, Rule, Settings, DEFAULT_VOLUME, MAX_REPEAT_MS};

pub const PAGE: &str = "keyboard";
const K_ON: &str = "on";
const K_PACK: &str = "pack";
const K_VOLUME: &str = "volume";
const K_GAME: &str = "off_in_game";
const K_REPEAT: &str = "repeat_ms";
const K_MOUSE: &str = "mouse_on";
const K_MOUSE_VOL: &str = "mouse_volume";
const K_PAD: &str = "pad_on";
const K_PLAY_ON: &str = "play_on";
const K_RULES: &str = "rules";
const K_BINDS: &str = "binds";
const K_MACROS: &str = "macros";
const K_SIZE: &str = "size";
const K_SIZE_SET: &str = "size_set";
// Order 090: each button's two sound layers, the mouse buttons' jobs, the mouse's click and the controller's own sound + volume
const K_KEY_LAYERS: &str = "key_layers";
const K_MOUSE_LAYERS: &str = "mouse_layers";
const K_PAD_LAYERS: &str = "pad_layers";
const K_MOUSE_BINDS: &str = "mouse_binds";
const K_MOUSE_CLICK: &str = "mouse_click";
const K_PAD_VOL: &str = "pad_volume";
const K_PAD_PACK: &str = "pad_pack";
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
    /// Order 096: the size was picked by hand (a known keyboard then no longer picks it by itself).
    pub size_manual: bool,
    /// Order 090: the sound layers of the keys (by scan code), the mouse buttons and the controller buttons.
    pub keys: Layers,
    pub mouse: Layers,
    pub pad: Layers,
    /// What the mouse's wheel click / side buttons do besides their own job (by button number), Order 090.
    pub mouse_binds: Binds,
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
        Prefs {
            on: false,
            s: Settings { keys_on: false, ..Settings::default() },
            binds: Binds::new(),
            macros: Vec::new(),
            size: Size::Full,
            size_manual: false,
            keys: Layers::new(),
            mouse: Layers::new(),
            pad: Layers::new(),
            mouse_binds: Binds::new(),
        }
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
        let on = store.bool_or(scope(), K_ON, false);
        let pad_pack = match store.str_or(scope(), K_PAD_PACK, "same") {
            "same" => None,
            k => Pack::from_key(k),
        };
        Prefs {
            on,
            s: Settings {
                keys_on: on,
                pack,
                volume: store.i64_or(scope(), K_VOLUME, i64::from(DEFAULT_VOLUME)).clamp(0, 100) as u8,
                off_in_game: store.bool_or(scope(), K_GAME, false),
                repeat_ms: store.i64_or(scope(), K_REPEAT, 0).clamp(0, i64::from(MAX_REPEAT_MS)) as u16,
                mouse_on: store.bool_or(scope(), K_MOUSE, false),
                mouse_volume: store.i64_or(scope(), K_MOUSE_VOL, i64::from(DEFAULT_VOLUME)).clamp(0, 100) as u8,
                mouse_click: click_from_key(store.str_or(scope(), K_MOUSE_CLICK, "auto")),
                pad_on: store.bool_or(scope(), K_PAD, false),
                pad_volume: store.i64_or(scope(), K_PAD_VOL, i64::from(DEFAULT_VOLUME)).clamp(0, 100) as u8,
                pad_pack,
                play_on: PlayOn::from_key(store.str_or(scope(), K_PLAY_ON, "both")),
                rules,
            },
            binds: Binds::from_lines(store.get_list(scope(), K_BINDS).unwrap_or_default()),
            macros: macros::from_lines(store.get_list(scope(), K_MACROS).unwrap_or_default()),
            size: Size::from_key(store.str_or(scope(), K_SIZE, "full")),
            // a size saved before this flag existed that is not the default was picked by hand
            size_manual: store.get_bool(scope(), K_SIZE_SET).unwrap_or_else(|| Size::from_key(store.str_or(scope(), K_SIZE, "full")) != Size::Full),
            keys: Layers::from_lines(store.get_list(scope(), K_KEY_LAYERS).unwrap_or_default()),
            mouse: Layers::from_lines(store.get_list(scope(), K_MOUSE_LAYERS).unwrap_or_default()),
            pad: Layers::from_lines(store.get_list(scope(), K_PAD_LAYERS).unwrap_or_default()),
            mouse_binds: Binds::from_lines(store.get_list(scope(), K_MOUSE_BINDS).unwrap_or_default()),
        }
    }

    /// The engine's settings: the Keyboard sounds switch is `on`.
    pub fn engine_settings(&self) -> Settings {
        Settings { keys_on: self.on, ..self.s.clone() }
    }

    /// Every "your sound" file the layers use.
    pub fn own_files(&self) -> Vec<String> {
        let mut v = self.keys.files();
        v.extend(self.mouse.files());
        v.extend(self.pad.files());
        v.sort();
        v.dedup();
        v
    }

    pub fn save(&self, store: &mut SettingsStore) {
        let _ = store.set_bool(scope(), K_ON, self.on);
        let _ = store.set_str(scope(), K_PACK, &self.s.pack.to_key());
        let _ = store.set_i64(scope(), K_VOLUME, i64::from(self.s.volume));
        let _ = store.set_bool(scope(), K_GAME, self.s.off_in_game);
        let _ = store.set_i64(scope(), K_REPEAT, i64::from(self.s.repeat_ms));
        let _ = store.set_bool(scope(), K_MOUSE, self.s.mouse_on);
        let _ = store.set_i64(scope(), K_MOUSE_VOL, i64::from(self.s.mouse_volume));
        let _ = store.set_bool(scope(), K_PAD, self.s.pad_on);
        let _ = store.set_str(scope(), K_PLAY_ON, self.s.play_on.key());
        let rules: Vec<String> = self.s.rules.iter().take(MAX_RULES).map(rule_text).collect();
        let _ = store.set_list(scope(), K_RULES, &rules);
        let _ = store.set_list(scope(), K_BINDS, &self.binds.to_lines());
        let _ = store.set_list(scope(), K_MACROS, &macros::to_lines(&self.macros));
        let _ = store.set_str(scope(), K_SIZE, self.size.key());
        let _ = store.set_bool(scope(), K_SIZE_SET, self.size_manual);
        let _ = store.set_list(scope(), K_KEY_LAYERS, &self.keys.to_lines());
        let _ = store.set_list(scope(), K_MOUSE_LAYERS, &self.mouse.to_lines());
        let _ = store.set_list(scope(), K_PAD_LAYERS, &self.pad.to_lines());
        let _ = store.set_list(scope(), K_MOUSE_BINDS, &self.mouse_binds.to_lines());
        let _ = store.set_str(scope(), K_MOUSE_CLICK, self.s.mouse_click.and_then(click_key).unwrap_or("auto"));
        let _ = store.set_i64(scope(), K_PAD_VOL, i64::from(self.s.pad_volume));
        let _ = store.set_str(scope(), K_PAD_PACK, &self.s.pad_pack.as_ref().map(Pack::to_key).unwrap_or_else(|| "same".into()));
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
        assert!(!p.s.pad_on, "Order 081: the controller sounds are off by default");
        assert_eq!(p.s.play_on, PlayOn::Both, "Order 076: press + release is the default");
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
        let mut keys = Layers::new();
        keys.set(0x1C, bu_keysound::Layer { press: Some("boing.wav".into()), pitch: -4.0, loud: 120, ..Default::default() }).unwrap();
        let mut mouse = Layers::new();
        mouse.set(4, bu_keysound::Layer { pack_on: false, ..Default::default() }).unwrap();
        let mut mouse_binds = Binds::new();
        mouse_binds.set(4, Bind::Preset(Preset::PlayPause)).unwrap();
        let p = Prefs {
            on: true,
            binds,
            macros: vec![m],
            size: Size::Tkl,
            size_manual: true,
            keys,
            mouse,
            pad: Layers::new(),
            mouse_binds,
            s: Settings {
                keys_on: true,
                pack: Pack::Imported("Holy Panda".into()),
                volume: 12,
                off_in_game: true,
                repeat_ms: 350,
                mouse_on: true,
                mouse_volume: 9,
                mouse_click: Some(bu_keysound::ClickStyle::Deep),
                pad_on: true,
                pad_volume: 33,
                pad_pack: Some(Pack::Made("My thock".into())),
                play_on: PlayOn::Release,
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
            assert!(
                [
                    "on", "pack", "volume", "off_in_game", "repeat_ms", "mouse_on", "mouse_volume", "pad_on", "play_on", "rules", "binds", "macros", "size", "size_set", "key_layers",
                    "mouse_layers", "pad_layers", "mouse_binds", "mouse_click", "pad_volume", "pad_pack"
                ]
                .contains(&key),
                "unexpected setting {key}"
            );
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
        let _ = store.set_i64(scope(), K_REPEAT, 9000);
        assert_eq!(Prefs::load(&store).s.repeat_ms, 400, "Order 090: up to 400 ms");
    }
}

//! What plays, and how loud: the settings and the decision for one key press (pure, no Windows).

use crate::synth::PackId;

/// The volume a fresh install starts at (the owner, Oct 8: "like 5%" — the app's sound rule).
pub const DEFAULT_VOLUME: u8 = 5;
/// Loudest sample at volume 100, as a share of full scale (the same curve as the mute sounds, Order 046).
const PEAK_AT_FULL: f32 = 0.5;
/// Volume 1 % is this many dB below volume 100 % (0 % is silence).
const RANGE_DB: f32 = 40.0;

/// The sample multiplier for a volume 0–100 (the mute sounds' perceptual curve): 0 → silence, otherwise `PEAK_AT_FULL` at
/// 100 % falling by 40 dB over the slider.
pub fn gain(volume: u8) -> f32 {
    let v = f32::from(volume.min(100));
    if v <= 0.0 {
        return 0.0;
    }
    PEAK_AT_FULL * 10f32.powf(-RANGE_DB * (1.0 - v / 100.0) / 20.0)
}

/// "Play on" (Order 076): which half of a press makes the sound - keys and mouse buttons alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayOn {
    /// A sound when the key / button goes down and another when it comes up (the default).
    #[default]
    Both,
    /// Only when it goes down.
    Press,
    /// Only when it comes up.
    Release,
}

impl PlayOn {
    pub const ALL: [PlayOn; 3] = [PlayOn::Both, PlayOn::Press, PlayOn::Release];

    /// The text kept in the settings file.
    pub fn key(self) -> &'static str {
        match self {
            PlayOn::Both => "both",
            PlayOn::Press => "press",
            PlayOn::Release => "release",
        }
    }

    pub fn from_key(s: &str) -> PlayOn {
        PlayOn::ALL.into_iter().find(|p| p.key() == s).unwrap_or_default()
    }

    pub fn label(self) -> &'static str {
        match self {
            PlayOn::Both => "Press + release",
            PlayOn::Press => "Press only",
            PlayOn::Release => "Release only",
        }
    }

    /// Does a key / button going `down` (true) or up (false) make a sound?
    pub fn plays(self, down: bool) -> bool {
        match self {
            PlayOn::Both => true,
            PlayOn::Press => down,
            PlayOn::Release => !down,
        }
    }
}

/// A sound pack: one of ours, or one the user imported (its folder name under the app's pack folder).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Pack {
    Builtin(PackId),
    Imported(String),
}

impl Pack {
    /// The text kept in the settings file: `linear`, `glass-tap`, … or `imported:<name>`.
    pub fn to_key(&self) -> String {
        match self {
            Pack::Builtin(p) => p.key().to_string(),
            Pack::Imported(n) => format!("imported:{n}"),
        }
    }

    pub fn from_key(s: &str) -> Option<Pack> {
        if let Some(n) = s.strip_prefix("imported:") {
            return (!n.is_empty()).then(|| Pack::Imported(n.to_string()));
        }
        PackId::from_key(s).map(Pack::Builtin)
    }
}

/// "In this app, play THIS pack" (or none at all). `exe` is the program's file name, lower case (`discord.exe`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub exe: String,
    /// None = Off in that app.
    pub pack: Option<Pack>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub pack: Pack,
    pub volume: u8,
    /// Silent while a full-screen app / game is in front.
    pub off_in_game: bool,
    /// "Ignore repeats within __ ms" (0-80; 0 = off): a key that comes down twice inside the window plays one sound.
    pub repeat_ms: u8,
    pub rules: Vec<Rule>,
    /// "Mouse clicks too" (Order 064, off by default): the mouse buttons make sounds as well. The side buttons play the chosen
    /// pack's key sound, the others a click of our own made to suit that pack. Needs the key sounds to be on.
    pub mouse_on: bool,
    /// The mouse sounds' own volume (0-100, default [`DEFAULT_VOLUME`]).
    pub mouse_volume: u8,
    /// "Play on" (Order 076): press + release (default), press only or release only - for the keys and the mouse.
    pub play_on: PlayOn,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            pack: Pack::Builtin(PackId::Linear),
            volume: DEFAULT_VOLUME,
            off_in_game: false,
            repeat_ms: 0,
            rules: Vec::new(),
            mouse_on: false,
            mouse_volume: DEFAULT_VOLUME,
            play_on: PlayOn::Both,
        }
    }
}

/// The pack in charge while `exe` is in front: a game in front is silent while "off while a game is in front" is on, whatever the
/// per-app rules say; otherwise the app's own rule wins over the general pack. None = silent.
fn pick<'a>(s: &'a Settings, exe: &str, game_in_front: bool) -> Option<&'a Pack> {
    if game_in_front && s.off_in_game {
        return None;
    }
    match s.rules.iter().find(|r| r.exe.eq_ignore_ascii_case(exe)) {
        Some(r) => r.pack.as_ref(),
        None => Some(&s.pack),
    }
}

/// The pack for a press while `exe` is in front and `game_in_front` says whether a full-screen app / game is: None = silent.
/// A game in front is silent while "off while a game is in front" is on, whatever the per-app rules say; otherwise the app's
/// own rule wins over the general pack; volume 0 is silent.
pub fn choose<'a>(s: &'a Settings, exe: &str, game_in_front: bool) -> Option<&'a Pack> {
    if s.volume == 0 {
        return None;
    }
    pick(s, exe, game_in_front)
}

/// The same for a mouse button (Order 064): the mouse sounds follow the same game switch and per-app rules as the keys (so
/// they use the pack the keys would use there), but have their own switch and volume. None = silent.
pub fn choose_mouse<'a>(s: &'a Settings, exe: &str, game_in_front: bool) -> Option<&'a Pack> {
    if !s.mouse_on || s.mouse_volume == 0 {
        return None;
    }
    pick(s, exe, game_in_front)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(exe: &str, pack: Option<Pack>) -> Rule {
        Rule { exe: exe.into(), pack }
    }

    #[test]
    fn defaults_are_quiet_and_play_in_games() {
        let s = Settings::default();
        assert_eq!(s.volume, 5);
        assert!(!s.off_in_game, "Order 059: sounds play in games unless the user turns it on");
        assert_eq!(s.repeat_ms, 0, "the chatter filter is off");
        assert_eq!(choose(&s, "notepad.exe", false), Some(&Pack::Builtin(PackId::Linear)));
    }

    #[test]
    fn a_game_in_front_is_silent_while_the_switch_is_on() {
        let mut s = Settings::default();
        assert!(choose(&s, "valorant-win64-shipping.exe", true).is_some(), "off by default: sounds play in games");
        s.off_in_game = true;
        assert_eq!(choose(&s, "valorant-win64-shipping.exe", true), None);
        // a per-app rule can't force sound over a game while the switch is on
        s.rules.push(rule("game.exe", Some(Pack::Builtin(PackId::Clicky))));
        assert_eq!(choose(&s, "game.exe", true), None);
    }

    #[test]
    fn per_app_rules_win_over_the_general_pack() {
        let mut s = Settings::default();
        s.rules.push(rule("discord.exe", None));
        s.rules.push(rule("notepad.exe", Some(Pack::Builtin(PackId::Typewriter))));
        assert_eq!(choose(&s, "Discord.EXE", false), None, "Off in Discord (names compare without case)");
        assert_eq!(choose(&s, "notepad.exe", false), Some(&Pack::Builtin(PackId::Typewriter)));
        assert_eq!(choose(&s, "chrome.exe", false), Some(&Pack::Builtin(PackId::Linear)));
    }

    #[test]
    fn mouse_sounds_are_off_quiet_and_follow_the_games_switch_and_the_app_rules() {
        let mut s = Settings::default();
        assert!(!s.mouse_on, "off by default");
        assert_eq!(s.mouse_volume, 5, "quiet by default");
        assert_eq!(choose_mouse(&s, "a.exe", false), None, "the switch is off");
        s.mouse_on = true;
        assert_eq!(choose_mouse(&s, "a.exe", false), Some(&Pack::Builtin(PackId::Linear)));
        // their own volume: the keys' volume 0 doesn't silence them, theirs does
        s.volume = 0;
        assert!(choose_mouse(&s, "a.exe", false).is_some());
        s.mouse_volume = 0;
        assert_eq!(choose_mouse(&s, "a.exe", false), None);
        s.mouse_volume = 5;
        // the same game switch and per-app rules as the keys
        s.off_in_game = true;
        assert_eq!(choose_mouse(&s, "game.exe", true), None, "a game in front: silent");
        s.off_in_game = false;
        s.rules.push(rule("discord.exe", None));
        s.rules.push(rule("notepad.exe", Some(Pack::Builtin(PackId::Typewriter))));
        assert_eq!(choose_mouse(&s, "discord.exe", false), None);
        assert_eq!(choose_mouse(&s, "notepad.exe", false), Some(&Pack::Builtin(PackId::Typewriter)));
    }

    #[test]
    fn volume_zero_is_silent_and_the_curve_matches_the_mute_sounds() {
        let mut s = Settings::default();
        s.volume = 0;
        assert_eq!(choose(&s, "a.exe", false), None);
        assert_eq!(gain(0), 0.0);
        assert!((gain(100) - 0.5).abs() < 1e-6);
        assert!((gain(5) - 0.5 * 10f32.powf(-40.0 * 0.95 / 20.0)).abs() < 1e-6);
        assert!(gain(50) > gain(5) && gain(100) > gain(50));
    }

    #[test]
    fn play_on_picks_which_half_of_a_press_sounds() {
        assert_eq!(Settings::default().play_on, PlayOn::Both, "press + release is the default");
        assert!(PlayOn::Both.plays(true) && PlayOn::Both.plays(false));
        assert!(PlayOn::Press.plays(true) && !PlayOn::Press.plays(false));
        assert!(!PlayOn::Release.plays(true) && PlayOn::Release.plays(false));
        for p in PlayOn::ALL {
            assert_eq!(PlayOn::from_key(p.key()), p);
        }
        assert_eq!(PlayOn::from_key("nonsense"), PlayOn::Both);
    }

    #[test]
    fn pack_keys_round_trip() {
        for p in PackId::ALL {
            let pack = Pack::Builtin(p);
            assert_eq!(Pack::from_key(&pack.to_key()), Some(pack));
        }
        let imp = Pack::Imported("Holy Panda".into());
        assert_eq!(imp.to_key(), "imported:Holy Panda");
        assert_eq!(Pack::from_key("imported:Holy Panda"), Some(imp));
        assert_eq!(Pack::from_key("imported:"), None);
        assert_eq!(Pack::from_key("nope"), None);
    }
}

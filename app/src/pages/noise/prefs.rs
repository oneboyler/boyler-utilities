//! What the Noise tab remembers (the settings store, page scope `noise`): the sound, the volume, the sleep timer, and (Order 080)
//! your own mix and the sounds you saved from it. NOT whether it plays: the noise is off until Play is pressed, every time
//! (the owner's sound rule).

use crate::settings::{Scope, SettingsStore};
use bu_noise::{Kind, Mix, Sound, DEFAULT_VOLUME, SLEEP_CHOICES};

pub const PAGE: &str = "noise";
const K_KIND: &str = "kind";
const K_VOLUME: &str = "volume";
const K_SLEEP: &str = "sleep";
const K_CUSTOM: &str = "custom";
const K_MINE: &str = "mine";
/// The `kind` word of the Custom entry; a saved sound is `mine:<name>`.
const WORD_CUSTOM: &str = "custom";
const WORD_MINE: &str = "mine:";

/// You can keep this many sounds of your own.
pub const MAX_MINE: usize = 12;
/// The longest name (characters).
pub const MAX_NAME: usize = 24;

/// A sound you saved from the Custom sliders.
#[derive(Debug, Clone, PartialEq)]
pub struct Mine {
    pub name: String,
    pub mix: Mix,
}

/// What is chosen in the list.
#[derive(Debug, Clone, PartialEq)]
pub enum Pick {
    Preset(Kind),
    /// The three sliders.
    Custom,
    /// A saved sound, by its name.
    Mine(String),
}

impl Pick {
    /// The name in the list.
    pub fn name(&self) -> String {
        match self {
            Pick::Preset(k) => k.name().to_string(),
            Pick::Custom => "Custom".to_string(),
            Pick::Mine(n) => n.clone(),
        }
    }

    /// The word the settings file keeps.
    pub fn key(&self) -> String {
        match self {
            Pick::Preset(k) => k.key().to_string(),
            Pick::Custom => WORD_CUSTOM.to_string(),
            Pick::Mine(n) => format!("{WORD_MINE}{n}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Prefs {
    pub pick: Pick,
    /// 0 .. 100 %
    pub volume: u8,
    /// Minutes, one of `SLEEP_CHOICES` (None = off).
    pub sleep: Option<u32>,
    /// What the Custom sliders are set to (kept for next time).
    pub custom: Mix,
    pub mine: Vec<Mine>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { pick: Pick::Preset(Kind::Brown), volume: DEFAULT_VOLUME, sleep: None, custom: Mix::DEFAULT, mine: Vec::new() }
    }
}

fn scope() -> Scope<'static> {
    Scope::Page(PAGE)
}

/// "tone,rumble,waves,name" - the name last, so it may hold commas.
fn mine_line(m: &Mine) -> String {
    format!("{},{},{},{}", m.mix.tone, m.mix.rumble, m.mix.waves, m.name)
}

fn parse_mine(s: &str) -> Option<Mine> {
    let mut it = s.splitn(4, ',');
    let n = |x: Option<&str>| x.and_then(|v| v.trim().parse::<u8>().ok()).filter(|v| *v <= Mix::MAX);
    let (tone, rumble, waves) = (n(it.next())?, n(it.next())?, n(it.next())?);
    let name = clean_name(it.next()?);
    if name.is_empty() {
        return None;
    }
    Some(Mine { name, mix: Mix::new(tone, rumble, waves) })
}

/// A name as it is kept: trimmed, one line, at most `MAX_NAME` characters.
pub fn clean_name(s: &str) -> String {
    let one: String = s.chars().filter(|c| !c.is_control()).collect();
    one.trim().chars().take(MAX_NAME).collect::<String>().trim().to_string()
}

impl Prefs {
    pub fn load(store: &SettingsStore) -> Prefs {
        let d = Prefs::default();
        let minutes = store.i64_or(scope(), K_SLEEP, 0);
        let mut mine: Vec<Mine> = Vec::new();
        for line in store.get_list(scope(), K_MINE).unwrap_or(&[]) {
            if let Some(m) = parse_mine(line) {
                if mine.len() < MAX_MINE && !mine.iter().any(|o| same_name(&o.name, &m.name)) {
                    mine.push(m);
                }
            }
        }
        let custom = store.get_str(scope(), K_CUSTOM).and_then(parse_mine_mix).unwrap_or(d.custom);
        let word = store.get_str(scope(), K_KIND).unwrap_or("");
        let pick = if word == WORD_CUSTOM {
            Pick::Custom
        } else if let Some(name) = word.strip_prefix(WORD_MINE) {
            // a saved sound that is gone (the file was edited): the default
            mine.iter().find(|m| same_name(&m.name, name)).map_or(d.pick.clone(), |m| Pick::Mine(m.name.clone()))
        } else {
            Kind::from_key(word).map_or(d.pick.clone(), Pick::Preset)
        };
        Prefs {
            pick,
            volume: store.i64_or(scope(), K_VOLUME, i64::from(d.volume)).clamp(0, 100) as u8,
            sleep: SLEEP_CHOICES.iter().copied().flatten().find(|m| i64::from(*m) == minutes),
            custom,
            mine,
        }
    }

    pub fn save(&self, store: &mut SettingsStore) {
        let _ = store.set_str(scope(), K_KIND, &self.pick.key());
        let _ = store.set_i64(scope(), K_VOLUME, i64::from(self.volume));
        let _ = store.set_i64(scope(), K_SLEEP, i64::from(self.sleep.unwrap_or(0)));
        let _ = store.set_str(scope(), K_CUSTOM, &format!("{},{},{}", self.custom.tone, self.custom.rumble, self.custom.waves));
        let lines: Vec<String> = self.mine.iter().map(mine_line).collect();
        let _ = store.set_list(scope(), K_MINE, &lines);
    }

    /// What plays for the current pick.
    pub fn sound(&self) -> Sound {
        match &self.pick {
            Pick::Preset(k) => Sound::Preset(*k),
            Pick::Custom => Sound::Mix(self.custom),
            Pick::Mine(n) => self.mine.iter().find(|m| same_name(&m.name, n)).map_or(Sound::Mix(self.custom), |m| Sound::Mix(m.mix)),
        }
    }

    /// The sliders' mix is saved under `name` (made unique: "Rain", "Rain 2", ...), picked at once. None = the list is full or
    /// the name is empty.
    pub fn save_mine(&mut self, name: &str) -> Option<String> {
        let base = clean_name(name);
        if base.is_empty() || self.mine.len() >= MAX_MINE {
            return None;
        }
        let mut name = base.clone();
        let mut n = 2;
        // a saved sound may not be called like a noise, "Custom" or another saved sound
        while self.taken(&name) {
            let suffix = format!(" {n}");
            let keep = MAX_NAME.saturating_sub(suffix.chars().count());
            name = format!("{}{suffix}", base.chars().take(keep).collect::<String>().trim_end());
            n += 1;
        }
        self.mine.push(Mine { name: name.clone(), mix: self.custom });
        self.pick = Pick::Mine(name.clone());
        Some(name)
    }

    fn taken(&self, name: &str) -> bool {
        name.eq_ignore_ascii_case("custom") || Kind::ALL.iter().any(|k| k.name().eq_ignore_ascii_case(name)) || self.mine.iter().any(|m| same_name(&m.name, name))
    }

    /// Removes a saved sound. If it was the pick, Custom is picked and holds that sound's values (what was playing stays).
    pub fn remove_mine(&mut self, name: &str) -> bool {
        let Some(i) = self.mine.iter().position(|m| same_name(&m.name, name)) else { return false };
        let gone = self.mine.remove(i);
        if self.pick == Pick::Mine(gone.name.clone()) {
            self.custom = gone.mix;
            self.pick = Pick::Custom;
        }
        true
    }
}

fn same_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn parse_mine_mix(s: &str) -> Option<Mix> {
    let mut it = s.split(',').map(|v| v.trim().parse::<u8>().ok().filter(|v| *v <= Mix::MAX));
    let (a, b, c) = (it.next()??, it.next()??, it.next()??);
    if it.next().is_some() {
        return None;
    }
    Some(Mix::new(a, b, c))
}

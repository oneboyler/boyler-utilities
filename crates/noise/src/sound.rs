//! What can play: one of the six noises ([`Kind`]) or your own mix of three sliders ([`Mix`]) (Order 080).

use crate::kind::Kind;

/// Your own mix: three sliders, each 0 ..= 100.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Mix {
    /// Deep .. bright: 0 = brown (falls 6 dB per octave), 50 = pink (3 dB), 100 = white (flat).
    pub tone: u8,
    /// How much low bass there is: 0 = none added, 100 = a low shelf of +15 dB below about 120 Hz.
    pub rumble: u8,
    /// Steady .. slow sea-like swell: 0 = steady, 100 = the level rises and falls like waves (about 4 to 11 s apart).
    pub waves: u8,
}

impl Mix {
    pub const MAX: u8 = 100;
    /// What "Custom" starts as (a soft, deep sound without waves).
    pub const DEFAULT: Mix = Mix { tone: 35, rumble: 30, waves: 0 };

    pub fn new(tone: u8, rumble: u8, waves: u8) -> Mix {
        Mix { tone: tone.min(Self::MAX), rumble: rumble.min(Self::MAX), waves: waves.min(Self::MAX) }
    }
}

impl Default for Mix {
    fn default() -> Self {
        Mix::DEFAULT
    }
}

/// A sound the player can play.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sound {
    Preset(Kind),
    Mix(Mix),
}

impl Default for Sound {
    fn default() -> Self {
        Sound::Preset(Kind::default())
    }
}

impl From<Kind> for Sound {
    fn from(k: Kind) -> Sound {
        Sound::Preset(k)
    }
}

impl From<Mix> for Sound {
    fn from(m: Mix) -> Sound {
        Sound::Mix(m.clamped())
    }
}

impl Mix {
    fn clamped(self) -> Mix {
        Mix::new(self.tone, self.rumble, self.waves)
    }
}

impl Sound {
    pub fn is_mix(self) -> bool {
        matches!(self, Sound::Mix(_))
    }
}

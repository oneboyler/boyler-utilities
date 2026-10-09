//! The six noises (Order 062).

/// What the noise sounds like: its spectrum (level per frequency).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Kind {
    /// Every pitch equally loud: a bright hiss. Flat.
    White,
    /// Level falls 3 dB per octave: a softer hiss, like steady rain.
    Pink,
    /// Falls 6 dB per octave (from 40 Hz up): a deep rumble, like a distant waterfall.
    #[default]
    Brown,
    /// Falls 12 dB per octave from 200 Hz up (brown with the top cut again): deeper and darker.
    DarkBrown,
    /// Shaped like the ear's equal-loudness curve (ISO 226, 40 phon): nothing sticks out to the ear.
    Grey,
    /// Rises 3 dB per octave: thin and bright, mostly treble.
    Blue,
}

impl Kind {
    pub const ALL: [Kind; 6] = [Kind::White, Kind::Pink, Kind::Brown, Kind::DarkBrown, Kind::Grey, Kind::Blue];

    pub fn name(self) -> &'static str {
        match self {
            Kind::White => "White",
            Kind::Pink => "Pink",
            Kind::Brown => "Brown",
            Kind::DarkBrown => "Dark brown",
            Kind::Grey => "Grey",
            Kind::Blue => "Blue",
        }
    }

    /// One short line of what it sounds like (the page's row text).
    pub fn feel(self) -> &'static str {
        match self {
            Kind::White => "Bright hiss, every pitch equally loud",
            Kind::Pink => "Softer hiss, like steady rain",
            Kind::Brown => "Deep rumble, like a distant waterfall",
            Kind::DarkBrown => "Deeper and darker than brown",
            Kind::Grey => "Even to the ear, nothing sticks out",
            Kind::Blue => "Thin and bright, mostly treble",
        }
    }

    /// The word the settings file keeps.
    pub fn key(self) -> &'static str {
        match self {
            Kind::White => "white",
            Kind::Pink => "pink",
            Kind::Brown => "brown",
            Kind::DarkBrown => "dark-brown",
            Kind::Grey => "grey",
            Kind::Blue => "blue",
        }
    }

    pub fn from_key(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.key() == s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_and_brown_is_the_default() {
        for k in Kind::ALL {
            assert_eq!(Kind::from_key(k.key()), Some(k));
        }
        assert_eq!(Kind::from_key("purple"), None);
        assert_eq!(Kind::default(), Kind::Brown);
    }
}

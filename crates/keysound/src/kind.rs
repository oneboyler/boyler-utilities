//! Which of a pack's five sounds a key event gets: from its class (see `bu_rawin::SoundEvent`).

use bu_rawin::{SoundClass, SoundEvent};

/// The five sounds of a pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Any key going down.
    Down = 0,
    /// Any key coming up.
    Up = 1,
    Space = 2,
    Enter = 3,
    Backspace = 4,
}

pub const KINDS: usize = 5;

pub const ALL_KINDS: [Kind; KINDS] = [Kind::Down, Kind::Up, Kind::Space, Kind::Enter, Kind::Backspace];

/// The sound for an event: Space / Enter / Backspace have their own down sound; every other key falls back to the plain
/// down sound; every key coming up shares the one up sound.
pub fn kind_of(e: SoundEvent) -> Kind {
    if !e.down {
        return Kind::Up;
    }
    match e.class {
        SoundClass::Other => Kind::Down,
        SoundClass::Space => Kind::Space,
        SoundClass::Enter => Kind::Enter,
        SoundClass::Backspace => Kind::Backspace,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_pick_their_sound() {
        let e = |class, down| SoundEvent { class, down, key: 0x1E };
        assert_eq!(kind_of(e(SoundClass::Other, true)), Kind::Down);
        assert_eq!(kind_of(e(SoundClass::Space, true)), Kind::Space);
        assert_eq!(kind_of(e(SoundClass::Enter, true)), Kind::Enter);
        assert_eq!(kind_of(e(SoundClass::Backspace, true)), Kind::Backspace);
        for c in [SoundClass::Other, SoundClass::Space, SoundClass::Enter, SoundClass::Backspace] {
            assert_eq!(kind_of(e(c, false)), Kind::Up, "every key comes up the same way");
        }
    }

    /// Order 058 / 090: the event is (class, down, scan code) and nothing more - no text, no time, no window. The scan code picks
    /// a key's own sound and a made pack's pitch; the pack's five sounds still come from the class alone.
    #[test]
    fn the_event_is_small_and_the_pack_sound_comes_from_the_class() {
        assert_eq!(std::mem::size_of::<SoundEvent>(), 4);
        let mut seen = std::collections::HashSet::new();
        for class in [SoundClass::Other, SoundClass::Space, SoundClass::Enter, SoundClass::Backspace] {
            for down in [true, false] {
                for key in [0x1E, 0x30, 0xE01C] {
                    seen.insert(kind_of(SoundEvent { class, down, key }) as usize);
                }
            }
        }
        assert_eq!(seen.len(), KINDS);
    }
}

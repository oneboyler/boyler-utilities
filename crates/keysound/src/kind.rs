//! Which sound a key event gets. The ONLY thing taken from the key is its class (see `bu_rawin::SoundEvent`).

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
        let e = |class, down| SoundEvent { class, down };
        assert_eq!(kind_of(e(SoundClass::Other, true)), Kind::Down);
        assert_eq!(kind_of(e(SoundClass::Space, true)), Kind::Space);
        assert_eq!(kind_of(e(SoundClass::Enter, true)), Kind::Enter);
        assert_eq!(kind_of(e(SoundClass::Backspace, true)), Kind::Backspace);
        for c in [SoundClass::Other, SoundClass::Space, SoundClass::Enter, SoundClass::Backspace] {
            assert_eq!(kind_of(e(c, false)), Kind::Up, "every key comes up the same way");
        }
    }

    /// Order 058: the key is forgotten at once. The type this crate is given is (class, down) and nothing more, so
    /// there is no key to store: two bytes, and the whole engine API takes only that.
    #[test]
    fn the_engine_never_sees_a_key() {
        assert_eq!(std::mem::size_of::<SoundEvent>(), 2);
        // the only way in is `kind_of(SoundEvent)`: 5 possible sounds, so at most log2(5) bits of the key survive
        let mut seen = std::collections::HashSet::new();
        for class in [SoundClass::Other, SoundClass::Space, SoundClass::Enter, SoundClass::Backspace] {
            for down in [true, false] {
                seen.insert(kind_of(SoundEvent { class, down }) as usize);
            }
        }
        assert_eq!(seen.len(), KINDS);
    }
}

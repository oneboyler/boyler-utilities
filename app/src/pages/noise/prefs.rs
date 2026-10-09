//! What the Noise tab remembers (the settings store, page scope `noise`): the noise, the volume, the sleep timer. NOT whether
//! it plays: the noise is off until Play is pressed, every time (the owner's sound rule).

use crate::settings::{Scope, SettingsStore};
use bu_noise::{Kind, DEFAULT_VOLUME, SLEEP_CHOICES};

pub const PAGE: &str = "noise";
const K_KIND: &str = "kind";
const K_VOLUME: &str = "volume";
const K_SLEEP: &str = "sleep";

#[derive(Debug, Clone, PartialEq)]
pub struct Prefs {
    pub kind: Kind,
    /// 0 .. 100 %
    pub volume: u8,
    /// Minutes, one of `SLEEP_CHOICES` (None = off).
    pub sleep: Option<u32>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { kind: Kind::Brown, volume: DEFAULT_VOLUME, sleep: None }
    }
}

fn scope() -> Scope<'static> {
    Scope::Page(PAGE)
}

impl Prefs {
    pub fn load(store: &SettingsStore) -> Prefs {
        let d = Prefs::default();
        let minutes = store.i64_or(scope(), K_SLEEP, 0);
        Prefs {
            kind: store.get_str(scope(), K_KIND).and_then(Kind::from_key).unwrap_or(d.kind),
            volume: store.i64_or(scope(), K_VOLUME, i64::from(d.volume)).clamp(0, 100) as u8,
            sleep: SLEEP_CHOICES.iter().copied().flatten().find(|m| i64::from(*m) == minutes),
        }
    }

    pub fn save(&self, store: &mut SettingsStore) {
        let _ = store.set_str(scope(), K_KIND, self.kind.key());
        let _ = store.set_i64(scope(), K_VOLUME, i64::from(self.volume));
        let _ = store.set_i64(scope(), K_SLEEP, i64::from(self.sleep.unwrap_or(0)));
    }
}

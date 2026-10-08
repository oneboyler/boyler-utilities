//! FAKE keys OS layer (tests and test copies): registers nothing in Windows (no hotkeys, no Raw Input); a fixed Croatian
//! QWERTZ name table.
//! The table is the Croatian layout (KBDCR) as remembered — not read from Windows (guessed; the real layer asks Windows):
//! č = VK_OEM_1, ć = VK_OEM_7, ž = VK_OEM_5, š = VK_OEM_4, đ = VK_OEM_6, ' = VK_OEM_2, + = VK_OEM_PLUS, - = VK_OEM_MINUS,
//! ¸ = VK_OEM_3 (dead), < = VK_OEM_102; AltGr: Q \ · W | · E € · F [ · G ] · K ł · L Ł · V @ · B { · N } · M § · 1 ~ · 3 ^ …

use std::collections::BTreeMap;

use super::{Combo, KeysOs, Mods};

pub const VK_OEM_1: u16 = 0xBA;
pub const VK_OEM_PLUS: u16 = 0xBB;
pub const VK_OEM_COMMA: u16 = 0xBC;
pub const VK_OEM_MINUS: u16 = 0xBD;
pub const VK_OEM_PERIOD: u16 = 0xBE;
pub const VK_OEM_2: u16 = 0xBF;
pub const VK_OEM_3: u16 = 0xC0;
pub const VK_OEM_4: u16 = 0xDB;
pub const VK_OEM_5: u16 = 0xDC;
pub const VK_OEM_6: u16 = 0xDD;
pub const VK_OEM_7: u16 = 0xDE;
pub const VK_OEM_102: u16 = 0xE2;

#[derive(Debug, Default)]
pub struct FakeKeysOs {
    /// slot → combo, registered "with Windows".
    pub registered: BTreeMap<i32, Combo>,
    /// The raw devices registered now (keyboard, mouse).
    pub raw: (bool, bool),
    /// Raw Input registration refuses (a test of the error path).
    pub raw_fails: bool,
    /// What `mods_now` answers (None = unknown).
    pub mods: Option<Mods>,
    /// Combos Windows or another app holds: register refuses them.
    pub taken: Vec<Combo>,
    /// Every register / raw call, in order ("reg 1", "unreg 1", "raw kbd+mouse", "raw off").
    pub log: Vec<String>,
}

impl FakeKeysOs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Something outside the app holds this combo.
    pub fn take(mut self, combo: Combo) -> Self {
        self.taken.push(combo);
        self
    }

    /// The unshifted character of a key on the Croatian layout.
    fn base_char(vk: u16) -> Option<char> {
        Some(match vk {
            0x30..=0x39 => vk as u8 as char,
            0x41..=0x5A => (vk as u8 as char).to_ascii_lowercase(),
            VK_OEM_1 => 'č',
            VK_OEM_7 => 'ć',
            VK_OEM_5 => 'ž',
            VK_OEM_4 => 'š',
            VK_OEM_6 => 'đ',
            VK_OEM_2 => '\'',
            VK_OEM_PLUS => '+',
            VK_OEM_MINUS => '-',
            VK_OEM_COMMA => ',',
            VK_OEM_PERIOD => '.',
            VK_OEM_3 => '¸',
            VK_OEM_102 => '<',
            _ => return None,
        })
    }
}

impl KeysOs for FakeKeysOs {
    fn register(&mut self, slot: i32, combo: Combo) -> Result<(), String> {
        if self.taken.contains(&combo) || self.registered.values().any(|c| *c == combo) {
            self.log.push(format!("refused {slot}"));
            return Err("Hot key is already registered.".into());
        }
        self.log.push(format!("reg {slot}"));
        self.registered.insert(slot, combo);
        Ok(())
    }

    fn unregister(&mut self, slot: i32) {
        self.log.push(format!("unreg {slot}"));
        self.registered.remove(&slot);
    }

    fn raw_devices(&mut self, keyboard: bool, mouse: bool) -> Result<(), String> {
        if self.raw_fails && (keyboard || mouse) {
            return Err("RegisterRawInputDevices failed".into());
        }
        let what = match (keyboard, mouse) {
            (false, false) => "off",
            (true, false) => "kbd",
            (false, true) => "mouse",
            (true, true) => "kbd+mouse",
        };
        self.log.push(format!("raw {what}"));
        self.raw = (keyboard, mouse);
        Ok(())
    }

    fn mods_now(&self) -> Option<Mods> {
        self.mods
    }

    fn key_name(&self, vk: u16) -> String {
        match Self::base_char(vk) {
            Some(c) => c.to_uppercase().collect(),
            None => format!("Key {vk}"),
        }
    }
}

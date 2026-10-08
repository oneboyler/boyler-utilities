//! Raw Input routing (pure, no Windows calls): the keys that RegisterHotKey can't do — modifier-only keys ("Ctrl + Shift",
//! fire on release, as the drawing's key field commits them), mouse buttons 3 / 4 / 5 (fire on button down) and keys whose
//! release matters too (`Action::needs_release`: down + up). Boss A_014_01: Raw Input with RIDEV_INPUTSINK only — the app
//! just listens; every key and button still reaches the other apps (nothing blocked, nothing altered, no hook).
//! - [`RawRouter::needs`] says which raw devices must be registered: keyboard while a modifier-only or release key is
//!   bound; mouse while a mouse-button key is bound, and also while a modifier-only key is bound (so Ctrl + click /
//!   Ctrl + wheel don't count as a lone "Ctrl" tap). Nothing bound = nothing registered;
//! - [`RawRouter::feed`] takes one parsed packet ([`Packet`]) and calls `fire(slot, down)`; a mouse move (no button
//!   flags) returns at once — that is the 8000-a-second case.
//!   Modifiers are tracked per side from the packets themselves; the manager re-reads them from Windows
//!   (`KeysOs::mods_now`) only on a key-down / button-down packet, never on a move.

use super::{is_nav_vk, Combo, Mods};

// RAWKEYBOARD.Flags
pub const RI_KEY_BREAK: u16 = 1;
pub const RI_KEY_E0: u16 = 2;
// RAWMOUSE.usButtonFlags
pub const RI_MOUSE_LEFT_DOWN: u16 = 0x0001;
pub const RI_MOUSE_LEFT_UP: u16 = 0x0002;
pub const RI_MOUSE_RIGHT_DOWN: u16 = 0x0004;
pub const RI_MOUSE_RIGHT_UP: u16 = 0x0008;
pub const RI_MOUSE_MIDDLE_DOWN: u16 = 0x0010;
pub const RI_MOUSE_MIDDLE_UP: u16 = 0x0020;
pub const RI_MOUSE_BUTTON_4_DOWN: u16 = 0x0040;
pub const RI_MOUSE_BUTTON_4_UP: u16 = 0x0080;
pub const RI_MOUSE_BUTTON_5_DOWN: u16 = 0x0100;
pub const RI_MOUSE_BUTTON_5_UP: u16 = 0x0200;
pub const RI_MOUSE_WHEEL: u16 = 0x0400;
pub const RI_MOUSE_HWHEEL: u16 = 0x0800;

const DOWNS: u16 = RI_MOUSE_LEFT_DOWN | RI_MOUSE_RIGHT_DOWN | RI_MOUSE_MIDDLE_DOWN | RI_MOUSE_BUTTON_4_DOWN | RI_MOUSE_BUTTON_5_DOWN;
const UPS: u16 = RI_MOUSE_LEFT_UP | RI_MOUSE_RIGHT_UP | RI_MOUSE_MIDDLE_UP | RI_MOUSE_BUTTON_4_UP | RI_MOUSE_BUTTON_5_UP;

/// The virtual-key codes the manager uses for mouse buttons 3 / 4 / 5 (Windows' own: VK_MBUTTON, VK_XBUTTON1 / 2).
pub const VK_MBUTTON: u16 = 0x04;
pub const VK_XBUTTON1: u16 = 0x05;
pub const VK_XBUTTON2: u16 = 0x06;

/// One WM_INPUT packet, parsed (the real layer fills it from RAWINPUT).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Packet {
    /// RAWKEYBOARD: VKey, MakeCode, Flags.
    Key { vk: u16, make: u16, flags: u16 },
    /// RAWMOUSE.usButtonFlags (0 = a move).
    Mouse { buttons: u16 },
    Other,
}

impl Packet {
    /// A key or button going down (the manager re-reads the modifiers from Windows first).
    pub fn is_press(&self) -> bool {
        match *self {
            Packet::Key { vk, flags, .. } => flags & RI_KEY_BREAK == 0 && side_bit(vk, 0, 0).is_none(),
            Packet::Mouse { buttons } => buttons & DOWNS != 0,
            Packet::Other => false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Row {
    slot: i32,
    combo: Combo,
    /// A keyboard key whose up is reported too.
    release: bool,
    /// Extra modifiers held are allowed (`Action::extra_mods`).
    loose: bool,
}

// per-side modifier bits
const LCTRL: u8 = 1;
const RCTRL: u8 = 2;
const LALT: u8 = 4;
const RALT: u8 = 8;
const LSHIFT: u8 = 16;
const RSHIFT: u8 = 32;
const LWIN: u8 = 64;
const RWIN: u8 = 128;

/// The side bit a modifier packet sets (None = not a modifier).
fn side_bit(vk: u16, make: u16, flags: u16) -> Option<u8> {
    let e0 = flags & RI_KEY_E0 != 0;
    Some(match vk {
        0x11 => if e0 { RCTRL } else { LCTRL },
        0xA2 => LCTRL,
        0xA3 => RCTRL,
        0x12 => if e0 { RALT } else { LALT },
        0xA4 => LALT,
        0xA5 => RALT,
        0x10 => if make == 0x36 { RSHIFT } else { LSHIFT },
        0xA0 => LSHIFT,
        0xA1 => RSHIFT,
        0x5B => LWIN,
        0x5C => RWIN,
        _ => return None,
    })
}

fn mods_of(sides: u8) -> Mods {
    let mut m = Mods::NONE;
    if sides & (LCTRL | RCTRL) != 0 {
        m = m.with(Mods::CTRL);
    }
    if sides & (LALT | RALT) != 0 {
        m = m.with(Mods::ALT);
    }
    if sides & (LSHIFT | RSHIFT) != 0 {
        m = m.with(Mods::SHIFT);
    }
    if sides & (LWIN | RWIN) != 0 {
        m = m.with(Mods::WIN);
    }
    m
}

/// The routing table + the little state it needs. Owned by the keys manager.
#[derive(Debug, Default)]
pub struct RawRouter {
    rows: Vec<Row>,
    sides: u8,
    /// Every modifier held since the modifiers were last all up.
    peak: Mods,
    /// Something else was pressed while modifiers were held (or was already down): no modifier-only fire.
    dirty: bool,
    /// Non-modifier keys down now.
    keys_down: Vec<u16>,
    /// Mouse buttons down now (the DOWN bits).
    buttons_down: u16,
    /// Release rows whose key is down (slot, vk).
    down: Vec<(i32, u16)>,
}

impl RawRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, slot: i32, combo: Combo, release: bool) {
        self.add_loose(slot, combo, release, false);
    }

    /// `loose` = it also fires while extra modifiers are held (the combo's own must be held).
    pub fn add_loose(&mut self, slot: i32, combo: Combo, release: bool, loose: bool) {
        self.remove(slot);
        self.rows.push(Row { slot, combo, release, loose });
    }

    pub fn remove(&mut self, slot: i32) {
        self.rows.retain(|r| r.slot != slot);
        self.down.retain(|(s, _)| *s != slot);
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// (keyboard, mouse): which raw devices must be registered for the rows there are.
    pub fn needs(&self) -> (bool, bool) {
        let mods_only = self.rows.iter().any(|r| r.combo.is_mods_only());
        let keyboard = mods_only || self.rows.iter().any(|r| r.release && r.combo.mouse_button().is_none());
        let mouse = mods_only || self.rows.iter().any(|r| r.combo.mouse_button().is_some());
        (keyboard, mouse)
    }

    /// Forget what is held (a device was just registered: what happened before is unknown).
    pub fn reset_state(&mut self) {
        self.sides = 0;
        self.peak = Mods::NONE;
        self.dirty = false;
        self.keys_down.clear();
        self.buttons_down = 0;
        self.down.clear();
    }

    /// The modifiers held as tracked.
    pub fn held(&self) -> Mods {
        mods_of(self.sides)
    }

    /// Correct the tracked modifiers with Windows' own state (a key-up may have been missed, e.g. on the lock screen).
    pub fn resync(&mut self, m: Mods) {
        for (mm, l, r) in [(Mods::CTRL, LCTRL, RCTRL), (Mods::ALT, LALT, RALT), (Mods::SHIFT, LSHIFT, RSHIFT), (Mods::WIN, LWIN, RWIN)] {
            if !m.contains(mm) {
                self.sides &= !(l | r);
            } else if self.sides & (l | r) == 0 {
                self.sides |= l;
            }
        }
        if self.sides == 0 {
            self.peak = Mods::NONE;
        }
    }

    /// One packet; `fire(slot, down)` for every action it triggers.
    #[inline]
    pub fn feed(&mut self, p: Packet, fire: &mut impl FnMut(i32, bool)) {
        match p {
            Packet::Mouse { buttons: 0 } | Packet::Other => {}
            Packet::Mouse { buttons } => self.mouse(buttons, fire),
            Packet::Key { vk, make, flags } => self.key(vk, make, flags, fire),
        }
    }

    fn mouse(&mut self, buttons: u16, fire: &mut impl FnMut(i32, bool)) {
        let downs = buttons & DOWNS;
        if (downs != 0 || buttons & (RI_MOUSE_WHEEL | RI_MOUSE_HWHEEL) != 0) && self.sides != 0 {
            self.dirty = true;
        }
        if downs != 0 {
            self.buttons_down |= downs;
            let held = self.held();
            for (bit, vk) in [(RI_MOUSE_MIDDLE_DOWN, VK_MBUTTON), (RI_MOUSE_BUTTON_4_DOWN, VK_XBUTTON1), (RI_MOUSE_BUTTON_5_DOWN, VK_XBUTTON2)] {
                if downs & bit != 0 {
                    // an exact match wins; with none, the loose rows whose modifiers are all held
                    let exact: Vec<i32> = self.rows.iter().filter(|r| r.combo.vk == vk && r.combo.mods == held).map(|r| r.slot).collect();
                    let hits: Vec<i32> = if exact.is_empty() {
                        self.rows.iter().filter(|r| r.combo.vk == vk && r.loose && held.contains(r.combo.mods)).map(|r| r.slot).collect()
                    } else {
                        exact
                    };
                    for s in hits {
                        fire(s, true);
                    }
                }
            }
        }
        let ups = buttons & UPS;
        if ups != 0 {
            // each UP bit is its DOWN bit << 1
            self.buttons_down &= !(ups >> 1);
        }
    }

    fn key(&mut self, vk: u16, make: u16, flags: u16, fire: &mut impl FnMut(i32, bool)) {
        if vk == 0xFF || vk == 0 {
            return; // Windows' fake keys (part of Pause / some NumLock sequences)
        }
        let up = flags & RI_KEY_BREAK != 0;
        if let Some(bit) = side_bit(vk, make, flags) {
            if !up {
                if self.sides == 0 {
                    self.peak = Mods::NONE;
                    self.dirty = !self.keys_down.is_empty() || self.buttons_down != 0;
                }
                self.sides |= bit;
                self.peak = self.peak.with(self.held());
            } else {
                self.sides &= !bit;
                if self.sides == 0 {
                    if !self.dirty && !self.peak.is_empty() {
                        let peak = self.peak;
                        for r in self.rows.iter().filter(|r| r.combo.is_mods_only() && r.combo.mods == peak) {
                            fire(r.slot, true);
                        }
                    }
                    self.peak = Mods::NONE;
                    self.dirty = false;
                }
            }
            return;
        }
        if !up {
            if self.sides != 0 {
                self.dirty = true;
            }
            if !self.keys_down.contains(&vk) {
                self.keys_down.push(vk);
            }
            if self.down.iter().any(|(_, v)| *v == vk) {
                return; // auto-repeat of a held release key
            }
            let held = self.held();
            let extended = flags & RI_KEY_E0 != 0;
            let nav_ok = extended || !is_nav_vk(vk);
            // an exact match wins over a loose one (its modifiers held, extra ones too)
            let hit = self
                .rows
                .iter()
                .find(|r| r.release && r.combo.vk == vk && r.combo.mods == held && nav_ok)
                .or_else(|| self.rows.iter().find(|r| r.release && r.loose && r.combo.vk == vk && held.contains(r.combo.mods) && nav_ok));
            if let Some(r) = hit {
                self.down.push((r.slot, vk));
                fire(r.slot, true);
            }
        } else {
            self.keys_down.retain(|v| *v != vk);
            if let Some(i) = self.down.iter().position(|(_, v)| *v == vk) {
                let (slot, _) = self.down.remove(i);
                fire(slot, false);
            }
        }
    }
}

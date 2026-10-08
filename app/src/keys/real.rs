//! REAL keys OS layer, on the UI thread only. Never used by tests.
//! - normal keys: RegisterHotKey / UnregisterHotKey on the app's window (WM_HOTKEY, wParam = slot);
//! - modifier-only keys, mouse buttons 3 / 4 / 5 and release keys: Raw Input (boss A_014_01), listen only
//!   (RIDEV_INPUTSINK): nothing is blocked or altered, there is NO hook anywhere. Order 048: the registration and the
//!   reading are bu-rawin's (the process's one Raw Input owner, on its own thread with a message-only window — so the
//!   activity watcher's idle check can't take the devices away, and the 8000 mouse moves a second never wake the UI
//!   thread). `raw_devices` = `bu_rawin::set_keys(keyboard, mouse, message window + WM_RAWKEYS)`;
//! - the window procedure: `WM_RAWKEYS => services::raw_packets()` = `bu_rawin::take_packets()` (key packets, mouse
//!   buttons / wheel; one wake-up per batch) → [`Packet`] → `KeysManager::on_raw`. [`packet_of`] /
//!   [`KeysManager::on_rawinput`] stay for the benchmark test (a RAWINPUT → the router);
//! - key names: the character the layout gives the key (MapVirtualKeyEx, upper-cased: Č Ć Ž Š Đ on Croatian), else
//!   GetKeyNameText.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyNameTextW, GetKeyboardLayout, MapVirtualKeyExW, RegisterHotKey, UnregisterHotKey,
    HOT_KEY_MODIFIERS, MAPVK_VK_TO_CHAR, MAPVK_VK_TO_VSC, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
};
use windows::Win32::UI::Input::{RAWINPUT, RIM_TYPEKEYBOARD, RIM_TYPEMOUSE};

use super::{is_nav_vk, Combo, KeysManager, KeysOs, Mods, Packet, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT};
use crate::services::WM_RAWKEYS;

pub struct RealKeysOs {
    hwnd: isize,
    registered: Vec<i32>,
    /// Raw devices registered now (keyboard, mouse).
    raw: (bool, bool),
}

impl RealKeysOs {
    /// `hwnd` = the app's message window (gets WM_HOTKEY and bu-rawin's WM_RAWKEYS).
    pub fn new(hwnd: HWND) -> Self {
        RealKeysOs { hwnd: hwnd.0 as isize, registered: Vec::new(), raw: (false, false) }
    }

    fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut _)
    }
}

/// Mods → RegisterHotKey's flags (+ MOD_NOREPEAT: holding the key fires once).
pub fn hotkey_flags(m: Mods) -> HOT_KEY_MODIFIERS {
    let mut f = MOD_NOREPEAT;
    if m.contains(Mods::CTRL) {
        f |= MOD_CONTROL;
    }
    if m.contains(Mods::ALT) {
        f |= MOD_ALT;
    }
    if m.contains(Mods::SHIFT) {
        f |= MOD_SHIFT;
    }
    if m.contains(Mods::WIN) {
        f |= MOD_WIN;
    }
    f
}

/// The modifiers held right now.
pub fn mods_now() -> Mods {
    let down = |vk: u16| unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000 != 0;
    let mut m = Mods::NONE;
    if down(VK_CONTROL) {
        m = m.with(Mods::CTRL);
    }
    if down(VK_MENU) {
        m = m.with(Mods::ALT);
    }
    if down(VK_SHIFT) {
        m = m.with(Mods::SHIFT);
    }
    if down(VK_LWIN) || down(VK_RWIN) {
        m = m.with(Mods::WIN);
    }
    m
}

/// One RAWINPUT → the router's packet (mouse: usButtonFlags; keyboard: VKey, MakeCode, Flags).
#[inline]
pub fn packet_of(raw: &RAWINPUT) -> Packet {
    let t = raw.header.dwType;
    if t == RIM_TYPEMOUSE.0 {
        // SAFETY: dwType says the union holds a RAWMOUSE.
        Packet::Mouse { buttons: unsafe { raw.data.mouse.Anonymous.Anonymous.usButtonFlags } }
    } else if t == RIM_TYPEKEYBOARD.0 {
        // SAFETY: dwType says the union holds a RAWKEYBOARD.
        let k = unsafe { raw.data.keyboard };
        Packet::Key { vk: k.VKey, make: k.MakeCode, flags: k.Flags }
    } else {
        Packet::Other
    }
}

/// bu-rawin's packet → the router's (the same three fields).
impl From<bu_rawin::RawPacket> for Packet {
    #[inline]
    fn from(p: bu_rawin::RawPacket) -> Packet {
        match p {
            bu_rawin::RawPacket::Key { vk, make, flags } => Packet::Key { vk, make, flags },
            bu_rawin::RawPacket::Mouse { buttons } => Packet::Mouse { buttons },
        }
    }
}

impl<O: KeysOs> KeysManager<O> {
    /// One RAWINPUT: `fire(action id, down)` for every action it triggers (the benchmark's path).
    #[inline]
    pub fn on_rawinput(&mut self, raw: &RAWINPUT, fire: impl FnMut(&str, bool)) {
        self.on_raw(packet_of(raw), fire);
    }
}

impl KeysOs for RealKeysOs {
    fn register(&mut self, slot: i32, combo: Combo) -> Result<(), String> {
        unsafe { RegisterHotKey(Some(self.hwnd()), slot, hotkey_flags(combo.mods), combo.vk as u32) }
            .map_err(|e| e.message())?;
        self.registered.push(slot);
        Ok(())
    }

    fn unregister(&mut self, slot: i32) {
        let _ = unsafe { UnregisterHotKey(Some(self.hwnd()), slot) };
        self.registered.retain(|s| *s != slot);
    }

    fn raw_devices(&mut self, keyboard: bool, mouse: bool) -> Result<(), String> {
        // bu-rawin registers (on its own thread) what all its clients need together and wakes this window with
        // WM_RAWKEYS once per batch of packets that matter; nothing needed = no wake-ups
        let notify = (keyboard || mouse).then_some((self.hwnd, WM_RAWKEYS));
        bu_rawin::set_keys(keyboard, mouse, notify)?;
        self.raw = (keyboard, mouse);
        Ok(())
    }

    fn mods_now(&self) -> Option<Mods> {
        Some(mods_now())
    }

    fn key_name(&self, vk: u16) -> String {
        let hkl = unsafe { GetKeyboardLayout(0) };
        let ch = unsafe { MapVirtualKeyExW(vk as u32, MAPVK_VK_TO_CHAR, Some(hkl)) } & 0xFFFF;
        if let Some(c) = char::from_u32(ch).filter(|c| !c.is_control() && *c != '\0') {
            return c.to_uppercase().collect();
        }
        let scan = unsafe { MapVirtualKeyExW(vk as u32, MAPVK_VK_TO_VSC, Some(hkl)) };
        if scan != 0 {
            let ext = if is_nav_vk(vk) { 1 << 24 } else { 0 };
            let mut buf = [0u16; 64];
            let n = unsafe { GetKeyNameTextW(((scan as i32) << 16) | ext, &mut buf) };
            if n > 0 {
                return String::from_utf16_lossy(&buf[..n as usize]);
            }
        }
        format!("Key {vk}")
    }
}

impl Drop for RealKeysOs {
    fn drop(&mut self) {
        for slot in std::mem::take(&mut self.registered) {
            let _ = unsafe { UnregisterHotKey(Some(self.hwnd()), slot) };
        }
        if self.raw != (false, false) {
            let _ = self.raw_devices(false, false);
        }
    }
}

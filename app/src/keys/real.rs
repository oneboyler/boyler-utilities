//! REAL keys OS layer, on the UI thread only. Never used by tests.
//! - normal keys: RegisterHotKey / UnregisterHotKey on the app's window (WM_HOTKEY, wParam = slot);
//! - modifier-only keys, mouse buttons 3 / 4 / 5 and release keys: Raw Input (boss A_014_01) — RegisterRawInputDevices
//!   with RIDEV_INPUTSINK on the app's window (keyboard = usage page 1 / usage 6, mouse = 1 / 2), RIDEV_REMOVE when the
//!   last such key goes. The app only listens: nothing is blocked or altered, there is NO hook anywhere;
//! - the window procedure: `WM_INPUT => keys.on_wm_input(lparam, |action, down| …)`, then DefWindowProc as usual
//!   (Windows wants it for WM_INPUT). [`KeysManager::on_wm_input`] = GetRawInputData into a stack RAWINPUT +
//!   [`KeysManager::on_rawinput`] (= [`packet_of`] + `on_raw`), the exact path the benchmark test measures;
//! - key names: the character the layout gives the key (MapVirtualKeyEx, upper-cased: Č Ć Ž Š Đ on Croatian), else
//!   GetKeyNameText.

use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyNameTextW, GetKeyboardLayout, MapVirtualKeyExW, RegisterHotKey, UnregisterHotKey,
    HOT_KEY_MODIFIERS, MAPVK_VK_TO_CHAR, MAPVK_VK_TO_VSC, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
};
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RIDEV_INPUTSINK, RIDEV_REMOVE,
    RID_INPUT, RIM_TYPEKEYBOARD, RIM_TYPEMOUSE,
};

use super::{is_nav_vk, Combo, KeysManager, KeysOs, Mods, Packet, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT};

const USAGE_PAGE_GENERIC: u16 = 1;
const USAGE_MOUSE: u16 = 2;
const USAGE_KEYBOARD: u16 = 6;

pub struct RealKeysOs {
    hwnd: isize,
    registered: Vec<i32>,
    /// Raw devices registered now (keyboard, mouse).
    raw: (bool, bool),
}

impl RealKeysOs {
    /// `hwnd` = the app's message window (gets WM_HOTKEY and WM_INPUT).
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

impl<O: KeysOs> KeysManager<O> {
    /// One RAWINPUT (already copied out of WM_INPUT): `fire(action id, down)` for every action it triggers.
    #[inline]
    pub fn on_rawinput(&mut self, raw: &RAWINPUT, fire: impl FnMut(&str, bool)) {
        self.on_raw(packet_of(raw), fire);
    }
}

impl KeysManager<RealKeysOs> {
    /// WM_INPUT: read the packet (GetRawInputData into a stack RAWINPUT) and route it. The caller still passes the
    /// message on to DefWindowProc.
    pub fn on_wm_input(&mut self, lparam: LPARAM, fire: impl FnMut(&str, bool)) {
        let mut raw = RAWINPUT::default();
        let mut size = std::mem::size_of::<RAWINPUT>() as u32;
        let n = unsafe {
            GetRawInputData(
                HRAWINPUT(lparam.0 as *mut c_void),
                RID_INPUT,
                Some(&mut raw as *mut RAWINPUT as *mut c_void),
                &mut size,
                std::mem::size_of::<RAWINPUTHEADER>() as u32,
            )
        };
        if n == 0 || n == u32::MAX {
            return; // not a keyboard / mouse packet that fits (only those two are registered)
        }
        self.on_rawinput(&raw, fire);
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
        let mut list: Vec<RAWINPUTDEVICE> = Vec::with_capacity(2);
        for (usage, want, have) in [(USAGE_KEYBOARD, keyboard, self.raw.0), (USAGE_MOUSE, mouse, self.raw.1)] {
            if want && !have {
                list.push(RAWINPUTDEVICE {
                    usUsagePage: USAGE_PAGE_GENERIC,
                    usUsage: usage,
                    dwFlags: RIDEV_INPUTSINK,
                    hwndTarget: self.hwnd(),
                });
            } else if !want && have {
                list.push(RAWINPUTDEVICE {
                    usUsagePage: USAGE_PAGE_GENERIC,
                    usUsage: usage,
                    dwFlags: RIDEV_REMOVE,
                    hwndTarget: HWND(std::ptr::null_mut()),
                });
            }
        }
        if !list.is_empty() {
            unsafe { RegisterRawInputDevices(&list, std::mem::size_of::<RAWINPUTDEVICE>() as u32) }.map_err(|e| e.message())?;
        }
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

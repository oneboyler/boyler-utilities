//! What a key is called on the user's CURRENT Windows keyboard layout (Order 058, the keyboard picture): the Z key of a
//! US keyboard is the Y key of a Croatian QWERTZ one, and the key right of L is "Č". Nothing is hard-coded to one layout:
//! the label comes from asking the layout ([`Layout`]); only keys that print no character have fixed English names
//! ([`crate::remap::name`]). The pure [`label`] is tested with a QWERTZ table; [`WinLayout`] asks Windows.

use crate::remap::{name as fixed_name, Code};

/// A keyboard layout, as far as the labels need it.
pub trait Layout {
    /// The virtual key a scancode makes on this layout (None = none).
    fn vk_of(&self, code: Code) -> Option<u16>;
    /// The character a virtual key types without Shift (None = it types none).
    fn char_of(&self, vk: u16) -> Option<char>;
}

/// The key's label: a fixed name for Esc / Tab / Ctrl / arrows …, else the character it types (upper case), else its code.
pub fn label(l: &dyn Layout, code: Code) -> String {
    let n = fixed_name(code);
    if !n.starts_with("Key 0x") {
        return n;
    }
    match l.vk_of(code).and_then(|vk| l.char_of(vk)) {
        Some(c) if !c.is_control() => c.to_uppercase().collect(),
        _ => n,
    }
}

#[cfg(windows)]
pub use real::{code_of_vk, current, layout_id, WinLayout};

#[cfg(windows)]
mod real {
    use super::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, GetKeyboardLayout, GetKeyboardLayoutNameW, MapVirtualKeyExW, HKL, MAPVK_VK_TO_CHAR, MAPVK_VK_TO_VSC_EX, MAPVK_VSC_TO_VK_EX,
    };

    /// The layout of the calling thread (the menu's: what the user types with).
    pub struct WinLayout {
        hkl: isize,
    }

    fn hkl(l: &WinLayout) -> HKL {
        HKL(l.hkl as *mut std::ffi::c_void)
    }

    pub fn current() -> WinLayout {
        // SAFETY: a plain query of the calling thread's layout.
        WinLayout { hkl: unsafe { GetKeyboardLayout(0) }.0 as isize }
    }

    impl Layout for WinLayout {
        fn vk_of(&self, code: Code) -> Option<u16> {
            // SAFETY: plain lookup; an extended scancode carries 0xE0 in its high byte, as Windows wants it.
            let vk = unsafe { MapVirtualKeyExW(u32::from(code), MAPVK_VSC_TO_VK_EX, Some(hkl(self))) };
            (vk != 0).then_some(vk as u16)
        }
        fn char_of(&self, vk: u16) -> Option<char> {
            // SAFETY: plain lookup; bit 31 marks a dead key (the character is still the first word).
            let r = unsafe { MapVirtualKeyExW(u32::from(vk), MAPVK_VK_TO_CHAR, Some(hkl(self))) };
            char::from_u32(r & 0xFFFF).filter(|c| *c != '\0')
        }
    }

    /// The scancode (0xE0 high byte for an extended key) of the key a virtual-key code comes from. The menu reports Ctrl /
    /// Shift / Alt without a side: which one it was is read from what is held now.
    pub fn code_of_vk(vk: u16) -> Option<Code> {
        let held = |k: u16| (unsafe { GetAsyncKeyState(i32::from(k)) } as u16) & 0x8000 != 0;
        let vk = match vk {
            0x11 => if held(0xA3) { 0xA3 } else { 0xA2 },
            0x10 => if held(0xA1) { 0xA1 } else { 0xA0 },
            0x12 => if held(0xA5) { 0xA5 } else { 0xA4 },
            v => v,
        };
        let l = current();
        // SAFETY: plain lookup.
        let sc = unsafe { MapVirtualKeyExW(u32::from(vk), MAPVK_VK_TO_VSC_EX, Some(hkl(&l))) };
        (sc != 0).then_some(sc as u16)
    }

    /// The layout's id as Windows names it (`0000041A` = Croatian), for the page's line "Labels follow your layout (…)".
    pub fn layout_id() -> String {
        let mut b = [0u16; 9];
        // SAFETY: the buffer is the 9 characters Windows documents.
        if unsafe { GetKeyboardLayoutNameW(&mut b) }.is_ok() {
            let n = b.iter().position(|c| *c == 0).unwrap_or(b.len());
            String::from_utf16_lossy(&b[..n])
        } else {
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Croatian QWERTZ keyboard: the physical keys of a US one with Z / Y swapped and č ć ž š đ on the bracket / quote keys.
    struct Qwertz;
    impl Layout for Qwertz {
        fn vk_of(&self, code: Code) -> Option<u16> {
            // the "vk" here is simply the code: the layout test below is about the character table
            (code != 0xFFFF).then_some(code)
        }
        fn char_of(&self, vk: u16) -> Option<char> {
            Some(match vk {
                0x15 => 'z',
                0x2C => 'y',
                0x27 => 'č',
                0x28 => 'ć',
                0x2B => 'ž',
                0x1A => 'š',
                0x1B => 'đ',
                0x10 => 'q',
                0x1E => 'a',
                0x56 => '<',
                0x0D => '+',
                0x0C => '\'',
                0x29 => '¸',
                0x02 => '1',
                0x33 => ',',
                _ => return None,
            })
        }
    }

    #[test]
    fn labels_follow_the_layout_not_a_us_table() {
        let l = Qwertz;
        assert_eq!(label(&l, 0x15), "Z", "the key in the Y place of a US keyboard is Z here");
        assert_eq!(label(&l, 0x2C), "Y");
        assert_eq!(label(&l, 0x27), "Č");
        assert_eq!(label(&l, 0x28), "Ć");
        assert_eq!(label(&l, 0x2B), "Ž");
        assert_eq!(label(&l, 0x1A), "Š");
        assert_eq!(label(&l, 0x1B), "Đ");
        assert_eq!(label(&l, 0x56), "<");
        assert_eq!(label(&l, 0x10), "Q");
        assert_eq!(label(&l, 0x29), "¸");
    }

    #[test]
    fn keys_without_a_character_keep_their_fixed_names() {
        let l = Qwertz;
        assert_eq!(label(&l, 0x01), "Esc");
        assert_eq!(label(&l, 0x3A), "Caps Lock");
        assert_eq!(label(&l, 0xE038), "Right Alt");
        assert_eq!(label(&l, 0x3B), "F1");
        assert_eq!(label(&l, 0xE04B), "Left");
    }

    #[test]
    fn an_unknown_key_is_its_code() {
        let l = Qwertz;
        assert_eq!(label(&l, 0x77), "Key 0x77");
        assert_eq!(label(&l, 0xFFFF), "Key 0xFFFF");
    }
}

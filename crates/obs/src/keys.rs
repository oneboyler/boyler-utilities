//! OBS's hotkey names (keys.c): OBS's profile stores keys as `{"key":"OBS_KEY_F9","control":true}`; ClipPing reads them
//! (Save clip, instant replay on/off, recording on/off) and writes them back when changed in its settings.

/// Modifier bits as ClipPing keeps them (also in its settings file's packed `SwitchKey`).
pub const MOD_C: u8 = 1;
pub const MOD_S: u8 = 2;
pub const MOD_A: u8 = 4;
pub const MOD_W: u8 = 8;

/// One key: a Windows virtual-key code + ClipPing's modifier bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct KeyBind {
    pub vk: u16,
    pub mods: u8,
}

impl KeyBind {
    pub fn new(vk: u16, mods: u8) -> Self {
        KeyBind { vk, mods }
    }
    /// ClipPing's `key_pack`: vk | mods << 16.
    pub fn pack(self) -> u32 {
        self.vk as u32 | (self.mods as u32) << 16
    }
    pub fn unpack(v: u32) -> Self {
        KeyBind { vk: (v & 0xFFFF) as u16, mods: (v >> 16) as u8 }
    }
}

/// How many bindings of the Save clip key ClipPing listens to.
pub const MAX_BINDS: usize = 4;

const NAMES: &[(&str, u16)] = &[
    ("RETURN", 0x0D), ("ENTER", 0x0D), ("ESCAPE", 0x1B), ("TAB", 0x09),
    ("BACKSPACE", 0x08), ("INSERT", 0x2D), ("DELETE", 0x2E), ("PAUSE", 0x13),
    ("PRINT", 0x2C), ("SYSREQ", 0x2C), ("CLEAR", 0x0C), ("HOME", 0x24),
    ("END", 0x23), ("LEFT", 0x25), ("UP", 0x26), ("RIGHT", 0x27), ("DOWN", 0x28),
    ("PAGEUP", 0x21), ("PAGEDOWN", 0x22), ("SHIFT", 0x10), ("CONTROL", 0x11),
    ("ALT", 0x12), ("META", 0x5B), ("CAPSLOCK", 0x14), ("NUMLOCK", 0x90),
    ("SCROLLLOCK", 0x91), ("MENU", 0x5D), ("SPACE", 0x20),
    ("NUMASTERISK", 0x6A), ("NUMPLUS", 0x6B), ("NUMMINUS", 0x6D),
    ("NUMPERIOD", 0x6E), ("NUMSLASH", 0x6F), ("NUMCOMMA", 0x6C),
    ("MINUS", 0xBD), ("EQUAL", 0xBB), ("PLUS", 0xBB), ("COMMA", 0xBC),
    ("PERIOD", 0xBE), ("SLASH", 0xBF), ("SEMICOLON", 0xBA), ("APOSTROPHE", 0xDE),
    ("QUOTELEFT", 0xC0), ("ASCIITILDE", 0xC0), ("BRACKETLEFT", 0xDB),
    ("BACKSLASH", 0xDC), ("BRACKETRIGHT", 0xDD), ("LESS", 0xE2),
    ("MOUSE1", 0x01), ("MOUSE2", 0x02), ("MOUSE3", 0x04),
    ("MOUSE4", 0x05), ("MOUSE5", 0x06),
    ("VK_VOLUME_MUTE", 0xAD), ("VK_VOLUME_DOWN", 0xAE), ("VK_VOLUME_UP", 0xAF),
    ("VK_MEDIA_PLAY_PAUSE", 0xB3), ("VK_MEDIA_STOP", 0xB2),
    ("VK_MEDIA_PREV_TRACK", 0xB1), ("VK_MEDIA_NEXT_TRACK", 0xB0),
];

const VK_F1: u16 = 0x70;
const VK_F24: u16 = 0x87;
const VK_NUMPAD0: u16 = 0x60;

/// "OBS_KEY_F9" -> VK_F9 (0 = unknown).
pub fn obs_key_to_vk(name: &str) -> u16 {
    let Some(n) = name.strip_prefix("OBS_KEY_") else { return 0 };
    let b = n.as_bytes();
    if b.len() == 1 && (b[0].is_ascii_uppercase() || b[0].is_ascii_digit()) {
        return b[0] as u16;
    }
    if b.len() >= 2 && b[0] == b'F' && (b'1'..=b'9').contains(&b[1]) {
        let v = crate::ini::atoi(&n[1..]);
        if (1..=24).contains(&v) {
            return VK_F1 + v as u16 - 1;
        }
    }
    if b.len() == 4 && n.starts_with("NUM") && b[3].is_ascii_digit() {
        return VK_NUMPAD0 + (b[3] - b'0') as u16;
    }
    NAMES.iter().find(|(s, _)| *s == n).map(|(_, v)| *v).unwrap_or(0)
}

/// A `[Hotkeys]` value: `{"ReplayBuffer.Save":[...]}` (array "ReplayBuffer.Save") or `{"bindings":[...]}` for OBS's own
/// start / stop pairs (array "bindings"). Unknown keys are skipped; at most `max`.
pub fn parse_obs_hotkey(json: &str, array: &str, max: usize) -> Vec<KeyBind> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    let mut out = Vec::new();
    if let Some(arr) = v.get(array).and_then(|a| a.as_array()) {
        for e in arr {
            if out.len() >= max {
                break;
            }
            let vk = obs_key_to_vk(e.get("key").and_then(|k| k.as_str()).unwrap_or(""));
            if vk == 0 {
                continue;
            }
            let flag = |k: &str| e.get(k).map(json_true).unwrap_or(false);
            let mods = if flag("control") { MOD_C } else { 0 }
                | if flag("shift") { MOD_S } else { 0 }
                | if flag("alt") { MOD_A } else { 0 }
                | if flag("command") { MOD_W } else { 0 };
            out.push(KeyBind { vk, mods });
        }
    }
    out
}

/// ClipPing's `jbool`: true, or a non-zero number.
fn json_true(v: &serde_json::Value) -> bool {
    v.as_bool().unwrap_or_else(|| v.as_f64().is_some_and(|n| n != 0.0))
}

/// Our key -> OBS's name ("OBS_KEY_F9"); None = OBS has no name for it.
pub fn vk_to_obs_key(vk: u16) -> Option<String> {
    let vk = match vk {
        0xA0 | 0xA1 => 0x10,
        0xA2 | 0xA3 => 0x11,
        0xA4 | 0xA5 => 0x12,
        0x5C => 0x5B,
        v => v,
    };
    let t = if (b'A' as u16..=b'Z' as u16).contains(&vk) || (b'0' as u16..=b'9' as u16).contains(&vk) {
        ((vk as u8) as char).to_string()
    } else if (VK_F1..=VK_F24).contains(&vk) {
        format!("F{}", vk - VK_F1 + 1)
    } else if (VK_NUMPAD0..=VK_NUMPAD0 + 9).contains(&vk) {
        format!("NUM{}", vk - VK_NUMPAD0)
    } else {
        NAMES.iter().find(|(_, v)| *v == vk)?.0.to_string()
    };
    Some(format!("OBS_KEY_{t}"))
}

/// One OBS binding object, e.g. `{"key":"OBS_KEY_F9","control":true}` (keys.c `binding_json`, same key order).
pub fn binding_json(k: KeyBind) -> Option<String> {
    let name = vk_to_obs_key(k.vk)?;
    let mut s = format!("{{\"key\":\"{name}\"");
    if k.mods & MOD_S != 0 {
        s.push_str(",\"shift\":true");
    }
    if k.mods & MOD_C != 0 {
        s.push_str(",\"control\":true");
    }
    if k.mods & MOD_A != 0 {
        s.push_str(",\"alt\":true");
    }
    if k.mods & MOD_W != 0 {
        s.push_str(",\"command\":true");
    }
    s.push('}');
    Some(s)
}

pub fn is_mouse(vk: u16) -> bool {
    matches!(vk, 0x01 | 0x02 | 0x04 | 0x05 | 0x06)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obs_names_both_ways() {
        assert_eq!(obs_key_to_vk("OBS_KEY_F9"), 0x78);
        assert_eq!(obs_key_to_vk("OBS_KEY_F24"), 0x87);
        assert_eq!(obs_key_to_vk("OBS_KEY_A"), 0x41);
        assert_eq!(obs_key_to_vk("OBS_KEY_NUM5"), 0x65);
        assert_eq!(obs_key_to_vk("OBS_KEY_INSERT"), 0x2D);
        assert_eq!(obs_key_to_vk("OBS_KEY_MOUSE4"), 0x05);
        assert_eq!(obs_key_to_vk("KEY_A"), 0);
        assert_eq!(vk_to_obs_key(0x78).as_deref(), Some("OBS_KEY_F9"));
        assert_eq!(vk_to_obs_key(0x13).as_deref(), Some("OBS_KEY_PAUSE"));
        assert_eq!(vk_to_obs_key(0xBB).as_deref(), Some("OBS_KEY_EQUAL"));
        assert_eq!(vk_to_obs_key(0xFF), None);
    }

    #[test]
    fn hotkey_json_round_trip() {
        let v = parse_obs_hotkey(r#"{"ReplayBuffer.Save":[{"key":"OBS_KEY_INSERT","control":true},{"key":"OBS_KEY_NOPE"},{"key":"OBS_KEY_F9","shift":true,"alt":true}]}"#, "ReplayBuffer.Save", 4);
        assert_eq!(v, vec![KeyBind::new(0x2D, MOD_C), KeyBind::new(0x78, MOD_S | MOD_A)]);
        assert_eq!(binding_json(v[1]).unwrap(), r#"{"key":"OBS_KEY_F9","shift":true,"alt":true}"#);
        assert_eq!(parse_obs_hotkey(r#"{"bindings":[{"key":"OBS_KEY_F10"}]}"#, "bindings", 1), vec![KeyBind::new(0x79, 0)]);
        assert_eq!(KeyBind::unpack(0x13), KeyBind::new(0x13, 0));
        assert_eq!(KeyBind::new(0x78, 3).pack(), 0x30078);
    }
}

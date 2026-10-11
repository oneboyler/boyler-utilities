//! Order 096: which keyboard is plugged in -> which picture size it gets by itself (Full / TKL / 75 % / 60 %).
//!
//! Read-only: the HID list the Mouse tab already reads (interface attributes + product name, nothing is sent). A known
//! keyboard (maker + product id, else words in its product name) picks the size when the tab opens, unless the user ever
//! picked a size by hand (that pick stays). Unknown keyboards change nothing.

use super::prefs::Size;

/// Wooting (vendor 31E3): the product id's high byte is the model, the low byte is the USB interface (80HE shows up as
/// 1400 / 1402 ...). One = 11xx TKL, Two / Two HE = 12xx full, 60HE (all revisions) = 13xx, 80HE = 14xx (a 75 % board).
const WOOTING: u16 = 0x31E3;

/// Words in a product name (lower case, checked in this order, the first hit wins). Only layouts the maker's own product
/// line makes certain; anything else stays unknown. "Mini" is only trusted together with a maker word.
const NAMES: &[(&str, Size)] = &[
    ("tenkeyless", Size::Tkl),
    ("tkl", Size::Tkl),
    ("60%", Size::P60),
    ("75%", Size::P75),
    ("k65 rgb mini", Size::P60),
    ("wooting 80he", Size::P75),
    ("wooting 60he", Size::P60),
    ("wooting uwu", Size::P60),
    ("wooting one", Size::Tkl),
    ("wooting two", Size::Full),
    ("huntsman mini", Size::P60),
    ("huntsman tournament", Size::Tkl),
    ("huntsman v2 analog", Size::Full),
    ("huntsman", Size::Full),
    ("blackwidow", Size::Full),
    ("apex pro mini", Size::P60),
    ("apex pro", Size::Full),
    ("apex 7", Size::Full),
    ("apex 5", Size::Full),
    ("g pro x 60", Size::P60),
    ("g pro x", Size::Tkl),
    ("g pro", Size::Tkl),
    ("g915", Size::Full),
    ("g815", Size::Full),
    ("g513", Size::Full),
    ("g413", Size::Full),
    ("k65 mini", Size::P60),
    ("k65", Size::Tkl),
    ("k70", Size::Full),
    ("k95", Size::Full),
    ("k100", Size::Full),
    ("origins 60", Size::P60),
    ("alloy origins core", Size::Tkl),
    ("alloy origins", Size::Full),
    ("alloy elite", Size::Full),
    ("keychron q1", Size::P75),
    ("keychron k2", Size::P75),
    ("keychron q3", Size::Tkl),
    ("keychron k8", Size::Tkl),
    ("keychron k4", Size::Full),
    ("keychron q60", Size::P60),
    ("one 2 mini", Size::P60),
    ("one 3 mini", Size::P60),
    ("ducky one 2 sf", Size::P60),
    ("gmmk pro", Size::P75),
    ("rog falchion", Size::P60),
    ("strix scope tkl", Size::Tkl),
    ("strix scope", Size::Full),
    ("cherry mx board 3.0", Size::Full),
];

/// The size a known keyboard gets, or None (not known: nothing is changed).
pub fn known_size(vid: u16, pid: u16, name: &str) -> Option<Size> {
    if vid == WOOTING {
        match pid >> 8 {
            0x11 => return Some(Size::Tkl),
            0x12 => return Some(Size::Full),
            0x13 => return Some(Size::P60),
            0x14 => return Some(Size::P75),
            _ => {}
        }
    }
    let n = format!(" {}", name.to_lowercase());
    NAMES.iter().find(|(w, _)| n.contains(&format!(" {w}"))).map(|(_, s)| *s)
}

/// One keyboard of the HID list.
pub struct Kbd {
    pub vid: u16,
    pub pid: u16,
    pub name: String,
}

/// The size to pick for the keyboards that are plugged in: the first known one.
pub fn pick(kbds: &[Kbd]) -> Option<Size> {
    kbds.iter().find_map(|k| known_size(k.vid, k.pid, &k.name))
}

/// The keyboards on this PC now (HID usage page 1 / usage 6), read-only.
pub fn keyboards() -> Vec<Kbd> {
    bu_mouse::win::hid::list()
        .unwrap_or_default()
        .into_iter()
        .filter(|h| h.usage_page == 0x01 && h.usage == 0x06)
        .map(|h| Kbd { vid: h.vid, pid: h.pid, name: h.product.unwrap_or_default() })
        .collect()
}

/// The job's answer: the size's key, or "" for nothing known.
pub fn detect_text() -> String {
    pick(&keyboards()).map(|s| s.key().to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wooting_80he_is_75() {
        assert_eq!(known_size(0x31E3, 0x1402, "Wooting 80HE"), Some(Size::P75));
        assert_eq!(known_size(0x31E3, 0x1400, ""), Some(Size::P75));
        assert_eq!(known_size(0x31E3, 0x1300, "Wooting 60HE"), Some(Size::P60));
        assert_eq!(known_size(0x31E3, 0x1220, "Wooting Two HE"), Some(Size::Full));
    }

    #[test]
    fn the_common_brands_by_name() {
        let t = |n: &str| known_size(0x1234, 0x5678, n);
        assert_eq!(t("Razer Huntsman Mini"), Some(Size::P60));
        assert_eq!(t("Razer BlackWidow V3 Tenkeyless"), Some(Size::Tkl));
        assert_eq!(t("Razer BlackWidow V3"), Some(Size::Full));
        assert_eq!(t("CORSAIR K65 RGB MINI"), Some(Size::P60));
        assert_eq!(t("CORSAIR K70 RGB TKL Gaming Keyboard"), Some(Size::Tkl));
        assert_eq!(t("Logitech G915 TKL"), Some(Size::Tkl));
        assert_eq!(t("SteelSeries Apex Pro Mini"), Some(Size::P60));
        assert_eq!(t("Keychron Q1 Pro"), Some(Size::P75));
        assert_eq!(t("HyperX Alloy Origins 60"), Some(Size::P60));
    }

    #[test]
    fn unknown_keyboards_change_nothing() {
        assert_eq!(known_size(0x046D, 0xC31C, "USB Keyboard"), None);
        assert_eq!(known_size(0, 0, ""), None);
        assert_eq!(pick(&[]), None);
    }

    #[test]
    fn the_first_known_keyboard_wins() {
        let k = |n: &str| Kbd { vid: 1, pid: 2, name: n.into() };
        assert_eq!(pick(&[k("USB Keyboard"), k("Razer Huntsman Mini"), k("Razer BlackWidow V3")]), Some(Size::P60));
    }
}

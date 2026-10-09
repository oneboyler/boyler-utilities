//! Mouse model names by USB id (VID:PID), so a mouse is named even when Windows' own product string is only
//! "USB Receiver" or "2.4G Wireless Receiver", and so a mouse on a shared chip-maker id (3554, 258A, 25A7 ...) gets its
//! brand from the model, not from the vendor id.
//!
//! Sources (facts only - ids and names; nothing copied from any code):
//! - libratbag, MIT licence (github.com/libratbag/libratbag, `data/devices/*.device`, entries of `DeviceType=mouse` with a
//!   `usb:` match): the table [`LIBRATBAG`].
//! - pulsar-mouse-linux, MIT licence (github.com/packerlschupfer/pulsar-mouse-linux, README "Supported Mice"): the table
//!   [`PULSAR`] - the whole Pulsar family, incl. the X2 V2 / X2A Wireless on the shared id 3554.
//!
//! NOT used: OpenMouse (AGPL-3.0) - copying its tables would put the app under the AGPL (Order 009 decision, kept in Order 061).

/// Pulsar mice: (vid, pid, name). Includes Pulsar mice that sit on a chip maker's id (3554 CompX, 25A7).
pub const PULSAR: &[(u16, u16, &str)] = &[
    (0x3710, 0x1401, "Pulsar Xlite Wired"),
    (0x3710, 0x1402, "Pulsar X2 Wired"),
    (0x3710, 0x1403, "Pulsar X2H Wired Medium"),
    (0x3710, 0x1404, "Pulsar X2A Medium Wired"),
    (0x3710, 0x3401, "Pulsar Xlite V4"),
    (0x3710, 0x3414, "Pulsar X2 CrazyLight"),
    (0x3710, 0x5402, "Pulsar Xlite V4 Wireless"),
    (0x3710, 0x5404, "Pulsar Feinmann 8K (FO1)"),
    (0x3710, 0x5406, "Pulsar X2 CrazyLight"),
    (0x3710, 0x5504, "Pulsar Feinmann F01 Noctua Edition"),
    (0x3710, 0x7507, "Pulsar Feinmann F01 Noctua Edition"),
    (0x3554, 0xF507, "Pulsar X2A Wireless / X2 V2 Mini"),
    (0x3554, 0xF508, "Pulsar X2A Wireless / X2 V2 Mini"),
    (0x25A7, 0xFA7B, "Pulsar X2 Wireless"),
    (0x25A7, 0xFA7C, "Pulsar X2 Wireless"),
];

/// Other mice, sorted by (vid, pid): (vid, pid, name).
pub const LIBRATBAG: &[(u16, u16, &str)] = &[
    (0x046D, 0x0A89, "Logitech G635"),
    (0x046D, 0x0AB5, "Logitech G733 Gaming Headset"),
    (0x046D, 0x0AFE, "Logitech G733 Gaming Headset"),
    (0x046D, 0x101B, "Logitech M705"),
    (0x046D, 0x1028, "Logitech M570"),
    (0x046D, 0x400A, "Logitech M325"),
    (0x046D, 0x4011, "Logitech Wireless Touchpad"),
    (0x046D, 0x402C, "Logitech G602"),
    (0x046D, 0x4041, "Logitech MX Master"),
    (0x046D, 0x404A, "Logitech MX Anywhere 2"),
    (0x046D, 0x4052, "Logitech M545"),
    (0x046D, 0x4053, "Logitech G900"),
    (0x046D, 0x405D, "Logitech G403 Wireless"),
    (0x046D, 0x405E, "Logitech M720"),
    (0x046D, 0x4060, "Logitech MX Master"),
    (0x046D, 0x4063, "Logitech MX Anywhere 2"),
    (0x046D, 0x4067, "Logitech G903"),
    (0x046D, 0x4069, "Logitech MX Master 2S"),
    (0x046D, 0x406A, "Logitech MX Anywhere 2S"),
    (0x046D, 0x406B, "Logitech M585/M590"),
    (0x046D, 0x406C, "Logitech G603"),
    (0x046D, 0x406D, "Logitech Marathon M705"),
    (0x046D, 0x406F, "Logitech MX Ergo"),
    (0x046D, 0x4070, "Logitech G703"),
    (0x046D, 0x4071, "Logitech MX Master"),
    (0x046D, 0x4072, "Logitech MX Anywhere 2"),
    (0x046D, 0x4074, "Logitech Gaming Mouse G305"),
    (0x046D, 0x4079, "Logitech G Pro Wireless"),
    (0x046D, 0x407B, "Logitech MX Vertical"),
    (0x046D, 0x407F, "Logitech G502 Hero Wireless"),
    (0x046D, 0x4082, "Logitech MX Master 3"),
    (0x046D, 0x4085, "Logitech G604"),
    (0x046D, 0x4086, "Logitech G703 Hero"),
    (0x046D, 0x4087, "Logitech G903 Hero"),
    (0x046D, 0x4090, "Logitech MX Anywhere 3"),
    (0x046D, 0x4093, "Logitech G Pro X Wireless Superlight"),
    (0x046D, 0x4099, "Logitech G502 X PLUS"),
    (0x046D, 0x409D, "Logitech G705"),
    (0x046D, 0x409F, "Logitech G502 X Wireless"),
    (0x046D, 0x4101, "Logitech T650"),
    (0x046D, 0xC041, "G5"),
    (0x046D, 0xC048, "Logitech G9"),
    (0x046D, 0xC049, "Logitech G5"),
    (0x046D, 0xC066, "G9x [Original]"),
    (0x046D, 0xC068, "Logitech G500"),
    (0x046D, 0xC06B, "Logitech G700"),
    (0x046D, 0xC077, "Logitech MX Vertical"),
    (0x046D, 0xC07C, "Logitech G700s"),
    (0x046D, 0xC07D, "Logitech G502 Proteus Core"),
    (0x046D, 0xC07E, "Logitech G402 Gaming Mouse"),
    (0x046D, 0xC07F, "Logitech Gaming Mouse G302"),
    (0x046D, 0xC080, "Logitech Gaming Mouse G303"),
    (0x046D, 0xC081, "Logitech G900"),
    (0x046D, 0xC082, "Logitech G403 Wireless"),
    (0x046D, 0xC083, "Logitech G403"),
    (0x046D, 0xC084, "Logitech Gaming Mouse G102/G103/G203"),
    (0x046D, 0xC085, "Logitech Gaming Mouse G Pro"),
    (0x046D, 0xC086, "Logitech G903"),
    (0x046D, 0xC087, "Logitech G703"),
    (0x046D, 0xC088, "Logitech G Pro Wireless"),
    (0x046D, 0xC08A, "Logitech MX Vertical"),
    (0x046D, 0xC08B, "Logitech G502 Hero"),
    (0x046D, 0xC08C, "Logitech Gaming Mouse G Pro"),
    (0x046D, 0xC08D, "Logitech G502 Hero Wireless"),
    (0x046D, 0xC08E, "MX518"),
    (0x046D, 0xC08F, "Logitech G403 Hero"),
    (0x046D, 0xC090, "Logitech G703 Hero"),
    (0x046D, 0xC091, "Logitech G903 Hero"),
    (0x046D, 0xC092, "Logitech Gaming Mouse G102/G103/G203"),
    (0x046D, 0xC093, "Logitech M500s"),
    (0x046D, 0xC094, "Logitech G Pro X Wireless Superlight"),
    (0x046D, 0xC095, "Logitech G502 X PLUS"),
    (0x046D, 0xC096, "Logitech G705"),
    (0x046D, 0xC097, "Logitech G303 Shroud Edition"),
    (0x046D, 0xC098, "Logitech G502 X Wireless"),
    (0x046D, 0xC099, "Logitech G502 X"),
    (0x046D, 0xC09D, "Logitech Gaming Mouse G102/G103/G203"),
    (0x046D, 0xC246, "Logitech G300"),
    (0x046D, 0xC249, "Logitech G9x [Call of Duty MW3 Edition]"),
    (0x046D, 0xC24A, "Logitech G600"),
    (0x046D, 0xC24E, "Logitech G500s"),
    (0x046D, 0xC332, "Logitech G502 Proteus Spectrum"),
    (0x046D, 0xC51A, "Logitech G7"),
    (0x046D, 0xC531, "Logitech G700"),
    (0x04D9, 0xFA58, "Mars Gaming MM4"),
    (0x0B05, 0x1816, "ASUS ROG GX860 Buzzard Mouse"),
    (0x0B05, 0x1845, "ASUS ROG Gladius II"),
    (0x0B05, 0x1846, "ASUS ROG Pugio"),
    (0x0B05, 0x1847, "ASUS ROG Strix Impact"),
    (0x0B05, 0x1877, "ASUS ROG Gladius II Origin"),
    (0x0B05, 0x18B4, "ASUS ROG Strix Carry"),
    (0x0B05, 0x18CD, "ASUS ROG Gladius II Origin PNK LTD"),
    (0x0B05, 0x18E1, "ASUS ROG Strix Impact II"),
    (0x0B05, 0x18E3, "ASUS ROG Chakram"),
    (0x0B05, 0x18E5, "ASUS ROG Chakram"),
    (0x0B05, 0x1947, "ASUS ROG Strix Impact II Wireless"),
    (0x0B05, 0x1949, "ASUS ROG Strix Impact II Wireless"),
    (0x0B05, 0x1958, "ASUS ROG Chakram Core"),
    (0x0B05, 0x195C, "ASUS ROG Keris"),
    (0x0B05, 0x195E, "ASUS ROG Keris Wireless"),
    (0x0B05, 0x1960, "ASUS ROG Keris Wireless"),
    (0x0B05, 0x1977, "ASUS ROG Spatha X"),
    (0x0B05, 0x1979, "ASUS ROG Spatha X"),
    (0x0B05, 0x1A03, "ASUS TUF GAMING M4 AIR"),
    (0x0B05, 0x1A18, "ASUS ROG Chakram X"),
    (0x0B05, 0x1A1A, "ASUS ROG Chakram X"),
    (0x0B05, 0x1A66, "ASUS ROG Keris Wireless AimPoint"),
    (0x0B05, 0x1A68, "ASUS ROG Keris Wireless AimPoint"),
    (0x0B05, 0x1A88, "ASUS ROG Strix Impact III"),
    (0x0B05, 0x1A92, "ASUS ROG Harpe Wireless"),
    (0x0B05, 0x1A9B, "ASUSTeK TUF GAMING M3 GEN II"),
    (0x0B05, 0x1ACE, "ASUS ROG Omni Receiver"),
    (0x0B05, 0x1AD7, "ASUS ROG Strix Impact III Wireless"),
    (0x0B05, 0x1C56, "ASUS TUF GAMING MINI WL MOUSE MIKU"),
    (0x0B05, 0x1C57, "ASUS TUF GAMING MINI WL MOUSE MIKU"),
    (0x1038, 0x1366, "SteelSeries Kinzu V2 Pro Edition"),
    (0x1038, 0x1369, "SteelSeries Sensei Raw"),
    (0x1038, 0x1378, "SteelSeries Kinzu V2"),
    (0x1038, 0x1384, "SteelSeries Rival"),
    (0x1038, 0x1388, "SteelSeries Kinzu V3"),
    (0x1038, 0x1392, "SteelSeries Rival"),
    (0x1038, 0x1394, "SteelSeries Rival"),
    (0x1038, 0x1702, "SteelSeries Rival 100/105"),
    (0x1038, 0x170A, "SteelSeries Rival 100/105"),
    (0x1038, 0x170B, "SteelSeries Rival 100/105"),
    (0x1038, 0x170C, "SteelSeries Rival 100/105"),
    (0x1038, 0x1710, "SteelSeries Rival"),
    (0x1038, 0x1712, "SteelSeries Rival"),
    (0x1038, 0x1714, "SteelSeries Rival"),
    (0x1038, 0x1716, "SteelSeries Rival"),
    (0x1038, 0x1718, "SteelSeries Rival"),
    (0x1038, 0x171A, "SteelSeries Rival"),
    (0x1038, 0x171C, "SteelSeries Rival"),
    (0x1038, 0x1720, "SteelSeries Rival 310"),
    (0x1038, 0x1722, "SteelSeries Sensei 310"),
    (0x1038, 0x1724, "SteelSeries Rival 600"),
    (0x1038, 0x1726, "SteelSeries Rival 650 Wireless"),
    (0x1038, 0x172B, "SteelSeries Rival 650 Wireless"),
    (0x1038, 0x1814, "SteelSeries Rival 100/105"),
    (0x1D50, 0x616A, "openinput"),
    (0x1E7D, 0x2DBE, "Roccat Kone Pure"),
    (0x1E7D, 0x2DC2, "Roccat Kone Pure"),
    (0x1E7D, 0x2DCB, "Roccat Kone Pure"),
    (0x1E7D, 0x2E22, "Roccat Kone XTD"),
    (0x1E7D, 0x2E24, "Roccat Kone EMP"),
    (0x1EA7, 0x4011, "Etekcity Scroll Alpha"),
    (0x258A, 0x0012, "Nubwo x7 spectrum"),
    (0x258A, 0x0027, "SinoWealth Generic Mouse (0027)"),
    (0x258A, 0x0029, "SinoWealth Generic Mouse (0029)"),
    (0x258A, 0x0033, "Glorious Model D"),
    (0x258A, 0x0036, "Glorious Model O/O-"),
    (0x258A, 0x0051, "Sinowealth Generic Mouse (0051)"),
    (0x258A, 0x1007, "SinoWealth Generic Mouse (1007)"),
    (0x28DA, 0x3101, "G.Skill MX-780"),
    (0x3794, 0xA000, "Glorious Model O Eternal"),
];

/// The model name for a USB id, if a source above knows it.
pub fn model(vid: u16, pid: u16) -> Option<&'static str> {
    PULSAR
        .iter()
        .find(|e| e.0 == vid && e.1 == pid)
        .map(|e| e.2)
        .or_else(|| LIBRATBAG.binary_search_by_key(&(vid, pid), |e| (e.0, e.1)).ok().map(|i| LIBRATBAG[i].2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libratbag_table_is_sorted_and_unique() {
        assert!(LIBRATBAG.windows(2).all(|w| (w[0].0, w[0].1) < (w[1].0, w[1].1)));
    }

    #[test]
    fn pulsar_on_a_shared_id_is_named_by_its_pid() {
        assert_eq!(model(0x3554, 0xF507), Some("Pulsar X2A Wireless / X2 V2 Mini"));
        assert_eq!(model(0x3554, 0x0001), None);
        assert_eq!(model(0x3710, 0x5406), Some("Pulsar X2 CrazyLight"));
        assert!(model(0x046D, 0xC08B).is_some());
    }
}

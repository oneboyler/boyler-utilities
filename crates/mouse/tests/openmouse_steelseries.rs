//! Order 104 check, SteelSeries: our requests = the bytes OpenMouse sends for the same setting (encode), and answers shaped
//! the way OpenMouse reads them decode to the same values (decode). The expected bytes are worked out by hand from OpenMouse
//! (@openmouse/protocol 0.40.1: `dist/steelseries/*.js` - one codec per model line - and `dist/drivers/steelseries/*-hid.js`),
//! never from our own encoder.
//!
//! How OpenMouse counts: every request is `sendReport(0x00, payload)` - an unnumbered output report, the payload WITHOUT the
//! report id; an answer is the first input report's `event.data`, also without the id. Windows puts the report id 0 in
//! byte 0 and wants exactly OutputReportByteLength bytes: OpenMouse's payload byte i is our byte i + 1, zero-padded.

use bu_mouse::family::{Driver, Family, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::os::{HidInfo, HidTransfer};
use bu_mouse::steelseries::{self as ss, Line};

/// The report as Windows sends it: report id 0, OpenMouse's payload, zeros up to the collection's 65-byte output length.
fn rep(payload: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 65];
    v[1..1 + payload.len()].copy_from_slice(payload);
    v
}

/// The config collection of a SteelSeries mouse as Windows lists it: vendor usage page 0xFFC0, interface `mi`.
fn cfg(pid: u16, mi: u8) -> HidInfo {
    let mut h = hid_info(&format!("cfg{pid:04x}"), 0x1038, pid, 0xFFC0, 0x01, (65, 65, 0));
    h.interface = Some(mi);
    h
}

fn mouse_col(pid: u16, mi: u8) -> HidInfo {
    let mut h = hid_info(&format!("mouse{pid:04x}"), 0x1038, pid, 0x01, 0x02, (8, 0, 0));
    h.interface = Some(mi);
    h
}

// ---------------------------------------------------------------- the sensor tables

#[test]
fn dpi_tables_are_openmouses() {
    // rival3.js TRUEMOVE_CORE_DPI_TO_BYTE (Rival 3, Aerox 3): 200→0x04 … 1200→0x1b, 1600→0x24, 8500→0xc5 (pinned in its doc),
    // 800→0x12, 300→0x06, 500→0x0b (the +2/+2/+3 pattern)
    for (dpi, b) in [(200, 0x04), (300, 0x06), (500, 0x0B), (800, 0x12), (1200, 0x1B), (1600, 0x24), (3200, 0x4A), (8500, 0xC5)] {
        assert_eq!(ss::truemove_core(dpi), Some(b), "{dpi}");
    }
    for dpi in [100, 250, 8600, 0] {
        assert_eq!(ss::truemove_core(dpi), None, "{dpi} is not in the TrueMove Core table");
    }
    // rival3-wireless.js TRUEMOVE_AIR_DPI_TO_BYTE (Rival 3 Wireless, Aerox 5 / 5 Wireless / 9 Wireless, Prime Mini Wireless):
    // 100→0x00, 400→0x04, 1600→0x12, 18000→0xD6 (pinned in its doc); 200→0x02 (0x01 is skipped), 800→0x09
    for (dpi, b) in [(100, 0x00), (200, 0x02), (400, 0x04), (800, 0x09), (1600, 0x12), (3000, 0x23), (3100, 0x25), (18000, 0xD6)] {
        assert_eq!(ss::truemove_air(dpi), Some(b), "{dpi}");
    }
    for dpi in [50, 150, 18100] {
        assert_eq!(ss::truemove_air(dpi), None, "{dpi} is not in the TrueMove Air table");
    }
    // both tables only climb (a copying slip would show as a step back)
    let core: Vec<u8> = (200..=8500).step_by(100).map(|d| ss::truemove_core(d).unwrap()).collect();
    let air: Vec<u8> = (100..=18000).step_by(100).map(|d| ss::truemove_air(d).unwrap()).collect();
    assert_eq!((core.len(), air.len()), (84, 180));
    assert!(core.windows(2).all(|w| w[0] < w[1]) && air.windows(2).all(|w| w[0] < w[1]));
}

// ---------------------------------------------------------------- encode, per model line

#[test]
fn rival3_requests_are_openmouses() {
    // setDpi(d) = steelseriesRival3EncodeDpiPresets([d], 0) = [0B 00 | count 1 | selected 0 + 1 | core(d)], then save 09 00
    assert_eq!(ss::dpi_request(Line::Rival3, 1600, false), Some(vec![0x0B, 0x00, 0x01, 0x01, 0x24]));
    assert_eq!(ss::dpi_request(Line::Rival3, 1650, false), None, "off the table");
    // setPollingRate: 04 00 <v>, 1000→1, 500→2, 250→3, 125→4
    for (hz, v) in [(1000, 1), (500, 2), (250, 3), (125, 4)] {
        assert_eq!(ss::polling_request(Line::Rival3, hz, false), Some(vec![0x04, 0x00, v]), "{hz}");
    }
    assert_eq!(ss::polling_request(Line::Rival3, 2000, false), None);
    assert_eq!(ss::save_request(Line::Rival3, false), vec![0x09, 0x00]);
    // probeFirmware: 10 00
    assert_eq!(ss::probe_request(Line::Rival3, false), Some(vec![0x10, 0x00]));
}

#[test]
fn aerox3_requests_are_openmouses() {
    // 2D | count 1 | selected 0 + 1 (first_preset 1) | core(d); save 11 00; polling 2B <v> (1000→1 … 125→4)
    assert_eq!(ss::dpi_request(Line::Aerox3, 800, false), Some(vec![0x2D, 0x01, 0x01, 0x12]));
    assert_eq!(ss::polling_request(Line::Aerox3, 250, false), Some(vec![0x2B, 0x03]));
    assert_eq!(ss::polling_request(Line::Aerox3, 1000, false), Some(vec![0x2B, 0x01]));
    assert_eq!(ss::save_request(Line::Aerox3, false), vec![0x11, 0x00]);
    // aerox3.py defines no getter and no firmware query: nothing to ask
    assert_eq!(ss::probe_request(Line::Aerox3, false), None);
}

#[test]
fn rival3_wireless_requests_are_openmouses() {
    // 20 | count 1 | selected 0 + 1 | air(d) as 2 bytes little-endian (dpi_length_byte 2): 1600 → 12 00
    assert_eq!(ss::dpi_request(Line::Rival3Wireless, 1600, false), Some(vec![0x20, 0x01, 0x01, 0x12, 0x00]));
    assert_eq!(ss::dpi_request(Line::Rival3Wireless, 18000, false), Some(vec![0x20, 0x01, 0x01, 0xD6, 0x00]));
    // 17 <v> with 1000→0, 500→1, 250→2, 125→3
    for (hz, v) in [(1000, 0), (500, 1), (250, 2), (125, 3)] {
        assert_eq!(ss::polling_request(Line::Rival3Wireless, hz, false), Some(vec![0x17, v]), "{hz}");
    }
    // save is ONE byte here (09), battery query AA 01 (OpenMouse's connectivity probe for this mouse)
    assert_eq!(ss::save_request(Line::Rival3Wireless, false), vec![0x09]);
    assert_eq!(ss::probe_request(Line::Rival3Wireless, false), Some(vec![0xAA, 0x01]));
}

#[test]
fn aerox5_requests_are_openmouses() {
    // 2D | count 1 | selected 0 (first_preset 0: no + 1) | air(d); polling 2B with 1000→1 … 125→4; save 11 00
    assert_eq!(ss::dpi_request(Line::Aerox5, 1600, false), Some(vec![0x2D, 0x01, 0x00, 0x12]));
    assert_eq!(ss::polling_request(Line::Aerox5, 1000, false), Some(vec![0x2B, 0x01]));
    assert_eq!(ss::polling_request(Line::Aerox5, 125, false), Some(vec![0x2B, 0x04]));
    assert_eq!(ss::save_request(Line::Aerox5, false), vec![0x11, 0x00]);
    assert_eq!(ss::probe_request(Line::Aerox5, false), None);
}

#[test]
fn aerox_wireless_lines_are_openmouses_with_the_dongle_flag() {
    // Aerox 5 Wireless, Aerox 9 Wireless and Prime Mini Wireless share these bytes: 2D | 1 | 0 | air(d); 2B <v> with
    // 1000→0 … 125→3; save 11 00; battery 92. In 2.4 GHz mode applyWirelessFlag ORs 0x40 into byte 0: 6D, 6B, 51 00, D2.
    for line in [Line::Aerox5Wireless, Line::Aerox9Wireless, Line::PrimeMiniWireless] {
        assert_eq!(ss::dpi_request(line, 400, false), Some(vec![0x2D, 0x01, 0x00, 0x04]), "{line:?}");
        assert_eq!(ss::dpi_request(line, 400, true), Some(vec![0x6D, 0x01, 0x00, 0x04]), "{line:?}");
        assert_eq!(ss::polling_request(line, 1000, false), Some(vec![0x2B, 0x00]), "{line:?}");
        assert_eq!(ss::polling_request(line, 500, true), Some(vec![0x6B, 0x01]), "{line:?}");
        assert_eq!(ss::polling_request(line, 125, false), Some(vec![0x2B, 0x03]), "{line:?}");
        assert_eq!(ss::save_request(line, false), vec![0x11, 0x00], "{line:?}");
        assert_eq!(ss::save_request(line, true), vec![0x51, 0x00], "{line:?}");
        assert_eq!(ss::probe_request(line, false), Some(vec![0x92]), "{line:?}");
        assert_eq!(ss::probe_request(line, true), Some(vec![0xD2]), "{line:?}");
    }
}

#[test]
fn rival650_requests_are_openmouses() {
    // setDpi = sensitivity preset 1 only: 15 01 <(dpi − 100) / 100>; 800 → 7, 12000 → 0x77
    assert_eq!(ss::dpi_request(Line::Rival650, 800, false), Some(vec![0x15, 0x01, 0x07]));
    assert_eq!(ss::dpi_request(Line::Rival650, 12000, false), Some(vec![0x15, 0x01, 0x77]));
    assert_eq!(ss::dpi_request(Line::Rival650, 12100, false), None);
    assert_eq!(ss::dpi_request(Line::Rival650, 850, false), None);
    // 17 <v> with 1000→1 … 125→4 (the same command byte as the Rival 3 Wireless, the other mapping)
    assert_eq!(ss::polling_request(Line::Rival650, 250, false), Some(vec![0x17, 0x03]));
    assert_eq!(ss::polling_request(Line::Rival650, 1000, false), Some(vec![0x17, 0x01]));
    assert_eq!(ss::save_request(Line::Rival650, false), vec![0x09]);
    assert_eq!(ss::probe_request(Line::Rival650, false), Some(vec![0xAA, 0x01]));
}

#[test]
fn rival310_requests_are_openmouses() {
    // sensitivity preset 1: 53 00 01 <(dpi − 100) / 100> 00 42
    assert_eq!(ss::dpi_request(Line::Rival310, 800, false), Some(vec![0x53, 0x00, 0x01, 0x07, 0x00, 0x42]));
    assert_eq!(ss::dpi_request(Line::Rival310, 100, false), Some(vec![0x53, 0x00, 0x01, 0x00, 0x00, 0x42]));
    assert_eq!(ss::polling_request(Line::Rival310, 125, false), Some(vec![0x54, 0x00, 0x04]));
    assert_eq!(ss::save_request(Line::Rival310, false), vec![0x59, 0x00]);
    assert_eq!(ss::probe_request(Line::Rival310, false), Some(vec![0x90, 0x00]));
}

#[test]
fn prime_plus_requests_are_openmouses() {
    // 61 | count 1 | selected 0 (first_preset 0) | value little-endian, value = (dpi − 50) / 50 + 1: 800 → 16, 18000 → 0x0168
    assert_eq!(ss::dpi_request(Line::PrimePlus, 800, false), Some(vec![0x61, 0x01, 0x00, 0x10, 0x00]));
    assert_eq!(ss::dpi_request(Line::PrimePlus, 50, false), Some(vec![0x61, 0x01, 0x00, 0x01, 0x00]));
    assert_eq!(ss::dpi_request(Line::PrimePlus, 18000, false), Some(vec![0x61, 0x01, 0x00, 0x68, 0x01]));
    assert_eq!(ss::dpi_request(Line::PrimePlus, 825, false), None);
    assert_eq!(ss::polling_request(Line::PrimePlus, 500, false), Some(vec![0x5D, 0x02]));
    // save 59, one byte
    assert_eq!(ss::save_request(Line::PrimePlus, false), vec![0x59]);
    assert_eq!(ss::probe_request(Line::PrimePlus, false), None);
}

#[test]
fn sensei_ten_requests_are_openmouses() {
    // 55 00 | count flag 0xFF >> (8 − 1) = 0x01 | selected 0 + 1 | dpi / 50 little-endian: 800 → 16, 12800 → 0x0100
    assert_eq!(ss::dpi_request(Line::SenseiTen, 800, false), Some(vec![0x55, 0x00, 0x01, 0x01, 0x10, 0x00]));
    assert_eq!(ss::dpi_request(Line::SenseiTen, 12800, false), Some(vec![0x55, 0x00, 0x01, 0x01, 0x00, 0x01]));
    assert_eq!(ss::dpi_request(Line::SenseiTen, 18050, false), None);
    assert_eq!(ss::polling_request(Line::SenseiTen, 1000, false), Some(vec![0x54, 0x00, 0x01]));
    assert_eq!(ss::save_request(Line::SenseiTen, false), vec![0x59, 0x00]);
    assert_eq!(ss::probe_request(Line::SenseiTen, false), Some(vec![0x90, 0x00]));
}

#[test]
fn windows_reports_carry_the_id_and_the_padding() {
    assert_eq!(ss::report(&[0x2D, 0x01, 0x00, 0x12], 65), Some(rep(&[0x2D, 0x01, 0x00, 0x12])));
    assert_eq!(ss::report(&[0x09], 2), Some(vec![0x00, 0x09]));
    assert_eq!(ss::report(&[0x53, 0x00, 0x01, 0x07, 0x00, 0x42], 6), None, "does not fit a 6-byte report with its id");
}

// ---------------------------------------------------------------- decode

#[test]
fn battery_answers_decode_like_openmouse() {
    // Rival 3 Wireless / Rival 650 (AA 01): level = data[0], charging = data[2] != 0 - our answer bytes 1 and 3
    assert_eq!(ss::decode_battery_aa(&[77, 0x00, 0x01]), Some((Some(77), true)));
    assert_eq!(ss::decode_battery_aa(&[100, 0x55, 0x00]), Some((Some(100), false)));
    assert_eq!(ss::decode_battery_aa(&[0xFF, 0x00, 0x00]), Some((None, false)), "over 100 is no reading");
    assert_eq!(ss::decode_battery_aa(&[50, 0]), None, "shorter than three bytes");
    // Aerox 5 / 9 Wireless, Prime Mini Wireless (92): status = data[1]; charging = bit 7; level = ((status & 0x7F) − 1) × 5
    assert_eq!(ss::decode_battery_92(&[0x00, 0x8B]), Some((Some(50), true)));
    assert_eq!(ss::decode_battery_92(&[0x00, 0x15]), Some((Some(100), false)));
    assert_eq!(ss::decode_battery_92(&[0x00, 0x01]), Some((Some(0), false)));
    assert_eq!(ss::decode_battery_92(&[0x00, 0x80]), Some((None, true)), "raw 0 would be −5 %: no reading");
    assert_eq!(ss::decode_battery_92(&[0x00, 0x7F]), Some((None, false)), "over 100 is no reading");
    assert_eq!(ss::decode_battery_92(&[0x00]), None);
}

// ---------------------------------------------------------------- the allow-list

#[test]
fn only_read_requests_pass_the_allow_list() {
    for q in [vec![0x10, 0x00], vec![0x90, 0x00], vec![0xAA, 0x01], vec![0x92], vec![0xD2]] {
        assert!(ss::read_request_allowed(&rep(&q)), "{q:02x?}");
    }
    assert!(ss::read_request_allowed(&[0x00, 0x92]), "the shortest report");
    // every write any line sends - DPI, polling, lift-off, save, both link modes - is refused
    let lines = [
        Line::Rival3,
        Line::Aerox3,
        Line::Rival3Wireless,
        Line::Aerox5,
        Line::Aerox5Wireless,
        Line::Rival650,
        Line::Aerox9Wireless,
        Line::Rival310,
        Line::PrimePlus,
        Line::PrimeMiniWireless,
        Line::SenseiTen,
    ];
    for line in lines {
        for dongle in [false, true] {
            let mut writes = vec![ss::save_request(line, dongle)];
            writes.extend([100, 200, 400, 800, 1600, 3200, 12000, 18000].iter().filter_map(|d| ss::dpi_request(line, *d, dongle)));
            writes.extend([125, 250, 500, 1000].iter().filter_map(|h| ss::polling_request(line, *h, dongle)));
            for w in writes {
                assert!(!ss::read_request_allowed(&rep(&w)), "{line:?}: {w:02x?} is a write");
            }
            if let Some(q) = ss::probe_request(line, dongle) {
                assert!(ss::read_request_allowed(&rep(&q)), "{line:?}: its own read request");
            }
        }
    }
    // malformed: another report id, junk after the query, a bare 90
    let mut bad = rep(&[0x92]);
    bad[0] = 0x01;
    assert!(!ss::read_request_allowed(&bad));
    assert!(!ss::read_request_allowed(&rep(&[0x92, 0x01])));
    assert!(!ss::read_request_allowed(&rep(&[0x10, 0x00, 0x00, 0x01])));
    assert!(!ss::read_request_allowed(&[0x00, 0x90]));
    assert!(!ss::read_request_allowed(&[]));
}

// ---------------------------------------------------------------- device table + matcher

#[test]
fn the_table_is_openmouses_supported_list() {
    // supported-devices-data.ts, status "supported", every pid listed there; the line from devices.js STEELSERIES_PRODUCTS
    let want: [(u16, &str, Line, bool, bool); 25] = [
        (0x185A, "SteelSeries Aerox 9 Wireless", Line::Aerox9Wireless, false, false),
        (0x1876, "SteelSeries Aerox 9 Wireless", Line::Aerox9Wireless, false, false),
        (0x1858, "SteelSeries Aerox 9 Wireless", Line::Aerox9Wireless, true, true),
        (0x1874, "SteelSeries Aerox 9 Wireless", Line::Aerox9Wireless, true, true),
        (0x1832, "SteelSeries Sensei Ten", Line::SenseiTen, false, false),
        (0x1834, "SteelSeries Sensei Ten", Line::SenseiTen, false, false),
        (0x1850, "SteelSeries Aerox 5", Line::Aerox5, false, false),
        (0x1854, "SteelSeries Aerox 5", Line::Aerox5Wireless, false, false),
        (0x185E, "SteelSeries Aerox 5", Line::Aerox5Wireless, false, false),
        (0x1862, "SteelSeries Aerox 5", Line::Aerox5Wireless, false, false),
        (0x1852, "SteelSeries Aerox 5", Line::Aerox5Wireless, true, true),
        (0x185C, "SteelSeries Aerox 5", Line::Aerox5Wireless, true, true),
        (0x1860, "SteelSeries Aerox 5", Line::Aerox5Wireless, true, true),
        (0x1830, "SteelSeries Rival 3 Wireless", Line::Rival3Wireless, true, false),
        (0x1824, "SteelSeries Rival 3", Line::Rival3, false, false),
        (0x184C, "SteelSeries Rival 3", Line::Rival3, false, false),
        (0x172B, "SteelSeries Rival 650 Wireless", Line::Rival650, false, false),
        (0x1726, "SteelSeries Rival 650 Wireless", Line::Rival650, true, false),
        (0x1836, "SteelSeries Aerox 3", Line::Aerox3, false, false),
        (0x182C, "SteelSeries Prime+", Line::PrimePlus, false, false),
        (0x1720, "SteelSeries Rival 310", Line::Rival310, false, false),
        (0x171E, "SteelSeries Rival 310", Line::Rival310, false, false),
        (0x1736, "SteelSeries Rival 310", Line::Rival310, false, false),
        (0x184A, "SteelSeries Prime Mini Wireless", Line::PrimeMiniWireless, false, false),
        (0x1848, "SteelSeries Prime Mini Wireless", Line::PrimeMiniWireless, true, true),
    ];
    assert_eq!(ss::PRODUCTS.len(), want.len());
    for (pid, model, line, wireless, dongle) in want {
        let mi = if matches!(line, Line::Rival650 | Line::Rival310 | Line::PrimePlus | Line::SenseiTen) { 0 } else { 3 };
        let c = cfg(pid, mi);
        let m = ss::matches(&[&mouse_col(pid, 0), &c], 0x1038, pid).unwrap_or_else(|| panic!("{pid:04x}"));
        assert_eq!((m.family, m.model, m.wireless), (Family::SteelSeries, Some(model), Some(wireless)), "{pid:04x}");
        assert_eq!(m.config.path, c.path, "{pid:04x}: the config collection");
        assert_eq!(ss::line_of(m.variant), Some(line), "{pid:04x}");
        assert_eq!(m.variant & ss::DONGLE != 0, dongle, "{pid:04x}");
        assert_eq!(bu_mouse::family::find(&[mouse_col(pid, 0), c.clone()], 0x1038, pid).map(|m| m.family), Some(Family::SteelSeries));
    }
}

#[test]
fn the_matcher_claims_nothing_else() {
    // Aerox 3 Wireless (OpenMouse has a client, but its SUPPORTED entry is "pending"), Rival 3 Gen 2 / Wireless Gen 2 (not in
    // OpenMouse at all), plain Prime / Prime Wireless (devices.js leaves them out), a keyboard id - and no vid-only match
    for pid in [0x183A, 0x187A, 0x1838, 0x1878, 0x1870, 0x1872, 0x182E, 0x1842, 0x1840, 0x1612, 0x12AD] {
        assert!(ss::matches(&[&cfg(pid, 3)], 0x1038, pid).is_none(), "{pid:04x}");
    }
    assert!(ss::matches(&[&cfg(0x1824, 3)], 0x3710, 0x1824).is_none(), "another vendor");
    // the interface rivalcfg names (endpoint 3 / 0); a vendor collection with an output report there
    assert!(ss::matches(&[&cfg(0x1824, 0)], 0x1038, 0x1824).is_none(), "Rival 3 talks on interface 3");
    assert!(ss::matches(&[&cfg(0x1720, 3)], 0x1038, 0x1720).is_none(), "Rival 310 talks on interface 0");
    assert!(ss::matches(&[&mouse_col(0x1824, 3)], 0x1038, 0x1824).is_none(), "the mouse collection is no config channel");
    let mut no_out = cfg(0x1824, 3);
    no_out.output_len = 0;
    assert!(ss::matches(&[&no_out], 0x1038, 0x1824).is_none(), "no output report");
    let mut no_in = cfg(0x1830, 3);
    no_in.input_len = 0;
    assert!(ss::matches(&[&no_in], 0x1038, 0x1830).is_none(), "the battery answer needs an input report");
    // of two vendor collections, the 0xFFC0 one
    let mut other = cfg(0x1836, 3);
    other.usage_page = 0xFF00;
    other.path = "other".into();
    let c = cfg(0x1836, 3);
    assert_eq!(ss::matches(&[&other, &c], 0x1038, 0x1836).map(|m| m.config.path), Some(c.path.clone()));
    // a single-interface device (no mi_ in its path) still matches
    let mut lone = cfg(0x182C, 0);
    lone.interface = None;
    assert!(ss::matches(&[&lone], 0x1038, 0x182C).is_some());
}

#[test]
fn caps_are_what_openmouse_offers() {
    let caps = |pid: u16| ss::caps(ss::line_of(ss::PRODUCTS.iter().find(|p| p.pid == pid).unwrap().variant()).unwrap());
    // every line: OpenMouse's polling rates, no lift-off (not ported)
    for pid in [0x1824, 0x1836, 0x1830, 0x1850, 0x1858, 0x184A, 0x1726, 0x1720, 0x182C, 0x1832] {
        let c = caps(pid);
        assert_eq!((c.polling.clone(), c.lift_off.is_empty()), (vec![125, 250, 500, 1000], true), "{pid:04x}");
    }
    // DPI only where OpenMouse's setDpi changes ONE preset (Rival 650 / 310: linear 100-12000 / 100); on every other line it
    // replaces the whole preset table (Order 104 review: a profile wipe) - no DPI row there
    let c = caps(0x1726);
    assert_eq!((c.dpi.as_ref().map(|d| (d.min(), d.max())), c.battery), (Some((100, 12000)), true));
    assert!(c.dpi.as_ref().unwrap().allows(12000) && !c.dpi.as_ref().unwrap().allows(150));
    assert_eq!(caps(0x1720).dpi.map(|d| (d.min(), d.max())), Some((100, 12000)));
    for pid in [0x1824, 0x1836, 0x1830, 0x1850, 0x1858, 0x184A, 0x182C, 0x1832] {
        assert!(caps(pid).dpi.is_none(), "{pid:04x}: its DPI write would erase the saved presets");
    }
    assert!(caps(0x1830).battery && caps(0x1858).battery);
    assert!(!caps(0x1850).battery && !caps(0x1832).battery);
}

// ---------------------------------------------------------------- the driver against a scripted mouse

/// A scripted SteelSeries mouse: writes answer nothing; the read queries answer as `answer` says (`None` = silence).
fn port(info: HidInfo, answer: impl Fn(&[u8]) -> Option<Vec<u8>> + 'static) -> ScriptPort {
    ScriptPort::new(info, move |out, how| match how {
        HidTransfer::OutputOnly => Ok(Vec::new()),
        HidTransfer::OutputThenInput { .. } => answer(&out[1..]).ok_or_else(|| bu_mouse::Error::BadAnswer("no answer in time".into())),
        other => panic!("SteelSeries sends output reports only, not {other:?}"),
    })
}

/// What OpenMouse sends for one change: the setting, (50 ms), the save command.
fn change(set: &[u8], save: &[u8]) -> Vec<(Vec<u8>, HidTransfer)> {
    vec![(rep(set), HidTransfer::OutputOnly), (rep(save), HidTransfer::OutputOnly)]
}

/// (pid, interface, DPI, its payload, polling Hz, its payload, the save command)
type Case = (u16, u8, u32, &'static [u8], u32, &'static [u8], &'static [u8]);

#[test]
fn every_line_changes_settings_the_openmouse_way() {
    // (pid, interface, DPI change → OpenMouse's payload, polling change → its payload, save)
    let cases: [Case; 14] = [
        (0x1824, 3, 1600, &[0x0B, 0x00, 0x01, 0x01, 0x24], 500, &[0x04, 0x00, 0x02], &[0x09, 0x00]),
        (0x1836, 3, 800, &[0x2D, 0x01, 0x01, 0x12], 250, &[0x2B, 0x03], &[0x11, 0x00]),
        (0x1830, 3, 1600, &[0x20, 0x01, 0x01, 0x12, 0x00], 125, &[0x17, 0x03], &[0x09]),
        (0x1850, 3, 1600, &[0x2D, 0x01, 0x00, 0x12], 1000, &[0x2B, 0x01], &[0x11, 0x00]),
        (0x1854, 3, 400, &[0x2D, 0x01, 0x00, 0x04], 1000, &[0x2B, 0x00], &[0x11, 0x00]),
        (0x1852, 3, 400, &[0x6D, 0x01, 0x00, 0x04], 500, &[0x6B, 0x01], &[0x51, 0x00]),
        (0x185A, 3, 800, &[0x2D, 0x01, 0x00, 0x09], 250, &[0x2B, 0x02], &[0x11, 0x00]),
        (0x1858, 3, 800, &[0x6D, 0x01, 0x00, 0x09], 125, &[0x6B, 0x03], &[0x51, 0x00]),
        (0x184A, 3, 3200, &[0x2D, 0x01, 0x00, 0x26], 1000, &[0x2B, 0x00], &[0x11, 0x00]),
        (0x1848, 3, 3200, &[0x6D, 0x01, 0x00, 0x26], 1000, &[0x6B, 0x00], &[0x51, 0x00]),
        (0x1726, 0, 800, &[0x15, 0x01, 0x07], 250, &[0x17, 0x03], &[0x09]),
        (0x1720, 0, 800, &[0x53, 0x00, 0x01, 0x07, 0x00, 0x42], 125, &[0x54, 0x00, 0x04], &[0x59, 0x00]),
        (0x182C, 0, 800, &[0x61, 0x01, 0x00, 0x10, 0x00], 500, &[0x5D, 0x02], &[0x59]),
        (0x1832, 0, 800, &[0x55, 0x00, 0x01, 0x01, 0x10, 0x00], 1000, &[0x54, 0x00, 0x01], &[0x59, 0x00]),
    ];
    for (pid, mi, dpi, dpi_req, hz, hz_req, save) in cases {
        let info = cfg(pid, mi);
        let m = ss::matches(&[&info], 0x1038, pid).unwrap();
        let mut p = port(info, |_| None);
        // the bytes are OpenMouse's setDpi for every line (the codec)
        assert_eq!(rep(&ss::dpi_request(ss::line_of(m.variant).unwrap(), dpi, m.variant & ss::DONGLE != 0).unwrap()), change(dpi_req, save)[0].0, "{pid:04x} DPI bytes");
        if [0x1726, 0x1720].contains(&pid) {
            // write-only settings: nothing to read before or after (OpenMouse reads nothing back either)
            let c = ss::SteelSeries.write(&m, &mut p, Setting::Dpi(dpi)).unwrap();
            assert_eq!((c.now, c.before), (dpi, None), "{pid:04x}");
            assert_eq!(p.sent, change(dpi_req, save), "{pid:04x} DPI");
        } else {
            // a one-preset table + save would erase the mouse's other DPI presets: refused, nothing sent
            assert!(ss::SteelSeries.write(&m, &mut p, Setting::Dpi(dpi)).is_err(), "{pid:04x}");
            assert!(p.sent.is_empty(), "{pid:04x}");
        }
        p.sent.clear();
        let c = ss::SteelSeries.write(&m, &mut p, Setting::Polling(hz)).unwrap();
        assert_eq!((c.now, c.before), (hz, None), "{pid:04x}");
        assert_eq!(p.sent, change(hz_req, save), "{pid:04x} polling");
        p.sent.clear();
        // what OpenMouse can't store is refused before a byte leaves
        assert!(ss::SteelSeries.write(&m, &mut p, Setting::Polling(2000)).is_err(), "{pid:04x}");
        assert!(ss::SteelSeries.write(&m, &mut p, Setting::Dpi(dpi + 1)).is_err(), "{pid:04x}");
        if pid != 0x1726 {
            assert!(ss::SteelSeries.write(&m, &mut p, Setting::LiftOff(10)).is_err(), "{pid:04x}: no lift-off");
        }
        assert!(p.sent.is_empty(), "{pid:04x}");
    }
    // Rival 650: lift-off is not ported (OpenMouse's page never sends setLiftOffDistanceMm) - refused, nothing sent
    let info = cfg(0x1726, 0);
    let m = ss::matches(&[&info], 0x1038, 0x1726).unwrap();
    let mut p = port(info, |_| None);
    assert!(ss::SteelSeries.write(&m, &mut p, Setting::LiftOff(20)).is_err());
    assert!(p.sent.is_empty());
}

#[test]
fn reading_asks_only_what_openmouse_asks() {
    let read = |pid: u16, mi: u8, answer: fn(&[u8]) -> Option<Vec<u8>>| {
        let info = cfg(pid, mi);
        let m = ss::matches(&[&info], 0x1038, pid).unwrap();
        let mut p = port(info, answer);
        let r = ss::SteelSeries.read(&m, &mut p, ReadOptions::default()).unwrap();
        assert!(p.sent.iter().all(|s| ss::read_request_allowed(&s.0)), "{pid:04x}: read requests only");
        let asked: Vec<Vec<u8>> = p.sent.iter().map(|s| s.0.clone()).collect();
        (r, asked, p.sent.iter().map(|s| s.1.clone()).collect::<Vec<_>>())
    };
    // Rival 3 Wireless: battery AA 01 → a 3-byte answer [level, -, charging] (our bytes 1..4)
    let (r, asked, how) = read(0x1830, 3, |q| (q[..2] == [0xAA, 0x01]).then(|| vec![0x00, 64, 0x00, 0x01]));
    assert_eq!(asked, vec![rep(&[0xAA, 0x01])]);
    assert_eq!(how, vec![HidTransfer::OutputThenInput { reply_len: 65, max_reads: 1, timeout_ms: 500, echo: None }]);
    assert_eq!((r.online, r.battery_percent, r.charging, r.dpi, r.polling_hz, r.lift_off), (true, Some(64), Some(true), None, None, None));
    assert_eq!(r.wireless, Some(true));
    // Rival 650: the same battery shape
    let (r, asked, _) = read(0x172B, 0, |q| (q[..2] == [0xAA, 0x01]).then(|| vec![0x00, 90, 0x00, 0x00]));
    assert_eq!(asked, vec![rep(&[0xAA, 0x01])]);
    assert_eq!((r.battery_percent, r.charging, r.wireless), (Some(90), Some(false), Some(false)));
    // Aerox 9 Wireless over the dongle: D2 → [-, status], 0x8B = charging, 50 %
    let (r, asked, _) = read(0x1858, 3, |q| (q[0] == 0xD2).then(|| vec![0x00, 0x00, 0x8B]));
    assert_eq!(asked, vec![rep(&[0xD2])]);
    assert_eq!((r.online, r.battery_percent, r.charging), (true, Some(50), Some(true)));
    // Aerox 5 Wireless on its cable: 92
    let (r, asked, _) = read(0x1854, 3, |q| (q[0] == 0x92).then(|| vec![0x00, 0x00, 0x15]));
    assert_eq!(asked, vec![rep(&[0x92])]);
    assert_eq!((r.battery_percent, r.charging), (Some(100), Some(false)));
    // Prime Mini Wireless asleep behind its dongle: no answer = offline, nothing invented
    let (r, asked, _) = read(0x1848, 3, |_| None);
    assert_eq!(asked, vec![rep(&[0xD2])]);
    assert_eq!((r.online, r.battery_percent, r.charging), (false, None, None));
    // Rival 3 / Rival 310 / Sensei Ten: the firmware query is the only readable value (OpenMouse's connectivity probe)
    let (r, asked, _) = read(0x184C, 3, |q| (q[..2] == [0x10, 0x00]).then(|| vec![0x00, 0x00, 0x25]));
    assert_eq!(asked, vec![rep(&[0x10, 0x00])]);
    assert_eq!((r.online, r.battery_percent, r.dpi, r.polling_hz), (true, None, None, None));
    assert!(!r.caps.battery);
    let (r, asked, _) = read(0x1736, 0, |q| (q[..2] == [0x90, 0x00]).then(|| vec![0x00, 0x01, 0x02]));
    assert_eq!((asked, r.online), (vec![rep(&[0x90, 0x00])], true));
    let (r, asked, _) = read(0x1834, 0, |_| None);
    assert_eq!((asked, r.online), (vec![rep(&[0x90, 0x00])], false));
    // Aerox 3 / Aerox 5 / Prime+: nothing can be asked at all - no request, the caps only
    for (pid, mi) in [(0x1836, 3), (0x1850, 3), (0x182C, 0)] {
        let (r, asked, _) = read(pid, mi, |_| panic!("nothing may be asked"));
        assert!(asked.is_empty(), "{pid:04x}");
        assert_eq!((r.online, r.dpi, r.polling_hz, r.battery_percent), (true, None, None, None), "{pid:04x}");
        assert_eq!(r.caps.polling, vec![125, 250, 500, 1000]);
    }
}

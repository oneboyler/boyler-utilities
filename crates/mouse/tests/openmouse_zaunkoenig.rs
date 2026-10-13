//! Order 105 check, Zaunkoenig (M3K, M2K): our requests = the bytes OpenMouse sends for the same setting (encode), and
//! answers shaped the way OpenMouse reads them decode to the same values (decode). The expected bytes are worked out by hand
//! from OpenMouse (@openmouse/protocol 0.40.1: `dist/zaunkoenig/index.js` and `dist/drivers/zaunkoenig/hid.js`), never from
//! our own encoder.
//!
//! The settings word (zaunkoenigEncodeConfigWord): `(dpi / step - 1) & 0x1ff` | `rawLod << 9` (M3K: mm - 1, M2K: mm) |
//! `pollingCode << 11` (8000/4000/2000/1000 Hz = 0/1/2/3) | 0x2000 USB high speed | 0x4000 angle snapping | 0x8000 primary
//! right. OpenMouse sends it with `sendFeatureReport(3, [lo, hi, 0, 0])`; Windows puts the report id in front: `[3, lo, hi, 0, 0]`.

use bu_mouse::error::Error;
use bu_mouse::family::{self, Change, Driver, Extra, Family, Kind, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::os::{HidInfo, HidTransfer};
use bu_mouse::zaunkoenig as z;
use std::cell::RefCell;
use std::rc::Rc;

const FIRMWARE: &str = "parawizard new v0.8 (test)";

/// A report from its leading bytes, zero-padded to the 8-byte feature report of `control()`.
fn padded(b: &[u8]) -> Vec<u8> {
    let mut v = b.to_vec();
    v.resize(8, 0);
    v
}

fn control(pid: u16) -> HidInfo {
    let mut h = hid_info(&format!("ctl{pid:04x}"), 0x0483, pid, 0xFF00, 0x01, (0, 0, 8));
    h.interface = Some(1);
    h
}

/// The mouse as the OpenMouse client sees it: a firmware text (report 2) and one settings word (report 3).
struct Mouse {
    word: u16,
    firmware: String,
    /// the mouse ignores writes (keeps the word)
    deaf: bool,
    seen: Vec<Vec<u8>>,
}

fn port_for(info: &HidInfo, m: Rc<RefCell<Mouse>>) -> ScriptPort {
    ScriptPort::new(info.clone(), move |out, how| {
        let mut m = m.borrow_mut();
        m.seen.push(out.to_vec());
        match how {
            HidTransfer::FeatureIn { reply_id: 2, .. } => {
                let mut v = vec![2u8];
                v.extend(m.firmware.bytes());
                v.resize(40, 0);
                Ok(v)
            }
            HidTransfer::FeatureIn { reply_id: 3, .. } => Ok(vec![3, m.word as u8, (m.word >> 8) as u8, 0, 0, 0, 0, 0]),
            HidTransfer::FeatureOut => {
                assert_eq!((out[0], out.len()), (3, 8), "a config write is report 3, zero-padded to the collection's 8-byte feature report");
                assert!(out[3..].iter().all(|b| *b == 0), "the control word and the padding stay 0 (0xFFFF would be the factory reset)");
                if !m.deaf {
                    m.word = out[1] as u16 | ((out[2] as u16) << 8);
                }
                Ok(Vec::new())
            }
            other => panic!("unexpected transfer {other:?}"),
        }
    })
}

fn mouse(word: u16) -> Rc<RefCell<Mouse>> {
    Rc::new(RefCell::new(Mouse { word, firmware: FIRMWARE.into(), deaf: false, seen: Vec::new() }))
}

/// M3K: 800 DPI (800 / 50 - 1 = 15 = 0x00f), 2 mm (raw 1 -> 0x200), 1000 Hz code 3 (0x1800), full speed:
/// 0x00f | 0x200 | 0x1800 = 0x1a0f.
const M3K_BASE: u16 = 0x1A0F;
/// M2K: 800 DPI (800 / 100 - 1 = 7), 2 mm (raw 2 -> 0x400), 1000 Hz code 3 (0x1800): 0x007 | 0x400 | 0x1800 = 0x1c07.
const M2K_BASE: u16 = 0x1C07;

// ---------------------------------------------------------------- the word

#[test]
fn the_m3k_word_is_openmouses() {
    let d = z::device(0xA462).unwrap();
    let c = z::decode_word(M3K_BASE, d).unwrap();
    // (0x1a0f & 0x1ff) + 1 = 16 -> 16 * 50 = 800; (0x1a0f >> 9) & 3 = 13 & 3 = 1 -> M3K mm = 2; (0x1a0f >> 11) & 3 = 3 -> code 3
    assert_eq!((c.dpi, c.lift_off_mm, c.polling_code, c.high_speed, c.polling_hz), (800, 2, 3, false, 1000));
    assert!(!c.angle_snapping && !c.primary_right);
    assert_eq!(z::encode_word(&c, d).unwrap(), M3K_BASE);
    // 1600 DPI: 1600 / 50 - 1 = 31 = 0x01f -> 0x01f | 0x200 | 0x1800
    assert_eq!(z::encode_word(&z::Config { dpi: 1600, ..c }, d).unwrap(), 0x1A1F);
    // 3 mm: raw 2 -> 2 << 9 = 0x400
    assert_eq!(z::encode_word(&z::Config { lift_off_mm: 3, ..c }, d).unwrap(), 0x1C0F);
    // 1 mm: raw 0
    assert_eq!(z::encode_word(&z::Config { lift_off_mm: 1, ..c }, d).unwrap(), 0x180F);
    // 8000 Hz on high speed: code 0, bit 0x2000 -> 0x00f | 0x200 | 0 | 0x2000
    assert_eq!(z::encode_word(&z::Config { high_speed: true, polling_hz: 8000, polling_code: 0, ..c }, d).unwrap(), 0x220F);
    // 4000 Hz: code 1 << 11 = 0x800; 2000 Hz: code 2 << 11 = 0x1000
    assert_eq!(z::encode_word(&z::Config { high_speed: true, polling_hz: 4000, polling_code: 1, ..c }, d).unwrap(), 0x2A0F);
    assert_eq!(z::encode_word(&z::Config { high_speed: true, polling_hz: 2000, polling_code: 2, ..c }, d).unwrap(), 0x320F);
    // the 20 000 DPI ceiling: 20000 / 50 - 1 = 399 = 0x18f
    assert_eq!(z::encode_word(&z::Config { dpi: 20_000, ..c }, d).unwrap() & 0x1FF, 0x18F);
    // angle snapping 0x4000 and the primary button 0x8000 are copied through
    let both = z::decode_word(M3K_BASE | 0xC000, d).unwrap();
    assert!(both.angle_snapping && both.primary_right);
    assert_eq!(z::encode_word(&both, d).unwrap(), M3K_BASE | 0xC000);
}

#[test]
fn the_m2k_word_is_openmouses() {
    let d = z::device(0xA3CF).unwrap();
    let c = z::decode_word(M2K_BASE, d).unwrap();
    // (0x1c07 & 0x1ff) + 1 = 8 -> 8 * 100 = 800; (0x1c07 >> 9) & 3 = 14 & 3 = 2 -> M2K mm = raw = 2
    assert_eq!((c.dpi, c.lift_off_mm, c.polling_code), (800, 2, 3));
    assert_eq!(z::encode_word(&c, d).unwrap(), M2K_BASE);
    // 3 mm: raw 3 -> 0x600; 1600 DPI: 1600 / 100 - 1 = 15
    assert_eq!(z::encode_word(&z::Config { lift_off_mm: 3, ..c }, d).unwrap(), 0x1E07);
    assert_eq!(z::encode_word(&z::Config { dpi: 1600, ..c }, d).unwrap(), 0x1C0F);
    // 12 000 DPI: 12000 / 100 - 1 = 119 = 0x077
    assert_eq!(z::encode_word(&z::Config { dpi: 12_000, ..c }, d).unwrap() & 0x1FF, 0x77);
    // 1 mm does not exist on the M2K (raw 1 -> mm 1 is refused by the decoder, 1 mm by the encoder)
    assert!(z::encode_word(&z::Config { lift_off_mm: 1, ..c }, d).is_err());
    assert!(z::decode_word(0x1A07, d).is_err(), "raw 1 = 1 mm is not an M2K value");
}

#[test]
fn the_encoder_refuses_what_openmouse_refuses() {
    let m3k = z::device(0xA462).unwrap();
    let m2k = z::device(0xA3CF).unwrap();
    let c = z::decode_word(M3K_BASE, m3k).unwrap();
    for dpi in [0, 25, 20_050, 825, 49] {
        assert!(z::encode_word(&z::Config { dpi, ..c }, m3k).is_err(), "{dpi} on the M3K");
    }
    for dpi in [50, 150, 12_100] {
        assert!(z::encode_word(&z::Config { dpi, ..z::decode_word(M2K_BASE, m2k).unwrap() }, m2k).is_err(), "{dpi} on the M2K");
    }
    assert!(z::encode_word(&z::Config { polling_hz: 500, ..c }, m3k).is_err());
    // USB full speed carries only 1000 Hz
    assert!(z::encode_word(&z::Config { polling_hz: 4000, ..c }, m3k).is_err());
    assert!(z::encode_word(&z::Config { polling_code: 4, ..c }, m3k).is_err());
    assert!(z::encode_word(&z::Config { lift_off_mm: 4, ..c }, m3k).is_err());
}

#[test]
fn full_speed_keeps_the_interval_bits_and_reads_1000_hz() {
    let d = z::device(0xA462).unwrap();
    // full speed with code 1 (4000 Hz) stored: reads 1000 Hz, keeps code 1, and the word encodes back to itself
    let word = 0x000F | 0x0200 | (1 << 11);
    let c = z::decode_word(word, d).unwrap();
    assert_eq!((c.polling_hz, c.polling_code, c.high_speed), (1000, 1, false));
    assert_eq!(z::encode_word(&c, d).unwrap(), word);
}

#[test]
fn reports_decode_the_openmouse_way() {
    let d = z::device(0xA462).unwrap();
    // zaunkoenigDecodeVersionReport: a leading report id 2 is skipped, text runs to the first 0, trimmed
    let mut v = vec![2u8];
    v.extend(b"  parawizard new v0.8 x ");
    v.extend([0, b'z', b'z']);
    assert_eq!(z::decode_version(&v), "parawizard new v0.8 x");
    assert_eq!(z::decode_version(b"parawizard new v0.8"), "parawizard new v0.8", "no id, no terminator");
    assert!(z::firmware_is_supported("xx parawizard new v0.8 yy") && !z::firmware_is_supported("parawizard new v0.7"));
    // zaunkoenigDecodeConfigReport: id 3 skipped, little-endian word
    assert_eq!(z::decode_config_report(&[3, 0x0F, 0x1A, 0, 0], d).unwrap().dpi, 800);
    assert_eq!(z::decode_config_report(&[0x0F, 0x1A], d).unwrap().dpi, 800, "without the id too");
    assert!(z::decode_config_report(&[3, 0x0F], d).is_err());
    // the write: sendFeatureReport(3, [lo, hi, 0, 0]) with the id in front
    assert_eq!(z::config_report(0x1A1F, 5), vec![3, 0x1F, 0x1A, 0, 0]);
    assert_eq!(z::config_report(0x1A1F, 8), padded(&[3, 0x1F, 0x1A]), "zero-padded to the collection's feature size");
    assert_eq!(z::config_report(0x1A1F, 2), vec![3, 0x1F, 0x1A, 0, 0], "never shorter than the 5-byte payload");
}

// ---------------------------------------------------------------- the device table

#[test]
fn the_device_table_is_the_supported_list() {
    for (pid, name, step, max) in [(0xA462u16, "Zaunkoenig M3K", 50u32, 20_000u32), (0xA3CF, "Zaunkoenig M2K", 100, 12_000)] {
        let info = control(pid);
        let m = z::matches(&[&info], 0x0483, pid).unwrap_or_else(|| panic!("{pid:04x}"));
        assert_eq!((m.family, m.model, m.wireless, m.config.path.as_str()), (Family::Zaunkoenig, Some(name), Some(false), info.path.as_str()));
        let caps = z::caps(z::device(pid).unwrap());
        let dpi = caps.dpi.unwrap();
        assert_eq!((dpi.min(), dpi.max()), (step, max));
        assert!(dpi.allows(step) && dpi.allows(max) && !dpi.allows(max + step));
        assert_eq!(caps.polling, vec![1000, 2000, 4000, 8000]);
        assert!(!caps.battery);
        // the same through the family registry
        let f = family::find(std::slice::from_ref(&info), 0x0483, pid).unwrap();
        assert_eq!(f.family, Family::Zaunkoenig);
    }
    assert_eq!(z::caps(z::device(0xA462).unwrap()).lift_off, vec![10, 20, 30]);
    assert_eq!(z::caps(z::device(0xA3CF).unwrap()).lift_off, vec![20, 30]);
    // a wrong product id, a wrong vendor id, a collection that is not the vendor one or has no feature reports
    assert!(z::matches(&[&control(0xA463)], 0x0483, 0xA463).is_none());
    assert!(z::matches(&[&control(0xA462)], 0x0484, 0xA462).is_none());
    let mouse_col = hid_info("mi_00", 0x0483, 0xA462, 0x01, 0x02, (8, 0, 0));
    assert!(z::matches(&[&mouse_col], 0x0483, 0xA462).is_none());
    let mut thin = control(0xA462);
    thin.feature_len = 0;
    assert!(z::matches(&[&thin], 0x0483, 0xA462).is_none());
    // the mouse collection is skipped for the vendor one
    let m = z::matches(&[&mouse_col, &control(0xA462)], 0x0483, 0xA462).unwrap();
    assert_eq!(m.config.usage_page, 0xFF00);
}

#[test]
fn no_other_family_claims_a_zaunkoenig_id() {
    for pid in [0xA462u16, 0xA3CF] {
        let info = control(pid);
        let own = [&info];
        assert!(bu_mouse::pulsar::matches_cmouse(&own, 0x0483, pid).is_none());
        assert!(bu_mouse::pulsar::matches_xs1(&own, 0x0483, pid).is_none());
        assert!(bu_mouse::logitech::matches(&own, 0x0483, pid).is_none());
        assert!(bu_mouse::razer::matches(&own, 0x0483, pid).is_none());
        assert!(bu_mouse::steelseries::matches(&own, 0x0483, pid).is_none());
    }
}

// ---------------------------------------------------------------- the driver

#[test]
fn reading_asks_the_firmware_then_the_word() {
    let info = control(0xA462);
    let m = z::matches(&[&info], 0x0483, 0xA462).unwrap();
    let mouse = mouse(M3K_BASE);
    let mut port = port_for(&info, mouse.clone());
    let s = z::Zaunkoenig.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((s.online, s.dpi, s.polling_hz, s.lift_off, s.battery_percent, s.wireless), (true, Some(800), Some(1000), Some(20), None, Some(false)));
    assert_eq!(s.caps, z::caps(z::device(0xA462).unwrap()));
    assert_eq!(mouse.borrow().seen, vec![vec![2u8], vec![3u8]]);
    assert_eq!(port.sent[0].1, HidTransfer::FeatureIn { reply_id: 2, reply_len: 8, wait_ms: 0 });
    assert!(port.sent.iter().all(|s| matches!(s.1, HidTransfer::FeatureIn { .. })));
}

#[test]
fn firmware_without_the_marker_is_refused_like_openmouse() {
    let info = control(0xA462);
    let m = z::matches(&[&info], 0x0483, 0xA462).unwrap();
    let mouse = mouse(M3K_BASE);
    mouse.borrow_mut().firmware = "parawizard old v0.6".into();
    let mut port = port_for(&info, mouse.clone());
    let e = z::Zaunkoenig.read(&m, &mut port, ReadOptions::default()).unwrap_err();
    assert!(matches!(&e, Error::UnsupportedMouse(t) if t.contains("parawizard old v0.6") && t.contains("parawizard new v0.8")), "{e}");
    assert_eq!(port.sent.len(), 1, "the settings word is not read");
}

#[test]
fn setting_dpi_reads_writes_and_reads_back() {
    let info = control(0xA462);
    let m = z::matches(&[&info], 0x0483, 0xA462).unwrap();
    let mouse = mouse(M3K_BASE | 0x4000);
    let mut port = port_for(&info, mouse.clone());
    let c = z::Zaunkoenig.write(&m, &mut port, Setting::Dpi(1600)).unwrap();
    assert_eq!((c.now, c.before), (1600, Some(800)));
    // read word, write word (angle snapping copied through: 0x5a1f), read word
    assert_eq!(mouse.borrow().seen, vec![vec![3], padded(&[3, 0x1F, 0x5A]), vec![3]]);
    assert_eq!(port.sent[1].1, HidTransfer::FeatureOut);
    assert_eq!(mouse.borrow().word, 0x5A1F);
}

#[test]
fn setting_polling_above_1000_switches_to_high_speed_like_openmouse() {
    let info = control(0xA462);
    let m = z::matches(&[&info], 0x0483, 0xA462).unwrap();
    let mouse = mouse(M3K_BASE);
    let mut port = port_for(&info, mouse.clone());
    let c = z::Zaunkoenig.write(&m, &mut port, Setting::Polling(8000)).unwrap();
    assert_eq!((c.now, c.before), (8000, Some(1000)));
    assert_eq!(mouse.borrow().seen[1], padded(&[3, 0x0F, 0x22]), "code 0 << 11, high speed 0x2000, dpi 800, 2 mm: 0x220f");
    // back to 1000 Hz: stays on high speed (usbSpeed: config.usbSpeed), code 3: 0x00f | 0x200 | 0x1800 | 0x2000 = 0x3a0f
    mouse.borrow_mut().seen.clear();
    let c = z::Zaunkoenig.write(&m, &mut port, Setting::Polling(1000)).unwrap();
    assert_eq!((c.now, c.before), (1000, Some(8000)));
    assert_eq!(mouse.borrow().seen[1], padded(&[3, 0x0F, 0x3A]));
    assert!(z::Zaunkoenig.write(&m, &mut port, Setting::Polling(500)).is_err());
}

#[test]
fn setting_lift_off_takes_whole_millimetres() {
    let info = control(0xA3CF);
    let m = z::matches(&[&info], 0x0483, 0xA3CF).unwrap();
    let mouse = mouse(M2K_BASE);
    let mut port = port_for(&info, mouse.clone());
    let c = z::Zaunkoenig.write(&m, &mut port, Setting::LiftOff(30)).unwrap();
    assert_eq!((c.now, c.before), (30, Some(20)));
    assert_eq!(mouse.borrow().seen[1], padded(&[3, 0x07, 0x1E]), "M2K raw = mm = 3 -> 0x600: 0x1e07");
    mouse.borrow_mut().seen.clear();
    assert!(z::Zaunkoenig.write(&m, &mut port, Setting::LiftOff(10)).is_err(), "1 mm is not an M2K value");
    assert!(z::Zaunkoenig.write(&m, &mut port, Setting::LiftOff(25)).is_err());
    assert_eq!(mouse.borrow().seen.len(), 2, "two refused changes: one read each, no write");
    assert!(mouse.borrow().seen.iter().all(|s| s == &vec![3]));
}

#[test]
fn an_unchanged_value_is_not_written_again() {
    let info = control(0xA462);
    let m = z::matches(&[&info], 0x0483, 0xA462).unwrap();
    let mouse = mouse(M3K_BASE);
    let mut port = port_for(&info, mouse.clone());
    let c = z::Zaunkoenig.write(&m, &mut port, Setting::Dpi(800)).unwrap();
    assert_eq!((c.now, c.before), (800, Some(800)));
    assert_eq!(mouse.borrow().seen, vec![vec![3u8], vec![3u8]], "OpenMouse sends no write when the word is the same");
}

#[test]
fn a_mouse_that_does_not_keep_the_word_is_an_error() {
    let info = control(0xA462);
    let m = z::matches(&[&info], 0x0483, 0xA462).unwrap();
    let mouse = mouse(M3K_BASE);
    mouse.borrow_mut().deaf = true;
    let mut port = port_for(&info, mouse.clone());
    let e = z::Zaunkoenig.write(&m, &mut port, Setting::Dpi(1600)).unwrap_err();
    assert!(matches!(&e, Error::BadAnswer(t) if t.contains("did not retain")), "{e}");
    assert_eq!(mouse.borrow().word, M3K_BASE);
}

#[test]
fn out_of_range_values_send_no_write() {
    let info = control(0xA462);
    let m = z::matches(&[&info], 0x0483, 0xA462).unwrap();
    let mouse = mouse(M3K_BASE);
    let mut port = port_for(&info, mouse.clone());
    for dpi in [0, 30, 825, 20_050] {
        assert!(matches!(z::Zaunkoenig.write(&m, &mut port, Setting::Dpi(dpi)), Err(Error::OutOfRange { .. })), "{dpi}");
    }
    assert!(port.sent.iter().all(|s| s.1 != HidTransfer::FeatureOut));
}

// ---------------------------------------------------------------- the allow-list

#[test]
fn no_frame_passes_the_allow_list_and_reads_are_feature_in_only() {
    // both reads are HidD_GetFeature (a FeatureIn: nothing is sent, the transfer kind itself passes the gate); the one-byte
    // [2] / [3] that is the `out` of such a read must not pass as a SetFeature
    assert!(!z::read_request_allowed(&[2]) && !z::read_request_allowed(&[3]));
    // the write (report 3 with a payload), the factory reset OpenMouse can send ([3, 0, 0, 0xff, 0xff]) and anything else
    assert!(!z::read_request_allowed(&[3, 0x1F, 0x1A, 0, 0]));
    assert!(!z::read_request_allowed(&[3, 0, 0, 0xFF, 0xFF]));
    assert!(!z::read_request_allowed(&[]) && !z::read_request_allowed(&[4]) && !z::read_request_allowed(&[2, 0]));
    assert!(!family::read_request_allowed(Family::Zaunkoenig, &[3]) && !family::read_request_allowed(Family::Zaunkoenig, &[3, 1, 2, 0, 0]));
    // a read() sends FeatureIn transfers only; the write has exactly one FeatureOut
    let info = control(0xA3CF);
    let m = z::matches(&[&info], 0x0483, 0xA3CF).unwrap();
    let mut port = port_for(&info, mouse(M2K_BASE));
    z::Zaunkoenig.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert!(port.sent.iter().all(|s| matches!(s.1, HidTransfer::FeatureIn { .. })));
    port.sent.clear();
    z::Zaunkoenig.write(&m, &mut port, Setting::Dpi(1600)).unwrap();
    let writes: Vec<_> = port.sent.iter().filter(|s| s.1 == HidTransfer::FeatureOut).collect();
    assert_eq!(writes.len(), 1);
    assert!(!z::read_request_allowed(&writes[0].0));
}

#[test]
fn the_write_is_the_full_feature_report_with_zero_control_bytes() {
    // a collection whose feature reports are 20 bytes (report 2 holds the firmware text): the write is padded to 20, bytes 3-4 are 0
    let mut info = control(0xA462);
    info.feature_len = 20;
    let m = z::matches(&[&info], 0x0483, 0xA462).unwrap();
    let mouse = mouse(M3K_BASE);
    let sent = Rc::new(RefCell::new(Vec::<Vec<u8>>::new()));
    let (s2, m2) = (sent.clone(), mouse.clone());
    let mut port = ScriptPort::new(info.clone(), move |out, how| {
        if *how == HidTransfer::FeatureOut {
            s2.borrow_mut().push(out.to_vec());
            m2.borrow_mut().word = out[1] as u16 | ((out[2] as u16) << 8);
            return Ok(Vec::new());
        }
        let w = m2.borrow().word;
        Ok(vec![3, w as u8, (w >> 8) as u8, 0, 0, 0, 0, 0])
    });
    z::Zaunkoenig.write(&m, &mut port, Setting::Dpi(1600)).unwrap();
    let w = sent.borrow();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].len(), 20);
    assert_eq!(w[0][..3], [3, 0x1F, 0x1A]);
    assert!(w[0][3..].iter().all(|b| *b == 0), "control bytes 3-4 (0xFFFF = factory reset) and the padding are 0");
}

#[test]
fn a_saved_dpi_above_the_models_maximum_does_not_block_polling_or_lift_off() {
    // DPI field 0x1FF = (511 + 1) * 50 = 25600 DPI > 20 000: encode_word refuses that, but it has nothing to do with these changes
    let word: u16 = 0x1FF | 0x200 | 0x1800;
    let info = control(0xA462);
    let m = z::matches(&[&info], 0x0483, 0xA462).unwrap();
    let mouse = mouse(word);
    let mut port = port_for(&info, mouse.clone());
    let c = z::Zaunkoenig.write(&m, &mut port, Setting::Polling(4000)).unwrap();
    assert_eq!((c.now, c.before), (4000, Some(1000)));
    // code 1 << 11, high speed 0x2000, lift 2 mm (0x200), the stored DPI bits untouched
    assert_eq!(mouse.borrow().word, 0x1FF | 0x200 | 0x800 | 0x2000);
    let c = z::Zaunkoenig.write(&m, &mut port, Setting::LiftOff(30)).unwrap();
    assert_eq!((c.now, c.before), (30, Some(20)));
    assert_eq!(mouse.borrow().word & 0x1FF, 0x1FF);
    assert_eq!((mouse.borrow().word >> 9) & 3, 2);
    // a DPI change to a valid value still works and replaces the odd value
    let c = z::Zaunkoenig.write(&m, &mut port, Setting::Dpi(1600)).unwrap();
    assert_eq!((c.now, c.before), (1600, Some(25_600)));
    // and a value that is out of range is still refused with the DPI error
    assert!(matches!(z::Zaunkoenig.write(&m, &mut port, Setting::Dpi(25_600)), Err(Error::OutOfRange { .. })));
}

// ---------------------------------------------------------------- Order 110: angle snapping (the client's only one of the order's settings)

#[test]
fn angle_snapping_is_bit_14_of_the_word() {
    let info = control(0xA462);
    let m = z::matches(&[&info], 0x0483, 0xA462).unwrap();
    // read: angleSnapping = Boolean(word & 0x4000)
    let mo = mouse(M3K_BASE | 0x4000);
    let s = z::Zaunkoenig.read(&m, &mut port_for(&info, mo), ReadOptions::default()).unwrap();
    assert_eq!(s.extras.iter().map(|e| (e.extra, e.value, e.kind.clone())).collect::<Vec<_>>(), vec![(Extra::AngleSnap, Some(1), Kind::Switch)]);
    let s = z::Zaunkoenig.read(&m, &mut port_for(&info, mouse(M3K_BASE)), ReadOptions::default()).unwrap();
    assert_eq!(s.extra(Extra::AngleSnap), Some(0));
    // setAngleSnapping(true) on 0x1a0f: update = read, write 0x1a0f | 0x4000 = 0x5a0f as [3, 0x0F, 0x5A, 0, 0 …], read back
    let mo = mouse(M3K_BASE);
    let mut port = port_for(&info, mo.clone());
    z::Zaunkoenig.change(&m, &mut port, Change::Extra(Extra::AngleSnap, 1)).unwrap();
    assert_eq!(mo.borrow().seen, vec![vec![3], padded(&[3, 0x0F, 0x5A]), vec![3]]);
    assert_eq!(port.sent[1].1, HidTransfer::FeatureOut);
    // off again: 0x1a0f = [3, 0x0F, 0x1A]
    z::Zaunkoenig.change(&m, &mut port, Change::Extra(Extra::AngleSnap, 0)).unwrap();
    assert_eq!(mo.borrow().seen[4], padded(&[3, 0x0F, 0x1A]));
    // unchanged: no write (wantedWord === the current word)
    z::Zaunkoenig.change(&m, &mut port, Change::Extra(Extra::AngleSnap, 0)).unwrap();
    assert_eq!(mo.borrow().seen.len(), 8);
    // a mouse that does not keep it
    mo.borrow_mut().deaf = true;
    assert!(z::Zaunkoenig.change(&m, &mut port, Change::Extra(Extra::AngleSnap, 1)).is_err());
    // refused before a byte leaves
    let mo = mouse(M3K_BASE);
    let mut port = port_for(&info, mo.clone());
    for c in [Change::Extra(Extra::AngleSnap, 2), Change::Extra(Extra::MotionSync, 1), Change::Extra(Extra::Debounce, 4)] {
        assert!(z::Zaunkoenig.change(&m, &mut port, c).is_err(), "{c:?}");
    }
    assert!(mo.borrow().seen.is_empty());
}

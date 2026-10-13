//! Order 105 check, moddoMOUSE: our feature reports = the bytes OpenMouse sends for the same setting (encode), and answers
//! shaped the way OpenMouse reads them decode to the same values (decode). Expected bytes are worked out by hand from OpenMouse
//! (@openmouse/protocol 0.40.1: `dist/moddo/index.js`, `dist/drivers/moddo/hid.js`).
//!
//! OpenMouse's settings report is 11 bytes: `[polling u16le | lift | dpiX u16le | dpiY u16le | invert/swap | angle i16le |
//! deep sleep]`, sent as feature report 0x02 (Windows: the id 0x02 in front, zero padding up to the collection's size).

use bu_mouse::family::{self, Driver, Family, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::moddo as md;
use bu_mouse::os::{HidInfo, HidTransfer};
use bu_mouse::Error;
use std::cell::RefCell;
use std::rc::Rc;

/// The device says it is a moddo mouse (its product string).
fn named(mut h: HidInfo) -> HidInfo {
    h.product = Some("moddoMOUSE".into());
    h
}

fn cfg(pid: u16, usage: u16) -> HidInfo {
    named(hid_info("moddo-cfg", 0x2FE3, pid, 0x00FF, usage, (0, 0, 64)))
}

/// polling 1000 (E8 03) · lift 1 · dpiX 1600 (40 06) · dpiY 1600 (40 06) · invert/swap 5 · angle −3 (FD FF) · deep sleep 10
const CONFIG: [u8; 11] = [0xE8, 0x03, 0x01, 0x40, 0x06, 0x40, 0x06, 0x05, 0xFD, 0xFF, 0x0A];

// ---------------------------------------------------------------- codec

#[test]
fn the_settings_report_is_openmouses_layout() {
    let c = md::decode_config(&CONFIG).unwrap();
    assert_eq!((c.polling, c.lift, c.dpi_x, c.dpi_y, c.invert_swap, c.angle, c.deep_sleep), (1000, 1, 1600, 1600, 5, -3, 10));
    assert_eq!(md::encode_config(&c), CONFIG, "decode then encode gives the same 11 bytes");
    // setDpi(3200 = 0x0C80) on that report: only the two DPI fields change
    let mut want = CONFIG;
    want[3..7].copy_from_slice(&[0x80, 0x0C, 0x80, 0x0C]);
    assert_eq!(md::encode_config(&md::Config { dpi_x: 3200, dpi_y: 3200, ..c }), want);
    // setPollingRate(250 = 0x00FA) and setLiftOffDistance(High → 2)
    let mut want = CONFIG;
    want[0..2].copy_from_slice(&[0xFA, 0x00]);
    assert_eq!(md::encode_config(&md::Config { polling: 250, ..c }), want);
    let mut want = CONFIG;
    want[2] = 2;
    assert_eq!(md::encode_config(&md::Config { lift: 2, ..c }), want);
}

#[test]
fn a_report_that_is_not_ready_decodes_to_nothing() {
    // decodeModdoConfig: shorter than 7 bytes, a zeroed report (polling 0), a DPI below 50 or above 26000 → null
    assert!(md::decode_config(&[0u8; 11]).is_none());
    assert!(md::decode_config(&CONFIG[..6]).is_none());
    let mut low = CONFIG;
    low[3..5].copy_from_slice(&[49, 0]);
    assert!(md::decode_config(&low).is_none());
    let mut high = CONFIG;
    high[3..5].copy_from_slice(&26050u16.to_le_bytes());
    assert!(md::decode_config(&high).is_none());
    // a 7-byte report is enough to read; the missing fields read as 0
    let c = md::decode_config(&CONFIG[..7]).unwrap();
    assert_eq!((c.invert_swap, c.angle, c.deep_sleep), (0, 0, 0));
}

#[test]
fn battery_and_firmware_decode_like_openmouse() {
    // [remaining, -, status, charger flags]: bit 0 of the flags = a cell is present
    assert_eq!(md::decode_battery(&[87, 0, 0x45, 1]), Some((Some(87), false)), "0x45 discharging");
    assert_eq!(md::decode_battery(&[55, 0, 0x44, 3]), Some((Some(55), true)), "0x44 charging");
    assert_eq!(md::decode_battery(&[100, 0, 0x46, 1]), Some((Some(100), false)), "0x46 full is not 'charging'");
    assert_eq!(md::decode_battery(&[101, 0, 0x45, 1]), Some((None, false)), "over 100 is no reading");
    assert_eq!(md::decode_battery(&[87, 0, 0x45, 0]), None, "no cell present (wired)");
    assert_eq!(md::decode_battery(&[87, 0, 0x45]), None, "too short");
    // firmware: bytes 0..4 the dongle, 4..8 the mouse; a non-zero dongle version = the receiver answered
    assert_eq!(md::dongle_connected(&[2, 1, 0, 0, 1, 0, 5, 0]), Some(true));
    assert_eq!(md::dongle_connected(&[0, 0, 0, 0, 1, 0, 5, 0]), Some(false));
    assert_eq!(md::dongle_connected(&[0, 0, 0]), None);
    // lift-off: raw 1 = 1 mm, raw 2 = 2 mm, anything else unknown
    assert_eq!((md::lift_off_tenths(1), md::lift_off_tenths(2), md::lift_off_tenths(0), md::lift_off_tenths(3)), (Some(10), Some(20), None, None));
    assert_eq!((md::lift_off_raw(10), md::lift_off_raw(20), md::lift_off_raw(7)), (Some(1), Some(2), None));
}

// ---------------------------------------------------------------- allow-list + table

#[test]
fn no_frame_passes_the_allow_list() {
    // every moddoMOUSE read is a HidD_GetFeature (a FeatureIn, which sends nothing and passes by its kind); the blank frame that
    // is the `out` of such a read would be a settings WRITE of polling 0 / DPI 0 if it were sent as a SetFeature: it must not pass
    for id in [2u8, 3, 4] {
        let mut r = vec![0u8; 64];
        r[0] = id;
        assert!(!md::read_request_allowed(&r), "blank report {id}");
        assert!(!md::read_request_allowed(&r[..12]), "blank report {id}, 12 bytes");
    }
    // the settings write
    let mut w = vec![0u8; 64];
    w[0] = 2;
    w[1..12].copy_from_slice(&CONFIG);
    assert!(!md::read_request_allowed(&w));
    assert!(!md::read_request_allowed(&[2, 0, 0]));
    assert!(!md::read_request_allowed(&[]));
    // and what the driver really sends in a read: only FeatureIn transfers (nothing goes out)
    let st = fake();
    let mut p = port(&st);
    md::Moddo.read(&matched(), &mut p, ReadOptions::default()).unwrap();
    assert!(p.sent.iter().all(|s| matches!(s.1, HidTransfer::FeatureIn { .. })), "a read sends no SetFeature");
}

#[test]
fn the_table_is_openmouses_is_supported() {
    // supported-devices-data.ts lists moddoMOUSE without product ids: the vendor id + a vendor collection decide
    let c = cfg(0x0001, 1);
    let m = md::matches(&[&c], 0x2FE3, 0x0001).unwrap();
    assert_eq!((m.family, m.model, m.wireless, m.variant), (Family::Moddo, Some("moddoMOUSE"), None, 1));
    assert_eq!(m.config.path, c.path);
    assert_eq!(family::find(std::slice::from_ref(&c), 0x2FE3, 0x0001).map(|m| m.family), Some(Family::Moddo));
    // the legacy usage 2 works too; usage 1 is preferred when both exist
    let legacy = cfg(0x0002, 2);
    assert!(md::matches(&[&legacy], 0x2FE3, 0x0002).is_some());
    let mut both_legacy = cfg(0x0001, 2);
    both_legacy.path = "legacy".into();
    assert_eq!(md::matches(&[&both_legacy, &c], 0x2FE3, 0x0001).map(|m| m.config.path), Some(c.path.clone()));
    // not a match: another vendor, another usage page (0xFF00 ≠ 0xFF), another usage, a feature report too small, the mouse
    assert!(md::matches(&[&c], 0x2FE4, 0x0001).is_none());
    assert!(md::matches(&[&named(hid_info("x", 0x2FE3, 1, 0xFF00, 1, (0, 0, 64)))], 0x2FE3, 1).is_none());
    assert!(md::matches(&[&cfg(1, 3)], 0x2FE3, 1).is_none());
    assert!(md::matches(&[&named(hid_info("x", 0x2FE3, 1, 0x00FF, 1, (0, 0, 8)))], 0x2FE3, 1).is_none());
    assert!(md::matches(&[&named(hid_info("m", 0x2FE3, 1, 0x01, 0x02, (8, 0, 0)))], 0x2FE3, 1).is_none());
}

#[test]
fn caps_are_what_openmouse_offers() {
    let c = md::caps();
    let d = c.dpi.unwrap();
    assert_eq!((d.min(), d.max()), (50, 26000));
    assert!(d.allows(50) && d.allows(26000) && !d.allows(75) && !d.allows(26050));
    assert_eq!((c.polling, c.lift_off, c.battery), (vec![125, 250, 500, 1000], vec![10, 20], true));
}

// ---------------------------------------------------------------- the driver against a scripted mouse

struct Fake {
    config: Vec<u8>,
    battery: Vec<u8>,
    firmware: Vec<u8>,
    /// the next N settings reads after a write still show the old value
    lag: u32,
    old: Vec<u8>,
    /// reads of the settings answer a zeroed report (asleep)
    asleep: bool,
    writes: Vec<Vec<u8>>,
    config_reads: u32,
    /// after a write the firmware changes this byte of the settings (index, value)
    normalise: Option<(usize, u8)>,
}

fn fake() -> Rc<RefCell<Fake>> {
    Rc::new(RefCell::new(Fake {
        config: CONFIG.to_vec(),
        battery: vec![87, 0, 0x45, 1],
        firmware: vec![2, 1, 0, 0, 1, 0, 5, 0],
        lag: 0,
        old: CONFIG.to_vec(),
        asleep: false,
        writes: Vec::new(),
        config_reads: 0,
        normalise: None,
    }))
}

fn port(st: &Rc<RefCell<Fake>>) -> ScriptPort {
    let st = st.clone();
    ScriptPort::new(cfg(1, 1), move |out, how| {
        let mut s = st.borrow_mut();
        assert_eq!(out.len(), 64, "padded to the collection's feature report size");
        match how {
            HidTransfer::FeatureIn { reply_id, reply_len, .. } => {
                assert_eq!((*reply_id, *reply_len), (out[0], 64));
                let body = match out[0] {
                    2 => {
                        s.config_reads += 1;
                        if s.asleep {
                            vec![0u8; 11]
                        } else if s.lag > 0 {
                            s.lag -= 1;
                            s.old.clone()
                        } else {
                            s.config.clone()
                        }
                    }
                    3 => s.firmware.clone(),
                    4 => s.battery.clone(),
                    id => panic!("unexpected report {id}"),
                };
                let mut v = vec![out[0]];
                v.extend(body);
                v.resize(64, 0);
                Ok(v)
            }
            HidTransfer::FeatureOut => {
                assert_eq!(out[0], 2);
                s.writes.push(out.to_vec());
                s.old = s.config.clone();
                s.config = out[1..12].to_vec();
                if let Some((i, v)) = s.normalise {
                    s.config[i] = v;
                }
                Ok(Vec::new())
            }
            other => panic!("unexpected transfer {other:?}"),
        }
    })
}

fn matched() -> bu_mouse::family::Matched {
    md::matches(&[&cfg(1, 1)], 0x2FE3, 1).unwrap()
}

#[test]
fn reading_asks_settings_battery_and_firmware() {
    let st = fake();
    let mut p = port(&st);
    let r = md::Moddo.read(&matched(), &mut p, ReadOptions::default()).unwrap();
    // the real read-only gate (win/hid.rs): a FeatureIn fetch always passes (it sends nothing), anything else must be allow-listed
    assert!(p.sent.iter().all(|s| matches!(s.1, HidTransfer::FeatureIn { .. }) || md::read_request_allowed(&s.0)), "read requests only");
    assert_eq!(p.sent.iter().map(|s| s.0[0]).collect::<Vec<_>>(), vec![2, 4, 3]);
    assert_eq!((r.online, r.dpi, r.polling_hz, r.lift_off, r.battery_percent, r.charging, r.wireless), (true, Some(1600), Some(1000), Some(10), Some(87), Some(false), Some(true)));
    assert_eq!(r.caps, md::caps());
    // a wired mouse: no cell, no dongle
    st.borrow_mut().battery = vec![0, 0, 0, 0];
    st.borrow_mut().firmware = vec![0, 0, 0, 0, 1, 0, 5, 0];
    let r = md::Moddo.read(&matched(), &mut port(&st), ReadOptions::default()).unwrap();
    assert_eq!((r.battery_percent, r.charging, r.wireless), (None, None, Some(false)));
}

#[test]
fn a_sleeping_mouse_is_offline_not_zero() {
    let st = fake();
    st.borrow_mut().asleep = true;
    let r = md::Moddo.read(&matched(), &mut port(&st), ReadOptions::default()).unwrap();
    assert_eq!((r.online, r.dpi, r.polling_hz, r.lift_off), (false, None, None, None));
    // and a change is refused: OpenMouse would persist zeros
    let mut p = port(&st);
    assert!(md::Moddo.write(&matched(), &mut p, Setting::Dpi(800)).is_err());
    assert!(p.sent.iter().all(|s| s.1 != HidTransfer::FeatureOut));
    let mut gone = ScriptPort::new(cfg(1, 1), |_, _| Err(Error::MouseGone("m".into())));
    assert!(matches!(md::Moddo.read(&matched(), &mut gone, ReadOptions::default()), Err(Error::MouseGone(_))));
}

#[test]
fn each_change_rewrites_the_whole_report_with_one_field_changed() {
    let st = fake();
    let c = md::Moddo.write(&matched(), &mut port(&st), Setting::Dpi(3200)).unwrap();
    assert_eq!((c.now, c.before), (3200, Some(1600)));
    // [id 02][polling 1000][lift 1][3200 = 80 0C, twice][05][FD FF][0A], zeros to 64
    let mut want = vec![0x02, 0xE8, 0x03, 0x01, 0x80, 0x0C, 0x80, 0x0C, 0x05, 0xFD, 0xFF, 0x0A];
    want.resize(64, 0);
    assert_eq!(st.borrow().writes, vec![want]);
    let c = md::Moddo.write(&matched(), &mut port(&st), Setting::Polling(250)).unwrap();
    assert_eq!((c.now, c.before), (250, Some(1000)));
    assert_eq!(st.borrow().config, vec![0xFA, 0x00, 0x01, 0x80, 0x0C, 0x80, 0x0C, 0x05, 0xFD, 0xFF, 0x0A]);
    let c = md::Moddo.write(&matched(), &mut port(&st), Setting::LiftOff(20)).unwrap();
    assert_eq!((c.now, c.before), (20, Some(10)));
    assert_eq!(st.borrow().config[2], 2);
    // the preserved fields survived all three
    assert_eq!(st.borrow().config[7..], [0x05, 0xFD, 0xFF, 0x0A]);
}

#[test]
fn a_write_the_firmware_applies_late_is_read_again() {
    let st = fake();
    st.borrow_mut().lag = 4; // the read before the write uses one (old == current then)
    let mut p = port(&st);
    let c = md::Moddo.write(&matched(), &mut p, Setting::Dpi(800)).unwrap();
    assert_eq!(c.now, 800);
    // 1 read before, the write, then 4 reads (3 old, the 4th new)
    assert_eq!(st.borrow().config_reads, 5);
    // a mouse that never takes it: five reads after the write, then an error
    let st = fake();
    st.borrow_mut().lag = 99;
    let e = md::Moddo.write(&matched(), &mut port(&st), Setting::Dpi(800)).unwrap_err();
    assert!(matches!(e, Error::BadAnswer(_)), "{e:?}");
    assert_eq!(st.borrow().config_reads, 6, "1 + 5 attempts");
}

#[test]
fn values_the_mouse_cannot_store_are_refused_before_a_byte_leaves() {
    let st = fake();
    let mut p = port(&st);
    for s in [Setting::Dpi(49), Setting::Dpi(75), Setting::Dpi(26050), Setting::Polling(2000), Setting::Polling(0), Setting::LiftOff(7), Setting::LiftOff(30)] {
        assert!(md::Moddo.write(&matched(), &mut p, s).is_err(), "{s:?}");
    }
    assert!(p.sent.is_empty());
    // a settings report shorter than 11 bytes would have its preserved fields written as zeros: not written
    let mut p = ScriptPort::new(cfg(1, 1), |out, _| {
        let mut v = vec![out[0]];
        v.extend_from_slice(&CONFIG[..8]);
        Ok(v)
    });
    assert!(md::Moddo.write(&matched(), &mut p, Setting::Dpi(800)).is_err());
    assert_eq!(p.sent.len(), 1, "one read, no write");
}

// ---------------------------------------------------------------- review fixes

#[test]
fn another_zephyr_device_on_the_same_vendor_id_is_not_claimed() {
    // 0x2FE3 is the Zephyr RTOS' default id: the right collection alone is not enough, the strings must say moddo
    let plain = hid_info("z", 0x2FE3, 1, 0x00FF, 1, (0, 0, 64));
    assert!(md::matches(&[&plain], 0x2FE3, 1).is_none());
    let mut other = plain.clone();
    other.product = Some("Zephyr HID".into());
    other.manufacturer = Some("Nordic".into());
    assert!(md::matches(&[&other], 0x2FE3, 1).is_none());
    assert!(family::find(std::slice::from_ref(&other), 0x2FE3, 1).is_none());
    // the manufacturer alone, in any case; or the string on another interface of the same device
    let mut m = plain.clone();
    m.manufacturer = Some("MODDO.io".into());
    assert!(md::matches(&[&m], 0x2FE3, 1).is_some());
    let mut mouse = hid_info("mouse", 0x2FE3, 1, 0x01, 0x02, (8, 0, 0));
    mouse.product = Some("moddoMOUSE Wireless".into());
    assert!(md::matches(&[&mouse, &plain], 0x2FE3, 1).is_some());
}

#[test]
fn a_dpi_change_with_different_x_and_y_is_refused() {
    // saved X 800, Y 1600: setting 1000 would set both and "undo" could only restore X
    let st = fake();
    st.borrow_mut().config[3..7].copy_from_slice(&[0x20, 0x03, 0x40, 0x06]);
    let mut p = port(&st);
    let e = md::Moddo.write(&matched(), &mut p, Setting::Dpi(1000)).unwrap_err();
    assert!(matches!(e, Error::BadAnswer(ref m) if m.contains("nothing was written")), "{e:?}");
    assert!(st.borrow().writes.is_empty());
    // polling and lift-off keep Y as it is, so they still work
    assert!(md::Moddo.write(&matched(), &mut port(&st), Setting::Polling(500)).is_ok());
    assert_eq!(st.borrow().config[3..7], [0x20, 0x03, 0x40, 0x06]);
}

#[test]
fn a_settings_report_with_bytes_behind_the_eleven_is_not_overwritten() {
    // the write zero-fills behind byte 11: a non-zero byte there would be wiped, so nothing is sent
    let st = fake();
    st.borrow_mut().config.extend_from_slice(&[0, 0, 0x07]);
    let mut p = port(&st);
    for s in [Setting::Dpi(800), Setting::Polling(500), Setting::LiftOff(20)] {
        let e = md::Moddo.write(&matched(), &mut p, s).unwrap_err();
        assert!(matches!(e, Error::BadAnswer(ref m) if m.contains("nothing was written")), "{s:?}: {e:?}");
    }
    assert!(st.borrow().writes.is_empty());
    assert!(p.sent.iter().all(|s| matches!(s.1, HidTransfer::FeatureIn { .. })));
    // an all-zero tail is fine (the normal padded report)
    let st = fake();
    assert!(md::Moddo.write(&matched(), &mut port(&st), Setting::Dpi(800)).is_ok());
}

#[test]
fn a_write_is_confirmed_on_the_changed_field_only() {
    // the firmware rewrites the deep-sleep byte (index 10) after any write: the DPI change itself did succeed
    let st = fake();
    st.borrow_mut().normalise = Some((10, 0x0B));
    let c = md::Moddo.write(&matched(), &mut port(&st), Setting::Dpi(3200)).unwrap();
    assert_eq!((c.now, c.before), (3200, Some(1600)));
    let c = md::Moddo.write(&matched(), &mut port(&st), Setting::Polling(250)).unwrap();
    assert_eq!((c.now, c.before), (250, Some(1000)));
    assert_eq!(st.borrow().config[10], 0x0B);
    // a change that did not take is still an error (the changed field differs)
    let st = fake();
    st.borrow_mut().normalise = Some((2, 1));
    assert!(md::Moddo.write(&matched(), &mut port(&st), Setting::LiftOff(20)).is_err());
}

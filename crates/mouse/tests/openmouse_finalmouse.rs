//! Order 105 check, Finalmouse (UltralightX / Starlight-12 dongle): our requests = the bytes OpenMouse sends for the same
//! setting (encode), and answers shaped the way OpenMouse reads them decode to the same values (decode). The expected bytes
//! are worked out by hand from OpenMouse (@openmouse/protocol 0.40.1: `dist/finalmouse/index.js` and
//! `dist/drivers/finalmouse/hid.js`), never from our own encoder.
//!
//! buildFinalmouseReport(command, payload) is a 63-byte body: `[0] = 2 + payload length, [1] = 0x80 | command, [2] = payload
//! length, [3..] = payload`, sent with `sendReport(0x04, body)`; Windows puts the report id in front, so our byte i + 1 is
//! OpenMouse's byte i and the buffer is 64 bytes. An answer arrives as an input report: OpenMouse's `decodeFinalmouseReport(5,
//! data)` gets the data without the id (`data[0]` inner length, `data[1]` command, `data[2]` payload length, `data[3..]`
//! payload); the Windows buffer has the id 0x05 in front.

use bu_mouse::error::Error;
use bu_mouse::family::{self, Driver, Family, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::finalmouse as f;
use bu_mouse::os::{HidInfo, HidTransfer};
use std::cell::RefCell;
use std::rc::Rc;

fn frame(bytes: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 64];
    v[..bytes.len()].copy_from_slice(bytes);
    v
}

fn ulx() -> HidInfo {
    hid_info("ulx-vendor", 0x361D, 0x0100, 0xFF00, 0x01, (64, 64, 0))
}

// ---------------------------------------------------------------- encode

#[test]
fn the_requests_are_openmouses() {
    // setDpi(1600): writeU16(16, 1600) = payload [0x40, 0x06] (1600 = 0x0640), report 0x04:
    // body = [2 + 2 = 4, 0x80 | 16 = 0x90, 2, 0x40, 0x06]
    assert_eq!(f::dpi_request(1600, 64), Some(frame(&[0x04, 0x04, 0x90, 0x02, 0x40, 0x06])));
    // 50 DPI = 0x0032, 26000 = 0x6590
    assert_eq!(f::dpi_request(50, 64), Some(frame(&[0x04, 0x04, 0x90, 0x02, 0x32, 0x00])));
    assert_eq!(f::dpi_request(26_000, 64), Some(frame(&[0x04, 0x04, 0x90, 0x02, 0x90, 0x65])));
    assert_eq!(f::dpi_request(49, 64), None);
    assert_eq!(f::dpi_request(26_001, 64), None);
    // setPollingRate: command 17 -> 0x91; 8000 = 0x1f40, 1000 = 0x03e8, 500 = 0x01f4
    assert_eq!(f::polling_request(8000, 64), Some(frame(&[0x04, 0x04, 0x91, 0x02, 0x40, 0x1F])));
    assert_eq!(f::polling_request(1000, 64), Some(frame(&[0x04, 0x04, 0x91, 0x02, 0xE8, 0x03])));
    assert_eq!(f::polling_request(500, 64), Some(frame(&[0x04, 0x04, 0x91, 0x02, 0xF4, 0x01])));
    for hz in [0, 125, 250, 750, 16000] {
        assert_eq!(f::polling_request(hz, 64), None, "{hz}");
    }
    // setLiftOffDistance: command 21 -> 0x95, one byte (the mm); [2 + 1 = 3, 0x95, 1, mm]
    assert_eq!(f::lift_off_request(1, 64), Some(frame(&[0x04, 0x03, 0x95, 0x01, 0x01])));
    assert_eq!(f::lift_off_request(2, 64), Some(frame(&[0x04, 0x03, 0x95, 0x01, 0x02])));
    assert_eq!(f::lift_off_request(3, 64), None);
    assert_eq!(f::lift_off_request(0, 64), None);
}

#[test]
fn the_read_requests_are_openmouses() {
    let [lod, wake, dongle] = f::read_requests(64);
    // write(tx 0x04, liftOffDistance 21) with no payload: [2, 0x95, 0]
    assert_eq!(lod, frame(&[0x04, 0x02, 0x95, 0x00]));
    // write(0x04, wakeAll 96): 0x80 | 96 = 0xe0
    assert_eq!(wake, frame(&[0x04, 0x02, 0xE0, 0x00]));
    // write(0x02 = FINALMOUSE_REPORT.dongle, dongleInfo 10): 0x80 | 10 = 0x8a
    assert_eq!(dongle, frame(&[0x02, 0x02, 0x8A, 0x00]));
    // a longer output report is the same frame zero padded
    assert_eq!(f::request(0x04, 96, &[], 80).len(), 80);
}

// ---------------------------------------------------------------- decode

fn input(bytes: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 64];
    v[..bytes.len()].copy_from_slice(bytes);
    v
}

#[test]
fn answers_decode_like_openmouse() {
    use f::Update::*;
    // case 3: u16le(payload) = 800 = 0x0320; inner length 2 + 2 = 4
    assert_eq!(f::decode_report(&input(&[0x05, 4, 3, 2, 0x20, 0x03])), Some(Dpi(800)));
    // case 4: polling 8000 = 0x1f40
    assert_eq!(f::decode_report(&input(&[0x05, 4, 4, 2, 0x40, 0x1F])), Some(Polling(8000)));
    // case 5: millivolts 3700 = 0x0e74
    assert_eq!(f::decode_report(&input(&[0x05, 4, 5, 2, 0x74, 0x0E])), Some(BatteryMv(3700)));
    // case 21: lift-off mm; case 37: charging state; case 38: [soc, mv lo, mv hi]
    assert_eq!(f::decode_report(&input(&[0x05, 3, 21, 1, 2])), Some(LiftOffMm(2)));
    assert_eq!(f::decode_report(&input(&[0x05, 3, 37, 1, 1])), Some(Charging(1)));
    assert_eq!(f::decode_report(&input(&[0x05, 5, 38, 3, 87, 0x74, 0x0E])), Some(BatterySoc(87)));
    // too short a payload, the dongle's report id, an unknown command, a one-byte report: nothing
    assert_eq!(f::decode_report(&input(&[0x05, 3, 3, 1, 0x20])), None, "u16le needs two bytes");
    assert_eq!(f::decode_report(&input(&[0x05, 4, 38, 2, 87, 1])), None, "case 38 needs three bytes");
    assert_eq!(f::decode_report(&input(&[0x03, 4, 3, 2, 0x20, 0x03])), None);
    assert_eq!(f::decode_report(&input(&[0x05, 3, 99, 1, 7])), None);
    assert_eq!(f::decode_report(&[0x05, 4]), None);
    // the inner length caps what is read: inner 3 holds only command, length and one payload byte
    assert_eq!(f::decode_report(&input(&[0x05, 3, 3, 2, 0x20, 0x03])), None);
}

#[test]
fn the_battery_curve_is_openmouses() {
    // volts / percent: 3.0 / 0.2, 3.62 / 5, 3.66 / 10, 3.74 / 25, 3.88 / 50, 4.17 / 75, 4.38 / 100; linear in between, rounded
    for (mv, pct) in [(0, 0), (2900, 0), (3000, 0), (3620, 5), (3660, 10), (3740, 25), (3880, 50), (4170, 75), (4380, 100), (5000, 100)] {
        assert_eq!(f::battery_percent_from_mv(mv), pct, "{mv} mV");
    }
    // 3.76 V: between 3.74 (25) and 3.88 (50): f = 0.02 / 0.14 = 0.142857, 25 + 25 * 0.142857 = 28.57 -> 29
    assert_eq!(f::battery_percent_from_mv(3760), 29);
    // 4.0 V: between 3.88 (50) and 4.17 (75): f = 0.12 / 0.29 = 0.41379, 50 + 25 * 0.41379 = 60.34 -> 60
    assert_eq!(f::battery_percent_from_mv(4000), 60);
}

// ---------------------------------------------------------------- the device table

#[test]
fn the_device_table_is_the_supported_list() {
    let info = ulx();
    let m = f::matches(&[&info], 0x361D, 0x0100).unwrap();
    assert_eq!((m.family, m.model, m.wireless, m.config.path.as_str()), (Family::Finalmouse, Some("Finalmouse Starlight-12 / UltralightX"), Some(true), "ulx-vendor"));
    assert_eq!(family::find(std::slice::from_ref(&info), 0x361D, 0x0100).unwrap().family, Family::Finalmouse);
    let caps = f::caps();
    let dpi = caps.dpi.unwrap();
    assert_eq!((dpi.min(), dpi.max()), (50, 26_000));
    assert!(dpi.allows(50) && dpi.allows(26_000) && dpi.allows(1234));
    assert_eq!((caps.polling.clone(), caps.lift_off.clone(), caps.battery), (vec![500, 1000, 2000, 4000, 8000], vec![10, 20], true));
    // not matched: the Starlight X dongles / mice (not on the list), the ULX direct-mouse id 0x0102, another vendor, a wrong usage,
    // a collection too short for the 64-byte frames
    for pid in [0x0300u16, 0x0301, 0x0310, 0x0311, 0x0102, 0x0101] {
        assert!(f::matches(&[&hid_info("x", 0x361D, pid, 0xFF00, 0x01, (64, 64, 0))], 0x361D, pid).is_none(), "{pid:04x}");
    }
    assert!(f::matches(&[&info], 0x361E, 0x0100).is_none());
    assert!(f::matches(&[&hid_info("x", 0x361D, 0x0100, 0xFF00, 0x02, (64, 64, 0))], 0x361D, 0x0100).is_none());
    assert!(f::matches(&[&hid_info("x", 0x361D, 0x0100, 0x01, 0x02, (8, 0, 0))], 0x361D, 0x0100).is_none());
    assert!(f::matches(&[&hid_info("x", 0x361D, 0x0100, 0xFF00, 0x01, (64, 32, 0))], 0x361D, 0x0100).is_none());
    let mouse = hid_info("mi_00", 0x361D, 0x0100, 0x01, 0x02, (8, 0, 0));
    assert_eq!(f::matches(&[&mouse, &info], 0x361D, 0x0100).unwrap().config.path, "ulx-vendor");
}

#[test]
fn no_other_family_claims_the_finalmouse_id() {
    let info = ulx();
    let own = [&info];
    assert!(bu_mouse::pulsar::matches_cmouse(&own, 0x361D, 0x0100).is_none());
    assert!(bu_mouse::pulsar::matches_xs1(&own, 0x361D, 0x0100).is_none());
    assert!(bu_mouse::logitech::matches(&own, 0x361D, 0x0100).is_none());
    assert!(bu_mouse::razer::matches(&own, 0x361D, 0x0100).is_none());
    assert!(bu_mouse::steelseries::matches(&own, 0x361D, 0x0100).is_none());
}

// ---------------------------------------------------------------- the driver

/// A fake ULX dongle. Each of the three read requests elicits its own burst of input reports (`bursts[i]`, i = 0 lift-off query,
/// 1 wake-all, 2 dongle info); a write is remembered. The transfer must be what the driver says it is.
struct Mouse {
    bursts: [Vec<Vec<u8>>; 3],
    /// every request seen
    seen: Vec<Vec<u8>>,
    /// the prefixes the last read waited for
    last_prefixes: Vec<Vec<u8>>,
}

fn port_for(info: &HidInfo, m: Rc<RefCell<Mouse>>) -> ScriptPort {
    ScriptPort::new(info.clone(), move |out, how| {
        let mut m = m.borrow_mut();
        m.seen.push(out.to_vec());
        match how {
            HidTransfer::OutputOnly => Ok(Vec::new()),
            HidTransfer::OutputThenMatch { prefixes, .. } => {
                m.last_prefixes = prefixes.clone();
                let i = f::read_requests(64).iter().position(|r| r == out).expect("a read request");
                m.bursts[i]
                    .iter()
                    .find(|r| prefixes.iter().any(|p| r.starts_with(p)))
                    .cloned()
                    .ok_or_else(|| Error::BadAnswer("no matching answer".into()))
            }
            other => panic!("unexpected transfer {other:?}"),
        }
    })
}

fn dpi_r(d: u16) -> Vec<u8> {
    input(&[0x05, 4, 3, 2, d as u8, (d >> 8) as u8])
}
fn polling_r(hz: u16) -> Vec<u8> {
    input(&[0x05, 4, 4, 2, hz as u8, (hz >> 8) as u8])
}
fn soc_r(p: u8) -> Vec<u8> {
    input(&[0x05, 5, 38, 3, p, 0x74, 0x0E])
}
fn mv_r(mv: u16) -> Vec<u8> {
    input(&[0x05, 4, 5, 2, mv as u8, (mv >> 8) as u8])
}
fn charging_r(s: u8) -> Vec<u8> {
    input(&[0x05, 3, 37, 1, s])
}
fn lod_r(mm: u8) -> Vec<u8> {
    input(&[0x05, 3, 21, 1, mm])
}

fn mouse(bursts: [Vec<Vec<u8>>; 3]) -> Rc<RefCell<Mouse>> {
    Rc::new(RefCell::new(Mouse { bursts, seen: Vec::new(), last_prefixes: Vec::new() }))
}

#[test]
fn a_read_finds_the_request_that_is_answered_and_asks_the_rest_the_same_way() {
    let info = ulx();
    let m = f::matches(&[&info], 0x361D, 0x0100).unwrap();
    // the lift-off query answers only the lift-off; wake-all answers the burst
    let mouse = mouse([vec![lod_r(2)], vec![dpi_r(800), polling_r(1000), soc_r(87), charging_r(1), mv_r(3700)], vec![input(&[0x03, 4, 11, 2, 1, 2])]]);
    let mut port = port_for(&info, mouse.clone());
    let s = f::Finalmouse.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((s.online, s.dpi, s.polling_hz, s.battery_percent, s.charging, s.lift_off, s.wireless), (true, Some(800), Some(1000), Some(87), Some(true), Some(20), Some(true)));
    assert_eq!(s.caps, f::caps());
    let [q0, q1, _] = f::read_requests(64);
    // DPI: the lift-off query (nothing), then wake-all; polling, state of charge, charging all with wake-all; lift-off with its own query
    assert_eq!(mouse.borrow().seen, vec![q0.clone(), q1.clone(), q1.clone(), q1.clone(), q1.clone(), q0.clone()]);
    assert!(port.sent.iter().all(|s| f::read_request_allowed(&s.0)));
    // the wait is for [0x05, n, command, length] for every n from 2 + length up to 62
    let prefixes = mouse.borrow().last_prefixes.clone();
    assert_eq!((prefixes.first().cloned(), prefixes.last().cloned(), prefixes.len()), (Some(vec![5, 3, 21, 1]), Some(vec![5, 62, 21, 1]), 60));
    match &port.sent[1].1 {
        HidTransfer::OutputThenMatch { reply_len, prefixes, .. } => {
            assert_eq!((*reply_len, prefixes.first().cloned(), prefixes.last().cloned(), prefixes.len()), (64, Some(vec![5, 4, 3, 2]), Some(vec![5, 62, 3, 2]), 59));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn without_a_state_of_charge_the_battery_comes_from_the_voltage_curve() {
    let info = ulx();
    let m = f::matches(&[&info], 0x361D, 0x0100).unwrap();
    let mouse = mouse([vec![lod_r(1), dpi_r(1600), polling_r(8000), mv_r(3880)], vec![], vec![]]);
    let mut port = port_for(&info, mouse);
    let s = f::Finalmouse.read(&m, &mut port, ReadOptions::default()).unwrap();
    // 3880 mV = 3.88 V -> 50 %; no charging report: unknown; the first request already answered the DPI
    assert_eq!((s.dpi, s.polling_hz, s.battery_percent, s.charging, s.lift_off), (Some(1600), Some(8000), Some(50), None, Some(10)));
}


#[test]
fn a_state_of_charge_above_100_is_not_a_battery_reading() {
    let info = ulx();
    let m = f::matches(&[&info], 0x361D, 0x0100).unwrap();
    // 0xFF (255 %) with no voltage report: no battery; with a voltage report the curve is used (3880 mV = 50 %)
    let mo = mouse([vec![lod_r(1), dpi_r(1600), polling_r(1000), soc_r(0xFF)], vec![], vec![]]);
    let s = f::Finalmouse.read(&m, &mut port_for(&info, mo), ReadOptions::default()).unwrap();
    assert_eq!((s.online, s.dpi, s.battery_percent), (true, Some(1600), None));
    let mo = mouse([vec![lod_r(1), dpi_r(1600), polling_r(1000), soc_r(101), mv_r(3880)], vec![], vec![]]);
    let s = f::Finalmouse.read(&m, &mut port_for(&info, mo), ReadOptions::default()).unwrap();
    assert_eq!(s.battery_percent, Some(50));
    // 100 % itself is fine
    let mo = mouse([vec![lod_r(1), dpi_r(1600), polling_r(1000), soc_r(100)], vec![], vec![]]);
    let s = f::Finalmouse.read(&m, &mut port_for(&info, mo), ReadOptions::default()).unwrap();
    assert_eq!(s.battery_percent, Some(100));
}
#[test]
fn a_dongle_with_no_answers_is_offline() {
    let info = ulx();
    let m = f::matches(&[&info], 0x361D, 0x0100).unwrap();
    let mouse = mouse([vec![], vec![], vec![]]);
    let mut port = port_for(&info, mouse.clone());
    let s = f::Finalmouse.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((s.online, s.dpi, s.polling_hz, s.battery_percent), (false, None, None, None));
    // all three of OpenMouse's requests, once each, then it gives up
    assert_eq!(mouse.borrow().seen, f::read_requests(64).to_vec());
    // the dongle answers only with its own report: still no DPI
    let mouse = self::mouse([vec![], vec![], vec![input(&[0x03, 4, 11, 2, 1, 2])]]);
    let mut port = port_for(&info, mouse);
    assert!(!f::Finalmouse.read(&m, &mut port, ReadOptions::default()).unwrap().online);
}

#[test]
fn writes_send_one_frame_and_read_nothing_back() {
    let info = ulx();
    let m = f::matches(&[&info], 0x361D, 0x0100).unwrap();
    let mouse = mouse([vec![], vec![], vec![]]);
    let mut port = port_for(&info, mouse.clone());
    let c = f::Finalmouse.write(&m, &mut port, Setting::Dpi(1600)).unwrap();
    assert_eq!((c.now, c.before), (1600, None));
    assert_eq!(f::Finalmouse.write(&m, &mut port, Setting::Polling(4000)).unwrap().now, 4000);
    assert_eq!(f::Finalmouse.write(&m, &mut port, Setting::LiftOff(20)).unwrap().now, 20);
    assert_eq!(
        mouse.borrow().seen,
        vec![
            frame(&[0x04, 0x04, 0x90, 0x02, 0x40, 0x06]),
            // 4000 = 0x0fa0
            frame(&[0x04, 0x04, 0x91, 0x02, 0xA0, 0x0F]),
            frame(&[0x04, 0x03, 0x95, 0x01, 0x02]),
        ]
    );
    assert!(port.sent.iter().all(|s| s.1 == HidTransfer::OutputOnly));
    // refused values send nothing
    for s in [Setting::Dpi(10), Setting::Dpi(30_000), Setting::Polling(125), Setting::LiftOff(15), Setting::LiftOff(30), Setting::LiftOff(5)] {
        assert!(matches!(f::Finalmouse.write(&m, &mut port, s), Err(Error::OutOfRange { .. })), "{s:?}");
    }
    assert_eq!(mouse.borrow().seen.len(), 3);
}

#[test]
fn a_longer_output_report_gets_the_same_frame_padded() {
    let info = hid_info("ulx-vendor", 0x361D, 0x0100, 0xFF00, 0x01, (64, 80, 0));
    let m = f::matches(&[&info], 0x361D, 0x0100).unwrap();
    let mouse = mouse([vec![], vec![], vec![]]);
    let mut port = port_for(&info, mouse.clone());
    f::Finalmouse.write(&m, &mut port, Setting::Dpi(800)).unwrap();
    let seen = mouse.borrow().seen[0].clone();
    assert_eq!(seen.len(), 80);
    assert_eq!(&seen[..6], &[0x04, 0x04, 0x90, 0x02, 0x20, 0x03]);
    assert!(seen[6..].iter().all(|b| *b == 0));
}

// ---------------------------------------------------------------- the allow-list

#[test]
fn only_the_three_read_requests_pass_the_allow_list() {
    for r in f::read_requests(64) {
        assert!(f::read_request_allowed(&r));
        assert!(family::read_request_allowed(Family::Finalmouse, &r));
    }
    // padded longer is the same request
    assert!(f::read_request_allowed(&f::read_requests(80)[1]));
    // the writes (with a payload) and a lift-off WRITE of 0 payload bytes cannot be confused with the query
    for w in [f::dpi_request(800, 64).unwrap(), f::polling_request(1000, 64).unwrap(), f::lift_off_request(1, 64).unwrap(), f::lift_off_request(2, 64).unwrap()] {
        assert!(!f::read_request_allowed(&w), "{w:02x?}");
    }
    // a wake-all with anything extra, another command, a short frame, nothing
    let mut odd = f::read_requests(64)[1].clone();
    odd[10] = 1;
    assert!(!f::read_request_allowed(&odd));
    assert!(!f::read_request_allowed(&f::request(0x04, 99, &[], 64)));
    assert!(!f::read_request_allowed(&f::request(0x04, 96, &[1], 64)));
    assert!(!f::read_request_allowed(&f::read_requests(64)[0][..10]));
    assert!(!f::read_request_allowed(&[]));
}

//! Order 105 check, Rapoo VT9 Pro (battery only): the one request is the feature report 0x2A, and its answer decodes to the
//! percentage OpenMouse reads from it. Expected values are worked out by hand from OpenMouse (@openmouse/protocol 0.40.1:
//! `dist/rapoo/index.js` - `rapooStatusBatteryPercent` and its documented example - and `dist/drivers/rapoo/hid.js`).

use bu_mouse::family::{self, Driver, Family, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::os::{HidInfo, HidTransfer};
use bu_mouse::rapoo as r;
use bu_mouse::Error;

fn status(pid: u16) -> HidInfo {
    hid_info("rapoo-status", 0x24AE, pid, 0xFF0B, 0x0104, (0, 0, 32))
}
fn config(pid: u16) -> HidInfo {
    hid_info("rapoo-config", 0x24AE, pid, 0xFF00, 0x000E, (32, 32, 0))
}

#[test]
fn the_battery_decodes_like_openmouse() {
    // OpenMouse's documented example on the cable: `2a 01 00 00 04 21 50 00 00 62` - report data after the id are
    // `01 00 00 04 21 50 00 00 62`; byte offset + 8 (offset 1 behind the id 0x2A) = 0x62 = 98 %
    assert_eq!(r::battery_percent(&[0x2A, 0x01, 0x00, 0x00, 0x04, 0x21, 0x50, 0x00, 0x00, 0x62]), Some(98));
    // on the receiver byte 5 differs (`11 20`), the percentage is at the same place
    assert_eq!(r::battery_percent(&[0x2A, 0x01, 0x00, 0x00, 0x04, 0x11, 0x20, 0x00, 0x00, 0x62, 0, 0]), Some(98));
    // exactly 100 is a reading, 101 is not; a shorter buffer or another report id is none
    assert_eq!(r::battery_percent(&[0x2A, 0, 0, 0, 0, 0, 0, 0, 0, 100]), Some(100));
    assert_eq!(r::battery_percent(&[0x2A, 0, 0, 0, 0, 0, 0, 0, 0, 101]), None);
    assert_eq!(r::battery_percent(&[0x2A, 0, 0, 0, 0, 0, 0, 0, 0]), None);
    assert_eq!(r::battery_percent(&[0x2B, 0, 0, 0, 0, 0, 0, 0, 0, 50]), None);
    assert_eq!(r::battery_percent(&[]), None);
    assert_eq!(r::status_request(32)[..3], [0x2A, 0, 0]);
    assert_eq!(r::status_request(32).len(), 32);
}

#[test]
fn no_frame_passes_the_allow_list_the_read_is_feature_in_only() {
    // the status read is a HidD_GetFeature (a FeatureIn, which sends nothing and passes by its kind); its blank `out` frame
    // would be a write if it were ever sent as a SetFeature
    assert!(!r::read_request_allowed(&r::status_request(32)));
    assert!(!r::read_request_allowed(&r::status_request(10)));
    // the register channel's reads and writes (report 0xBA), and anything else
    for out in [vec![0xBA, 0xA5, 0xA4, 0x04, 0x80, 0x08], vec![0xBA, 0xA5, 0xA5, 0x04, 0x80, 0x08], vec![0x2B, 0, 0, 0, 0, 0, 0, 0, 0, 0], vec![]] {
        assert!(!r::read_request_allowed(&out), "{out:02x?}");
    }
    let mut junk = r::status_request(32);
    junk[5] = 1;
    assert!(!r::read_request_allowed(&junk));
    assert!(!r::read_request_allowed(&[0x2A, 0, 0]));
}

#[test]
fn the_table_is_openmouses_supported_list() {
    // supported-devices-data.ts: VT9 Pro [0x1205, 0x4405]; both need the configuration collection (0xFF00 / 0x000E) like
    // OpenMouse's isSupported, and the status collection to read through
    for (pid, wireless) in [(0x1205u16, true), (0x4405, false)] {
        let (c, s) = (config(pid), status(pid));
        let m = r::matches(&[&c, &s], 0x24AE, pid).unwrap();
        assert_eq!((m.family, m.model, m.wireless, m.variant), (Family::Rapoo, Some("Rapoo VT9 Pro"), Some(wireless), pid));
        assert_eq!(m.config.path, s.path, "the status collection is where the feature report is read");
        assert_eq!(family::find(&[c.clone(), s.clone()], 0x24AE, pid).map(|m| m.family), Some(Family::Rapoo));
        // either collection missing: nothing to talk to
        assert!(r::matches(&[&s], 0x24AE, pid).is_none());
        assert!(r::matches(&[&c], 0x24AE, pid).is_none());
    }
    // Rapoo's other models (M1620, MT760 Pro, VT7 Air Max V2, V25, V25S, VT200, VT3, VT7 Max) are "pending" on the list
    for pid in [0x1206u16, 0x4406, 0x0000] {
        assert!(r::matches(&[&config(pid), &status(pid)], 0x24AE, pid).is_none(), "{pid:04x}");
    }
    assert!(r::matches(&[&config(0x1205), &status(0x1205)], 0x24AF, 0x1205).is_none());
    // a status report too small to hold the percentage
    let small = hid_info("small", 0x24AE, 0x1205, 0xFF0B, 0x0104, (0, 0, 8));
    assert!(r::matches(&[&config(0x1205), &small], 0x24AE, 0x1205).is_none());
}

#[test]
fn caps_are_battery_only() {
    let c = r::caps();
    assert!(c.dpi.is_none() && c.polling.is_empty() && c.lift_off.is_empty() && c.battery);
}

fn matched(pid: u16) -> bu_mouse::family::Matched {
    r::matches(&[&config(pid), &status(pid)], 0x24AE, pid).unwrap()
}

#[test]
fn reading_asks_the_one_feature_report() {
    let mut answer = vec![0u8; 32];
    answer[..10].copy_from_slice(&[0x2A, 0x01, 0x00, 0x00, 0x04, 0x21, 0x50, 0x00, 0x00, 0x62]);
    let mut p = ScriptPort::new(status(0x4405), move |out, how| {
        assert_eq!(*how, HidTransfer::FeatureIn { reply_id: 0x2A, reply_len: 32, wait_ms: 0 });
        assert_eq!(out[0], 0x2A);
        Ok(answer.clone())
    });
    let res = r::Rapoo.read(&matched(0x4405), &mut p, ReadOptions::default()).unwrap();
    assert_eq!(p.sent.len(), 1);
    assert!(matches!(p.sent[0].1, HidTransfer::FeatureIn { .. }), "a read is a HidD_GetFeature only");
    assert_eq!((res.online, res.battery_percent, res.charging, res.dpi, res.polling_hz, res.lift_off, res.wireless), (true, Some(98), None, None, None, None, Some(false)));
    assert_eq!(res.caps, r::caps());
}

#[test]
fn no_answer_is_no_battery() {
    let mut p = ScriptPort::new(status(0x1205), |_, _| Err(Error::BadAnswer("no answer".into())));
    let res = r::Rapoo.read(&matched(0x1205), &mut p, ReadOptions::default()).unwrap();
    assert_eq!((res.online, res.battery_percent, res.wireless), (false, None, Some(true)));
    // a percentage above 100 is "online, no reading"
    let mut p = ScriptPort::new(status(0x1205), |_, _| {
        let mut v = vec![0u8; 32];
        v[0] = 0x2A;
        v[9] = 255;
        Ok(v)
    });
    let res = r::Rapoo.read(&matched(0x1205), &mut p, ReadOptions::default()).unwrap();
    assert_eq!((res.online, res.battery_percent), (true, None));
    let mut gone = ScriptPort::new(status(0x1205), |_, _| Err(Error::MouseGone("r".into())));
    assert!(matches!(r::Rapoo.read(&matched(0x1205), &mut gone, ReadOptions::default()), Err(Error::MouseGone(_))));
}

#[test]
fn nothing_can_be_written() {
    let mut p = ScriptPort::new(status(0x1205), |_, _| panic!("nothing may be sent"));
    for s in [Setting::Dpi(800), Setting::Polling(1000), Setting::LiftOff(10)] {
        assert!(r::Rapoo.write(&matched(0x1205), &mut p, s).is_err(), "{s:?}");
    }
    assert!(p.sent.is_empty());
}

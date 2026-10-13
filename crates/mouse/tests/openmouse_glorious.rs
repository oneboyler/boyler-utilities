//! Order 105 check, Glorious (classic protocol): our requests = the bytes OpenMouse sends for the same setting (encode), and
//! answers shaped the way OpenMouse reads them decode to the same values (decode). The expected bytes are worked out by hand
//! from OpenMouse (@openmouse/protocol 0.40.1: `dist/glorious-classic/index.js`, `dist/drivers/glorious/classic-hid.js`,
//! `dist/drivers/vendors.js`), never from our own encoder.
//!
//! How OpenMouse counts: a command is a 64-byte body built with `bfr(body, n, v)` = `body[n - 1] = v` (the source tools' 65-byte
//! buffer minus the report-id slot), sent with `sendFeatureReport(0, body)`. Windows wants the report id in front, so
//! OpenMouse's `bfr(n)` is exactly our byte n, and the buffer is 65 bytes long.

use bu_mouse::error::Error;
use bu_mouse::family::{self, Driver, Family, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::glorious as g;
use bu_mouse::os::{HidInfo, HidTransfer};
use std::cell::RefCell;
use std::rc::Rc;

/// A 65-byte request with the given (index, value) bytes set.
fn req(set: &[(usize, u8)]) -> Vec<u8> {
    let mut v = vec![0u8; 65];
    for (i, b) in set {
        v[*i] = *b;
    }
    v
}

/// The config channel of a classic Glorious as Windows lists it: a vendor collection with one 64-byte feature report.
fn cfg(pid: u16, page: u16) -> HidInfo {
    let mut h = hid_info(&format!("cfg{pid:04x}"), 0x258A, pid, page, 0x00, (0, 0, 65));
    h.interface = Some(2);
    h
}

// ---------------------------------------------------------------- encode

#[test]
fn polling_requests_are_openmouses() {
    // buildGloriousClassicPollingRatePayload(ms): bfr[3]=0x02 bfr[4]=0x01 bfr[5]=0x01 bfr[7]=ms; 1000 Hz = 1 ms, 500 = 2,
    // 250 = 4, 125 = 8 (GLORIOUS_CLASSIC_POLLING_RATES)
    for (hz, ms) in [(1000, 1u8), (500, 2), (250, 4), (125, 8)] {
        assert_eq!(g::polling_interval(hz), Some(ms), "{hz} Hz");
        assert_eq!(g::polling_request(ms), req(&[(3, 2), (4, 1), (5, 1), (7, ms)]), "{hz} Hz");
    }
    // no 2K / 4K / 8K on this generation
    for hz in [0, 100, 2000, 4000, 8000] {
        assert_eq!(g::polling_interval(hz), None, "{hz}");
    }
}

#[test]
fn lift_off_requests_are_openmouses() {
    // buildGloriousClassicLiftOffPayload(mm): bfr[3]=0x02 bfr[4]=0x01 bfr[5]=0x01 bfr[6]=0x07 bfr[7]=mm - 1
    assert_eq!(g::lift_off_request(1), req(&[(3, 2), (4, 1), (5, 1), (6, 7), (7, 0)]));
    assert_eq!(g::lift_off_request(2), req(&[(3, 2), (4, 1), (5, 1), (6, 7), (7, 1)]));
}

#[test]
fn the_battery_request_is_openmouses() {
    // buildGloriousClassicBatteryRequestPayload: bfr[3]=0x02 bfr[4]=0x02 bfr[6]=0x83
    assert_eq!(g::battery_request(), req(&[(3, 2), (4, 2), (6, 0x83)]));
}

// ---------------------------------------------------------------- decode

/// An answer as Windows hands it over: report id 0, then OpenMouse's body (status at body[0] = bfr[1], echo body[5] = bfr[6],
/// charging body[6] = bfr[7], percent body[7] = bfr[8]).
fn answer(status: u8, echo: u8, charging: u8, percent: u8) -> Vec<u8> {
    req(&[(1, status), (6, echo), (7, charging), (8, percent)])
}

#[test]
fn battery_answers_decode_like_openmouse() {
    use g::LinkState::*;
    // normal (0xa1), 73 %, not charging
    let b = g::decode_battery(&answer(0xA1, 0x83, 0, 73));
    assert_eq!((b.state, b.percent, b.charging), (Normal, Some(73), false));
    // charging flag is exactly 1
    assert!(g::decode_battery(&answer(0xA1, 0x83, 1, 50)).charging);
    assert!(!g::decode_battery(&answer(0xA1, 0x83, 2, 50)).charging);
    // percent 0 -> 1 (`rawPercent === 0 ? 1`), 100 ok, above 100 nothing
    assert_eq!(g::decode_battery(&answer(0xA1, 0x83, 0, 0)).percent, Some(1));
    assert_eq!(g::decode_battery(&answer(0xA1, 0x83, 0, 100)).percent, Some(100));
    assert_eq!(g::decode_battery(&answer(0xA1, 0x83, 0, 101)).percent, None);
    // asleep 0xa4, waking 0xa0, 0xa2 / 0xa3 / anything else unknown: never a percent
    for (status, state) in [(0xA4, Asleep), (0xA0, WakingUp), (0xA2, Unknown), (0xA3, Unknown), (0x00, Unknown)] {
        let b = g::decode_battery(&answer(status, 0x83, 1, 80));
        assert_eq!((b.state, b.percent, b.charging), (state, None, false), "{status:#x}");
    }
    // an answer that does not echo 0x83 is "Unknown"
    let b = g::decode_battery(&answer(0xA1, 0x82, 0, 80));
    assert_eq!((b.state, b.percent), (Unknown, None));
    assert_eq!(g::decode_battery(&[]).state, Unknown);
}

// ---------------------------------------------------------------- the device table

#[test]
fn the_device_table_is_the_supported_list() {
    // (pid, model, receiver): Model O 2 PRO 0x2019 / 0x2033, Model O 0x2011 / 0x2022, Model D Wireless 0x2012 / 0x2023,
    // Model D 0x2012, Model O Pro 0x2015 / 0x2027 - eight distinct ids
    let table = [
        (0x2019u16, "Glorious Model O 2 PRO", false),
        (0x2033, "Glorious Model O 2 PRO", true),
        (0x2011, "Glorious Model O", false),
        (0x2022, "Glorious Model O", true),
        (0x2012, "Glorious Model D / Model D Wireless", false),
        (0x2023, "Glorious Model D Wireless", true),
        (0x2015, "Glorious Model O Pro", false),
        (0x2027, "Glorious Model O Pro", true),
    ];
    assert_eq!(g::PRODUCTS.len(), table.len());
    for (pid, model, receiver) in table {
        // the config channel is found on any of the three pages OpenMouse names
        for page in [0xFF00u16, 0xFF01, 0xFFFF] {
            let info = cfg(pid, page);
            let m = g::matches(&[&info], 0x258A, pid).unwrap_or_else(|| panic!("{pid:04x} on page {page:#x}"));
            assert_eq!((m.family, m.model, m.wireless), (Family::Glorious, Some(model), Some(receiver)), "{pid:04x}");
            assert_eq!(m.config.path, info.path);
            let caps = g::caps(g::product(pid).unwrap());
            assert_eq!((caps.dpi, caps.polling.clone(), caps.lift_off.clone(), caps.battery), (None, vec![125, 250, 500, 1000], vec![10, 20], receiver), "{pid:04x}");
            assert_eq!(family::find(std::slice::from_ref(&info), 0x258A, pid).unwrap().family, Family::Glorious);
        }
    }
    // not matched: another product id of the vendor (Redragon's M690 PRO shares 0x258a), another vendor id with a Glorious id,
    // the ids OpenMouse's other Glorious clients own, a non-vendor page, a config report that is not 64 bytes, no feature report
    assert!(g::matches(&[&cfg(0x2013, 0xFF00)], 0x258A, 0x2013).is_none(), "Model O- is not on the supported list");
    assert!(g::matches(&[&cfg(0x201b, 0xFF00)], 0x258A, 0x201b).is_none(), "Model O2 Pro 4K/8K has its own (unsupported) client");
    assert!(g::matches(&[&cfg(0x2019, 0xFF00)], 0x093A, 0x2019).is_none());
    assert!(g::matches(&[&cfg(0x822a, 0xFF00)], 0x093A, 0x822a).is_none(), "the Pixart Model O 2 family is not on the list");
    let mouse = hid_info("mi_00", 0x258A, 0x2019, 0x01, 0x02, (8, 0, 0));
    assert!(g::matches(&[&mouse], 0x258A, 0x2019).is_none());
    let mut odd = cfg(0x2019, 0xFF00);
    odd.feature_len = 264;
    assert!(g::matches(&[&odd], 0x258A, 0x2019).is_none(), "isConfirmedReportLength: only 64-byte reports are written");
    odd.feature_len = 0;
    assert!(g::matches(&[&odd], 0x258A, 0x2019).is_none());
    // the config collection is picked among the others
    let m = g::matches(&[&mouse, &cfg(0x2019, 0xFF00)], 0x258A, 0x2019).unwrap();
    assert_eq!(m.config.usage_page, 0xFF00);
}

#[test]
fn no_other_family_claims_a_glorious_id() {
    for p in g::PRODUCTS {
        let info = cfg(p.pid, 0xFF00);
        let own = [&info];
        assert!(bu_mouse::pulsar::matches_cmouse(&own, 0x258A, p.pid).is_none());
        assert!(bu_mouse::pulsar::matches_xs1(&own, 0x258A, p.pid).is_none());
        assert!(bu_mouse::logitech::matches(&own, 0x258A, p.pid).is_none());
        assert!(bu_mouse::razer::matches(&own, 0x258A, p.pid).is_none());
        assert!(bu_mouse::steelseries::matches(&own, 0x258A, p.pid).is_none());
    }
}

// ---------------------------------------------------------------- the driver

/// A fake classic Glorious receiver: answers the battery request with `status` / `percent`, remembers every request.
struct Mouse {
    status: u8,
    percent: u8,
    charging: u8,
    seen: Vec<Vec<u8>>,
    /// answer with something that is not the battery echo
    deaf: bool,
}

fn port_for(info: &HidInfo, m: Rc<RefCell<Mouse>>) -> ScriptPort {
    ScriptPort::new(info.clone(), move |out, how| {
        let mut m = m.borrow_mut();
        m.seen.push(out.to_vec());
        match how {
            HidTransfer::Feature { reply_id: 0, reply_len: 65, wait_ms: 60 } => {
                assert_eq!(out, g::battery_request().as_slice());
                if m.deaf {
                    return Ok(vec![0u8; 65]);
                }
                Ok(answer(m.status, 0x83, m.charging, m.percent))
            }
            HidTransfer::FeatureOut => Ok(Vec::new()),
            other => panic!("unexpected transfer {other:?}"),
        }
    })
}

fn mouse(status: u8, percent: u8) -> Rc<RefCell<Mouse>> {
    Rc::new(RefCell::new(Mouse { status, percent, charging: 0, seen: Vec::new(), deaf: false }))
}

#[test]
fn a_receiver_reads_the_battery() {
    let info = cfg(0x2033, 0xFF00);
    let m = g::matches(&[&info], 0x258A, 0x2033).unwrap();
    let mouse = mouse(0xA1, 64);
    mouse.borrow_mut().charging = 1;
    let mut port = port_for(&info, mouse.clone());
    let s = g::Glorious.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((s.online, s.battery_percent, s.charging, s.wireless), (true, Some(64), Some(true), Some(true)));
    assert_eq!((s.dpi, s.polling_hz, s.lift_off), (None, None, None), "the protocol cannot read them");
    assert!(s.caps.battery && s.caps.dpi.is_none());
    assert_eq!(mouse.borrow().seen, vec![req(&[(3, 2), (4, 2), (6, 0x83)])]);
    assert!(port.sent.iter().all(|s| g::read_request_allowed(&s.0)));
}

#[test]
fn a_wired_id_sends_nothing_when_read() {
    let info = cfg(0x2019, 0xFF00);
    let m = g::matches(&[&info], 0x258A, 0x2019).unwrap();
    let mut port = port_for(&info, mouse(0xA1, 50));
    let s = g::Glorious.read(&m, &mut port, ReadOptions::default()).unwrap();
    // OpenMouse: `this.isWireless() ? readBattery() : null`
    assert_eq!((s.online, s.battery_percent, s.wireless, s.caps.battery), (true, None, Some(false), false));
    assert!(port.sent.is_empty());
}

#[test]
fn a_sleeping_receiver_is_offline_with_no_battery() {
    let info = cfg(0x2027, 0xFF00);
    let m = g::matches(&[&info], 0x258A, 0x2027).unwrap();
    for status in [0xA4u8, 0xA0] {
        let mut port = port_for(&info, mouse(status, 80));
        let s = g::Glorious.read(&m, &mut port, ReadOptions::default()).unwrap();
        assert_eq!((s.online, s.battery_percent, s.charging), (false, None, None), "{status:#x}");
        assert_eq!(port.sent.len(), 1, "asked once, no loop");
    }
    // an answer without the 0x83 echo: the receiver is there but says nothing useful -> no battery, still listed
    let deaf = mouse(0xA1, 80);
    deaf.borrow_mut().deaf = true;
    let mut port = port_for(&info, deaf);
    let s = g::Glorious.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!(s.battery_percent, None);
}

#[test]
fn setting_polling_sends_the_one_command() {
    let info = cfg(0x2011, 0xFFFF);
    let m = g::matches(&[&info], 0x258A, 0x2011).unwrap();
    let mouse = mouse(0xA1, 50);
    let mut port = port_for(&info, mouse.clone());
    let c = g::Glorious.write(&m, &mut port, Setting::Polling(500)).unwrap();
    assert_eq!((c.now, c.before), (500, None));
    assert_eq!(mouse.borrow().seen, vec![req(&[(3, 2), (4, 1), (5, 1), (7, 2)])]);
    assert_eq!(port.sent[0].1, HidTransfer::FeatureOut);
    assert!(matches!(g::Glorious.write(&m, &mut port, Setting::Polling(8000)), Err(Error::OutOfRange { .. })));
    assert_eq!(mouse.borrow().seen.len(), 1, "a refused rate sends nothing");
}

#[test]
fn setting_lift_off_sends_the_one_command() {
    let info = cfg(0x2015, 0xFF01);
    let m = g::matches(&[&info], 0x258A, 0x2015).unwrap();
    let mouse = mouse(0xA1, 50);
    let mut port = port_for(&info, mouse.clone());
    assert_eq!(g::Glorious.write(&m, &mut port, Setting::LiftOff(20)).unwrap().now, 20);
    assert_eq!(g::Glorious.write(&m, &mut port, Setting::LiftOff(10)).unwrap().now, 10);
    assert_eq!(mouse.borrow().seen, vec![req(&[(3, 2), (4, 1), (5, 1), (6, 7), (7, 1)]), req(&[(3, 2), (4, 1), (5, 1), (6, 7), (7, 0)])]);
    for t in [0, 5, 15, 30] {
        assert!(matches!(g::Glorious.write(&m, &mut port, Setting::LiftOff(t)), Err(Error::OutOfRange { .. })), "{t}");
    }
    assert_eq!(mouse.borrow().seen.len(), 2);
}

#[test]
fn dpi_is_held_back_because_it_would_overwrite_the_saved_stages() {
    let info = cfg(0x2019, 0xFF00);
    let m = g::matches(&[&info], 0x258A, 0x2019).unwrap();
    let mouse = mouse(0xA1, 50);
    let mut port = port_for(&info, mouse.clone());
    let e = g::Glorious.write(&m, &mut port, Setting::Dpi(1600)).unwrap_err();
    assert!(matches!(e, Error::OutOfRange { .. }), "{e}");
    assert!(mouse.borrow().seen.is_empty(), "nothing is sent");
}

// ---------------------------------------------------------------- the allow-list

#[test]
fn only_the_battery_request_passes_the_allow_list() {
    assert!(g::read_request_allowed(&g::battery_request()));
    assert!(family::read_request_allowed(Family::Glorious, &g::battery_request()));
    for w in [g::polling_request(1), g::polling_request(8), g::lift_off_request(1), g::lift_off_request(2), vec![0u8; 65], vec![], g::battery_request()[..64].to_vec()] {
        assert!(!g::read_request_allowed(&w), "{w:02x?}");
    }
    // OpenMouse's RGB / debounce / DPI-stage commands (class 0x02, other lengths) never pass either
    let mut rgb = g::battery_request();
    rgb[4] = 0x05;
    let mut dpi = req(&[(3, 2), (4, 0x12), (5, 1), (6, 1), (7, 1), (8, 4)]);
    dpi[9] = 0x03;
    assert!(!g::read_request_allowed(&rgb) && !g::read_request_allowed(&dpi));
    let mut longer = g::battery_request();
    longer.push(0);
    assert!(!g::read_request_allowed(&longer));
}

#[test]
fn an_os_error_on_the_battery_read_is_no_battery_not_a_failed_read() {
    // OpenMouse: readBattery().catch(() => null); only a mouse that is gone fails the read
    let info = cfg(0x2027, 0xFF00);
    let m = g::matches(&[&info], 0x258A, 0x2027).unwrap();
    let mut port = ScriptPort::new(info.clone(), |_, _| Err(Error::Os { op: "HidD_GetFeature".into(), code: 31 }));
    let s = g::Glorious.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((s.online, s.battery_percent, s.charging), (false, None, None));
    assert_eq!(port.sent.len(), 1);
    assert!(port.sent.iter().all(|s| g::read_request_allowed(&s.0)));
    let mut gone = ScriptPort::new(info, |_, _| Err(Error::MouseGone("g".into())));
    assert!(matches!(g::Glorious.read(&m, &mut gone, ReadOptions::default()), Err(Error::MouseGone(_))));
}

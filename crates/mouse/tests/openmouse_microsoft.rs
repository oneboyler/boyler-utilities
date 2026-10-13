//! Order 105 check, Microsoft (Pro / Classic IntelliMouse): our feature reports = the bytes OpenMouse sends for the same
//! setting (encode), and answers shaped the way OpenMouse reads them decode to the same values (decode). Expected bytes are
//! worked out by hand from OpenMouse (@openmouse/protocol 0.40.1: `dist/microsoft/index.js`, `dist/drivers/microsoft/hid.js`).
//!
//! OpenMouse: `sendFeatureReport(0x24, payload)` with payload `[property, length or 0x01, data...]`, zero-padded to
//! getWriteLength() - 1 (Pro 72, Classic 31). Windows puts the report id in front: OpenMouse's payload byte i is our byte i + 1.

use bu_mouse::family::{self, Driver, Family, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::microsoft as ms;
use bu_mouse::os::HidTransfer;
use bu_mouse::Error;
use std::cell::RefCell;
use std::rc::Rc;

/// A report of `len` bytes from its non-zero bytes.
fn rep(len: usize, bytes: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; len];
    v[..bytes.len()].copy_from_slice(bytes);
    v
}

fn pro_info() -> bu_mouse::os::HidInfo {
    hid_info("ms-pro", 0x045E, 0x082A, 0xFF07, 0x0212, (0, 0, 73))
}
fn classic_info() -> bu_mouse::os::HidInfo {
    hid_info("ms-classic", 0x045E, 0x0823, 0x000C, 0x0001, (0, 0, 32))
}

// ---------------------------------------------------------------- encode

#[test]
fn pro_requests_are_openmouses_bytes() {
    // readProperty(0x97): payload [0x97, 0x01] → 72 bytes; with the id 0x24 in front: 73
    assert_eq!(ms::read_request(0x97, 73), rep(73, &[0x24, 0x97, 0x01]));
    assert_eq!(ms::read_request(0x84, 73), rep(73, &[0x24, 0x84, 0x01]));
    // setDpi(1600): finalDpi 1600 = 0x0640 → writeProperty(0x96, [0x40, 0x06]) = [0x96, 2, 0x40, 0x06]
    assert_eq!(ms::pro_set_dpi(1600, 73), rep(73, &[0x24, 0x96, 0x02, 0x40, 0x06]));
    // 16000 = 0x3E80, 200 = 0x00C8
    assert_eq!(ms::pro_set_dpi(16000, 73), rep(73, &[0x24, 0x96, 0x02, 0x80, 0x3E]));
    assert_eq!(ms::pro_set_dpi(200, 73), rep(73, &[0x24, 0x96, 0x02, 0xC8, 0x00]));
    // setPollingRate: rate <= 125 → 0x02, <= 500 → 0x01, else 0x00; writeProperty(0x83, [val]) = [0x83, 1, val]
    assert_eq!(ms::pro_set_polling(125, 73), Some(rep(73, &[0x24, 0x83, 0x01, 0x02])));
    assert_eq!(ms::pro_set_polling(500, 73), Some(rep(73, &[0x24, 0x83, 0x01, 0x01])));
    assert_eq!(ms::pro_set_polling(1000, 73), Some(rep(73, &[0x24, 0x83, 0x01, 0x00])));
    assert_eq!(ms::pro_set_polling(250, 73), None, "OpenMouse offers 125 / 500 / 1000 only");
}

#[test]
fn classic_dpi_request_is_openmouses_bytes() {
    // Classic setDpi(1600): writeProperty(0x96, [0x00, 0x40, 0x06]) = [0x96, 3, 0x00, 0x40, 0x06], 31 bytes → 32 with the id
    assert_eq!(ms::classic_set_dpi(1600, 32), rep(32, &[0x24, 0x96, 0x03, 0x00, 0x40, 0x06]));
    // 3200 = 0x0C80, 400 = 0x0190
    assert_eq!(ms::classic_set_dpi(3200, 32), rep(32, &[0x24, 0x96, 0x03, 0x00, 0x80, 0x0C]));
    assert_eq!(ms::classic_set_dpi(400, 32), rep(32, &[0x24, 0x96, 0x03, 0x00, 0x90, 0x01]));
}

// ---------------------------------------------------------------- decode

#[test]
fn pro_answers_decode_like_openmouse() {
    // readDpi: view.getUint16(3, true) on the payload [0x97, 0, 2, lo, hi]; 1600 = 0x0640
    assert_eq!(ms::pro_dpi(&[0x97, 0x00, 0x02, 0x40, 0x06]), Some(1600));
    assert_eq!(ms::pro_dpi(&[0x97, 0x00, 0x02, 0x80, 0x3E]), Some(16000));
    assert_eq!(ms::pro_dpi(&[0x97, 0x00, 0x02, 0x40]), None, "too short");
    // readPollingRate: getUint8(3): 2 → 125, 1 → 500, 0 (and anything else) → 1000
    for (code, hz) in [(2u8, 125), (1, 500), (0, 1000), (7, 1000)] {
        assert_eq!(ms::pro_polling(&[0x84, 0x00, 0x01, code]), Some(hz), "{code}");
    }
    assert_eq!(ms::pro_polling(&[0x84, 0x00, 0x01]), None);
}

// ---------------------------------------------------------------- allow-list

#[test]
fn only_the_pro_reads_pass_the_allow_list() {
    assert!(ms::read_request_allowed(&ms::read_request(0x97, 73)));
    assert!(ms::read_request_allowed(&ms::read_request(0x84, 73)));
    assert!(ms::read_request_allowed(&ms::read_request(0x97, 32)));
    for w in [ms::pro_set_dpi(1600, 73), ms::classic_set_dpi(1600, 32), ms::pro_set_polling(500, 73).unwrap(), ms::read_request(0xB3, 73), ms::read_request(0xB6, 73)] {
        assert!(!ms::read_request_allowed(&w), "{:02x?}", &w[..6]);
    }
    let mut junk = ms::read_request(0x97, 73);
    junk[10] = 1;
    assert!(!ms::read_request_allowed(&junk));
    let mut other_id = ms::read_request(0x97, 73);
    other_id[0] = 0x27;
    assert!(!ms::read_request_allowed(&other_id));
    assert!(!ms::read_request_allowed(&[0x24, 0x97]));
}

// ---------------------------------------------------------------- device table

#[test]
fn the_table_is_openmouses_supported_list() {
    // supported-devices-data.ts: Pro Intellimouse [0x82a], Classic Intellimouse [0x823]
    let pro = pro_info();
    let m = ms::matches(&[&pro], 0x045E, 0x082A).unwrap();
    assert_eq!((m.family, m.model, m.wireless, m.variant), (Family::Microsoft, Some("Microsoft Pro Intellimouse"), Some(false), 0x082A));
    assert_eq!(m.config.path, pro.path);
    let classic = classic_info();
    assert_eq!(family::find(std::slice::from_ref(&pro), 0x045E, 0x082A).map(|m| m.family), Some(Family::Microsoft));
    // the Classic is held back (its DPI can only be written blind): its collection is not claimed
    assert!(ms::matches(&[&classic], 0x045E, 0x0823).is_none());
    assert!(family::find(std::slice::from_ref(&classic), 0x045E, 0x0823).is_none());
    // another Microsoft id, another vendor
    assert!(ms::matches(&[&pro], 0x045E, 0x0000).is_none());
    assert!(ms::matches(&[&pro], 0x046D, 0x082A).is_none());
    // each model needs ITS collection: the Pro's page on the Classic's id and the other way round do not match
    assert!(ms::matches(&[&pro], 0x045E, 0x0823).is_none());
    assert!(ms::matches(&[&classic], 0x045E, 0x082A).is_none());
    // and a feature report big enough for the request
    let small = hid_info("small", 0x045E, 0x082A, 0xFF07, 0x0212, (0, 0, 10));
    assert!(ms::matches(&[&small], 0x045E, 0x082A).is_none());
    // the plain mouse collection is no control channel
    let mouse = hid_info("mouse", 0x045E, 0x082A, 0x01, 0x02, (8, 0, 0));
    assert_eq!(ms::matches(&[&mouse, &pro], 0x045E, 0x082A).map(|m| m.config.path), Some(pro.path.clone()));
    assert!(ms::matches(&[&mouse], 0x045E, 0x082A).is_none());
}

#[test]
fn caps_are_what_openmouse_offers() {
    let p = ms::caps(0x082A);
    let d = p.dpi.unwrap();
    assert_eq!((d.min(), d.max()), (200, 16000));
    assert!(d.allows(250) && !d.allows(225) && !d.allows(16050));
    assert_eq!(p.polling, vec![125, 500, 1000]);
    assert!(p.lift_off.is_empty() && !p.battery);
    let c = ms::caps(0x0823);
    let d = c.dpi.unwrap();
    assert_eq!((d.min(), d.max()), (400, 3200));
    assert!(d.allows(1600) && !d.allows(1500) && !d.allows(3400));
    assert!(c.polling.is_empty(), "OpenMouse marks the Classic's polling read-only");
}

// ---------------------------------------------------------------- the driver against a scripted mouse

struct Fake {
    dpi: u16,
    poll_code: u8,
    /// the property of the last request (a FeatureIn fetches its answer)
    last: u8,
    /// answer this many fetches with another property's report (a stale one) before the right one
    stale: u32,
    /// answer nothing at all
    silent: bool,
    fetches: u32,
    writes: Vec<Vec<u8>>,
}

fn fake() -> Rc<RefCell<Fake>> {
    Rc::new(RefCell::new(Fake { dpi: 800, poll_code: 0, last: 0, stale: 0, silent: false, fetches: 0, writes: Vec::new() }))
}

fn pro_port(st: &Rc<RefCell<Fake>>) -> ScriptPort {
    let st = st.clone();
    ScriptPort::new(pro_info(), move |out, how| {
        let mut s = st.borrow_mut();
        let answer = |s: &mut Fake| -> Result<Vec<u8>, Error> {
            s.fetches += 1;
            if s.silent {
                return Err(Error::BadAnswer("no answer".into()));
            }
            if s.stale > 0 {
                s.stale -= 1;
                return Ok(rep(73, &[0x27, 0x42, 0x00, 0x02]));
            }
            Ok(match s.last {
                0x97 => rep(73, &[0x27, 0x97, 0x00, 0x02, s.dpi as u8, (s.dpi >> 8) as u8]),
                0x84 => rep(73, &[0x27, 0x84, 0x00, 0x01, s.poll_code]),
                p => panic!("unexpected property {p:#04x}"),
            })
        };
        match how {
            HidTransfer::Feature { reply_id, reply_len, wait_ms } => {
                assert_eq!((*reply_id, *reply_len, *wait_ms, out[0], out[2]), (0x27, 73, 50, 0x24, 1), "the first fetch comes 50 ms after the request");
                s.last = out[1];
                answer(&mut s)
            }
            HidTransfer::FeatureIn { reply_id, reply_len, wait_ms } => {
                assert_eq!((*reply_id, *reply_len, *wait_ms), (0x27, 73, 10), "then every 10 ms, nothing sent");
                answer(&mut s)
            }
            HidTransfer::FeatureOut => {
                s.writes.push(out.to_vec());
                match out[1] {
                    0x96 => s.dpi = u16::from_le_bytes([out[3], out[4]]),
                    0x83 => s.poll_code = out[3],
                    p => panic!("unexpected write {p:#04x}"),
                }
                Ok(Vec::new())
            }
            other => panic!("unexpected transfer {other:?}"),
        }
    })
}

fn pro() -> bu_mouse::family::Matched {
    ms::matches(&[&pro_info()], 0x045E, 0x082A).unwrap()
}

#[test]
fn reading_the_pro_asks_dpi_then_polling() {
    let st = fake();
    st.borrow_mut().poll_code = 1;
    let mut p = pro_port(&st);
    let r = ms::Microsoft.read(&pro(), &mut p, ReadOptions::default()).unwrap();
    assert!(p.sent.iter().all(|s| ms::read_request_allowed(&s.0)), "read requests only");
    assert_eq!(p.sent.len(), 2);
    assert_eq!((r.online, r.dpi, r.polling_hz, r.battery_percent, r.lift_off, r.wireless), (true, Some(800), Some(500), None, None, Some(false)));
    assert_eq!(r.caps, ms::caps(0x082A));
}

#[test]
fn a_stale_answer_is_polled_past() {
    // OpenMouse polls until the echoed property is the one asked for
    let st = fake();
    st.borrow_mut().stale = 3;
    let mut p = pro_port(&st);
    let r = ms::Microsoft.read(&pro(), &mut p, ReadOptions::default()).unwrap();
    assert_eq!(r.dpi, Some(800));
    assert!(matches!(p.sent[0].1, HidTransfer::Feature { .. }) && p.sent[1..4].iter().all(|s| matches!(s.1, HidTransfer::FeatureIn { .. })));
}

#[test]
fn a_silent_pro_is_offline_without_looping_forever() {
    let st = fake();
    st.borrow_mut().silent = true;
    let mut p = pro_port(&st);
    let r = ms::Microsoft.read(&pro(), &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.online, r.dpi, r.polling_hz), (false, None, None));
    assert!(p.sent.len() <= 100, "a bounded number of polls: {}", p.sent.len());
    let mut gone = ScriptPort::new(pro_info(), |_, _| Err(Error::MouseGone("ms".into())));
    assert!(matches!(ms::Microsoft.read(&pro(), &mut gone, ReadOptions::default()), Err(Error::MouseGone(_))));
}

#[test]
fn the_pro_dpi_change_is_read_write_read() {
    let st = fake();
    let mut p = pro_port(&st);
    let c = ms::Microsoft.write(&pro(), &mut p, Setting::Dpi(1600)).unwrap();
    assert_eq!((c.now, c.before), (1600, Some(800)));
    assert_eq!(st.borrow().writes, vec![rep(73, &[0x24, 0x96, 0x02, 0x40, 0x06])]);
    // request 0x97, the write, request 0x97 again
    assert_eq!(p.sent.len(), 3);
    assert_eq!((&p.sent[0].0, &p.sent[1].1, &p.sent[2].0), (&ms::read_request(0x97, 73), &HidTransfer::FeatureOut, &ms::read_request(0x97, 73)));
    assert!(!ms::read_request_allowed(&p.sent[1].0));
}

#[test]
fn the_pro_polling_change_is_read_write_read() {
    let st = fake();
    let mut p = pro_port(&st);
    let c = ms::Microsoft.write(&pro(), &mut p, Setting::Polling(125)).unwrap();
    assert_eq!((c.now, c.before), (125, Some(1000)));
    assert_eq!(st.borrow().poll_code, 2);
    assert_eq!(st.borrow().writes, vec![rep(73, &[0x24, 0x83, 0x01, 0x02])]);
    let c = ms::Microsoft.write(&pro(), &mut pro_port(&st), Setting::Polling(500)).unwrap();
    assert_eq!((c.now, c.before), (500, Some(125)));
}

#[test]
fn values_the_mouse_cannot_store_are_refused_before_a_byte_leaves() {
    let st = fake();
    let mut p = pro_port(&st);
    for s in [Setting::Dpi(150), Setting::Dpi(225), Setting::Dpi(16050), Setting::Polling(250), Setting::Polling(2000), Setting::LiftOff(20)] {
        assert!(ms::Microsoft.write(&pro(), &mut p, s).is_err(), "{s:?}");
    }
    assert!(p.sent.is_empty());
}

#[test]
fn a_change_the_pro_does_not_keep_is_an_error() {
    // a mouse that ignores the DPI write
    let st = fake();
    let inner_state = st.clone();
    let mut inner = pro_port(&st);
    let mut p = ScriptPort::new(pro_info(), move |out, how| {
        use bu_mouse::family::Port;
        let r = inner.exchange(out, how);
        if matches!(how, HidTransfer::FeatureOut) {
            inner_state.borrow_mut().dpi = 800;
        }
        r
    });
    let e = ms::Microsoft.write(&pro(), &mut p, Setting::Dpi(1600)).unwrap_err();
    assert!(matches!(e, Error::BadAnswer(ref m) if m.contains("800")), "{e:?}");
}


#[test]
fn the_classic_dpi_is_held_back_and_nothing_is_sent() {
    // matches() does not claim the Classic; a hand-made Matched (what the driver would get if it did) still sends nothing
    let m = bu_mouse::family::Matched { family: Family::Microsoft, model: Some("Microsoft Classic Intellimouse"), wireless: Some(false), config: classic_info(), variant: 0x0823 };
    let mut p = ScriptPort::new(classic_info(), |_, how| panic!("the Classic must get no transfer, not {how:?}"));
    let r = ms::Microsoft.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.dpi, r.polling_hz), (None, None));
    for s in [Setting::Dpi(1600), Setting::Dpi(1500), Setting::Dpi(3400), Setting::Polling(500), Setting::LiftOff(10)] {
        assert!(ms::Microsoft.write(&m, &mut p, s).is_err(), "{s:?}");
    }
    assert!(p.sent.is_empty());
}

#[test]
fn a_request_that_fails_to_go_out_is_an_error_not_a_stale_answer() {
    // the first transfer (the SetFeature of the request) fails with an OS error: report 0x27 may hold an older answer, so
    // nothing may be polled
    let mut p = ScriptPort::new(pro_info(), |_, _| Err(Error::Os { op: "HidD_SetFeature".into(), code: 31 }));
    let e = ms::Microsoft.write(&pro(), &mut p, Setting::Dpi(1600)).unwrap_err();
    assert!(matches!(e, Error::Os { .. }), "{e:?}");
    assert_eq!(p.sent.len(), 1, "one try, no polling, no write");
    let mut p = ScriptPort::new(pro_info(), |_, _| Err(Error::Os { op: "HidD_SetFeature".into(), code: 31 }));
    assert!(ms::Microsoft.read(&pro(), &mut p, ReadOptions::default()).is_err());
    assert_eq!(p.sent.len(), 1);
}

#[test]
fn a_fetch_that_fails_ends_the_polling_at_once() {
    // the request goes out, every later fetch fails with an OS error (each could block up to a second): one fetch, then give up
    let st = fake();
    let mut inner = pro_port(&st);
    let mut p = ScriptPort::new(pro_info(), move |out, how| {
        use bu_mouse::family::Port;
        match how {
            HidTransfer::Feature { .. } => Err(Error::BadAnswer("not ready".into())),
            HidTransfer::FeatureIn { .. } => Err(Error::Os { op: "HidD_GetFeature".into(), code: 1167 }),
            _ => inner.exchange(out, how),
        }
    });
    let r = ms::Microsoft.read(&pro(), &mut p, ReadOptions::default()).unwrap();
    assert!(!r.online);
    assert_eq!(p.sent.len(), 2, "the request, one failing fetch, then stop");
}

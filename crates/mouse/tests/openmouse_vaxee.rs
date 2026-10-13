//! Order 105 check, VAXEE: our requests = the bytes OpenMouse sends for the same setting (encode), and answers shaped the way
//! OpenMouse reads them decode to the same values (decode). The expected bytes are worked out by hand from OpenMouse
//! (@openmouse/protocol 0.40.1: `dist/vaxee/index.js`, `dist/drivers/vaxee/hid.js`), never from our own encoder.
//!
//! OpenMouse's feature report (id 0x0e) is 63 bytes without the id: `[0]` 0xa5 · `[1]` command · `[2]` 1 read / 2 write ·
//! `[3]` data length · `[4..]` values (vaxeeRequest). An answer is `[0]` 0xa5 · `[1]` command · `[2]` 3 · `[3]` 1 · `[4]` data
//! length · `[5..]` data (vaxeeReply). Windows puts the report id 0x0e in front: OpenMouse's byte i = our byte i + 1.

use bu_mouse::family::{Driver, Family, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::os::{HidInfo, HidTransfer};
use bu_mouse::vaxee as vx;
use bu_mouse::Error;
use std::cell::RefCell;
use std::rc::Rc;

/// A request as Windows sends it: id 0x0e, then `a5 command kind length values...`, zero padded to 64 bytes. By hand.
fn rq(command: u8, kind: u8, len: u8, values: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 64];
    v[0] = 0x0e;
    v[1] = 0xa5;
    v[2] = command;
    v[3] = kind;
    v[4] = len;
    v[5..5 + values.len()].copy_from_slice(values);
    v
}

/// An answer as GetFeature hands it over: id 0x0e, `a5 command 03 01 length data...`.
fn answer(command: u8, data: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 64];
    v[0] = 0x0e;
    v[1] = 0xa5;
    v[2] = command;
    v[3] = 3;
    v[4] = 1;
    v[5] = data.len() as u8;
    v[6..6 + data.len()].copy_from_slice(data);
    v
}

fn cfg(pid: u16) -> HidInfo {
    hid_info(&format!("cfg{pid:04x}"), 0x3057, pid, 0xff05, 0x01, (64, 64, 64))
}

// ---------------------------------------------------------------- encode

#[test]
fn reads_are_openmouses_vaxee_requests() {
    // exchange(VAXEE_COMMAND.mousePid = 0x0a, 2, []): a5 0a 01 02
    assert_eq!(vx::report(0x0a, 2, &[], false, 64), rq(0x0a, 1, 2, &[]));
    // dpiStage 0x02, length 1: a5 02 01 01
    assert_eq!(vx::report(0x02, 1, &[], false, 64), rq(0x02, 1, 1, &[]));
    // dpiValue 0x04, length 3, values [stage]: a5 04 01 03 02
    assert_eq!(vx::report(0x04, 3, &[2], false, 64), rq(0x04, 1, 3, &[2]));
    // polling 0x07, battery 0x0b, charging 0x10: length 1
    assert_eq!(vx::report(0x07, 1, &[], false, 64), rq(0x07, 1, 1, &[]));
    assert_eq!(vx::report(0x0b, 1, &[], false, 64), rq(0x0b, 1, 1, &[]));
    assert_eq!(vx::report(0x10, 1, &[], false, 64), rq(0x10, 1, 1, &[]));
    // the payload alone is 63 bytes, the report 64; a longer collection is padded
    assert_eq!(vx::request(0x0a, 2, &[], false).len(), 63);
    assert_eq!(vx::report(0x0a, 2, &[], false, 80).len(), 80);
}

#[test]
fn writes_are_openmouses_vaxee_requests() {
    // setDpi(1600) on stage 2: exchange(dpiValue 0x04, 3, [stage, dpi & 255, dpi >> 8], 1, write): 1600 = 0x0640
    // a5 04 02 03 | 02 40 06
    assert_eq!(vx::report(0x04, 3, &[2, 0x40, 0x06], true, 64), rq(0x04, 2, 3, &[2, 0x40, 0x06]));
    // 26000 = 0x6590 on stage 4: 90 65
    assert_eq!(vx::report(0x04, 3, &[4, 0x90, 0x65], true, 64), rq(0x04, 2, 3, &[4, 0x90, 0x65]));
    // setPollingRate: exchange(polling 0x07, 1, [code], 1, write); codes 500 -> 1, 1000 -> 2, 2000 -> 3, 4000 -> 4, 8000 -> 5
    for (hz, code) in [(500, 1), (1000, 2), (2000, 3), (4000, 4), (8000, 5)] {
        assert_eq!(vx::polling_code(hz), Some(code), "{hz}");
        assert_eq!(vx::report(0x07, 1, &[code], true, 64), rq(0x07, 2, 1, &[code]), "{hz}");
    }
    assert_eq!(vx::polling_code(250), None);
}

// ---------------------------------------------------------------- decode

#[test]
fn answers_decode_like_openmouse() {
    // vaxeeDpi: reply[6] | reply[7] << 8 (reply[5] is the stage): stage 2, 1600 = 40 06
    let a = &answer(0x04, &[2, 0x40, 0x06])[1..];
    assert_eq!(vx::decode_dpi(a), Some(1600));
    let a = &answer(0x04, &[1, 0xa0, 0x0f])[1..];
    assert_eq!(vx::decode_dpi(a), Some(4000));
    // vaxeeReply refuses: another command, a wrong envelope, a data length below what is needed, a short buffer
    assert!(vx::check_reply(&answer(0x04, &[2, 0x40, 0x06])[1..], 0x07, 1).is_none(), "another command's answer");
    let mut bad = answer(0x04, &[2, 0x40, 0x06]);
    bad[3] = 4;
    assert!(vx::check_reply(&bad[1..], 0x04, 3).is_none(), "byte 2 must be 3");
    let mut bad = answer(0x04, &[2, 0x40, 0x06]);
    bad[4] = 2;
    assert!(vx::check_reply(&bad[1..], 0x04, 3).is_none(), "byte 3 must be 1");
    let mut bad = answer(0x04, &[2, 0x40, 0x06]);
    bad[1] = 0x00;
    assert!(vx::check_reply(&bad[1..], 0x04, 3).is_none(), "byte 0 must be a5");
    assert!(vx::check_reply(&answer(0x04, &[2, 0x40])[1..], 0x04, 3).is_none(), "truncated data");
    assert!(vx::check_reply(&[0xa5, 0x04, 3, 1, 3, 2, 0x40], 0x04, 3).is_none(), "shorter than 5 + 3");
    // vaxeePollingRate: [0, 500, 1000, 2000, 4000, 8000][code]; 0 and beyond are errors
    for (c, hz) in [(1, 500), (2, 1000), (3, 2000), (4, 4000), (5, 8000)] {
        assert_eq!(vx::polling_hz(c), Some(hz));
    }
    assert_eq!((vx::polling_hz(0), vx::polling_hz(6)), (None, None));
    // battery: steps of 5, at most 20 (OpenMouse: batterySteps <= 20 ? steps * 5 : null)
    assert_eq!((vx::battery_percent(0), vx::battery_percent(13), vx::battery_percent(20), vx::battery_percent(21)), (Some(0), Some(65), Some(100), None));
}

// ---------------------------------------------------------------- the allow-list

#[test]
fn only_read_requests_pass_the_allow_list() {
    for (c, l, v) in [(0x0a, 2, vec![]), (0x02, 1, vec![]), (0x04, 3, vec![1]), (0x04, 3, vec![4]), (0x07, 1, vec![]), (0x0b, 1, vec![]), (0x10, 1, vec![])] {
        let r = vx::report(c, l, &v, false, 64);
        assert!(vx::read_request_allowed(&r), "{c:#x}");
        assert!(vx::read_request_allowed(&vx::report(c, l, &v, false, 80)), "{c:#x} on a longer report");
    }
    // every write OpenMouse sends: DPI on every stage, polling in every code
    for stage in 1..=4u8 {
        for dpi in [100u16, 800, 1600, 26000] {
            assert!(!vx::read_request_allowed(&vx::report(0x04, 3, &[stage, dpi as u8, (dpi >> 8) as u8], true, 64)), "{stage} {dpi}");
        }
    }
    for code in 1..=5u8 {
        assert!(!vx::read_request_allowed(&vx::report(0x07, 1, &[code], true, 64)), "{code}");
    }
    // the commands the client has beyond these (firmware 0x01, tracking 0x08, lod 0x09, ...) are not sent, so not allowed;
    // and a DPI read of stage 0 / 5, a write marked as a read with values, another report id, junk, too short
    for c in [0x01, 0x03, 0x05, 0x08, 0x09, 0x13, 0x14] {
        assert!(!vx::read_request_allowed(&vx::report(c, 1, &[], false, 64)), "{c:#x}");
    }
    assert!(!vx::read_request_allowed(&vx::report(0x04, 3, &[0], false, 64)));
    assert!(!vx::read_request_allowed(&vx::report(0x04, 3, &[5], false, 64)));
    assert!(!vx::read_request_allowed(&vx::report(0x07, 1, &[2], false, 64)), "a read with a value");
    let mut other_id = vx::report(0x07, 1, &[], false, 64);
    other_id[0] = 0x00;
    assert!(!vx::read_request_allowed(&other_id));
    let mut junk = vx::report(0x07, 1, &[], false, 80);
    junk[70] = 1;
    assert!(!vx::read_request_allowed(&junk));
    assert!(!vx::read_request_allowed(&[0x0e, 0xa5, 0x07, 0x01]));
    assert!(!vx::read_request_allowed(&[]));
}

// ---------------------------------------------------------------- the device table

#[test]
fn the_table_is_openmouses_supported_list() {
    // supported-devices-data.ts, status "supported": 13 VAXEE mice, pids 0x1003 - 0x1015 as listed, names from VAXEE_MOUSE_NAMES
    let want: [(u16, &str); 13] = [
        (0x1003, "VAXEE XE Wireless"),
        (0x1004, "VAXEE NP-01S Wireless"),
        (0x1005, "VAXEE AX Wireless"),
        (0x1006, "VAXEE NP-01 Wireless"),
        (0x1007, "VAXEE XE-S Wireless"),
        (0x1008, "VAXEE XE-S-L Wireless"),
        (0x1009, "VAXEE x Ninjutso Sora"),
        (0x1010, "VAXEE E1 Wireless"),
        (0x1011, "VAXEE NP-01S V2 Wireless"),
        (0x1012, "VAXEE XE V2 Wireless"),
        (0x1013, "VAXEE NP-01S Ergo Wireless"),
        (0x1014, "VAXEE NP-01S V3 Wireless"),
        (0x1015, "VAXEE NP-01 Ergo Wireless"),
    ];
    assert_eq!(vx::PRODUCTS.len(), 13);
    for (pid, model) in want {
        let c = cfg(pid);
        let mouse = hid_info("mouse", 0x3057, pid, 0x01, 0x02, (8, 0, 0));
        let m = vx::matches(&[&mouse, &c], 0x3057, pid).unwrap_or_else(|| panic!("{pid:04x}"));
        assert_eq!((m.family, m.model, m.wireless, m.variant), (Family::Vaxee, Some(model), Some(false), pid), "{pid:04x}");
        assert_eq!(m.config.path, c.path);
        assert_eq!(bu_mouse::family::find(&[mouse, c.clone()], 0x3057, pid).map(|m| m.family), Some(Family::Vaxee), "{pid:04x}");
    }
}

#[test]
fn the_matcher_claims_nothing_else() {
    // driver knows, list does not: the receivers 0x0005 / 0x1001 / 0x1002 / 0x2001 / 0x2002 and the ids between; a wrong id
    for pid in [0x0005, 0x1001, 0x1002, 0x2001, 0x2002, 0x1000, 0x1016, 0x0001] {
        assert!(vx::matches(&[&cfg(pid)], 0x3057, pid).is_none(), "{pid:04x}");
    }
    assert!(vx::matches(&[&cfg(0x1003)], 0x3710, 0x1003).is_none(), "another vendor");
    // OpenMouse's hasControlCollection: usage page 0xff05, usage 1, a feature report on id 0x0e
    let mut wrong_page = cfg(0x1003);
    wrong_page.usage_page = 0xff00;
    assert!(vx::matches(&[&wrong_page], 0x3057, 0x1003).is_none());
    let mut wrong_usage = cfg(0x1003);
    wrong_usage.usage = 2;
    assert!(vx::matches(&[&wrong_usage], 0x3057, 0x1003).is_none());
    let mut no_feature = cfg(0x1003);
    no_feature.feature_len = 0;
    assert!(vx::matches(&[&no_feature], 0x3057, 0x1003).is_none());
    assert!(vx::matches(&[&hid_info("m", 0x3057, 0x1003, 0x01, 0x02, (8, 0, 0))], 0x3057, 0x1003).is_none(), "the mouse collection is no control channel");
    // no overlap with the other new tables
    for pid in [0x1003u16, 0x1009, 0x1015] {
        let c = cfg(pid);
        assert!(bu_mouse::razer::matches(&[&c], 0x3057, pid).is_none() && bu_mouse::steelseries::matches(&[&c], 0x3057, pid).is_none());
        assert!(bu_mouse::wlmouse::matches(&[&c], 0x3057, pid).is_none() && bu_mouse::corsair::matches(&[&c], 0x3057, pid).is_none());
    }
}

#[test]
fn caps_are_what_openmouse_offers() {
    let c = vx::caps();
    let d = c.dpi.clone().unwrap();
    // getDpiOptions: 100 + 50 * i for 519 values = 100 .. 26000; setDpi: 100 - 26000 in steps of 50
    assert_eq!((d.min(), d.max()), (100, 26000));
    assert!(d.allows(100) && d.allows(26000) && d.allows(1650) && !d.allows(1625) && !d.allows(50) && !d.allows(26050));
    // getSupportedPollingRates: [500, 1000] for every product id of the list (4000 only for the receivers 0x1002 / 0x2001 / 0x2002)
    assert_eq!(c.polling, vec![500, 1000]);
    assert!(c.lift_off.is_empty(), "OpenMouse's LOD is Low / High with no millimetre figure");
    assert!(c.battery);
}

// ---------------------------------------------------------------- the driver against a scripted mouse

#[derive(Default)]
struct Mouse {
    pid: u16,
    awake: bool,
    stage: u8,
    /// DPI of stages 1 - 4
    dpi: [u32; 4],
    poll: u8,
    steps: u8,
    charging: u8,
    /// every write seen (the 64-byte report)
    writes: Vec<Vec<u8>>,
    /// DPI writes are accepted but not kept
    ignore_dpi: bool,
    /// answer to a command with the wrong command byte
    wrong_echo: Option<u8>,
}

impl Mouse {
    fn new() -> Self {
        Mouse { pid: 0x1004, awake: true, stage: 2, dpi: [400, 800, 1600, 3200], poll: 2, steps: 15, charging: 0, ..Default::default() }
    }
    fn reply(&mut self, req: &[u8]) -> Vec<u8> {
        assert_eq!((req.len(), req[0], req[1]), (64, 0x0e, 0xa5));
        if !self.awake {
            return vec![0u8; 64];
        }
        let (cmd, kind, v) = (req[2], req[3], &req[5..]);
        let echo = self.wrong_echo.unwrap_or(cmd);
        if kind == 2 {
            self.writes.push(req.to_vec());
            match cmd {
                0x04 if !self.ignore_dpi => self.dpi[v[0] as usize - 1] = v[1] as u32 | (v[2] as u32) << 8,
                0x07 => self.poll = v[0],
                _ => {}
            }
        }
        let d = match cmd {
            0x0a => vec![self.pid as u8, (self.pid >> 8) as u8],
            0x02 => vec![self.stage],
            0x04 => {
                let st = v[0];
                let x = self.dpi[st as usize - 1];
                vec![st, x as u8, (x >> 8) as u8]
            }
            0x07 => vec![self.poll],
            0x0b => vec![self.steps],
            0x10 => vec![self.charging],
            other => panic!("a command OpenMouse never sends here: {other:#x}"),
        };
        answer(echo, &d)
    }
}

fn port(info: HidInfo, mouse: &Rc<RefCell<Mouse>>) -> ScriptPort {
    let m = mouse.clone();
    ScriptPort::new(info, move |out, how| match how {
        HidTransfer::Feature { reply_id: 0x0e, reply_len: 64, wait_ms: 100 } => Ok(m.borrow_mut().reply(out)),
        other => panic!("VAXEE sends feature reports only, not {other:?}"),
    })
}

fn setup(pid: u16, mouse: &Rc<RefCell<Mouse>>) -> (bu_mouse::family::Matched, ScriptPort) {
    let info = cfg(pid);
    let m = vx::matches(&[&info], 0x3057, pid).unwrap();
    (m, port(info, mouse))
}

#[test]
fn reading_asks_only_what_openmouse_asks() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    let (m, mut p) = setup(0x1004, &mouse);
    let r = vx::Vaxee.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(p.sent.iter().all(|s| vx::read_request_allowed(&s.0)), "read requests only");
    // mouse pid, DPI stage, that stage's DPI, polling, battery, charging - OpenMouse's order (firmware and LOD are not asked)
    let want = vec![rq(0x0a, 1, 2, &[]), rq(0x02, 1, 1, &[]), rq(0x04, 1, 3, &[2]), rq(0x07, 1, 1, &[]), rq(0x0b, 1, 1, &[]), rq(0x10, 1, 1, &[])];
    assert_eq!(p.sent.iter().map(|s| s.0.clone()).collect::<Vec<_>>(), want);
    assert!(p.sent.iter().all(|s| s.1 == HidTransfer::Feature { reply_id: 0x0e, reply_len: 64, wait_ms: 100 }));
    // stage 2 = 800 DPI; polling code 2 = 1000 Hz; 15 steps = 75 %; the mouse names itself by the pid it reports
    assert_eq!((r.online, r.dpi, r.polling_hz, r.battery_percent, r.charging, r.wireless), (true, Some(800), Some(1000), Some(75), Some(false), Some(false)));
    assert_eq!(r.model.as_deref(), Some("VAXEE NP-01S Wireless"));
    assert_eq!(r.lift_off, None);
    assert_eq!((r.caps.polling.clone(), r.caps.battery, r.caps.lift_off.len()), (vec![500, 1000], true, 0));
    // charging, 500 Hz, stage 4; battery steps beyond 20 = no reading; an unknown pid has no name
    {
        let mut mm = mouse.borrow_mut();
        mm.charging = 1;
        mm.poll = 1;
        mm.stage = 4;
        mm.steps = 99;
        mm.pid = 0x7777;
    }
    let (m, mut p) = setup(0x1004, &mouse);
    let r = vx::Vaxee.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.dpi, r.polling_hz, r.battery_percent, r.charging, r.model), (Some(3200), Some(500), None, Some(true), None));
    assert_eq!(p.sent[2].0, rq(0x04, 1, 3, &[4]));
}

#[test]
fn a_value_that_cannot_be_read_is_left_out_and_a_silent_mouse_is_offline() {
    // an invalid stage (OpenMouse throws "Invalid VAXEE DPI stage"): no DPI, the rest is still read
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    mouse.borrow_mut().stage = 7;
    let (m, mut p) = setup(0x1003, &mouse);
    let r = vx::Vaxee.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.online, r.dpi, r.polling_hz), (true, None, Some(1000)));
    // polling code 0 is no rate
    mouse.borrow_mut().stage = 1;
    mouse.borrow_mut().poll = 0;
    let (m, mut p) = setup(0x1003, &mouse);
    let r = vx::Vaxee.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.dpi, r.polling_hz), (Some(400), None));
    // a mouse that answers nothing (zeros: not an a5 reply): offline, one request sent, no loop
    mouse.borrow_mut().awake = false;
    let (m, mut p) = setup(0x1003, &mouse);
    let r = vx::Vaxee.read(&m, &mut p, ReadOptions { online_tries: 1 }).unwrap();
    assert_eq!((r.online, r.dpi, r.polling_hz, r.battery_percent, r.model), (false, None, None, None, None));
    assert_eq!(p.sent.len(), 1);
    assert!(r.caps.dpi.is_some());
    // a write to it fails and writes nothing
    assert!(matches!(vx::Vaxee.write(&m, &mut p, Setting::Polling(500)), Err(Error::BadAnswer(_))));
    assert!(mouse.borrow().writes.is_empty());
}

#[test]
fn dpi_is_changed_the_openmouse_way() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    let (m, mut p) = setup(0x1005, &mouse);
    let c = vx::Vaxee.write(&m, &mut p, Setting::Dpi(1650)).unwrap();
    assert_eq!((c.now, c.before), (1650, Some(800)));
    // the stage, [the value for undo], the write on that stage (1650 = 0x0672: 72 06), the read-back
    let sent: Vec<Vec<u8>> = p.sent.iter().map(|s| s.0.clone()).collect();
    assert_eq!(sent, vec![rq(0x02, 1, 1, &[]), rq(0x04, 1, 3, &[2]), rq(0x04, 2, 3, &[2, 0x72, 0x06]), rq(0x04, 1, 3, &[2])]);
    // only the active stage changed
    assert_eq!(mouse.borrow().dpi, [400, 1650, 1600, 3200]);
    // the ends of the range: 100 = 0x0064, 26000 = 0x6590
    wx(&mouse, 100, [0x64, 0x00]);
    wx(&mouse, 26000, [0x90, 0x65]);
    // what the mouse can't store: refused before a byte is sent
    let (m, mut p) = setup(0x1005, &mouse);
    for bad in [0, 50, 99, 125, 26050, 40000] {
        assert!(matches!(vx::Vaxee.write(&m, &mut p, Setting::Dpi(bad)), Err(Error::OutOfRange { .. })), "{bad}");
    }
    assert!(p.sent.is_empty());
}

/// Writes one DPI on a fresh port and checks the write request's value bytes.
fn wx(mouse: &Rc<RefCell<Mouse>>, dpi: u32, lo_hi: [u8; 2]) {
    let (m, mut p) = setup(0x1005, mouse);
    vx::Vaxee.write(&m, &mut p, Setting::Dpi(dpi)).unwrap();
    assert_eq!(p.sent[2].0, rq(0x04, 2, 3, &[2, lo_hi[0], lo_hi[1]]), "{dpi}");
}

#[test]
fn polling_is_changed_the_openmouse_way() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    let (m, mut p) = setup(0x1006, &mouse);
    let c = vx::Vaxee.write(&m, &mut p, Setting::Polling(500)).unwrap();
    assert_eq!((c.now, c.before), (500, Some(1000)));
    let sent: Vec<Vec<u8>> = p.sent.iter().map(|s| s.0.clone()).collect();
    assert_eq!(sent, vec![rq(0x07, 1, 1, &[]), rq(0x07, 2, 1, &[1]), rq(0x07, 1, 1, &[])]);
    assert_eq!(mouse.borrow().poll, 1);
    let (m, mut p) = setup(0x1006, &mouse);
    assert_eq!(vx::Vaxee.write(&m, &mut p, Setting::Polling(1000)).unwrap().before, Some(500));
    assert_eq!(p.sent[1].0, rq(0x07, 2, 1, &[2]));
    // 2000 / 4000 / 8000 need a receiver (OpenMouse's list for these ids is [500, 1000]): refused, nothing sent
    let (m, mut p) = setup(0x1006, &mouse);
    for hz in [250, 2000, 4000, 8000, 0] {
        assert!(matches!(vx::Vaxee.write(&m, &mut p, Setting::Polling(hz)), Err(Error::OutOfRange { .. })), "{hz}");
    }
    assert!(p.sent.is_empty());
}

#[test]
fn a_mouse_that_does_not_keep_the_value_is_an_error() {
    // OpenMouse: "VAXEE DPI write did not persist."
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    mouse.borrow_mut().ignore_dpi = true;
    let (m, mut p) = setup(0x1007, &mouse);
    let e = vx::Vaxee.write(&m, &mut p, Setting::Dpi(1200)).unwrap_err();
    assert!(matches!(e, Error::BadAnswer(_)), "{e:?}");
    assert_eq!(mouse.borrow().dpi, [400, 800, 1600, 3200]);
    // an answer to another command: "Invalid VAXEE reply"
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    mouse.borrow_mut().wrong_echo = Some(0x0b);
    let (m, mut p) = setup(0x1007, &mouse);
    assert!(matches!(vx::Vaxee.write(&m, &mut p, Setting::Polling(500)), Err(Error::BadAnswer(_))));
    assert!(mouse.borrow().writes.is_empty(), "the read before the write already failed");
}

#[test]
fn lift_off_is_not_offered() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    let (m, mut p) = setup(0x1008, &mouse);
    assert!(vx::Vaxee.write(&m, &mut p, Setting::LiftOff(10)).is_err());
    assert!(p.sent.is_empty());
}

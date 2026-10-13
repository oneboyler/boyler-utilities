//! Order 105 check, Corsair (NIGHTSWORD RGB): our requests = the bytes OpenMouse sends for the same setting (encode), and
//! answers shaped the way OpenMouse reads them decode to the same values (decode). The expected bytes are worked out by hand
//! from OpenMouse (@openmouse/protocol 0.40.1: `dist/corsair/index.js`, `dist/drivers/corsair/hid.js`), never from our own
//! encoder.
//!
//! OpenMouse's feature report (id 0) is 64 bytes without the id: `[0]` command (0x0e GET / 0x07 SET) · `[1]` field (0x01
//! identity, 0x13 mouse) · `[2]` sub-command (0x02 current DPI, 0xd0 + n stage slot n) · `[3]` profile 0 (live) · `[4..]`
//! values. A GET answer echoes bytes 0 - 3. Windows puts the report id 0 in front: OpenMouse's byte i = our byte i + 1.

use bu_mouse::corsair as cs;
use bu_mouse::family::{Driver, Family, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::os::{HidInfo, HidTransfer};
use bu_mouse::Error;
use std::cell::RefCell;
use std::rc::Rc;

/// A request as Windows sends it: report id 0, OpenMouse's bytes, zero padded to 65. By hand.
fn rep(bytes: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 65];
    v[1..1 + bytes.len()].copy_from_slice(bytes);
    v
}

fn cfg() -> HidInfo {
    hid_info("cfg", 0x1b1c, 0x1b5c, 0xffc2, 4, (65, 65, 65))
}

// ---------------------------------------------------------------- encode

#[test]
fn gets_are_openmouses_corsair_packets() {
    // corsairEncode.ident = corsairPacket(GET 0x0e, FIELD.ident 0x01, 0)
    assert_eq!(cs::report(&cs::get_ident(), 65), rep(&[0x0e, 0x01, 0x00]));
    // corsairEncode.dpiStage = (0x0e, FIELD.mouse 0x13, MOUSE.dpi 0x02, LIVE_PROFILE 0)
    assert_eq!(cs::report(&cs::get_current(), 65), rep(&[0x0e, 0x13, 0x02, 0x00]));
    // corsairEncode.stage(n) = (0x0e, 0x13, MOUSE.dpiProfile 0xd0 | n, 0): d0 .. d5
    for n in 0..6u8 {
        assert_eq!(cs::report(&cs::get_stage(n), 65), rep(&[0x0e, 0x13, 0xd0 + n, 0x00]), "slot {n}");
    }
    assert_eq!(cs::get_ident().len(), 64);
    assert_eq!(cs::report(&cs::get_ident(), 90).len(), 90, "a longer collection is padded with zeros");
}

#[test]
fn the_dpi_set_is_openmouses_little_endian_packet() {
    // setStageDpi(stage, x, y, rgb) = (SET 0x07, 0x13, 0xd0 | stage, 0, x != y ? 1 : 0, x & ff, x >> 8, y & ff, y >> 8, r, g, b)
    // slot 2, 1600 = 0x0640 on both axes, colour 00 bf ff: 07 13 d2 00 | 00 | 40 06 | 40 06 | 00 bf ff
    assert_eq!(cs::report(&cs::set_stage_dpi(2, 1600, 1600, [0x00, 0xbf, 0xff]), 65), rep(&[0x07, 0x13, 0xd2, 0x00, 0x00, 0x40, 0x06, 0x40, 0x06, 0x00, 0xbf, 0xff]));
    // slot 0 (the Sniper slot, which OpenMouse's setDpi also rewrites when it is selected), 18000 = 0x4650, colour ff aa 00
    assert_eq!(cs::report(&cs::set_stage_dpi(0, 18000, 18000, [0xff, 0xaa, 0x00]), 65), rep(&[0x07, 0x13, 0xd0, 0x00, 0x00, 0x50, 0x46, 0x50, 0x46, 0xff, 0xaa, 0x00]));
    // 100 = 0x0064
    assert_eq!(cs::report(&cs::set_stage_dpi(5, 100, 100, [1, 2, 3]), 65), rep(&[0x07, 0x13, 0xd5, 0x00, 0x00, 0x64, 0x00, 0x64, 0x00, 1, 2, 3]));
    // separate axes would set the flag byte (800 = 0x0320, 1600 = 0x0640): shown here for the codec; the driver never sends it
    assert_eq!(cs::report(&cs::set_stage_dpi(1, 800, 1600, [9, 9, 9]), 65), rep(&[0x07, 0x13, 0xd1, 0x00, 0x01, 0x20, 0x03, 0x40, 0x06, 9, 9, 9]));
}

// ---------------------------------------------------------------- decode

#[test]
fn answers_decode_like_openmouse() {
    // corsairDecode.dpiStage: [4] slot, X big-endian [5..7], Y big-endian [7..9]: slot 2, 1600 = 06 40, 3200 = 0c 80
    let cur = [0x0e, 0x13, 0x02, 0x00, 0x02, 0x06, 0x40, 0x0c, 0x80];
    assert_eq!(cs::decode_current(&cur), Some((2, 1600, 3200)));
    assert_eq!(cs::decode_current(&cur[..8]), None, "shorter than 9 bytes");
    // corsairDecode.stage: X [5..7], Y [7..9] big-endian, colour [9..12]
    let st = [0x0e, 0x13, 0xd2, 0x00, 0x00, 0x03, 0x20, 0x03, 0x20, 0x00, 0xbf, 0xff];
    assert_eq!(cs::decode_stage(&st), Some((800, 800, [0x00, 0xbf, 0xff])));
    assert_eq!(cs::decode_stage(&st[..11]), None, "shorter than 12 bytes");
    // corsairDecode.ident: pollIntervalMs = reply[16]; rate = round(1000 / ms)
    let mut id = [0u8; 20];
    id[..3].copy_from_slice(&[0x0e, 0x01, 0x00]);
    for (ms, hz) in [(1u8, 1000u32), (2, 500), (4, 250), (8, 125), (3, 333)] {
        id[16] = ms;
        assert_eq!(cs::decode_polling_hz(&id), Some(hz), "{ms} ms");
    }
    id[16] = 0;
    assert_eq!(cs::decode_polling_hz(&id), None, "interval 0 = no rate");
    assert_eq!(cs::decode_polling_hz(&id[..16]), None, "shorter than 17 bytes");
    // corsairIsEcho: bytes 0 - 3 must repeat
    let req = [0x0e, 0x13, 0x02, 0x00];
    assert!(cs::is_echo(&req, &cur));
    assert!(!cs::is_echo(&req, &[0x0e, 0x13, 0x03, 0x00, 1]), "another sub-command");
    assert!(!cs::is_echo(&req, &[0x0e, 0x13, 0x02]), "shorter than 4");
    assert!(!cs::is_echo(&req, &[0x0e, 0x13, 0x02, 0x01]), "another profile");
}

// ---------------------------------------------------------------- the allow-list

#[test]
fn only_read_requests_pass_the_allow_list() {
    for p in [cs::get_ident(), cs::get_current(), cs::get_stage(0), cs::get_stage(3), cs::get_stage(5)] {
        assert!(cs::read_request_allowed(&cs::report(&p, 65)), "{:02x?}", &p[..4]);
        assert!(cs::read_request_allowed(&cs::report(&p, 90)), "on a longer report");
    }
    // every SET: the DPI of every slot, both DPI values; the other SETs the client has (stage select, mask, lift, snap, poll)
    for slot in 0..6u8 {
        assert!(!cs::read_request_allowed(&cs::report(&cs::set_stage_dpi(slot, 800, 800, [0, 0, 0]), 65)), "slot {slot}");
    }
    for bytes in [
        &[0x07u8, 0x13, 0x02, 0x00, 0x02][..],           // setStage
        &[0x07, 0x13, 0x05, 0x00, 0x3f],                 // setDpiMask
        &[0x07, 0x13, 0x03, 0x00, 0x03],                 // setLift
        &[0x07, 0x13, 0x04, 0x00, 0x01, 0x05],           // setSnap
        &[0x07, 0x0a, 0x00, 0x00, 0x02],                 // setPollMs
    ] {
        assert!(!cs::read_request_allowed(&rep(bytes)), "{bytes:02x?}");
    }
    // GETs OpenMouse has but this file never sends (mask, lift), a slot beyond 5, a GET with a value, another id, junk
    for bytes in [&[0x0eu8, 0x13, 0x05, 0x00][..], &[0x0e, 0x13, 0x03, 0x00], &[0x0e, 0x13, 0xd6, 0x00], &[0x0e, 0x13, 0x02, 0x01], &[0x0e, 0x13, 0x02, 0x00, 0x05], &[0x0e, 0x0a, 0x00]] {
        assert!(!cs::read_request_allowed(&rep(bytes)), "{bytes:02x?}");
    }
    let mut id1 = rep(&[0x0e, 0x01, 0x00]);
    id1[0] = 1;
    assert!(!cs::read_request_allowed(&id1));
    let mut junk = cs::report(&cs::get_ident(), 90);
    junk[80] = 1;
    assert!(!cs::read_request_allowed(&junk));
    assert!(!cs::read_request_allowed(&[0x00, 0x0e, 0x01, 0x00]));
    assert!(!cs::read_request_allowed(&[]));
}

// ---------------------------------------------------------------- the device table

#[test]
fn the_table_is_the_nightsword_rgb_only() {
    // supported-devices-data.ts: Corsair NIGHTSWORD RGB, status "supported", pid 0x1b5c - the only supported Corsair
    let mouse = hid_info("mouse", 0x1b1c, 0x1b5c, 0x01, 0x02, (8, 0, 0));
    // MI_00 also carries an 0xffc2 collection, usage 3, that never answers
    let usage3 = hid_info("usage3", 0x1b1c, 0x1b5c, 0xffc2, 3, (65, 0, 0));
    let c = cfg();
    let m = cs::matches(&[&mouse, &usage3, &c], 0x1b1c, 0x1b5c).unwrap();
    assert_eq!((m.family, m.model, m.wireless, m.variant), (Family::Corsair, Some("Corsair NIGHTSWORD RGB"), Some(false), 0x1b5c));
    assert_eq!(m.config.path, "cfg", "the usage-4 collection, not usage 3");
    assert_eq!(bu_mouse::family::find(&[mouse, usage3, c.clone()], 0x1b1c, 0x1b5c).map(|m| m.family), Some(Family::Corsair));
}

#[test]
fn the_matcher_claims_nothing_else() {
    // the other Corsair entries are "driver" / "pr" (not supported): Ironclaw RGB Wireless 0x1b4c, its receivers 0x1b66 /
    // 0x1bdc (Bragi), and any other Corsair id
    for pid in [0x1b4c, 0x1b66, 0x1bdc, 0x1b5d, 0x1b3c, 0x0000] {
        let mut h = cfg();
        h.pid = pid;
        assert!(cs::matches(&[&h], 0x1b1c, pid).is_none(), "{pid:04x}");
    }
    assert!(cs::matches(&[&cfg()], 0x3710, 0x1b5c).is_none(), "another vendor");
    let mut wrong_page = cfg();
    wrong_page.usage_page = 0xff42;
    assert!(cs::matches(&[&wrong_page], 0x1b1c, 0x1b5c).is_none());
    let mut usage3 = cfg();
    usage3.usage = 3;
    assert!(cs::matches(&[&usage3], 0x1b1c, 0x1b5c).is_none(), "usage 3 never answers");
    let mut no_feature = cfg();
    no_feature.feature_len = 0;
    assert!(cs::matches(&[&no_feature], 0x1b1c, 0x1b5c).is_none());
    assert!(cs::matches(&[&hid_info("m", 0x1b1c, 0x1b5c, 0x01, 0x02, (8, 0, 0))], 0x1b1c, 0x1b5c).is_none());
    // no overlap with the other new tables
    let c = cfg();
    assert!(bu_mouse::razer::matches(&[&c], 0x1b1c, 0x1b5c).is_none() && bu_mouse::steelseries::matches(&[&c], 0x1b1c, 0x1b5c).is_none());
    assert!(bu_mouse::wlmouse::matches(&[&c], 0x1b1c, 0x1b5c).is_none() && bu_mouse::vaxee::matches(&[&c], 0x1b1c, 0x1b5c).is_none());
}

#[test]
fn caps_are_what_openmouse_does() {
    let c = cs::caps();
    let d = c.dpi.unwrap();
    // the option list: 100 + 50 * i up to the product's dpiMax 18000; setDpi: 100 - 18000
    assert_eq!((d.min(), d.max()), (100, 18000));
    assert!(d.allows(100) && d.allows(18000) && d.allows(1650) && !d.allows(1625) && !d.allows(50) && !d.allows(18050));
    // polling is read-only in OpenMouse's client (pollingReadOnly), lift-off has no mm figure, the mouse is wired
    assert!(c.polling.is_empty() && c.lift_off.is_empty() && !c.battery);
}

// ---------------------------------------------------------------- the driver against a scripted mouse

#[derive(Default)]
struct Mouse {
    awake: bool,
    /// the selected slot
    slot: u8,
    /// per slot: X, Y, colour
    stages: Vec<(u16, u16, [u8; 3])>,
    interval_ms: u8,
    /// every SET seen (the 64-byte packet)
    sets: Vec<Vec<u8>>,
    /// SETs are accepted but ignored
    ignore_sets: bool,
    /// a SET clears the colour (what OpenMouse says happens when a write omits it)
    clear_colour: bool,
    /// the first n GET answers are a stale buffer (another command's answer)
    /// Order 110: angle snapping (MOUSE_SNAP 0x04: answer byte 4)
    snap: u8,
    stale: u32,
}

impl Mouse {
    fn new() -> Self {
        let colours = [[0xff, 0xff, 0x00], [0x00, 0xbf, 0xff], [0xff, 0x00, 0x00], [0x00, 0xff, 0x00], [0xff, 0x00, 0xff], [0x10, 0x20, 0x30]];
        Mouse {
            awake: true,
            slot: 2,
            stages: (0..6).map(|i| (400 * (i as u16 + 1), 400 * (i as u16 + 1), colours[i])).collect(),
            interval_ms: 1,
            ..Default::default()
        }
    }
    fn get(&mut self, p: &[u8]) -> Vec<u8> {
        let mut r = vec![0u8; 65];
        if !self.awake {
            return r;
        }
        r[1..5].copy_from_slice(&p[..4]);
        if self.stale > 0 {
            self.stale -= 1;
            r[1] = 0x0e;
            r[2] = 0x13;
            r[3] = 0x02;
            r[4] = 0x7f; // a different echo
            return r;
        }
        match (p[1], p[2]) {
            (0x01, _) => r[17] = self.interval_ms,
            (0x13, 0x04) => r[5] = self.snap,
            (0x13, 0x02) => {
                let (x, y, _) = self.stages[self.slot as usize];
                r[5] = self.slot;
                r[6..8].copy_from_slice(&x.to_be_bytes());
                r[8..10].copy_from_slice(&y.to_be_bytes());
            }
            (0x13, sub) if (0xd0..0xd6).contains(&sub) => {
                let (x, y, c) = self.stages[(sub - 0xd0) as usize];
                r[6..8].copy_from_slice(&x.to_be_bytes());
                r[8..10].copy_from_slice(&y.to_be_bytes());
                r[10..13].copy_from_slice(&c);
            }
            other => panic!("a GET OpenMouse never sends here: {other:?}"),
        }
        r
    }
    fn set(&mut self, p: &[u8]) {
        self.sets.push(p.to_vec());
        assert_eq!((p[0], p[1]), (0x07, 0x13), "the only SETs are the stage write and setSnap");
        if self.ignore_sets {
            return;
        }
        if p[2] == 0x04 {
            assert_eq!((p[3], p[5]), (0x00, 0x05), "setSnap: [07 13 04 00 on 05]");
            self.snap = p[4];
            return;
        }
        let slot = (p[2] - 0xd0) as usize;
        let colour = if self.clear_colour { [0, 0, 0] } else { [p[9], p[10], p[11]] };
        self.stages[slot] = (p[5] as u16 | (p[6] as u16) << 8, p[7] as u16 | (p[8] as u16) << 8, colour);
    }
}

fn port(mouse: &Rc<RefCell<Mouse>>) -> ScriptPort {
    let m = mouse.clone();
    ScriptPort::new(cfg(), move |out, how| match how {
        HidTransfer::Feature { reply_id: 0, reply_len: 65, wait_ms: 20 } => {
            assert_eq!((out.len(), out[0]), (65, 0));
            assert_eq!(out[1], 0x0e, "a GET");
            Ok(m.borrow_mut().get(&out[1..]))
        }
        HidTransfer::FeatureOut => {
            assert_eq!((out.len(), out[0]), (65, 0));
            m.borrow_mut().set(&out[1..]);
            Ok(Vec::new())
        }
        other => panic!("Corsair sends feature reports only, not {other:?}"),
    })
}

fn matched() -> bu_mouse::family::Matched {
    cs::matches(&[&cfg()], 0x1b1c, 0x1b5c).unwrap()
}

#[test]
fn reading_asks_only_what_openmouse_asks() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    let mut p = port(&mouse);
    let r = cs::Corsair.read(&matched(), &mut p, ReadOptions::default()).unwrap();
    assert!(p.sent.iter().all(|s| cs::read_request_allowed(&s.0)), "read requests only");
    // the identity, then the current DPI
    // (Order 110: then angle snapping, corsairEncode.snap 0e 13 04 00)
    assert_eq!(p.sent.iter().map(|s| s.0.clone()).collect::<Vec<_>>(), vec![rep(&[0x0e, 0x01, 0x00]), rep(&[0x0e, 0x13, 0x02, 0x00]), rep(&[0x0e, 0x13, 0x04, 0x00])]);
    assert!(p.sent.iter().all(|s| s.1 == HidTransfer::Feature { reply_id: 0, reply_len: 65, wait_ms: 20 }));
    // slot 2 = 1200 DPI; interval 1 ms = 1000 Hz
    assert_eq!((r.online, r.dpi, r.polling_hz, r.battery_percent, r.lift_off, r.wireless), (true, Some(1200), Some(1000), None, None, Some(false)));
    assert!(r.caps.polling.is_empty() && r.caps.dpi.is_some());
    // 4 ms = 250 Hz; another selected slot
    {
        let mut mm = mouse.borrow_mut();
        mm.interval_ms = 4;
        mm.slot = 5;
    }
    let mut p = port(&mouse);
    let r = cs::Corsair.read(&matched(), &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.dpi, r.polling_hz), (Some(2400), Some(250)));
}

#[test]
fn a_stale_answer_is_asked_again_and_a_silent_mouse_is_offline() {
    // the first answer is a stale buffer (the echo is wrong): the GET is sent again, up to 3 tries
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    mouse.borrow_mut().stale = 1;
    let mut p = port(&mouse);
    let r = cs::Corsair.read(&matched(), &mut p, ReadOptions::default()).unwrap();
    assert!(r.online && r.dpi == Some(1200));
    assert_eq!(p.sent.len(), 4, "ident twice, current once, snap once");
    assert_eq!(p.sent[0].0, p.sent[1].0);
    // three stale answers in a row: no answer
    mouse.borrow_mut().stale = 3;
    let mut p = port(&mouse);
    let r = cs::Corsair.read(&matched(), &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.online, r.dpi, r.polling_hz), (false, None, None));
    assert_eq!(p.sent.len(), 3);
    // the DPI read failing after the identity answered: the mouse is there, no DPI shown
    mouse.borrow_mut().stale = 0;
    let mm = mouse.clone();
    let mut n = 0;
    let mut p = ScriptPort::new(cfg(), move |out, _| {
        n += 1;
        if n > 1 {
            return Ok(vec![0u8; 65]);
        }
        Ok(mm.borrow_mut().get(&out[1..]))
    });
    let r = cs::Corsair.read(&matched(), &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.online, r.polling_hz, r.dpi), (true, Some(1000), None));
    // an asleep / unplugged mouse
    mouse.borrow_mut().awake = false;
    let mut p = port(&mouse);
    let r = cs::Corsair.read(&matched(), &mut p, ReadOptions { online_tries: 1 }).unwrap();
    assert!(!r.online && r.caps.dpi.is_some());
    assert!(matches!(cs::Corsair.write(&matched(), &mut p, Setting::Dpi(800)), Err(Error::BadAnswer(_))));
    assert!(mouse.borrow().sets.is_empty());
}

#[test]
fn dpi_is_changed_the_openmouse_way() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    let mut p = port(&mouse);
    let c = cs::Corsair.write(&matched(), &mut p, Setting::Dpi(1600)).unwrap();
    assert_eq!((c.now, c.before), (1600, Some(1200)));
    // the selected slot (GET), the slot's contents for the colour (GET), the SET with that colour, the slot again (GET).
    // slot 2's colour is ff 00 00; 1600 = 0x0640
    let sent: Vec<(Vec<u8>, HidTransfer)> = p.sent.clone();
    let get = HidTransfer::Feature { reply_id: 0, reply_len: 65, wait_ms: 20 };
    assert_eq!(
        sent,
        vec![
            (rep(&[0x0e, 0x13, 0x02, 0x00]), get.clone()),
            (rep(&[0x0e, 0x13, 0xd2, 0x00]), get.clone()),
            (rep(&[0x07, 0x13, 0xd2, 0x00, 0x00, 0x40, 0x06, 0x40, 0x06, 0xff, 0x00, 0x00]), HidTransfer::FeatureOut),
            (rep(&[0x0e, 0x13, 0xd2, 0x00]), get),
        ]
    );
    // only that slot changed, its colour is kept, the selection is unchanged
    let m = mouse.borrow();
    assert_eq!(m.stages[2], (1600, 1600, [0xff, 0x00, 0x00]));
    assert_eq!((m.stages[1].0, m.stages[3].0, m.slot), (800, 1600, 2));
    drop(m);
    // the ends: 100 and 18000; the Sniper slot 0 when it is the selected one
    mouse.borrow_mut().slot = 0;
    let mut p = port(&mouse);
    cs::Corsair.write(&matched(), &mut p, Setting::Dpi(18000)).unwrap();
    assert_eq!(p.sent[2].0, rep(&[0x07, 0x13, 0xd0, 0x00, 0x00, 0x50, 0x46, 0x50, 0x46, 0xff, 0xff, 0x00]));
    let mut p = port(&mouse);
    cs::Corsair.write(&matched(), &mut p, Setting::Dpi(100)).unwrap();
    assert_eq!(p.sent[2].0, rep(&[0x07, 0x13, 0xd0, 0x00, 0x00, 0x64, 0x00, 0x64, 0x00, 0xff, 0xff, 0x00]));
    // values OpenMouse's list does not offer: refused before a byte is sent
    let mut p = port(&mouse);
    for bad in [0, 50, 99, 125, 18050, 40000] {
        assert!(matches!(cs::Corsair.write(&matched(), &mut p, Setting::Dpi(bad)), Err(Error::OutOfRange { .. })), "{bad}");
    }
    assert!(p.sent.is_empty());
}

#[test]
fn what_the_mouse_does_not_keep_is_an_error() {
    // OpenMouse: "The mouse kept ... DPI on stage ... instead of ..."
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    mouse.borrow_mut().ignore_sets = true;
    let mut p = port(&mouse);
    assert!(matches!(cs::Corsair.write(&matched(), &mut p, Setting::Dpi(1600)), Err(Error::BadAnswer(_))));
    // a colour that did not survive
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    mouse.borrow_mut().clear_colour = true;
    let mut p = port(&mouse);
    assert!(matches!(cs::Corsair.write(&matched(), &mut p, Setting::Dpi(1600)), Err(Error::BadAnswer(_))));
}

#[test]
fn a_slot_with_separate_axes_and_a_bad_slot_are_held_back() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    mouse.borrow_mut().stages[2].1 = 3200;
    let mut p = port(&mouse);
    assert!(matches!(cs::Corsair.write(&matched(), &mut p, Setting::Dpi(1600)), Err(Error::OutOfRange { .. })));
    assert!(mouse.borrow().sets.is_empty(), "nothing written");
    // a selected slot beyond the six OpenMouse knows (it throws "Corsair DPI stage must be 0-5")
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    let mm = mouse.clone();
    let mut p = ScriptPort::new(cfg(), move |out, _| {
        let mut r = mm.borrow_mut().get(&out[1..]);
        if out[3] == 0x02 {
            r[5] = 9;
        }
        Ok(r)
    });
    assert!(matches!(cs::Corsair.write(&matched(), &mut p, Setting::Dpi(1600)), Err(Error::BadAnswer(_))));
    assert!(mouse.borrow().sets.is_empty());
}

#[test]
fn polling_and_lift_off_are_not_set() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    let mut p = port(&mouse);
    assert!(cs::Corsair.write(&matched(), &mut p, Setting::Polling(500)).is_err());
    assert!(cs::Corsair.write(&matched(), &mut p, Setting::LiftOff(10)).is_err());
    assert!(p.sent.is_empty());
}

// ---------------------------------------------------------------- Order 110: angle snapping

use bu_mouse::family::{Change, Extra};

#[test]
fn order_110_angle_snapping_is_openmouses_snap_get_and_set() {
    // corsairEncode.snap = corsairPacket(GET 0x0e, mouse 0x13, snap 0x04, live profile 0); setSnap adds [on, 0x05]
    assert_eq!(cs::report(&cs::get_snap(), 65), rep(&[0x0e, 0x13, 0x04, 0x00]));
    assert_eq!(cs::report(&cs::set_snap(true), 65), rep(&[0x07, 0x13, 0x04, 0x00, 0x01, 0x05]));
    assert_eq!(cs::report(&cs::set_snap(false), 65), rep(&[0x07, 0x13, 0x04, 0x00, 0x00, 0x05]));
    // corsairDecode.snap: byte 4 !== 0 (OpenMouse's reply has no report id)
    assert_eq!((cs::decode_snap(&[0x0e, 0x13, 0x04, 0x00, 0x02]), cs::decode_snap(&[0x0e, 0x13, 0x04, 0x00, 0x00]), cs::decode_snap(&[0x0e])), (Some(true), Some(false), None));
    // read: the row with the mouse's value
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    mouse.borrow_mut().snap = 1;
    let mut p = port(&mouse);
    let r = cs::Corsair.read(&matched(), &mut p, ReadOptions::default()).unwrap();
    assert_eq!(r.extras.iter().map(|x| (x.extra, x.value)).collect::<Vec<_>>(), vec![(Extra::AngleSnap, Some(1))]);
    // setAngleSnapping(false): the SET (no answer), then the GET confirms
    let mut p = port(&mouse);
    cs::Corsair.change(&matched(), &mut p, Change::Extra(Extra::AngleSnap, 0)).unwrap();
    assert_eq!(p.sent.iter().map(|s| s.0.clone()).collect::<Vec<_>>(), vec![rep(&[0x07, 0x13, 0x04, 0x00, 0x00, 0x05]), rep(&[0x0e, 0x13, 0x04, 0x00])]);
    assert_eq!(p.sent[0].1, HidTransfer::FeatureOut);
    assert_eq!(mouse.borrow().snap, 0);
    assert!(!cs::read_request_allowed(&p.sent[0].0) && cs::read_request_allowed(&p.sent[1].0));
    // a mouse that keeps the old value
    mouse.borrow_mut().ignore_sets = true;
    let mut p = port(&mouse);
    assert!(matches!(cs::Corsair.change(&matched(), &mut p, Change::Extra(Extra::AngleSnap, 1)), Err(Error::BadAnswer(_))));
    // refused before a byte leaves: 0 / 1 only, nothing else (OpenMouse's NIGHTSWORD client has no Motion Sync / ripple /
    // debounce / turbo)
    let mut p = port(&mouse);
    for c in [Change::Extra(Extra::AngleSnap, 2), Change::Extra(Extra::MotionSync, 1), Change::Extra(Extra::RippleControl, 1), Change::Extra(Extra::Debounce, 4), Change::Extra(Extra::Turbo, 1)] {
        assert!(cs::Corsair.change(&matched(), &mut p, c).is_err(), "{c:?}");
    }
    assert!(p.sent.is_empty());
}

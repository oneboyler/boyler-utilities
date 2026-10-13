//! Order 105 check, Orbital (Ghost / Pathfinder V2): our packets = the bytes OpenMouse sends for the same setting (encode),
//! and answers shaped the way OpenMouse reads them decode to the same values (decode). The expected bytes are worked out by
//! hand from OpenMouse (@openmouse/protocol 0.40.1: `dist/orbital/index.js`, `dist/drivers/orbital/hid.js`,
//! `dist/drivers/orbital/host-protocol.js`), never from our own encoder.
//!
//! OpenMouse sends `sendReport(0, 64 bytes)`: byte i of its packet is byte i + 1 on Windows (report id 0 in front). The
//! checksum is `(0xA1 - (sum of bytes 0..62)) & 0xFF`, byte 63.

use bu_mouse::family::{self, Change, Driver, Extra, Family, Kind, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::orbital as o;
use bu_mouse::os::{HidInfo, HidTransfer};
use bu_mouse::Error;
use std::cell::RefCell;
use std::rc::Rc;

/// A 64-byte OpenMouse packet from its non-zero bytes (index, value).
fn pk(bytes: &[(usize, u8)]) -> o::Packet {
    let mut p = [0u8; 64];
    for (i, v) in bytes {
        p[*i] = *v;
    }
    p
}

/// What Windows sends: report id 0 + the packet.
fn wire(p: &o::Packet) -> Vec<u8> {
    let mut v = vec![0u8];
    v.extend_from_slice(p);
    v
}

fn cfg(pid: u16) -> HidInfo {
    hid_info("orbital-cfg", 0x1915, pid, 0xFF0A, 1, (65, 65, 0))
}

// ---------------------------------------------------------------- encode: reads and the save

#[test]
fn read_requests_and_the_save_are_openmouses_bytes() {
    // detectProtocol: [0]=1 [2]=0x81 [3]=1; sum = 1 + 0x81 + 1 = 0x83; crc = 0xA1 - 0x83 = 0x1E
    assert_eq!(o::handshake(), pk(&[(0, 1), (2, 0x81), (3, 1), (63, 0x1E)]));
    // readDmsV2Settings sensor: [0]=4 [2]=0x81 [3]=1; sum = 4 + 0x81 + 1 = 0x86; crc = 0xA1 - 0x86 = 0x1B
    assert_eq!(o::read_sensor(), pk(&[(0, 4), (2, 0x81), (3, 1), (63, 0x1B)]));
    // system: [0]=4 [2]=0x83 [3]=3; sum = 4 + 0x83 + 3 = 0x8A; crc = 0xA1 - 0x8A = 0x17
    assert_eq!(o::read_system(), pk(&[(0, 4), (2, 0x83), (3, 3), (63, 0x17)]));
    // saveSettings: [0]=10 [2]=0x81 [3]=1; sum = 10 + 0x81 + 1 = 0x8C; crc = 0xA1 - 0x8C = 0x15
    assert_eq!(o::save(), pk(&[(0, 10), (2, 0x81), (3, 1), (63, 0x15)]));
}

#[test]
fn the_checksum_wraps_like_openmouses() {
    // a sum over 255 wraps first: 0xFF + 0xFF + 0x10 = 0x20E → & 0xFF = 0x0E; crc = 0xA1 - 0x0E = 0x93
    let mut p = pk(&[(0, 0xFF), (1, 0xFF), (2, 0x10)]);
    o::finish(&mut p);
    assert_eq!(p[63], 0x93);
    // a sum above 0xA1: 0xD0 → 0xA1 - 0xD0 = -0x2F → & 0xFF = 0xD1
    let mut p = pk(&[(0, 0xD0)]);
    o::finish(&mut p);
    assert_eq!(p[63], 0xD1);
}

// ---------------------------------------------------------------- a Ghost's two settings blocks, by hand

/// The sensor answer as OpenMouse reads it (its bytes; payload = sensorData = bytes from 4): angle snapping off, ripple off,
/// motion sync on, overclock off, high performance on, lift-down off, glass off, lift-off level 1, angle 0, active stage 1,
/// stage mask 0b11111, stages 400/800/1600/3200/4800 (X = Y, little-endian from sensorData[11]), colours from sensorData[39].
fn sensor_answer() -> Vec<u8> {
    let mut v = vec![0u8; 64];
    v[0] = 4;
    v[2] = 0x81;
    v[3] = 1;
    let data: [(usize, u8); 14] = [(2, 1), (4, 1), (7, 1), (9, 1), (10, 0x1F), (11, 0x90), (12, 0x01), (13, 0x90), (14, 0x01), (15, 0x20), (16, 0x03), (17, 0x20), (18, 0x03), (19, 0x40)];
    for (i, b) in data {
        v[4 + i] = b;
    }
    // 1600 = 0x0640 (X, Y), 3200 = 0x0C80, 4800 = 0x12C0
    for (i, b) in [(20, 0x06), (21, 0x40), (22, 0x06), (23, 0x80), (24, 0x0C), (25, 0x80), (26, 0x0C), (27, 0xC0), (28, 0x12), (29, 0xC0), (30, 0x12)] {
        v[4 + i] = b;
    }
    // colours: [255,0,0] [0,255,0] [0,0,255] [255,255,255] [1,2,3]
    for (i, b) in [(39, 255), (43, 255), (47, 255), (48, 255), (49, 255), (50, 255), (51, 1), (52, 2), (53, 3)] {
        v[4 + i] = b;
    }
    v[63] = 0x77; // the answer's own checksum byte: not part of the rewrite
    v
}

/// The system answer: sleep 300 s (2C 01) twice, raw rate 3 (= index 2 = 1000 Hz), debounce 4, quick response on, button and
/// move wake-up on, wheel wake-up off, wheel reverse off, pass-through bytes 0.
fn system_answer() -> Vec<u8> {
    let mut v = vec![0u8; 64];
    v[0] = 4;
    v[2] = 0x83;
    v[3] = 3;
    for (i, b) in [(0, 0x2C), (1, 0x01), (2, 0x2C), (3, 0x01), (4, 3), (5, 4), (6, 1), (7, 1), (8, 1)] {
        v[4 + i] = b;
    }
    v[63] = 0x66;
    v
}

#[test]
fn settings_answers_decode_like_openmouse() {
    let s = o::decode_settings(&sensor_answer(), &system_answer()).unwrap();
    assert_eq!(s.stages, [(400, 400), (800, 800), (1600, 1600), (3200, 3200), (4800, 4800)]);
    assert_eq!((s.active_stage, s.enabled_stages, s.dpi()), (1, 5, 800));
    assert_eq!((s.rate_index, s.polling_hz()), (2, Some(1000)), "raw 3 → index 2 (raw > 1 → raw − 1) → 1000 Hz");
    assert_eq!((s.sleep, s.ble_sleep, s.debounce), (300, 300, 4));
    assert_eq!((s.angle_snapping, s.motion_sync, s.max_speed, s.full_speed, s.lift_off), (false, true, true, false, 1));
    assert_eq!(s.colors, [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255], [1, 2, 3]]);
    assert!(s.quick_response && s.button_wakeup && s.move_wakeup && !s.wheel_wakeup && !s.wheel_reverse);
    // raw rate 0 and raw rate 1 both read as index 0 (125 Hz); raw 6 → index 5 → 8000 Hz; raw 7 → index 6: no such rate
    for (raw, idx, hz) in [(0u8, 0usize, Some(125)), (1, 0, Some(125)), (2, 1, Some(500)), (6, 5, Some(8000)), (7, 6, None)] {
        let mut sys = system_answer();
        sys[8] = raw;
        let s = o::decode_settings(&sensor_answer(), &sys).unwrap();
        assert_eq!((s.rate_index, s.polling_hz()), (idx, hz), "raw {raw}");
    }
    // the stage mask counts set bits (0b00111 → 3 stages), and zero means 5; the active stage is clamped to 0..4
    let mut sen = sensor_answer();
    sen[14] = 0b00111;
    sen[13] = 9;
    let s = o::decode_settings(&sen, &system_answer()).unwrap();
    assert_eq!((s.enabled_stages, s.active_stage), (3, 4));
    sen[14] = 0;
    assert_eq!(o::decode_settings(&sen, &system_answer()).unwrap().enabled_stages, 5);
    // signedByte: 5 → 5, 30 → 30, 31 → −225, 251 → −5
    for (raw, want) in [(5u8, 5), (30, 30), (31, -225), (251, -5)] {
        let mut sen = sensor_answer();
        sen[12] = raw;
        assert_eq!(o::decode_settings(&sen, &system_answer()).unwrap().angle, want);
    }
    assert!(o::decode_settings(&sensor_answer()[..40], &system_answer()).is_none(), "an answer shorter than 64 bytes");
}

// ---------------------------------------------------------------- encode: the two write packets

#[test]
fn the_dpi_change_is_openmouses_sensor_packet() {
    let cur = o::decode_settings(&sensor_answer(), &system_answer()).unwrap();
    let packet = o::sensor_write(&o::merge(&cur, o::Change::Dpi(1600)), &cur);
    // setDpi(1600) → stages[active 1] = [1600, 1600]; writeDmsV2Settings, sensor packet:
    // [0]=4 [2]=0xBC [3]=2 · [4..11] = angle snap 0, ripple 0, motion sync 1, overclock 0, high performance 1, lift-down 0,
    // glass 0 · [11] lift-off 1 · [12] angle 0 · [13] active stage 1 · [14] mask (1 << 5) − 1 = 0x1F ·
    // [15..35] 400 = 90 01, 1600 = 40 06, 1600 = 40 06, 3200 = 80 0C, 4800 = C0 12, each as X then Y · [43..58] the colours
    // checksum: 4 + 0xBC + 2 = 194; flags/lift-off/angle/active/mask 2 + 1 + 0 + 1 + 31 = 35 → 229;
    // stages 2·(0x90+1) = 290, 2·(0x40+6) = 140, 140, 2·(0x80+0x0C) = 280, 2·(0xC0+0x12) = 420 → 1270;
    // colours 255·3 + 255·3 + 6 = 1536 → total 3035 = 11·256 + 219 → sum & 0xFF = 0xDB; crc = 0xA1 − 0xDB = −58 → 0xC6
    let want = pk(&[
        (0, 4), (2, 0xBC), (3, 2), (6, 1), (8, 1), (11, 1), (13, 1), (14, 0x1F),
        (15, 0x90), (16, 0x01), (17, 0x90), (18, 0x01),
        (19, 0x40), (20, 0x06), (21, 0x40), (22, 0x06),
        (23, 0x40), (24, 0x06), (25, 0x40), (26, 0x06),
        (27, 0x80), (28, 0x0C), (29, 0x80), (30, 0x0C),
        (31, 0xC0), (32, 0x12), (33, 0xC0), (34, 0x12),
        (43, 255), (47, 255), (51, 255), (52, 255), (53, 255), (54, 255), (55, 1), (56, 2), (57, 3),
        (63, 0xC6),
    ]);
    assert_eq!(packet, want);
    // what the mouse returned, rewritten with no change, is the same bytes (payload 4..63): the rewrite loses nothing here
    let same = o::sensor_write(&o::merge(&cur, o::Change::None), &cur);
    assert_eq!(same[4..63], sensor_answer()[4..63]);
    assert_eq!(o::rewrite_is_lossless(&cur, &sensor_answer(), &system_answer()), (true, true));
}

#[test]
fn the_polling_change_is_openmouses_system_packet() {
    let cur = o::decode_settings(&sensor_answer(), &system_answer()).unwrap();
    // setPollingRate(2000) → reportRateIndex 3 → encodeReportRate = 4. System packet:
    // [0]=4 [2]=0x90 [3]=4 · [4..6] sleep 300 = 2C 01 · [6..8] the same · [8] rate · [9] debounce 4 · [10] quick 1 ·
    // [11] button wake 1 · [12] move wake 1 · [13] wheel wake 0 · [14] wheel reverse 0 · [15..20] 0
    // checksum: 4 + 0x90 + 4 = 152; sleep 2·(0x2C + 1) = 90 → 242; rate 4 + debounce 4 + 1 + 1 + 1 = 11 → 253;
    // crc = 0xA1 − 0xFD = −92 → 0xA4
    let packet = o::system_write(&o::merge(&cur, o::Change::Rate(3)), &cur);
    let want = pk(&[(0, 4), (2, 0x90), (3, 4), (4, 0x2C), (5, 1), (6, 0x2C), (7, 1), (8, 4), (9, 4), (10, 1), (11, 1), (12, 1), (63, 0xA4)]);
    assert_eq!(packet, want);
    // index 0 (125 Hz) is raw 0, not 1: encodeReportRate(0) = 0; 8000 Hz = index 5 = raw 6
    assert_eq!(o::system_write(&o::merge(&cur, o::Change::Rate(0)), &cur)[8], 0);
    assert_eq!(o::system_write(&o::merge(&cur, o::Change::Rate(5)), &cur)[8], 6);
    // the unchanged rewrite of the system block (1000 Hz = raw 3): checksum 152 + 90 + 3 + 4 + 3 = 252 → 0xA1 − 0xFC = 0xA5
    let same = o::system_write(&o::merge(&cur, o::Change::None), &cur);
    assert_eq!((same[8], same[63]), (3, 0xA5));
    assert_eq!(same[4..63], system_answer()[4..63]);
}

#[test]
fn merge_normalises_the_way_openmouse_does() {
    // normalizeDpi: Math.round(v / 50) * 50 clamped to 50..30000 (a stored 0 becomes 50, a stored 65000 becomes 30000)
    for (v, want) in [(0u32, 50u16), (24, 50), (25, 50), (74, 50), (75, 100), (1600, 1600), (30010, 30000), (65000, 30000)] {
        assert_eq!(o::normalize_dpi(v), want, "{v}");
    }
    let mut cur = o::decode_settings(&sensor_answer(), &system_answer()).unwrap();
    cur.sleep = 0;
    cur.debounce = 99;
    cur.angle = -225;
    cur.lift_off = 7;
    cur.rate_index = 200;
    cur.enabled_stages = 3;
    cur.active_stage = 4;
    let n = o::merge(&cur, o::Change::None);
    // sleep clamped to 1.., debounce 0..30, angle −30..30, lift-off 0..2, rate index 0..5, active ≤ enabled − 1
    assert_eq!((n.sleep, n.debounce, n.angle, n.lift_off, n.rate_index, n.active_stage), (1, 30, -30, 2, 5, 2));
    // the rewrite of such a block is NOT the stored block → the driver refuses it
    let mut sen = sensor_answer();
    sen[14] = 0b10101;
    let odd = o::decode_settings(&sen, &system_answer()).unwrap();
    assert_eq!(odd.enabled_stages, 3, "three bits set");
    assert!(!o::rewrite_is_lossless(&odd, &sen, &system_answer()).0, "mask 0b10101 would be rewritten as 0b00111");
}

// ---------------------------------------------------------------- the allow-list

#[test]
fn only_read_requests_pass_the_allow_list() {
    for r in [o::handshake(), o::read_sensor(), o::read_system()] {
        assert!(o::read_request_allowed(&wire(&r)));
        let mut padded = wire(&r);
        padded.extend_from_slice(&[0, 0, 0]);
        assert!(o::read_request_allowed(&padded), "zero padding is fine");
    }
    // the save, every write packet and the factory reset are refused
    let cur = o::decode_settings(&sensor_answer(), &system_answer()).unwrap();
    let mut writes = vec![o::save(), o::sensor_write(&o::merge(&cur, o::Change::Dpi(800)), &cur), o::system_write(&o::merge(&cur, o::Change::Rate(2)), &cur)];
    // factory reset (v2): [0]=9 [2]=129 [3]=255 [4]=0 - never sent by this app, still refused
    let mut reset = pk(&[(0, 9), (2, 129), (3, 255)]);
    o::finish(&mut reset);
    writes.push(reset);
    for w in writes {
        assert!(!o::read_request_allowed(&wire(&w)), "{:02x?}", &w[..4]);
    }
    // a read with a wrong checksum, another report id, junk behind, too short
    let mut bad = o::read_sensor();
    bad[63] ^= 1;
    assert!(!o::read_request_allowed(&wire(&bad)));
    let mut other = wire(&o::read_sensor());
    other[0] = 1;
    assert!(!o::read_request_allowed(&other));
    let mut junk = wire(&o::read_sensor());
    junk.push(1);
    assert!(!o::read_request_allowed(&junk));
    assert!(!o::read_request_allowed(&wire(&o::read_sensor())[..64]));
    assert!(!o::read_request_allowed(&[]));
}

// ---------------------------------------------------------------- device table

#[test]
fn the_table_is_openmouses_supported_list() {
    // supported-devices-data.ts: Orbital "Ghost / Pathfinder V2" pids [0x80c]
    let c = cfg(0x080C);
    let m = o::matches(&[&c], 0x1915, 0x080C).unwrap();
    assert_eq!((m.family, m.model, m.wireless, m.variant), (Family::Orbital, Some("Orbital Ghost / Pathfinder V2"), Some(false), 0x080C));
    assert_eq!(m.config.path, c.path);
    assert_eq!(family::find(std::slice::from_ref(&c), 0x1915, 0x080C).map(|m| m.family), Some(Family::Orbital));
    // the driver also knows the receiver (0x080b), the V1 mouse (0x0747) and its receiver (0x0746) - the list does not
    for pid in [0x080B, 0x0747, 0x0746, 0x0001] {
        assert!(o::matches(&[&cfg(pid)], 0x1915, pid).is_none(), "{pid:04x}");
    }
    assert!(o::matches(&[&c], 0x1916, 0x080C).is_none(), "another vendor");
    // the wrong collection: the mouse collection, another usage page, another usage, no 64-byte reports
    for (page, usage, lens) in [(0x01, 0x02, (65, 0, 0)), (0xFF0B, 1, (65, 65, 0)), (0xFF0A, 2, (65, 65, 0)), (0xFF0A, 1, (9, 9, 0)), (0xFF0A, 1, (65, 0, 0))] {
        let h = hid_info("x", 0x1915, 0x080C, page, usage, lens);
        assert!(o::matches(&[&h], 0x1915, 0x080C).is_none(), "{page:04x}:{usage}");
    }
    // of two collections, the config one
    let mouse = hid_info("mouse", 0x1915, 0x080C, 0x01, 0x02, (8, 0, 0));
    assert_eq!(o::matches(&[&mouse, &c], 0x1915, 0x080C).map(|m| m.config.path), Some(c.path.clone()));
}

#[test]
fn caps_are_what_openmouse_offers() {
    let c = o::caps();
    let d = c.dpi.unwrap();
    assert_eq!((d.min(), d.max()), (50, 30000));
    assert!(d.allows(50) && d.allows(30000) && d.allows(1650) && !d.allows(1625) && !d.allows(30050));
    assert_eq!(c.polling, vec![125, 500, 1000, 2000, 4000, 8000]);
    assert!(c.lift_off.is_empty(), "three levels without a millimetre figure");
    assert!(!c.battery, "OpenMouse reads the battery of the receiver only");
}

// ---------------------------------------------------------------- the driver against a scripted mouse

struct Fake {
    sensor: Vec<u8>,
    system: Vec<u8>,
    online: bool,
    /// every request body (without the report id), in order
    seen: Vec<Vec<u8>>,
}

fn fake() -> Rc<RefCell<Fake>> {
    Rc::new(RefCell::new(Fake { sensor: sensor_answer(), system: system_answer(), online: true, seen: Vec::new() }))
}

/// A Ghost: reads return the stored blocks, a write stores its payload (bytes 4..63) and answers with its sub command, the
/// save answers on family 10, the handshake on family 1.
fn port(state: &Rc<RefCell<Fake>>) -> ScriptPort {
    let st = state.clone();
    ScriptPort::new(cfg(0x080C), move |out, how| {
        let HidTransfer::OutputThenInput { echo, reply_len, .. } = how else { panic!("Orbital uses output + input reports, not {how:?}") };
        assert_eq!((out.len(), out[0], *reply_len), (65, 0, 65));
        let body = out[1..].to_vec();
        assert_eq!(*echo, Some((1, body[0])), "the answer must start with the request's family byte (id byte first)");
        let mut s = st.borrow_mut();
        s.seen.push(body.clone());
        if !s.online {
            return Err(Error::BadAnswer("no answer in time".into()));
        }
        let mut ans = vec![0u8; 64];
        match (body[0], body[2], body[3]) {
            (1, 0x81, 1) => ans[0] = 1,
            (4, 0x81, 1) => ans = s.sensor.clone(),
            (4, 0x83, 3) => ans = s.system.clone(),
            (4, 0xBC, 2) => {
                s.sensor[4..63].copy_from_slice(&body[4..63]);
                ans[0] = 4;
                ans[3] = 2;
            }
            (4, 0x90, 4) => {
                s.system[4..63].copy_from_slice(&body[4..63]);
                ans[0] = 4;
                ans[3] = 4;
            }
            (10, 0x81, 1) => ans[0] = 10,
            other => panic!("unexpected request {other:02x?}"),
        }
        let mut v = vec![0u8];
        v.extend(ans);
        Ok(v)
    })
}

fn matched() -> bu_mouse::family::Matched {
    o::matches(&[&cfg(0x080C)], 0x1915, 0x080C).unwrap()
}

fn kinds(st: &Rc<RefCell<Fake>>) -> Vec<(u8, u8)> {
    st.borrow().seen.iter().map(|b| (b[0], b[2])).collect()
}

#[test]
fn reading_asks_only_what_openmouse_asks() {
    let st = fake();
    let mut p = port(&st);
    let r = o::Orbital.read(&matched(), &mut p, ReadOptions::default()).unwrap();
    assert!(p.sent.iter().all(|s| o::read_request_allowed(&s.0)), "read requests only");
    assert_eq!(kinds(&st), vec![(1, 0x81), (4, 0x81), (4, 0x83)]);
    assert_eq!((r.online, r.dpi, r.polling_hz, r.battery_percent, r.charging, r.lift_off, r.wireless), (true, Some(800), Some(1000), None, None, None, Some(false)));
    assert_eq!(r.caps, o::caps());
}

#[test]
fn a_mouse_that_does_not_answer_is_offline() {
    let st = fake();
    st.borrow_mut().online = false;
    let mut p = port(&st);
    let r = o::Orbital.read(&matched(), &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.online, r.dpi, r.polling_hz), (false, None, None));
    assert_eq!(kinds(&st), vec![(1, 0x81)], "one handshake, no loop");
    // a mouse that is gone is an error, not "offline"
    let mut gone = ScriptPort::new(cfg(0x080C), |_, _| Err(Error::MouseGone("orbital".into())));
    assert!(matches!(o::Orbital.read(&matched(), &mut gone, ReadOptions::default()), Err(Error::MouseGone(_))));
}

#[test]
fn a_dpi_change_writes_the_sensor_block_saves_and_reads_back() {
    let st = fake();
    let mut p = port(&st);
    let c = o::Orbital.write(&matched(), &mut p, Setting::Dpi(1600)).unwrap();
    assert_eq!((c.now, c.before), (1600, Some(800)));
    // read sensor, read system, write sensor, save, read sensor, read system
    assert_eq!(kinds(&st), vec![(4, 0x81), (4, 0x83), (4, 0xBC), (10, 0x81), (4, 0x81), (4, 0x83)]);
    let want = o::sensor_write(&o::merge(&o::decode_settings(&sensor_answer(), &system_answer()).unwrap(), o::Change::Dpi(1600)), &o::decode_settings(&sensor_answer(), &system_answer()).unwrap());
    assert_eq!(p.sent[2].0, wire(&want));
    assert_eq!(p.sent[3].0, wire(&o::save()));
    let now = o::decode_settings(&st.borrow().sensor, &st.borrow().system).unwrap();
    assert_eq!((now.stages[1], now.stages[0], now.stages[2]), ((1600, 1600), (400, 400), (1600, 1600)), "only the active stage changed");
    assert_eq!(now.colors, o::decode_settings(&sensor_answer(), &system_answer()).unwrap().colors, "the stage colours are copied back");
    // the writes are refused by the read-only allow-list, the reads are not
    assert!(!o::read_request_allowed(&p.sent[2].0) && !o::read_request_allowed(&p.sent[3].0));
    assert!(o::read_request_allowed(&p.sent[0].0) && o::read_request_allowed(&p.sent[4].0));
}

#[test]
fn a_polling_change_writes_the_system_block_only() {
    let st = fake();
    let mut p = port(&st);
    let c = o::Orbital.write(&matched(), &mut p, Setting::Polling(4000)).unwrap();
    assert_eq!((c.now, c.before), (4000, Some(1000)));
    assert_eq!(kinds(&st), vec![(4, 0x81), (4, 0x83), (4, 0x90), (10, 0x81), (4, 0x81), (4, 0x83)]);
    // index 4 → raw 5; the rest of the block is the stored one (sleep, debounce, wake flags)
    let s = st.borrow();
    assert_eq!((s.system[8], s.system[9], s.system[10]), (5, 4, 1));
    assert_eq!(s.sensor, sensor_answer(), "the sensor block is untouched");
    drop(s);
    let again = o::Orbital.write(&matched(), &mut port(&st), Setting::Polling(125)).unwrap();
    assert_eq!((again.now, again.before), (125, Some(4000)));
    assert_eq!(st.borrow().system[8], 0, "125 Hz is raw 0");
}

#[test]
fn a_change_the_mouse_does_not_keep_is_an_error() {
    // a mouse that ignores the write: the read-back shows the old value
    let st = fake();
    let stale = st.clone();
    let inner = port(&st);
    let mut p = ScriptPort::new(cfg(0x080C), {
        let mut inner = inner;
        move |out, how| {
            use bu_mouse::family::Port;
            let body0 = out[1];
            let r = inner.exchange(out, how);
            if body0 == 4 && out[3] == 0xBC {
                // undo the stored write
                stale.borrow_mut().sensor = sensor_answer();
            }
            r
        }
    });
    let e = o::Orbital.write(&matched(), &mut p, Setting::Dpi(3200)).unwrap_err();
    assert!(matches!(e, Error::BadAnswer(ref m) if m.contains("kept 800 DPI")), "{e:?}");
}

#[test]
fn a_rewrite_that_would_change_saved_values_is_refused_with_nothing_written() {
    // two different sleep times: OpenMouse's system packet writes the sleep time into both slots
    let st = fake();
    st.borrow_mut().system[4 + 2] = 0x58;
    st.borrow_mut().system[4 + 3] = 0x02;
    let mut p = port(&st);
    assert!(matches!(o::Orbital.write(&matched(), &mut p, Setting::Polling(2000)), Err(Error::BadAnswer(m)) if m.contains("nothing was written")));
    assert_eq!(kinds(&st), vec![(4, 0x81), (4, 0x83)], "reads only");
    // the sensor block is fine, so a DPI change still goes through
    assert!(o::Orbital.write(&matched(), &mut port(&st), Setting::Dpi(1600)).is_ok());
    // an unknown non-zero byte in the sensor block (sensorData[33], between the stages and the colours): DPI refused
    let st = fake();
    st.borrow_mut().sensor[4 + 33] = 7;
    let mut p = port(&st);
    assert!(o::Orbital.write(&matched(), &mut p, Setting::Dpi(1600)).is_err());
    assert_eq!(kinds(&st), vec![(4, 0x81), (4, 0x83)]);
    // a stored stage that is not on the 50-DPI grid would be rounded by OpenMouse's rewrite: refused too
    let st = fake();
    st.borrow_mut().sensor[4 + 11] = 0x91;
    let mut p = port(&st);
    assert!(o::Orbital.write(&matched(), &mut p, Setting::Dpi(1600)).is_err());
}

#[test]
fn settings_the_mouse_cannot_store_are_refused_before_a_byte_leaves() {
    let st = fake();
    let mut p = port(&st);
    for s in [Setting::Dpi(49), Setting::Dpi(1625), Setting::Dpi(30050), Setting::Polling(250), Setting::Polling(3000), Setting::LiftOff(10)] {
        assert!(o::Orbital.write(&matched(), &mut p, s).is_err(), "{s:?}");
    }
    assert!(p.sent.is_empty());
}

// ---------------------------------------------------------------- Order 110: debounce, Motion Sync, angle snapping, ripple

#[test]
fn ghost_read_shows_debounce_and_the_sensor_switches() {
    let st = fake();
    let mut p = port(&st);
    let r = o::Orbital.read(&matched(), &mut p, ReadOptions::default()).unwrap();
    // readStatus: debounceMs = systemData[5] = 4; motionSync / angleSnapping / rippleControl = sensorData[2] / [0] / [1] != 0
    let got: Vec<(Extra, Option<i32>)> = r.extras.iter().map(|e| (e.extra, e.value)).collect();
    assert_eq!(got, vec![(Extra::Debounce, Some(4)), (Extra::MotionSync, Some(1)), (Extra::AngleSnap, Some(0)), (Extra::RippleControl, Some(0))]);
    // mergeEditableSettings: debounceMs clamped to 0..30
    assert_eq!(r.extras[0].kind, Kind::Range { min: 0, max: 30, step: 1, unit: "ms" });
    assert!(p.sent.iter().all(|s| o::read_request_allowed(&s.0)));
}

#[test]
fn ghost_changes_are_openmouses_block_writes() {
    // the sensor block with nothing changed sums to 2965: header 4 + 0xBC + 2 = 194; flags 1 + 1 + lift-off 1 + active 1 +
    // mask 31 = 35; stages 2·(0x90+1) + 2·(0x20+3) + 2·(0x40+6) + 2·(0x80+0x0C) + 2·(0xC0+0x12) = 1200; colours 1536
    let st = fake();
    let go = |c: Change| -> Vec<u8> {
        st.borrow_mut().seen.clear();
        let mut p = port(&st);
        o::Orbital.change(&matched(), &mut p, c).unwrap();
        // read sensor, read system, the one block write, save, read sensor, read system
        let k = kinds(&st);
        assert_eq!((k.len(), k[0], k[1], k[3], k[4], k[5]), (6, (4, 0x81), (4, 0x83), (10, 0x81), (4, 0x81), (4, 0x83)), "{c:?}");
        p.sent[2].0.clone()
    };
    let sensor = |set: &[(usize, u8)], crc: u8| {
        let mut b = sensor_answer();
        b[0..4].copy_from_slice(&[4, 0, 0xBC, 2]);
        for (i, v) in set {
            b[*i] = *v;
        }
        b[63] = crc;
        let mut p = [0u8; 64];
        p.copy_from_slice(&b);
        wire(&p)
    };
    // setMotionSync(false): [6] = 0 → 2964 = 11·256 + 148 → 0xA1 − 0x94 = 0x0D
    assert_eq!(go(Change::Extra(Extra::MotionSync, 0)), sensor(&[(6, 0)], 0x0D));
    // setAngleSnapping(true) (motion sync now off): [4] = 1 → 2965 = 11·256 + 149 → 0xA1 − 0x95 = 0x0C
    assert_eq!(go(Change::Extra(Extra::AngleSnap, 1)), sensor(&[(4, 1), (6, 0)], 0x0C));
    // setRippleControl(true): [5] = 1 → 2966 → 0xA1 − 0x96 = 0x0B
    assert_eq!(go(Change::Extra(Extra::RippleControl, 1)), sensor(&[(4, 1), (5, 1), (6, 0)], 0x0B));
    // setDebounceTime(30): the system block, [9] = 30; 152 + sleep 90 + rate 3 + 30 + 3 = 278 → 22 → 0xA1 − 0x16 = 0x8B
    let want = pk(&[(0, 4), (2, 0x90), (3, 4), (4, 0x2C), (5, 1), (6, 0x2C), (7, 1), (8, 3), (9, 30), (10, 1), (11, 1), (12, 1), (63, 0x8B)]);
    assert_eq!(go(Change::Extra(Extra::Debounce, 30)), wire(&want));
    let r = o::Orbital.read(&matched(), &mut port(&st), ReadOptions::default()).unwrap();
    let got: Vec<Option<i32>> = r.extras.iter().map(|e| e.value).collect();
    assert_eq!(got, vec![Some(30), Some(0), Some(1), Some(1)]);
    assert_eq!(r.dpi, Some(800), "DPI untouched");
}

#[test]
fn ghost_refuses_what_openmouse_does_not_take_before_a_byte_leaves() {
    let st = fake();
    for c in [Change::Extra(Extra::Debounce, 31), Change::Extra(Extra::MotionSync, 2), Change::Extra(Extra::Turbo, 1)] {
        let mut p = port(&st);
        assert!(o::Orbital.change(&matched(), &mut p, c).is_err(), "{c:?}");
        assert!(p.sent.is_empty(), "{c:?}");
    }
    // a block the rewrite would change (an unknown non-zero byte): refused after the reads, nothing written
    st.borrow_mut().sensor[4 + 36] = 9;
    let mut p = port(&st);
    assert!(o::Orbital.change(&matched(), &mut p, Change::Extra(Extra::MotionSync, 0)).is_err());
    assert_eq!(kinds(&st), vec![(4, 0x81), (4, 0x83)]);
}

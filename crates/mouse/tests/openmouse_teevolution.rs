//! Order 105 check, Teevolution Terra Pro: our packets = the bytes OpenMouse sends for the same setting (encode), and answers
//! shaped the way OpenMouse reads them decode to the same values (decode). Expected bytes are worked out by hand from OpenMouse
//! (@openmouse/protocol 0.40.1: `dist/teevolution/index.js`, `dist/drivers/teevolution/hid.js`).
//!
//! OpenMouse sends `sendReport(8, 16 bytes)`; byte 15 is the checksum `(0x55 - (sum of bytes 0..14) - 8) & 0xFF`. Windows puts
//! the report id 8 in front (17 bytes): OpenMouse's byte i is our byte i + 1.

use bu_mouse::family::{self, Driver, Family, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::os::{HidInfo, HidTransfer};
use bu_mouse::teevolution as t;
use bu_mouse::Error;
use std::cell::RefCell;
use std::rc::Rc;

fn pl(bytes: &[(usize, u8)]) -> t::Payload {
    let mut p = [0u8; 16];
    for (i, b) in bytes {
        p[*i] = *b;
    }
    p
}

fn cfg(pid: u16) -> HidInfo {
    hid_info("tee-cfg", 0x3554, pid, 0xFF02, 1, (17, 17, 0))
}

/// The checksum the way the fake mouse seals its answers (same formula, written out).
fn sealed(mut p: t::Payload) -> t::Payload {
    let sum: u32 = p[..15].iter().map(|b| *b as u32).sum();
    p[15] = ((0x55 + 512 - (sum & 0xFF) - 8) & 0xFF) as u8;
    p
}

// ---------------------------------------------------------------- encode

#[test]
fn requests_are_openmouses_payloads() {
    // battery: [04, 0..]: 85 − 4 − 8 = 73 = 0x49
    assert_eq!(t::simple(4), pl(&[(0, 4), (15, 0x49)]));
    // host control: [03, 0, 0, 0, 0, enabled]; entry sum 4 → 73 = 0x49, exit sum 3 → 74 = 0x4A
    assert_eq!(t::online_request(true), pl(&[(0, 3), (5, 1), (15, 0x49)]));
    assert_eq!(t::online_request(false), pl(&[(0, 3), (15, 0x4A)]));
    // identification: [01, 0, 0, 0, 08, challenge x4, 0..]; challenge 12 34 56 78: sum = 1 + 8 + 0x12 + 0x34 + 0x56 + 0x78
    // = 285 → & 0xFF = 29; 85 − 29 − 8 = 48 = 0x30
    assert_eq!(t::identify_request([0x12, 0x34, 0x56, 0x78]), pl(&[(0, 1), (4, 8), (5, 0x12), (6, 0x34), (7, 0x56), (8, 0x78), (15, 0x30)]));
    // flash reads: [08, 0, addrHi, addrLo, length]; crc = 85 − 8 − (8 + addrHi + addrLo + length)
    // (0, 10): 59 = 0x3B · (10, 10): 49 = 0x31 · (20, 10): 39 = 0x27 · (2, 2): 85 − 8 − 12 = 65 = 0x41
    // (4, 2): 63 = 0x3F · (16, 4): 85 − 8 − 28 = 49 = 0x31 · (0, 2): 67 = 0x43
    for ((a, l), crc) in [(0u16, 10u8), (10, 10), (20, 10), (2, 2), (4, 2), (16, 4), (0, 2)].into_iter().zip([0x3B, 0x31, 0x27, 0x41, 0x3F, 0x31, 0x43]) {
        assert_eq!(t::read_request(a, l), pl(&[(0, 8), (2, (a >> 8) as u8), (3, a as u8), (4, l), (15, crc)]), "{a}");
    }
}

#[test]
fn dpi_records_are_openmouses_encoding() {
    // teevolutionEncodeDpi: raw = dpi / 50 − 1; below 30100: dpiEx 0; low = raw & 0xFF, high = raw >> 8,
    // flags = (high << 2) | (high << 6) | dpiEx | (dpiEx << 4); record [low, low, flags, 0x55 − low − low − flags]
    // 800: raw 15 → [0F 0F 00 37] · 1600: raw 31 → 85 − 62 = 23 → [1F 1F 00 17] · 50: [00 00 00 55]
    assert_eq!(t::encode_dpi(800), Some([0x0F, 0x0F, 0x00, 0x37]));
    assert_eq!(t::encode_dpi(1600), Some([0x1F, 0x1F, 0x00, 0x17]));
    assert_eq!(t::encode_dpi(50), Some([0x00, 0x00, 0x00, 0x55]));
    // 30000: raw 599 = 0x257 → low 0x57, high 2 → flags 0x08 | 0x80 = 0x88; 85 − 87 − 87 − 136 = −225 → 31 = 0x1F
    assert_eq!(t::encode_dpi(30000), Some([0x57, 0x57, 0x88, 0x1F]));
    // 30100 and up: dpiEx 0x11, the DPI halved: 15050 → raw 300 = 0x12C → low 0x2C, high 1 → flags = 0x04 | 0x40 | 0x11 |
    // (0x110 & 0xFF = 0x10) = 0x55; 85 − 44 − 44 − 85 = −88 → 168 = 0xA8
    assert_eq!(t::encode_dpi(30100), Some([0x2C, 0x2C, 0x55, 0xA8]));
    // 42000 → 21000 → raw 419 = 0x1A3 → low 0xA3, high 1 → flags 0x55; 85 − 163 − 163 − 85 = −326 → 186 = 0xBA
    assert_eq!(t::encode_dpi(42000), Some([0xA3, 0xA3, 0x55, 0xBA]));
    for bad in [0, 49, 75] {
        assert_eq!(t::encode_dpi(bad), None, "{bad}");
    }
    // teevolutionDecodeDpi: raw = low + (((flags & 0x0C) >> 2) << 8); dpi = (raw + 1) * 50, doubled for each dpiEx bit
    assert_eq!(t::decode_dpi(&[0x1F, 0x1F, 0x00, 0x17]), Some(1600));
    assert_eq!(t::decode_dpi(&[0x57, 0x57, 0x88, 0x1F]), Some(30000));
    assert_eq!(t::decode_dpi(&[0x2C, 0x2C, 0x55, 0xA8]), Some(30100));
    assert_eq!(t::decode_dpi(&[0xA3, 0xA3, 0x55, 0xBA]), Some(42000));
    assert_eq!(t::decode_dpi(&[0x0F, 0x0F, 0x02, 0x00]), Some(1600), "dpiEx bit 1 also doubles (the checksum is not checked)");
    assert_eq!(t::decode_dpi(&[0x0F, 0x0F]), None);
    // every option the app offers survives encode → decode
    for (lo, hi, step) in t::DPI_BANDS {
        for d in (lo..=hi).step_by(step as usize) {
            assert_eq!(t::decode_dpi(&t::encode_dpi(d).unwrap()), Some(d), "{d}");
        }
    }
}

#[test]
fn the_writes_are_openmouses_flash_payloads() {
    // writeDpiStageValue(stage 1, 1600): writeFlash(12 + 4 = 16, record) = [07, 0, 00, 10, 04, 1F 1F 00 17]
    // crc: 85 − 8 − (7 + 0x10 + 4 + 0x1F + 0x1F + 0x17) = 85 − 8 − 112 = −35 → 221 = 0xDD
    assert_eq!(t::write_request(16, &[0x1F, 0x1F, 0x00, 0x17]), pl(&[(0, 7), (3, 0x10), (4, 4), (5, 0x1F), (6, 0x1F), (8, 0x17), (15, 0xDD)]));
    // writeCheckedByte(0, 16) for 2000 Hz (rate / 2000 × 16): [07, 0, 0, 0, 02, 10, 45]; 85 − 8 − 94 = −17 → 239 = 0xEF
    assert_eq!(t::write_byte_request(0, 16), pl(&[(0, 7), (4, 2), (5, 0x10), (6, 0x45), (15, 0xEF)]));
    for (hz, code) in [(125, 8), (250, 4), (500, 2), (1000, 1), (2000, 16), (4000, 32), (8000, 64)] {
        assert_eq!(t::encode_polling(hz), Some(code), "{hz}");
        assert_eq!(t::decode_polling(code), Some(hz), "{code}");
    }
    assert_eq!((t::encode_polling(1500), t::decode_polling(3), t::decode_polling(0)), (None, None, None));
    let w = t::wire(&t::simple(4), 17);
    assert_eq!((w.len(), w[0], w[1], w[16]), (17, 8, 4, 0x49));
}

// ---------------------------------------------------------------- decode

#[test]
fn answers_decode_like_openmouse() {
    // identification: [01, status, ..., cid at 9, mid 10, type 11, dongle type 12]
    let id = |cid: u8, kind: u8| sealed(pl(&[(0, 1), (9, cid), (10, 7), (11, kind), (12, 2)]));
    let i = t::decode_identity(&id(14, 3)).unwrap();
    assert_eq!((i.cid, i.mid, i.kind, i.dongle_type, i.wired, i.max_polling_hz), (14, 7, 3, 2, true, 8000));
    // TEEVOLUTION_TYPE_MAX_POLL: 0 → 1000, 1 → 4000, 2 → 1000, 3 → 8000, 4 → 2000, 5 → 8000; unknown → 1000
    for (kind, max, wired) in [(0u8, 1000u32, false), (1, 4000, false), (2, 1000, true), (3, 8000, true), (4, 2000, false), (5, 8000, false), (9, 1000, false)] {
        let i = t::decode_identity(&id(14, kind)).unwrap();
        assert_eq!((i.max_polling_hz, i.wired), (max, wired), "type {kind}");
    }
    // another cid: unsupported, nothing may be read or written; a rejected status: an error
    assert!(matches!(t::decode_identity(&id(15, 3)), Err(Error::UnsupportedMouse(_))));
    assert!(matches!(t::decode_identity(&sealed(pl(&[(0, 1), (1, 1), (9, 14)]))), Err(Error::BadAnswer(_))));
    // battery: [04, 0, -, -, -, percent, charging]
    assert_eq!(t::parse_battery(&sealed(pl(&[(0, 4), (5, 87), (6, 1)]))), Some((Some(87), true)));
    assert_eq!(t::parse_battery(&sealed(pl(&[(0, 4), (5, 101)]))), Some((None, false)), "over 100 → no percentage");
    assert_eq!(t::parse_battery(&sealed(pl(&[(0, 4), (1, 1), (5, 50)]))), None);
    assert_eq!(t::parse_battery(&pl(&[(0, 4), (5, 50)])), None, "checksum");
    // a read answer
    let r = sealed(pl(&[(0, 8), (3, 4), (4, 2), (5, 0x01), (6, 0x54)]));
    assert_eq!(t::parse_read(&r, 4, 2), Some(vec![0x01, 0x54]));
    assert_eq!((t::parse_read(&r, 5, 2), t::parse_read(&r, 4, 3)), (None, None));
}

// ---------------------------------------------------------------- allow-list + table

#[test]
fn only_reads_and_the_exit_pass_the_allow_list() {
    for p in [t::identify_request([1, 2, 3, 4]), t::simple(4), t::read_request(0, 10), t::read_request(16, 4), t::online_request(false)] {
        assert!(t::read_request_allowed(&t::wire(&p, 17)), "{:02x?}", &p[..6]);
    }
    // host control ENTERED, flash writes, other commands, a profile switch (command 15), reads of 0 / 11 bytes: refused
    let switch = pl(&[(0, 0x0F), (4, 1), (5, 2)]);
    for p in [t::online_request(true), t::write_request(16, &[0x1F, 0x1F, 0, 0x17]), t::write_byte_request(0, 16), t::simple(0x12), t::simple(0x0F), switch, t::read_request(0, 11), t::read_request(0, 0), t::simple(1)] {
        assert!(!t::read_request_allowed(&t::wire(&p, 17)), "{:02x?}", &p[..6]);
    }
    // an identify query carrying extra bytes beyond the challenge
    let mut extra = t::identify_request([1, 2, 3, 4]);
    extra[10] = 9;
    assert!(!t::read_request_allowed(&t::wire(&extra, 17)));
    assert!(!t::read_request_allowed(&[8, 4]));
}

#[test]
fn the_table_is_openmouses_supported_list() {
    // supported-devices-data.ts: Teevolution Terra Pro pids [0xf520, 0xf523, 0xf5bb, 0xf522]
    for pid in [0xF520u16, 0xF523, 0xF5BB, 0xF522] {
        let c = cfg(pid);
        let m = t::matches(&[&c], 0x3554, pid).unwrap();
        assert_eq!((m.family, m.model, m.wireless, m.variant), (Family::Teevolution, Some("Teevolution Terra Pro"), None, pid));
        assert_eq!(family::find(std::slice::from_ref(&c), 0x3554, pid).map(|m| m.family), Some(Family::Teevolution));
    }
    // VGN's ids share the vendor id; another vendor; an unknown id
    for pid in [0xFB56u16, 0xFB57, 0xF521, 0x0000] {
        assert!(t::matches(&[&cfg(pid)], 0x3554, pid).is_none(), "{pid:04x}");
    }
    assert!(t::matches(&[&cfg(0xF520)], 0x3555, 0xF520).is_none());
    // the report collection: a 16-byte report 8 in and out only (the mouse collection and the LCD one don't fit)
    for lens in [(8u16, 0u16, 0u16), (17, 0, 0), (0, 17, 0), (65, 65, 0), (41, 41, 0)] {
        let h = hid_info("x", 0x3554, 0xF520, 0xFF02, 1, lens);
        assert!(t::matches(&[&h], 0x3554, 0xF520).is_none(), "{lens:?}");
    }
    let mouse = hid_info("mouse", 0x3554, 0xF520, 0x01, 0x02, (8, 0, 0));
    let c = cfg(0xF520);
    assert_eq!(t::matches(&[&mouse, &c], 0x3554, 0xF520).map(|m| m.config.path), Some(c.path.clone()));
    // of two 17-byte collections, the vendor one
    let plain = hid_info("plain", 0x3554, 0xF520, 0x01, 0x06, (17, 17, 0));
    assert_eq!(t::matches(&[&plain, &c], 0x3554, 0xF520).map(|m| m.config.path), Some(c.path.clone()));
}

#[test]
fn caps_are_what_openmouse_offers() {
    let c = t::caps(8000);
    let d = c.dpi.unwrap();
    assert_eq!((d.min(), d.max()), (50, 42000));
    assert!(d.allows(30000) && !d.allows(30050) && d.allows(30100) && !d.allows(30150) && d.allows(42000) && !d.allows(42100));
    assert_eq!(c.polling, vec![125, 250, 500, 1000, 2000, 4000, 8000]);
    assert_eq!(t::caps(1000).polling, vec![125, 250, 500, 1000], "the connection's maximum");
    assert_eq!(t::caps(2000).polling.last(), Some(&2000));
    assert!(c.lift_off.is_empty() && c.battery);
}

// ---------------------------------------------------------------- the driver against a scripted mouse

struct Fake {
    flash: Vec<u8>,
    cid: u8,
    kind: u8,
    /// the identification gets no answer
    silent: bool,
    /// the mouse behind the receiver is asleep (the host-control entry answers "offline")
    asleep: bool,
    /// the next N host-control answers say "busy"
    busy: u32,
    locked: bool,
    /// every request (the 16 payload bytes), in order
    seen: Vec<Vec<u8>>,
}

/// 4 stages, stage 1 active, 1000 Hz, 400 / 800 / 1600 / 3200
fn flash() -> Vec<u8> {
    let mut f = vec![0u8; 190];
    // Order 110 (TEEVOLUTION_FLASH, `[v, 0x55 − v]` pairs): debounceTime 169 = 4 ms, motionSync 171 on, angleSnapping 175 off,
    // rippleControl 177 on, performanceState 181 on
    for (a, v) in [(169usize, 4u8), (171, 1), (175, 0), (177, 1), (181, 1)] {
        f[a] = v;
        f[a + 1] = 0x55 - v;
    }
    f[0..2].copy_from_slice(&[0x01, 0x54]);
    f[2..4].copy_from_slice(&[0x04, 0x51]);
    f[4..6].copy_from_slice(&[0x01, 0x54]);
    f[12..16].copy_from_slice(&[0x07, 0x07, 0x00, 0x47]);
    f[16..20].copy_from_slice(&[0x0F, 0x0F, 0x00, 0x37]);
    f[20..24].copy_from_slice(&[0x1F, 0x1F, 0x00, 0x17]);
    f[24..28].copy_from_slice(&[0x3F, 0x3F, 0x00, 0xD7]);
    f
}

fn fake() -> Rc<RefCell<Fake>> {
    Rc::new(RefCell::new(Fake { flash: flash(), cid: 14, kind: 3, silent: false, asleep: false, busy: 0, locked: false, seen: Vec::new() }))
}

fn port(st: &Rc<RefCell<Fake>>) -> ScriptPort {
    let st = st.clone();
    ScriptPort::new(cfg(0xF520), move |out, how| {
        let HidTransfer::OutputThenInput { echo, reply_len, .. } = how else { panic!("Teevolution uses output + input reports, not {how:?}") };
        assert_eq!((out.len(), out[0], *reply_len), (17, 8, 17));
        let q = out[1..].to_vec();
        assert_eq!(*echo, Some((1, q[0])), "the answer must carry the command in its first byte");
        let mut s = st.borrow_mut();
        s.seen.push(q.clone());
        if s.silent {
            return Err(Error::BadAnswer("no answer in time".into()));
        }
        let mut a = [0u8; 16];
        a[0] = q[0];
        match q[0] {
            1 => {
                a[9] = s.cid;
                a[10] = 5;
                a[11] = s.kind;
                a[12] = 1;
            }
            3 => {
                if s.busy > 0 {
                    s.busy -= 1;
                    a[9] = 1;
                } else {
                    s.locked = q[5] == 1;
                    a[5] = (!s.asleep) as u8;
                }
            }
            4 => {
                a[5] = 87;
                a[6] = 1;
            }
            8 => {
                let (addr, len) = (u16::from_be_bytes([q[2], q[3]]) as usize, q[4] as usize);
                a[2..5].copy_from_slice(&q[2..5]);
                if !s.locked {
                    a[1] = 1;
                } else {
                    a[5..5 + len].copy_from_slice(&s.flash[addr..addr + len]);
                }
            }
            7 => {
                let (addr, len) = (u16::from_be_bytes([q[2], q[3]]) as usize, q[4] as usize);
                if !s.locked {
                    a[1] = 1;
                } else {
                    s.flash[addr..addr + len].copy_from_slice(&q[5..5 + len]);
                }
            }
            c => panic!("unexpected command {c}"),
        }
        let mut v = vec![8u8];
        v.extend_from_slice(&sealed(a));
        Ok(v)
    })
}

fn matched() -> bu_mouse::family::Matched {
    t::matches(&[&cfg(0xF520)], 0x3554, 0xF520).unwrap()
}

/// (command, address low, length / byte 5) per request
fn cmds(st: &Rc<RefCell<Fake>>) -> Vec<(u8, u8, u8)> {
    st.borrow().seen.iter().map(|q| (q[0], q[3], if q[0] == 3 { q[5] } else { q[4] })).collect()
}

#[test]
fn reading_identifies_enters_host_control_reads_and_leaves() {
    let st = fake();
    let mut p = port(&st);
    let r = t::Teevolution.read(&matched(), &mut p, ReadOptions::default()).unwrap();
    // readFlash(reportRate 0, sensorMode 185 + 2): 18 chunks of ten, then 180..187 (7)
    let mut want = vec![(1, 0, 8), (3, 0, 1)];
    want.extend((0..18).map(|i| (8, i * 10, 10)));
    want.extend([(8, 180, 7), (4, 0, 0), (3, 0, 0)]);
    assert_eq!(cmds(&st), want);
    assert_eq!((r.online, r.dpi, r.polling_hz, r.battery_percent, r.charging, r.lift_off, r.wireless), (true, Some(800), Some(1000), Some(87), Some(true), None, Some(false)));
    assert_eq!(r.caps, t::caps(8000));
    // everything except the host-control entry is an allowed read request
    let refused: Vec<usize> = p.sent.iter().enumerate().filter(|(_, s)| !t::read_request_allowed(&s.0)).map(|(i, _)| i).collect();
    assert_eq!(refused, vec![1], "only the host-control entry");
    // a wireless 1000 Hz link (type 0): wireless, polling rates capped
    st.borrow_mut().kind = 0;
    let r = t::Teevolution.read(&matched(), &mut port(&st), ReadOptions::default()).unwrap();
    assert_eq!((r.wireless, r.caps.polling.last().copied()), (Some(true), Some(1000)));
}

#[test]
fn a_mouse_that_is_not_there_is_offline() {
    // no answer to the identification
    let st = fake();
    st.borrow_mut().silent = true;
    let r = t::Teevolution.read(&matched(), &mut port(&st), ReadOptions::default()).unwrap();
    assert_eq!((r.online, r.dpi), (false, None));
    assert_eq!(cmds(&st), vec![(1, 0, 8)]);
    // the receiver is there, the mouse is asleep: no flash read, and host control is not left (it was never entered)
    let st = fake();
    st.borrow_mut().asleep = true;
    let r = t::Teevolution.read(&matched(), &mut port(&st), ReadOptions::default()).unwrap();
    assert_eq!((r.online, r.dpi, r.polling_hz), (false, None, None));
    assert_eq!(cmds(&st), vec![(1, 0, 8), (3, 0, 1)]);
    // a busy receiver is asked again (10 ms apart), up to the tries: 3 busy then ready
    let st = fake();
    st.borrow_mut().busy = 3;
    let r = t::Teevolution.read(&matched(), &mut port(&st), ReadOptions::default()).unwrap();
    assert!(r.online);
    assert_eq!(cmds(&st).iter().filter(|c| c.0 == 3 && c.2 == 1).count(), 4);
    // the read-only proof asks once ("do not loop"): busy once = offline
    let st = fake();
    st.borrow_mut().busy = 1;
    let r = t::Teevolution.read(&matched(), &mut port(&st), ReadOptions { online_tries: 1 }).unwrap();
    assert!(!r.online);
    assert_eq!(cmds(&st), vec![(1, 0, 8), (3, 0, 1)]);
    // a different cid: not a Terra Pro, an error and nothing else is sent
    let st = fake();
    st.borrow_mut().cid = 20;
    assert!(matches!(t::Teevolution.read(&matched(), &mut port(&st), ReadOptions::default()), Err(Error::UnsupportedMouse(_))));
    assert_eq!(cmds(&st), vec![(1, 0, 8)]);
    let mut gone = ScriptPort::new(cfg(0xF520), |_, _| Err(Error::MouseGone("t".into())));
    assert!(matches!(t::Teevolution.read(&matched(), &mut gone, ReadOptions::default()), Err(Error::MouseGone(_))));
}

#[test]
fn a_failed_read_still_leaves_host_control() {
    let st = fake();
    // the flash read is rejected by the receiver (status 1) after the entry
    let inner = port(&st);
    let mut inner = inner;
    let mut p = ScriptPort::new(cfg(0xF520), move |out, how| {
        use bu_mouse::family::Port;
        let mut r = inner.exchange(out, how)?;
        if out[1] == 8 {
            r[2] = 1;
            let mut a = [0u8; 16];
            a.copy_from_slice(&r[1..17]);
            r = std::iter::once(8u8).chain(sealed(a)).collect();
        }
        Ok(r)
    });
    assert!(t::Teevolution.read(&matched(), &mut p, ReadOptions::default()).is_err());
    let last = *cmds(&st).last().unwrap();
    assert_eq!(last, (3, 0, 0), "the exit is always sent");
}

#[test]
fn a_dpi_change_is_identify_enter_read_write_read_leave() {
    let st = fake();
    let mut p = port(&st);
    let c = t::Teevolution.write(&matched(), &mut p, Setting::Dpi(1600)).unwrap();
    assert_eq!((c.now, c.before), (1600, Some(800)));
    // identify, enter, stage count (2), active stage (4), the stored record (16), the write, the read-back, leave
    assert_eq!(cmds(&st), vec![(1, 0, 8), (3, 0, 1), (8, 2, 2), (8, 4, 2), (8, 16, 4), (7, 16, 4), (8, 16, 4), (3, 0, 0)]);
    assert_eq!(p.sent[5].0, t::wire(&t::write_request(16, &[0x1F, 0x1F, 0x00, 0x17]), 17));
    assert!(!t::read_request_allowed(&p.sent[5].0));
    assert_eq!(st.borrow().flash[12..28], [0x07, 0x07, 0x00, 0x47, 0x1F, 0x1F, 0x00, 0x17, 0x1F, 0x1F, 0x00, 0x17, 0x3F, 0x3F, 0x00, 0xD7], "only the active stage changed");
    // above 30000: dpiEx 0x11 and steps of 100
    let c = t::Teevolution.write(&matched(), &mut port(&st), Setting::Dpi(42000)).unwrap();
    assert_eq!((c.now, c.before), (42000, Some(1600)));
    assert_eq!(st.borrow().flash[16..20], [0xA3, 0xA3, 0x55, 0xBA]);
    // active stage 2 → the record at 12 + 8 = 20
    st.borrow_mut().flash[4..6].copy_from_slice(&[0x02, 0x53]);
    t::Teevolution.write(&matched(), &mut port(&st), Setting::Dpi(3000)).unwrap();
    assert_eq!(st.borrow().flash[20..24], t::encode_dpi(3000).unwrap());
}

#[test]
fn a_dpi_stage_with_unknown_bits_is_not_overwritten() {
    let st = fake();
    st.borrow_mut().flash[18] = 0x01; // flags: not something OpenMouse's encoding produces
    let mut p = port(&st);
    let e = t::Teevolution.write(&matched(), &mut p, Setting::Dpi(1600)).unwrap_err();
    assert!(matches!(e, Error::BadAnswer(ref m) if m.contains("nothing was written")), "{e:?}");
    assert!(st.borrow().seen.iter().all(|q| q[0] != 7), "no flash write");
    assert_eq!(cmds(&st).last(), Some(&(3, 0, 0)), "host control left");
}

#[test]
fn a_polling_change_is_read_write_read() {
    let st = fake();
    let mut p = port(&st);
    let c = t::Teevolution.write(&matched(), &mut p, Setting::Polling(2000)).unwrap();
    assert_eq!((c.now, c.before), (2000, Some(1000)));
    assert_eq!(cmds(&st), vec![(1, 0, 8), (3, 0, 1), (8, 0, 2), (7, 0, 2), (8, 0, 2), (3, 0, 0)]);
    assert_eq!(p.sent[3].0, t::wire(&t::write_byte_request(0, 16), 17));
    assert_eq!(st.borrow().flash[0..2], [0x10, 0x45]);
    assert_eq!(t::Teevolution.write(&matched(), &mut port(&st), Setting::Polling(125)).unwrap().before, Some(2000));
    assert_eq!(st.borrow().flash[0..2], [0x08, 0x4D]);
}

#[test]
fn what_the_connection_cannot_do_is_refused_before_host_control() {
    // a wired 1000 Hz link (type 2): 2000 Hz is above its maximum
    let st = fake();
    st.borrow_mut().kind = 2;
    let mut p = port(&st);
    assert!(t::Teevolution.write(&matched(), &mut p, Setting::Polling(2000)).is_err());
    assert_eq!(cmds(&st), vec![(1, 0, 8)], "identified, then refused");
    // a sleeping mouse
    let st = fake();
    st.borrow_mut().asleep = true;
    let e = t::Teevolution.write(&matched(), &mut port(&st), Setting::Dpi(1600)).unwrap_err();
    assert!(matches!(e, Error::BadAnswer(ref m) if m.contains("offline")), "{e:?}");
    assert!(st.borrow().seen.iter().all(|q| q[0] != 7));
    // another cid
    let st = fake();
    st.borrow_mut().cid = 3;
    assert!(matches!(t::Teevolution.write(&matched(), &mut port(&st), Setting::Dpi(1600)), Err(Error::UnsupportedMouse(_))));
}

#[test]
fn values_the_mouse_cannot_store_are_refused_before_a_byte_leaves() {
    let st = fake();
    let mut p = port(&st);
    for s in [Setting::Dpi(49), Setting::Dpi(75), Setting::Dpi(30050), Setting::Dpi(42100), Setting::Polling(1500), Setting::Polling(0), Setting::LiftOff(10)] {
        assert!(t::Teevolution.write(&matched(), &mut p, s).is_err(), "{s:?}");
    }
    assert!(p.sent.is_empty());
}

// ---------------------------------------------------------------- Order 110: debounce, Motion Sync, angle snapping, ripple, turbo

use bu_mouse::family::{Change, Extra, Kind};

#[test]
fn terra_read_shows_every_setting_openmouse_reads() {
    let st = fake();
    let mut p = port(&st);
    let r = t::Teevolution.read(&matched(), &mut p, ReadOptions::default()).unwrap();
    // readStatus: debounceMs = flash[169]; motionSync / angleSnapping / rippleControl / performanceMode = flash[x] === 1
    let got: Vec<(Extra, Option<i32>)> = r.extras.iter().map(|e| (e.extra, e.value)).collect();
    assert_eq!(got, vec![(Extra::Debounce, Some(4)), (Extra::MotionSync, Some(1)), (Extra::AngleSnap, Some(0)), (Extra::RippleControl, Some(1)), (Extra::Turbo, Some(1))]);
    // the Terra Pro profile: debounce { min 0, max 20 }
    assert_eq!(r.extras[0].kind, Kind::Range { min: 0, max: 20, step: 1, unit: "ms" });
    assert!(r.extras[1..].iter().all(|e| e.kind == Kind::Switch));
    // reading: everything but the host-control entry passes the allow-list
    let refused: Vec<usize> = p.sent.iter().enumerate().filter(|(_, s)| !t::read_request_allowed(&s.0)).map(|(i, _)| i).collect();
    assert_eq!(refused, vec![1]);
}

/// The flash writes (command 7) a change sent, as Windows sends them.
fn flash_writes(p: &ScriptPort) -> Vec<Vec<u8>> {
    p.sent.iter().map(|s| s.0.clone()).filter(|f| f[1] == 7).collect()
}

#[test]
fn terra_changes_are_openmouses_writeflash_bytes() {
    let st = fake();
    let go = |c: Change| -> Vec<Vec<u8>> {
        let mut p = port(&st);
        t::Teevolution.change(&matched(), &mut p, c).unwrap();
        // identify, enter host control, the write, its read-back (addr, 2), leave
        assert_eq!(cmds(&st).iter().rev().take(5).rev().copied().collect::<Vec<_>>(), vec![(1, 0, 8), (3, 0, 1), (7, p.sent[2].0[4], 2), (8, p.sent[2].0[4], 2), (3, 0, 0)], "{c:?}");
        flash_writes(&p)
    };
    // writeCheckedByte(addr, v) = [07, 0, 0, addr, 02, v, 0x55 − v]; the sum is 7 + 2 + 0x55 + addr (v and 0x55 − v add to
    // 0x55), so the checksum is 85 − 8 − 94 − addr = 239 − addr; Windows puts the id 8 in front
    let w8 = |addr: u8, v: u8, crc: u8| t::wire(&pl(&[(0, 7), (3, addr), (4, 2), (5, v), (6, 0x55u8.wrapping_sub(v)), (15, crc)]), 17);
    // setDebounceTime(8): 169 = 0xA9 → 70 = 0x46 ([.., 08, 4D])
    assert_eq!(go(Change::Extra(Extra::Debounce, 8)), vec![w8(0xA9, 8, 0x46)]);
    assert_eq!(go(Change::Extra(Extra::Debounce, 8))[0][6..8], [0x08, 0x4D]);
    // setMotionSync(false): 171 = 0xAB → 68 = 0x44
    assert_eq!(go(Change::Extra(Extra::MotionSync, 0)), vec![w8(0xAB, 0, 0x44)]);
    // setAngleSnapping(true): 175 = 0xAF → 64 = 0x40 ([.., 01, 54])
    assert_eq!(go(Change::Extra(Extra::AngleSnap, 1)), vec![w8(0xAF, 1, 0x40)]);
    // setRippleControl(false): 177 = 0xB1 → 62 = 0x3E
    assert_eq!(go(Change::Extra(Extra::RippleControl, 0)), vec![w8(0xB1, 0, 0x3E)]);
    // setPerformanceMode(false): 181 = 0xB5 → 58 = 0x3A
    assert_eq!(go(Change::Extra(Extra::Turbo, 0)), vec![w8(0xB5, 0, 0x3A)]);
    // setDebounceTime(20): the top of the range; 0x55 − 20 = 0x41
    assert_eq!(go(Change::Extra(Extra::Debounce, 20)), vec![w8(0xA9, 20, 0x46)]);
    // the fake mouse holds what was written; a read shows it
    let r = t::Teevolution.read(&matched(), &mut port(&st), ReadOptions::default()).unwrap();
    let got: Vec<Option<i32>> = r.extras.iter().map(|e| e.value).collect();
    assert_eq!(got, vec![Some(20), Some(0), Some(1), Some(0), Some(0)]);
    // a sleeping mouse: offline, nothing written
    st.borrow_mut().asleep = true;
    let mut p = port(&st);
    assert!(t::Teevolution.change(&matched(), &mut p, Change::Extra(Extra::MotionSync, 1)).is_err());
    assert!(flash_writes(&p).is_empty());
}

#[test]
fn terra_refuses_what_openmouse_does_not_take_before_a_byte_leaves() {
    let st = fake();
    for c in [
        Change::Extra(Extra::Debounce, 21),
        Change::Extra(Extra::Debounce, -1),
        Change::Extra(Extra::MotionSync, 2),
    ] {
        let mut p = port(&st);
        assert!(t::Teevolution.change(&matched(), &mut p, c).is_err(), "{c:?}");
        assert!(p.sent.is_empty(), "{c:?}: nothing sent");
    }
}

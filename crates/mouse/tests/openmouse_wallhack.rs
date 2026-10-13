//! Order 105 check, WALLHACK M-001: our packets = the bytes OpenMouse sends for the same setting (encode), and answers shaped
//! the way OpenMouse reads them decode to the same values (decode). Expected bytes are worked out by hand from OpenMouse
//! (@openmouse/protocol 0.40.1: `dist/wallhack/index.js`, `dist/drivers/wallhack/mouse-hid.js`).
//!
//! OpenMouse sends `sendReport(4, 63 bytes)` with no checksum: simple `[0, 0, cmd]`, read `[0, 0, 0xA4, n, addrLo, addrHi, 0]`,
//! write `[0, 0, 0xA5, n, addrLo, addrHi, 0, data...]`. Windows puts the id 4 in front: OpenMouse's byte i is our byte i + 1.

use bu_mouse::family::{self, Change, Driver, Extra, Family, Kind, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::os::{HidInfo, HidTransfer};
use bu_mouse::wallhack as w;
use bu_mouse::Error;
use std::cell::RefCell;
use std::rc::Rc;

/// A 64-byte report (id 4 first) from its non-zero bytes (index counted WITH the id).
fn rep(bytes: &[(usize, u8)]) -> Vec<u8> {
    let mut v = vec![0u8; 64];
    v[0] = 4;
    for (i, b) in bytes {
        v[*i] = *b;
    }
    v
}

fn cfg(pid: u16) -> HidInfo {
    hid_info("wh-cfg", 0x3879, pid, 0xFF1C, 0x92, (64, 64, 0))
}

// ---------------------------------------------------------------- encode

#[test]
fn reads_are_openmouses_bytes() {
    // battery: wallhackBuildSimple(0xBA) = [0, 0, 0xBA]
    assert_eq!(w::wire(&w::simple(0xBA), 64), rep(&[(3, 0xBA)]));
    // wallhackBuildRead(address, count) = [0, 0, 0xA4, count, addrLo, addrHi, 0]
    // active DPI stage (12), wired polling rank (11), lift-off (110 = 0x6E): one byte each
    assert_eq!(w::wire(&w::read_request(12, 1), 64), rep(&[(3, 0xA4), (4, 1), (5, 12)]));
    assert_eq!(w::wire(&w::read_request(11, 1), 64), rep(&[(3, 0xA4), (4, 1), (5, 11)]));
    assert_eq!(w::wire(&w::read_request(110, 1), 64), rep(&[(3, 0xA4), (4, 1), (5, 0x6E)]));
    // the DPI block: 9 bytes at 77 = 0x4D
    assert_eq!(w::wire(&w::read_request(77, 9), 64), rep(&[(3, 0xA4), (4, 9), (5, 0x4D)]));
    // an address above 255: little-endian (0x0280 = 640, a curve table address)
    assert_eq!(w::wire(&w::read_request(640, 10), 64), rep(&[(3, 0xA4), (4, 10), (5, 0x80), (6, 0x02)]));
}

#[test]
fn writes_are_openmouses_bytes() {
    // setDpi(1600 = 0x0640): wallhackBuildWrite(77, [1, 0, 0x40, 0x06, 0x90, 1, 0xFF, 0xFF, 0]) =
    // [0, 0, 0xA5, 9, 0x4D, 0x00, 0, 1, 0, 0x40, 0x06, 0x90, 1, 0xFF, 0xFF, 0]
    assert_eq!(
        w::wire(&w::set_dpi_request(1600), 64),
        rep(&[(3, 0xA5), (4, 9), (5, 0x4D), (8, 1), (10, 0x40), (11, 0x06), (12, 0x90), (13, 1), (14, 0xFF), (15, 0xFF)])
    );
    // 26000 = 0x6590, 50 = 0x0032
    assert_eq!(w::set_dpi_request(26000)[9..11], [0x90, 0x65]);
    assert_eq!(w::set_dpi_request(50)[9..11], [0x32, 0x00]);
    // setPollingRate(4000): rank 9 → writeByte(11, 9) then writeByte(10, 9): [0, 0, 0xA5, 1, addrLo, 0, 0, 9]
    assert_eq!(w::wire(&w::write_request(11, &[9]), 64), rep(&[(3, 0xA5), (4, 1), (5, 11), (8, 9)]));
    assert_eq!(w::wire(&w::write_request(10, &[9]), 64), rep(&[(3, 0xA5), (4, 1), (5, 10), (8, 9)]));
    // setLiftOffDistance(Medium → 1) at 110
    assert_eq!(w::wire(&w::write_request(110, &[1]), 64), rep(&[(3, 0xA5), (4, 1), (5, 0x6E), (8, 1)]));
    assert_eq!(w::wire(&w::simple(4), 70).len(), 70, "padded to the collection's report size");
}

#[test]
fn polling_ranks_and_lift_off_codes_are_openmouses_tables() {
    // WALLHACK_POLLING_BY_RANK
    let want = [(0u8, 125u32), (1, 250), (2, 500), (3, 1000), (4, 1500), (5, 2000), (6, 2500), (7, 3000), (8, 3500), (9, 4000), (10, 4500), (11, 5000), (12, 5500), (13, 6000), (14, 6500), (15, 7000), (16, 7500), (17, 8000)];
    for (rank, hz) in want {
        assert_eq!((w::rank_to_hz(rank), w::hz_to_rank(hz)), (Some(hz), Some(rank)), "{rank}");
    }
    assert_eq!((w::rank_to_hz(18), w::hz_to_rank(3200), w::hz_to_rank(0)), (None, None, None));
    assert_eq!(w::rates(), want.iter().map(|r| r.1).collect::<Vec<_>>());
    // WALLHACK_LOD_MM_BY_CODE: 0 → 0.7 mm, 1 → 1 mm, 2 → 2 mm
    assert_eq!((w::lift_off_tenths(0), w::lift_off_tenths(1), w::lift_off_tenths(2), w::lift_off_tenths(3)), (Some(7), Some(10), Some(20), None));
    assert_eq!((w::lift_off_code(7), w::lift_off_code(10), w::lift_off_code(20), w::lift_off_code(15)), (Some(0), Some(1), Some(2), None));
}

// ---------------------------------------------------------------- decode

/// A function-area answer the way OpenMouse reads it (its bytes, no id): [0, 0, 0xA4, n, addrLo, addrHi, 0, data...].
fn area_answer(addr: u16, data: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 63];
    v[2] = 0xA4;
    v[3] = data.len() as u8;
    v[4] = addr as u8;
    v[5] = (addr >> 8) as u8;
    v[7..7 + data.len()].copy_from_slice(data);
    v
}

#[test]
fn answers_decode_like_openmouse() {
    // wallhackReadByte: byte 7
    assert_eq!(w::read_byte(&area_answer(11, &[9])), Some(9));
    assert_eq!(w::read_byte(&[0u8; 7]), None);
    // wallhackReadDpi: bytes 9-10 little-endian of the 9-byte block [enabled, 0, lo, hi, 0x90, 1, 0xFF, 0xFF, 0]
    let block = area_answer(77, &[1, 0, 0x40, 0x06, 0x90, 1, 0xFF, 0xFF, 0]);
    assert_eq!(w::read_dpi(&block), Some(1600));
    assert_eq!(w::read_dpi(&block[..10]), None);
    assert_eq!(w::block_bytes(&block), Some([1, 0, 0x40, 0x06, 0x90, 1, 0xFF, 0xFF, 0]));
    // battery: percent at byte 7, charging (== 1) at byte 8
    let mut b = vec![0u8; 63];
    b[2] = 0xBA;
    b[7] = 64;
    b[8] = 1;
    assert_eq!(w::decode_battery(&b), Some((Some(64), true)));
    b[7] = 200;
    b[8] = 0;
    assert_eq!(w::decode_battery(&b), Some((None, false)), "over 100 → unknown");
    assert_eq!(w::decode_battery(&b[..8]), None);
}

// ---------------------------------------------------------------- allow-list + table

#[test]
fn only_reads_pass_the_allow_list() {
    for p in [w::simple(0xBA), w::read_request(12, 1), w::read_request(77, 9), w::read_request(640, 16)] {
        assert!(w::read_request_allowed(&w::wire(&p, 64)), "{:02x?}", &p[..7]);
        assert!(w::read_request_allowed(&w::wire(&p, 70)), "zero padding");
    }
    // writes, the keys / macro tables, pairing, clear pairing, factory reset, fast begin/end, anything else: refused
    for p in [
        w::set_dpi_request(1600),
        w::write_request(11, &[9]),
        w::simple(0xA8),
        w::simple(0xAD),
        w::simple(0xAE),
        w::simple(0xAF),
        w::simple(0xAB),
        w::simple(0xA1),
        w::simple(0xA2),
        w::simple(0xBC),
        w::read_request(0, 17),
        w::read_request(0, 0),
    ] {
        assert!(!w::read_request_allowed(&w::wire(&p, 64)), "{:02x?}", &p[..8]);
    }
    let mut junk = w::wire(&w::simple(0xBA), 64);
    junk[20] = 1;
    assert!(!w::read_request_allowed(&junk));
    let mut other = w::wire(&w::simple(0xBA), 64);
    other[0] = 0;
    assert!(!w::read_request_allowed(&other));
    assert!(!w::read_request_allowed(&[4, 0, 0, 0xBA]));
}

#[test]
fn the_table_is_openmouses_supported_list() {
    // supported-devices-data.ts: WALLHACK M-001 pids [0x1110, 0x807]
    for pid in [0x1110u16, 0x0807] {
        let c = cfg(pid);
        let m = w::matches(&[&c], 0x3879, pid).unwrap();
        assert_eq!((m.family, m.model, m.wireless, m.variant), (Family::Wallhack, Some("WALLHACK M-001"), None, pid));
        assert_eq!(m.config.path, c.path);
        assert_eq!(family::find(std::slice::from_ref(&c), 0x3879, pid).map(|m| m.family), Some(Family::Wallhack));
    }
    // the keyboard (K-001, 0x0806 under 0x3879 and 0x1caa) is not a mouse; another vendor; an unknown id
    assert!(w::matches(&[&hid_info("kb", 0x3879, 0x0806, 0xFFA0, 1, (64, 64, 0))], 0x3879, 0x0806).is_none());
    assert!(w::matches(&[&hid_info("kb", 0x1CAA, 0x0806, 0xFFA0, 1, (64, 64, 0))], 0x1CAA, 0x0806).is_none());
    assert!(w::matches(&[&cfg(0x1110)], 0x3880, 0x1110).is_none());
    assert!(w::matches(&[&cfg(0x1111)], 0x3879, 0x1111).is_none());
    // the command collection: usage page 0xFF1C with 63-byte reports (any usage); the keyboard's page and the mouse don't fit
    assert!(w::matches(&[&hid_info("x", 0x3879, 0x1110, 0xFF1C, 0x01, (64, 64, 0))], 0x3879, 0x1110).is_some(), "any usage");
    for (page, lens) in [(0xFFA0u16, (64u16, 64u16)), (0x01, (8, 0)), (0xFF1C, (9, 9)), (0xFF1C, (64, 0))] {
        let h = hid_info("x", 0x3879, 0x1110, page, 0x92, (lens.0, lens.1, 0));
        assert!(w::matches(&[&h], 0x3879, 0x1110).is_none(), "{page:04x} {lens:?}");
    }
    let mouse = hid_info("mouse", 0x3879, 0x1110, 0x01, 0x02, (8, 0, 0));
    let c = cfg(0x1110);
    assert_eq!(w::matches(&[&mouse, &c], 0x3879, 0x1110).map(|m| m.config.path), Some(c.path.clone()));
}

#[test]
fn caps_are_what_openmouse_offers() {
    let c = w::caps();
    let d = c.dpi.unwrap();
    assert_eq!((d.min(), d.max()), (50, 26000));
    assert!(d.allows(26000) && !d.allows(26050) && !d.allows(60));
    assert_eq!(c.polling.len(), 18);
    assert_eq!((c.polling[0], c.polling[3], c.polling[17]), (125, 1000, 8000));
    assert_eq!(c.lift_off, vec![7, 10, 20]);
    assert!(c.battery);
}

// ---------------------------------------------------------------- the driver against a scripted mouse

struct Fake {
    /// the function area
    area: Vec<u8>,
    battery: Option<(u8, u8)>,
    silent: bool,
    /// ignore writes
    ignore: bool,
    /// area reads at this address get no answer
    mute_addr: Option<u8>,
    /// area reads are answered with the data of another address (the echoed address is wrong)
    wrong_echo: bool,
    /// every request (63 bytes), in order
    seen: Vec<Vec<u8>>,
}

fn fake() -> Rc<RefCell<Fake>> {
    let mut area = vec![0u8; 128];
    area[10] = 3; // wireless 1000 Hz
    area[11] = 3; // wired 1000 Hz
    area[12] = 0; // stage 1
    area[77..86].copy_from_slice(&[1, 0, 0x20, 0x03, 0x90, 1, 0xFF, 0xFF, 0]); // 800 DPI
    area[110] = 1; // 1 mm
    // Order 110: keyDebounceTime 109 = 4 ms, angleSnapEnable 111 off, rippleControlEnable 112 on, motionSyncEnable 113 on
    area[109] = 4;
    area[112] = 1;
    area[113] = 1;
    Rc::new(RefCell::new(Fake { area, battery: Some((77, 0)), silent: false, ignore: false, mute_addr: None, wrong_echo: false, seen: Vec::new() }))
}

fn port(st: &Rc<RefCell<Fake>>) -> ScriptPort {
    let st = st.clone();
    ScriptPort::new(cfg(0x1110), move |out, how| {
        assert_eq!(out.len(), 64);
        assert_eq!(out[0], 4);
        let q = out[1..].to_vec();
        let mut s = st.borrow_mut();
        s.seen.push(q.clone());
        match (q[2], how) {
            (0xA5, HidTransfer::OutputOnly) => {
                let (addr, n) = (q[4] as usize | (q[5] as usize) << 8, q[3] as usize);
                if !s.ignore {
                    s.area[addr..addr + n].copy_from_slice(&q[7..7 + n]);
                }
                Ok(Vec::new())
            }
            (cmd, HidTransfer::OutputThenInput { echo, reply_len, .. }) => {
                assert_eq!((*echo, *reply_len), (Some((3, cmd)), 64), "the answer must echo the command at byte 2 (id byte first)");
                if s.silent {
                    return Err(Error::BadAnswer("no answer in time".into()));
                }
                let mut a = vec![0u8; 63];
                a[2] = cmd;
                match cmd {
                    0xBA => match s.battery {
                        Some((p, c)) => {
                            a[7] = p;
                            a[8] = c;
                        }
                        None => return Err(Error::BadAnswer("no answer in time".into())),
                    },
                    0xA4 => {
                        let (addr, n) = (q[4] as usize | (q[5] as usize) << 8, q[3] as usize);
                        if s.mute_addr == Some(q[4]) {
                            return Err(Error::BadAnswer("no answer in time".into()));
                        }
                        a[3] = q[3];
                        a[4] = q[4];
                        a[5] = q[5];
                        if s.wrong_echo {
                            a[4] = q[4].wrapping_add(1);
                        }
                        a[7..7 + n].copy_from_slice(&s.area[addr..addr + n]);
                    }
                    c => panic!("unexpected command {c:#04x}"),
                }
                let mut v = vec![4u8];
                v.extend(a);
                Ok(v)
            }
            (c, h) => panic!("unexpected {c:#04x} / {h:?}"),
        }
    })
}

fn matched() -> bu_mouse::family::Matched {
    w::matches(&[&cfg(0x1110)], 0x3879, 0x1110).unwrap()
}

/// (command, address) per request
fn cmds(st: &Rc<RefCell<Fake>>) -> Vec<(u8, u8)> {
    st.borrow().seen.iter().map(|q| (q[2], if q[2] == 0xBA { 0 } else { q[4] })).collect()
}

#[test]
fn reading_asks_battery_stage_dpi_polling_and_lift_off() {
    let st = fake();
    let mut p = port(&st);
    let r = w::Wallhack.read(&matched(), &mut p, ReadOptions::default()).unwrap();
    assert!(p.sent.iter().all(|s| w::read_request_allowed(&s.0)), "read requests only");
    // (Order 110: then Motion Sync 113, angle snapping 111, ripple 112, debounce 109 - readStatus' order)
    assert_eq!(cmds(&st), vec![(0xBA, 0), (0xA4, 12), (0xA4, 77), (0xA4, 11), (0xA4, 110), (0xA4, 113), (0xA4, 111), (0xA4, 112), (0xA4, 109)]);
    assert_eq!((r.online, r.dpi, r.polling_hz, r.lift_off, r.battery_percent, r.charging), (true, Some(800), Some(1000), Some(10), Some(77), Some(false)));
    assert_eq!(r.caps, w::caps());
    // on another DPI stage the first record is not "the DPI": unknown, and the block is not even read
    let st = fake();
    st.borrow_mut().area[12] = 2;
    let r = w::Wallhack.read(&matched(), &mut port(&st), ReadOptions::default()).unwrap();
    assert_eq!(r.dpi, None);
    assert!(!cmds(&st).contains(&(0xA4, 77)));
    // a battery that does not answer but the area does
    let st = fake();
    st.borrow_mut().battery = None;
    let r = w::Wallhack.read(&matched(), &mut port(&st), ReadOptions::default()).unwrap();
    assert_eq!((r.online, r.battery_percent, r.dpi), (true, None, Some(800)));
    // 0.7 mm and a 7000 Hz rank
    st.borrow_mut().area[110] = 0;
    st.borrow_mut().area[11] = 15;
    let r = w::Wallhack.read(&matched(), &mut port(&st), ReadOptions::default()).unwrap();
    assert_eq!((r.lift_off, r.polling_hz), (Some(7), Some(7000)));
}

#[test]
fn a_mouse_that_does_not_answer_is_offline() {
    let st = fake();
    st.borrow_mut().silent = true;
    let r = w::Wallhack.read(&matched(), &mut port(&st), ReadOptions::default()).unwrap();
    assert_eq!((r.online, r.dpi, r.polling_hz, r.battery_percent), (false, None, None, None));
    assert_eq!(cmds(&st), vec![(0xBA, 0), (0xA4, 12)], "two questions, then it gives up");
    let mut gone = ScriptPort::new(cfg(0x1110), |_, _| Err(Error::MouseGone("w".into())));
    assert!(matches!(w::Wallhack.read(&matched(), &mut gone, ReadOptions::default()), Err(Error::MouseGone(_))));
}

#[test]
fn a_dpi_change_writes_the_first_stage_record_and_reads_it_back() {
    let st = fake();
    let mut p = port(&st);
    let c = w::Wallhack.write(&matched(), &mut p, Setting::Dpi(1600)).unwrap();
    assert_eq!((c.now, c.before), (1600, Some(800)));
    // active stage, the record, the write, the record again
    assert_eq!(cmds(&st), vec![(0xA4, 12), (0xA4, 77), (0xA5, 77), (0xA4, 77)]);
    assert_eq!(p.sent[2], (w::wire(&w::set_dpi_request(1600), 64), HidTransfer::OutputOnly));
    assert!(!w::read_request_allowed(&p.sent[2].0));
    assert_eq!(st.borrow().area[77..86], [1, 0, 0x40, 0x06, 0x90, 1, 0xFF, 0xFF, 0]);
}

#[test]
fn a_dpi_stage_with_another_colour_or_stage_is_not_overwritten() {
    // the stage's LED colour bytes are not OpenMouse's defaults: its write would replace them
    let st = fake();
    st.borrow_mut().area[82] = 0x10;
    let mut p = port(&st);
    let e = w::Wallhack.write(&matched(), &mut p, Setting::Dpi(1600)).unwrap_err();
    assert!(matches!(e, Error::BadAnswer(ref m) if m.contains("nothing was written")), "{e:?}");
    assert!(st.borrow().seen.iter().all(|q| q[2] != 0xA5));
    // the enabled flag is 0
    let st = fake();
    st.borrow_mut().area[77] = 0;
    assert!(w::Wallhack.write(&matched(), &mut port(&st), Setting::Dpi(1600)).is_err());
    assert!(st.borrow().seen.iter().all(|q| q[2] != 0xA5));
    // another active stage: refused
    let st = fake();
    st.borrow_mut().area[12] = 1;
    let e = w::Wallhack.write(&matched(), &mut port(&st), Setting::Dpi(1600)).unwrap_err();
    assert!(matches!(e, Error::BadAnswer(ref m) if m.contains("another DPI stage")), "{e:?}");
    assert_eq!(cmds(&st), vec![(0xA4, 12)]);
}

#[test]
fn a_polling_change_writes_both_rank_bytes_and_reads_both_back() {
    let st = fake();
    let mut p = port(&st);
    let c = w::Wallhack.write(&matched(), &mut p, Setting::Polling(4000)).unwrap();
    assert_eq!((c.now, c.before), (4000, Some(1000)));
    assert_eq!(cmds(&st), vec![(0xA4, 11), (0xA4, 10), (0xA5, 11), (0xA5, 10), (0xA4, 11), (0xA4, 10)]);
    assert_eq!(p.sent[2].0, w::wire(&w::write_request(11, &[9]), 64));
    assert_eq!(p.sent[3].0, w::wire(&w::write_request(10, &[9]), 64));
    assert_eq!((st.borrow().area[10], st.borrow().area[11]), (9, 9));
}

#[test]
fn a_lift_off_change_writes_the_code_and_reads_it_back() {
    let st = fake();
    let c = w::Wallhack.write(&matched(), &mut port(&st), Setting::LiftOff(20)).unwrap();
    assert_eq!((c.now, c.before), (20, Some(10)));
    assert_eq!(st.borrow().area[110], 2);
    assert_eq!(cmds(&st), vec![(0xA4, 110), (0xA5, 110), (0xA4, 110)]);
    let c = w::Wallhack.write(&matched(), &mut port(&st), Setting::LiftOff(7)).unwrap();
    assert_eq!((c.now, c.before), (7, Some(20)));
    assert_eq!(st.borrow().area[110], 0);
}

#[test]
fn a_change_the_mouse_does_not_keep_is_an_error() {
    let st = fake();
    st.borrow_mut().ignore = true;
    for s in [Setting::Dpi(1600), Setting::Polling(4000), Setting::LiftOff(20)] {
        let e = w::Wallhack.write(&matched(), &mut port(&st), s).unwrap_err();
        assert!(matches!(e, Error::BadAnswer(_)), "{s:?}: {e:?}");
    }
}

#[test]
fn values_the_mouse_cannot_store_are_refused_before_a_byte_leaves() {
    let st = fake();
    let mut p = port(&st);
    for s in [Setting::Dpi(49), Setting::Dpi(1625), Setting::Dpi(26050), Setting::Polling(3200), Setting::Polling(0), Setting::LiftOff(15), Setting::LiftOff(30)] {
        assert!(w::Wallhack.write(&matched(), &mut p, s).is_err(), "{s:?}");
    }
    assert!(p.sent.is_empty());
}

#[test]
fn a_stage_read_that_gets_no_answer_does_not_count_as_stage_one() {
    // the active-stage byte (12) times out: OpenMouse would treat that as 0; here nothing is written
    let st = fake();
    st.borrow_mut().mute_addr = Some(12);
    let mut p = port(&st);
    let e = w::Wallhack.write(&matched(), &mut p, Setting::Dpi(1600)).unwrap_err();
    assert!(matches!(e, Error::BadAnswer(ref m) if m.contains("another DPI stage")), "{e:?}");
    assert!(st.borrow().seen.iter().all(|q| q[2] != 0xA5), "nothing written");
    assert_eq!(st.borrow().area[77..86], [1, 0, 0x20, 0x03, 0x90, 1, 0xFF, 0xFF, 0]);
    assert!(p.sent.iter().all(|s| w::read_request_allowed(&s.0)));
}

#[test]
fn a_polling_change_with_different_wired_and_wireless_rates_is_refused() {
    // wired 1000 Hz (rank 3), wireless 4000 Hz (rank 9): writing both would lose the wireless rate
    let st = fake();
    st.borrow_mut().area[10] = 9;
    let mut p = port(&st);
    let e = w::Wallhack.write(&matched(), &mut p, Setting::Polling(2000)).unwrap_err();
    assert!(matches!(e, Error::BadAnswer(ref m) if m.contains("nothing was written")), "{e:?}");
    assert!(st.borrow().seen.iter().all(|q| q[2] != 0xA5), "nothing written");
    assert_eq!((st.borrow().area[10], st.borrow().area[11]), (9, 3));
    // the wireless byte does not answer: also refused
    let st = fake();
    st.borrow_mut().mute_addr = Some(10);
    assert!(w::Wallhack.write(&matched(), &mut port(&st), Setting::Polling(2000)).is_err());
    assert!(st.borrow().seen.iter().all(|q| q[2] != 0xA5));
}

#[test]
fn an_area_answer_for_another_address_is_not_taken() {
    // every area read is answered with a wrong echoed address: reads see nothing, writes refuse, nothing is written
    let st = fake();
    st.borrow_mut().wrong_echo = true;
    let r = w::Wallhack.read(&matched(), &mut port(&st), ReadOptions::default()).unwrap();
    assert_eq!((r.dpi, r.polling_hz, r.lift_off), (None, None, None));
    assert_eq!(r.battery_percent, Some(77), "the battery answer has no area echo");
    for s in [Setting::Dpi(1600), Setting::Polling(4000)] {
        assert!(w::Wallhack.write(&matched(), &mut port(&st), s).is_err(), "{s:?}");
    }
    // a lift-off write is sent (its before is unknown) but its read-back is not accepted
    assert!(w::Wallhack.write(&matched(), &mut port(&st), Setting::LiftOff(20)).is_err());
}

// ---------------------------------------------------------------- Order 110: debounce, Motion Sync, angle snapping, ripple

#[test]
fn the_extras_are_openmouses_function_area_bytes() {
    let st = fake();
    let mut p = port(&st);
    let r = w::Wallhack.read(&matched(), &mut p, ReadOptions::default()).unwrap();
    // readStatus: debounceMs = the raw byte 109; the switches = byte === 1
    let got: Vec<(Extra, Option<i32>)> = r.extras.iter().map(|e| (e.extra, e.value)).collect();
    assert_eq!(got, vec![(Extra::Debounce, Some(4)), (Extra::MotionSync, Some(1)), (Extra::AngleSnap, Some(0)), (Extra::RippleControl, Some(1))]);
    // setDebounceTime: 0-20 ms
    assert_eq!(r.extras[0].kind, Kind::Range { min: 0, max: 20, step: 1, unit: "ms" });
    // writeVerifiedByte(address, v) = wallhackBuildWrite(address, [v]) = [0, 0, 0xA5, 1, address, 0, 0, v], then readByte(address)
    for (c, addr, v) in [(Change::Extra(Extra::Debounce, 20), 0x6Du8, 20u8), (Change::Extra(Extra::MotionSync, 0), 0x71, 0), (Change::Extra(Extra::AngleSnap, 1), 0x6F, 1), (Change::Extra(Extra::RippleControl, 0), 0x70, 0)] {
        p.sent.clear();
        w::Wallhack.change(&matched(), &mut p, c).unwrap();
        assert_eq!(p.sent.iter().map(|s| s.0.clone()).collect::<Vec<_>>(), vec![rep(&[(3, 0xA5), (4, 1), (5, addr), (8, v)]), rep(&[(3, 0xA4), (4, 1), (5, addr)])], "{c:?}");
        assert_eq!(p.sent[0].1, HidTransfer::OutputOnly);
        assert!(!w::read_request_allowed(&p.sent[0].0));
    }
    let r = w::Wallhack.read(&matched(), &mut port(&st), ReadOptions::default()).unwrap();
    assert_eq!(r.extras.iter().map(|e| e.value).collect::<Vec<_>>(), vec![Some(20), Some(0), Some(1), Some(0)]);
    // a mouse that keeps its value
    st.borrow_mut().ignore = true;
    assert!(w::Wallhack.change(&matched(), &mut port(&st), Change::Extra(Extra::MotionSync, 1)).is_err());
    // refused before a byte leaves
    let mut p = port(&st);
    for c in [Change::Extra(Extra::Debounce, 21), Change::Extra(Extra::MotionSync, 2), Change::Extra(Extra::Turbo, 1)] {
        assert!(w::Wallhack.change(&matched(), &mut p, c).is_err(), "{c:?}");
    }
    assert!(p.sent.is_empty());
    // an area byte that is not answered: no row
    let st = fake();
    st.borrow_mut().mute_addr = Some(112);
    let r = w::Wallhack.read(&matched(), &mut port(&st), ReadOptions::default()).unwrap();
    assert!(r.extras.iter().all(|e| e.extra != Extra::RippleControl) && r.extras.len() == 3);
}

//! Order 105 check, VGN Dragonfly F2 Master+: our packets = the bytes OpenMouse sends for the same setting (encode), and
//! answers shaped the way OpenMouse reads them decode to the same values (decode). Expected bytes are worked out by hand from
//! OpenMouse (@openmouse/protocol 0.40.1: `dist/vgn/index.js`, `dist/drivers/vgn/hid.js`).
//!
//! OpenMouse sends `sendReport(8, 16 bytes)`; byte 15 is the checksum `(0x55 - (8 + sum of bytes 0..14)) & 0xFF`. Windows puts
//! the report id 8 in front (17 bytes): OpenMouse's byte i is our byte i + 1.

use bu_mouse::family::{self, Driver, Family, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::os::{HidInfo, HidTransfer};
use bu_mouse::vgn as v;
use bu_mouse::Error;
use std::cell::RefCell;
use std::rc::Rc;

/// A 16-byte payload from its non-zero bytes (index, value).
fn pl(bytes: &[(usize, u8)]) -> v::Payload {
    let mut p = [0u8; 16];
    for (i, b) in bytes {
        p[*i] = *b;
    }
    p
}

fn cfg(pid: u16) -> HidInfo {
    hid_info("vgn-cfg", 0x3554, pid, 0xFF02, 1, (17, 17, 0))
}

/// The checksum the way the fake mouse computes it for its answers (same formula, written out).
fn sealed(mut p: v::Payload) -> v::Payload {
    let sum: u32 = 8 + p[..15].iter().map(|b| *b as u32).sum::<u32>();
    p[15] = ((0x55 + 256 - (sum & 0xFF)) & 0xFF) as u8;
    p
}

// ---------------------------------------------------------------- encode

#[test]
fn reads_are_openmouses_payloads() {
    // battery: [04, 0...]: sum = 8 + 4 = 12; 0x55 − 12 = 85 − 12 = 73 = 0x49
    assert_eq!(v::simple(0x04), pl(&[(0, 4), (15, 0x49)]));
    // readProfile's chunks: [08, 0, addrHi, addrLo, length, ...]; crc = 85 − (8 + 8 + addrHi + addrLo + length)
    // (0, 10): 85 − 26 = 59 = 0x3B · (10, 10): 85 − 36 = 49 = 0x31 · (20, 10): 85 − 46 = 39 = 0x27
    // (30, 10): 85 − 56 = 29 = 0x1D · (40, 4): 85 − 60 = 25 = 0x19
    for ((addr, len), crc) in v::PROFILE_CHUNKS.iter().zip([0x3B, 0x31, 0x27, 0x1D, 0x19]) {
        assert_eq!(v::read_payload(*addr, *len), pl(&[(0, 8), (2, (addr >> 8) as u8), (3, *addr as u8), (4, *len), (15, crc)]), "{addr:#x}");
    }
    assert_eq!(v::PROFILE_CHUNKS, [(0, 10), (10, 10), (20, 10), (30, 10), (40, 4)]);
    // writeScalar's read-back read(0, 2): 85 − (8 + 8 + 2) = 67 = 0x43
    assert_eq!(v::read_payload(0, 2), pl(&[(0, 8), (4, 2), (15, 0x43)]));
    // an address above 0xFF: read(0x0123, 5): 85 − (8 + 8 + 1 + 0x23 + 5) = 85 − 57 = 28 = 0x1C
    assert_eq!(v::read_payload(0x0123, 5), pl(&[(0, 8), (2, 1), (3, 0x23), (4, 5), (15, 0x1C)]));
}

#[test]
fn dpi_records_are_openmouses_encoding() {
    // vgnEncodeDpi: enc = dpi / 50 − 1; low = enc & 0xFF; high = (enc >> 8) & 3; flags = (high << 2) | (high << 6);
    // record [low, low, flags, 0x55 − low − low − flags]
    // 800: enc 15 = 0x0F → [0F, 0F, 00, 85 − 30 = 55 = 0x37]
    assert_eq!(v::encode_dpi(800), Some([0x0F, 0x0F, 0x00, 0x37]));
    // 1600: enc 31 = 0x1F → 85 − 62 = 23 = 0x17
    assert_eq!(v::encode_dpi(1600), Some([0x1F, 0x1F, 0x00, 0x17]));
    // 50: enc 0 → [00, 00, 00, 0x55]
    assert_eq!(v::encode_dpi(50), Some([0x00, 0x00, 0x00, 0x55]));
    // 26000: enc 519 = 0x207 → low 0x07, high 2 → flags 0x08 | 0x80 = 0x88; 85 − 7 − 7 − 136 = −65 → 191 = 0xBF
    assert_eq!(v::encode_dpi(26000), Some([0x07, 0x07, 0x88, 0xBF]));
    // 12850: enc 256 = 0x100 → low 0, high 1 → flags 0x44; 85 − 68 = 17 = 0x11
    assert_eq!(v::encode_dpi(12850), Some([0x00, 0x00, 0x44, 0x11]));
    for bad in [0, 49, 75, 26050, 26001] {
        assert_eq!(v::encode_dpi(bad), None, "{bad}");
    }
    // the write is the 4-byte record at 0x0C + 4 × stage: stage 1 → 0x10: [07, 0, 0x00, 0x10, 04, 1F 1F 00 17]
    // crc: 85 − (8 + 7 + 0x10 + 4 + 0x1F + 0x1F + 0x17) = 85 − 120 = −35 → 221 = 0xDD
    assert_eq!(v::write_payload(0x10, &[0x1F, 0x1F, 0x00, 0x17]), pl(&[(0, 7), (3, 0x10), (4, 4), (5, 0x1F), (6, 0x1F), (8, 0x17), (15, 0xDD)]));
}

#[test]
fn polling_writes_are_openmouses_scalars() {
    // vgnEncodePollingRate: 125 → 8, 250 → 4, 500 → 2, 1000 → 1, 2000 → 16, 4000 → 32, 8000 → 64
    for (hz, raw) in [(125, 8), (250, 4), (500, 2), (1000, 1), (2000, 16), (4000, 32), (8000, 64)] {
        assert_eq!(v::encode_polling(hz), Some(raw), "{hz}");
        assert_eq!(v::decode_polling(raw), Some(hz), "{raw}");
    }
    assert_eq!((v::encode_polling(3000), v::decode_polling(3)), (None, None));
    // writeScalar(0, 16): write(0, [0x10, 0x55 − 0x10 = 0x45]) = [07, 0, 0, 0, 02, 10, 45]
    // crc: 85 − (8 + 7 + 2 + 0x10 + 0x45) = 85 − 102 = −17 → 239 = 0xEF
    assert_eq!(v::write_scalar_payload(0, 16), pl(&[(0, 7), (4, 2), (5, 0x10), (6, 0x45), (15, 0xEF)]));
}

#[test]
fn windows_reports_carry_the_id() {
    let w = v::wire(&v::simple(4), 17);
    assert_eq!((w.len(), w[0], w[1], w[16]), (17, 8, 4, 0x49));
    assert_eq!(v::wire(&v::simple(4), 20).len(), 20);
}

// ---------------------------------------------------------------- decode

/// The flash image of a mouse at 3 stages (400 / 800 / 1600), stage 1 active, 1000 Hz.
fn flash() -> Vec<u8> {
    let mut f = vec![0u8; 0x2C];
    // scalars [value, 0x55 − value]: polling raw 1 → 54; stage count 3 → 52; active stage 1 → 54
    f[0..2].copy_from_slice(&[0x01, 0x54]);
    f[2..4].copy_from_slice(&[0x03, 0x52]);
    f[4..6].copy_from_slice(&[0x01, 0x54]);
    // stages at 0x0C: 400 = enc 7 → [07 07 00 47], 800 = [0F 0F 00 37], 1600 = [1F 1F 00 17]
    f[0x0C..0x10].copy_from_slice(&[0x07, 0x07, 0x00, 0x47]);
    f[0x10..0x14].copy_from_slice(&[0x0F, 0x0F, 0x00, 0x37]);
    f[0x14..0x18].copy_from_slice(&[0x1F, 0x1F, 0x00, 0x17]);
    f
}

#[test]
fn the_profile_decodes_like_openmouse() {
    let p = v::decode_profile(&flash()).unwrap();
    assert_eq!((p.stage_count, p.active_stage, p.dpi, p.polling_hz), (3, 1, Some(800), Some(1000)));
    assert_eq!(p.stage_bytes, [0x0F, 0x0F, 0x00, 0x37]);
    // a stage count above 8 clamps to 8, one below 1 to 1; the active stage to count − 1
    let mut f = flash();
    f[2..4].copy_from_slice(&[0x09, 0x4C]);
    f[4..6].copy_from_slice(&[0x07, 0x4E]);
    let p = v::decode_profile(&f).unwrap();
    assert_eq!((p.stage_count, p.active_stage), (8, 7));
    // a scalar whose parity byte is wrong is no value: count 1, stage 0 (OpenMouse's defaults)
    let mut f = flash();
    f[2] = 0x04;
    f[4] = 0x02;
    let p = v::decode_profile(&f).unwrap();
    assert_eq!((p.stage_count, p.active_stage, p.dpi), (1, 0, Some(400)));
    // a stage record with a wrong checksum / differing low bytes is unreadable (OpenMouse invents 800; this port says unknown)
    let mut f = flash();
    f[0x11] = 0x0E;
    assert_eq!(v::decode_profile(&f).unwrap().dpi, None);
    let mut f = flash();
    f[0x13] = 0x00;
    assert_eq!(v::decode_profile(&f).unwrap().dpi, None);
    // 26000 and the high bits
    let mut f = flash();
    f[0x10..0x14].copy_from_slice(&[0x07, 0x07, 0x88, 0xBF]);
    assert_eq!(v::decode_profile(&f).unwrap().dpi, Some(26000));
    // polling: an unknown code is unknown
    let mut f = flash();
    f[0..2].copy_from_slice(&[0x03, 0x52]);
    assert_eq!(v::decode_profile(&f).unwrap().polling_hz, None);
    assert!(v::decode_profile(&flash()[..40]).is_none());
}

#[test]
fn answers_decode_like_openmouse() {
    // battery: [04, 0, -, -, -, percent, charging, mvHi, mvLo]
    let b = sealed(pl(&[(0, 4), (5, 87), (6, 1), (7, 0x0B), (8, 0xB8)]));
    assert_eq!(v::parse_battery(&b), Some((87, true)));
    assert_eq!(v::parse_battery(&sealed(pl(&[(0, 4), (5, 101)]))), None, "over 100");
    assert_eq!(v::parse_battery(&sealed(pl(&[(0, 4), (1, 1), (5, 50)]))), None, "status 1");
    let mut bad = b;
    bad[15] ^= 1;
    assert_eq!(v::parse_battery(&bad), None, "bad checksum");
    // a read answer: [08, 0, addrHi, addrLo, length, data...]
    let r = sealed(pl(&[(0, 8), (3, 10), (4, 2), (5, 0xAA), (6, 0xBB)]));
    assert_eq!(v::parse_read(&r, 10, 2), Some(vec![0xAA, 0xBB]));
    assert_eq!(v::parse_read(&r, 11, 2), None, "another address");
    assert_eq!(v::parse_read(&r, 10, 3), None, "another length");
    assert!(v::write_ok(&sealed(pl(&[(0, 7)]))) && !v::write_ok(&sealed(pl(&[(0, 7), (1, 1)]))) && !v::write_ok(&sealed(pl(&[(0, 8)]))));
}

// ---------------------------------------------------------------- allow-list + table

#[test]
fn only_reads_pass_the_allow_list() {
    for p in [v::simple(4), v::read_payload(0, 10), v::read_payload(40, 4), v::read_payload(0, 2)] {
        assert!(v::read_request_allowed(&v::wire(&p, 17)), "{:02x?}", &p[..5]);
        assert!(v::read_request_allowed(&v::wire(&p, 20)), "zero padding");
    }
    for p in [v::write_payload(0x10, &[0x1F, 0x1F, 0, 0x17]), v::write_scalar_payload(0, 16), v::simple(0x12), v::simple(0x01), v::simple(0x03), v::read_payload(0, 11), v::read_payload(0, 0)] {
        assert!(!v::read_request_allowed(&v::wire(&p, 17)), "{:02x?}", &p[..5]);
    }
    let mut bad = v::wire(&v::read_payload(0, 10), 17);
    bad[16] ^= 1;
    assert!(!v::read_request_allowed(&bad));
    let mut other = v::wire(&v::simple(4), 17);
    other[0] = 0;
    assert!(!v::read_request_allowed(&other));
    assert!(!v::read_request_allowed(&[8, 4]));
}

#[test]
fn the_table_is_openmouses_supported_list() {
    // supported-devices-data.ts: VGN Dragonfly F2 Master+ pids [0xfb56, 0xfb57]
    for (pid, wireless) in [(0xFB56u16, true), (0xFB57, false)] {
        let c = cfg(pid);
        let m = v::matches(&[&c], 0x3554, pid).unwrap();
        assert_eq!((m.family, m.model, m.wireless, m.variant), (Family::Vgn, Some("VGN Dragonfly F2 Master+"), Some(wireless), pid));
        assert_eq!(family::find(std::slice::from_ref(&c), 0x3554, pid).map(|m| m.family), Some(Family::Vgn));
    }
    // VGN's other models are "pending" on the list; the Teevolution Terra Pro shares the vendor id
    for pid in [0xFB55u16, 0xFB58, 0xF520, 0xF523, 0xF5BB, 0xF522, 0x0000] {
        assert!(v::matches(&[&cfg(pid)], 0x3554, pid).is_none(), "{pid:04x}");
    }
    assert!(v::matches(&[&cfg(0xFB57)], 0x3555, 0xFB57).is_none());
    // the report collection: usage page 0xFF02 with a 16-byte report 8 both ways
    for (page, lens) in [(0xFF01u16, (17u16, 17u16, 0u16)), (0xFF02, (16, 16, 0)), (0xFF02, (17, 0, 0)), (0xFF02, (0, 17, 0)), (0x01, (8, 0, 0))] {
        let h = hid_info("x", 0x3554, 0xFB57, page, 1, lens);
        assert!(v::matches(&[&h], 0x3554, 0xFB57).is_none(), "{page:04x} {lens:?}");
    }
    // a collection with longer reports still carries the 16-byte report 8
    assert!(v::matches(&[&hid_info("x", 0x3554, 0xFB57, 0xFF02, 1, (65, 65, 0))], 0x3554, 0xFB57).is_some());
    assert!(v::matches(&[&hid_info("x", 0x3554, 0xFB57, 0xFF02, 1, (17, 64, 0))], 0x3554, 0xFB57).is_some());
    let mouse = hid_info("mouse", 0x3554, 0xFB57, 0x01, 0x02, (8, 0, 0));
    let c = cfg(0xFB57);
    assert_eq!(v::matches(&[&mouse, &c], 0x3554, 0xFB57).map(|m| m.config.path), Some(c.path.clone()));
}

#[test]
fn caps_are_what_openmouse_offers() {
    let c = v::caps();
    let d = c.dpi.unwrap();
    assert_eq!((d.min(), d.max()), (50, 26000));
    assert!(d.allows(26000) && !d.allows(26050) && !d.allows(60));
    assert_eq!(c.polling, vec![125, 250, 500, 1000, 2000, 4000, 8000]);
    assert!(c.lift_off.is_empty() && c.battery);
}

// ---------------------------------------------------------------- the driver against a scripted mouse

struct Fake {
    flash: Vec<u8>,
    silent: bool,
    /// refuse flash writes (status 1)
    refuse: bool,
    /// ignore flash writes (status 0, nothing stored)
    ignore: bool,
    /// every request (the 16 payload bytes), in order
    seen: Vec<Vec<u8>>,
}

fn fake() -> Rc<RefCell<Fake>> {
    Rc::new(RefCell::new(Fake { flash: flash(), silent: false, refuse: false, ignore: false, seen: Vec::new() }))
}

fn port(st: &Rc<RefCell<Fake>>, pid: u16) -> ScriptPort {
    let st = st.clone();
    ScriptPort::new(cfg(pid), move |out, how| {
        let HidTransfer::OutputThenInput { echo, reply_len, .. } = how else { panic!("VGN uses output + input reports, not {how:?}") };
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
            4 => {
                a[5] = 87;
                a[7] = 0x0B;
                a[8] = 0xB8;
            }
            8 => {
                let (addr, len) = (u16::from_be_bytes([q[2], q[3]]) as usize, q[4] as usize);
                a[2..5].copy_from_slice(&q[2..5]);
                a[5..5 + len].copy_from_slice(&s.flash[addr..addr + len]);
            }
            7 => {
                let (addr, len) = (u16::from_be_bytes([q[2], q[3]]) as usize, q[4] as usize);
                if s.refuse {
                    a[1] = 1;
                } else if !s.ignore {
                    s.flash[addr..addr + len].copy_from_slice(&q[5..5 + len]);
                }
                a[2..5].copy_from_slice(&q[2..5]);
            }
            c => panic!("unexpected command {c}"),
        }
        let mut v = vec![8u8];
        v.extend_from_slice(&sealed(a));
        Ok(v)
    })
}

fn matched(pid: u16) -> bu_mouse::family::Matched {
    v::matches(&[&cfg(pid)], 0x3554, pid).unwrap()
}

fn cmds(st: &Rc<RefCell<Fake>>) -> Vec<(u8, u8, u8)> {
    st.borrow().seen.iter().map(|q| (q[0], q[3], q[4])).collect()
}

const READ_PROFILE: [(u8, u8, u8); 5] = [(8, 0, 10), (8, 10, 10), (8, 20, 10), (8, 30, 10), (8, 40, 4)];

#[test]
fn reading_asks_the_battery_then_the_profile() {
    let st = fake();
    let mut p = port(&st, 0xFB57);
    let r = v::Vgn.read(&matched(0xFB57), &mut p, ReadOptions::default()).unwrap();
    assert!(p.sent.iter().all(|s| v::read_request_allowed(&s.0)), "read requests only");
    let mut want = vec![(4, 0, 0)];
    want.extend(READ_PROFILE);
    assert_eq!(cmds(&st), want);
    assert_eq!((r.online, r.dpi, r.polling_hz, r.battery_percent, r.charging, r.lift_off, r.wireless), (true, Some(800), Some(1000), Some(87), Some(false), None, Some(false)));
    assert_eq!(r.caps, v::caps());
    let r = v::Vgn.read(&matched(0xFB56), &mut port(&fake(), 0xFB56), ReadOptions::default()).unwrap();
    assert_eq!(r.wireless, Some(true));
}

#[test]
fn a_mouse_that_does_not_answer_is_offline() {
    let st = fake();
    st.borrow_mut().silent = true;
    let r = v::Vgn.read(&matched(0xFB56), &mut port(&st, 0xFB56), ReadOptions::default()).unwrap();
    assert_eq!((r.online, r.dpi, r.polling_hz, r.battery_percent), (false, None, None, None));
    assert_eq!(cmds(&st), vec![(4, 0, 0)], "one battery query, no loop");
    let mut gone = ScriptPort::new(cfg(0xFB57), |_, _| Err(Error::MouseGone("vgn".into())));
    assert!(matches!(v::Vgn.read(&matched(0xFB57), &mut gone, ReadOptions::default()), Err(Error::MouseGone(_))));
}

#[test]
fn a_dpi_change_writes_the_active_stage_and_reads_back() {
    let st = fake();
    let mut p = port(&st, 0xFB57);
    let c = v::Vgn.write(&matched(0xFB57), &mut p, Setting::Dpi(1600)).unwrap();
    assert_eq!((c.now, c.before), (1600, Some(800)));
    let mut want: Vec<(u8, u8, u8)> = READ_PROFILE.to_vec();
    want.push((7, 0x10, 4));
    want.extend(READ_PROFILE);
    assert_eq!(cmds(&st), want);
    assert_eq!(p.sent[5].0, v::wire(&v::write_payload(0x10, &[0x1F, 0x1F, 0x00, 0x17]), 17));
    assert!(!v::read_request_allowed(&p.sent[5].0));
    let f = st.borrow().flash.clone();
    assert_eq!(f[0x0C..0x18], [0x07, 0x07, 0x00, 0x47, 0x1F, 0x1F, 0x00, 0x17, 0x1F, 0x1F, 0x00, 0x17], "only stage 1 changed");
    // the highest value with its high bits
    let c = v::Vgn.write(&matched(0xFB57), &mut port(&st, 0xFB57), Setting::Dpi(26000)).unwrap();
    assert_eq!((c.now, c.before), (26000, Some(1600)));
    assert_eq!(st.borrow().flash[0x10..0x14], [0x07, 0x07, 0x88, 0xBF]);
}

#[test]
fn a_dpi_stage_with_unknown_bits_is_not_overwritten() {
    // flags 0x01 is not something OpenMouse's encoding produces: its write would wipe it
    let st = fake();
    st.borrow_mut().flash[0x12] = 0x01;
    st.borrow_mut().flash[0x13] = 0x36; // keep the record's own checksum valid: 0x55 − 0x0F − 0x0F − 0x01
    let mut p = port(&st, 0xFB57);
    let e = v::Vgn.write(&matched(0xFB57), &mut p, Setting::Dpi(1600)).unwrap_err();
    assert!(matches!(e, Error::BadAnswer(ref m) if m.contains("nothing was written")), "{e:?}");
    assert!(st.borrow().seen.iter().all(|q| q[0] != 7), "no flash write");
    // an unreadable record: also refused
    let st = fake();
    st.borrow_mut().flash[0x13] = 0x00;
    assert!(v::Vgn.write(&matched(0xFB57), &mut port(&st, 0xFB57), Setting::Dpi(1600)).is_err());
    assert!(st.borrow().seen.iter().all(|q| q[0] != 7));
}

#[test]
fn a_polling_change_writes_the_scalar_and_confirms_it() {
    let st = fake();
    let mut p = port(&st, 0xFB57);
    let c = v::Vgn.write(&matched(0xFB57), &mut p, Setting::Polling(2000)).unwrap();
    assert_eq!((c.now, c.before), (2000, Some(1000)));
    let mut want: Vec<(u8, u8, u8)> = READ_PROFILE.to_vec();
    want.push((7, 0, 2));
    want.push((8, 0, 2));
    want.extend(READ_PROFILE);
    assert_eq!(cmds(&st), want);
    assert_eq!(p.sent[5].0, v::wire(&v::write_scalar_payload(0, 16), 17));
    assert_eq!(st.borrow().flash[0..2], [0x10, 0x45]);
    assert_eq!(v::Vgn.write(&matched(0xFB57), &mut port(&st, 0xFB57), Setting::Polling(125)).unwrap().before, Some(2000));
    assert_eq!(st.borrow().flash[0..2], [0x08, 0x4D]);
}

#[test]
fn a_refused_or_ignored_write_is_an_error() {
    let st = fake();
    st.borrow_mut().refuse = true;
    assert!(v::Vgn.write(&matched(0xFB57), &mut port(&st, 0xFB57), Setting::Dpi(1600)).is_err());
    assert!(v::Vgn.write(&matched(0xFB57), &mut port(&st, 0xFB57), Setting::Polling(500)).is_err());
    let st = fake();
    st.borrow_mut().ignore = true;
    let e = v::Vgn.write(&matched(0xFB57), &mut port(&st, 0xFB57), Setting::Dpi(1600)).unwrap_err();
    assert!(matches!(e, Error::BadAnswer(ref m) if m.contains("kept")), "{e:?}");
    assert!(v::Vgn.write(&matched(0xFB57), &mut port(&st, 0xFB57), Setting::Polling(500)).is_err());
}

#[test]
fn values_the_mouse_cannot_store_are_refused_before_a_byte_leaves() {
    let st = fake();
    let mut p = port(&st, 0xFB57);
    for s in [Setting::Dpi(49), Setting::Dpi(75), Setting::Dpi(26050), Setting::Polling(3000), Setting::Polling(0), Setting::LiftOff(10)] {
        assert!(v::Vgn.write(&matched(0xFB57), &mut p, s).is_err(), "{s:?}");
    }
    assert!(p.sent.is_empty());
}

#[test]
fn the_active_stage_is_certain_only_when_count_and_stage_pass_parity_and_range() {
    assert!(v::decode_profile(&flash()).unwrap().stage_certain);
    // active stage 4 of a count of 2: OpenMouse clamps to stage 1 (the profile decodes), but the mouse is on stage 4
    let mut f = flash();
    f[2..4].copy_from_slice(&[0x02, 0x53]);
    f[4..6].copy_from_slice(&[0x04, 0x51]);
    let p = v::decode_profile(&f).unwrap();
    assert_eq!((p.stage_count, p.active_stage, p.stage_certain), (2, 1, false));
    // a wrong parity byte on either scalar
    let mut f = flash();
    f[3] = 0x00;
    assert!(!v::decode_profile(&f).unwrap().stage_certain);
    let mut f = flash();
    f[5] = 0x00;
    assert!(!v::decode_profile(&f).unwrap().stage_certain);
    // a count of 0 or above 8
    let mut f = flash();
    f[2..4].copy_from_slice(&[0x09, 0x4C]);
    f[4..6].copy_from_slice(&[0x01, 0x54]);
    assert!(!v::decode_profile(&f).unwrap().stage_certain);
    let mut f = flash();
    f[2..4].copy_from_slice(&[0x00, 0x55]);
    f[4..6].copy_from_slice(&[0x00, 0x55]);
    assert!(!v::decode_profile(&f).unwrap().stage_certain);
}

#[test]
fn a_dpi_change_is_refused_when_the_active_stage_is_not_certain() {
    for tweak in [
        // count 2, active 4: the clamped stage 1 would be rewritten while the mouse stays on stage 4
        (2usize, [0x02u8, 0x53], [0x04u8, 0x51]),
        // wrong parity on the active stage (OpenMouse would fall back to stage 0)
        (2, [0x03, 0x52], [0x01, 0x00]),
    ] {
        let st = fake();
        st.borrow_mut().flash[tweak.0..tweak.0 + 2].copy_from_slice(&tweak.1);
        st.borrow_mut().flash[4..6].copy_from_slice(&tweak.2);
        let mut p = port(&st, 0xFB57);
        let e = v::Vgn.write(&matched(0xFB57), &mut p, Setting::Dpi(1600)).unwrap_err();
        assert!(matches!(e, Error::BadAnswer(ref m) if m.contains("nothing was written")), "{e:?}");
        assert!(st.borrow().seen.iter().all(|q| q[0] != 7), "nothing written");
        assert!(p.sent.iter().all(|s| v::read_request_allowed(&s.0)));
    }
    // the polling change does not depend on the stage and still works
    let st = fake();
    st.borrow_mut().flash[2..4].copy_from_slice(&[0x02, 0x53]);
    st.borrow_mut().flash[4..6].copy_from_slice(&[0x04, 0x51]);
    assert!(v::Vgn.write(&matched(0xFB57), &mut port(&st, 0xFB57), Setting::Polling(500)).is_ok());
}

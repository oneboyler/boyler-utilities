//! Order 105 check, GearHub-V5 (Lingbao M5 Pro, Attack Shark R2): our requests = the bytes OpenMouse sends for the same
//! setting (encode), and answers shaped the way OpenMouse reads them decode to the same values (decode). The expected
//! bytes are worked out by hand from OpenMouse (@openmouse/protocol 0.40.1: `dist/gearhub/index.js`,
//! `dist/drivers/gearhub/hid.js`), never from our own encoder.
//!
//! OpenMouse's report is 64 bytes WITHOUT the report id (id 0 is unnumbered): Windows puts id 0 in front, so OpenMouse's
//! byte i is our byte i + 1. A command is 9 bytes + zero padding with the Bit7 checksum in byte 7: 255 - (sum of bytes 0..7).
//! Nothing here ran on a real mouse: the fake below is a model of what OpenMouse's code expects.

use bu_mouse::error::Error;
use bu_mouse::family::{Driver, Family, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::gearhub as g;
use bu_mouse::os::{HidInfo, HidTransfer};
use std::cell::RefCell;
use std::rc::Rc;

/// A Windows feature report: id 0, the 64 bytes given, zero-padded.
fn rep(bytes: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8];
    v.extend_from_slice(bytes);
    v.resize(65, 0);
    v
}

// the six frames of the relay and the three checksummed reads, checksums worked out by hand:
//   8F: 255 - 0x8f (143) = 112 = 0x70    D3: 255 - 0xd3 (211) = 44 = 0x2c    D4 00: 255 - 0xd4 (212) = 43 = 0x2b
fn select() -> Vec<u8> {
    rep(&[0xf6, 0x05])
}
fn poll() -> Vec<u8> {
    rep(&[0xf7])
}
fn notice() -> Vec<u8> {
    rep(&[0xfc])
}
fn get_id() -> Vec<u8> {
    rep(&[0x8f, 0, 0, 0, 0, 0, 0, 0x70])
}
fn get_opt0() -> Vec<u8> {
    rep(&[0xd3, 0, 0, 0, 0, 0, 0, 0x2c])
}
fn get_dpi() -> Vec<u8> {
    rep(&[0xd4, 0, 0, 0, 0, 0, 0, 0x2b])
}

// ---------------------------------------------------------------- encode

#[test]
fn the_bit7_checksum_is_openmouses() {
    // encodeCommand([0x8f]): padded to 9 bytes, byte 7 = 255 - 0x8f = 0x70
    assert_eq!(g::encode_command(&[0x8f]), vec![0x8f, 0, 0, 0, 0, 0, 0, 0x70, 0]);
    // [0xd3]: 255 - 211 = 44 = 0x2c
    assert_eq!(g::encode_command(&[0xd3]), vec![0xd3, 0, 0, 0, 0, 0, 0, 0x2c, 0]);
    // [0xd4, 0x00]: 255 - 212 = 43 = 0x2b
    assert_eq!(g::encode_command(&[0xd4, 0x00]), vec![0xd4, 0, 0, 0, 0, 0, 0, 0x2b, 0]);
    // a profile 1 read: [0xd4, 0x01] sums to 213 -> 42 = 0x2a
    assert_eq!(g::encode_command(&[0xd4, 0x01])[7], 0x2a);
    // [0x53] (OPTIONPARAM0 write): 255 - 83 = 172 = 0xac
    assert_eq!(g::encode_command(&[0x53])[7], 0xac);
    // [0x54, 0x00, 0x01, 0x03]: 84 + 0 + 1 + 3 = 88 -> 255 - 88 = 167 = 0xa7
    assert_eq!(g::encode_command(&[0x54, 0x00, 0x01, 0x03])[7], 0xa7);
    // the sum wraps at 256: [0xff, 0xff, 0x02] = 512 -> 0 -> 255
    assert_eq!(g::encode_command(&[0xff, 0xff, 0x02])[7], 0xff);
    // a 64-byte block stays 64 bytes, the checksum over its first 7 bytes only
    let mut block = vec![0u8; 64];
    block[0] = 0x53;
    block[9] = 1;
    block[52] = 1;
    block[9] = 0x77;
    let e = g::encode_command(&block);
    assert_eq!((e.len(), e[7], e[9]), (64, 0xac, 0x77));
}

#[test]
fn reports_are_id_0_plus_64_bytes() {
    assert_eq!(g::report(&[0xf7], 65), rep(&[0xf7]));
    // Windows wants the collection's FeatureReportByteLength: zero-padded behind
    assert_eq!(g::report(&[0xf7], 70).len(), 70);
    // rawSend keeps the first 64 bytes only
    assert_eq!(g::report(&[1u8; 80], 65), [vec![0u8], vec![1u8; 64]].concat());
    // the shortest report is 65
    assert_eq!(g::report(&[0xf7], 10).len(), 65);
}

#[test]
fn rate_codes_are_openmouses() {
    // REPORT_RATE_ENCODE
    for (hz, code) in [(8000, 129u8), (4000, 130), (2000, 132), (1000, 1), (500, 2), (250, 4), (125, 8)] {
        assert_eq!((g::rate_code(hz), g::rate_hz(code)), (Some(code), Some(hz)), "{hz}");
    }
    assert_eq!((g::rate_code(3000), g::rate_hz(0), g::rate_hz(3)), (None, None, None));
    assert_eq!(g::RATES, [125, 250, 500, 1000, 2000, 4000, 8000]);
}

#[test]
fn the_dpi_table_command_is_openmouses_writedpitable() {
    // three stages 800 / 1600 / 3200 (X = Y), colours red / green / blue, stage 2 (index 1) active:
    // [0] 0x54  [1] profile 0  [2] active 1  [3] count 3  [7] checksum 0xa7 (stamped by encodeCommand)
    // X at 8 + 2i little-endian: 800 = 0x0320 -> 20 03, 1600 = 0x0640 -> 40 06, 3200 = 0x0c80 -> 80 0c
    // Y at 24 + 2i the same; colour at 40 + 3i: ff 00 00 | 00 ff 00 | 00 00 ff
    let stages = [g::Stage { x: 800, y: 800, rgb: 0xff0000 }, g::Stage { x: 1600, y: 1600, rgb: 0x00ff00 }, g::Stage { x: 3200, y: 3200, rgb: 0x0000ff }];
    let mut want = vec![0u8; 64];
    want[..4].copy_from_slice(&[0x54, 0, 1, 3]);
    want[8..14].copy_from_slice(&[0x20, 0x03, 0x40, 0x06, 0x80, 0x0c]);
    want[24..30].copy_from_slice(&[0x20, 0x03, 0x40, 0x06, 0x80, 0x0c]);
    want[40..49].copy_from_slice(&[0xff, 0, 0, 0, 0xff, 0, 0, 0, 0xff]);
    assert_eq!(g::dpi_table_command(&stages, 1), want);
    want[7] = 0xa7;
    assert_eq!(g::encode_command(&g::dpi_table_command(&stages, 1)), want);
    // eight stages fill the block: X 8..24, Y 24..40, colours 40..64
    let eight: Vec<g::Stage> = (0..8).map(|i| g::Stage { x: 100 * (i + 1), y: 100 * (i + 1), rgb: 0x010203 }).collect();
    let c = g::dpi_table_command(&eight, 0);
    assert_eq!((c.len(), c[3], c[22], c[23], c[38], c[39], c[61], c[62], c[63]), (64, 8, 0x20, 0x03, 0x20, 0x03, 1, 2, 3));
}

#[test]
fn the_option0_command_copies_the_block_and_stamps_17_and_18() {
    // GET reply: byte 0 = D3, bytes 1..7 arbitrary (NOT copied), bytes 8..63 = (5 i + 3) mod 256 with byte 9 = 1 (1000 Hz)
    let mut cur = vec![0u8; 64];
    for (i, b) in cur.iter_mut().enumerate().skip(8) {
        *b = (5 * i + 3) as u8;
    }
    cur[0] = 0xd3;
    cur[1] = 0x77;
    cur[3] = 0x55;
    cur[9] = 1;
    // the mouse already holds 255 / 8 in bytes 17 / 18 (what patchOptionParam0 would stamp), so the stamp changes nothing
    cur[17] = 255;
    cur[18] = 8;
    // patchOptionParam0({9: 130}): [0] = 0x53, [1..8) = 0, [8..64) = current, [17] = 255, [18] = 8, then [9] = 130
    let mut want = vec![0u8; 64];
    for (i, b) in want.iter_mut().enumerate().skip(8) {
        *b = (5 * i + 3) as u8;
    }
    want[0] = 0x53;
    want[9] = 130;
    want[17] = 255;
    want[18] = 8;
    assert_eq!(g::option0_command(&cur, 9, 130).unwrap(), want);
    // patchOptionParam0({52: 2}): the rate byte stays as read (1)
    want[9] = 1;
    want[52] = 2;
    assert_eq!(g::option0_command(&cur, 52, 2).unwrap(), want);
    // stamped by encodeCommand: byte 7 = 255 - 0x53 = 0xac
    assert_eq!(g::encode_command(&g::option0_command(&cur, 52, 2).unwrap())[7], 0xac);
}

// ---------------------------------------------------------------- decode

#[test]
fn answers_decode_like_openmouse() {
    // pollStatus: the measured layout `01 00 2D 01 00 01`: ready to read, no keyboard, mouse at 0x2D = 45 %, keyboard offline,
    // mouse online (0 = online), ready to send
    let s = g::decode_status(&[1, 0, 0x2d, 1, 0, 1]).unwrap();
    assert_eq!((s.can_read, s.keyboard_battery, s.mouse_battery, s.keyboard_online, s.mouse_online, s.can_send), (true, 0, 45, false, true, true));
    assert_eq!(g::battery_percent(&s), Some(45));
    let asleep = g::decode_status(&[0, 0, 0, 1, 1, 1]).unwrap();
    assert!(!asleep.mouse_online && !asleep.can_read && asleep.can_send);
    // readStatus: a battery byte of 0 or above 100 is "none"
    for (b, want) in [(0u8, None), (1, Some(1)), (100, Some(100)), (101, None), (255, None)] {
        let st = g::decode_status(&[1, 0, b, 1, 0, 1]).unwrap();
        assert_eq!(g::battery_percent(&st), want, "{b}");
    }
    assert!(g::decode_status(&[1, 2, 3]).is_none());
    // getDpi: active [2], count [3] (at most 8), X at 8 + 2i LE, Y at 24 + 2i, colour at 40 + 3i (r g b)
    let mut r = vec![0u8; 64];
    r[0] = 0xd4;
    r[2] = 1;
    r[3] = 2;
    r[8..12].copy_from_slice(&[0x20, 0x03, 0x40, 0x06]);
    r[24..28].copy_from_slice(&[0x20, 0x03, 0xd0, 0x07]);
    r[40..46].copy_from_slice(&[0x12, 0x34, 0x56, 0xab, 0xcd, 0xef]);
    let (stages, active) = g::decode_dpi(&r).unwrap();
    assert_eq!(active, 1);
    assert_eq!(stages, vec![g::Stage { x: 800, y: 800, rgb: 0x123456 }, g::Stage { x: 1600, y: 2000, rgb: 0xabcdef }]);
    // an active index >= 8 reads as 0; a count above 8 is cut to 8; a count of 0 is an error
    r[2] = 8;
    assert_eq!(g::decode_dpi(&r).unwrap().1, 0);
    r[3] = 200;
    assert_eq!(g::decode_dpi(&r).unwrap().0.len(), 8);
    r[3] = 0;
    assert!(matches!(g::decode_dpi(&r), Err(Error::BadAnswer(_))));
    assert!(g::decode_dpi(&r[..30]).is_err());
}

#[test]
fn the_models_and_their_caps_are_the_supported_lists() {
    // device ids: 2285 = M5 Pro, 1893 and 3009 = R2, anything else (R3 1643, AJ179 PRO 1851, unknown) is not on the list
    assert_eq!(g::profile_for(2285).map(|p| p.model), Some("Lingbao M5 Pro"));
    assert_eq!(g::profile_for(1893).map(|p| p.model), Some("Attack Shark R2"));
    assert_eq!(g::profile_for(3009).map(|p| p.model), Some("Attack Shark R2"));
    assert!(g::profile_for(1643).is_none() && g::profile_for(1851).is_none() && g::profile_for(0).is_none());
    // the M5 Pro is PAW3395 (50..26000), the R2 PAW3950 (50..42000), both in steps of 50
    let (m5, r2) = (g::profile_for(2285).unwrap(), g::profile_for(1893).unwrap());
    assert_eq!((m5.min_dpi, m5.max_dpi, m5.dpi_step, r2.min_dpi, r2.max_dpi, r2.dpi_step), (50, 26000, 50, 50, 42000, 50));
    assert!(m5.allows_dpi(50) && m5.allows_dpi(26000) && !m5.allows_dpi(26050) && !m5.allows_dpi(25) && !m5.allows_dpi(1234) && r2.allows_dpi(42000));
    // polling: the receiver reaches 8000 Hz, the cable is capped at 1000
    assert_eq!(m5.polling_rates(g::DONGLE), vec![125, 250, 500, 1000, 2000, 4000, 8000]);
    assert_eq!(m5.polling_rates(g::DIRECT), vec![125, 250, 500, 1000]);
    // lift-off stops (tenths of a mm, stated in OpenMouse's comment): PAW3395 1 / 2 mm, PAW3950 0.7 / 1 / 2 mm
    assert_eq!((m5.lift_off, r2.lift_off), (&[10u32, 20][..], &[7u32, 10, 20][..]));
    assert_eq!((g::lift_off_tenths(m5, 0), g::lift_off_tenths(m5, 1), g::lift_off_tenths(m5, 9)), (Some(10), Some(20), Some(10)), "an index off the list reads as the first stop");
}

// ---------------------------------------------------------------- the device table

fn config(pid: u16) -> HidInfo {
    hid_info("col01", 0x3151, pid, 0xffff, 0x02, (65, 65, 65))
}

#[test]
fn the_receiver_and_the_cable_match_on_the_vendor_interface() {
    let m = g::matches(&[&config(0x402d)], 0x3151, 0x402d).unwrap();
    assert_eq!((m.family, m.model, m.wireless, m.variant), (Family::GearHub, None, Some(true), g::DONGLE));
    let m = g::matches(&[&config(0x4026)], 0x3151, 0x4026).unwrap();
    assert_eq!((m.family, m.model, m.wireless, m.variant), (Family::GearHub, None, Some(false), g::DIRECT));
    assert_eq!(g::PRODUCTS.iter().map(|p| p.0).collect::<Vec<_>>(), vec![0x402d, 0x4026]);
    // the vendor collection is picked among the mouse's other interfaces
    let mouse = hid_info("mi_00", 0x3151, 0x402d, 0x01, 0x02, (8, 0, 0));
    let kb = hid_info("mi_01", 0x3151, 0x402d, 0x01, 0x06, (8, 1, 0));
    let cfg = config(0x402d);
    assert_eq!(g::matches(&[&mouse, &kb, &cfg], 0x3151, 0x402d).unwrap().config.path, "col01");
}

#[test]
fn other_ids_and_other_interfaces_do_not_match() {
    // the Bluetooth id (AJAZZ AJ179 PRO, not on the list), other 0x3151 products (Fantech's own hardware), other vendors
    assert!(g::matches(&[&config(0x402c)], 0x3151, 0x402c).is_none());
    assert!(g::matches(&[&config(0x5043)], 0x3151, 0x5043).is_none());
    assert!(g::matches(&[&config(0x402d)], 0x1d57, 0x402d).is_none());
    assert!(g::matches(&[&config(0x402d)], 0x0000, 0x402d).is_none());
    // the plain mouse / keyboard interfaces, another vendor page, another usage
    for h in [
        hid_info("a", 0x3151, 0x402d, 0x01, 0x02, (8, 0, 0)),
        hid_info("b", 0x3151, 0x402d, 0x01, 0x06, (8, 1, 0)),
        hid_info("c", 0x3151, 0x402d, 0xff00, 0x02, (65, 65, 65)),
        hid_info("d", 0x3151, 0x402d, 0xffff, 0x01, (65, 65, 65)),
    ] {
        assert!(g::matches(&[&h], 0x3151, 0x402d).is_none(), "{h:?}");
    }
    assert!(g::matches(&[], 0x3151, 0x402d).is_none());
}

#[test]
fn no_other_family_claims_the_gearhub_ids() {
    for pid in [0x402du16, 0x4026] {
        let cfg = config(pid);
        let found = bu_mouse::family::find(std::slice::from_ref(&cfg), 0x3151, pid).unwrap();
        assert_eq!(found.family, Family::GearHub, "{pid:#06x}");
        let own = [&cfg];
        assert!(bu_mouse::pulsar::matches_cmouse(&own, 0x3151, pid).is_none() && bu_mouse::pulsar::matches_xs1(&own, 0x3151, pid).is_none());
        assert!(bu_mouse::logitech::matches(&own, 0x3151, pid).is_none());
        assert!(bu_mouse::razer::matches(&own, 0x3151, pid).is_none());
        assert!(bu_mouse::steelseries::matches(&own, 0x3151, pid).is_none());
    }
}

// ---------------------------------------------------------------- the read-only allow-list

#[test]
fn only_the_read_requests_and_the_relay_handshake_pass_the_allow_list() {
    for read in [poll(), select(), notice(), get_id(), get_opt0(), get_dpi()] {
        assert!(g::read_request_allowed(&read), "{read:02x?}");
    }
    // a longer collection: zero padding is fine, anything else is not
    let mut long = poll();
    long.extend_from_slice(&[0, 0]);
    assert!(g::read_request_allowed(&long));
    long[66] = 1;
    assert!(!g::read_request_allowed(&long));
    // every write is refused: the OPTIONPARAM0 write (checksum 0xac), the DPI table write (0xa7), button / other commands
    let mut opt0 = vec![0u8; 64];
    opt0[0] = 0x53;
    opt0[7] = 0xac;
    let mut dpi = vec![0u8; 64];
    dpi[..4].copy_from_slice(&[0x54, 0, 1, 3]);
    dpi[7] = 0xa7;
    for w in [rep(&opt0), rep(&dpi), rep(&[0x50, 0, 2, 0, 0, 0, 0, 0xad]), rep(&[0x80, 0, 0, 0, 0, 0, 0, 0x7f]), rep(&[0xd0, 0, 0, 0, 0, 0, 0, 0x2f])] {
        assert!(!g::read_request_allowed(&w), "{w:02x?}");
    }
    // a relay frame with another argument (`F6 04` = another target), `F7 01`, the right read with a wrong checksum, a stray byte
    assert!(!g::read_request_allowed(&rep(&[0xf6, 0x04])));
    assert!(!g::read_request_allowed(&rep(&[0xf7, 0x01])));
    assert!(!g::read_request_allowed(&rep(&[0xd4, 0, 0, 0, 0, 0, 0, 0x2a])));
    assert!(!g::read_request_allowed(&rep(&[0xd4, 1, 0, 0, 0, 0, 0, 0x2a])), "profile 1 is never read");
    let mut stray = get_id();
    stray[20] = 1;
    assert!(!g::read_request_allowed(&stray));
    // malformed: a report id, too short
    let mut id = poll();
    id[0] = 1;
    assert!(!g::read_request_allowed(&id));
    assert!(!g::read_request_allowed(&poll()[..64]));
    assert!(!g::read_request_allowed(&[]));
}

// ---------------------------------------------------------------- a scripted GearHub mouse

/// A fake GearHub-V5 mouse behind a fake receiver (or on the cable): answers the relay frames and the commands, keeps its
/// state, and PANICS on a bad checksum / a command outside the relay's order - a real one would stay silent.
struct Mouse {
    dongle: bool,
    device_id: u32,
    online: bool,
    battery: u8,
    selected: bool,
    pending: Option<Vec<u8>>,
    opt0: Vec<u8>,
    stages: Vec<(u16, u16, u32)>,
    active: u8,
    /// status polls answered "not ready" before the receiver is ready
    busy_polls: u32,
    never_ready: bool,
    ignore_writes: bool,
    silent: bool,
    wrong_id: bool,
    /// every command received (decoded 64 data bytes), in order
    commands: Vec<Vec<u8>>,
    /// a hand-made `D4` answer (bytes 1..63) instead of the one built from `stages`
    d4_raw: Option<Vec<u8>>,
}

impl Mouse {
    fn new(dongle: bool, device_id: u32) -> Self {
        // OPTIONPARAM0: bytes 8..64 = (5 i + 3) mod 256, 1000 Hz (code 1) at 9, lift-off index 1 at 52, junk at 1 and 3
        let mut opt0 = vec![0u8; 64];
        for (i, b) in opt0.iter_mut().enumerate().skip(8) {
            *b = (5 * i + 3) as u8;
        }
        opt0[1] = 0x77;
        opt0[3] = 0x55;
        opt0[9] = 1;
        // bytes 17 / 18 hold what GearHub's own writer stamps (255 / 8), which is what a rewrite of the block may leave alone
        opt0[17] = 255;
        opt0[18] = 8;
        opt0[52] = 1;
        Mouse {
            dongle,
            device_id,
            online: true,
            battery: 45,
            selected: false,
            pending: None,
            opt0,
            stages: vec![(800, 800, 0xff0000), (1600, 1600, 0x00ff00), (3200, 3200, 0x0000ff)],
            active: 1,
            busy_polls: 0,
            never_ready: false,
            ignore_writes: false,
            silent: false,
            wrong_id: false,
            commands: Vec::new(),
            d4_raw: None,
        }
    }

    fn reply_for(&mut self, cmd: &[u8]) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        if self.silent {
            return r;
        }
        match cmd[0] {
            0x8f => {
                r[0] = 0x8f;
                r[1..5].copy_from_slice(&self.device_id.to_le_bytes());
            }
            0xd3 => r.copy_from_slice(&self.opt0),
            0xd4 if self.d4_raw.is_some() => r.copy_from_slice(self.d4_raw.as_ref().unwrap()),
            0xd4 => {
                r[0] = 0xd4;
                r[2] = self.active;
                r[3] = self.stages.len() as u8;
                r[4..7].copy_from_slice(&[0x99, 0x99, 0x99]);
                for (i, (x, y, rgb)) in self.stages.iter().enumerate() {
                    r[8 + 2 * i..10 + 2 * i].copy_from_slice(&x.to_le_bytes());
                    r[24 + 2 * i..26 + 2 * i].copy_from_slice(&y.to_le_bytes());
                    r[40 + 3 * i..43 + 3 * i].copy_from_slice(&[(rgb >> 16) as u8, (rgb >> 8) as u8, *rgb as u8]);
                }
            }
            other => panic!("the fake does not know the read {other:#04x}"),
        }
        r[0] = if self.wrong_id { 0x11 } else { cmd[0] };
        r
    }

    fn apply_write(&mut self, c: &[u8]) {
        if self.ignore_writes {
            return;
        }
        match c[0] {
            0x53 => {
                assert_eq!((c[17], c[18]), (255, 8), "stamped like GearHub's own writer");
                self.opt0[8..].copy_from_slice(&c[8..]);
            }
            0x54 => {
                self.active = c[2];
                self.stages = (0..c[3] as usize)
                    .map(|i| (u16::from_le_bytes([c[8 + 2 * i], c[9 + 2 * i]]), u16::from_le_bytes([c[24 + 2 * i], c[25 + 2 * i]]), (c[40 + 3 * i] as u32) << 16 | (c[41 + 3 * i] as u32) << 8 | c[42 + 3 * i] as u32))
                    .collect();
            }
            other => panic!("the fake does not know the write {other:#04x}"),
        }
    }

    fn command(&mut self, payload: &[u8]) -> Option<Vec<u8>> {
        let sum = payload[..7].iter().fold(0u8, |s, b| s.wrapping_add(*b));
        assert_eq!(payload[7], 255 - sum, "a wrong checksum: a real mouse would stay silent");
        self.commands.push(payload.to_vec());
        match payload[0] {
            0x53 | 0x54 => {
                self.apply_write(payload);
                None
            }
            _ => Some(self.reply_for(payload)),
        }
    }

    fn answer(&mut self, out: &[u8], how: &HidTransfer) -> Result<Vec<u8>, Error> {
        assert_eq!((out.len(), out[0]), (65, 0));
        let p = &out[1..];
        let mut a = vec![0u8; 65];
        match how {
            HidTransfer::FeatureOut => {
                if p[0] == 0xf6 {
                    assert_eq!(p[1], 0x05);
                    self.selected = true;
                } else {
                    assert!(!self.dongle || self.selected, "a relayed command before the mouse was selected");
                    if let Some(r) = self.command(p) {
                        self.pending = Some(r);
                    }
                }
            }
            HidTransfer::Feature { reply_id: 0, reply_len: 65, wait_ms: 10 } => match p[0] {
                0xf7 => {
                    assert!(self.dongle);
                    let busy = self.never_ready || self.busy_polls > 0;
                    self.busy_polls = self.busy_polls.saturating_sub(1);
                    // [canRead, keyboard battery, mouse battery, keyboard offline = 1, mouse online = 0, canSend]
                    a[1..7].copy_from_slice(&[(!busy && self.pending.is_some()) as u8, 0, self.battery, 1, !self.online as u8, !busy as u8]);
                }
                0xfc => {
                    assert!(self.dongle);
                    a[1..].copy_from_slice(&self.pending.take().expect("a notice-read with nothing waiting"));
                }
                _ => {
                    assert!(!self.dongle, "the receiver is not asked for a command directly");
                    let r = self.command(p).expect("a read");
                    a[1..].copy_from_slice(&r);
                }
            },
            other => panic!("unexpected transfer {other:?}"),
        }
        Ok(a)
    }
}

fn port_for(info: &HidInfo, mouse: Rc<RefCell<Mouse>>) -> ScriptPort {
    ScriptPort::new(info.clone(), move |out, how| mouse.borrow_mut().answer(out, how))
}

fn sent(port: &ScriptPort) -> Vec<Vec<u8>> {
    port.sent.iter().map(|s| s.0.clone()).collect()
}

/// The frames of one relayed READ: ready-to-send poll, the command, ready-to-read poll, notice-read.
fn relayed(cmd: Vec<u8>) -> Vec<Vec<u8>> {
    vec![poll(), cmd, poll(), notice()]
}

fn dongle_setup(device_id: u32) -> (HidInfo, bu_mouse::family::Matched, Rc<RefCell<Mouse>>, ScriptPort) {
    let info = config(0x402d);
    let m = g::matches(&[&info], 0x3151, 0x402d).unwrap();
    let mouse = Rc::new(RefCell::new(Mouse::new(true, device_id)));
    let port = port_for(&info, mouse.clone());
    (info, m, mouse, port)
}

fn cable_setup(device_id: u32) -> (HidInfo, bu_mouse::family::Matched, Rc<RefCell<Mouse>>, ScriptPort) {
    let info = config(0x4026);
    let m = g::matches(&[&info], 0x3151, 0x4026).unwrap();
    let mouse = Rc::new(RefCell::new(Mouse::new(false, device_id)));
    let port = port_for(&info, mouse.clone());
    (info, m, mouse, port)
}

// ---------------------------------------------------------------- driver: the receiver (M5 Pro and R2)

#[test]
fn the_m5_pro_behind_the_receiver_is_read_the_openmouse_way() {
    let (_, m, _mouse, mut port) = dongle_setup(2285);
    let s = g::GearHub.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((s.online, s.model.as_deref(), s.dpi, s.polling_hz, s.lift_off, s.battery_percent, s.charging, s.wireless), (true, Some("Lingbao M5 Pro"), Some(1600), Some(1000), None, Some(45), None, Some(true)));
    // readDongleStatus; getDeviceId (select once, then the relay); getDpi; getOptionParam0
    let mut want = vec![poll(), select()];
    want.extend(relayed(get_id()));
    want.extend(relayed(get_dpi()));
    want.extend(relayed(get_opt0()));
    assert_eq!(sent(&port), want);
    assert!(port.sent.iter().all(|x| g::read_request_allowed(&x.0)), "reading = read requests and the relay handshake only");
    // the status polls and the notice-read fetch with the 10 ms OpenMouse waits; the select and the commands are plain sends
    for (out, how) in &port.sent {
        let is_send = *out == select() || *out == get_id() || *out == get_dpi() || *out == get_opt0();
        let want_how = if is_send { HidTransfer::FeatureOut } else { HidTransfer::Feature { reply_id: 0, reply_len: 65, wait_ms: 10 } };
        assert_eq!(*how, want_how, "{out:02x?}");
    }
    // caps: 50..26000 in 50s, the receiver's 8K ladder, lift-off 1 / 2 mm, battery
    assert_eq!((s.caps.dpi.as_ref().map(|d| (d.min(), d.max(), d.allows(1250), d.allows(1234))), s.caps.polling.clone(), s.caps.lift_off.clone(), s.caps.battery), (Some((50, 26000, true, false)), vec![125, 250, 500, 1000, 2000, 4000, 8000], Vec::<u32>::new(), true));
}

#[test]
fn the_r2_is_told_apart_by_its_device_id() {
    for id in [1893u32, 3009] {
        let (_, m, _mouse, mut port) = dongle_setup(id);
        let s = g::GearHub.read(&m, &mut port, ReadOptions::default()).unwrap();
        // lift-off index 1 = the R2's middle stop, 1 mm; 50..42000
        assert_eq!((s.model.as_deref(), s.dpi, s.lift_off), (Some("Attack Shark R2"), Some(1600), None), "{id}");
        assert_eq!((s.caps.dpi.map(|d| d.max()), s.caps.lift_off), (Some(42000), Vec::<u32>::new()));
    }
}

#[test]
fn a_ready_receiver_that_is_slow_is_polled_again() {
    let (_, m, mouse, mut port) = dongle_setup(2285);
    mouse.borrow_mut().busy_polls = 3;
    let s = g::GearHub.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert!(s.online);
    // the first status poll (not ready is no problem there) used one busy answer, the next ready-to-send poll two more
    let polls = port.sent.iter().filter(|x| x.0 == poll()).count();
    assert!(polls > 1 + 2 * 3, "more polls than a ready receiver needs ({polls})");
}

#[test]
fn a_sleeping_mouse_is_offline_after_one_status_poll() {
    let (_, m, mouse, mut port) = dongle_setup(2285);
    mouse.borrow_mut().online = false;
    let s = g::GearHub.read(&m, &mut port, ReadOptions { online_tries: 1 }).unwrap();
    assert!(!s.online && s.dpi.is_none() && s.battery_percent.is_none() && s.polling_hz.is_none());
    assert_eq!(s.wireless, Some(true));
    assert_eq!(sent(&port), vec![poll()], "OpenMouse asks once; nothing else is sent to a receiver without a mouse");
    // a write to a sleeping mouse is refused before any command
    port.sent.clear();
    assert!(matches!(g::GearHub.write(&m, &mut port, Setting::Dpi(1600)), Err(Error::MouseGone(_))));
    assert_eq!(sent(&port), vec![poll()]);
}

#[test]
fn a_receiver_that_never_becomes_ready_is_an_error_after_eight_polls() {
    let (_, m, mouse, mut port) = dongle_setup(2285);
    mouse.borrow_mut().never_ready = true;
    assert!(matches!(g::GearHub.read(&m, &mut port, ReadOptions::default()), Err(Error::BadAnswer(_))));
    // 1 status poll + select + 8 polls of waitReady("send"), and nothing else
    assert_eq!(port.sent.len(), 1 + 1 + 8);
    assert!(port.sent.iter().all(|x| x.0 == poll() || x.0 == select()));
}

#[test]
fn silent_and_confused_answers_are_errors() {
    // 64 zeros = "was not answered"
    let (_, m, mouse, mut port) = dongle_setup(2285);
    mouse.borrow_mut().silent = true;
    assert!(matches!(g::GearHub.read(&m, &mut port, ReadOptions::default()), Err(Error::BadAnswer(_))));
    // an answer to another command
    let (_, m, mouse, mut port) = dongle_setup(2285);
    mouse.borrow_mut().wrong_id = true;
    assert!(matches!(g::GearHub.read(&m, &mut port, ReadOptions::default()), Err(Error::BadAnswer(_))));
    // a short report
    let info = config(0x402d);
    let m = g::matches(&[&info], 0x3151, 0x402d).unwrap();
    let mut short = ScriptPort::new(info, |_, _| Ok(vec![0u8; 10]));
    assert!(matches!(g::GearHub.read(&m, &mut short, ReadOptions::default()), Err(Error::BadAnswer(_))));
    // the mouse is unplugged
    let info = config(0x402d);
    let mut gone = ScriptPort::new(info, |_, _| Err(Error::MouseGone("col01".into())));
    assert!(matches!(g::GearHub.read(&m, &mut gone, ReadOptions::default()), Err(Error::MouseGone(_))));
}

#[test]
fn a_gearhub_mouse_off_the_supported_list_is_refused() {
    // Attack Shark R3 (device id 1643) and the AJAZZ AJ179 PRO (1851) answer the same protocol but are not on the list
    for id in [1643u32, 1851, 12345] {
        let (_, m, _mouse, mut port) = dongle_setup(id);
        assert!(matches!(g::GearHub.read(&m, &mut port, ReadOptions::default()), Err(Error::UnsupportedMouse(_))), "{id}");
        port.sent.clear();
        assert!(matches!(g::GearHub.write(&m, &mut port, Setting::Dpi(800)), Err(Error::UnsupportedMouse(_))), "{id}");
        assert!(port.sent.iter().all(|x| g::read_request_allowed(&x.0)), "only reads were sent");
    }
}

#[test]
fn set_dpi_replaces_the_active_stage_and_writes_the_whole_table_back() {
    let (_, m, mouse, mut port) = dongle_setup(2285);
    let c = g::GearHub.write(&m, &mut port, Setting::Dpi(2000)).unwrap();
    assert_eq!((c.now, c.before), (2000, Some(1600)));
    // the table OpenMouse's setDpiForStage sends: [54, 0, active 1, count 3, 0 0 0, a7, X.., Y.., colours..], 2000 = 0x07d0
    let mut table = vec![0u8; 64];
    table[..4].copy_from_slice(&[0x54, 0, 1, 3]);
    // bytes 4..6 are copied from the D4 answer (the fake stores 0x99 there), not zeroed as OpenMouse's writeDpiTable would
    table[4..7].copy_from_slice(&[0x99, 0x99, 0x99]);
    // checksum over bytes 0..7: 0x54 + 0 + 1 + 3 + 3 * 0x99 = 84 + 4 + 459 = 547 = 0x223 -> 0x23 = 35 -> 255 - 35 = 220 = 0xdc
    table[7] = 0xdc;
    table[8..14].copy_from_slice(&[0x20, 0x03, 0xd0, 0x07, 0x80, 0x0c]);
    table[24..30].copy_from_slice(&[0x20, 0x03, 0xd0, 0x07, 0x80, 0x0c]);
    table[40..49].copy_from_slice(&[0xff, 0, 0, 0, 0xff, 0, 0, 0, 0xff]);
    // sequence: the status poll, getDeviceId, getDpi, the write (select already done: poll, send, 50 ms), the read-back getDpi
    let mut want = vec![poll(), select()];
    want.extend(relayed(get_id()));
    want.extend(relayed(get_dpi()));
    want.extend([poll(), rep(&table[..])]);
    want.extend(relayed(get_dpi()));
    assert_eq!(sent(&port), want);
    // the mouse's other stages and colours are as they were
    assert_eq!(mouse.borrow().stages, vec![(800, 800, 0xff0000), (2000, 2000, 0x00ff00), (3200, 3200, 0x0000ff)]);
    assert_eq!(mouse.borrow().active, 1);
    // the write is NOT a read request
    assert!(!g::read_request_allowed(&rep(&table)));
}

#[test]
fn set_dpi_keeps_the_y_of_nothing_it_does_not_own_and_validates_first() {
    let (_, m, mouse, mut port) = dongle_setup(2285);
    // the active stage had X != Y: setDpi(dpi, dpiY = dpi) sets both
    mouse.borrow_mut().stages[1] = (1600, 1200, 0x00ff00);
    g::GearHub.write(&m, &mut port, Setting::Dpi(26000)).unwrap();
    assert_eq!(mouse.borrow().stages[1], (26000, 26000, 0x00ff00));
    // outside the model / off the 50 grid / 42000 is the R2's, not the M5 Pro's: refused before any write
    port.sent.clear();
    for bad in [49u32, 25, 1234, 26050, 42000] {
        assert!(matches!(g::GearHub.write(&m, &mut port, Setting::Dpi(bad)), Err(Error::OutOfRange { .. })), "{bad}");
    }
    assert!(port.sent.iter().all(|x| g::read_request_allowed(&x.0)), "no write left for a refused value");
    // the R2 does 42000
    let (_, m2, mouse2, mut port2) = dongle_setup(1893);
    assert_eq!(g::GearHub.write(&m2, &mut port2, Setting::Dpi(42000)).unwrap().now, 42000);
    assert_eq!(mouse2.borrow().stages[1].0, 42000);
}

#[test]
fn set_polling_rewrites_optionparam0_with_every_other_byte_copied_back() {
    let (_, m, mouse, mut port) = dongle_setup(2285);
    let c = g::GearHub.write(&m, &mut port, Setting::Polling(4000)).unwrap();
    assert_eq!((c.now, c.before), (4000, Some(1000)));
    // the block: [0] 0x53, [7] = 255 - 0x53 = 0xac, [8..64) = what D3 returned ((5 i + 3) mod 256), [9] = 130 (4000 Hz),
    // [17] = 255, [18] = 8
    let mut block = vec![0u8; 64];
    for (i, b) in block.iter_mut().enumerate().skip(8) {
        *b = (5 * i + 3) as u8;
    }
    block[0] = 0x53;
    block[9] = 1;
    block[52] = 1;
    block[7] = 0xac;
    block[9] = 130;
    block[17] = 255;
    block[18] = 8;
    let mut want = vec![poll(), select()];
    want.extend(relayed(get_id()));
    want.extend(relayed(get_opt0()));
    want.extend([poll(), rep(&block)]);
    want.extend(relayed(get_opt0()));
    assert_eq!(sent(&port), want);
    assert_eq!(mouse.borrow().opt0[9], 130);
    let s = g::GearHub.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!(s.polling_hz, Some(4000));
    // every rate of the ladder
    for (hz, code) in [(125u32, 8u8), (250, 4), (500, 2), (1000, 1), (2000, 132), (8000, 129)] {
        assert_eq!(g::GearHub.write(&m, &mut port, Setting::Polling(hz)).unwrap().now, hz);
        assert_eq!(mouse.borrow().opt0[9], code);
    }
    // not a rate of the ladder
    port.sent.clear();
    assert!(matches!(g::GearHub.write(&m, &mut port, Setting::Polling(3000)), Err(Error::OutOfRange { .. })));
    assert!(port.sent.iter().all(|x| g::read_request_allowed(&x.0)));
}

#[test]
fn lift_off_is_not_offered_and_nothing_is_sent_for_it() {
    // caps.lift_off is empty and read() reports none (the millimetres stand only in a comment of OpenMouse's source), so a
    // lift-off write is refused before a single frame goes out - for every value, on both models and both links
    for id in [2285u32, 1893] {
        let (_, m, mouse, mut port) = dongle_setup(id);
        let before = mouse.borrow().opt0.clone();
        for tenths in [7u32, 10, 20, 15, 0] {
            assert!(matches!(g::GearHub.write(&m, &mut port, Setting::LiftOff(tenths)), Err(Error::OutOfRange { .. })), "{id} {tenths}");
        }
        assert!(port.sent.is_empty(), "{id}: nothing sent");
        assert_eq!(mouse.borrow().opt0, before);
    }
    let (_, m, _mouse, mut port) = cable_setup(1893);
    assert!(matches!(g::GearHub.write(&m, &mut port, Setting::LiftOff(10)), Err(Error::OutOfRange { .. })));
    assert!(port.sent.is_empty());
}

#[test]
fn the_dpi_frame_keeps_every_stored_byte_and_changes_only_the_active_stage() {
    // a D4 answer with four stages whose slots 5..8 hold stored values, bytes 4..6 and colours set; stage 3 (index 2) active
    let mut a = vec![0u8; 64];
    a[0] = 0xd4;
    a[2] = 2;
    a[3] = 4;
    a[4..7].copy_from_slice(&[0x11, 0x22, 0x33]);
    for i in 0..8usize {
        let (x, y) = (400 + 200 * i as u16, 500 + 100 * i as u16);
        a[8 + 2 * i..10 + 2 * i].copy_from_slice(&x.to_le_bytes());
        a[24 + 2 * i..26 + 2 * i].copy_from_slice(&y.to_le_bytes());
        a[40 + 3 * i..43 + 3 * i].copy_from_slice(&[i as u8 + 1, i as u8 + 2, i as u8 + 3]);
    }
    // only X (8 + 2*2 = 12, 13) and Y (24 + 4 = 28, 29) of index 2 change: 2000 = 0x07d0 -> d0 07; [0] = 0x54, [1] = 0
    let mut want = a.clone();
    want[0] = 0x54;
    want[12..14].copy_from_slice(&[0xd0, 0x07]);
    want[28..30].copy_from_slice(&[0xd0, 0x07]);
    assert_eq!(g::dpi_write_frame(&a, 2000).unwrap(), want);
    // through the driver: the same frame goes out (checksum stamped over bytes 0..7: 0x54 + 0 + 2 + 4 + 0x11 + 0x22 + 0x33 =
    // 84 + 6 + 102 = 192 -> 255 - 192 = 63 = 0x3f), slots 5..8 intact
    let (_, m, mouse, mut port) = dongle_setup(2285);
    {
        let mut ms = mouse.borrow_mut();
        let mut raw = a.clone();
        raw[1] = 0;
        ms.d4_raw = Some(raw);
    }
    // the mouse never applies it in this fake (it keeps answering the hand-made table), so only the sent frame is checked
    let _ = g::GearHub.write(&m, &mut port, Setting::Dpi(2000));
    let sent54 = mouse.borrow().commands.iter().find(|c| c[0] == 0x54).cloned().expect("a 54 frame went out");
    let mut full = want.clone();
    full[7] = 0x3f;
    assert_eq!(sent54, full);
    assert_eq!(&sent54[16..24], &a[16..24], "stage slots 5..8 (X) are as stored");
    assert_eq!(&sent54[32..40], &a[32..40], "stage slots 5..8 (Y) are as stored");
}

#[test]
fn a_dpi_table_that_cannot_be_rewritten_exactly_is_refused_and_nothing_is_written() {
    let base = |active: u8, count: u8| {
        let mut a = vec![0u8; 64];
        a[0] = 0xd4;
        a[2] = active;
        a[3] = count;
        a[8..10].copy_from_slice(&800u16.to_le_bytes());
        a[24..26].copy_from_slice(&800u16.to_le_bytes());
        a
    };
    // active index at or past the count (OpenMouse would clamp it / write 0), count 0, count above 8, active 8 or more
    for (active, count) in [(3u8, 3u8), (4, 3), (255, 3), (0, 0), (0, 9), (2, 255), (8, 8), (9, 9)] {
        assert!(matches!(g::dpi_write_frame(&base(active, count), 1600), Err(Error::BadAnswer(_))), "{active} {count}");
    }
    assert!(g::dpi_write_frame(&base(0, 1), 1600).is_ok());
    assert!(g::dpi_write_frame(&base(7, 8), 1600).is_ok());
    assert!(matches!(g::dpi_write_frame(&[0xd4; 10], 1600), Err(Error::BadAnswer(_))), "a short answer");
    // through the driver: only reads go out
    for (active, count) in [(3u8, 3u8), (9, 3), (0, 9)] {
        let (_, m, mouse, mut port) = dongle_setup(2285);
        mouse.borrow_mut().d4_raw = Some(base(active, count));
        let e = g::GearHub.write(&m, &mut port, Setting::Dpi(1600)).unwrap_err();
        assert!(matches!(e, Error::BadAnswer(_) | Error::OutOfRange { .. }), "{e:?}");
        assert!(port.sent.iter().all(|x| g::read_request_allowed(&x.0)), "{active} {count}: nothing but reads");
        assert!(mouse.borrow().commands.iter().all(|c| c[0] != 0x54));
    }
}

#[test]
fn an_option0_block_with_other_bytes_in_17_and_18_is_not_rewritten() {
    // OpenMouse stamps 255 / 8 over whatever is there; unknown saved values would change, so the write is refused
    let mut cur = vec![0u8; 64];
    cur[17] = 255;
    cur[18] = 8;
    assert!(g::option0_command(&cur, 9, 2).is_ok());
    for (b17, b18) in [(0u8, 0u8), (255, 7), (254, 8), (88, 93)] {
        cur[17] = b17;
        cur[18] = b18;
        assert!(matches!(g::option0_command(&cur, 9, 2), Err(Error::BadAnswer(_))), "{b17} {b18}");
    }
    assert!(matches!(g::option0_command(&cur[..40], 9, 2), Err(Error::BadAnswer(_))), "a short answer");
    // through the driver: the polling write is refused after the read, no 53 frame goes out, the mouse is unchanged
    let (_, m, mouse, mut port) = dongle_setup(2285);
    mouse.borrow_mut().opt0[17] = 0x42;
    let e = g::GearHub.write(&m, &mut port, Setting::Polling(500)).unwrap_err();
    assert!(matches!(e, Error::BadAnswer(_)), "{e:?}");
    assert!(port.sent.iter().all(|x| g::read_request_allowed(&x.0)), "only reads went out");
    assert!(mouse.borrow().commands.iter().all(|c| c[0] != 0x53));
    assert_eq!(mouse.borrow().opt0[9], 1);
}

#[test]
fn a_mouse_that_keeps_its_old_value_is_caught_by_the_readback() {
    let (_, m, mouse, mut port) = dongle_setup(2285);
    mouse.borrow_mut().ignore_writes = true;
    assert!(matches!(g::GearHub.write(&m, &mut port, Setting::Dpi(2000)), Err(Error::BadAnswer(_))));
    assert!(matches!(g::GearHub.write(&m, &mut port, Setting::Polling(500)), Err(Error::BadAnswer(_))));
}

// ---------------------------------------------------------------- driver: the cable (no relay)

#[test]
fn the_cable_talks_directly_and_has_no_battery_and_a_1000_hz_ceiling() {
    let (_, m, _mouse, mut port) = cable_setup(1893);
    let s = g::GearHub.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((s.online, s.model.as_deref(), s.dpi, s.polling_hz, s.lift_off, s.battery_percent, s.wireless), (true, Some("Attack Shark R2"), Some(1600), Some(1000), None, None, Some(false)));
    // no status poll, no select: each read is the request, 10 ms, the answer
    assert_eq!(sent(&port), vec![get_id(), get_dpi(), get_opt0()]);
    assert!(port.sent.iter().all(|x| x.1 == HidTransfer::Feature { reply_id: 0, reply_len: 65, wait_ms: 10 }));
    assert!(port.sent.iter().all(|x| g::read_request_allowed(&x.0)));
    assert_eq!((s.caps.polling.clone(), s.caps.battery, s.caps.dpi.map(|d| d.max())), (vec![125, 250, 500, 1000], false, Some(42000)));
}

#[test]
fn the_cable_writes_without_the_relay() {
    let (_, m, mouse, mut port) = cable_setup(2285);
    let c = g::GearHub.write(&m, &mut port, Setting::Polling(500)).unwrap();
    assert_eq!((c.now, c.before), (500, Some(1000)));
    let mut block = vec![0u8; 64];
    for (i, b) in block.iter_mut().enumerate().skip(8) {
        *b = (5 * i + 3) as u8;
    }
    block[0] = 0x53;
    block[9] = 1;
    block[52] = 1;
    block[7] = 0xac;
    block[9] = 2;
    block[17] = 255;
    block[18] = 8;
    // getDeviceId, getOptionParam0, the write (a plain send, then 10 ms), the read-back
    assert_eq!(sent(&port), vec![get_id(), get_opt0(), rep(&block), get_opt0()]);
    assert_eq!(port.sent[2].1, HidTransfer::FeatureOut);
    assert_eq!(mouse.borrow().opt0[9], 2);
    // 2000 Hz is the receiver's: refused on the cable
    port.sent.clear();
    assert!(matches!(g::GearHub.write(&m, &mut port, Setting::Polling(2000)), Err(Error::OutOfRange { .. })));
    assert!(port.sent.iter().all(|x| g::read_request_allowed(&x.0)));
    // DPI on the cable
    assert_eq!(g::GearHub.write(&m, &mut port, Setting::Dpi(3200)).unwrap().now, 3200);
    assert_eq!(mouse.borrow().stages[1].0, 3200);
}

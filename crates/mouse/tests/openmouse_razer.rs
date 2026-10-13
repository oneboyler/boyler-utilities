//! Order 104 check, Razer: our requests = the bytes OpenMouse sends for the same setting (encode), and answers shaped the
//! way OpenMouse reads them decode to the same values (decode). The expected bytes are worked out by hand from OpenMouse
//! (@openmouse/protocol 0.40.1: `dist/razer/codec.js`, `dist/razer/devices.js`, `dist/razer/v4.js`,
//! `dist/drivers/razer/hid.js`, `cobra-hid.js`, `viper-mini-hid.js`, `viper-v4-pro-hid.js`), never from our own encoder.
//!
//! OpenMouse's packet is 90 bytes without the report id: `[0]` status (0 in requests) · `[1]` transaction id · `[2..5]` 0
//! · `[5]` data size · `[6]` command class · `[7]` command id · `[8..]` arguments · `[88]` crc = XOR of bytes 2..87 · `[89]` 0.
//! Windows adds report id 0 in front: OpenMouse's byte i = our byte i + 1 (tid 2, size 6, class 7, id 8, args 9.., crc 89).
//! The crc only covers size, class, id and the arguments (everything else in 2..87 is zero), so each crc below is the XOR of
//! those bytes, done by hand in the comment.

use bu_mouse::family::{Driver, Family, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::os::{HidInfo, HidTransfer};
use bu_mouse::razer as r;
use std::cell::RefCell;
use std::rc::Rc;

/// A request as Windows sends it: report id 0, status 0, then OpenMouse's fields; the crc is given by hand.
fn rz(tid: u8, size: u8, class: u8, id: u8, args: &[u8], crc: u8) -> Vec<u8> {
    let mut v = vec![0u8; 91];
    v[2] = tid;
    v[6] = size;
    v[7] = class;
    v[8] = id;
    v[9..9 + args.len()].copy_from_slice(args);
    v[89] = crc;
    v
}

/// An answer packet the way OpenMouse's receiveFeatureReport returns it (90 bytes, no report id), crc by hand.
fn reply(status: u8, tid: u8, size: u8, class: u8, id: u8, args: &[u8], crc: u8) -> Vec<u8> {
    let mut v = rz(tid, size, class, id, args, crc)[1..].to_vec();
    v[0] = status;
    v
}

// ---------------------------------------------------------------- encode: reads

#[test]
fn reads_are_openmouses_encoderazerrequest_bytes() {
    // RAZER_READ.firmware {0x00, 0x81, 0x02}: crc = 02 ^ 00 ^ 81 = 83
    assert_eq!(r::report(&r::read_firmware(), r::TID_1F, 91), rz(0x1f, 0x02, 0x00, 0x81, &[], 0x83));
    assert_eq!(r::report(&r::read_firmware(), r::TID_FF, 91), rz(0xff, 0x02, 0x00, 0x81, &[], 0x83), "the tid is outside the crc");
    // RAZER_READ.battery {0x07, 0x80, 0x02}: 02 ^ 07 ^ 80 = 85
    assert_eq!(r::report(&r::read_battery(), r::TID_3F, 91), rz(0x3f, 0x02, 0x07, 0x80, &[], 0x85));
    // RAZER_READ.charging {0x07, 0x84, 0x02}: 02 ^ 07 ^ 84 = 81
    assert_eq!(r::report(&r::read_charging(), r::TID_1F, 91), rz(0x1f, 0x02, 0x07, 0x84, &[], 0x81));
    // razerReadDpiCommand(RAZER_STORAGE = 1) {0x04, 0x85, 0x07, [01]}: 07 ^ 04 ^ 85 ^ 01 = 87
    assert_eq!(r::report(&r::read_dpi(1), r::TID_1F, 91), rz(0x1f, 0x07, 0x04, 0x85, &[0x01], 0x87));
    // razerReadDpiCommand(0x00) - DeathAdder V2's dpiStorageByte, and the Cobra / Viper Mini DPI read: 07 ^ 04 ^ 85 = 86
    assert_eq!(r::report(&r::read_dpi(0), r::TID_3F, 91), rz(0x3f, 0x07, 0x04, 0x85, &[], 0x86));
    // RAZER_READ.pollingRate {0x00, 0x85, 0x01}: 01 ^ 00 ^ 85 = 84
    assert_eq!(r::report(&r::read_polling_legacy(), r::TID_FF, 91), rz(0xff, 0x01, 0x00, 0x85, &[], 0x84));
    // RAZER_READ.pollingRateExtended {0x00, 0xc0, 0x02, [00]}: 02 ^ 00 ^ c0 = c2
    assert_eq!(r::report(&r::read_polling_extended(), r::TID_1F, 91), rz(0x1f, 0x02, 0x00, 0xc0, &[0x00], 0xc2));
    // Viper V4 Pro: command(0x00, 0xc0, 2, [1]) on buildRazerV4Report's fixed tid 0x1f: 02 ^ c0 ^ 01 = c3
    assert_eq!(r::report(&r::v4_read_polling(), r::TID_1F, 91), rz(0x1f, 0x02, 0x00, 0xc0, &[0x01], 0xc3));
    // Viper V4 Pro readDpi: command(0x04, 0x86, 80): 50 ^ 04 ^ 86 = d2
    assert_eq!(r::report(&r::v4_read_dpi_stages(), r::TID_1F, 91), rz(0x1f, 0x50, 0x04, 0x86, &[], 0xd2));
    // Viper V4 Pro battery / charging: command(0x07, 0x80 | 0x84, 2, [0, 0]) - zero arguments, so the very same bytes
    assert_eq!(r::report(&r::read_battery(), r::TID_1F, 91), rz(0x1f, 0x02, 0x07, 0x80, &[0, 0], 0x85));
    // Windows wants the collection's FeatureReportByteLength: zero-padded behind the 91 bytes
    let long = r::report(&r::read_firmware(), r::TID_1F, 96);
    assert_eq!((long.len(), &long[..91], &long[91..]), (96, &rz(0x1f, 0x02, 0x00, 0x81, &[], 0x83)[..], &[0u8; 5][..]));
}

// ---------------------------------------------------------------- encode: writes

#[test]
fn writes_are_openmouses_encoderazerrequest_bytes() {
    // razerSetDpiCommand(1600 = 0x0640, 1600, 1): {0x04, 0x05, 0x07, [01 06 40 06 40 00 00]}
    // crc: 07 ^ 04 = 03, ^ 05 = 06, ^ 01 = 07, ^ 06 = 01, ^ 40 = 41, ^ 06 = 47, ^ 40 = 07
    assert_eq!(r::report(&r::set_dpi(1600, 1600, 1), r::TID_1F, 91), rz(0x1f, 0x07, 0x04, 0x05, &[0x01, 0x06, 0x40, 0x06, 0x40, 0, 0], 0x07));
    // razerSetDpiCommand(800 = 0x0320, 800, 0x00) - DeathAdder V2: 06 ^ 00 ^ 03 ^ 20 ^ 03 ^ 20 = 06
    assert_eq!(r::report(&r::set_dpi(800, 800, 0), r::TID_3F, 91), rz(0x3f, 0x07, 0x04, 0x05, &[0x00, 0x03, 0x20, 0x03, 0x20, 0, 0], 0x06));
    // razerSetLegacyPollingCommand(500): {0x00, 0x05, 0x01, [1000 / 500 = 02]}: 01 ^ 05 ^ 02 = 06
    assert_eq!(r::report(&r::set_polling_legacy(500).unwrap(), r::TID_FF, 91), rz(0xff, 0x01, 0x00, 0x05, &[0x02], 0x06));
    // razerSetExtendedPollingCommand(4000): {0x00, 0x40, 0x02, [00, 8000 / 4000 = 02]}: 02 ^ 40 ^ 02 = 40
    assert_eq!(r::report(&r::set_polling_extended(4000).unwrap(), r::TID_1F, 91), rz(0x1f, 0x02, 0x00, 0x40, &[0x00, 0x02], 0x40));
    // razerSetExtendedPollingCommand(1000): [00, 08]: 02 ^ 40 ^ 08 = 4a
    assert_eq!(r::report(&r::set_polling_extended(1000).unwrap(), r::TID_1F, 91), rz(0x1f, 0x02, 0x00, 0x40, &[0x00, 0x08], 0x4a));
    // Viper V4 Pro setPollingRate(8000): command(0x00, 0x40, 2, [1, code 0x01]): 02 ^ 40 ^ 01 ^ 01 = 42
    assert_eq!(r::report(&r::v4_set_polling(8000).unwrap(), r::TID_1F, 91), rz(0x1f, 0x02, 0x00, 0x40, &[0x01, 0x01], 0x42));
    // setPollingRate(1000): code 0x08: 02 ^ 40 ^ 01 ^ 08 = 4b
    assert_eq!(r::report(&r::v4_set_polling(1000).unwrap(), r::TID_1F, 91), rz(0x1f, 0x02, 0x00, 0x40, &[0x01, 0x08], 0x4b));
    // Viper V4 Pro writeDpi: stages 800 / 3200 (0x0c80), stage 2 active: args [01, 02, 02, 00 03 20 03 20 00 00, 01 0c 80 0c
    // 80 00 00], data size 3 + 2 * 7 = 0x11. crc: 11 ^ 04 ^ 06 = 13, and the arguments XOR to 0 (every value comes twice;
    // 01 ^ 00 ^ 01 left over = 0)
    let stages = r::v4_set_dpi_stages(1, &[(800, 800), (3200, 3200)]);
    let args = [0x01, 0x02, 0x02, 0x00, 0x03, 0x20, 0x03, 0x20, 0, 0, 0x01, 0x0c, 0x80, 0x0c, 0x80, 0, 0];
    assert_eq!(r::report(&stages, r::TID_1F, 91), rz(0x1f, 0x11, 0x04, 0x06, &args, 0x13));
}

#[test]
fn polling_encodings_are_openmouses() {
    // pollingDivisor(1000, rate): 125 -> 8, 500 -> 2, 1000 -> 1; a rate that is no whole divisor is refused
    for (hz, d) in [(125, 8), (250, 4), (500, 2), (1000, 1)] {
        assert_eq!(r::set_polling_legacy(hz).map(|c| c.args), Some(vec![d]), "{hz}");
    }
    assert!(r::set_polling_legacy(2000).is_none() && r::set_polling_legacy(300).is_none());
    // pollingDivisor(8000, rate)
    for (hz, d) in [(125, 64), (500, 16), (1000, 8), (2000, 4), (4000, 2), (8000, 1)] {
        assert_eq!(r::set_polling_extended(hz).map(|c| c.args), Some(vec![0, d]), "{hz}");
    }
    assert!(r::set_polling_extended(3000).is_none() && r::set_polling_extended(16000).is_none());
    // RAZER_V4_POLLING_CODES
    for (hz, code) in [(125, 0x40), (500, 0x10), (1000, 0x08), (2000, 0x04), (4000, 0x02), (8000, 0x01)] {
        assert_eq!((r::v4_polling_code(hz), r::v4_polling_hz(code)), (Some(code), Some(hz)), "{hz}");
    }
    assert_eq!((r::v4_polling_code(250), r::v4_polling_hz(0x20)), (None, None));
}

// ---------------------------------------------------------------- decode

#[test]
fn answers_decode_like_openmouse() {
    // decodeDpi: x = args[1] << 8 | args[2], y = args[3] << 8 | args[4]
    assert_eq!(r::decode_dpi(&[0x01, 0x06, 0x40, 0x03, 0x20, 0, 0]), Some((1600, 800)));
    // decodeBatteryPercent: Math.round(args[1] * 100 / 255): 0xb4 = 180 -> 70.59 -> 71; 255 -> 100; 0 -> 0
    assert_eq!((r::battery_percent(&[0, 0xb4]), r::battery_percent(&[0, 0xff]), r::battery_percent(&[0, 0])), (Some(71), Some(100), Some(0)));
    // decodeCharging: args[1] === 1
    assert_eq!((r::charging(&[0, 1]), r::charging(&[0, 0]), r::charging(&[1, 2])), (Some(true), Some(false), Some(false)));
    // decodeLegacyPollingRate: Math.round(1000 / args[0]); 0 = "unknown polling rate"
    assert_eq!((r::legacy_polling_hz(&[1]), r::legacy_polling_hz(&[8]), r::legacy_polling_hz(&[3]), r::legacy_polling_hz(&[0])), (Some(1000), Some(125), Some(333), None));
    // decodeExtendedPollingRate: Math.round(8000 / args[1]) (args[0] echoes the request): 8000 / 128 = 62.5 -> 63
    assert_eq!((r::extended_polling_hz(&[0, 2]), r::extended_polling_hz(&[0, 0x80]), r::extended_polling_hz(&[0, 0])), (Some(4000), Some(63), None));
    // decodeRazerV4DpiState: count = min(args[2], 5), stage i at 3 + 7i, active = args[1] - 1 clamped
    let mut a = vec![0u8; 80];
    a[..17].copy_from_slice(&[0x01, 0x02, 0x02, 0x00, 0x03, 0x20, 0x03, 0x20, 0, 0, 0x01, 0x06, 0x40, 0x06, 0x40, 0, 0]);
    assert_eq!(r::v4_dpi_state(&a), Some((1, vec![(800, 800), (1600, 1600)])));
    a[1] = 9;
    assert_eq!(r::v4_dpi_state(&a).map(|s| s.0), Some(1), "a corrupt active index is clamped to the last stage");
    a[2] = 0;
    assert_eq!(r::v4_dpi_state(&a), None, "no stages = OpenMouse's 'reported no DPI stages'");
}

#[test]
fn replies_are_checked_like_decoderazerresponse() {
    let cmd = r::read_dpi(1);
    // status 0x02 (ok), echoed class / id, data size 7: the arguments are bytes 8..15. crc 07 ^ 04 ^ 85 ^ 01 ^ 06 ^ 40 ^ 06 ^ 40 = 87
    let ok = reply(0x02, 0x1f, 0x07, 0x04, 0x85, &[0x01, 0x06, 0x40, 0x06, 0x40, 0, 0], 0x87);
    assert_eq!(r::decode_reply(&ok, &cmd), Ok(vec![0x01, 0x06, 0x40, 0x06, 0x40, 0, 0]));
    // busy (0x01): the same request answers later
    assert_eq!(r::decode_reply(&reply(0x01, 0x1f, 0x07, 0x04, 0x85, &[0x01], 0x87), &cmd), Err(r::Reply::Busy));
    // unsupported (0x05) / failure (0x03): the mouse's own answer
    assert_eq!(r::decode_reply(&reply(0x05, 0x1f, 0x07, 0x04, 0x85, &[0x01], 0x87), &cmd), Err(r::Reply::Status(0x05)));
    assert_eq!(r::decode_reply(&reply(0x03, 0x1f, 0x07, 0x04, 0x85, &[0x01], 0x87), &cmd), Err(r::Reply::Status(0x03)));
    // a bad crc or a wrong length carries no status: transport corruption
    assert_eq!(r::decode_reply(&reply(0x02, 0x1f, 0x07, 0x04, 0x85, &[0x01], 0x88), &cmd), Err(r::Reply::Corrupt));
    assert_eq!(r::decode_reply(&ok[..89], &cmd), Err(r::Reply::Corrupt));
    // the answer to another command (battery: 02 ^ 07 ^ 80 = 85): stale
    assert_eq!(r::decode_reply(&reply(0x02, 0x1f, 0x02, 0x07, 0x80, &[], 0x85), &cmd), Err(r::Reply::Stale));
}

// ---------------------------------------------------------------- the read-only allow-list

#[test]
fn only_openmouses_read_requests_pass_the_allow_list() {
    for read in [
        rz(0x1f, 0x02, 0x00, 0x81, &[], 0x83),
        rz(0xff, 0x02, 0x07, 0x80, &[], 0x85),
        rz(0x3f, 0x02, 0x07, 0x84, &[], 0x81),
        rz(0x1f, 0x07, 0x04, 0x85, &[0x01], 0x87),
        rz(0x3f, 0x07, 0x04, 0x85, &[], 0x86),
        rz(0xff, 0x01, 0x00, 0x85, &[], 0x84),
        rz(0x1f, 0x02, 0x00, 0xc0, &[], 0xc2),
        rz(0x1f, 0x02, 0x00, 0xc0, &[0x01], 0xc3),
        rz(0x1f, 0x50, 0x04, 0x86, &[], 0xd2),
    ] {
        assert!(r::read_request_allowed(&read), "{read:02x?}");
    }
    // a longer collection: zero padding is fine, anything else is not
    let mut padded = rz(0x1f, 0x02, 0x00, 0x81, &[], 0x83);
    padded.extend_from_slice(&[0, 0, 0]);
    assert!(r::read_request_allowed(&padded));
    padded[92] = 1;
    assert!(!r::read_request_allowed(&padded));
    // every write is refused
    for write in [
        rz(0x1f, 0x07, 0x04, 0x05, &[0x01, 0x06, 0x40, 0x06, 0x40, 0, 0], 0x07),
        rz(0xff, 0x01, 0x00, 0x05, &[0x02], 0x06),
        rz(0x1f, 0x02, 0x00, 0x40, &[0x00, 0x02], 0x40),
        rz(0x1f, 0x02, 0x00, 0x40, &[0x01, 0x01], 0x42),
        rz(0x1f, 0x11, 0x04, 0x06, &[0x01, 0x02, 0x02, 0x00, 0x03, 0x20, 0x03, 0x20, 0, 0, 0x01, 0x0c, 0x80, 0x0c, 0x80, 0, 0], 0x13),
        // sensor setting (tracking distance) 0x0b / 0x0b: 04 ^ 0b ^ 0b ^ 00 ^ 04 ^ 01 ^ 01 = 00; sleep timeout 0x07 / 0x03
        rz(0x1f, 0x04, 0x0b, 0x0b, &[0x00, 0x04, 0x01, 0x01], 0x00),
        rz(0x1f, 0x02, 0x07, 0x03, &[0x01, 0x2c], 0x02 ^ 0x07 ^ 0x03 ^ 0x01 ^ 0x2c),
    ] {
        assert!(!r::read_request_allowed(&write), "{write:02x?}");
    }
    // a read this app never sends (serial 0x00 / 0x82, size 0x16: 16 ^ 82 = 94) is not on the list either
    assert!(!r::read_request_allowed(&rz(0x1f, 0x16, 0x00, 0x82, &[], 0x94)));
    // malformed: bad crc, report id, short, a status byte, an unknown transaction id, a changed argument
    assert!(!r::read_request_allowed(&rz(0x1f, 0x02, 0x00, 0x81, &[], 0x84)));
    let mut id = rz(0x1f, 0x02, 0x00, 0x81, &[], 0x83);
    id[0] = 8;
    assert!(!r::read_request_allowed(&id));
    assert!(!r::read_request_allowed(&rz(0x1f, 0x02, 0x00, 0x81, &[], 0x83)[..90]));
    let mut st = rz(0x1f, 0x02, 0x00, 0x81, &[], 0x83);
    st[1] = 2;
    assert!(!r::read_request_allowed(&st));
    assert!(!r::read_request_allowed(&rz(0x10, 0x02, 0x00, 0x81, &[], 0x83)));
    assert!(!r::read_request_allowed(&rz(0x1f, 0x07, 0x04, 0x85, &[0x02], 0x84)));
}

// ---------------------------------------------------------------- a scripted Razer mouse

/// A fake Razer mouse answering the 90-byte protocol from a little state, for the driver tests. It answers status 0x02
/// with the request's class / id / size echoed; `busy` answers 0x01 that many times first; `corrupt` breaks that many
/// answers' crc; `unsupported` lists (class, id) it answers with 0x05.
struct Mouse {
    tid: u8,
    dpi: (u16, u16),
    legacy: u8,
    extended: u8,
    battery: u8,
    charging: u8,
    stages: Vec<(u16, u16)>,
    active: u8,
    busy: u32,
    corrupt: u32,
    unsupported: Vec<(u8, u8)>,
    pending: Vec<u8>,
}

impl Mouse {
    fn new(tid: u8) -> Self {
        Mouse { tid, dpi: (800, 800), legacy: 1, extended: 8, battery: 0xb4, charging: 1, stages: vec![(800, 800), (1600, 1600)], active: 2, busy: 0, corrupt: 0, unsupported: Vec::new(), pending: Vec::new() }
    }

    fn answer(&mut self, out: &[u8], how: &HidTransfer) -> Vec<u8> {
        if let HidTransfer::Feature { .. } = how {
            assert_eq!(out.len(), 91);
            let p = &out[1..91];
            assert_eq!(p[1], self.tid, "a wrong transaction id: a real mouse would never answer");
            let (size, class, id, args) = (p[5], p[6], p[7], &p[8..88]);
            let mut a = vec![0u8; 80];
            let mut status = 0x02;
            match (class, id) {
                _ if self.unsupported.contains(&(class, id)) => status = 0x05,
                (0x00, 0x81) => a[..2].copy_from_slice(&[1, 12]),
                (0x04, 0x85) => a[..5].copy_from_slice(&[args[0], (self.dpi.0 >> 8) as u8, self.dpi.0 as u8, (self.dpi.1 >> 8) as u8, self.dpi.1 as u8]),
                (0x04, 0x05) => self.dpi = (u16::from_be_bytes([args[1], args[2]]), u16::from_be_bytes([args[3], args[4]])),
                (0x00, 0x85) => a[0] = self.legacy,
                (0x00, 0x05) => self.legacy = args[0],
                (0x00, 0xc0) => a[..2].copy_from_slice(&[args[0], self.extended]),
                (0x00, 0x40) => self.extended = args[1],
                (0x07, 0x80) => a[1] = self.battery,
                (0x07, 0x84) => a[1] = self.charging,
                (0x04, 0x86) => {
                    a[..3].copy_from_slice(&[args[0], self.active, self.stages.len() as u8]);
                    for (i, (x, y)) in self.stages.iter().enumerate() {
                        let o = 3 + i * 7;
                        a[o..o + 5].copy_from_slice(&[i as u8, (x >> 8) as u8, *x as u8, (y >> 8) as u8, *y as u8]);
                    }
                }
                (0x04, 0x06) => {
                    self.active = args[1];
                    self.stages = (0..args[2] as usize).map(|i| (u16::from_be_bytes([args[4 + i * 7], args[5 + i * 7]]), u16::from_be_bytes([args[6 + i * 7], args[7 + i * 7]]))).collect();
                }
                _ => panic!("the fake does not know {class:#04x} / {id:#04x}"),
            }
            let mut q = vec![0u8; 90];
            q[0] = status;
            q[1] = p[1];
            q[5] = size;
            q[6] = class;
            q[7] = id;
            q[8..88].copy_from_slice(&a);
            q[88] = q[2..88].iter().fold(0, |c, b| c ^ b);
            self.pending = q;
        }
        let mut q = self.pending.clone();
        if self.busy > 0 {
            self.busy -= 1;
            q[0] = 0x01;
        } else if self.corrupt > 0 {
            self.corrupt -= 1;
            q[88] ^= 0xff;
        }
        let mut v = vec![0u8];
        v.extend_from_slice(&q);
        v
    }
}

fn port_for(info: &HidInfo, mouse: Rc<RefCell<Mouse>>) -> ScriptPort {
    ScriptPort::new(info.clone(), move |out, how| Ok(mouse.borrow_mut().answer(out, how)))
}

/// Razer's control collection on Windows: Generic Desktop Mouse with the 91-byte feature report (id 0 + 90).
fn control(pid: u16) -> HidInfo {
    hid_info("mi_00", 0x1532, pid, 0x01, 0x02, (8, 0, 91))
}

fn sent(port: &ScriptPort) -> Vec<Vec<u8>> {
    port.sent.iter().map(|s| s.0.clone()).collect()
}

// ---------------------------------------------------------------- driver: OpenMouse's generic RazerHidClient

#[test]
fn generic_viper_v3_pro_reads_and_writes_the_openmouse_way() {
    let info = control(0x00c1);
    let m = r::matches(&[&info], 0x1532, 0x00c1).unwrap();
    assert_eq!((m.family, m.model, m.wireless), (Family::Razer, Some("Razer Viper V3 Pro"), Some(true)));
    let mouse = Rc::new(RefCell::new(Mouse::new(0x1f)));
    let mut port = port_for(&info, mouse.clone());
    // readStatus: firmware, battery + charging (hasBattery), DPI (store 1), polling (extended first: highRatePolling)
    let s = r::Razer.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((s.online, s.dpi, s.polling_hz, s.battery_percent, s.charging, s.wireless), (true, Some(800), Some(1000), Some(71), Some(true), Some(true)));
    assert_eq!(
        sent(&port),
        vec![
            rz(0x1f, 0x02, 0x00, 0x81, &[], 0x83),
            rz(0x1f, 0x02, 0x07, 0x80, &[], 0x85),
            rz(0x1f, 0x02, 0x07, 0x84, &[], 0x81),
            rz(0x1f, 0x07, 0x04, 0x85, &[0x01], 0x87),
            rz(0x1f, 0x02, 0x00, 0xc0, &[], 0xc2),
        ]
    );
    assert!(port.sent.iter().all(|s| r::read_request_allowed(&s.0)), "reading = read requests only");
    // RESPONSE_DELAY_MS: the answer is fetched 100 ms after the request
    assert!(port.sent.iter().all(|s| s.1 == HidTransfer::Feature { reply_id: 0, reply_len: 91, wait_ms: 100 }));
    // caps: 100 .. maxDpi (35000) in whole DPI, RATES_8K on the receiver, battery, no lift-off
    assert_eq!((s.caps.dpi.as_ref().map(|d| (d.min(), d.max(), d.allows(1234))), s.caps.polling.clone(), s.caps.battery), (Some((100, 35000, true)), vec![125, 500, 1000, 2000, 4000, 8000], true));
    assert!(s.caps.lift_off.is_empty());

    // setDpi(1600): the DPI write, then the read-back (our "before" read first)
    port.sent.clear();
    let c = r::Razer.write(&m, &mut port, Setting::Dpi(1600)).unwrap();
    assert_eq!((c.now, c.before), (1600, Some(800)));
    let read_dpi = rz(0x1f, 0x07, 0x04, 0x85, &[0x01], 0x87);
    assert_eq!(sent(&port), vec![read_dpi.clone(), rz(0x1f, 0x07, 0x04, 0x05, &[0x01, 0x06, 0x40, 0x06, 0x40, 0, 0], 0x07), read_dpi]);

    // setPollingRate(4000): the extended write (divisor of 8000), then the read-back
    port.sent.clear();
    let c = r::Razer.write(&m, &mut port, Setting::Polling(4000)).unwrap();
    assert_eq!((c.now, c.before), (4000, Some(1000)));
    let read_ext = rz(0x1f, 0x02, 0x00, 0xc0, &[], 0xc2);
    assert_eq!(sent(&port), vec![read_ext.clone(), rz(0x1f, 0x02, 0x00, 0x40, &[0x00, 0x02], 0x40), read_ext]);
    assert!(!r::read_request_allowed(&port.sent[1].0), "a write is not a read request");

    // refused before a byte leaves: out of range, not one of this connection's rates, lift-off
    port.sent.clear();
    assert!(r::Razer.write(&m, &mut port, Setting::Dpi(99)).is_err());
    assert!(r::Razer.write(&m, &mut port, Setting::Dpi(35001)).is_err());
    assert!(r::Razer.write(&m, &mut port, Setting::Polling(250)).is_err());
    assert!(r::Razer.write(&m, &mut port, Setting::LiftOff(10)).is_err());
    assert!(port.sent.is_empty());
}

#[test]
fn generic_deathadder_v2_uses_its_own_store_tid_and_the_legacy_polling() {
    let info = control(0x0084);
    let m = r::matches(&[&info], 0x1532, 0x0084).unwrap();
    assert_eq!((m.model, m.wireless), (Some("Razer DeathAdder V2"), Some(false)));
    let mouse = Rc::new(RefCell::new(Mouse::new(0x3f)));
    // the cable answers the extended polling read as unsupported (codec: "Wired answers this command as unsupported")
    mouse.borrow_mut().unsupported.push((0x00, 0xc0));
    let mut port = port_for(&info, mouse.clone());
    let s = r::Razer.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((s.dpi, s.polling_hz, s.battery_percent, s.charging), (Some(800), Some(1000), None, None));
    // no battery on this wired model (hasBattery false); DPI through NOSTORE (dpiStorageByte 0x00); legacy polling first
    assert_eq!(sent(&port), vec![rz(0x3f, 0x02, 0x00, 0x81, &[], 0x83), rz(0x3f, 0x07, 0x04, 0x85, &[], 0x86), rz(0x3f, 0x01, 0x00, 0x85, &[], 0x84)]);
    assert_eq!((s.caps.polling.clone(), s.caps.battery, s.caps.dpi.map(|d| d.max())), (vec![125, 500, 1000], false, Some(20000)));
    port.sent.clear();
    let c = r::Razer.write(&m, &mut port, Setting::Dpi(1600)).unwrap();
    assert_eq!((c.now, c.before), (1600, Some(800)));
    // razerSetDpiCommand(1600, 1600, 0x00): 07 ^ 04 ^ 05 = 06, then 00 ^ 06 ^ 40 ^ 06 ^ 40 = 00: crc 06
    assert_eq!(port.sent[1].0, rz(0x3f, 0x07, 0x04, 0x05, &[0x00, 0x06, 0x40, 0x06, 0x40, 0, 0], 0x06));
    // setPollingRate(500): razerSetLegacyPollingCommand, divisor 2 (crc 06)
    port.sent.clear();
    let c = r::Razer.write(&m, &mut port, Setting::Polling(500)).unwrap();
    assert_eq!((c.now, c.before), (500, Some(1000)));
    assert_eq!(port.sent[1].0, rz(0x3f, 0x01, 0x00, 0x05, &[0x02], 0x06));
    assert!(r::Razer.write(&m, &mut port, Setting::Polling(2000)).is_err(), "RATES_1K on this model");
}

#[test]
fn generic_exchange_rereads_a_busy_answer_and_resends_after_a_corrupt_one() {
    let info = control(0x00b7);
    let m = r::matches(&[&info], 0x1532, 0x00b7).unwrap();
    let mouse = Rc::new(RefCell::new(Mouse::new(0x1f)));
    let mut port = port_for(&info, mouse.clone());
    // busy twice: OpenMouse's awaitReply reads again (100 ms apart) without sending the request again
    mouse.borrow_mut().busy = 2;
    let c = r::Razer.write(&m, &mut port, Setting::Dpi(1600)).unwrap();
    assert_eq!(c.before, Some(800));
    let kinds: Vec<&HidTransfer> = port.sent.iter().map(|s| &s.1).collect();
    assert_eq!(kinds[..3], [&HidTransfer::Feature { reply_id: 0, reply_len: 91, wait_ms: 100 }, &HidTransfer::FeatureIn { reply_id: 0, reply_len: 91, wait_ms: 100 }, &HidTransfer::FeatureIn { reply_id: 0, reply_len: 91, wait_ms: 100 }]);
    assert!(port.sent[..3].iter().all(|s| s.0 == rz(0x1f, 0x07, 0x04, 0x85, &[0x01], 0x87)), "the re-reads name the same request");
    assert_eq!(port.sent.len(), 3 + 2, "then the write and its read-back");
    // a corrupt answer is a lost exchange: OpenMouse's exchange sends the request again
    port.sent.clear();
    mouse.borrow_mut().corrupt = 1;
    let s = r::Razer.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert!(s.online);
    assert_eq!((&port.sent[0].1, &port.sent[1].1), (&HidTransfer::Feature { reply_id: 0, reply_len: 91, wait_ms: 100 }, &HidTransfer::Feature { reply_id: 0, reply_len: 91, wait_ms: 100 }));
    assert_eq!(port.sent[0].0, port.sent[1].0);
    // the DeathAdder V3 Pro receiver uses the legacy command (highRatePolling false) and RATES_1K
    assert_eq!((s.polling_hz, s.caps.polling.clone()), (Some(1000), vec![125, 500, 1000]));
    assert!(port.sent.iter().any(|x| x.0 == rz(0x1f, 0x01, 0x00, 0x85, &[], 0x84)));
}

#[test]
fn a_mouse_that_does_not_answer_is_offline_not_an_error() {
    let info = control(0x00a6);
    let m = r::matches(&[&info], 0x1532, 0x00a6).unwrap();
    let mouse = Rc::new(RefCell::new(Mouse::new(0x1f)));
    mouse.borrow_mut().busy = 1000;
    let mut port = port_for(&info, mouse);
    let s = r::Razer.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert!(!s.online && s.dpi.is_none());
    // RESPONSE_ATTEMPTS sends x RESPONSE_ATTEMPTS reads of the firmware request, nothing else
    assert_eq!(port.sent.len(), 36);
    assert_eq!(port.sent.iter().filter(|s| matches!(s.1, HidTransfer::Feature { .. })).count(), 6);
    assert!(port.sent.iter().all(|s| s.0 == rz(0x1f, 0x02, 0x00, 0x81, &[], 0x83)));
}

// ---------------------------------------------------------------- driver: Cobra and Viper Mini (their own OpenMouse clients)

#[test]
fn cobra_reads_with_nostore_and_writes_with_the_store() {
    // the Cobra also accepts a vendor collection (isControlInterface)
    let vendor = hid_info("mi_02", 0x1532, 0x00a3, 0xFF00, 0x01, (0, 0, 91));
    let m = r::matches(&[&vendor], 0x1532, 0x00a3).unwrap();
    assert_eq!((m.model, m.wireless, m.config.path.as_str()), (Some("Razer Cobra"), Some(false), "mi_02"));
    let mouse = Rc::new(RefCell::new(Mouse::new(0xff)));
    let mut port = port_for(&vendor, mouse.clone());
    let s = r::Razer.read(&m, &mut port, ReadOptions::default()).unwrap();
    // COBRA_TRANSACTION_ID 0xff: firmware, COBRA_DPI_READ (store 0x00), the legacy polling read; no battery
    assert_eq!(sent(&port), vec![rz(0xff, 0x02, 0x00, 0x81, &[], 0x83), rz(0xff, 0x07, 0x04, 0x85, &[], 0x86), rz(0xff, 0x01, 0x00, 0x85, &[], 0x84)]);
    assert_eq!((s.dpi, s.polling_hz, s.caps.battery, s.caps.dpi.map(|d| d.max())), (Some(800), Some(1000), false, Some(8500)));
    port.sent.clear();
    let c = r::Razer.write(&m, &mut port, Setting::Dpi(1600)).unwrap();
    assert_eq!((c.now, c.before), (1600, Some(800)));
    // razerSetDpiCommand(dpi, dpiY) with its default store 0x01, read back through COBRA_DPI_READ (0x00)
    assert_eq!(sent(&port), vec![rz(0xff, 0x07, 0x04, 0x85, &[], 0x86), rz(0xff, 0x07, 0x04, 0x05, &[0x01, 0x06, 0x40, 0x06, 0x40, 0, 0], 0x07), rz(0xff, 0x07, 0x04, 0x85, &[], 0x86)]);
    // the Cobra's exchange sends once and only re-reads on busy; a corrupt answer is an error, not a re-send
    port.sent.clear();
    mouse.borrow_mut().corrupt = 1;
    assert!(r::Razer.write(&m, &mut port, Setting::Polling(500)).is_err());
    assert_eq!(port.sent.len(), 1);
}

#[test]
fn viper_mini_is_the_mouse_collection_only() {
    let vendor = hid_info("mi_02", 0x1532, 0x008a, 0xFF00, 0x01, (0, 0, 91));
    assert!(r::matches(&[&vendor], 0x1532, 0x008a).is_none(), "isControlInterface: the only collection is Generic Desktop Mouse");
    let info = control(0x008a);
    let m = r::matches(&[&vendor, &info], 0x1532, 0x008a).unwrap();
    assert_eq!((m.model, m.config.path.as_str()), (Some("Razer Viper Mini"), "mi_00"));
    let mouse = Rc::new(RefCell::new(Mouse::new(0xff)));
    let mut port = port_for(&info, mouse);
    // setPollingRate(500): razerSetLegacyPollingCommand, read back with the legacy read
    let c = r::Razer.write(&m, &mut port, Setting::Polling(500)).unwrap();
    assert_eq!((c.now, c.before), (500, Some(1000)));
    assert_eq!(sent(&port), vec![rz(0xff, 0x01, 0x00, 0x85, &[], 0x84), rz(0xff, 0x01, 0x00, 0x05, &[0x02], 0x06), rz(0xff, 0x01, 0x00, 0x85, &[], 0x84)]);
    port.sent.clear();
    let s = r::Razer.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((s.polling_hz, s.caps.polling.clone()), (Some(500), vec![125, 500, 1000]));
    assert!(port.sent.iter().all(|s| r::read_request_allowed(&s.0)));
}

// ---------------------------------------------------------------- driver: Viper V4 Pro (its own OpenMouse client)

#[test]
fn viper_v4_pro_reads_and_writes_the_openmouse_way() {
    // isSupported: one Generic Desktop / Consumer collection WITH the feature report; the plain pointer one has none
    let pointer = hid_info("mi_00", 0x1532, 0x00e6, 0x01, 0x02, (8, 0, 0));
    let ctl = hid_info("mi_02", 0x1532, 0x00e6, 0x0C, 0x01, (0, 0, 91));
    assert!(r::matches(&[&pointer], 0x1532, 0x00e6).is_none());
    let m = r::matches(&[&pointer, &ctl], 0x1532, 0x00e6).unwrap();
    assert_eq!((m.model, m.wireless, m.config.path.as_str()), (Some("Razer Viper V4 Pro"), Some(true), "mi_02"));
    let mouse = Rc::new(RefCell::new(Mouse::new(0x1f)));
    let mut port = port_for(&ctl, mouse.clone());
    // readStatus: DPI stage table, polling (selector 1), battery, charging (sleep / low power are not ported)
    let s = r::Razer.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((s.dpi, s.polling_hz, s.battery_percent, s.charging), (Some(1600), Some(1000), Some(71), Some(true)));
    assert_eq!(
        sent(&port),
        vec![rz(0x1f, 0x50, 0x04, 0x86, &[], 0xd2), rz(0x1f, 0x02, 0x00, 0xc0, &[0x01], 0xc3), rz(0x1f, 0x02, 0x07, 0x80, &[], 0x85), rz(0x1f, 0x02, 0x07, 0x84, &[], 0x81)]
    );
    assert!(port.sent.iter().all(|s| r::read_request_allowed(&s.0)));
    // the V4 client reads at once after the send (no initial delay)
    assert!(port.sent.iter().all(|s| s.1 == HidTransfer::Feature { reply_id: 0, reply_len: 91, wait_ms: 0 }));
    assert_eq!((s.caps.dpi.as_ref().map(|d| (d.min(), d.max(), d.allows(1625))), s.caps.polling.clone()), (Some((100, 50000, false)), vec![125, 500, 1000, 2000, 4000, 8000]));

    // setDpi(3200): readDpi, writeDpi (the active stage replaced), readDpi
    port.sent.clear();
    let c = r::Razer.write(&m, &mut port, Setting::Dpi(3200)).unwrap();
    assert_eq!((c.now, c.before), (3200, Some(1600)));
    let stages = [0x01, 0x02, 0x02, 0x00, 0x03, 0x20, 0x03, 0x20, 0, 0, 0x01, 0x0c, 0x80, 0x0c, 0x80, 0, 0];
    assert_eq!(sent(&port), vec![rz(0x1f, 0x50, 0x04, 0x86, &[], 0xd2), rz(0x1f, 0x11, 0x04, 0x06, &stages, 0x13), rz(0x1f, 0x50, 0x04, 0x86, &[], 0xd2)]);
    assert!(r::Razer.write(&m, &mut port, Setting::Dpi(3210)).is_err(), "50-DPI steps");

    // setPollingRate(8000): command(0x00, 0x40, 2, [1, 0x01]), the 150 ms settle, read back
    port.sent.clear();
    let c = r::Razer.write(&m, &mut port, Setting::Polling(8000)).unwrap();
    assert_eq!((c.now, c.before), (8000, Some(1000)));
    assert_eq!(sent(&port), vec![rz(0x1f, 0x02, 0x00, 0xc0, &[0x01], 0xc3), rz(0x1f, 0x02, 0x00, 0x40, &[0x01, 0x01], 0x42), rz(0x1f, 0x02, 0x00, 0xc0, &[0x01], 0xc3)]);

    // busy: re-read 35 ms later without sending again
    port.sent.clear();
    mouse.borrow_mut().busy = 1;
    r::Razer.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((&port.sent[0].1, &port.sent[1].1), (&HidTransfer::Feature { reply_id: 0, reply_len: 91, wait_ms: 0 }, &HidTransfer::FeatureIn { reply_id: 0, reply_len: 91, wait_ms: 35 }));
}

// ---------------------------------------------------------------- the device table and the matcher

#[test]
fn the_table_is_openmouses_supported_razer_list() {
    // (pid, model, wireless) for every "supported" Razer entry of supported-devices-data.ts; DeathAdder V3 (wired) has no
    // pids there - 0x00b2 is that model's row in devices.js
    let table = [
        (0x00b2, "Razer DeathAdder V3", false),
        (0x0084, "Razer DeathAdder V2", false),
        (0x007c, "Razer DeathAdder V2 Pro", false),
        (0x007d, "Razer DeathAdder V2 Pro", true),
        (0x00a3, "Razer Cobra", false),
        (0x00c0, "Razer Viper V3 Pro", false),
        (0x00c1, "Razer Viper V3 Pro", true),
        (0x00a5, "Razer Viper V2 Pro", false),
        (0x00a6, "Razer Viper V2 Pro", true),
        (0x006e, "Razer DeathAdder Essential", false),
        (0x0071, "Razer DeathAdder Essential", false),
        (0x0098, "Razer DeathAdder Essential", false),
        (0x008a, "Razer Viper Mini", false),
        (0x00b8, "Razer Viper V3 HyperSpeed", true),
        (0x00e5, "Razer Viper V4 Pro", false),
        (0x00e6, "Razer Viper V4 Pro", true),
        (0x00b6, "Razer DeathAdder V3 Pro", false),
        (0x00b7, "Razer DeathAdder V3 Pro", true),
        (0x00be, "Razer DeathAdder V4 Pro", false),
        (0x00bf, "Razer DeathAdder V4 Pro", true),
        (0x00ef, "Razer DeathAdder V4 Pro Carbon Fiber Edition", false),
        (0x00f0, "Razer DeathAdder V4 Pro Carbon Fiber Edition", true),
        (0x004c, "Razer Diamondback Chroma", false),
    ];
    assert_eq!(r::PRODUCTS.len(), table.len());
    for (pid, model, wireless) in table {
        let m = r::matches(&[&control(pid)], 0x1532, pid).unwrap_or_else(|| panic!("{pid:#06x}"));
        assert_eq!((m.family, m.model, m.wireless, m.variant), (Family::Razer, Some(model), Some(wireless), pid), "{pid:#06x}");
    }
    // transaction ids (devices.js TRANSACTION_3F / TRANSACTION_1F / the 0xff fallback; Cobra / Viper Mini 0xff)
    let tid = |pid: u16| r::product(pid).unwrap().tid;
    assert_eq!((tid(0x0084), tid(0x007d), tid(0x006e), tid(0x00b2), tid(0x00c1), tid(0x00ef), tid(0x004c), tid(0x00a3), tid(0x008a), tid(0x00e5)), (0x3f, 0x3f, 0x3f, 0x1f, 0x1f, 0x1f, 0xff, 0xff, 0xff, 0x1f));
    // "likely" / not listed: Viper V2 Pro HyperPolling dongle 0xb3, DeathAdder V3 Pro's other pair 0xc2 / 0xc3, Viper 0x78,
    // Basilisk V3 0x99, Cobra Pro 0xaf
    for pid in [0x00b3, 0x00c2, 0x00c3, 0x0078, 0x0099, 0x00af] {
        assert!(r::matches(&[&control(pid)], 0x1532, pid).is_none(), "{pid:#06x}");
    }
    assert!(r::matches(&[&control(0x00c1)], 0x1533, 0x00c1).is_none(), "Razer's vendor id only");
}

#[test]
fn the_interface_is_picked_the_way_openmouse_claims_it() {
    // generic: the Generic Desktop Mouse collection; on Windows the one that carries the 91-byte feature report wins
    let keyboard = hid_info("mi_01", 0x1532, 0x00c1, 0x01, 0x06, (8, 1, 0));
    let bare = hid_info("mi_02&col01", 0x1532, 0x00c1, 0x01, 0x02, (8, 0, 0));
    let ctl = control(0x00c1);
    assert_eq!(r::matches(&[&keyboard, &bare, &ctl], 0x1532, 0x00c1).map(|m| m.config.path), Some("mi_00".into()));
    assert_eq!(r::matches(&[&keyboard, &bare], 0x1532, 0x00c1).map(|m| m.config.path), Some("mi_02&col01".into()));
    assert!(r::matches(&[&keyboard], 0x1532, 0x00c1).is_none());
    // a vendor collection only for the vendorControlInterface models (DeathAdder Essential / V2 / V3, Diamondback Chroma)
    let vendor = |pid| hid_info("mi_03", 0x1532, pid, 0xFF00, 0x01, (0, 0, 91));
    assert!(r::matches(&[&vendor(0x00c1)], 0x1532, 0x00c1).is_none());
    for pid in [0x006e, 0x0084, 0x00b2, 0x004c] {
        assert_eq!(r::matches(&[&vendor(pid)], 0x1532, pid).map(|m| m.config.path), Some("mi_03".into()), "{pid:#06x}");
    }
    // and the whole-device lookup lands on the Razer family
    let all = [keyboard.clone(), ctl.clone()];
    assert_eq!(bu_mouse::family::find(&all, 0x1532, 0x00c1).map(|m| (m.family, m.config.path)), Some((Family::Razer, "mi_00".into())));
}

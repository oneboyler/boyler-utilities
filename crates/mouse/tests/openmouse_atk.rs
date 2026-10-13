//! Order 105 check, ATK / VXE: our requests = the bytes OpenMouse sends for the same setting (encode), and answers shaped the
//! way OpenMouse reads them decode to the same values (decode). The expected bytes are worked out by hand from OpenMouse
//! (@openmouse/protocol 0.40.1: `dist/atk/index.js`, `dist/drivers/atk/hid.js` + `bitmouse-hid.js`, `dist/bitmouse/index.js`,
//! `dist/endgame-gear/wireless.js` for the WE framing), never from our own encoder.
//!
//! How OpenMouse counts. hid.js: `sendReport(0x08, 16-byte frame)` - the frame is `[cmd, 0, hi, lo, len, data.., crc]` and
//! crc = 0x55 - (0x08 + the sum of the first 15 bytes), so on Windows the report is `08` + the 16 frame bytes (zero-padded to the
//! interface's output length). Every value is stored as a pair `[v, 0x55 - v]`, a DPI stage as `[x, y, mode, 0x55 - sum]`.
//! BITMOUSE: `sendReport(0x08, 63-byte frame)`, frame = `[checksum, 0x72, paramLen, 0, target, cmd, cmdLen, payload..]`, checksum
//! = the low byte of the sum of frame bytes 1..62; the answer is `[0x72, status, sn, target, cmd, len, payload..]` (Windows adds
//! the report id 8 in front of both).

use bu_mouse::atk::{self, Sensor};
use bu_mouse::error::Error;
use bu_mouse::family::{self, Changed, Driver, Family, Matched, OnMouse, ReadOptions, Setting};
use bu_mouse::fake::{hex, hid_info, ScriptPort};
use bu_mouse::os::{HidInfo, HidTransfer};
use std::cell::RefCell;
use std::rc::Rc;

const ATK: u16 = 0x373B;
const COMPX: u16 = 0x3554;

/// "08 10 00" -> bytes
fn h(s: &str) -> Vec<u8> {
    s.split_whitespace().map(|b| u8::from_str_radix(b, 16).expect("hex")).collect()
}

/// A hid.js report as Windows sends it: `08`, the frame prefix, zeros up to the crc byte (index 16), the crc worked out by hand.
fn f(prefix: &str, crc: u8) -> String {
    let mut v = vec![0x08];
    v.extend(h(prefix));
    v.resize(16, 0);
    v.push(crc);
    hex(&v)
}

/// A BITMOUSE report as Windows sends it: the bytes, zeros up to 64.
fn bm(prefix: &str) -> String {
    let mut v = h(prefix);
    v.resize(64, 0);
    hex(&v)
}

fn sent(p: &ScriptPort) -> Vec<String> {
    p.sent.iter().map(|(o, _)| hex(o)).collect()
}

// ------------------------------------------------------------------ hand-worked hid.js frames (crc by hand, see the notes)
//
// crc = 0x55 - ((0x08 + sum of frame bytes 0..14) & 0xff)
fn id_req() -> String {
    f("10", 0x3D) // 0x08 + 0x10 = 0x18; 0x55 - 0x18
}
fn batt_req() -> String {
    f("04", 0x49) // 0x08 + 0x04 = 0x0c; 0x55 - 0x0c
}
fn online_req() -> String {
    f("03", 0x4A) // 0x08 + 0x03 = 0x0b
}
fn rd_system6() -> String {
    f("08 00 00 00 06", 0x3F) // 0x08 + 0x08 + 0x06 = 0x16
}
fn rd_system10() -> String {
    f("08 00 00 00 0a", 0x3B) // 0x08 + 0x08 + 0x0a = 0x1a
}
fn rd_lift() -> String {
    f("08 00 00 0a 02", 0x39) // 0x08 + 0x08 + 0x0a + 2 = 0x1c
}
fn rd_stage0() -> String {
    f("08 00 00 0c 04", 0x35) // 0x08 + 0x08 + 0x0c + 4 = 0x20
}
fn rd_stage1() -> String {
    f("08 00 00 10 04", 0x31) // ... + 0x10 + 4 = 0x24
}
fn rd_group() -> String {
    f("08 00 00 0c 08", 0x31) // 0x08 + 0x08 + 0x0c + 8 = 0x24
}
fn rd_live() -> String {
    f("08 00 00 70 04", 0xD1) // 0x08 + 0x08 + 0x70 + 4 = 0x84; 0x55 - 0x84 = -0x2f = 0xd1
}

// ------------------------------------------------------------------ fake hid.js mice

/// A fake mouse of the hid.js family: a small EEPROM, a CID/MID, a battery, and the receiver's echo of a write.
struct HidMouse {
    mem: Vec<u8>,
    id: (u8, u8),
    awake: bool,
    online: bool,
    ignore_writes: bool,
    in_len: usize,
    /// every EEPROM write applied: (address, bytes)
    writes: Vec<(u16, Vec<u8>)>,
}

fn crc15(first15: &[u8]) -> u8 {
    (0x55u32.wrapping_sub((8 + first15.iter().map(|b| *b as u32).sum::<u32>()) & 0xFF) & 0xFF) as u8
}

impl HidMouse {
    /// Pro Max / A9 Plus layout (PAW3395): 1000 Hz, 4 stages, stage 1 active (800), stages 400 / 800 / 1600 / 3200, lift-off 1 mm.
    fn new(id: (u8, u8)) -> Self {
        let mut mem = vec![0u8; 0x100];
        mem[0..10].copy_from_slice(&h("01 54 04 51 01 54 00 55 11 44"));
        mem[0x0A..0x0C].copy_from_slice(&h("01 54"));
        mem[0x0C..0x1C].copy_from_slice(&h("07 07 00 47  0f 0f 00 37  1f 1f 00 17  3f 3f 00 d7"));
        Self { mem, id, awake: true, online: true, ignore_writes: false, in_len: 17, writes: Vec::new() }
    }
    /// The R1 SE+ layout (PAW3395SE codes): stages 400 / 800 / 200 / 200.
    fn new_se(id: (u8, u8)) -> Self {
        let mut m = Self::new(id);
        m.mem[0x0C..0x1C].copy_from_slice(&h("09 09 00 43  12 12 00 31  04 04 00 4d  04 04 00 4d"));
        m
    }
    fn reply(&self, f: &[u8]) -> Vec<u8> {
        let mut fr = f.to_vec();
        fr.resize(15, 0);
        fr.push(crc15(&fr));
        let mut v = vec![8];
        v.extend(fr);
        v.resize(self.in_len.max(17), 0);
        v
    }
    fn answer(&mut self, out: &[u8], how: &HidTransfer) -> Result<Vec<u8>, Error> {
        assert_eq!(out[0], 8, "report id");
        let fr = &out[1..17];
        assert_eq!(fr[15], crc15(&fr[..15]), "request crc");
        assert!(out[17..].iter().all(|b| *b == 0), "padding is zero");
        if !self.awake {
            return Err(Error::BadAnswer("the mouse did not answer in time".into()));
        }
        match fr[0] {
            0x10 => Ok(self.reply(&[0x10, 0, 0, 0, 2, self.id.0, self.id.1])),
            0x04 => Ok(self.reply(&[0x04, 0, 0, 0, 4, 87, 1, 0x0F, 0xA0])),
            0x03 => Ok(self.reply(&[0x03, 0, 0, 0, 1, self.online as u8])),
            0x08 => {
                let addr = u16::from_be_bytes([fr[2], fr[3]]) as usize;
                let len = fr[4] as usize;
                let mut r = vec![0x08, 0, fr[2], fr[3], fr[4]];
                r.extend_from_slice(&self.mem[addr..addr + len]);
                Ok(self.reply(&r))
            }
            0x07 => {
                let addr = u16::from_be_bytes([fr[2], fr[3]]);
                let len = fr[4] as usize;
                let data = fr[5..5 + len].to_vec();
                if !self.ignore_writes {
                    self.mem[addr as usize..addr as usize + len].copy_from_slice(&data);
                }
                self.writes.push((addr, data));
                match how {
                    HidTransfer::OutputOnly => Ok(Vec::new()),
                    _ => Ok(self.reply(&fr[..15])),
                }
            }
            c => panic!("unexpected command {c:#04x}"),
        }
    }
}

fn hid_if(vid: u16, pid: u16) -> HidInfo {
    hid_info("cfg", vid, pid, 0xFF02, 0x0002, (17, 17, 0))
}

/// Matched for a table entry + a port on a fake hid.js mouse.
fn hid_port(vid: u16, pid: u16, mouse: HidMouse) -> (Matched, ScriptPort, Rc<RefCell<HidMouse>>) {
    let info = hid_if(vid, pid);
    let m = atk::matches(&[&info], vid, pid).expect("in the table");
    let st = Rc::new(RefCell::new(mouse));
    let s2 = st.clone();
    (m, ScriptPort::new(info, move |o, how| s2.borrow_mut().answer(o, how)), st)
}

const ONE: ReadOptions = ReadOptions { online_tries: 1 };

fn read(m: &Matched, p: &mut ScriptPort) -> OnMouse {
    atk::Atk.read(m, p, ONE).expect("read")
}

// ================================================================== codec: hid.js

#[test]
fn hid_read_requests_are_openmouses() {
    // weBuildCmdPayload(cmd, args): payload[0] = cmd, args after it, payload[15] = crc over (0x08 + the first 15 bytes)
    assert_eq!(hex(&atk::hid_wire(&atk::hid_frame(0x10, &[]), 17)), id_req());
    assert_eq!(hex(&atk::hid_wire(&atk::hid_frame(0x04, &[]), 17)), batt_req());
    assert_eq!(hex(&atk::hid_wire(&atk::hid_frame(0x03, &[]), 17)), online_req());
    // read(address, length) = weBuildCmdPayload(0x08, [0, address >> 8, address & 0xff, length])
    assert_eq!(hex(&atk::hid_wire(&atk::read_eeprom_frame(0x0000, 6), 17)), rd_system6());
    assert_eq!(hex(&atk::hid_wire(&atk::read_eeprom_frame(0x0000, 10), 17)), rd_system10());
    assert_eq!(hex(&atk::hid_wire(&atk::read_eeprom_frame(0x000A, 2), 17)), rd_lift());
    assert_eq!(hex(&atk::hid_wire(&atk::read_eeprom_frame(0x000C, 4), 17)), rd_stage0());
    assert_eq!(hex(&atk::hid_wire(&atk::read_eeprom_frame(0x0010, 4), 17)), rd_stage1());
    assert_eq!(hex(&atk::hid_wire(&atk::read_eeprom_frame(0x000C, 8), 17)), rd_group());
    assert_eq!(hex(&atk::hid_wire(&atk::read_eeprom_frame(0x0070, 4), 17)), rd_live());
    // Windows wants the whole output report: padded with zeros
    let w = atk::hid_wire(&atk::hid_frame(0x10, &[]), 65);
    assert_eq!((w.len(), w[0], w[1], w[16], w[17..].iter().all(|b| *b == 0)), (65, 0x08, 0x10, 0x3D, true));
}

#[test]
fn hid_write_frames_are_openmouses() {
    // setPollingRate: write(0x0000, wePackScalarPair(code)); sum = 8 + 7 + 2 + (code + 0x55 - code = 0x55) = 0x66 for ANY code
    // (address 0), so crc = 0x55 - 0x66 = 0xef. Codes: 125 -> 0x08, 250 -> 0x04, 500 -> 0x02, 1000 -> 0x01.
    for (hz, code) in [(125u32, 0x08u8), (250, 0x04), (500, 0x02), (1000, 0x01)] {
        assert_eq!(atk::hid_polling_code(hz), Some(code), "{hz}");
        let want = f(&format!("07 00 00 00 02 {code:02x} {:02x}", 0x55 - code), 0xEF);
        assert_eq!(hex(&atk::hid_wire(&atk::write_eeprom_frame(0, &atk::scalar_pair(code)), 17)), want, "{hz} Hz");
    }
    // lift-off (R1 codes Low = 1, High = 2) at 0x000a: 8 + 7 + 0x0a + 2 + 0x55 = 0x70 -> crc 0x55 - 0x70 = 0xe5
    assert_eq!(hex(&atk::hid_wire(&atk::write_eeprom_frame(0x000A, &atk::scalar_pair(1)), 17)), f("07 00 00 0a 02 01 54", 0xE5));
    assert_eq!(hex(&atk::hid_wire(&atk::write_eeprom_frame(0x000A, &atk::scalar_pair(2)), 17)), f("07 00 00 0a 02 02 53", 0xE5));
    // a DPI stage record sums to 0x55 too: stage 0 (0x000c): 8 + 7 + 0x0c + 4 + 0x55 = 0x74 -> 0xe1; stage 1 (0x0010): 0x78 -> 0xdd
    assert_eq!(hex(&atk::hid_wire(&atk::write_eeprom_frame(0x000C, &h("0f 0f 00 37")), 17)), f("07 00 00 0c 04 0f 0f 00 37", 0xE1));
    assert_eq!(hex(&atk::hid_wire(&atk::write_eeprom_frame(0x0010, &h("1f 1f 00 17")), 17)), f("07 00 00 10 04 1f 1f 00 17", 0xDD));
    // the live row (R1 SE+ receiver): atkPackVxeR1LiveSetting(selector, value) = [selector, value, 0, 0x55 - value] at 0x0070.
    // polling selector 0x0b, 500 Hz = code 2: 8 + 7 + 0x70 + 4 + 0x0b + 2 + 0 + 0x53 = 0xe3 -> 0x72 (1000 Hz: code 1, same sum)
    assert_eq!(atk::pack_live_setting(0x0B, 2), [0x0B, 0x02, 0x00, 0x53]);
    assert_eq!(hex(&atk::hid_wire(&atk::write_eeprom_frame(0x0070, &atk::pack_live_setting(0x0B, 2)), 17)), f("07 00 00 70 04 0b 02 00 53", 0x72));
    assert_eq!((atk::live_polling_code(250), atk::live_polling_code(500), atk::live_polling_code(1000), atk::live_polling_code(125)), (Some(3), Some(2), Some(1), None));
    // lift-off selector 0x03, High = 2: 8 + 7 + 0x70 + 4 + 3 + 2 + 0x53 = 0xdb -> 0x55 - 0xdb = 0x7a (Low = 1: the same sum)
    assert_eq!(hex(&atk::hid_wire(&atk::write_eeprom_frame(0x0070, &atk::pack_live_setting(0x03, 2)), 17)), f("07 00 00 70 04 03 02 00 53", 0x7A));
    // the R1 Pro Max receiver's whole 10-byte system row, polling pair replaced, the tail copied: 8 + 7 + 0 + 0 + 10 + 5 pairs of
    // sum 0x55 = 25 + 425 = 450 = 0x1c2 -> 0xc2 -> crc 0x93
    assert_eq!(
        hex(&atk::hid_wire(&atk::write_eeprom_frame(0x0000, &h("02 53 04 51 01 54 00 55 11 44")), 17)),
        f("07 00 00 00 0a 02 53 04 51 01 54 00 55 11 44", 0x93)
    );
    // and its 8-byte DPI group (two stages): 8 + 7 + 0x0c + 8 + 0x55 + 0x55 = 205 = 0xcd -> 0x88
    assert_eq!(
        hex(&atk::hid_wire(&atk::write_eeprom_frame(0x000C, &h("07 07 00 47 1f 1f 00 17")), 17)),
        f("07 00 00 0c 08 07 07 00 47 1f 1f 00 17", 0x88)
    );
    // the pair helpers: [v, 0x55 - v] and the check
    assert_eq!((atk::scalar_pair(0x01), atk::scalar_pair(0xF0)), ([0x01, 0x54], [0xF0, 0x65]));
    assert_eq!((atk::unpack_pair(0x01, 0x54), atk::unpack_pair(0x01, 0x55)), (Some(1), None));
}

#[test]
fn polling_and_lift_off_tables_are_openmouses() {
    // POLLING_RATES of hid.js: 0x08 -> 125, 0x04 -> 250, 0x02 -> 500, 0x01 -> 1000, 0x10 -> 2000, 0x20 -> 4000, 0x40 -> 8000
    for (code, hz) in [(0x08u8, 125u32), (0x04, 250), (0x02, 500), (0x01, 1000), (0x10, 2000), (0x20, 4000), (0x40, 8000)] {
        assert_eq!((atk::hid_polling_hz(code), atk::hid_polling_code(hz)), (Some(hz), Some(code)));
    }
    assert_eq!((atk::hid_polling_hz(0x03), atk::hid_polling_code(1500)), (None, None));
    // R1_LIFT_OFF_CODES: [1, Low], [2, High] = 1 mm, 2 mm ("R1 lift-off levels are 1 mm and 2 mm")
    assert_eq!((atk::lift_off_code(10), atk::lift_off_code(20), atk::lift_off_code(7), atk::lift_off_code(15)), (Some(1), Some(2), None, None));
    assert_eq!((atk::lift_off_tenths(1), atk::lift_off_tenths(2), atk::lift_off_tenths(3), atk::lift_off_tenths(0)), (Some(10), Some(20), None, None));
    // bitmouse: [0, 1000], [1, 500], [2, 250], [3, 125], [4, 8000], [5, 4000], [6, 2000]
    for (code, hz) in [(0u8, 1000u32), (1, 500), (2, 250), (3, 125), (4, 8000), (5, 4000), (6, 2000)] {
        assert_eq!((atk::bm_polling_hz(code), atk::bm_polling_code(hz)), (Some(hz), Some(code)));
    }
    assert_eq!(atk::bm_polling_hz(7), None);
}

#[test]
fn dpi_stage_bytes_paw3395_are_openmouses() {
    // atkPackDpiStageForSensor("PAW3395") = step50: count = round(dpi / 50) - 1, byte = count & 0xff, nibble = (count >> 8 & 3) << 2;
    // mode = (nibble y << 4) | nibble x; crc = 0x55 - ((x + y + mode) & 0xff)
    // 800: count 15 = 0x0f, mode 0, sum 0x1e, crc 0x37.  1600: 31 = 0x1f, sum 0x3e, crc 0x17.  400: 7, sum 0x0e, crc 0x47.
    // 3200: 63 = 0x3f, sum 0x7e, crc 0xd7.  100: 1, sum 2, crc 0x53.
    // 30000: count 599 = 0x257 -> byte 0x57, nibble 2 << 2 = 8 -> mode 0x88; sum 0x57 + 0x57 + 0x88 = 0x136 -> 0x36; crc 0x1f
    for (dpi, want) in [
        (800, "0f 0f 00 37"),
        (1600, "1f 1f 00 17"),
        (400, "07 07 00 47"),
        (3200, "3f 3f 00 d7"),
        (100, "01 01 00 53"),
        (30000, "57 57 88 1f"),
    ] {
        let rec = atk::encode_dpi_stage(Sensor::Paw3395, dpi, dpi).unwrap_or_else(|| panic!("{dpi}"));
        assert_eq!(rec.to_vec(), h(want), "{dpi} DPI");
        // decodes to the same value (atkDecodeDpiAxisStep50: (count + 1) * 50)
        assert_eq!(atk::decode_dpi_stage(Sensor::Paw3395, &rec), Some((dpi, dpi)));
    }
    // x and y differ: 800 / 1600 -> [0x0f, 0x1f, 0, 0x55 - 0x2e = 0x27]
    assert_eq!(atk::encode_dpi_stage(Sensor::Paw3395, 800, 1600).map(|r| r.to_vec()), Some(h("0f 1f 00 27")));
    assert_eq!(atk::decode_dpi_stage(Sensor::Paw3395, &h("0f 1f 00 27")), Some((800, 1600)));
    // not offered: below 100, off the 50 grid, above 30000
    for dpi in [50, 125, 810, 30050, 30100, 0] {
        assert_eq!(atk::encode_dpi_stage(Sensor::Paw3395, dpi, dpi), None, "{dpi}");
    }
    // a record whose crc is wrong is not read
    assert_eq!(atk::decode_dpi_stage(Sensor::Paw3395, &h("0f 0f 00 38")), None);
    assert_eq!(atk::decode_dpi_stage(Sensor::Paw3395, &h("0f 0f 00")), None);
    let c = atk::dpi_caps(Sensor::Paw3395);
    assert_eq!((c.min(), c.max(), c.allows(800), c.allows(825), c.allows(30000), c.allows(30050)), (100, 30000, true, false, true, false));
}

#[test]
fn dpi_stage_bytes_paw3395_se_are_openmouses() {
    // atkEncodeDpiAxisPaw3395Se: base = dpi (or dpi / 2 above 10000, nibble 2); code = PAW3395SE_CODES[base / 50 - 1], where the
    // codes are 1..235 without 7, 13, 20, 26, 33, 40, 46, 53, ...
    // 200: index 3 -> code 4 (1, 2, 3, 4).  400: index 7 -> codes 1-6, 8, 9 -> 9.  800: index 15 -> 1-6, 8-12, 14-18 -> 18 = 0x12.
    // 10000: index 199 -> the last code, 235 = 0xeb (the invalid ones end at 233).
    // 10100: base 5050, index 100 -> 118 (118 - 17 invalid <= 118 = 101st code) = 0x76, nibble 2 -> mode 0x22
    // 18000: base 9000, index 179 -> 211 (211 - 31 invalid = 180th code) = 0xd3, mode 0x22
    // crc = 0x55 - ((x + y + mode) & 0xff): 200: 8 -> 0x4d; 400: 0x12 -> 0x43; 800: 0x24 -> 0x31; 10000: 0x1d6 -> 0xd6 -> 0x7f;
    // 10100: 0x76 + 0x76 + 0x22 = 0x10e -> 0x0e -> 0x47; 18000: 0xd3 + 0xd3 + 0x22 = 0x2c8 -> 0xc8 -> 0x8d
    for (dpi, want) in [
        (200, "04 04 00 4d"),
        (400, "09 09 00 43"),
        (800, "12 12 00 31"),
        (10000, "eb eb 00 7f"),
        (10100, "76 76 22 47"),
        (18000, "d3 d3 22 8d"),
    ] {
        let rec = atk::encode_dpi_stage(Sensor::Paw3395Se, dpi, dpi).unwrap_or_else(|| panic!("{dpi}"));
        assert_eq!(rec.to_vec(), h(want), "{dpi} DPI");
        assert_eq!(atk::decode_dpi_stage(Sensor::Paw3395Se, &rec), Some((dpi, dpi)), "{dpi} decodes back");
    }
    let codes = atk::se_codes();
    assert_eq!((codes.len(), codes[0], codes[3], codes[15], codes[100], codes[179], codes[199]), (200, 1, 4, 18, 118, 211, 235));
    assert!(!codes.contains(&7) && !codes.contains(&233) && codes.contains(&234));
    // not offered: below 200 (the sensor's own floor), off the grid (10050 is not a 100 step above 10000), above 18000
    for dpi in [50, 150, 10050, 18100, 825] {
        assert_eq!(atk::encode_dpi_stage(Sensor::Paw3395Se, dpi, dpi), None, "{dpi}");
    }
    // an unknown code, or a nibble other than 0 / 2, is not a DPI value
    assert_eq!(atk::decode_dpi_stage(Sensor::Paw3395Se, &h("07 07 00 47")), None, "7 is an invalid SE code");
    assert_eq!(atk::decode_dpi_stage(Sensor::Paw3395Se, &h("12 12 01 30")), None);
    let c = atk::dpi_caps(Sensor::Paw3395Se);
    assert_eq!((c.min(), c.max(), c.allows(10000), c.allows(10050), c.allows(10100), c.allows(150)), (200, 18000, true, false, true, false));
}

// ================================================================== driver: hid.js, reads

#[test]
fn pro_max_wired_read() {
    let (m, mut p, _) = hid_port(COMPX, 0xF58C, HidMouse::new((2, 27)));
    assert_eq!((m.model, m.wireless), (Some("VXE R1 Pro Max"), Some(false)));
    let r = read(&m, &mut p);
    // OpenMouse: identify, battery, system row, the active stage, lift-off (the polling rate is byte 0 of the system row)
    // (the Order 110 advanced-block read follows: see the order 110 tests)
    assert_eq!(sent(&p)[..5], [id_req(), batt_req(), rd_system6(), rd_stage1(), rd_lift()]);
    assert!(p.sent.iter().all(|(_, how)| matches!(how, HidTransfer::OutputThenInput { .. })));
    assert_eq!(
        r,
        OnMouse {
            model: Some("VXE R1 Pro Max".into()),
            online: true,
            battery_percent: Some(87),
            charging: Some(true),
            dpi: Some(800),
            polling_hz: Some(1000),
            lift_off: Some(10),
            wireless: Some(false),
            caps: r.caps.clone(),
            extras: r.extras.clone(),
        }
    );
    assert_eq!(r.caps.dpi, Some(atk::dpi_caps(Sensor::Paw3395)));
    assert_eq!((r.caps.polling.clone(), r.caps.lift_off.clone(), r.caps.battery), (vec![125, 250, 500, 1000], vec![10, 20], true));
}

#[test]
fn pro_max_receiver_read_is_the_same_reads() {
    let (m, mut p, _) = hid_port(COMPX, 0xF58A, HidMouse::new((2, 27)));
    assert_eq!((m.model, m.wireless), (Some("VXE R1 Pro Max"), Some(true)));
    let r = read(&m, &mut p);
    // (the Order 110 advanced-block read follows: see the order 110 tests)
    assert_eq!(sent(&p)[..5], [id_req(), batt_req(), rd_system6(), rd_stage1(), rd_lift()]);
    assert_eq!((r.dpi, r.polling_hz, r.lift_off, r.online, r.wireless), (Some(800), Some(1000), Some(10), true, Some(true)));
}

#[test]
fn a_reply_longer_than_the_frame_still_reads() {
    // the Windows input report may be longer than 17 bytes: the frame is the first 16 bytes after the id
    let mut mouse = HidMouse::new((2, 27));
    mouse.in_len = 65;
    let info = hid_info("cfg", COMPX, 0xF58C, 0xFF02, 2, (65, 65, 0));
    let m = atk::matches(&[&info], COMPX, 0xF58C).unwrap();
    let st = Rc::new(RefCell::new(mouse));
    let s2 = st.clone();
    let mut p = ScriptPort::new(info, move |o, how| s2.borrow_mut().answer(o, how));
    let r = read(&m, &mut p);
    assert_eq!((r.dpi, r.polling_hz, r.battery_percent), (Some(800), Some(1000), Some(87)));
    assert!(p.sent.iter().all(|(o, _)| o.len() == 65), "every request fills the output report");
}

#[test]
fn se_wired_read_decodes_the_se_codes_and_the_catalog_identity_is_read_only() {
    let (m, mut p, _) = hid_port(COMPX, 0xF58F, HidMouse::new_se((2, 32)));
    assert_eq!(m.model, Some("VXE R1 SE+"));
    let r = read(&m, &mut p);
    assert_eq!((r.dpi, r.polling_hz, r.lift_off, r.online), (Some(800), Some(1000), Some(10), true));
    assert_eq!(r.caps.dpi, Some(atk::dpi_caps(Sensor::Paw3395Se)));
    // the catalog's other R1SE+ identity (2,31): readable, but OpenMouse blocks its EEPROM writes, so no rows to set
    let (m, mut p, _) = hid_port(COMPX, 0xF58F, HidMouse::new_se((2, 31)));
    let r = read(&m, &mut p);
    assert_eq!((r.dpi, r.polling_hz, r.lift_off, r.battery_percent), (Some(800), Some(1000), Some(10), Some(87)));
    assert_eq!((r.caps.dpi.is_none(), r.caps.polling.is_empty(), r.caps.lift_off.is_empty(), r.caps.battery), (true, true, true, true));
    for s in [Setting::Dpi(1600), Setting::Polling(500), Setting::LiftOff(20)] {
        let n = p.sent.len();
        assert!(matches!(atk::Atk.write(&m, &mut p, s), Err(Error::UnsupportedMouse(_))), "{s:?}");
        assert!(p.sent[n..].iter().all(|(o, _)| o[1] == 0x10), "only the identity was asked before refusing {s:?}");
    }
}

#[test]
fn se_receiver_read_asks_if_the_mouse_is_online_and_reads_the_live_row() {
    let mut mouse = HidMouse::new_se((2, 32));
    mouse.mem[0x70..0x74].copy_from_slice(&h("0b 02 00 53")); // the live row holds the polling setting: 500 Hz
    let (m, mut p, _) = hid_port(COMPX, 0xF58E, mouse);
    assert_eq!((m.model, m.wireless), (Some("VXE R1 SE+"), Some(true)));
    let r = read(&m, &mut p);
    // no lift-off read: the receiver's lift-off is held back (the live row cannot be confirmed), so no row and no 0x000a read
    assert_eq!(sent(&p)[..6], [id_req(), batt_req(), online_req(), rd_system6(), rd_stage1(), rd_live()]);
    assert_eq!((r.dpi, r.polling_hz, r.lift_off, r.online), (Some(800), Some(500), None, true));
    assert_eq!((r.caps.polling.clone(), r.caps.lift_off.clone()), (vec![250, 500, 1000], Vec::<u32>::new()));
    // the row holds another setting last (here debounce, selector 2): its value byte is not a polling rate
    let mut mouse = HidMouse::new_se((2, 32));
    mouse.mem[0x70..0x74].copy_from_slice(&h("02 04 00 51"));
    let (m, mut p, _) = hid_port(COMPX, 0xF58E, mouse);
    assert_eq!(read(&m, &mut p).polling_hz, None);
    // the receiver says the mouse is offline: nothing more is asked
    let mut mouse = HidMouse::new_se((2, 32));
    mouse.online = false;
    let (m, mut p, _) = hid_port(COMPX, 0xF58E, mouse);
    let r = read(&m, &mut p);
    assert_eq!(sent(&p), vec![id_req(), batt_req(), online_req()]);
    assert_eq!((r.online, r.dpi, r.battery_percent), (false, None, Some(87)));
    assert!(r.caps.dpi.is_some(), "the rows stay, the values wait");
}

#[test]
fn a9_plus_read() {
    let (m, mut p, _) = hid_port(ATK, 0x1115, HidMouse::new((2, 83)));
    assert_eq!((m.model, m.wireless), (Some("ATK A9 Plus"), None));
    let r = read(&m, &mut p);
    // (the Order 110 advanced-block read follows: see the order 110 tests)
    assert_eq!(sent(&p)[..5], [id_req(), batt_req(), rd_system6(), rd_stage1(), rd_lift()]);
    assert_eq!(r.model.as_deref(), Some("ATK A9 Plus Nearlink"));
    assert_eq!((r.dpi, r.polling_hz, r.lift_off, r.caps.polling.clone()), (Some(800), Some(1000), Some(10), vec![125, 250, 500, 1000]));
    // not more than 6 stages: a count byte of 8 is cut to 6 (OpenMouse MAX_DPI_STAGES), the active stage to the last one
    let mut mouse = HidMouse::new((2, 83));
    mouse.mem[2..4].copy_from_slice(&h("08 4d"));
    mouse.mem[4..6].copy_from_slice(&h("07 4e"));
    let (m, mut p, _) = hid_port(ATK, 0x1115, mouse);
    read(&m, &mut p);
    assert_eq!(sent(&p)[3], f("08 00 00 20 04", 0x21), "stage 5 = 0x0c + 5 * 4 = 0x20; 0x55 - (8 + 8 + 0x20 + 4 = 0x34)");
}

#[test]
fn a_mouse_that_is_not_the_verified_model_is_refused() {
    // Pro Max answers CID/MID 2,7 (a plain R1 Pro on the same id f58c, per the catalog): OpenMouse would run it unverified; we do not
    let (m, mut p, st) = hid_port(COMPX, 0xF58C, HidMouse::new((2, 7)));
    assert!(matches!(atk::Atk.read(&m, &mut p, ONE), Err(Error::UnsupportedMouse(_))));
    assert!(matches!(atk::Atk.write(&m, &mut p, Setting::Dpi(1600)), Err(Error::UnsupportedMouse(_))));
    assert!(st.borrow().writes.is_empty());
    assert!(p.sent.iter().all(|(o, _)| o[1] == 0x10), "nothing but the identity question went out");
    // an R1 SE+ identity behind the SE+ receiver that is not 2,32: the receiver's DPI codec depends on it
    let (m, mut p, _) = hid_port(COMPX, 0xF58E, HidMouse::new_se((2, 31)));
    assert!(matches!(atk::Atk.read(&m, &mut p, ONE), Err(Error::UnsupportedMouse(_))));
}

#[test]
fn an_asleep_mouse_reads_as_offline_and_cannot_be_written() {
    let mut mouse = HidMouse::new((2, 27));
    mouse.awake = false;
    let (m, mut p, _) = hid_port(COMPX, 0xF58A, mouse);
    let r = atk::Atk.read(&m, &mut p, ReadOptions { online_tries: 150 }).unwrap();
    assert!(!r.online);
    assert_eq!((r.dpi, r.polling_hz, r.battery_percent), (None, None, None));
    assert!(r.caps.dpi.is_some() && r.caps.battery, "the rows stay");
    // OpenMouse tries the identity three times; online_tries 150 does not make it loop longer
    assert_eq!(p.sent.len(), 3, "three identity asks, no more");
    let mut mouse = HidMouse::new((2, 27));
    mouse.awake = false;
    let (m, mut p, _) = hid_port(COMPX, 0xF58A, mouse);
    atk::Atk.read(&m, &mut p, ONE).unwrap();
    assert_eq!(p.sent.len(), 1, "the read-only proof asks once");
    assert!(matches!(atk::Atk.write(&m, &mut p, Setting::Polling(500)), Err(Error::MouseGone(_))));
}

// ================================================================== driver: hid.js, writes

fn write_ok(m: &Matched, p: &mut ScriptPort, s: Setting) -> Changed {
    atk::Atk.write(m, p, s).unwrap_or_else(|e| panic!("{s:?}: {e}"))
}

fn is_out_only(p: &ScriptPort, i: usize) -> bool {
    matches!(p.sent[i].1, HidTransfer::OutputOnly)
}

#[test]
fn pro_max_wired_writes_one_setting_and_reads_it_back() {
    // polling 500: identify, read the row (before), write the pair at 0x0000 (no answer), read it back
    let (m, mut p, st) = hid_port(COMPX, 0xF58C, HidMouse::new((2, 27)));
    let c = write_ok(&m, &mut p, Setting::Polling(500));
    assert_eq!(c, Changed { now: 500, before: Some(1000) });
    assert_eq!(sent(&p), vec![id_req(), rd_system6(), f("07 00 00 00 02 02 53", 0xEF), rd_system6()]);
    assert!(is_out_only(&p, 2) && !is_out_only(&p, 3));
    assert_eq!(st.borrow().writes, vec![(0x0000, h("02 53"))]);
    assert_eq!(st.borrow().mem[2..10], h("04 51 01 54 00 55 11 44"), "the rest of the system row was not written");

    // DPI 1600 on the active stage (index 1 -> 0x0010): identify, system row, the stage (before), write the 4 bytes, read it back
    let (m, mut p, st) = hid_port(COMPX, 0xF58C, HidMouse::new((2, 27)));
    let c = write_ok(&m, &mut p, Setting::Dpi(1600));
    assert_eq!(c, Changed { now: 1600, before: Some(800) });
    assert_eq!(sent(&p), vec![id_req(), rd_system6(), rd_stage1(), f("07 00 00 10 04 1f 1f 00 17", 0xDD), rd_stage1()]);
    assert!(is_out_only(&p, 3));
    assert_eq!(st.borrow().writes, vec![(0x0010, h("1f 1f 00 17"))]);

    // lift-off 2 mm: identify, read (before), write the pair at 0x000a, read it back
    let (m, mut p, st) = hid_port(COMPX, 0xF58C, HidMouse::new((2, 27)));
    let c = write_ok(&m, &mut p, Setting::LiftOff(20));
    assert_eq!(c, Changed { now: 20, before: Some(10) });
    assert_eq!(sent(&p), vec![id_req(), rd_lift(), f("07 00 00 0a 02 02 53", 0xE5), rd_lift()]);
    assert_eq!(st.borrow().writes, vec![(0x000A, h("02 53"))]);
}

#[test]
fn pro_max_receiver_writes_whole_rows_and_checks_the_echo() {
    // polling 500: the 10-byte system row is read, the polling pair replaced, the row written back whole (the tail copied), echoed
    let (m, mut p, st) = hid_port(COMPX, 0xF58A, HidMouse::new((2, 27)));
    let c = write_ok(&m, &mut p, Setting::Polling(500));
    assert_eq!(c, Changed { now: 500, before: Some(1000) });
    assert_eq!(
        sent(&p),
        vec![id_req(), rd_system10(), f("07 00 00 00 0a 02 53 04 51 01 54 00 55 11 44", 0x93), rd_system6()]
    );
    assert!(matches!(p.sent[2].1, HidTransfer::OutputThenInput { echo: Some((1, 0x07)), .. }), "the receiver's echo is waited for");
    assert_eq!(st.borrow().writes, vec![(0x0000, h("02 53 04 51 01 54 00 55 11 44"))]);

    // DPI 1600 on stage 1: the 8-byte pair of stages 0 + 1 is read, stage 1 replaced, stage 0 copied back unchanged
    let (m, mut p, st) = hid_port(COMPX, 0xF58A, HidMouse::new((2, 27)));
    let c = write_ok(&m, &mut p, Setting::Dpi(1600));
    assert_eq!(c, Changed { now: 1600, before: Some(800) });
    assert_eq!(
        sent(&p),
        vec![id_req(), rd_system6(), rd_stage1(), rd_group(), f("07 00 00 0c 08 07 07 00 47 1f 1f 00 17", 0x88), rd_stage1()]
    );
    assert_eq!(st.borrow().writes, vec![(0x000C, h("07 07 00 47 1f 1f 00 17"))]);

    // lift-off 2 mm: the pair, echoed
    let (m, mut p, _) = hid_port(COMPX, 0xF58A, HidMouse::new((2, 27)));
    let c = write_ok(&m, &mut p, Setting::LiftOff(20));
    assert_eq!(c, Changed { now: 20, before: Some(10) });
    assert_eq!(sent(&p), vec![id_req(), rd_lift(), f("07 00 00 0a 02 02 53", 0xE5), rd_lift()]);
    assert!(matches!(p.sent[2].1, HidTransfer::OutputThenInput { echo: Some((1, 0x07)), .. }));
}

#[test]
fn se_wired_writes_with_the_se_codes() {
    // DPI 10100 (doubled range) on stage 1: [76 76 22 47] at 0x0010 (crc: 8 + 7 + 0x10 + 4 + 0x55 = 0x78 -> 0xdd)
    let (m, mut p, _) = hid_port(COMPX, 0xF58F, HidMouse::new_se((2, 32)));
    let c = write_ok(&m, &mut p, Setting::Dpi(10100));
    assert_eq!(c, Changed { now: 10100, before: Some(800) });
    assert_eq!(sent(&p)[3], f("07 00 00 10 04 76 76 22 47", 0xDD));
    // polling 250 over the cable: the wired register, 125..1000
    let (m, mut p, _) = hid_port(COMPX, 0xF58F, HidMouse::new_se((2, 32)));
    assert_eq!(write_ok(&m, &mut p, Setting::Polling(250)), Changed { now: 250, before: Some(1000) });
    assert_eq!(sent(&p)[2], f("07 00 00 00 02 04 51", 0xEF));
    // 2000 Hz is not an R1 rate
    assert!(matches!(atk::Atk.write(&m, &mut p, Setting::Polling(2000)), Err(Error::OutOfRange { .. })));
    // a DPI the sensor does not offer (150 is below its 200 floor)
    assert!(matches!(atk::Atk.write(&m, &mut p, Setting::Dpi(150)), Err(Error::OutOfRange { .. })));
}

#[test]
fn se_receiver_writes_through_the_live_row() {
    // polling 500: read the live row (before), write [0b 02 00 53] at 0x0070 (no answer), read the row, (250 ms), done
    let mut mouse = HidMouse::new_se((2, 32));
    mouse.mem[0x70..0x74].copy_from_slice(&h("0b 01 00 54"));
    let (m, mut p, st) = hid_port(COMPX, 0xF58E, mouse);
    let c = write_ok(&m, &mut p, Setting::Polling(500));
    assert_eq!(c, Changed { now: 500, before: Some(1000) });
    assert_eq!(sent(&p), vec![id_req(), rd_live(), f("07 00 00 70 04 0b 02 00 53", 0x72), rd_live()]);
    assert!(is_out_only(&p, 2));
    assert_eq!(st.borrow().writes, vec![(0x0070, h("0b 02 00 53"))]);
    // lift-off through the live row is held back: see se_receiver_lift_off_is_refused_and_nothing_is_sent
    // DPI goes to the EEPROM stage as on the cable (the live-settings receiver is allowed to write it)
    let (m, mut p, st) = hid_port(COMPX, 0xF58E, HidMouse::new_se((2, 32)));
    let c = write_ok(&m, &mut p, Setting::Dpi(400));
    assert_eq!(c, Changed { now: 400, before: Some(800) });
    assert_eq!(st.borrow().writes, vec![(0x0010, h("09 09 00 43"))]);
    // 125 Hz is not a rate of the 1K receiver
    assert!(matches!(atk::Atk.write(&m, &mut p, Setting::Polling(125)), Err(Error::OutOfRange { .. })));
}

#[test]
fn a9_plus_writes() {
    let (m, mut p, st) = hid_port(ATK, 0x1115, HidMouse::new((2, 83)));
    assert_eq!(write_ok(&m, &mut p, Setting::Polling(125)), Changed { now: 125, before: Some(1000) });
    assert_eq!(sent(&p)[2], f("07 00 00 00 02 08 4d", 0xEF));
    assert_eq!(write_ok(&m, &mut p, Setting::LiftOff(10)), Changed { now: 10, before: Some(10) });
    assert_eq!(write_ok(&m, &mut p, Setting::Dpi(3200)), Changed { now: 3200, before: Some(800) });
    assert_eq!(st.borrow().writes, vec![(0x0000, h("08 4d")), (0x000A, h("01 54")), (0x0010, h("3f 3f 00 d7"))]);
    // the A9 Plus tops out at 1000 Hz (maxPollingHz)
    assert!(matches!(atk::Atk.write(&m, &mut p, Setting::Polling(2000)), Err(Error::OutOfRange { .. })));
    // and has no lift-off levels other than 1 / 2 mm
    assert!(matches!(atk::Atk.write(&m, &mut p, Setting::LiftOff(7)), Err(Error::OutOfRange { .. })));
}

#[test]
fn a_mouse_that_keeps_the_old_value_is_an_error() {
    for (vid, pid, s) in [(COMPX, 0xF58C, Setting::Polling(500)), (COMPX, 0xF58C, Setting::Dpi(1600)), (COMPX, 0xF58C, Setting::LiftOff(20)), (ATK, 0x1115, Setting::Dpi(1600))] {
        let id = if pid == 0x1115 { (2, 83) } else { (2, 27) };
        let mut mouse = HidMouse::new(id);
        mouse.ignore_writes = true;
        let (m, mut p, _) = hid_port(vid, pid, mouse);
        assert!(matches!(atk::Atk.write(&m, &mut p, s), Err(Error::BadAnswer(_))), "{s:?}");
    }
    // a receiver that echoes something else than what was written
    let (m, mut p, _) = hid_port(COMPX, 0xF58A, HidMouse::new((2, 27)));
    let st = Rc::new(RefCell::new(HidMouse::new((2, 27))));
    let s2 = st.clone();
    p.answer = Box::new(move |o, how| {
        let mut a = s2.borrow_mut().answer(o, how)?;
        if o[1] == 0x07 {
            a[6] ^= 1; // a different data byte in the echo (the crc is fixed up so only the content differs)
            let c = crc15(&a[1..16]);
            a[16] = c;
        }
        Ok(a)
    });
    assert!(matches!(atk::Atk.write(&m, &mut p, Setting::Polling(500)), Err(Error::BadAnswer(_))));
}

#[test]
fn se_receiver_lift_off_is_refused_and_nothing_is_sent() {
    // OpenMouse overwrites the live row 0x0070 with [03, code, 0, 0x55 - code], reads nothing before or after and takes it as
    // done: it cannot be confirmed, so it is held back - no lift-off row, and a write is refused before a single frame goes out
    let (m, mut p, st) = hid_port(COMPX, 0xF58E, HidMouse::new_se((2, 32)));
    for t in [10, 20, 7, 15] {
        assert!(matches!(atk::Atk.write(&m, &mut p, Setting::LiftOff(t)), Err(Error::OutOfRange { .. })), "{t}");
    }
    assert!(p.sent.is_empty(), "nothing sent");
    assert!(st.borrow().writes.is_empty());
    // the wired SE+ (and the Pro Max, A9 Plus) keep their lift-off rows
    let (m, mut p, _) = hid_port(COMPX, 0xF58F, HidMouse::new_se((2, 32)));
    assert_eq!(read(&m, &mut p).caps.lift_off, vec![10, 20]);
}

#[test]
fn se_receiver_polling_is_an_error_when_the_live_row_does_not_show_the_new_rate() {
    // the row still holds 1000 Hz after the write (the mouse ignored it): OpenMouse reads the row back; the old answer
    // ("500 Hz, as asked") would have been wrong
    let mut mouse = HidMouse::new_se((2, 32));
    mouse.mem[0x70..0x74].copy_from_slice(&h("0b 01 00 54"));
    mouse.ignore_writes = true;
    let (m, mut p, st) = hid_port(COMPX, 0xF58E, mouse);
    let e = atk::Atk.write(&m, &mut p, Setting::Polling(500)).unwrap_err();
    assert!(matches!(&e, Error::BadAnswer(t) if t.contains("1000") && t.contains("500")), "{e:?}");
    assert_eq!(sent(&p), vec![id_req(), rd_live(), f("07 00 00 70 04 0b 02 00 53", 0x72), rd_live()]);
    assert_eq!(st.borrow().writes.len(), 1);
    // the row holds another setting (debounce, selector 2) and does not decode as a rate: an error, not "as asked"
    let mut mouse = HidMouse::new_se((2, 32));
    mouse.mem[0x70..0x74].copy_from_slice(&h("02 04 00 51"));
    mouse.ignore_writes = true;
    let (m, mut p, _) = hid_port(COMPX, 0xF58E, mouse);
    assert!(matches!(atk::Atk.write(&m, &mut p, Setting::Polling(500)), Err(Error::BadAnswer(_))));
    // a row that decodes to ANOTHER rate than asked
    let mut mouse = HidMouse::new_se((2, 32));
    mouse.mem[0x70..0x74].copy_from_slice(&h("0b 03 00 52")); // 250 Hz
    mouse.ignore_writes = true;
    let (m, mut p, _) = hid_port(COMPX, 0xF58E, mouse);
    let e = atk::Atk.write(&m, &mut p, Setting::Polling(500)).unwrap_err();
    assert!(matches!(&e, Error::BadAnswer(t) if t.contains("250")), "{e:?}");
    // and a row that shows the asked-for rate is fine (the working case of se_receiver_writes_through_the_live_row)
    let mut mouse = HidMouse::new_se((2, 32));
    mouse.mem[0x70..0x74].copy_from_slice(&h("0b 01 00 54"));
    let (m, mut p, _) = hid_port(COMPX, 0xF58E, mouse);
    assert_eq!(write_ok(&m, &mut p, Setting::Polling(250)), Changed { now: 250, before: Some(1000) });
}

#[test]
fn the_pro_max_receiver_write_is_sent_once_whatever_comes_back() {
    let writes = |p: &ScriptPort| p.sent.iter().filter(|(o, _)| o[1] == 0x07).count();
    // no answer at all (the receiver is silent / the wait ran out): one write, then the error - no second write
    let (m, mut p, _) = hid_port(COMPX, 0xF58A, HidMouse::new((2, 27)));
    let st = Rc::new(RefCell::new(HidMouse::new((2, 27))));
    let s2 = st.clone();
    p.answer = Box::new(move |o, how| if o[1] == 0x07 { Err(Error::BadAnswer("the mouse did not answer in time".into())) } else { s2.borrow_mut().answer(o, how) });
    assert!(matches!(atk::Atk.write(&m, &mut p, Setting::Polling(500)), Err(Error::BadAnswer(_))));
    assert_eq!(writes(&p), 1, "polling: sent once");
    for s in [Setting::Dpi(1600), Setting::LiftOff(20)] {
        p.sent.clear();
        assert!(matches!(atk::Atk.write(&m, &mut p, s), Err(Error::BadAnswer(_))), "{s:?}");
        assert_eq!(writes(&p), 1, "{s:?}: sent once");
    }
    // an unrelated / malformed answer (too short, wrong report): one write, an error
    for bad in [vec![0u8; 5], vec![9u8; 17]] {
        let (m, mut p, _) = hid_port(COMPX, 0xF58A, HidMouse::new((2, 27)));
        let st = Rc::new(RefCell::new(HidMouse::new((2, 27))));
        let s2 = st.clone();
        p.answer = Box::new(move |o, how| if o[1] == 0x07 { Ok(bad.clone()) } else { s2.borrow_mut().answer(o, how) });
        assert!(matches!(atk::Atk.write(&m, &mut p, Setting::Polling(500)), Err(Error::BadAnswer(_))));
        assert_eq!(writes(&p), 1);
    }
    // an echo of another write (right command byte, different content, valid checksum): one write, an error
    let (m, mut p, _) = hid_port(COMPX, 0xF58A, HidMouse::new((2, 27)));
    let st = Rc::new(RefCell::new(HidMouse::new((2, 27))));
    let s2 = st.clone();
    p.answer = Box::new(move |o, how| {
        let mut a = s2.borrow_mut().answer(o, how)?;
        if o[1] == 0x07 {
            a[6] ^= 1;
            let c = crc15(&a[1..16]);
            a[16] = c;
        }
        Ok(a)
    });
    assert!(matches!(atk::Atk.write(&m, &mut p, Setting::Dpi(1600)), Err(Error::BadAnswer(_))));
    assert_eq!(writes(&p), 1);
}

// ================================================================== allow-list, device table

#[test]
fn every_hid_read_passes_the_allow_list_and_every_write_fails_it() {
    let mut seen_reads = 0;
    for (vid, pid, id, se) in [(COMPX, 0xF58C, (2, 27), false), (COMPX, 0xF58A, (2, 27), false), (COMPX, 0xF58F, (2, 32), true), (COMPX, 0xF58E, (2, 32), true), (ATK, 0x1115, (2, 83), false)] {
        let mouse = || if se { HidMouse::new_se(id) } else { HidMouse::new(id) };
        let (m, mut p, _) = hid_port(vid, pid, mouse());
        read(&m, &mut p);
        for s in [Setting::Polling(250), Setting::Dpi(400), Setting::LiftOff(10)] {
            if pid == 0xF58E && matches!(s, Setting::LiftOff(_)) {
                continue; // held back on the live-row receiver
            }
            let (m, mut q, _) = hid_port(vid, pid, mouse());
            write_ok(&m, &mut q, s);
            p.sent.extend(q.sent);
        }
        for (out, _) in &p.sent {
            let is_write = out[1] == 0x07;
            assert_eq!(atk::read_request_allowed(out), !is_write, "{pid:04x}: {}", hex(out));
            assert_eq!(family::read_request_allowed(Family::Atk, out), !is_write);
            seen_reads += !is_write as usize;
        }
    }
    assert!(seen_reads > 40);
}

#[test]
fn the_allow_list_refuses_everything_else() {
    let ok = h(&id_req());
    assert!(atk::read_request_allowed(&ok));
    let mut padded = ok.clone();
    padded.resize(65, 0);
    assert!(atk::read_request_allowed(&padded), "zero padding to the output length is fine");
    let mut bad = ok.clone();
    bad[16] ^= 1;
    assert!(!atk::read_request_allowed(&bad), "a wrong crc");
    let mut bad = padded.clone();
    bad[40] = 1;
    assert!(!atk::read_request_allowed(&bad), "payload after the frame");
    assert!(!atk::read_request_allowed(&[]));
    assert!(!atk::read_request_allowed(&ok[..10]));
    // commands OpenMouse's codec has and this port must never send: version 0x12, set profile 0x0f, factory restore 0x09,
    // USB upgrade 0x0d, pairing 0x05, dongle light 0x14, long range 0x16 - with a correct crc each
    for cmd in [0x12u8, 0x0F, 0x09, 0x0D, 0x05, 0x14, 0x16, 0x0E, 0x06, 0x13, 0x5A, 0x00] {
        let fr = atk::hid_frame(cmd, &[]);
        assert!(!atk::read_request_allowed(&atk::hid_wire(&fr, 17)), "command {cmd:#04x}");
    }
    // a read of an address this port never reads (the button table 0x0060, macros 0x0300, the dpi lighting 0x004c)
    for addr in [0x0060u16, 0x0300, 0x004C, 0x0100, 0x00A9, 0x2C00] {
        assert!(!atk::read_request_allowed(&atk::hid_wire(&atk::read_eeprom_frame(addr, 4), 17)), "{addr:#06x}");
    }
    assert!(!atk::read_request_allowed(&atk::hid_wire(&atk::read_eeprom_frame(0x0000, 0), 17)));
    assert!(!atk::read_request_allowed(&atk::hid_wire(&atk::read_eeprom_frame(0x0000, 11), 17)));
    // a read frame with data after the length byte is not a read
    assert!(!atk::read_request_allowed(&atk::hid_wire(&atk::hid_frame(0x08, &[0, 0, 0, 6, 1]), 17)));
    // any write is refused, whatever it writes
    for (addr, data) in [(0x0000u16, vec![1u8, 0x54]), (0x000A, vec![2, 0x53]), (0x0070, vec![0x0B, 1, 0, 0x54])] {
        assert!(!atk::read_request_allowed(&atk::hid_wire(&atk::write_eeprom_frame(addr, &data), 17)));
    }
    // an online question is a read; with a stray argument it is not
    assert!(atk::read_request_allowed(&atk::hid_wire(&atk::hid_frame(0x03, &[]), 17)));
    assert!(!atk::read_request_allowed(&atk::hid_wire(&atk::hid_frame(0x03, &[1]), 17)));
}

fn mouse_collection(vid: u16, pid: u16) -> HidInfo {
    hid_info("mouse", vid, pid, 0x01, 0x02, (8, 0, 0))
}

#[test]
fn device_table() {
    // (vid, pid, model, wireless, variant, client)
    let table = [
        (ATK, 0x1154u16, "ATK ZERO", Some(false), atk::V_ZERO_WIRED, true),
        (ATK, 0x1155, "ATK ZERO", Some(true), atk::V_ZERO_RECEIVER, true),
        (COMPX, 0xF58C, "VXE R1 Pro Max", Some(false), atk::V_PRO_MAX_WIRED, false),
        (COMPX, 0xF58A, "VXE R1 Pro Max", Some(true), atk::V_PRO_MAX_RECEIVER, false),
        (COMPX, 0xF58F, "VXE R1 SE+", Some(false), atk::V_SE_WIRED, false),
        (COMPX, 0xF58E, "VXE R1 SE+", Some(true), atk::V_SE_RECEIVER, false),
        (ATK, 0x1115, "ATK A9 Plus", None, atk::V_A9_PLUS, false),
    ];
    assert_eq!(atk::PRODUCTS.len(), 7);
    for (vid, pid, model, wireless, variant, bitmouse) in table {
        // hid.js: vendor collection 0xFF02:0x0002; BITMOUSE: 0xFF05:0x0001 with a 64-byte output report
        let cfg = if bitmouse { hid_info("cfg", vid, pid, 0xFF05, 1, (64, 64, 0)) } else { hid_if(vid, pid) };
        let mouse = mouse_collection(vid, pid);
        let hids = vec![mouse.clone(), cfg.clone()];
        let own: Vec<&HidInfo> = hids.iter().collect();
        let got = atk::matches(&own, vid, pid).unwrap_or_else(|| panic!("{vid:04x}:{pid:04x}"));
        assert_eq!((got.family, got.model, got.wireless, got.variant), (Family::Atk, Some(model), wireless, variant));
        assert_eq!(got.config, cfg, "talks to the vendor collection, not the mouse collection");
        // and the family search finds it (the earlier families - Pulsar, Logitech, Razer, SteelSeries - do not claim it)
        let found = family::find(&hids, vid, pid).expect("found");
        assert_eq!((found.family, found.variant), (Family::Atk, variant));
        // other families' matchers do not take the ATK interface
        assert!(bu_mouse::pulsar::matches_cmouse(&own, vid, pid).is_none() && bu_mouse::pulsar::matches_xs1(&own, vid, pid).is_none());
        assert!(bu_mouse::logitech::matches(&own, vid, pid).is_none() && bu_mouse::razer::matches(&own, vid, pid).is_none());
        assert!(bu_mouse::steelseries::matches(&own, vid, pid).is_none());
        // only the mouse's own collection: no vendor interface to talk to
        let only_mouse = [mouse.clone()];
        assert!(atk::matches(&only_mouse.iter().collect::<Vec<_>>(), vid, pid).is_none(), "{pid:04x}");
        // the other client's collection does not do
        let wrong = if bitmouse { hid_if(vid, pid) } else { hid_info("cfg", vid, pid, 0xFF05, 1, (64, 64, 0)) };
        assert!(atk::matches(&[&wrong], vid, pid).is_none(), "{pid:04x}");
        // a vendor collection with the wrong usage, or too short a report
        let wrong_usage = hid_info("cfg", vid, pid, if bitmouse { 0xFF05 } else { 0xFF02 }, 3, (64, 64, 0));
        assert!(atk::matches(&[&wrong_usage], vid, pid).is_none());
        let short = hid_info("cfg", vid, pid, if bitmouse { 0xFF05 } else { 0xFF02 }, if bitmouse { 1 } else { 2 }, (8, 8, 0));
        assert!(atk::matches(&[&short], vid, pid).is_none());
        // the same product id under another vendor id is another mouse
        let other = hid_info("cfg", 0x1234, pid, if bitmouse { 0xFF05 } else { 0xFF02 }, if bitmouse { 1 } else { 2 }, (64, 64, 0));
        assert!(atk::matches(&[&other], 0x1234, pid).is_none());
    }
    // ids that are close but not on the list
    for (vid, pid) in [(ATK, 0x1156u16), (ATK, 0x1153), (COMPX, 0xF58B), (COMPX, 0xF58D), (COMPX, 0xF590), (ATK, 0xF58C), (COMPX, 0x1154), (ATK, 0x10C9), (ATK, 0x124E), (COMPX, 0xF506)] {
        for cfg in [hid_if(vid, pid), hid_info("cfg", vid, pid, 0xFF05, 1, (64, 64, 0))] {
            assert!(atk::matches(&[&cfg], vid, pid).is_none(), "{vid:04x}:{pid:04x}");
        }
    }
}

// ================================================================== BITMOUSE (ATK ZERO)

/// A fake ATK ZERO: config block, battery, DPI table; it answers like the firmware (an error status for a request whose length
/// pair is wrong), and a tail of 0xEE after a reply's reported length (the device does not clear it).
struct ZeroMouse {
    target: u8,
    poll_code: u8,
    offset_cal: u8,
    battery: u8,
    charging: u8,
    /// the DPI table from address 1: current, count, 8 stages of 8 bytes, padded to 70
    table: Vec<u8>,
    awake: bool,
    ignore_writes: bool,
    writes: Vec<(u8, Vec<u8>)>,
    /// Order 110: the config block's flags (payload 6 motion sync, 7 linear correction, 8 ripple) and debounce (payload 11)
    flags: [u8; 3],
    debounce: u8,
}

impl ZeroMouse {
    fn new(target: u8) -> Self {
        let mut table = h("01 08  20 03 20 03 10 20 30 00  40 06 40 06 10 20 30 00"); // current 1; stage 0 = 800, stage 1 = 1600 (colour b 10 g 20 r 30)
        table.resize(70, 0);
        Self { target, poll_code: 2, offset_cal: 3, battery: 64, charging: 0, table, awake: true, ignore_writes: false, writes: Vec::new(), flags: [0, 0, 0], debounce: 4 }
    }
    fn reply(&self, cmd: u8, status: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![8, 0x72, status, 0x3A, self.target, cmd, payload.len() as u8];
        v.extend_from_slice(payload);
        v.resize(64, 0xEE);
        v
    }
    fn answer(&mut self, out: &[u8], how: &HidTransfer) -> Result<Vec<u8>, Error> {
        assert_eq!((out.len(), out[0]), (64, 8));
        let fr = &out[1..64];
        assert_eq!(fr[0], fr[1..].iter().fold(0u8, |s, b| s.wrapping_add(*b)), "checksum");
        assert_eq!((fr[1], fr[3], fr[4]), (0x72, 0, self.target), "constant, sn, target");
        if !self.awake {
            return Err(Error::BadAnswer("the mouse did not answer in time".into()));
        }
        let (cmd, lens, p) = (fr[5], (fr[2], fr[6]), &fr[7..]);
        let expected = match cmd {
            1 => (3, 1),
            2 => (12, 10),
            3 => (4, 2),
            7 | 15 => (2, 1),
            11 | 12 | 13 | 22 => (3, 1),
            9 => (19, 17),
            23 => (10, 13),
            c => panic!("unexpected command {c}"),
        };
        if lens != expected {
            return Ok(self.reply(cmd, 0xFF, &[]));
        }
        match (cmd, how) {
            (1..=3 | 11..=13 | 22, HidTransfer::OutputOnly) => {
                self.writes.push((cmd, p[..expected.1 as usize].to_vec()));
                if !self.ignore_writes {
                    match cmd {
                        1 => self.poll_code = p[0],
                        3 => self.offset_cal = p[1],
                        13 => self.flags[0] = p[0],
                        11 => self.flags[1] = p[0],
                        12 => self.flags[2] = p[0],
                        22 => self.debounce = p[0],
                        _ => {
                            let i = 2 + 8 * p[0] as usize;
                            self.table[i..i + 8].copy_from_slice(&[p[1], p[2], p[3], p[4], p[5], p[6], p[7], 0]);
                            if p[9] == 1 {
                                self.table[0] = p[0];
                            }
                        }
                    }
                }
                Ok(Vec::new())
            }
            (1..=3 | 11..=13 | 22, _) => panic!("a write must not wait for an answer"),
            (9, _) => {
                let (fl, d) = (self.flags, self.debounce);
                Ok(self.reply(9, 0, &[1, 2, self.poll_code, 0, 0, self.offset_cal, fl[0], fl[1], fl[2], 0x1E, 0, d, 0, 0, 0, 0, 0]))
            }
            (7, _) => Ok(self.reply(7, 0, &[self.battery])),
            (15, _) => Ok(self.reply(15, 0, &[self.charging])),
            (23, _) => {
                let addr = p[0] as usize | (p[1] as usize) << 8;
                let len = p[2] as usize;
                let mut pl = vec![p[0], p[1], p[2]];
                pl.extend_from_slice(&self.table[addr - 1..addr - 1 + len]);
                Ok(self.reply(23, 0, &pl))
            }
            _ => unreachable!(),
        }
    }
}

fn zero_port(pid: u16, mouse: ZeroMouse) -> (Matched, ScriptPort, Rc<RefCell<ZeroMouse>>) {
    let info = hid_info("cfg", ATK, pid, 0xFF05, 1, (64, 64, 0));
    let m = atk::matches(&[&info], ATK, pid).expect("in the table");
    let st = Rc::new(RefCell::new(mouse));
    let s2 = st.clone();
    (m, ScriptPort::new(info, move |o, how| s2.borrow_mut().answer(o, how)), st)
}

/// The seven DPI-table reads of target `t`: checksum = 0x72 + paramLen 10 + sn 0 + target + cmd 23 + cmdLen 13 + (addr lo + addr hi 0
/// + length 10) = 160 + target + 10 + addr = 170 + addr + target, for addresses 1, 11, 21, 31, 41, 51, 61.
fn table_reads(t: u8) -> Vec<String> {
    [1u8, 11, 21, 31, 41, 51, 61].iter().map(|a| bm(&format!("08 {:02x} 72 0a 00 {t:02x} 17 0d {a:02x} 00 0a", 170 + a + t))).collect()
}

#[test]
fn bitmouse_requests_are_openmouses() {
    // frame = [checksum, 0x72, paramLen, 0, target, cmd, cmdLen, payload..]; checksum = low byte of the sum of bytes 1..62
    // getCurrentMouseConfig (cmd 9, lengths 19 / 17): 0x72 + 19 + 0 + t + 9 + 17 = 159 + t = 0x9f
    let w = |cmd: u8, lens: (u8, u8), t: u8, pl: &[u8]| hex(&atk::bm_wire(&atk::bm_frame(cmd, lens, t, pl), 64));
    assert_eq!(w(9, atk::BM_LEN_CONFIG, 0, &[]), bm("08 9f 72 13 00 00 09 11"));
    assert_eq!(w(9, atk::BM_LEN_CONFIG, 1, &[]), bm("08 a0 72 13 00 01 09 11"), "behind the receiver: target 1, checksum + 1");
    // getBatteryLevel (7, lengths 2 / 1): 114 + 2 + 7 + 1 = 124 = 0x7c; getBatteryChargingStatus (15): 114 + 2 + 15 + 1 = 132 = 0x84
    assert_eq!(w(7, atk::BM_LEN_BATTERY, 0, &[]), bm("08 7c 72 02 00 00 07 01"));
    assert_eq!(w(15, atk::BM_LEN_CHARGING, 0, &[]), bm("08 84 72 02 00 00 0f 01"));
    assert_eq!(w(7, atk::BM_LEN_BATTERY, 1, &[]), bm("08 7d 72 02 00 01 07 01"));
    // getAddressData (23, lengths 10 / 13), payload [addr lo, addr hi, length]: 114 + 10 + 23 + 13 + addr + 10 = 170 + addr
    assert_eq!(atk::bm_address_payload(1, 10), [1, 0, 10]);
    assert_eq!(w(23, atk::BM_LEN_ADDRESS_DATA, 0, &atk::bm_address_payload(1, 10)), bm("08 ab 72 0a 00 00 17 0d 01 00 0a"));
    assert_eq!(w(23, atk::BM_LEN_ADDRESS_DATA, 0, &atk::bm_address_payload(61, 10)), bm("08 e7 72 0a 00 00 17 0d 3d 00 0a"));
    assert_eq!(atk::bm_dpi_addresses(), vec![1, 11, 21, 31, 41, 51, 61]);
    assert_eq!(table_reads(0)[1], bm("08 b5 72 0a 00 00 17 0d 0b 00 0a"), "address 11: 181 = 0xb5");
    // setReportRate (1, lengths 3 / 1), payload [code]: 114 + 3 + 1 + 1 + code = 119 + code
    assert_eq!(w(1, atk::BM_LEN_SET_REPORT_RATE, 0, &[3]), bm("08 7a 72 03 00 00 01 01 03")); // 125 Hz
    assert_eq!(w(1, atk::BM_LEN_SET_REPORT_RATE, 0, &[4]), bm("08 7b 72 03 00 00 01 01 04")); // 8000 Hz
    assert_eq!(w(1, atk::BM_LEN_SET_REPORT_RATE, 1, &[6]), bm("08 7e 72 03 00 01 01 01 06")); // 2000 Hz behind the receiver
    // setSilentHeight (3, lengths 4 / 2), payload [0, level - 1]: 114 + 4 + 3 + 2 + (level - 1) = 123 + level - 1
    assert_eq!(w(3, atk::BM_LEN_SET_SILENT_HEIGHT, 0, &[0, 3]), bm("08 7e 72 04 00 00 03 02 00 03")); // 1.0 mm = level 4
    assert_eq!(w(3, atk::BM_LEN_SET_SILENT_HEIGHT, 0, &[0, 10]), bm("08 85 72 04 00 00 03 02 00 0a")); // 1.7 mm = level 11
    // setDpi (2, lengths 12 / 10), payload [index, x lo, x hi, y lo, y hi, blue, green, red, 0, enable]
    // stage 1, 3200 (0x0c80), colour b 10 g 20 r 30, enable: payload sum 1 + 0x80 + 0x0c + 0x80 + 0x0c + 0x10 + 0x20 + 0x30 + 0 + 1
    // = 378; 114 + 12 + 2 + 10 = 138; 138 + 378 = 516 = 0x204 -> 0x04
    let pl = atk::bm_set_dpi_payload(1, 3200, 3200, 0x10, 0x20, 0x30, true);
    assert_eq!(pl.to_vec(), h("01 80 0c 80 0c 10 20 30 00 01"));
    assert_eq!(w(2, atk::BM_LEN_SET_DPI, 0, &pl), bm("08 04 72 0c 00 00 02 0a 01 80 0c 80 0c 10 20 30 00 01"));
    assert_eq!(w(2, atk::BM_LEN_SET_DPI, 1, &pl), bm("08 05 72 0c 00 01 02 0a 01 80 0c 80 0c 10 20 30 00 01"));
    // enable 0 (not the active stage) - the last byte only
    assert_eq!(atk::bm_set_dpi_payload(0, 800, 800, 1, 2, 3, false).to_vec(), h("00 20 03 20 03 01 02 03 00 00"));
}

#[test]
fn bitmouse_answers_decode_like_openmouse() {
    // reply = [0x72, status, sn, target, cmd, len, payload..] (+ the report id 8 Windows puts in front); the payload is cut to
    // the reported length - the bytes after it are leftovers
    let r = atk::bm_decode_reply(&h("08 72 00 3a 00 07 01 40 ee ee ee")).unwrap();
    assert_eq!((r.cmd, r.is_error, r.payload.clone()), (7, false, vec![0x40]));
    let r = atk::bm_decode_reply(&h("08 72 ff 3a 00 09 00")).unwrap();
    assert_eq!((r.cmd, r.is_error, r.payload.is_empty()), (9, true, true));
    assert!(atk::bm_decode_reply(&h("08 73 00 3a 00 07 01 40")).is_none(), "not a 0x72 answer");
    assert!(atk::bm_decode_reply(&h("08 72 00")).is_none());
    // a length longer than the report is cut to what is there
    assert_eq!(atk::bm_decode_reply(&h("08 72 00 3a 00 07 05 01 02")).unwrap().payload, vec![1, 2]);
    // the config block: payload[2] polling code, payload[5] offsetCalibration (lift-off level = that + 1; 0.7 mm = level 1)
    let c = atk::bm_decode_config(&h("01 02 04 00 00 03 00 00 00 1e 00 04")).unwrap();
    assert_eq!((c.polling_hz, c.offset_calibration), (Some(8000), 3));
    assert!(atk::bm_decode_config(&h("01 02 04 00 00 03 00 00 00 1e 00")).is_none(), "shorter than 12 bytes");
    assert_eq!((atk::bm_lift_off_tenths(0), atk::bm_lift_off_tenths(3), atk::bm_lift_off_tenths(10), atk::bm_lift_off_tenths(11)), (Some(7), Some(10), Some(17), None));
    // the DPI table: current, count, then 8 x (x lo, x hi, y lo, y hi, blue, green, red, reserved)
    let mut t = h("02 08  20 03 20 03 10 20 30 00  40 06 40 06 11 21 31 00  80 0c 80 0c 12 22 32 00");
    t.resize(66, 0);
    let b = atk::bm_decode_dpi_block(&t).unwrap();
    assert_eq!(b.current, 2);
    assert_eq!(b.stages.len(), 8);
    assert_eq!((b.stages[0].x, b.stages[1].x, b.stages[2].x, b.stages[2].y), (800, 1600, 3200, 3200));
    assert_eq!((b.stages[2].blue, b.stages[2].green, b.stages[2].red), (0x12, 0x22, 0x32));
    assert!(atk::bm_decode_dpi_block(&t[..60]).is_none());
    // the ranges: 10 by 10 up to 9990, 50 by 50 from 10000 up to 29950, then 100 by 100 up to 42000
    let c = atk::bm_dpi_caps();
    assert_eq!((c.min(), c.max(), c.allows(10), c.allows(15), c.allows(9990), c.allows(10000), c.allows(10010), c.allows(30000), c.allows(30050), c.allows(42000)), (10, 42000, true, false, true, true, false, true, false, true));
}

#[test]
fn zero_wired_read() {
    let (m, mut p, _) = zero_port(0x1154, ZeroMouse::new(0));
    assert_eq!((m.model, m.wireless), (Some("ATK ZERO"), Some(false)));
    let r = read(&m, &mut p);
    let mut want = vec![bm("08 9f 72 13 00 00 09 11"), bm("08 7c 72 02 00 00 07 01"), bm("08 84 72 02 00 00 0f 01")];
    want.extend(table_reads(0));
    assert_eq!(sent(&p), want);
    assert!(p.sent.iter().all(|(_, how)| matches!(how, HidTransfer::OutputThenInput { echo: Some((5, _)), .. })));
    assert_eq!(
        r,
        OnMouse {
            model: Some("ATK ZERO".into()),
            online: true,
            battery_percent: Some(64),
            charging: Some(false),
            dpi: Some(1600),
            polling_hz: Some(250),
            lift_off: Some(10),
            wireless: Some(false),
            caps: r.caps.clone(),
            extras: r.extras.clone(),
        }
    );
    assert_eq!(r.caps.dpi, Some(atk::bm_dpi_caps()));
    assert_eq!(r.caps.polling, vec![125, 250, 500, 1000, 2000, 4000, 8000]);
    assert_eq!(r.caps.lift_off, (7..=17).collect::<Vec<u32>>(), "0.7 - 1.7 mm in tenths");
    assert!(r.caps.battery);
}

#[test]
fn zero_receiver_talks_to_the_mouse_behind_it() {
    let (m, mut p, _) = zero_port(0x1155, ZeroMouse::new(1));
    assert_eq!((m.model, m.wireless), (Some("ATK ZERO"), Some(true)));
    let mut mouse = ZeroMouse::new(1);
    mouse.charging = 1;
    mouse.poll_code = 4;
    mouse.offset_cal = 0;
    let (_, mut p2, _) = zero_port(0x1155, mouse);
    let r = read(&m, &mut p2);
    let mut want = vec![bm("08 a0 72 13 00 01 09 11"), bm("08 7d 72 02 00 01 07 01"), bm("08 85 72 02 00 01 0f 01")];
    want.extend(table_reads(1));
    assert_eq!(sent(&p2), want);
    assert_eq!((r.polling_hz, r.lift_off, r.charging, r.wireless), (Some(8000), Some(7), Some(true), Some(true)));
    let _ = &mut p;
}

#[test]
fn zero_asleep_behind_the_receiver_reads_as_offline() {
    let mut mouse = ZeroMouse::new(1);
    mouse.awake = false;
    let (m, mut p, _) = zero_port(0x1155, mouse);
    let r = atk::Atk.read(&m, &mut p, ReadOptions { online_tries: 150 }).unwrap();
    assert!(!r.online);
    assert_eq!((r.dpi, r.polling_hz, r.battery_percent, r.lift_off), (None, None, None, None));
    assert!(r.caps.dpi.is_some());
    assert_eq!(p.sent.len(), 3, "the config is asked three times at most");
    let mut mouse = ZeroMouse::new(1);
    mouse.awake = false;
    let (m, mut p, _) = zero_port(0x1155, mouse);
    atk::Atk.read(&m, &mut p, ONE).unwrap();
    assert_eq!(p.sent.len(), 1);
    // and a write to a sleeping mouse fails (its read-back finds nothing)
    for s in [Setting::Polling(500), Setting::Dpi(1600), Setting::LiftOff(10)] {
        assert!(atk::Atk.write(&m, &mut p, s).is_err(), "{s:?}");
    }
}

#[test]
fn zero_writes_one_setting_and_reads_it_back() {
    // polling 8000: read the config (before), write [4] with command 1, read the config again
    let (m, mut p, st) = zero_port(0x1154, ZeroMouse::new(0));
    let c = write_ok(&m, &mut p, Setting::Polling(8000));
    assert_eq!(c, Changed { now: 8000, before: Some(250) });
    assert_eq!(sent(&p), vec![bm("08 9f 72 13 00 00 09 11"), bm("08 7b 72 03 00 00 01 01 04"), bm("08 9f 72 13 00 00 09 11")]);
    assert!(is_out_only(&p, 1) && !is_out_only(&p, 0));
    assert_eq!(st.borrow().writes, vec![(1, vec![4])]);

    // lift-off 1.7 mm = level 11: command 3 with [0, 10]
    let (m, mut p, st) = zero_port(0x1154, ZeroMouse::new(0));
    let c = write_ok(&m, &mut p, Setting::LiftOff(17));
    assert_eq!(c, Changed { now: 17, before: Some(10) });
    assert_eq!(sent(&p), vec![bm("08 9f 72 13 00 00 09 11"), bm("08 85 72 04 00 00 03 02 00 0a"), bm("08 9f 72 13 00 00 09 11")]);
    assert_eq!(st.borrow().writes, vec![(3, vec![0, 10])]);
    // 0.7 mm = level 1: [0, 0]
    write_ok(&m, &mut p, Setting::LiftOff(7));
    assert_eq!(st.borrow().writes.last(), Some(&(3, vec![0, 0])));

    // DPI 3200 on the ACTIVE stage (1): the table is read (7 reads), stage 1 is written with its colour carried over and
    // enable 1, the table is read again
    let (m, mut p, st) = zero_port(0x1154, ZeroMouse::new(0));
    let c = write_ok(&m, &mut p, Setting::Dpi(3200));
    assert_eq!(c, Changed { now: 3200, before: Some(1600) });
    let mut want = table_reads(0);
    want.push(bm("08 04 72 0c 00 00 02 0a 01 80 0c 80 0c 10 20 30 00 01"));
    want.extend(table_reads(0));
    assert_eq!(sent(&p), want);
    assert_eq!(st.borrow().writes, vec![(2, h("01 80 0c 80 0c 10 20 30 00 01"))]);
    assert_eq!(st.borrow().table[0], 1, "the active stage stays 1");

    // behind the receiver every request is target 1 (checksums + 1)
    let (m, mut p, _) = zero_port(0x1155, ZeroMouse::new(1));
    write_ok(&m, &mut p, Setting::Polling(125));
    assert_eq!(sent(&p)[1], bm("08 7b 72 03 00 01 01 01 03"));
    let (m, mut p, _) = zero_port(0x1155, ZeroMouse::new(1));
    write_ok(&m, &mut p, Setting::Dpi(3200));
    assert_eq!(sent(&p)[7], bm("08 05 72 0c 00 01 02 0a 01 80 0c 80 0c 10 20 30 00 01"));
}

#[test]
fn zero_refuses_what_it_cannot_store_and_notices_a_mouse_that_kept_the_old_value() {
    let (m, mut p, _) = zero_port(0x1154, ZeroMouse::new(0));
    for s in [Setting::Dpi(5), Setting::Dpi(15), Setting::Dpi(30050), Setting::Dpi(43000), Setting::Polling(750), Setting::LiftOff(6), Setting::LiftOff(18), Setting::LiftOff(0)] {
        assert!(matches!(atk::Atk.write(&m, &mut p, s), Err(Error::OutOfRange { .. })), "{s:?}");
    }
    assert!(p.sent.is_empty(), "nothing went to the mouse");
    for s in [Setting::Polling(500), Setting::Dpi(1600), Setting::LiftOff(17)] {
        let mut mouse = ZeroMouse::new(0);
        mouse.ignore_writes = true;
        let (m, mut p, _) = zero_port(0x1154, mouse);
        let r = atk::Atk.write(&m, &mut p, s);
        // DPI 1600 is already what stage 1 holds, so only the other two can show the refusal
        if s == Setting::Dpi(1600) {
            continue;
        }
        assert!(matches!(r, Err(Error::BadAnswer(_))), "{s:?}");
    }
    let mut mouse = ZeroMouse::new(0);
    mouse.ignore_writes = true;
    let (m, mut p, _) = zero_port(0x1154, mouse);
    assert!(matches!(atk::Atk.write(&m, &mut p, Setting::Dpi(3200)), Err(Error::BadAnswer(_))));
    // a mouse whose DPI table does not answer cannot be written
    let mut mouse = ZeroMouse::new(0);
    mouse.table.clear();
    mouse.table.resize(70, 0);
    mouse.table[1] = 0;
    let (m, mut p, _) = zero_port(0x1154, mouse);
    // current index 0, stage 0 holds 0 DPI: the write still goes to stage 0 (OpenMouse writes the stage it is on)
    let c = atk::Atk.write(&m, &mut p, Setting::Dpi(800));
    assert_eq!(c, Ok(Changed { now: 800, before: Some(0) }));
}

#[test]
fn every_bitmouse_read_passes_the_allow_list_and_every_write_fails_it() {
    let mut reads = 0;
    for pid in [0x1154u16, 0x1155] {
        let t = (pid - 0x1154) as u8;
        let (m, mut p, _) = zero_port(pid, ZeroMouse::new(t));
        read(&m, &mut p);
        for s in [Setting::Polling(500), Setting::Dpi(1600), Setting::LiftOff(10)] {
            let (m, mut q, _) = zero_port(pid, ZeroMouse::new(t));
            write_ok(&m, &mut q, s);
            p.sent.extend(q.sent);
        }
        for (out, _) in &p.sent {
            let cmd = out[6];
            let is_write = matches!(cmd, 1..=3);
            assert_eq!(atk::read_request_allowed(out), !is_write, "{}", hex(out));
            assert_eq!(family::read_request_allowed(Family::Atk, out), !is_write);
            reads += !is_write as usize;
        }
    }
    assert!(reads > 30);
}

#[test]
fn the_bitmouse_allow_list_refuses_everything_else() {
    let ok = h(&bm("08 9f 72 13 00 00 09 11"));
    assert!(atk::read_request_allowed(&ok));
    let mut bad = ok.clone();
    bad[1] ^= 1;
    assert!(!atk::read_request_allowed(&bad), "wrong checksum");
    let mut bad = ok.clone();
    bad[63] = 1; // checksum no longer matches, and the frame is not a known read
    assert!(!atk::read_request_allowed(&bad));
    assert!(!atk::read_request_allowed(&ok[..40]));
    // other commands of the codec, with correct checksums: sensor model 31, far distance 27, linear correction 11, ripple 12,
    // motion sync 13, sleep 21, debounce 22, device version 28, cid/mid 74, dongle status 129
    for cmd in [31u8, 27, 11, 12, 13, 21, 22, 28, 74, 129, 136, 137, 26, 0] {
        let w = atk::bm_wire(&atk::bm_frame(cmd, (3, 1), 0, &[1]), 64);
        assert!(!atk::read_request_allowed(&w), "command {cmd}");
    }
    // a read command with the wrong lengths, a payload where there is none, a bad target
    assert!(!atk::read_request_allowed(&atk::bm_wire(&atk::bm_frame(9, (3, 1), 0, &[]), 64)));
    assert!(!atk::read_request_allowed(&atk::bm_wire(&atk::bm_frame(9, atk::BM_LEN_CONFIG, 0, &[1]), 64)));
    assert!(!atk::read_request_allowed(&atk::bm_wire(&atk::bm_frame(9, atk::BM_LEN_CONFIG, 2, &[]), 64)));
    // address data: only the DPI table's seven chunks, ten bytes each (not the sensor angle at 2692, nor an arbitrary address)
    assert!(atk::read_request_allowed(&atk::bm_wire(&atk::bm_frame(23, atk::BM_LEN_ADDRESS_DATA, 0, &atk::bm_address_payload(31, 10)), 64)));
    for (a, l) in [(2692u16, 1u8), (74, 1), (75, 1), (2, 10), (31, 11), (1, 255), (0, 10)] {
        assert!(!atk::read_request_allowed(&atk::bm_wire(&atk::bm_frame(23, atk::BM_LEN_ADDRESS_DATA, 0, &atk::bm_address_payload(a, l)), 64)), "{a}/{l}");
    }
    // writes
    assert!(!atk::read_request_allowed(&atk::bm_wire(&atk::bm_frame(1, atk::BM_LEN_SET_REPORT_RATE, 0, &[0]), 64)));
    assert!(!atk::read_request_allowed(&atk::bm_wire(&atk::bm_frame(2, atk::BM_LEN_SET_DPI, 1, &atk::bm_set_dpi_payload(0, 800, 800, 0, 0, 0, true)), 64)));
    assert!(!atk::read_request_allowed(&atk::bm_wire(&atk::bm_frame(3, atk::BM_LEN_SET_SILENT_HEIGHT, 0, &[0, 3]), 64)));
}

#[test]
fn the_two_clients_never_claim_the_same_interface() {
    // OpenMouse's registry tries the BITMOUSE client first, then hid.js; here the two kinds are told apart by usage page, so the
    // order cannot matter: the ZERO pids need 0xFF05:1, everything else 0xFF02:2
    let both = [hid_if(ATK, 0x1154), hid_info("cfg5", ATK, 0x1154, 0xFF05, 1, (64, 64, 0))];
    let own: Vec<&HidInfo> = both.iter().collect();
    let got = atk::matches(&own, ATK, 0x1154).unwrap();
    assert_eq!((got.config.usage_page, got.variant), (0xFF05, atk::V_ZERO_WIRED));
    let both = [hid_if(COMPX, 0xF58C), hid_info("cfg5", COMPX, 0xF58C, 0xFF05, 1, (64, 64, 0))];
    let own: Vec<&HidInfo> = both.iter().collect();
    let got = atk::matches(&own, COMPX, 0xF58C).unwrap();
    assert_eq!(got.config.usage_page, 0xFF02);
}

// ================================================================== Order 110: debounce, Motion Sync, angle snapping, ripple

use bu_mouse::family::{Change, Extra, Kind};

/// readStatus' advanced read: read(0x00a9, 10) - crc 0x55 - (8 + 8 + 0xa9 + 0x0a = 0xc3) = 0x92
fn rd_adv() -> String {
    f("08 00 00 a9 0a", 0x92)
}

/// The advanced block written whole (writeAdvanced): every pair sums to 0x55, so crc = 0x55 - ((8 + 7 + 0 + 0xa9 + 0x0a + 5 x 0x55
/// = 0xc2 + 0x1a9 = 0x26b) & 0xff) = 0x55 - 0x6b = 0xea for any values.
fn wr_adv(pairs: &str) -> String {
    f(&format!("07 00 00 a9 0a {pairs}"), 0xEA)
}

/// A hid.js mouse whose advanced block holds debounce 4 ms, Motion Sync on, sleep 6 (1 min), angle snapping on, ripple off.
fn with_adv(mut m: HidMouse) -> HidMouse {
    m.mem[0xA9..0xB3].copy_from_slice(&h("04 51 01 54 06 4f 01 54 00 55"));
    m
}

fn extras(r: &OnMouse) -> Vec<(Extra, Option<i32>)> {
    r.extras.iter().map(|e| (e.extra, e.value)).collect()
}

#[test]
fn order_110_hid_read_shows_every_setting_openmouse_reads() {
    // R1 Pro Max (wired + receiver), R1 SE+ wired 2,32, A9 Plus: debounceMs = advanced[0], motionSync = advanced[2] === 1,
    // angleSnapping = advanced[6] === 1 (R1 / A9 "straight-line correction"), rippleControl = advanced[8] === 1
    for (vid, pid, mouse) in [(COMPX, 0xF58C, HidMouse::new((2, 27))), (COMPX, 0xF58A, HidMouse::new((2, 27))), (COMPX, 0xF58F, HidMouse::new_se((2, 32))), (ATK, 0x1115, HidMouse::new((2, 83)))] {
        let (m, mut p, _) = hid_port(vid, pid, with_adv(mouse));
        let r = read(&m, &mut p);
        assert_eq!(sent(&p), vec![id_req(), batt_req(), rd_system6(), rd_stage1(), rd_lift(), rd_adv()], "{pid:04x}");
        assert_eq!(extras(&r), vec![(Extra::Debounce, Some(4)), (Extra::MotionSync, Some(1)), (Extra::AngleSnap, Some(1)), (Extra::RippleControl, Some(0))], "{pid:04x}");
        // OpenMouse's R1 debounce list runs 0-20 ms
        assert_eq!(r.extras[0].kind, Kind::Steps { values: vec![0, 1, 2, 4, 8, 15, 20], unit: "ms" });
        assert_eq!(r.extras[1].kind, Kind::Switch);
        assert!(p.sent.iter().all(|(o, _)| atk::read_request_allowed(o)));
    }
    // R1 SE+ receiver: Motion Sync and ripple only (its debounce / angle snapping go through the unconfirmed live row)
    let mut mouse = with_adv(HidMouse::new_se((2, 32)));
    mouse.mem[0x70..0x74].copy_from_slice(&h("0b 02 00 53"));
    let (m, mut p, _) = hid_port(COMPX, 0xF58E, mouse);
    let r = read(&m, &mut p);
    assert_eq!(sent(&p), vec![id_req(), batt_req(), online_req(), rd_system6(), rd_stage1(), rd_live(), rd_adv()]);
    assert_eq!(extras(&r), vec![(Extra::MotionSync, Some(1)), (Extra::RippleControl, Some(0))]);
    // the catalog-only R1 SE+ 2,31: OpenMouse writes nothing to it, so no rows and no advanced read
    let (m, mut p, _) = hid_port(COMPX, 0xF58F, with_adv(HidMouse::new_se((2, 31))));
    let r = read(&m, &mut p);
    assert!(r.extras.is_empty() && !sent(&p).contains(&rd_adv()));
    // an advanced block that is not answered: the rows stay, their values are unknown
    let st = Rc::new(RefCell::new(with_adv(HidMouse::new((2, 27)))));
    let s2 = st.clone();
    let (m, mut p, _) = hid_port(COMPX, 0xF58C, HidMouse::new((2, 27)));
    p.answer = Box::new(move |o, how| if o[1] == 0x08 && o[4] == 0xA9 { Err(Error::BadAnswer("no answer".into())) } else { s2.borrow_mut().answer(o, how) });
    let r = atk::Atk.read(&m, &mut p, ONE).unwrap();
    assert_eq!(extras(&r), vec![(Extra::Debounce, None), (Extra::MotionSync, None), (Extra::AngleSnap, None), (Extra::RippleControl, None)]);
}

#[test]
fn order_110_hid_changes_are_openmouses_writeadvanced_bytes() {
    let writes = |p: &ScriptPort| p.sent.iter().filter(|(o, _)| o[1] == 0x07).map(|(o, how)| (hex(o), matches!(how, HidTransfer::OutputOnly))).collect::<Vec<_>>();
    // R1 Pro Max over the cable: identify, read the block, write it whole with one pair replaced (no answer), read it back
    let (m, mut p, st) = hid_port(COMPX, 0xF58C, with_adv(HidMouse::new((2, 27))));
    for (c, pairs) in [
        (Change::Extra(Extra::Debounce, 8), "08 4d 01 54 06 4f 01 54 00 55"),     // setDebounceTime(8): offset 0 <- [08, 0x55 - 8]
        (Change::Extra(Extra::MotionSync, 0), "08 4d 00 55 06 4f 01 54 00 55"),   // setMotionSync(false): offset 2
        (Change::Extra(Extra::AngleSnap, 0), "08 4d 00 55 06 4f 00 55 00 55"),    // setAngleSnapping(false): offset 6
        (Change::Extra(Extra::RippleControl, 1), "08 4d 00 55 06 4f 00 55 01 54"), // setRippleControl(true): offset 8
    ] {
        p.sent.clear();
        atk::Atk.change(&m, &mut p, c).unwrap_or_else(|e| panic!("{c:?}: {e}"));
        assert_eq!(sent(&p), vec![id_req(), rd_adv(), wr_adv(pairs), rd_adv()], "{c:?}");
        assert!(is_out_only(&p, 2), "{c:?}: fire-and-forget over the cable");
    }
    assert_eq!(st.borrow().mem[0xA9..0xB3], h("08 4d 00 55 06 4f 00 55 01 54"), "sleep (offset 4) copied back unchanged");
    let r = read(&m, &mut p);
    assert_eq!(extras(&r), vec![(Extra::Debounce, Some(8)), (Extra::MotionSync, Some(0)), (Extra::AngleSnap, Some(0)), (Extra::RippleControl, Some(1))]);
    // the Pro Max receiver: writeR1ProMaxReceiverAdvanced - the same block, echoed back by the receiver
    let (m, mut p, _) = hid_port(COMPX, 0xF58A, with_adv(HidMouse::new((2, 27))));
    atk::Atk.change(&m, &mut p, Change::Extra(Extra::Debounce, 20)).unwrap();
    assert_eq!(sent(&p), vec![id_req(), rd_adv(), wr_adv("14 41 01 54 06 4f 01 54 00 55"), rd_adv()]);
    assert!(matches!(p.sent[2].1, HidTransfer::OutputThenInput { echo: Some((1, 0x07)), .. }));
    // the R1 SE+ receiver (write() is allowed on the live-settings transport) and the A9 Plus
    let (m, mut p, _) = hid_port(COMPX, 0xF58E, with_adv(HidMouse::new_se((2, 32))));
    atk::Atk.change(&m, &mut p, Change::Extra(Extra::MotionSync, 0)).unwrap();
    assert_eq!(writes(&p), vec![(wr_adv("04 51 00 55 06 4f 01 54 00 55"), true)]);
    let (m, mut p, _) = hid_port(ATK, 0x1115, with_adv(HidMouse::new((2, 83))));
    atk::Atk.change(&m, &mut p, Change::Extra(Extra::RippleControl, 1)).unwrap();
    assert_eq!(writes(&p), vec![(wr_adv("04 51 01 54 06 4f 01 54 01 54"), true)]);
    // a mouse that keeps the old value is an error
    let mut mouse = with_adv(HidMouse::new((2, 27)));
    mouse.ignore_writes = true;
    let (m, mut p, _) = hid_port(COMPX, 0xF58C, mouse);
    assert!(matches!(atk::Atk.change(&m, &mut p, Change::Extra(Extra::MotionSync, 0)), Err(Error::BadAnswer(_))));
}

#[test]
fn order_110_hid_refuses_what_openmouse_does_not_take_before_a_byte_leaves() {
    let no_writes = |p: &ScriptPort| p.sent.iter().all(|(o, _)| atk::read_request_allowed(o));
    let (m, mut p, _) = hid_port(COMPX, 0xF58C, with_adv(HidMouse::new((2, 27))));
    // debounce: only OpenMouse's 0/1/2/4/8/15/20 ms; a switch takes 0 / 1; turbo is not offered (the 0x00b5 performance
    // mode is ATK's generic "performanceMode", not the Pulsar turbo byte)
    for c in [
        Change::Extra(Extra::Debounce, 3),
        Change::Extra(Extra::Debounce, 21),
        Change::Extra(Extra::Debounce, -1),
        Change::Extra(Extra::MotionSync, 2),
        Change::Extra(Extra::Turbo, 1),
    ] {
        p.sent.clear();
        assert!(atk::Atk.change(&m, &mut p, c).is_err(), "{c:?}");
        assert!(no_writes(&p), "{c:?}: nothing but reads");
    }
    // the SE+ receiver: debounce and angle snapping are held back (live row, unconfirmed)
    let (m, mut p, st) = hid_port(COMPX, 0xF58E, with_adv(HidMouse::new_se((2, 32))));
    for c in [Change::Extra(Extra::Debounce, 4), Change::Extra(Extra::AngleSnap, 1)] {
        assert!(atk::Atk.change(&m, &mut p, c).is_err(), "{c:?}");
    }
    assert!(no_writes(&p) && st.borrow().writes.is_empty());
    // the SE+ 2,31: nothing
    let (m, mut p, st) = hid_port(COMPX, 0xF58F, with_adv(HidMouse::new_se((2, 31))));
    assert!(atk::Atk.change(&m, &mut p, Change::Extra(Extra::MotionSync, 0)).is_err());
    assert!(no_writes(&p) && st.borrow().writes.is_empty());
    // the allow-list: the advanced read passes, its write and a read of another length do not
    assert!(atk::read_request_allowed(&h(&rd_adv())));
    assert!(!atk::read_request_allowed(&h(&wr_adv("04 51 01 54 06 4f 01 54 00 55"))));
    assert!(!atk::read_request_allowed(&atk::hid_wire(&atk::read_eeprom_frame(0x00A9, 4), 17)));
}

#[test]
fn order_110_zero_reads_and_writes_the_config_flags_like_openmouse() {
    // bitmouseDecodeConfig: payload[6] motionSync, [7] linearCorrection (angle snapping), [8] rippleControl, [11] debounceMs
    let mut mouse = ZeroMouse::new(0);
    mouse.flags = [1, 1, 0];
    mouse.debounce = 8;
    let (m, mut p, st) = zero_port(0x1154, mouse);
    let r = read(&m, &mut p);
    assert_eq!(extras(&r), vec![(Extra::Debounce, Some(8)), (Extra::MotionSync, Some(1)), (Extra::AngleSnap, Some(1)), (Extra::RippleControl, Some(0))]);
    assert_eq!(r.extras[0].kind, Kind::Range { min: 0, max: 15, step: 1, unit: "ms" });
    assert_eq!(p.sent.len(), 10, "no read beyond the Order 104 ones: config, battery, charging, 7 table chunks");
    // bitmouseSetFlagRequest(cmd, [3, 1], value): checksum = 0x72 + 3 + 0 + target + cmd + 1 + value
    let cfg = bm("08 9f 72 13 00 00 09 11");
    for (c, frame) in [
        (Change::Extra(Extra::MotionSync, 0), bm("08 83 72 03 00 00 0d 01 00")),    // 114 + 3 + 13 + 1 + 0 = 131
        (Change::Extra(Extra::AngleSnap, 0), bm("08 81 72 03 00 00 0b 01 00")),     // 114 + 3 + 11 + 1 + 0 = 129
        (Change::Extra(Extra::RippleControl, 1), bm("08 83 72 03 00 00 0c 01 01")), // 114 + 3 + 12 + 1 + 1 = 131
        (Change::Extra(Extra::Debounce, 15), bm("08 9b 72 03 00 00 16 01 0f")),     // 114 + 3 + 22 + 1 + 15 = 155
    ] {
        p.sent.clear();
        atk::Atk.change(&m, &mut p, c).unwrap_or_else(|e| panic!("{c:?}: {e}"));
        assert_eq!(sent(&p), vec![frame, cfg.clone()], "{c:?}");
        assert!(is_out_only(&p, 0), "{c:?}: fire-and-forget");
    }
    assert_eq!((st.borrow().flags, st.borrow().debounce), ([0, 0, 1], 15));
    // behind the receiver: target 1, checksum + 1 (debounce 2: 114 + 3 + 1 + 22 + 1 + 2 = 143)
    let (m, mut p, _) = zero_port(0x1155, ZeroMouse::new(1));
    atk::Atk.change(&m, &mut p, Change::Extra(Extra::Debounce, 2)).unwrap();
    assert_eq!(sent(&p)[0], bm("08 8f 72 03 00 01 16 01 02"));
    // a mouse that keeps the old value
    let mut mouse = ZeroMouse::new(0);
    mouse.ignore_writes = true;
    let (m, mut p, _) = zero_port(0x1154, mouse);
    assert!(matches!(atk::Atk.change(&m, &mut p, Change::Extra(Extra::MotionSync, 1)), Err(Error::BadAnswer(_))));
    // refused before a byte leaves: debounce 0-15 only, switches 0 / 1, nothing else
    let (m, mut p, _) = zero_port(0x1154, ZeroMouse::new(0));
    for c in [Change::Extra(Extra::Debounce, 16), Change::Extra(Extra::Debounce, -1), Change::Extra(Extra::RippleControl, 2), Change::Extra(Extra::Turbo, 1)] {
        assert!(atk::Atk.change(&m, &mut p, c).is_err(), "{c:?}");
    }
    assert!(p.sent.is_empty(), "nothing sent");
    // the flag writes are not reads
    for frame in ["08 83 72 03 00 00 0d 01 00", "08 9b 72 03 00 00 16 01 0f"] {
        assert!(!atk::read_request_allowed(&h(&bm(frame))));
    }
}

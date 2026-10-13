//! ATK / VXE - ported from OpenMouse (`crate::family`: AGPL-3.0-or-later, @openmouse/protocol 0.40.1), DPI / polling /
//! lift-off / battery only. OpenMouse has TWO ATK clients (registry.js, in this order, first match wins - they never claim the
//! same interface, so the order does not matter for the models here):
//!
//! | client (OpenMouse file) | claims | models here |
//! |---|---|---|
//! | `AtkBitmouseHidClient` (`drivers/atk/bitmouse-hid.js` + `bitmouse/index.js`) | vendor 0x373B, pid 0x1154 / 0x1155, vendor collection 0xFF05:0x0001 with output report 0x08 | ATK ZERO (wired 0x1154, receiver 0x1155) |
//! | `AtkHidClient` (`drivers/atk/hid.js` + `atk/index.js`, `products.js`, `device-catalog.generated.js`) | vendor 0x373B (any pid) or CompX 0x3554 pids f58a / f58c / f58e / f58f, collection 0xFF02:0x0002 | VXE R1 Pro Max (f58a receiver, f58c wired), VXE R1 SE+ (f58e receiver, f58f wired), ATK A9 Plus (0x373B:0x1115) |
//!
//! ## `hid.js` (the older "EEPROM" protocol - also the Endgame Gear WE framing)
//! One OUTPUT report id 0x08 carrying 16 bytes `[cmd, 0, hi, lo, len, data.., crc]`; the answer is an INPUT report id 0x08 with
//! 16 bytes. crc = `0x55 - (0x08 + sum of the first 15 bytes)` (so the whole report, id included, sums to 0x55). Commands used:
//! 0x10 CID/MID (identity: `[cid, mid]` at bytes 5-6), 0x04 battery (`[percent, charging, mV hi, mV lo]` at 5..), 0x08 read EEPROM
//! (`[0, addr hi, addr lo, len]`; answer echoes address and has data at byte 5), 0x07 write EEPROM (`[0, hi, lo, len, data..]`), 0x03
//! "is the mouse online" (receiver f58e only: byte 5 == 1). Every stored value is a pair `[v, 0x55 - v]`. Registers: 0x0000 system
//! (polling code, stage count, active stage - pairs), 0x000A lift-off (pair), 0x000C DPI stages (4 bytes each:
//! `[x, y, mode nibbles, crc]`), 0x0070 live-settings row `[selector, value, 0, 0x55 - value]` (R1 SE+ receiver only).
//! Wired mice are written fire-and-forget (`sendReport`, then 10 ms; R1 mice wait 250 ms more after polling); the R1 Pro Max
//! RECEIVER (f58a) is written with echo-checks: the whole 10-byte system row (polling), the 8-byte stage pair (DPI), the lift-off
//! pair, each echoed back by the receiver before the normal read-back.
//! The mouse is identified first (CID/MID); each model here is accepted ONLY with the identity OpenMouse hand-verified
//! (`ATK_PRODUCTS`): R1 Pro Max 2,27; R1 SE+ 2,32 (the catalog also lists 2,31 on f58f - OpenMouse blocks EEPROM writes for it,
//! so it is read-only here); A9 Plus Nearlink 2,83. Any other identity is refused ([`Error::UnsupportedMouse`]).
//!
//! | model | sensor / DPI | polling | lift-off | battery |
//! |---|---|---|---|---|
//! | VXE R1 Pro Max | PAW3395: 100-30000, step 50 | 125 250 500 1000 | 1 mm, 2 mm (R1 codes 1 / 2) | yes |
//! | VXE R1 SE+ | PAW3395SE: 200-10000 step 50, then 10100-18000 step 100 (code table) | wired 125..1000; receiver (live row) 250 500 1000 | wired 1 mm, 2 mm (receiver: none, held back) | yes |
//! | ATK A9 Plus (Nearlink) | PAW3395: 100-30000, step 50 | 125 250 500 1000 (`maxPollingHz` 1000) | 1 mm, 2 mm | yes |
//!
//! ## `bitmouse-hid.js` (ATK ZERO)
//! Output report 0x08 with a 63-byte frame `[checksum, 0x72, paramLen, 0, target, cmd, cmdLen, payload..]` (checksum = sum of
//! bytes 1..62; target 1 = the mouse behind the receiver); the answer is input report 0x08 `[0x72, status (0xFF = error), sn,
//! target, cmd, len, payload..]`. Reads: 9 config (polling code at payload[2], lift-off level = payload[5] + 1), 7 battery, 15
//! charging, 23 address data (the DPI table: 7 chunks of 10 bytes from address 1 = `[current stage, count, 8 stages x
//! (x lo hi, y lo hi, blue, green, red, reserved)]`). Writes (fire-and-forget, then 120 ms, then the read-back): 1 report rate
//! `[code]`, 2 DPI `[stage, x, y, blue, green, red, 0, enable]` for the ACTIVE stage (colour copied from what was read), 3
//! lift-off `[0, level - 1]`. DPI 10-42000 (10 up to 10000, 50 up to 30000, 100 above); polling 125..8000 (all seven codes, as
//! OpenMouse offers them); lift-off 0.7-1.7 mm in 0.1 mm steps (level + 6 tenths: OpenMouse's slider is in mm).
//!
//! ## Left out on purpose
//! Everything that is not DPI / polling / lift-off / battery: buttons, macros, RGB and DPI-LED colours, DPI stage count / active
//! stage / stage colours, debounce, sleep, motion sync, ripple, angle snapping, performance / sensor modes, long-range, profile
//! switching (`setCurrentConfig` 0x0f), receiver pairing (0x05 / 0x06), dongle light, firmware version, and every command the
//! codec lists for firmware work (0x09 factory restore, 0x0d USB upgrade mode, 0x5a / 0x5b upgrade reports) - none is built here.
//! R1 SE+ receiver polling read: OpenMouse decodes byte 1 of the live row whatever setting was written last (and falls back to
//! 1000 Hz); this port decodes it only when the row's selector byte is the polling selector (0x0b), else the rate is unknown.
//! R1 SE+ receiver (live row 0x0070): polling is written and the row read back like OpenMouse; the result is only accepted when
//! the row shows the asked-for rate (another rate, or a row that does not decode, is an error). Its LIFT-OFF is HELD BACK: OpenMouse
//! overwrites the whole live row with `[03, code, 0, 0x55 - code]`, reads nothing before or after and takes it as done, so nothing
//! can confirm it (and the "before" would come from another register): no lift-off row, the write is refused, no read of 0x000a.
//! A write the R1 Pro Max receiver must echo is sent ONCE (as OpenMouse does); a report that is a write echo but not this write
//! is an error and is never answered with a second write. NEEDS TRANSFER (small): OpenMouse keeps listening after such a
//! mismatching echo; our `OutputThenInput` cannot skip it by content, only by the command byte.
//! Never run on a real unit; proven against OpenMouse's bytes (tests/openmouse_atk.rs).

use crate::error::{Error, Result};
use crate::family::{Caps, Changed, DpiCaps, Driver, Family, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

/// ATK's own USB vendor id (`VENDOR_ID.atk`).
pub const VENDOR_ATK: u16 = 0x373B;
/// CompX's id (`VENDOR_ID.vgn`), where the VXE R1 Pro Max / R1 SE+ sit.
pub const VENDOR_COMPX: u16 = 0x3554;
/// Both protocols use report id 8.
pub const REPORT_ID: u8 = 0x08;

// ---------------------------------------------------------------- the device table

/// `Matched.variant` values - which client and which model line (see [`Kind`]).
pub const V_ZERO_WIRED: u16 = 1;
pub const V_ZERO_RECEIVER: u16 = 2;
pub const V_PRO_MAX_WIRED: u16 = 3;
pub const V_PRO_MAX_RECEIVER: u16 = 4;
pub const V_SE_WIRED: u16 = 5;
pub const V_SE_RECEIVER: u16 = 6;
pub const V_A9_PLUS: u16 = 7;

/// One row of the table: (vendor, product, model name from OpenMouse's SUPPORTED list, id says wireless?, variant).
pub const PRODUCTS: [(u16, u16, &str, Option<bool>, u16); 7] = [
    (VENDOR_ATK, 0x1154, "ATK ZERO", Some(false), V_ZERO_WIRED),
    (VENDOR_ATK, 0x1155, "ATK ZERO", Some(true), V_ZERO_RECEIVER),
    (VENDOR_COMPX, 0xF58C, "VXE R1 Pro Max", Some(false), V_PRO_MAX_WIRED),
    (VENDOR_COMPX, 0xF58A, "VXE R1 Pro Max", Some(true), V_PRO_MAX_RECEIVER),
    (VENDOR_COMPX, 0xF58F, "VXE R1 SE+", Some(false), V_SE_WIRED),
    (VENDOR_COMPX, 0xF58E, "VXE R1 SE+", Some(true), V_SE_RECEIVER),
    (VENDOR_ATK, 0x1115, "ATK A9 Plus", None, V_A9_PLUS),
];

/// The model line behind a `Matched.variant`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    ZeroWired,
    ZeroReceiver,
    ProMaxWired,
    ProMaxReceiver,
    SeWired,
    SeReceiver,
    A9Plus,
}

impl Kind {
    fn of(variant: u16) -> Option<Kind> {
        Some(match variant {
            V_ZERO_WIRED => Kind::ZeroWired,
            V_ZERO_RECEIVER => Kind::ZeroReceiver,
            V_PRO_MAX_WIRED => Kind::ProMaxWired,
            V_PRO_MAX_RECEIVER => Kind::ProMaxReceiver,
            V_SE_WIRED => Kind::SeWired,
            V_SE_RECEIVER => Kind::SeReceiver,
            V_A9_PLUS => Kind::A9Plus,
            _ => return None,
        })
    }
    /// the BITMOUSE client (ATK ZERO) - else `hid.js`
    fn bitmouse(self) -> bool {
        matches!(self, Kind::ZeroWired | Kind::ZeroReceiver)
    }
    fn receiver(self) -> bool {
        matches!(self, Kind::ZeroReceiver | Kind::ProMaxReceiver | Kind::SeReceiver)
    }
    /// OpenMouse `isR1()`: up to 8 DPI stages, 250 ms settle after a polling write
    fn r1(self) -> bool {
        matches!(self, Kind::ProMaxWired | Kind::ProMaxReceiver | Kind::SeWired | Kind::SeReceiver)
    }
    /// OpenMouse `usesR1LiveSettings()`: polling / lift-off go through the live row 0x0070
    fn live(self) -> bool {
        self == Kind::SeReceiver
    }
    /// the R1 Pro Max receiver is written with echo-checked whole rows
    fn echo_writes(self) -> bool {
        self == Kind::ProMaxReceiver
    }
    fn max_stages(self) -> usize {
        if self.r1() {
            8
        } else {
            6
        }
    }
    fn sensor(self) -> Sensor {
        match self {
            Kind::SeWired | Kind::SeReceiver => Sensor::Paw3395Se,
            _ => Sensor::Paw3395,
        }
    }
    /// the CID/MID identities OpenMouse hand-verifies for this model (`ATK_PRODUCTS`, plus the catalog's 2,31 on f58f)
    fn identities(self) -> &'static [(u8, u8)] {
        match self {
            Kind::ProMaxWired | Kind::ProMaxReceiver => &[(2, 27)],
            Kind::SeWired => &[(2, 32), (2, 31)],
            Kind::SeReceiver => &[(2, 32)],
            Kind::A9Plus => &[(2, 83)],
            Kind::ZeroWired | Kind::ZeroReceiver => &[],
        }
    }
    /// what the mouse itself is called (`brand model` of `ATK_PRODUCTS`)
    fn mouse_name(self) -> &'static str {
        match self {
            Kind::ProMaxWired | Kind::ProMaxReceiver => "VXE R1 Pro Max",
            Kind::SeWired | Kind::SeReceiver => "VXE R1 SE+",
            Kind::A9Plus => "ATK A9 Plus Nearlink",
            Kind::ZeroWired | Kind::ZeroReceiver => "ATK ZERO",
        }
    }
    /// OpenMouse writes EEPROM for this identity? (R1 mice: only the "verified wired transport", i.e. SE+ 2,32; the SE+ 2,31
    /// of the catalog is blocked by `write()`)
    fn writable(self, id: (u8, u8)) -> bool {
        !(self == Kind::SeWired && id == (2, 31))
    }
    fn polling_rates(self) -> Vec<u32> {
        match self {
            // BITMOUSE_POLLING_RATES sorted ascending (all seven codes, as OpenMouse's getSupportedPollingRates)
            Kind::ZeroWired | Kind::ZeroReceiver => vec![125, 250, 500, 1000, 2000, 4000, 8000],
            Kind::SeReceiver => LIVE_POLLING.iter().map(|p| p.1).collect(),
            _ => vec![125, 250, 500, 1000],
        }
    }
    fn caps(self, id: (u8, u8)) -> Caps {
        if self.bitmouse() {
            return Caps { dpi: Some(bm_dpi_caps()), polling: self.polling_rates(), lift_off: (BM_LIFT_MIN + 6..=BM_LIFT_MAX + 6).collect(), battery: true };
        }
        if !self.writable(id) {
            return Caps { battery: true, ..Default::default() };
        }
        // the R1 SE+ receiver (live row) has no lift-off: its write cannot be read back and OpenMouse only assumes it worked
        let lift_off = if self.live() { Vec::new() } else { vec![10, 20] };
        Caps { dpi: Some(dpi_caps(self.sensor())), polling: self.polling_rates(), lift_off, battery: true }
    }
}

/// The `hid.js` config collection: vendor page 0xFF02, usage 2, a 17-byte report each way (id + 16).
fn hid_interface<'a>(own: &[&'a HidInfo]) -> Option<&'a HidInfo> {
    own.iter().find(|h| h.usage_page == 0xFF02 && h.usage == 0x0002 && h.output_len as usize >= HID_WIRE_LEN && h.input_len as usize >= HID_WIRE_LEN).copied()
}

/// The BITMOUSE config collection: vendor page 0xFF05, usage 1, a 64-byte output report (id + 63 frame bytes).
fn bm_interface<'a>(own: &[&'a HidInfo]) -> Option<&'a HidInfo> {
    own.iter().find(|h| h.usage_page == BM_USAGE_PAGE && h.usage == BM_USAGE && h.output_len as usize >= BM_WIRE_LEN && h.input_len as usize >= BM_MIN_IN).copied()
}

/// The device table: VID:PID -> model (OpenMouse's SUPPORTED list only) and the interface to talk to.
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    let &(_, _, model, wireless, variant) = PRODUCTS.iter().find(|p| p.0 == vid && p.1 == pid)?;
    let kind = Kind::of(variant)?;
    let config = if kind.bitmouse() { bm_interface(own)? } else { hid_interface(own)? };
    Some(Matched { family: Family::Atk, model: Some(model), wireless, config: config.clone(), variant })
}

// ================================================================ hid.js codec

pub const HID_FRAME_LEN: usize = 16;
/// report id + 16 bytes
pub const HID_WIRE_LEN: usize = 17;
pub const CMD_ONLINE: u8 = 0x03;
pub const CMD_BATTERY: u8 = 0x04;
pub const CMD_WRITE_EEPROM: u8 = 0x07;
pub const CMD_READ_EEPROM: u8 = 0x08;
pub const CMD_CIDMID: u8 = 0x10;
pub const ADDR_SYSTEM: u16 = 0x0000;
pub const ADDR_LIFT_OFF: u16 = 0x000A;
pub const ADDR_DPI: u16 = 0x000C;
pub const ADDR_LIVE: u16 = 0x0070;
/// the live row's selectors (`ATK_VXE_R1_*_SELECTOR`)
pub const SEL_LIFT_OFF: u8 = 0x03;
pub const SEL_POLLING: u8 = 0x0B;
/// OpenMouse's `REPLY_TIMEOUT_MS`
const HID_TIMEOUT_MS: u32 = 500;
/// `WRITE_SETTLE_MS`
const HID_SETTLE_MS: u64 = 10;
/// `R1_LIVE_WRITE_SETTLE_MS`
const R1_SETTLE_MS: u64 = 250;
/// asks of one read / identify (OpenMouse's `MAX_IDENTIFY_ATTEMPTS` is 3)
const TRIES: usize = 3;

/// `weReportChecksum(0x08, first 15 bytes)`: `0x55 - (0x08 + sum)`.
pub fn hid_crc(first15: &[u8]) -> u8 {
    let sum = first15.iter().fold(REPORT_ID as u32, |s, b| s + *b as u32);
    (0x55u32.wrapping_sub(sum & 0xFF) & 0xFF) as u8
}

/// `weBuildCmdPayload(cmd, args)`: the 16-byte frame.
pub fn hid_frame(cmd: u8, args: &[u8]) -> [u8; HID_FRAME_LEN] {
    let mut f = [0u8; HID_FRAME_LEN];
    f[0] = cmd;
    for (i, b) in args.iter().take(14).enumerate() {
        f[i + 1] = *b;
    }
    f[15] = hid_crc(&f[..15]);
    f
}

/// The report as Windows sends it: id 0x08, the frame, zero-padded to the interface's output length.
pub fn hid_wire(frame: &[u8; HID_FRAME_LEN], output_len: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(output_len.max(HID_WIRE_LEN));
    v.push(REPORT_ID);
    v.extend_from_slice(frame);
    v.resize(output_len.max(HID_WIRE_LEN), 0);
    v
}

pub fn read_eeprom_frame(addr: u16, len: u8) -> [u8; HID_FRAME_LEN] {
    hid_frame(CMD_READ_EEPROM, &[0, (addr >> 8) as u8, addr as u8, len])
}

pub fn write_eeprom_frame(addr: u16, data: &[u8]) -> [u8; HID_FRAME_LEN] {
    let mut args = vec![0, (addr >> 8) as u8, addr as u8, data.len() as u8];
    args.extend_from_slice(data);
    hid_frame(CMD_WRITE_EEPROM, &args)
}

/// `wePackScalarPair`: `[v, 0x55 - v]`.
pub fn scalar_pair(v: u8) -> [u8; 2] {
    [v, 0x55u8.wrapping_sub(v)]
}

/// `weUnpackScalarPair`: the value, when the pair checks.
pub fn unpack_pair(v: u8, crc: u8) -> Option<u8> {
    (0x55u8.wrapping_sub(v) == crc).then_some(v)
}

fn frame_ok(f: &[u8]) -> bool {
    f.len() >= HID_FRAME_LEN && f[15] == hid_crc(&f[..15])
}

/// `POLLING_RATES` of hid.js: register code -> Hz.
pub const HID_POLLING: [(u8, u32); 7] = [(0x08, 125), (0x04, 250), (0x02, 500), (0x01, 1000), (0x10, 2000), (0x20, 4000), (0x40, 8000)];
/// `VXE_POLLING_CODES` (the R1 SE+ receiver's live row): code -> Hz.
pub const LIVE_POLLING: [(u8, u32); 3] = [(0x03, 250), (0x02, 500), (0x01, 1000)];

pub fn hid_polling_code(hz: u32) -> Option<u8> {
    HID_POLLING.iter().find(|p| p.1 == hz).map(|p| p.0)
}
pub fn hid_polling_hz(code: u8) -> Option<u32> {
    HID_POLLING.iter().find(|p| p.0 == code).map(|p| p.1)
}

/// `atkPackVxeR1LiveSetting`: `[selector, value, 0, 0x55 - value]`.
pub fn pack_live_setting(selector: u8, value: u8) -> [u8; 4] {
    [selector, value, 0, 0x55u8.wrapping_sub(value)]
}
pub fn live_polling_code(hz: u32) -> Option<u8> {
    LIVE_POLLING.iter().find(|p| p.1 == hz).map(|p| p.0)
}
pub fn live_polling_hz(code: u8) -> Option<u32> {
    LIVE_POLLING.iter().find(|p| p.0 == code).map(|p| p.1)
}

/// R1 / A9 Plus Nearlink lift-off codes (`R1_LIFT_OFF_CODES`: 1 = Low = 1 mm, 2 = High = 2 mm): tenths of a mm -> code.
pub fn lift_off_code(tenths: u32) -> Option<u8> {
    match tenths {
        10 => Some(1),
        20 => Some(2),
        _ => None,
    }
}
pub fn lift_off_tenths(code: u8) -> Option<u32> {
    match code {
        1 => Some(10),
        2 => Some(20),
        _ => None,
    }
}

/// The DPI encodings these models use (`ATK_SENSORS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sensor {
    /// PAW3395 (R1 Pro Max, A9 Plus): "step50", 100-30000
    Paw3395,
    /// PAW3395SE (R1 SE+): code table, 200-18000
    Paw3395Se,
}

/// `atkDpiOptionsForSensor` as bands.
pub fn dpi_caps(s: Sensor) -> DpiCaps {
    match s {
        Sensor::Paw3395 => DpiCaps::uniform(100, 30000, 50),
        Sensor::Paw3395Se => DpiCaps { bands: vec![(200, 10000, 50), (10100, 18000, 100)] },
    }
}

/// `PAW3395SE_CODES`: 1..=235 without the invalid codes - the code of DPI `(index + 1) * 50`.
pub fn se_codes() -> Vec<u8> {
    const INVALID: [u16; 35] = [7, 13, 20, 26, 33, 40, 46, 53, 60, 66, 73, 80, 86, 93, 100, 106, 113, 120, 126, 133, 140, 146, 153, 160, 166, 173, 180, 186, 193, 200, 206, 213, 220, 226, 233];
    (1u16..=235).filter(|c| !INVALID.contains(c)).map(|c| c as u8).collect()
}

/// One axis: (byte, nibble) - `atkEncodeDpiAxisStep50` / `atkEncodeDpiAxisPaw3395Se`.
fn encode_axis(s: Sensor, dpi: u32) -> Option<(u8, u8)> {
    match s {
        Sensor::Paw3395 => {
            // doubled above 30000 (never offered: the sensor stops at 30000)
            if dpi == 0 || dpi > 30000 {
                return None;
            }
            let count = (dpi as f64 / 50.0).round() as u32 - 1;
            Some(((count & 0xFF) as u8, (((count >> 8) & 3) << 2) as u8))
        }
        Sensor::Paw3395Se => {
            let doubled = dpi > 10000;
            let base = if doubled {
                if !dpi.is_multiple_of(2) {
                    return None;
                }
                dpi / 2
            } else {
                dpi
            };
            if !(50..=10000).contains(&base) || !base.is_multiple_of(50) {
                return None;
            }
            let code = *se_codes().get((base / 50 - 1) as usize)?;
            Some((code, if doubled { 2 } else { 0 }))
        }
    }
}

fn decode_axis(s: Sensor, byte: u8, nibble: u8) -> Option<u32> {
    match s {
        Sensor::Paw3395 => {
            let count = byte as u32 | ((((nibble >> 2) & 3) as u32) << 8);
            let dpi = (count + 1) * 50;
            Some(if nibble & 1 != 0 { dpi * 2 } else { dpi })
        }
        Sensor::Paw3395Se => {
            if nibble & !2 != 0 {
                return None;
            }
            let idx = se_codes().iter().position(|c| *c == byte)?;
            let base = (idx as u32 + 1) * 50;
            if nibble & 2 != 0 {
                (base > 5000).then_some(base * 2)
            } else {
                Some(base)
            }
        }
    }
}

/// `atkPackDpiStageForSensor(sensor, x, y)`: `[x byte, y byte, (y nibble << 4) | x nibble, 0x55 - sum]`.
pub fn encode_dpi_stage(s: Sensor, x: u32, y: u32) -> Option<[u8; 4]> {
    if !dpi_caps(s).allows(x) || !dpi_caps(s).allows(y) {
        return None;
    }
    let (bx, nx) = encode_axis(s, x)?;
    let (by, ny) = encode_axis(s, y)?;
    let mode = ((ny & 0x0F) << 4) | (nx & 0x0F);
    let sum = bx.wrapping_add(by).wrapping_add(mode);
    Some([bx, by, mode, 0x55u8.wrapping_sub(sum)])
}

/// `atkUnpackDpiStageForSensor`: (x, y), when the 4 bytes check.
pub fn decode_dpi_stage(s: Sensor, d: &[u8]) -> Option<(u32, u32)> {
    if d.len() < 4 || d[0].wrapping_add(d[1]).wrapping_add(d[2]).wrapping_add(d[3]) != 0x55 {
        return None;
    }
    Some((decode_axis(s, d[0], d[2] & 0x0F)?, decode_axis(s, d[1], d[2] >> 4)?))
}

// ---------------------------------------------------------------- hid.js exchanges

fn gone() -> Error {
    Error::MouseGone("the mouse did not answer - it may be asleep or out of range".into())
}

/// One request -> the first answer with the right report id (and, with `echo`, the echoed command in byte 1).
fn ask(port: &mut dyn Port, frame: &[u8; HID_FRAME_LEN], echo: Option<u8>) -> Result<[u8; HID_FRAME_LEN]> {
    let (o, i) = (port.info().output_len as usize, port.info().input_len as usize);
    let how = HidTransfer::OutputThenInput { reply_len: i.max(HID_WIRE_LEN), max_reads: 16, timeout_ms: HID_TIMEOUT_MS, echo: echo.map(|c| (1, c)) };
    let a = port.exchange(&hid_wire(frame, o), &how)?;
    if a.len() < HID_WIRE_LEN || a[0] != REPORT_ID {
        return Err(Error::BadAnswer(format!("answer too short or not report 8 ({} bytes)", a.len())));
    }
    let mut f = [0u8; HID_FRAME_LEN];
    f.copy_from_slice(&a[1..HID_WIRE_LEN]);
    Ok(f)
}

/// OpenMouse's `read(address, length)`: the answer must echo the address, carry >= length bytes, check its crc (it may come
/// back under the write command id).
fn read_eeprom(port: &mut dyn Port, addr: u16, len: usize) -> Result<Vec<u8>> {
    let mut last = Error::BadAnswer(format!("read {addr:#06x}"));
    for _ in 0..TRIES {
        match ask(port, &read_eeprom_frame(addr, len as u8), None) {
            Ok(f) => {
                if (f[0] == CMD_READ_EEPROM || f[0] == CMD_WRITE_EEPROM) && f[1] == 0 && frame_ok(&f) && f[2] == (addr >> 8) as u8 && f[3] == addr as u8 && f[4] as usize >= len {
                    return Ok(f[5..5 + len].to_vec());
                }
                last = Error::BadAnswer(format!("read {addr:#06x}: unexpected answer {:02x?}", &f[..6]));
            }
            Err(e @ Error::BadAnswer(_)) => last = e,
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

/// A write the mouse does not answer (wired mice): the report, then OpenMouse's 10 ms settle.
fn write_fire(port: &mut dyn Port, addr: u16, data: &[u8]) -> Result<()> {
    let o = port.info().output_len as usize;
    port.exchange(&hid_wire(&write_eeprom_frame(addr, data), o), &HidTransfer::OutputOnly)?;
    port.pause_ms(HID_SETTLE_MS);
    Ok(())
}

/// A write the receiver must echo (the R1 Pro Max receiver): the answer is the same write, byte for byte.
fn write_echo(port: &mut dyn Port, addr: u16, data: &[u8]) -> Result<()> {
    // OpenMouse sends the write ONCE and then waits for the echo (an unrelated report is ignored, never answered with a second
    // write): one send here too. The service skips input reports that do not echo the write command; the first one that does
    // must be this write, byte for byte - else the error stands and nothing is sent again.
    let frame = write_eeprom_frame(addr, data);
    let f = ask(port, &frame, Some(CMD_WRITE_EEPROM))?;
    if f[..15] == frame[..15] && frame_ok(&f) {
        Ok(())
    } else {
        Err(Error::BadAnswer(format!("write {addr:#06x}: the echo differs from the write")))
    }
}

fn put(kind: Kind, port: &mut dyn Port, addr: u16, data: &[u8]) -> Result<()> {
    if kind.echo_writes() {
        write_echo(port, addr, data)
    } else {
        write_fire(port, addr, data)
    }
}

/// OpenMouse's `identify()`: command 0x10, up to `tries` asks (OpenMouse stops after 3); `None` = never answered.
fn identify(port: &mut dyn Port, tries: u32) -> Result<Option<(u8, u8)>> {
    for _ in 0..tries.clamp(1, TRIES as u32) {
        match ask(port, &hid_frame(CMD_CIDMID, &[]), Some(CMD_CIDMID)) {
            Ok(f) if f[4] >= 2 => return Ok(Some((f[5], f[6]))),
            Ok(_) | Err(Error::BadAnswer(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// `readBattery`: (percent, charging); None when unanswered.
fn read_battery(port: &mut dyn Port) -> Result<Option<(Option<u8>, Option<bool>)>> {
    match ask(port, &hid_frame(CMD_BATTERY, &[]), Some(CMD_BATTERY)) {
        Ok(f) => {
            let len = (f[4] as usize).min(10);
            Ok(Some((if len >= 1 { Some(f[5].min(100)) } else { None }, if len >= 2 { Some(f[6] != 0) } else { None })))
        }
        Err(Error::BadAnswer(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

/// `readR1ReceiverInfo`'s first command (0x03): is the mouse behind the receiver online? `None` = no usable answer.
fn read_online(port: &mut dyn Port) -> Result<Option<bool>> {
    match ask(port, &hid_frame(CMD_ONLINE, &[]), Some(CMD_ONLINE)) {
        Ok(f) if f[1] == 0 && f[4] >= 1 && frame_ok(&f) => Ok(Some(f[5] == 1)),
        Ok(_) | Err(Error::BadAnswer(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

fn soft<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(Error::BadAnswer(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

/// `stageCount` / `stageIndex` of OpenMouse: from the first 6 bytes of the system row.
fn stage_of(kind: Kind, system: &[u8]) -> (usize, usize) {
    let count = (system[2] as usize).clamp(1, kind.max_stages());
    (count, (system[4] as usize).min(count - 1))
}

fn stage_addr(index: usize) -> u16 {
    ADDR_DPI + 4 * index as u16
}

/// Who answered CID/MID, and is it a mouse this model line accepts?
fn check_identity(kind: Kind, id: (u8, u8)) -> Result<()> {
    if kind.identities().contains(&id) {
        Ok(())
    } else {
        Err(Error::UnsupportedMouse(format!("{} answered CID/MID {},{} - not the {} this driver was verified for", kind.mouse_name(), id.0, id.1, kind.mouse_name())))
    }
}

fn hid_read(kind: Kind, m: &Matched, port: &mut dyn Port, opt: ReadOptions) -> Result<OnMouse> {
    let default_id = kind.identities()[0];
    let mut r = OnMouse { model: Some(kind.mouse_name().to_string()), wireless: m.wireless, caps: kind.caps(default_id), ..Default::default() };
    let Some(id) = identify(port, opt.online_tries)? else {
        return Ok(r);
    };
    check_identity(kind, id)?;
    r.caps = kind.caps(id);
    if let Some((pct, charging)) = read_battery(port)? {
        r.battery_percent = pct;
        r.charging = charging;
    }
    if kind.live() && read_online(port)? == Some(false) {
        return Ok(r);
    }
    let Some(system) = soft(read_eeprom(port, ADDR_SYSTEM, 6))? else {
        return Ok(r);
    };
    r.online = true;
    let (_, active) = stage_of(kind, &system);
    r.dpi = soft(read_eeprom(port, stage_addr(active), 4))?.and_then(|d| decode_dpi_stage(kind.sensor(), &d)).map(|d| d.0);
    r.polling_hz = if kind.live() { read_live_polling(port)? } else { hid_polling_hz(system[0]) };
    // not on the live-row receiver: its lift-off is set through the live row, which cannot be read (see the header)
    if !kind.live() {
        r.lift_off = soft(read_eeprom(port, ADDR_LIFT_OFF, 2))?.and_then(|d| lift_off_tenths(d[0]));
    }
    Ok(r)
}

/// The live row's polling rate: only when its selector byte says polling.
fn read_live_polling(port: &mut dyn Port) -> Result<Option<u32>> {
    Ok(soft(read_eeprom(port, ADDR_LIVE, 4))?.and_then(|row| if row[0] == SEL_POLLING { live_polling_hz(row[1]) } else { None }))
}

fn hid_write(kind: Kind, port: &mut dyn Port, s: Setting) -> Result<Changed> {
    // R1 SE+ receiver lift-off: OpenMouse overwrites the live row 0x0070 with [03, code, 0, 0x55 - code], reads nothing before or
    // after, and takes it as done - it cannot be confirmed (and "before" would come from another register). Held back: refused
    // before anything is sent.
    if kind.live() && matches!(s, Setting::LiftOff(_)) {
        return Err(Error::range("lift-off distance", format!("the {} receiver's lift-off write cannot be confirmed - not offered", kind.mouse_name())));
    }
    let id = identify(port, TRIES as u32)?.ok_or_else(gone)?;
    check_identity(kind, id)?;
    if !kind.writable(id) {
        return Err(Error::UnsupportedMouse(format!("OpenMouse does not write this {} (CID/MID {},{})", kind.mouse_name(), id.0, id.1)));
    }
    let sensor = kind.sensor();
    match s {
        Setting::Polling(hz) => {
            if !kind.polling_rates().contains(&hz) {
                return Err(Error::range("polling rate", format!("{hz} Hz is not offered for the {}", kind.mouse_name())));
            }
            if kind.live() {
                let code = live_polling_code(hz).ok_or_else(|| Error::range("polling rate", format!("{hz} Hz")))?;
                let before = read_live_polling(port)?;
                write_fire(port, ADDR_LIVE, &pack_live_setting(SEL_POLLING, code))?;
                // OpenMouse reads the row back, then waits 250 ms
                let row = soft(read_eeprom(port, ADDR_LIVE, 4))?;
                port.pause_ms(R1_SETTLE_MS);
                // the row must show the rate that was asked for: another rate, or a row that does not decode, is an error
                let now = row.filter(|r| r[0] == SEL_POLLING).and_then(|r| live_polling_hz(r[1]));
                if now != Some(hz) {
                    return Err(Error::BadAnswer(format!("the receiver's live row shows {now:?} Hz instead of {hz} Hz")));
                }
                return Ok(Changed { now: hz, before });
            }
            let code = hid_polling_code(hz).ok_or_else(|| Error::range("polling rate", format!("{hz} Hz")))?;
            let before_row = read_eeprom(port, ADDR_SYSTEM, if kind.echo_writes() { 10 } else { 6 })?;
            let before = hid_polling_hz(before_row[0]);
            if kind.echo_writes() {
                // the whole 10-byte system row: stage count, active stage and the unknown tail are copied back unchanged
                let mut row = before_row.clone();
                row[..2].copy_from_slice(&scalar_pair(code));
                write_echo(port, ADDR_SYSTEM, &row)?;
            } else {
                write_fire(port, ADDR_SYSTEM, &scalar_pair(code))?;
            }
            if kind.r1() {
                port.pause_ms(R1_SETTLE_MS);
            }
            let now = hid_polling_hz(read_eeprom(port, ADDR_SYSTEM, 6)?[0]);
            if now != Some(hz) {
                return Err(Error::BadAnswer(format!("the mouse kept {now:?} Hz instead of {hz} Hz")));
            }
            Ok(Changed { now: hz, before })
        }
        Setting::Dpi(dpi) => {
            let stage = encode_dpi_stage(sensor, dpi, dpi).ok_or_else(|| Error::range("DPI", format!("{dpi} can't be stored by this sensor")))?;
            let (_, index) = stage_of(kind, &read_eeprom(port, ADDR_SYSTEM, 6)?);
            let before = soft(read_eeprom(port, stage_addr(index), 4))?.and_then(|d| decode_dpi_stage(sensor, &d)).map(|d| d.0);
            if kind.echo_writes() {
                // an 8-byte pair of adjacent stages: the neighbour is copied back unchanged
                let addr = ADDR_DPI + (index / 2) as u16 * 8;
                let mut group = read_eeprom(port, addr, 8)?;
                group[(index % 2) * 4..(index % 2) * 4 + 4].copy_from_slice(&stage);
                write_echo(port, addr, &group)?;
            } else {
                write_fire(port, stage_addr(index), &stage)?;
            }
            let now = decode_dpi_stage(sensor, &read_eeprom(port, stage_addr(index), 4)?);
            if now != Some((dpi, dpi)) {
                return Err(Error::BadAnswer(format!("the mouse kept {now:?} instead of {dpi} DPI")));
            }
            Ok(Changed { now: dpi, before })
        }
        Setting::LiftOff(t) => {
            let code = lift_off_code(t).ok_or_else(|| Error::range("lift-off distance", format!("{t} tenths of a mm (the {} offers 1 mm and 2 mm)", kind.mouse_name())))?;
            let before = soft(read_eeprom(port, ADDR_LIFT_OFF, 2))?.and_then(|d| lift_off_tenths(d[0]));
            put(kind, port, ADDR_LIFT_OFF, &scalar_pair(code))?;
            let now = lift_off_tenths(read_eeprom(port, ADDR_LIFT_OFF, 2)?[0]);
            if now != Some(t) {
                return Err(Error::BadAnswer(format!("the mouse kept {now:?} instead of {t} tenths of a mm")));
            }
            Ok(Changed { now: t, before })
        }
    }
}

// ================================================================ BITMOUSE codec (ATK ZERO)

pub const BM_USAGE_PAGE: u16 = 0xFF05;
pub const BM_USAGE: u16 = 0x0001;
/// the frame without the report id
pub const BM_FRAME_LEN: usize = 63;
/// report id + frame
pub const BM_WIRE_LEN: usize = 64;
/// the shortest input report this driver talks to: id + 6 header bytes + the 17-byte config payload
const BM_MIN_IN: usize = 24;
pub const BM_CMD_CODE: u8 = 0x72;
pub const BM_PAYLOAD_OFFSET: usize = 7;
/// `BITMOUSE_ADDRESS_DATA_OFFSET`: getAddressData echoes address (2) and length (1) before the bytes
pub const BM_ADDR_DATA_OFFSET: usize = 3;
pub const BM_TARGET_DEVICE: u8 = 0;
pub const BM_TARGET_BEHIND_RECEIVER: u8 = 1;
pub const BM_CMD_SET_REPORT_RATE: u8 = 1;
pub const BM_CMD_SET_DPI: u8 = 2;
pub const BM_CMD_SET_SILENT_HEIGHT: u8 = 3;
pub const BM_CMD_BATTERY: u8 = 7;
pub const BM_CMD_CONFIG: u8 = 9;
pub const BM_CMD_CHARGING: u8 = 15;
pub const BM_CMD_ADDRESS_DATA: u8 = 23;
/// the request lengths the firmware wants, as (paramLen, cmdLen) - `BITMOUSE_LENGTHS`
pub const BM_LEN_SET_REPORT_RATE: (u8, u8) = (3, 1);
pub const BM_LEN_SET_DPI: (u8, u8) = (12, 10);
pub const BM_LEN_SET_SILENT_HEIGHT: (u8, u8) = (4, 2);
pub const BM_LEN_BATTERY: (u8, u8) = (2, 1);
pub const BM_LEN_CONFIG: (u8, u8) = (19, 17);
pub const BM_LEN_CHARGING: (u8, u8) = (2, 1);
pub const BM_LEN_ADDRESS_DATA: (u8, u8) = (10, 13);
/// the DPI table: 7 reads of 10 bytes from address 1
pub const BM_DPI_ADDRESS: u16 = 1;
pub const BM_DPI_CHUNK: u8 = 10;
pub const BM_DPI_CHUNKS: u16 = 7;
pub const BM_LIFT_MIN: u32 = 1;
pub const BM_LIFT_MAX: u32 = 11;
/// OpenMouse's `REPLY_TIMEOUT_MS` / `WRITE_SETTLE_MS` for this client
const BM_TIMEOUT_MS: u32 = 700;
const BM_SETTLE_MS: u64 = 120;

/// `BITMOUSE_POLLING_RATES`: code -> Hz.
pub const BM_POLLING: [(u8, u32); 7] = [(0, 1000), (1, 500), (2, 250), (3, 125), (4, 8000), (5, 4000), (6, 2000)];

pub fn bm_polling_code(hz: u32) -> Option<u8> {
    BM_POLLING.iter().find(|p| p.1 == hz).map(|p| p.0)
}
pub fn bm_polling_hz(code: u8) -> Option<u32> {
    BM_POLLING.iter().find(|p| p.0 == code).map(|p| p.1)
}

/// `BITMOUSE_DPI_RANGES.PAW3950Ultra` + `bitmouseDpiOptions`: 10 up to 9990 by 10, 10000 up to 29950 by 50, 30000 up to 42000 by 100.
pub fn bm_dpi_caps() -> DpiCaps {
    DpiCaps { bands: vec![(10, 9990, 10), (10000, 29950, 50), (30000, 42000, 100)] }
}

/// `bitmouseChecksum`: the sum of bytes 1.. of the frame.
pub fn bm_checksum(frame: &[u8]) -> u8 {
    frame.iter().skip(1).fold(0u8, |s, b| s.wrapping_add(*b))
}

/// `bitmouseEncodeRequest`: the 63-byte frame.
pub fn bm_frame(cmd: u8, lens: (u8, u8), target: u8, payload: &[u8]) -> [u8; BM_FRAME_LEN] {
    let mut f = [0u8; BM_FRAME_LEN];
    f[1] = BM_CMD_CODE;
    f[2] = lens.0;
    f[3] = 0;
    f[4] = target;
    f[5] = cmd;
    f[6] = lens.1;
    for (i, b) in payload.iter().take(BM_FRAME_LEN - BM_PAYLOAD_OFFSET).enumerate() {
        f[BM_PAYLOAD_OFFSET + i] = *b;
    }
    f[0] = bm_checksum(&f);
    f
}

/// The report as Windows sends it: id 0x08, the frame, zero-padded to the output length.
pub fn bm_wire(frame: &[u8; BM_FRAME_LEN], output_len: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(output_len.max(BM_WIRE_LEN));
    v.push(REPORT_ID);
    v.extend_from_slice(frame);
    v.resize(output_len.max(BM_WIRE_LEN), 0);
    v
}

/// `bitmouseAddressDataRequest(address, length)` payload.
pub fn bm_address_payload(address: u16, len: u8) -> [u8; 3] {
    [address as u8, (address >> 8) as u8, len]
}

/// The addresses of the DPI table reads (`bitmouseDpiBlockAddresses`): 1, 11, ... 61.
pub fn bm_dpi_addresses() -> Vec<u16> {
    (0..BM_DPI_CHUNKS).map(|i| BM_DPI_ADDRESS + i * BM_DPI_CHUNK as u16).collect()
}

/// A decoded answer (`bitmouseDecodeReply`; the bytes include the leading report id 8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BmReply {
    pub cmd: u8,
    pub is_error: bool,
    pub payload: Vec<u8>,
}

/// `[0x08, 0x72, status, sn, target, cmd, len, payload..]` - the payload is cut to the reported length (the tail of a reply
/// still holds the previous exchange's bytes).
pub fn bm_decode_reply(wire: &[u8]) -> Option<BmReply> {
    if wire.len() < 7 || wire[0] != REPORT_ID || wire[1] != BM_CMD_CODE {
        return None;
    }
    let reported = wire[6] as usize;
    let avail = wire.len() - 7;
    Some(BmReply { cmd: wire[5], is_error: wire[2] == 0xFF, payload: wire[7..7 + reported.min(avail)].to_vec() })
}

/// `bitmouseDecodeConfig` (the fields this driver uses).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BmConfig {
    pub polling_hz: Option<u32>,
    pub offset_calibration: u8,
}

pub fn bm_decode_config(p: &[u8]) -> Option<BmConfig> {
    if p.len() < 12 {
        return None;
    }
    Some(BmConfig { polling_hz: bm_polling_hz(p[2]), offset_calibration: p[5] })
}

/// Lift-off in tenths of a mm from the config block: level = offsetCalibration + 1, level + 6 tenths (0.7 mm = level 1).
pub fn bm_lift_off_tenths(offset_calibration: u8) -> Option<u32> {
    let level = offset_calibration as u32 + 1;
    (BM_LIFT_MIN..=BM_LIFT_MAX).contains(&level).then_some(level + 6)
}

/// One DPI stage of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BmStage {
    pub x: u32,
    pub y: u32,
    pub blue: u8,
    pub green: u8,
    pub red: u8,
}

/// `bitmouseDecodeDpiBlock`: `[current index, stage count, 8 x (x lo hi, y lo hi, blue, green, red, reserved)]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BmDpiBlock {
    pub current: usize,
    pub stages: Vec<BmStage>,
}

pub fn bm_decode_dpi_block(b: &[u8]) -> Option<BmDpiBlock> {
    if b.len() < 2 + 8 * 8 {
        return None;
    }
    let stages = (0..8)
        .map(|i| {
            let at = 2 + i * 8;
            BmStage { x: b[at] as u32 | (b[at + 1] as u32) << 8, y: b[at + 2] as u32 | (b[at + 3] as u32) << 8, blue: b[at + 4], green: b[at + 5], red: b[at + 6] }
        })
        .collect();
    Some(BmDpiBlock { current: b[0] as usize, stages })
}

/// `bitmouseSetDpiRequest` payload: `[index, x lo, x hi, y lo, y hi, blue, green, red, 0, enable]`.
pub fn bm_set_dpi_payload(index: u8, x: u32, y: u32, blue: u8, green: u8, red: u8, enable: bool) -> [u8; 10] {
    [index, x as u8, (x >> 8) as u8, y as u8, (y >> 8) as u8, blue, green, red, 0, enable as u8]
}

// ---------------------------------------------------------------- bitmouse exchanges

/// One request -> its decoded answer (the answer carrying the same command id; an error status is an error).
fn bm_ask(port: &mut dyn Port, target: u8, cmd: u8, lens: (u8, u8), payload: &[u8]) -> Result<BmReply> {
    let (o, i) = (port.info().output_len as usize, port.info().input_len as usize);
    let how = HidTransfer::OutputThenInput { reply_len: i.max(BM_MIN_IN), max_reads: 16, timeout_ms: BM_TIMEOUT_MS, echo: Some((5, cmd)) };
    let a = port.exchange(&bm_wire(&bm_frame(cmd, lens, target, payload), o), &how)?;
    let r = bm_decode_reply(&a).ok_or_else(|| Error::BadAnswer("not a BITMOUSE answer".into()))?;
    if r.cmd != cmd {
        return Err(Error::BadAnswer(format!("answer is for command {}, not {cmd}", r.cmd)));
    }
    if r.is_error {
        return Err(Error::BadAnswer(format!("the mouse rejected command {cmd}")));
    }
    Ok(r)
}

/// A write the mouse does not answer: the report, then OpenMouse's 120 ms settle.
fn bm_fire(port: &mut dyn Port, target: u8, cmd: u8, lens: (u8, u8), payload: &[u8]) -> Result<()> {
    let o = port.info().output_len as usize;
    port.exchange(&bm_wire(&bm_frame(cmd, lens, target, payload), o), &HidTransfer::OutputOnly)?;
    port.pause_ms(BM_SETTLE_MS);
    Ok(())
}

fn bm_config(port: &mut dyn Port, target: u8) -> Result<BmConfig> {
    let r = bm_ask(port, target, BM_CMD_CONFIG, BM_LEN_CONFIG, &[])?;
    bm_decode_config(&r.payload).ok_or_else(|| Error::BadAnswer("the config block is too short".into()))
}

fn bm_byte(port: &mut dyn Port, target: u8, cmd: u8, lens: (u8, u8)) -> Result<Option<u8>> {
    Ok(soft(bm_ask(port, target, cmd, lens, &[]))?.and_then(|r| r.payload.first().copied()))
}

/// Seven 10-byte reads assemble the table; a gap makes it unusable (`None`).
fn bm_dpi_block(port: &mut dyn Port, target: u8) -> Result<Option<BmDpiBlock>> {
    let mut bytes = Vec::new();
    for addr in bm_dpi_addresses() {
        let Some(r) = soft(bm_ask(port, target, BM_CMD_ADDRESS_DATA, BM_LEN_ADDRESS_DATA, &bm_address_payload(addr, BM_DPI_CHUNK)))? else {
            return Ok(None);
        };
        let chunk = r.payload.get(BM_ADDR_DATA_OFFSET..BM_ADDR_DATA_OFFSET + BM_DPI_CHUNK as usize);
        match chunk {
            Some(c) => bytes.extend_from_slice(c),
            None => return Ok(None),
        }
    }
    Ok(bm_decode_dpi_block(&bytes))
}

fn bm_target(kind: Kind) -> u8 {
    if kind.receiver() {
        BM_TARGET_BEHIND_RECEIVER
    } else {
        BM_TARGET_DEVICE
    }
}

fn bm_read(kind: Kind, m: &Matched, port: &mut dyn Port, opt: ReadOptions) -> Result<OnMouse> {
    let target = bm_target(kind);
    let mut r = OnMouse { model: Some(kind.mouse_name().to_string()), wireless: m.wireless, caps: kind.caps((0, 0)), ..Default::default() };
    let mut config = None;
    for _ in 0..opt.online_tries.clamp(1, TRIES as u32) {
        if let Some(c) = soft(bm_config(port, target))? {
            config = Some(c);
            break;
        }
    }
    let Some(config) = config else {
        // a sleeping mouse behind the receiver answers nothing
        return Ok(r);
    };
    r.online = true;
    r.polling_hz = config.polling_hz;
    r.lift_off = bm_lift_off_tenths(config.offset_calibration);
    r.battery_percent = bm_byte(port, target, BM_CMD_BATTERY, BM_LEN_BATTERY)?;
    r.charging = bm_byte(port, target, BM_CMD_CHARGING, BM_LEN_CHARGING)?.map(|b| b == 1);
    r.dpi = bm_dpi_block(port, target)?.and_then(|b| b.stages.get(b.current).map(|s| s.x));
    Ok(r)
}

fn bm_write(kind: Kind, port: &mut dyn Port, s: Setting) -> Result<Changed> {
    let target = bm_target(kind);
    match s {
        Setting::Polling(hz) => {
            let code = bm_polling_code(hz).ok_or_else(|| Error::range("polling rate", format!("{hz} Hz is not a polling rate of the ATK ZERO")))?;
            let before = soft(bm_config(port, target))?.and_then(|c| c.polling_hz);
            bm_fire(port, target, BM_CMD_SET_REPORT_RATE, BM_LEN_SET_REPORT_RATE, &[code])?;
            let now = bm_config(port, target)?.polling_hz;
            if now != Some(hz) {
                return Err(Error::BadAnswer(format!("the mouse kept {now:?} Hz instead of {hz} Hz")));
            }
            Ok(Changed { now: hz, before })
        }
        Setting::Dpi(dpi) => {
            if !bm_dpi_caps().allows(dpi) {
                return Err(Error::range("DPI", format!("{dpi} is not a DPI step of the ATK ZERO")));
            }
            let block = bm_dpi_block(port, target)?.ok_or_else(|| Error::BadAnswer("the mouse did not report its DPI stages".into()))?;
            let index = block.current;
            let stage = *block.stages.get(index).ok_or_else(|| Error::BadAnswer("the mouse did not report its DPI stages".into()))?;
            // the colour rides along; `enable` moves the mouse onto this stage, which is right for the active one
            let payload = bm_set_dpi_payload(index as u8, dpi, dpi, stage.blue, stage.green, stage.red, true);
            bm_fire(port, target, BM_CMD_SET_DPI, BM_LEN_SET_DPI, &payload)?;
            let after = bm_dpi_block(port, target)?;
            let now = after.and_then(|b| b.stages.get(index).map(|s| s.x));
            if now != Some(dpi) {
                return Err(Error::BadAnswer(format!("the mouse kept {now:?} instead of {dpi} DPI")));
            }
            Ok(Changed { now: dpi, before: Some(stage.x) })
        }
        Setting::LiftOff(t) => {
            let level = t.checked_sub(6).filter(|l| (BM_LIFT_MIN..=BM_LIFT_MAX).contains(l)).ok_or_else(|| Error::range("lift-off distance", format!("{t} tenths of a mm (the ATK ZERO runs 0.7-1.7 mm)")))?;
            let before = soft(bm_config(port, target))?.and_then(|c| bm_lift_off_tenths(c.offset_calibration));
            bm_fire(port, target, BM_CMD_SET_SILENT_HEIGHT, BM_LEN_SET_SILENT_HEIGHT, &[0, (level - 1) as u8])?;
            let now = bm_lift_off_tenths(bm_config(port, target)?.offset_calibration);
            if now != Some(t) {
                return Err(Error::BadAnswer(format!("the mouse kept {now:?} instead of {t} tenths of a mm")));
            }
            Ok(Changed { now: t, before })
        }
    }
}

// ================================================================ allow-list and the driver

/// EEPROM addresses a read may name: the system row, lift-off, the DPI stages, the live row.
fn eeprom_read_ok(addr: u16, len: u8) -> bool {
    let end = addr as u32 + len as u32;
    let system = addr == ADDR_SYSTEM && end <= 0x0A;
    let lift_off = addr == ADDR_LIFT_OFF && len <= 2;
    let stages = (ADDR_DPI..=0x2B).contains(&addr) && end <= 0x2C;
    let live = addr == ADDR_LIVE && len <= 4;
    (1..=10).contains(&len) && (system || lift_off || stages || live)
}

fn hid_read_allowed(out: &[u8]) -> bool {
    if out.len() < HID_WIRE_LEN || out[0] != REPORT_ID || out[HID_WIRE_LEN..].iter().any(|b| *b != 0) {
        return false;
    }
    let f = &out[1..HID_WIRE_LEN];
    if f[15] != hid_crc(&f[..15]) {
        return false;
    }
    match f[0] {
        CMD_CIDMID | CMD_BATTERY | CMD_ONLINE => f[1..15].iter().all(|b| *b == 0),
        CMD_READ_EEPROM => f[1] == 0 && f[5..15].iter().all(|b| *b == 0) && eeprom_read_ok(u16::from_be_bytes([f[2], f[3]]), f[4]),
        _ => false,
    }
}

fn bm_read_allowed(out: &[u8]) -> bool {
    if out.len() < BM_WIRE_LEN || out[0] != REPORT_ID || out[BM_WIRE_LEN..].iter().any(|b| *b != 0) {
        return false;
    }
    let f = &out[1..BM_WIRE_LEN];
    if f[0] != bm_checksum(f) || f[1] != BM_CMD_CODE || f[3] != 0 || f[4] > 1 {
        return false;
    }
    let lens = (f[2], f[6]);
    let payload = &f[BM_PAYLOAD_OFFSET..];
    match f[5] {
        BM_CMD_BATTERY => lens == BM_LEN_BATTERY && payload.iter().all(|b| *b == 0),
        BM_CMD_CHARGING => lens == BM_LEN_CHARGING && payload.iter().all(|b| *b == 0),
        BM_CMD_CONFIG => lens == BM_LEN_CONFIG && payload.iter().all(|b| *b == 0),
        BM_CMD_ADDRESS_DATA => {
            lens == BM_LEN_ADDRESS_DATA && payload[3..].iter().all(|b| *b == 0) && payload[2] == BM_DPI_CHUNK && bm_dpi_addresses().contains(&u16::from_le_bytes([payload[0], payload[1]]))
        }
        _ => false,
    }
}

/// The allow-list for the read-only proof on a real mouse: only this family's READ requests (CID/MID, battery, "online", a
/// few EEPROM reads for `hid.js`; config, battery, charging and the DPI-table reads for BITMOUSE). Every write is refused.
pub fn read_request_allowed(out: &[u8]) -> bool {
    hid_read_allowed(out) || bm_read_allowed(out)
}

/// The ATK / VXE driver.
pub struct Atk;

impl Driver for Atk {
    fn read(&self, m: &Matched, port: &mut dyn Port, opt: ReadOptions) -> Result<OnMouse> {
        let kind = Kind::of(m.variant).ok_or_else(|| Error::UnsupportedMouse("ATK / VXE (unknown model line)".into()))?;
        if kind.bitmouse() {
            bm_read(kind, m, port, opt)
        } else {
            hid_read(kind, m, port, opt)
        }
    }
    fn write(&self, m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        let kind = Kind::of(m.variant).ok_or_else(|| Error::UnsupportedMouse("ATK / VXE (unknown model line)".into()))?;
        if kind.bitmouse() {
            bm_write(kind, port, s)
        } else {
            hid_write(kind, port, s)
        }
    }
}

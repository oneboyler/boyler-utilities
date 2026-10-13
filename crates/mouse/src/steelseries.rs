//! SteelSeries - ported from OpenMouse (`crate::family`: AGPL-3.0-or-later, @openmouse/protocol 0.40.1), DPI / polling /
//! lift-off / battery only. SteelSeries is many protocols under one vendor id: each model line has its own command bytes,
//! DPI encoding, polling table and save command. Ported per line (the codec file under `dist/steelseries/`, the client
//! under `dist/drivers/steelseries/`), the line kept in `Matched.variant` ([`Line`], plus [`DONGLE`] for a 2.4 GHz-mode id):
//!
//! | line | codec / client | DPI | polling | lift-off | reads |
//! |---|---|---|---|---|---|
//! | Rival 3 | `rival3.js` / `hid.js` | `0B 00` + TrueMove Core table | `04 00` | - | firmware `10 00` (probe only) |
//! | Aerox 3 | `aerox3.js` / `aerox3-hid.js` | `2D` + Core table | `2B` | - | nothing |
//! | Rival 3 Wireless | `rival3-wireless.js` / `rival3-wireless-hid.js` | `20` + TrueMove Air table, 2 bytes | `17` | - | battery `AA 01` |
//! | Aerox 5 | `aerox5.js` / `aerox5-hid.js` | `2D` + Air table | `2B` | - | nothing |
//! | Aerox 5 Wireless, Aerox 9 Wireless, Prime Mini Wireless | `aerox5-wireless.js`, `aerox9-wireless.js`, `prime-mini-wireless.js` (+ `-hid.js`) | `2D` + Air table | `2B` | - | battery `92` |
//! | Rival 650 Wireless | `rival650.js` / `rival650-hid.js` | `15 01` linear | `17` | - (not sent by OpenMouse's page) | battery `AA 01` |
//! | Rival 310 | `rival310.js` / `rival310-hid.js` | `53 00 01` linear | `54 00` | - | firmware `90 00` (probe only) |
//! | Prime+ | `prime-plus.js` / `prime-plus-hid.js` | `61` linear, 2 bytes | `5D` | - | nothing |
//! | Sensei Ten | `sensei-ten.js` / `sensei-ten-hid.js` | `55 00` linear, 2 bytes | `54 00` | - | firmware `90 00` (probe only) |
//!
//! Transport (every line): an unnumbered HID OUTPUT report (report id 0) on the vendor config interface rivalcfg names
//! (`"endpoint"`: 3 or 0); answers are the next INPUT report. OpenMouse's `sendReport(0x00, payload)` passes the payload
//! without the id and reads `event.data` without it: its byte i is our byte i + 1 (Windows: id 0 in byte 0, zero-padded to
//! OutputReportByteLength). In 2.4 GHz mode the Aerox 5 / 9 Wireless and Prime Mini Wireless OR 0x40 into byte 0 of every
//! command (OpenMouse's applyWirelessFlag). Every setter is the setting, 50 ms, then the line's save command (rivalcfg's
//! default: the settings are committed to the mouse's onboard memory - a settings save, not a firmware flash).
//!
//! **No SteelSeries mouse can report its DPI or polling rate** (rivalcfg has no getter for any of them; OpenMouse shows its
//! own last-written value with "valuesVerified: false"). So `read` returns `None` for DPI / polling / lift-off, `write`
//! returns `before: None` and reads nothing back, and DPI changes REPLACE the mouse's preset table with one preset (OpenMouse's
//! setDpi; the Rival 650 / 310 write preset 1 only). Battery is read only where OpenMouse reads it.
//!
//! Left out on purpose: lighting, buttons, sleep / dim timers, the Rival 650 / 310 second DPI preset, multi-preset tables,
//! the firmware version text (the firmware query is sent only as OpenMouse's "does it answer?" probe). OpenMouse has no
//! firmware-update, bootloader or reset command for SteelSeries; none is here. The Aerox 3 Wireless client OpenMouse has
//! (`aerox3-wireless-hid.js`) is not ported: its model is "pending" on OpenMouse's SUPPORTED list.

use crate::error::{Error, Result};
use crate::family::{Caps, Changed, DpiCaps, Family, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x1038;
/// Every SteelSeries command is an unnumbered report: id 0.
pub const REPORT_ID: u8 = 0x00;
/// rivalcfg's `command_approve_delay` (OpenMouse COMMAND_DELAY_MS) between a setting and its save command.
pub const COMMAND_DELAY_MS: u64 = 50;
/// OpenMouse's RESPONSE_TIMEOUT_MS / FIRMWARE_TIMEOUT_MS for the answer to a query.
pub const ANSWER_TIMEOUT_MS: u32 = 500;
/// `Matched.variant` bit: a 2.4 GHz-mode id whose line ORs [`WIRELESS_FLAG`] into every command.
pub const DONGLE: u16 = 0x0100;
/// `_WIRELESS_FLAG = 0b01000000` (OpenMouse's applyWirelessFlag).
pub const WIRELESS_FLAG: u8 = 0x40;

/// The model lines (OpenMouse's `family` field in STEELSERIES_PRODUCTS) - `Matched.variant & 0xFF`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Line {
    Rival3 = 1,
    Aerox3 = 2,
    Rival3Wireless = 3,
    Aerox5 = 4,
    Aerox5Wireless = 5,
    Rival650 = 6,
    Aerox9Wireless = 7,
    Rival310 = 8,
    PrimePlus = 9,
    PrimeMiniWireless = 10,
    SenseiTen = 11,
}

const LINES: [Line; 11] = [
    Line::Rival3,
    Line::Aerox3,
    Line::Rival3Wireless,
    Line::Aerox5,
    Line::Aerox5Wireless,
    Line::Rival650,
    Line::Aerox9Wireless,
    Line::Rival310,
    Line::PrimePlus,
    Line::PrimeMiniWireless,
    Line::SenseiTen,
];

/// The line of a `Matched.variant`.
pub fn line_of(variant: u16) -> Option<Line> {
    LINES.iter().copied().find(|l| *l as u16 == variant & 0xFF)
}

impl Line {
    /// The USB interface of the config channel (rivalcfg's `"endpoint"`, quoted in each OpenMouse client's doc).
    pub fn interface(self) -> u8 {
        match self {
            Line::Rival650 | Line::Rival310 | Line::PrimePlus | Line::SenseiTen => 0,
            _ => 3,
        }
    }
    /// The lines whose 2.4 GHz-mode ids OR 0x40 into byte 0 (the `_patch_command` pair of rivalcfg files).
    fn flags_dongle(self) -> bool {
        matches!(self, Line::Aerox5Wireless | Line::Aerox9Wireless | Line::PrimeMiniWireless)
    }
    /// Does OpenMouse's DPI write leave the mouse's other DPI presets alone (it sets preset 1 only)? Every other line's
    /// setDpi replaces the whole preset table with one preset.
    pub fn dpi_keeps_presets(self) -> bool {
        matches!(self, Line::Rival650 | Line::Rival310)
    }
}

// ============================================================================ codec

/// rivalcfg's TrueMove Core table (`rival3.js` TRUEMOVE_CORE_DPI_TO_BYTE): 200..=8500 DPI in 100s, one byte each.
const TRUEMOVE_CORE: [u8; 84] = [
    0x04, 0x06, 0x08, 0x0b, 0x0d, 0x0f, 0x12, 0x14, 0x16, 0x19, 0x1b, 0x1d, 0x20, 0x22, 0x24, 0x27, 0x29, 0x2b, 0x2e, 0x30, 0x32, 0x35, 0x37, 0x39,
    0x3c, 0x3e, 0x40, 0x43, 0x45, 0x47, 0x4a, 0x4c, 0x4e, 0x51, 0x53, 0x55, 0x58, 0x5a, 0x5c, 0x5f, 0x61, 0x63, 0x66, 0x68, 0x6a, 0x6d, 0x6f, 0x71,
    0x74, 0x76, 0x78, 0x7b, 0x7d, 0x7f, 0x82, 0x84, 0x86, 0x89, 0x8b, 0x8d, 0x90, 0x92, 0x94, 0x97, 0x99, 0x9b, 0x9e, 0xa0, 0xa2, 0xa5, 0xa7, 0xa9,
    0xac, 0xae, 0xb0, 0xb3, 0xb5, 0xb7, 0xba, 0xbc, 0xbe, 0xc1, 0xc3, 0xc5,
];

/// rivalcfg's TrueMove Air table (`rival3-wireless.js` TRUEMOVE_AIR_DPI_TO_BYTE): 100..=18000 DPI in 100s, one byte each.
const TRUEMOVE_AIR: [u8; 180] = [
    0x00, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x10, 0x11, 0x12, 0x13, 0x14, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b,
    0x1d, 0x1e, 0x1f, 0x20, 0x21, 0x23, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2c, 0x2d, 0x2e, 0x2f, 0x30, 0x32, 0x33, 0x34, 0x35, 0x36, 0x38, 0x39,
    0x3a, 0x3b, 0x3c, 0x3e, 0x3f, 0x40, 0x41, 0x42, 0x44, 0x45, 0x46, 0x47, 0x48, 0x4a, 0x4b, 0x4c, 0x4d, 0x4e, 0x50, 0x51, 0x52, 0x53, 0x54, 0x56,
    0x57, 0x58, 0x59, 0x5a, 0x5c, 0x5d, 0x5e, 0x5f, 0x60, 0x62, 0x63, 0x64, 0x65, 0x66, 0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6e, 0x6f, 0x70, 0x71, 0x72,
    0x74, 0x75, 0x76, 0x77, 0x78, 0x7a, 0x7b, 0x7c, 0x7d, 0x7e, 0x80, 0x81, 0x82, 0x83, 0x84, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8c, 0x8d, 0x8e, 0x8f,
    0x90, 0x92, 0x93, 0x94, 0x95, 0x96, 0x98, 0x99, 0x9a, 0x9b, 0x9c, 0x9e, 0x9f, 0xa0, 0xa1, 0xa2, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xaa, 0xab, 0xac,
    0xad, 0xae, 0xb0, 0xb1, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xbb, 0xbc, 0xbd, 0xbf, 0xc0, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7,
    0xc9, 0xca, 0xcb, 0xcc, 0xcd, 0xcf, 0xd0, 0xd1, 0xd2, 0xd3, 0xd5, 0xd6,
];

/// A DPI → its byte in a 100-step table that starts at `first` (exact table keys only, as OpenMouse: no rounding).
fn table_byte(t: &[u8], first: u32, dpi: u32) -> Option<u8> {
    (dpi >= first && dpi.is_multiple_of(100)).then(|| t.get(((dpi - first) / 100) as usize).copied()).flatten()
}

pub fn truemove_core(dpi: u32) -> Option<u8> {
    table_byte(&TRUEMOVE_CORE, 200, dpi)
}

pub fn truemove_air(dpi: u32) -> Option<u8> {
    table_byte(&TRUEMOVE_AIR, 100, dpi)
}

/// `(dpi − from) / step` when the DPI is on the grid (rivalcfg's linear `range`, OpenMouse's encodeDpiByte / dpiToWord).
fn linear(dpi: u32, from: u32, to: u32, step: u32) -> Option<u32> {
    ((from..=to).contains(&dpi) && (dpi - from).is_multiple_of(step)).then(|| (dpi - from) / step)
}

/// Byte 0 of a command in this link mode (applyWirelessFlag when `dongle` and the line uses it).
fn cmd(line: Line, dongle: bool, c: u8) -> u8 {
    if dongle && line.flags_dongle() {
        c | WIRELESS_FLAG
    } else {
        c
    }
}

/// The DPI request OpenMouse's setDpi sends (its payload, without the report id) - a one-preset table, preset selected
/// (Rival 650 / 310: sensitivity preset 1). `None` = the line can't store that DPI.
pub fn dpi_request(line: Line, dpi: u32, dongle: bool) -> Option<Vec<u8>> {
    Some(match line {
        // 0B 00 | count | selected + 1 | core
        Line::Rival3 => vec![0x0B, 0x00, 0x01, 0x01, truemove_core(dpi)?],
        // 2D | count | selected + 1 (first_preset 1) | core
        Line::Aerox3 => vec![0x2D, 0x01, 0x01, truemove_core(dpi)?],
        // 20 | count | selected + 1 | air, little-endian 2 bytes
        Line::Rival3Wireless => vec![0x20, 0x01, 0x01, truemove_air(dpi)?, 0x00],
        // 2D | count | selected (first_preset 0) | air (6D in 2.4 GHz mode)
        Line::Aerox5 | Line::Aerox5Wireless | Line::Aerox9Wireless | Line::PrimeMiniWireless => vec![cmd(line, dongle, 0x2D), 0x01, 0x00, truemove_air(dpi)?],
        // 15 01 | (dpi − 100) / 100
        Line::Rival650 => vec![0x15, 0x01, linear(dpi, 100, 12000, 100)? as u8],
        // 53 00 01 | (dpi − 100) / 100 | suffix 00 42
        Line::Rival310 => vec![0x53, 0x00, 0x01, linear(dpi, 100, 12000, 100)? as u8, 0x00, 0x42],
        // 61 | count | selected (first_preset 0) | (dpi − 50) / 50 + 1, little-endian 2 bytes
        Line::PrimePlus => {
            let v = linear(dpi, 50, 18000, 50)? + 1;
            vec![0x61, 0x01, 0x00, v as u8, (v >> 8) as u8]
        }
        // 55 00 | count flag (0xFF >> 7 = 1 preset) | selected + 1 | dpi / 50, little-endian 2 bytes
        Line::SenseiTen => {
            let v = linear(dpi, 50, 18000, 50)? + 1;
            vec![0x55, 0x00, 0x01, 0x01, v as u8, (v >> 8) as u8]
        }
    })
}

/// The polling rates every line can be set to (OpenMouse's *_POLLING_RATES).
pub const POLLING_RATES: [u32; 4] = [125, 250, 500, 1000];

/// Hz → code, 1000→1, 500→2, 250→3, 125→4 (most lines).
fn poll_1_4(hz: u32) -> Option<u8> {
    Some(match hz {
        1000 => 1,
        500 => 2,
        250 => 3,
        125 => 4,
        _ => return None,
    })
}

/// Hz → code, 1000→0, 500→1, 250→2, 125→3 (Rival 3 Wireless and the Aerox 5 Wireless command set).
fn poll_0_3(hz: u32) -> Option<u8> {
    poll_1_4(hz).map(|c| c - 1)
}

/// The polling request OpenMouse's setPollingRate sends.
pub fn polling_request(line: Line, hz: u32, dongle: bool) -> Option<Vec<u8>> {
    Some(match line {
        Line::Rival3 => vec![0x04, 0x00, poll_1_4(hz)?],
        Line::Aerox3 | Line::Aerox5 => vec![0x2B, poll_1_4(hz)?],
        Line::Rival3Wireless => vec![0x17, poll_0_3(hz)?],
        Line::Aerox5Wireless | Line::Aerox9Wireless | Line::PrimeMiniWireless => vec![cmd(line, dongle, 0x2B), poll_0_3(hz)?],
        Line::Rival650 => vec![0x17, poll_1_4(hz)?],
        Line::Rival310 | Line::SenseiTen => vec![0x54, 0x00, poll_1_4(hz)?],
        Line::PrimePlus => vec![0x5D, poll_1_4(hz)?],
    })
}

// Lift-off: not ported. OpenMouse's Rival 650 client has setLiftOffDistanceMm, but OpenMouse's own page never sends it (its
// doc: "not currently wired to any UI control") - Order 104 ports only the commands OpenMouse sends.

/// The save command every OpenMouse setter sends after its setting (commits it to the mouse's onboard memory).
pub fn save_request(line: Line, dongle: bool) -> Vec<u8> {
    match line {
        Line::Rival3 => vec![0x09, 0x00],
        Line::Rival3Wireless | Line::Rival650 => vec![0x09],
        Line::Aerox3 | Line::Aerox5 => vec![0x11, 0x00],
        Line::Aerox5Wireless | Line::Aerox9Wireless | Line::PrimeMiniWireless => vec![cmd(line, dongle, 0x11), 0x00],
        Line::Rival310 | Line::SenseiTen => vec![0x59, 0x00],
        Line::PrimePlus => vec![0x59],
    }
}

/// What a line's read query answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    /// a two-byte firmware version - OpenMouse uses it only as "does the interface answer?"
    Firmware,
    /// `AA 01`: three bytes, level / unused / charging
    BatteryAa,
    /// `92`: two bytes, unused / charging bit 7 + (level / 5 + 1)
    Battery92,
}

pub fn probe(line: Line) -> Option<Probe> {
    match line {
        Line::Rival3 | Line::Rival310 | Line::SenseiTen => Some(Probe::Firmware),
        Line::Rival3Wireless | Line::Rival650 => Some(Probe::BatteryAa),
        Line::Aerox5Wireless | Line::Aerox9Wireless | Line::PrimeMiniWireless => Some(Probe::Battery92),
        // aerox3.py, aerox5.py, prime_plus.py define no getter and no firmware query: nothing to ask
        Line::Aerox3 | Line::Aerox5 | Line::PrimePlus => None,
    }
}

/// The one query OpenMouse's readStatus sends (the battery where there is one, else the firmware probe).
pub fn probe_request(line: Line, dongle: bool) -> Option<Vec<u8>> {
    Some(match (line, probe(line)?) {
        (Line::Rival3, _) => vec![0x10, 0x00],
        (_, Probe::Firmware) => vec![0x90, 0x00],
        (_, Probe::BatteryAa) => vec![0xAA, 0x01],
        (_, Probe::Battery92) => vec![cmd(line, dongle, 0x92)],
    })
}

/// `AA 01` answer (OpenMouse's event.data, no report id) → (percent, charging): `data[0]` level, `data[2] != 0` charging.
/// A level above 100 is no reading.
pub fn decode_battery_aa(d: &[u8]) -> Option<(Option<u8>, bool)> {
    (d.len() >= 3).then(|| ((d[0] <= 100).then_some(d[0]), d[2] != 0))
}

/// `92` answer → (percent, charging): status `data[1]`, charging = bit 7, level = ((status & 0x7F) − 1) × 5. A level outside
/// 0-100 (raw 0 would be −5) is no reading.
pub fn decode_battery_92(d: &[u8]) -> Option<(Option<u8>, bool)> {
    let s = *d.get(1)?;
    let level = ((s & 0x7F) as i32 - 1) * 5;
    Some(((0..=100).contains(&level).then_some(level as u8), s & 0x80 != 0))
}

/// A payload as Windows sends it: the report id 0, the payload, zeros up to the collection's output length. `None` = it
/// does not fit.
pub fn report(payload: &[u8], len: u16) -> Option<Vec<u8>> {
    let len = len as usize;
    if payload.len() + 1 > len {
        return None;
    }
    let mut v = vec![0u8; len];
    v[0] = REPORT_ID;
    v[1..1 + payload.len()].copy_from_slice(payload);
    Some(v)
}

/// What the Mouse page offers per line: OpenMouse's getDpiOptions and supportedPollingRates.
pub fn caps(line: Line) -> Caps {
    let dpi = match line {
        Line::Rival3 | Line::Aerox3 => DpiCaps::uniform(200, 8500, 100),
        Line::Rival3Wireless | Line::Aerox5 | Line::Aerox5Wireless | Line::Aerox9Wireless | Line::PrimeMiniWireless => DpiCaps::uniform(100, 18000, 100),
        Line::Rival650 | Line::Rival310 => DpiCaps::uniform(100, 12000, 100),
        Line::PrimePlus | Line::SenseiTen => DpiCaps::uniform(50, 18000, 50),
    };
    // Order 104 review: DPI only where OpenMouse's setDpi changes one preset (Rival 650 / 310); on every other line it writes
    // a ONE-preset table and saves it - the mouse's other DPI presets would be erased for good (the mice can't report them,
    // so not even undo could bring them back). That is a profile wipe, which this app never does: no DPI row there.
    Caps { dpi: line.dpi_keeps_presets().then_some(dpi), polling: POLLING_RATES.to_vec(), lift_off: Vec::new(), battery: matches!(probe(line), Some(Probe::BatteryAa | Probe::Battery92)) }
}

/// The read-only proof's allow-list: a report id 0 carrying exactly one of the SteelSeries READ queries OpenMouse sends -
/// `10 00` (Rival 3 firmware), `90 00` (Rival 3 Wireless / Rival 310 / Sensei Ten firmware), `AA 01` (Rival 3 Wireless /
/// Rival 650 battery), `92` / `D2` (Aerox 5 / 9 Wireless, Prime Mini Wireless battery, cable / dongle) - and only zeros after
/// it. No line uses any of these bytes for a write; every setting, save and lighting command is refused.
pub fn read_request_allowed(out: &[u8]) -> bool {
    let Some((&id, p)) = out.split_first() else { return false };
    if id != REPORT_ID {
        return false;
    }
    let n = match p {
        [0x10, 0x00, ..] | [0x90, 0x00, ..] | [0xAA, 0x01, ..] => 2,
        [0x92, ..] | [0xD2, ..] => 1,
        _ => return false,
    };
    p[n..].iter().all(|b| *b == 0)
}

// ============================================================================ device table

/// One SteelSeries product id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Product {
    pub pid: u16,
    pub line: Line,
    /// the model's name on OpenMouse's SUPPORTED list
    pub model: &'static str,
    /// the id is the 2.4 GHz link (`false` = the cable)
    pub wireless: bool,
    /// a 2.4 GHz-mode id of a line that ORs 0x40 into its commands
    pub dongle: bool,
}

impl Product {
    pub fn variant(&self) -> u16 {
        self.line as u16 | if self.dongle { DONGLE } else { 0 }
    }
}

const fn p(pid: u16, line: Line, model: &'static str, wireless: bool, dongle: bool) -> Product {
    Product { pid, line, model, wireless, dongle }
}

/// OpenMouse's SUPPORTED list (supported-devices-data.ts, status "supported", the pids it lists), each id's line from
/// devices.js STEELSERIES_PRODUCTS. "Aerox 5" there lists the plain Aerox 5 (0x1850) AND the Aerox 5 Wireless ids
/// (devices.js family "aerox5-wireless"). Left out: the Aerox 3 Wireless ids (0x183A / 0x187A / 0x1838 / 0x1878 - its
/// SUPPORTED entry is "pending"), and every SteelSeries id OpenMouse does not list (keyboards and headsets share the vendor id).
pub const PRODUCTS: [Product; 25] = [
    p(0x185A, Line::Aerox9Wireless, "SteelSeries Aerox 9 Wireless", false, false),
    p(0x1876, Line::Aerox9Wireless, "SteelSeries Aerox 9 Wireless", false, false),
    p(0x1858, Line::Aerox9Wireless, "SteelSeries Aerox 9 Wireless", true, true),
    p(0x1874, Line::Aerox9Wireless, "SteelSeries Aerox 9 Wireless", true, true),
    p(0x1832, Line::SenseiTen, "SteelSeries Sensei Ten", false, false),
    p(0x1834, Line::SenseiTen, "SteelSeries Sensei Ten", false, false),
    p(0x1850, Line::Aerox5, "SteelSeries Aerox 5", false, false),
    p(0x1854, Line::Aerox5Wireless, "SteelSeries Aerox 5", false, false),
    p(0x185E, Line::Aerox5Wireless, "SteelSeries Aerox 5", false, false),
    p(0x1862, Line::Aerox5Wireless, "SteelSeries Aerox 5", false, false),
    p(0x1852, Line::Aerox5Wireless, "SteelSeries Aerox 5", true, true),
    p(0x185C, Line::Aerox5Wireless, "SteelSeries Aerox 5", true, true),
    p(0x1860, Line::Aerox5Wireless, "SteelSeries Aerox 5", true, true),
    // rival3_wireless.py: "2.4 GHz mode"; same command bytes in every mode (no flag)
    p(0x1830, Line::Rival3Wireless, "SteelSeries Rival 3 Wireless", true, false),
    p(0x1824, Line::Rival3, "SteelSeries Rival 3", false, false),
    p(0x184C, Line::Rival3, "SteelSeries Rival 3", false, false),
    // rival650.py: the same bytes on the cable and over 2.4 GHz
    p(0x172B, Line::Rival650, "SteelSeries Rival 650 Wireless", false, false),
    p(0x1726, Line::Rival650, "SteelSeries Rival 650 Wireless", true, false),
    p(0x1836, Line::Aerox3, "SteelSeries Aerox 3", false, false),
    p(0x182C, Line::PrimePlus, "SteelSeries Prime+", false, false),
    p(0x1720, Line::Rival310, "SteelSeries Rival 310", false, false),
    p(0x171E, Line::Rival310, "SteelSeries Rival 310", false, false),
    p(0x1736, Line::Rival310, "SteelSeries Rival 310", false, false),
    p(0x184A, Line::PrimeMiniWireless, "SteelSeries Prime Mini Wireless", false, false),
    p(0x1848, Line::PrimeMiniWireless, "SteelSeries Prime Mini Wireless", true, true),
];

/// The config collection: on the line's interface (or a device without interface numbers), a vendor usage page (0xFFxx)
/// with an output report - and an input report when the line reads something; the 0xFFC0 one first (the Aerox 3 Wireless
/// capture in OpenMouse: "the only collection with a 64-byte output report, the others refuse writes"). OpenMouse itself
/// asks for the whole device by id and lets the wrong collections fail its probe; Windows needs the one path.
pub fn config_interface<'a>(own: &[&'a HidInfo], line: Line) -> Option<&'a HidInfo> {
    let reads = probe(line).is_some();
    let fits = |h: &&&HidInfo| {
        h.usage_page >= 0xFF00 && h.output_len > 1 && (!reads || h.input_len > 1) && h.interface.is_none_or(|i| i == line.interface())
    };
    own.iter().filter(fits).find(|h| h.usage_page == 0xFFC0).or_else(|| own.iter().find(fits)).copied()
}

/// OpenMouse's SteelSeries isSupported: the vendor id AND one of the listed product ids (never a vendor-only match), then
/// the config collection.
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID {
        return None;
    }
    let p = PRODUCTS.iter().find(|p| p.pid == pid)?;
    let config = config_interface(own, p.line)?;
    Some(Matched { family: Family::SteelSeries, model: Some(p.model), wireless: Some(p.wireless), config: config.clone(), variant: p.variant() })
}

// ============================================================================ driver

/// The SteelSeries driver (every line; `Matched.variant` says which).
pub struct SteelSeries;

fn line(m: &Matched) -> Result<(Line, bool)> {
    let l = line_of(m.variant).ok_or_else(|| Error::UnsupportedMouse(format!("SteelSeries variant {:#06x}", m.variant)))?;
    Ok((l, m.variant & DONGLE != 0))
}

/// One command, no answer (OpenMouse's write = sendReport).
fn send(port: &mut dyn Port, payload: &[u8]) -> Result<()> {
    let out = report(payload, port.info().output_len).ok_or_else(|| Error::UnsupportedMouse(format!("the config report is too short for {payload:02x?}")))?;
    port.exchange(&out, &HidTransfer::OutputOnly).map(|_| ())
}

/// One query and the first input report after it (OpenMouse's awaitResponse: any report, 500 ms), without the report id.
/// `None` = no answer in time (or one too short to decode).
fn ask(port: &mut dyn Port, payload: &[u8], need: usize) -> Result<Option<Vec<u8>>> {
    let out = report(payload, port.info().output_len).ok_or_else(|| Error::UnsupportedMouse(format!("the config report is too short for {payload:02x?}")))?;
    let how = HidTransfer::OutputThenInput { reply_len: port.info().input_len as usize, max_reads: 1, timeout_ms: ANSWER_TIMEOUT_MS, echo: None };
    match port.exchange(&out, &how) {
        Ok(a) if a.len() > need => Ok(Some(a[1..].to_vec())),
        Ok(_) | Err(Error::BadAnswer(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

impl crate::family::Driver for SteelSeries {
    /// OpenMouse's readStatus: the one query of the line (battery, or the firmware probe), nothing at all on the lines that
    /// have none. DPI / polling / lift-off can't be read on any SteelSeries (`None`: no lit chip).
    fn read(&self, m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let (l, dongle) = line(m)?;
        // no query = OpenMouse's "a successful open is the only signal": the mouse is listed, so it is there
        let mut r = OnMouse { model: None, online: true, wireless: m.wireless, caps: caps(l), ..Default::default() };
        let (Some(kind), Some(q)) = (probe(l), probe_request(l, dongle)) else { return Ok(r) };
        let need = match kind {
            Probe::Firmware | Probe::Battery92 => 2,
            Probe::BatteryAa => 3,
        };
        // OpenMouse asks once (no answer = it throws "did not answer"); here no answer = offline, nothing invented
        let Some(a) = ask(port, &q, need)? else {
            r.online = false;
            return Ok(r);
        };
        let battery = match kind {
            Probe::Firmware => None,
            Probe::BatteryAa => decode_battery_aa(&a),
            Probe::Battery92 => decode_battery_92(&a),
        };
        if let Some((pct, charging)) = battery {
            r.battery_percent = pct;
            r.charging = Some(charging);
        }
        Ok(r)
    }

    /// OpenMouse's setDpi / setPollingRate: the setting, 50 ms, the save command. Nothing is read
    /// before or after (no SteelSeries can report these), so `before` is `None`.
    fn write(&self, m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        let (l, dongle) = line(m)?;
        let req = match s {
            Setting::Dpi(_) if !l.dpi_keeps_presets() => {
                return Err(Error::range("DPI", "on this SteelSeries mouse a DPI change would erase its saved DPI presets - set it in SteelSeries GG".to_string()))
            }
            Setting::Dpi(d) => dpi_request(l, d, dongle).ok_or_else(|| Error::range("DPI", format!("{d} DPI can't be stored on this SteelSeries mouse")))?,
            Setting::Polling(hz) => polling_request(l, hz, dongle).ok_or_else(|| Error::range("polling rate", format!("{hz} Hz is not a polling rate of this mouse")))?,
            Setting::LiftOff(t) => return Err(Error::range("lift-off distance", format!("{t} tenths of a mm: lift-off can't be set on a SteelSeries mouse here"))),
        };
        send(port, &req)?;
        port.pause_ms(COMMAND_DELAY_MS);
        send(port, &save_request(l, dongle))?;
        Ok(Changed { now: s.value(), before: None })
    }
}

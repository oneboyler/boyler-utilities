//! Delux M800 Mini - ported from OpenMouse (`crate::family`: AGPL-3.0-or-later, @openmouse/protocol 0.40.1), DPI and polling
//! only:
//!
//! - the client: `dist/drivers/delux/hid.js` (DeluxHidClient: setPollingRate, setDpi, readDpiState, writeDpiTable);
//! - the catalogue: `dist/delux/index.js` (ids, names, the polling list);
//! - the DPI report: `dist/compx/x11-dpi.js` (the "X11" firmware's 52 / 56-byte DPI feature report: byte map, mask / high
//!   flags, checksum, and its decoder). The same firmware is the Attack Shark X11's: the ids 0x1d57:0xfa60 / 0xfa55 are
//!   shared, and OpenMouse tells the Delux apart by its product string ("Delux ..."), so does the matcher here.
//!
//! Transport: HID FEATURE reports on the config sub-collection of USB interface 2 (usage page 0x0B, the only collection of
//! the device with a feature report). Report 0x06 sets the polling rate: `[06, 09, 01, rate, 0xFF - rate, 0, 0, 0, 0]`
//! (rate byte 125 = 08, 250 = 04, 500 = 02, 1000 = 01). Report 0x04 is the DPI table: `[04, 38, 01, angle snap, ripple, 3F,
//! mask, mask, six DPI bytes, 0, 0, six "high" flags, 0, 0, active stage, ...fixed bytes..., checksum (2 bytes, big endian,
//! sum of bytes 3..49)]` - 56 bytes over the receiver (0xfa60), 52 over the cable (0xfa55). Windows wants the collection's
//! whole feature-report length: the report is zero-padded behind these bytes (unclear whether this firmware takes that;
//! the first attempt on real hardware tells - a refusal comes back as an error).
//!
//! What OpenMouse does NOT do (and so neither can this file): read the polling rate (it keeps its own last-written value),
//! and read the battery by asking - the battery arrives only as an unsolicited INPUT report (`03 55 40 01 <percent>`) that
//! OpenMouse listens for. NEEDS TRANSFER: an input-only wait (no write first) that returns the next input report whose first
//! bytes are given, within a timeout. Until then the battery row is off.
//!
//! DPI is OpenMouse's setDpi: it rewrites the WHOLE DPI table from its cached state (six stages, active stage, angle
//! snapping, ripple control) with fixed constants for every byte it does not model. Only the receiver (0xfa60) is read
//! first (GetFeature 0x04, decoded); on the cable OpenMouse never reads the table and would overwrite it with the factory
//! defaults, so the cable's DPI is HELD BACK. On the receiver this file adds one safety check OpenMouse does not have: the
//! report it read must be byte for byte what OpenMouse would write for the same state, zero padding to the whole feature-report
//! length included (every unmodelled byte already the
//! constant OpenMouse writes); only then is the table written, with just the active stage changed, and it is read back to
//! confirm. A mouse whose table differs elsewhere is left alone with an explanation.
//!
//! Order 110 (A_110_02 scope): angle snapping and ripple control (setAngleSnapping / setRippleControl: the same table write,
//! bytes 3 / 4) - receiver only, with the same safety check and read-back as the DPI change; on the cable they are held back
//! like its DPI (OpenMouse would write the table from factory defaults there).
//!
//! Left out: the other DPI stages and the active stage (setActiveDpiStage: setters of the
//! same table), lighting, buttons. OpenMouse sends no firmware-update, reset or pairing command to a Delux mouse and neither
//! does this file. The Delux M600 Pro (0xfa71 wired, and a receiver that shares 0xfa60 but names itself only by a model id in
//! its messages) is in the driver but NOT on OpenMouse's supported list: not in the table.

use crate::error::{Error, Result};
use crate::family::{check_extra, Caps, Change, Changed, DpiCaps, Extra, ExtraRow, Family, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x1d57;
/// The M800 Mini on its 2.4 GHz receiver (shared with the Attack Shark X11 and other mice: the product string decides).
pub const PID_WIRELESS: u16 = 0xfa60;
/// The M800 Mini on its cable.
pub const PID_WIRED: u16 = 0xfa55;
pub const MODEL: &str = "Delux M800 Mini";
/// The config sub-collection's usage page.
pub const CONFIG_USAGE_PAGE: u16 = 0x0b;
pub const DPI_REPORT_ID: u8 = 0x04;
pub const POLLING_REPORT_ID: u8 = 0x06;
/// The DPI report's length over the receiver / the cable.
pub const DPI_REPORT_LEN_WIRELESS: usize = 56;
pub const DPI_REPORT_LEN_WIRED: usize = 52;
/// OpenMouse's CMD_DELAY_MS after a feature report.
pub const CMD_DELAY_MS: u64 = 300;
pub const STAGES: usize = 6;
pub const DPI_MIN: u32 = 50;
pub const DPI_MAX: u32 = 22000;
pub const POLLING_RATES: [u32; 4] = [125, 250, 500, 1000];

/// X11_DPI_STEP_MAP: DPI → hardware byte (50-step to 10000, 100-step above; above 10000 the bytes repeat across register
/// pages, the mask / high flags disambiguate).
const STEP_MAP: [(u32, u8); 320] = [
    (50,0x01), (100,0x02), (150,0x03), (200,0x04), (250,0x05), (300,0x06), (350,0x08), (400,0x09),
    (450,0x0a), (500,0x0b), (550,0x0c), (600,0x0e), (650,0x0f), (700,0x10), (750,0x11), (800,0x12),
    (850,0x13), (900,0x15), (950,0x16), (1000,0x17), (1050,0x18), (1100,0x19), (1150,0x1b), (1200,0x1c),
    (1250,0x1d), (1300,0x1e), (1350,0x1f), (1400,0x20), (1450,0x22), (1500,0x23), (1550,0x24), (1600,0x25),
    (1650,0x26), (1700,0x27), (1750,0x29), (1800,0x2a), (1850,0x2b), (1900,0x2c), (1950,0x2d), (2000,0x2f),
    (2050,0x30), (2100,0x31), (2150,0x32), (2200,0x33), (2250,0x34), (2300,0x36), (2350,0x37), (2400,0x38),
    (2450,0x39), (2500,0x3a), (2550,0x3b), (2600,0x3d), (2650,0x3e), (2700,0x3f), (2750,0x40), (2800,0x41),
    (2850,0x43), (2900,0x44), (2950,0x45), (3000,0x46), (3050,0x47), (3100,0x48), (3150,0x4a), (3200,0x4b),
    (3250,0x4c), (3300,0x4d), (3350,0x4e), (3400,0x4f), (3450,0x51), (3500,0x52), (3550,0x53), (3600,0x54),
    (3650,0x55), (3700,0x57), (3750,0x58), (3800,0x59), (3850,0x5a), (3900,0x5b), (3950,0x5c), (4000,0x5e),
    (4050,0x5f), (4100,0x60), (4150,0x61), (4200,0x62), (4250,0x63), (4300,0x65), (4350,0x66), (4400,0x67),
    (4450,0x68), (4500,0x69), (4550,0x6b), (4600,0x6c), (4650,0x6d), (4700,0x6e), (4750,0x6f), (4800,0x70),
    (4850,0x72), (4900,0x73), (4950,0x74), (5000,0x75), (5050,0x76), (5100,0x77), (5150,0x79), (5200,0x7a),
    (5250,0x7b), (5300,0x7c), (5350,0x7d), (5400,0x7f), (5450,0x80), (5500,0x81), (5550,0x82), (5600,0x83),
    (5650,0x84), (5700,0x86), (5750,0x87), (5800,0x88), (5850,0x89), (5900,0x8a), (5950,0x8b), (6000,0x8d),
    (6050,0x8e), (6100,0x8f), (6150,0x90), (6200,0x91), (6250,0x93), (6300,0x94), (6350,0x95), (6400,0x96),
    (6450,0x97), (6500,0x98), (6550,0x9a), (6600,0x9b), (6650,0x9c), (6700,0x9d), (6750,0x9e), (6800,0x9f),
    (6850,0xa1), (6900,0xa2), (6950,0xa3), (7000,0xa4), (7050,0xa5), (7100,0xa7), (7150,0xa8), (7200,0xa9),
    (7250,0xaa), (7300,0xab), (7350,0xac), (7400,0xae), (7450,0xaf), (7500,0xb0), (7550,0xb1), (7600,0xb2),
    (7650,0xb3), (7700,0xb5), (7750,0xb6), (7800,0xb7), (7850,0xb8), (7900,0xb9), (7950,0xbb), (8000,0xbc),
    (8050,0xbd), (8100,0xbe), (8150,0xbf), (8200,0xc0), (8250,0xc2), (8300,0xc3), (8350,0xc4), (8400,0xc5),
    (8450,0xc6), (8500,0xc7), (8550,0xc9), (8600,0xca), (8650,0xcb), (8700,0xcc), (8750,0xcd), (8800,0xcf),
    (8850,0xd0), (8900,0xd1), (8950,0xd2), (9000,0xd3), (9050,0xd4), (9100,0xd6), (9150,0xd7), (9200,0xd8),
    (9250,0xd9), (9300,0xda), (9350,0xdb), (9400,0xdd), (9450,0xde), (9500,0xdf), (9550,0xe0), (9600,0xe1),
    (9650,0xe3), (9700,0xe4), (9750,0xe5), (9800,0xe6), (9850,0xe7), (9900,0xe8), (9950,0xea), (10000,0xeb),
    (10100,0x76), (10200,0x77), (10300,0x79), (10400,0x7a), (10500,0x7b), (10600,0x7c), (10700,0x7d), (10800,0x7f),
    (10900,0x80), (11000,0x81), (11100,0x82), (11200,0x83), (11300,0x84), (11400,0x86), (11500,0x87), (11600,0x88),
    (11700,0x89), (11800,0x8a), (11900,0x8b), (12000,0x8d), (12100,0x8e), (12200,0x8f), (12300,0x90), (12400,0x91),
    (12500,0x93), (12600,0x94), (12700,0x95), (12800,0x96), (12900,0x97), (13000,0x98), (13100,0x9a), (13200,0x9b),
    (13300,0x9c), (13400,0x9d), (13500,0x9e), (13600,0x9f), (13700,0xa1), (13800,0xa2), (13900,0xa3), (14000,0xa4),
    (14100,0xa5), (14200,0xa7), (14300,0xa8), (14400,0xa9), (14500,0xaa), (14600,0xab), (14700,0xac), (14800,0xae),
    (14900,0xaf), (15000,0xb0), (15100,0xb1), (15200,0xb2), (15300,0xb3), (15400,0xb5), (15500,0xb6), (15600,0xb7),
    (15700,0xb8), (15800,0xb9), (15900,0xbb), (16000,0xbc), (16100,0xbd), (16200,0xbe), (16300,0xbf), (16400,0xc0),
    (16500,0xc2), (16600,0xc3), (16700,0xc4), (16800,0xc5), (16900,0xc6), (17000,0xc7), (17100,0xc9), (17200,0xca),
    (17300,0xcb), (17400,0xcc), (17500,0xcd), (17600,0xcf), (17700,0xd0), (17800,0xd1), (17900,0xd2), (18000,0xd3),
    (18100,0xd4), (18200,0xd6), (18300,0xd7), (18400,0xd8), (18500,0xd9), (18600,0xda), (18700,0xdb), (18800,0xdd),
    (18900,0xde), (19000,0xdf), (19100,0xe0), (19200,0xe1), (19300,0xe3), (19400,0xe4), (19500,0xe5), (19600,0xe6),
    (19700,0xe7), (19800,0xe8), (19900,0xea), (20000,0xeb), (20100,0xeb), (20200,0x76), (20300,0x76), (20400,0x77),
    (20500,0x77), (20600,0x79), (20700,0x79), (20800,0x7a), (20900,0x7a), (21000,0x7b), (21100,0x7b), (21200,0x7c),
    (21300,0x7c), (21400,0x7d), (21500,0x7d), (21600,0x7f), (21700,0x7f), (21800,0x80), (21900,0x80), (22000,0x81),
];

// ============================================================================ codec

/// `nearestX11Dpi`: the supported value at or above `dpi`, clamped to 50..22000.
pub fn nearest_up(dpi: u32) -> u32 {
    let target = dpi.clamp(DPI_MIN, DPI_MAX);
    STEP_MAP.iter().find(|(v, _)| *v >= target).map(|(v, _)| *v).unwrap_or(DPI_MAX)
}

/// `encodeX11DpiByte`.
pub fn dpi_byte(dpi: u32) -> u8 {
    let v = nearest_up(dpi);
    STEP_MAP.iter().find(|(d, _)| *d == v).map(|(_, b)| *b).expect("nearest_up gives a table value")
}

/// What the DPI report holds (the part OpenMouse models).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DpiState {
    pub stages: [u32; STAGES],
    /// 1-based
    pub active: u8,
    pub angle_snap: bool,
    pub ripple: bool,
}

fn checksum(buf: &[u8]) -> u16 {
    buf[3..=49].iter().fold(0u16, |s, b| s.wrapping_add(*b as u16))
}

/// `buildX11DpiReport`: the report including its leading id byte, 56 bytes over the receiver, 52 over the cable.
pub fn build_dpi_report(s: &DpiState, wired: bool) -> Vec<u8> {
    let mut b = vec![0u8; DPI_REPORT_LEN_WIRELESS];
    b[0] = DPI_REPORT_ID;
    b[1] = 0x38;
    b[2] = 0x01;
    b[3] = s.angle_snap as u8;
    b[4] = s.ripple as u8;
    b[5] = 0x3f;
    let mut mask = 0u8;
    for (i, dpi) in s.stages.iter().enumerate() {
        b[8 + i] = dpi_byte(*dpi);
        if *dpi > 12000 {
            mask |= 1 << i;
        }
        let high = (10100..=12000).contains(dpi) || (20100..=22000).contains(dpi);
        b[16 + i] = high as u8;
    }
    b[6] = mask;
    b[7] = mask;
    // 14, 15, 22, 23 stay zero
    b[24] = s.active;
    for i in [25, 29, 33, 34, 35, 38, 39, 40, 42, 43, 46, 47, 48] {
        b[i] = 0xff;
    }
    b[44] = 0x40;
    b[49] = 0x02;
    let sum = checksum(&b);
    b[50] = (sum >> 8) as u8;
    b[51] = (sum & 0xff) as u8;
    if wired {
        b.truncate(DPI_REPORT_LEN_WIRED);
    }
    b
}

/// The first table entry with this byte inside a DPI range (`reverseRange`: ascending, the lower value is kept).
fn reverse(min: u32, max: u32, byte: u8) -> Option<u32> {
    STEP_MAP.iter().find(|(d, b)| *b == byte && (min..=max).contains(d)).map(|(d, _)| *d)
}

/// `decodeX11DpiReport`: the report as the mouse returns it (id first, or with the id cut off). `None` = wrong signature or
/// checksum, or a byte off the map.
pub fn decode_dpi_report(data: &[u8]) -> Option<DpiState> {
    let owned;
    let buf: &[u8] = if data.len() >= 2 && data[0] == 0x38 && data[1] == 0x01 {
        let mut full = vec![DPI_REPORT_ID];
        full.extend_from_slice(data);
        owned = full;
        &owned
    } else {
        data
    };
    if buf.len() < DPI_REPORT_LEN_WIRED || buf[0] != DPI_REPORT_ID || buf[1] != 0x38 || buf[2] != 0x01 {
        return None;
    }
    if checksum(buf) != (((buf[50] as u16) << 8) | buf[51] as u16) {
        return None;
    }
    let mut stages = [0u32; STAGES];
    for (i, st) in stages.iter_mut().enumerate() {
        let byte = buf[8 + i];
        let mask = (buf[6] >> i) & 1 == 1;
        let high = buf[16 + i] == 1;
        let (lo, hi) = match (mask, high) {
            (true, true) => (20100, 22000),
            (true, false) => (12100, 20000),
            (false, true) => (10100, 12000),
            (false, false) => (50, 10000),
        };
        *st = reverse(lo, hi, byte).or_else(|| reverse(50, 10000, byte))?;
    }
    Some(DpiState { stages, active: buf[24].clamp(1, 6), angle_snap: buf[3] == 1, ripple: buf[4] == 1 })
}

/// The polling request OpenMouse sends (report 0x06, without padding): `[06, 09, 01, rate, 0xFF - rate, 0, 0, 0, 0]`.
pub fn polling_request(hz: u32) -> Option<Vec<u8>> {
    let rate: u8 = match hz {
        125 => 0x08,
        250 => 0x04,
        500 => 0x02,
        1000 => 0x01,
        _ => return None,
    };
    Some(vec![POLLING_REPORT_ID, 0x09, 0x01, rate, 0xff - rate, 0, 0, 0, 0])
}

/// A report as Windows sends it: zero-padded to the collection's feature-report length (`None` = it does not fit).
pub fn padded(report: &[u8], len: usize) -> Option<Vec<u8>> {
    (report.len() <= len).then(|| {
        let mut v = report.to_vec();
        v.resize(len, 0);
        v
    })
}

/// The request that reads the DPI table: a GetFeature of report 0x04 (nothing is sent; this is the allow-list's view of it:
/// the report id and zeros).
pub fn read_dpi_request(len: usize) -> Vec<u8> {
    let mut v = vec![0u8; len.max(1)];
    v[0] = DPI_REPORT_ID;
    v
}

/// What the Mouse page offers: OpenMouse's DPI values (50-step to 10000, 100-step to 20200, 200-step to 22000) on the receiver
/// only, polling 125 / 250 / 500 / 1000, no lift-off, no battery (it only arrives unasked). OpenMouse itself offers every value
/// of its map, but nine of them (20300, 20500 ... 21900) write the same byte and flags as the value 100 below and the mouse reads
/// back the lower one, so they are not offered (see [`dpi_is_exact`]).
pub fn caps(wired: bool) -> Caps {
    Caps { dpi: (!wired).then(|| DpiCaps { bands: vec![(50, 10000, 50), (10100, 20200, 100), (20400, 22000, 200)] }), polling: POLLING_RATES.to_vec(), lift_off: Vec::new(), battery: false }
}

/// Is `dpi` a value the report can hold without the mouse reading back a different one? (`false` for the nine map entries that
/// share their byte and flags with the value 100 below: 20300, 20500 ... 21900.)
pub fn dpi_is_exact(dpi: u32) -> bool {
    let v = nearest_up(dpi);
    v == dpi && {
        let s = DpiState { stages: [v; STAGES], active: 1, angle_snap: false, ripple: false };
        decode_dpi_report(&build_dpi_report(&s, false)).is_some_and(|d| d.stages[0] == v)
    }
}

/// The read-only proof's allow-list: NO request passes. The one Delux read is a GetFeature of report 0x04
/// (`HidTransfer::FeatureIn`), which sends nothing and so passes the gate by itself. The allow-list sees only bytes, and a blank
/// report 0x04 sent as a SetFeature would be a DPI-table write with a valid checksum, so it must not pass. Every write - the
/// polling report 0x06, the DPI table (its second byte is 0x38) - is refused as well.
pub fn read_request_allowed(_out: &[u8]) -> bool {
    false
}

// ============================================================================ device table

fn has_delux_word(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    l.match_indices("delux").any(|(i, w)| {
        let before = l[..i].chars().next_back().is_none_or(|c| !c.is_alphanumeric() && c != '_');
        let after = l[i + w.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric() && c != '_');
        before && after
    })
}

/// OpenMouse's isSupported for the M800 Mini: the vendor id, one of the two ids AND a product string with the word "Delux"
/// (the ids are shared with other mice); the interface is the config sub-collection (usage page 0x0B with a feature report).
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID || (pid != PID_WIRELESS && pid != PID_WIRED) {
        return None;
    }
    if !own.iter().any(|h| h.product.as_deref().is_some_and(has_delux_word)) {
        return None;
    }
    let cfg = own.iter().find(|h| h.usage_page == CONFIG_USAGE_PAGE && h.feature_len > 0)?;
    Some(Matched { family: Family::Delux, model: Some(MODEL), wireless: Some(pid == PID_WIRELESS), config: (*cfg).clone(), variant: 0 })
}

/// readStatus' angleSnapping / rippleControl (the table's bytes 3 / 4 `== 1`) in the card's order (`None` = the guard's rows).
pub fn extras(s: Option<&DpiState>) -> Vec<ExtraRow> {
    vec![ExtraRow::switch(Extra::AngleSnap, s.map(|s| s.angle_snap)), ExtraRow::switch(Extra::RippleControl, s.map(|s| s.ripple))]
}

/// The receiver's table rewrite with one change (DPI or a switch): read, the safety check (the table must already be exactly
/// what OpenMouse would write for this state, zero padding included), the change, the write, 300 ms, the read-back.
fn rewrite_table(port: &mut dyn Port, change: impl FnOnce(&mut DpiState)) -> Result<(DpiState, DpiState)> {
    let len = port.info().feature_len as usize;
    let (cur, raw) = read_table(port)?;
    let same = padded(&build_dpi_report(&cur, false), raw.len()).is_some_and(|want| want == raw);
    if !same {
        return Err(Error::range("DPI table", "this mouse's DPI table holds settings OpenMouse's write would replace - it is left alone".to_string()));
    }
    let mut next = cur.clone();
    change(&mut next);
    let out = padded(&build_dpi_report(&next, false), len).ok_or_else(|| Error::UnsupportedMouse("the Delux config report is too short".into()))?;
    port.exchange(&out, &HidTransfer::FeatureOut)?;
    port.pause_ms(CMD_DELAY_MS);
    let (after, _) = read_table(port)?;
    Ok((cur, after))
}

// ============================================================================ driver

/// The Delux driver.
pub struct Delux;

fn is_wired(m: &Matched) -> bool {
    m.wireless == Some(false)
}

/// GetFeature of the DPI table (nothing is sent): the decoded state and the raw report.
fn read_table(port: &mut dyn Port) -> Result<(DpiState, Vec<u8>)> {
    let len = port.info().feature_len as usize;
    let raw = port.exchange(&read_dpi_request(len), &HidTransfer::FeatureIn { reply_id: DPI_REPORT_ID, reply_len: len, wait_ms: 0 })?;
    let state = decode_dpi_report(&raw).ok_or_else(|| Error::BadAnswer("the Delux DPI table could not be read".into()))?;
    Ok((state, raw))
}

impl crate::family::Driver for Delux {
    /// The receiver: the DPI table (OpenMouse's readDpiState, the active stage's DPI). The cable: OpenMouse reads nothing,
    /// so nothing is asked and nothing is known (the mouse is listed, so it is there). Polling and battery can't be read.
    fn read(&self, m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let wired = is_wired(m);
        let mut r = OnMouse { model: None, online: true, wireless: m.wireless, caps: caps(wired), ..Default::default() };
        if wired {
            return Ok(r);
        }
        match read_table(port) {
            Ok((s, _)) => {
                r.dpi = s.stages.get(s.active as usize - 1).copied();
                r.extras = extras(Some(&s));
            }
            Err(Error::BadAnswer(_) | Error::Os { .. }) => r.online = false,
            Err(e) => return Err(e),
        }
        Ok(r)
    }

    /// Order 110: setAngleSnapping / setRippleControl - checked before a byte leaves; receiver only (the cable's table is held
    /// back); the table rewrite with the one flag changed, read back.
    fn change(&self, m: &Matched, port: &mut dyn Port, c: Change) -> Result<()> {
        let Change::Extra(e, v) = c;
        check_extra(&extras(None), e, v)?;
        if is_wired(m) {
            return Err(Error::range(e.label(), "on the wired Delux M800 Mini OpenMouse never reads the DPI table this setting lives in, so a change would replace it with the factory defaults - use the receiver or Delux's own app".to_string()));
        }
        let on = v == 1;
        let (_, after) = rewrite_table(port, |s| if e == Extra::AngleSnap { s.angle_snap = on } else { s.ripple = on })?;
        if (if e == Extra::AngleSnap { after.angle_snap } else { after.ripple }) != on {
            return Err(Error::BadAnswer(format!("the mouse did not confirm {}", e.label())));
        }
        Ok(())
    }

    /// Polling: OpenMouse's setPollingRate (report 0x06, 300 ms), nothing read back (the mouse can't say). DPI (receiver
    /// only): read the table, check it is exactly what OpenMouse would write, change the active stage, write, read back.
    fn write(&self, m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        let wired = is_wired(m);
        let len = port.info().feature_len as usize;
        match s {
            Setting::Polling(hz) => {
                let req = polling_request(hz).ok_or_else(|| Error::range("polling rate", format!("{hz} Hz is not a polling rate of this mouse")))?;
                let out = padded(&req, len).ok_or_else(|| Error::UnsupportedMouse("the Delux config report is too short".into()))?;
                port.exchange(&out, &HidTransfer::FeatureOut)?;
                port.pause_ms(CMD_DELAY_MS);
                Ok(Changed { now: hz, before: None })
            }
            Setting::Dpi(dpi) => {
                if wired {
                    return Err(Error::range("DPI", "on the wired Delux M800 Mini OpenMouse never reads the DPI table, so a change would replace it with the factory defaults - use the receiver or Delux's own app".to_string()));
                }
                if !(DPI_MIN..=DPI_MAX).contains(&dpi) {
                    return Err(Error::range("DPI", format!("{dpi} DPI is outside {DPI_MIN}-{DPI_MAX}")));
                }
                let value = nearest_up(dpi);
                if !dpi_is_exact(value) {
                    return Err(Error::range("DPI", format!("{value} DPI shares its hardware byte with the value 100 below, the mouse would read back a different one - not written")));
                }
                // the safety check (in rewrite_table): the table must already be exactly what OpenMouse would write for this
                // state. The write is zero-padded to the whole feature-report length, so the WHOLE report read (not only its
                // first 56 bytes) must equal that padded report - nothing stored behind byte 56 may be overwritten with zeros.
                let (cur, after) = rewrite_table(port, |s| s.stages[s.active as usize - 1] = value)?;
                let at = cur.active as usize - 1;
                if after.stages[at] != value {
                    return Err(Error::BadAnswer("the mouse did not confirm the new DPI".into()));
                }
                Ok(Changed { now: value, before: Some(cur.stages[at]) })
            }
            Setting::LiftOff(t) => Err(Error::range("lift-off distance", format!("{t} tenths of a mm: the Delux M800 Mini has no lift-off setting here"))),
        }
    }
}

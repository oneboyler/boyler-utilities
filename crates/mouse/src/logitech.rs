//! Logitech HID++ 2.0 - ported from OpenMouse (`crate::family`: AGPL-3.0-or-later, @openmouse/protocol 0.40.1), DPI /
//! polling / lift-off / battery only:
//! - `dist/drivers/logitech/hidpp.js` (LogitechHidppClient): the device-index probe, getFeature, the DPI (0x2202 / legacy
//!   0x2201), report-rate (0x8061 / legacy 0x8060) and battery (0x1004 / 0x1000 / 0x1001 / 0x1f20) reads and writes, the
//!   identity read (0x0003, wired or not) and `ensureHostControl` (0x8100 onboard mode, before a write);
//! - `dist/logitech/index.js`: software id 5, device indices and their probe order, the 0x8060 rate bitmap, the battery
//!   states, the HID++ error words, the direct-connect and Bolt product ids;
//! - `dist/drivers/logitech/bolt.js`: which collections are HID++ and how a probe answer counts;
//! - `dist/drivers/vendors.js`: the receiver product ids;
//! - `dist/drivers/logitech/onboard-profiles.js`: ONLY the per-profile-format limits OpenMouse's live paths read - the
//!   lift-off level numbering, the report-rate ceilings per link, which formats keep a writable polling rate, the legacy
//!   DPI grid. Not one profile read or write.
//!
//! Frame (a long report, 20 bytes on Windows incl. the id): `[0]` 0x11 · `[1]` device index (0xFF = the mouse itself on
//! its cable or Bluetooth, 1..6 = a receiver's pairing slot) · `[2]` feature index (0 = root) · `[3]` function << 4 |
//! software id 5 · `[4..20]` parameters. The answer starts with the same four bytes; an error is `[1]` device, `[2]` 0xFF
//! (HID++ 2.0) or 0x8F (HID++ 1.0), `[3]` feature index, `[4]` function | software id, `[5]` error code. A feature's
//! index is the mouse's own: root getFeature(feature id) asks for it first. OpenMouse counts without the report id (its
//! `reply[i]` is our `a[i + 1]`); the decoders here take the answer WITHOUT its report id, so their indices are OpenMouse's.
//!
//! Windows: the HID++ short (0x10, usage 0xFF00:1) and long (0x11, usage 0xFF00:2) reports are separate collections, each
//! its own path, and the mouse answers HID++ 2.0 in long reports (OpenMouse reads `reply[12]` of the DPI answer - past a
//! short report's end). So every request goes as a long report on the long collection (or the Bluetooth page 0xFF43) and
//! its answer is read there. That is OpenMouse's own path when the report device has no short collection (Bolt,
//! Bluetooth, split collections): `requestLong`, the same bytes as its short form plus zero padding.
//! [`HidTransfer::OutputThenMatch`] skips notifications and takes the reply or the error frame of this one request.
//!
//! Left out on purpose: every onboard-profile write (OpenMouse's profile-flash fallback when a cabled mouse rejects the
//! live polling write, profile DPI stages, resets, the write probes), DFU / firmware (dfu.js), Bolt pairing, lighting,
//! buttons, hosts, names, wheels, haptics; the device-name read (its later chunks ask function 1 with an offset, which the
//! read allow-list cannot tell from a write). Unifying / Nano receivers are not in the table: no SUPPORTED model is pinned
//! to one. A collection with only short reports is not claimed.

use crate::error::{Error, Result};
use crate::family::{Caps, Changed, DpiCaps, Family, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};
use std::cell::RefCell;

pub const VENDOR_ID: u16 = 0x046D;
pub const LONG_REPORT_ID: u8 = 0x11;
/// A long report incl. its id (OpenMouse: 19 data bytes).
pub const LONG_LEN: usize = 20;
/// OpenMouse's SOFTWARE_ID (0 is the mouse's own notifications).
pub const SOFTWARE_ID: u8 = 0x05;
/// The mouse itself (cable / Bluetooth); 1..6 are a receiver's pairing slots.
pub const DEVICE_INDEX_DIRECT: u8 = 0xFF;
pub const HIDPP_USAGE_PAGE: u16 = 0xFF00;
/// Usage 1 = the short-report collection, 2 = the long one.
pub const HIDPP_LONG_USAGE: u16 = 0x0002;
/// HID++ over Bluetooth (and the PRO X 3 Superstrike's USB interface): long reports only.
pub const HIDPP_BLUETOOTH_USAGE_PAGE: u16 = 0xFF43;
/// An error answer's byte `[2]` (with the report id): HID++ 2.0 / HID++ 1.0.
pub const ERROR_HIDPP20: u8 = 0xFF;
pub const ERROR_HIDPP10: u8 = 0x8F;

pub const FEAT_ROOT: u16 = 0x0000;
/// Firmware / identity: the probe's question, and the transport ids that say "cable or radio".
pub const FEAT_FIRMWARE: u16 = 0x0003;
pub const FEAT_BATTERY_STATUS: u16 = 0x1000;
pub const FEAT_BATTERY_VOLTAGE: u16 = 0x1001;
pub const FEAT_UNIFIED_BATTERY: u16 = 0x1004;
pub const FEAT_ADC_MEASUREMENT: u16 = 0x1F20;
pub const FEAT_ADJUSTABLE_DPI: u16 = 0x2201;
pub const FEAT_EXTENDED_DPI: u16 = 0x2202;
pub const FEAT_REPORT_RATE: u16 = 0x8060;
pub const FEAT_EXTENDED_REPORT_RATE: u16 = 0x8061;
pub const FEAT_ONBOARD_PROFILES: u16 = 0x8100;

/// 0x8061's rate table (also the index its notification carries).
pub const REPORT_RATE_HZ: [u32; 7] = [125, 250, 500, 1000, 2000, 4000, 8000];
/// 0x8100 onboard mode: 1 = the mouse runs its stored profile, 2 = host (software) control.
pub const ONBOARD_MODE_HOST: u8 = 0x02;
pub const ONBOARD_MODE_ONBOARD: u8 = 0x01;
/// The battery features in OpenMouse's order of preference (a percentage outright first, then the voltage estimates).
pub const BATTERY_FEATURES: [u16; 4] = [FEAT_UNIFIED_BATTERY, FEAT_BATTERY_STATUS, FEAT_BATTERY_VOLTAGE, FEAT_ADC_MEASUREMENT];

/// OpenMouse's withSoftwareId: the function nibble + software id 5.
pub fn with_software_id(func: u8) -> u8 {
    (func & 0xF0) | SOFTWARE_ID
}

/// One long request (OpenMouse's requestLong): `[0x11, device, feature index, function | 5, params…]`, zero-padded.
pub fn request(dev: u8, feature_index: u8, func: u8, params: &[u8]) -> [u8; LONG_LEN] {
    let mut f = [0u8; LONG_LEN];
    f[0] = LONG_REPORT_ID;
    f[1] = dev;
    f[2] = feature_index;
    f[3] = with_software_id(func);
    let n = params.len().min(16);
    f[4..4 + n].copy_from_slice(&params[..n]);
    f
}

/// Root getFeature(feature id) → the feature's index on this mouse.
pub fn get_feature(dev: u8, feature_id: u16) -> [u8; LONG_LEN] {
    request(dev, 0x00, 0x00, &[(feature_id >> 8) as u8, feature_id as u8])
}

/// 0x0003 function 0: the identity (unit id, model id, transport ids).
pub fn identity(dev: u8, idx: u8) -> [u8; LONG_LEN] {
    request(dev, idx, 0x00, &[])
}

/// The battery read of one of the [`BATTERY_FEATURES`]: 0x1004 getStatus is function 1, the others function 0.
pub fn battery_request(feature_id: u16, dev: u8, idx: u8) -> [u8; LONG_LEN] {
    request(dev, idx, if feature_id == FEAT_UNIFIED_BATTERY { 0x10 } else { 0x00 }, &[])
}

/// 0x2202 readDpiConfiguration (function 5).
pub fn read_dpi(dev: u8, idx: u8) -> [u8; LONG_LEN] {
    request(dev, idx, 0x50, &[])
}

/// 0x2202 sensor capabilities (function 1, sensor 0): separate axes, lift-off control.
pub fn dpi_capabilities(dev: u8, idx: u8) -> [u8; LONG_LEN] {
    request(dev, idx, 0x10, &[0x00])
}

/// 0x2202 DPI list, one page (function 2: sensor 0, 0, page).
pub fn dpi_list_page(dev: u8, idx: u8, page: u8) -> [u8; LONG_LEN] {
    request(dev, idx, 0x20, &[0x00, 0x00, page])
}

/// 0x2202 set DPI (function 6, OpenMouse's setDpi / setLiftOffDistance): sensor 0, x and y big-endian, lift-off byte.
pub fn set_dpi(dev: u8, idx: u8, x: u16, y: u16, lod: u8) -> [u8; LONG_LEN] {
    request(dev, idx, 0x60, &[0x00, (x >> 8) as u8, x as u8, (y >> 8) as u8, y as u8, lod])
}

/// 0x2201 getSensorDpi (function 2, sensor 0).
pub fn legacy_read_dpi(dev: u8, idx: u8) -> [u8; LONG_LEN] {
    request(dev, idx, 0x20, &[0x00])
}

/// 0x2201 getSensorDpiList (function 1, sensor 0).
pub fn legacy_dpi_list(dev: u8, idx: u8) -> [u8; LONG_LEN] {
    request(dev, idx, 0x10, &[0x00])
}

/// 0x2201 setSensorDpi (function 3): sensor 0, DPI big-endian.
pub fn legacy_set_dpi(dev: u8, idx: u8, dpi: u16) -> [u8; LONG_LEN] {
    request(dev, idx, 0x30, &[0x00, (dpi >> 8) as u8, dpi as u8])
}

/// 0x8061 supported rates (function 1).
pub fn rate_list(dev: u8, idx: u8) -> [u8; LONG_LEN] {
    request(dev, idx, 0x10, &[])
}

/// 0x8061 current rate (function 2).
pub fn read_rate(dev: u8, idx: u8) -> [u8; LONG_LEN] {
    request(dev, idx, 0x20, &[])
}

/// 0x8061 set rate (function 3, the rate's index in [`REPORT_RATE_HZ`]).
pub fn set_rate(dev: u8, idx: u8, hz: u32) -> Option<[u8; LONG_LEN]> {
    rate_index(hz).map(|i| request(dev, idx, 0x30, &[i]))
}

/// 0x8060 getReportRateList (function 0).
pub fn legacy_rate_list(dev: u8, idx: u8) -> [u8; LONG_LEN] {
    request(dev, idx, 0x00, &[])
}

/// 0x8060 getReportRate (function 1).
pub fn legacy_read_rate(dev: u8, idx: u8) -> [u8; LONG_LEN] {
    request(dev, idx, 0x10, &[])
}

/// 0x8060 setReportRate (function 2, the interval in ms).
pub fn legacy_set_rate(dev: u8, idx: u8, hz: u32) -> Option<[u8; LONG_LEN]> {
    legacy_rate_ms(hz).map(|ms| request(dev, idx, 0x20, &[ms]))
}

/// 0x8100 getInfo (function 0): the profile format id.
pub fn profiles_info(dev: u8, idx: u8) -> [u8; LONG_LEN] {
    request(dev, idx, 0x00, &[])
}

/// 0x8100 getMode (function 2).
pub fn get_onboard_mode(dev: u8, idx: u8) -> [u8; LONG_LEN] {
    request(dev, idx, 0x20, &[])
}

/// 0x8100 setMode(host) (function 1) - OpenMouse's ensureHostControl before a write on a mouse behind a receiver. A
/// switch of the running mode (no flash write); refused by the allow-list.
pub fn set_host_mode(dev: u8, idx: u8) -> [u8; LONG_LEN] {
    request(dev, idx, 0x10, &[ONBOARD_MODE_HOST])
}

/// 0x8100 setMode(onboard) (function 1, ONBOARD_MODE.onboard = 1) - OpenMouse's setOnboardMode("Onboard"): sent only to put
/// back a mouse this app switched to host mode for a write that then failed. Refused by the allow-list.
pub fn set_onboard_mode(dev: u8, idx: u8) -> [u8; LONG_LEN] {
    request(dev, idx, 0x10, &[ONBOARD_MODE_ONBOARD])
}

/// The input reports that answer `req` (report id included): its reply, a HID++ 2.0 error, a HID++ 1.0 error.
pub fn answer_prefixes(req: &[u8]) -> Vec<Vec<u8>> {
    vec![req[..4].to_vec(), vec![LONG_REPORT_ID, req[1], ERROR_HIDPP20, req[2], req[3]], vec![LONG_REPORT_ID, req[1], ERROR_HIDPP10, req[2], req[3]]]
}

/// What came back for a request (the answer WITHOUT its report id - OpenMouse's `report`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Reply,
    Rejected { hidpp10: bool, code: u8 },
    Other,
}

/// OpenMouse's matching: `[dev, feature, function | id]` = the reply; `[dev, 0xFF / 0x8F, feature, function | id, code]`
/// = an error naming this request (hidppErrorForRequest).
pub fn classify(r: &[u8], dev: u8, feature_index: u8, func_with_id: u8) -> Answer {
    if r.len() >= 3 && r[0] == dev && r[1] == feature_index && r[2] == func_with_id {
        return Answer::Reply;
    }
    if r.len() >= 4 && r[0] == dev && (r[1] == ERROR_HIDPP20 || r[1] == ERROR_HIDPP10) && r[2] == feature_index && r[3] == func_with_id {
        return Answer::Rejected { hidpp10: r[1] == ERROR_HIDPP10, code: r.get(4).copied().unwrap_or(0) };
    }
    Answer::Other
}

/// OpenMouse's hidppErrorMessage / hidpp10ErrorMessage.
pub fn error_words(hidpp10: bool, code: u8) -> String {
    let w = if hidpp10 {
        match code {
            0x01 => "invalid command",
            0x02 => "invalid address",
            0x03 => "invalid value",
            0x04 => "connection request failed",
            0x05 => "too many devices",
            0x06 => "already exists",
            0x07 => "device busy",
            0x08 => "unknown device",
            0x09 => "resource error",
            0x0A => "request unavailable",
            0x0B => "unsupported parameter value",
            0x0C => "wrong PIN code",
            _ => return format!("The mouse rejected that setting (HID++ 1.0 error {code:#04x})."),
        }
    } else {
        match code {
            0x01 => "unknown request",
            0x02 => "invalid argument",
            0x03 => "value out of range",
            0x04 => "hardware error",
            0x05 => "Logitech internal error",
            0x06 => "invalid feature index",
            0x07 => "invalid function",
            0x08 => "device busy",
            0x09 => "unsupported",
            _ => return format!("The mouse rejected that setting (HID++ 2.0 error {code:#04x})."),
        }
    };
    if hidpp10 {
        format!("The mouse rejected that setting (HID++ 1.0: {w}).")
    } else {
        format!("The mouse rejected that setting ({w}).")
    }
}

fn at(r: &[u8], i: usize) -> u8 {
    r.get(i).copied().unwrap_or(0)
}

fn be16(r: &[u8], i: usize) -> u32 {
    ((at(r, i) as u32) << 8) | at(r, i + 1) as u32
}

/// getFeature answer → (index, version): `reply[3]`, `reply[6]`. Index 0 = the mouse does not have the feature.
pub fn decode_feature(r: &[u8]) -> (u8, u8) {
    (at(r, 3), at(r, 6))
}

/// A battery reading. `charging`: `None` = the state is unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Battery {
    pub percent: Option<u8>,
    pub charging: Option<bool>,
    pub voltage_mv: Option<u32>,
}

/// 0x1004 getStatus: `reply[3]` state of charge (> 100 = none), `reply[5]` charging status (decodeUnifiedBatteryState:
/// 0 discharging, 1 charging, 2 charging slowly, 3 full; 4 = a charging fault = unknown).
pub fn decode_unified_battery(r: &[u8]) -> Battery {
    let charging = match r.get(5) {
        Some(0) | Some(3) => Some(false),
        Some(1) | Some(2) => Some(true),
        _ => None,
    };
    Battery { percent: r.get(3).copied().filter(|p| *p <= 100), charging, voltage_mv: None }
}

/// 0x1000 getBatteryLevelStatus: `reply[3]` level (> 100 = none), `reply[5]` status (decodeBatteryLevelState: 0
/// discharging, 1 charging, 2 almost full, 3 full, 4 charging slowly; 5-7 faults = unknown).
pub fn decode_battery_level_status(r: &[u8]) -> Battery {
    let charging = match r.get(5) {
        Some(0) | Some(3) => Some(false),
        Some(1) | Some(2) | Some(4) => Some(true),
        _ => None,
    };
    Battery { percent: Some(r.get(3).copied().unwrap_or(0xFF)).filter(|p| *p <= 100), charging, voltage_mv: None }
}

/// 0x1001 voltage: `reply[3..5]` millivolts big-endian, `reply[5]` flags (0x80 charging; charging with flags & 3 = 2 is
/// full; 0x10 charging slowly). The percentage is OpenMouse's estimate from its voltage curve.
pub fn decode_battery_voltage(r: &[u8]) -> Battery {
    let mv = be16(r, 3);
    let flags = at(r, 5);
    let charging = flags & 0x80 != 0;
    let full = charging && flags & 0x03 == 0x02;
    let charging = if full { false } else { flags & 0x10 != 0 || charging };
    Battery { percent: (mv != 0).then(|| estimate_battery_percent(mv)), charging: Some(charging), voltage_mv: (mv != 0).then_some(mv) }
}

/// 0x1f20 ADC measurement: `reply[3..5]` millivolts, `reply[5]` flags (bit 0 = valid, bit 1 = charging).
pub fn decode_adc_measurement(r: &[u8]) -> Battery {
    let mv = be16(r, 3);
    let flags = at(r, 5);
    if flags & 0x01 == 0 || mv == 0 {
        return Battery::default();
    }
    Battery { percent: Some(estimate_battery_percent(mv)), charging: Some(flags & 0x02 != 0), voltage_mv: Some(mv) }
}

/// OpenMouse's BATTERY_VOLTAGE_CURVE (mV, %).
const BATTERY_VOLTAGE_CURVE: [(u32, u32); 13] = [(4186, 100), (4067, 90), (3989, 80), (3922, 70), (3859, 60), (3811, 50), (3778, 40), (3751, 30), (3717, 20), (3671, 10), (3646, 5), (3579, 2), (3500, 0)];

/// OpenMouse's estimateBatteryPercent: linear between the curve's points, rounded (halves up).
pub fn estimate_battery_percent(mv: u32) -> u8 {
    if mv >= BATTERY_VOLTAGE_CURVE[0].0 {
        return 100;
    }
    if mv <= BATTERY_VOLTAGE_CURVE[BATTERY_VOLTAGE_CURVE.len() - 1].0 {
        return 0;
    }
    for w in BATTERY_VOLTAGE_CURVE.windows(2) {
        let ((hi_mv, hi_p), (lo_mv, lo_p)) = (w[0], w[1]);
        if mv >= lo_mv {
            // lo_p + (hi_p − lo_p)(mv − lo_mv) / (hi_mv − lo_mv), rounded half up
            let num = (hi_p - lo_p) * (mv - lo_mv);
            let den = hi_mv - lo_mv;
            return (lo_p + (2 * num + den) / (2 * den)) as u8;
        }
    }
    0
}

/// 0x2202 readDpiConfiguration → (x, y, lift-off byte): `reply[4..6]`, `reply[8..10]`, `reply[12]`.
pub fn decode_dpi_configuration(r: &[u8]) -> (u32, u32, u8) {
    (be16(r, 4), be16(r, 8), at(r, 12))
}

/// 0x2202 capabilities → (separate axes, lift-off control): `reply[5]` bits 0 and 1.
pub fn decode_dpi_capabilities(r: &[u8]) -> (bool, bool) {
    let f = at(r, 5);
    (f & 0x01 != 0, f & 0x02 != 0)
}

/// 0x2201 getSensorDpi → `reply[4..6]`.
pub fn decode_legacy_dpi(r: &[u8]) -> u32 {
    be16(r, 4)
}

/// OpenMouse's page loop stops once the bytes so far hold two zeros in a row.
pub fn dpi_list_done(bytes: &[u8]) -> bool {
    bytes.windows(2).any(|w| w == [0, 0])
}

/// A DPI list (16-bit big-endian values, 0 ends it; a value with the top three bits set is `0xE000 | step`, from the
/// previous value to the next one) → every DPI it allows. `strict` (0x2202): a bad range is an error; else (0x2201) the
/// list just ends there.
pub fn parse_dpi_list(bytes: &[u8], strict: bool) -> Result<Vec<u32>> {
    let mut out: Vec<u32> = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let v = be16(bytes, i);
        if v == 0 {
            break;
        }
        if v >> 13 == 0b111 {
            let step = v & 0x1FFF;
            let last = be16(bytes, i + 2);
            let first = out.last().copied().unwrap_or(0);
            if first == 0 || last == 0 || last <= first || step == 0 {
                if strict {
                    return Err(Error::BadAnswer("The mouse returned an invalid DPI range.".into()));
                }
                break;
            }
            let mut d = first + step;
            while d <= last {
                out.push(d);
                d += step;
            }
            i += 4;
        } else {
            out.push(v);
            i += 2;
        }
    }
    Ok(out)
}

/// The DPI values a mouse allows → `DpiCaps` bands (runs with one step); `None` when there are none.
pub fn dpi_caps(options: &[u32]) -> Option<DpiCaps> {
    let mut v: Vec<u32> = options.to_vec();
    v.sort_unstable();
    v.dedup();
    let mut bands = Vec::new();
    let mut i = 0;
    while i < v.len() {
        if i + 1 == v.len() {
            bands.push((v[i], v[i], 1));
            break;
        }
        let step = v[i + 1] - v[i];
        let mut j = i + 1;
        while j + 1 < v.len() && v[j + 1] - v[j] == step {
            j += 1;
        }
        bands.push((v[i], v[j], step));
        i = j + 1;
    }
    (!bands.is_empty()).then_some(DpiCaps { bands })
}

/// Math.round(a / b) for positive numbers (halves up).
fn round_div(a: u32, b: u32) -> u32 {
    (2 * a + b) / (2 * b)
}

/// Hz → 0x8061's rate index.
pub fn rate_index(hz: u32) -> Option<u8> {
    REPORT_RATE_HZ.iter().position(|r| *r == hz).map(|i| i as u8)
}

/// Hz → 0x8060's interval in ms (round(1000 / Hz), 1..8 - OpenMouse's setLegacyReportRate check).
pub fn legacy_rate_ms(hz: u32) -> Option<u8> {
    if hz == 0 {
        return None;
    }
    let ms = round_div(1000, hz);
    (1..=8).contains(&ms).then_some(ms as u8)
}

/// 0x8061 supported rates: `reply[3..5]` big-endian flags, bit i = [`REPORT_RATE_HZ`]`[i]`.
pub fn decode_supported_rates(r: &[u8]) -> Vec<u32> {
    let flags = be16(r, 3);
    REPORT_RATE_HZ.iter().enumerate().filter(|(i, _)| flags & (1 << i) != 0).map(|(_, r)| *r).collect()
}

/// 0x8061 current rate: `reply[3]` = index into [`REPORT_RATE_HZ`].
pub fn decode_rate(r: &[u8]) -> Option<u32> {
    r.get(3).and_then(|i| REPORT_RATE_HZ.get(*i as usize)).copied()
}

/// OpenMouse's decodeReportRateBitmap (0x8060): bit i = an interval of i + 1 ms, ascending Hz.
pub fn decode_report_rate_bitmap(bits: u8) -> Vec<u32> {
    let mut v: Vec<u32> = (0..8).filter(|b| bits & (1 << b) != 0).map(|b| round_div(1000, b + 1)).collect();
    v.sort_unstable();
    v
}

/// 0x8060 rate list: `reply[3]` = the bitmap.
pub fn decode_legacy_rates(r: &[u8]) -> Vec<u32> {
    decode_report_rate_bitmap(at(r, 3))
}

/// 0x8060 current rate: `reply[3]` = ms (0 = unknown).
pub fn decode_legacy_rate(r: &[u8]) -> Option<u32> {
    let ms = at(r, 3) as u32;
    (ms != 0).then(|| round_div(1000, ms))
}

/// The 0x8061 "rate changed" notification OpenMouse waits for after a write: `[dev, feature, 0x00, 0x01, rate index]`.
pub fn decode_rate_notification(r: &[u8], dev: u8, feature_index: u8) -> Option<u32> {
    (r.len() >= 5 && r[0] == dev && r[1] == feature_index && r[2] == 0x00 && r[3] == 0x01).then(|| REPORT_RATE_HZ.get(r[4] as usize).copied()).flatten()
}

/// How a mouse can be linked, as 0x0003's identity names its transports (in its order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Bluetooth,
    BluetoothLe,
    Wireless,
    Usb,
}

/// 0x0003 identity (readIdentity): payload = `reply[3..]`, `payload[6]` transport flags (0x01 Bluetooth, 0x02 BLE, 0x04
/// Wireless, 0x08 USB), each present one takes the next two bytes of the model id `payload[7..13]` as its product id.
pub fn decode_identity(r: &[u8]) -> Vec<(Transport, u16)> {
    let p = r.get(3..).unwrap_or(&[]);
    if p.len() < 13 {
        return Vec::new();
    }
    let model = &p[7..13];
    let mut out = Vec::new();
    let mut off = 0;
    for (t, flag) in [(Transport::Bluetooth, 0x01), (Transport::BluetoothLe, 0x02), (Transport::Wireless, 0x04), (Transport::Usb, 0x08)] {
        if p[6] & flag != 0 && off + 2 <= model.len() {
            out.push((t, u16::from_be_bytes([model[off], model[off + 1]])));
            off += 2;
        }
    }
    out
}

/// OpenMouse's isWiredHidppConnection: the transport whose id is the connected product id says it (USB = wired);
/// no match (a receiver's id) → the direct-index answer decides.
pub fn is_wired_connection(pid: u16, ids: &[(Transport, u16)], direct_index: bool) -> bool {
    match ids.iter().find(|(_, id)| *id == pid) {
        Some((t, _)) => *t == Transport::Usb,
        None => direct_index,
    }
}

/// 0x8100 getInfo → the profile format id (`reply[4]`).
pub fn decode_profile_format(r: &[u8]) -> u8 {
    at(r, 4)
}

/// onboard-profiles.js reportRates per format: (wireless max, wired max) Hz; `None` = not established.
fn format_rate_ceilings(format: Option<u8>) -> Option<(u32, u32)> {
    match format? {
        1..=4 => Some((1000, 1000)),
        7 | 8 => Some((8000, 1000)),
        _ => None,
    }
}

/// OpenMouse's connection ceiling (reportRateCapabilitiesFor): the format's, with the cable lifted to the wireless one for
/// a product known to run the full range over USB (transport id C0A9, the PRO X 3 Superstrike).
pub fn rate_ceiling(format: Option<u8>, usb_transport: Option<u16>, wired: bool) -> Option<u32> {
    let (wireless, cable) = format_rate_ceilings(format)?;
    let cable = if usb_transport == Some(0xC0A9) { wireless } else { cable };
    Some(if wired { cable } else { wireless })
}

/// A cabled (direct-index) mouse keeps its polling rate in its onboard profile; OpenMouse lets it be changed only when the
/// format is one it writes (2, 3, 4, 7, 8) and carries a rate - otherwise it reads it without changing it.
pub fn profile_rate_writable(format: Option<u8>) -> bool {
    matches!(format, Some(2 | 3 | 4 | 7 | 8)) && format_rate_ceilings(format).is_some()
}

/// The format's captured DPI grid (min, max, step) - OpenMouse's fallback when a legacy 0x2201 list comes back empty.
pub fn legacy_dpi_grid(format: Option<u8>) -> Option<(u32, u32, u32)> {
    Some(match format? {
        1 => (252, 4032, 84),
        2 => (100, 25600, 50),
        3 => (50, 12000, 50),
        4 => (50, 8000, 50),
        7 => (100, 32000, 50),
        8 => (100, 48000, 50),
        _ => return None,
    })
}

/// The 0x2202 lift-off byte of Low / Medium / High: formats 7 and 8 count from 1 (LOD_ENCODING); every other mouse with
/// live lift-off control from 0 (OpenMouse's G502 X-family override, after Solaar).
pub fn lod_codes(format: Option<u8>) -> [u8; 3] {
    if matches!(format, Some(7 | 8)) {
        [1, 2, 3]
    } else {
        [0, 1, 2]
    }
}

// Lift-off is NOT offered for Logitech (Order 104): OpenMouse names only levels (Low / Medium / High), never millimetres,
// and the page sets lift-off in millimetres - a mapping would be made up. A DPI write keeps the mouse's current byte.

// ============================================================================ device table

/// VID 046D product ids → model (`None` = a receiver many models pair with) + wireless (`None` = the id does not say),
/// from OpenMouse's SUPPORTED list ("supported" entries only). Cabled mice from its pids, or for a pid-less entry the
/// LOGITECH_PRODUCTS id of that exact model (G303 Shroud Edition c097, G903 Lightspeed c086, G703 (wired) c087, G Pro
/// Wireless c088, G502 Lightspeed c08d). Receivers: the Superstrikes' own (named by SUPPORTED), Bolt c548 (MX Master 3S,
/// MX Anywhere 3 / 3S, MX Ergo S, MX Master 4) and the Lightspeed receivers OpenMouse lists (G305, G304, G309, G603, G604,
/// G703 / G903 / G502 Lightspeed, G Pro Wireless, the Superlights, G PRO 2).
pub const TABLE: [(u16, Option<&str>, Option<bool>); 36] = [
    (0xC07D, Some("Logitech G502 (all variants)"), Some(false)),
    (0xC08B, Some("Logitech G502 HERO (wired)"), Some(false)),
    (0xC092, Some("Logitech G203 LIGHTSYNC"), Some(false)),
    (0xC084, Some("Logitech G203 PRODIGY"), Some(false)),
    (0xC089, Some("Logitech G102 LIGHTSYNC"), Some(false)),
    (0xC07E, Some("Logitech G402 Hyperion Fury"), Some(false)),
    (0xC080, Some("Logitech G303 Daedalus Apex"), Some(false)),
    (0xC097, Some("Logitech G303 Shroud Edition"), Some(false)),
    (0xC085, Some("Logitech G Pro (2017)"), Some(false)),
    (0xC08C, Some("Logitech G Pro Hero"), Some(false)),
    (0xC08E, Some("Logitech MX518 (Legendary)"), Some(false)),
    (0xC086, Some("Logitech G903 Lightspeed"), Some(false)),
    (0xC08F, Some("Logitech G403 HERO (wired)"), Some(false)),
    (0xC087, Some("Logitech G703 (wired)"), Some(false)),
    (0xC094, Some("Logitech G Pro X Superlight"), Some(false)),
    (0xC09B, Some("Logitech PRO X SUPERLIGHT 2"), Some(false)),
    (0xC0A0, Some("Logitech PRO X SUPERLIGHT 2 DEX"), Some(false)),
    (0xC099, Some("Logitech G502 X (wired)"), Some(false)),
    (0xC095, Some("Logitech G502 X PLUS (wireless)"), Some(false)),
    (0xC098, Some("Logitech G502 X LIGHTSPEED"), Some(false)),
    (0xC088, Some("Logitech G Pro Wireless"), Some(false)),
    (0xC09A, Some("Logitech G Pro 2 Lightspeed"), Some(false)),
    (0xC08D, Some("Logitech G502 Lightspeed"), Some(false)),
    // the PRO X 2 Superstrike: c0a8 its USB interface, 40bd its own receiver (OpenMouse: "transportIds {Wireless: 40BD,
    // USB: C0A8}") - which one is the radio is not certain, so neither says
    (0xC0A8, Some("Logitech PRO X2 SUPERSTRIKE"), None),
    (0x40BD, Some("Logitech PRO X2 SUPERSTRIKE"), None),
    (0xC54F, Some("Logitech PRO X3 SUPERSTRIKE"), Some(true)),
    (0xC548, None, Some(true)),
    (0xC539, None, Some(true)),
    (0xC53A, None, Some(true)),
    (0xC53D, None, Some(true)),
    (0xC53F, None, Some(true)),
    (0xC541, None, Some(true)),
    (0xC545, None, Some(true)),
    (0xC547, None, Some(true)),
    (0xC54D, None, Some(true)),
    (0xC543, None, Some(true)),
];

/// OpenMouse's LOGITECH_RECEIVER_PRODUCT_IDS (vendors.js): only for the device-index probe order.
pub const RECEIVER_PRODUCT_IDS: [u16; 26] = [
    0xC54D, 0xC543, 0xC539, 0xC0A8, 0xC547, 0x40BD, 0xC54F, 0xC52B, 0xC532, 0xC53A, 0xC53D, 0xC53F, 0xC541, 0xC545, 0xC518, 0xC51A, 0xC51B, 0xC521, 0xC525, 0xC526, 0xC52E,
    0xC52F, 0xC531, 0xC534, 0xC537, 0xC548,
];

/// `Matched.variant` bit: the device has a collection on the Bluetooth HID++ page (OpenMouse's isBluetooth: never a
/// "direct" mouse, so a write first takes host control like behind a receiver).
pub const VARIANT_BLUETOOTH_PAGE: u16 = 0x0001;

pub fn is_known_receiver(pid: u16) -> bool {
    RECEIVER_PRODUCT_IDS.contains(&pid)
}

/// OpenMouse's hidppDeviceIndexCandidates: a receiver's six slots then the direct index; else direct, then slot 1.
pub fn index_candidates(pid: u16) -> Vec<u8> {
    if is_known_receiver(pid) {
        vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, DEVICE_INDEX_DIRECT]
    } else {
        vec![DEVICE_INDEX_DIRECT, 0x01]
    }
}

/// The collection to talk through: HID++ long (0xFF00 usage 2), else the Bluetooth page 0xFF43 - both with ≥ 20-byte
/// input and output reports (OpenMouse's hasHidppLongCollection / hasHidppBluetoothCollection).
pub fn hidpp_interface<'a>(own: &[&'a HidInfo]) -> Option<&'a HidInfo> {
    let long = |h: &&&HidInfo| h.output_len as usize >= LONG_LEN && h.input_len as usize >= LONG_LEN;
    own.iter()
        .filter(long)
        .find(|h| h.usage_page == HIDPP_USAGE_PAGE && h.usage == HIDPP_LONG_USAGE)
        .or_else(|| own.iter().filter(long).find(|h| h.usage_page == HIDPP_BLUETOOTH_USAGE_PAGE))
        .copied()
}

/// A Logitech id from [`TABLE`] with a HID++ long-report collection.
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID {
        return None;
    }
    let (_, model, wireless) = *TABLE.iter().find(|t| t.0 == pid)?;
    let config = hidpp_interface(own)?;
    let bt = own.iter().any(|h| h.usage_page == HIDPP_BLUETOOTH_USAGE_PAGE);
    Some(Matched { family: Family::Logitech, model, wireless, config: config.clone(), variant: if bt { VARIANT_BLUETOOTH_PAGE } else { 0 } })
}

// ============================================================================ allow-list

thread_local! {
    /// (device index, feature index, feature id) the mouse itself named in answers to root getFeature on this thread.
    static LEARNED: RefCell<Vec<(u8, u8, u16)>> = const { RefCell::new(Vec::new()) };
}

/// Keeps what a root getFeature answer says (which feature sits at which index) for [`read_request_allowed`]. `req` and
/// `a` with their report ids; anything that is not such a pair is ignored.
pub fn learn_from_answer(req: &[u8], a: &[u8]) {
    if req.len() < 6 || a.len() < 5 || req[0] != LONG_REPORT_ID || req[2] != 0x00 || req[3] != with_software_id(0x00) || a[..4] != req[..4] || a[4] == 0 {
        return;
    }
    let e = (req[1], a[4], u16::from_be_bytes([req[4], req[5]]));
    LEARNED.with(|l| {
        let mut l = l.borrow_mut();
        if !l.contains(&e) {
            l.push(e);
        }
    });
}

/// Forgets every learned feature index (tests).
pub fn forget_learned() {
    LEARNED.with(|l| l.borrow_mut().clear());
}

/// The read requests of each feature this driver sends: (feature id, function, parameters must be all zero except the
/// DPI list's page byte).
fn read_shape(feature: u16, func: u8, p: &[u8]) -> bool {
    let zero_from = |n: usize| p.iter().skip(n).all(|b| *b == 0);
    match (feature, func) {
        (FEAT_FIRMWARE, 0x00) | (FEAT_BATTERY_STATUS, 0x00) | (FEAT_BATTERY_VOLTAGE, 0x00) | (FEAT_ADC_MEASUREMENT, 0x00) | (FEAT_UNIFIED_BATTERY, 0x10) => zero_from(0),
        (FEAT_EXTENDED_DPI, 0x10) | (FEAT_EXTENDED_DPI, 0x50) => zero_from(0),
        // the DPI list: sensor 0, 0, page < 32
        (FEAT_EXTENDED_DPI, 0x20) => p.len() >= 3 && p[0] == 0 && p[1] == 0 && p[2] < 32 && zero_from(3),
        (FEAT_ADJUSTABLE_DPI, 0x10) | (FEAT_ADJUSTABLE_DPI, 0x20) => zero_from(0),
        (FEAT_EXTENDED_REPORT_RATE, 0x10) | (FEAT_EXTENDED_REPORT_RATE, 0x20) => zero_from(0),
        (FEAT_REPORT_RATE, 0x00) | (FEAT_REPORT_RATE, 0x10) => zero_from(0),
        (FEAT_ONBOARD_PROFILES, 0x00) | (FEAT_ONBOARD_PROFILES, 0x20) => zero_from(0),
        _ => false,
    }
}

/// The read-only proof's allow-list (A_009_01). A well-formed long request (id 0x11, ≥ 20 bytes, device 1..6 or 0xFF,
/// software id 5) that is root getFeature (a lookup - any feature id), or one of [`read_shape`]'s reads on a feature
/// index the mouse itself named in a root getFeature answer on this thread (an index is the mouse's own: function 1 is a
/// read on 0x2202 but "switch to host mode" on 0x8100, so without the mouse's answer nothing but root passes). An index
/// two answers named differently is refused. Every write - set DPI, set rate, set mode, anything unknown - is refused.
pub fn read_request_allowed(out: &[u8]) -> bool {
    if out.len() < LONG_LEN || out[0] != LONG_REPORT_ID || !((1..=6).contains(&out[1]) || out[1] == DEVICE_INDEX_DIRECT) || out[3] & 0x0F != SOFTWARE_ID {
        return false;
    }
    let (dev, idx, func, p) = (out[1], out[2], out[3] & 0xF0, &out[4..]);
    if idx == 0 {
        return func == 0x00 && p[2..].iter().all(|b| *b == 0);
    }
    let ids: Vec<u16> = LEARNED.with(|l| l.borrow().iter().filter(|e| e.0 == dev && e.1 == idx).map(|e| e.2).collect());
    ids.len() == 1 && read_shape(ids[0], func, p)
}

// ============================================================================ driver

/// The index probe's wait (OpenMouse: BOLT_INDEX_PROBE_TIMEOUT_MS for Bolt; used for every probe here - an awake mouse
/// answers in milliseconds, and an empty slot's HID++ 1.0 error comes as a short report Windows puts on the other
/// collection, so an empty slot always runs to this timeout).
pub const PROBE_TIMEOUT_MS: u32 = 800;
/// One request's wait (OpenMouse waits 6000 ms; the worker should not hang that long on a mouse that went away).
pub const REQUEST_TIMEOUT_MS: u32 = 2000;
/// The wait for 0x8061's "rate changed" notification after a rate write.
pub const RATE_CONFIRM_TIMEOUT_MS: u32 = 3000;

/// What came back for one request.
enum Reply {
    /// the answer without its report id (OpenMouse's `reply`)
    Data(Vec<u8>),
    Rejected { hidpp10: bool, code: u8 },
}

fn send(port: &mut dyn Port, req: &[u8; LONG_LEN], prefixes: Vec<Vec<u8>>, timeout_ms: u32) -> Result<Vec<u8>> {
    // Windows wants exactly OutputReportByteLength bytes, zero-padded.
    let (out_len, in_len) = (port.info().output_len as usize, port.info().input_len as usize);
    let mut out = req.to_vec();
    out.resize(out_len.max(LONG_LEN), 0);
    let how = HidTransfer::OutputThenMatch { reply_len: in_len.max(LONG_LEN), max_reads: 32, timeout_ms, prefixes };
    let a = port.exchange(&out, &how)?;
    learn_from_answer(&out, &a);
    if a.first() != Some(&LONG_REPORT_ID) {
        return Err(Error::BadAnswer(format!("not a HID++ long report: {:02x?}", a.first())));
    }
    Ok(a[1..].to_vec())
}

fn exchange(port: &mut dyn Port, req: &[u8; LONG_LEN], timeout_ms: u32) -> Result<Reply> {
    let r = send(port, req, answer_prefixes(req), timeout_ms)?;
    match classify(&r, req[1], req[2], req[3]) {
        Answer::Reply => Ok(Reply::Data(r)),
        Answer::Rejected { hidpp10, code } => Ok(Reply::Rejected { hidpp10, code }),
        Answer::Other => Err(Error::BadAnswer(format!("an answer for another request: {:02x?}", &r[..r.len().min(4)]))),
    }
}

/// One request that must be answered (a rejection is an error with OpenMouse's words).
fn call(port: &mut dyn Port, req: &[u8; LONG_LEN]) -> Result<Vec<u8>> {
    match exchange(port, req, REQUEST_TIMEOUT_MS)? {
        Reply::Data(r) => Ok(r),
        Reply::Rejected { hidpp10, code } => Err(Error::BadAnswer(error_words(hidpp10, code))),
    }
}

/// getFeature → the index (0 = the mouse does not have it; a rejected lookup counts as not having it).
fn feature(port: &mut dyn Port, dev: u8, id: u16) -> Result<u8> {
    match exchange(port, &get_feature(dev, id), REQUEST_TIMEOUT_MS)? {
        Reply::Data(r) => Ok(decode_feature(&r).0),
        Reply::Rejected { .. } => Ok(0),
    }
}

/// The mouse found by the probe.
#[derive(Clone, Copy, Debug)]
struct Mouse {
    dev: u8,
    /// OpenMouse's isDirectConnect: the direct index and not the Bluetooth page
    direct: bool,
    /// the 0x0003 index (identity)
    firmware: u8,
    /// the DPI feature's index and whether it is the legacy 0x2201
    dpi: u8,
    legacy_dpi: bool,
}

/// OpenMouse's resolveDeviceIndex + its sensor check (hasDpiFeature / resolveDpiFeature): root getFeature(0x0003) on
/// each candidate index; a reply or a HID++ 2.0 error = someone is there (a HID++ 1.0 error or silence = nobody); it is
/// the mouse once it has a DPI feature (0x2202, else 0x2201) - a keyboard on the same receiver has none. `Ok(None)` = no
/// one answered (asleep / switched off); answered but no sensor = not a mouse.
fn find_mouse(port: &mut dyn Port, m: &Matched) -> Result<Option<Mouse>> {
    let bt = m.variant & VARIANT_BLUETOOTH_PAGE != 0;
    let mut answered = false;
    for dev in index_candidates(m.config.pid) {
        let firmware = match exchange(port, &get_feature(dev, FEAT_FIRMWARE), PROBE_TIMEOUT_MS) {
            Ok(Reply::Data(r)) => decode_feature(&r).0,
            Ok(Reply::Rejected { hidpp10: false, .. }) => 0,
            Ok(Reply::Rejected { hidpp10: true, .. }) | Err(Error::BadAnswer(_)) => continue,
            Err(e) => return Err(e),
        };
        answered = true;
        // hasDpiFeature; a lookup whose answer got lost skips this index (Order 104 review: counting it as "no 0x2202"
        // could pick the legacy 0x2201 path for a mouse that has 0x2202)
        let (dpi, legacy_dpi) = match feature(port, dev, FEAT_EXTENDED_DPI) {
            Ok(0) => match feature(port, dev, FEAT_ADJUSTABLE_DPI) {
                Ok(i) => (i, true),
                Err(Error::BadAnswer(_)) => continue,
                Err(e) => return Err(e),
            },
            Ok(i) => (i, false),
            Err(Error::BadAnswer(_)) => continue,
            Err(e) => return Err(e),
        };
        if dpi != 0 {
            return Ok(Some(Mouse { dev, direct: !bt && dev == DEVICE_INDEX_DIRECT, firmware, dpi, legacy_dpi }));
        }
    }
    if answered {
        return Err(Error::UnsupportedMouse(format!(
            "{} is not a mouse - it speaks Logitech's protocol but has no sensor to configure. Pick your mouse, or its receiver, instead.",
            m.config.product.clone().unwrap_or_else(|| "That Logitech device".into())
        )));
    }
    Ok(None)
}

fn asleep() -> Error {
    Error::MouseGone("the Logitech mouse did not answer - move it or click a button, then try again".into())
}

/// readIdentity → (wired, USB transport id).
fn link(port: &mut dyn Port, ms: &Mouse, pid: u16) -> Result<(bool, Option<u16>)> {
    let ids = if ms.firmware != 0 { decode_identity(&call(port, &identity(ms.dev, ms.firmware))?) } else { Vec::new() };
    let usb = ids.iter().find(|(t, _)| *t == Transport::Usb).map(|(_, id)| *id);
    Ok((is_wired_connection(pid, &ids, ms.direct), usb))
}

/// 0x8100 getInfo's profile format (`None` = no 0x8100, or the request failed - OpenMouse's `.catch(() => null)`).
fn profile_format(port: &mut dyn Port, dev: u8) -> Result<Option<u8>> {
    let idx = feature(port, dev, FEAT_ONBOARD_PROFILES)?;
    if idx == 0 {
        return Ok(None);
    }
    match exchange(port, &profiles_info(dev, idx), REQUEST_TIMEOUT_MS) {
        Ok(Reply::Data(r)) => Ok(Some(decode_profile_format(&r))),
        Ok(Reply::Rejected { .. }) | Err(Error::BadAnswer(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

/// OpenMouse's getDpiOptions: 0x2202's pages (up to 32, until two zeros in a row), or 0x2201's list - an empty one falls
/// back to the profile format's grid (`format` is asked only then).
fn dpi_options(port: &mut dyn Port, ms: &Mouse) -> Result<Vec<u32>> {
    if ms.legacy_dpi {
        let r = call(port, &legacy_dpi_list(ms.dev, ms.dpi))?;
        let list = parse_dpi_list(r.get(4..).unwrap_or(&[]), false)?;
        if !list.is_empty() {
            return Ok(list);
        }
        return Ok(match legacy_dpi_grid(profile_format(port, ms.dev)?) {
            Some((a, b, s)) => (0..=(b - a) / s).map(|i| a + i * s).collect(),
            None => Vec::new(),
        });
    }
    let mut bytes = Vec::new();
    for page in 0..32u8 {
        let r = call(port, &dpi_list_page(ms.dev, ms.dpi, page))?;
        bytes.extend_from_slice(r.get(6..).unwrap_or(&[]));
        if dpi_list_done(&bytes) {
            break;
        }
    }
    parse_dpi_list(&bytes, true)
}

/// resolveReportRateFeature: 0x8061, else 0x8060 → (index, legacy); `None` = no rate feature (MX Master 3S).
fn rate_feature(port: &mut dyn Port, dev: u8) -> Result<Option<(u8, bool)>> {
    let ext = feature(port, dev, FEAT_EXTENDED_REPORT_RATE)?;
    if ext != 0 {
        return Ok(Some((ext, false)));
    }
    let legacy = feature(port, dev, FEAT_REPORT_RATE)?;
    Ok((legacy != 0).then_some((legacy, true)))
}

fn current_rate(port: &mut dyn Port, dev: u8, (idx, legacy): (u8, bool)) -> Result<Option<u32>> {
    Ok(if legacy { decode_legacy_rate(&call(port, &legacy_read_rate(dev, idx))?) } else { decode_rate(&call(port, &read_rate(dev, idx))?) })
}

fn supported_rates(port: &mut dyn Port, dev: u8, (idx, legacy): (u8, bool)) -> Result<Vec<u32>> {
    Ok(if legacy { decode_legacy_rates(&call(port, &legacy_rate_list(dev, idx))?) } else { decode_supported_rates(&call(port, &rate_list(dev, idx))?) })
}

/// The battery from the first of [`BATTERY_FEATURES`] the mouse has (`None` = none of them).
fn battery(port: &mut dyn Port, dev: u8) -> Result<Option<Battery>> {
    for id in BATTERY_FEATURES {
        let idx = feature(port, dev, id)?;
        if idx == 0 {
            continue;
        }
        let r = call(port, &battery_request(id, dev, idx))?;
        return Ok(Some(match id {
            FEAT_UNIFIED_BATTERY => decode_unified_battery(&r),
            FEAT_BATTERY_STATUS => decode_battery_level_status(&r),
            FEAT_BATTERY_VOLTAGE => decode_battery_voltage(&r),
            _ => decode_adc_measurement(&r),
        }));
    }
    Ok(None)
}

/// OpenMouse's ensureHostControl (before every write, not on a cabled mouse): a mouse behind a receiver (or on
/// Bluetooth) running its onboard profile is switched to host mode (0x8100 setMode 2), then asked again; one that refuses
/// keeps its settings in onboard memory only (OnboardOnlyError) - nothing is written then. `Some(index)` = it was
/// switched now (the 0x8100 index, for [`set_onboard_mode`] when the write then fails).
fn ensure_host_control(port: &mut dyn Port, ms: &Mouse) -> Result<Option<u8>> {
    if ms.direct {
        return Ok(None);
    }
    let idx = feature(port, ms.dev, FEAT_ONBOARD_PROFILES)?;
    if idx == 0 {
        return Ok(None);
    }
    if at(&call(port, &get_onboard_mode(ms.dev, idx))?, 3) == ONBOARD_MODE_HOST {
        return Ok(None);
    }
    let refused = match exchange(port, &set_host_mode(ms.dev, idx), REQUEST_TIMEOUT_MS) {
        Ok(Reply::Data(_)) => false,
        Ok(Reply::Rejected { .. }) | Err(Error::BadAnswer(_)) => true,
        Err(e) => return Err(e),
    };
    let confirmed = if refused { None } else { call(port, &get_onboard_mode(ms.dev, idx)).ok().map(|r| at(&r, 3)) };
    if refused || confirmed != Some(ONBOARD_MODE_HOST) {
        return Err(Error::UnsupportedMouse(
            "This mouse keeps its settings in onboard memory and will not hand control to software, so this can only be changed in Logitech's own app."
                .into(),
        ));
    }
    Ok(Some(idx))
}

/// The Logitech HID++ 2.0 driver.
pub struct Logitech;

impl crate::family::Driver for Logitech {
    /// The probe, then battery, DPI (+ its list and lift-off), identity (cable or radio), profile format (limits) and the
    /// report rate - READ requests only (root getFeature and the [`read_shape`] reads).
    fn read(&self, m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        // the allow-list's learned feature indices are per mouse and per visit (Order 104 review)
        forget_learned();
        let Some(ms) = find_mouse(port, m)? else {
            return Ok(OnMouse { online: false, wireless: m.wireless, ..Default::default() });
        };
        let bat = battery(port, ms.dev)?;
        let dpi = if ms.legacy_dpi {
            decode_legacy_dpi(&call(port, &legacy_read_dpi(ms.dev, ms.dpi))?)
        } else {
            decode_dpi_configuration(&call(port, &read_dpi(ms.dev, ms.dpi))?).0
        };
        let options = dpi_options(port, &ms)?;
        let (wired, usb) = link(port, &ms, m.config.pid)?;
        let format = profile_format(port, ms.dev)?;
        let ceiling = rate_ceiling(format, usb, wired);
        let (mut polling_hz, mut polling) = (None, Vec::new());
        if let Some(rf) = rate_feature(port, ms.dev)? {
            let rates: Vec<u32> = supported_rates(port, ms.dev, rf)?.into_iter().filter(|r| ceiling.is_none_or(|c| *r <= c)).collect();
            polling_hz = current_rate(port, ms.dev, rf)?.map(|r| ceiling.map_or(r, |c| r.min(c)));
            // pollingReadOnly: a cabled mouse keeps its rate in its onboard profile
            if !ms.direct || profile_rate_writable(format) {
                polling = rates;
            }
        }
        Ok(OnMouse {
            model: None,
            online: true,
            battery_percent: bat.and_then(|b| b.percent),
            charging: bat.and_then(|b| b.charging),
            dpi: Some(dpi),
            polling_hz,
            lift_off: None,
            wireless: Some(!wired),
            caps: Caps { dpi: dpi_caps(&options), polling, lift_off: Vec::new(), battery: bat.is_some() },
        })
    }

    /// OpenMouse's setDpi / setPollingRate. A mouse this write switched to host mode is put back into onboard mode
    /// (OpenMouse's setOnboardMode("Onboard")) when the write then fails - never left half-changed (Order 104 review).
    fn write(&self, m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        forget_learned();
        let mut switched = None;
        let r = self.write_setting(m, port, s, &mut switched);
        if let (Err(_), Some(idx)) = (&r, switched) {
            if let Ok(Some(ms)) = find_mouse(port, m) {
                let _ = exchange(port, &set_onboard_mode(ms.dev, idx), REQUEST_TIMEOUT_MS);
            }
        }
        r
    }
}

impl Logitech {
    /// The write itself, after finding the mouse and what the check needs (reads); `switched` = the 0x8100 index when it
    /// switched the mouse to host mode.
    fn write_setting(&self, m: &Matched, port: &mut dyn Port, s: Setting, switched: &mut Option<u8>) -> Result<Changed> {
        let ms = find_mouse(port, m)?.ok_or_else(asleep)?;
        let dev = ms.dev;
        match s {
            Setting::Dpi(dpi) => {
                if !dpi_options(port, &ms)?.contains(&dpi) {
                    return Err(Error::range("DPI", format!("{dpi} DPI is not advertised by this mouse")));
                }
                if ms.legacy_dpi {
                    // setLegacyDpi: host control, set, read back
                    let before = decode_legacy_dpi(&call(port, &legacy_read_dpi(dev, ms.dpi))?);
                    *switched = ensure_host_control(port, &ms)?;
                    call(port, &legacy_set_dpi(dev, ms.dpi, dpi as u16))?;
                    let now = decode_legacy_dpi(&call(port, &legacy_read_dpi(dev, ms.dpi))?);
                    if now != dpi {
                        return Err(Error::BadAnswer(format!("The mouse kept {now} DPI instead of {dpi} DPI.")));
                    }
                    return Ok(Changed { now, before: Some(before) });
                }
                // setDpi: host control, getFeature(0x2202), current (keeps its lift-off byte), set x = y, read back. The
                // user's DPI for undo is read BEFORE the host-mode switch (which may change the active DPI)
                let before = decode_dpi_configuration(&call(port, &read_dpi(dev, ms.dpi))?).0;
                *switched = ensure_host_control(port, &ms)?;
                let idx = feature(port, dev, FEAT_EXTENDED_DPI)?;
                if idx == 0 {
                    return Err(Error::BadAnswer("the mouse no longer names its DPI feature".into()));
                }
                let (_, _, lod) = decode_dpi_configuration(&call(port, &read_dpi(dev, idx))?);
                call(port, &set_dpi(dev, idx, dpi as u16, dpi as u16, lod))?;
                let (x, y, _) = decode_dpi_configuration(&call(port, &read_dpi(dev, idx))?);
                if x != dpi || y != dpi {
                    return Err(Error::BadAnswer(format!("The mouse kept {x}/{y} DPI instead of {dpi}/{dpi} DPI.")));
                }
                Ok(Changed { now: dpi, before: Some(before) })
            }
            Setting::LiftOff(_) => Err(Error::range("lift-off distance", "Logitech names lift-off levels only, not millimetres - set it in Logitech's own app".to_string())),
            Setting::Polling(hz) => {
                let rf = rate_feature(port, dev)?.ok_or_else(|| Error::range("polling rate", "This mouse does not expose a HID++ polling-rate control.".to_string()))?;
                let format = profile_format(port, dev)?;
                if ms.direct && !profile_rate_writable(format) {
                    return Err(Error::range("polling rate", "This mouse stores its polling rate in the onboard profile, so it is read without changing it.".to_string()));
                }
                let (wired, usb) = link(port, &ms, m.config.pid)?;
                if let Some(c) = rate_ceiling(format, usb, wired) {
                    if hz > c {
                        return Err(Error::range("polling rate", format!("This connection supports up to {c} Hz.")));
                    }
                }
                if !supported_rates(port, dev, rf)?.contains(&hz) {
                    return Err(Error::range("polling rate", format!("{hz} Hz is not a rate this mouse offers")));
                }
                let before = current_rate(port, dev, rf)?;
                // a cabled mouse that rejects the live write wants it in its onboard profile - OpenMouse then writes the
                // profile flash, which this app never does
                let rejected = |hidpp10: bool, code: u8| {
                    let w = error_words(hidpp10, code);
                    Error::BadAnswer(if ms.direct { format!("{w} This mouse takes its polling rate only into its onboard profile, which this app does not write.") } else { w })
                };
                let (idx, legacy) = rf;
                if legacy {
                    // setLegacyReportRate: host control, set (ms), read back
                    let req = legacy_set_rate(dev, idx, hz).ok_or_else(|| Error::range("polling rate", "Unsupported polling rate.".to_string()))?;
                    *switched = ensure_host_control(port, &ms)?;
                    if let Reply::Rejected { hidpp10, code } = exchange(port, &req, REQUEST_TIMEOUT_MS)? {
                        return Err(rejected(hidpp10, code));
                    }
                    let now = current_rate(port, dev, rf)?;
                    if now != Some(hz) {
                        return Err(Error::BadAnswer(format!("The mouse kept {now:?} Hz instead of {hz} Hz.")));
                    }
                    return Ok(Changed { now: hz, before });
                }
                // setExtendedReportRate: host control, set (rate index), then the mouse's "rate changed" notification
                let req = set_rate(dev, idx, hz).ok_or_else(|| Error::range("polling rate", "Unsupported polling rate.".to_string()))?;
                *switched = ensure_host_control(port, &ms)?;
                let mut prefixes = answer_prefixes(&req);
                // the notification instead of the plain acknowledgement: [0x11, dev, feature, 0x00, 0x01, rate index]
                prefixes[0] = vec![LONG_REPORT_ID, dev, idx, 0x00, 0x01, req[4]];
                match send(port, &req, prefixes, RATE_CONFIRM_TIMEOUT_MS) {
                    Ok(r) => {
                        if let Answer::Rejected { hidpp10, code } = classify(&r, dev, idx, req[3]) {
                            return Err(rejected(hidpp10, code));
                        }
                        if decode_rate_notification(&r, dev, idx) != Some(hz) {
                            return Err(Error::BadAnswer("The mouse acknowledged the rate write but did not confirm the new active rate.".into()));
                        }
                    }
                    // Windows: a notification can come as a short report, on the other collection - ask the rate instead
                    Err(Error::BadAnswer(_)) => {
                        if current_rate(port, dev, rf)? != Some(hz) {
                            return Err(Error::BadAnswer("The mouse acknowledged the rate write but did not confirm the new active rate.".into()));
                        }
                    }
                    Err(e) => return Err(e),
                }
                Ok(Changed { now: hz, before })
            }
        }
    }
}

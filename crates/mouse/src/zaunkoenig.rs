//! Zaunkoenig (M3K, M2K) - ported from OpenMouse (`crate::family`: AGPL-3.0-or-later, @openmouse/protocol 0.40.1), DPI /
//! polling / lift-off only (the mouse is wired and has no battery):
//!
//! - the codec: `zaunkoenig/index.js` (the public "Zaunkoenfigurator" protocol: every setting lives in ONE little-endian
//!   16-bit word) - [`decode_word`] / [`encode_word`];
//! - the client: `drivers/zaunkoenig/hid.js` (ZaunkoenigHidClient: readStatus / setDpi / setPollingRate /
//!   setLiftOffDistance, all through `update`: read the word, change one field, write the word if it differs, read it again
//!   and refuse when the mouse did not keep it).
//!
//! Interface: the vendor collection (usage page 0xFF00) that carries feature report 2 (the firmware text) and feature
//! report 3 (the settings word). Windows tells us only the collection's biggest feature report, so the collection is the
//! 0xFF00 one whose feature reports are at least report id + 4 bytes ([`CONFIG_LEN`]).
//!
//! Packets (Windows' buffer; byte 0 is the report id, which OpenMouse's `sendFeatureReport(id, data)` keeps apart):
//! - firmware text, a read: `HidD_GetFeature(2)` -> `[2, ascii..., 0]` (OpenMouse refuses firmware that does not contain
//!   "parawizard new v0.8");
//! - settings word, a read: `HidD_GetFeature(3)` -> `[3, word lo, word hi, ..]`;
//! - settings word, the write: `HidD_SetFeature([3, word lo, word hi, 0, 0, zeros up to the collection's feature report size])`
//!   (OpenMouse: a 4-byte payload, which WebHID pads to the report size; Windows needs the full size; the two control bytes
//!   are the control word, 0 = apply; 0xFFFF = factory reset, which this file never builds).
//!
//! The word: bits 0-8 DPI/step - 1 · 9-10 lift-off (M3K: mm - 1, M2K: mm) · 11-12 polling code (8000, 4000, 2000, 1000 Hz =
//! 0..3) · 13 USB high speed · 14 angle snapping · 15 primary button right. A read-modify-write of one word whose every
//! field is read back first, so nothing we do not show is lost (angle snapping and the primary button are copied through).
//!
//! Left out on purpose: angle snapping, USB speed (changed only as OpenMouse's setPollingRate does: a rate above 1000 Hz
//! switches the mouse to high speed), primary button, and the factory reset (`zaunkoenigBuildFactoryResetPayload`).
//! Lift-off: OpenMouse's `liftOffDistanceMm` is a real millimetre figure (1, 2, 3 mm; the M2K 2 and 3), so it is offered.
//! Proven only against OpenMouse's bytes (tests); never run on a real unit.

use crate::error::{Error, Result};
use crate::family::{Caps, Changed, DpiCaps, Family, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x0483;
pub const M3K: u16 = 0xA462;
pub const M2K: u16 = 0xA3CF;
pub const USAGE_PAGE: u16 = 0xFF00;
pub const VERSION_REPORT_ID: u8 = 2;
pub const CONFIG_REPORT_ID: u8 = 3;
/// report id + the 4-byte config payload
pub const CONFIG_LEN: usize = 5;
pub const MINIMUM_FIRMWARE_MARKER: &str = "parawizard new v0.8";
/// the polling rates OpenMouse offers (`POLLING_RATES` in the client)
pub const POLLING: [u32; 4] = [1000, 2000, 4000, 8000];

/// One model of the table (`ZAUNKOENIG_DEVICES`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Device {
    pub pid: u16,
    /// name on OpenMouse's SUPPORTED list
    pub model: &'static str,
    pub dpi_step: u32,
    pub dpi_max: u32,
    /// the lift-off distances in mm the firmware accepts
    pub lift_off_mm: &'static [u32],
}

/// OpenMouse's SUPPORTED list: "M3K" 0xa462 and "M2K" 0xa3cf (devices.js: step 50 / max 20 000 / 1-3 mm, step 100 / max
/// 12 000 / 2-3 mm).
pub const DEVICES: [Device; 2] = [
    Device { pid: M3K, model: "Zaunkoenig M3K", dpi_step: 50, dpi_max: 20_000, lift_off_mm: &[1, 2, 3] },
    Device { pid: M2K, model: "Zaunkoenig M2K", dpi_step: 100, dpi_max: 12_000, lift_off_mm: &[2, 3] },
];

pub fn device(pid: u16) -> Option<&'static Device> {
    DEVICES.iter().find(|d| d.pid == pid)
}

/// The settings word, decoded (`zaunkoenigDecodeConfigWord`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// USB high speed (bit 13); full speed always reports at 1000 Hz
    pub high_speed: bool,
    pub polling_hz: u32,
    /// the 2-bit polling code kept in the word (8000, 4000, 2000, 1000 Hz = 0..3)
    pub polling_code: u8,
    pub lift_off_mm: u32,
    pub angle_snapping: bool,
    pub primary_right: bool,
    pub dpi: u32,
}

/// `pollingCode`: 8000 -> 0, 4000 -> 1, 2000 -> 2, 1000 -> 3.
pub fn polling_code(hz: u32) -> u8 {
    match hz {
        8000 => 0,
        4000 => 1,
        2000 => 2,
        _ => 3,
    }
}

fn code_hz(code: u8) -> u32 {
    [8000, 4000, 2000, 1000][(code & 3) as usize]
}

/// `zaunkoenigDecodeConfigWord`.
pub fn decode_word(word: u16, d: &Device) -> Result<Config> {
    let w = word as u32;
    let code = ((w >> 11) & 3) as u8;
    let raw_lod = (w >> 9) & 3;
    let mm = if d.pid == M2K { raw_lod } else { raw_lod + 1 };
    if !d.lift_off_mm.contains(&mm) {
        return Err(Error::BadAnswer(format!("{} reported unsupported lift-off code {raw_lod}", d.model)));
    }
    let high_speed = w & 0x2000 != 0;
    Ok(Config {
        high_speed,
        polling_hz: if high_speed { code_hz(code) } else { 1000 },
        polling_code: code,
        lift_off_mm: mm,
        angle_snapping: w & 0x4000 != 0,
        primary_right: w & 0x8000 != 0,
        dpi: ((w & 0x01FF) + 1) * d.dpi_step,
    })
}

/// `zaunkoenigEncodeConfigWord` (with every refusal it makes).
pub fn encode_word(c: &Config, d: &Device) -> Result<u16> {
    if c.dpi < d.dpi_step || c.dpi > d.dpi_max || !c.dpi.is_multiple_of(d.dpi_step) {
        return Err(Error::range("DPI", format!("{} DPI must be {}-{} in {} DPI steps", d.model, d.dpi_step, d.dpi_max, d.dpi_step)));
    }
    if !d.lift_off_mm.contains(&c.lift_off_mm) {
        return Err(Error::range("lift-off distance", format!("{} lift-off distance must be {} mm", d.model, d.lift_off_mm.iter().map(|m| m.to_string()).collect::<Vec<_>>().join(" or "))));
    }
    if !POLLING.contains(&c.polling_hz) {
        return Err(Error::range("polling rate", "Zaunkoenig polling rate must be 1000, 2000, 4000, or 8000 Hz"));
    }
    if c.polling_code > 3 {
        return Err(Error::range("polling rate", "Zaunkoenig polling-rate code must be an integer from 0 to 3"));
    }
    if !c.high_speed && c.polling_hz != 1000 {
        return Err(Error::range("polling rate", "USB Full-speed supports only 1000 Hz polling"));
    }
    // Full speed always reports 1 kHz, but the firmware keeps the interval bits for when high speed comes back
    let code = if c.high_speed { polling_code(c.polling_hz) } else { c.polling_code };
    let raw_lod = if d.pid == M2K { c.lift_off_mm } else { c.lift_off_mm - 1 };
    let mut word = ((c.dpi / d.dpi_step - 1) & 0x01FF) | ((raw_lod & 3) << 9) | ((code as u32 & 3) << 11);
    if c.high_speed {
        word |= 0x2000;
    }
    if c.angle_snapping {
        word |= 0x4000;
    }
    if c.primary_right {
        word |= 0x8000;
    }
    Ok(word as u16)
}

/// `zaunkoenigBuildConfigPayload` with the report id in front, as `HidD_SetFeature` wants it: `[3, lo, hi, 0, 0]`, zero-padded to
/// `len` (the collection's feature report size - Windows refuses a SetFeature shorter than that, and OpenMouse's WebHID pads the
/// same way). The control bytes 3-4 stay 0 (0 = apply; 0xFFFF would be the factory reset).
pub fn config_report(word: u16, len: usize) -> Vec<u8> {
    let mut v = vec![0u8; len.max(CONFIG_LEN)];
    v[0] = CONFIG_REPORT_ID;
    v[1] = word as u8;
    v[2] = (word >> 8) as u8;
    v
}

/// `zaunkoenigDecodeVersionReport`: the text after the report id up to the first 0, trimmed.
pub fn decode_version(report: &[u8]) -> String {
    let start = usize::from(report.first() == Some(&VERSION_REPORT_ID));
    let end = report.iter().skip(start).position(|b| *b == 0).map_or(report.len(), |p| p + start);
    report[start.min(end)..end].iter().map(|b| *b as char).collect::<String>().trim().to_string()
}

/// `zaunkoenigFirmwareIsSupported`.
pub fn firmware_is_supported(version: &str) -> bool {
    version.contains(MINIMUM_FIRMWARE_MARKER)
}

/// `zaunkoenigDecodeConfigReport`: the word is the two bytes after the report id.
pub fn decode_config_report(report: &[u8], d: &Device) -> Result<Config> {
    let start = usize::from(report.first() == Some(&CONFIG_REPORT_ID));
    if report.len() < start + 2 {
        return Err(Error::BadAnswer("Zaunkoenig configuration report is shorter than two bytes".into()));
    }
    decode_word(report[start] as u16 | ((report[start + 1] as u16) << 8), d)
}

/// What a model can do, as OpenMouse's client offers it.
pub fn caps(d: &Device) -> Caps {
    Caps { dpi: Some(DpiCaps::uniform(d.dpi_step, d.dpi_max, d.dpi_step)), polling: POLLING.to_vec(), lift_off: d.lift_off_mm.iter().map(|m| m * 10).collect(), battery: false }
}

/// OpenMouse's isSupported: Zaunkoenig's vendor id, one of the two product ids, and the control collection (usage page
/// 0xFF00 with the feature reports; see the header).
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID {
        return None;
    }
    let d = device(pid)?;
    let config = own.iter().find(|h| h.usage_page == USAGE_PAGE && h.feature_len as usize >= CONFIG_LEN)?;
    Some(Matched { family: Family::Zaunkoenig, model: Some(d.model), wireless: Some(false), config: (*config).clone(), variant: pid })
}

/// The allow-list for the read-only proof on a real mouse: NO frame passes. Both reads (report 2 = the firmware text, report 3 =
/// the settings word) are `HidD_GetFeature` ([`HidTransfer::FeatureIn`]: nothing is sent, and the transfer kind itself is let
/// through). The one-byte `[2]` / `[3]` that is the `out` of such a read must not pass as a SetFeature; the only frame this
/// family ever sends is the settings write.
pub fn read_request_allowed(_out: &[u8]) -> bool {
    false
}

/// The Zaunkoenig driver.
pub struct Zaunkoenig;

fn the_device(m: &Matched) -> Result<&'static Device> {
    device(m.variant).ok_or_else(|| Error::UnsupportedMouse(format!("Zaunkoenig variant {:#06x}", m.variant)))
}

/// `receiveFeatureReport(id)`: Windows' buffer starts with the report id.
fn get(port: &mut dyn Port, id: u8) -> Result<Vec<u8>> {
    let len = (port.info().feature_len as usize).max(CONFIG_LEN);
    port.exchange(&[id], &HidTransfer::FeatureIn { reply_id: id, reply_len: len, wait_ms: 0 })
}

/// The settings word as the mouse stores it (the two bytes behind the report id) and its decoding.
fn read_word(port: &mut dyn Port, d: &Device) -> Result<(u16, Config)> {
    let r = get(port, CONFIG_REPORT_ID)?;
    let c = decode_config_report(&r, d)?;
    let start = usize::from(r.first() == Some(&CONFIG_REPORT_ID));
    Ok((r[start] as u16 | ((r[start + 1] as u16) << 8), c))
}

impl crate::family::Driver for Zaunkoenig {
    /// OpenMouse's readStatus: the firmware text (refused when it is not "parawizard new v0.8"), then the settings word.
    fn read(&self, m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let d = the_device(m)?;
        let firmware = decode_version(&get(port, VERSION_REPORT_ID)?);
        if !firmware_is_supported(&firmware) {
            let shown = if firmware.is_empty() { "unknown" } else { firmware.as_str() };
            return Err(Error::UnsupportedMouse(format!("Unsupported Zaunkoenig firmware: {shown}. {MINIMUM_FIRMWARE_MARKER} is required.")));
        }
        let c = read_word(port, d)?.1;
        Ok(OnMouse { online: true, dpi: Some(c.dpi), polling_hz: Some(c.polling_hz), lift_off: Some(c.lift_off_mm * 10), wireless: Some(false), caps: caps(d), ..Default::default() })
    }

    /// OpenMouse's `update`: read the word, change one field, write it when it differs, read it again and refuse when the
    /// mouse did not keep it.
    fn write(&self, m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        let d = the_device(m)?;
        let (current_word, current) = read_word(port, d)?;
        let mut wanted = current;
        let before = match s {
            Setting::Dpi(v) => {
                wanted.dpi = v;
                current.dpi
            }
            Setting::Polling(hz) => {
                if !POLLING.contains(&hz) {
                    return Err(Error::range("polling rate", "Zaunkoenig polling rate must be 1000, 2000, 4000, or 8000 Hz"));
                }
                wanted.polling_hz = hz;
                wanted.polling_code = polling_code(hz);
                wanted.high_speed = hz > 1000 || current.high_speed;
                current.polling_hz
            }
            Setting::LiftOff(t) => {
                // OpenMouse's Low / Medium / High are 1 / 2 / 3 mm
                if !t.is_multiple_of(10) {
                    return Err(Error::range("lift-off distance", format!("{t} tenths of a mm: this mouse takes whole millimetres")));
                }
                wanted.lift_off_mm = t / 10;
                current.lift_off_mm * 10
            }
        };
        let word = match s {
            Setting::Dpi(_) => encode_word(&wanted, d)?,
            // a polling / lift-off change keeps the stored DPI bits exactly as they are, even when the saved DPI is above the
            // model's maximum (encode_word would refuse it with a DPI error that has nothing to do with this change)
            _ => {
                let valid_dpi = Config { dpi: d.dpi_step, ..wanted };
                (encode_word(&valid_dpi, d)? & !0x01FF) | (current_word & 0x01FF)
            }
        };
        if word != current_word {
            let len = port.info().feature_len as usize;
            port.exchange(&config_report(word, len), &HidTransfer::FeatureOut)?;
        }
        let (confirmed_word, confirmed) = read_word(port, d)?;
        if confirmed_word != word {
            return Err(Error::BadAnswer("The Zaunkoenig mouse did not retain the requested configuration".into()));
        }
        let now = match s {
            Setting::Dpi(_) => confirmed.dpi,
            Setting::Polling(_) => confirmed.polling_hz,
            Setting::LiftOff(_) => confirmed.lift_off_mm * 10,
        };
        Ok(Changed { now, before: Some(before) })
    }
}

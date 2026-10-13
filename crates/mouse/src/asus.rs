//! ASUS ROG Gladius II (P502, id 0x1845, wired) - 64-byte reports on the vendor config collection, ported from OpenMouse
//! (`crate::family`: AGPL-3.0-or-later): DPI and polling rate.
//!
//! Ported from `asus/index.js` (the codec: read requests, the settings / profile decoders, the DPI and polling write requests,
//! the save request, polling-rate table), `asus/devices.js` (the Gladius II's row: 2 DPI stages, 100-12000 in steps of 100,
//! `verified: true` - exercised on hardware by OpenMouse's author) and `drivers/asus/hid.js` (AsusHidClient: exchange,
//! readSettings, readProfile, setDpi / setDpiStageValue, setPollingRate, save). The other ASUS entries are not on OpenMouse's
//! SUPPORTED list (status "driver"/"pending") and the ROG Omni receiver (pairing, reboot, unpair) is not ported.
//!
//! Packet (OpenMouse sends `sendReport(0, 64 bytes)`; Windows puts report id 0 in front, so OpenMouse's byte i is our byte i + 1):
//! reads `12 04 00` (settings) and `12 00` (profile); writes `51 31 <field> 00 <value...>` (DPI: field = stage, value =
//! `dpi/100 - 1` u16 little-endian; polling: field 2 (= the stage count), value = the rate index in 125 / 250 / 500 / 1000 /
//! 2000 / 4000 / 8000) followed by the save `50 03`. An answer is the first input report that starts with the request's first
//! three bytes (two for the save); `FF AA` as the first two bytes is the firmware saying "asleep, out of range, or refusing".
//! Settings answer: DPI stage i at byte `4 + 2i` (u16 little-endian, `(v+1)*100`), polling rate byte at `4 + 2*stages`
//! (`& 7` = index), profile answer: byte 10 = profile (zero-based), byte 11 = active DPI stage (1-based).
//!
//! Safety: a polling change writes the whole rate byte, so it is refused when the stored byte carries anything above the three
//! rate bits (the "polling booster" nibble of newer firmware); nothing else is overwritten (a DPI write sets one stage).
//!
//! Left out on purpose: lift-off (`51 35 FF 00 FF <0|1>` - OpenMouse offers Low / High with no millimetre figure, and the
//! command "also resets the surface calibration"), debounce, angle snapping, lighting, profile switch (`50 02`), active-stage
//! switch (`51 31 09 00 <stage+1>`), the battery (the Gladius II has none), polling above 1000 Hz (needs the Omni booster).

use crate::error::{Error, Result};
use crate::family::{Caps, Changed, DpiCaps, Driver, Family, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x0B05;
pub const PID_GLADIUS_II: u16 = 0x1845;
pub const MODEL: &str = "ASUS ROG Gladius II P502";
pub const USAGE_PAGE: u16 = 0xFF01;
pub const USAGE: u16 = 0x0001;
pub const REPORT_ID: u8 = 0;
pub const PACKET_LEN: usize = 64;
/// Windows: the report id + the packet.
pub const REPORT_LEN: usize = PACKET_LEN + 1;
/// ASUS_POLLING_RATES (the index is the wire value); 2000 Hz and up need the Omni polling booster.
pub const POLLING_RATES: [u32; 7] = [125, 250, 500, 1000, 2000, 4000, 8000];
/// The rates this wired Gladius II can be set to (no booster).
pub const RATES: [u32; 4] = [125, 250, 500, 1000];
/// The Gladius II's DPI stages and range.
pub const DPI_STAGES: usize = 2;
pub const DPI_MIN: u32 = 100;
pub const DPI_MAX: u32 = 12000;
pub const DPI_STEP: u32 = 100;
const TIMEOUT_MS: u32 = 1000;

pub type Packet = [u8; PACKET_LEN];

// ---------------------------------------------------------------- packets

fn request(bytes: &[u8]) -> Packet {
    let mut p = [0u8; PACKET_LEN];
    p[..bytes.len()].copy_from_slice(bytes);
    p
}

/// asusReadSettingsRequest.
pub fn read_settings_request() -> Packet {
    request(&[0x12, 0x04, 0x00])
}

/// asusReadProfileRequest.
pub fn read_profile_request() -> Packet {
    request(&[0x12, 0x00])
}

/// asusSetDpiRequest(model, stage, dpi): `51 31 <stage> 00 <lo> <hi>` with `dpi/100 - 1`.
pub fn set_dpi_request(stage: u8, dpi: u32) -> Option<Packet> {
    if usize::from(stage) >= DPI_STAGES || !(DPI_MIN..=DPI_MAX).contains(&dpi) || !dpi.is_multiple_of(DPI_STEP) {
        return None;
    }
    let enc = dpi / DPI_STEP - 1;
    Some(request(&[0x51, 0x31, stage, 0x00, enc as u8, (enc >> 8) as u8]))
}

/// asusSetPollingRateRequest: `51 31 <stages> 00 <index>` (the rate field follows the DPI stages).
pub fn set_polling_request(hz: u32) -> Option<Packet> {
    let idx = POLLING_RATES.iter().position(|r| *r == hz)?;
    Some(request(&[0x51, 0x31, DPI_STAGES as u8, 0x00, idx as u8]))
}

/// asusSaveRequest: `50 03`.
pub fn save_request() -> Packet {
    request(&[0x50, 0x03])
}

/// What Windows sends: report id 0, the packet, zero-padded to the collection's output size.
pub fn wire(p: &Packet, len: usize) -> Vec<u8> {
    let mut v = vec![REPORT_ID];
    v.extend_from_slice(p);
    v.resize(len.max(REPORT_LEN), 0);
    v
}

// ---------------------------------------------------------------- answers

fn prefix_ok(data: &[u8], expected: &[u8], min_len: usize) -> bool {
    data.len() >= min_len && data.starts_with(expected)
}

/// settingOffset(model, field) = 4 + stages * 2 + field * 2.
fn setting_offset(field: usize) -> usize {
    4 + DPI_STAGES * 2 + field * 2
}

/// What asusDecodeSettings gives for the Gladius II: the DPI of each stage, the polling index byte and the polling rate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub dpi_stages: [u32; DPI_STAGES],
    /// the whole rate byte (the low three bits are the rate index)
    pub rate_byte: u8,
    pub polling_hz: u32,
}

/// asusDecodeSettings (no booster): `None` for an answer that is not the settings block; an unknown polling value is `None`
/// too (OpenMouse throws).
pub fn decode_settings(data: &[u8]) -> Option<Settings> {
    if !prefix_ok(data, &[0x12, 0x04, 0x00], setting_offset(2) + 1) {
        return None;
    }
    let rate_byte = data[setting_offset(0)];
    let polling_hz = POLLING_RATES.get((rate_byte & 0x07) as usize).copied()?;
    let mut dpi_stages = [0u32; DPI_STAGES];
    for (i, d) in dpi_stages.iter_mut().enumerate() {
        *d = (u16::from_le_bytes([data[4 + i * 2], data[5 + i * 2]]) as u32 + 1) * DPI_STEP;
    }
    Some(Settings { dpi_stages, rate_byte, polling_hz })
}

/// asusDecodeProfile: (onboard profile 1-based, active DPI stage 0-based); `None` = not a profile answer or a stage outside
/// 1..=2.
pub fn decode_profile(data: &[u8]) -> Option<(u8, usize)> {
    if !prefix_ok(data, &[0x12, 0x00, 0x00], 12) {
        return None;
    }
    let stage = data[11] as usize;
    // OpenMouse adds 1 in a JS number; a profile byte of 0xFF would overflow a u8, so it is no profile answer
    let profile = data[10].checked_add(1)?;
    (1..=DPI_STAGES).contains(&stage).then(|| (profile, stage - 1))
}

// ---------------------------------------------------------------- device table + allow-list

/// OpenMouse's isSupported: ASUS's vendor id, the Gladius II's product id, and a collection on usage page 0xFF01 / usage 1 that
/// carries 64-byte reports both ways.
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID || pid != PID_GLADIUS_II {
        return None;
    }
    let c = own.iter().find(|h| h.usage_page == USAGE_PAGE && h.usage == USAGE && h.input_len as usize >= REPORT_LEN && h.output_len as usize >= REPORT_LEN)?;
    Some(Matched { family: Family::Asus, model: Some(MODEL), wireless: Some(false), config: (*c).clone(), variant: pid })
}

/// The allow-list for the read-only proof on a real mouse: report id 0 carrying exactly the settings read (`12 04 00`) or the
/// profile read (`12 00`), zeros behind. Every write, the save, lighting, battery and lift-off requests are refused.
pub fn read_request_allowed(out: &[u8]) -> bool {
    if out.len() < REPORT_LEN || out[0] != REPORT_ID || out[REPORT_LEN..].iter().any(|b| *b != 0) {
        return false;
    }
    [read_settings_request(), read_profile_request()].iter().any(|r| r[..] == out[1..REPORT_LEN])
}

pub fn caps() -> Caps {
    Caps { dpi: Some(DpiCaps::uniform(DPI_MIN, DPI_MAX, DPI_STEP)), polling: RATES.to_vec(), lift_off: Vec::new(), battery: false }
}

// ---------------------------------------------------------------- the exchange

/// exchange(request, matchLength): the first input report that starts with the request's first `match_len` bytes, or the
/// `FF AA` refusal. Returns the 64 data bytes (behind the report id).
fn exchange(port: &mut dyn Port, p: &Packet, match_len: usize) -> Result<Vec<u8>> {
    let out_len = port.info().output_len as usize;
    let in_len = (port.info().input_len as usize).max(REPORT_LEN);
    let mut want = vec![REPORT_ID];
    want.extend_from_slice(&p[..match_len]);
    let how = HidTransfer::OutputThenMatch { reply_len: in_len, max_reads: 32, timeout_ms: TIMEOUT_MS, prefixes: vec![vec![REPORT_ID, 0xFF, 0xAA], want] };
    let a = port.exchange(&wire(p, out_len), &how)?;
    let data = a.get(1..).filter(|d| d.len() >= PACKET_LEN).ok_or_else(|| Error::BadAnswer("an ASUS answer shorter than 64 bytes".into()))?;
    if data[0] == 0xFF && data[1] == 0xAA {
        return Err(Error::BadAnswer("the ASUS mouse is asleep or refused the request".into()));
    }
    Ok(data[..PACKET_LEN].to_vec())
}

fn optional<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(Error::BadAnswer(_) | Error::Os { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

fn read_settings(port: &mut dyn Port) -> Result<Settings> {
    decode_settings(&exchange(port, &read_settings_request(), 3)?).ok_or_else(|| Error::BadAnswer("the ASUS settings answer was not understood".into()))
}

fn read_profile(port: &mut dyn Port) -> Result<(u8, usize)> {
    decode_profile(&exchange(port, &read_profile_request(), 3)?).ok_or_else(|| Error::BadAnswer("the ASUS profile answer was not understood".into()))
}

// ---------------------------------------------------------------- the driver

/// The ASUS driver.
pub struct Asus;

impl Driver for Asus {
    /// readStatus' settings and profile reads (no answer, or the `FF AA` refusal = offline).
    fn read(&self, _m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let mut r = OnMouse { wireless: Some(false), caps: caps(), ..Default::default() };
        let Some(s) = optional(read_settings(port))? else { return Ok(r) };
        let Some((_, stage)) = optional(read_profile(port))? else { return Ok(r) };
        r.online = true;
        r.dpi = Some(s.dpi_stages[stage]);
        r.polling_hz = Some(s.polling_hz);
        Ok(r)
    }

    /// setDpi: profile (the active stage), the stage's write, the save, the settings read-back. setPollingRate: the write, the
    /// save, the read-back.
    fn write(&self, _m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        match s {
            Setting::Dpi(d) => {
                if set_dpi_request(0, d).is_none() {
                    return Err(Error::range("DPI", format!("{d} DPI is not one this mouse can store (100 - 12000 in steps of 100)")));
                }
                let (_, stage) = read_profile(port)?;
                let before = read_settings(port)?.dpi_stages[stage];
                let req = set_dpi_request(stage as u8, d).expect("checked");
                exchange(port, &req, 3)?;
                exchange(port, &save_request(), 2)?;
                let now = read_settings(port)?.dpi_stages[stage];
                if now != d {
                    return Err(Error::BadAnswer(format!("the mouse kept {now} DPI instead of {d} DPI")));
                }
                Ok(Changed { now: d, before: Some(before) })
            }
            Setting::Polling(hz) => {
                let req = if RATES.contains(&hz) { set_polling_request(hz) } else { None };
                let req = req.ok_or_else(|| Error::range("polling rate", format!("{hz} Hz is not supported by this mouse on this connection")))?;
                let cur = read_settings(port)?;
                if cur.rate_byte & !0x07 != 0 {
                    return Err(Error::BadAnswer("the ASUS rate byte carries more than the rate (a polling booster setting) - nothing was written".into()));
                }
                exchange(port, &req, 3)?;
                exchange(port, &save_request(), 2)?;
                let now = read_settings(port)?.polling_hz;
                if now != hz {
                    return Err(Error::BadAnswer(format!("the mouse kept {now} Hz instead of {hz} Hz")));
                }
                Ok(Changed { now: hz, before: Some(cur.polling_hz) })
            }
            Setting::LiftOff(_) => Err(Error::range("lift-off distance", "this app can't set an ASUS mouse's lift-off (its levels have no millimetre figure, and the command resets the surface calibration)".to_string())),
        }
    }
}

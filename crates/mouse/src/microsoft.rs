//! Microsoft (Pro IntelliMouse 0x082A, Classic IntelliMouse 0x0823) - feature reports 0x24 (request) / 0x27 (answer), ported
//! from OpenMouse (`crate::family`: AGPL-3.0-or-later): DPI and polling rate only.
//!
//! Ported from `microsoft/index.js` (ids, report ids, property codes, collections) and `drivers/microsoft/hid.js`
//! (MicrosoftHidClient: readProperty / writeProperty / readDpi / setDpi / readPollingRate / setPollingRate).
//!
//! Packet (OpenMouse counts WITHOUT the report id; Windows puts the report id in front): a property request is the feature
//! report 0x24 whose payload is `[property, length-or-mode, data...]` zero-padded to the collection's feature report size
//! (Pro: 73 bytes with the id, Classic: 32). A READ is `[property, 0x01]` (mode 1); a WRITE is `[property, data length,
//! data...]`. The answer is the feature report 0x27 whose first payload byte echoes the property. OpenMouse sends the request
//! (HidD_SetFeature), waits 50 ms, then polls the answer every 10 ms for up to a second until the echo matches
//! ([`HidTransfer::Feature`] then [`HidTransfer::FeatureIn`]).
//!
//! Pro (usage page 0xFF07 / usage 0x0212): DPI 200-16000 in 50s (read 0x97, write 0x96 `[lo, hi]`, value little-endian at
//! answer byte 3), polling 125 / 500 / 1000 Hz (read 0x84, write 0x83 `[0x02 | 0x01 | 0x00]`, value at answer byte 3: 2 = 125,
//! 1 = 500, anything else = 1000), 250 ms after each write.
//!
//! Classic (usage page 0x000C / usage 1, the Consumer Control collection): HELD BACK - `matches()` does not claim 045E:0823, so
//! the Classic gets no control at all. OpenMouse's DPI write for it (400-3200 in 200s, 0x96 `[0x00, lo, hi]`, 50 ms after;
//! the 0x00 byte is unexplained) can only be sent blind: OpenMouse reads the Classic through an input report on that collection,
//! which WebHID on Windows blocks (its own comment) - it falls back to a made-up 1600 DPI - and Windows blocks it for this app
//! too (a system-owned collection: input reports can't be opened, only feature reports). With no read there is no "before" and
//! no read-back, so an unconfirmed write is not offered. The codec ([`classic_set_dpi`]) and its tests stay. NEEDS TRANSFER (not
//! built): "SetFeature 0x24 `[0x97, 0x01]` on the Consumer Control collection, then read the INPUT report 0x27 (ReadFile or
//! HidD_GetInputReport) until byte 0 = 0x97; DPI = u16 little-endian at answer bytes 4..6".
//!
//! Left out on purpose: lift-off (the Pro's two levels are shown by OpenMouse as "Low" / "High" with no millimetre figure;
//! a code comment guesses 2 mm / 3 mm - not a stated value, so not mapped), the Pro's tail-light colour (0xB2 / 0xB3), the
//! Classic's fixed 1000 Hz (OpenMouse marks polling read-only). OpenMouse sends no firmware or reset command to this family
//! and neither does this file.

use crate::error::{Error, Result};
use crate::family::{Caps, Changed, DpiCaps, Driver, Family, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x045E;
pub const PID_CLASSIC: u16 = 0x0823;
pub const PID_PRO: u16 = 0x082A;
pub const REPORT_ID_WRITE: u8 = 0x24;
pub const REPORT_ID_READ: u8 = 0x27;
pub const PROPERTY_DPI_WRITE: u8 = 0x96;
pub const PROPERTY_DPI_READ: u8 = 0x97;
pub const PROPERTY_POLLING_WRITE: u8 = 0x83;
pub const PROPERTY_POLLING_READ: u8 = 0x84;

/// getWriteLength(): the feature report size with its id.
pub const PRO_REPORT_LEN: usize = 73;
pub const CLASSIC_REPORT_LEN: usize = 32;

/// The two models, as OpenMouse's SUPPORTED list names them.
pub const PRODUCTS: [(u16, &str); 2] = [(PID_PRO, "Microsoft Pro Intellimouse"), (PID_CLASSIC, "Microsoft Classic Intellimouse")];

/// The Pro's polling rates (`supportedPollingRates`).
pub const PRO_RATES: [u32; 3] = [125, 500, 1000];

fn is_pro(pid: u16) -> bool {
    pid == PID_PRO
}

/// The collection OpenMouse's isSupported looks for: Pro 0xFF07 / 0x0212, Classic 0x000C / 0x0001.
fn collection(pid: u16) -> (u16, u16, usize) {
    if is_pro(pid) {
        (0xFF07, 0x0212, PRO_REPORT_LEN)
    } else {
        (0x000C, 0x0001, CLASSIC_REPORT_LEN)
    }
}

/// OpenMouse's isSupported: Microsoft's vendor id, one of the two product ids, and that model's collection (with a feature
/// report big enough for the 0x24 request). The Classic (045E:0823) is NOT claimed: OpenMouse can only write its DPI blind
/// (held back, see the header), so it gets no control.
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID {
        return None;
    }
    let model = PRODUCTS.iter().find(|p| p.0 == pid)?.1;
    if !is_pro(pid) {
        return None;
    }
    let (page, usage, len) = collection(pid);
    let c = own.iter().find(|h| h.usage_page == page && h.usage == usage && h.feature_len as usize >= len)?;
    Some(Matched { family: Family::Microsoft, model: Some(model), wireless: Some(false), config: (*c).clone(), variant: pid })
}

pub fn caps(pid: u16) -> Caps {
    if is_pro(pid) {
        Caps { dpi: Some(DpiCaps::uniform(200, 16000, 50)), polling: PRO_RATES.to_vec(), lift_off: Vec::new(), battery: false }
    } else {
        Caps { dpi: Some(DpiCaps::uniform(400, 3200, 200)), polling: Vec::new(), lift_off: Vec::new(), battery: false }
    }
}

// ---------------------------------------------------------------- the requests

/// The feature report Windows sends: report id 0x24, `[property, mode_or_length, data...]`, zero-padded to `len`.
fn request(property: u8, mode: u8, data: &[u8], len: usize) -> Vec<u8> {
    let mut v = vec![0u8; len];
    v[0] = REPORT_ID_WRITE;
    v[1] = property;
    v[2] = mode;
    v[3..3 + data.len()].copy_from_slice(data);
    v
}

/// readProperty's request: `[property, 0x01]`.
pub fn read_request(property: u8, len: usize) -> Vec<u8> {
    request(property, 0x01, &[], len)
}

/// Pro setDpi: `[0x96, 2, lo, hi]`.
pub fn pro_set_dpi(dpi: u32, len: usize) -> Vec<u8> {
    request(PROPERTY_DPI_WRITE, 2, &[dpi as u8, (dpi >> 8) as u8], len)
}

/// Classic setDpi: `[0x96, 3, 0x00, lo, hi]`.
pub fn classic_set_dpi(dpi: u32, len: usize) -> Vec<u8> {
    request(PROPERTY_DPI_WRITE, 3, &[0x00, dpi as u8, (dpi >> 8) as u8], len)
}

/// Pro setPollingRate: `[0x83, 1, val]` with 125 → 2, 500 → 1, 1000 → 0.
pub fn pro_set_polling(hz: u32, len: usize) -> Option<Vec<u8>> {
    let val = match hz {
        125 => 0x02,
        500 => 0x01,
        1000 => 0x00,
        _ => return None,
    };
    Some(request(PROPERTY_POLLING_WRITE, 1, &[val], len))
}

/// The Pro's polling code → Hz (readPollingRate): 2 = 125, 1 = 500, anything else 1000.
pub fn pro_polling_hz(code: u8) -> u32 {
    match code {
        0x02 => 125,
        0x01 => 500,
        _ => 1000,
    }
}

/// The Pro's answer to a property read (the payload behind the report id) → its value bytes: DPI = u16 little-endian at
/// payload byte 3.
pub fn pro_dpi(payload: &[u8]) -> Option<u32> {
    (payload.len() >= 5).then(|| u16::from_le_bytes([payload[3], payload[4]]) as u32)
}

/// The Pro's polling answer: payload byte 3.
pub fn pro_polling(payload: &[u8]) -> Option<u32> {
    payload.get(3).map(|c| pro_polling_hz(*c))
}

/// The allow-list for the read-only proof on a real mouse: a feature report 0x24 carrying exactly the Pro's DPI read (0x97)
/// or polling read (0x84) - `[property, 0x01]` and zeros behind. The DPI and polling writes (0x96, 0x83), the lighting and
/// every other property are refused.
pub fn read_request_allowed(out: &[u8]) -> bool {
    out.len() >= PRO_REPORT_LEN.min(CLASSIC_REPORT_LEN)
        && out[0] == REPORT_ID_WRITE
        && [PROPERTY_DPI_READ, PROPERTY_POLLING_READ].contains(&out[1])
        && out[2] == 0x01
        && out[3..].iter().all(|b| *b == 0)
}

// ---------------------------------------------------------------- the exchange

/// readProperty (Pro): send the request, wait 50 ms, then poll the answer every 10 ms (up to a second) until its first payload
/// byte is the property. Returns the payload (behind the report id). A wrong echo / "not ready" answer is polled past
/// (OpenMouse ignores it); a failure of the request itself (the first transfer) is returned, because then nothing was asked and
/// report 0x27 may still hold an older answer; a failing fetch later ends the polling at once (each can take up to a second,
/// OpenMouse's whole wait is one second); a mouse that is gone fails.
fn read_property(port: &mut dyn Port, property: u8) -> Result<Vec<u8>> {
    let len = (port.info().feature_len as usize).max(PRO_REPORT_LEN);
    let out = read_request(property, len);
    for i in 0..96 {
        let how = if i == 0 {
            HidTransfer::Feature { reply_id: REPORT_ID_READ, reply_len: len, wait_ms: 50 }
        } else {
            HidTransfer::FeatureIn { reply_id: REPORT_ID_READ, reply_len: len, wait_ms: 10 }
        };
        match port.exchange(&out, &how) {
            Ok(a) if a.first() == Some(&REPORT_ID_READ) && a.get(1) == Some(&property) => return Ok(a[1..].to_vec()),
            Ok(_) | Err(Error::BadAnswer(_)) => {}
            Err(e @ Error::Os { .. }) if i == 0 => return Err(e),
            Err(Error::Os { .. }) => break,
            Err(e) => return Err(e),
        }
    }
    Err(Error::BadAnswer(format!("the mouse did not answer property {property:#04x} in time")))
}

fn optional<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(Error::BadAnswer(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------- the driver

/// The Microsoft driver (`Matched.variant` is the product id).
pub struct Microsoft;

impl Driver for Microsoft {
    /// Pro: readStatus' DPI and polling reads (no answer = offline). Classic: nothing can be read (see the header) - the mouse
    /// is listed, so it is there.
    fn read(&self, m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let mut r = OnMouse { wireless: Some(false), caps: caps(m.variant), ..Default::default() };
        if !is_pro(m.variant) {
            r.online = true;
            return Ok(r);
        }
        let Some(d) = optional(read_property(port, PROPERTY_DPI_READ))? else { return Ok(r) };
        r.online = true;
        r.dpi = pro_dpi(&d);
        r.polling_hz = optional(read_property(port, PROPERTY_POLLING_READ))?.and_then(|p| pro_polling(&p));
        Ok(r)
    }

    fn write(&self, m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        let pro = is_pro(m.variant);
        let len = (port.info().feature_len as usize).max(if pro { PRO_REPORT_LEN } else { CLASSIC_REPORT_LEN });
        match s {
            Setting::Dpi(d) => {
                if !caps(m.variant).dpi.is_some_and(|c| c.allows(d)) {
                    let (lo, hi, step) = if pro { (200, 16000, 50) } else { (400, 3200, 200) };
                    return Err(Error::range("DPI", format!("{d} DPI is not one this mouse can store ({lo} - {hi} in steps of {step})")));
                }
                if !pro {
                    // held back: the Classic can't be read, so a write would be blind (matches() never claims it either)
                    return Err(Error::UnsupportedMouse("the Microsoft Classic IntelliMouse's DPI can only be written blind - held back".into()));
                }
                let before = pro_dpi(&read_property(port, PROPERTY_DPI_READ)?);
                port.exchange(&pro_set_dpi(d, len), &HidTransfer::FeatureOut)?;
                port.pause_ms(250);
                let now = pro_dpi(&read_property(port, PROPERTY_DPI_READ)?);
                if now != Some(d) {
                    return Err(Error::BadAnswer(format!("the mouse kept {now:?} DPI instead of {d}")));
                }
                Ok(Changed { now: d, before })
            }
            Setting::Polling(hz) => {
                let set = if pro { pro_set_polling(hz, len) } else { None };
                let set = set.ok_or_else(|| Error::range("polling rate", format!("{hz} Hz can't be set on this mouse")))?;
                let before = pro_polling(&read_property(port, PROPERTY_POLLING_READ)?);
                port.exchange(&set, &HidTransfer::FeatureOut)?;
                port.pause_ms(250);
                let now = pro_polling(&read_property(port, PROPERTY_POLLING_READ)?);
                if now != Some(hz) {
                    return Err(Error::BadAnswer(format!("the mouse kept {now:?} Hz instead of {hz} Hz")));
                }
                Ok(Changed { now: hz, before })
            }
            Setting::LiftOff(_) => Err(Error::range("lift-off distance", "this app can't set a Microsoft mouse's lift-off (its levels have no millimetre figure)".to_string())),
        }
    }
}

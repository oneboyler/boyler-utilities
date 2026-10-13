//! Corsair NIGHTSWORD RGB - 64-byte feature reports on report id 0, ported from OpenMouse (`crate::family`: AGPL-3.0-or-later,
//! @openmouse/protocol 0.40.1), DPI only (polling is READ here, not set):
//!
//! - the codec: `corsair/index.js` (the NXP-family protocol as the NIGHTSWORD RGB, firmware 3.41, speaks it: corsairEncode /
//!   corsairDecode / corsairIsEcho, the field and sub-command numbers, vendor id 0x1b1c, product id 0x1b5c);
//! - the client: `drivers/corsair/hid.js` (CorsairHidClient: isSupported, readStatus, setDpi, request / send).
//!
//! The only Corsair on OpenMouse's SUPPORTED list is the NIGHTSWORD RGB (0x1b5c); the other Corsair entries are "driver" /
//! "pr" and are not ported - including the Bragi client (`drivers/corsair/bragi-hid.js`, `corsair/bragi.js`: the IRONCLAW RGB
//! WIRELESS 0x1b4c and its receivers 0x1b66 / 0x1bdc, "pr", and OpenMouse's own client is read-only).
//!
//! Packet (64 bytes = OpenMouse's feature report id 0 payload; Windows puts the report id 0 in front, so OpenMouse's byte i is
//! our byte i + 1; the collection's feature length is 65): `[0]` command (0x0e GET, 0x07 SET) · `[1]` field (0x01 identity,
//! 0x13 mouse settings) · `[2]` sub-command (0x02 current DPI, 0xd0 + n DPI stage slot n) · `[3]` 0 = the LIVE profile ·
//! `[4..]` values. A GET is HidD_SetFeature, a 20 ms pause, HidD_GetFeature ([`HidTransfer::Feature`]); the answer must echo
//! bytes 0 - 3 of the request or the GET is sent again (3 tries). A SET gets no answer: HidD_SetFeature and a 20 ms pause
//! ([`HidTransfer::FeatureOut`]), then OpenMouse confirms it with the matching GET.
//!
//! Answers (OpenMouse's bytes): identity `[16]` = the polling interval in ms (rate = round(1000 / ms) Hz); current DPI `[4]`
//! slot, `[5..7]` X and `[7..9]` Y as BIG-endian words; stage slot n `[5..7]` X, `[7..9]` Y big-endian, `[9..12]` colour.
//! The SET payload carries DPI LITTLE-endian: `07 13 d0+slot 00 | flag (X != Y) | X lo hi | Y lo hi | r g b`.
//!
//! OpenMouse's setDpi rewrites the DPI of whichever slot is selected: it reads the slot, writes it back with the new DPI AND
//! the colour it just read (a write without the colour clears it), and reads the slot again. Held back (refused, nothing is
//! sent): a slot with separate X / Y values - setDpi would flatten Y to X. The writes address the live profile only; the mouse
//! forgets them when it loses power (OpenMouse's own note) - nothing stored is overwritten.
//!
//! Order 110 (A_110_02 scope): angle snapping - `MOUSE_SNAP` 0x04: GET `0e 13 04 00` (answer byte 4 != 0 = on), SET `07 13 04
//! 00 | on | 05` (corsairEncode.setSnap), then the GET must answer the new value. Read only after the DPI read answered, as
//! OpenMouse's readStatus does.
//!
//! Left out on purpose: polling-rate SET (OpenMouse's client marks polling read-only: the mouse re-enumerates after it),
//! lift-off (a raw 1 - 5 byte; OpenMouse's Low / Medium / High names are "this driver's own mapping" with no millimetre
//! figure, so a tenths-of-a-mm row would be made up), stage editor / count / colour / selection, the DPI-stage
//! mask, firmware, battery (a wired mouse). OpenMouse sends no firmware-update, DFU or reset command to a Corsair NXP mouse
//! and neither does this file. Nothing here ran on a real unit.

use crate::error::{Error, Result};
use crate::family::{check_extra, Caps, Change, Changed, DpiCaps, Driver, Extra, ExtraRow, Family, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x1b1c;
pub const NIGHTSWORD_PRODUCT_ID: u16 = 0x1b5c;
pub const MODEL: &str = "Corsair NIGHTSWORD RGB";
pub const USAGE_PAGE: u16 = 0xffc2;
/// The config collection (MI_00 also has an 0xffc2 collection, usage 3, that never answers).
pub const USAGE: u16 = 4;
pub const REPORT_ID: u8 = 0;
pub const PACKET_LEN: usize = 64;
/// Windows' feature report: report id 0 + the 64-byte packet.
pub const REPORT_LEN: usize = PACKET_LEN + 1;

pub const COMMAND_GET: u8 = 0x0e;
pub const COMMAND_SET: u8 = 0x07;
pub const FIELD_IDENT: u8 = 0x01;
pub const FIELD_MOUSE: u8 = 0x13;
pub const MOUSE_DPI: u8 = 0x02;
pub const MOUSE_DPI_PROFILE: u8 = 0xd0;
pub const LIVE_PROFILE: u8 = 0;
pub const STAGE_COUNT: u8 = 6;

/// Gap between sendFeatureReport and receiveFeatureReport (REPLY_DELAY_MS).
pub const REPLY_DELAY_MS: u32 = 20;
pub const REQUEST_ATTEMPTS: u32 = 3;

/// DPI_MIN, the product's dpiMax, DPI_OPTION_STEP: OpenMouse's setDpi accepts any whole DPI from 100 to 18000, its option
/// list (what the page shows) steps by 50.
pub const DPI_MIN: u32 = 100;
pub const DPI_MAX: u32 = 18_000;
pub const DPI_STEP: u32 = 50;

// ---------------------------------------------------------------- the codec

/// corsairPacket: the bytes at the front of a zero-padded 64-byte packet.
pub fn packet(bytes: &[u8]) -> [u8; PACKET_LEN] {
    let mut p = [0u8; PACKET_LEN];
    p[..bytes.len()].copy_from_slice(bytes);
    p
}

/// corsairEncode.ident: `0e 01 00`
pub fn get_ident() -> [u8; PACKET_LEN] {
    packet(&[COMMAND_GET, FIELD_IDENT, 0])
}
/// corsairEncode.dpiStage (the selected slot and its DPI): `0e 13 02 00`
pub fn get_current() -> [u8; PACKET_LEN] {
    packet(&[COMMAND_GET, FIELD_MOUSE, MOUSE_DPI, LIVE_PROFILE])
}
/// corsairEncode.stage(slot): `0e 13 d0|slot 00` (slot 0 - 5)
pub fn get_stage(slot: u8) -> [u8; PACKET_LEN] {
    packet(&[COMMAND_GET, FIELD_MOUSE, MOUSE_DPI_PROFILE | slot, LIVE_PROFILE])
}
/// corsairEncode.setStageDpi: `07 13 d0|slot 00 | X != Y | X lo hi | Y lo hi | r g b` (DPI little-endian).
pub fn set_stage_dpi(slot: u8, x: u16, y: u16, rgb: [u8; 3]) -> [u8; PACKET_LEN] {
    packet(&[COMMAND_SET, FIELD_MOUSE, MOUSE_DPI_PROFILE | slot, LIVE_PROFILE, (x != y) as u8, x as u8, (x >> 8) as u8, y as u8, (y >> 8) as u8, rgb[0], rgb[1], rgb[2]])
}

/// Order 110: `CORSAIR_MOUSE.snap`
pub const MOUSE_SNAP: u8 = 0x04;
/// Order 110: corsairEncode.snap: `0e 13 04 00`
pub fn get_snap() -> [u8; PACKET_LEN] {
    packet(&[COMMAND_GET, FIELD_MOUSE, MOUSE_SNAP, LIVE_PROFILE])
}
/// Order 110: corsairEncode.setSnap(enabled): `07 13 04 00 | on | 05`
pub fn set_snap(on: bool) -> [u8; PACKET_LEN] {
    packet(&[COMMAND_SET, FIELD_MOUSE, MOUSE_SNAP, LIVE_PROFILE, on as u8, 0x05])
}
/// Order 110: corsairDecode.snap: answer byte 4 != 0.
pub fn decode_snap(reply: &[u8]) -> Option<bool> {
    reply.get(4).map(|b| *b != 0)
}

/// The packet as Windows sends it: report id 0, the packet, zeros up to the collection's feature length.
pub fn report(p: &[u8; PACKET_LEN], feature_len: usize) -> Vec<u8> {
    let mut v = vec![0u8; feature_len.max(REPORT_LEN)];
    v[0] = REPORT_ID;
    v[1..REPORT_LEN].copy_from_slice(p);
    v
}

/// corsairIsEcho: the answer repeats the request's bytes 0 - 3.
pub fn is_echo(request: &[u8], reply: &[u8]) -> bool {
    reply.len() >= 4 && request.len() >= 4 && reply[..4] == request[..4]
}

/// corsairDecode.ident: the polling rate from the interval byte 16 (`None` = a short answer or interval 0).
pub fn decode_polling_hz(reply: &[u8]) -> Option<u32> {
    let ms = *reply.get(16)? as f64;
    (reply.len() >= 17 && ms > 0.0).then(|| (1000.0 / ms).round() as u32)
}

/// corsairDecode.dpiStage: (slot, X, Y), big-endian words.
pub fn decode_current(reply: &[u8]) -> Option<(u8, u16, u16)> {
    (reply.len() >= 9).then(|| (reply[4], u16::from_be_bytes([reply[5], reply[6]]), u16::from_be_bytes([reply[7], reply[8]])))
}

/// corsairDecode.stage: (X, Y, colour), big-endian words.
pub fn decode_stage(reply: &[u8]) -> Option<(u16, u16, [u8; 3])> {
    (reply.len() >= 12).then(|| (u16::from_be_bytes([reply[5], reply[6]]), u16::from_be_bytes([reply[7], reply[8]]), [reply[9], reply[10], reply[11]]))
}

/// What the Mouse page offers: DPI 100 - 18000 in OpenMouse's 50 steps; polling is read, not set; no lift-off (see the
/// header); no battery (wired).
pub fn caps() -> Caps {
    Caps { dpi: Some(DpiCaps::uniform(DPI_MIN, DPI_MAX, DPI_STEP)), polling: Vec::new(), lift_off: Vec::new(), battery: false }
}

/// The read-only proof's allow-list: report id 0 and a packet that is exactly one of the GET requests this file sends - the
/// identity, the current DPI, or a DPI stage slot 0 - 5 - zeros after it. Every SET (byte 0 = 0x07) is refused.
pub fn read_request_allowed(out: &[u8]) -> bool {
    if out.len() < REPORT_LEN || out[0] != REPORT_ID || out[REPORT_LEN..].iter().any(|b| *b != 0) {
        return false;
    }
    let p = &out[1..REPORT_LEN];
    p == get_ident() || p == get_current() || p == get_snap() || (0..STAGE_COUNT).any(|slot| p == get_stage(slot))
}

// ---------------------------------------------------------------- the device table

/// hasConfigCollection: usage page 0xffc2, usage 4, with a feature report on id 0 (Windows: a feature length of 65).
pub fn config_interface<'a>(own: &[&'a HidInfo]) -> Option<&'a HidInfo> {
    own.iter().find(|h| h.usage_page == USAGE_PAGE && h.usage == USAGE && h.feature_len as usize >= REPORT_LEN).copied()
}

/// OpenMouse's CorsairHidClient.isSupported: Corsair's vendor id, the NIGHTSWORD RGB's product id, the config collection.
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID || pid != NIGHTSWORD_PRODUCT_ID {
        return None;
    }
    let config = config_interface(own)?;
    Some(Matched { family: Family::Corsair, model: Some(MODEL), wireless: Some(false), config: config.clone(), variant: pid })
}

// ---------------------------------------------------------------- the exchange

/// One GET: send, wait, receive; the answer (without its report id) must echo the request, else the GET is sent again (3
/// tries). `Err(BadAnswer)` after the last.
fn get(port: &mut dyn Port, p: &[u8; PACKET_LEN]) -> Result<Vec<u8>> {
    let len = (port.info().feature_len as usize).max(REPORT_LEN);
    let out = report(p, len);
    let how = HidTransfer::Feature { reply_id: REPORT_ID, reply_len: len, wait_ms: REPLY_DELAY_MS };
    for _ in 0..REQUEST_ATTEMPTS {
        match port.exchange(&out, &how) {
            Ok(a) => {
                let data = a.get(1..).unwrap_or(&[]);
                if is_echo(p, data) {
                    return Ok(data.to_vec());
                }
            }
            Err(Error::BadAnswer(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Err(Error::BadAnswer(format!("the Corsair mouse did not answer command {:#04x}/{:#04x}", p[1], p[2])))
}

/// One SET: no answer, then the mouse gets its settle time.
fn set(port: &mut dyn Port, p: &[u8; PACKET_LEN]) -> Result<()> {
    let len = (port.info().feature_len as usize).max(REPORT_LEN);
    port.exchange(&report(p, len), &HidTransfer::FeatureOut)?;
    port.pause_ms(REPLY_DELAY_MS as u64);
    Ok(())
}

// ---------------------------------------------------------------- the driver

/// The Corsair driver (CorsairHidClient, NIGHTSWORD RGB).
pub struct Corsair;

impl Driver for Corsair {
    /// READ requests only: the identity (polling interval; the "is it answering?" call) and the current DPI. A mouse that does
    /// not answer the identity is offline; one that does not answer the DPI read just has no DPI shown.
    fn read(&self, _m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let mut r = OnMouse { wireless: Some(false), caps: caps(), ..Default::default() };
        let ident = match get(port, &get_ident()) {
            Ok(a) => a,
            Err(Error::BadAnswer(_)) => return Ok(r),
            Err(e) => return Err(e),
        };
        r.online = true;
        r.polling_hz = decode_polling_hz(&ident);
        let current = match get(port, &get_current()) {
            Ok(a) => Some(a),
            Err(Error::BadAnswer(_)) => None,
            Err(e) => return Err(e),
        };
        r.dpi = current.as_ref().and_then(|a| decode_current(a)).map(|c| c.1 as u32);
        // Order 110: angle snapping, asked only when the DPI read answered (readStatus: `dpi ? request(snap) : null`)
        // best-effort as in OpenMouse: any failure only leaves the row out
        let snap = if current.is_some() { get(port, &get_snap()).ok().and_then(|a| decode_snap(&a)) } else { None };
        r.extras = snap.map(|s| vec![ExtraRow::switch(Extra::AngleSnap, Some(s))]).unwrap_or_default();
        Ok(r)
    }

    /// Order 110: setAngleSnapping - the SET (no answer, 20 ms), then the GET must hold the new value.
    fn change(&self, _m: &Matched, port: &mut dyn Port, c: Change) -> Result<()> {
        let Change::Extra(e, v) = c;
        check_extra(&[ExtraRow::switch(Extra::AngleSnap, None)], e, v)?;
        set(port, &set_snap(v == 1))?;
        let now = decode_snap(&get(port, &get_snap())?);
        if now != Some(v == 1) {
            return Err(Error::BadAnswer(format!("the Corsair mouse kept angle snapping {now:?} instead of {}", v == 1)));
        }
        Ok(())
    }

    /// OpenMouse's setDpi: the selected slot (a GET), the slot's colour (a GET), the SET with the new DPI and that colour,
    /// the slot again (a GET) - which must hold the DPI and the colour. Polling and lift-off are not set.
    fn write(&self, _m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        let dpi = match s {
            Setting::Dpi(d) => d,
            Setting::Polling(_) => return Err(Error::range("polling rate", "a Corsair NIGHTSWORD's polling rate is read-only here: changing it makes the mouse re-enumerate".to_string())),
            Setting::LiftOff(t) => return Err(Error::range("lift-off distance", format!("{t} tenths of a mm: a Corsair's lift-off has no millimetre figure, so it is not set here"))),
        };
        if !caps().dpi.is_some_and(|d| d.allows(dpi)) {
            return Err(Error::range("DPI", format!("{dpi} DPI is not one this Corsair mouse offers (100 - 18000 in steps of 50)")));
        }
        let (slot, _, _) = decode_current(&get(port, &get_current())?).ok_or_else(|| Error::BadAnswer("a short Corsair DPI answer".into()))?;
        if slot >= STAGE_COUNT {
            return Err(Error::BadAnswer(format!("the Corsair mouse reported DPI slot {slot} (0 - 5 exist)")));
        }
        let (x, y, rgb) = decode_stage(&get(port, &get_stage(slot))?).ok_or_else(|| Error::BadAnswer("a short Corsair DPI-stage answer".into()))?;
        if x != y {
            return Err(Error::range("DPI", "the selected DPI stage has separate X and Y values; changing it here would overwrite Y - use iCUE".to_string()));
        }
        set(port, &set_stage_dpi(slot, dpi as u16, dpi as u16, rgb))?;
        let (nx, ny, nrgb) = decode_stage(&get(port, &get_stage(slot))?).ok_or_else(|| Error::BadAnswer("a short Corsair DPI-stage answer".into()))?;
        if nx as u32 != dpi || ny as u32 != dpi {
            return Err(Error::BadAnswer(format!("the Corsair mouse kept {nx} DPI on stage {slot} instead of {dpi}")));
        }
        if nrgb != rgb {
            return Err(Error::BadAnswer(format!("the Corsair mouse changed the colour of stage {slot} ({rgb:02x?} -> {nrgb:02x?})")));
        }
        Ok(Changed { now: dpi, before: Some(x as u32) })
    }
}

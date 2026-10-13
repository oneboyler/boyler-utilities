//! Incott (G23 V2 SE / Pro and its shared-id siblings) - feature report 9, ported from OpenMouse
//! (`crate::family`: AGPL-3.0-or-later), DPI / polling / lift-off only:
//!
//! - the codec: `incott/index.js` (INCOTT_CMD_* commands, incottEncodeSetDpi / SetPollingRate / SetLiftOff, the
//!   incottDecode* answers, incottFrameMatches, the polling and lift-off wire tables, the identity decode);
//! - the driver: `drivers/incott/hid.js` (IncottTransactionQueue: discard what is latched, send, wait 50 ms, read, up to 10
//!   times, accept a frame only when it echoes the request; IncottHidClient: readStatus / setDpi / setPollingRate /
//!   setLiftOffDistance). The device table is `INCOTT_PRODUCT_IDS`: 0x522c (the 2.4 GHz dongle) and 0x622c (the cable), both
//!   on OpenMouse's SUPPORTED list as "G23 V2 (SE / Pro)".
//!
//! Packet (Windows counts the report id): a request is `HidD_SetFeature([0x09, cmd, sub, 0, 0, 0, 0, 0, 0])` zero-padded to
//! the collection's feature length; the answer is `HidD_GetFeature(0x09)` - a shared buffer holding the LAST answer, so
//! OpenMouse first reads it once to throw away a stale one, then sends, waits 50 ms and reads until `[0]` = 9, `[1]` = the
//! command and (for the commands that echo it) `[2]` = the sub-command. Writes are feature reports with no answer; the
//! setting is confirmed by reading it back.
//!
//! The mouse's id is not its model: all six Incott models (Ghero, G23, G24, Zero 29, Zero 39, G23 V2) share the two product
//! ids, so the table names no model (`model: None`); `read` names it from the identity answer (`0x8f`). The models other
//! than G23 V2 are "likely" on OpenMouse's list, not supported - they match here only because their id is the same.
//!
//! Order 110 (A_110_02 scope): debounce 0-30 ms (`[05, 01, ms]`, read `[85, 01]`), Motion Sync / angle snapping / ripple
//! (`[04, sub 4 / 3 / 2, on]`, read `[84, sub]`) - setDebounceTime / setMotionSync / setAngleSnapping / setRippleControl: the
//! write, then the read it confirms with; a read that is not answered leaves its row out.
//!
//! Left out on purpose: battery (OpenMouse reads it from an UNSOLICITED input report the mouse sends only while it is being
//! used - there is no request to send, and `HidTransfer` has no passive read: NEEDS TRANSFER, see the report), the DPI stage
//! count / stage select / per-axis DPI, sleep, performance mode (HP / Corded / LP: not the Pulsar turbo), receiver LED, buttons, fire key,
//! macros (none is DPI / polling / lift-off / battery). OpenMouse sends no firmware, DFU or reset command to this mouse and
//! neither does this file.

use crate::error::{Error, Result};
use crate::family::{check_extra, Caps, Change, Changed, DpiCaps, Extra, ExtraRow, Family, Kind, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x093A;
/// The 2.4 GHz dongle.
pub const PID_DONGLE: u16 = 0x522C;
/// The cable (it enumerates only while plugged in).
pub const PID_WIRED: u16 = 0x622C;
pub const USAGE_PAGE: u16 = 0xFF05;
pub const REPORT_ID: u8 = 0x09;
/// INCOTT_PAYLOAD_LENGTH: a request's bytes after the report id.
pub const PAYLOAD_LEN: usize = 8;
/// INCOTT_RESPONSE_LENGTH: OpenMouse reads 64 bytes of an answer (a shorter one is zero-filled).
pub const RESPONSE_LEN: usize = 64;
/// The report id + the payload: the shortest feature report that carries a request.
pub const REQUEST_LEN: usize = 1 + PAYLOAD_LEN;

// the commands (index.js INCOTT_CMD_*)
pub const CMD_SET_POLLING: u8 = 0x01;
pub const CMD_SET_DPI: u8 = 0x02;
pub const CMD_SET_SENSOR: u8 = 0x04;
pub const CMD_QUERY_POLLING: u8 = 0x81;
pub const CMD_QUERY_DPI_STAGE_VALUE: u8 = 0x82;
pub const CMD_QUERY_DPI_STAGE: u8 = 0x83;
pub const CMD_QUERY_SENSOR: u8 = 0x84;
pub const CMD_QUERY_IDENTITY: u8 = 0x8F;
/// INCOTT_SUB_LOD
pub const SUB_LOD: u8 = 0x01;
/// Order 110: INCOTT_SUB_RIPPLE / ANGLE_SNAP / MOTION_SYNC (the 0x04 write / 0x84 read toggles), INCOTT_CMD_SET_TIMING /
/// QUERY_TIMING with INCOTT_SUB_DEBOUNCE.
pub const SUB_RIPPLE: u8 = 0x02;
pub const SUB_ANGLE_SNAP: u8 = 0x03;
pub const SUB_MOTION_SYNC: u8 = 0x04;
pub const CMD_SET_TIMING: u8 = 0x05;
pub const CMD_QUERY_TIMING: u8 = 0x85;
pub const SUB_DEBOUNCE: u8 = 0x01;
/// INCOTT_DEBOUNCE_MIN_MS / MAX_MS.
pub const DEBOUNCE_MAX_MS: i32 = 30;
/// INCOTT_DPI_STAGE_COUNT
pub const DPI_STAGE_COUNT: u8 = 6;
pub const DPI_MIN: u32 = 50;
pub const DPI_MAX: u32 = 45000;
/// INCOTT_DPI_MAX_PAW3395: the PAW3395 models stop here.
pub const DPI_MAX_PAW3395: u32 = 32000;
pub const DPI_STEP: u32 = 50;
/// INCOTT_POLLING_STEPS_HZ (wireless) and _WIRED (the cable only reaches 1000 Hz), ascending.
pub const POLLING_WIRELESS: [u32; 7] = [125, 250, 500, 1000, 2000, 4000, 8000];
pub const POLLING_WIRED: [u32; 4] = [125, 250, 500, 1000];
/// INCOTT_LOD_STEPS_TENTHS: 0.7, 1 and 2 mm (hardware-verified for 0.7 mm; 1 and 2 follow from the mapping).
pub const LIFT_OFF_TENTHS: [u32; 3] = [7, 10, 20];

/// OpenMouse's frame budget per request: 10 reads, 50 ms apart.
const ATTEMPTS: u32 = 10;
const SETTLE_MS: u32 = 50;

// ---------------------------------------------------------------- the codec

/// payload(...): the request's bytes after the report id, zero-padded to 8.
fn payload(values: &[u8]) -> [u8; PAYLOAD_LEN] {
    let mut p = [0u8; PAYLOAD_LEN];
    p[..values.len()].copy_from_slice(values);
    p
}

/// incottEncodeQuery(cmd, sub).
pub fn query(cmd: u8, sub: u8) -> [u8; PAYLOAD_LEN] {
    payload(&[cmd, sub])
}

/// The feature report Windows sends: report id 9, the 8-byte payload, zero-padded to the collection's feature length.
pub fn report(p: &[u8; PAYLOAD_LEN], len: usize) -> Vec<u8> {
    let mut v = vec![REPORT_ID];
    v.extend_from_slice(p);
    v.resize(len.max(REQUEST_LEN), 0);
    v
}

/// INCOTT_DPI_AXIS: both axes (the only one this file writes).
const AXIS_BOTH: u8 = 0;

/// incottEncodeSetDpi(stage, dpi, "both"): `[0x02, stage, wire lo, wire hi, 0, 0, 0, axis]`, wire = dpi / 50 - 1.
pub fn encode_set_dpi(stage: u8, dpi: u32) -> Option<[u8; PAYLOAD_LEN]> {
    if stage >= DPI_STAGE_COUNT || !dpi_valid(dpi) {
        return None;
    }
    let wire = dpi / DPI_STEP - 1;
    Some(payload(&[CMD_SET_DPI, stage, wire as u8, (wire >> 8) as u8, 0, 0, 0, AXIS_BOTH]))
}

/// incottValidateDpi: 50..=45000, a multiple of 50.
pub fn dpi_valid(dpi: u32) -> bool {
    (DPI_MIN..=DPI_MAX).contains(&dpi) && dpi.is_multiple_of(DPI_STEP)
}

/// INCOTT_POLLING_HZ_TO_WIRE.
pub fn polling_wire(hz: u32) -> Option<u8> {
    Some(match hz {
        1000 => 0,
        500 => 1,
        250 => 2,
        125 => 3,
        8000 => 4,
        4000 => 5,
        2000 => 6,
        _ => return None,
    })
}

/// INCOTT_POLLING_WIRE_TO_HZ.
pub fn polling_hz(wire: u8) -> Option<u32> {
    Some(match wire {
        0 => 1000,
        1 => 500,
        2 => 250,
        3 => 125,
        4 => 8000,
        5 => 4000,
        6 => 2000,
        _ => return None,
    })
}

/// incottEncodeSetPollingRate: `[0x01, wire]`.
pub fn encode_set_polling(hz: u32) -> Option<[u8; PAYLOAD_LEN]> {
    Some(payload(&[CMD_SET_POLLING, polling_wire(hz)?]))
}

/// INCOTT_LOD_TENTHS_TO_WIRE: 1 mm = 0, 2 mm = 1, 0.7 mm = 2.
pub fn lift_off_wire(tenths: u32) -> Option<u8> {
    Some(match tenths {
        10 => 0,
        20 => 1,
        7 => 2,
        _ => return None,
    })
}

/// incottEncodeSetLiftOff: `[0x04, 0x01, wire]`.
pub fn encode_set_lift_off(tenths: u32) -> Option<[u8; PAYLOAD_LEN]> {
    Some(payload(&[CMD_SET_SENSOR, SUB_LOD, lift_off_wire(tenths)?]))
}

/// Order 110: incottEncodeSetToggle(kind, on) = `[04, sub, on]`; incottEncodeSetDebounce(ms) = `[05, 01, ms]` (0-30).
pub fn encode_set_extra(e: Extra, v: i32) -> Option<[u8; PAYLOAD_LEN]> {
    let on = (v == 1) as u8;
    Some(match e {
        Extra::Debounce => payload(&[CMD_SET_TIMING, SUB_DEBOUNCE, u8::try_from(v).ok().filter(|v| *v as i32 <= DEBOUNCE_MAX_MS)?]),
        Extra::MotionSync => payload(&[CMD_SET_SENSOR, SUB_MOTION_SYNC, on]),
        Extra::AngleSnap => payload(&[CMD_SET_SENSOR, SUB_ANGLE_SNAP, on]),
        Extra::RippleControl => payload(&[CMD_SET_SENSOR, SUB_RIPPLE, on]),
        Extra::Turbo => return None,
    })
}

/// The read query (cmd, sub) of an extra.
fn extra_query(e: Extra) -> Option<(u8, u8)> {
    Some(match e {
        Extra::Debounce => (CMD_QUERY_TIMING, SUB_DEBOUNCE),
        Extra::MotionSync => (CMD_QUERY_SENSOR, SUB_MOTION_SYNC),
        Extra::AngleSnap => (CMD_QUERY_SENSOR, SUB_ANGLE_SNAP),
        Extra::RippleControl => (CMD_QUERY_SENSOR, SUB_RIPPLE),
        Extra::Turbo => return None,
    })
}

/// incottDecodeToggle (`[3]` 1 = on, 0 = off, else none) / incottDecodeDebounce (`[3]` 0-30, else none) of an answer to
/// `extra_query(e)`.
pub fn decode_extra(f: &[u8; RESPONSE_LEN], e: Extra) -> Option<i32> {
    let (cmd, sub) = extra_query(e)?;
    if !frame_matches(f, cmd, Some(sub)) {
        return None;
    }
    let v = f[3] as i32;
    let max = if e == Extra::Debounce { DEBOUNCE_MAX_MS } else { 1 };
    (0..=max).contains(&v).then_some(v)
}

/// The rows the card can have, in its order (the guard's rows; values none).
pub fn extra_rows() -> Vec<ExtraRow> {
    vec![
        ExtraRow::new(Extra::Debounce, Kind::Range { min: 0, max: DEBOUNCE_MAX_MS, step: 1, unit: "ms" }, None),
        ExtraRow::switch(Extra::MotionSync, None),
        ExtraRow::switch(Extra::AngleSnap, None),
        ExtraRow::switch(Extra::RippleControl, None),
    ]
}

/// The answer as OpenMouse holds it: at most 64 bytes, zero-filled to 64 (so every `frame[i]` of the JS reads 0 past the end).
pub fn frame_of(answer: &[u8]) -> [u8; RESPONSE_LEN] {
    let mut f = [0u8; RESPONSE_LEN];
    let n = answer.len().min(RESPONSE_LEN);
    f[..n].copy_from_slice(&answer[..n]);
    f
}

/// incottFrameMatches: report id 9, the echoed command, and (when asked) the echoed sub-command.
pub fn frame_matches(f: &[u8; RESPONSE_LEN], cmd: u8, sub: Option<u8>) -> bool {
    f[0] == REPORT_ID && f[1] == cmd && sub.is_none_or(|s| f[2] == s)
}

/// Do these queries echo their sub-command in byte 2 (SUB_ECHOING_QUERIES)? For polling, DPI-stage and identity it carries data.
fn echoes_sub(cmd: u8) -> bool {
    matches!(cmd, CMD_QUERY_DPI_STAGE_VALUE | CMD_QUERY_SENSOR | 0x85 | 0x86)
}

/// incottDecodeDpiCycle: (stage count, active stage) - `[2]` = count 1..=6, `[3]` = active, below the count.
pub fn decode_dpi_cycle(f: &[u8; RESPONSE_LEN]) -> Option<(u8, u8)> {
    if !frame_matches(f, CMD_QUERY_DPI_STAGE, None) {
        return None;
    }
    let (count, active) = (f[2], f[3]);
    ((1..=DPI_STAGE_COUNT).contains(&count) && active < count).then_some((count, active))
}

/// incottDecodeDpiStage: stage `stage`'s DPI = ([3] | [4] << 8) + 1) * 50, 50..=45000.
pub fn decode_dpi_stage(f: &[u8; RESPONSE_LEN], stage: u8) -> Option<u32> {
    if !frame_matches(f, CMD_QUERY_DPI_STAGE_VALUE, Some(stage)) {
        return None;
    }
    let dpi = ((f[3] as u32 | (f[4] as u32) << 8) + 1) * DPI_STEP;
    (DPI_MIN..=DPI_MAX).contains(&dpi).then_some(dpi)
}

/// incottDecodePollingRate: `[2]` is data here (not an echo).
pub fn decode_polling(f: &[u8; RESPONSE_LEN]) -> Option<u32> {
    if !frame_matches(f, CMD_QUERY_POLLING, None) {
        return None;
    }
    polling_hz(f[2])
}

/// incottDecodeLiftOffDirect: tenths of a mm from `[3]` (0 = 1 mm, 1 = 2 mm, 2 = 0.7 mm).
pub fn decode_lift_off(f: &[u8; RESPONSE_LEN]) -> Option<u32> {
    if !frame_matches(f, CMD_QUERY_SENSOR, Some(SUB_LOD)) {
        return None;
    }
    Some(match f[3] {
        0 => 10,
        1 => 20,
        2 => 7,
        _ => return None,
    })
}

/// What the identity answer (`0x8f`) says: the model's name and the sensor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// "G23V2", "G23V2 Pro", … (`None` = a model code outside OpenMouse's table)
    pub name: Option<String>,
    /// the PAW3950 reaches 45000 DPI, the PAW3395 stops at 32000 (`None` = unknown: the identity was not the well-formed shape)
    pub paw3950: Option<bool>,
}

/// incottDecodeIdentity: `None` when the frame is not an identity answer at all (a dead collection, a stale frame).
pub fn decode_identity(f: &[u8; RESPONSE_LEN]) -> Option<Identity> {
    if f[0] != REPORT_ID || f[1] != CMD_QUERY_IDENTITY {
        return None;
    }
    if f[2] != 0x01 {
        return Some(Identity { name: None, paw3950: None });
    }
    // INCOTT_MODEL_BY_CODE
    let model = match f[3] {
        0x01 => Some("Ghero"),
        0x02 => Some("G23"),
        0x03 => Some("G24"),
        0x06 => Some("Zero 29"),
        0x08 | 0x0E => Some("G23V2"),
        0x09 => Some("Zero 39"),
        _ => None,
    };
    // the sensor id 0xf1 in [5] or [6] = PAW3950 (the Pro), else PAW3395
    let pro = f[5] == 0xF1 || f[6] == 0xF1;
    Some(Identity { name: model.map(|m| if pro { format!("{m} Pro") } else { m.to_string() }), paw3950: Some(pro) })
}

// ---------------------------------------------------------------- the device table

/// One product id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Product {
    pub pid: u16,
    pub wireless: bool,
}

/// OpenMouse's SUPPORTED Incott ids (INCOTT_PRODUCT_IDS) - "G23 V2 (SE / Pro)". No row names a model: see the header.
pub const PRODUCTS: [Product; 2] = [Product { pid: PID_DONGLE, wireless: true }, Product { pid: PID_WIRED, wireless: false }];

pub fn product(pid: u16) -> Option<&'static Product> {
    PRODUCTS.iter().find(|p| p.pid == pid)
}

/// OpenMouse's isSupported: Incott's vendor id, one of the two product ids, and a collection on usage page 0xFF05 that
/// carries feature report 9 (Windows shows the report's length: at least the 9 bytes of a request). The cable shows TWO such
/// collections of which only one answers (OpenMouse's `open()` probes to find out); their descriptors look alike and which one
/// answers is UNCLEAR from here. So the pick must at least not depend on the order Windows lists them in: the lowest USB
/// interface number, then the larger feature report, then the lower usage, then the path. A wrong pick shows the cable mouse
/// offline (the identity read fails) - the writes stay safe, because every write is preceded by a read of the same collection.
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID {
        return None;
    }
    let p = product(pid)?;
    let config = own
        .iter()
        .filter(|h| h.usage_page == USAGE_PAGE && h.feature_len as usize >= REQUEST_LEN)
        .min_by(|a, b| (a.interface.unwrap_or(u8::MAX), std::cmp::Reverse(a.feature_len), a.usage, &a.path).cmp(&(b.interface.unwrap_or(u8::MAX), std::cmp::Reverse(b.feature_len), b.usage, &b.path)))?;
    Some(Matched { family: Family::Incott, model: None, wireless: Some(p.wireless), config: (*config).clone(), variant: pid })
}

/// The polling rates that can be SET on this connection.
pub fn polling_rates(wireless: bool) -> Vec<u32> {
    if wireless { POLLING_WIRELESS.to_vec() } else { POLLING_WIRED.to_vec() }
}

/// What the driver does for a mouse: DPI 50..(45000, or 32000 on a PAW3395) in 50s, the connection's polling rates, lift-off
/// 0.7 / 1 / 2 mm. No battery (see the header).
pub fn caps(wireless: bool, paw3950: Option<bool>) -> Caps {
    let max = if paw3950 == Some(false) { DPI_MAX_PAW3395 } else { DPI_MAX };
    Caps { dpi: Some(DpiCaps::uniform(DPI_MIN, max, DPI_STEP)), polling: polling_rates(wireless), lift_off: LIFT_OFF_TENTHS.to_vec(), battery: false }
}

/// The allow-list for the read-only proof on a real mouse (A_009_01): a feature report 9 carrying exactly one of the read
/// queries this file sends - identity, DPI cycle, one DPI stage's value (stages 0..=5), polling, the lift-off sensor query -
/// with nothing else set and only zero padding behind. Every write is refused.
pub fn read_request_allowed(out: &[u8]) -> bool {
    if out.len() < REQUEST_LEN || out[0] != REPORT_ID || out[REQUEST_LEN..].iter().any(|b| *b != 0) {
        return false;
    }
    let p = &out[1..REQUEST_LEN];
    let rest_zero = |from: usize| p[from..].iter().all(|b| *b == 0);
    match p[0] {
        CMD_QUERY_IDENTITY | CMD_QUERY_DPI_STAGE | CMD_QUERY_POLLING => p[1] == 0 && rest_zero(1),
        CMD_QUERY_DPI_STAGE_VALUE => p[1] < DPI_STAGE_COUNT && rest_zero(2),
        CMD_QUERY_SENSOR => matches!(p[1], SUB_LOD | SUB_RIPPLE | SUB_ANGLE_SNAP | SUB_MOTION_SYNC) && rest_zero(2),
        CMD_QUERY_TIMING => p[1] == SUB_DEBOUNCE && rest_zero(2),
        _ => false,
    }
}

// ---------------------------------------------------------------- the exchange

/// `request(...).catch(() => null)`: a mouse that does not answer (or the collection would not take the report) = `None`; a
/// mouse that is gone or a request the read-only layer refused still fails.
fn optional<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(Error::BadAnswer(_) | Error::Os { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// IncottTransactionQueue.request: throw away the latched answer, send, then read every 50 ms until a frame echoes the
/// request (10 tries). `None` = no matching frame (asleep, dead collection, a stale shared buffer).
fn request(port: &mut dyn Port, p: &[u8; PAYLOAD_LEN], cmd: u8, sub: Option<u8>) -> Result<Option<[u8; RESPONSE_LEN]>> {
    let len = (port.info().feature_len as usize).max(REQUEST_LEN);
    let out = report(p, len);
    // "Discard anything latched by a previous exchange" - a fetch only (nothing is sent), errors are ignored
    if let Err(e @ Error::MouseGone(_)) = port.exchange(&out, &HidTransfer::FeatureIn { reply_id: REPORT_ID, reply_len: len, wait_ms: 0 }) {
        return Err(e);
    }
    // the send alone (HidD_SetFeature): a collection that refuses it (the cable's dead twin: "Incorrect function") ends the
    // exchange, as OpenMouse's catch around the whole exchange does. Only the SEND failing stops it early: a failing read is
    // just a try.
    match port.exchange(&out, &HidTransfer::FeatureOut) {
        Ok(_) => {}
        Err(Error::BadAnswer(_) | Error::Os { .. }) => return Ok(None),
        Err(e) => return Err(e),
    }
    // then up to ATTEMPTS reads (HidD_GetFeature), SETTLE_MS apart; an OS error from one read is not the end (OpenMouse's
    // read() catches it and the loop goes on)
    let how = HidTransfer::FeatureIn { reply_id: REPORT_ID, reply_len: len, wait_ms: SETTLE_MS };
    for _ in 0..ATTEMPTS {
        match port.exchange(&out, &how) {
            Ok(a) => {
                let f = frame_of(&a);
                if frame_matches(&f, cmd, sub.filter(|_| echoes_sub(cmd))) {
                    return Ok(Some(f));
                }
            }
            Err(Error::BadAnswer(_) | Error::Os { .. }) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// A write: a feature report, no answer (IncottTransactionQueue.send - a failed write shows as a failed read-back).
fn write_report(port: &mut dyn Port, p: &[u8; PAYLOAD_LEN]) -> Result<()> {
    let len = (port.info().feature_len as usize).max(REQUEST_LEN);
    optional(port.exchange(&report(p, len), &HidTransfer::FeatureOut)).map(|_| ())
}

fn read_cycle(port: &mut dyn Port) -> Result<Option<(u8, u8)>> {
    Ok(request(port, &query(CMD_QUERY_DPI_STAGE, 0), CMD_QUERY_DPI_STAGE, None)?.and_then(|f| decode_dpi_cycle(&f)))
}

fn read_stage_dpi(port: &mut dyn Port, stage: u8) -> Result<Option<u32>> {
    Ok(request(port, &query(CMD_QUERY_DPI_STAGE_VALUE, stage), CMD_QUERY_DPI_STAGE_VALUE, Some(stage))?.and_then(|f| decode_dpi_stage(&f, stage)))
}

fn read_polling(port: &mut dyn Port) -> Result<Option<u32>> {
    Ok(request(port, &query(CMD_QUERY_POLLING, 0), CMD_QUERY_POLLING, None)?.and_then(|f| decode_polling(&f)))
}

fn read_lift_off(port: &mut dyn Port) -> Result<Option<u32>> {
    Ok(request(port, &query(CMD_QUERY_SENSOR, SUB_LOD), CMD_QUERY_SENSOR, Some(SUB_LOD))?.and_then(|f| decode_lift_off(&f)))
}

fn read_extra(port: &mut dyn Port, e: Extra) -> Result<Option<i32>> {
    let Some((cmd, sub)) = extra_query(e) else { return Ok(None) };
    Ok(request(port, &query(cmd, sub), cmd, Some(sub))?.and_then(|f| decode_extra(&f, e)))
}

/// readStatus' motionSync, rippleControl, angleSnapping, debounceMs (its order), as rows in the card's order; a read that is
/// not answered leaves its row out.
fn read_extras(port: &mut dyn Port) -> Result<Vec<ExtraRow>> {
    let mut got = Vec::new();
    for e in [Extra::MotionSync, Extra::RippleControl, Extra::AngleSnap, Extra::Debounce] {
        got.push((e, read_extra(port, e)?));
    }
    Ok(extra_rows()
        .into_iter()
        .filter_map(|mut r| {
            r.value = got.iter().find(|g| g.0 == r.extra).and_then(|g| g.1);
            r.value.is_some().then_some(r)
        })
        .collect())
}

fn read_identity(port: &mut dyn Port) -> Result<Option<Identity>> {
    Ok(request(port, &query(CMD_QUERY_IDENTITY, 0), CMD_QUERY_IDENTITY, None)?.and_then(|f| decode_identity(&f)))
}

fn the_product(m: &Matched) -> Result<&'static Product> {
    product(m.variant).ok_or_else(|| Error::UnsupportedMouse(format!("Incott product id {:#06x}", m.variant)))
}

// ---------------------------------------------------------------- the driver

/// The Incott driver (IncottHidClient).
pub struct Incott;

impl crate::family::Driver for Incott {
    /// READ requests only: the identity probe (OpenMouse's `open()`: a collection that does not answer it is the dead twin),
    /// the DPI cycle and the active stage's value, polling, lift-off. Every one passes [`read_request_allowed`]. A mouse that
    /// stays silent is `online: false` (OpenMouse's degraded status), nothing is invented.
    fn read(&self, m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let p = the_product(m)?;
        let mut r = OnMouse { wireless: Some(p.wireless), caps: caps(p.wireless, None), ..Default::default() };
        let Some(identity) = read_identity(port)? else {
            return Ok(r);
        };
        r.online = true;
        r.caps = caps(p.wireless, identity.paw3950);
        r.model = identity.name.map(|n| format!("Incott {n}"));
        if let Some((_, active)) = read_cycle(port)? {
            r.dpi = read_stage_dpi(port, active)?;
        }
        r.polling_hz = read_polling(port)?;
        r.lift_off = read_lift_off(port)?;
        r.extras = read_extras(port)?;
        Ok(r)
    }

    /// Order 110: setDebounceTime / setMotionSync / setAngleSnapping / setRippleControl - checked before a byte leaves, then the
    /// write and the read it confirms with.
    fn change(&self, _m: &Matched, port: &mut dyn Port, c: Change) -> Result<()> {
        let Change::Extra(e, v) = c;
        check_extra(&extra_rows(), e, v)?;
        let set = encode_set_extra(e, v).ok_or_else(|| Error::range(e.label(), format!("{v}")))?;
        write_report(port, &set)?;
        let got = read_extra(port, e)?;
        if got != Some(v) {
            return Err(Error::BadAnswer(format!("the mouse kept {got:?} instead of {v} ({})", e.label())));
        }
        Ok(())
    }

    /// setDpi / setPollingRate / setLiftOffDistance the way IncottHidClient does them: the write, then the read-back it
    /// confirms with. Before the write, one read request for undo ("before").
    fn write(&self, m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        let p = the_product(m)?;
        match s {
            Setting::Dpi(dpi) => {
                // incottValidateDpi first, before any round-trip
                if !dpi_valid(dpi) {
                    return Err(Error::range("DPI", format!("{dpi} DPI is not one this Incott mouse can store (50 - 45000 in steps of 50)")));
                }
                // setDpi = the ACTIVE stage's value
                let (_, stage) = read_cycle(port)?.ok_or_else(|| Error::BadAnswer("could not read the active DPI stage to write".into()))?;
                let before = read_stage_dpi(port, stage)?.ok_or_else(|| Error::BadAnswer("the mouse did not report the active DPI stage's value".into()))?;
                let set = encode_set_dpi(stage, dpi).ok_or_else(|| Error::range("DPI", format!("{dpi} DPI")))?;
                write_report(port, &set)?;
                let got = read_stage_dpi(port, stage)?;
                if got != Some(dpi) {
                    return Err(Error::BadAnswer(format!("the mouse kept {got:?} DPI instead of {dpi}")));
                }
                Ok(Changed { now: dpi, before: Some(before) })
            }
            Setting::Polling(hz) => {
                // OpenMouse's page offers only the connection's own ladder (the cable never reaches above 1000 Hz)
                let set = encode_set_polling(hz).filter(|_| polling_rates(p.wireless).contains(&hz)).ok_or_else(|| Error::range("polling rate", format!("this mouse does not support {hz} Hz on this connection")))?;
                let before = read_polling(port)?.ok_or_else(|| Error::BadAnswer("the mouse did not report its polling rate".into()))?;
                write_report(port, &set)?;
                let got = read_polling(port)?;
                if got != Some(hz) {
                    return Err(Error::BadAnswer(format!("the mouse kept {got:?} Hz instead of {hz} Hz")));
                }
                Ok(Changed { now: hz, before: Some(before) })
            }
            Setting::LiftOff(tenths) => {
                let set = encode_set_lift_off(tenths).ok_or_else(|| Error::range("lift-off distance", format!("this mouse has no {tenths}/10 mm lift-off (0.7, 1 or 2 mm)")))?;
                let before = read_lift_off(port)?.ok_or_else(|| Error::BadAnswer("the mouse did not report its lift-off distance".into()))?;
                write_report(port, &set)?;
                let got = read_lift_off(port)?;
                if got != Some(tenths) {
                    return Err(Error::BadAnswer(format!("the mouse kept a {got:?}/10 mm lift-off distance instead of {tenths}/10 mm")));
                }
                Ok(Changed { now: tenths, before: Some(before) })
            }
        }
    }
}

//! Dareu A950 PRO Mg - ported from OpenMouse (`crate::family`: AGPL-3.0-or-later, @openmouse/protocol 0.40.1), DPI /
//! polling / battery only:
//!
//! - the codec: `dist/dareu/index.js` (the Jm checksum, the report-8 frames, the memory read / write records, the DPI id
//!   map, the polling ids);
//! - the client: `dist/drivers/dareu/hid.js` (DareuHidClient: isSupported, readStatus, setDpi, setPollingRate and the
//!   exchange / readBuffer / writeBuffer plumbing).
//!
//! Frame: an output report with id 8 and a 16-byte body. OpenMouse's `sendReport(8, body)` passes the body without the id;
//! Windows puts the id in byte 0, so OpenMouse's body byte i is our byte i + 1. Body: `[0]` command, `[1]` 0, `[2..4]` memory
//! address (high, low) and `[4]` length for a memory read / write, `[5..]` data, `[15]` checksum. The checksum is Jm's: the
//! sum of the report id and body bytes 0..14, then `85 - sum` when the sum is under 55, else `341 - sum` (low byte).
//! Commands used: 3 "is the mouse online?" (answer byte 5 = 1), 4 battery (answer byte 5 = percent, byte 6 = status), 8
//! read memory (at most 10 bytes; the answer echoes address and length, data from byte 5), 7 write memory (at most 10 bytes).
//! Answers are the next input reports with id 8 and the same command; "event 10" reports are skipped.
//!
//! Memory (all values are followed by their own checksum byte, the same Jm rule over the value bytes): address 0 polling
//! rate id (`[id, chk]`: 125→8, 250→4, 500→2, 1000→1, 2000→16, 4000→32); address 2 DPI header (`[count, chk, active, chk]`);
//! address 12 the legacy DPI stages, four bytes each `[x low, y low, high byte shared by x and y, chk]`; a DPI value maps to
//! an id `dpi / 50 - 1` (ids 256..511 carry high byte 0x44, 512.. carry 0x88).
//!
//! A change reads first (online? / the current record), writes ONLY the one record (the active DPI stage's four bytes, or the
//! two polling bytes) and reads it back to confirm - the rest of the memory is not touched.
//!
//! Left out on purpose: the active profile, firmware versions, the DPI stage count / active stage, DPI indicator lighting,
//! sleep timers and button mappings (none is DPI / polling / lift-off / battery). Lift-off: this Dareu has none. OpenMouse
//! sends no firmware-update, reset or profile-wipe command to a Dareu mouse and neither does this file. One difference
//! from OpenMouse on purpose: a stage whose X and Y DPI differ is not rewritten (OpenMouse would set Y to X; the model has a
//! single DPI value) - the change is refused.

use crate::error::{Error, Result};
use crate::family::{Caps, Changed, DpiCaps, Family, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x260d;
/// The A950 PRO Mg on its 2.4 GHz receiver (TM265).
pub const PID_RECEIVER: u16 = 0x1114;
/// The A950 PRO Mg on its cable (TM271F).
pub const PID_WIRED: u16 = 0x1117;
pub const MODEL: &str = "Dareu A950 PRO Mg";

pub const REPORT_ID: u8 = 0x08;
/// The body behind the report id.
pub const BODY_LEN: usize = 16;
/// Windows' report: the id + the body.
pub const REPORT_LEN: usize = BODY_LEN + 1;
/// Longest memory read / write (DAREU_MAX_BUFFER_CHUNK).
pub const MAX_CHUNK: usize = 10;
/// OpenMouse's RESPONSE_TIMEOUT_MS.
pub const ANSWER_TIMEOUT_MS: u32 = 800;

pub const CMD_CHECK_ACTIVE: u8 = 3;
pub const CMD_GET_BATTERY: u8 = 4;
pub const CMD_WRITE_BUFFER: u8 = 7;
pub const CMD_READ_BUFFER: u8 = 8;

/// DAREU_MEMORY_ADDRESS (the ones this file uses).
pub const ADDR_POLLING: u16 = 0;
pub const ADDR_DPI_HEADER: u16 = 2;
pub const ADDR_DPI_STAGES: u16 = 12;

pub const DPI_MIN: u32 = 100;
pub const DPI_MAX: u32 = 26000;
pub const DPI_STEP: u32 = 50;
pub const MAX_DPI_STAGES: usize = 5;
pub const WIRED_POLLING: [u32; 4] = [125, 250, 500, 1000];
pub const RECEIVER_POLLING: [u32; 6] = [125, 250, 500, 1000, 2000, 4000];

// ============================================================================ codec

/// Jm checksum over every byte but the last (`bytes` includes the checksum slot).
pub fn checksum(bytes: &[u8]) -> u8 {
    let sum: u32 = bytes[..bytes.len().saturating_sub(1)].iter().map(|b| *b as u32).sum();
    let c = if sum < 55 { 85u32.wrapping_sub(sum) } else { 341u32.wrapping_sub(sum) };
    (c & 0xFF) as u8
}

/// `data` followed by its own checksum byte (dareuWithMemoryChecksum).
pub fn with_memory_checksum(data: &[u8]) -> Vec<u8> {
    let mut v = data.to_vec();
    v.push(0);
    let c = checksum(&v);
    *v.last_mut().expect("just pushed") = c;
    v
}

/// Is the last byte the Jm checksum of the others (dareuHasValidMemoryChecksum)?
pub fn has_valid_memory_checksum(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[bytes.len() - 1] == checksum(bytes)
}

/// A frame as Windows sends it: id 8, the command, zeros, the checksum in the last byte.
fn frame(cmd: u8) -> [u8; REPORT_LEN] {
    let mut f = [0u8; REPORT_LEN];
    f[0] = REPORT_ID;
    f[1] = cmd;
    f
}

fn seal(mut f: [u8; REPORT_LEN]) -> Vec<u8> {
    f[REPORT_LEN - 1] = checksum(&f);
    f.to_vec()
}

/// dareuCheckActiveRequest.
pub fn check_active_request() -> Vec<u8> {
    seal(frame(CMD_CHECK_ACTIVE))
}

/// dareuGetBatteryRequest.
pub fn battery_request() -> Vec<u8> {
    seal(frame(CMD_GET_BATTERY))
}

/// One memory read of at most [`MAX_CHUNK`] bytes (readBufferRequest).
pub fn read_buffer_request(address: u16, len: usize) -> Vec<u8> {
    let mut f = frame(CMD_READ_BUFFER);
    f[3] = (address >> 8) as u8;
    f[4] = (address & 0xFF) as u8;
    f[5] = len as u8;
    seal(f)
}

/// A memory read split into ten-byte requests (dareuReadBufferRequests).
pub fn read_buffer_requests(address: u16, len: usize) -> Vec<Vec<u8>> {
    (0..len).step_by(MAX_CHUNK).map(|off| read_buffer_request(address + off as u16, MAX_CHUNK.min(len - off))).collect()
}

/// A memory write split into ten-byte requests (dareuWriteBufferRequests).
pub fn write_buffer_requests(address: u16, data: &[u8]) -> Vec<Vec<u8>> {
    data.chunks(MAX_CHUNK)
        .enumerate()
        .map(|(i, chunk)| {
            let a = address + (i * MAX_CHUNK) as u16;
            let mut f = frame(CMD_WRITE_BUFFER);
            f[3] = (a >> 8) as u8;
            f[4] = (a & 0xFF) as u8;
            f[5] = chunk.len() as u8;
            f[6..6 + chunk.len()].copy_from_slice(chunk);
            seal(f)
        })
        .collect()
}

/// dareuMatchesReply: `request` / `reply` are 16-byte bodies (no report id). Not an event (10), the same command, and for a
/// memory read the echoed address and length.
pub fn matches_reply(request: &[u8], reply: &[u8]) -> bool {
    if request.len() != BODY_LEN || reply.len() != BODY_LEN || reply[0] == 10 || request[0] != reply[0] {
        return false;
    }
    request[0] != CMD_READ_BUFFER || (request[2] == reply[2] && request[3] == reply[3] && request[4] == reply[4])
}

/// dareuDecodeReadBufferReply: the data of a memory-read answer (bodies).
pub fn decode_read_buffer_reply(request: &[u8], reply: &[u8]) -> Option<Vec<u8>> {
    if !matches_reply(request, reply) || request[0] != CMD_READ_BUFFER {
        return None;
    }
    let len = request[4] as usize;
    if !(1..=MAX_CHUNK).contains(&len) || reply.len() < 5 + len {
        return None;
    }
    Some(reply[5..5 + len].to_vec())
}

/// dareuDecodeBattery: (percent, raw status) of a battery answer (body).
pub fn decode_battery(reply: &[u8]) -> Option<(u8, u8)> {
    (reply.len() == BODY_LEN && reply[0] == CMD_GET_BATTERY && reply[5] <= 100).then(|| (reply[5], reply[6]))
}

/// Dareu's energy page: status 1 and 2 are charging, 0 is not; any other value has no label.
pub fn charging_of(status: u8) -> Option<bool> {
    match status {
        1 | 2 => Some(true),
        0 => Some(false),
        _ => None,
    }
}

/// dareuDpiId: a DPI → the legacy sensor-map id (`None` = not a value the mouse stores).
pub fn dpi_id(dpi: u32) -> Option<u16> {
    if !(DPI_MIN..=DPI_MAX).contains(&dpi) || !(dpi - DPI_MIN).is_multiple_of(DPI_STEP) {
        return None;
    }
    let index = dpi / DPI_STEP - 1;
    Some(match index {
        0..=255 => index as u16,
        256..=511 => (index - 256) as u16 | 0x4400,
        _ => (index - 512) as u16 | 0x8800,
    })
}

/// dareuDpiFromId.
pub fn dpi_from_id(id: u16) -> Option<u32> {
    let index = match id >> 8 {
        0 => id as u32,
        0x44 => 256 + (id & 0xFF) as u32,
        0x88 => 512 + (id & 0xFF) as u32,
        _ => return None,
    };
    let dpi = DPI_STEP * (index + 1);
    ((DPI_MIN..=DPI_MAX).contains(&dpi) && (dpi - DPI_MIN).is_multiple_of(DPI_STEP)).then_some(dpi)
}

/// One stage record as stored: `[x low, y low, high byte, chk]` (writeDpiStage writes x = y).
pub fn dpi_stage_record(dpi: u32) -> Option<Vec<u8>> {
    let id = dpi_id(dpi)?;
    let mut r = vec![(id & 0xFF) as u8, (id & 0xFF) as u8, (id >> 8) as u8, 0];
    r[3] = checksum(&r);
    Some(r)
}

/// The stages' (x, y) DPI of a four-byte record (readDpiConfig): `None` = bad checksum or an id off the map.
pub fn decode_stage_record(raw: &[u8]) -> Option<(u32, u32)> {
    if raw.len() != 4 || !has_valid_memory_checksum(raw) {
        return None;
    }
    let hi = (raw[2] as u16) << 8;
    Some((dpi_from_id(raw[0] as u16 | hi)?, dpi_from_id(raw[1] as u16 | hi)?))
}

/// The polling id byte of a rate on a link (dareuPollingRateId); 2000 / 4000 only through the receiver.
pub fn polling_id(hz: u32, receiver: bool) -> Option<u8> {
    let id = match hz {
        125 => 8,
        250 => 4,
        500 => 2,
        1000 => 1,
        2000 => 16,
        4000 => 32,
        _ => return None,
    };
    (receiver || hz <= 1000).then_some(id)
}

/// dareuPollingRateHz.
pub fn polling_hz(id: u8, receiver: bool) -> Option<u32> {
    [125u32, 250, 500, 1000, 2000, 4000].into_iter().find(|hz| polling_id(*hz, receiver) == Some(id))
}

/// What the mouse's DPI memory holds (readDpiConfig).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DpiConfig {
    pub stage_count: usize,
    pub active: usize,
    /// (x, y) per stage
    pub stages: Vec<(u32, u32)>,
}

/// What the Mouse page offers: OpenMouse's getDpiOptions (100..26000 by 50) and the link's polling rates.
pub fn caps(receiver: bool) -> Caps {
    Caps { dpi: Some(DpiCaps::uniform(DPI_MIN, DPI_MAX, DPI_STEP)), polling: if receiver { RECEIVER_POLLING.to_vec() } else { WIRED_POLLING.to_vec() }, lift_off: Vec::new(), battery: true }
}

/// The read-only proof's allow-list: a report-8 frame with a valid checksum carrying one of the Dareu READ requests this
/// driver sends - online? (3), battery (4), or a memory read (8, at most ten bytes, inside the first 256 bytes of memory,
/// where the polling rate, the DPI header and the DPI stages live) - and nothing else in it. Every write (7), profile,
/// firmware-less setting or button command is refused.
pub fn read_request_allowed(out: &[u8]) -> bool {
    if out.len() != REPORT_LEN || out[0] != REPORT_ID || out[REPORT_LEN - 1] != checksum(out) {
        return false;
    }
    match out[1] {
        CMD_CHECK_ACTIVE | CMD_GET_BATTERY => out[2..REPORT_LEN - 1].iter().all(|b| *b == 0),
        CMD_READ_BUFFER => {
            let (addr, len) = ((out[3] as usize) << 8 | out[4] as usize, out[5] as usize);
            out[2] == 0 && out[6..REPORT_LEN - 1].iter().all(|b| *b == 0) && (1..=MAX_CHUNK).contains(&len) && addr + len <= 0x100
        }
        _ => false,
    }
}

// ============================================================================ device table

/// OpenMouse's isSupported for the A950 PRO Mg: the vendor id and one of the two ids, the receiver-service collection
/// (usage page 0xFF05, usage 0) AND the command collection (0xFF02 / usage 2) with report 8 of 16 bytes both ways.
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID || (pid != PID_RECEIVER && pid != PID_WIRED) {
        return None;
    }
    own.iter().find(|h| h.usage_page == 0xFF05 && h.usage == 0x00)?;
    let cmd = own.iter().find(|h| h.usage_page == 0xFF02 && h.usage == 0x02 && h.input_len as usize == REPORT_LEN && h.output_len as usize == REPORT_LEN)?;
    Some(Matched { family: Family::Dareu, model: Some(MODEL), wireless: Some(pid == PID_RECEIVER), config: (*cmd).clone(), variant: 0 })
}

// ============================================================================ driver

/// The Dareu driver.
pub struct Dareu;

fn receiver_of(m: &Matched) -> bool {
    m.config.pid == PID_RECEIVER
}

/// One request and the answer to it (OpenMouse's exchange): `Ok(body)` is the 16-byte body behind the report id.
fn exchange(port: &mut dyn Port, request: &[u8]) -> Result<Vec<u8>> {
    let how = HidTransfer::OutputThenInput { reply_len: (port.info().input_len as usize).max(REPORT_LEN), max_reads: 16, timeout_ms: ANSWER_TIMEOUT_MS, echo: Some((1, request[1])) };
    let a = port.exchange(request, &how)?;
    if a.len() != REPORT_LEN {
        return Err(Error::BadAnswer(format!("Dareu answered {} bytes, not {REPORT_LEN}", a.len())));
    }
    let body = a[1..].to_vec();
    if !matches_reply(&request[1..], &body) {
        return Err(Error::BadAnswer(format!("Dareu answer does not match command {:#04x}", request[1])));
    }
    Ok(body)
}

/// `await this.optional`: no usable answer = `None`; a mouse that is gone, or a request the read-only layer refused, still fails.
fn optional<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(Error::BadAnswer(_) | Error::Os { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// ensureActive: the mouse answers "online" (byte 5 = 1). `Ok(false)` = it answered "not linked" or did not answer.
fn is_online(port: &mut dyn Port) -> Result<bool> {
    Ok(optional(exchange(port, &check_active_request()))?.is_some_and(|b| b[5] == 1))
}

fn read_buffer(port: &mut dyn Port, address: u16, len: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(len);
    for (i, req) in read_buffer_requests(address, len).iter().enumerate() {
        let body = exchange(port, req)?;
        let chunk = decode_read_buffer_reply(&req[1..], &body).ok_or_else(|| Error::BadAnswer(format!("Dareu memory read failed at {:#x}", address as usize + i * MAX_CHUNK)))?;
        out.extend(chunk);
    }
    Ok(out)
}

/// writeBuffer: each request waits for its (command 7) answer; the answer's content is not checked, the read-back is.
fn write_buffer(port: &mut dyn Port, address: u16, data: &[u8]) -> Result<()> {
    for req in write_buffer_requests(address, data) {
        exchange(port, &req)?;
    }
    Ok(())
}

fn read_dpi_config(port: &mut dyn Port) -> Result<DpiConfig> {
    let h = read_buffer(port, ADDR_DPI_HEADER, 4)?;
    if !has_valid_memory_checksum(&h[0..2]) || !has_valid_memory_checksum(&h[2..4]) {
        return Err(Error::BadAnswer("the Dareu DPI header checksum is invalid".into()));
    }
    let (stage_count, active) = (h[0] as usize, h[2] as usize);
    if !(1..=MAX_DPI_STAGES).contains(&stage_count) || active >= stage_count {
        return Err(Error::BadAnswer("the Dareu DPI stage header is invalid".into()));
    }
    let data = read_buffer(port, ADDR_DPI_STAGES, stage_count * 4)?;
    let mut stages = Vec::new();
    for (i, raw) in data.chunks(4).enumerate() {
        stages.push(decode_stage_record(raw).ok_or_else(|| Error::BadAnswer(format!("Dareu DPI stage {} is invalid", i + 1)))?);
    }
    Ok(DpiConfig { stage_count, active, stages })
}

fn read_polling(port: &mut dyn Port, receiver: bool) -> Result<u32> {
    let d = read_buffer(port, ADDR_POLLING, 2)?;
    if !has_valid_memory_checksum(&d) {
        return Err(Error::BadAnswer("the Dareu polling-rate checksum is invalid".into()));
    }
    polling_hz(d[0], receiver).ok_or_else(|| Error::BadAnswer("the Dareu polling-rate value is unsupported on this connection".into()))
}

impl crate::family::Driver for Dareu {
    /// OpenMouse's readStatus, cut to what the page shows: online?, battery, the active stage's DPI, polling. Each read that
    /// fails is simply left out (OpenMouse's `.catch(() => null)`). A mouse that does not answer "online" is offline and
    /// nothing else is asked.
    fn read(&self, m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let receiver = receiver_of(m);
        let mut r = OnMouse { model: None, online: false, wireless: m.wireless, caps: caps(receiver), ..Default::default() };
        if !is_online(port)? {
            return Ok(r);
        }
        r.online = true;
        if let Some((pct, status)) = optional(exchange(port, &battery_request()).and_then(|b| decode_battery(&b).ok_or_else(|| Error::BadAnswer("the Dareu battery reply was invalid".into()))))? {
            r.battery_percent = Some(pct);
            r.charging = charging_of(status);
        }
        if let Some(c) = optional(read_dpi_config(port))? {
            r.dpi = c.stages.get(c.active).map(|s| s.0);
        }
        r.polling_hz = optional(read_polling(port, receiver))?;
        Ok(r)
    }

    /// OpenMouse's setDpi (the active stage) / setPollingRate: online?, read the current record, write that one record, read
    /// it back and compare.
    fn write(&self, m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        let receiver = receiver_of(m);
        match s {
            Setting::Dpi(dpi) => {
                let record = dpi_stage_record(dpi).ok_or_else(|| Error::range("DPI", format!("{dpi} DPI can't be stored (Dareu: {DPI_MIN}-{DPI_MAX} in steps of {DPI_STEP})")))?;
                ensure_online(port)?;
                let c = read_dpi_config(port)?;
                let before = c.stages[c.active];
                if before.0 != before.1 {
                    return Err(Error::range("DPI", "this stage has different X and Y DPI - this mouse model has a single DPI value, so the stage is left alone".to_string()));
                }
                write_buffer(port, ADDR_DPI_STAGES + (c.active * 4) as u16, &record)?;
                let now = read_dpi_config(port)?.stages.get(c.active).copied().ok_or_else(|| Error::BadAnswer("the DPI stage vanished".into()))?;
                if now.0 != dpi || now.1 != dpi {
                    return Err(Error::BadAnswer(format!("the mouse kept {} DPI instead of {dpi} DPI", now.0)));
                }
                Ok(Changed { now: now.0, before: Some(before.0) })
            }
            Setting::Polling(hz) => {
                let id = polling_id(hz, receiver).ok_or_else(|| Error::range("polling rate", format!("{hz} Hz is unavailable on this Dareu connection")))?;
                ensure_online(port)?;
                let before = read_polling(port, receiver)?;
                write_buffer(port, ADDR_POLLING, &with_memory_checksum(&[id]))?;
                let now = read_polling(port, receiver)?;
                if now != hz {
                    return Err(Error::BadAnswer(format!("the mouse kept {now} Hz instead of {hz} Hz")));
                }
                Ok(Changed { now, before: Some(before) })
            }
            Setting::LiftOff(t) => Err(Error::range("lift-off distance", format!("{t} tenths of a mm: this Dareu has no lift-off setting"))),
        }
    }
}

fn ensure_online(port: &mut dyn Port) -> Result<()> {
    if is_online(port)? {
        Ok(())
    } else {
        Err(Error::MouseGone("the Dareu mouse is offline - wake it or pair it to this receiver, then retry".into()))
    }
}

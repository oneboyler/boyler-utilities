//! RAWM - ported from OpenMouse (`crate::family`: AGPL-3.0-or-later, @openmouse/protocol 0.40.1), DPI / polling / battery
//! only, but **HELD BACK**: the device table is switched off ([`matches`] gives `None`) until a HID transfer exists that
//! this crate does not have (see NEEDS TRANSFER). Everything else is written and proven:
//!
//! - the codec: `dist/rawm/index.js` (ids and names, the status query, the event framing, the CRC-16, the JSON identity
//!   parser and `buildRawmMouseParams`, the whole settings record);
//! - the client: `dist/drivers/rawm/hid.js` (RawmHidClient: isSupported, readStatus, setDpi, setPollingRate, updateParams).
//!
//! Protocol: unnumbered output / input reports (report id 0) of 64 bytes on the vendor collection (usage page 0xFF00 and
//! usage 1) - OpenMouse's `sendReport(0, frame)`; on Windows the frame sits behind the id byte 0 (65 bytes). A frame is
//! `[0x80 | n, n payload bytes (n <= 63), zeros]`; a longer message is cut into 63-byte pieces, 25 ms apart. The status query
//! is a 13-byte message `[01, 0D, 03, 00, 00, unix time seconds, 8 bytes little endian]`; the mouse answers with a JSON text
//! spread over SEVERAL input reports: the payloads of the input frames are glued together and searched for
//! `FF FF FF FF, (len high << 4 | 2), len low, JSON...` (JSON identity: `battery`, `chr`, `cpi`, `polling`, `cpi_l`, `lod`,
//! `kd`, `ms`, `at`, `as`, `rctrl`, `cpi_l_c`, ..., `crc`). A change is a read-modify-write: query, change ONE field of the
//! identity, rebuild the WHOLE settings record from it (`buildRawmMouseParams`: command 0x03 0x15, every field the mouse
//! reported echoed back - OpenMouse would fill eight optional fields with defaults when absent, this port refuses the write; wrapped with a CRC
//! when the mouse says `crc`), send it in 63-byte pieces, wait 100 ms, query again and compare. OpenMouse refuses the write
//! when a required field is missing from the identity ("refusing a partial configuration write") and so does this file.
//!
//! (Was NEEDS TRANSFER, built by the lead in Order 105 as `HidTransfer::OutputThenStream`; the text below is the helper's
//! description of why it is needed.) "write one output report, then collect SEVERAL input reports on the same
//! open handle". `HidTransfer::OutputThenInput` / `OutputThenMatch` return only the FIRST matching input report and then close
//! the handle, and the status answer (a few hundred bytes of JSON) arrives in 4-8 input reports; a second request opens a new
//! handle and starts a new answer, so the pieces can't be put together. Wanted: `OutputThenStream { reply_len, max_reads,
//! timeout_ms, idle_ms }` = WriteFile(out), then ReadFile input reports (each `reply_len` bytes, report id included) on the same
//! handle until `max_reads` reports came, or `timeout_ms` passed, or `idle_ms` went by without a new one after the first;
//! the answer is all the reports back to back. The driver is written against the small [`Link`] trait; [`PortLink`] is its
//! `Port` adapter and today refuses the query - give it that transfer and set [`TRANSPORT_READY`] to switch the table on.
//!
//! Lift-off: left out. The mouse reports a level 1 (Low) or 2 (High) and OpenMouse offers only "Low" / "High", no millimetres.
//! Also left out: motion sync, angle snapping / tuning, ripple control, performance mode, debounce, sleep timeout (the
//! sleep command), lighting, DPI stages and their colours (echoed back unchanged by every write), the receiver framing
//! (`0xC0 | channel`) - none of the twelve supported models is a receiver. OpenMouse sends no firmware-update, reset or
//! pairing command to a RAWM mouse and neither does this file. One difference from OpenMouse on purpose: a mouse whose DPI
//! is stored as two separate axes (X and Y differ) is not rewritten (OpenMouse would set both to the same value).
//! The receiver ids the driver knows (0x232a, 0x232b, 0x2335, 0x2336) and the "Leviathan V4" (unknown status) are not in the
//! table: not models on OpenMouse's supported list.

use crate::error::{Error, Result};
use crate::family::{Caps, Changed, DpiCaps, Family, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};
use serde_json::Value;

pub const VENDOR_ID: u16 = 0x1915;
pub const USAGE_PAGE: u16 = 0xFF00;
pub const USAGE: u16 = 0x01;
/// A frame: 64 bytes behind the (unnumbered) report id.
pub const PACKET_SIZE: usize = 64;
/// Windows' report: the id 0 + the frame.
pub const REPORT_LEN: usize = PACKET_SIZE + 1;
/// Switch for the device table: on since Order 105 added `HidTransfer::OutputThenStream` (os.rs / win/hid.rs) - the multi-report
/// transfer the status answer needs.
pub const TRANSPORT_READY: bool = true;
/// How long the stream waits for the next input report after the first (the answer is a burst of reports).
pub const STREAM_IDLE_MS: u32 = 250;
/// At most this many input reports are collected for one status answer.
pub const STREAM_MAX_REPORTS: u32 = 24;
/// OpenMouse's WRITE_CHUNK_DELAY_MS between the pieces of a long message.
pub const WRITE_CHUNK_DELAY_MS: u64 = 25;
/// OpenMouse's WRITE_SETTLE_MS before the status is asked again.
pub const WRITE_SETTLE_MS: u64 = 100;
/// OpenMouse's QUERY_TIMEOUT_MS.
pub const QUERY_TIMEOUT_MS: u32 = 1500;
pub const DPI_MIN: u32 = 50;
pub const DPI_MAX: u32 = 42000;
pub const DPI_STEP: u32 = 50;
pub const POLLING_RATES: [u32; 7] = [125, 250, 500, 1000, 2000, 4000, 8000];

/// The models on OpenMouse's SUPPORTED list (supported-devices-data.ts), with the names the driver's catalogue gives them.
pub const MODELS: [(u16, &str); 14] = [
    (0x2328, "Blade 1"),
    (0x2329, "ML01"),
    (0x232c, "MH01"),
    (0x232d, "SL01"),
    (0x232e, "SH01"),
    (0x232f, "SH01"),
    (0x2330, "SH01 Pro"),
    (0x2332, "SH01 Pro"),
    (0x2331, "MH01 Pro"),
    (0x2333, "ES21"),
    (0x2334, "ES21 Pro"),
    (0x2337, "ES21M"),
    (0x2338, "ER21 Pro"),
    (0x2339, "ER21M"),
];

// ============================================================================ codec

/// `rawmCrc16`: CRC-16/CCITT-FALSE (polynomial 0x1021, start 0xFFFF), done the byte-swap way OpenMouse does it.
pub fn crc16(bytes: &[u8]) -> u16 {
    let mut crc: u32 = 0xffff;
    for b in bytes {
        crc = ((crc >> 8) | (crc << 8)) & 0xffff;
        crc ^= *b as u32;
        crc ^= (crc & 0xff) >> 4;
        crc ^= (crc << 12) & 0xffff;
        crc ^= ((crc & 0xff) << 5) & 0xffff;
    }
    (crc & 0xffff) as u16
}

/// `frameRawmEvent` (no receiver channel): `[0x80 | n, payload, zeros]`, 64 bytes.
pub fn frame_event(payload: &[u8]) -> Option<[u8; PACKET_SIZE]> {
    if payload.len() > 63 {
        return None;
    }
    let mut p = [0u8; PACKET_SIZE];
    p[0] = 0x80 | payload.len() as u8;
    p[1..1 + payload.len()].copy_from_slice(payload);
    Some(p)
}

/// `frameRawmEvents`: a message cut into 63-byte frames.
pub fn frame_events(payload: &[u8]) -> Vec<[u8; PACKET_SIZE]> {
    payload.chunks(63).map(|c| frame_event(c).expect("chunks of 63")).collect()
}

/// `rawmEventPayload` (no receiver channel): the payload of one input frame (no report id).
pub fn event_payload(report: &[u8]) -> Option<Vec<u8>> {
    let first = *report.first()?;
    let offset = if first & 0xc0 == 0xc0 { 1 } else { 0 };
    let head = *report.get(offset)?;
    if head & 0xc0 != 0x80 {
        return None;
    }
    let len = (head & 0x3f) as usize;
    Some(report.get(offset + 1..(offset + 1 + len).min(report.len()))?.to_vec())
}

/// `buildRawmQuery`: the 13-byte status query with the time (seconds, little endian) in bytes 5..13.
pub fn build_query(timestamp_seconds: u64) -> Vec<u8> {
    let mut p = vec![0u8; 13];
    p[0] = 0x01;
    p[1] = 13;
    p[2] = 0x03;
    p[5..13].copy_from_slice(&timestamp_seconds.to_le_bytes());
    p
}

/// What the status JSON says (`parseRawmIdentity`): only the fields this port reads or echoes back. A field of the wrong
/// JSON type is absent, as in OpenMouse (`typeof value === "number"`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Identity {
    pub battery: Option<i64>,
    pub charging: Option<bool>,
    pub dpi: Option<i64>,
    pub polling: Option<i64>,
    pub dpi_levels: Option<Vec<i64>>,
    pub power_mode: Option<i64>,
    pub device_name: Option<String>,
    pub lod: Option<i64>,
    pub key_delay: Option<Vec<i64>>,
    pub motion_sync: Option<bool>,
    /// the signed byte (`at`)
    pub angle_tuning: Option<i64>,
    pub angle_snapping: Option<bool>,
    pub ripple_control: Option<bool>,
    pub wired: Option<bool>,
    pub light: Option<i64>,
    pub level_colors: Option<Vec<i64>>,
    pub onboard: Option<i64>,
    pub tx_output_power: Option<i64>,
    pub battery_levels: Option<Vec<i64>>,
    pub auto_tx_power: Option<i64>,
    pub onboard_status: Option<Vec<i64>>,
    pub glass_mode_enabled: Option<i64>,
    pub crc_supported: Option<bool>,
}

fn num(v: &Value, k: &str) -> Option<i64> {
    match v.get(k)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        _ => None,
    }
}

fn flag(v: &Value, k: &str) -> Option<bool> {
    num(v, k).map(|n| n != 0)
}

fn nums(v: &Value, k: &str) -> Option<Vec<i64>> {
    match v.get(k)? {
        Value::Array(a) => Some(a.iter().filter_map(|x| if let Value::Number(n) = x { n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)) } else { None }).collect()),
        _ => None,
    }
}

/// A JS signed byte (`signedByteValue`).
fn signed_byte(n: i64) -> i64 {
    let b = n & 0xff;
    if b >= 0x80 { b - 0x100 } else { b }
}

pub fn parse_identity(json: &str) -> Option<Identity> {
    let v: Value = serde_json::from_str(json).ok()?;
    if !v.is_object() {
        return None;
    }
    let glass = match v.get("gm") {
        Some(Value::Array(a)) => a.get(1).and_then(|x| x.as_f64()).map(|f| f as i64),
        Some(_) => Some(1),
        None => None,
    };
    Some(Identity {
        battery: num(&v, "battery"),
        charging: flag(&v, "chr"),
        dpi: num(&v, "cpi"),
        polling: num(&v, "polling"),
        dpi_levels: nums(&v, "cpi_l"),
        power_mode: num(&v, "pm"),
        device_name: v.get("dn").and_then(|x| x.as_str()).map(String::from),
        lod: num(&v, "lod"),
        key_delay: nums(&v, "kd"),
        motion_sync: flag(&v, "ms"),
        angle_tuning: num(&v, "at").map(signed_byte),
        angle_snapping: flag(&v, "as"),
        ripple_control: flag(&v, "rctrl"),
        wired: flag(&v, "wired"),
        light: num(&v, "light"),
        level_colors: nums(&v, "cpi_l_c"),
        onboard: num(&v, "ob"),
        tx_output_power: num(&v, "top"),
        battery_levels: nums(&v, "bl"),
        auto_tx_power: num(&v, "atp"),
        onboard_status: nums(&v, "ocs"),
        glass_mode_enabled: glass,
        crc_supported: flag(&v, "crc"),
    })
}

/// `parseRawmQueryResult`: the first JSON identity in the glued-together payloads (`None` = not complete yet / not there).
pub fn parse_query_result(stream: &[u8]) -> Option<Identity> {
    let sync = [0xffu8; 4];
    let mut index = 0;
    while index + sync.len() <= stream.len() {
        if stream[index..index + 4] != sync {
            index += 1;
            continue;
        }
        let start = index + 4;
        index += 1;
        if stream.len() < start + 2 || stream[start] & 0x0f != 0x02 {
            continue;
        }
        let length = (((stream[start] & 0xf0) as usize) << 4) | stream[start + 1] as usize;
        if length < 2 || stream.len() < start + length {
            continue;
        }
        let mut body = &stream[start + 2..start + length];
        if body.last() == Some(&0) {
            body = &body[..body.len() - 1];
        }
        if let Some(id) = std::str::from_utf8(body).ok().and_then(parse_identity) {
            return Some(id);
        }
    }
    None
}

fn require<T: Clone>(v: &Option<T>, what: &str) -> Result<T> {
    v.clone().ok_or_else(|| Error::BadAnswer(format!("RAWM did not report {what}; refusing a partial configuration write.")))
}

/// `finishRawmCommand`: the length nibble / byte in the header, wrapped with the CRC when the mouse has one.
fn finish_command(mut base: Vec<u8>, crc_supported: bool) -> Vec<u8> {
    let length = base.len();
    base[0] = (((length >> 4) & 0xf0) as u8) | (base[0] & 0x0f);
    base[1] = (length & 0xff) as u8;
    if !crc_supported {
        return base;
    }
    let crc = crc16(&base);
    let total = length + 5;
    let mut w = vec![0u8; total];
    w[0] = (((total >> 4) & 0xf0) as u8) | 0x03;
    w[1] = (total & 0xff) as u8;
    w[2] = 0x24;
    w[3] = (crc & 0xff) as u8;
    w[4] = (crc >> 8) as u8;
    w[5..].copy_from_slice(&base);
    w
}

/// `buildRawmMouseParams`: the whole CONFIG_TYPE_MOUSE_PARAM record RAWM HUB sends, from the identity the mouse reported.
/// Errors (nothing may be sent) when a field is missing - the eight that OpenMouse defaults (light, onboard, power mode, tx power,
/// battery table, auto tx power, onboard status, glass mode) are required here too, so no made-up value is ever written.
pub fn build_mouse_params(id: &Identity) -> Result<Vec<u8>> {
    let dpi = require(&id.dpi, "current DPI")? as u32;
    let polling = require(&id.polling, "polling rate")? as u32;
    // OpenMouse fills these optional fields with defaults when the answer lacks them (a made-up battery table, power mode ...);
    // this port refuses instead (Order 105 review): a write only echoes what the mouse itself reported.
    let light = require(&id.light, "the light setting")?;
    let levels: Vec<u32> = require(&id.dpi_levels, "DPI stages")?.into_iter().map(|v| v as u32).collect();
    let colors = require(&id.level_colors, "DPI stage colors")?;
    let key_delay = require(&id.key_delay, "button debounce values")?;
    let battery_levels = require(&id.battery_levels, "the battery voltage table")?;
    let onboard_status = require(&id.onboard_status, "the onboard status")?;
    let mut b: Vec<i64> = vec![0x03, 0, 0x15];
    let wide_dpi = dpi & 0xffff_0000 != 0;
    b.push(if wide_dpi { 0 } else { (dpi & 0xff) as i64 });
    b.push(if wide_dpi { 0 } else { ((dpi >> 8) & 0xff) as i64 });
    b.push((polling & 0xff) as i64);
    b.push(((polling >> 8) & 0xff) as i64);
    b.push(light);
    let wide_levels = levels.iter().any(|v| v & 0xffff_0000 != 0);
    b.push(if wide_levels { 0 } else { levels.len() as i64 });
    if !wide_levels {
        for v in &levels {
            b.push((v & 0xff) as i64);
            b.push(((v >> 8) & 0xff) as i64);
        }
    }
    b.push(require(&id.onboard, "the onboard setting")?);
    b.push(require(&id.power_mode, "the power mode")?);
    for shift in [0, 8, 16, 24] {
        b.push(if wide_dpi { ((dpi >> shift) & 0xff) as i64 } else { 0 });
    }
    b.push(if wide_levels { levels.len() as i64 } else { 0 });
    if wide_levels {
        for v in &levels {
            for shift in [0, 8, 16, 24] {
                b.push(((v >> shift) & 0xff) as i64);
            }
        }
    }
    b.push(require(&id.lod, "lift-off distance")?);
    b.push(key_delay.len() as i64);
    b.extend(key_delay.iter().copied());
    b.push(require(&id.motion_sync, "Motion Sync")? as i64);
    b.push(require(&id.angle_tuning, "angle tuning")? & 0xff);
    b.push(require(&id.angle_snapping, "angle snapping")? as i64);
    b.push(require(&id.ripple_control, "ripple control")? as i64);
    b.push(colors.len() as i64);
    b.extend(colors.iter().map(|c| c & 7));
    b.push(require(&id.tx_output_power, "the transmit power")?);
    b.push(battery_levels.len() as i64);
    for v in &battery_levels {
        b.push(v & 0xff);
        b.push((v >> 8) & 0xff);
    }
    b.push(require(&id.auto_tx_power, "auto transmit power")?);
    b.push(onboard_status.len() as i64);
    b.extend(onboard_status.iter().copied());
    b.push(require(&id.glass_mode_enabled, "glass mode")?);
    // Uint8Array.from: every value modulo 256
    Ok(finish_command(b.into_iter().map(|v| v as u8).collect(), id.crc_supported == Some(true)))
}

/// What the Mouse page offers: OpenMouse's `getDpiOptions` (50..42000 by 50) and the seven rates it can set. No lift-off in
/// millimetres. Battery only on a mouse whose status reports one.
pub fn caps(has_battery: bool) -> Caps {
    Caps { dpi: Some(DpiCaps::uniform(DPI_MIN, DPI_MAX, DPI_STEP)), polling: POLLING_RATES.to_vec(), lift_off: Vec::new(), battery: has_battery }
}

/// The report as Windows sends it: id 0 + the 64-byte frame.
pub fn report(frame: &[u8; PACKET_SIZE]) -> Vec<u8> {
    let mut v = vec![0u8];
    v.extend_from_slice(frame);
    v
}

/// The read-only proof's allow-list: the one RAWM read, the status query - report id 0, a frame holding the 13-byte message
/// `01 0D 03 00 00 <8 time bytes>` and zeros after it. Every settings record (its message starts `0x?3 <len> 0x15`) is refused.
pub fn read_request_allowed(out: &[u8]) -> bool {
    out.len() >= REPORT_LEN && out[0] == 0 && out[1] == 0x80 | 13 && out[2..7] == [0x01, 13, 0x03, 0x00, 0x00] && out[15..].iter().all(|b| *b == 0)
}

// ============================================================================ device table

/// OpenMouse's isSupported: the vendor id, a listed product and the collection with usage page 0xFF00 / usage 1 that has
/// report 0 both ways, exactly 65 bytes with the id (the driver reads and writes 65-byte frames, so a longer collection would
/// misalign its reads).
pub fn table_matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID {
        return None;
    }
    let (_, name) = MODELS.iter().find(|(p, _)| *p == pid)?;
    let cfg = own.iter().find(|h| h.usage_page == USAGE_PAGE && h.usage == USAGE && h.input_len as usize == REPORT_LEN && h.output_len as usize == REPORT_LEN)?;
    Some(Matched { family: Family::Rawm, model: Some(name), wireless: None, config: (*cfg).clone(), variant: 0 })
}

/// The device table (on while [`TRANSPORT_READY`]).
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if TRANSPORT_READY { table_matches(own, vid, pid) } else { None }
}

// ============================================================================ driver

/// What the driver needs of the interface: send one frame, and send the query and collect the answer's input frames.
pub trait Link {
    /// One frame, no answer (OpenMouse's `sendReport(0, frame)`).
    fn send(&mut self, frame: &[u8; PACKET_SIZE]) -> Result<()>;
    /// The query frame, then the input frames (64 bytes each, no report id) that follow it, until the answer is in or time
    /// ran out. A mouse that does not answer is `Err(BadAnswer)`.
    fn query_frames(&mut self, frame: &[u8; PACKET_SIZE]) -> Result<Vec<Vec<u8>>>;
    fn pause_ms(&mut self, ms: u64);
    /// Seconds since 1970 (goes into the query).
    fn now_secs(&self) -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    }
}

/// [`Link`] over a [`Port`]: sending works; the multi-report answer needs the transfer in NEEDS TRANSFER, so the query is
/// refused for now.
pub struct PortLink<'a>(pub &'a mut dyn Port);

impl Link for PortLink<'_> {
    fn send(&mut self, frame: &[u8; PACKET_SIZE]) -> Result<()> {
        self.0.exchange(&report(frame), &HidTransfer::OutputOnly).map(|_| ())
    }
    /// WriteFile(query), then every input report that follows on the same handle ([`HidTransfer::OutputThenStream`]); each is
    /// `REPORT_LEN` bytes with the (unnumbered) report id 0 in front, which is cut off.
    fn query_frames(&mut self, frame: &[u8; PACKET_SIZE]) -> Result<Vec<Vec<u8>>> {
        let how = HidTransfer::OutputThenStream { reply_len: REPORT_LEN, max_reads: STREAM_MAX_REPORTS, timeout_ms: QUERY_TIMEOUT_MS, idle_ms: STREAM_IDLE_MS };
        let all = self.0.exchange(&report(frame), &how)?;
        Ok(all.chunks(REPORT_LEN).filter(|c| c.len() == REPORT_LEN).map(|c| c[1..].to_vec()).collect())
    }
    fn pause_ms(&mut self, ms: u64) {
        self.0.pause_ms(ms)
    }
}

/// The RAWM driver.
pub struct Rawm;

/// `query()`: ask, glue the answer's payloads together, parse the JSON identity.
pub fn query(link: &mut dyn Link) -> Result<Identity> {
    let frame = frame_event(&build_query(link.now_secs())).expect("13 bytes");
    let mut stream = Vec::new();
    for f in link.query_frames(&frame)? {
        if let Some(p) = event_payload(&f) {
            stream.extend(p);
        }
    }
    parse_query_result(&stream).ok_or_else(|| Error::BadAnswer("RAWM did not answer the status query".into()))
}

/// readStatus, cut to the page: `dpi = (packed & 0xffff) || first positive stage`.
pub fn read_with(link: &mut dyn Link, model: Option<&str>, _opt: ReadOptions) -> Result<OnMouse> {
    let mut r = OnMouse { model: model.map(String::from), online: false, caps: caps(false), ..Default::default() };
    let id = match query(link) {
        Ok(i) => i,
        Err(Error::BadAnswer(_) | Error::Os { .. }) => return Ok(r),
        Err(e) => return Err(e),
    };
    r.online = true;
    r.model = id.device_name.clone().or(r.model);
    if let Some(b) = id.battery {
        r.battery_percent = Some(b.clamp(0, 100) as u8);
        r.charging = Some(id.charging == Some(true));
    }
    r.caps = caps(id.battery.is_some());
    let packed = id.dpi.unwrap_or(0) as u32;
    let first = id.dpi_levels.as_ref().and_then(|l| l.iter().copied().find(|v| *v > 0)).unwrap_or(0) as u32;
    let dpi = if packed & 0xffff != 0 { packed & 0xffff } else { first };
    r.dpi = (dpi > 0).then_some(dpi);
    r.polling_hz = id.polling.filter(|p| *p > 0).map(|p| p as u32);
    r.wireless = id.wired.map(|w| !w);
    Ok(r)
}

/// updateParams: query, change one field, send the rebuilt record in pieces, wait, query again.
fn update_params(link: &mut dyn Link, change: &dyn Fn(&mut Identity) -> Result<()>) -> Result<(Identity, Identity)> {
    let before = query(link)?;
    let mut next = before.clone();
    change(&mut next)?;
    let cmd = build_mouse_params(&next)?;
    let frames = frame_events(&cmd);
    let n = frames.len();
    for (i, f) in frames.iter().enumerate() {
        link.send(f)?;
        if i + 1 < n {
            link.pause_ms(WRITE_CHUNK_DELAY_MS);
        }
    }
    link.pause_ms(WRITE_SETTLE_MS);
    let after = query(link)?;
    Ok((before, after))
}

pub fn write_with(link: &mut dyn Link, s: Setting) -> Result<Changed> {
    match s {
        Setting::Dpi(dpi) => {
            if !(DPI_MIN..=DPI_MAX).contains(&dpi) {
                return Err(Error::range("DPI", format!("{dpi} DPI is outside {DPI_MIN}-{DPI_MAX}")));
            }
            let want = dpi as i64;
            let (before, after) = update_params(link, &|i| {
                // a mouse with separate X / Y DPI is not collapsed to one value
                if i.dpi.is_some_and(|d| d > 0xffff) {
                    return Err(Error::range("DPI", "this mouse has different X and Y DPI - it is left alone".to_string()));
                }
                i.dpi = Some(want);
                Ok(())
            })?;
            if after.dpi != Some(want) {
                return Err(Error::BadAnswer("RAWM did not confirm the requested DPI.".into()));
            }
            Ok(Changed { now: dpi, before: before.dpi.map(|d| (d & 0xffff) as u32) })
        }
        Setting::Polling(hz) => {
            if !POLLING_RATES.contains(&hz) {
                return Err(Error::range("polling rate", format!("{hz} Hz is not a RAWM polling rate")));
            }
            let (before, after) = update_params(link, &|i| {
                i.polling = Some(hz as i64);
                Ok(())
            })?;
            if after.polling != Some(hz as i64) {
                return Err(Error::BadAnswer("RAWM did not confirm the requested polling rate.".into()));
            }
            Ok(Changed { now: hz, before: before.polling.map(|p| p as u32) })
        }
        Setting::LiftOff(t) => Err(Error::range("lift-off distance", format!("{t} tenths of a mm: RAWM offers Low / High only, no millimetres"))),
    }
}

impl crate::family::Driver for Rawm {
    fn read(&self, m: &Matched, port: &mut dyn Port, opt: ReadOptions) -> Result<OnMouse> {
        read_with(&mut PortLink(port), m.model, opt)
    }
    fn write(&self, _m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        write_with(&mut PortLink(port), s)
    }
}

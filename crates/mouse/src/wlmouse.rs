//! WLMouse ("WLmouse") - ported from OpenMouse (`crate::family`: AGPL-3.0-or-later, @openmouse/protocol 0.40.1), DPI /
//! polling / lift-off / battery only:
//!
//! - the codec: `compx/codec.js` (the page/command frame WLMouse shares with Lamzu: compaxEncodeRequest, the status bytes,
//!   compaxDecodeDpiStages / PollingRate / LiftOff) and `wlmouse/index.js` (vendor id, the polling-rate table);
//! - the client: `drivers/wlmouse/hid.js` (WLMouseHidClient: readStatus / setDpi / setPollingRate / setLiftOffDistance and
//!   its request/answer loop); the device table is `WLMOUSE_PRODUCTS` in `drivers/vendors.js`, cut down to the five models on
//!   OpenMouse's SUPPORTED list (`supported-devices-data.ts`).
//!
//! Packet (64 bytes = OpenMouse's feature report id 0 payload; Windows puts the report id 0 in front, so OpenMouse's byte i
//! is our byte i + 1; the collection's feature length is 65):
//! request `[0]` 0 · `[1]` 0 · `[2]` target (0 dongle, 1 pairing, 2 mouse) · `[3]` length · `[4]` page (0 device, 1 profile,
//! 3 buttons) · `[5]` command (high bit set = a read) · `[6..]` arguments;
//! answer `[0]` status (0xa1 ok, 0xa2 unsupported; 0xa0 pending / 0xa3 busy are asked again) · `[3]` payload length ·
//! `[4]`/`[5]` page / command echoed · `[6..]` payload. OpenMouse also accepts an answer shifted by one byte (some platforms
//! keep the report id in front); that is kept. Every request is HidD_SetFeature, a 30 ms pause, HidD_GetFeature
//! ([`HidTransfer::Feature`]); an answer for another command, or a pending / busy one, is asked for again (12 tries, 30 ms
//! apart for the first three and 300 ms after - OpenMouse's wake loop for a sleeping dongle mouse).
//!
//! The settings live in the ACTIVE PROFILE: every profile command carries the profile number, read first (page 0 command
//! 0x85, never below 1) - writing to profile 1 while another is active is silently ignored by the mouse.
//!
//! What OpenMouse does for each setting, and what is ported:
//! - polling: profile page command 0x00 `[profile, code]` (125 -> 0x08, 250 -> 0x04, 500 -> 0x02, 1000 -> 0x01, 2000 ->
//!   0x20, 4000 -> 0x40, 8000 -> 0x80), read back with command 0x80. The rates offered are OpenMouse's: all seven, cut at
//!   the model's "<n>K" marker in its USB product name (`maxPollingRateHz`) when there is one.
//! - lift-off: profile page command 0x08 `[profile, code]`, read back with 0x88. OpenMouse names the three codes Low /
//!   Medium / High; the codes themselves are millimetres in the codec's own decoder (`compaxDecodeLiftOff`: bit 0x80 set =
//!   tenths of a mm in the low 7 bits, else whole mm): Low 0x87 = 0.7 mm, Medium 0x01 = 1 mm, High 0x02 = 2 mm.
//! - DPI: OpenMouse's `setDpi` reads the active profile's stage table (command 0x81 on the profile page), replaces the ACTIVE
//!   stage with `(dpi, dpi)` and writes the WHOLE table back in one packet (command 0x01: `[profile, count, x hi, x lo,
//!   y hi, y lo, ...]`, big-endian), then reads the table again. The other stages are copied back as read. Held back (the
//!   write is refused, nothing is sent) when the table can't be copied back exactly: the answer holds fewer stages than
//!   its count says (a write would shrink the count), or the active stage has separate X / Y values (OpenMouse's setDpi
//!   would flatten Y to X - this app never overwrites a value it did not show).
//! - battery: page 0 command 0x83, `[charging (1), percent]`.
//!
//! - Order 110 (A_110_02 scope): debounce (device page write 0x08 `[profile, ms]` 0-15 ms, read 0x88), Motion Sync (profile
//!   page 0x09 / 0x89), angle snapping (0x04 / 0x84), ripple (0x0a / 0x8a), turbo mode (0x13 / 0x93 - the client's
//!   `turboMode`; the mouse only keeps it with high-speed mode on, else the read-back is off and the change is an error, as
//!   in OpenMouse): the active profile, the write, the read-back. The five reads are optional.
//!
//! Left out on purpose: the Beast X 4K (0xa887, `beast-x-4k-hid.js`: another protocol, report 4 - its model is "pr", not
//! supported), the 1K receiver (0xa882, not on the list), stage editor / stage count / active stage, sleep timeout,
//! high-speed (hyper) mode, angle tuning, button combination, firmware / serial
//! / paired-product reads (none is DPI / polling / lift-off / battery), the notification channel. OpenMouse sends no firmware
//! update, DFU or reset command to a WLMouse and neither does this file. Nothing here ran on a real unit.

use crate::error::{Error, Result};
use crate::family::{check_extra, Caps, Change, Changed, DpiCaps, Driver, Extra, ExtraRow, Family, Kind, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x36a7;
pub const REPORT_ID: u8 = 0;
/// OpenMouse's COMPX_PACKET_LENGTH: the payload of the feature report.
pub const PACKET_LEN: usize = 64;
/// Windows' feature report: report id 0 + the 64-byte packet.
pub const REPORT_LEN: usize = PACKET_LEN + 1;
const HEADER_LEN: usize = 6;

pub const STATUS_OK: u8 = 0xa1;
pub const STATUS_UNSUPPORTED: u8 = 0xa2;

pub const TARGET_DONGLE: u8 = 0x00;
pub const TARGET_MOUSE: u8 = 0x02;
pub const PAGE_DEVICE: u8 = 0x00;
pub const PAGE_PROFILE: u8 = 0x01;

/// OpenMouse's RESPONSE_ATTEMPTS / RESPONSE_DELAY_MS / WAKE_DELAY_MS / QUICK_ATTEMPTS.
pub const ATTEMPTS: u32 = 12;
pub const RESPONSE_DELAY_MS: u32 = 30;
pub const WAKE_DELAY_MS: u64 = 300;
pub const QUICK_ATTEMPTS: u32 = 3;

/// DPI_STEP / DPI_MAX of the client; the stage table holds at most DPI_STAGE_MAX stages.
pub const DPI_MIN: u32 = 50;
pub const DPI_MAX: u32 = 30000;
pub const DPI_STEP: u32 = 50;
pub const DPI_STAGE_MAX: u8 = 6;

/// WLMOUSE_POLLING_RATES: (code, Hz); a write uses the FIRST code of a rate, a read knows both 1000 Hz codes.
pub const POLLING_CODES: [(u8, u32); 8] = [(0x08, 125), (0x04, 250), (0x02, 500), (0x01, 1000), (0x10, 1000), (0x20, 2000), (0x40, 4000), (0x80, 8000)];
/// The rates the client can offer (before the model's cut): the table's distinct rates, ascending.
pub const POLLING_RATES: [u32; 7] = [125, 250, 500, 1000, 2000, 4000, 8000];

/// LIFT_OFF_DISTANCES: Low 0x87, Medium 0x01, High 0x02 - in tenths of a mm 7, 10, 20 (see the header).
pub const LIFT_OFF_CODES: [(u8, u32); 3] = [(0x87, 7), (0x01, 10), (0x02, 20)];

// ---------------------------------------------------------------- the codec

/// One request: OpenMouse's `spec` (target, page, command, declared length, arguments).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    pub target: u8,
    pub page: u8,
    pub command: u8,
    pub length: u8,
    pub args: Vec<u8>,
    /// a request-specific number of tries instead of [`ATTEMPTS`]
    pub attempts: Option<u32>,
}

fn read(target: u8, page: u8, command: u8, length: u8, args: &[u8]) -> Spec {
    Spec { target, page, command, length, args: args.to_vec(), attempts: None }
}

/// READ.firmware - used only as OpenMouse's first call: "does the mouse answer?".
pub fn read_firmware() -> Spec {
    read(TARGET_MOUSE, PAGE_DEVICE, 0x81, 0x10, &[])
}
/// READ.battery
pub fn read_battery() -> Spec {
    read(TARGET_MOUSE, PAGE_DEVICE, 0x83, 0x02, &[])
}
/// READ.activeProfile
pub fn read_active_profile() -> Spec {
    read(TARGET_MOUSE, PAGE_DEVICE, 0x85, 0x01, &[])
}
/// PROFILE_READ.dpiStages(profile): arguments `[profile, DPI_STAGE_MAX]`, length 0x0a
pub fn read_dpi_stages(profile: u8) -> Spec {
    read(TARGET_MOUSE, PAGE_PROFILE, 0x81, 0x0a, &[profile, DPI_STAGE_MAX])
}
/// PROFILE_READ.activeStage(profile)
pub fn read_active_stage(profile: u8) -> Spec {
    read(TARGET_MOUSE, PAGE_PROFILE, 0x82, 0x02, &[profile])
}
/// PROFILE_READ.pollingRate(profile)
pub fn read_polling_rate(profile: u8) -> Spec {
    read(TARGET_MOUSE, PAGE_PROFILE, 0x80, 0x02, &[profile])
}
/// PROFILE_READ.liftOffDistance(profile)
pub fn read_lift_off(profile: u8) -> Spec {
    read(TARGET_MOUSE, PAGE_PROFILE, 0x88, 0x02, &[profile])
}

/// Order 110: PROFILE_READ.debounce(profile) - the DEVICE page, command 0x88, answer `[1]` = ms.
pub fn read_debounce(profile: u8) -> Spec {
    read(TARGET_MOUSE, PAGE_DEVICE, 0x88, 0x02, &[profile])
}
/// Order 110: PROFILE_READ of a profile-page flag, answer `[1]` = 1 when on.
pub fn read_flag(profile: u8, command: u8) -> Spec {
    read(TARGET_MOUSE, PAGE_PROFILE, command, 0x02, &[profile])
}
/// Order 110: the profile-page flags as (setting, WRITE command, PROFILE_READ command), card order: motionSync 0x09 / 0x89,
/// angleSnapping 0x04 / 0x84, rippleControl 0x0a / 0x8a, turboMode 0x13 / 0x93 (the client's own "turbo mode": the sensor
/// pinned at 20K FPS).
pub const FLAGS: [(Extra, u8, u8); 4] = [(Extra::MotionSync, 0x09, 0x89), (Extra::AngleSnap, 0x04, 0x84), (Extra::RippleControl, 0x0a, 0x8a), (Extra::Turbo, 0x13, 0x93)];
/// `DEBOUNCE_MAX_MS` (setDebounceTime: 0-15 ms; write = device page 0x08).
pub const DEBOUNCE_MAX_MS: i32 = 15;

/// The Order 110 rows in the card's order, values `None` = not read.
fn extra_rows(debounce: Option<i32>, flags: [Option<bool>; 4]) -> Vec<ExtraRow> {
    let mut v = vec![ExtraRow::new(Extra::Debounce, Kind::Range { min: 0, max: DEBOUNCE_MAX_MS, step: 1, unit: "ms" }, debounce)];
    v.extend(FLAGS.iter().zip(flags).map(|(f, on)| ExtraRow::switch(f.0, on)));
    v
}

/// The client's `write(page, command, profile, values)`: arguments `[profile, ...values]`, length = their count.
pub fn write_spec(page: u8, command: u8, profile: u8, values: &[u8]) -> Spec {
    let mut args = vec![profile];
    args.extend_from_slice(values);
    Spec { target: TARGET_MOUSE, page, command, length: args.len() as u8, args, attempts: None }
}

/// compaxEncodeRequest: the 64-byte packet (status 0, target, length, page, command, arguments).
pub fn encode_request(s: &Spec) -> [u8; PACKET_LEN] {
    let mut p = [0u8; PACKET_LEN];
    p[2] = s.target;
    p[3] = s.length;
    p[4] = s.page;
    p[5] = s.command;
    let n = s.args.len().min(PACKET_LEN - HEADER_LEN);
    p[HEADER_LEN..HEADER_LEN + n].copy_from_slice(&s.args[..n]);
    p
}

/// The packet as Windows sends it: report id 0, the packet, zeros up to the collection's feature length.
pub fn report(s: &Spec, feature_len: usize) -> Vec<u8> {
    let mut v = vec![0u8; feature_len.max(REPORT_LEN)];
    v[0] = REPORT_ID;
    v[1..REPORT_LEN].copy_from_slice(&encode_request(s));
    v
}

/// How one answer reads for a request.
#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    /// status ok: the payload
    Ok(Vec<u8>),
    /// status 0xa2: the mouse does not know the command
    Unsupported,
    /// not this request's answer (another command, pending, busy): ask again
    Skip,
}

/// OpenMouse's exchange check on an answer WITHOUT its report id: at offset 0 or 1 the page and command must be echoed; then
/// status 0xa2 = unsupported, anything but 0xa1 = ask again, else the payload (declared length, at most up to byte 64).
pub fn decode_reply(s: &Spec, data: &[u8]) -> Reply {
    for off in [0usize, 1] {
        if data.get(4 + off) != Some(&s.page) || data.get(5 + off) != Some(&s.command) {
            continue;
        }
        match data.get(off) {
            Some(&STATUS_UNSUPPORTED) => return Reply::Unsupported,
            Some(&STATUS_OK) => {}
            _ => continue,
        }
        let start = HEADER_LEN + off;
        let n = (data.get(3 + off).copied().unwrap_or(0) as usize).min(PACKET_LEN - start);
        let end = (start + n).min(data.len());
        return Reply::Ok(data[start.min(end)..end].to_vec());
    }
    Reply::Skip
}

/// compaxDecodeDpiStages: `payload[1]` stages follow from byte 2, four bytes each (x, y; big-endian); a table cut short by
/// the payload ends the list.
pub fn decode_dpi_stages(payload: &[u8]) -> Vec<(u16, u16)> {
    let mut out = Vec::new();
    for i in 0..payload.get(1).copied().unwrap_or(0) as usize {
        let o = 2 + i * 4;
        if o + 3 >= payload.len() {
            break;
        }
        out.push((u16::from_be_bytes([payload[o], payload[o + 1]]), u16::from_be_bytes([payload[o + 2], payload[o + 3]])));
    }
    out
}

/// The stage table's own count byte (what a write back must keep).
pub fn declared_stage_count(payload: &[u8]) -> usize {
    payload.get(1).copied().unwrap_or(0) as usize
}

/// stageIndex: the reported 1-based stage, kept inside the table, as a 0-based index.
pub fn stage_index(reported: u8, count: usize) -> usize {
    (reported as usize).max(1).min(count.max(1)) - 1
}

/// The arguments of OpenMouse's writeStages after the profile: `[count, x hi, x lo, y hi, y lo, ...]`.
pub fn stage_table_bytes(stages: &[(u16, u16)]) -> Vec<u8> {
    let mut v = vec![stages.len() as u8];
    for (x, y) in stages {
        v.extend_from_slice(&x.to_be_bytes());
        v.extend_from_slice(&y.to_be_bytes());
    }
    v
}

/// decodePollingRate: the code's rate (`None` for a code the table does not know).
pub fn polling_hz(code: u8) -> Option<u32> {
    POLLING_CODES.iter().find(|(c, _)| *c == code).map(|(_, hz)| *hz)
}

/// The code OpenMouse writes for a rate (the first of the table).
pub fn polling_code(hz: u32) -> Option<u8> {
    POLLING_CODES.iter().find(|(_, h)| *h == hz).map(|(c, _)| *c)
}

/// The code OpenMouse writes for a lift-off distance in tenths of a mm.
pub fn lift_off_code(tenths: u32) -> Option<u8> {
    LIFT_OFF_CODES.iter().find(|(_, t)| *t == tenths).map(|(c, _)| *c)
}

/// compaxDecodeLiftOff, kept to what is certain: OpenMouse knows only the level (Low below 1 mm, Medium below 2, High), but
/// this mouse is only ever shown a distance it can be set to - 0x87 = 0.7 mm, 0x01 = 1 mm, 0x02 = 2 mm. Any other code
/// (0x85 = 0.5 mm, 0x03 = 3 mm, ...) is `None`, not rounded into one of the three.
pub fn lift_off_tenths(code: u8) -> Option<u32> {
    LIFT_OFF_CODES.iter().find(|(c, _)| *c == code).map(|(_, t)| *t)
}

/// `maxPollingRateHz`: the first "<digits> K" in the USB product name (regex `(\d+)\s*K\b`, case-insensitive) times 1000.
pub fn max_polling_from_name(name: &str) -> Option<u32> {
    let b = name.as_bytes();
    for i in 0..b.len() {
        if !b[i].is_ascii_digit() {
            continue;
        }
        let mut j = i;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        let digits = &name[i..j];
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if j < b.len() && (b[j] == b'K' || b[j] == b'k') && b.get(j + 1).is_none_or(|c| !(c.is_ascii_alphanumeric() || *c == b'_')) {
            return digits.parse::<u32>().ok().and_then(|n| n.checked_mul(1000));
        }
    }
    None
}

/// getSupportedPollingRates: the table's rates, ascending, cut at the model's maximum when it has one.
pub fn supported_polling(max: Option<u32>) -> Vec<u32> {
    POLLING_RATES.iter().copied().filter(|r| max.is_none_or(|m| *r <= m)).collect()
}

/// What the Mouse page offers: OpenMouse's DPI options (50 - 30000 in 50s), the polling rates, three lift-off levels (tenths
/// of a mm, see the header), battery.
pub fn caps(max_polling: Option<u32>) -> Caps {
    Caps {
        dpi: Some(DpiCaps::uniform(DPI_MIN, DPI_MAX, DPI_STEP)),
        polling: supported_polling(max_polling),
        lift_off: LIFT_OFF_CODES.iter().map(|(_, t)| *t).collect(),
        battery: true,
    }
}

/// The read-only proof's allow-list: a report id 0 and a packet that is exactly one of the READ requests above (profile reads
/// with any profile number from 1 up), zeros everywhere else. Every write (polling, lift-off, stage table) is refused.
pub fn read_request_allowed(out: &[u8]) -> bool {
    if out.len() < REPORT_LEN || out[0] != REPORT_ID || out[REPORT_LEN..].iter().any(|b| *b != 0) {
        return false;
    }
    let p = &out[1..REPORT_LEN];
    let profile = if p[HEADER_LEN] == 0 { 1 } else { p[HEADER_LEN] };
    let mut reads = vec![read_firmware(), read_battery(), read_active_profile(), read_dpi_stages(profile), read_active_stage(profile), read_polling_rate(profile), read_lift_off(profile), read_debounce(profile)];
    reads.extend(FLAGS.iter().map(|f| read_flag(profile, f.2)));
    reads
        .iter()
        .any(|s| encode_request(s)[..] == *p)
}

// ---------------------------------------------------------------- the device table

/// One WLMouse product id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Product {
    pub pid: u16,
    /// the model's name (OpenMouse's displayName: "WLmouse " + the table name)
    pub model: &'static str,
    /// the id is the 2.4 GHz receiver (`false` = the cable)
    pub wireless: bool,
}

const fn p(pid: u16, model: &'static str, wireless: bool) -> Product {
    Product { pid, model, wireless }
}

/// OpenMouse's SUPPORTED list (supported-devices-data.ts, status "supported"), each pair (receiver, cable) from
/// `WLMOUSE_PRODUCTS` (vendors.js). Left out: Beast X 4K (0xa887, "pr", its own report-4 protocol), the 1K receiver (0xa882)
/// and the catalogue's other models (Beast G, Huan, Beast Miao, Beast Mini Pro, Strider, Ying) - the driver knows them, the
/// list does not.
pub const PRODUCTS: [Product; 10] = [
    p(0xa883, "WLmouse Beast X", true),
    p(0xa884, "WLmouse Beast X", false),
    p(0xa885, "WLmouse Beast Mini", true),
    p(0xa886, "WLmouse Beast Mini", false),
    p(0xa870, "WLmouse Beast X Pro", true),
    p(0xa871, "WLmouse Beast X Pro", false),
    p(0xa878, "WLmouse Sword X", true),
    p(0xa879, "WLmouse Sword X", false),
    p(0xa880, "WLmouse Beast Max", true),
    p(0xa881, "WLmouse Beast Max", false),
];

pub fn product(pid: u16) -> Option<&'static Product> {
    PRODUCTS.iter().find(|p| p.pid == pid)
}

/// OpenMouse's isSupported: the vendor id and a collection with a feature report on id 0 (any usage). On Windows a collection
/// shows its feature report as `feature_len`: 65 = report id + 64 bytes. A vendor-defined collection (usage page 0xFFxx) wins
/// when several fit. Unclear: WebHID does not say which collection OpenMouse ends up with, and no descriptor of these mice was
/// available - so the collection is picked by the report length, not by a usage page.
pub fn config_interface<'a>(own: &[&'a HidInfo]) -> Option<&'a HidInfo> {
    own.iter().filter(|h| h.feature_len as usize >= REPORT_LEN).min_by_key(|h| h.usage_page < 0xFF00).copied()
}

/// VID:PID of a listed model + its config collection (`variant` = the product id).
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID {
        return None;
    }
    let p = product(pid)?;
    let config = config_interface(own)?;
    Some(Matched { family: Family::WlMouse, model: Some(p.model), wireless: Some(p.wireless), config: config.clone(), variant: pid })
}

// ---------------------------------------------------------------- the exchange

/// One request -> its payload, the way OpenMouse's `exchange` does it. `limit` caps the tries (the read-only proof asks the
/// wake question once). An unanswered try (the fetch timed out) counts like a wrong answer; `Err(BadAnswer)` at the end.
fn exchange(port: &mut dyn Port, s: &Spec, limit: u32) -> Result<Vec<u8>> {
    let len = (port.info().feature_len as usize).max(REPORT_LEN);
    let out = report(s, len);
    let how = HidTransfer::Feature { reply_id: REPORT_ID, reply_len: len, wait_ms: RESPONSE_DELAY_MS };
    let tries = s.attempts.unwrap_or(ATTEMPTS).min(limit).max(1);
    let what = format!("WLmouse page {:#04x} command {:#04x}", s.page, s.command);
    for attempt in 0..tries {
        match port.exchange(&out, &how) {
            // the answer without its report id (OpenMouse's receiveFeatureReport data)
            Ok(a) => match decode_reply(s, a.get(1..).unwrap_or(&[])) {
                Reply::Ok(payload) => return Ok(payload),
                Reply::Unsupported => return Err(Error::BadAnswer(format!("{what} is not supported by this mouse"))),
                Reply::Skip => {}
            },
            Err(Error::BadAnswer(_)) => {}
            Err(e) => return Err(e),
        }
        port.pause_ms(if attempt < QUICK_ATTEMPTS { RESPONSE_DELAY_MS as u64 } else { WAKE_DELAY_MS });
    }
    Err(Error::BadAnswer(format!("{what} got no answer, the mouse may be asleep or out of range")))
}

fn ask(port: &mut dyn Port, s: &Spec) -> Result<Vec<u8>> {
    exchange(port, s, ATTEMPTS)
}

/// `request(...).catch(() => null)`: no answer / unsupported = `None`; anything else (a mouse that is gone) still fails.
fn optional<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(Error::BadAnswer(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

/// currentProfile: the active profile (never below 1).
fn current_profile(port: &mut dyn Port) -> Result<u8> {
    Ok(ask(port, &read_active_profile())?.first().copied().unwrap_or(1).max(1))
}

fn read_stages(port: &mut dyn Port, profile: u8) -> Result<(Vec<(u16, u16)>, usize)> {
    let payload = ask(port, &read_dpi_stages(profile))?;
    Ok((decode_dpi_stages(&payload), declared_stage_count(&payload)))
}

fn read_active_stage_index(port: &mut dyn Port, profile: u8, count: usize) -> Result<usize> {
    let a = ask(port, &read_active_stage(profile))?;
    let reported = *a.get(1).ok_or_else(|| Error::BadAnswer("WLmouse: a short active-stage answer".into()))?;
    Ok(stage_index(reported, count))
}

fn read_polling(port: &mut dyn Port, profile: u8) -> Result<Option<u32>> {
    Ok(ask(port, &read_polling_rate(profile))?.get(1).copied().and_then(polling_hz))
}

fn read_lift_off_level(port: &mut dyn Port, profile: u8) -> Result<Option<u32>> {
    Ok(ask(port, &read_lift_off(profile))?.get(1).copied().and_then(lift_off_tenths))
}

fn max_polling(m: &Matched) -> Option<u32> {
    m.config.product.as_deref().and_then(max_polling_from_name)
}

// ---------------------------------------------------------------- the driver

/// The WLMouse driver (WLMouseHidClient).
pub struct WlMouse;

impl Driver for WlMouse {
    /// READ requests only: OpenMouse's readStatus, cut to what the page shows - firmware (the "is it answering?" call; the
    /// only request tried `online_tries` times), battery, active profile, the stage table + active stage (DPI), polling,
    /// lift-off. A value the mouse does not answer is left out; a mouse that does not answer the first call is asleep.
    fn read(&self, m: &Matched, port: &mut dyn Port, opt: ReadOptions) -> Result<OnMouse> {
        let mut r = OnMouse { model: None, online: false, wireless: m.wireless, caps: caps(max_polling(m)), ..Default::default() };
        if optional(exchange(port, &read_firmware(), opt.online_tries))?.is_none() {
            return Ok(r);
        }
        r.online = true;
        if let Some(b) = optional(ask(port, &read_battery()))? {
            r.charging = b.first().map(|c| *c == 1);
            r.battery_percent = b.get(1).copied().filter(|p| *p <= 100);
        }
        // a profile read that fails reads profile 1, as OpenMouse does
        let profile = optional(ask(port, &read_active_profile()))?.and_then(|a| a.first().copied()).unwrap_or(1).max(1);
        if let Some((stages, _)) = optional(read_stages(port, profile))? {
            if let Some(active) = optional(read_active_stage_index(port, profile, stages.len()))? {
                r.dpi = stages.get(active).map(|s| s.0 as u32);
            }
        }
        r.polling_hz = optional(read_polling(port, profile))?.flatten();
        r.lift_off = optional(read_lift_off_level(port, profile))?.flatten();
        // Order 110: debounce, then angle snapping, Motion Sync, ripple, turbo (readStatus' order), each `.catch(() => null)`
        let debounce = optional(ask(port, &read_debounce(profile)))?.and_then(|a| a.get(1).map(|b| *b as i32));
        let mut flags = [None; 4];
        for i in [1usize, 0, 2, 3] {
            flags[i] = optional(ask(port, &read_flag(profile, FLAGS[i].2)))?.and_then(|a| a.get(1).map(|b| *b == 1));
        }
        // a setting the mouse did not answer (unsupported on this model) gets no row, as OpenMouse hides its control
        r.extras = extra_rows(debounce, flags).into_iter().filter(|x| x.value.is_some()).collect();
        Ok(r)
    }

    /// Order 110: setDebounceTime / setMotionSync / setAngleSnapping / setRippleControl / setTurboMode - checked first, then
    /// the active profile, the write, the read-back (`[1]`) must hold the value.
    fn change(&self, _m: &Matched, port: &mut dyn Port, c: Change) -> Result<()> {
        let Change::Extra(e, v) = c;
        check_extra(&extra_rows(None, [None; 4]), e, v)?;
        let profile = current_profile(port)?;
        let (write, read) = match FLAGS.iter().find(|f| f.0 == e) {
            Some(f) => (write_spec(PAGE_PROFILE, f.1, profile, &[v as u8]), read_flag(profile, f.2)),
            None => (write_spec(PAGE_DEVICE, 0x08, profile, &[v as u8]), read_debounce(profile)),
        };
        ask(port, &write)?;
        // OpenMouse's confirm: a flag reads on when its byte === 1 (anything else = off); debounce is the byte itself
        let now = ask(port, &read)?.get(1).copied().map(|b| if e == Extra::Debounce { b as i32 } else { (b == 1) as i32 });
        if now != Some(v) {
            return Err(Error::BadAnswer(format!("the mouse kept {now:?} instead of {v} ({})", e.label())));
        }
        Ok(())
    }

    /// OpenMouse's setPollingRate / setLiftOffDistance / setDpi: the active profile first, the write, the read-back that is
    /// compared with the request. Before the write, one read of the same value for undo ("before"); a failed one stops the
    /// change.
    fn write(&self, m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        match s {
            Setting::Polling(hz) => {
                let code = polling_code(hz).filter(|_| supported_polling(max_polling(m)).contains(&hz));
                let code = code.ok_or_else(|| Error::range("polling rate", format!("this mouse does not support {hz} Hz")))?;
                let profile = current_profile(port)?;
                let before = read_polling(port, profile)?;
                ask(port, &write_spec(PAGE_PROFILE, 0x00, profile, &[code]))?;
                let now = read_polling(port, profile)?;
                if now != Some(hz) {
                    return Err(Error::BadAnswer(format!("the mouse kept {now:?} Hz instead of {hz} Hz")));
                }
                Ok(Changed { now: hz, before })
            }
            Setting::LiftOff(t) => {
                let code = lift_off_code(t).ok_or_else(|| Error::range("lift-off distance", format!("{t} tenths of a mm is not one of this mouse's lift-off levels (0.7, 1, 2 mm)")))?;
                let profile = current_profile(port)?;
                let before = read_lift_off_level(port, profile)?;
                ask(port, &write_spec(PAGE_PROFILE, 0x08, profile, &[code]))?;
                let now = read_lift_off_level(port, profile)?;
                if now != Some(t) {
                    return Err(Error::BadAnswer(format!("the mouse kept a lift-off of {now:?} tenths of a mm instead of {t}")));
                }
                Ok(Changed { now: t, before })
            }
            Setting::Dpi(dpi) => {
                if !caps(None).dpi.is_some_and(|d| d.allows(dpi)) {
                    return Err(Error::range("DPI", format!("{dpi} DPI is not a supported DPI value (50 - 30000 in steps of 50)")));
                }
                let profile = current_profile(port)?;
                let payload = ask(port, &read_dpi_stages(profile))?;
                let mut stages = decode_dpi_stages(&payload);
                let count = declared_stage_count(&payload);
                if count == 0 || count > DPI_STAGE_MAX as usize || stages.len() != count {
                    return Err(Error::BadAnswer(format!("the mouse's DPI stage table could not be read whole ({} of {count} stages) - changing the DPI would rewrite it wrongly", stages.len())));
                }
                let active = read_active_stage_index(port, profile, stages.len())?;
                let before = stages[active];
                if before.0 != before.1 {
                    return Err(Error::range("DPI", "the active DPI stage has separate X and Y values; changing it here would overwrite Y - use WLmouse's own software".to_string()));
                }
                stages[active] = (dpi as u16, dpi as u16);
                ask(port, &write_spec(PAGE_PROFILE, 0x01, profile, &stage_table_bytes(&stages)))?;
                let (now, _) = read_stages(port, profile)?;
                if now.get(active) != Some(&(dpi as u16, dpi as u16)) {
                    return Err(Error::BadAnswer(format!("the mouse kept {:?} DPI instead of {dpi}", now.get(active).map(|s| s.0))));
                }
                Ok(Changed { now: dpi, before: Some(before.0 as u32) })
            }
        }
    }
}

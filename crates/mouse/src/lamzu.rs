//! Lamzu - the "CompX" page-command protocol: 64-byte feature reports on report id 0, ported from OpenMouse
//! (`crate::family`: AGPL-3.0-or-later), DPI / polling / lift-off / battery only:
//!
//! - the driver: `drivers/lamzu/hid.js` (LamzuHidClient: isSupported / readStatus / setDpi / setPollingRate /
//!   setLiftOffDistance / currentProfile / request / exchange);
//! - the framing and decoders: `compx/codec.js` (compaxEncodeRequest, compaxDecodeDpiStages, compaxDecodePollingRate,
//!   compaxDecodeLiftOff, the status bytes) - the framing WLMouse's receivers share;
//! - the device table: `lamzu/index.js` (LAMZU_PRODUCTS under the shared CompX vendor id 0x373e, LAMZU_INCA_PRODUCTS under
//!   Lamzu's own 0x37b0, LAMZU_POLLING_RATES, `lamzuProduct`) for the models on OpenMouse's SUPPORTED list.
//!
//! One OpenMouse client drives all of them: LamzuHidClient claims a device whose vendor id is 0x373e or 0x37b0, whose
//! product id is in that vendor's catalog, and that has a feature report with id 0 on some collection. The catalog gives the
//! model's brand, which is why CRDRAKO's KO-ONE, LunaFury's LUNA33 and Attack Shark's R5 Ultra (all 0x373e) are this driver
//! too (`deviceBrand()` / `displayName()`): the model is carried in `Matched::variant` (an index into [`PRODUCTS`]).
//!
//! Packet (64 bytes, OpenMouse counts WITHOUT the report id; Windows puts report id 0 in front, so OpenMouse's byte i is our
//! i + 1): request `[0]` status 0 · `[1]` 0 · `[2]` target (0 = the receiver, 2 = the mouse; a model may override the mouse
//! target) · `[3]` length (data bytes; for a read it is the length of the answer wanted) · `[4]` page (0 device, 1 profile)
//! · `[5]` command (high bit set = a read) · `[6..]` arguments. Answer: `[0]` status (0xa0 pending, 0xa1 ok, 0xa2 not
//! supported, 0xa3 busy) · `[3]` data length · `[4]` page and `[5]` command echoed · `[6..6+len]` data. A request goes out with
//! HidD_SetFeature; the answer is fetched with HidD_GetFeature up to 12 times (30 ms apart for the first three, 300 ms after;
//! no re-send) until one has status ok AND the echoed page + command - an answer to some earlier request is skipped.
//!
//! Profile: every profile-page command names a profile. OpenMouse asks the mouse which profile is active (device page 0x85)
//! before each write and addresses that one (a write to profile 1 is ignored while another is active).
//!
//! DPI: OpenMouse's setDpi reads the active profile's stage table, replaces the ACTIVE stage's x and y with the new value,
//! and writes the whole table back (count, then x/y of every stage, big-endian) - the other stages are copied byte for byte
//! from what was read. Then it reads the table back. That read-modify-write is the same here.
//!
//! Lift-off: OpenMouse shows Low / Medium / High only, but the protocol value is in millimetres (compaxDecodeLiftOff: bit 7
//! set = tenths of a mm, else whole mm; Low 0x87 = 0.7 mm, Medium 0x01 = 1 mm, High 0x02 = 2 mm): the three settings OpenMouse
//! can write are 7 / 10 / 20 tenths here. (Decision for the lead: the labels carry no mm, the wire value does.)
//!
//! Order 110 (A_110_02 scope): debounce (device page: write 0x08 `[profile, ms]` 0-15 ms, read 0x88), Motion Sync (profile
//! page 0x09 / 0x89), angle snapping (0x04 / 0x84), ripple control (0x0a / 0x8a): OpenMouse's setFlag / setDebounceTime -
//! the active profile asked first, the write, the read-back (answer byte 1) must hold the value. Each read is optional.
//!
//! Left out on purpose: sleep timeout, DPI-stage count, competitive / hyper mode, the dongle LED, the LunaFury controls (Lightning
//! Trigger, per-button latency, wheel guard, sensor angle) and the magnetic-switch commands (RAWM Leviathan V4 GT only, not
//! on the list). The firmware reads are sent only as "is it answering" (the mouse's, as readStatus does first); the receiver's
//! firmware is not read. OpenMouse sends no firmware-update, DFU or reset command to these mice and neither does this file.
//! The Atlantis client (`drivers/lamzu-atlantis/hid.js`) is not ported: its only list entry is "likely", not "supported".

use crate::error::{Error, Result};
use crate::family::{check_extra, Caps, Change, Changed, DpiCaps, Driver, Extra, ExtraRow, Family, Kind, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

/// The shared CompX ODM vendor id (Lamzu's older models, CRDRAKO, LunaFury, Attack Shark ...).
pub const VENDOR_COMPX: u16 = 0x373e;
/// Lamzu's own vendor id (the Inca 8K).
pub const VENDOR_LAMZU: u16 = 0x37b0;

pub const REPORT_ID: u8 = 0;
pub const PACKET_LEN: usize = 64;
/// Windows' feature report: report id 0 + the 64-byte packet.
pub const REPORT_LEN: usize = PACKET_LEN + 1;
pub const HEADER_LEN: usize = 6;

pub const STATUS_PENDING: u8 = 0xa0;
pub const STATUS_OK: u8 = 0xa1;
pub const STATUS_UNSUPPORTED: u8 = 0xa2;
pub const STATUS_BUSY: u8 = 0xa3;

/// TARGET: the receiver, and the mouse behind it.
pub const TARGET_DONGLE: u8 = 0x00;
pub const TARGET_MOUSE: u8 = 0x02;
/// PAGE: device and profile.
pub const PAGE_DEVICE: u8 = 0x00;
pub const PAGE_PROFILE: u8 = 0x01;

/// RESPONSE_ATTEMPTS / RESPONSE_DELAY_MS / WAKE_DELAY_MS / QUICK_ATTEMPTS.
const RESPONSE_ATTEMPTS: u32 = 12;
const RESPONSE_DELAY_MS: u32 = 30;
const WAKE_DELAY_MS: u32 = 300;
const QUICK_ATTEMPTS: u32 = 3;

/// DPI_STEP and the default DPI ceiling (DPI_MAX); a model may have its own ceiling.
const DPI_STEP: u32 = 50;

/// LAMZU_POLLING_RATES: (code, Hz). 1000 Hz has two codes; the first is the one OpenMouse writes.
const POLLING_CODES: [(u8, u32); 8] = [(0x08, 125), (0x04, 250), (0x02, 500), (0x01, 1000), (0x10, 1000), (0x20, 2000), (0x40, 4000), (0x80, 8000)];

/// LIFT_OFF_DISTANCES as OpenMouse writes them: (value, tenths of a mm). Low 0x87, Medium 0x01, High 0x02.
const LIFT_OFF_CODES: [(u8, u32); 3] = [(0x87, 7), (0x01, 10), (0x02, 20)];

const RATES_1K: &[u32] = &[125, 250, 500, 1000];
const RATES_8K: &[u32] = &[500, 1000, 2000, 4000, 8000];
const RATES_8K_FULL: &[u32] = &[125, 250, 500, 1000, 2000, 4000, 8000];

/// One catalog row (LAMZU_PRODUCTS / LAMZU_INCA_PRODUCTS) for a model on OpenMouse's SUPPORTED list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Product {
    pub vid: u16,
    pub pid: u16,
    /// OpenMouse's displayName: brand + model
    pub model: &'static str,
    pub wireless: bool,
    /// the polling rates `getSupportedPollingRates` lists (ascending)
    pub rates: &'static [u32],
    /// `mouseTarget` (default 0x02)
    pub mouse_target: u8,
    /// `maxDpi` (default 30000)
    pub max_dpi: u32,
    /// `maxDpiStages` (default 6) - the second argument of the stage-table read
    pub max_stages: u8,
}

const fn row(vid: u16, pid: u16, model: &'static str, wireless: bool, rates: &'static [u32]) -> Product {
    Product { vid, pid, model, wireless, rates, mouse_target: TARGET_MOUSE, max_dpi: 30000, max_stages: 6 }
}

const fn row_target(vid: u16, pid: u16, model: &'static str, wireless: bool, rates: &'static [u32], mouse_target: u8) -> Product {
    Product { mouse_target, ..row(vid, pid, model, wireless, rates) }
}

const fn row_dpi(p: Product, max_dpi: u32) -> Product {
    Product { max_dpi, ..p }
}

/// The device table: every "supported" Lamzu-client model of OpenMouse's list (supported-devices-data.ts), with the ids,
/// rates and overrides of `lamzu/index.js`. Index = `Matched::variant`.
pub const PRODUCTS: [Product; 12] = [
    // Lamzu Maya X (list pids 0x1c, 0x1d, 0x1e): the cable and the 1K and 8K receivers
    row(VENDOR_COMPX, 0x001c, "Lamzu Maya X", false, RATES_1K),
    row(VENDOR_COMPX, 0x001d, "Lamzu Maya X", true, RATES_1K),
    row(VENDOR_COMPX, 0x001e, "Lamzu Maya X", true, RATES_8K),
    // Lamzu Inca (list pids 0x9, 0xf, 0x10), under Lamzu's own vendor id; OpenMouse's model name is "Inca 8K"
    row(VENDOR_LAMZU, 0x0009, "Lamzu Inca 8K", false, RATES_1K),
    row(VENDOR_LAMZU, 0x000f, "Lamzu Inca 8K", true, RATES_1K),
    row(VENDOR_LAMZU, 0x0010, "Lamzu Inca 8K", true, RATES_8K),
    // CRDRAKO KO-ONE (list pids 0x6a, 0x6b): the cable answers on target 0, the receiver on 2
    row_target(VENDOR_COMPX, 0x006a, "CRDRAKO KO-ONE", false, RATES_8K_FULL, TARGET_DONGLE),
    row_target(VENDOR_COMPX, 0x006b, "CRDRAKO KO-ONE", true, RATES_8K_FULL, TARGET_MOUSE),
    // LunaFury LUNA33 (list pids 0x32, 0x33): maxDpi 30000, 6 stages
    row_target(VENDOR_COMPX, 0x0032, "LunaFury LUNA33", false, RATES_1K, TARGET_DONGLE),
    row(VENDOR_COMPX, 0x0033, "LunaFury LUNA33", true, RATES_8K_FULL),
    // Attack Shark R5 Ultra (list pids 0x46, 0x47): also this client (it is first in OpenMouse's registry); maxDpi 42000.
    row_dpi(row(VENDOR_COMPX, 0x0046, "Attack Shark R5 Ultra", false, RATES_1K), 42000),
    row_dpi(row(VENDOR_COMPX, 0x0047, "Attack Shark R5 Ultra", true, RATES_8K_FULL), 42000),
];

/// The row for a vendor id + product id (`lamzuProduct`: a vendor id this family does not use resolves to nothing, and the
/// same product id under the other vendor id is another device).
pub fn product(vid: u16, pid: u16) -> Option<&'static Product> {
    PRODUCTS.iter().find(|p| p.vid == vid && p.pid == pid)
}

/// The row of a [`Matched::variant`].
pub fn product_of_variant(variant: u16) -> Option<&'static Product> {
    PRODUCTS.get(variant as usize)
}

// ---------------------------------------------------------------- requests (compaxEncodeRequest)

/// One request: OpenMouse's spec `{target, page, command, length, args}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Req {
    pub target: u8,
    pub page: u8,
    pub command: u8,
    pub length: u8,
    pub args: Vec<u8>,
}

fn req(page: u8, command: u8, length: u8, args: &[u8]) -> Req {
    Req { target: TARGET_MOUSE, page, command, length, args: args.to_vec() }
}

/// READ.firmware {mouse, device, 0x81, 0x10}.
pub fn read_firmware() -> Req {
    req(PAGE_DEVICE, 0x81, 0x10, &[])
}

/// READ.battery {mouse, device, 0x83, 2}: answer `[0]` = 1 when charging, `[1]` = percent.
pub fn read_battery() -> Req {
    req(PAGE_DEVICE, 0x83, 0x02, &[])
}

/// READ.activeProfile {mouse, device, 0x85, 1}: answer `[0]` = the active profile.
pub fn read_active_profile() -> Req {
    req(PAGE_DEVICE, 0x85, 0x01, &[])
}

/// PROFILE_READ.dpiStages(profile, maxStages) {mouse, profile page, 0x81, 0x0a, [profile, maxStages]}.
pub fn read_dpi_stages(profile: u8, max_stages: u8) -> Req {
    req(PAGE_PROFILE, 0x81, 0x0a, &[profile, max_stages])
}

/// PROFILE_READ.activeStage(profile) {mouse, profile page, 0x82, 2, [profile]}: answer `[1]` = the 1-based active stage.
pub fn read_active_stage(profile: u8) -> Req {
    req(PAGE_PROFILE, 0x82, 0x02, &[profile])
}

/// PROFILE_READ.pollingRate(profile) {mouse, profile page, 0x80, 2, [profile]}: answer `[1]` = a polling code.
pub fn read_polling(profile: u8) -> Req {
    req(PAGE_PROFILE, 0x80, 0x02, &[profile])
}

/// PROFILE_READ.liftOffDistance(profile) {mouse, profile page, 0x88, 2, [profile]}: answer `[1]` = the lift-off value.
pub fn read_lift_off(profile: u8) -> Req {
    req(PAGE_PROFILE, 0x88, 0x02, &[profile])
}

/// Order 110: PROFILE_READ.debounce(profile) {mouse, DEVICE page, 0x88, 2, [profile]}: answer `[1]` = ms.
pub fn read_debounce(profile: u8) -> Req {
    req(PAGE_DEVICE, 0x88, 0x02, &[profile])
}

/// Order 110: write(device page, WRITE.debounce = 0x08, profile, [ms]).
pub fn write_debounce(profile: u8, ms: u8) -> Req {
    req(PAGE_DEVICE, 0x08, 2, &[profile, ms])
}

/// Order 110: the profile-page flags as (setting, write command, read command): WRITE / PROFILE_READ motionSync 0x09 / 0x89,
/// angleSnapping 0x04 / 0x84, rippleControl 0x0a / 0x8a.
pub const FLAGS: [(Extra, u8, u8); 3] = [(Extra::MotionSync, 0x09, 0x89), (Extra::AngleSnap, 0x04, 0x84), (Extra::RippleControl, 0x0a, 0x8a)];
/// `DEBOUNCE_MAX_MS`
pub const DEBOUNCE_MAX_MS: i32 = 15;

/// Order 110: PROFILE_READ of a flag {mouse, profile page, read command, 2, [profile]}: answer `[1]` = 1 when on.
pub fn read_flag(profile: u8, command: u8) -> Req {
    req(PAGE_PROFILE, command, 0x02, &[profile])
}

/// Order 110: write(profile page, flag command, profile, [on]).
pub fn write_flag(profile: u8, command: u8, on: bool) -> Req {
    req(PAGE_PROFILE, command, 2, &[profile, on as u8])
}

/// The Order 110 rows in the card's order (Debounce, Motion Sync, angle snapping, ripple), values `None` = not read.
fn rows(debounce: Option<i32>, flags: [Option<bool>; 3]) -> Vec<ExtraRow> {
    let mut v = vec![ExtraRow::new(Extra::Debounce, Kind::Range { min: 0, max: DEBOUNCE_MAX_MS, step: 1, unit: "ms" }, debounce)];
    v.extend(FLAGS.iter().zip(flags).map(|(f, on)| ExtraRow::switch(f.0, on)));
    v
}

/// write(profile page, WRITE.pollingRate = 0x00, profile, [code]): length = the argument count.
pub fn write_polling(profile: u8, code: u8) -> Req {
    req(PAGE_PROFILE, 0x00, 2, &[profile, code])
}

/// write(profile page, WRITE.liftOffDistance = 0x08, profile, [code]).
pub fn write_lift_off(profile: u8, code: u8) -> Req {
    req(PAGE_PROFILE, 0x08, 2, &[profile, code])
}

/// write(profile page, WRITE.dpiStages = 0x01, profile, [count, then x hi, x lo, y hi, y lo of every stage]).
pub fn write_dpi_stages(profile: u8, stages: &[(u16, u16)]) -> Req {
    let mut args = vec![profile, stages.len() as u8];
    for (x, y) in stages {
        args.extend_from_slice(&x.to_be_bytes());
        args.extend_from_slice(&y.to_be_bytes());
    }
    let length = args.len() as u8;
    Req { target: TARGET_MOUSE, page: PAGE_PROFILE, command: 0x01, length, args }
}

/// compaxEncodeRequest: the 64-byte packet (no report id). A request for the mouse goes to the model's `mouse_target`.
pub fn packet(r: &Req, mouse_target: u8) -> [u8; PACKET_LEN] {
    let mut p = [0u8; PACKET_LEN];
    p[0] = 0; // COMPX_STATUS.request
    p[2] = if r.target == TARGET_MOUSE { mouse_target } else { r.target };
    p[3] = r.length;
    p[4] = r.page;
    p[5] = r.command;
    let n = r.args.len().min(PACKET_LEN - HEADER_LEN);
    p[HEADER_LEN..HEADER_LEN + n].copy_from_slice(&r.args[..n]);
    p
}

/// The feature report Windows sends: report id 0, the packet, zero-padded to the collection's `len` (at least 65).
pub fn report(r: &Req, mouse_target: u8, len: usize) -> Vec<u8> {
    let mut v = vec![REPORT_ID];
    v.extend_from_slice(&packet(r, mouse_target));
    v.resize(len.max(REPORT_LEN), 0);
    v
}

// ---------------------------------------------------------------- answers

/// What one fetched answer is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// status ok with the request's page + command: the data bytes
    Data(Vec<u8>),
    /// pending, busy, or ok for another command: fetch again
    Again,
    /// status 0xa2
    Unsupported,
    /// any other status
    Status(u8),
}

/// The exchange loop's judgement of one 64-byte answer (no report id) to `r`.
pub fn decode_reply(p: &[u8], r: &Req) -> Reply {
    let status = p.first().copied().unwrap_or(0);
    if status == STATUS_UNSUPPORTED {
        return Reply::Unsupported;
    }
    if status == STATUS_OK && p.get(4) == Some(&r.page) && p.get(5) == Some(&r.command) {
        // the data length is the answer's own, at most 58
        let len = (p.get(3).copied().unwrap_or(0) as usize).min(PACKET_LEN - HEADER_LEN);
        let end = (HEADER_LEN + len).min(p.len());
        return Reply::Data(p.get(HEADER_LEN..end).unwrap_or(&[]).to_vec());
    }
    match status {
        STATUS_PENDING | STATUS_BUSY | STATUS_OK => Reply::Again,
        s => Reply::Status(s),
    }
}

/// compaxDecodeDpiStages: count at `[1]`, stage i (x, y big-endian) at 2 + 4i; stops where the data ends.
pub fn decode_dpi_stages(payload: &[u8]) -> Vec<(u16, u16)> {
    let mut stages = Vec::new();
    for i in 0..payload.get(1).copied().unwrap_or(0) as usize {
        let o = 2 + i * 4;
        if o + 3 >= payload.len() {
            break;
        }
        stages.push((u16::from_be_bytes([payload[o], payload[o + 1]]), u16::from_be_bytes([payload[o + 2], payload[o + 3]])));
    }
    stages
}

/// stageIndex: the mouse's 1-based stage, clamped into the table, as an index.
pub fn stage_index(reported: u8, count: usize) -> usize {
    (reported.max(1) as usize).min(count.max(1)) - 1
}

/// `Math.max(1, reply[0])`: the profile the mouse is running.
pub fn profile_of(reply: &[u8]) -> u8 {
    reply.first().copied().unwrap_or(0).max(1)
}

/// compaxDecodePollingRate: the code → Hz (`None` = a value OpenMouse throws on).
pub fn polling_hz(code: u8) -> Option<u32> {
    POLLING_CODES.iter().find(|c| c.0 == code).map(|c| c.1)
}

/// setPollingRate's `POLLING_RATES.find(([, hertz]) => hertz === rate)`: the FIRST code for a rate (1000 Hz → 0x01).
pub fn polling_code(hz: u32) -> Option<u8> {
    POLLING_CODES.iter().find(|c| c.1 == hz).map(|c| c.0)
}

/// compaxDecodeLiftOff in tenths of a mm: 0 = none; bit 7 set = tenths of a mm, else whole mm.
pub fn lift_off_tenths(value: u8) -> Option<u32> {
    match value {
        0 => None,
        // 0x80 would be "0 mm" with the bit set: not a real distance, so not shown as one
        v if v & 0x80 != 0 => Some((v & 0x7f) as u32).filter(|t| *t > 0),
        v => Some(v as u32 * 10),
    }
}

/// setLiftOffDistance's value for a setting OpenMouse can write (0.7 / 1 / 2 mm).
pub fn lift_off_code(tenths: u32) -> Option<u8> {
    LIFT_OFF_CODES.iter().find(|c| c.1 == tenths).map(|c| c.0)
}

/// readStatus' battery: percent when `[1]` <= 100 (else unknown), charging when `[0]` is 1.
pub fn battery(payload: &[u8]) -> (Option<u8>, Option<bool>) {
    (payload.get(1).copied().filter(|p| *p <= 100), payload.first().map(|c| *c == 1))
}

// ---------------------------------------------------------------- the device table and the matcher

fn has_feature_report(h: &HidInfo) -> bool {
    h.feature_len as usize >= REPORT_LEN
}

/// LamzuHidClient.isSupported wants a feature report with id 0 on some collection (the control channel; the Inca's is
/// 0xffff / 0x0000 on its third interface). On Windows that is the collection whose feature report is 65 bytes (id 0 + 64).
/// A vendor-defined collection is preferred when several carry one. (Unclear: the exact usage of the control collection on
/// the other models - OpenMouse checks only for the report id, and so does this.)
pub fn control_interface<'a>(own: &[&'a HidInfo]) -> Option<&'a HidInfo> {
    own.iter().filter(|h| has_feature_report(h)).min_by_key(|h| h.usage_page < 0xFF00).copied()
}

/// OpenMouse's Lamzu isSupported: a vendor id of this family, a product id of that vendor's catalog, and the control
/// collection. (Only the models of OpenMouse's SUPPORTED list are in the table.)
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    let p = product(vid, pid)?;
    let config = control_interface(own)?;
    let variant = PRODUCTS.iter().position(|q| q == p)? as u16;
    Some(Matched { family: Family::Lamzu, model: Some(p.model), wireless: Some(p.wireless), config: config.clone(), variant })
}

/// The allow-list for proofs on the real mouse (A_009_01): a well-formed feature report (report id 0, any zero padding)
/// carrying exactly one of the read requests this file sends, addressed to the receiver or the mouse (the target a model
/// answers on), for any profile number 1..=255. Every write (and every other read) is refused.
pub fn read_request_allowed(out: &[u8]) -> bool {
    if out.len() < REPORT_LEN || out[0] != REPORT_ID || out[REPORT_LEN..].iter().any(|b| *b != 0) {
        return false;
    }
    let p = &out[1..REPORT_LEN];
    // status 0, byte 1 unused, target
    if p[0] != 0 || p[1] != 0 || (p[2] != TARGET_DONGLE && p[2] != TARGET_MOUSE) {
        return false;
    }
    let (length, page, command, args) = (p[3], p[4], p[5], &p[HEADER_LEN..]);
    let zeros = |from: usize| args[from..].iter().all(|b| *b == 0);
    match (page, command, length) {
        // firmware, battery, active profile: no arguments
        (PAGE_DEVICE, 0x81, 0x10) | (PAGE_DEVICE, 0x83, 0x02) | (PAGE_DEVICE, 0x85, 0x01) => zeros(0),
        // stage table: [profile, maxDpiStages = 6 - the default every model on the list uses]
        (PAGE_PROFILE, 0x81, 0x0a) => args[0] >= 1 && args[1] == 6 && zeros(2),
        // active stage, polling rate, lift-off distance: [profile]
        (PAGE_PROFILE, 0x82, 0x02) | (PAGE_PROFILE, 0x80, 0x02) | (PAGE_PROFILE, 0x88, 0x02) => args[0] >= 1 && zeros(1),
        // Order 110: debounce (device page), Motion Sync / angle snapping / ripple (profile page): [profile]
        (PAGE_DEVICE, 0x88, 0x02) | (PAGE_PROFILE, 0x89, 0x02) | (PAGE_PROFILE, 0x84, 0x02) | (PAGE_PROFILE, 0x8a, 0x02) => args[0] >= 1 && zeros(1),
        _ => false,
    }
}

// ---------------------------------------------------------------- the exchange

fn refused(r: &Req, why: &str) -> Error {
    Error::BadAnswer(format!("Lamzu page {:#04x} command {:#04x} {why}", r.page, r.command))
}

/// One request → its answer's data, the way LamzuHidClient.exchange does it: send (HidD_SetFeature), fetch (HidD_GetFeature)
/// up to 12 times without sending again - 30 ms between the first fetches, 300 ms after (a sleeping mouse wakes) - until an
/// answer with status ok and the request's page + command arrives. "Not supported" and any other status end it at once.
fn request(port: &mut dyn Port, p: &Product, r: &Req) -> Result<Vec<u8>> {
    let len = (port.info().feature_len as usize).max(REPORT_LEN);
    let out = report(r, p.mouse_target, len);
    for attempt in 0..RESPONSE_ATTEMPTS {
        let how = if attempt == 0 {
            HidTransfer::Feature { reply_id: REPORT_ID, reply_len: len, wait_ms: 0 }
        } else {
            // the delay OpenMouse waits after attempt (n - 1): 30 ms while that was one of the first three
            let wait_ms = if attempt - 1 < QUICK_ATTEMPTS { RESPONSE_DELAY_MS } else { WAKE_DELAY_MS };
            HidTransfer::FeatureIn { reply_id: REPORT_ID, reply_len: len, wait_ms }
        };
        let a = port.exchange(&out, &how)?;
        // the answer's packet behind the report id (a longer collection's padding cut off)
        let pk = a.get(1..a.len().min(REPORT_LEN)).unwrap_or(&[]);
        match decode_reply(pk, r) {
            Reply::Data(d) => return Ok(d),
            Reply::Again => {}
            Reply::Unsupported => return Err(refused(r, "is not supported by this mouse")),
            Reply::Status(s) => return Err(refused(r, &format!("returned an unexpected status {s:#04x}"))),
        }
    }
    Err(refused(r, "got no answer - the mouse may be asleep or out of range"))
}

/// `request(...).catch(() => null)`: no answer / refused / the collection would not take it = `None`; a mouse that is gone
/// or a request the read-only layer refused still fails.
fn optional<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(Error::BadAnswer(_) | Error::Os { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

fn caps(p: &Product) -> Caps {
    Caps {
        dpi: Some(DpiCaps::uniform(DPI_STEP, p.max_dpi, DPI_STEP)),
        polling: p.rates.to_vec(),
        lift_off: LIFT_OFF_CODES.iter().map(|c| c.1).collect(),
        battery: true,
    }
}

fn the_product(m: &Matched) -> Result<&'static Product> {
    product_of_variant(m.variant).ok_or_else(|| Error::UnsupportedMouse(format!("Lamzu catalog entry {}", m.variant)))
}

/// currentProfile: asks the mouse which profile is active.
fn current_profile(port: &mut dyn Port, p: &Product) -> Result<u8> {
    Ok(profile_of(&request(port, p, &read_active_profile())?))
}

/// The stage table and the active stage's index (setDpi's two reads).
fn stage_table(port: &mut dyn Port, p: &Product, profile: u8) -> Result<(Vec<(u16, u16)>, usize)> {
    let payload = request(port, p, &read_dpi_stages(profile, p.max_stages))?;
    let stages = decode_dpi_stages(&payload);
    // the write sends the whole table back with count = stages.len(): a table that did not decode completely (the answer's
    // data ran out before its own count) would lose the stages behind it
    let count = payload.get(1).copied().unwrap_or(0) as usize;
    if count == 0 || count > p.max_stages as usize || count != stages.len() {
        return Err(Error::BadAnswer(format!("the mouse's DPI stage table is incomplete ({} of {count} stages read) - not rewritten", stages.len())));
    }
    let reported = request(port, p, &read_active_stage(profile))?.get(1).copied().unwrap_or(0);
    let active = stage_index(reported, stages.len());
    if stages.get(active).is_none() {
        return Err(Error::BadAnswer("the mouse did not report any DPI stages".into()));
    }
    Ok((stages, active))
}

fn polling_now(port: &mut dyn Port, p: &Product, profile: u8) -> Result<Option<u32>> {
    Ok(request(port, p, &read_polling(profile))?.get(1).copied().and_then(polling_hz))
}

fn lift_off_now(port: &mut dyn Port, p: &Product, profile: u8) -> Result<Option<u32>> {
    Ok(request(port, p, &read_lift_off(profile))?.get(1).copied().and_then(lift_off_tenths))
}

// ---------------------------------------------------------------- the driver

/// The Lamzu driver: LamzuHidClient.
pub struct Lamzu;

impl Driver for Lamzu {
    /// READ requests only: the reads OpenMouse's readStatus sends for DPI, polling, lift-off and battery (plus its first, the
    /// mouse's firmware version, as "is it answering"; no answer = asleep or out of range = `online: false`). Every one passes
    /// [`read_request_allowed`]. Order 110 adds its debounce and flag reads; its other reads (sleep, receiver firmware) are not sent. There is no
    /// wake loop in OpenMouse to count, so `online_tries` has nothing to limit: one request waits at most ~2.5 s.
    fn read(&self, m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let p = the_product(m)?;
        let mut r = OnMouse { wireless: Some(p.wireless), caps: caps(p), ..Default::default() };
        // no answer to the firmware read = the mouse is not there (OpenMouse: "did not report a firmware version")
        if optional(request(port, p, &read_firmware()))?.is_none() {
            return Ok(r);
        }
        r.online = true;
        // a battery read that fails drops only the battery
        if let Some(b) = optional(request(port, p, &read_battery()))? {
            let (percent, charging) = battery(&b);
            r.battery_percent = percent;
            r.charging = charging;
        }
        // profile-page commands address the profile the mouse is running (1 when it will not say)
        let profile = optional(request(port, p, &read_active_profile()))?.map(|a| profile_of(&a)).unwrap_or(1);
        if let Some(t) = optional(request(port, p, &read_dpi_stages(profile, p.max_stages)))? {
            let stages = decode_dpi_stages(&t);
            if let Some(a) = optional(request(port, p, &read_active_stage(profile)))? {
                r.dpi = stages.get(stage_index(a.get(1).copied().unwrap_or(0), stages.len())).map(|s| s.0 as u32);
            }
        }
        r.polling_hz = optional(polling_now(port, p, profile))?.flatten();
        r.lift_off = optional(lift_off_now(port, p, profile))?.flatten();
        // Order 110: debounce, then the flags (readStatus' order), each `.catch(() => null)`
        let debounce = optional(request(port, p, &read_debounce(profile)))?.and_then(|a| a.get(1).map(|b| *b as i32));
        let mut flags = [None; 3];
        for (i, f) in [1usize, 0, 2].into_iter().map(|i| (i, FLAGS[i])) {
            flags[i] = optional(request(port, p, &read_flag(profile, f.2)))?.and_then(|a| a.get(1).map(|b| *b == 1));
        }
        // a setting the mouse did not answer (unsupported on this model) gets no row, as OpenMouse hides its control
        r.extras = rows(debounce, flags).into_iter().filter(|x| x.value.is_some()).collect();
        Ok(r)
    }

    /// Order 110: setDebounceTime / setMotionSync / setAngleSnapping / setRippleControl - checked first, then the active
    /// profile, the write, the read-back.
    fn change(&self, m: &Matched, port: &mut dyn Port, c: Change) -> Result<()> {
        let p = the_product(m)?;
        let Change::Extra(e, v) = c;
        check_extra(&rows(None, [None; 3]), e, v)?;
        let profile = current_profile(port, p)?;
        let (write, read) = match FLAGS.iter().find(|f| f.0 == e) {
            Some(f) => (write_flag(profile, f.1, v == 1), read_flag(profile, f.2)),
            None => (write_debounce(profile, v as u8), read_debounce(profile)),
        };
        request(port, p, &write)?;
        // OpenMouse's confirm: a flag reads on when its byte === 1 (anything else = off); debounce is the byte itself
        let now = request(port, p, &read)?.get(1).copied().map(|b| if e == Extra::Debounce { b as i32 } else { (b == 1) as i32 });
        if now != Some(v) {
            return Err(Error::BadAnswer(format!("the mouse kept {now:?} instead of {v} ({})", e.label())));
        }
        Ok(())
    }

    /// setDpi / setPollingRate / setLiftOffDistance the way LamzuHidClient does them: ask for the active profile, the write,
    /// then the read-back it confirms with. (Polling and lift-off read the old value once first, for undo - a failed read stops
    /// the change; setDpi's own reads already give it.)
    fn write(&self, m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        let p = the_product(m)?;
        match s {
            Setting::Dpi(dpi) => {
                if !caps(p).dpi.is_some_and(|d| d.allows(dpi)) {
                    return Err(Error::range("DPI", format!("{dpi} DPI is not one this mouse can store (50 - {} in steps of 50)", p.max_dpi)));
                }
                let profile = current_profile(port, p)?;
                // the stage table and the active stage; the active stage's x and y replaced, the others written back as read
                let (mut stages, active) = stage_table(port, p, profile)?;
                // X and Y are one number here (setDpi(dpi) sets both): a stage with a separate Y is not flattened
                if stages[active].0 != stages[active].1 {
                    return Err(Error::range("DPI", format!("the active stage has a separate Y ({} x {}), which this would overwrite", stages[active].0, stages[active].1)));
                }
                let before = stages[active].0 as u32;
                stages[active] = (dpi as u16, dpi as u16);
                request(port, p, &write_dpi_stages(profile, &stages))?;
                let now = decode_dpi_stages(&request(port, p, &read_dpi_stages(profile, p.max_stages))?).get(active).copied();
                match now {
                    Some(n) if n == (dpi as u16, dpi as u16) => Ok(Changed { now: dpi, before: Some(before) }),
                    Some(n) => Err(Error::BadAnswer(format!("the mouse kept {} DPI instead of {dpi}", n.0))),
                    None => Err(Error::BadAnswer(format!("the mouse kept an unknown DPI instead of {dpi}"))),
                }
            }
            Setting::Polling(hz) => {
                let code = polling_code(hz).filter(|_| p.rates.contains(&hz)).ok_or_else(|| Error::range("polling rate", format!("this mouse does not support {hz} Hz")))?;
                let profile = current_profile(port, p)?;
                let before = polling_now(port, p, profile)?;
                request(port, p, &write_polling(profile, code))?;
                let now = polling_now(port, p, profile)?;
                if now != Some(hz) {
                    return Err(Error::BadAnswer(format!("the mouse kept {now:?} Hz instead of {hz} Hz")));
                }
                Ok(Changed { now: hz, before })
            }
            Setting::LiftOff(tenths) => {
                let code = lift_off_code(tenths).ok_or_else(|| Error::range("lift-off distance", format!("this mouse does not support {tenths} tenths of a mm")))?;
                let profile = current_profile(port, p)?;
                let before = lift_off_now(port, p, profile)?;
                request(port, p, &write_lift_off(profile, code))?;
                let now = lift_off_now(port, p, profile)?;
                if now != Some(tenths) {
                    return Err(Error::BadAnswer(format!("the mouse kept {now:?} tenths of a mm instead of {tenths}")));
                }
                Ok(Changed { now: tenths, before })
            }
        }
    }
}

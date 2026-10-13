//! Keychron - the Nape Pro (VIA raw HID) and the M6 (Keychron Launcher's "8k" protocol), ported from OpenMouse
//! (`crate::family`: AGPL-3.0-or-later), DPI / polling / lift-off / battery only:
//!
//! - the Nape Pro (pid 0x0440, wired): `drivers/keychron/nape-hid.js` (KeychronNapeHidClient) + `keychron/index.js` (the
//!   KEYCHRON_COMMAND / KEYCHRON_NAPE_COMMAND / KEYCHRON_MISC_COMMAND bytes, keychronDecodePolling / DecodeBattery,
//!   KEYCHRON_POLLING_TABLE, the 50-4000 DPI range). One 32-byte report on report id 0, usage page 0xFF60 / usage 0x61:
//!   `[0xa7, command, args...]`; the answer echoes `[0xa7, command]`. Writes are fire-and-forget (40 ms pause), confirmed by
//!   a read. The Nape has no lift-off.
//! - the M6 (pids 0xd060 wired, 0xd029 the Link-KM Type C receiver): `drivers/keychron/mouse-8k-hid.js`
//!   (Keychron8kHidClient) + `launcher-mouse.js` (keychronDecodeSettings / ActiveStage / ActiveGear / EncodeDpi /
//!   EncodeSensor / LiftOff, the model row `{ productId: 0xd060, "Keychron M6", dpi [100, 26000], maxPollingHz 1000, lod
//!   [[3, 0.7], [1, 1], [2, 2]] }` from `keychron/index.js` KEYCHRON_LAUNCHER_MICE). Usage page 0xFFC1 / usage 1: a
//!   63-byte command report 0xb3 (answers on 0xb4) and a 20-byte settings report 0xb5 (answers on 0xb6), both OUTPUT reports;
//!   a settings write is acknowledged `[0xe4, code, command]` (code 0 = ok, 7 = not supported on this connection).
//!   Behind the receiver the mouse is whatever is paired: OpenMouse names it from the receiver's own answers (the 0x02
//!   product id, else the 0x03 connected-mouse list), and so does this file - only an M6 (product id 0xd060) is driven, any
//!   other paired Keychron mouse is "not supported" (the other 51 Keychron rows on OpenMouse's list are status "pr").
//!
//! `variant` is the product id (0x0440, 0xd060, 0xd029). Both of the list's models are in the table: "Nape Pro" 0x0440 and
//! "M6" 0xd060 / 0xd029.
//!
//! Windows counts the report id: OpenMouse's byte i of a request or an answer is our byte i + 1 (the id is byte 0). A report
//! is zero-padded to the collection's output length (the M6's two report ids share one buffer length).
//!
//! The settings the M6 stores are changed by the same packets OpenMouse sends: 0x40 (the DPI slots + stage + count, copied
//! from the status read with the one slot changed), 0x41 / 0x4a (the polling gears), 0x42 (all the sensor options at once - the
//! other options are re-sent as read from the status, byte for byte as OpenMouse does), or 0x48 (DPI with separate X / Y
//! on firmware that flags it). Firmware flags decide which form a mouse uses; all forms are ported.
//!
//! Order 110 (A_110_02 scope, M6 only - the Nape client throws "not supported" for all of them): debounce 0-20 ms
//! (keychronEncodeDebounce `[0x43, ms]`) and Motion Sync / angle snapping / ripple (writeSensorFlag: the 0x42 packet with every
//! sensor option as read, the one switch changed), each confirmed by the status read (byte 17, flags byte 15). The "20K FPS"
//! switch (setPerformanceMode / maxSpeed) is not the Pulsar turbo: left out.
//!
//! Left out on purpose: the Nape's DPI stage select / orientation / layers / keymap / sleep, and for the M6 the DPI stage
//! select and count, sleep, profile, angle tuning, the 20K FPS toggle,
//! buttons, lighting, firmware query (none is DPI / polling / lift-off / battery). The Nape via a receiver (0xd026 / 0xd029) is
//! in OpenMouse's driver but not on the SUPPORTED list ("driver knows, list doesn't"). OpenMouse sends no firmware, DFU or
//! reset command to a Keychron mouse and neither does this file.
//!
//! NOTE for the app: `device.rs` treats vendor 0x3434 as keyboard-only, which drops these two mice from the list of mice
//! before any driver is asked - see [`MOUSE_PRODUCT_IDS`].

use crate::error::{Error, Result};
use crate::family::{check_extra, Caps, Change, Changed, DpiCaps, Extra, ExtraRow, Family, Kind, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x3434;
/// The Nape Pro (wired).
pub const PID_NAPE: u16 = 0x0440;
/// The M6 on its cable.
pub const PID_M6: u16 = 0xD060;
/// The Link-KM Type C receiver (KEYCHRON_M6_RECEIVER_PRODUCT_ID): behind it, the paired mouse - only an M6 is driven.
pub const PID_M6_RECEIVER: u16 = 0xD029;
/// The product ids of Keychron's SUPPORTED mice - what `device.rs` would have to let through its keyboard-only vendor list.
pub const MOUSE_PRODUCT_IDS: [u16; 3] = [PID_NAPE, PID_M6, PID_M6_RECEIVER];

// ---- Nape
pub const NAPE_USAGE_PAGE: u16 = 0xFF60;
pub const NAPE_USAGE: u16 = 0x61;
/// KEYCHRON_PACKET_LENGTH; Windows adds the report id 0.
pub const NAPE_PACKET_LEN: usize = 32;
pub const NAPE_REPORT_LEN: usize = NAPE_PACKET_LEN + 1;
const CMD_MISC_GROUP: u8 = 0xA7;
const NAPE_GET_DPI_STAGE: u8 = 33;
const NAPE_SET_DPI_STAGE: u8 = 34;
const NAPE_SET_DPI_VALUE: u8 = 35;
const NAPE_GET_DPI_VALUE: u8 = 36;
const NAPE_GET_BATTERY: u8 = 49;
const NAPE_GET_CUSTOM_DPI: u8 = 54;
const NAPE_SET_CUSTOM_DPI: u8 = 55;
const MISC_GET_POLLING: u8 = 13;
const MISC_SET_POLLING: u8 = 14;
pub const NAPE_DPI_STAGES: u8 = 5;
pub const NAPE_DPI_MIN: u32 = 50;
pub const NAPE_DPI_MAX: u32 = 4000;
pub const NAPE_DPI_STEP: u32 = 50;
/// KEYCHRON_POLLING_TABLE: the mask bit / shift index of each rate.
pub const NAPE_POLLING_TABLE: [u32; 7] = [8000, 4000, 2000, 1000, 500, 250, 125];
/// The pause after a fire-and-forget write (nape-hid.js: 40 ms).
const NAPE_WRITE_PAUSE_MS: u64 = 40;
const NAPE_QUERY_TIMEOUT_MS: u32 = 1200;

// ---- M6 (Launcher "8k")
pub const M6_USAGE_PAGE: u16 = 0xFFC1;
pub const M6_USAGE: u16 = 0x01;
pub const M6_COMMAND_REPORT: u8 = 0xB3;
pub const M6_COMMAND_ANSWER: u8 = 0xB4;
pub const M6_SETTINGS_REPORT: u8 = 0xB5;
pub const M6_SETTINGS_ANSWER: u8 = 0xB6;
pub const M6_COMMAND_LEN: usize = 63;
pub const M6_SETTINGS_LEN: usize = 20;
const ACK: u8 = 0xE4;
const STATUS_COMMAND: u8 = 0x06;
const READ_XY: u8 = 0x49;
const WRITE_XY: u8 = 0x48;
const SET_VERSION: u8 = 0x02;
const SET_RECEIVER_STATE: u8 = 0x03;
const SET_DPI: u8 = 0x40;
const SET_POLLING: u8 = 0x41;
const SET_SENSOR: u8 = 0x42;
/// Order 110: KEYCHRON_SET.debounce and KEYCHRON_DEBOUNCE_MAX_MS.
const SET_DEBOUNCE: u8 = 0x43;
pub const M6_DEBOUNCE_MAX_MS: i32 = 20;
const WRITE_POLLING_SETS: u8 = 0x4A;
const READ_POLLING_SETS: u8 = 0x4B;
const M6_QUERY_TIMEOUT_MS: u32 = 1200;
const DPI_SLOTS: usize = 5;
const POLLING_GEARS: usize = 6;
/// From this protocol version the feature flags sit in the 0x02 answer instead of the status report.
const FLAGS_IN_VERSION_REPLY: u16 = 6;
/// KEYCHRON_POLLING_RATES: a gear's value indexes this.
pub const M6_POLLING_RATES: [u32; 6] = [125, 500, 1000, 2000, 4000, 8000];
/// The M6 row: its product id, DPI floor / ceiling, fastest polling rate.
const M6_DPI: (u32, u32) = (100, 26000);
const M6_MAX_POLLING_HZ: u32 = 1000;
/// KEYCHRON_DPI_STEP when the firmware does not flag its own.
const M6_DPI_STEP: u32 = 50;
/// The M6's lift-off codes with their millimetres in tenths: `lod [[3, 0.7], [1, 1], [2, 2]]`.
const M6_LOD: [(u8, u32); 3] = [(3, 7), (1, 10), (2, 20)];

// ---------------------------------------------------------------- the Nape codec

/// keychronPacket: the command zero-padded to 32 bytes.
pub fn nape_packet(command: &[u8]) -> [u8; NAPE_PACKET_LEN] {
    let mut p = [0u8; NAPE_PACKET_LEN];
    p[..command.len()].copy_from_slice(command);
    p
}

/// The output report Windows sends: report id 0, the packet, zero-padded to the collection's length.
pub fn nape_report(command: &[u8], len: usize) -> Vec<u8> {
    let mut v = vec![0u8];
    v.extend_from_slice(&nape_packet(command));
    v.resize(len.max(NAPE_REPORT_LEN), 0);
    v
}

/// The read requests (bytes after the report id, before the zero padding).
pub fn nape_get_dpi_stage() -> [u8; 2] {
    [CMD_MISC_GROUP, NAPE_GET_DPI_STAGE]
}
pub fn nape_get_dpi_value(stage: u8) -> [u8; 3] {
    [CMD_MISC_GROUP, NAPE_GET_DPI_VALUE, stage]
}
pub fn nape_get_battery() -> [u8; 2] {
    [CMD_MISC_GROUP, NAPE_GET_BATTERY]
}
pub fn nape_get_polling() -> [u8; 2] {
    [CMD_MISC_GROUP, MISC_GET_POLLING]
}
pub fn nape_get_custom_dpi() -> [u8; 2] {
    [CMD_MISC_GROUP, NAPE_GET_CUSTOM_DPI]
}

/// The writes: set a stage's DPI value (`writeDpiStageValue`), select the active stage, the custom DPI fallback, the polling
/// shift.
pub fn nape_set_dpi_value(stage: u8, dpi: u32) -> [u8; 5] {
    [CMD_MISC_GROUP, NAPE_SET_DPI_VALUE, stage, dpi as u8, (dpi >> 8) as u8]
}
pub fn nape_set_dpi_stage(stage: u8) -> [u8; 3] {
    [CMD_MISC_GROUP, NAPE_SET_DPI_STAGE, stage]
}
pub fn nape_set_custom_dpi(dpi: u32) -> [u8; 4] {
    [CMD_MISC_GROUP, NAPE_SET_CUSTOM_DPI, dpi as u8, (dpi >> 8) as u8]
}
/// setPollingRate: `[0xa7, 14, shift, shift]`, shift = the rate's index in [`NAPE_POLLING_TABLE`].
pub fn nape_set_polling(hz: u32) -> Option<[u8; 4]> {
    let shift = NAPE_POLLING_TABLE.iter().position(|r| *r == hz)? as u8;
    Some([CMD_MISC_GROUP, MISC_SET_POLLING, shift, shift])
}

/// getDpiStage: `min(response[2], 4)`. `r` is OpenMouse's answer (no report id).
pub fn nape_decode_dpi_stage(r: &[u8]) -> u8 {
    r.get(2).copied().unwrap_or(0).min(NAPE_DPI_STAGES - 1)
}

/// getDpiValue / getCustomDpi: `response[2] | response[3] << 8`.
pub fn nape_decode_dpi(r: &[u8]) -> u32 {
    r.get(2).copied().unwrap_or(0) as u32 | (r.get(3).copied().unwrap_or(0) as u32) << 8
}

/// keychronDecodeBattery: (percent when <= 100, charging). `[2]` = percent (0xff when missing), `[3]` = status (1 charging,
/// 2 full).
pub fn nape_decode_battery(r: &[u8]) -> (Option<u8>, bool) {
    let percent = r.get(2).copied().unwrap_or(0xFF);
    let status = r.get(3).copied().unwrap_or(0);
    ((percent <= 100).then_some(percent), status == 1)
}

/// keychronDecodePolling: (current rate, rates it can be set to, ascending). `None` when the answer is all zero from byte 2:
/// OpenMouse then claims 1000 Hz and [125, 500, 1000] without any evidence - not shown here.
pub fn nape_decode_polling(r: &[u8]) -> Option<(u32, Vec<u32>)> {
    if r.get(2..).is_none_or(|t| t.iter().all(|b| *b == 0)) {
        return None;
    }
    let mask = r.get(5).copied().unwrap_or(0);
    let mut supported: Vec<u32> = NAPE_POLLING_TABLE.iter().enumerate().filter(|(i, _)| (mask >> i) & 1 == 1).map(|(_, r)| *r).collect();
    supported.sort_unstable();
    let idx = (r.get(6).copied().unwrap_or(3) as usize).min(NAPE_POLLING_TABLE.len() - 1);
    let rate = NAPE_POLLING_TABLE[idx];
    if supported.is_empty() {
        supported.push(rate);
    }
    Some((rate, supported))
}

// ---------------------------------------------------------------- the M6 codec (OpenMouse's byte numbering: no report id)

fn at(b: &[u8], i: usize) -> u8 {
    b.get(i).copied().unwrap_or(0)
}

fn read_u16(b: &[u8], i: usize) -> u32 {
    at(b, i) as u32 | (at(b, i + 1) as u32) << 8
}

/// A report as Windows sends it: the report id, the packet, zero-padded to the collection's output length.
pub fn m6_report(report_id: u8, packet: &[u8], out_len: usize) -> Vec<u8> {
    let mut v = vec![report_id];
    v.extend_from_slice(packet);
    v.resize(out_len.max(1 + M6_COMMAND_LEN), 0);
    v
}

fn m6_settings_packet(values: &[u8]) -> [u8; M6_SETTINGS_LEN] {
    let mut p = [0u8; M6_SETTINGS_LEN];
    p[..values.len()].copy_from_slice(values);
    p
}

fn m6_command_packet(values: &[u8]) -> [u8; M6_COMMAND_LEN] {
    let mut p = [0u8; M6_COMMAND_LEN];
    p[..values.len()].copy_from_slice(values);
    p
}

/// The status request (0x06 on the 63-byte report).
pub fn m6_status_request() -> [u8; M6_COMMAND_LEN] {
    m6_command_packet(&[STATUS_COMMAND])
}
/// The version / connection-mode request (0x02 on the 20-byte report, `[fallbackMode]`).
pub fn m6_version_request(fallback_mode: u8) -> [u8; M6_SETTINGS_LEN] {
    m6_settings_packet(&[SET_VERSION, fallback_mode])
}
/// The connected-mouse list request (0x03).
pub fn m6_receiver_state_request() -> [u8; M6_SETTINGS_LEN] {
    m6_settings_packet(&[SET_RECEIVER_STATE])
}
/// The separate X / Y DPI read (0x49 on the 63-byte report).
pub fn m6_read_xy_request() -> [u8; M6_COMMAND_LEN] {
    m6_command_packet(&[READ_XY])
}
/// The polling-gear sets read (0x4b).
pub fn m6_read_polling_sets_request() -> [u8; M6_SETTINGS_LEN] {
    m6_settings_packet(&[READ_POLLING_SETS])
}

/// What the 0x02 answer says (`readIdentity`): the connection (0 USB, 1 2.4 GHz, 2 Bluetooth), the protocol version, the id
/// of the mouse behind a receiver and the four feature bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    pub work_mode: u8,
    pub version: u32,
    pub product_id: Option<u32>,
    pub flags: [u8; 4],
}

/// `readIdentity` with a version answer; `fallback_mode` (1 behind a receiver, else 0) is used when the query got none.
pub fn m6_decode_version(b: Option<&[u8]>, fallback_mode: u8) -> Identity {
    match b {
        Some(b) => Identity { work_mode: at(b, 9) & 0x07, version: read_u16(b, 1), product_id: Some(read_u16(b, 5)), flags: [at(b, 11), at(b, 12), at(b, 13), at(b, 15)] },
        None => Identity { work_mode: fallback_mode, version: 0, product_id: None, flags: [0; 4] },
    }
}

/// keychronDecodeConnectedMouse: the product id of the first mouse the 0x03 list shows as connected (`state` byte 1).
pub fn m6_connected_mouse(b: &[u8]) -> Option<u32> {
    let count = (at(b, 1) as usize).min(b.len().saturating_sub(2) / 5);
    (0..count).find(|e| at(b, 2 + e * 5 + 4) == 1).map(|e| read_u16(b, 2 + e * 5 + 2))
}

/// The firmware's feature flags (decodeFeatures).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Features {
    pub dpi_limits: bool,
    pub polling_gears: bool,
    pub fps20k: bool,
    pub separate_dpi: bool,
    pub separate_polling: bool,
    pub lod_level: bool,
}

fn decode_features(f: [u8; 4]) -> Features {
    Features {
        dpi_limits: f[0] & 0x08 != 0,
        polling_gears: f[0] & 0x10 != 0,
        fps20k: f[0] & 0x80 != 0,
        separate_dpi: f[2] & 0x01 != 0,
        separate_polling: f[2] & 0x02 != 0,
        lod_level: f[3] & 0x10 != 0,
    }
}

/// The 0x06 status report, decoded (keychronDecodeSettings + parseStatus).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    /// per connection (USB, 2.4 GHz, Bluetooth): DPI stage in the low nibble, polling gear in the high nibble
    pub levels: [u8; 3],
    pub dpi_stages: [u32; DPI_SLOTS],
    pub stage_count: usize,
    pub lod: u8,
    pub ripple_control: bool,
    pub angle_snapping: bool,
    pub motion_sync: bool,
    pub scroll_reversed: bool,
    /// Order 110: keychronDecodeSettings' debounceMs (byte 17)
    pub debounce_ms: u8,
    pub polling_table: Vec<u8>,
    pub polling_count: usize,
    pub dpi_max: u32,
    pub dpi_step: u32,
    pub max_speed: bool,
    pub lod_level: u8,
    pub lod_count: usize,
    pub battery_percent: u8,
    pub charging: bool,
    pub features: Features,
}

/// parseStatus: the answer must start with 0x06 and be at least 51 bytes.
pub fn m6_decode_status(b: &[u8], ident: &Identity) -> Option<Status> {
    if b.len() < 51 || b[0] != STATUS_COMMAND {
        return None;
    }
    let flags = if ident.version >= FLAGS_IN_VERSION_REPLY as u32 { ident.flags } else { [at(b, 26), at(b, 53), at(b, 60), 0] };
    let f = at(b, 15);
    let mut dpi_stages = [0u32; DPI_SLOTS];
    for (i, s) in dpi_stages.iter_mut().enumerate() {
        *s = read_u16(b, 5 + i * 2);
    }
    let tail = b.get(43..).unwrap_or(&[]);
    Some(Status {
        levels: [at(b, 2), at(b, 3), at(b, 4)],
        dpi_stages,
        stage_count: (if at(b, 16) == 0 { DPI_SLOTS } else { at(b, 16) as usize }).min(DPI_SLOTS),
        lod: f & 0x03,
        ripple_control: f & 0x04 != 0,
        angle_snapping: f & 0x08 != 0,
        motion_sync: f & 0x10 != 0,
        scroll_reversed: f & 0x40 != 0,
        debounce_ms: at(b, 17),
        polling_table: tail[..tail.len().min(POLLING_GEARS)].to_vec(),
        polling_count: (if at(b, 49) == 0 { POLLING_GEARS } else { at(b, 49) as usize }).min(POLLING_GEARS),
        dpi_max: read_u16(b, 40),
        dpi_step: at(b, 42) as u32,
        max_speed: at(b, 52) & 0x01 != 0,
        lod_level: at(b, 61) & 0x0F,
        lod_count: (at(b, 61) >> 4) as usize,
        battery_percent: at(b, 19) & 0x7F,
        charging: at(b, 19) & 0x80 != 0,
        features: decode_features(flags),
    })
}

/// keychronActiveStage / keychronActiveGear.
fn active_stage(s: &Status, work_mode: u8) -> usize {
    ((s.levels[(work_mode as usize).min(2)] & 0x0F) as usize).min(s.stage_count.saturating_sub(1))
}
fn active_gear(s: &Status, work_mode: u8) -> usize {
    ((s.levels[(work_mode as usize).min(2)] >> 4) & 0x0F) as usize
}

/// The DPI part of the mouse: the active stage, the five slots, the count, and (firmware that flags separate X / Y) the Y
/// values and the per-stage enable flags the 0x48 write must send back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DpiState {
    pub active: usize,
    pub stages: [u32; DPI_SLOTS],
    pub count: usize,
    pub xy: Option<Xy>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xy {
    pub y: [u32; DPI_SLOTS],
    pub enable: Vec<u8>,
}

/// readDpi without separate X / Y: from the status.
pub fn m6_dpi_from_status(s: &Status, work_mode: u8) -> DpiState {
    DpiState { active: active_stage(s, work_mode), stages: s.dpi_stages, count: s.stage_count, xy: None }
}

/// readDpi with separate X / Y: from the 0x49 answer.
pub fn m6_dpi_from_xy(b: &[u8], ident: &Identity) -> DpiState {
    let count = (if at(b, 4) == 0 { DPI_SLOTS } else { at(b, 4) as usize }).min(DPI_SLOTS);
    let levels = at(b, 1 + (ident.work_mode as usize).min(2));
    let mut stages = [0u32; DPI_SLOTS];
    let mut y = [0u32; DPI_SLOTS];
    for i in 0..DPI_SLOTS {
        stages[i] = read_u16(b, 5 + i * 2);
        y[i] = read_u16(b, 21 + i * 2);
    }
    let enable = if ident.version >= FLAGS_IN_VERSION_REPLY as u32 { (0..8).map(|bit| (at(b, 37) >> bit) & 1).collect() } else { (0..8).map(|i| at(b, 37 + i)).collect() };
    DpiState { active: ((levels & 0x0F) as usize).min(count - 1), stages, count, xy: Some(Xy { y, enable }) }
}

/// keychronEncodeDpi (0x40): the stage in the three connection bytes, the five slots little-endian from byte 4, the count at 14.
pub fn m6_encode_dpi(active: usize, stages: &[u32; DPI_SLOTS], count: usize) -> [u8; M6_SETTINGS_LEN] {
    let mut p = [0u8; M6_SETTINGS_LEN];
    p[0] = SET_DPI;
    p[1..4].fill(active as u8);
    for (i, s) in stages.iter().enumerate() {
        p[4 + i * 2] = *s as u8;
        p[5 + i * 2] = (*s >> 8) as u8;
    }
    p[14] = count as u8;
    p
}

/// writeDpi's 0x48 form (separate X / Y): the stage in the connection bytes (`active << 4 | active` before protocol 6), the
/// count at 4, X at 5.., Y at 21.. (an unchanged slot keeps its Y, a changed one gets Y = X), the enable flags at 37.
pub fn m6_encode_dpi_xy(current: &DpiState, next: &DpiState, ident: &Identity) -> Option<[u8; M6_COMMAND_LEN]> {
    let xy = current.xy.as_ref()?;
    let v6 = ident.version >= FLAGS_IN_VERSION_REPLY as u32;
    let mut p = [0u8; M6_COMMAND_LEN];
    p[0] = WRITE_XY;
    let stage_byte = if v6 { next.active as u8 } else { ((next.active << 4) | next.active) as u8 };
    p[1..4].fill(stage_byte);
    p[4] = next.count as u8;
    for (i, x) in next.stages.iter().enumerate() {
        p[5 + i * 2] = *x as u8;
        p[6 + i * 2] = (*x >> 8) as u8;
        let y = if *x == current.stages[i] { xy.y[i] } else { *x };
        p[21 + i * 2] = y as u8;
        p[22 + i * 2] = (y >> 8) as u8;
    }
    if v6 {
        p[37] = xy.enable.iter().enumerate().fold(0u8, |mask, (i, bit)| mask | ((bit & 1) << i));
    } else {
        for (i, b) in xy.enable.iter().take(8).enumerate() {
            p[37 + i] = *b;
        }
    }
    Some(p)
}

/// The polling gears of the connection: the active gear, how many are in use and their rate indexes (into
/// [`M6_POLLING_RATES`]); `sets` on firmware that keeps one set per link (USB / 2.4 GHz) and the one in use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gears {
    pub level: usize,
    pub count: usize,
    pub table: Vec<u8>,
    pub sets: Option<[(usize, usize, Vec<u8>); 2]>,
    pub set: usize,
}

pub fn m6_gears_from_status(s: &Status, work_mode: u8) -> Gears {
    Gears { level: active_gear(s, work_mode), count: s.polling_count, table: s.polling_table.clone(), sets: None, set: 0 }
}

/// readPolling with separate sets: from the 0x4b answer.
pub fn m6_gears_from_sets(b: &[u8], ident: &Identity) -> Gears {
    let one = |s: usize| {
        let count = (if at(b, 3 + s) == 0 { POLLING_GEARS } else { at(b, 3 + s) as usize }).min(POLLING_GEARS);
        let table = (0..POLLING_GEARS).filter_map(|i| b.get(5 + s * POLLING_GEARS + i).copied()).collect::<Vec<u8>>();
        (at(b, 1 + s) as usize, count, table)
    };
    let sets = [one(0), one(1)];
    let set = if ident.work_mode == 1 { 1 } else { 0 };
    let (level, count, table) = sets[set].clone();
    Gears { level, count, table, sets: Some(sets), set }
}

impl Gears {
    /// `POLLING_RATES[table[level] ?? 2] ?? 1000`, but `None` where OpenMouse would fall back to a guess.
    pub fn rate(&self) -> Option<u32> {
        self.table.get(self.level).and_then(|i| M6_POLLING_RATES.get(*i as usize)).copied()
    }
}

/// pollingPacket: 0x41 (this connection's gears) or, with separate sets, 0x4a (both sets, the active one replaced).
pub fn m6_encode_polling(current: &Gears, next: &Gears) -> [u8; M6_SETTINGS_LEN] {
    let mut p = [0u8; M6_SETTINGS_LEN];
    match &current.sets {
        None => {
            p[0] = SET_POLLING;
            p[1] = next.level as u8;
            p[2] = next.level as u8;
            for (i, t) in next.table.iter().take(next.count).enumerate() {
                p[3 + i] = *t;
            }
            p[9] = next.count as u8;
        }
        Some(sets) => {
            p[0] = WRITE_POLLING_SETS;
            for (i, set) in sets.iter().enumerate() {
                let (level, count, table) = if i == current.set { (next.level, next.count, &next.table) } else { (set.0, set.1, &set.2) };
                p[1 + i] = level as u8;
                p[3 + i] = count as u8;
                for (j, t) in table.iter().take(POLLING_GEARS).enumerate() {
                    p[5 + i * POLLING_GEARS + j] = *t;
                }
            }
        }
    }
    p
}

/// supportedRates for the M6 row: the model's rates (up to 1000 Hz) when the gears can be rewritten, else the rates the
/// gears hold. Ascending, no repeats, never empty.
pub fn m6_supported_rates(s: &Status, g: &Gears) -> Vec<u32> {
    let mut rates: Vec<u32> = if s.features.polling_gears {
        M6_POLLING_RATES.iter().copied().filter(|r| *r <= M6_MAX_POLLING_HZ).collect()
    } else {
        g.table.iter().take(g.count).filter_map(|i| M6_POLLING_RATES.get(*i as usize).copied()).collect()
    };
    rates.sort_unstable();
    rates.dedup();
    if rates.is_empty() {
        rates.extend(g.rate());
    }
    rates
}

/// How the lift-off is stored: as the 2-bit code, or (firmware that flags levels) as a level byte.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiftOffChoices {
    pub level_mode: bool,
    /// (code, tenths of a mm)
    pub choices: Vec<(u8, u32)>,
}

/// liftOffChoices for the M6 row.
pub fn m6_lift_off_choices(s: &Status) -> LiftOffChoices {
    if !s.features.lod_level {
        return LiftOffChoices { level_mode: false, choices: M6_LOD.to_vec() };
    }
    let mut levels = M6_LOD.to_vec();
    levels.sort_by_key(|c| c.0);
    if s.lod_count > 0 {
        levels.truncate(s.lod_count);
    }
    LiftOffChoices { level_mode: true, choices: levels }
}

impl LiftOffChoices {
    /// The lift-off distances that can be set, ascending, tenths of a mm.
    pub fn tenths(&self) -> Vec<u32> {
        let mut t: Vec<u32> = self.choices.iter().map(|c| c.1).collect();
        t.sort_unstable();
        t
    }
    pub fn code_for(&self, tenths: u32) -> Option<u8> {
        self.choices.iter().find(|c| c.1 == tenths).map(|c| c.0)
    }
    /// The distance of the code the mouse reports now.
    pub fn current(&self, s: &Status) -> Option<u32> {
        let code = if self.level_mode { s.lod_level } else { s.lod };
        self.choices.iter().find(|c| c.0 == code).map(|c| c.1)
    }
}

/// keychronEncodeSensor (0x42) with `lod` / `lodLevel` as writeSensor sends them: every sensor option re-sent as read (1 = on,
/// 2 = off, except `maxSpeed`: 2 = on, 1 = off; always sent), the lift-off in the code byte or the level byte, the other 0.
pub fn m6_encode_sensor(s: &Status, level_mode: bool, code: u8) -> [u8; M6_SETTINGS_LEN] {
    m6_encode_sensor_with(s, level_mode, code, (s.ripple_control, s.angle_snapping, s.motion_sync))
}

/// Order 110: writeSensorFlag - the same 0x42 packet with the lift-off as read (code or level byte) and the three switches
/// (ripple, angle snapping, Motion Sync) as given.
pub fn m6_encode_sensor_flags(s: &Status, level_mode: bool, flags: (bool, bool, bool)) -> [u8; M6_SETTINGS_LEN] {
    m6_encode_sensor_with(s, level_mode, if level_mode { s.lod_level } else { s.lod }, flags)
}

/// Order 110: keychronEncodeDebounce: `[0x43, ms]`.
pub fn m6_encode_debounce(ms: u8) -> [u8; M6_SETTINGS_LEN] {
    m6_settings_packet(&[SET_DEBOUNCE, ms])
}

fn m6_encode_sensor_with(s: &Status, level_mode: bool, code: u8, (ripple, angle, motion): (bool, bool, bool)) -> [u8; M6_SETTINGS_LEN] {
    let flag = |on: bool| if on { 1 } else { 2 };
    let mut p = [0u8; M6_SETTINGS_LEN];
    p[0] = SET_SENSOR;
    p[1] = if level_mode { 0 } else { code };
    p[2] = flag(ripple);
    p[3] = flag(angle);
    p[4] = flag(motion);
    p[6] = if s.scroll_reversed { 2 } else { 1 };
    p[8] = if s.max_speed { 2 } else { 1 }; // inverted vs the other toggles, as keychronEncodeSensor: maxSpeed ? 2 : 1
    p[11] = if level_mode { code } else { 0 };
    p
}

/// DPI limits as modelDpiLimits: the model's floor, the mouse's own ceiling and step when it flags them.
pub fn m6_dpi_limits(s: &Status) -> (u32, u32, u32) {
    let (min, max) = M6_DPI;
    (min, if s.features.dpi_limits && s.dpi_max > min { s.dpi_max } else { max }, if s.features.dpi_limits && s.dpi_step > 0 { s.dpi_step } else { M6_DPI_STEP })
}

/// requireDpi: in range and a multiple of the step.
pub fn m6_dpi_ok(dpi: u32, s: &Status) -> bool {
    let (min, max, step) = m6_dpi_limits(s);
    dpi >= min && dpi <= max && dpi.is_multiple_of(step)
}

// ---------------------------------------------------------------- the device table

fn find_col<'a>(own: &[&'a HidInfo], page: u16, usage: u16, out_min: usize, in_min: usize) -> Option<&'a HidInfo> {
    own.iter().copied().find(|h| h.usage_page == page && h.usage == usage && h.output_len as usize >= out_min && h.input_len as usize >= in_min)
}

/// OpenMouse's isSupported for the SUPPORTED models: Keychron's vendor id, and
/// - 0x0440 (Nape Pro): the 0xFF60 / 0x61 raw-HID collection (report id 0, 32 bytes each way);
/// - 0xd060 (M6) / 0xd029 (its receiver): the 0xFFC1 / 1 collection with the 63-byte command report 0xb3 (answers 0xb4)
///   - Windows shows the longest report: at least 64 bytes each way. Which report ids it declares can't be seen from here.
///
/// The Nape through a receiver (0xd026 / 0xd029 with a 0xFF60 collection) is not on the SUPPORTED list and does not match.
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID {
        return None;
    }
    match pid {
        PID_NAPE => {
            let c = find_col(own, NAPE_USAGE_PAGE, NAPE_USAGE, NAPE_REPORT_LEN, NAPE_REPORT_LEN)?;
            Some(Matched { family: Family::Keychron, model: Some("Keychron Nape Pro"), wireless: Some(false), config: c.clone(), variant: pid })
        }
        PID_M6 | PID_M6_RECEIVER => {
            let c = find_col(own, M6_USAGE_PAGE, M6_USAGE, 1 + M6_COMMAND_LEN, 1 + M6_COMMAND_LEN)?;
            Some(Matched { family: Family::Keychron, model: Some("Keychron M6"), wireless: Some(pid == PID_M6_RECEIVER), config: c.clone(), variant: pid })
        }
        _ => None,
    }
}

/// The allow-list for the read-only proof on a real mouse (A_009_01): only the read requests this file sends, each with
/// nothing but zero padding behind -
/// - Nape (report id 0, 32-byte packet): `a7 21` (stage), `a7 24 <0..4>` (a stage's DPI), `a7 31` (battery), `a7 0d`
///   (polling), `a7 36` (custom DPI);
/// - M6 command report 0xb3: `06` (status), `49` (separate X / Y DPI);
/// - M6 settings report 0xb5: `02 <0|1>` (version), `03` (connected mice), `4b` (polling sets).
///
/// Every write (and everything else) is refused.
pub fn read_request_allowed(out: &[u8]) -> bool {
    let zeros = |from: usize| out.get(from..).is_some_and(|t| t.iter().all(|b| *b == 0));
    match out.first() {
        Some(0x00) => {
            out.len() >= NAPE_REPORT_LEN
                && out[1] == CMD_MISC_GROUP
                && match out[2] {
                    NAPE_GET_DPI_STAGE | NAPE_GET_BATTERY | MISC_GET_POLLING | NAPE_GET_CUSTOM_DPI => zeros(3),
                    NAPE_GET_DPI_VALUE => out[3] < NAPE_DPI_STAGES && zeros(4),
                    _ => false,
                }
        }
        Some(&M6_COMMAND_REPORT) => out.len() > M6_COMMAND_LEN && matches!(out[1], STATUS_COMMAND | READ_XY) && zeros(2),
        Some(&M6_SETTINGS_REPORT) => {
            out.len() > M6_SETTINGS_LEN
                && match out[1] {
                    SET_VERSION => out[2] <= 1 && zeros(3),
                    SET_RECEIVER_STATE | READ_POLLING_SETS => zeros(2),
                    _ => false,
                }
        }
        _ => false,
    }
}

// ---------------------------------------------------------------- the exchange

/// `request(...).catch(() => null)`: no answer / refused = `None`; a mouse that is gone still fails.
fn optional<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(Error::BadAnswer(_) | Error::Os { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

fn the_variant(m: &Matched) -> Result<u16> {
    match m.variant {
        v @ (PID_NAPE | PID_M6 | PID_M6_RECEIVER) => Ok(v),
        v => Err(Error::UnsupportedMouse(format!("Keychron product id {v:#06x}"))),
    }
}

// ---- Nape

/// nape-hid.js `query`: send, take the first answer that echoes `[0xa7, command]` (1200 ms).
fn nape_query(port: &mut dyn Port, command: &[u8]) -> Result<Vec<u8>> {
    let (out_len, in_len) = (port.info().output_len as usize, port.info().input_len as usize);
    let prefix = vec![0x00, command[0], command[1]];
    let how = HidTransfer::OutputThenMatch { reply_len: in_len.max(NAPE_REPORT_LEN), max_reads: 32, timeout_ms: NAPE_QUERY_TIMEOUT_MS, prefixes: vec![prefix] };
    let a = port.exchange(&nape_report(command, out_len), &how)?;
    Ok(a.get(1..).unwrap_or(&[]).to_vec())
}

/// nape-hid.js `write`: a fire-and-forget output report, then 40 ms.
fn nape_write(port: &mut dyn Port, command: &[u8]) -> Result<()> {
    let out_len = port.info().output_len as usize;
    port.exchange(&nape_report(command, out_len), &HidTransfer::OutputOnly)?;
    port.pause_ms(NAPE_WRITE_PAUSE_MS);
    Ok(())
}

fn nape_caps(polling: Vec<u32>) -> Caps {
    Caps { dpi: Some(DpiCaps::uniform(NAPE_DPI_MIN, NAPE_DPI_MAX, NAPE_DPI_STEP)), polling, lift_off: Vec::new(), battery: true }
}

fn nape_read(port: &mut dyn Port, r: &mut OnMouse) -> Result<()> {
    // getDpiStage is the one that decides "is it there" (OpenMouse's readStatus throws without it)
    let Some(stage) = optional(nape_query(port, &nape_get_dpi_stage()))?.map(|a| nape_decode_dpi_stage(&a)) else {
        return Ok(());
    };
    r.online = true;
    r.dpi = optional(nape_query(port, &nape_get_dpi_value(stage)))?.map(|a| nape_decode_dpi(&a));
    if let Some(a) = optional(nape_query(port, &nape_get_battery()))? {
        let (percent, charging) = nape_decode_battery(&a);
        r.battery_percent = percent;
        r.charging = percent.map(|_| charging);
    }
    if let Some((rate, supported)) = optional(nape_query(port, &nape_get_polling()))?.and_then(|a| nape_decode_polling(&a)) {
        r.polling_hz = Some(rate);
        r.caps.polling = supported;
    }
    Ok(())
}

/// writeDpiStageValue for the active stage, then the confirmation OpenMouse does (and an error where it would pass a wrong
/// value on silently).
fn nape_set_dpi(port: &mut dyn Port, dpi: u32) -> Result<Changed> {
    if !(NAPE_DPI_MIN..=NAPE_DPI_MAX).contains(&dpi) || !dpi.is_multiple_of(NAPE_DPI_STEP) {
        return Err(Error::range("DPI", format!("{dpi} DPI is not one the Nape Pro can store ({NAPE_DPI_MIN} - {NAPE_DPI_MAX} in steps of {NAPE_DPI_STEP})")));
    }
    let stage = nape_decode_dpi_stage(&nape_query(port, &nape_get_dpi_stage())?);
    let before = nape_decode_dpi(&nape_query(port, &nape_get_dpi_value(stage))?);
    nape_write(port, &nape_set_dpi_value(stage, dpi))?;
    let active = nape_decode_dpi_stage(&nape_query(port, &nape_get_dpi_stage())?);
    if active == stage {
        nape_write(port, &nape_set_dpi_stage(stage))?;
    }
    let mut confirmed = nape_decode_dpi(&nape_query(port, &nape_get_dpi_value(stage))?);
    if confirmed != dpi && active == stage {
        // the custom-DPI fallback OpenMouse tries when the stage still holds the old value
        nape_write(port, &nape_set_custom_dpi(dpi))?;
        confirmed = match optional(nape_query(port, &nape_get_custom_dpi()))? {
            Some(a) => nape_decode_dpi(&a),
            None => nape_decode_dpi(&nape_query(port, &nape_get_dpi_value(stage))?),
        };
    }
    if confirmed != dpi {
        return Err(Error::BadAnswer(format!("the mouse kept {confirmed} DPI instead of {dpi}")));
    }
    Ok(Changed { now: dpi, before: Some(before) })
}

fn nape_set_polling_rate(port: &mut dyn Port, hz: u32) -> Result<Changed> {
    let (before, supported) = nape_decode_polling(&nape_query(port, &nape_get_polling())?).ok_or_else(|| Error::BadAnswer("this Nape Pro's firmware does not report its polling rate".into()))?;
    if !supported.contains(&hz) {
        return Err(Error::range("polling rate", format!("this Nape Pro connection does not support {hz} Hz")));
    }
    let set = nape_set_polling(hz).ok_or_else(|| Error::range("polling rate", format!("unsupported polling rate {hz} Hz")))?;
    nape_write(port, &set)?;
    let now = nape_decode_polling(&nape_query(port, &nape_get_polling())?).map(|p| p.0);
    if now != Some(hz) {
        return Err(Error::BadAnswer(format!("the Nape Pro kept {now:?} Hz instead of {hz} Hz")));
    }
    Ok(Changed { now: hz, before: Some(before) })
}

// ---- M6

/// How an answer is recognised (the JS `match` of each query).
#[derive(Clone, Copy)]
enum Expect {
    /// `bytes[0] === command`
    Cmd(u8),
    /// `bytes[0] === command || (bytes[0] === ACK && bytes[2] === command)`
    CmdOrAck(u8),
    /// `bytes[0] === ACK && bytes[2] === command`
    Ack(u8),
}

/// The Windows prefixes of every answer that matches (the JS listener does not look at the report id, so the answer may
/// come on 0xb4 or 0xb6).
fn prefixes(e: Expect) -> Vec<Vec<u8>> {
    let mut v = Vec::new();
    for rid in [M6_COMMAND_ANSWER, M6_SETTINGS_ANSWER] {
        let (cmd, ack) = match e {
            Expect::Cmd(c) => (Some(c), None),
            Expect::CmdOrAck(c) => (Some(c), Some(c)),
            Expect::Ack(c) => (None, Some(c)),
        };
        if let Some(c) = cmd {
            v.push(vec![rid, c]);
        }
        if let Some(c) = ack {
            v.extend((0..=255u8).map(|code| vec![rid, ACK, code, c]));
        }
    }
    v
}

/// keychron8k `query`: send a report, take the first answer matching `e` (1200 ms). The answer is returned as OpenMouse sees
/// it (no report id).
fn m6_query(port: &mut dyn Port, report_id: u8, packet: &[u8], e: Expect) -> Result<Vec<u8>> {
    let (out_len, in_len) = (port.info().output_len as usize, port.info().input_len as usize);
    let how = HidTransfer::OutputThenMatch { reply_len: in_len.max(1 + M6_COMMAND_LEN), max_reads: 32, timeout_ms: M6_QUERY_TIMEOUT_MS, prefixes: prefixes(e) };
    let a = port.exchange(&m6_report(report_id, packet, out_len), &how)?;
    Ok(a.get(1..).unwrap_or(&[]).to_vec())
}

/// writeSettings: a 20-byte report acknowledged `[0xe4, code, command]` (or answered by the command's own echo).
fn m6_write_settings(port: &mut dyn Port, packet: &[u8; M6_SETTINGS_LEN]) -> Result<()> {
    let command = packet[0];
    let r = m6_query(port, M6_SETTINGS_REPORT, packet, Expect::CmdOrAck(command))?;
    if r.first() == Some(&ACK) && at(&r, 1) != 0 {
        return Err(Error::BadAnswer(format!("the mouse rejected command {command:#04x} (code {})", at(&r, 1))));
    }
    Ok(())
}

/// What a session with an M6 knows: how it is connected and what it is.
struct M6 {
    ident: Identity,
}

/// The paired mouse's identity, per OpenMouse's readModel: the USB product id, else the id in the 0x02 answer, else (behind a
/// receiver) the first connected mouse in the 0x03 list. `None` = unknown.
fn m6_model_id(port: &mut dyn Port, pid: u16, ident: &Identity) -> Result<Option<u32>> {
    if pid == PID_M6 {
        return Ok(Some(PID_M6 as u32));
    }
    if let Some(id) = ident.product_id.filter(|id| *id == PID_M6 as u32) {
        return Ok(Some(id));
    }
    if ident.work_mode == 1 {
        let list = optional(m6_query(port, M6_SETTINGS_REPORT, &m6_receiver_state_request(), Expect::Cmd(SET_RECEIVER_STATE)))?;
        if let Some(id) = list.and_then(|l| m6_connected_mouse(&l)) {
            return Ok(Some(id));
        }
    }
    Ok(ident.product_id.filter(|id| *id != 0))
}

fn m6_identity(port: &mut dyn Port, pid: u16) -> Result<Identity> {
    let fallback = if pid == PID_M6_RECEIVER { 1 } else { 0 };
    let version = optional(m6_query(port, M6_SETTINGS_REPORT, &m6_version_request(fallback), Expect::Cmd(SET_VERSION)))?;
    Ok(m6_decode_version(version.as_deref(), fallback))
}

/// Identity + "is it an M6": `Ok(None)` = cannot tell (nothing paired / asleep), `Err` = another mouse.
fn m6_open(port: &mut dyn Port, pid: u16) -> Result<Option<M6>> {
    let ident = m6_identity(port, pid)?;
    match m6_model_id(port, pid, &ident)? {
        Some(id) if id == PID_M6 as u32 => Ok(Some(M6 { ident })),
        Some(id) => Err(Error::UnsupportedMouse(format!("Keychron mouse {id:#06x} behind this receiver (only the M6 is supported)"))),
        None => Ok(None),
    }
}

fn m6_status(port: &mut dyn Port, s: &M6) -> Result<Status> {
    let b = m6_query(port, M6_COMMAND_REPORT, &m6_status_request(), Expect::Cmd(STATUS_COMMAND))?;
    m6_decode_status(&b, &s.ident).ok_or_else(|| Error::BadAnswer("the Keychron M6 returned an invalid status report".into()))
}

fn m6_read_dpi(port: &mut dyn Port, s: &M6, st: &Status) -> Result<DpiState> {
    if !st.features.separate_dpi {
        return Ok(m6_dpi_from_status(st, s.ident.work_mode));
    }
    let b = m6_query(port, M6_COMMAND_REPORT, &m6_read_xy_request(), Expect::Cmd(READ_XY))?;
    Ok(m6_dpi_from_xy(&b, &s.ident))
}

fn m6_read_gears(port: &mut dyn Port, s: &M6, st: &Status) -> Result<Gears> {
    if !st.features.separate_polling {
        return Ok(m6_gears_from_status(st, s.ident.work_mode));
    }
    let b = m6_query(port, M6_SETTINGS_REPORT, &m6_read_polling_sets_request(), Expect::Cmd(READ_POLLING_SETS))?;
    Ok(m6_gears_from_sets(&b, &s.ident))
}

fn m6_caps(st: &Status, g: &Gears) -> Caps {
    let (min, max, step) = m6_dpi_limits(st);
    Caps { dpi: Some(DpiCaps::uniform(min, max, step)), polling: m6_supported_rates(st, g), lift_off: m6_lift_off_choices(st).tenths(), battery: true }
}

fn m6_read(port: &mut dyn Port, pid: u16, r: &mut OnMouse) -> Result<()> {
    let Some(s) = m6_open(port, pid)? else {
        return Ok(());
    };
    r.wireless = Some(s.ident.work_mode != 0);
    // any failure of the three reads is OpenMouse's "unreachable" status (the name only)
    let Some(st) = optional(m6_status(port, &s))? else {
        return Ok(());
    };
    let (Some(dpi), Some(g)) = (optional(m6_read_dpi(port, &s, &st))?, optional(m6_read_gears(port, &s, &st))?) else {
        return Ok(());
    };
    r.online = true;
    r.caps = m6_caps(&st, &g);
    r.dpi = dpi.stages.get(dpi.active).copied();
    r.polling_hz = g.rate();
    r.lift_off = m6_lift_off_choices(&st).current(&st);
    r.extras = m6_extras(Some(&st));
    if st.battery_percent <= 100 {
        r.battery_percent = Some(st.battery_percent);
        r.charging = Some(st.charging);
    }
    Ok(())
}

fn m6_write(port: &mut dyn Port, pid: u16, set: Setting) -> Result<Changed> {
    let s = m6_open(port, pid)?.ok_or_else(|| Error::BadAnswer("the Keychron M6 did not answer - it may be asleep or not paired to this receiver".into()))?;
    let st = m6_status(port, &s)?;
    match set {
        Setting::Dpi(dpi) => {
            if !m6_dpi_ok(dpi, &st) {
                let (min, max, step) = m6_dpi_limits(&st);
                return Err(Error::range("DPI", format!("the Keychron M6 DPI must be a multiple of {step} between {min} and {max}")));
            }
            let current = m6_read_dpi(port, &s, &st)?;
            let before = current.stages[current.active];
            let mut next = current.clone();
            next.stages[current.active] = dpi;
            match m6_encode_dpi_xy(&current, &next, &s.ident) {
                Some(p) => {
                    let r = m6_query(port, M6_COMMAND_REPORT, &p, Expect::Ack(WRITE_XY))?;
                    if at(&r, 1) != 0 {
                        return Err(Error::BadAnswer(format!("the Keychron M6 rejected command 0x48 (code {})", at(&r, 1))));
                    }
                }
                None => m6_write_settings(port, &m6_encode_dpi(next.active, &next.stages, next.count))?,
            }
            let st2 = m6_status(port, &s)?;
            let now = m6_read_dpi(port, &s, &st2)?;
            let got = now.stages[current.active];
            if got != dpi {
                return Err(Error::BadAnswer(format!("the Keychron M6 kept {got} DPI on stage {} instead of {dpi} DPI", current.active + 1)));
            }
            Ok(Changed { now: dpi, before: Some(before) })
        }
        Setting::Polling(hz) => {
            let current = m6_read_gears(port, &s, &st)?;
            let before = current.rate();
            let rate = M6_POLLING_RATES.iter().position(|r| *r == hz);
            let gear = rate.and_then(|r| current.table.iter().take(current.count).position(|t| *t as usize == r));
            let next = match (rate, gear) {
                (Some(_), Some(g)) => Gears { level: g, ..current.clone() },
                (Some(r), None) if st.features.polling_gears && m6_supported_rates(&st, &current).contains(&hz) => {
                    let mut n = current.clone();
                    if let Some(slot) = n.table.get_mut(current.level) {
                        *slot = r as u8;
                    }
                    n
                }
                _ => return Err(Error::range("polling rate", format!("the Keychron M6 does not support {hz} Hz on this connection"))),
            };
            m6_write_settings(port, &m6_encode_polling(&current, &next))?;
            let st2 = m6_status(port, &s)?;
            let now = m6_read_gears(port, &s, &st2)?.rate();
            if now != Some(hz) {
                return Err(Error::BadAnswer(format!("the Keychron M6 kept {now:?} Hz instead of {hz} Hz")));
            }
            Ok(Changed { now: hz, before })
        }
        Setting::LiftOff(tenths) => {
            let choices = m6_lift_off_choices(&st);
            let code = choices.code_for(tenths).ok_or_else(|| Error::range("lift-off distance", format!("the Keychron M6 has no {tenths}/10 mm lift-off (0.7, 1 or 2 mm)")))?;
            let before = choices.current(&st);
            m6_write_settings(port, &m6_encode_sensor(&st, choices.level_mode, code))?;
            let st2 = m6_status(port, &s)?;
            let now = m6_lift_off_choices(&st2).current(&st2);
            if now != Some(tenths) {
                return Err(Error::BadAnswer(format!("the Keychron M6 kept lift-off {now:?}/10 mm instead of {tenths}/10 mm")));
            }
            Ok(Changed { now: tenths, before })
        }
    }
}

/// Order 110: the status' debounceMs (byte 17) and the three switches (flags byte 15: ripple 0x04, angle snapping 0x08,
/// Motion Sync 0x10), in the card's order (`None` = the guard's rows).
pub fn m6_extras(st: Option<&Status>) -> Vec<ExtraRow> {
    vec![
        ExtraRow::new(Extra::Debounce, Kind::Range { min: 0, max: M6_DEBOUNCE_MAX_MS, step: 1, unit: "ms" }, st.map(|s| s.debounce_ms as i32)),
        ExtraRow::switch(Extra::MotionSync, st.map(|s| s.motion_sync)),
        ExtraRow::switch(Extra::AngleSnap, st.map(|s| s.angle_snapping)),
        ExtraRow::switch(Extra::RippleControl, st.map(|s| s.ripple_control)),
    ]
}

/// Order 110 (M6): setDebounceTime (0x43, then the status read back) / writeSensorFlag (0x42 with every sensor option as
/// read and the one switch changed, then the status read back).
fn m6_change(port: &mut dyn Port, pid: u16, c: Change) -> Result<()> {
    let Change::Extra(e, v) = c;
    check_extra(&m6_extras(None), e, v)?;
    let s = m6_open(port, pid)?.ok_or_else(|| Error::BadAnswer("the Keychron M6 did not answer - it may be asleep or not paired to this receiver".into()))?;
    let st = m6_status(port, &s)?;
    let on = v == 1;
    let packet = match e {
        Extra::Debounce => m6_encode_debounce(v as u8),
        Extra::MotionSync => m6_encode_sensor_flags(&st, m6_lift_off_choices(&st).level_mode, (st.ripple_control, st.angle_snapping, on)),
        Extra::AngleSnap => m6_encode_sensor_flags(&st, m6_lift_off_choices(&st).level_mode, (st.ripple_control, on, st.motion_sync)),
        Extra::RippleControl => m6_encode_sensor_flags(&st, m6_lift_off_choices(&st).level_mode, (on, st.angle_snapping, st.motion_sync)),
        Extra::Turbo => return Err(crate::family::not_offered(c)),
    };
    m6_write_settings(port, &packet)?;
    let now = m6_extras(Some(&m6_status(port, &s)?)).into_iter().find(|r| r.extra == e).and_then(|r| r.value);
    if now != Some(v) {
        return Err(Error::BadAnswer(format!("the Keychron M6 kept {now:?} instead of {v} ({})", e.label())));
    }
    Ok(())
}

// ---------------------------------------------------------------- the driver

/// The Keychron driver: the Nape Pro client or the M6's Launcher client (by the product id).
pub struct Keychron;

impl crate::family::Driver for Keychron {
    /// READ requests only; every one passes [`read_request_allowed`]. A mouse that does not answer (asleep behind the
    /// receiver) is `online: false`.
    fn read(&self, m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let pid = the_variant(m)?;
        let mut r = OnMouse { model: m.model.map(str::to_string), wireless: m.wireless, ..Default::default() };
        if pid == PID_NAPE {
            r.caps = nape_caps(Vec::new());
            nape_read(port, &mut r)?;
        } else {
            r.caps = Caps { battery: true, ..Default::default() };
            m6_read(port, pid, &mut r)?;
        }
        Ok(r)
    }

    /// Order 110: the M6's debounce and sensor switches (the Nape client throws "not supported" for all of them).
    fn change(&self, m: &Matched, port: &mut dyn Port, c: Change) -> Result<()> {
        let pid = the_variant(m)?;
        if pid == PID_NAPE {
            return Err(crate::family::not_offered(c));
        }
        m6_change(port, pid, c)
    }

    /// One setting the way the model's OpenMouse client writes it, then the read-back that confirms it. Before the write, the
    /// reads for undo ("before").
    fn write(&self, m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        let pid = the_variant(m)?;
        if pid == PID_NAPE {
            return match s {
                Setting::Dpi(dpi) => nape_set_dpi(port, dpi),
                Setting::Polling(hz) => nape_set_polling_rate(port, hz),
                Setting::LiftOff(_) => Err(Error::range("lift-off distance", "the Nape Pro has no lift-off setting".to_string())),
            };
        }
        m6_write(port, pid, s)
    }
}

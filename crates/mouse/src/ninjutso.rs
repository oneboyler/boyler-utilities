//! Ninjutso (Sora V2, Sora V3, TEN) - ported from OpenMouse (`crate::family`: AGPL-3.0-or-later, @openmouse/protocol 0.40.1):
//! DPI, polling rate and battery. Two protocols, as in OpenMouse's NinjaForce-derived client:
//!
//! - the codec: `ninjutso/index.js` (command table, [`current_request`], [`encode_dpi`] / [`decode_dpi`], the legacy frames);
//! - the client: `drivers/ninjutso/hid.js` (NinjutsoHidClient). "current" = vendor 0x093a, feature report 6 (15-byte payload):
//!   Sora V3 0xe010 and TEN 0xe020 / 0xea01. "legacy" = vendor 0x1915, feature reports 5 (31 bytes) and 4 (703 bytes, or 73 on
//!   a product named "Ninjutso Sora V2 8K"): Sora V2 0xae11 - 0xae16.
//!
//! Only the ids on OpenMouse's SUPPORTED list ("Sora V2" 0xae11-0xae16, "Sora V3" 0xe010, "TEN" 0xe020 / 0xea01) are matched; they
//! are all the mouse's own (wired) ids. The receivers the driver also knows (0xeb01 / 0xeb02, 0xae1c / 0xae8c / 0xae8a) are not on
//! the list and are not matched.
//!
//! Current protocol, Windows' buffer (byte 0 = report id 6; OpenMouse's payload byte i is our i + 1):
//! `[6, command, 0, 0, 1, 0, argument count, profile, arguments..]`, zero padded to 16 bytes; sent with `HidD_SetFeature`, then a
//! pause (30 ms, 60 ms on a re-try), then `HidD_GetFeature(6)`; the answer is good when `answer[1] == command` and its value is
//! `answer[8..]` (up to 4 tries). Reads: 16 profile, 28 active DPI stage, 18 battery %, 17 charging, 4 DPI of a stage, 6 polling.
//! Writes: 3 set DPI (`stage, lo, hi, 0`), 5 set polling (index in 1000/2000/4000/8000 + 1). The DPI write is wrapped in
//! `setControl(false)` / `setControl(true)`: feature report 3, 63 bytes, `[27, 26, 0, 0, 1]` then `[28, 27, 0, 0, 1]`, 30 ms each -
//! what those two do is unclear, OpenMouse sends them around every DPI write.
//!
//! Legacy protocol: profile query (report 5, command 13, answer byte 9), then the settings dump (report 4, request
//! `[38, 0, 0, 1, 0, profile]`, 90 ms, answer checked by a sum of its first 701 / 71 bytes against the little-endian u16 after
//! it, settings = `answer[9..]`): data[7] battery %, [11] stage count, [12 + 2 i] stage i DPI (u16 LE, `(v + 1) * 50`), [20] active
//! stage. A DPI write replaces the active stage in OpenMouse's 4-slot table and writes the whole table back with
//! `[5, 0, 0, 1, 0, profile, 22, 0, stage count, 8 bytes of stages]` - exactly as OpenMouse does, which also writes 0 into the
//! slots beyond the stage count.
//!
//! Left out on purpose: lift-off (OpenMouse's levels are Low / Medium / High with no millimetre figure), polling on Sora V2 (its
//! mouse ids offer 1000 Hz only), every other command (sleep, system mode, HyperClick, Slam-Click, optical engine, angle tuning,
//! lighting, firmware, DPI stage count / active stage).
//! Order 110 (A_110_02 scope): Motion Sync - the only one of debounce / Motion Sync / angle snapping / ripple / turbo OpenMouse's
//! NinjutsoHidClient has. Current protocol: read command 14 (optional: one ask; not answered = no row, as OpenMouse hides it),
//! `sendCurrent(13, [on])` then 14 confirms it. Sora V2: dump data[23]; report-5 command 6 `[6, 0, 0, 1, 0, profile, 22, 0, on]`
//! between two settings dumps. The "current" answers are read as OpenMouse reads them: the
//! report id stays in byte 0 (unclear in OpenMouse: its request payload has the arguments at byte 7 but its answers are read at
//! byte 8, which only fits if the id is kept).
//! Proven only against OpenMouse's bytes (tests); never run on a real unit.

use crate::error::{Error, Result};
use crate::family::{check_extra, Caps, Change, Changed, DpiCaps, Extra, ExtraRow, Family, Kind as EKind, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

/// NINJUTSO_VENDOR_ID (shared with Glorious Pixart and Incott; disjoint by product id)
pub const VENDOR_ID: u16 = 0x093A;
/// NINJUTSO_LEGACY_VENDOR_ID
pub const LEGACY_VENDOR_ID: u16 = 0x1915;
pub const REPORT_ID: u8 = 6;
pub const PAYLOAD_LEN: usize = 15;
pub const CONTROL_REPORT_ID: u8 = 3;
pub const CONTROL_PAYLOAD_LEN: usize = 63;
pub const LEGACY_REPORT_ID: u8 = 5;
pub const LEGACY_PAYLOAD_LEN: usize = 31;
pub const LEGACY_DATA_REPORT_ID: u8 = 4;

/// NINJUTSO_COMMAND
pub const CMD_PROFILE: u8 = 16;
pub const CMD_ACTIVE_DPI_STAGE: u8 = 28;
pub const CMD_BATTERY_PERCENT: u8 = 18;
pub const CMD_BATTERY_CHARGING: u8 = 17;
pub const CMD_DPI: u8 = 4;
pub const CMD_POLLING: u8 = 6;
pub const CMD_SET_DPI: u8 = 3;
pub const CMD_SET_POLLING: u8 = 5;
/// Order 110: Motion Sync read / set (current protocol); the Sora V2's report-5 command 6.
pub const CMD_MOTION_SYNC: u8 = 14;
pub const CMD_SET_MOTION_SYNC: u8 = 13;
pub const LEGACY_SET_MOTION_SYNC: u8 = 6;

/// NINJUTSO_POLLING_RATES
pub const POLLING: [u32; 4] = [1000, 2000, 4000, 8000];

/// Which protocol a model speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Sora V3: DPI sent directly (1 - 45 000, step 1)
    DirectV3,
    /// TEN: DPI index of 50 (50 - 30 000, step 50)
    Ten,
    /// Sora V2
    Legacy,
}

/// One id of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Product {
    pub vid: u16,
    pub pid: u16,
    /// name on OpenMouse's SUPPORTED list
    pub model: &'static str,
    pub kind: Kind,
}

const fn p(vid: u16, pid: u16, model: &'static str, kind: Kind) -> Product {
    Product { vid, pid, model, kind }
}

/// OpenMouse's SUPPORTED list (status "supported"): Sora V2 0xae11-0xae16 (vendor 0x1915), Sora V3 0xe010, TEN 0xe020 / 0xea01.
pub const PRODUCTS: [Product; 9] = [
    p(LEGACY_VENDOR_ID, 0xAE11, "Ninjutso Sora V2", Kind::Legacy),
    p(LEGACY_VENDOR_ID, 0xAE12, "Ninjutso Sora V2", Kind::Legacy),
    p(LEGACY_VENDOR_ID, 0xAE13, "Ninjutso Sora V2", Kind::Legacy),
    p(LEGACY_VENDOR_ID, 0xAE14, "Ninjutso Sora V2", Kind::Legacy),
    p(LEGACY_VENDOR_ID, 0xAE15, "Ninjutso Sora V2", Kind::Legacy),
    p(LEGACY_VENDOR_ID, 0xAE16, "Ninjutso Sora V2", Kind::Legacy),
    p(VENDOR_ID, 0xE010, "Ninjutso Sora V3", Kind::DirectV3),
    p(VENDOR_ID, 0xE020, "Ninjutso TEN", Kind::Ten),
    p(VENDOR_ID, 0xEA01, "Ninjutso TEN", Kind::Ten),
];

pub fn product(vid: u16, pid: u16) -> Option<&'static Product> {
    PRODUCTS.iter().find(|p| p.vid == vid && p.pid == pid)
}

// ---------------------------------------------------------------- current protocol: codec

/// The commands whose request carries the profile (readCurrent's `profileCommands`).
fn uses_profile(command: u8) -> bool {
    matches!(command, 28 | 30 | 4 | 6 | 8 | 10 | 14 | 12 | 23 | 42 | 50 | 25 | 32 | 34 | 38 | 40 | 48)
}

/// `ninjutsoBuildRequest(command, profile, args)` as Windows sends it: the report id, the 15-byte payload
/// `[command, 0, 0, 1, 0, args.length, profile, args..]`.
pub fn current_request(command: u8, profile: u8, args: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; PAYLOAD_LEN + 1];
    v[0] = REPORT_ID;
    v[1] = command;
    v[4] = 1;
    v[6] = args.len() as u8;
    v[7] = profile;
    v[8..8 + args.len()].copy_from_slice(args);
    v
}

/// The request a READ of `command` sends: the profile only for the commands `readCurrent` gives it to.
pub fn read_request(command: u8, profile: u8, args: &[u8]) -> Vec<u8> {
    current_request(command, if uses_profile(command) { profile } else { 0 }, args)
}

/// `ninjutsoResponseValue`: `None` unless the answer is at least 9 bytes and echoes the command in byte 1; the value is byte 8 on.
pub fn response_value(answer: &[u8], command: u8) -> Option<&[u8]> {
    (answer.len() >= 9 && answer[1] == command).then(|| &answer[8..])
}

/// `ninjutsoEncodeDpi`: `[lo, hi, 0]`, or `None` outside what the model accepts.
pub fn encode_dpi(dpi: u32, direct: bool) -> Option<[u8; 3]> {
    let (min, max, step) = if direct { (1, 45_000, 1) } else { (50, 30_000, 50) };
    if !(min..=max).contains(&dpi) || !dpi.is_multiple_of(step) {
        return None;
    }
    let e = if direct { dpi } else { dpi / 50 - 1 };
    Some([e as u8, (e >> 8) as u8, 0])
}

/// `ninjutsoDecodeDpi(low, high, tenth, direct)`; the Sora V3's tenth of a DPI is dropped (whole DPI).
pub fn decode_dpi(low: u8, high: u8, direct: bool) -> u32 {
    let e = ((high as u32) << 8) | low as u32;
    if direct {
        e
    } else {
        (e + 1) * 50
    }
}

/// `ninjutsoEncodePollingRate`: 1000 -> 1, 2000 -> 2, 4000 -> 3, 8000 -> 4.
pub fn encode_polling(hz: u32) -> Option<u8> {
    POLLING.iter().position(|r| *r == hz).map(|i| i as u8 + 1)
}

/// `ninjutsoDecodePollingRate`.
pub fn decode_polling(code: u8) -> Option<u32> {
    (code as usize).checked_sub(1).and_then(|i| POLLING.get(i)).copied()
}

/// `setControl(resume)`: the feature report 3 OpenMouse sends before (`false`) and after (`true`) a DPI write.
pub fn control_request(resume: bool) -> Vec<u8> {
    let mut v = vec![0u8; CONTROL_PAYLOAD_LEN + 1];
    v[0] = CONTROL_REPORT_ID;
    let b: [u8; 5] = if resume { [28, 27, 0, 0, 1] } else { [27, 26, 0, 0, 1] };
    v[1..6].copy_from_slice(&b);
    v
}

// ---------------------------------------------------------------- legacy protocol: codec

/// `ninjutsoLegacyRequest(command, profile, value)` as Windows sends it (report 5).
pub fn legacy_request(command: u8, profile: u8, value: Option<u8>) -> Vec<u8> {
    let mut v = vec![0u8; LEGACY_PAYLOAD_LEN + 1];
    v[0] = LEGACY_REPORT_ID;
    v[1] = command;
    v[4] = 1;
    v[6] = profile;
    v[7] = 22;
    if let Some(x) = value {
        v[9] = x;
    }
    v
}

/// The settings dump's payload length: 73 for a product named "Ninjutso Sora V2 8K", else 703.
pub fn legacy_data_len(product_name: Option<&str>) -> usize {
    if product_name == Some("Ninjutso Sora V2 8K") {
        73
    } else {
        703
    }
}

/// The settings dump request (report 4): `[38, 0, 0, 1, 0, profile]` in a zero payload of `payload_len` bytes.
pub fn legacy_data_request(profile: u8, payload_len: usize) -> Vec<u8> {
    let mut v = vec![0u8; payload_len + 1];
    v[0] = LEGACY_DATA_REPORT_ID;
    v[1] = 38;
    v[4] = 1;
    v[6] = profile;
    v
}

/// `ninjutsoLegacyEncodeDpi`.
pub fn legacy_encode_dpi(dpi: u32) -> Option<[u8; 2]> {
    if !(50..=30_000).contains(&dpi) || !dpi.is_multiple_of(50) {
        return None;
    }
    let e = dpi / 50 - 1;
    Some([e as u8, (e >> 8) as u8])
}

/// `ninjutsoLegacyDecodeDpi`.
pub fn legacy_decode_dpi(low: u8, high: u8) -> u32 {
    (((high as u32) << 8) | low as u32) * 50 + 50
}

/// `readLegacySettings`' check: the sum of the first 701 (71) bytes against the little-endian u16 at 702 (72); the settings are
/// `answer[9..]`. `payload_len` is 703 / 73.
pub fn legacy_settings(answer: &[u8], payload_len: usize) -> Result<Vec<u8>> {
    let (sum_end, at) = if payload_len == 73 { (71, 72) } else { (701, 702) };
    // an all-zero answer sums to 0 and "matches" the 0 behind it: it must also be a report 4 (Windows keeps the id in byte 0) and
    // carry a non-zero sum, or a dead buffer would show a Sora V2 online at 50 DPI and 0 % battery
    if answer.first() != Some(&LEGACY_DATA_REPORT_ID) {
        return Err(Error::BadAnswer("The Sora V2 settings answer is not report 4".into()));
    }
    let sum: u32 = answer.iter().take(sum_end).map(|b| *b as u32).sum();
    if sum == 0 {
        return Err(Error::BadAnswer("The Sora V2 settings answer is empty".into()));
    }
    let g = |i: usize| answer.get(i).copied().unwrap_or(0) as u32;
    let expected = (g(at + 1) << 8) | g(at);
    if sum != expected {
        return Err(Error::BadAnswer("The Sora V2 settings checksum did not match".into()));
    }
    Ok(answer.get(9..).unwrap_or(&[]).to_vec())
}

/// What a legacy settings dump says (the fields this file shows).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyState {
    pub profile: u8,
    pub stage_count: usize,
    pub stage: usize,
    /// the four DPI slots, decoded
    pub stages: [u32; 4],
    pub battery: u8,
    /// Order 110: Motion Sync (data[23] === 1)
    pub motion_sync: bool,
}

impl LegacyState {
    pub fn from_data(profile: u8, data: &[u8]) -> LegacyState {
        let at = |i: usize| data.get(i).copied().unwrap_or(0);
        let stage_count = (at(11) as usize).clamp(1, 4);
        let stage = (at(20) as usize).min(stage_count - 1);
        let mut stages = [0u32; 4];
        for (i, s) in stages.iter_mut().enumerate() {
            *s = legacy_decode_dpi(at(12 + i * 2), at(13 + i * 2));
        }
        LegacyState { profile, stage_count, stage, stages, battery: at(7).min(100), motion_sync: at(23) == 1 }
    }
    pub fn dpi(&self) -> u32 {
        self.stages[self.stage]
    }
}

/// `setLegacyDpi`'s write: `ninjutsoLegacyRequest(5, profile)` with `[8]` = stage count and the four slots (0 beyond the count).
pub fn legacy_dpi_write(state: &LegacyState, dpi: u32) -> Option<Vec<u8>> {
    let mut stages = state.stages;
    stages[state.stage] = dpi;
    let mut v = legacy_request(5, state.profile, None);
    v[9] = state.stage_count as u8;
    for (i, s) in stages.iter().enumerate() {
        let b = if i < state.stage_count { legacy_encode_dpi(*s)? } else { [0, 0] };
        v[10 + i * 2] = b[0];
        v[11 + i * 2] = b[1];
    }
    Some(v)
}

// ---------------------------------------------------------------- table, caps, allow-list

pub fn caps(p: &Product) -> Caps {
    let dpi = match p.kind {
        Kind::DirectV3 => Some(DpiCaps::uniform(1, 45_000, 1)),
        Kind::Ten => Some(DpiCaps::uniform(50, 30_000, 50)),
        // a Sora V2's DPI is read, not set (see `write`)
        Kind::Legacy => None,
    };
    Caps { dpi, polling: if p.kind == Kind::Legacy { Vec::new() } else { POLLING.to_vec() }, lift_off: Vec::new(), battery: true }
}

/// The collection OpenMouse talks to: current = the one with feature report 6 (a vendor page first, then any whose feature report
/// is long enough for 1 + 15 bytes); legacy = the one with the 704-byte (74 on an "8K") report 4 next to report 5.
pub fn config_interface<'a>(own: &[&'a HidInfo], p: &Product) -> Option<&'a HidInfo> {
    match p.kind {
        Kind::Legacy => own.iter().find(|h| h.feature_len as usize == legacy_data_len(h.product.as_deref()) + 1).copied(),
        _ => {
            let fits = |h: &&&HidInfo| h.feature_len as usize > PAYLOAD_LEN;
            own.iter().filter(fits).find(|h| h.usage_page >= 0xFF00).or_else(|| own.iter().find(fits)).copied()
        }
    }
}

/// OpenMouse's isSupported for the ids of the SUPPORTED list: vendor id, product id, and the collection with the feature reports.
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    let p = product(vid, pid)?;
    let config = config_interface(own, p)?;
    Some(Matched { family: Family::Ninjutso, model: Some(p.model), wireless: Some(false), config: config.clone(), variant: pid })
}

/// The allow-list for the real mouse: the READ requests of readStatus - current: report 6 frames for commands 16, 28, 18, 17,
/// 4 (one stage argument) and 6 in exactly the shape [`read_request`] builds; legacy: the profile query (report 5, command 13)
/// and the settings dump request (report 4, command 38), each the exact request followed by zero padding only (the SetFeature buffer
/// is padded to the collection's feature-report length). Writes (commands 3 and 5, report 3, legacy command 5) never pass.
pub fn read_request_allowed(out: &[u8]) -> bool {
    // the exact request, then zero padding only (Windows wants a SetFeature buffer of the collection's whole feature-report length)
    let zero_after = |from: usize| out.len() >= from && out[from..].iter().all(|b| *b == 0);
    match out.first() {
        Some(&REPORT_ID) if zero_after(PAYLOAD_LEN + 1) => {
            let (command, profile) = (out[1], out[7]);
            let shaped = |args: &[u8]| out[..PAYLOAD_LEN + 1] == *read_request(command, profile, args).as_slice();
            match command {
                CMD_PROFILE | CMD_BATTERY_PERCENT | CMD_BATTERY_CHARGING => shaped(&[]),
                CMD_ACTIVE_DPI_STAGE | CMD_POLLING => shaped(&[]),
                CMD_DPI => out[8] <= 3 && shaped(&[out[8]]),
                CMD_MOTION_SYNC => shaped(&[]),
                _ => false,
            }
        }
        Some(&LEGACY_REPORT_ID) => out.len() > LEGACY_PAYLOAD_LEN && out[..LEGACY_PAYLOAD_LEN + 1] == *legacy_request(13, 0, None).as_slice() && zero_after(LEGACY_PAYLOAD_LEN + 1),
        Some(&LEGACY_DATA_REPORT_ID) => {
            (out.len() == 74 || out.len() == 704) && out == legacy_data_request(out.get(6).copied().unwrap_or(0), out.len() - 1).as_slice()
        }
        _ => false,
    }
}

// ---------------------------------------------------------------- the driver

/// The Ninjutso driver (both protocols; the model decides).
pub struct Ninjutso;

fn the_product(m: &Matched) -> Result<&'static Product> {
    product(m.config.vid, m.variant).ok_or_else(|| Error::UnsupportedMouse(format!("Ninjutso {:04x}:{:04x}", m.config.vid, m.variant)))
}

/// A SetFeature buffer zero-padded to the collection's feature-report length (Windows rejects a shorter one; hidapi documents it).
fn padded(mut v: Vec<u8>, port: &dyn Port) -> Vec<u8> {
    let len = (port.info().feature_len as usize).max(v.len());
    v.resize(len, 0);
    v
}

fn reply_len(port: &dyn Port, min: usize) -> usize {
    (port.info().feature_len as usize).max(min)
}

/// `readCurrent`: the request, a pause, the answer; up to `attempts` times. `None` = never answered (NinjutsoCommandTimeoutError).
fn read_current(port: &mut dyn Port, command: u8, profile: u8, args: &[u8], attempts: u32) -> Result<Option<Vec<u8>>> {
    let req = padded(read_request(command, profile, args), port);
    for attempt in 0..attempts {
        let how = HidTransfer::Feature { reply_id: REPORT_ID, reply_len: reply_len(port, PAYLOAD_LEN + 1), wait_ms: if attempt == 0 { 30 } else { 60 } };
        match port.exchange(&req, &how) {
            Ok(a) => {
                if let Some(v) = response_value(&a, command) {
                    return Ok(Some(v.to_vec()));
                }
            }
            Err(Error::BadAnswer(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

fn must(v: Option<Vec<u8>>, what: &str) -> Result<Vec<u8>> {
    v.ok_or_else(|| Error::BadAnswer(format!("The Ninjutso mouse did not answer {what}")))
}

/// One exchange; `None` = the mouse did not answer in time.
fn try_exchange(port: &mut dyn Port, out: &[u8], how: &HidTransfer) -> Result<Option<Vec<u8>>> {
    match port.exchange(out, how) {
        Ok(a) => Ok(Some(a)),
        Err(Error::BadAnswer(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

/// `readLegacySettings`: the profile query (once more when it answers 0), then the settings dump. `None` = no answer; a dump
/// whose checksum is wrong is an error.
fn read_legacy(port: &mut dyn Port, m: &Matched) -> Result<Option<LegacyState>> {
    let q = padded(legacy_request(13, 0, None), port);
    let how = HidTransfer::Feature { reply_id: LEGACY_REPORT_ID, reply_len: reply_len(port, LEGACY_PAYLOAD_LEN + 1), wait_ms: 15 };
    let Some(a) = try_exchange(port, &q, &how)? else { return Ok(None) };
    let mut profile = a.get(9).copied().unwrap_or(0);
    if profile == 0 {
        let Some(a) = try_exchange(port, &q, &how)? else { return Ok(None) };
        profile = a.get(9).copied().unwrap_or(0);
    }
    let len = legacy_data_len(m.config.product.as_deref());
    let how = HidTransfer::Feature { reply_id: LEGACY_DATA_REPORT_ID, reply_len: reply_len(port, len + 1), wait_ms: 90 };
    let dump_request = padded(legacy_data_request(profile, len), port);
    let Some(a) = try_exchange(port, &dump_request, &how)? else { return Ok(None) };
    Ok(Some(LegacyState::from_data(profile, &legacy_settings(&a, len)?)))
}

/// readLegacySettings that must answer (the settings around a Sora V2 Motion Sync write).
fn legacy_state(port: &mut dyn Port, m: &Matched) -> Result<LegacyState> {
    read_legacy(port, m)?.ok_or_else(|| Error::BadAnswer("The Sora V2 did not answer".into()))
}

impl Ninjutso {
    fn read_current_status(&self, p: &Product, port: &mut dyn Port, opt: ReadOptions, r: &mut OnMouse) -> Result<()> {
        let attempts = opt.online_tries.clamp(1, 4);
        // readCurrentStatus order: profile, active stage, battery, charging, DPI of that stage, polling
        let Some(profile) = read_current(port, CMD_PROFILE, 0, &[], attempts)? else {
            r.online = false;
            return Ok(());
        };
        let profile = profile[0].max(1);
        let stage = must(read_current(port, CMD_ACTIVE_DPI_STAGE, profile, &[], attempts)?, "the active DPI stage")?;
        let battery = must(read_current(port, CMD_BATTERY_PERCENT, 0, &[], attempts)?, "the battery")?;
        let charging = must(read_current(port, CMD_BATTERY_CHARGING, 0, &[], attempts)?, "the charging state")?;
        let dpi = must(read_current(port, CMD_DPI, profile, &[stage[0]], attempts)?, "the DPI")?;
        let polling = must(read_current(port, CMD_POLLING, profile, &[], attempts)?, "the polling rate")?;
        let direct = p.kind == Kind::DirectV3;
        r.battery_percent = Some(battery[0].min(100));
        r.charging = Some(charging[0] != 0);
        r.dpi = Some(decode_dpi(dpi[0], dpi.get(1).copied().unwrap_or(0), direct));
        r.polling_hz = decode_polling(polling[0]);
        // Order 110: Motion Sync, optional (readCurrentOptional: one ask; not answered = OpenMouse hides it)
        if let Some(v) = read_current(port, CMD_MOTION_SYNC, profile, &[], 1)? {
            r.extras = vec![ExtraRow::switch(Extra::MotionSync, Some(v[0] == 1))];
        }
        Ok(())
    }

    fn write_current(&self, p: &Product, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        let direct = p.kind == Kind::DirectV3;
        let tries = 4;
        let profile = must(read_current(port, CMD_PROFILE, 0, &[], tries)?, "the profile")?[0].max(1);
        match s {
            Setting::Dpi(dpi) => {
                let stage = must(read_current(port, CMD_ACTIVE_DPI_STAGE, profile, &[], tries)?, "the active DPI stage")?[0];
                let enc = encode_dpi(dpi, direct).ok_or_else(|| {
                    Error::range("DPI", if direct { "Ninjutso DPI must be 1-45,000".to_string() } else { "Ninjutso DPI must be 50-30,000 in 50 DPI steps".to_string() })
                })?;
                let before = read_current(port, CMD_DPI, profile, &[stage], tries)?.map(|v| decode_dpi(v[0], v.get(1).copied().unwrap_or(0), direct));
                port.exchange(&padded(control_request(false), port), &HidTransfer::FeatureOut)?;
                port.pause_ms(30);
                let sent = port.exchange(&padded(current_request(CMD_SET_DPI, profile, &[stage, enc[0], enc[1], enc[2]]), port), &HidTransfer::FeatureOut);
                // the `finally`: resume even when the write failed
                let resumed = port.exchange(&padded(control_request(true), port), &HidTransfer::FeatureOut);
                port.pause_ms(30);
                sent?;
                resumed?;
                let v = must(read_current(port, CMD_DPI, profile, &[stage], tries)?, "the DPI")?;
                let confirmed = decode_dpi(v[0], v.get(1).copied().unwrap_or(0), direct);
                if confirmed != dpi {
                    return Err(Error::BadAnswer(format!("The mouse kept {confirmed} DPI instead of {dpi} DPI")));
                }
                Ok(Changed { now: confirmed, before })
            }
            Setting::Polling(hz) => {
                let code = encode_polling(hz).ok_or_else(|| Error::range("polling rate", format!("This Ninjutso connection does not support {hz} Hz")))?;
                let before = read_current(port, CMD_POLLING, profile, &[], tries)?.and_then(|v| decode_polling(v[0]));
                port.exchange(&padded(current_request(CMD_SET_POLLING, profile, &[code]), port), &HidTransfer::FeatureOut)?;
                let v = must(read_current(port, CMD_POLLING, profile, &[], tries)?, "the polling rate")?;
                let confirmed = decode_polling(v[0]).unwrap_or(0);
                if confirmed != hz {
                    return Err(Error::BadAnswer(format!("The mouse kept {confirmed} Hz instead of {hz} Hz")));
                }
                Ok(Changed { now: confirmed, before })
            }
            Setting::LiftOff(_) => Err(Error::range("lift-off distance", "this app can't set a Ninjutso mouse's lift-off (OpenMouse's levels have no millimetre figure)".to_string())),
        }
    }
}

impl crate::family::Driver for Ninjutso {
    /// OpenMouse's readStatus for the values this file shows. A mouse that does not answer the first request is offline.
    fn read(&self, m: &Matched, port: &mut dyn Port, opt: ReadOptions) -> Result<OnMouse> {
        let p = the_product(m)?;
        let mut r = OnMouse { online: true, wireless: Some(false), caps: caps(p), ..Default::default() };
        if p.kind == Kind::Legacy {
            match read_legacy(port, m)? {
                Some(st) => {
                    r.dpi = Some(st.dpi());
                    r.battery_percent = Some(st.battery);
                    r.extras = vec![ExtraRow::switch(Extra::MotionSync, Some(st.motion_sync))];
                }
                None => r.online = false,
            }
        } else {
            self.read_current_status(p, port, opt, &mut r)?;
        }
        Ok(r)
    }

    /// Order 110: setMotionSync, checked before a byte leaves. Current: the profile, `sendCurrent(13, [on])`, command 14
    /// confirms it. Sora V2: the settings (for the profile), report-5 command 6, the settings again (data[23]).
    fn change(&self, m: &Matched, port: &mut dyn Port, c: Change) -> Result<()> {
        let p = the_product(m)?;
        let Change::Extra(e, v) = c;
        check_extra(&[ExtraRow::new(Extra::MotionSync, EKind::Switch, None)], e, v)?;
        let on = v == 1;
        let now = if p.kind == Kind::Legacy {
            let st = legacy_state(port, m)?;
            port.exchange(&padded(legacy_request(LEGACY_SET_MOTION_SYNC, st.profile, Some(on as u8)), port), &HidTransfer::FeatureOut)?;
            legacy_state(port, m)?.motion_sync
        } else {
            let profile = must(read_current(port, CMD_PROFILE, 0, &[], 4)?, "the profile")?[0].max(1);
            port.exchange(&padded(current_request(CMD_SET_MOTION_SYNC, profile, &[on as u8]), port), &HidTransfer::FeatureOut)?;
            must(read_current(port, CMD_MOTION_SYNC, profile, &[], 4)?, "Motion Sync")?[0] == 1
        };
        if now != on {
            return Err(Error::BadAnswer(format!("The mouse left Motion Sync {}.", if now { "on" } else { "off" })));
        }
        Ok(())
    }

    /// OpenMouse's setDpi / setPollingRate with the read-back it confirms with (plus one read before, for undo).
    fn write(&self, m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        let p = the_product(m)?;
        if p.kind != Kind::Legacy {
            return self.write_current(p, port, s);
        }
        match s {
            // HELD BACK (Order 105): setLegacyDpi rewrites the mouse's whole 4-slot stage table and writes 0 into the slots beyond the
            // stage count - a stored DPI would be lost. The codec ([`legacy_dpi_write`]) stays, tested against OpenMouse's bytes.
            Setting::Dpi(_) => Err(Error::range("DPI", "this app does not set a Sora V2's DPI (OpenMouse's write replaces the whole stage table)".to_string())),
            Setting::Polling(hz) => Err(Error::range("polling rate", format!("{hz} Hz: a Sora V2 mouse offers 1000 Hz only"))),
            Setting::LiftOff(_) => Err(Error::range("lift-off distance", "this app can't set a Ninjutso mouse's lift-off (OpenMouse's levels have no millimetre figure)".to_string())),
        }
    }
}

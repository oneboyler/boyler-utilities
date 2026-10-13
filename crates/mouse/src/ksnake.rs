//! K-snake X11 and Noir Gear M2-NEX - 64-byte output reports with input-report answers, ported from OpenMouse
//! (`crate::family`: AGPL-3.0-or-later), DPI / polling / battery:
//!
//! - the codec: `ksnake/index.js` (the GET_CONFIG / SET_CONFIG / GET_BATTERY requests, ksnakeDecodeConfig / DecodeBattery,
//!   the polling and DPI limits, isNoirM2NexDevice);
//! - the driver: `drivers/ksnake/hid.js` (KsnakeHidClient: readStatus, setDpi, setPollingRate, the resend-on-timeout and
//!   read-back-after-250-ms flow). The device table is the SUPPORTED list's "K-snake X11" (pid 0x2255 on vendor 0xA8A4 =
//!   cable, 0xA8A5 = 2.4 GHz dongle) and "Noir Gear M2-NEX" (no ids on the list: it is an OEM rebrand of the X11 with the SAME
//!   ids; OpenMouse tells it apart only by the product string "M2-NEX" - `registry.js deviceBrand` / `isNoirM2NexDevice`).
//!   `drivers/noir/m1-nex-hid.js` is the Noir M1-NEX (a different mouse on the Lamzu Atlantis client, not on the list) - not
//!   used. `variant` is [`VARIANT_X11`] or [`VARIANT_M2_NEX`].
//!
//! Packet (Windows counts the report id): a request is WriteFile of `[0x00, 0x55, command, 8 bytes of per-command tail, zeros]`
//! = 65 bytes; the answer is the next input report (64 bytes behind report id 0); OpenMouse's reply[i] is our byte i + 1. The
//! reply is not checked for an echo, so a stray report on the dongle can be taken for the answer: OpenMouse validates what it
//! decodes and asks once more, and so does this file.
//!
//! WRITES ARE HELD BACK ([`HOLD_WRITES`]). The mouse has no per-setting command: every change is SET_CONFIG, a full-block
//! rewrite built from the decoded fields (OpenMouse's `ksnakeEncodeSetConfig`). It does NOT copy the block back byte for
//! byte: reply bytes 25..47 (23 bytes), 54 and 56.. are neither decoded nor sent (they go out as 0), the wake-up / move-light
//! flags are read from reply[55] but written to byte 54 with the two nibbles swapped (OpenMouse: "mirrors the vendor
//! byte-for-byte"), and a blank / erased config is replaced by OpenMouse's own defaults. What those bytes hold is unclear. By
//! the order's rule (a setter that replaces saved data we do not read back is held back) `write` refuses, and `caps` offers
//! no DPI / polling row. The whole write flow is ported and proven against OpenMouse's bytes in [`apply`]; switching
//! [`HOLD_WRITES`] to `false` turns it on (the lead's call).
//!
//! Left out on purpose: lift-off (the mouse stores 1 or 2, OpenMouse calls them Low / High and only guesses "likely 1 mm /
//! 2 mm" - no millimetre figure to show), DPI stage select / edit, sleep, scroll direction, lighting, key map, macros (none is
//! DPI / polling / lift-off / battery), the version query. OpenMouse sends no firmware, DFU or reset command and neither does
//! this file.

use crate::error::{Error, Result};
use crate::family::{Caps, Changed, DpiCaps, Family, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

/// The cable (KSNAKE_USB_VENDOR_ID) and the 2.4 GHz dongle (KSNAKE_DONGLE_VENDOR_ID).
pub const VENDOR_USB: u16 = 0xA8A4;
pub const VENDOR_DONGLE: u16 = 0xA8A5;
pub const PRODUCT_ID: u16 = 0x2255;
pub const USAGE_PAGE: u16 = 0xFF01;
pub const USAGE: u16 = 0x10;
/// KSNAKE_REPORT_SIZE (the body) and Windows' report: report id 0 + the body.
pub const BODY_LEN: usize = 64;
pub const REPORT_LEN: usize = BODY_LEN + 1;
pub const MAGIC: u8 = 0x55;

pub const VARIANT_X11: u16 = 0;
pub const VARIANT_M2_NEX: u16 = 1;

/// Whether writes are held back (see the header). `apply` is the ported flow.
pub const HOLD_WRITES: bool = true;

const CMD_GET_CONFIG: u8 = 0x0E;
const CMD_SET_CONFIG: u8 = 0x0F;
const CMD_GET_BATTERY: u8 = 0x30;
const GET_CONFIG_TAIL: [u8; 8] = [0xA5, 0x0B, 0x2F, 0x01, 0x01, 0x00, 0x00, 0x00];
const SET_CONFIG_HEAD: [u8; 7] = [0xAE, 0x0A, 0x2F, 0x01, 0x01, 0x00, 0x00];
const GET_BATTERY_TAIL: [u8; 8] = [0xA5, 0x0B, 0x2E, 0x01, 0x01, 0x00, 0x00, 0x00];

/// KSNAKE_POLLING_RATES: index = the config's report-rate field.
pub const POLLING_RATES: [u32; 4] = [125, 250, 500, 1000];
pub const DPI_MIN: u32 = 200;
pub const DPI_MAX: u32 = 12000;
/// The vendor slider's step (getDpiOptions / dpiStageEditor).
pub const DPI_STEP: u32 = 100;

/// REPLY_TIMEOUT_MS and SETTLE_AFTER_WRITE_MS.
const REPLY_TIMEOUT_MS: u32 = 800;
const SETTLE_AFTER_WRITE_MS: u64 = 250;

// ---------------------------------------------------------------- the codec

fn body(command: u8, tail: &[u8]) -> [u8; BODY_LEN] {
    let mut b = [0u8; BODY_LEN];
    b[0] = MAGIC;
    b[1] = command;
    b[2..2 + tail.len()].copy_from_slice(tail);
    b
}

/// ksnakeGetConfigRequest.
pub fn get_config_request() -> [u8; BODY_LEN] {
    body(CMD_GET_CONFIG, &GET_CONFIG_TAIL)
}

/// ksnakeGetBatteryRequest.
pub fn get_battery_request() -> [u8; BODY_LEN] {
    body(CMD_GET_BATTERY, &GET_BATTERY_TAIL)
}

/// The output report Windows sends: report id 0, the 64-byte body, zero-padded to the collection's length.
pub fn report(b: &[u8; BODY_LEN], len: usize) -> Vec<u8> {
    let mut v = vec![0u8];
    v.extend_from_slice(b);
    v.resize(len.max(REPORT_LEN), 0);
    v
}

/// The decoded config block (ksnakeDecodeConfig's fields). `blank` = the mouse sent an erased / all-zero stage table and
/// OpenMouse substitutes its own defaults - the values are then NOT the mouse's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub light_mode: u8,
    /// index into [`POLLING_RATES`] (reply[10] - 1; may be out of range on a garbled reply)
    pub report_rate: i32,
    pub dpi_index: i32,
    pub dpi_count: u8,
    pub stages: [u32; 6],
    pub scroll_flag: u8,
    pub lod_value: u8,
    pub sensor_flag: u8,
    pub key_respond: u8,
    pub sleep_light: u8,
    pub highspeed_mode: u8,
    pub wakeup_flag: u8,
    pub move_light_flag: u8,
    pub blank: bool,
}

/// KSNAKE_DEFAULT_CONFIG.
fn default_config() -> Config {
    Config {
        light_mode: 2,
        report_rate: 3,
        dpi_index: 2,
        dpi_count: 6,
        stages: [800, 1200, 1600, 3200, 5000, 12000],
        scroll_flag: 0,
        lod_value: 1,
        sensor_flag: 53,
        key_respond: 2,
        sleep_light: 10,
        highspeed_mode: 0,
        wakeup_flag: 1,
        move_light_flag: 1,
        blank: true,
    }
}

fn le16(lo: u8, hi: u8) -> u32 {
    ((hi as u32) << 8) | lo as u32
}

/// ksnakeDecodeConfig: `reply` is OpenMouse's reply (no report id); at least 56 bytes. Index fields keep OpenMouse's
/// arithmetic (reply[10] - 1, reply[12] - 1), so a garbled 0 shows as -1 and fails the validity check.
pub fn decode_config(reply: &[u8]) -> Option<Config> {
    if reply.len() < 56 {
        return None;
    }
    let blank = reply[13] == 0 && reply[14] == 0 && reply[15] == 0;
    let erased = reply[13] == 255 && reply[14] == 255 && reply[15] == 255;
    if blank || erased {
        return Some(default_config());
    }
    let mut stages = [0u32; 6];
    for (i, s) in stages.iter_mut().enumerate() {
        *s = le16(reply[13 + 2 * i], reply[14 + 2 * i]);
    }
    Some(Config {
        light_mode: reply[9],
        report_rate: reply[10] as i32 - 1,
        dpi_index: reply[12] as i32 - 1,
        dpi_count: reply[11],
        stages,
        scroll_flag: reply[48],
        lod_value: reply[49],
        sensor_flag: reply[50],
        key_respond: reply[51],
        sleep_light: reply[52],
        highspeed_mode: reply[53],
        // the vendor's asymmetry, mirrored byte for byte: wake-up from the LOW nibble of reply[55]
        wakeup_flag: reply[55] & 15,
        move_light_flag: (reply[55] >> 4) & 15,
        blank: false,
    })
}

/// ksnakeEncodeSetConfig: the 64-byte body. Bytes 25..47, 55.. stay 0 (see the header).
pub fn encode_set_config(c: &Config) -> [u8; BODY_LEN] {
    let mut b = [0u8; BODY_LEN];
    b[0] = MAGIC;
    b[1] = CMD_SET_CONFIG;
    b[2..9].copy_from_slice(&SET_CONFIG_HEAD);
    b[9] = c.light_mode;
    b[10] = (c.report_rate + 1) as u8;
    b[11] = c.dpi_count;
    b[12] = (c.dpi_index + 1) as u8;
    for (i, s) in c.stages.iter().enumerate() {
        b[13 + 2 * i] = *s as u8;
        b[14 + 2 * i] = (*s >> 8) as u8;
    }
    b[48] = c.scroll_flag;
    b[49] = c.lod_value;
    b[50] = c.sensor_flag;
    b[51] = c.key_respond;
    b[52] = c.sleep_light;
    b[53] = c.highspeed_mode;
    // the nibbles go the other way round than the decode reads them
    b[54] = (c.wakeup_flag << 4) | (c.move_light_flag & 15);
    b
}

/// ksnakeDecodePollingRate.
pub fn polling_hz(index: i32) -> Option<u32> {
    usize::try_from(index).ok().and_then(|i| POLLING_RATES.get(i)).copied()
}

/// ksnakeEncodePollingRate.
pub fn polling_index(hz: u32) -> Option<i32> {
    POLLING_RATES.iter().position(|r| *r == hz).map(|i| i as i32)
}

/// ksnakeIsValidDpi: a whole number 200..=12000.
pub fn dpi_valid(dpi: u32) -> bool {
    (DPI_MIN..=DPI_MAX).contains(&dpi)
}

impl Config {
    /// The stage the mouse is on, when the index is one of its six.
    pub fn active_dpi(&self) -> Option<u32> {
        usize::try_from(self.dpi_index).ok().and_then(|i| self.stages.get(i)).copied()
    }
    /// readStatus' validity check: a known polling index and a DPI index inside the stage table.
    fn plausible(&self) -> bool {
        polling_hz(self.report_rate).is_some() && self.active_dpi().is_some()
    }
}

/// ksnakeDecodeBattery: (percent, charging) from reply[8], reply[9].
pub fn decode_battery(reply: &[u8]) -> Option<(u8, bool)> {
    (reply.len() >= 10).then(|| (reply[8], reply[9] != 0))
}

// ---------------------------------------------------------------- the device table

/// isNoirM2NexDevice: the product string, lower-cased with spaces, underscores and hyphens removed, is "m2nex".
pub fn is_noir_m2_nex(product: &str) -> bool {
    let compact: String = product.trim().to_lowercase().chars().filter(|c| !c.is_whitespace() && *c != '_' && *c != '-').collect();
    compact == "m2nex"
}

/// OpenMouse's isSupported: the pid and one of the two vendor ids, and the 0xFF01 / 0x10 collection (with room for a
/// report-id-0 64-byte output and input report). Which of the two mice by the product string (any interface's).
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if pid != PRODUCT_ID || (vid != VENDOR_USB && vid != VENDOR_DONGLE) {
        return None;
    }
    let config = own.iter().find(|h| h.usage_page == USAGE_PAGE && h.usage == USAGE && h.output_len as usize >= REPORT_LEN && h.input_len as usize >= REPORT_LEN)?;
    let m2 = own.iter().filter_map(|h| h.product.as_deref()).any(is_noir_m2_nex);
    let (model, variant) = if m2 { ("Noir Gear M2-NEX", VARIANT_M2_NEX) } else { ("K-snake X11", VARIANT_X11) };
    Some(Matched { family: Family::Ksnake, model: Some(model), wireless: Some(vid == VENDOR_DONGLE), config: (*config).clone(), variant })
}

/// What the driver offers. While writes are held back ([`HOLD_WRITES`]) only the battery; with them on, DPI 200..12000 in
/// 100s and the four polling rates.
pub fn caps() -> Caps {
    if HOLD_WRITES {
        Caps { battery: true, ..Default::default() }
    } else {
        full_caps()
    }
}

/// The caps once writes are on.
pub fn full_caps() -> Caps {
    Caps { dpi: Some(DpiCaps::uniform(DPI_MIN, DPI_MAX, DPI_STEP)), polling: POLLING_RATES.to_vec(), lift_off: Vec::new(), battery: true }
}

/// The allow-list for the read-only proof on a real mouse (A_009_01): exactly the GET_CONFIG or the GET_BATTERY request
/// (report id 0, the 64-byte body, nothing but zeros behind). SET_CONFIG and everything else is refused.
pub fn read_request_allowed(out: &[u8]) -> bool {
    out.len() >= REPORT_LEN && out[0] == 0 && out[REPORT_LEN..].iter().all(|b| *b == 0) && (out[1..REPORT_LEN] == get_config_request() || out[1..REPORT_LEN] == get_battery_request())
}

// ---------------------------------------------------------------- the exchange

/// `exchange`: WriteFile the request, take the next input report (reply without its report id). A timeout is `BadAnswer`.
fn exchange(port: &mut dyn Port, b: &[u8; BODY_LEN]) -> Result<Vec<u8>> {
    let (out_len, in_len) = (port.info().output_len as usize, port.info().input_len as usize);
    let how = HidTransfer::OutputThenInput { reply_len: in_len.max(REPORT_LEN), max_reads: 1, timeout_ms: REPLY_TIMEOUT_MS, echo: None };
    let a = port.exchange(&report(b, out_len), &how)?;
    Ok(a.get(1..).unwrap_or(&[]).to_vec())
}

/// exchangeRetrying: resend after a timeout (`BadAnswer`), up to `attempts`; any other failure ends it.
fn exchange_retrying(port: &mut dyn Port, b: &[u8; BODY_LEN], attempts: u32) -> Result<Vec<u8>> {
    let mut last = Error::BadAnswer("the mouse did not answer - it may be asleep or out of range".into());
    for _ in 0..attempts {
        match exchange(port, b) {
            Ok(r) => return Ok(r),
            Err(e @ Error::BadAnswer(_)) => last = e,
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

/// readValidated: two tries; `None` = nothing valid (silence or a stray report).
fn read_validated<T>(port: &mut dyn Port, b: &[u8; BODY_LEN], decode: impl Fn(&[u8]) -> Option<T>, valid: impl Fn(&T) -> bool) -> Result<Option<T>> {
    for _ in 0..2 {
        match exchange(port, b) {
            Ok(r) => {
                if let Some(v) = decode(&r).filter(|v| valid(v)) {
                    return Ok(Some(v));
                }
            }
            Err(Error::BadAnswer(_) | Error::Os { .. }) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// The config as the mouse reports it now (plausible and not blank - a blank one is asked once more, like a stray report), `None` when it is silent or has none.
fn read_config(port: &mut dyn Port) -> Result<Option<Config>> {
    read_validated(port, &get_config_request(), decode_config, |c| c.plausible() && !c.blank)
}

// ---------------------------------------------------------------- the driver

/// The K-snake / Noir Gear driver (KsnakeHidClient).
pub struct Ksnake;

impl crate::family::Driver for Ksnake {
    /// READ requests only: GET_CONFIG (DPI of the active stage, polling) and GET_BATTERY, each asked up to twice, one after
    /// the other. Both pass [`read_request_allowed`]. Silence on both = `online: false`. A blank / erased config shows no DPI
    /// and no polling (OpenMouse would show its defaults).
    fn read(&self, m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let mut r = OnMouse { model: m.model.map(str::to_string), wireless: m.wireless, caps: caps(), ..Default::default() };
        let config = read_config(port)?;
        let battery = read_validated(port, &get_battery_request(), decode_battery, |b| b.0 <= 100)?;
        r.online = config.is_some() || battery.is_some();
        if let Some(c) = config.filter(|c| !c.blank) {
            r.dpi = c.active_dpi();
            r.polling_hz = polling_hz(c.report_rate);
        }
        if let Some((pct, charging)) = battery {
            r.battery_percent = Some(pct);
            r.charging = Some(charging);
        }
        Ok(r)
    }

    fn write(&self, m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        if HOLD_WRITES {
            return Err(Error::UnsupportedMouse("K-snake / Noir Gear: changing settings is held back (SET_CONFIG rewrites bytes this app cannot read back)".into()));
        }
        apply(m, port, s)
    }
}

/// setDpi / setPollingRate the way KsnakeHidClient does them: read the config (3 tries), change one field, SET_CONFIG (resent
/// after a timeout, 2 tries), wait 250 ms, read the config back (3 tries, only a non-blank plausible answer counts) and compare.
/// Refuses a blank / erased or implausible config, and one that differs between two reads (the write is built from it).
/// Not reachable through [`Driver::write`] while [`HOLD_WRITES`] is on.
pub fn apply(_m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
    let next = |c: &Config| -> Result<(Config, u32, u32)> {
        match s {
            Setting::Dpi(dpi) => {
                if !dpi_valid(dpi) {
                    return Err(Error::range("DPI", format!("DPI must be a whole number between {DPI_MIN} and {DPI_MAX} (got {dpi})")));
                }
                let (before, mut n) = (c.active_dpi().ok_or_else(|| Error::BadAnswer("the mouse reported no active DPI stage".into()))?, c.clone());
                n.stages[c.dpi_index as usize] = dpi;
                Ok((n, before, dpi))
            }
            Setting::Polling(hz) => {
                let idx = polling_index(hz).ok_or_else(|| Error::range("polling rate", format!("this mouse does not support {hz} Hz")))?;
                let before = polling_hz(c.report_rate).ok_or_else(|| Error::BadAnswer("the mouse reported no polling rate".into()))?;
                Ok((Config { report_rate: idx, ..c.clone() }, before, hz))
            }
            Setting::LiftOff(_) => Err(Error::range("lift-off distance", "this app can't set a K-snake mouse's lift-off (the mouse has two stops with no millimetre figure)".to_string())),
        }
    };
    // checked before a byte leaves
    let probe = default_config();
    next(&probe)?;
    // GET_CONFIG has no command echo, so a stray report can look like a config. The whole block is about to be rewritten from it,
    // so it must be a real one: not blank / erased, plausible (known polling index, active stage inside the table), and the
    // SAME twice in a row.
    let read_once = |port: &mut dyn Port| -> Result<Config> {
        let raw = exchange_retrying(port, &get_config_request(), 3).map_err(|_| Error::BadAnswer("could not read the current config from the mouse".into()))?;
        let c = decode_config(&raw).ok_or_else(|| Error::BadAnswer("could not read the current config from the mouse".into()))?;
        if c.blank {
            return Err(Error::BadAnswer("the mouse reports an empty config; not overwriting it with defaults".into()));
        }
        if !c.plausible() {
            return Err(Error::BadAnswer("the mouse's config answer does not look like a config; not writing".into()));
        }
        Ok(c)
    };
    let config = read_once(port)?;
    if read_once(port)? != config {
        return Err(Error::BadAnswer("the mouse's config answer changed between two reads; not writing".into()));
    }
    let (changed, before, want) = next(&config)?;
    exchange_retrying(port, &encode_set_config(&changed), 2)?;
    port.pause_ms(SETTLE_AFTER_WRITE_MS);
    let mut back = None;
    for _ in 0..3 {
        if let Ok(r) = exchange(port, &get_config_request()) {
            // a blank / erased block would decode as OpenMouse's defaults (1600 / 1000): not a confirmation
            if let Some(c) = decode_config(&r).filter(|c| !c.blank && c.plausible()) {
                back = Some(c);
                break;
            }
        }
    }
    let got = back.as_ref().and_then(|c| match s {
        Setting::Dpi(_) => c.active_dpi(),
        _ => polling_hz(c.report_rate),
    });
    if got != Some(want) {
        return Err(Error::BadAnswer(format!("the mouse kept {got:?} instead of {want}")));
    }
    Ok(Changed { now: want, before: Some(before) })
}

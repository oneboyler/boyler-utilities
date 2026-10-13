//! Razer - 90-byte feature reports on report id 0, ported from OpenMouse (`crate::family`: AGPL-3.0-or-later), DPI /
//! polling / battery only:
//!
//! - the codec: `razer/codec.js` (encodeRazerRequest / decodeRazerResponse and the DPI, polling, battery commands) and
//!   `razer/v4.js` (the Viper V4 Pro's stage-table DPI and polling codes);
//! - the device table: `razer/devices.js` (RAZER_PRODUCTS: transaction id, DPI ceiling, polling ladder and command, battery,
//!   DPI store byte, vendor control interface) for the models on OpenMouse's SUPPORTED list, plus the three models with their
//!   own clients (`drivers/razer/cobra-hid.js`, `viper-mini-hid.js`, `viper-v4-pro-hid.js`);
//! - the drivers: `drivers/razer/hid.js` (RazerHidClient: readStatus / setDpi / setPollingRate / exchange) and the three
//!   model clients; which collection each one claims: their isSupported (`drivers/vendors.js` only filters the picker).
//!
//! Packet (OpenMouse counts WITHOUT the report id; Windows puts report id 0 in front, so OpenMouse's byte i is our i + 1):
//! `[0]` status (0 in requests; answers 0x01 busy, 0x02 ok, 0x03 failure, 0x04 timeout, 0x05 unsupported) · `[1]`
//! transaction id (per product: 0x1f, 0x3f or 0xff - a wrong one is silent) · `[2..5]` 0 · `[5]` data size · `[6]` command
//! class · `[7]` command id (high bit set = a read) · `[8..88]` arguments · `[88]` crc = XOR of bytes 2..87 · `[89]` 0.
//! A request goes out with HidD_SetFeature; its answer is fetched with HidD_GetFeature after OpenMouse's pause; a "busy"
//! answer is fetched again without sending the request again ([`HidTransfer::FeatureIn`]).
//!
//! Left out on purpose: lift-off. OpenMouse sets the Viper V2 Pro / V3 Pro / V3 HyperSpeed tracking distance as Low /
//! Medium / High (and an asymmetric 2..26 pair whose mode can only be found by a probing WRITE) with no millimetre figure
//! anywhere, and this app's lift-off is in tenths of a mm - a made-up mm mapping would mislabel the mouse. Also left out:
//! the stage editor, sleep timeout, low-power threshold, button mappings, lighting, serial (none is DPI / polling /
//! lift-off / battery). OpenMouse sends no firmware, DFU or reset command to a Razer mouse and neither does this file.

use crate::error::{Error, Result};
use crate::family::{Caps, Changed, DpiCaps, Family, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x1532;
pub const REPORT_ID: u8 = 0;
pub const PACKET_LEN: usize = 90;
/// Windows' feature report: report id 0 + the 90-byte packet.
pub const REPORT_LEN: usize = PACKET_LEN + 1;

/// Razer's three transaction ids (codec.js: named by value - none is a safe default).
pub const TID_FF: u8 = 0xff;
pub const TID_3F: u8 = 0x3f;
pub const TID_1F: u8 = 0x1f;

pub const STATUS_BUSY: u8 = 0x01;
pub const STATUS_OK: u8 = 0x02;
pub const STATUS_UNSUPPORTED: u8 = 0x05;

/// RAZER_STORAGE: the store byte DPI is read / written through (the V2 generation uses 0x00, "NOSTORE").
pub const STORE: u8 = 0x01;
pub const NOSTORE: u8 = 0x00;

/// One command: class, id, declared data size, arguments (OpenMouse's RazerCommand).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cmd {
    pub class: u8,
    pub id: u8,
    pub size: u8,
    pub args: Vec<u8>,
}

impl Cmd {
    fn new(class: u8, id: u8, size: u8, args: &[u8]) -> Cmd {
        Cmd { class, id, size, args: args.to_vec() }
    }
    /// isRazerGetter: Razer pairs each read with a write that clears the high bit of the command id.
    pub fn is_getter(&self) -> bool {
        self.id & 0x80 != 0
    }
}

// ---------------------------------------------------------------- the read requests (RAZER_READ)

/// Firmware version {0x00, 0x81, 0x02} - readStatus's first request (no answer = the mouse is not there).
pub fn read_firmware() -> Cmd {
    Cmd::new(0x00, 0x81, 0x02, &[])
}

/// Battery level {0x07, 0x80, 0x02}: answer `[1]` on a 0-255 scale.
pub fn read_battery() -> Cmd {
    Cmd::new(0x07, 0x80, 0x02, &[])
}

/// Charging {0x07, 0x84, 0x02}: answer `[1]` = 1 when charging.
pub fn read_charging() -> Cmd {
    Cmd::new(0x07, 0x84, 0x02, &[])
}

/// razerReadDpiCommand(store) {0x04, 0x85, 0x07, [store]}: answer `[1..3]` x, `[3..5]` y, big-endian.
pub fn read_dpi(store: u8) -> Cmd {
    Cmd::new(0x04, 0x85, 0x07, &[store])
}

/// The legacy polling read {0x00, 0x85, 0x01}: answer `[0]` = a divisor of 1000.
pub fn read_polling_legacy() -> Cmd {
    Cmd::new(0x00, 0x85, 0x01, &[])
}

/// The extended (HyperPolling) read {0x00, 0xc0, 0x02, [0x00]}: answer `[1]` = a divisor of 8000.
pub fn read_polling_extended() -> Cmd {
    Cmd::new(0x00, 0xc0, 0x02, &[0x00])
}

/// Viper V4 Pro readDpi: the stage table {0x04, 0x86, 80}.
pub fn v4_read_dpi_stages() -> Cmd {
    Cmd::new(0x04, 0x86, 0x50, &[])
}

/// Viper V4 Pro polling read {0x00, 0xc0, 2, [1]}: answer `[1]` = a RAZER_V4_POLLING_CODES code.
pub fn v4_read_polling() -> Cmd {
    Cmd::new(0x00, 0xc0, 0x02, &[0x01])
}

/// Every read request this file sends - the allow-list compares against exactly these.
fn read_requests() -> [Cmd; 9] {
    [read_firmware(), read_battery(), read_charging(), read_dpi(STORE), read_dpi(NOSTORE), read_polling_legacy(), read_polling_extended(), v4_read_polling(), v4_read_dpi_stages()]
}

// ---------------------------------------------------------------- the writes (RAZER_WRITE) - only from a click

/// razerSetDpiCommand(x, y, store) {0x04, 0x05, 0x07, [store, xHi, xLo, yHi, yLo, 0, 0]}.
pub fn set_dpi(x: u16, y: u16, store: u8) -> Cmd {
    let ([xh, xl], [yh, yl]) = (x.to_be_bytes(), y.to_be_bytes());
    Cmd::new(0x04, 0x05, 0x07, &[store, xh, xl, yh, yl, 0, 0])
}

/// pollingDivisor: `ceiling / hz` when it is a whole number 1..=255, else the rate can't be encoded.
fn divisor(ceiling: u32, hz: u32) -> Option<u8> {
    (hz > 0 && ceiling.is_multiple_of(hz) && (1..=255).contains(&(ceiling / hz))).then(|| (ceiling / hz) as u8)
}

/// razerSetLegacyPollingCommand {0x00, 0x05, 0x01, [1000 / hz]}.
pub fn set_polling_legacy(hz: u32) -> Option<Cmd> {
    Some(Cmd::new(0x00, 0x05, 0x01, &[divisor(1000, hz)?]))
}

/// razerSetExtendedPollingCommand {0x00, 0x40, 0x02, [0x00, 8000 / hz]}.
pub fn set_polling_extended(hz: u32) -> Option<Cmd> {
    Some(Cmd::new(0x00, 0x40, 0x02, &[0x00, divisor(8000, hz)?]))
}

/// Viper V4 Pro setPollingRate {0x00, 0x40, 2, [1, code]}.
pub fn v4_set_polling(hz: u32) -> Option<Cmd> {
    Some(Cmd::new(0x00, 0x40, 0x02, &[0x01, v4_polling_code(hz)?]))
}

/// Viper V4 Pro writeDpi {0x04, 0x06, 3 + 7n}: `[1, active (1-based), n, per stage: index, xHi, xLo, yHi, yLo, 0, 0]`.
/// `active` is zero-based here.
pub fn v4_set_dpi_stages(active: usize, stages: &[(u16, u16)]) -> Cmd {
    let mut a = vec![0x01, active as u8 + 1, stages.len() as u8];
    for (i, (x, y)) in stages.iter().enumerate() {
        let ([xh, xl], [yh, yl]) = (x.to_be_bytes(), y.to_be_bytes());
        a.extend_from_slice(&[i as u8, xh, xl, yh, yl, 0, 0]);
    }
    let n = a.len() as u8;
    Cmd { class: 0x04, id: 0x06, size: n, args: a }
}

/// RAZER_V4_POLLING_CODES.
const V4_POLLING: [(u32, u8); 6] = [(125, 0x40), (500, 0x10), (1000, 0x08), (2000, 0x04), (4000, 0x02), (8000, 0x01)];

pub fn v4_polling_code(hz: u32) -> Option<u8> {
    V4_POLLING.iter().find(|p| p.0 == hz).map(|p| p.1)
}

pub fn v4_polling_hz(code: u8) -> Option<u32> {
    V4_POLLING.iter().find(|p| p.1 == code).map(|p| p.0)
}

// ---------------------------------------------------------------- packet + answers

/// razerChecksum: XOR of packet bytes 2..87.
pub fn checksum(p: &[u8]) -> u8 {
    p[2..88].iter().fold(0, |c, b| c ^ b)
}

/// encodeRazerRequest: the 90-byte packet (no report id).
pub fn packet(c: &Cmd, tid: u8) -> [u8; PACKET_LEN] {
    let mut p = [0u8; PACKET_LEN];
    p[1] = tid;
    p[5] = c.size;
    p[6] = c.class;
    p[7] = c.id;
    let n = c.args.len().min(80);
    p[8..8 + n].copy_from_slice(&c.args[..n]);
    p[88] = checksum(&p);
    p
}

/// The feature report Windows sends: report id 0, the packet, zero-padded to the collection's `len` (at least 91).
pub fn report(c: &Cmd, tid: u8, len: usize) -> Vec<u8> {
    let mut v = vec![REPORT_ID];
    v.extend_from_slice(&packet(c, tid));
    v.resize(len.max(REPORT_LEN), 0);
    v
}

/// Why an answer is not the one asked for (decodeRazerResponse's outcomes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// status 0x01: the same request answers a little later
    Busy,
    /// wrong length or bad crc: the exchange itself was lost (no status the mouse meant)
    Corrupt,
    /// the answer to another command (an earlier exchange's, still buffered)
    Stale,
    /// the mouse's own refusal: 0x03 failure, 0x04 timeout, 0x05 unsupported, …
    Status(u8),
}

/// decodeRazerResponse: a 90-byte answer packet (no report id) → its arguments (`[8..8 + data size]`).
pub fn decode_reply(p: &[u8], c: &Cmd) -> std::result::Result<Vec<u8>, Reply> {
    if p.len() != PACKET_LEN || p[88] != checksum(p) {
        return Err(Reply::Corrupt);
    }
    match p[0] {
        STATUS_OK => {}
        STATUS_BUSY => return Err(Reply::Busy),
        s => return Err(Reply::Status(s)),
    }
    if p[6] != c.class || p[7] != c.id {
        return Err(Reply::Stale);
    }
    let n = (p[5] as usize).min(PACKET_LEN - 8);
    Ok(p[8..8 + n].to_vec())
}

/// decodeDpi: (x, y).
pub fn decode_dpi(a: &[u8]) -> Option<(u32, u32)> {
    (a.len() >= 5).then(|| (u16::from_be_bytes([a[1], a[2]]) as u32, u16::from_be_bytes([a[3], a[4]]) as u32))
}

/// Math.round(n / d) for whole numbers.
fn round_div(n: u32, d: u32) -> u32 {
    (2 * n + d) / (2 * d)
}

/// decodeBatteryPercent: Math.round(args[1] × 100 / 255).
pub fn battery_percent(a: &[u8]) -> Option<u8> {
    a.get(1).map(|v| round_div(*v as u32 * 100, 255) as u8)
}

/// decodeCharging: args[1] === 1.
pub fn charging(a: &[u8]) -> Option<bool> {
    a.get(1).map(|v| *v == 1)
}

/// decodeLegacyPollingRate: Math.round(1000 / args[0]); 0 = unknown.
pub fn legacy_polling_hz(a: &[u8]) -> Option<u32> {
    a.first().filter(|d| **d != 0).map(|d| round_div(1000, *d as u32))
}

/// decodeExtendedPollingRate: Math.round(8000 / args[1]) (args[0] echoes the request); 0 = unknown.
pub fn extended_polling_hz(a: &[u8]) -> Option<u32> {
    a.get(1).filter(|d| **d != 0).map(|d| round_div(8000, *d as u32))
}

/// decodeRazerV4DpiState: (active stage, zero-based, clamped; the stages (x, y)) - `None` when it reports no stages.
pub fn v4_dpi_state(a: &[u8]) -> Option<(usize, Vec<(u16, u16)>)> {
    let at = |i: usize| a.get(i).copied().unwrap_or(0);
    let count = (at(2) as usize).min(5);
    let stages: Vec<(u16, u16)> = (0..count).map(|i| 3 + i * 7).map(|o| (u16::from_be_bytes([at(o + 1), at(o + 2)]), u16::from_be_bytes([at(o + 3), at(o + 4)]))).collect();
    if stages.is_empty() {
        return None;
    }
    let active = (at(1).max(1) as usize - 1).min(stages.len() - 1);
    Some((active, stages))
}

// ---------------------------------------------------------------- the device table

/// Which OpenMouse client drives the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Client {
    /// RazerHidClient (hid.js) - every RAZER_PRODUCTS row
    Generic,
    /// RazerCobraHidClient
    Cobra,
    /// RazerViperMiniHidClient
    ViperMini,
    /// RazerViperV4ProHidClient (v4.js)
    ViperV4Pro,
}

/// One product id, as OpenMouse's tables describe it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Product {
    pub pid: u16,
    pub model: &'static str,
    pub client: Client,
    pub wireless: bool,
    pub tid: u8,
    pub max_dpi: u32,
    /// the polling rates that can be set, ascending
    pub polling: &'static [u32],
    /// highRatePolling: the extended command (divisor of 8000) instead of the legacy one (divisor of 1000)
    pub high_rate: bool,
    pub battery: bool,
    /// the store byte of the DPI read / the DPI write (the Cobra and Viper Mini read with 0x00 and write with 0x01)
    pub dpi_read_store: u8,
    pub dpi_write_store: u8,
    /// vendorControlInterface: a vendor-defined collection (usage page ≥ 0xFF00) may carry the control channel too
    pub vendor_control: bool,
}

/// devices.js RATES_1K / RATES_8K.
pub const RATES_1K: &[u32] = &[125, 500, 1000];
pub const RATES_8K: &[u32] = &[125, 500, 1000, 2000, 4000, 8000];

#[allow(clippy::too_many_arguments)] // one table row, field by field
const fn generic(pid: u16, model: &'static str, wireless: bool, tid: u8, max_dpi: u32, polling: &'static [u32], high_rate: bool, battery: bool, store: u8, vendor_control: bool) -> Product {
    Product { pid, model, client: Client::Generic, wireless, tid, max_dpi, polling, high_rate, battery, dpi_read_store: store, dpi_write_store: store, vendor_control }
}

/// COBRA / VIPER_MINI: wired, 0xff, 100-8500 DPI, RATES_WIRED with the legacy command, no battery; DPI read 0x00, write 0x01.
const fn wired_legacy(pid: u16, model: &'static str, client: Client) -> Product {
    Product { pid, model, client, wireless: false, tid: TID_FF, max_dpi: 8500, polling: RATES_1K, high_rate: false, battery: false, dpi_read_store: NOSTORE, dpi_write_store: STORE, vendor_control: false }
}

const fn v4(pid: u16, wireless: bool) -> Product {
    Product { pid, model: "Razer Viper V4 Pro", client: Client::ViperV4Pro, wireless, tid: TID_1F, max_dpi: 50000, polling: RATES_8K, high_rate: true, battery: true, dpi_read_store: NOSTORE, dpi_write_store: STORE, vendor_control: false }
}

/// Every Razer model on OpenMouse's SUPPORTED list (status "supported"), one row per product id, with the RAZER_PRODUCTS
/// row's facts (presets spread as devices.js spreads them). DeathAdder V3 (wired) has no pids on the list: 0x00b2 is its
/// devices.js row. Names follow the SUPPORTED list ("DeathAdder Essential" for all three ids). Every id is one model, so
/// no row has `model: None`.
pub const PRODUCTS: [Product; 23] = [
    // STANDARD + maxDpi 30000, RATES_8K, highRatePolling (TRANSACTION_1F)
    generic(0x00b2, "Razer DeathAdder V3", false, TID_1F, 30000, RATES_8K, true, false, STORE, true),
    // STANDARD + maxDpi 20000 (DPI_FOCUS), dpiStorageByte 0x00 (TRANSACTION_3F)
    generic(0x0084, "Razer DeathAdder V2", false, TID_3F, 20000, RATES_1K, false, false, NOSTORE, true),
    // VIPER_RECEIVER_WIRED / _WIRELESS (MODERN_WIRED / MODERN_RECEIVER: DPI_FOCUS, battery) (TRANSACTION_3F)
    generic(0x007c, "Razer DeathAdder V2 Pro", false, TID_3F, 20000, RATES_1K, false, true, STORE, false),
    generic(0x007d, "Razer DeathAdder V2 Pro", true, TID_3F, 20000, RATES_1K, true, true, STORE, false),
    wired_legacy(0x00a3, "Razer Cobra", Client::Cobra),
    // VIPER_V3_PRO (35000, battery); the cable RATES_1K legacy, the receiver RATES_8K extended
    generic(0x00c0, "Razer Viper V3 Pro", false, TID_1F, 35000, RATES_1K, false, true, STORE, false),
    generic(0x00c1, "Razer Viper V3 Pro", true, TID_1F, 35000, RATES_8K, true, true, STORE, false),
    // VIPER_V2_PRO (30000, RATES_1K, battery); the receiver uses the extended command
    generic(0x00a5, "Razer Viper V2 Pro", false, TID_1F, 30000, RATES_1K, false, true, STORE, false),
    generic(0x00a6, "Razer Viper V2 Pro", true, TID_1F, 30000, RATES_1K, true, true, STORE, false),
    // DEATHADDER_ESSENTIAL (6400, no battery, vendorControlInterface) (TRANSACTION_3F)
    generic(0x006e, "Razer DeathAdder Essential", false, TID_3F, 6400, RATES_1K, false, false, STORE, true),
    generic(0x0071, "Razer DeathAdder Essential", false, TID_3F, 6400, RATES_1K, false, false, STORE, true),
    generic(0x0098, "Razer DeathAdder Essential", false, TID_3F, 6400, RATES_1K, false, false, STORE, true),
    wired_legacy(0x008a, "Razer Viper Mini", Client::ViperMini),
    // VIPER_RECEIVER_WIRELESS + highRatePolling false, maxDpi 30000
    generic(0x00b8, "Razer Viper V3 HyperSpeed", true, TID_1F, 30000, RATES_1K, false, true, STORE, false),
    v4(0x00e5, false),
    v4(0x00e6, true),
    // MODERN_WIRED / MODERN_RECEIVER + highRatePolling false, maxDpi 30000
    generic(0x00b6, "Razer DeathAdder V3 Pro", false, TID_1F, 30000, RATES_1K, false, true, STORE, false),
    generic(0x00b7, "Razer DeathAdder V3 Pro", true, TID_1F, 30000, RATES_1K, false, true, STORE, false),
    // MODERN_WIRED + RATES_8K + highRatePolling / HYPERPOLLING_RECEIVER, maxDpi 45000 (DPI_DEATHADDER_V4_PRO)
    generic(0x00be, "Razer DeathAdder V4 Pro", false, TID_1F, 45000, RATES_8K, true, true, STORE, false),
    generic(0x00bf, "Razer DeathAdder V4 Pro", true, TID_1F, 45000, RATES_8K, true, true, STORE, false),
    generic(0x00ef, "Razer DeathAdder V4 Pro Carbon Fiber Edition", false, TID_1F, 45000, RATES_8K, true, true, STORE, false),
    generic(0x00f0, "Razer DeathAdder V4 Pro Carbon Fiber Edition", true, TID_1F, 45000, RATES_8K, true, true, STORE, false),
    // STANDARD (16000, no battery, vendorControlInterface); in neither tid list = the 0xff fallback
    generic(0x004c, "Razer Diamondback Chroma", false, TID_FF, 16000, RATES_1K, false, false, STORE, true),
];

pub fn product(pid: u16) -> Option<&'static Product> {
    PRODUCTS.iter().find(|p| p.pid == pid)
}

fn is_mouse(h: &HidInfo) -> bool {
    h.usage_page == 0x01 && h.usage == 0x02
}

fn is_vendor(h: &HidInfo) -> bool {
    h.usage_page >= 0xFF00
}

fn has_feature(h: &HidInfo) -> bool {
    h.feature_len as usize >= REPORT_LEN
}

/// The control collection, claimed the way each OpenMouse client's isSupported does (on Windows every top-level collection
/// is its own path):
/// - RazerHidClient: a Generic Desktop Mouse collection, or (vendorControlInterface models) a vendor-defined one;
/// - Cobra: either; Viper Mini: the Generic Desktop Mouse collection only;
/// - Viper V4 Pro: a Generic Desktop (0x01) or Consumer (0x0C) collection that carries the feature report.
///
/// Where several fit, the one with the 91-byte feature report wins (Windows shows which; WebHID's picker leaves it to the
/// user), the mouse collection before a vendor one.
pub fn control_interface<'a>(own: &[&'a HidInfo], p: &Product) -> Option<&'a HidInfo> {
    let fits = |h: &HidInfo| match p.client {
        Client::Generic => is_mouse(h) || (p.vendor_control && is_vendor(h)),
        Client::Cobra => is_mouse(h) || is_vendor(h),
        Client::ViperMini => is_mouse(h),
        Client::ViperV4Pro => (h.usage_page == 0x01 || h.usage_page == 0x0C) && has_feature(h),
    };
    own.iter().filter(|h| fits(h)).min_by_key(|h| (!has_feature(h), !is_mouse(h))).copied()
}

/// OpenMouse's Razer isSupported: Razer's vendor id, a product id of [`PRODUCTS`], and its control collection.
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID {
        return None;
    }
    let p = product(pid)?;
    let config = control_interface(own, p)?;
    Some(Matched { family: Family::Razer, model: Some(p.model), wireless: Some(p.wireless), config: config.clone(), variant: pid })
}

/// The allow-list for proofs on the real mouse (A_009_01): a well-formed feature report (report id 0, any zero padding)
/// carrying exactly one of the read requests this file sends ([`read_requests`]) on one of the three transaction ids -
/// status byte 0, crc right. Every write (and every other read) is refused.
pub fn read_request_allowed(out: &[u8]) -> bool {
    if out.len() < REPORT_LEN || out[0] != REPORT_ID || out[REPORT_LEN..].iter().any(|b| *b != 0) {
        return false;
    }
    let p = &out[1..REPORT_LEN];
    [TID_1F, TID_3F, TID_FF].contains(&p[1]) && read_requests().iter().any(|c| packet(c, p[1])[..] == p[..])
}

// ---------------------------------------------------------------- the exchange

/// How one client waits for its answers.
struct Timing {
    /// before the first fetch (RESPONSE_DELAY_MS 100; the V4 client fetches at once)
    first_ms: u32,
    /// before each fetch after a busy answer (100; the V4 client's 35)
    again_ms: u32,
    /// fetches per send (RESPONSE_ATTEMPTS 6; the V4 client 16)
    reads: u32,
    /// sends (RazerHidClient re-sends a lost exchange, up to 6 times; the model clients send once)
    sends: u32,
}

const GENERIC: Timing = Timing { first_ms: 100, again_ms: 100, reads: 6, sends: 6 };
const LEGACY: Timing = Timing { first_ms: 100, again_ms: 100, reads: 6, sends: 1 };
const V4: Timing = Timing { first_ms: 0, again_ms: 35, reads: 16, sends: 1 };

/// polling-rate change → read-back: POLLING_LINK_SETTLE_MS (the generic client on a wireless link; the V4 client always)
const POLLING_SETTLE_MS: u64 = 150;

fn timing(c: Client) -> &'static Timing {
    match c {
        Client::Generic => &GENERIC,
        Client::Cobra | Client::ViperMini => &LEGACY,
        Client::ViperV4Pro => &V4,
    }
}

fn refused(c: &Cmd, why: Reply) -> Error {
    let what = format!("Razer command {:#04x}/{:#04x}", c.class, c.id);
    match why {
        Reply::Status(STATUS_UNSUPPORTED) => Error::BadAnswer(format!("{what} is not supported by this mouse")),
        Reply::Status(s) => Error::BadAnswer(format!("{what} returned status {s:#04x}")),
        Reply::Corrupt => Error::BadAnswer(format!("{what}: a garbled answer")),
        Reply::Stale => Error::BadAnswer(format!("{what} was answered by a different command")),
        Reply::Busy => Error::BadAnswer(format!("{what}: the mouse stayed busy - it may be asleep or out of range")),
    }
}

/// One request → its answer's arguments, the way the product's OpenMouse client exchanges it: send (HidD_SetFeature),
/// fetch after the pause; busy → fetch again (no re-send). RazerHidClient also re-sends after a garbled answer, or after
/// another command's answer to a read, or when every fetch was busy; the model clients give up instead.
fn exchange(port: &mut dyn Port, p: &Product, c: &Cmd) -> Result<Vec<u8>> {
    let t = timing(p.client);
    let len = (port.info().feature_len as usize).max(REPORT_LEN);
    let out = report(c, p.tid, len);
    let resend = t.sends > 1;
    let mut last = Reply::Busy;
    for _ in 0..t.sends {
        for i in 0..t.reads {
            let how = if i == 0 { HidTransfer::Feature { reply_id: REPORT_ID, reply_len: len, wait_ms: t.first_ms } } else { HidTransfer::FeatureIn { reply_id: REPORT_ID, reply_len: len, wait_ms: t.again_ms } };
            let a = port.exchange(&out, &how)?;
            // the answer's packet behind the report id (a longer collection's padding cut off)
            let pk = a.get(1..a.len().min(REPORT_LEN)).unwrap_or(&[]);
            match decode_reply(pk, c) {
                Ok(args) => return Ok(args),
                Err(Reply::Busy) => last = Reply::Busy,
                Err(r @ Reply::Corrupt) if resend => {
                    last = r;
                    break;
                }
                Err(r @ Reply::Stale) if resend && c.is_getter() => {
                    last = r;
                    break;
                }
                Err(r) => return Err(refused(c, r)),
            }
        }
    }
    Err(refused(c, last))
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

/// An answer's arguments → Hz.
type Decode = fn(&[u8]) -> Option<u32>;

/// readPollingRateHz: the product's own command first, the other as the fallback; the first that answers decides.
fn read_polling(port: &mut dyn Port, p: &Product) -> Result<Option<u32>> {
    let legacy: (Cmd, Decode) = (read_polling_legacy(), legacy_polling_hz);
    let extended: (Cmd, Decode) = (read_polling_extended(), extended_polling_hz);
    let order = if p.high_rate { [extended, legacy] } else { [legacy, extended] };
    for (c, decode) in order {
        if let Some(a) = optional(exchange(port, p, &c))? {
            return Ok(decode(&a));
        }
    }
    Ok(None)
}

fn caps(p: &Product) -> Caps {
    // RazerHidClient / Cobra / Viper Mini: every whole DPI from 100 (DPI_MIN); the V4 client: 100-50000 in 50s
    let step = if p.client == Client::ViperV4Pro { 50 } else { 1 };
    Caps { dpi: Some(DpiCaps::uniform(100, p.max_dpi, step)), polling: p.polling.to_vec(), lift_off: Vec::new(), battery: p.battery }
}

fn the_product(m: &Matched) -> Result<&'static Product> {
    product(m.variant).ok_or_else(|| Error::UnsupportedMouse(format!("Razer product id {:#06x}", m.variant)))
}

fn check_dpi(p: &Product, dpi: u32) -> Result<()> {
    if caps(p).dpi.is_some_and(|d| d.allows(dpi)) {
        Ok(())
    } else {
        Err(Error::range("DPI", format!("{dpi} DPI is not one this Razer mouse can store (100 - {})", p.max_dpi)))
    }
}

// ---------------------------------------------------------------- the driver

/// The Razer driver: RazerHidClient, or the model's own OpenMouse client (by the table's [`Client`]).
pub struct Razer;

impl Razer {
    /// RazerHidClient.readStatus (and the Cobra / Viper Mini clients'): firmware, battery + charging (models with a cell),
    /// DPI, polling.
    fn read_classic(&self, p: &Product, port: &mut dyn Port, r: &mut OnMouse) -> Result<()> {
        // no answer to the firmware read = the mouse is not there (OpenMouse: "did not report a firmware version")
        if optional(exchange(port, p, &read_firmware()))?.is_none() {
            return Ok(());
        }
        r.online = true;
        if p.battery {
            // the level may answer without the charging state; a battery read that fails drops only the battery
            if let Some(a) = optional(exchange(port, p, &read_battery()))? {
                r.battery_percent = battery_percent(&a);
                r.charging = optional(exchange(port, p, &read_charging()))?.and_then(|a| charging(&a));
            }
        }
        r.dpi = decode_dpi(&exchange(port, p, &read_dpi(p.dpi_read_store))?).map(|d| d.0);
        r.polling_hz = match p.client {
            Client::Generic => read_polling(port, p)?,
            _ => legacy_polling_hz(&exchange(port, p, &read_polling_legacy())?),
        };
        Ok(())
    }

    /// RazerViperV4ProHidClient.readStatus: the DPI stage table, polling, battery, charging.
    fn read_v4(&self, p: &Product, port: &mut dyn Port, r: &mut OnMouse) -> Result<()> {
        let Some(a) = optional(exchange(port, p, &v4_read_dpi_stages()))? else {
            return Ok(());
        };
        r.online = true;
        r.dpi = v4_dpi_state(&a).map(|(act, s)| s[act].0 as u32);
        r.polling_hz = exchange(port, p, &v4_read_polling())?.get(1).copied().and_then(v4_polling_hz);
        r.battery_percent = battery_percent(&exchange(port, p, &read_battery())?);
        r.charging = charging(&exchange(port, p, &read_charging())?);
        Ok(())
    }

    fn polling_classic(&self, p: &Product, port: &mut dyn Port) -> Result<Option<u32>> {
        match p.client {
            Client::Generic => read_polling(port, p),
            _ => Ok(legacy_polling_hz(&exchange(port, p, &read_polling_legacy())?)),
        }
    }

    fn v4_dpi(&self, p: &Product, port: &mut dyn Port) -> Result<(usize, Vec<(u16, u16)>)> {
        v4_dpi_state(&exchange(port, p, &v4_read_dpi_stages())?).ok_or_else(|| Error::BadAnswer("the Viper V4 Pro reported no DPI stages".into()))
    }

    fn v4_polling(&self, p: &Product, port: &mut dyn Port) -> Result<Option<u32>> {
        Ok(exchange(port, p, &v4_read_polling())?.get(1).copied().and_then(v4_polling_hz))
    }
}

impl crate::family::Driver for Razer {
    /// READ requests only: the reads OpenMouse's readStatus sends for DPI, polling and battery (plus its first, the firmware
    /// version, as "is it answering"). Every one passes [`read_request_allowed`].
    fn read(&self, m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let p = the_product(m)?;
        let mut r = OnMouse { wireless: Some(p.wireless), caps: caps(p), ..Default::default() };
        match p.client {
            Client::ViperV4Pro => self.read_v4(p, port, &mut r)?,
            _ => self.read_classic(p, port, &mut r)?,
        }
        Ok(r)
    }

    /// setDpi / setPollingRate the way the model's OpenMouse client does them: the write, then the read-back it confirms
    /// with. Before the write, one read request for undo ("before"); a failed one stops the change.
    fn write(&self, m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        let p = the_product(m)?;
        match (s, p.client) {
            (Setting::Dpi(dpi), Client::ViperV4Pro) => {
                check_dpi(p, dpi)?;
                // readDpi, the active stage replaced (both axes), writeDpi, readDpi
                let (act, mut stages) = self.v4_dpi(p, port)?;
                let before = stages[act].0 as u32;
                stages[act] = (dpi as u16, dpi as u16);
                exchange(port, p, &v4_set_dpi_stages(act, &stages))?;
                let (act2, now) = self.v4_dpi(p, port)?;
                if now[act2] != (dpi as u16, dpi as u16) {
                    return Err(Error::BadAnswer(format!("the Viper V4 Pro kept {} DPI instead of {dpi}", now[act2].0)));
                }
                Ok(Changed { now: dpi, before: Some(before) })
            }
            (Setting::Dpi(dpi), _) => {
                check_dpi(p, dpi)?;
                let before = decode_dpi(&exchange(port, p, &read_dpi(p.dpi_read_store))?).map(|d| d.0);
                exchange(port, p, &set_dpi(dpi as u16, dpi as u16, p.dpi_write_store))?;
                let now = decode_dpi(&exchange(port, p, &read_dpi(p.dpi_read_store))?);
                if now != Some((dpi, dpi)) {
                    return Err(Error::BadAnswer(format!("the mouse kept {now:?} DPI instead of {dpi}")));
                }
                Ok(Changed { now: dpi, before })
            }
            (Setting::Polling(hz), Client::ViperV4Pro) => {
                let set = v4_set_polling(hz).ok_or_else(|| Error::range("polling rate", format!("{hz} Hz is not a Viper V4 Pro polling rate")))?;
                let before = self.v4_polling(p, port)?;
                exchange(port, p, &set)?;
                port.pause_ms(POLLING_SETTLE_MS);
                let now = self.v4_polling(p, port)?;
                if now != Some(hz) {
                    return Err(Error::BadAnswer(format!("the Viper V4 Pro kept {now:?} Hz instead of {hz} Hz")));
                }
                Ok(Changed { now: hz, before })
            }
            (Setting::Polling(hz), _) => {
                // getSupportedPollingRates, then the product's command (the model clients: always the legacy one)
                let set = if p.polling.contains(&hz) { if p.high_rate && p.client == Client::Generic { set_polling_extended(hz) } else { set_polling_legacy(hz) } } else { None };
                let set = set.ok_or_else(|| Error::range("polling rate", format!("this mouse does not support {hz} Hz on this connection")))?;
                let before = self.polling_classic(p, port)?;
                exchange(port, p, &set)?;
                if p.wireless {
                    port.pause_ms(POLLING_SETTLE_MS);
                }
                let now = self.polling_classic(p, port)?;
                if now != Some(hz) {
                    return Err(Error::BadAnswer(format!("the mouse kept {now:?} Hz instead of {hz} Hz")));
                }
                Ok(Changed { now: hz, before })
            }
            (Setting::LiftOff(_), _) => {
                Err(Error::range("lift-off distance", "this app can't set a Razer mouse's lift-off (Razer's levels have no millimetre figure)".to_string()))
            }
        }
    }
}

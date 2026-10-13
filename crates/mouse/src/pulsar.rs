//! Pulsar - two protocols, both ported from OpenMouse (`crate::family`: AGPL-3.0-or-later, `drivers/pulsar/pulsar-hid.js`,
//! `drivers/pulsar/pulsar-xs1-hid.js`, `pulsar/index.js`): DPI / polling / lift-off / battery (Order 104), and since
//! Order 110 (the owner, A_110_02) the five settings OpenMouse's Pulsar clients also write: debounce time, Motion Sync, angle
//! snapping, ripple control and (cMouse only) turbo = PULSAR_FLASH.performanceState; plus the 0.7 mm lift-off. Nothing else
//! (no DPI stages, no light, no sleep): the mouse's own DPI stages are never written.
//!
//! **cMouse** (report 8, 17 bytes incl. the id): the X2 CrazyLight and every Pulsar-vendor (0x3710) mouse with the report-8
//! config collection - OpenMouse's PulsarHidClient claims them all by vendor + collection, which is how the newer
//! Pulsars (Tenz Signature, X2h, X2 V3 ES Mini, X2F: on OpenMouse's SUPPORTED list with no product ids) are found.
//! Frame: `[0]` report id 0x08 · `[1]` command (echoed) · `[2]` status in answers (0 = accepted) · `[3..5]` settings-memory
//! address, big-endian · `[5]` length (≤ 10) · `[6..16]` data · `[16]` checksum = 0x55 − sum(bytes 0..16) (mod 256).
//! (OpenMouse counts without the report id: its index i is our i + 1.)
//!
//! **XS-1** (X3 Medium): 64-byte feature reports, report id 0 (unnumbered), a 16-bit little-endian checksum of bytes 0..61 in
//! bytes 62..63. OpenMouse reads its polling rate but never sets it ("pollingReadOnly").
//!
//! The protocol facts were first written from MIT sources (pulsar-mouse-linux, PulsarBattery, Bibimbap,
//! pulsar-battery-notifier - Order 009); since Order 104 every request is the byte sequence OpenMouse sends.

use crate::error::{Error, Result};
use crate::family::{check_extra, Caps, Change, Changed, DpiCaps, Extra, ExtraRow, Family, Kind, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x3710;
pub const REPORT_ID: u8 = 0x08;
pub const FRAME_LEN: usize = 17;

/// Identify ("encryptionData"): 4 random bytes + 4 zeros; answer `[10]` family (CID, 0x57), `[11]` model (MID), `[12]`
/// connection type.
pub const CMD_IDENTIFY: u8 = 0x01;
/// Host control ("deviceOnline"), length 0, `[6]` = 1 enter / 0 leave. Answer: status 0, `[6]` = 1 when the mouse is
/// online behind the dongle, `[10]` = busy (ask again). OpenMouse enters it around every write; the leave form (`[6]` = 0)
/// is also the read-only "is it online?" question.
pub const CMD_ONLINE: u8 = 0x03;
/// Battery: answer `[6]` percent, `[7]` charging, `[8..10]` millivolts big-endian.
pub const CMD_BATTERY: u8 = 0x04;
/// Write settings memory ("writeFlashData") - only from a click; NEVER in tests or proofs on a real mouse.
pub const CMD_WRITE: u8 = 0x07;
/// Read settings memory ("readFlashData").
pub const CMD_READ: u8 = 0x08;
/// Factory reset - never sent by this app (OpenMouse does not send it either); refused by the allow-list.
pub const CMD_RESET: u8 = 0x09;
/// Get active profile ("getCurrentConfig").
pub const CMD_GET_PROFILE: u8 = 0x0E;
/// Set active profile - never sent; refused by the allow-list.
pub const CMD_SET_PROFILE: u8 = 0x0F;
/// Mouse firmware version ("readVersionId").
pub const CMD_VERSION: u8 = 0x12;
/// Dongle firmware version ("getDongleVersion").
pub const CMD_DONGLE_VERSION: u8 = 0x1D;

/// The family code (CID) every cMouse answers in the identify reply.
pub const FAMILY_CMOUSE: u8 = 0x57;

/// Settings-memory addresses (OpenMouse's PULSAR_FLASH; they always show the ACTIVE onboard profile).
pub const ADDR_POLLING: u16 = 0x0000;
pub const ADDR_STAGE_COUNT: u16 = 0x0002;
pub const ADDR_ACTIVE_STAGE: u16 = 0x0004;
pub const ADDR_LIFT_OFF: u16 = 0x000A;
pub const ADDR_DPI_STAGE0: u16 = 0x000C;
/// Order 110 (PULSAR_FLASH): debounceTime, motionSync, angleSnapping, rippleControl, performanceState - each a pair.
pub const ADDR_DEBOUNCE: u16 = 169;
pub const ADDR_MOTION_SYNC: u16 = 171;
pub const ADDR_ANGLE_SNAP: u16 = 175;
pub const ADDR_RIPPLE: u16 = 177;
pub const ADDR_PERFORMANCE: u16 = 181;
/// The row of settings memory those five sit in (169 .. 183).
pub const EXTRAS_FROM: u16 = ADDR_DEBOUNCE;
pub const EXTRAS_LEN: u16 = ADDR_PERFORMANCE + 2 - ADDR_DEBOUNCE;
/// Debounce 0-15 ms (OpenMouse's getDebounceOptions / setDebounceTime).
pub const DEBOUNCE_MAX_MS: i32 = 15;

pub fn checksum(b: &[u8]) -> u8 {
    0x55u8.wrapping_sub(b.iter().fold(0u8, |a, x| a.wrapping_add(*x)))
}

/// Builds a 17-byte request (length byte = the data's length).
pub fn frame(cmd: u8, addr: u16, data: &[u8]) -> [u8; FRAME_LEN] {
    let mut f = [0u8; FRAME_LEN];
    f[0] = REPORT_ID;
    f[1] = cmd;
    f[3] = (addr >> 8) as u8;
    f[4] = addr as u8;
    let n = data.len().min(10);
    f[5] = n as u8;
    f[6..6 + n].copy_from_slice(&data[..n]);
    f[16] = checksum(&f[..16]);
    f
}

/// A read-settings-memory request (`len` bytes, ≤ 10).
pub fn read_frame(addr: u16, len: u8) -> [u8; FRAME_LEN] {
    let mut f = frame(CMD_READ, addr, &[]);
    f[5] = len.min(10);
    f[16] = checksum(&f[..16]);
    f
}

/// Host control as OpenMouse sends it: length 0, `[6]` = 1 (enter, before writes) or 0 (leave).
pub fn online_frame(enter: bool) -> [u8; FRAME_LEN] {
    let mut f = frame(CMD_ONLINE, 0, &[]);
    f[6] = enter as u8;
    f[16] = checksum(&f[..16]);
    f
}

/// "Is the mouse online?" - the leave form of [`online_frame`] (changes nothing).
pub fn online_query_frame() -> [u8; FRAME_LEN] {
    online_frame(false)
}

/// The identify request with a 4-byte challenge (an all-zero frame is rejected by the mouse).
pub fn identify_frame(challenge: [u8; 4]) -> [u8; FRAME_LEN] {
    frame(CMD_IDENTIFY, 0, &[challenge[0], challenge[1], challenge[2], challenge[3], 0, 0, 0, 0])
}

/// A one-byte setting written as `[value, 0x55 − value]` (OpenMouse's writeCheckedByte).
pub fn write_byte_frame(addr: u16, v: u8) -> [u8; FRAME_LEN] {
    frame(CMD_WRITE, addr, &scalar_pair(v))
}

/// The allow-list for proofs on the real mouse (A_009_01): only READ requests may leave — identify, online with the
/// byte 0 (leave / question), battery, read settings memory, get profile, versions. Never write (0x07), reset (0x09),
/// set profile (0x0F), or host control entered. The frame must be well-formed (report id, checksum).
pub fn read_request_allowed(out: &[u8]) -> bool {
    if out.len() < FRAME_LEN || out[0] != REPORT_ID || checksum(&out[..16]) != out[16] {
        return false;
    }
    match out[1] {
        CMD_IDENTIFY | CMD_BATTERY | CMD_READ | CMD_GET_PROFILE | CMD_VERSION | CMD_DONGLE_VERSION => true,
        CMD_ONLINE => out[6] == 0,
        _ => false,
    }
}

/// Checks an answer: report id, echoed command, checksum over the first 17 bytes (longer dongle reports are cut).
pub fn check_answer(cmd: u8, a: &[u8]) -> std::result::Result<(), String> {
    if a.len() < FRAME_LEN {
        return Err(format!("answer too short ({} bytes)", a.len()));
    }
    if a[0] != REPORT_ID || a[1] != cmd {
        return Err(format!("answer is for command {:#04x}, not {cmd:#04x}", a[1]));
    }
    if checksum(&a[..16]) != a[16] {
        return Err("answer checksum wrong".into());
    }
    Ok(())
}

/// A one-byte setting is stored as `[value, 0x55 − value]`.
pub fn scalar_pair(v: u8) -> [u8; 2] {
    [v, 0x55u8.wrapping_sub(v)]
}

pub fn scalar_from_pair(b: &[u8]) -> Option<u8> {
    (b.len() >= 2 && b[1] == 0x55u8.wrapping_sub(b[0])).then(|| b[0])
}

/// The polling rates a cMouse can be set to (OpenMouse's PULSAR_POLLING_RATES).
pub const POLLING_RATES: [u32; 7] = [125, 250, 500, 1000, 2000, 4000, 8000];

/// Polling rate Hz → code (OpenMouse's pulsarEncodePollingRate): ≤ 1000 Hz = 1000 / rate, above = rate / 2000 × 16.
pub fn polling_code(hz: u32) -> Option<u8> {
    POLLING_RATES.contains(&hz).then(|| if hz <= 1000 { 1000 / hz } else { hz / 2000 * 16 } as u8)
}

/// Code → Hz (pulsarDecodePollingRate): ≥ 16 = code / 16 × 2000, else 1000 / code.
pub fn polling_hz(code: u8) -> Option<u32> {
    let c = code as u32;
    let hz = if c >= 16 { c * 2000 / 16 } else { 1000u32.checked_div(c).unwrap_or(0) };
    POLLING_RATES.contains(&hz).then_some(hz)
}

/// Lift-off distance in tenths of a millimetre ↔ code (OpenMouse: Low = 3, Medium = 1, High = 2).
pub fn lift_off_code(tenths_mm: u32) -> Option<u8> {
    Some(match tenths_mm {
        10 => 0x01,
        20 => 0x02,
        7 => 0x03,
        _ => return None,
    })
}

pub fn lift_off_tenths(code: u8) -> Option<u32> {
    Some(match code {
        0x01 => 10,
        0x02 => 20,
        0x03 => 7,
        _ => return None,
    })
}

/// How the dongle / cable is linked (identify answer byte 12) and the highest polling rate it allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    Wireless1k,
    Wireless2k,
    Wireless4k,
    Wireless8k,
    Wired1k,
    Wired8k,
    Unknown(u8),
}

impl Link {
    pub fn from_code(c: u8) -> Link {
        match c {
            0 => Link::Wireless1k,
            1 => Link::Wireless4k,
            2 => Link::Wired1k,
            3 => Link::Wired8k,
            4 => Link::Wireless2k,
            5 => Link::Wireless8k,
            o => Link::Unknown(o),
        }
    }
    pub fn wireless(self) -> Option<bool> {
        match self {
            Link::Wireless1k | Link::Wireless2k | Link::Wireless4k | Link::Wireless8k => Some(true),
            Link::Wired1k | Link::Wired8k => Some(false),
            Link::Unknown(_) => None,
        }
    }
    /// OpenMouse's maximumPollingRateHz: {0: 1000, 1: 4000, 2: 1000, 3: 8000, 4: 2000, 5: 8000}, else 1000.
    pub fn max_polling_hz(self) -> u32 {
        match self {
            Link::Wireless1k | Link::Wired1k | Link::Unknown(_) => 1000,
            Link::Wireless2k => 2000,
            Link::Wireless4k => 4000,
            Link::Wireless8k | Link::Wired8k => 8000,
        }
    }
}

/// Can a cMouse store this DPI (OpenMouse's pulsarDpiOptions: 10-step to 10000, 50-step 10050–30000, 100-step
/// 30100–32000)?
pub fn dpi_storable(dpi: u32) -> bool {
    ((10..=10000).contains(&dpi) && dpi.is_multiple_of(10)) || ((10050..=30000).contains(&dpi) && dpi.is_multiple_of(50)) || ((30100..=32000).contains(&dpi) && dpi.is_multiple_of(100))
}

/// What the Mouse page offers for a cMouse: OpenMouse's steps inside the app's 50–26000 (Order 042: the X2-class sensors).
pub fn dpi_caps() -> DpiCaps {
    DpiCaps { bands: vec![(50, 10000, 10), (10050, 26000, 50)] }
}

/// DPI → the 4-byte stage record (OpenMouse's pulsarEncodeDpi): `[raw, raw, flags, 0x55 − sum]`,
/// flags = (high << 2) | (high << 6) | ex | (ex << 4) with ex 0x22 from 10050 and 0x33 from 30100.
pub fn encode_dpi_stage(dpi: u32) -> Option<[u8; 4]> {
    if !dpi_storable(dpi) {
        return None;
    }
    let (raw, ex) = if dpi >= 30100 {
        ((dpi / 2 - 10050) / 50, 0x33u32)
    } else if dpi >= 10050 {
        ((dpi - 10050) / 50, 0x22)
    } else {
        (dpi / 10 - 1, 0)
    };
    let high = raw >> 8;
    let lo = raw as u8;
    let flags = ((high << 2) | (high << 6) | ex | (ex << 4)) as u8;
    let mut r = [lo, lo, flags, 0];
    r[3] = checksum(&r[..3]);
    Some(r)
}

/// A stage record → DPI (OpenMouse's pulsarDecodeDpi): both low bytes equal, the four bytes summing to 0x55;
/// (raw + 1) × 10, then × 5 + 10000 when flag 0x02, then × 2 when flag 0x01.
pub fn decode_dpi_stage(b: &[u8]) -> Option<u32> {
    if b.len() < 4 || b[0] != b[1] || b[..4].iter().fold(0u8, |a, x| a.wrapping_add(*x)) != 0x55 {
        return None;
    }
    let f = b[2] as u32;
    let raw = b[0] as u32 + (((f & 0x0C) >> 2) << 8);
    let mut dpi = (raw + 1) * 10;
    if f & 0x02 != 0 {
        dpi = dpi * 5 + 10000;
    }
    if f & 0x01 != 0 {
        dpi *= 2;
    }
    Some(dpi)
}

/// Battery answer → (percent, charging, millivolts); a percent above 100 is no reading (OpenMouse).
pub fn decode_battery(a: &[u8]) -> Option<(Option<u8>, bool, u16)> {
    (a.len() >= 10).then(|| ((a[6] <= 100).then_some(a[6]), a[7] == 1, u16::from_be_bytes([a[8], a[9]])))
}

/// Model name from the identify answer's model code (MID), per PulsarBattery's catalogue (MIT) — only the codes the
/// research could confirm; anything else keeps the name found from the USB id.
pub fn model_name(family: u8, mid: u8) -> Option<&'static str> {
    if family != FAMILY_CMOUSE {
        return None;
    }
    Some(match mid {
        1..=6 | 9 | 10 => "Pulsar X2 CrazyLight",
        27 => "Pulsar X2 CrazyLight Medium",
        23 | 24 => "Pulsar X2H CrazyLight",
        17 | 18 => "Pulsar Xlite CrazyLight",
        19..=22 => "Pulsar X3 CrazyLight",
        _ => return None,
    })
}

// ============================================================================ cMouse: device table + driver

/// The cMouse ids this app has always named (X2 CrazyLight: 8K dongle, cable - the MIT sources; the owner's mouse is 3710:5406).
const CMOUSE_NAMED: [(u16, &str, bool); 2] = [(0x5406, "Pulsar X2 CrazyLight", true), (0x3414, "Pulsar X2 CrazyLight", false)];

/// The report-8 config collection: vendor usage page 0xFF02 with output AND input reports of ≥ 17 bytes (Windows:
/// interface 1, `col05`); else any 0xFFxx collection of interface 1 with ≥ 17-byte reports. (Windows does not list report
/// ids; OpenMouse's check is "one input + one output report, both id 8".)
pub fn cmouse_interface<'a>(own: &[&'a HidInfo]) -> Option<&'a HidInfo> {
    let fits = |h: &&&HidInfo| h.output_len as usize >= FRAME_LEN && h.input_len as usize >= FRAME_LEN;
    own.iter().filter(fits).find(|h| h.usage_page == 0xFF02).or_else(|| own.iter().filter(fits).find(|h| h.usage_page >= 0xFF00 && h.interface == Some(1))).copied()
}

/// OpenMouse's PulsarHidClient.isSupported: Pulsar's vendor id + the report-8 config collection (any product id).
pub fn matches_cmouse(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID {
        return None;
    }
    let named = CMOUSE_NAMED.iter().find(|n| n.0 == pid);
    // the measured 0xFF02 collection; the interface-1 fallback only for the ids this app has always driven
    let config = if named.is_some() { cmouse_interface(own)? } else { own.iter().find(|h| h.usage_page == 0xFF02 && h.output_len as usize >= FRAME_LEN && h.input_len as usize >= FRAME_LEN).copied()? };
    Some(Matched { family: Family::PulsarCmouse, model: named.map(|n| n.1), wireless: named.map(|n| n.2), config: config.clone(), variant: 0 })
}

/// The pause between two online polls. With the default tries a waking mouse gets ~3 s to link (PulsarBattery waits 3 s,
/// 20 ms apart, before every settings session).
pub const ONLINE_POLL_MS: u64 = 20;
/// The longest the online poll (and host control) waits, answers or not.
pub const ONLINE_WAIT: std::time::Duration = std::time::Duration::from_millis(3000);
/// Asks of one settings read over a wireless link that drops a frame now and then.
const TRIES: usize = 5;

fn challenge() -> [u8; 4] {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(1));
    let v = h.finish().to_le_bytes();
    let c = [v[0], v[1], v[2], v[3]];
    // an all-zero challenge is rejected by the mouse
    if c == [0; 4] {
        [1, 0, 0, 0]
    } else {
        c
    }
}

/// The cMouse driver.
pub struct Cmouse;

fn send(port: &mut dyn Port, req: [u8; FRAME_LEN]) -> Result<Vec<u8>> {
    // Windows wants exactly OutputReportByteLength bytes, zero-padded.
    let (out_len, in_len) = (port.info().output_len as usize, port.info().input_len as usize);
    let mut out = req.to_vec();
    out.resize(out_len.max(FRAME_LEN), 0);
    let how = HidTransfer::OutputThenInput { reply_len: in_len.max(FRAME_LEN), max_reads: 64, timeout_ms: 1200, echo: Some((1, req[1])) };
    let a = port.exchange(&out, &how)?;
    check_answer(req[1], &a).map_err(Error::BadAnswer)?;
    Ok(a)
}

fn read_once(port: &mut dyn Port, addr: u16, len: u8) -> Result<Vec<u8>> {
    let a = send(port, read_frame(addr, len))?;
    if a[2] != 0 || u16::from_be_bytes([a[3], a[4]]) != addr {
        return Err(Error::BadAnswer(format!("read {addr:#06x}: status {} / address {:#04x}{:02x}", a[2], a[3], a[4])));
    }
    Ok(a[6..6 + len.min(10) as usize].to_vec())
}

/// One settings read, asked again (up to [`TRIES`]) when a frame got lost or garbled on the way.
fn read_mem(port: &mut dyn Port, addr: u16, len: u8) -> Result<Vec<u8>> {
    let mut last = None;
    for _ in 0..TRIES {
        match read_once(port, addr, len) {
            Ok(v) => return Ok(v),
            Err(e @ Error::BadAnswer(_)) => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| Error::BadAnswer(format!("read {addr:#06x}"))))
}

/// Asks "is the mouse online?" (host control's leave form) until it is online and not busy, [`ONLINE_POLL_MS`] apart:
/// a mouse that just woke up takes a moment to link to its dongle.
fn online(port: &mut dyn Port, tries: u32) -> Result<bool> {
    // and at most ONLINE_WAIT in all: a dongle that does not answer at all must not hold the worker for minutes
    let t0 = std::time::Instant::now();
    for i in 0..tries.max(1) {
        if i > 0 {
            if t0.elapsed() >= ONLINE_WAIT {
                break;
            }
            port.pause_ms(ONLINE_POLL_MS);
        }
        let a = match send(port, online_query_frame()) {
            Ok(a) => a,
            // no answer in time = asleep / not linked
            Err(Error::BadAnswer(_)) => continue,
            Err(e) => return Err(e),
        };
        if a[2] == 0 && a[6] == 1 && a[10] == 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

/// OpenMouse's setDeviceOnline: up to 20 asks, 10 ms apart, while the receiver answers busy; entering needs the mouse
/// online.
fn host_control(port: &mut dyn Port, enter: bool) -> Result<()> {
    for i in 0..20 {
        if i > 0 {
            port.pause_ms(10);
        }
        let a = send(port, online_frame(enter))?;
        if a[2] != 0 {
            return Err(Error::BadAnswer(format!("the Pulsar receiver rejected host control (status {})", a[2])));
        }
        if a[10] == 1 {
            continue;
        }
        if enter && a[6] != 1 {
            return Err(Error::MouseGone("the mouse is asleep or not linked — move it and try again".into()));
        }
        return Ok(());
    }
    Err(Error::BadAnswer("the Pulsar receiver stayed busy".into()))
}

/// One write (OpenMouse's writeFlash: one ≤ 10-byte chunk), accepted = status 0.
fn write_mem(port: &mut dyn Port, addr: u16, data: &[u8]) -> Result<()> {
    let a = send(port, frame(CMD_WRITE, addr, data))?;
    if a[2] != 0 {
        return Err(Error::BadAnswer(format!("the mouse rejected the write at {addr:#06x} (status {})", a[2])));
    }
    Ok(())
}

/// OpenMouse's withDeviceControl: enter host control, the operation, leave (also after a failure).
fn session<T>(port: &mut dyn Port, f: impl FnOnce(&mut dyn Port) -> Result<T>) -> Result<T> {
    if let Err(e) = host_control(port, true) {
        // the enter may have arrived with only its answer lost: never leave it held (the leave is the read-only question)
        let _ = host_control(port, false);
        return Err(e);
    }
    let r = f(port);
    let _ = host_control(port, false);
    r
}

impl Cmouse {
    fn identify(&self, port: &mut dyn Port) -> Result<(u8, u8, Link)> {
        let id = send(port, identify_frame(challenge()))?;
        if id[2] != 0 {
            return Err(Error::BadAnswer(format!("the Pulsar receiver rejected identification (status {})", id[2])));
        }
        let (family, mid, link) = (id[10], id[11], Link::from_code(id[12]));
        if family != FAMILY_CMOUSE {
            return Err(Error::UnsupportedMouse(format!("family code {family:#04x}, not 0x57")));
        }
        Ok((family, mid, link))
    }

    fn caps(link: Link) -> Caps {
        Caps { dpi: Some(dpi_caps()), polling: POLLING_RATES.iter().copied().filter(|r| *r <= link.max_polling_hz()).collect(), lift_off: vec![7, 10, 20], battery: true }
    }

    fn read_settings(&self, port: &mut dyn Port, r: &mut OnMouse) -> Result<()> {
        r.polling_hz = scalar_from_pair(&read_mem(port, ADDR_POLLING, 2)?).and_then(polling_hz);
        if let Some(s) = scalar_from_pair(&read_mem(port, ADDR_ACTIVE_STAGE, 2)?) {
            r.dpi = decode_dpi_stage(&read_mem(port, ADDR_DPI_STAGE0 + 4 * s.min(7) as u16, 4)?);
        }
        r.lift_off = scalar_from_pair(&read_mem(port, ADDR_LIFT_OFF, 2)?).and_then(lift_off_tenths);
        // Order 110: the row OpenMouse's readStatus takes debounce / Motion Sync / angle snapping / ripple / performance
        // from (its readFlash reads it in 10-byte pieces; a value it cannot read stays unknown, never an error)
        let row = read_mem(port, EXTRAS_FROM, 10)
            .and_then(|mut a| {
                a.extend(read_mem(port, EXTRAS_FROM + 10, (EXTRAS_LEN - 10) as u8)?);
                Ok(a)
            })
            .ok();
        r.extras = extras_from_row(row.as_deref());
        Ok(())
    }

    fn stage_addr(port: &mut dyn Port) -> Result<u16> {
        let s = read_mem(port, ADDR_ACTIVE_STAGE, 2)?[0];
        Ok(ADDR_DPI_STAGE0 + 4 * s.min(7) as u16)
    }
}

/// Order 110: the five rows from the 14 bytes at 169 (OpenMouse's readStatus: `debounceMs: flash[169]`, `motionSync:
/// flash[171] === 1`, `angleSnapping: flash[175] === 1`, `rippleControl: flash[177] === 1`, `performanceMode: flash[181]
/// === 1`), in the card's order. `None` = not read: the rows show, their values unknown.
pub fn extras_from_row(row: Option<&[u8]>) -> Vec<ExtraRow> {
    let at = |a: u16| row.and_then(|r| r.get((a - EXTRAS_FROM) as usize)).copied();
    let on = |a: u16| at(a).map(|v| v == 1);
    vec![
        ExtraRow::new(Extra::Debounce, Kind::Range { min: 0, max: DEBOUNCE_MAX_MS, step: 1, unit: "ms" }, at(ADDR_DEBOUNCE).map(i32::from)),
        ExtraRow::switch(Extra::MotionSync, on(ADDR_MOTION_SYNC)),
        ExtraRow::switch(Extra::AngleSnap, on(ADDR_ANGLE_SNAP)),
        ExtraRow::switch(Extra::RippleControl, on(ADDR_RIPPLE)),
        ExtraRow::switch(Extra::Turbo, on(ADDR_PERFORMANCE)),
    ]
}

/// Where an extra lives in settings memory.
fn extra_addr(e: Extra) -> u16 {
    match e {
        Extra::Debounce => ADDR_DEBOUNCE,
        Extra::MotionSync => ADDR_MOTION_SYNC,
        Extra::AngleSnap => ADDR_ANGLE_SNAP,
        Extra::RippleControl => ADDR_RIPPLE,
        Extra::Turbo => ADDR_PERFORMANCE,
    }
}

impl crate::family::Driver for Cmouse {
    /// Order 110: OpenMouse's setDebounceTime / setMotionSync / setAngleSnapping / setRippleControl / setPerformanceMode =
    /// setVerifiedByte: checked against what the mouse takes before a byte leaves, then host control, writeCheckedByte
    /// `[v, 0x55 − v]`, read it back (the first byte must be `v`), leave.
    fn change(&self, _m: &Matched, port: &mut dyn Port, c: Change) -> Result<()> {
        let Change::Extra(e, v) = c;
        check_extra(&extras_from_row(None), e, v)?;
        let addr = extra_addr(e);
        session(port, |p| {
            write_mem(p, addr, &scalar_pair(v as u8))?;
            let now = read_mem(p, addr, 2)?[0];
            if now as i32 != v {
                return Err(Error::BadAnswer(format!("the mouse kept {now} instead of {v} ({})", e.label())));
            }
            Ok(())
        })
    }

    /// identify, battery, then (if the mouse is awake) polling, active stage + its DPI, lift-off. Only READ requests:
    /// identify, battery, the online question (host control's leave form) and settings-memory reads.
    fn read(&self, _m: &Matched, port: &mut dyn Port, opt: ReadOptions) -> Result<OnMouse> {
        let (family, mid, link) = self.identify(port)?;
        let mut r = OnMouse { model: model_name(family, mid).map(str::to_string), wireless: link.wireless(), caps: Self::caps(link), ..Default::default() };
        // A sleeping mouse behind the dongle answers nothing (measured on the real X2 CrazyLight: identify answered by the
        // dongle, battery timed out) — then battery stays unknown and the mouse counts as offline.
        match send(port, frame(CMD_BATTERY, 0, &[])) {
            Ok(b) if b[2] == 0 => {
                if let Some((pct, ch, _mv)) = decode_battery(&b) {
                    r.battery_percent = pct;
                    r.charging = Some(ch);
                }
            }
            Ok(_) | Err(Error::BadAnswer(_)) => {}
            Err(e) => return Err(e),
        }
        r.online = online(port, opt.online_tries)?;
        if r.online {
            self.read_settings(port, &mut r)?;
        }
        Ok(r)
    }

    /// OpenMouse's setDpi / setPollingRate / setLiftOffDistance: host control, write, read back, leave.
    fn write(&self, _m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        match s {
            Setting::Dpi(dpi) => {
                let rec = encode_dpi_stage(dpi).ok_or_else(|| Error::range("DPI", format!("{dpi} can't be stored")))?;
                session(port, |p| {
                    let addr = Self::stage_addr(p)?;
                    let before = decode_dpi_stage(&read_mem(p, addr, 4)?);
                    write_mem(p, addr, &rec)?;
                    let now = decode_dpi_stage(&read_mem(p, addr, 4)?);
                    if now != Some(dpi) {
                        return Err(Error::BadAnswer(format!("the mouse kept {now:?} instead of {dpi} DPI")));
                    }
                    Ok(Changed { now: dpi, before })
                })
            }
            Setting::Polling(hz) => {
                let code = polling_code(hz).ok_or_else(|| Error::range("polling rate", format!("{hz} Hz is not a polling rate")))?;
                let (_, _, link) = self.identify(port)?;
                if hz > link.max_polling_hz() {
                    return Err(Error::range("polling rate", format!("{hz} Hz needs a faster receiver (this link allows {} Hz)", link.max_polling_hz())));
                }
                session(port, |p| {
                    let before = scalar_from_pair(&read_mem(p, ADDR_POLLING, 2)?).and_then(polling_hz);
                    write_mem(p, ADDR_POLLING, &scalar_pair(code))?;
                    let now = read_mem(p, ADDR_POLLING, 2)?.first().copied().and_then(polling_hz);
                    if now != Some(hz) {
                        return Err(Error::BadAnswer(format!("the mouse kept {now:?} Hz instead of {hz} Hz")));
                    }
                    Ok(Changed { now: hz, before })
                })
            }
            Setting::LiftOff(t) => {
                let code = lift_off_code(t).ok_or_else(|| Error::range("lift-off distance", format!("{t} tenths of a mm")))?;
                session(port, |p| {
                    let before = scalar_from_pair(&read_mem(p, ADDR_LIFT_OFF, 2)?).and_then(lift_off_tenths);
                    write_mem(p, ADDR_LIFT_OFF, &scalar_pair(code))?;
                    let now = read_mem(p, ADDR_LIFT_OFF, 2)?.first().copied().and_then(lift_off_tenths);
                    if now != Some(t) {
                        return Err(Error::BadAnswer(format!("the mouse kept {now:?} instead of {t} tenths of a mm")));
                    }
                    Ok(Changed { now: t, before })
                })
            }
        }
    }
}

// ============================================================================ XS-1 (X3 family)

pub const XS1_PACKET_LEN: usize = 64;
/// Windows' unnumbered feature report: report id 0, then the 64-byte packet.
pub const XS1_REPORT_LEN: usize = XS1_PACKET_LEN + 1;
/// OpenMouse waits 25 ms between sending a query and fetching its answer.
pub const XS1_WAIT_MS: u32 = 25;

/// XS-1 queries (OpenMouse's QUERY) - the only XS-1 requests the read-only proof lets through.
pub const XS1_Q_BATTERY: [u8; 3] = [0x08, 0x81, 0x01];
pub const XS1_Q_DPI: [u8; 3] = [0x05, 0x82, 0x05];
pub const XS1_Q_STAGE: [u8; 3] = [0x05, 0x81, 0x02];
pub const XS1_Q_LIFT_OFF: [u8; 3] = [0x07, 0x82, 0x03];
pub const XS1_Q_POLLING: [u8; 3] = [0x08, 0x85, 0x03];
pub const XS1_Q_VERSION: [u8; 3] = [0x01, 0x87, 0x04];
/// Order 110: OpenMouse QUERY.debounce / motionSync / angleSnapping / rippleControl.
pub const XS1_Q_DEBOUNCE: [u8; 3] = [0x04, 0x83, 0x03];
pub const XS1_Q_MOTION_SYNC: [u8; 3] = [0x07, 0x85, 0x02];
pub const XS1_Q_ANGLE_SNAP: [u8; 3] = [0x07, 0x84, 0x02];
pub const XS1_Q_RIPPLE: [u8; 3] = [0x07, 0x83, 0x02];
const XS1_QUERIES: [[u8; 3]; 10] = [XS1_Q_BATTERY, XS1_Q_DPI, XS1_Q_STAGE, XS1_Q_LIFT_OFF, XS1_Q_POLLING, XS1_Q_VERSION, XS1_Q_DEBOUNCE, XS1_Q_MOTION_SYNC, XS1_Q_ANGLE_SNAP, XS1_Q_RIPPLE];

/// A flag / debounce write (OpenMouse WRITE.debounce / motionSync / angleSnapping / rippleControl): the query with its 0x80
/// bit cleared, `00 00 01`, the value.
pub fn xs1_value_write(q: [u8; 3], v: u8) -> [u8; 7] {
    [q[0], q[1] & 0x7F, q[2], 0x00, 0x00, 0x01, v]
}

/// The XS-1's Order 110 rows (no turbo: OpenMouse's XS-1 client has none), in the card's order, with their queries.
const XS1_EXTRAS: [(Extra, [u8; 3]); 4] = [(Extra::Debounce, XS1_Q_DEBOUNCE), (Extra::MotionSync, XS1_Q_MOTION_SYNC), (Extra::AngleSnap, XS1_Q_ANGLE_SNAP), (Extra::RippleControl, XS1_Q_RIPPLE)];

fn xs1_kind(e: Extra) -> Kind {
    if e == Extra::Debounce {
        Kind::Range { min: 0, max: DEBOUNCE_MAX_MS, step: 1, unit: "ms" }
    } else {
        Kind::Switch
    }
}

/// XS-1 product ids OpenMouse knows (PULSAR_XS1_PRODUCT_IDS) whose model is on its SUPPORTED list: the X3 Medium
/// (0x3409, 0x3410, 0x5402) and the shared 8K receiver 0x5403 (its id names no model). 0x3404 (X2A v3) is not SUPPORTED.
pub const XS1_TABLE: [(u16, Option<&str>, bool); 4] = [(0x3409, Some("Pulsar X3 Medium"), false), (0x3410, Some("Pulsar X3 Medium"), false), (0x5402, Some("Pulsar X3 Medium"), true), (0x5403, None, true)];
const XS1_8K_RECEIVER: u16 = 0x5403;
pub const XS1_POLLING_RATES: [u32; 4] = [125, 250, 500, 1000];
pub const XS1_LIFT_OFF: [(u8, u32); 3] = [(0x07, 7), (0x0A, 10), (0x14, 20)];

/// The 16-bit checksum: sum of bytes 0..62 of the packet, little-endian in 62..64.
pub fn xs1_checksum(p: &[u8]) -> u16 {
    p[..XS1_PACKET_LEN - 2].iter().fold(0u16, |a, x| a.wrapping_add(*x as u16))
}

/// OpenMouse's pulsarXs1EncodeRequest: `[0x00, command…]`, checksum at 62..64.
pub fn xs1_packet(cmd: &[u8]) -> [u8; XS1_PACKET_LEN] {
    let mut p = [0u8; XS1_PACKET_LEN];
    let n = cmd.len().min(XS1_PACKET_LEN - 3);
    p[1..1 + n].copy_from_slice(&cmd[..n]);
    let c = xs1_checksum(&p);
    p[62] = c as u8;
    p[63] = (c >> 8) as u8;
    p
}

/// The DPI write (OpenMouse WRITE.dpi): both axes, little-endian.
pub fn xs1_dpi_write(dpi: u16) -> [u8; 10] {
    let [lo, hi] = dpi.to_le_bytes();
    [0x05, 0x02, 0x05, 0x00, 0x00, 0x01, lo, hi, lo, hi]
}

/// The lift-off write (OpenMouse WRITE.liftOffDistance).
pub fn xs1_lift_off_write(code: u8) -> [u8; 8] {
    [0x07, 0x02, 0x03, 0x00, 0x00, 0x01, 0x02, code]
}

/// Polling answer byte 7 → Hz (OpenMouse's query table: 240 = 125 … 4 = 8000).
pub fn xs1_polling_hz(v: u8) -> Option<u32> {
    Some(match v {
        240 => 125,
        120 => 250,
        60 => 500,
        30 => 1000,
        15 => 2000,
        8 => 4000,
        4 => 8000,
        _ => return None,
    })
}

pub fn xs1_lift_off_tenths(code: u8) -> Option<u32> {
    XS1_LIFT_OFF.iter().find(|l| l.0 == code).map(|l| l.1)
}

/// The XS-1 interface: vendor usage page 0xFFFF, usage 0x01, with a feature report.
fn xs1_interface<'a>(own: &[&'a HidInfo]) -> Option<&'a HidInfo> {
    own.iter().find(|h| h.usage_page == 0xFFFF && h.usage == 0x01 && h.feature_len as usize >= XS1_PACKET_LEN).copied()
}

/// OpenMouse's PulsarXs1HidClient.isSupported: an XS-1 product id, the 0xFFFF:0x01 feature collection, and NO report-8
/// control collection (that one makes it a cMouse).
pub fn matches_xs1(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    let (_, model, wireless) = *XS1_TABLE.iter().find(|t| vid == VENDOR_ID && t.0 == pid)?;
    if matches_cmouse(own, vid, pid).is_some() {
        return None;
    }
    Some(Matched { family: Family::PulsarXs1, model, wireless: Some(wireless), config: xs1_interface(own)?.clone(), variant: pid })
}

/// The XS-1 allow-list: a well-formed 65-byte feature report (id 0) carrying one of the [`XS1_QUERIES`].
pub fn xs1_read_request_allowed(out: &[u8]) -> bool {
    if out.len() != XS1_REPORT_LEN || out[0] != 0 {
        return false;
    }
    let p = &out[1..];
    let c = xs1_checksum(p);
    p[62] == c as u8 && p[63] == (c >> 8) as u8 && p[0] == 0 && XS1_QUERIES.iter().any(|q| p[1..4] == q[..]) && p[4..62].iter().all(|b| *b == 0)
}

/// The XS-1 driver.
pub struct Xs1;

fn xs1_report(cmd: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8];
    out.extend_from_slice(&xs1_packet(cmd));
    out
}

/// One query: set feature, 25 ms, get feature - asked up to 5 times (OpenMouse's RESPONSE_ATTEMPTS). The answer's packet
/// (without the report id).
fn xs1_query(port: &mut dyn Port, q: &[u8; 3]) -> Result<Vec<u8>> {
    let mut last = String::new();
    for i in 0..5 {
        if i > 0 {
            port.pause_ms(XS1_WAIT_MS as u64);
        }
        match port.exchange(&xs1_report(q), &HidTransfer::Feature { reply_id: 0, reply_len: XS1_REPORT_LEN, wait_ms: XS1_WAIT_MS }) {
            Ok(a) if a.len() == XS1_REPORT_LEN => return Ok(a[1..].to_vec()),
            Ok(a) => last = format!("answered {} bytes instead of {XS1_PACKET_LEN}", a.len().saturating_sub(1)),
            Err(e @ (Error::MouseGone(_) | Error::ReadOnly(_))) => return Err(e),
            Err(e) => last = e.to_string(),
        }
    }
    Err(Error::BadAnswer(format!("the Pulsar XS-1 mouse did not answer {q:02x?}: {last}")))
}

fn xs1_send(port: &mut dyn Port, cmd: &[u8]) -> Result<()> {
    port.exchange(&xs1_report(cmd), &HidTransfer::FeatureOut).map(|_| ())
}

fn xs1_dpi(a: &[u8]) -> (u32, u32) {
    (u16::from_le_bytes([a[7], a[8]]) as u32, u16::from_le_bytes([a[9], a[10]]) as u32)
}

impl crate::family::Driver for Xs1 {
    /// battery, DPI, lift-off, polling - the queries OpenMouse's readStatus sends.
    fn read(&self, m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let battery = xs1_query(port, &XS1_Q_BATTERY)?;
        let dpi = xs1_query(port, &XS1_Q_DPI)?;
        // Order 110: OpenMouse's readStatus asks each of these, each optional (the packet's byte 7)
        let extras = XS1_EXTRAS
            .iter()
            .map(|(e, q)| {
                let v = xs1_query(port, q).ok().map(|a| a[7] as i32);
                ExtraRow::new(*e, xs1_kind(*e), if *e == Extra::Debounce { v } else { v.map(|x| (x == 1) as i32) })
            })
            .collect();
        let lod = xs1_query(port, &XS1_Q_LIFT_OFF).ok();
        let polling = xs1_query(port, &XS1_Q_POLLING).ok();
        let rates: &[u32] = if m.variant == XS1_8K_RECEIVER { &POLLING_RATES } else { &XS1_POLLING_RATES };
        Ok(OnMouse {
            model: None,
            online: true,
            battery_percent: (battery[6] <= 100).then_some(battery[6]),
            charging: None,
            dpi: Some(xs1_dpi(&dpi).0),
            polling_hz: polling.and_then(|p| xs1_polling_hz(p[7])).filter(|h| rates.contains(h)),
            lift_off: lod.and_then(|l| xs1_lift_off_tenths(l[8])),
            wireless: m.wireless,
            // polling is read-only on XS-1 (OpenMouse never writes it)
            caps: Caps { dpi: Some(DpiCaps::uniform(50, 26000, 50)), polling: Vec::new(), lift_off: XS1_LIFT_OFF.iter().map(|l| l.1).collect(), battery: true },
            extras,
        })
    }

    /// Order 110: OpenMouse's setDebounceTime / setMotionSync / setAngleSnapping / setRippleControl: the write, then the
    /// query must answer the value.
    fn change(&self, _m: &Matched, port: &mut dyn Port, c: Change) -> Result<()> {
        let Change::Extra(e, v) = c;
        let rows: Vec<ExtraRow> = XS1_EXTRAS.iter().map(|(e, _)| ExtraRow::new(*e, xs1_kind(*e), None)).collect();
        check_extra(&rows, e, v)?;
        let q = XS1_EXTRAS.iter().find(|x| x.0 == e).map(|x| x.1).ok_or_else(|| crate::family::not_offered(c))?;
        xs1_send(port, &xs1_value_write(q, v as u8))?;
        let now = xs1_query(port, &q)?[7] as i32;
        if now != v {
            return Err(Error::BadAnswer(format!("the mouse kept {now} instead of {v} ({})", e.label())));
        }
        Ok(())
    }

    fn write(&self, _m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        match s {
            Setting::Dpi(dpi) => {
                if !DpiCaps::uniform(50, 26000, 50).allows(dpi) {
                    return Err(Error::range("DPI", format!("{dpi} DPI is not supported by this Pulsar sensor")));
                }
                let before = xs1_query(port, &XS1_Q_DPI).ok().map(|a| xs1_dpi(&a).0);
                xs1_send(port, &xs1_dpi_write(dpi as u16))?;
                let (x, y) = xs1_dpi(&xs1_query(port, &XS1_Q_DPI)?);
                if x != dpi || y != dpi {
                    return Err(Error::BadAnswer(format!("the mouse kept {x} DPI instead of {dpi}")));
                }
                Ok(Changed { now: dpi, before })
            }
            Setting::LiftOff(t) => {
                let code = XS1_LIFT_OFF.iter().find(|l| l.1 == t).map(|l| l.0).ok_or_else(|| Error::range("lift-off distance", format!("{t} tenths of a mm")))?;
                let before = xs1_query(port, &XS1_Q_LIFT_OFF).ok().and_then(|a| xs1_lift_off_tenths(a[8]));
                xs1_send(port, &xs1_lift_off_write(code))?;
                let now = xs1_lift_off_tenths(xs1_query(port, &XS1_Q_LIFT_OFF)?[8]);
                if now != Some(t) {
                    return Err(Error::BadAnswer(format!("the mouse kept {now:?} instead of {t} tenths of a mm")));
                }
                Ok(Changed { now: t, before })
            }
            Setting::Polling(_) => Err(Error::range("polling rate", "this Pulsar's polling rate can only be changed in Pulsar's software".to_string())),
        }
    }
}

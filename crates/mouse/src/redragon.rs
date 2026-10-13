//! Redragon (Holtek vendor id 0x04d9) - ported from OpenMouse (`crate::family`: AGPL-3.0-or-later, @openmouse/protocol
//! 0.40.1), polling and DPI only, for the two models on OpenMouse's SUPPORTED list:
//!
//! - the codec: `redragon/index.js` (frames, the M724's DPI / polling encodings, the M612's flat settings memory and its
//!   DPI label table);
//! - the drivers: `drivers/redragon/hid.js` - `RedragonHidClient` (the write-only M724 K1NG 1K, `04d9:fc7a`) and its
//!   `RedragonM612HidClient` subclass (the Predator M612, `04d9:fc61`, which answers reads).
//!   `drivers/redragon/m690-pro-hid.js` (RedragonM690ProHidClient, SinoWealth 0x258a) is NOT ported: no M690 Pro is on the
//!   SUPPORTED list.
//!
//! Transport: the Holtek vendor collection (usage page 0xFFA0, usage 0x01, USB interface 2) with ONE numbered feature
//! report, id 2, 16 bytes on Windows (id + 15). A write is `HidD_SetFeature`, 30 ms (OpenMouse's WRITE_DELAY_MS) after every
//! frame; a read is a `F2` request frame (SetFeature) followed 30 ms later by `HidD_GetFeature`. Frames (our byte 0 is the
//! report id 2; OpenMouse's byte i is our i + 1 - it passes `frame.slice(1)` to sendFeatureReport):
//!
//! | frame | bytes |
//! |---|---|
//! | session hello / close | `02 F5 00 ..` / `02 F5 01 ..` (a session left open hangs the mouse until replug: the close is always sent) |
//! | commit (closes every write) | `02 F1 02 code ..` for code 04, 01, 02, 08, 10 (OpenMouse: without them a write is stored but only works after a reboot) |
//! | M612 read | `02 F2 addrLo addrHi len ..` (len 1-8); answer `02 08 addrLo 32+addrHi len ?? FA FA data..` |
//! | M612 write | `02 F3 addrLo addrHi len 00 00 00 data..` (len 1-8) |
//! | M724 polling | `02 F3 32 00 06 00 00 00 RR 00 01 00 01 00 00 00`, RR = 1000 / Hz |
//!
//! M612 settings memory (all read AND written through the frames above): active profile 0-4 at 0x2C; polling block at 0x32
//! (6 bytes, byte 0 = 1000 / Hz); per profile (bases 0x42, 0x102, 0x1B2, 0x262, 0x312) byte 0 = active DPI stage 0-4 and the
//! five stage slots at base + 2 + 6n, 5 bytes each: `[enabled, x, xRange, y, yRange]`. The DPI code is the slider position
//! `p` of RDCfg's 146-label table: `value = 14 + p, range 0` for p < 93, `value = p - 39, range 1` for p >= 93; OpenMouse
//! snaps a requested DPI to the nearest label (a tie goes to the lower one).
//!
//! What this file does: read = the config-channel probe (a bare GET_FEATURE whose answer must end in `FA FA`), then on the
//! M612 the active profile, its active stage's DPI and the polling code. write = what OpenMouse's setDpiStageValue (on
//! the ACTIVE stage - OpenMouse has no plain "set DPI" on a Redragon, the stage editor is its DPI control) and
//! setPollingRate do: hello, the changed bytes, the commit block, close, then read back and compare. The M612 setters copy
//! the slot's enabled byte / the polling block's other five bytes back exactly as read, like OpenMouse. A DPI write is refused
//! when the active slot holds a separate Y (OpenMouse writes one code on both axes; that would flatten it).
//!
//! The M724 K1NG 1K (`04d9:fc7a`) is DRIVER KNOWS, HELD BACK: it never answers a read (OpenMouse: "write-only like the
//! SteelSeries Rival 3"), so nothing can be shown for it and nothing written can be checked. Its polling frame (codec
//! `m724_polling_frame`, bytes proven in the tests) replaces the whole 6-byte block at 0x32 with `RR 00 01 00 01 00`, five bytes
//! this code does not understand, on a mouse that can never be read back (the M612's block at the same address holds 02
//! where the frame writes 01). Its DPI setter rewrites the whole profile-1 DPI table from a session cache that starts as the
//! FACTORY table (1200 / 2400 / 3500 / 5500 / 12400), i.e. it would overwrite the user's saved stages. So the M724 has NO
//! control: `matches()` does not claim it (OpenMouse can only write the M724 blind: held back) and the driver refuses it.
//!
//! Left out on purpose (not DPI / polling / lift-off / battery): the active-stage selector, profile switch, button
//! mappings, lighting. OpenMouse has no firmware / reset command for Redragon and none is here. Lift-off: Redragon has none
//! (`liftOffDistance: null`); battery: both are wired (`batteryPercent: null`).

use crate::error::{Error, Result};
use crate::family::{Caps, Changed, DpiCaps, Family, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x04d9;
pub const CONFIG_USAGE_PAGE: u16 = 0xffa0;
pub const CONFIG_USAGE: u16 = 0x01;
pub const REPORT_ID: u8 = 2;
/// The feature report: id 2 + 15 bytes.
pub const FRAME_LEN: usize = 16;
/// OpenMouse's WRITE_DELAY_MS: the pause after every frame (and before a GET_FEATURE answer).
pub const WRITE_DELAY_MS: u32 = 30;

/// `Matched.variant`: the Predator M612 (answers reads).
pub const M612: u16 = 1;
/// `Matched.variant`: the M724 K1NG 1K (write-only; held back - `matches()` never returns it, the driver refuses it).
pub const M724: u16 = 2;

/// The Redragon models this app drives: (product id, variant, model). The M724 K1NG 1K (`04d9:fc7a`, on the SUPPORTED list)
/// is NOT here: OpenMouse can only write it blind, so it is held back (see the header) - "driver knows, held back".
pub const PRODUCTS: [(u16, u16, &str); 1] = [(0xfc61, M612, "Redragon PREDATOR M612")];

/// The polling rates OpenMouse offers (REDRAGON_POLLING_RATES), ascending.
pub const POLLING_RATES: [u32; 4] = [125, 250, 500, 1000];

const CMD_WRITE: u8 = 0xf3;
const CMD_HELLO: u8 = 0xf5;
const CMD_COMMIT: u8 = 0xf1;
const CMD_READ: u8 = 0xf2;
const COMMIT_SECTION: u8 = 0x02;
/// REDRAGON_COMMIT_CODES: the codes closing every write session, in order.
pub const COMMIT_CODES: [u8; 5] = [0x04, 0x01, 0x02, 0x08, 0x10];
const REPLY_TAG: u8 = 0x08;
const REPLY_HEADER: usize = 7;
const MAX_TRANSFER: usize = 8;

const ACTIVE_PROFILE_ADDRESS: u16 = 0x002c;
const POLL_ADDRESS: u16 = 0x0032;
const POLL_LENGTH: usize = 6;
const PROFILE_BASES: [u16; 5] = [0x0042, 0x0102, 0x01b2, 0x0262, 0x0312];
const STAGE_COUNT: usize = 5;
const SLOT_LENGTH: usize = 5;
/// The slider position where RDCfg switches to range 1.
const RANGE1_FIRST_POSITION: usize = 93;

/// REDRAGON_M612_DPI_LABELS: the DPI RDCfg shows for each of the 146 positions of its slider.
pub const M612_DPI_LABELS: [u32; 146] = [
    500, 570, 600, 640, 680, 700, 760, 800, 830, 870, 900, 950, 980, 1000, 1060, 1100, 1140, 1170, 1200, 1250, 1300, 1330, 1360, 1400, 1440, 1480, 1500, 1550, 1600, 1630, 1670, 1700, 1740, 1780, 1800, 1860,
    1900, 1930, 1970, 2000, 2050, 2090, 2100, 2160, 2200, 2240, 2280, 2300, 2350, 2400, 2430, 2470, 2500, 2540, 2580, 2600, 2660, 2700, 2730, 2770, 2800, 2850, 2880, 2900, 2960, 3000, 3040, 3070, 3100, 3150,
    3200, 3230, 3260, 3300, 3340, 3380, 3400, 3450, 3500, 3530, 3570, 3600, 3640, 3680, 3700, 3760, 3800, 3830, 3870, 3900, 3950, 3990, 4000, 4100, 4180, 4250, 4330, 4400, 4480, 4560, 4630, 4700, 4780, 4860,
    4940, 5000, 5100, 5160, 5240, 5300, 5400, 5470, 5540, 5600, 5700, 5770, 5850, 5900, 6000, 6080, 6150, 6230, 6300, 6380, 6460, 6530, 6600, 6680, 6760, 6840, 6900, 7000, 7060, 7140, 7200, 7300, 7370, 7440,
    7500, 7600, 7670, 7750, 7800, 7900, 7980, 8000,
];

// ============================================================================ codec

fn blank() -> [u8; FRAME_LEN] {
    let mut f = [0u8; FRAME_LEN];
    f[0] = REPORT_ID;
    f
}

/// redragonSession: `02 F5 00` opens an RDCfg session, `02 F5 01` closes it.
pub fn session_frame(begin: bool) -> [u8; FRAME_LEN] {
    let mut f = blank();
    f[1] = CMD_HELLO;
    f[2] = if begin { 0x00 } else { 0x01 };
    f
}

/// redragonEncodeCommit: `02 F1 02 code`.
pub fn commit_frame(code: u8) -> [u8; FRAME_LEN] {
    let mut f = blank();
    f[1] = CMD_COMMIT;
    f[2] = COMMIT_SECTION;
    f[3] = code;
    f
}

fn check_transfer(len: usize) -> Result<()> {
    if !(1..=MAX_TRANSFER).contains(&len) {
        return Err(Error::range("Redragon transfer", format!("{len} bytes is outside 1-{MAX_TRANSFER}")));
    }
    Ok(())
}

/// redragonEncodeRead: `02 F2 addrLo addrHi len`.
pub fn read_frame(address: u16, len: usize) -> Result<[u8; FRAME_LEN]> {
    check_transfer(len)?;
    let mut f = blank();
    f[1] = CMD_READ;
    f[2] = address as u8;
    f[3] = (address >> 8) as u8;
    f[4] = len as u8;
    Ok(f)
}

/// redragonEncodeWrite: `02 F3 addrLo addrHi len 00 00 00 data..`.
pub fn write_frame(address: u16, data: &[u8]) -> Result<[u8; FRAME_LEN]> {
    check_transfer(data.len())?;
    let mut f = blank();
    f[1] = CMD_WRITE;
    f[2] = address as u8;
    f[3] = (address >> 8) as u8;
    f[4] = data.len() as u8;
    f[8..8 + data.len()].copy_from_slice(data);
    Ok(f)
}

/// REDRAGON_POLLING_CODES: rate = 1000 / code.
pub fn polling_code(hz: u32) -> Option<u8> {
    match hz {
        1000 => Some(0x01),
        500 => Some(0x02),
        250 => Some(0x04),
        125 => Some(0x08),
        _ => None,
    }
}

/// The rate of a polling code (`REDRAGON_POLLING_RATES.find(hz => CODES[hz] === code)`).
pub fn polling_hz(code: u8) -> Option<u32> {
    POLLING_RATES.iter().copied().find(|hz| polling_code(*hz) == Some(code))
}

/// redragonEncodePollingRate - the M724's polling frame: `02 F3 32 00 06 00 00 00 RR 00 01 00 01 00 00 00`.
pub fn m724_polling_frame(hz: u32) -> Option<[u8; FRAME_LEN]> {
    let mut f = blank();
    f[1] = CMD_WRITE;
    f[2] = 0x32;
    f[3] = 0x00;
    f[4] = 0x06;
    f[8] = polling_code(hz)?;
    f[10] = 0x01;
    f[12] = 0x01;
    Some(f)
}

/// redragonM612DpiCodeAt: the wire code of RDCfg's slider position `p` (0-145): (value, range).
pub fn m612_code_at(position: usize) -> Option<(u8, u8)> {
    if position >= M612_DPI_LABELS.len() {
        return None;
    }
    Some(if position < RANGE1_FIRST_POSITION { ((14 + position) as u8, 0) } else { ((position - 39) as u8, 1) })
}

/// redragonM612EncodeDpi: snaps a DPI to the nearest slider label (a tie goes to the lower one) and gives that label with
/// its wire code. `None` = outside 500-8000 (OpenMouse throws before anything is sent).
pub fn m612_encode_dpi(dpi: u32) -> Option<(u32, u8, u8)> {
    if dpi < M612_DPI_LABELS[0] || dpi > M612_DPI_LABELS[M612_DPI_LABELS.len() - 1] {
        return None;
    }
    let mut best = 0;
    for (p, label) in M612_DPI_LABELS.iter().enumerate().skip(1) {
        if label.abs_diff(dpi) < M612_DPI_LABELS[best].abs_diff(dpi) {
            best = p;
        }
    }
    let (value, range) = m612_code_at(best)?;
    Some((M612_DPI_LABELS[best], value, range))
}

/// redragonM612DecodeDpi: a wire code -> the DPI RDCfg labels it with; any other code falls back to the linear sensor scale
/// (4000 DPI per 106 counts, rounded to 10).
pub fn m612_decode_dpi(value: u8, range: u8) -> u32 {
    let v = value as i32;
    let position: i32 = match range {
        0 => v - 14,
        1 => v + 39,
        _ => -1,
    };
    let in_range = if range == 0 { position < RANGE1_FIRST_POSITION as i32 } else { position >= RANGE1_FIRST_POSITION as i32 };
    if in_range && position >= 0 && (position as usize) < M612_DPI_LABELS.len() {
        return M612_DPI_LABELS[position as usize];
    }
    let scale = if range == 1 { 2.0 } else { 1.0 };
    ((value as f64 * scale * 4000.0 / 106.0 / 10.0).round() * 10.0) as u32
}

/// The address of one M612 DPI stage slot (`None` = profile or stage out of range).
pub fn m612_slot_address(profile: usize, stage: usize) -> Option<u16> {
    (stage < STAGE_COUNT).then(|| PROFILE_BASES.get(profile).map(|b| b + 2 + 6 * stage as u16)).flatten()
}

/// redragonDecodeRead: the data bytes of the GET_FEATURE answer to `read_frame(address, len)`; the answer may or may not
/// start with the report id.
pub fn decode_read(answer: &[u8], address: u16, len: usize) -> Result<Vec<u8>> {
    check_transfer(len)?;
    let body = if answer.len() > 1 && answer[0] == REPORT_ID && answer[1] == REPLY_TAG { &answer[1..] } else { answer };
    if body.len() < REPLY_HEADER + len || body[0] != REPLY_TAG || body[1] != address as u8 || body[2] != 0x32u8.wrapping_add((address >> 8) as u8) || body[3] != len as u8 || body[5] != 0xfa || body[6] != 0xfa {
        return Err(Error::BadAnswer(format!("Redragon read of {len} bytes at {address:#06x} got an unexpected answer: {answer:02x?}")));
    }
    Ok(body[REPLY_HEADER..REPLY_HEADER + len].to_vec())
}

/// probeConfigChannel: every GET_FEATURE on the config channel ends in `FA FA` (the keyboard / mouse interfaces have no
/// feature report 2 at all).
pub fn has_config_marker(echo: &[u8]) -> bool {
    echo.len() >= 2 && echo[echo.len() - 2] == 0xfa && echo[echo.len() - 1] == 0xfa
}

// ============================================================================ the device table

fn is_config(h: &HidInfo) -> bool {
    h.usage_page == CONFIG_USAGE_PAGE && h.usage == CONFIG_USAGE && h.feature_len as usize >= FRAME_LEN
}

/// OpenMouse's RedragonHidClient.isSupported: Holtek's vendor id, a product id of the SUPPORTED list, and the vendor
/// collection 0xFFA0 / 0x01 with feature report 2.
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID {
        return None;
    }
    let (_, variant, model) = PRODUCTS.iter().find(|p| p.0 == pid)?;
    let config = own.iter().find(|h| is_config(h))?;
    Some(Matched { family: Family::Redragon, model: Some(model), wireless: Some(false), config: (*config).clone(), variant: *variant })
}

/// The read requests this file sends: the M612's `F2` reads (address, length). Everything else - hello / close, writes,
/// commits - is a write. (The probe's blank frame is only ever fetched with GET_FEATURE, which nothing gates; it is not a
/// request and does not pass here.)
fn read_addresses() -> Vec<(u16, usize)> {
    let mut v = vec![(ACTIVE_PROFILE_ADDRESS, 1), (POLL_ADDRESS, 1), (POLL_ADDRESS, POLL_LENGTH)];
    for (profile, base) in PROFILE_BASES.iter().enumerate() {
        v.push((*base, 1));
        for stage in 0..STAGE_COUNT {
            if let Some(a) = m612_slot_address(profile, stage) {
                v.push((a, SLOT_LENGTH));
            }
        }
    }
    v
}

/// The allow-list for proofs on a real mouse: exactly the `F2` read requests above, zero-padded to the interface's length.
/// Every write is refused - and so is the blank frame `02 00 ..` (the probe passes it with GET_FEATURE only, which sends
/// nothing and is let through by the gate itself).
pub fn read_request_allowed(out: &[u8]) -> bool {
    if out.len() < FRAME_LEN || out[0] != REPORT_ID || out[FRAME_LEN..].iter().any(|b| *b != 0) {
        return false;
    }
    let f = &out[..FRAME_LEN];
    f[1] == CMD_READ && f[5..].iter().all(|b| *b == 0) && read_addresses().iter().any(|(a, l)| f[2] == *a as u8 && f[3] == (*a >> 8) as u8 && f[4] == *l as u8)
}

// ============================================================================ the driver

fn model_variant(m: &Matched) -> Result<u16> {
    match m.variant {
        M612 => Ok(M612),
        M724 => Err(Error::UnsupportedMouse("Redragon M724 K1NG 1K: OpenMouse can only write it blind - held back".into())),
        v => Err(Error::UnsupportedMouse(format!("Redragon variant {v}"))),
    }
}

fn caps(variant: u16) -> Caps {
    match variant {
        // the stage editor's range (minDpi labels[0], maxDpi labels[last], stepDpi 10); the mouse stores the nearest label
        M612 => Caps { dpi: Some(DpiCaps::uniform(500, 8000, 10)), polling: POLLING_RATES.to_vec(), lift_off: Vec::new(), battery: false },
        // the M724 is held back (nothing to show or set)
        _ => Caps::default(),
    }
}

fn buf_len(port: &dyn Port) -> usize {
    (port.info().feature_len as usize).max(FRAME_LEN)
}

fn padded(f: &[u8; FRAME_LEN], len: usize) -> Vec<u8> {
    let mut v = f.to_vec();
    v.resize(len, 0);
    v
}

/// sendFrame: one feature report, then OpenMouse's 30 ms.
fn send(port: &mut dyn Port, f: &[u8; FRAME_LEN]) -> Result<()> {
    let out = padded(f, buf_len(port));
    port.exchange(&out, &HidTransfer::FeatureOut)?;
    port.pause_ms(WRITE_DELAY_MS as u64);
    Ok(())
}

/// probeConfigChannel: GET_FEATURE alone (nothing is sent; `out` is the blank frame the allow-list knows).
fn probe(port: &mut dyn Port) -> Result<()> {
    let len = buf_len(port);
    let a = port.exchange(&padded(&blank(), len), &HidTransfer::FeatureIn { reply_id: REPORT_ID, reply_len: len, wait_ms: 0 })?;
    if has_config_marker(&a) {
        Ok(())
    } else {
        Err(Error::BadAnswer(format!("the Redragon config channel answered {} bytes without its FA FA marker", a.len())))
    }
}

/// readBytes: the read request, 30 ms, the GET_FEATURE answer.
fn read_bytes(port: &mut dyn Port, address: u16, len: usize) -> Result<Vec<u8>> {
    let out = padded(&read_frame(address, len)?, buf_len(port));
    let how = HidTransfer::Feature { reply_id: REPORT_ID, reply_len: buf_len(port), wait_ms: WRITE_DELAY_MS };
    let a = port.exchange(&out, &how)?;
    decode_read(&a, address, len)
}

/// writeSession: hello, the operation, close - the close is sent even when the operation failed (an open session hangs the
/// mouse's interface until it is replugged).
fn write_session(port: &mut dyn Port, op: impl FnOnce(&mut dyn Port) -> Result<()>) -> Result<()> {
    send(port, &session_frame(true))?;
    let done = op(port);
    let closed = send(port, &session_frame(false));
    done.and(closed)
}

fn send_commits(port: &mut dyn Port) -> Result<()> {
    for code in COMMIT_CODES {
        send(port, &commit_frame(code))?;
    }
    Ok(())
}

/// writeAndConfirm: one write inside the session bracket + the commit block, then a read-back that must match.
fn write_and_confirm(port: &mut dyn Port, address: u16, data: &[u8]) -> Result<()> {
    write_session(port, |p| {
        send(p, &write_frame(address, data)?)?;
        send_commits(p)
    })?;
    let back = read_bytes(port, address, data.len())?;
    if back != data {
        return Err(Error::BadAnswer(format!("the Redragon M612 kept {back:02x?} at {address:#06x} instead of {data:02x?}")));
    }
    Ok(())
}

fn read_active_profile(port: &mut dyn Port) -> Result<usize> {
    let p = read_bytes(port, ACTIVE_PROFILE_ADDRESS, 1)?[0] as usize;
    if p >= PROFILE_BASES.len() {
        return Err(Error::BadAnswer(format!("the Redragon M612 reported profile {p}; expected 0-4")));
    }
    Ok(p)
}

/// The active profile, its active stage (0-4) and that stage's slot address.
fn read_active_stage(port: &mut dyn Port) -> Result<(usize, u16)> {
    let profile = read_active_profile(port)?;
    let stage = read_bytes(port, PROFILE_BASES[profile], 1)?[0] as usize;
    let address = m612_slot_address(profile, stage).ok_or_else(|| Error::BadAnswer(format!("the Redragon M612 reported DPI stage {stage}; expected 0-4")))?;
    Ok((profile, address))
}

/// The Redragon driver (`Matched.variant`: [`M612`] or [`M724`]).
pub struct Redragon;

impl crate::family::Driver for Redragon {
    /// readStatus: the config-channel probe; then the active stage's DPI and the polling code (M612 only - the M724 is
    /// held back and refused by `model_variant`).
    fn read(&self, m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let variant = model_variant(m)?;
        let mut r = OnMouse { model: None, online: true, wireless: Some(false), caps: caps(variant), ..Default::default() };
        probe(port)?;
        let (_, address) = read_active_stage(port)?;
        let slot = read_bytes(port, address, SLOT_LENGTH)?;
        r.dpi = Some(m612_decode_dpi(slot[1], slot[2]));
        // an unknown code: nothing is shown (OpenMouse's readSettings would refuse the whole status)
        r.polling_hz = polling_hz(read_bytes(port, POLL_ADDRESS, 1)?[0]);
        Ok(r)
    }

    /// setDpiStageValue (the ACTIVE stage) / setPollingRate as OpenMouse's client does them, then the read-back.
    fn write(&self, m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        model_variant(m)?;
        match s {
            Setting::Dpi(dpi) => {
                // validated before anything is sent
                let (label, value, range) = m612_encode_dpi(dpi).ok_or_else(|| Error::range("DPI", format!("{dpi} DPI is outside 500 - 8000 on the Redragon M612")))?;
                let (_, address) = read_active_stage(port)?;
                let slot = read_bytes(port, address, SLOT_LENGTH)?;
                // X and Y are one number here (setDpiStageValue writes the same code on both axes): a slot with a separate Y
                // is not flattened
                if slot[1..3] != slot[3..5] {
                    return Err(Error::range("DPI", format!("the active DPI stage has a separate Y ({} x {} DPI), which this would overwrite", m612_decode_dpi(slot[1], slot[2]), m612_decode_dpi(slot[3], slot[4]))));
                }
                let before = m612_decode_dpi(slot[1], slot[2]);
                write_and_confirm(port, address, &[slot[0], value, range, value, range])?;
                Ok(Changed { now: label, before: Some(before) })
            }
            Setting::Polling(hz) => {
                let code = polling_code(hz).ok_or_else(|| Error::range("polling rate", format!("{hz} Hz is not one of 125, 250, 500, 1000")))?;
                let mut block = read_bytes(port, POLL_ADDRESS, POLL_LENGTH)?;
                let before = polling_hz(block[0]);
                block[0] = code;
                write_and_confirm(port, POLL_ADDRESS, &block)?;
                Ok(Changed { now: hz, before })
            }
            Setting::LiftOff(_) => Err(Error::range("lift-off distance", "Redragon mice have no lift-off setting here".to_string())),
        }
    }
}

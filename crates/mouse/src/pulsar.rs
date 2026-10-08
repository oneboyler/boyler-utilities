//! Pulsar "cMouse" 17-byte HID protocol (X2 CrazyLight family) — frames, checksums and value codecs. Pure functions, no I/O.
//!
//! Written from the protocol FACTS published in MIT-licensed projects, each verified on a real X2 CrazyLight:
//! pulsar-mouse-linux (packerlschupfer, `docs/protocol-x2-crazylight.md`), PulsarBattery (darthsoup), Bibimbap (amassias,
//! `docs/protocol.md`), pulsar-battery-notifier (shuukree). No code was copied from any of them, and nothing from OpenMouse
//! (AGPL-3.0). Pulsar publishes no protocol documentation of its own.
//!
//! Frame (17 bytes, report id included): `[0]` report id 0x08 · `[1]` command (echoed) · `[2]` status in answers (0 = data,
//! 1 = ack / unsupported) · `[3..5]` settings-memory address, big-endian · `[5]` length (≤ 10) · `[6..16]` data ·
//! `[16]` checksum = 0x55 − sum(bytes 0..16) (mod 256).

pub const REPORT_ID: u8 = 0x08;
pub const FRAME_LEN: usize = 17;

/// Identify ("EncryptionData"): 4 random bytes + 4 zeros; answer `[10]` family (0x57), `[11]` model, `[12]` connection type.
pub const CMD_IDENTIFY: u8 = 0x01;
/// Config software present ("PCDriverStatus"): `[6]` = 1 announces the app, 0 releases it; echoed. Sent only around a
/// settings session that failed without it (PulsarBattery's "active session") - never by the read-only proof tools.
pub const CMD_DRIVER: u8 = 0x02;
/// Online / write lock ("DeviceOnLine"), two forms:
/// - the QUERY (length 0, no data): answer status 0, `[6]` = 1 when the mouse is online behind the dongle, `[10]` = busy
///   (ask again until 0) - [`online_query_frame`];
/// - the HOLD (length 1): `[6]` = 1 locks for writes, 0 unlocks; answered with status 1 (an ack, "no data") that echoes the
///   byte - it says nothing about the mouse (measured on the real 8K dongle: `08 03 01 00 00 01 00 …` for the unlock).
pub const CMD_ONLINE: u8 = 0x03;
/// Battery: answer `[6]` percent, `[7]` charging, `[8..10]` millivolts big-endian.
pub const CMD_BATTERY: u8 = 0x04;
/// Write settings memory — NEVER sent in tests or proofs.
pub const CMD_WRITE: u8 = 0x07;
/// Read settings memory.
pub const CMD_READ: u8 = 0x08;
/// Factory reset — never used by this app.
pub const CMD_RESET: u8 = 0x09;
/// Get active profile.
pub const CMD_GET_PROFILE: u8 = 0x0E;
/// Set active profile — never used by this app.
pub const CMD_SET_PROFILE: u8 = 0x0F;
/// Mouse firmware version.
pub const CMD_VERSION: u8 = 0x12;
/// Dongle firmware version.
pub const CMD_DONGLE_VERSION: u8 = 0x1D;

/// The family code every cMouse answers in the identify reply.
pub const FAMILY_CMOUSE: u8 = 0x57;

/// Settings-memory addresses (they always show the ACTIVE onboard profile).
pub const ADDR_POLLING: u16 = 0x0000;
pub const ADDR_STAGE_COUNT: u16 = 0x0002;
pub const ADDR_ACTIVE_STAGE: u16 = 0x0004;
pub const ADDR_LIFT_OFF: u16 = 0x000A;
pub const ADDR_DPI_STAGE0: u16 = 0x000C;

pub fn checksum(b: &[u8]) -> u8 {
    0x55u8.wrapping_sub(b.iter().fold(0u8, |a, x| a.wrapping_add(*x)))
}

/// Builds a 17-byte request.
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

/// "Is the mouse online?" - the query form of [`CMD_ONLINE`] (length 0).
pub fn online_query_frame() -> [u8; FRAME_LEN] {
    frame(CMD_ONLINE, 0, &[])
}

/// The write hold: `lock` = take it before writes, else release it.
pub fn hold_frame(lock: bool) -> [u8; FRAME_LEN] {
    frame(CMD_ONLINE, 0, &[lock as u8])
}

/// The identify request with a 4-byte challenge (an all-zero frame is rejected by the mouse).
pub fn identify_frame(challenge: [u8; 4]) -> [u8; FRAME_LEN] {
    frame(CMD_IDENTIFY, 0, &[challenge[0], challenge[1], challenge[2], challenge[3], 0, 0, 0, 0])
}

/// The allow-list for proofs on the real mouse (A_009_01): only READ requests may leave — identify, online with the
/// lock byte 0, battery, read settings memory, get profile, versions. Never write (0x07), reset (0x09), set profile (0x0F),
/// or the online command with the write lock set. The frame must be well-formed (report id, checksum).
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
pub fn check_answer(cmd: u8, a: &[u8]) -> Result<(), String> {
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

/// Polling rate code ↔ Hz: 0x08 = 125, 0x04 = 250, 0x02 = 500, 0x01 = 1000, 0x10 = 2000, 0x20 = 4000, 0x40 = 8000.
pub fn polling_code(hz: u32) -> Option<u8> {
    Some(match hz {
        125 => 0x08,
        250 => 0x04,
        500 => 0x02,
        1000 => 0x01,
        2000 => 0x10,
        4000 => 0x20,
        8000 => 0x40,
        _ => return None,
    })
}

pub fn polling_hz(code: u8) -> Option<u32> {
    Some(match code {
        0x08 => 125,
        0x04 => 250,
        0x02 => 500,
        0x01 => 1000,
        0x10 => 2000,
        0x20 => 4000,
        0x40 => 8000,
        _ => return None,
    })
}

/// Lift-off distance in tenths of a millimetre ↔ code: 0x01 = 1 mm, 0x02 = 2 mm, 0x03 = 0.7 mm.
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
    pub fn max_polling_hz(self) -> u32 {
        match self {
            Link::Wireless1k | Link::Wired1k | Link::Unknown(_) => 1000,
            Link::Wireless2k => 2000,
            Link::Wireless4k => 4000,
            Link::Wireless8k | Link::Wired8k => 8000,
        }
    }
}

/// One axis of a DPI stage: `raw` (10 bits) + range code `ex` → DPI.
fn axis_dpi(raw: u32, ex: u8) -> Option<u32> {
    match ex {
        0 => Some((raw + 1) * 10),
        2 => Some((raw + 201) * 50),
        3 => Some((raw + 201) * 100),
        _ => None,
    }
}

/// DPI → (raw, ex), switching range like Pulsar's own app: 10-DPI steps up to 10240, 50-DPI steps up to 25600, then 100.
fn axis_raw(dpi: u32) -> Option<(u32, u8)> {
    if (10..=10240).contains(&dpi) && dpi.is_multiple_of(10) {
        Some((dpi / 10 - 1, 0))
    } else if dpi > 10240 && dpi <= 25600 && dpi.is_multiple_of(50) {
        Some((dpi / 50 - 201, 2))
    } else if dpi > 25600 && dpi <= 32000 && dpi.is_multiple_of(100) {
        Some((dpi / 100 - 201, 3))
    } else {
        None
    }
}

/// A DPI stage record `[x_lo, y_lo, flags, cksum]` → (x DPI, y DPI).
/// flags = (x_hi << 2) | (y_hi << 6) | x_ex | (y_ex << 4); cksum = 0x55 − (x_lo + y_lo + flags).
pub fn decode_dpi_stage(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 4 || checksum(&b[..3]) != b[3] {
        return None;
    }
    let f = b[2];
    let x = axis_dpi(b[0] as u32 + 256 * ((f >> 2) & 3) as u32, f & 3)?;
    let y = axis_dpi(b[1] as u32 + 256 * ((f >> 6) & 3) as u32, (f >> 4) & 3)?;
    Some((x, y))
}

/// Same DPI on both axes → the 4-byte stage record.
pub fn encode_dpi_stage(dpi: u32) -> Option<[u8; 4]> {
    let (raw, ex) = axis_raw(dpi)?;
    let lo = (raw & 0xFF) as u8;
    let hi = ((raw >> 8) & 3) as u8;
    let flags = (hi << 2) | (hi << 6) | ex | (ex << 4);
    let mut r = [lo, lo, flags, 0];
    r[3] = checksum(&r[..3]);
    Some(r)
}

/// Battery answer → (percent, charging, millivolts).
pub fn decode_battery(a: &[u8]) -> Option<(u8, bool, u16)> {
    (a.len() >= 10).then(|| (a[6].min(100), a[7] == 1, u16::from_be_bytes([a[8], a[9]])))
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

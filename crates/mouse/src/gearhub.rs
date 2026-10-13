//! GearHub-V5 (Lingbao M5 Pro, Attack Shark R2) - ported from OpenMouse (`crate::family`: AGPL-3.0-or-later,
//! @openmouse/protocol 0.40.1), DPI / polling / lift-off / battery only:
//!
//! - the codec: `gearhub/index.js` (command ids, the Bit7 checksum, report-rate codes, the DPI table layout, the device
//!   profiles and their lift-off stops);
//! - the driver: `drivers/gearhub/hid.js` (GearHubHidClient: the 2.4 GHz receiver relay, `readStatus`, `setDpi`,
//!   `setPollingRate`, `setLiftOffDistance`); which collection it claims: its `isSupported`. Registered ahead of Fantech's
//!   client because the receiver shares VID 0x3151 and usage 0xFFFF / 0x02 with it (`drivers/registry.js` comment).
//!
//! Transport: the vendor collection usage page 0xFFFF, usage 0x02 with ONE unnumbered feature report (id 0), 64 bytes - on
//! Windows 65 bytes, id 0 in front. Every request is `HidD_SetFeature`; an answer is `HidD_GetFeature` after OpenMouse's
//! 10 ms. A command is 9 bytes padded to 64: `[cmd, args.., 0.., checksum at [7]]`, checksum = `255 - (sum of bytes 0..7)`
//! (OpenMouse's "Bit7 checksum"). Answers echo the command id in byte 0; 64 zeros = "not answered".
//!
//! Two links, told apart by the product id: the 2.4 GHz receiver `3151:402D` (a RELAY) and the cable `3151:4026` (direct;
//! polling capped at 1000 Hz). The Bluetooth id 0x402C is the AJAZZ AJ179 PRO's (not on the SUPPORTED list): not ported.
//! The RECEIVER RELAY, per command (these are raw receiver frames, no checksum): `F6 05` once = select the mouse as the
//! target; then poll `F7` (answer `[canRead, kbBattery, mouseBattery, kbOnline==0, mouseOnline==0, canSend]`, up to 8 times
//! 100 ms apart) until canSend; send the checksummed command; poll `F7` until canRead; `FC` = notice-read, answer = the
//! command's reply. A write is select, wait for canSend, send, 50 ms. THE HANDSHAKE IS PART OF EVERY READ (OpenMouse
//! cannot read the mouse through the receiver without it), so it is ported and is in the read allow-list: `F6 05` only
//! picks which paired device the receiver talks to (volatile routing, no setting, no pairing); `F7` / `FC` only read.
//!
//! Commands ported (all that OpenMouse sends for the model, DPI, polling, lift-off and battery): `8F` device id (LE u32; the
//! model: 2285 = Lingbao M5 Pro, 1893 / 3009 = Attack Shark R2), `D4 00` DPI table (read), `54` DPI table (write), `D3` the
//! 64-byte OPTIONPARAM0 block (read; byte 9 = polling code, byte 52 = lift-off index), `53` the same block (write). Battery
//! is the receiver's `F7` answer byte 2 (the wired mouse has none).
//! - DPI: `D4` returns active index `[2]`, stage count `[3]`, X at `[8 + 2i]` (LE16), Y at `[24 + 2i]`, stage colour at
//!   `[40 + 3i]` (r g b). OpenMouse's setDpi replaces X AND Y of the ACTIVE stage and writes the whole table back with `54`
//!   (`[54, profile 0, active, count, 0 0 0, checksum, X.., Y.., rgb..]`, bytes OpenMouse does not decode are written as 0),
//!   every other stage and every colour carried over as read.
//! - polling and lift-off: OpenMouse reads OPTIONPARAM0, copies bytes 8..63 back unchanged into a `53` block, stamps
//!   bytes 17 = 255 and 18 = 8 "the way GearHub's own writer does", changes ONE byte (9 = rate code, 52 = lift-off index) and
//!   writes it. Done the same here, with a read-back of the block afterwards. Rate codes: 8000 = 129, 4000 = 130, 2000 = 132,
//!   1000 = 1, 500 = 2, 250 = 4, 125 = 8.
//!
//! Lift-off in millimetres: the values OpenMouse's UI/driver exchange are the words Low / Medium / High (index into the
//! sensor's list). The millimetres are only stated in a comment of OpenMouse's own source, taken from GearHub's
//! `getLiftOffDistance()`: PAW3950 (the R2) 0.7 / 1 / 2 mm, PAW3395 (the M5 Pro) 1 / 2 mm. They are used here as tenths
//! 7 / 10 / 20 and 10 / 20 in the driver's tables - but a figure that stands only in a comment is not a figure OpenMouse
//! itself uses (Order 104 rule, as for Logitech / Razer), so `Caps.lift_off` is EMPTY: the Mouse page shows no lift-off row for these.
//!
//! Left out on purpose: button mappings (`D0` / `50`), firmware (`80`), debounce, sleep time, angle snapping, ripple
//! correction, DPI stage colour / count / active-stage selection (none is DPI / polling / lift-off / battery); OpenMouse's
//! readStatus also reads the key matrix and the firmware - not needed here. There is NO firmware-update, reset or
//! pairing command in OpenMouse's GearHub client and none here. A device id other than the three above (Attack Shark R3,
//! AJAZZ AJ179 PRO, an unknown GearHub-V5 mouse) is refused as `UnsupportedMouse`: it is not on the SUPPORTED list.
//! Unclear: the M5 Pro was seen to answer polling code 0 (= 8000 Hz?) in OpenMouse's notes, but its own table has no code 0;
//! an unknown code reads as "polling rate not shown" here (OpenMouse falls back to 1000).

use crate::error::{Error, Result};
use crate::family::{Caps, Changed, DpiCaps, Family, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x3151;
pub const USAGE_PAGE: u16 = 0xffff;
pub const USAGE: u16 = 0x02;
/// The report's data bytes (OpenMouse's GEARHUB_REPORT_SIZE).
pub const REPORT_SIZE: usize = 64;
/// Windows' feature report: report id 0 + [`REPORT_SIZE`].
pub const REPORT_LEN: usize = REPORT_SIZE + 1;
/// A command before its zero padding (GEARHUB_CMD_SIZE).
pub const CMD_SIZE: usize = 9;
/// DPI stages the table layout has room for.
pub const MAX_DPI_STAGES: usize = 8;
/// OpenMouse's pause between a request and its GET_FEATURE (and after a direct write).
pub const ANSWER_DELAY_MS: u32 = 10;
/// writeThrough's pause after a relayed write.
pub const RELAY_WRITE_DELAY_MS: u64 = 50;
/// selectMouse's pause.
pub const SELECT_DELAY_MS: u64 = 50;
/// waitReady: pause before each of the (at most 8) status polls.
pub const READY_POLL_MS: u64 = 100;
pub const READY_ATTEMPTS: u32 = 8;

/// `Matched.variant`: the 2.4 GHz receiver (relay).
pub const DONGLE: u16 = 1;
/// `Matched.variant`: the cable (direct, polling capped at 1000 Hz).
pub const DIRECT: u16 = 2;
/// The product ids of the family that are on the SUPPORTED list (both models use both): (id, link).
pub const PRODUCTS: [(u16, u16); 2] = [(0x402d, DONGLE), (0x4026, DIRECT)];

pub const CMD_GET_USB_VERSION: u8 = 0x8f;
pub const CMD_GET_OPTIONPARAM0: u8 = 0xd3;
pub const CMD_SET_OPTIONPARAM0: u8 = 0x53;
pub const CMD_GET_DPI: u8 = 0xd4;
pub const CMD_SET_DPI: u8 = 0x54;
pub const DONGLE_SELECT_TARGET: u8 = 0xf6;
pub const DONGLE_GET_STATUS: u8 = 0xf7;
pub const DONGLE_NOTICE_READ: u8 = 0xfc;
pub const TARGET_MOUSE: u8 = 0x05;

pub const OPT0_REPORT_RATE: usize = 9;
pub const OPT0_SILENT_HEIGHT: usize = 52;
pub const DPI_X_OFFSET: usize = 8;
pub const DPI_Y_OFFSET: usize = 24;
pub const DPI_RGB_OFFSET: usize = 40;

/// GEARHUB_RATES: every rate the codec can express, ascending.
pub const RATES: [u32; 7] = [125, 250, 500, 1000, 2000, 4000, 8000];

/// One model (OpenMouse's GearHub device profile, only the fields this port uses).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile {
    pub device_id: u32,
    pub model: &'static str,
    pub min_dpi: u32,
    pub max_dpi: u32,
    pub dpi_step: u32,
    pub max_polling: u32,
    /// the lift-off stops in tenths of a mm, in OPTIONPARAM0 byte-52 index order (see the header)
    pub lift_off: &'static [u32],
}

/// The SUPPORTED list's models, by the device id `8F` answers.
pub const M5_PRO: Profile = Profile { device_id: 2285, model: "Lingbao M5 Pro", min_dpi: 50, max_dpi: 26000, dpi_step: 50, max_polling: 8000, lift_off: &[10, 20] };
pub const R2: Profile = Profile { device_id: 1893, model: "Attack Shark R2", min_dpi: 50, max_dpi: 42000, dpi_step: 50, max_polling: 8000, lift_off: &[7, 10, 20] };
/// OpenMouse maps device id 3009 (a later R2 batch, "not hardware-verified") to the R2's profile too.
pub const PROFILES: [(u32, &Profile); 3] = [(2285, &M5_PRO), (1893, &R2), (3009, &R2)];

pub fn profile_for(device_id: u32) -> Option<&'static Profile> {
    PROFILES.iter().find(|p| p.0 == device_id).map(|p| p.1)
}

impl Profile {
    pub fn allows_dpi(&self, dpi: u32) -> bool {
        (self.min_dpi..=self.max_dpi).contains(&dpi) && (dpi - self.min_dpi).is_multiple_of(self.dpi_step)
    }
    /// supportedPollingRates: the codec's rates up to the model's ceiling and the link's (the cable: 1000 Hz).
    pub fn polling_rates(&self, link: u16) -> Vec<u32> {
        let ceiling = if link == DIRECT { self.max_polling.min(1000) } else { self.max_polling };
        RATES.iter().copied().filter(|hz| *hz <= ceiling).collect()
    }
}

// ============================================================================ codec

/// encodeCommand: the command padded to 9 bytes with the Bit7 checksum `255 - (sum of bytes 0..7)` in byte 7.
pub fn encode_command(bytes: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; CMD_SIZE.max(bytes.len())];
    out[..bytes.len()].copy_from_slice(bytes);
    let sum = out[..7].iter().fold(0u8, |s, b| s.wrapping_add(*b));
    out[7] = 255 - sum;
    out
}

/// rawSend's report on Windows: report id 0, then the first 64 bytes of `payload` zero-padded, up to `len` bytes.
pub fn report(payload: &[u8], len: usize) -> Vec<u8> {
    let mut v = vec![0u8; len.max(REPORT_LEN)];
    let n = payload.len().min(REPORT_SIZE);
    v[1..1 + n].copy_from_slice(&payload[..n]);
    v
}

/// REPORT_RATE_ENCODE.
pub fn rate_code(hz: u32) -> Option<u8> {
    Some(match hz {
        8000 => 129,
        4000 => 130,
        2000 => 132,
        1000 => 1,
        500 => 2,
        250 => 4,
        125 => 8,
        _ => return None,
    })
}

/// REPORT_RATE_DECODE.
pub fn rate_hz(code: u8) -> Option<u32> {
    RATES.iter().copied().find(|hz| rate_code(*hz) == Some(code))
}

/// `F7`'s answer (readDongleStatus / pollStatus).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub can_read: bool,
    pub keyboard_battery: u8,
    pub mouse_battery: u8,
    pub keyboard_online: bool,
    pub mouse_online: bool,
    pub can_send: bool,
}

pub fn decode_status(r: &[u8]) -> Option<Status> {
    (r.len() >= 6).then(|| Status { can_read: r[0] == 1, keyboard_battery: r[1], mouse_battery: r[2], keyboard_online: r[3] == 0, mouse_online: r[4] == 0, can_send: r[5] == 1 })
}

/// A battery percentage as OpenMouse reads it: the receiver's byte, 1..=100, else none.
pub fn battery_percent(status: &Status) -> Option<u8> {
    (1..=100).contains(&status.mouse_battery).then_some(status.mouse_battery)
}

/// One DPI stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stage {
    pub x: u32,
    pub y: u32,
    pub rgb: u32,
}

/// getDpi's decoding of the `D4` answer: (stages, active index).
pub fn decode_dpi(r: &[u8]) -> Result<(Vec<Stage>, usize)> {
    if r.len() < REPORT_SIZE {
        return Err(Error::BadAnswer(format!("GearHub GET_DPI answered {} bytes", r.len())));
    }
    let active = if r[2] as usize >= MAX_DPI_STAGES { 0 } else { r[2] as usize };
    let count = (r[3] as usize).min(MAX_DPI_STAGES);
    if count == 0 {
        return Err(Error::BadAnswer("GearHub GET_DPI reported no DPI stages".into()));
    }
    let le = |o: usize| r[o] as u32 | (r[o + 1] as u32) << 8;
    let stages = (0..count).map(|i| Stage { x: le(DPI_X_OFFSET + i * 2), y: le(DPI_Y_OFFSET + i * 2), rgb: (r[DPI_RGB_OFFSET + i * 3] as u32) << 16 | (r[DPI_RGB_OFFSET + 1 + i * 3] as u32) << 8 | r[DPI_RGB_OFFSET + 2 + i * 3] as u32 }).collect();
    Ok((stages, active))
}

/// writeDpiTable, from scratch: the whole stage table as one `54` command (before the checksum), 64 bytes, every byte the
/// caller does not give as 0. OpenMouse's own bytes, kept for the proof; the driver does NOT use it (see [`dpi_write_frame`]).
pub fn dpi_table_command(stages: &[Stage], active: usize) -> Vec<u8> {
    let mut cmd = vec![0u8; REPORT_SIZE];
    cmd[0] = CMD_SET_DPI;
    cmd[1] = 0; // currentProfile
    cmd[2] = active as u8;
    cmd[3] = stages.len() as u8;
    for (i, s) in stages.iter().enumerate().take(MAX_DPI_STAGES) {
        cmd[DPI_X_OFFSET + i * 2] = s.x as u8;
        cmd[DPI_X_OFFSET + 1 + i * 2] = (s.x >> 8) as u8;
        cmd[DPI_Y_OFFSET + i * 2] = s.y as u8;
        cmd[DPI_Y_OFFSET + 1 + i * 2] = (s.y >> 8) as u8;
        cmd[DPI_RGB_OFFSET + i * 3] = (s.rgb >> 16) as u8;
        cmd[DPI_RGB_OFFSET + 1 + i * 3] = (s.rgb >> 8) as u8;
        cmd[DPI_RGB_OFFSET + 2 + i * 3] = s.rgb as u8;
    }
    cmd
}

/// The `54` frame the driver sends: built from the `D4` answer itself so that nothing the mouse stored is lost. OpenMouse's
/// writeDpiTable starts from a zeroed buffer, so the bytes it does not decode (4..7, every slot behind the stage count, a
/// count above 8) go out as 0; here the answer's bytes 8..63 (X, Y and colour of ALL eight slots) and 4..6 are copied as read,
/// `[0]` = `54`, `[1]` = 0 (OpenMouse's currentProfile), `[2]` / `[3]` = the active index and count as read, and ONLY the active
/// stage's X and Y are replaced (setDpi(dpi, dpi)). Refused (`BadAnswer`) when the answer is short, when the stage count is 0
/// or above 8, or when the active index is not below the count (OpenMouse would clamp it or write active = 0; nothing is sent).
pub fn dpi_write_frame(answer: &[u8], dpi: u32) -> Result<Vec<u8>> {
    if answer.len() < REPORT_SIZE {
        return Err(Error::BadAnswer(format!("GearHub GET_DPI answered {} bytes", answer.len())));
    }
    let (active, count) = (answer[2] as usize, answer[3] as usize);
    if count == 0 || count > MAX_DPI_STAGES || active >= count {
        return Err(Error::BadAnswer(format!("GearHub GET_DPI reported active stage {active} of {count} stages (1..={MAX_DPI_STAGES}) - the table is not rewritten")));
    }
    let mut cmd = answer[..REPORT_SIZE].to_vec();
    cmd[0] = CMD_SET_DPI;
    cmd[1] = 0; // currentProfile
    cmd[DPI_X_OFFSET + active * 2] = dpi as u8;
    cmd[DPI_X_OFFSET + 1 + active * 2] = (dpi >> 8) as u8;
    cmd[DPI_Y_OFFSET + active * 2] = dpi as u8;
    cmd[DPI_Y_OFFSET + 1 + active * 2] = (dpi >> 8) as u8;
    Ok(cmd)
}

/// patchOptionParam0's block: bytes 8..63 copied from `current` (the `D3` answer), `53` in byte 0, 255 / 8 stamped into
/// bytes 17 / 18, then the one override. 64 bytes, before the checksum. OpenMouse stamps 255 / 8 whatever the mouse reported;
/// here that is refused (`BadAnswer`) unless the mouse already holds exactly 255 / 8 there, so a saved field this code does not
/// understand is never changed. Also refused for an answer under 64 bytes.
pub fn option0_command(current: &[u8], offset: usize, value: u8) -> Result<Vec<u8>> {
    if current.len() < REPORT_SIZE {
        return Err(Error::BadAnswer(format!("GearHub OPTIONPARAM0 answered {} bytes", current.len())));
    }
    if current[17] != 255 || current[18] != 8 {
        return Err(Error::BadAnswer(format!("GearHub OPTIONPARAM0 holds {} / {} in bytes 17 / 18 (expected 255 / 8) - not rewritten", current[17], current[18])));
    }
    let mut block = vec![0u8; REPORT_SIZE];
    block[8..].copy_from_slice(&current[8..REPORT_SIZE]);
    block[0] = CMD_SET_OPTIONPARAM0;
    block[17] = 255;
    block[18] = 8;
    block[offset] = value;
    Ok(block)
}

/// The lift-off stop (tenths of a mm) of a byte-52 index: `levels[index] ?? levels[0]`.
pub fn lift_off_tenths(p: &Profile, index: u8) -> Option<u32> {
    p.lift_off.get(index as usize).or(p.lift_off.first()).copied()
}

// ============================================================================ the device table

/// OpenMouse's GearHubHidClient.isSupported: VID 0x3151, a product id of the family, the vendor collection 0xFFFF / 0x02.
/// `model` stays `None`: one id is shared by the M5 Pro and the R2 - the device id (`8F`) names the model.
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID {
        return None;
    }
    let (_, link) = PRODUCTS.iter().find(|p| p.0 == pid)?;
    let config = own.iter().find(|h| h.usage_page == USAGE_PAGE && h.usage == USAGE)?;
    Some(Matched { family: Family::GearHub, model: None, wireless: Some(*link == DONGLE), config: (*config).clone(), variant: *link })
}

/// The allow-list for proofs on a real mouse: a feature report (id 0, any zero padding) whose 64 bytes are exactly one of the
/// read requests of this file - the relay frames `F7` (status), `F6 05` (select the mouse) and `FC` (notice-read), or the
/// checksummed `8F`, `D3`, `D4 00`. Every write (`53`, `54`, ...) is refused.
pub fn read_request_allowed(out: &[u8]) -> bool {
    if out.len() < REPORT_LEN || out[0] != 0 || out[REPORT_LEN..].iter().any(|b| *b != 0) {
        return false;
    }
    let p = &out[1..REPORT_LEN];
    let raw = |bytes: &[u8]| p[..bytes.len()] == *bytes && p[bytes.len()..].iter().all(|b| *b == 0);
    let enc = |bytes: &[u8]| {
        let e = encode_command(bytes);
        p[..CMD_SIZE] == e[..] && p[CMD_SIZE..].iter().all(|b| *b == 0)
    };
    raw(&[DONGLE_GET_STATUS]) || raw(&[DONGLE_SELECT_TARGET, TARGET_MOUSE]) || raw(&[DONGLE_NOTICE_READ]) || enc(&[CMD_GET_USB_VERSION]) || enc(&[CMD_GET_OPTIONPARAM0]) || enc(&[CMD_GET_DPI, 0])
}

// ============================================================================ the link (relay or direct)

/// One run of commands over one link, the way GearHubHidClient exchanges them (it keeps `targetSelected` per client).
struct Link {
    dongle: bool,
    selected: bool,
    len: usize,
}

impl Link {
    fn new(m: &Matched, port: &dyn Port) -> Result<Link> {
        match m.variant {
            DONGLE | DIRECT => Ok(Link { dongle: m.variant == DONGLE, selected: false, len: (port.info().feature_len as usize).max(REPORT_LEN) }),
            v => Err(Error::UnsupportedMouse(format!("GearHub link {v}"))),
        }
    }

    /// rawSend: a feature report, no answer.
    fn send(&self, port: &mut dyn Port, payload: &[u8]) -> Result<()> {
        port.exchange(&report(payload, self.len), &HidTransfer::FeatureOut).map(|_| ())
    }

    /// rawSend, 10 ms, rawRead: the 64 data bytes of the answer.
    fn ask(&self, port: &mut dyn Port, payload: &[u8]) -> Result<Vec<u8>> {
        let a = port.exchange(&report(payload, self.len), &HidTransfer::Feature { reply_id: 0, reply_len: self.len, wait_ms: ANSWER_DELAY_MS })?;
        if a.len() < REPORT_LEN {
            return Err(Error::BadAnswer(format!("GearHub returned {} bytes, expected {REPORT_LEN}", a.len())));
        }
        Ok(a[1..REPORT_LEN].to_vec())
    }

    /// pollStatus.
    fn poll(&self, port: &mut dyn Port) -> Result<Status> {
        let r = self.ask(port, &[DONGLE_GET_STATUS])?;
        decode_status(&r).ok_or_else(|| Error::BadAnswer("GearHub receiver status too short".into()))
    }

    /// selectMouse (once per run).
    fn select(&mut self, port: &mut dyn Port) -> Result<()> {
        if self.selected {
            return Ok(());
        }
        self.send(port, &[DONGLE_SELECT_TARGET, TARGET_MOUSE])?;
        port.pause_ms(SELECT_DELAY_MS);
        self.selected = true;
        Ok(())
    }

    /// waitReady: up to 8 polls, 100 ms apart, until the receiver can send / read.
    fn wait_ready(&self, port: &mut dyn Port, to_send: bool) -> Result<bool> {
        for _ in 0..READY_ATTEMPTS {
            port.pause_ms(READY_POLL_MS);
            let s = self.poll(port)?;
            if (to_send && s.can_send) || (!to_send && s.can_read) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// command() / exchange(): one command and its verified reply.
    fn command(&mut self, port: &mut dyn Port, bytes: &[u8]) -> Result<Vec<u8>> {
        let enc = encode_command(bytes);
        let reply = if self.dongle {
            self.select(port)?;
            if !self.wait_ready(port, true)? {
                return Err(Error::BadAnswer("the 2.4 GHz receiver never became ready to send".into()));
            }
            self.send(port, &enc)?;
            if !self.wait_ready(port, false)? {
                return Err(Error::BadAnswer("the 2.4 GHz receiver never became ready to read".into()));
            }
            self.ask(port, &[DONGLE_NOTICE_READ])?
        } else {
            self.ask(port, &enc)?
        };
        if reply.iter().all(|b| *b == 0) {
            return Err(Error::BadAnswer(format!("GearHub command {:#04x} was not answered (all-zero response)", bytes[0])));
        }
        if reply[0] != bytes[0] {
            return Err(Error::BadAnswer(format!("GearHub command {:#04x} answered with id {:#04x}", bytes[0], reply[0])));
        }
        Ok(reply)
    }

    /// writeThrough: a write has no reply (relay: select, wait for canSend, send, 50 ms; direct: send, 10 ms).
    fn write(&mut self, port: &mut dyn Port, cmd: &[u8]) -> Result<()> {
        let enc = encode_command(cmd);
        if self.dongle {
            self.select(port)?;
            if !self.wait_ready(port, true)? {
                return Err(Error::BadAnswer("the 2.4 GHz receiver never became ready to send".into()));
            }
            self.send(port, &enc)?;
            port.pause_ms(RELAY_WRITE_DELAY_MS);
        } else {
            self.send(port, &enc)?;
            port.pause_ms(ANSWER_DELAY_MS as u64);
        }
        Ok(())
    }

    /// getDeviceId -> the model's profile (a device id off the SUPPORTED list is refused).
    fn identify(&mut self, port: &mut dyn Port) -> Result<&'static Profile> {
        let r = self.command(port, &[CMD_GET_USB_VERSION])?;
        let id = r[1] as u32 | (r[2] as u32) << 8 | (r[3] as u32) << 16 | (r[4] as u32) << 24;
        profile_for(id).ok_or_else(|| Error::UnsupportedMouse(format!("GearHub device id {id} is not on OpenMouse's supported list")))
    }

    fn dpi_raw(&mut self, port: &mut dyn Port) -> Result<Vec<u8>> {
        self.command(port, &[CMD_GET_DPI, 0])
    }

    fn dpi(&mut self, port: &mut dyn Port) -> Result<(Vec<Stage>, usize)> {
        decode_dpi(&self.dpi_raw(port)?)
    }

    fn option0(&mut self, port: &mut dyn Port) -> Result<Vec<u8>> {
        self.command(port, &[CMD_GET_OPTIONPARAM0])
    }
}

fn caps(p: &Profile, link: u16) -> Caps {
    Caps { dpi: Some(DpiCaps::uniform(p.min_dpi, p.max_dpi, p.dpi_step)), polling: p.polling_rates(link), lift_off: Vec::new(), battery: link == DONGLE }
}

// ============================================================================ the driver

/// The GearHub driver (`Matched.variant`: [`DONGLE`] or [`DIRECT`]).
pub struct GearHub;

impl crate::family::Driver for GearHub {
    /// readStatus: the receiver status (online + battery), the device id, the DPI table, OPTIONPARAM0 (polling, lift-off).
    /// OpenMouse asks once and throws when the receiver has no mouse linked; that is `online: false` here (no waiting loop in
    /// OpenMouse, so none here: `online_tries` is not used).
    fn read(&self, m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let mut link = Link::new(m, port)?;
        let mut r = OnMouse { model: None, online: true, wireless: Some(link.dongle), ..Default::default() };
        if link.dongle {
            let status = link.poll(port)?;
            if !status.mouse_online {
                r.online = false;
                return Ok(r);
            }
            r.battery_percent = battery_percent(&status);
        }
        let profile = link.identify(port)?;
        r.model = Some(profile.model.to_string());
        r.caps = caps(profile, m.variant);
        let (stages, active) = link.dpi(port)?;
        r.dpi = stages.get(active).or(stages.first()).map(|s| s.x);
        // OpenMouse: a failed OPTIONPARAM0 read leaves the rate and lift-off unreported rather than sinking the status
        match link.option0(port) {
            Ok(o) => {
                r.polling_hz = rate_hz(o[OPT0_REPORT_RATE]);
                // lift-off is not shown (see the header): its millimetres stand only in a comment of OpenMouse's source
            }
            Err(Error::BadAnswer(_)) => {}
            Err(e) => return Err(e),
        }
        Ok(r)
    }

    /// setDpi / setPollingRate / setLiftOffDistance as OpenMouse's client does them (read-modify-write of the whole table /
    /// block), then a read-back of what was written.
    fn write(&self, m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        // lift-off: caps.lift_off is empty and read() reports none, so a write would change a setting nobody can see; refused
        // before anything is sent
        if let Setting::LiftOff(_) = s {
            return Err(lift_off_refused());
        }
        let mut link = Link::new(m, port)?;
        if link.dongle && !link.poll(port)?.mouse_online {
            return Err(Error::MouseGone("the mouse is asleep or out of range of its receiver".into()));
        }
        let profile = link.identify(port)?;
        match s {
            Setting::Dpi(dpi) => {
                if !profile.allows_dpi(dpi) {
                    return Err(Error::range("DPI", format!("{} must be {}..{} DPI in {} DPI steps", profile.model, profile.min_dpi, profile.max_dpi, profile.dpi_step)));
                }
                let answer = link.dpi_raw(port)?;
                // setDpi(dpi, dpiY = dpi): both axes of the active stage; every other byte of the table as the mouse reported it.
                // An active index outside the stage count is refused (OpenMouse would clamp it), before anything is written.
                let frame = dpi_write_frame(&answer, dpi)?;
                let (stages, active) = decode_dpi(&answer)?;
                let before = stages[active].x;
                link.write(port, &frame)?;
                let (now, now_active) = link.dpi(port)?;
                let got = now.get(now_active).or(now.first());
                if got.map(|s| (s.x, s.y)) != Some((dpi, dpi)) {
                    return Err(Error::BadAnswer(format!("the mouse kept {:?} DPI instead of {dpi}", got.map(|s| s.x))));
                }
                Ok(Changed { now: dpi, before: Some(before) })
            }
            Setting::Polling(hz) => {
                let code = rate_code(hz).filter(|_| profile.polling_rates(m.variant).contains(&hz)).ok_or_else(|| Error::range("polling rate", format!("{hz} Hz is not one of this mouse's rates on this connection")))?;
                let current = link.option0(port)?;
                let before = rate_hz(current[OPT0_REPORT_RATE]);
                link.write(port, &option0_command(&current, OPT0_REPORT_RATE, code)?)?;
                let now = rate_hz(link.option0(port)?[OPT0_REPORT_RATE]);
                if now != Some(hz) {
                    return Err(Error::BadAnswer(format!("the mouse kept {now:?} Hz instead of {hz} Hz")));
                }
                Ok(Changed { now: hz, before })
            }
            // refused at the top of this function, before anything is sent
            Setting::LiftOff(_) => Err(lift_off_refused()),
        }
    }
}

fn lift_off_refused() -> Error {
    Error::range("lift-off distance", "GearHub lift-off is not offered: OpenMouse gives its steps as Low / Medium / High only".to_string())
}

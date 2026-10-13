//! Endgame Gear - ported from OpenMouse (`crate::family`: AGPL-3.0-or-later, @openmouse/protocol 0.40.1), DPI / polling /
//! lift-off only. Endgame Gear is three protocols under one vendor id (0x3367); OpenMouse has one client for each, ported
//! here as far as this app's transfers reach:
//!
//! | client | OpenMouse files | mice | state |
//! |---|---|---|---|
//! | OP1 config protocol | `drivers/endgame/egg-op1-hid.js` + `endgame-gear/op1.js` | OP1 8K, OP1 8K Purple Frost, OP1 8K v2, XM2 8K, XM2 8K v2 (cable), OP1w 4K, OP1w 4K v2, XM2w 4K, XM2w 4K v2 (cable + the shared 4K dongle) | ported ([`Client::Op1`]) |
//! | OT platform | `drivers/endgame/egg-ot-hid.js` + `endgame-gear/ot.js` | OP1, OP1 RGB, XM1r | ported ([`Client::Ot`]) |
//! | WE series | `drivers/endgame/egg-we-hid.js`, `egg-we-control.js` + `endgame-gear/wireless.js` | OP1we, XM2we | NOT ported - see below |
//!
//! `drivers/registry.js` order: the OP1 client and the OT client both score 100, the WE client at most ~50, so a device that
//! both could claim goes to OP1 / OT (the 4K v2 dongle, pid 0x1970, is the case that matters). Their product ids do not
//! overlap, so [`matches`] needs no scoring; `Matched.variant` is the product id and picks the client and the profile.
//!
//! **OP1 config protocol** (feature reports on one vendor interface, report ids [`REPORT_CONFIG`] 0xA0 and
//! [`REPORT_COMMAND`] 0xA1). OpenMouse counts bytes WITHOUT the report id for what it sends; its replies are normalised to
//! "report id in byte 0" (`eggNormalizeFeatureReport`, ported as [`normalize_feature`]). Windows' HidD_GetFeature answer
//! already has the id in byte 0, so OpenMouse's WebHID view of a reply is our answer minus its first byte. Indexes below are
//! OpenMouse's (`EGG_OFFSET`, report id at 0).
//! - A command is a feature report on 0xA1: `[0xA1, op, header..]`, a payload at byte 16 (OpenMouse: "wire offset 16").
//! - Read (`readConfigRaw`): command `0x12` (load), 80 ms, then GET 0xA1 and GET 0xA0 alternately (6 attempts, back-off
//!   60 -> 400 ms, 3 rounds, `0x12` sent again each round) until one answer is a valid config: status byte `[1]` = 1, CPI
//!   stage count `[30]` 1-4, polling divider `[21]` 1-255, stage-1 X CPI `[52..54]` valid, at least 131 payload bytes.
//!   A read for DISPLAY keeps that order; a read that a write builds on (`updateConfig`) only asks GET 0xA0, because OpenMouse's
//!   length test (`rawLength >= 131`) cannot work on Windows (every GET returns the collection's full feature length), so a
//!   short 0xA1 command reply could pass as the config and the write would store it. Unclear: the 0xA1 command report is
//!   padded here to the interface's longest feature length (OpenMouse sends 64 bytes for it); not checked against a real unit.
//! - Fields: polling divider `[21]` (Hz = 8000 / divider; on the 4K mice 0x80 = 1000 Hz with power saving), filter flags
//!   `[22]`, lift-off `[25]`, motion sync `[28]`, CPI stage count `[30]`, CPI stage n at `51 + 5n`: split flag, X u16 LE, Y
//!   u16 LE; glass mode `[127]`.
//! - 8K models write the WHOLE config back (`writeConfigRaw`): feature report 0xA0 `[0xA0, 0x11 (store), config[2..]]`,
//!   300 ms, then GET 0xA1 until its status byte is 1 (3 = busy: back-off 60 -> 400 ms, 8 tries). Every other byte is the
//!   one just read, copied back unchanged. The 4K mice write only the blocks that changed (`eggBlockWrites`: sensor `0x14`,
//!   power `0x15`, buttons `0x16` in two chunks), each `[0xA1, op, 0x0f, declared length, 0, 0, chunk, .., payload@16]`,
//!   150 ms, GET 0xA1 for status 1. Both then read the config back and compare.
//! - The shared 4K dongle (pid 0x1970) serves OP1w 4K, OP1w 4K v2, XM2w 4K and XM2w 4K v2, and v1 / v2 encode lift-off and
//!   polling differently. So `Matched.model` is `None` there; the mouse-info command `0x0e` names the paired mouse (its VID /
//!   PID at wire bytes 16 / 18) and [`OnMouse::model`] carries that name. Until the mouse answers it, the read shows no
//!   lift-off and no caps, and a write is refused (OpenMouse refuses it too: "never write blind").
//!
//! **OT platform** (XM1r, OP1, OP1 RGB): 64-byte feature reports, report id 0 - `[0, comtype, packet id, payload..]`. A query
//! (`queryProfileData` 51) is answered by an ACK packet (comtype 20) and then the response (comtype 21); a `setProfileData`
//! (52) packet is answered by an ACK / response. A profile is 118 bytes in six blocks (main 0 = bytes 0-49, keyboard 1 = 50-70,
//! name 2 = 71-102, cpi-Y 3 = 103-110, wireless 4 = 111, rgb 6 = 112-117). Read = the main block (CPI stages `[4..12]`, active
//! stage `[16]`, polling index `[17]`, lift-off `[23]`). A change reads all six blocks, patches the 118 bytes and sends the six
//! packets back in the vendor's order (keyboard, name, cpi-Y, wireless, rgb, main), then reads the profile again.
//!
//! DECISIONS that differ from OpenMouse's byte stream (the lead can reverse them):
//! - DPI. OpenMouse's `setDpi` overwrites EVERY CPI stage with one value (OP1: every enabled stage; OT: all four). The
//!   stages are saved settings this app cannot hand back on undo, so this port writes ONE stage, with the bytes OpenMouse's
//!   own `setCpiStage` sends: OT = the active stage (OpenMouse reads the active stage back as "the DPI"); OP1 8K / 4K = stage
//!   1 (OpenMouse: "firmware does not persist the currently selected runtime stage. Stage 1 is the stable representative" -
//!   it is also what the Mouse page shows). Unclear: which stage the mouse is on while it runs.
//! - A CPI stage that is split (separate X / Y values, or the split flag set) is not overwritten: undo remembers one number,
//!   so the DPI write is refused (error, nothing sent) and the user changes such a stage in Endgame Gear's own tool.
//! - 8000 Hz on the models whose Motion Sync cannot run at 8 kHz (OP1 8K, XM2 8K): OpenMouse switches Motion Sync off as part
//!   of the polling write. That is a second saved setting the app does not restore, so 8000 Hz is offered (and accepted) only
//!   while Motion Sync is already off on the mouse; every other rate writes exactly OpenMouse's bytes.
//! - OT status reads ask only the main block (everything the Mouse page shows is in it); OpenMouse reads all six.
//! - The OT profile is patched in the raw 118 bytes instead of decoded and re-encoded. The result is the same bytes except
//!   that OpenMouse's re-encode zeroes whatever follows the first NUL of the profile name; this port keeps it.
//!
//! NOT ported, with the reason:
//! - **WE series (OP1we 0x1961 / 0x1962, XM2we 0x1960): NEEDS TRANSFER.** The WE client sends a 16-byte feature report on
//!   one HID collection (usage page 0xFF02, report id 8) and takes the answer from an INPUT report (id 9) on a SECOND
//!   collection (usage page 0xFF01) of the same device. [`HidTransfer`] talks to one path per request; there is no
//!   "set feature on path A, wait for the input report on path B". Held back: [`matches`] gives these ids nothing.
//! - Battery: no ported model reports one (OP1 client: `batteryPercent: null`; OT client: `null` - its wireless block is only
//!   meaningful on the unmapped wireless pid 0x1910, which OpenMouse leaves unsupported). The WE client has one but is held back.
//! - Everything else the clients can do: stages count, button mapping, filters, glass mode, sensor tuning, LEDs / RGB,
//!   handedness, firmware version, **factory reset (0x13), firmware flash and firmware reset (OT)** - none is DPI / polling /
//!   lift-off, and none of those commands is in this file.
//! - "Driver knows, list doesn't": the OP1 client also takes pid 0x1970 for the old OP1we dongle (WE client) and the OT
//!   client mentions pid 0x1910 (unmapped wireless); neither is on OpenMouse's SUPPORTED list as such.
//!
//! Nothing here ran on a real mouse; it is proven against OpenMouse's bytes by `tests/openmouse_endgame.rs`. Windows gives
//! this app only the feature-report length of each collection (not the report ids a descriptor holds), so the vendor
//! interface is found by that length - see [`config_interface`].

use crate::error::{Error, Result};
use crate::family::{Caps, Changed, DpiCaps, Driver, Family, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x3367;
/// The 2.4 GHz dongle shared by every OP1w / XM2w 4K, v1 and v2 (`EGG_4K_DONGLE_PID`).
pub const DONGLE_4K: u16 = 0x1970;

// ================================================================================== device table

/// Which OpenMouse client drives a product id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Client {
    /// `EggOp1HidClient`: the OP1 config protocol (feature reports 0xA0 / 0xA1)
    Op1,
    /// `EggOtHidClient`: the OT platform (64-byte feature reports, report id 0)
    Ot,
}

/// One Endgame Gear product id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Product {
    pub pid: u16,
    pub client: Client,
    /// the model's name on OpenMouse's SUPPORTED list (`None` = a shared id: the 4K dongle)
    pub model: Option<&'static str>,
    /// the id is the 2.4 GHz link (`false` = the cable)
    pub wireless: bool,
}

const fn pr(pid: u16, client: Client, model: Option<&'static str>, wireless: bool) -> Product {
    Product { pid, client, model, wireless }
}

/// OpenMouse's SUPPORTED list (supported-devices-data.ts, status "supported"; the pids it lists plus the ones the clients'
/// catalogs name for the same model). 12 of the list's 14 models; the OP1we (0x1961, 0x1962) and XM2we (0x1960) are the
/// held-back WE series. Names are "Endgame Gear " + the list's model, as OpenMouse's own profile names.
pub const PRODUCTS: [Product; 13] = [
    // OT platform (ot.js OT_DEVICE_PROFILES)
    pr(0x1906, Client::Ot, Some("Endgame Gear OP1"), false),
    pr(0x1905, Client::Ot, Some("Endgame Gear OP1 RGB"), false),
    pr(0x1903, Client::Ot, Some("Endgame Gear XM1r"), false),
    // OP1 config protocol (op1.js EGG_DEVICE_PROFILES)
    pr(0x1964, Client::Op1, Some("Endgame Gear OP1 8K"), false),
    pr(0x1976, Client::Op1, Some("Endgame Gear OP1 8K Purple Frost"), false),
    pr(0x1978, Client::Op1, Some("Endgame Gear OP1 8K v2"), false),
    pr(0x1966, Client::Op1, Some("Endgame Gear XM2 8K"), false),
    pr(0x1980, Client::Op1, Some("Endgame Gear XM2 8K v2"), false),
    pr(0x1972, Client::Op1, Some("Endgame Gear OP1w 4K"), false),
    pr(0x1968, Client::Op1, Some("Endgame Gear XM2w 4K"), false),
    pr(0x1984, Client::Op1, Some("Endgame Gear OP1w 4K v2"), false),
    pr(0x1982, Client::Op1, Some("Endgame Gear XM2w 4K v2"), false),
    pr(DONGLE_4K, Client::Op1, None, true),
];

pub fn product(pid: u16) -> Option<&'static Product> {
    PRODUCTS.iter().find(|p| p.pid == pid)
}

/// The smallest feature-report length (Windows: FeatureReportByteLength, report id included) of the OP1 config interface:
/// `isValidConfig` wants at least 131 payload bytes.
pub const OP1_MIN_FEATURE_LEN: u16 = 132;
/// The OT interface: a 64-byte packet (report id 0 + 63).
pub const OT_MIN_FEATURE_LEN: u16 = 64;

/// The vendor interface of a client: the collection whose feature reports are long enough (OpenMouse's isSupported asks
/// the descriptor for feature report 0xA1 / report id 0; Windows only tells this app each collection's length). A vendor
/// usage page (0xFFxx) first.
pub fn config_interface<'a>(own: &[&'a HidInfo], client: Client) -> Option<&'a HidInfo> {
    let need = match client {
        Client::Op1 => OP1_MIN_FEATURE_LEN,
        Client::Ot => OT_MIN_FEATURE_LEN,
    };
    let fits = |h: &&&HidInfo| h.feature_len >= need;
    own.iter().filter(fits).find(|h| h.usage_page >= 0xFF00).or_else(|| own.iter().find(fits)).copied()
}

/// OpenMouse's isSupported (vendor id and one of the product ids, plus the collection with the client's feature report).
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID {
        return None;
    }
    let p = product(pid)?;
    let config = config_interface(own, p.client)?;
    Some(Matched { family: Family::Endgame, model: p.model, wireless: Some(p.wireless), config: config.clone(), variant: pid })
}

fn the_product(m: &Matched) -> Result<&'static Product> {
    product(m.variant).ok_or_else(|| Error::UnsupportedMouse(format!("Endgame Gear product id {:#06x}", m.variant)))
}

fn u16le(b: &[u8], at: usize) -> u32 {
    b.get(at).copied().unwrap_or(0) as u32 | ((b.get(at + 1).copied().unwrap_or(0) as u32) << 8)
}

fn put_u16le(b: &mut [u8], at: usize, v: u32) {
    b[at] = v as u8;
    b[at + 1] = (v >> 8) as u8;
}

/// "The mouse is not there / asleep / answered nothing usable": the errors a read turns into `online: false`. A mouse that
/// is gone, or a request the read-only layer refused, still fails.
fn soft<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(Error::BadAnswer(_) | Error::Os { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

// ================================================================================== OP1 config protocol

pub const REPORT_CONFIG: u8 = 0xA0;
pub const REPORT_COMMAND: u8 = 0xA1;
/// `EGG_OPERATION`: the whole-config store (8K models).
pub const OP_STORE: u8 = 0x11;
/// `EGG_OPERATION.load`: load the saved config into the config report (a read).
pub const OP_LOAD: u8 = 0x12;
/// `EGG_OPERATION.writeSensor` / `writePower` / `writeButtons`: the 4K mice's block writes.
pub const OP_WRITE_SENSOR: u8 = 0x14;
pub const OP_WRITE_POWER: u8 = 0x15;
pub const OP_WRITE_BUTTONS: u8 = 0x16;
/// `EGG_OPERATION.mouseInfo`: the mouse's own VID / PID (a read; answered through the dongle).
pub const OP_MOUSE_INFO: u8 = 0x0e;
// (EGG_OPERATION.factoryReset 0x13 and .firmware 0x02 are not used by this file.)

const STATUS_OK: u8 = 0x01;
const STATUS_BUSY: u8 = 0x03;

/// `EGG_CONFIG_SIZE` / `EGG_COMMAND_SIZE`: the totals a normalised reply is padded to.
pub const CONFIG_SIZE: usize = 1041;
pub const COMMAND_SIZE: usize = 64;

/// `EGG_OFFSET` (index 0 = the report id).
pub const OFF_POLLING_DIVIDER: usize = 21;
pub const OFF_FILTER_FLAGS: usize = 22;
pub const OFF_LOD: usize = 25;
pub const OFF_MOTION_SYNC: usize = 28;
pub const OFF_CPI_LEVELS: usize = 30;
pub const OFF_FIRST_CPI: usize = 51;
pub const OFF_GLASS: usize = 127;
/// `EGG_4K_OFFSET.powerSavePolling`: a 4K polling byte meaning 1000 Hz with wireless power saving.
pub const POWER_SAVE_POLLING: u8 = 0x80;

const LOD_V1: &[u32] = &[7, 10, 20];
const LOD_V2: &[u32] = &[7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17];
const LOD_GLASS: &[u32] = &[10, 20];
const LOD_4K_V1: &[u32] = &[10, 20];

/// `EGG_DEVICE_PROFILES` entry (op1.js), lift-off in tenths of a millimetre (`"0.7 mm"` = 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile {
    pub pid: u16,
    pub name: &'static str,
    /// `configFamily === "v2"`
    pub v2: bool,
    pub cpi_min: u32,
    pub cpi_max: u32,
    pub step_low: u32,
    pub step_high: u32,
    pub lod_normal: &'static [u32],
    pub lod_glass: Option<&'static [u32]>,
    pub motion_sync_at_8k: bool,
    pub max_polling: u32,
    pub wireless_4k: bool,
}

#[allow(clippy::too_many_arguments)]
const fn p8k(pid: u16, name: &'static str, v2: bool, cpi_max: u32, step_low: u32, lod: &'static [u32], glass: bool, ms8k: bool) -> Profile {
    Profile {
        pid,
        name,
        v2,
        cpi_min: step_low,
        cpi_max,
        step_low,
        step_high: 50,
        lod_normal: lod,
        lod_glass: if glass { Some(LOD_GLASS) } else { None },
        motion_sync_at_8k: ms8k,
        max_polling: 8000,
        wireless_4k: false,
    }
}

const fn p4k(pid: u16, name: &'static str, v2: bool) -> Profile {
    if v2 {
        Profile { pid, name, v2, cpi_min: 10, cpi_max: 30_000, step_low: 10, step_high: 50, lod_normal: LOD_V2, lod_glass: Some(LOD_GLASS), motion_sync_at_8k: true, max_polling: 4000, wireless_4k: true }
    } else {
        Profile { pid, name, v2, cpi_min: 50, cpi_max: 26_000, step_low: 50, step_high: 50, lod_normal: LOD_4K_V1, lod_glass: None, motion_sync_at_8k: true, max_polling: 4000, wireless_4k: true }
    }
}

/// `EGG_DEVICE_PROFILES`, the ten ids this file serves (the dongle's entry is the neutral v2 profile OpenMouse starts from).
pub const PROFILES: [Profile; 10] = [
    p8k(0x1964, "Endgame Gear OP1 8K", false, 26_000, 50, LOD_V1, false, false),
    p8k(0x1966, "Endgame Gear XM2 8K", false, 26_000, 50, LOD_V1, false, false),
    p8k(0x1976, "Endgame Gear OP1 8K Purple Frost", false, 30_000, 50, LOD_V1, true, true),
    p8k(0x1978, "Endgame Gear OP1 8K v2", true, 30_000, 10, LOD_V2, true, true),
    p8k(0x1980, "Endgame Gear XM2 8K v2", true, 30_000, 10, LOD_V2, true, true),
    p4k(0x1972, "Endgame Gear OP1w 4K", false),
    p4k(0x1968, "Endgame Gear XM2w 4K", false),
    p4k(0x1984, "Endgame Gear OP1w 4K v2", true),
    p4k(0x1982, "Endgame Gear XM2w 4K v2", true),
    p4k(DONGLE_4K, "Endgame Gear OP1w/XM2w 4K", true),
];

pub fn op1_profile(pid: u16) -> Option<Profile> {
    PROFILES.iter().copied().find(|p| p.pid == pid)
}

impl Profile {
    /// `is4kV2`
    pub fn is_4k_v2(&self) -> bool {
        self.wireless_4k && self.v2
    }
    /// The shared dongle's neutral profile: v1 or v2 not known yet.
    pub fn is_neutral(&self) -> bool {
        self.pid == DONGLE_4K
    }
    /// `lodBase`: 4K v1 lift-off (and the 4K v2's in glass mode) is stored as whole millimetres, option 0 = wire value 1.
    pub fn lod_base(&self, glass: bool) -> u8 {
        (self.wireless_4k && (glass || !self.v2)) as u8
    }
    /// `eggLodOptions` in tenths of a mm.
    pub fn lod_options(&self, glass: bool) -> &'static [u32] {
        match (glass, self.lod_glass) {
            (true, Some(g)) => g,
            _ => self.lod_normal,
        }
    }
    /// `eggCpiStep`
    pub fn cpi_step(&self, cpi: u32) -> u32 {
        if self.v2 && cpi <= 10_000 {
            self.step_low
        } else {
            self.step_high
        }
    }
    /// `eggClampCpi`
    pub fn clamp_cpi(&self, v: u32) -> u32 {
        let c = v.clamp(self.cpi_min, self.cpi_max);
        let step = self.cpi_step(c);
        ((c + step / 2) / step * step).clamp(self.cpi_min, self.cpi_max)
    }
    /// `eggIsValidCpi`
    pub fn is_valid_cpi(&self, v: u32) -> bool {
        v == self.clamp_cpi(v)
    }
    /// `eggDpiOptions` as bands.
    pub fn dpi_caps(&self) -> DpiCaps {
        if self.v2 {
            DpiCaps { bands: vec![(self.cpi_min, 10_000, self.step_low), (10_050, self.cpi_max, self.step_high)] }
        } else {
            DpiCaps::uniform(self.cpi_min, self.cpi_max, self.step_high)
        }
    }
    /// `supportedPollingRates()`: the 4K vendor enum values, or every divider rate up to the model's maximum.
    pub fn polling_rates(&self) -> Vec<u32> {
        if self.wireless_4k {
            return if self.is_4k_v2() { vec![125, 1000, 2000, 4000] } else { vec![1000, 2000, 4000] };
        }
        [125, 250, 500, 1000, 2000, 4000, 8000].into_iter().filter(|r| *r <= self.max_polling).collect()
    }
    /// `decodePollingRate`: a divider byte -> Hz (`None` = 0, or a divider that is no whole rate).
    pub fn decode_polling(&self, divider: u8) -> Option<u32> {
        if divider == 0 {
            None
        } else if self.wireless_4k && divider == POWER_SAVE_POLLING {
            Some(1000)
        } else if 8000 % divider as u32 == 0 {
            Some(8000 / divider as u32)
        } else {
            None
        }
    }
}

/// Is the config in glass mode (`profile.lodGlass !== null && config[127] !== 0`)?
pub fn glass_mode(p: &Profile, config: &[u8]) -> bool {
    p.lod_glass.is_some() && config.get(OFF_GLASS).copied().unwrap_or(0) != 0
}

/// The lift-off the config holds, in tenths of a mm (`lodIndex = config[25] - lodBase`; `None` = outside the option list).
pub fn config_lift_off(p: &Profile, config: &[u8]) -> Option<u32> {
    let glass = glass_mode(p, config);
    let idx = (config.get(OFF_LOD).copied()? as i32) - p.lod_base(glass) as i32;
    usize::try_from(idx).ok().and_then(|i| p.lod_options(glass).get(i).copied())
}

/// The rates OpenMouse can set on this config: none in glass mode (`setPollingRate` throws); 8000 Hz on the models whose
/// Motion Sync cannot run at 8 kHz only while Motion Sync is already off (see the DECISIONS above).
pub fn config_polling_rates(p: &Profile, config: &[u8]) -> Vec<u32> {
    if glass_mode(p, config) {
        return Vec::new();
    }
    let motion = config.get(OFF_MOTION_SYNC).copied().unwrap_or(0) != 0;
    p.polling_rates().into_iter().filter(|r| *r != 8000 || p.motion_sync_at_8k || !motion).collect()
}

/// The lift-off values the Mouse page offers: OpenMouse's generic levels Low / Medium / High name 0.7 / 1.0 / 2.0 mm
/// (`setLiftOffDistance`), cut to what the current mode lists. Any option of the mode can still be set.
pub fn lift_off_chips(p: &Profile, glass: bool) -> Vec<u32> {
    p.lod_options(glass).iter().copied().filter(|t| matches!(*t, 7 | 10 | 20)).collect()
}

/// What the Mouse page offers for a read config.
pub fn op1_caps(p: &Profile, config: &[u8]) -> Caps {
    let glass = glass_mode(p, config);
    Caps { dpi: Some(p.dpi_caps()), polling: config_polling_rates(p, config), lift_off: lift_off_chips(p, glass), battery: false }
}

/// `eggNormalizeFeatureReport`: a feature reply as OpenMouse's WebHID view hands it over (`raw`: no report id) -> the
/// buffer with the report id in byte 0 and the status / payload behind it. `payload_len` = the report's payload length
/// (this port: feature length - 1).
pub fn normalize_feature(raw: &[u8], report_id: u8, expected_total: usize, payload_len: usize) -> Vec<u8> {
    // Some OP1 descriptors put the report's wire header (0 / A0 / A1) first and the status behind it; 1 OK, 3 busy,
    // 7 rejected, 8 mouse unreachable (RF sleep) are all framing markers (OpenMouse issue #107).
    let wire_header = raw.len() == payload_len && raw.len() >= 2 && matches!(raw[0], 0 | REPORT_CONFIG | REPORT_COMMAND) && matches!(raw[1], 1 | 3 | 7 | 8);
    if wire_header {
        let mut r = vec![0u8; expected_total.max(raw.len())];
        r[..raw.len()].copy_from_slice(raw);
        r[0] = report_id;
        return r;
    }
    let includes_id = if payload_len > 0 && raw.len() == payload_len {
        false
    } else if payload_len > 0 && raw.len() == payload_len + 1 && raw.first() == Some(&report_id) {
        true
    } else {
        raw.first() == Some(&report_id)
    };
    let mut r = vec![0u8; expected_total.max(raw.len() + if includes_id { 0 } else { 1 })];
    if includes_id {
        r[..raw.len()].copy_from_slice(raw);
    } else {
        r[0] = report_id;
        r[1..1 + raw.len()].copy_from_slice(raw);
    }
    r
}

/// A command report: `[0xA1, op, header.., 0.., payload at byte 16]`, zero-padded to the collection's feature length
/// (`sendCommand`: OpenMouse's `command.set([operation, ...header])`, payload at 15 behind the report id).
pub fn op1_command_report(op: u8, header: &[u8], payload: &[u8], len: usize) -> Vec<u8> {
    let mut v = vec![0u8; len.max(16 + payload.len() + 1).max(2 + header.len())];
    v[0] = REPORT_COMMAND;
    v[1] = op;
    v[2..2 + header.len()].copy_from_slice(header);
    v[16..16 + payload.len()].copy_from_slice(payload);
    v
}

/// `writeConfigRaw`'s report: `[0xA0, 0x11 (store), config[2..]]` - the read config copied back, byte for byte, to the
/// collection's feature length.
pub fn op1_store_report(config: &[u8], len: usize) -> Vec<u8> {
    let mut v: Vec<u8> = (0..len).map(|i| config.get(i).copied().unwrap_or(0)).collect();
    v[0] = REPORT_CONFIG;
    v[1] = OP_STORE;
    v
}

/// `isValidConfig`: a plausible config for this profile (`raw_len` = the payload length the reply had).
pub fn is_valid_config(p: &Profile, config: &[u8], raw_len: usize) -> bool {
    let (stages, divider) = (config.get(OFF_CPI_LEVELS).copied().unwrap_or(0), config.get(OFF_POLLING_DIVIDER).copied().unwrap_or(0));
    raw_len >= 131 && config.get(1) == Some(&STATUS_OK) && (1..=4).contains(&stages) && divider >= 1 && p.is_valid_cpi(u16le(config, OFF_FIRST_CPI + 1))
}

/// One of the 4K mice's block writes (`eggBlockWrites`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockWrite {
    pub command: u8,
    pub declared_length: u8,
    pub chunk: u8,
    pub payload: Vec<u8>,
}

/// `eggBlockWrites(config, hasGlassMode)`: the sensor, power and two button blocks built from a config (`at(o)` =
/// `config[16 + o]`: the vendor blob's offsets sit 16 bytes into the buffer).
pub fn block_writes(config: &[u8], has_glass: bool) -> Vec<BlockWrite> {
    let at = |o: usize| config.get(16 + o).copied().unwrap_or(0);
    let mut sensor = vec![0u8; 28];
    sensor[..8].copy_from_slice(&[at(0x07), at(0x08), at(0x09), at(0x0a), at(0x0b), at(0x01), at(0x0e), at(0x0d)]);
    for (i, b) in sensor[8..].iter_mut().enumerate() {
        *b = at(0x23 + i);
    }
    let mut power = vec![at(0x0c), at(0x05), at(0x06), at(0x04)];
    power.extend((0..5).map(|b| at(0x3d + b * 7)));
    power.push(at(0x03));
    if has_glass {
        power.push(at(0x6f));
    }
    let buttons: Vec<u8> = (0..56).map(|i| at(0x37 + i)).collect();
    vec![
        BlockWrite { command: OP_WRITE_SENSOR, declared_length: 28, chunk: 0, payload: sensor },
        BlockWrite { command: OP_WRITE_POWER, declared_length: 10, chunk: 0, payload: power },
        BlockWrite { command: OP_WRITE_BUTTONS, declared_length: 28, chunk: 1, payload: buttons[..28].to_vec() },
        BlockWrite { command: OP_WRITE_BUTTONS, declared_length: 28, chunk: 2, payload: buttons[28..].to_vec() },
    ]
}

/// The report of one block write: header after the op byte = target 0x0F (mouse), declared length, two zero bytes, chunk.
pub fn block_write_report(w: &BlockWrite, len: usize) -> Vec<u8> {
    op1_command_report(w.command, &[0x0f, w.declared_length, 0, 0, w.chunk], &w.payload, len)
}

/// Patches the config for each setting the way OpenMouse's setters do. `Err` = OpenMouse throws before writing.
pub mod patch {
    use super::*;

    /// `setCpiStage(0, d, d)`: stage 1 = split flag 0, X and Y = d. A stage that is split (flag set, or X != Y) is refused:
    /// undo remembers one number, so it could not give the second axis (or the flag) back.
    pub fn dpi_stage1(config: &mut [u8], dpi: u32) -> Result<()> {
        let (x, y) = (u16le(config, OFF_FIRST_CPI + 1), u16le(config, OFF_FIRST_CPI + 3));
        if config[OFF_FIRST_CPI] != 0 || x != y {
            return Err(Error::range("DPI", format!("CPI stage 1 is set to separate X {x} / Y {y} values - change it in Endgame Gear's own tool, this app only sets one value for both axes")));
        }
        config[OFF_FIRST_CPI] = 0;
        put_u16le(config, OFF_FIRST_CPI + 1, dpi);
        put_u16le(config, OFF_FIRST_CPI + 3, dpi);
        Ok(())
    }

    /// `setPollingRate`: the divider (8000 / rate). Glass mode: firmware controls the rate.
    pub fn polling(p: &Profile, config: &mut [u8], rate: u32) -> Result<()> {
        if glass_mode(p, config) {
            return Err(Error::range("polling rate", "Polling rate is controlled by firmware while Glass Mode is active."));
        }
        if rate == 8000 && !p.motion_sync_at_8k && config[OFF_MOTION_SYNC] != 0 {
            return Err(Error::range("polling rate", "8000 Hz on this mouse needs Motion Sync off - turn it off in Endgame Gear's own tool first"));
        }
        config[OFF_POLLING_DIVIDER] = (8000 / rate) as u8;
        Ok(())
    }

    /// `setEggLodIndex(index)`: the option `tenths` of the config's current mode.
    pub fn lift_off(p: &Profile, config: &mut [u8], tenths: u32) -> Result<()> {
        let glass = glass_mode(p, config);
        let idx = p.lod_options(glass).iter().position(|t| *t == tenths).ok_or_else(|| Error::range("lift-off distance", format!("{} does not expose {} in its current sensor mode", p.name, crate::family::lift_off_label(tenths))))?;
        config[OFF_LOD] = idx as u8 + p.lod_base(glass);
        Ok(())
    }
}

/// How hard a read tries: OpenMouse's `readConfigRaw` is 3 rounds of 6 attempts; the read-only proof (`online_tries` 1) asks once.
#[derive(Clone, Copy)]
struct Budget {
    rounds: u32,
    attempts: u32,
}

const FULL: Budget = Budget { rounds: 3, attempts: 6 };
const ONCE: Budget = Budget { rounds: 1, attempts: 1 };

fn feat_len(port: &dyn Port) -> usize {
    port.info().feature_len as usize
}

/// GET one feature report without sending anything (`out` = the request it answers) -> OpenMouse's normalised bytes and the
/// payload length the reply had.
fn op1_get(port: &mut dyn Port, sent: &[u8], reply_id: u8, expected_total: usize) -> Result<(Vec<u8>, usize)> {
    let len = feat_len(port);
    let a = port.exchange(sent, &HidTransfer::FeatureIn { reply_id, reply_len: len, wait_ms: 0 })?;
    let raw = a.get(1..).unwrap_or(&[]);
    Ok((normalize_feature(raw, reply_id, expected_total, len.saturating_sub(1)), raw.len()))
}

fn set_feature(port: &mut dyn Port, out: &[u8]) -> Result<()> {
    port.exchange(out, &HidTransfer::FeatureOut).map(|_| ())
}

/// `readConfigRaw`: load, then fetch until a valid config comes back. `None` = none did (asleep / not answering).
///
/// `for_write` = the config is going to be stored back (`updateConfig`). OpenMouse tells the short 0xA1 command report from
/// the config by its raw length (`rawLength >= 131`); Windows' HidD_GetFeature gives no byte count, every reply is as long
/// as the collection's feature report, so that test cannot fire here and a 0xA1 answer whose first bytes merely look like a
/// config would pass as the config. A write stores EVERY byte it holds, so a write only trusts a config that came from
/// GET 0xA0 (the config report itself); only a read for display also tries 0xA1 first, as OpenMouse does.
fn read_config(port: &mut dyn Port, p: &Profile, b: Budget, for_write: bool) -> Result<Option<Vec<u8>>> {
    let ids: &[u8] = if for_write { &[REPORT_CONFIG] } else { &[REPORT_COMMAND, REPORT_CONFIG] };
    let load = op1_command_report(OP_LOAD, &[], &[], feat_len(port));
    for _ in 0..b.rounds {
        set_feature(port, &load)?;
        port.pause_ms(80);
        let mut backoff = 60;
        for attempt in 0..b.attempts {
            for &id in ids {
                match op1_get(port, &load, id, CONFIG_SIZE) {
                    Ok((mut bytes, raw_len)) => {
                        if is_valid_config(p, &bytes, raw_len) {
                            bytes[0] = REPORT_CONFIG;
                            return Ok(Some(bytes));
                        }
                    }
                    Err(e @ Error::MouseGone(_)) => return Err(e),
                    Err(_) => {}
                }
            }
            if attempt + 1 < b.attempts {
                port.pause_ms(backoff);
                backoff = (backoff * 2).min(400);
            }
        }
    }
    Ok(None)
}

/// `pollCommandOk`: GET 0xA1 until its status byte is OK (busy: wait and ask again; anything else after the third try: no).
fn poll_command_ok(port: &mut dyn Port, sent: &[u8], attempts: u32) -> Result<bool> {
    let mut backoff = 60;
    for attempt in 0..attempts {
        let (bytes, _) = op1_get(port, sent, REPORT_COMMAND, COMMAND_SIZE)?;
        if bytes[1] == STATUS_OK {
            return Ok(true);
        }
        if bytes[1] != STATUS_BUSY && attempt > 1 {
            return Ok(false);
        }
        port.pause_ms(backoff);
        backoff = (backoff * 2).min(400);
    }
    Ok(false)
}

/// `identifyPairedMouse`: the mouse-info command, and the paired mouse's profile from the PID in its answer. `None` = the
/// mouse did not say (asleep / still linking). A PID that is not a 4K mouse leaves the neutral profile (OpenMouse's too).
fn identify(port: &mut dyn Port) -> Result<Option<Profile>> {
    let req = op1_command_report(OP_MOUSE_INFO, &[], &[], feat_len(port));
    set_feature(port, &req)?;
    port.pause_ms(150);
    for attempt in 0..3 {
        let (reply, _) = op1_get(port, &req, REPORT_COMMAND, COMMAND_SIZE)?;
        if reply[1] == STATUS_OK && u16le(&reply, 16) == VENDOR_ID as u32 {
            let paired = op1_profile(u16le(&reply, 18) as u16).filter(|p| p.wireless_4k);
            return Ok(Some(paired.unwrap_or_else(|| op1_profile(DONGLE_4K).expect("the dongle's profile exists"))));
        }
        // an OK without our VID is the previous command's held reply: the dongle has not relayed the mouse yet
        if reply[1] != STATUS_BUSY && reply[1] != STATUS_OK {
            return Ok(None);
        }
        port.pause_ms(200 * (attempt + 1) as u64);
    }
    Ok(None)
}

/// `writeConfigRaw` (8K models): the store report (4 tries), 300 ms, then the acknowledgement.
fn write_config_raw(port: &mut dyn Port, config: &[u8]) -> Result<()> {
    let out = op1_store_report(config, feat_len(port));
    let mut last = None;
    for _ in 0..4 {
        match set_feature(port, &out) {
            Ok(()) => {
                last = None;
                break;
            }
            Err(e @ Error::MouseGone(_)) => return Err(e),
            Err(e) => {
                last = Some(e);
                port.pause_ms(50);
            }
        }
    }
    if let Some(e) = last {
        return Err(e);
    }
    port.pause_ms(300);
    if !poll_command_ok(port, &out, 8)? {
        return Err(Error::BadAnswer("the Endgame Gear mouse did not acknowledge the configuration write".into()));
    }
    Ok(())
}

/// `writeConfigBlocks` (4K mice): only the blocks that changed, each acknowledged.
fn write_config_blocks(port: &mut dyn Port, p: &Profile, before: &[u8], after: &[u8]) -> Result<()> {
    let glass = p.lod_glass.is_some();
    let (old, new) = (block_writes(before, glass), block_writes(after, glass));
    let changed: Vec<u8> = new.iter().zip(&old).filter(|(n, o)| n.payload.iter().enumerate().any(|(i, b)| *b != o.payload.get(i).copied().unwrap_or(0))).map(|(n, _)| n.command).collect();
    for w in new.iter().filter(|w| changed.contains(&w.command)) {
        let out = block_write_report(w, feat_len(port));
        set_feature(port, &out)?;
        port.pause_ms(150);
        if !poll_command_ok(port, &out, 8)? {
            return Err(Error::BadAnswer("the Endgame Gear mouse did not acknowledge the configuration write".into()));
        }
    }
    Ok(())
}

/// What a change returns: the config read before it (undo's "before") and the one read back after it.
struct Updated {
    profile: Profile,
    before: Vec<u8>,
    confirmed: Vec<u8>,
}

/// `updateConfig`: (the dongle: name the paired mouse first) read, change, write, read back.
fn op1_update(port: &mut dyn Port, pid: u16, change: impl FnOnce(&Profile, &mut Vec<u8>) -> Result<()>) -> Result<Updated> {
    let mut profile = op1_profile(pid).ok_or_else(|| Error::UnsupportedMouse(format!("Endgame Gear product id {pid:#06x}")))?;
    if profile.is_neutral() {
        profile = identify(port)?.unwrap_or(profile);
        if profile.is_neutral() {
            return Err(Error::BadAnswer("the mouse behind the dongle has not identified itself yet - move it to wake it, then try again".into()));
        }
    }
    let unanswered = || Error::BadAnswer("the Endgame Gear mouse did not return a valid configuration".into());
    let mut config = read_config(port, &profile, FULL, true)?.ok_or_else(unanswered)?;
    let before = config.clone();
    change(&profile, &mut config)?;
    if profile.wireless_4k {
        write_config_blocks(port, &profile, &before, &config)?;
    } else {
        write_config_raw(port, &config)?;
    }
    let confirmed = read_config(port, &profile, FULL, true)?.ok_or_else(unanswered)?;
    Ok(Updated { profile, before, confirmed })
}

fn read_op1(m: &Matched, port: &mut dyn Port, opt: ReadOptions) -> Result<OnMouse> {
    let mut profile = op1_profile(m.variant).ok_or_else(|| Error::UnsupportedMouse(format!("Endgame Gear product id {:#06x}", m.variant)))?;
    let mut r = OnMouse { wireless: m.wireless, ..Default::default() };
    let budget = if opt.online_tries <= 1 { ONCE } else { FULL };
    let Some(config) = soft(read_config(port, &profile, budget, false))?.flatten() else {
        // asleep / silent: what the id alone says (the dongle says nothing about the mouse)
        if !profile.is_neutral() {
            // (Motion Sync unknown: 8000 Hz only where it cannot be in the way)
            let polling = profile.polling_rates().into_iter().filter(|r| *r != 8000 || profile.motion_sync_at_8k).collect();
            r.caps = Caps { dpi: Some(profile.dpi_caps()), polling, lift_off: lift_off_chips(&profile, false), battery: false };
        }
        return Ok(r);
    };
    r.online = true;
    if profile.is_neutral() {
        // retried on every read until the mouse answers: over the dongle it sleeps quickly
        if let Some(p) = soft(identify(port))?.flatten().filter(|p| !p.is_neutral()) {
            profile = p;
            r.model = Some(p.name.to_string());
        }
    }
    r.polling_hz = profile.decode_polling(config[OFF_POLLING_DIVIDER]);
    // the DPI readout is stage 1 (OpenMouse: the stable representative); both axes must be valid, as OpenMouse requires
    let (x, y) = (u16le(&config, OFF_FIRST_CPI + 1), u16le(&config, OFF_FIRST_CPI + 3));
    if !profile.is_valid_cpi(x) || !profile.is_valid_cpi(y) {
        return Err(Error::BadAnswer(format!("the mouse reported an unsupported X {x} / Y {y} CPI value")));
    }
    r.dpi = Some(x);
    if !profile.is_neutral() {
        r.lift_off = config_lift_off(&profile, &config);
        r.caps = op1_caps(&profile, &config);
    }
    // (a neutral profile = v1 or v2 not known: no lift-off, no caps, no writes)
    Ok(r)
}

fn write_op1(m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
    let pid = m.variant;
    match s {
        Setting::Dpi(d) => {
            let u = op1_update(port, pid, |p, c| {
                if !p.is_valid_cpi(d) {
                    return Err(Error::range("DPI", format!("{} CPI must be {} to {} using the device's supported steps", p.name, p.cpi_min, p.cpi_max)));
                }
                patch::dpi_stage1(c, d)
            })?;
            let (x, y) = (u16le(&u.confirmed, OFF_FIRST_CPI + 1), u16le(&u.confirmed, OFF_FIRST_CPI + 3));
            if x != d || y != d {
                return Err(Error::BadAnswer(format!("the mouse kept CPI stage 1 at {x}/{y} instead of {d} CPI")));
            }
            Ok(Changed { now: d, before: Some(u16le(&u.before, OFF_FIRST_CPI + 1)) })
        }
        Setting::Polling(hz) => {
            let u = op1_update(port, pid, |p, c| {
                if !p.polling_rates().contains(&hz) {
                    return Err(Error::range("polling rate", format!("{hz} Hz is not a polling rate of {}", p.name)));
                }
                patch::polling(p, c, hz)
            })?;
            let now = u.profile.decode_polling(u.confirmed[OFF_POLLING_DIVIDER]);
            if now != Some(hz) {
                return Err(Error::BadAnswer(format!("the mouse kept {now:?} Hz instead of {hz} Hz")));
            }
            // 4K "1000 Hz with power saving" (0x80) shows as 1000 Hz but is not what writing 1000 Hz stores (divider 8): no
            // undo value then, so undo cannot silently switch power saving off
            let was = u.before[OFF_POLLING_DIVIDER];
            let before = if u.profile.wireless_4k && was == POWER_SAVE_POLLING { None } else { u.profile.decode_polling(was) };
            Ok(Changed { now: hz, before })
        }
        Setting::LiftOff(t) => {
            let u = op1_update(port, pid, |p, c| patch::lift_off(p, c, t))?;
            let now = config_lift_off(&u.profile, &u.confirmed);
            if now != Some(t) {
                return Err(Error::BadAnswer("the mouse did not confirm the requested lift-off distance".into()));
            }
            Ok(Changed { now: t, before: config_lift_off(&u.profile, &u.before) })
        }
    }
}

// ================================================================================== OT platform

pub const OT_PACKET_SIZE: usize = 64;
pub const OT_PAYLOAD_INDEX: usize = 3;
pub const OT_EFFECT_SIZE: usize = 118;
/// `OT_COMTYPE`: queryProfileData / setProfileData / ack / response. (The same table holds firmware download (8 / 9), reset
/// (18), clearFlash, writeFlash, resetMouse ... - none is used here.)
pub const OT_QUERY_PROFILE_DATA: u8 = 51;
pub const OT_SET_PROFILE_DATA: u8 = 52;
pub const OT_ACK: u8 = 20;
pub const OT_RESPONSE: u8 = 21;
const OT_WRITE_RETRIES: u32 = 10;
const OT_READ_RETRIES: u32 = 10;
/// `OT_PROFILE_BLOCK`
pub const BLOCK_MAIN: u8 = 0;
pub const BLOCK_KEYBOARD: u8 = 1;
pub const BLOCK_NAME: u8 = 2;
pub const BLOCK_CPI_Y: u8 = 3;
pub const BLOCK_WIRELESS: u8 = 4;
pub const BLOCK_RGB: u8 = 6;
/// `OT_EFFECT_OFFSETS` (118-byte effect, no report id)
pub const OT_CPI: usize = 4;
pub const OT_CPI_INDEX: usize = 16;
pub const OT_POLLRATE_INDEX: usize = 17;
pub const OT_POLL_RATES: usize = 18;
pub const OT_DCU_THRESHOLD: usize = 23;
pub const OT_CPI_Y: usize = 103;
/// the main block's length (`OT_EFFECT_SIZE - 68`)
pub const OT_MAIN_LEN: usize = 50;

/// `otProfileBlocks()` in the vendor's SEND order (keyboard, name, cpi-Y, wireless, rgb, main): (block, offset, length).
pub const OT_BLOCKS: [(u8, usize, usize); 6] = [
    (BLOCK_KEYBOARD, OT_MAIN_LEN, 21),
    (BLOCK_NAME, OT_MAIN_LEN + 21, 32),
    (BLOCK_CPI_Y, OT_MAIN_LEN + 53, 8),
    (BLOCK_WIRELESS, OT_MAIN_LEN + 61, 1),
    (BLOCK_RGB, OT_MAIN_LEN + 62, 6),
    (BLOCK_MAIN, 0, OT_MAIN_LEN),
];

/// `OT_POLL_RATE_HZ`: poll-rate index -> Hz (0 = 1000 ... 3 = 125).
pub fn ot_poll_hz(index: u8) -> Option<u32> {
    [1000, 500, 250, 125].get(index as usize).copied()
}

/// `OT_POLLING_RATES`
pub const OT_POLLING_RATES: [u32; 4] = [125, 250, 500, 1000];

/// `otCpiStep`: 50, or 100 above 10 000 CPI.
pub fn ot_cpi_step(cpi: u32) -> u32 {
    if cpi > 10_000 {
        100
    } else {
        50
    }
}

/// `otIsValidCpi`
pub fn ot_is_valid_cpi(max: u32, v: u32) -> bool {
    let c = v.clamp(50, max);
    let step = ot_cpi_step(c);
    v == ((c + step / 2) / step * step).clamp(50, max)
}

fn ot_max_cpi(pid: u16) -> u32 {
    if pid == 0x1903 {
        19_000
    } else {
        26_000
    }
}

pub fn ot_caps(pid: u16) -> Caps {
    let max = ot_max_cpi(pid);
    Caps { dpi: Some(DpiCaps { bands: vec![(50, 10_000, 50), (10_100, max, 100)] }), polling: OT_POLLING_RATES.to_vec(), lift_off: vec![10, 20], battery: false }
}

/// `otEncodeQueryPacket(block, profile)`: `[0, 51, 0, block, profile, 0..]`.
pub fn ot_query_packet(block: u8, profile: u8) -> Vec<u8> {
    let mut v = vec![0u8; OT_PACKET_SIZE];
    v[1] = OT_QUERY_PROFILE_DATA;
    v[3] = block;
    v[4] = profile;
    v
}

/// `otEncodeProfilePacket(effect, block)`: `[0, 52, 0, block, ..]`; the main block carries effect[0..50) from byte 3, the
/// others effect[0..4) from byte 3 (byte 3 = the block id) and their slice from byte 7.
pub fn ot_profile_packet(effect: &[u8], block: u8) -> Vec<u8> {
    let mut v = vec![0u8; OT_PACKET_SIZE];
    v[1] = OT_SET_PROFILE_DATA;
    v[OT_PAYLOAD_INDEX] = block;
    if block == BLOCK_MAIN {
        v[OT_PAYLOAD_INDEX..OT_PAYLOAD_INDEX + OT_MAIN_LEN].copy_from_slice(&effect[..OT_MAIN_LEN]);
        return v;
    }
    let (_, offset, len) = OT_BLOCKS.iter().copied().find(|b| b.0 == block).expect("a known block");
    v[OT_PAYLOAD_INDEX..OT_PAYLOAD_INDEX + 4].copy_from_slice(&effect[..4]);
    v[OT_PAYLOAD_INDEX + 4..OT_PAYLOAD_INDEX + 4 + len].copy_from_slice(&effect[offset..offset + len]);
    v[OT_PAYLOAD_INDEX] = block;
    v
}

/// `otReassembleEffect`: the six replies (each the packet from byte 3 on) -> the 118-byte effect.
pub fn ot_reassemble(replies: &[(u8, Vec<u8>)]) -> Option<Vec<u8>> {
    let mut out = vec![0u8; OT_EFFECT_SIZE];
    for (block, offset, len) in OT_BLOCKS {
        let reply = &replies.iter().find(|r| r.0 == block)?.1;
        if block == BLOCK_MAIN {
            out[..OT_MAIN_LEN].copy_from_slice(reply.get(..OT_MAIN_LEN)?);
        } else {
            out[offset..offset + len].copy_from_slice(reply.get(4..4 + len)?);
        }
    }
    Some(out)
}

/// `otNormalizeFeatureReport`: a feature reply (`raw`: OpenMouse's WebHID view, no report id) -> the 64-byte packet.
pub fn ot_normalize(raw: &[u8]) -> Vec<u8> {
    if raw.len() == OT_PACKET_SIZE && raw[0] == 0 {
        return raw.to_vec();
    }
    let mut out = vec![0u8; OT_PACKET_SIZE];
    if raw.len() == OT_PACKET_SIZE - 1 {
        out[1..].copy_from_slice(raw);
    } else if raw.len() == OT_PACKET_SIZE + 1 && raw[0] == 0 {
        out.copy_from_slice(&raw[1..]);
    } else {
        let n = raw.len().min(OT_PACKET_SIZE);
        out[..n].copy_from_slice(&raw[..n]);
    }
    out
}

/// The packet as Windows sends it: report id 0 in byte 0, zero-padded to the collection's feature length.
fn ot_report(packet: &[u8], len: usize) -> Vec<u8> {
    let mut v = packet.to_vec();
    v.resize(len.max(OT_PACKET_SIZE), 0);
    v
}

/// One GET of the OT packet. The first one rides on the send (SetFeature, then GetFeature); the rest only GET.
fn ot_reply(port: &mut dyn Port, out: &[u8], first: bool) -> Result<Vec<u8>> {
    let reply_len = feat_len(port).max(OT_PACKET_SIZE);
    let how = if first { HidTransfer::Feature { reply_id: 0, reply_len, wait_ms: 0 } } else { HidTransfer::FeatureIn { reply_id: 0, reply_len, wait_ms: 0 } };
    let a = port.exchange(out, &how)?;
    Ok(ot_normalize(a.get(1..).unwrap_or(&[])))
}

/// `exchange(packet, true)`: send, read until the ACK and the response behind it. `None` = nothing came in `writes` sends.
fn ot_query(port: &mut dyn Port, packet: &[u8], writes: u32) -> Result<Option<Vec<u8>>> {
    let out = ot_report(packet, feat_len(port));
    for _ in 0..writes {
        let mut acked = false;
        for read in 0..OT_READ_RETRIES {
            let reply = ot_reply(port, &out, read == 0)?;
            if reply[1] == 0 {
                continue;
            }
            if reply[1] == OT_ACK {
                acked = true;
                continue;
            }
            if acked {
                return Ok(Some(reply));
            }
        }
    }
    Ok(None)
}

/// `sendPacket` for a `setProfileData` packet (no response expected): send until an ACK or response comes. The read budget
/// (10) is shared by all sends, as OpenMouse's `readCounter` is.
fn ot_set(port: &mut dyn Port, packet: &[u8]) -> Result<()> {
    let out = ot_report(packet, feat_len(port));
    let mut reads = 0;
    for _ in 0..OT_WRITE_RETRIES {
        if reads >= OT_READ_RETRIES {
            set_feature(port, &out)?;
            continue;
        }
        let mut first = true;
        while reads < OT_READ_RETRIES {
            reads += 1;
            let reply = ot_reply(port, &out, first)?;
            first = false;
            if reply[1] == OT_ACK || reply[1] == OT_RESPONSE {
                return Ok(());
            }
        }
    }
    Err(Error::BadAnswer("the Endgame Gear OT mouse did not acknowledge the packet".into()))
}

/// One block's reply payload (the packet from byte 3), or `None` when the mouse did not answer - or answered with something
/// that is not the reply to THIS query. OpenMouse takes whatever response comes next; a late reply to an earlier query would
/// then be spliced into the wrong block of the profile and written back. Here the reply must be a response packet (comtype
/// 21) whose first payload byte is the block asked for (the reply echoes the block id there).
///
/// `check_id` is off only for a status READ of the main block: there byte 0 of the payload is the profile's first byte
/// (the vendor struct calls it `transferFlags`); that it is 0 on every real mouse is unclear, and a wrong guess must not
/// make the Mouse page show a working mouse as offline. Every block that feeds a write is checked.
fn ot_block(port: &mut dyn Port, block: u8, writes: u32, check_id: bool) -> Result<Option<Vec<u8>>> {
    let Some(r) = ot_query(port, &ot_query_packet(block, 0), writes)? else { return Ok(None) };
    if r[1] != OT_RESPONSE || (check_id && r[OT_PAYLOAD_INDEX] != block) {
        return Ok(None);
    }
    Ok(Some(r[OT_PAYLOAD_INDEX..].to_vec()))
}

/// `loadProfile(0)`: the main block first, then the other five; the 118 effect bytes.
fn ot_load_profile(port: &mut dyn Port) -> Result<Vec<u8>> {
    let mut replies = Vec::new();
    for block in [BLOCK_MAIN, BLOCK_KEYBOARD, BLOCK_NAME, BLOCK_CPI_Y, BLOCK_WIRELESS, BLOCK_RGB] {
        let r = ot_block(port, block, OT_WRITE_RETRIES, true)?.ok_or_else(|| Error::BadAnswer(format!("the mouse did not answer OT profile block {block}")))?;
        replies.push((block, r));
    }
    ot_reassemble(&replies).ok_or_else(|| Error::BadAnswer("an OT profile block answer was too short".into()))
}

/// The polling rate an effect holds: `otPollRateHz(index) ?? otPollRateHz(pollRates[index])`.
pub fn ot_effect_polling(effect: &[u8]) -> Option<u32> {
    let idx = *effect.get(OT_POLLRATE_INDEX)?;
    ot_poll_hz(idx).or_else(|| ot_poll_hz(*effect.get(OT_POLL_RATES + idx as usize)?))
}

/// The DPI of the active stage (`cpi[cpiIndex]`; `None` = the index is outside the four stages).
pub fn ot_effect_dpi(effect: &[u8]) -> Option<u32> {
    let i = *effect.get(OT_CPI_INDEX)? as usize;
    (i < 4).then(|| u16le(effect, OT_CPI + 2 * i))
}

/// `otLodLabel(dcuThreshold)` in tenths of a mm: raw 1 = 1 mm, 2 = 2 mm.
pub fn ot_effect_lift_off(effect: &[u8]) -> Option<u32> {
    match effect.get(OT_DCU_THRESHOLD)? {
        1 => Some(10),
        2 => Some(20),
        _ => None,
    }
}

fn read_ot(m: &Matched, port: &mut dyn Port, opt: ReadOptions) -> Result<OnMouse> {
    let mut r = OnMouse { wireless: m.wireless, caps: ot_caps(m.variant), ..Default::default() };
    // everything the page shows sits in the main block (bytes 0-49 of the profile)
    let writes = if opt.online_tries <= 1 { 1 } else { OT_WRITE_RETRIES };
    let Some(main) = soft(ot_block(port, BLOCK_MAIN, writes, false))?.flatten() else { return Ok(r) };
    r.online = true;
    r.dpi = ot_effect_dpi(&main);
    r.polling_hz = ot_effect_polling(&main);
    r.lift_off = ot_effect_lift_off(&main);
    Ok(r)
}

/// `updateProfile(0, change)`: load the profile, patch it, send the six packets, load it again. -> (before, confirmed)
///
/// Every change sends all six blocks, not only the one that changed: the keyboard / button, name, wireless and RGB blocks go
/// back as byte copies of what was just read. That is OpenMouse's own byte stream; the protocol has no smaller write for the
/// main block's bytes. It is acceptable because [`ot_load_profile`] refuses (error, nothing sent) unless ALL six blocks came
/// back as correct responses to their own query, so no copy can hold another block's bytes.
fn ot_update(port: &mut dyn Port, change: impl FnOnce(&mut [u8]) -> Result<()>) -> Result<(Vec<u8>, Vec<u8>)> {
    let before = ot_load_profile(port)?;
    let mut patched = before.clone();
    change(&mut patched)?;
    for (block, _, _) in OT_BLOCKS {
        ot_set(port, &ot_profile_packet(&patched, block))?;
    }
    let confirmed = ot_load_profile(port)?;
    Ok((before, confirmed))
}

fn write_ot(m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
    let max = ot_max_cpi(m.variant);
    match s {
        Setting::Dpi(d) => {
            if !ot_is_valid_cpi(max, d) {
                return Err(Error::range("DPI", format!("CPI must be 50 to {max} in 50 CPI steps (100 above 10,000)")));
            }
            // OpenMouse's setCpiStage on the active stage (its setDpi would overwrite all four stages)
            let mut stage = 0usize;
            let (before, confirmed) = ot_update(port, |e| {
                stage = e[OT_CPI_INDEX] as usize;
                if stage >= 4 {
                    return Err(Error::BadAnswer(format!("the mouse reported CPI stage {stage}, outside its four")));
                }
                let (x, y) = (u16le(e, OT_CPI + 2 * stage), u16le(e, OT_CPI_Y + 2 * stage));
                if x != y {
                    // undo remembers X only: it could not give Y back
                    return Err(Error::range("DPI", format!("CPI stage {} is set to separate X {x} / Y {y} values - change it in Endgame Gear's own tool, this app only sets one value for both axes", stage + 1)));
                }
                put_u16le(e, OT_CPI + 2 * stage, d);
                put_u16le(e, OT_CPI_Y + 2 * stage, d);
                Ok(())
            })?;
            if u16le(&confirmed, OT_CPI + 2 * stage) != d || u16le(&confirmed, OT_CPI_Y + 2 * stage) != d {
                return Err(Error::BadAnswer(format!("the mouse did not confirm CPI stage {}", stage + 1)));
            }
            Ok(Changed { now: d, before: Some(u16le(&before, OT_CPI + 2 * stage)) })
        }
        Setting::Polling(hz) => {
            let idx = [1000u32, 500, 250, 125].iter().position(|r| *r == hz).ok_or_else(|| Error::range("polling rate", "Unsupported OT polling rate."))?;
            let (before, confirmed) = ot_update(port, |e| {
                e[OT_POLLRATE_INDEX] = idx as u8;
                Ok(())
            })?;
            let now = ot_poll_hz(confirmed[OT_POLLRATE_INDEX]);
            if now != Some(hz) {
                return Err(Error::BadAnswer(format!("the mouse kept {now:?} Hz instead of {hz} Hz")));
            }
            Ok(Changed { now: hz, before: ot_effect_polling(&before) })
        }
        Setting::LiftOff(t) => {
            let raw = match t {
                10 => 1u8,
                20 => 2,
                _ => return Err(Error::range("lift-off distance", "this mouse exposes only 1 mm and 2 mm lift-off distances")),
            };
            let (before, confirmed) = ot_update(port, |e| {
                e[OT_DCU_THRESHOLD] = raw;
                Ok(())
            })?;
            if confirmed[OT_DCU_THRESHOLD] != raw {
                return Err(Error::BadAnswer("the mouse did not confirm the lift-off distance".into()));
            }
            Ok(Changed { now: t, before: ot_effect_lift_off(&before) })
        }
    }
}

// ================================================================================== allow-list + driver

/// The allow-list for proofs on the real mouse (A_009_01): only the READ requests this file sends -
/// - OP1: report 0xA1 with op `0x12` (load the config) or `0x0e` (mouse info), nothing else set;
/// - OT: report 0, a `queryProfileData` (51) of block 0 / 1 / 2 / 3 / 4 / 6 of profile 0, nothing else set.
///
/// Every write (store 0x11 / block writes 0x14-0x16 / setProfileData 52) and anything else (factory reset 0x13, firmware
/// ops) is refused.
pub fn read_request_allowed(out: &[u8]) -> bool {
    match out {
        [REPORT_COMMAND, op, rest @ ..] => matches!(*op, OP_LOAD | OP_MOUSE_INFO) && rest.iter().all(|b| *b == 0),
        [0, OT_QUERY_PROFILE_DATA, 0, block, 0, rest @ ..] => matches!(*block, 0 | 1 | 2 | 3 | 4 | 6) && rest.iter().all(|b| *b == 0),
        _ => false,
    }
}

/// The Endgame Gear driver (OP1 config protocol or OT platform; `Matched.variant` = the product id says which).
pub struct Endgame;

impl Driver for Endgame {
    /// READ requests only (see [`read_request_allowed`]).
    fn read(&self, m: &Matched, port: &mut dyn Port, opt: ReadOptions) -> Result<OnMouse> {
        match the_product(m)?.client {
            Client::Op1 => read_op1(m, port, opt),
            Client::Ot => read_ot(m, port, opt),
        }
    }

    /// One setting the way OpenMouse's client writes it (with the DECISIONS in the header), read back to confirm.
    fn write(&self, m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        match the_product(m)?.client {
            Client::Op1 => write_op1(m, port, s),
            Client::Ot => write_ot(m, port, s),
        }
    }
}

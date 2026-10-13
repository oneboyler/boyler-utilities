//! Mchose - ported from OpenMouse (`crate::family`: AGPL-3.0-or-later, @openmouse/protocol 0.40.1), DPI / polling /
//! battery only. Two of OpenMouse's four Mchose clients are used by models on its SUPPORTED list:
//!
//! | client (`Matched.variant & 0xFF`) | OpenMouse files | models | state |
//! |---|---|---|---|
//! | [`CLIENT_V2`] | `dist/mchose/index.js` (codec) + `dist/drivers/mchose/hid.js` | A7 V2 Ultra+ (0x3837:0x4021 wired, 0x3837:0x100b 2.4 GHz receiver) | ported, in the device table |
//! | [`CLIENT_A5`] | `dist/mchose/a5-gen1.js` + `dist/drivers/mchose/a5-gen1-hid.js` | A5 Pro Max (0x2023:0xf019 wired, 0xf013 1K receiver, 0xf015 4K receiver) | ported, **held back** (see below) |
//!
//! Left out: `dock-hid.js` / `dock.js` (the MagDock charging base, 0x3837:0x1012): its only commands are "read lighting" and
//! "write lighting" (no DPI, polling, lift-off or battery), so nothing of it is ported - the dock is NOT in the device table.
//! `v3-hid.js` (A7 V3): those models are "likely" on OpenMouse's list, not "supported".
//!
//! **A7 V2 (ported)** - feature reports on the vendor collection (usage page 0xFF01, usage 1). Report 0x11 is the short
//! command channel (20 tokens), 0x12 the long one (64 tokens). The whole command body is bit-inverted, in both directions:
//! a request is `[report id] ++ (~cmd, ~arg0, ...)` (tokens beyond the command's own are inverted zeros, up to the
//! channel's token count, then zeros), an answer `[report id, ~cmd, ~payload0, ...]`. Windows pads both to the collection's
//! feature-report length (OpenMouse sends a 64-byte body behind the id: the collection must have at least 65). Reads:
//! 0x03 receiver identity (short), 0x06 battery (short; payload `[vid(2), pid(2), fw(4), flags, percent, charging]`),
//! 0x67 configuration (long; `[profile, wired, wireless, 0, six DPI stages u16 LE, ..., stage count at 16, ...]` where a
//! link byte is `rate index << 4 | DPI stage index`). OpenMouse polls every request (send, 90 ms, read; up to 20 times) and
//! accepts an answer only when the command echo matches, the accept check passes and the SAME payload came twice in a row.
//! Writes: one DPI stage's value or a link's polling-rate index go through the configuration write 0x57 (long) - the
//! payload that was just read is echoed back byte for byte (stale scratch bytes included, as OpenMouse does) and only the
//! changed fields differ; 400 ms later the configuration is read again to confirm. The polling rate is an index into the
//! model's list (Ultra+: 125, 500, 1000, 2000, 4000, 8000), one index per link (wired / 2.4 GHz); DPI is a value in the
//! stage that is active on the link in use (50..42000 step 50 for the Ultra / Ultra+).
//!
//! **A5 Pro Max (ported but not in the device table)** - feature reports on usage page 0xFFFF; request body `[0, 0, route 2,
//! length, page, command, data...]`, answer starts 0xA1 and echoes page and command. Reads: battery (page 0 cmd 0x83),
//! polling (page 1 cmd 0x80), the DPI stage table (page 1 cmd 0x81) and the active stage (cmd 0x82). The feature REPORT ID
//! is taken by OpenMouse from the device's own report descriptor (the first feature report of a vendor collection) and our
//! `HidInfo` does not carry report ids: without it not one byte can be sent. NEEDS: the report id on `HidInfo`; then
//! [`a5_matches`] (already written, takes the id) goes into [`matches`]. One thing differs from OpenMouse on purpose: its
//! DPI write rewrites every stage record as `[x, x]` (the answer's second pair is never read); a table whose records have
//! y different from x is not rewritten here (refused), so no stored value is lost.
//!
//! Lift-off: left out. OpenMouse's pages show Low / Medium / High; its model table (`MCHOSE_PRODUCTS.liftOffDistances`,
//! A7 V2 Ultra+: 0.7, 1, 2 mm) is only used for HOW MANY steps there are, never to say which step is which millimetre, so
//! a mm figure would be our guess. Also left out: profiles, stage count, sleep, debounce, motion sync / angle snapping /
//! ripple / performance mode / angle tuning, buttons, macros, profile names, firmware version. OpenMouse sends no
//! firmware-update, bootloader or reset command to a Mchose mouse and neither does this file.

use crate::error::{Error, Result};
use crate::family::{Caps, Changed, DpiCaps, Family, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x3837;
/// The A5 Pro Max's own vendor id (a different vendor id from the A7's).
pub const A5_VENDOR_ID: u16 = 0x2023;
/// `Matched.variant & 0xFF`: the A7 V2 client.
pub const CLIENT_V2: u16 = 2;
/// `Matched.variant & 0xFF`: the A5 Pro Max client; `variant >> 8` is its feature report id.
pub const CLIENT_A5: u16 = 5;

// ============================================================================ A7 V2 codec

pub mod v2 {
    //! The A7 V2 codec (`dist/mchose/index.js`).

    pub const CONFIG_USAGE_PAGE: u16 = 0xFF01;
    pub const CONFIG_USAGE: u16 = 0x0001;
    pub const SHORT_ID: u8 = 0x11;
    pub const SHORT_TOKENS: usize = 20;
    pub const LONG_ID: u8 = 0x12;
    pub const LONG_TOKENS: usize = 64;
    /// The body behind the report id (MCHOSE_REPORT_LENGTH).
    pub const BODY_LEN: usize = 64;
    pub const CMD_IDENTITY: u8 = 0x03;
    pub const CMD_BATTERY: u8 = 0x06;
    pub const CMD_CONFIG: u8 = 0x67;
    pub const CMD_WRITE_CONFIG: u8 = 0x57;
    pub const DPI_STAGES: usize = 6;
    pub const DPI_MIN: u32 = 50;
    pub const DPI_STEP: u32 = 50;
    /// OpenMouse's DPI ceiling when it can't name the model.
    pub const DPI_MAX_UNKNOWN_MODEL: u32 = 26000;
    /// bytes of the 0x67 answer that carry fields (CONFIG_MEANINGFUL_BYTES)
    pub const CONFIG_MEANINGFUL_BYTES: usize = 20;
    pub const BATTERY_MEANINGFUL_BYTES: usize = 11;

    /// mchoseEncodeCommand: every given token inverted, up to `token_count`; the rest of the 64-byte body stays zero.
    pub fn encode_command(tokens: &[u8], token_count: usize) -> [u8; BODY_LEN] {
        let mut body = [0u8; BODY_LEN];
        for (i, b) in body.iter_mut().enumerate().take(token_count.min(BODY_LEN)) {
            *b = tokens.get(i).copied().unwrap_or(0) ^ 0xFF;
        }
        body
    }

    /// A body as Windows sends it: the report id, the body, zeros up to the collection's feature-report length. `None` = the
    /// collection is too short for OpenMouse's 64-byte body.
    pub fn feature_report(report_id: u8, body: &[u8], len: usize) -> Option<Vec<u8>> {
        if len < 1 + body.len() {
            return None;
        }
        let mut v = vec![0u8; len];
        v[0] = report_id;
        v[1..1 + body.len()].copy_from_slice(body);
        Some(v)
    }

    /// mchoseDecodeReply: `raw` starts with the report id; the command and the payload come back un-inverted.
    pub fn decode_reply(raw: &[u8]) -> Option<(u8, Vec<u8>)> {
        if raw.len() < 3 {
            return None;
        }
        Some((!raw[1], raw[2..].iter().map(|b| !*b).collect()))
    }

    fn u16le(d: &[u8], at: usize) -> u32 {
        d.get(at).copied().unwrap_or(0) as u32 | (d.get(at + 1).copied().unwrap_or(0) as u32) << 8
    }

    /// mchoseDecodeIdentity.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Identity {
        pub bonded: bool,
        pub vendor_id: u32,
        pub product_id: u32,
        pub connected: bool,
        pub game_mode: u8,
    }

    pub fn decode_identity(p: &[u8]) -> Option<Identity> {
        (p.len() >= 7).then(|| Identity { bonded: p[0] != 0, vendor_id: u16le(p, 1), product_id: u16le(p, 3), connected: p[5] != 0, game_mode: p[6] })
    }

    /// mchoseDecodeBattery (the fields this port uses).
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Battery {
        /// the mouse's own model id (decisive for the model; the host-facing id is shared by the family)
        pub product_id: u32,
        pub connected: bool,
        pub percent: u8,
        pub charging: bool,
    }

    pub fn decode_battery(p: &[u8]) -> Option<Battery> {
        (p.len() >= BATTERY_MEANINGFUL_BYTES).then(|| Battery { product_id: u16le(p, 2), connected: (p[8] >> 4) & 1 != 0, percent: p[9], charging: p[10] != 0 })
    }

    /// mchoseDecodeConfig (the fields this port uses). A link byte is `rate index << 4 | DPI stage index`.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Config {
        pub profile_index: u8,
        pub wired_dpi_index: usize,
        pub wired_rate_index: usize,
        pub wireless_dpi_index: usize,
        pub wireless_rate_index: usize,
        pub dpi_stages: [u32; DPI_STAGES],
        pub stage_count: u8,
    }

    pub fn decode_config(p: &[u8]) -> Option<Config> {
        if p.len() < 4 + DPI_STAGES * 2 {
            return None;
        }
        let mut dpi_stages = [0u32; DPI_STAGES];
        for (i, s) in dpi_stages.iter_mut().enumerate() {
            *s = u16le(p, 4 + i * 2);
        }
        Some(Config {
            profile_index: p[0],
            wired_dpi_index: (p[1] & 0x0F) as usize,
            wired_rate_index: (p[1] >> 4) as usize,
            wireless_dpi_index: (p[2] & 0x0F) as usize,
            wireless_rate_index: (p[2] >> 4) as usize,
            dpi_stages,
            stage_count: p.get(16).copied().unwrap_or(0),
        })
    }

    /// `looksLikeConfig`: a config payload always carries a plausible first DPI stage.
    pub fn looks_like_config(p: &[u8]) -> bool {
        if p.len() < CONFIG_MEANINGFUL_BYTES {
            return false;
        }
        let first = u16le(p, 4);
        (DPI_MIN..=42000).contains(&first)
    }

    /// mchosePackLinkByte.
    pub fn pack_link_byte(rate_index: usize, dpi_index: usize) -> u8 {
        (((rate_index & 0x0F) << 4) | (dpi_index & 0x0F)) as u8
    }

    /// What a configuration write changes (mchoseEncodeConfigWrite's `changes`, the fields this port sets).
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct Changes {
        pub wired_rate_index: Option<usize>,
        pub wireless_rate_index: Option<usize>,
        pub dpi_stages: Option<[u32; DPI_STAGES]>,
    }

    /// mchoseEncodeConfigWrite: the tokens of the 0x57 write: the command byte, then the payload that was read, echoed byte
    /// for byte, with the two link bytes re-packed and (when given) the six DPI stages rewritten.
    pub fn encode_config_write(read_payload: &[u8], ch: &Changes) -> Option<Vec<u8>> {
        let cur = decode_config(read_payload)?;
        let mut out = Vec::with_capacity(1 + read_payload.len());
        out.push(CMD_WRITE_CONFIG);
        out.extend_from_slice(read_payload);
        // +1 throughout: the command byte shifts every field by one
        out[2] = pack_link_byte(ch.wired_rate_index.unwrap_or(cur.wired_rate_index), cur.wired_dpi_index);
        out[3] = pack_link_byte(ch.wireless_rate_index.unwrap_or(cur.wireless_rate_index), cur.wireless_dpi_index);
        if let Some(stages) = ch.dpi_stages {
            for (i, v) in stages.iter().enumerate() {
                out[1 + 4 + i * 2] = (v & 0xFF) as u8;
                out[1 + 4 + i * 2 + 1] = ((v >> 8) & 0xFF) as u8;
            }
        }
        Some(out)
    }

    /// One entry of the A7 V2 family's model table (`MCHOSE_PRODUCTS`).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Product {
        pub name: &'static str,
        pub product_id: u32,
        pub dpi_max: u32,
        /// the highest polling rate the model has (selects the rate list)
        pub max_polling: u32,
    }

    pub const PRODUCTS: [Product; 4] = [
        Product { name: "A7 V2 Pro", product_id: 0x4018, dpi_max: 26000, max_polling: 8000 },
        Product { name: "A7 V2 Pro+", product_id: 0x4023, dpi_max: 26000, max_polling: 8000 },
        Product { name: "A7 V2 Ultra", product_id: 0x4019, dpi_max: 42000, max_polling: 8000 },
        Product { name: "A7 V2 Ultra+", product_id: 0x4021, dpi_max: 42000, max_polling: 8000 },
    ];

    pub fn product_by_id(id: u32) -> Option<&'static Product> {
        PRODUCTS.iter().find(|p| p.product_id == id)
    }

    /// `MCHOSE_POLLING_RATES` for a model's maximum rate; the rate index on the wire indexes this list.
    pub fn polling_rates(max_polling: u32) -> Vec<u32> {
        match max_polling {
            4000 => vec![125, 500, 1000, 2000, 4000],
            8000 => vec![125, 500, 1000, 2000, 4000, 8000],
            _ => vec![125, 500, 1000],
        }
    }

    /// OpenMouse's `Math.round(dpi / 50) * 50` (halves go up).
    pub fn round_dpi(dpi: u32) -> u32 {
        (dpi + DPI_STEP / 2) / DPI_STEP * DPI_STEP
    }
}

// ============================================================================ A5 Pro Max codec

pub mod a5 {
    //! The first-generation A5 Pro Max codec (`dist/mchose/a5-gen1.js`) and the constants of its client.

    pub const USAGE_PAGE: u16 = 0xFFFF;
    /// the body behind the report id
    pub const BODY_LEN: usize = 64;
    pub const DPI_STAGES: usize = 6;
    pub const DPI_MIN: u32 = 50;
    pub const DPI_MAX: u32 = 26000;
    pub const DPI_STEP: u32 = 50;
    pub const RECEIVER_RATES: [u32; 4] = [125, 250, 500, 1000];
    pub const WIRED_RATES: [u32; 3] = [125, 500, 1000];

    /// mchoseA5Gen1EncodeRequest: `[0, 0, route, length, page, command, data (at most 57 bytes)...]`.
    pub fn encode_request(length: u8, page: u8, command: u8, data: &[u8], route: u8) -> [u8; BODY_LEN] {
        let mut b = [0u8; BODY_LEN];
        b[2] = route;
        b[3] = length;
        b[4] = page;
        b[5] = command;
        let n = data.len().min(57);
        b[6..6 + n].copy_from_slice(&data[..n]);
        b
    }

    /// mchoseA5Gen1DecodeReply: the 64-byte body of an answer. On Windows the buffer always starts with the report id byte (even
    /// for an unnumbered report), whatever the collection's feature-report length is, so the id is ALWAYS cut off, the rest padded
    /// or cut to 64 (OpenMouse cuts it only when the buffer is exactly 65 bytes long and would misalign any other length).
    pub fn decode_reply(raw: &[u8], _report_id: u8) -> Vec<u8> {
        let body = raw.get(1..).unwrap_or(&[]);
        let mut v = body[..body.len().min(BODY_LEN)].to_vec();
        v.resize(BODY_LEN, 0);
        v
    }

    /// mchoseA5Gen1ReplyMatches.
    pub fn reply_matches(reply: &[u8], page: u8, command: u8) -> bool {
        reply.len() >= 6 && reply[0] == 0xA1 && reply[4] == page && reply[5] == command
    }

    pub fn normalize_dpi(v: u32) -> u32 {
        ((v + DPI_STEP / 2) / DPI_STEP * DPI_STEP).clamp(DPI_MIN, DPI_MAX)
    }

    /// mchoseA5Gen1EncodeDpi: `[profile, count, (x hi, x lo, x hi, x lo) per stage]`.
    pub fn encode_dpi(profile: u8, stages: &[u32]) -> Vec<u8> {
        let n: Vec<u32> = stages.iter().take(DPI_STAGES).map(|v| normalize_dpi(*v)).collect();
        let mut d = vec![profile, n.len() as u8];
        for v in n {
            d.extend_from_slice(&[(v >> 8) as u8, (v & 0xFF) as u8, (v >> 8) as u8, (v & 0xFF) as u8]);
        }
        d
    }

    /// The stage table of a "read DPI" answer (mchoseA5Gen1DecodeDpi) with the second pair of each record kept too.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct DpiTable {
        pub count: usize,
        /// the count byte as the mouse sent it (OpenMouse reads 0 as 6 and cuts anything above 6)
        pub raw_count: usize,
        /// x per stage (the pair OpenMouse reads)
        pub stages: Vec<u32>,
        /// the record's second pair (OpenMouse never reads it; it writes it as a copy of x)
        pub second: Vec<u32>,
    }

    pub fn decode_dpi(reply: &[u8]) -> DpiTable {
        let at = |i: usize| reply.get(i).copied().unwrap_or(0) as u32;
        let raw_count = at(7) as usize;
        let count = (if raw_count == 0 { DPI_STAGES } else { raw_count }).clamp(1, DPI_STAGES);
        let stages = (0..count).map(|i| at(8 + i * 4) << 8 | at(9 + i * 4)).collect();
        let second = (0..count).map(|i| at(10 + i * 4) << 8 | at(11 + i * 4)).collect();
        DpiTable { count, raw_count, stages, second }
    }

    /// POLLING_CODES: 125→8, 250→4, 500→2, 1000→1.
    pub fn polling_code(hz: u32) -> Option<u8> {
        Some(match hz {
            125 => 0x08,
            250 => 0x04,
            500 => 0x02,
            1000 => 0x01,
            _ => return None,
        })
    }

    pub fn polling_hz(code: u8) -> Option<u32> {
        [125u32, 250, 500, 1000].into_iter().find(|hz| polling_code(*hz) == Some(code))
    }

    /// The rates a link offers (MCHOSE_A5_GEN1_*_RATES).
    pub fn rates(wired: bool) -> Vec<u32> {
        if wired { WIRED_RATES.to_vec() } else { RECEIVER_RATES.to_vec() }
    }
}

// ============================================================================ caps + allow-list

/// What the Mouse page offers for the A7 V2 (OpenMouse's stage editor 50..dpiMax step 50, the model's polling list). With no
/// known model: OpenMouse's 26000 ceiling and no polling control.
pub fn v2_caps(product: Option<&v2::Product>) -> Caps {
    let max = product.map(|p| p.dpi_max).unwrap_or(v2::DPI_MAX_UNKNOWN_MODEL);
    Caps { dpi: Some(DpiCaps::uniform(v2::DPI_MIN, max, v2::DPI_STEP)), polling: product.map(|p| v2::polling_rates(p.max_polling)).unwrap_or_default(), lift_off: Vec::new(), battery: true }
}

/// What the Mouse page offers for the A5 Pro Max.
pub fn a5_caps(wired: bool) -> Caps {
    Caps { dpi: Some(DpiCaps::uniform(a5::DPI_MIN, a5::DPI_MAX, a5::DPI_STEP)), polling: a5::rates(wired), lift_off: Vec::new(), battery: true }
}

/// The read-only proof's allow-list. A7 V2: a feature report 0x11 carrying the inverted identity (0x03) or battery (0x06)
/// query, or 0x12 carrying the inverted configuration read (0x67) - exactly OpenMouse's bytes, zeros after. A5 Pro Max: a
/// 64-byte request (behind any report id) for one of the five reads this driver sends: battery, active profile, polling,
/// the DPI table or the active DPI stage. The configuration write 0x57, the performance command 0x42, the sleep / profile /
/// button commands and every A5 setter fail.
pub fn read_request_allowed(out: &[u8]) -> bool {
    if out.len() < 1 + v2::BODY_LEN {
        return false;
    }
    let all = |s: &[u8], v: u8| s.iter().all(|b| *b == v);
    let (id, body) = (out[0], &out[1..]);
    // A7 V2: inverted tokens (0xFF for each zero token) up to the channel's count, then zeros
    for (rid, cmd, n) in [(v2::SHORT_ID, v2::CMD_IDENTITY, v2::SHORT_TOKENS), (v2::SHORT_ID, v2::CMD_BATTERY, v2::SHORT_TOKENS), (v2::LONG_ID, v2::CMD_CONFIG, v2::LONG_TOKENS)] {
        if id == rid && body[0] == !cmd && all(&body[1..n], 0xFF) && all(&body[n..], 0) {
            return true;
        }
    }
    // A5: [0, 0, route 2, length, page, command, data...] - the reads only. Not under 0x11 / 0x12: those report ids are the A7 V2's
    // channels, where an A5-shaped frame is an A7 V2 write of some other command.
    if id != v2::SHORT_ID && id != v2::LONG_ID && body[0] == 0 && body[1] == 0 && body[2] == 2 {
        let (len, page, cmd) = (body[3], body[4], body[5]);
        // (length, page, command, data bytes after the 6-byte header)
        let (want_len, data_len) = match (page, cmd) {
            (0, 0x83) => (2, 0),
            (0, 0x85) => (1, 0),
            (1, 0x80) => (1, 0),
            (1, 0x81) => (10, 2),
            (1, 0x82) => (2, 1),
            _ => return false,
        };
        let data_ok = match (page, cmd) {
            (1, 0x81) => body[7] == 6 && body[6] <= 4,
            (1, 0x82) => body[6] <= 4,
            _ => true,
        };
        return len == want_len && data_ok && all(&body[6 + data_len..], 0);
    }
    false
}

// ============================================================================ device table

/// One Mchose product id on OpenMouse's SUPPORTED list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Product {
    pub vid: u16,
    pub pid: u16,
    pub client: u16,
    pub model: &'static str,
    /// the id is a receiver (2.4 GHz), not the cable
    pub wireless: bool,
}

/// OpenMouse's SUPPORTED list (supported-devices-data.ts, status "supported"): A7 V2 Ultra+ 0x4021 / 0x100b, MagDock 0x1012
/// (NOT here: no DPI / polling / battery command), A5 Pro Max 0xf019 / 0xf013 / 0xf015. 0x100b is the receiver every A7 V2
/// shares - the real model comes from the mouse's own answer.
pub const PRODUCTS: [Product; 5] = [
    Product { vid: VENDOR_ID, pid: 0x4021, client: CLIENT_V2, model: "MCHOSE A7 V2 Ultra+", wireless: false },
    Product { vid: VENDOR_ID, pid: 0x100B, client: CLIENT_V2, model: "MCHOSE A7 V2 Ultra+", wireless: true },
    Product { vid: A5_VENDOR_ID, pid: 0xF019, client: CLIENT_A5, model: "MCHOSE A5 Pro Max", wireless: false },
    Product { vid: A5_VENDOR_ID, pid: 0xF013, client: CLIENT_A5, model: "MCHOSE A5 Pro Max", wireless: true },
    Product { vid: A5_VENDOR_ID, pid: 0xF015, client: CLIENT_A5, model: "MCHOSE A5 Pro Max", wireless: true },
];

/// The A7 V2 config collection: the vendor collection (usage page 0xFF01, usage 1) with a feature report big enough for
/// OpenMouse's 64-byte body behind the report id.
pub fn v2_interface<'a>(own: &[&'a HidInfo]) -> Option<&'a HidInfo> {
    own.iter().copied().find(|h| h.usage_page == v2::CONFIG_USAGE_PAGE && h.usage == v2::CONFIG_USAGE && h.feature_len as usize > v2::BODY_LEN)
}

/// The A5 command collection: usage page 0xFFFF with feature reports.
pub fn a5_interface<'a>(own: &[&'a HidInfo]) -> Option<&'a HidInfo> {
    own.iter().copied().find(|h| h.usage_page == a5::USAGE_PAGE && h.feature_len as usize > a5::BODY_LEN)
}

/// The A5 Pro Max matcher once the feature report id is known (see the header): the id goes into `variant >> 8`.
pub fn a5_matches(own: &[&HidInfo], vid: u16, pid: u16, report_id: u8) -> Option<Matched> {
    let p = PRODUCTS.iter().find(|p| p.client == CLIENT_A5 && p.vid == vid && p.pid == pid)?;
    let cfg = a5_interface(own)?;
    Some(Matched { family: Family::Mchose, model: Some(p.model), wireless: Some(p.wireless), config: cfg.clone(), variant: CLIENT_A5 | (report_id as u16) << 8 })
}

/// The device table: the A7 V2 Ultra+ (cable 0x4021 and receiver 0x100b). The A5 Pro Max is held back (no report id on
/// `HidInfo`): [`a5_matches`] is its matcher. The MagDock (0x1012) and every A7 V3 id are not in the table.
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    let p = PRODUCTS.iter().find(|p| p.client == CLIENT_V2 && p.vid == vid && p.pid == pid)?;
    let cfg = v2_interface(own)?;
    Some(Matched { family: Family::Mchose, model: Some(p.model), wireless: Some(p.wireless), config: cfg.clone(), variant: CLIENT_V2 })
}

// ============================================================================ A7 V2 driver

/// OpenMouse's POLL_DELAY_MS.
const POLL_DELAY_MS: u32 = 90;
/// OpenMouse's POLL_ATTEMPTS.
const POLL_ATTEMPTS: u32 = 20;
/// The read-only proof asks "do not loop": a few polls, enough for the "same answer twice" rule and one stale answer.
const PROOF_ATTEMPTS: u32 = 4;
/// OpenMouse's WRITE_SETTLE_MS: after a configuration write, before it is read again.
pub const WRITE_SETTLE_MS: u64 = 400;

fn attempts_for(opt: &ReadOptions) -> u32 {
    if opt.online_tries <= 1 { PROOF_ATTEMPTS } else { POLL_ATTEMPTS }
}

/// OpenMouse's `request`: send the inverted command, wait, read the feature report; accept an answer when its command echo
/// matches, `accept` passes and the same payload came twice in a row. `None` = never settled.
fn v2_request(port: &mut dyn Port, report_id: u8, command: u8, tokens: usize, accept: &dyn Fn(&[u8]) -> bool, attempts: u32) -> Result<Option<Vec<u8>>> {
    let len = port.info().feature_len as usize;
    let out = v2::feature_report(report_id, &v2::encode_command(&[command], tokens), len).ok_or_else(|| Error::UnsupportedMouse("the Mchose config report is too short".into()))?;
    let how = HidTransfer::Feature { reply_id: report_id, reply_len: len, wait_ms: POLL_DELAY_MS };
    let mut previous: Option<Vec<u8>> = None;
    for _ in 0..attempts {
        let raw = match port.exchange(&out, &how) {
            Ok(r) => r,
            Err(Error::BadAnswer(_) | Error::Os { .. }) => continue,
            Err(e) => return Err(e),
        };
        let Some((cmd, payload)) = v2::decode_reply(&raw) else { continue };
        if cmd != command || !accept(&payload) {
            continue;
        }
        if previous.as_ref() == Some(&payload) {
            return Ok(Some(payload));
        }
        previous = Some(payload);
    }
    Ok(None)
}

fn non_zero(n: usize) -> impl Fn(&[u8]) -> bool {
    move |p: &[u8]| p.iter().take(n).any(|b| *b != 0)
}

fn v2_read_config_payload(port: &mut dyn Port, attempts: u32) -> Result<Option<Vec<u8>>> {
    v2_request(port, v2::LONG_ID, v2::CMD_CONFIG, v2::LONG_TOKENS, &v2::looks_like_config, attempts)
}

fn v2_product_of(m: &Matched, battery_product_id: Option<u32>) -> Option<&'static v2::Product> {
    battery_product_id.and_then(v2::product_by_id).or_else(|| v2::product_by_id(m.config.pid as u32))
}

fn v2_read(m: &Matched, port: &mut dyn Port, opt: ReadOptions) -> Result<OnMouse> {
    let attempts = attempts_for(&opt);
    let wired = m.wireless == Some(false);
    let mut r = OnMouse { model: None, online: false, wireless: m.wireless, caps: v2_caps(v2_product_of(m, None)), ..Default::default() };
    // identity answers from the receiver itself: a receiver that says "mouse not linked" has nothing more to ask
    let identity = v2_request(port, v2::SHORT_ID, v2::CMD_IDENTITY, v2::SHORT_TOKENS, &non_zero(7), attempts)?.and_then(|p| v2::decode_identity(&p));
    if !wired && identity.as_ref().is_some_and(|i| !i.connected) {
        return Ok(r);
    }
    let battery = v2_request(port, v2::SHORT_ID, v2::CMD_BATTERY, v2::SHORT_TOKENS, &non_zero(v2::BATTERY_MEANINGFUL_BYTES), attempts)?.and_then(|p| v2::decode_battery(&p));
    let config = v2_read_config_payload(port, attempts)?.and_then(|p| v2::decode_config(&p));
    r.online = battery.is_some() || config.is_some();
    let product = v2_product_of(m, battery.as_ref().map(|b| b.product_id));
    r.caps = v2_caps(product);
    r.model = product.map(|p| format!("MCHOSE {}", p.name));
    if let Some(b) = &battery {
        r.battery_percent = Some(b.percent.min(100));
        r.charging = Some(b.charging);
    }
    if let Some(c) = &config {
        let (stage, rate) = if wired { (c.wired_dpi_index, c.wired_rate_index) } else { (c.wireless_dpi_index, c.wireless_rate_index) };
        r.dpi = c.dpi_stages.get(stage).copied().filter(|d| *d > 0);
        r.polling_hz = product.and_then(|p| v2::polling_rates(p.max_polling).get(rate).copied());
    }
    Ok(r)
}

/// Send a configuration write (0x57, long), wait, and read the configuration again.
fn v2_write_config(port: &mut dyn Port, payload: &[u8], ch: &v2::Changes) -> Result<v2::Config> {
    let tokens = v2::encode_config_write(payload, ch).ok_or_else(|| Error::BadAnswer("the Mchose configuration could not be decoded".into()))?;
    let len = port.info().feature_len as usize;
    let out = v2::feature_report(v2::LONG_ID, &v2::encode_command(&tokens, v2::LONG_TOKENS), len).ok_or_else(|| Error::UnsupportedMouse("the Mchose config report is too short".into()))?;
    port.exchange(&out, &HidTransfer::FeatureOut)?;
    port.pause_ms(WRITE_SETTLE_MS);
    let after = v2_read_config_payload(port, POLL_ATTEMPTS)?.ok_or_else(|| Error::BadAnswer("the mouse did not confirm the new configuration".into()))?;
    v2::decode_config(&after).ok_or_else(|| Error::BadAnswer("the mouse did not confirm the new configuration".into()))
}

fn v2_write(m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
    let wired = m.wireless == Some(false);
    // OpenMouse's cachedProduct: the model the battery answer names (a receiver is shared by the whole family)
    let product = match v2_product_of(m, None) {
        Some(p) if wired => Some(p),
        _ => {
            let b = v2_request(port, v2::SHORT_ID, v2::CMD_BATTERY, v2::SHORT_TOKENS, &non_zero(v2::BATTERY_MEANINGFUL_BYTES), POLL_ATTEMPTS)?.and_then(|p| v2::decode_battery(&p));
            v2_product_of(m, b.map(|b| b.product_id))
        }
    };
    match s {
        Setting::Dpi(dpi) => {
            let max = product.map(|p| p.dpi_max).unwrap_or(v2::DPI_MAX_UNKNOWN_MODEL);
            if !(v2::DPI_MIN..=max).contains(&dpi) {
                return Err(Error::range("DPI", format!("{dpi} DPI is outside {}-{max}", v2::DPI_MIN)));
            }
            let rounded = v2::round_dpi(dpi);
            let payload = v2_read_config_payload(port, POLL_ATTEMPTS)?.ok_or_else(|| Error::BadAnswer("the mouse did not return its configuration".into()))?;
            let cur = v2::decode_config(&payload).ok_or_else(|| Error::BadAnswer("the mouse did not return its configuration".into()))?;
            let stage = if wired { cur.wired_dpi_index } else { cur.wireless_dpi_index };
            if stage >= v2::DPI_STAGES {
                return Err(Error::BadAnswer(format!("the mouse's active DPI stage {stage} is out of range")));
            }
            let mut stages = cur.dpi_stages;
            stages[stage] = rounded;
            let after = v2_write_config(port, &payload, &v2::Changes { dpi_stages: Some(stages), ..Default::default() })?;
            if after.dpi_stages[stage] != rounded {
                return Err(Error::BadAnswer("the mouse did not accept the new DPI value".into()));
            }
            Ok(Changed { now: rounded, before: Some(cur.dpi_stages[stage]).filter(|d| *d > 0) })
        }
        Setting::Polling(hz) => {
            let p = product.ok_or_else(|| Error::UnsupportedMouse("the Mchose model behind this receiver is not known".into()))?;
            let index = v2::polling_rates(p.max_polling).iter().position(|r| *r == hz).ok_or_else(|| Error::range("polling rate", format!("{hz} Hz is not supported on this connection")))?;
            let payload = v2_read_config_payload(port, POLL_ATTEMPTS)?.ok_or_else(|| Error::BadAnswer("the mouse did not return its configuration".into()))?;
            let cur = v2::decode_config(&payload).ok_or_else(|| Error::BadAnswer("the mouse did not return its configuration".into()))?;
            let before = v2::polling_rates(p.max_polling).get(if wired { cur.wired_rate_index } else { cur.wireless_rate_index }).copied();
            let ch = if wired { v2::Changes { wired_rate_index: Some(index), ..Default::default() } } else { v2::Changes { wireless_rate_index: Some(index), ..Default::default() } };
            let after = v2_write_config(port, &payload, &ch)?;
            if (if wired { after.wired_rate_index } else { after.wireless_rate_index }) != index {
                return Err(Error::BadAnswer("the mouse did not accept the new polling rate".into()));
            }
            Ok(Changed { now: hz, before })
        }
        Setting::LiftOff(t) => Err(Error::range("lift-off distance", format!("{t} tenths of a mm: lift-off is not set on a Mchose mouse here (OpenMouse names no millimetres)"))),
    }
}

// ============================================================================ A5 Pro Max driver

fn a5_report_id(m: &Matched) -> u8 {
    (m.variant >> 8) as u8
}

/// OpenMouse's `request` for the A5: send, wait 34 ms, then up to eight reads 30 ms apart, the request sent again after the
/// fourth. Returns the 64-byte answer that echoes `page` and `command`.
fn a5_request(port: &mut dyn Port, report_id: u8, length: u8, page: u8, command: u8, data: &[u8]) -> Result<Vec<u8>> {
    let len = port.info().feature_len as usize;
    let out = v2::feature_report(report_id, &a5::encode_request(length, page, command, data, 2), len).ok_or_else(|| Error::UnsupportedMouse("the Mchose A5 config report is too short".into()))?;
    for attempt in 0..8u32 {
        let how = match attempt {
            0 => HidTransfer::Feature { reply_id: report_id, reply_len: len, wait_ms: 34 },
            4 => HidTransfer::Feature { reply_id: report_id, reply_len: len, wait_ms: 30 },
            _ => HidTransfer::FeatureIn { reply_id: report_id, reply_len: len, wait_ms: 30 },
        };
        match port.exchange(&out, &how) {
            Ok(raw) => {
                let reply = a5::decode_reply(&raw, report_id);
                if a5::reply_matches(&reply, page, command) {
                    return Ok(reply);
                }
            }
            Err(Error::BadAnswer(_) | Error::Os { .. }) => {}
            Err(e) => return Err(e),
        }
    }
    Err(Error::BadAnswer(format!("the Mchose A5 did not acknowledge page {page:#x} command {command:#x}")))
}

fn a5_optional<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(Error::BadAnswer(_) | Error::Os { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

fn a5_profile(port: &mut dyn Port, id: u8) -> Result<u8> {
    let r = a5_request(port, id, 1, 0, 0x85, &[])?;
    Ok(if r[6] == 0 { 1 } else { r[6] })
}

/// readDpi: the stage table and the active stage (1-based, clamped to the count).
fn a5_read_dpi(port: &mut dyn Port, id: u8, profile: u8) -> Result<(a5::DpiTable, usize, usize)> {
    let table = a5::decode_dpi(&a5_request(port, id, 10, 1, 0x81, &[profile, a5::DPI_STAGES as u8])?);
    let active_reply = a5_request(port, id, 2, 1, 0x82, &[profile])?;
    let raw = if active_reply[7] == 0 { 1 } else { active_reply[7] as usize };
    Ok((table.clone(), raw.clamp(1, table.count), active_reply[7] as usize))
}

fn a5_read_polling(port: &mut dyn Port, id: u8) -> Result<Option<u32>> {
    Ok(a5::polling_hz(a5_request(port, id, 1, 1, 0x80, &[])?[6]))
}

fn a5_read(m: &Matched, port: &mut dyn Port) -> Result<OnMouse> {
    let id = a5_report_id(m);
    let wired = m.wireless == Some(false);
    let mut r = OnMouse { model: None, online: false, wireless: m.wireless, caps: a5_caps(wired), ..Default::default() };
    let battery = a5_optional(a5_request(port, id, 2, 0, 0x83, &[]))?;
    let profile = a5_optional(a5_profile(port, id))?;
    let polling = a5_optional(a5_read_polling(port, id))?;
    let dpi = a5_optional(a5_read_dpi(port, id, profile.unwrap_or(1)))?;
    r.online = battery.is_some() || profile.is_some() || polling.is_some() || dpi.is_some();
    if let Some(b) = battery {
        r.battery_percent = Some(b[7].min(100));
        r.charging = Some(b[6] != 0);
    }
    r.polling_hz = polling.flatten();
    r.dpi = dpi.and_then(|(t, active, _)| t.stages.get(active - 1).copied()).filter(|d| *d > 0);
    Ok(r)
}

fn a5_write(m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
    let id = a5_report_id(m);
    let wired = m.wireless == Some(false);
    match s {
        Setting::Polling(hz) => {
            if !a5::rates(wired).contains(&hz) {
                return Err(Error::range("polling rate", format!("{hz} Hz is not supported on this connection")));
            }
            let code = a5::polling_code(hz).ok_or_else(|| Error::range("polling rate", format!("unsupported polling rate {hz}")))?;
            let before = a5_optional(a5_read_polling(port, id))?.flatten();
            a5_request(port, id, 1, 1, 0x00, &[code])?;
            let confirmed = a5_read_polling(port, id)?;
            if confirmed != Some(hz) {
                return Err(Error::BadAnswer("the mouse did not confirm the polling-rate change".into()));
            }
            Ok(Changed { now: hz, before })
        }
        Setting::Dpi(dpi) => {
            if !(a5::DPI_MIN..=a5::DPI_MAX).contains(&dpi) || !(dpi - a5::DPI_MIN).is_multiple_of(a5::DPI_STEP) {
                return Err(Error::range("DPI", format!("{dpi} DPI can't be stored (A5 Pro Max: {}-{} in steps of {})", a5::DPI_MIN, a5::DPI_MAX, a5::DPI_STEP)));
            }
            let profile = a5_profile(port, id)?;
            let (table, active, raw_active) = a5_read_dpi(port, id, profile)?;
            // OpenMouse rewrites the table from what it decoded: a raw stage count of 0 becomes 6, more than 6 is cut to 6, the active
            // stage is clamped, and every stage goes through normalize_dpi (26100 would be stored as 26000). Only a table that
            // survives that unchanged is rewritten here.
            if !(1..=a5::DPI_STAGES).contains(&table.raw_count) || !(1..=table.raw_count).contains(&raw_active) || table.stages.iter().any(|v| a5::normalize_dpi(*v) != *v) {
                return Err(Error::range("DPI", "the stage table of this mouse is not what OpenMouse's write would store back unchanged - it is left alone".to_string()));
            }
            // OpenMouse rewrites every record as [x, x]: only when no record has a different second pair is that lossless
            if table.stages.iter().zip(&table.second).any(|(x, y)| x != y) {
                return Err(Error::range("DPI", "a stage of this mouse has different X and Y DPI - the stage table is left alone".to_string()));
            }
            let mut stages = table.stages.clone();
            stages[active - 1] = dpi;
            let data = a5::encode_dpi(profile, &stages);
            a5_request(port, id, data.len() as u8, 1, 0x01, &data)?;
            a5_request(port, id, 2, 1, 0x02, &[profile, active as u8])?;
            let (confirmed, _, _) = a5_read_dpi(port, id, profile)?;
            let expected: Vec<u32> = stages.iter().map(|v| a5::normalize_dpi(*v)).collect();
            if confirmed.stages != expected {
                return Err(Error::BadAnswer("the mouse did not confirm the DPI change".into()));
            }
            Ok(Changed { now: confirmed.stages[active - 1], before: Some(table.stages[active - 1]) })
        }
        Setting::LiftOff(t) => Err(Error::range("lift-off distance", format!("{t} tenths of a mm: lift-off is not set on a Mchose mouse here (OpenMouse names no millimetres)"))),
    }
}

// ============================================================================ the driver

/// The Mchose driver (`Matched.variant & 0xFF` says which client).
pub struct Mchose;

impl crate::family::Driver for Mchose {
    fn read(&self, m: &Matched, port: &mut dyn Port, opt: ReadOptions) -> Result<OnMouse> {
        match m.variant & 0xFF {
            CLIENT_V2 => v2_read(m, port, opt),
            CLIENT_A5 => a5_read(m, port),
            v => Err(Error::UnsupportedMouse(format!("Mchose client {v:#06x}"))),
        }
    }
    fn write(&self, m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        match m.variant & 0xFF {
            CLIENT_V2 => v2_write(m, port, s),
            CLIENT_A5 => a5_write(m, port, s),
            v => Err(Error::UnsupportedMouse(format!("Mchose client {v:#06x}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An A7 V2 read request as Windows sends it: the report id, the inverted command, inverted zero tokens up to the channel's
    /// count, then zeros up to the collection's feature-report length. Used by the allow-list test.
    fn v2_read_frame(report_id: u8, command: u8, tokens: usize, len: usize) -> Vec<u8> {
        v2::feature_report(report_id, &v2::encode_command(&[command], tokens), len).expect("len checked by the caller")
    }


    #[test]
    fn the_read_frames_pass_the_allow_list() {
        for (rid, cmd, n) in [(0x11, 0x03, 20), (0x11, 0x06, 20), (0x12, 0x67, 64)] {
            assert!(read_request_allowed(&v2_read_frame(rid, cmd, n, 65)), "{cmd:02x}");
            assert!(read_request_allowed(&v2_read_frame(rid, cmd, n, 70)), "{cmd:02x} behind a longer collection");
        }
        assert!(!read_request_allowed(&v2_read_frame(0x11, 0x57, 20, 65)));
        assert!(!read_request_allowed(&v2_read_frame(0x12, 0x03, 20, 65)), "identity belongs to the short channel");
    }
}

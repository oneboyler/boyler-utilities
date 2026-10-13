//! Orbital (Ghost / Pathfinder V2, wired) - 64-byte reports on the vendor config collection, ported from OpenMouse
//! (`crate::family`: AGPL-3.0-or-later): DPI and polling rate only.
//!
//! Ported from `orbital/index.js` (device table, report rates, the checksum `orbitalFinishPacket`),
//! `drivers/orbital/hid.js` (OrbitalHidClient: readStatus / setDpi / setPollingRate) and `drivers/orbital/host-protocol.js`
//! (OrbitalMouseClient: detectProtocol, readDmsV2Settings, writeDmsV2Settings, saveSettings).
//!
//! Packet: OpenMouse sends `sendReport(0, 64 bytes)` and takes the first input report that satisfies a per-request check.
//! Windows puts report id 0 in front, so OpenMouse's byte i is our byte i + 1 on the wire (the codec here works on OpenMouse's
//! 64 bytes, without the id). `[0]` command family (0x40 = "route through the receiver" - only the receiver ids use it, and
//! they are not on OpenMouse's list) · `[2]` opcode · `[3]` sub command · `[4..63]` payload · `[63]` checksum
//! = `(0xA1 - (sum of bytes 0..62)) & 0xFF`.
//!
//! Requests this file sends: handshake `01 00 81 01` (answer: first byte 1) · sensor read `04 00 81 01` and system read
//! `04 00 83 03` (answer: first byte 4, byte 3 = the sub command) · for a change the sensor write `04 00 BC 02` (DPI) or the
//! system write `04 00 90 04` (polling), each followed by the save `0A 00 81 01` (answer: first byte 10).
//!
//! Safety rule for the rewrite: OpenMouse writes a WHOLE block (sensor: toggles, lift-off, angle, stage mask, five X/Y stages,
//! five stage colours; system: sleep, rate, debounce, wake flags, ...) and fills the bytes it does not edit from what it read.
//! Bytes it does not know (sensor `35..43`, `58..63`; system `20..63`) it writes as zero, and it normalises some values
//! (50-DPI rounding of every stage, sleep clamped to 1.., the BLE sleep time overwritten with the sleep time, a stage mask
//! rewritten as "the first n stages"). Before writing, this file builds the block OpenMouse would write for "no change" and
//! compares it with the block the mouse just returned; any difference (an unknown non-zero byte, a different BLE sleep time,
//! ...) means the rewrite would silently change a saved value, and the change is refused with nothing written. It has never
//! been run on a real unit, so whether a real Ghost passes that check is unclear.
//!
//! Only the system block is written for a polling change and only the sensor block for a DPI change (OpenMouse writes both
//! every time; the one it would not change is byte-identical to the stored one, as the check above proves).
//!
//! Order 110 (A_110_02 scope): debounce (0-30 ms: mergeEditableSettings' clamp), Motion Sync, angle snapping and ripple control:
//! OpenMouse's setDebounceTime / setMotionSync / setAngleSnapping / setRippleControl = `apply({...})`, through the same
//! read, rewrite check, block write, save and read-back as DPI / polling (debounce = the system block, the switches = the
//! sensor block). Its setPerformanceMode is the sensor's maxSpeedMode, not the Pulsar turbo: left out.
//!
//! Left out on purpose: lift-off (OpenMouse's three levels Low / Medium / High have no millimetre figure), battery (the
//! wired mouse has none; OpenMouse reads it for the receiver only), the receiver ids (0x080b, 0x0746), the V1 mouse
//! (0x0747, "dms" protocol) - none of them is on OpenMouse's SUPPORTED list - profiles, sleep, angle tuning, performance
//! modes, wheel reverse, lighting-colour, factory reset (`09 .. 81 FF`; never sent).

use crate::error::{Error, Result};
use crate::family::{check_extra, not_offered, Caps, Change as FChange, Changed, DpiCaps, Driver, Extra, ExtraRow, Family, Kind, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::{HidInfo, HidTransfer};

pub const VENDOR_ID: u16 = 0x1915;
/// Ghost / Pathfinder V2 (wired, protocol "dms_v2").
pub const GHOST_PID: u16 = 0x080C;
pub const MODEL: &str = "Orbital Ghost / Pathfinder V2";
pub const REPORT_ID: u8 = 0;
pub const PACKET_LEN: usize = 64;
/// Windows: the report id + the packet.
pub const REPORT_LEN: usize = PACKET_LEN + 1;
pub const USAGE_PAGE: u16 = 0xFF0A;
pub const USAGE: u16 = 0x01;
/// ORBITAL_REPORT_RATES_HZ (the Ghost's maxReportRateIndex is 5: all six).
pub const RATES_HZ: [u32; 6] = [125, 500, 1000, 2000, 4000, 8000];
/// ORBITAL_DPI_LIMITS.
pub const DPI_MIN: u32 = 50;
pub const DPI_MAX: u32 = 30000;
pub const DPI_STEP: u32 = 50;

const TIMEOUT_MS: u32 = 1800;
const HANDSHAKE_TIMEOUT_MS: u32 = 2500;

/// One OpenMouse packet (no report id).
pub type Packet = [u8; PACKET_LEN];

// ---------------------------------------------------------------- packets

/// orbitalFinishPacket (no routing): the checksum byte.
pub fn finish(p: &mut Packet) {
    let sum: u32 = p[..PACKET_LEN - 1].iter().map(|b| *b as u32).sum();
    p[PACKET_LEN - 1] = 0xA1u8.wrapping_sub((sum & 0xFF) as u8);
}

fn packet(family: u8, opcode: u8, sub: u8) -> Packet {
    let mut p = [0u8; PACKET_LEN];
    p[0] = family;
    p[2] = opcode;
    p[3] = sub;
    p
}

/// detectProtocol: `[0]=1 [2]=0x81 [3]=1` (answered by any report whose first byte is 1).
pub fn handshake() -> Packet {
    let mut p = packet(1, 0x81, 1);
    finish(&mut p);
    p
}

/// readDmsV2Settings, sensor/DPI block: `[0]=4 [2]=0x81 [3]=1`.
pub fn read_sensor() -> Packet {
    let mut p = packet(4, 0x81, 1);
    finish(&mut p);
    p
}

/// readDmsV2Settings, system block: `[0]=4 [2]=0x83 [3]=3`.
pub fn read_system() -> Packet {
    let mut p = packet(4, 0x83, 3);
    finish(&mut p);
    p
}

/// saveSettings: `[0]=10 [2]=0x81 [3]=1`.
pub fn save() -> Packet {
    let mut p = packet(10, 0x81, 1);
    finish(&mut p);
    p
}

// ---------------------------------------------------------------- the settings (DMS v2)

/// What readDmsV2Settings returns (the fields the two write packets carry).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub angle_snapping: bool,
    pub ripple_control: bool,
    pub motion_sync: bool,
    /// fullSpeedMode ("overclocked")
    pub full_speed: bool,
    /// maxSpeedMode ("high performance")
    pub max_speed: bool,
    pub lift_down: bool,
    pub glass_mode: bool,
    /// lift-off level 0, 1 or 2 (not millimetres)
    pub lift_off: u8,
    /// signedByte(): `<= 30` as is, else minus 256
    pub angle: i32,
    /// zero-based, clamped to 0..=4
    pub active_stage: u8,
    /// countSetBits of the stage mask, or 5 when it is zero
    pub enabled_stages: u8,
    pub stages: [(u16, u16); 5],
    pub colors: [[u8; 3]; 5],
    pub sleep: u16,
    pub ble_sleep: u16,
    /// 0 for a raw rate value of 0 or 1, else raw - 1
    pub rate_index: usize,
    pub debounce: u8,
    pub quick_response: bool,
    pub button_wakeup: bool,
    pub move_wakeup: bool,
    pub wheel_wakeup: bool,
    pub wheel_reverse: bool,
    pub ir_button_mode: u8,
    pub left_key_mode: u8,
    pub right_key_mode: u8,
    pub rf_power_mode: u8,
    pub ble_num: u8,
}

impl Settings {
    pub fn dpi(&self) -> u32 {
        self.stages[self.active_stage as usize].0 as u32
    }
    pub fn polling_hz(&self) -> Option<u32> {
        RATES_HZ.get(self.rate_index).copied()
    }
}

fn u16le(b: &[u8], o: usize) -> u16 {
    b[o] as u16 | (b[o + 1] as u16) << 8
}

fn signed_byte(v: u8) -> i32 {
    if v <= 30 {
        v as i32
    } else {
        v as i32 - 256
    }
}

/// readDmsV2Settings: the two answers (OpenMouse's bytes, 64 each) → the settings. `None` = an answer too short.
pub fn decode_settings(sensor: &[u8], system: &[u8]) -> Option<Settings> {
    if sensor.len() < PACKET_LEN || system.len() < PACKET_LEN {
        return None;
    }
    let (s, y) = (&sensor[4..], &system[4..]);
    let mut stages = [(0u16, 0u16); 5];
    let mut colors = [[0u8; 3]; 5];
    for i in 0..5 {
        stages[i] = (u16le(s, 11 + i * 4), u16le(s, 13 + i * 4));
        colors[i] = [s[39 + i * 3], s[40 + i * 3], s[41 + i * 3]];
    }
    let bits = s[10].count_ones() as u8;
    Some(Settings {
        angle_snapping: s[0] != 0,
        ripple_control: s[1] != 0,
        motion_sync: s[2] != 0,
        full_speed: s[3] != 0,
        max_speed: s[4] != 0,
        lift_down: s[5] != 0,
        glass_mode: s[6] != 0,
        lift_off: s[7],
        angle: signed_byte(s[8]),
        active_stage: s[9].min(4),
        enabled_stages: if bits == 0 { 5 } else { bits },
        stages,
        colors,
        sleep: u16le(y, 0),
        ble_sleep: u16le(y, 2),
        rate_index: if y[4] > 1 { y[4] as usize - 1 } else { 0 },
        debounce: y[5],
        quick_response: y[6] != 0,
        button_wakeup: y[7] != 0,
        move_wakeup: y[8] != 0,
        wheel_wakeup: y[9] != 0,
        wheel_reverse: y[10] != 0,
        ir_button_mode: y[11],
        left_key_mode: y[12],
        right_key_mode: y[13],
        rf_power_mode: y[14],
        ble_num: y[15],
    })
}

/// What a click changes (OpenMouse's `changes` object).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// nothing: the block as OpenMouse would rewrite it untouched
    None,
    /// setDpi(dpi): the active stage becomes `[dpi, dpi]`
    Dpi(u32),
    /// setPollingRate: reportRateIndex
    Rate(usize),
    /// Order 110: setDebounceTime: debounceMs
    Debounce(u8),
    /// setMotionSync / setAngleSnapping / setRippleControl
    MotionSync(bool),
    AngleSnap(bool),
    Ripple(bool),
}

/// mergeEditableSettings' result, as far as the two packets use it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Next {
    pub stages: [(u16, u16); 5],
    pub enabled_stages: u8,
    pub active_stage: u8,
    pub rate_index: usize,
    pub sleep: u16,
    pub debounce: u8,
    pub lift_off: u8,
    pub angle: i32,
    pub angle_snapping: bool,
    pub ripple_control: bool,
    pub motion_sync: bool,
    pub high_performance: bool,
    pub overclocked: bool,
    pub quick_response: bool,
    pub wheel_reverse: bool,
}

/// normalizeDpi: Math.round(v / 50) * 50, clamped to 50..=30000.
pub fn normalize_dpi(v: u32) -> u16 {
    ((v + DPI_STEP / 2) / DPI_STEP * DPI_STEP).clamp(DPI_MIN, DPI_MAX) as u16
}

/// mergeEditableSettings(current, changes) for the Ghost (maxReportRateIndex 5).
pub fn merge(cur: &Settings, change: Change) -> Next {
    let mut stages = cur.stages;
    let mut rate_index = cur.rate_index;
    let (mut debounce, mut angle_snapping, mut ripple_control, mut motion_sync) = (cur.debounce, cur.angle_snapping, cur.ripple_control, cur.motion_sync);
    match change {
        Change::None => {}
        Change::Dpi(d) => stages[cur.active_stage as usize] = (normalize_dpi(d), normalize_dpi(d)),
        Change::Rate(i) => rate_index = i,
        Change::Debounce(v) => debounce = v,
        Change::MotionSync(b) => motion_sync = b,
        Change::AngleSnap(b) => angle_snapping = b,
        Change::Ripple(b) => ripple_control = b,
    }
    for st in stages.iter_mut() {
        *st = (normalize_dpi(st.0 as u32), normalize_dpi(st.1 as u32));
    }
    let enabled = cur.enabled_stages.clamp(1, 5);
    Next {
        stages,
        enabled_stages: enabled,
        active_stage: cur.active_stage.min(enabled - 1),
        rate_index: rate_index.min(5),
        sleep: cur.sleep.max(1),
        debounce: debounce.min(30),
        lift_off: cur.lift_off.min(2),
        angle: cur.angle.clamp(-30, 30),
        angle_snapping,
        ripple_control,
        motion_sync,
        high_performance: cur.max_speed,
        overclocked: cur.full_speed,
        quick_response: cur.quick_response,
        wheel_reverse: cur.wheel_reverse,
    }
}

/// encodeReportRate: index > 0 → index + 1, else 0.
fn encode_rate(index: usize) -> u8 {
    if index > 0 {
        index as u8 + 1
    } else {
        0
    }
}

fn put_u16(p: &mut Packet, o: usize, v: u16) {
    p[o] = v as u8;
    p[o + 1] = (v >> 8) as u8;
}

/// writeDmsV2Settings, sensor packet `04 00 BC 02`: toggles `[4..11]`, lift-off, angle, active stage, stage mask, five X/Y
/// pairs from `[15]`, the stage colours (copied from the read) from `[43]`.
pub fn sensor_write(next: &Next, prev: &Settings) -> Packet {
    let mut p = packet(4, 0xBC, 2);
    p[4] = next.angle_snapping as u8;
    p[5] = next.ripple_control as u8;
    p[6] = next.motion_sync as u8;
    p[7] = next.overclocked as u8;
    p[8] = next.high_performance as u8;
    p[9] = prev.lift_down as u8;
    p[10] = prev.glass_mode as u8;
    p[11] = next.lift_off;
    p[12] = (next.angle & 0xFF) as u8;
    p[13] = next.active_stage;
    p[14] = (1u8 << next.enabled_stages) - 1;
    for (i, (x, y)) in next.stages.iter().enumerate() {
        put_u16(&mut p, 15 + i * 4, *x);
        put_u16(&mut p, 17 + i * 4, *y);
    }
    for (i, rgb) in prev.colors.iter().enumerate() {
        p[43 + i * 3..46 + i * 3].copy_from_slice(rgb);
    }
    finish(&mut p);
    p
}

/// writeDmsV2Settings, system packet `04 00 90 04`: sleep (twice), rate, debounce, quick response, three wake flags, wheel
/// reverse, four pass-through bytes and the BLE number (copied from the read).
pub fn system_write(next: &Next, prev: &Settings) -> Packet {
    let mut p = packet(4, 0x90, 4);
    put_u16(&mut p, 4, next.sleep);
    put_u16(&mut p, 6, next.sleep);
    p[8] = encode_rate(next.rate_index);
    p[9] = next.debounce;
    p[10] = next.quick_response as u8;
    p[11] = prev.button_wakeup as u8;
    p[12] = prev.move_wakeup as u8;
    p[13] = prev.wheel_wakeup as u8;
    p[14] = next.wheel_reverse as u8;
    p[15] = prev.ir_button_mode;
    p[16] = prev.left_key_mode;
    p[17] = prev.right_key_mode;
    p[18] = prev.rf_power_mode;
    p[19] = prev.ble_num;
    finish(&mut p);
    p
}

/// Would OpenMouse's rewrite of this block, with nothing changed, give back exactly the bytes the mouse returned (payload
/// `[4..63]`; byte 63 is each packet's own checksum)? `false` = the rewrite would alter a saved value.
pub fn rewrite_is_lossless(cur: &Settings, sensor: &[u8], system: &[u8]) -> (bool, bool) {
    let same = merge(cur, Change::None);
    let ok = |built: Packet, answer: &[u8]| answer.len() >= PACKET_LEN && built[4..PACKET_LEN - 1] == answer[4..PACKET_LEN - 1];
    (ok(sensor_write(&same, cur), sensor), ok(system_write(&same, cur), system))
}

// ---------------------------------------------------------------- the device table + the allow-list

/// OpenMouse's isSupported: Orbital's vendor id, the Ghost / Pathfinder V2 product id, a collection on usage page 0xFF0A /
/// usage 1 (the DMS config interface) that carries 64-byte reports both ways.
pub fn matches(own: &[&HidInfo], vid: u16, pid: u16) -> Option<Matched> {
    if vid != VENDOR_ID || pid != GHOST_PID {
        return None;
    }
    let c = own.iter().find(|h| h.usage_page == USAGE_PAGE && h.usage == USAGE && h.output_len as usize >= REPORT_LEN && h.input_len as usize >= REPORT_LEN)?;
    Some(Matched { family: Family::Orbital, model: Some(MODEL), wireless: Some(false), config: (*c).clone(), variant: pid })
}

/// The allow-list for the read-only proof on a real mouse: report id 0, then exactly the handshake, the sensor read or the
/// system read (checksum right), zeros behind. The save and both write packets are refused.
pub fn read_request_allowed(out: &[u8]) -> bool {
    if out.len() < REPORT_LEN || out[0] != REPORT_ID || out[REPORT_LEN..].iter().any(|b| *b != 0) {
        return false;
    }
    [handshake(), read_sensor(), read_system()].iter().any(|r| r[..] == out[1..REPORT_LEN])
}

// ---------------------------------------------------------------- the exchange

/// One packet → the first input report whose first byte is `reply_family` (OpenMouse's waiters; other reports - the live
/// status ones - are skipped by the OS layer), without the report id. `sub`: the sub command the answer must carry in byte 3.
fn ask(port: &mut dyn Port, p: &Packet, reply_family: u8, sub: Option<u8>, timeout_ms: u32) -> Result<Vec<u8>> {
    let out_len = (port.info().output_len as usize).max(REPORT_LEN);
    let in_len = (port.info().input_len as usize).max(REPORT_LEN);
    let mut out = vec![REPORT_ID];
    out.extend_from_slice(p);
    out.resize(out_len, 0);
    let how = HidTransfer::OutputThenInput { reply_len: in_len, max_reads: 32, timeout_ms, echo: Some((1, reply_family)) };
    let a = port.exchange(&out, &how)?;
    let r = a.get(1..).filter(|r| r.len() >= PACKET_LEN).ok_or_else(|| Error::BadAnswer("an Orbital answer shorter than 64 bytes".into()))?;
    if sub.is_some_and(|s| r[3] != s) {
        return Err(Error::BadAnswer(format!("an Orbital answer for another request (sub command {:#04x})", r[3])));
    }
    Ok(r.to_vec())
}

/// `.catch(() => null)`: no answer in time = `None`; a mouse that is gone still fails.
fn optional<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(Error::BadAnswer(_) | Error::Os { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// readSettings: the sensor block and the system block (their raw answers kept for the rewrite check).
fn read_blocks(port: &mut dyn Port) -> Result<(Settings, Vec<u8>, Vec<u8>)> {
    let sensor = ask(port, &read_sensor(), 4, Some(1), TIMEOUT_MS)?;
    let system = ask(port, &read_system(), 4, Some(3), TIMEOUT_MS)?;
    let s = decode_settings(&sensor, &system).ok_or_else(|| Error::BadAnswer("the Orbital settings answers were too short".into()))?;
    Ok((s, sensor, system))
}

fn save_settings(port: &mut dyn Port) -> Result<()> {
    ask(port, &save(), 10, None, TIMEOUT_MS).map(|_| ())
}

/// readStatus' debounceMs and sensor switches, in the card's order (`None` = no values: the guard's rows).
pub fn extras(s: Option<&Settings>) -> Vec<ExtraRow> {
    vec![
        ExtraRow::new(Extra::Debounce, Kind::Range { min: 0, max: 30, step: 1, unit: "ms" }, s.map(|s| s.debounce as i32)),
        ExtraRow::switch(Extra::MotionSync, s.map(|s| s.motion_sync)),
        ExtraRow::switch(Extra::AngleSnap, s.map(|s| s.angle_snapping)),
        ExtraRow::switch(Extra::RippleControl, s.map(|s| s.ripple_control)),
    ]
}

pub fn caps() -> Caps {
    Caps { dpi: Some(DpiCaps::uniform(DPI_MIN, DPI_MAX, DPI_STEP)), polling: RATES_HZ.to_vec(), lift_off: Vec::new(), battery: false }
}

// ---------------------------------------------------------------- the driver

/// The Orbital driver.
pub struct Orbital;

impl Driver for Orbital {
    /// OpenMouse's readStatus for the wired mouse: the handshake (no answer = offline), then both settings blocks.
    fn read(&self, _m: &Matched, port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        let mut r = OnMouse { wireless: Some(false), caps: caps(), ..Default::default() };
        if optional(ask(port, &handshake(), 1, None, HANDSHAKE_TIMEOUT_MS))?.is_none() {
            return Ok(r);
        }
        r.online = true;
        let (s, _, _) = read_blocks(port)?;
        r.dpi = Some(s.dpi());
        r.polling_hz = s.polling_hz();
        r.extras = extras(Some(&s));
        Ok(r)
    }

    /// Order 110: apply({debounceMs | motionSync | angleSnapping | rippleControl}) - checked before a byte leaves, then both
    /// blocks read, the rewrite check of the block it changes, that block written, the save, the read-back.
    fn change(&self, _m: &Matched, port: &mut dyn Port, c: FChange) -> Result<()> {
        let FChange::Extra(e, v) = c;
        check_extra(&extras(None), e, v)?;
        let change = match e {
            Extra::Debounce => Change::Debounce(v as u8),
            Extra::MotionSync => Change::MotionSync(v == 1),
            Extra::AngleSnap => Change::AngleSnap(v == 1),
            Extra::RippleControl => Change::Ripple(v == 1),
            Extra::Turbo => return Err(not_offered(c)),
        };
        let (cur, sensor, system) = read_blocks(port)?;
        let (sensor_ok, system_ok) = rewrite_is_lossless(&cur, &sensor, &system);
        let next = merge(&cur, change);
        let (packet, ok) = if e == Extra::Debounce { (system_write(&next, &cur), system_ok) } else { (sensor_write(&next, &cur), sensor_ok) };
        if !ok {
            return Err(Error::BadAnswer("this Orbital mouse holds settings OpenMouse's way of saving would change (unknown bytes, or two different sleep times) - nothing was written".into()));
        }
        ask(port, &packet, 4, Some(packet[3]), TIMEOUT_MS)?;
        save_settings(port)?;
        let (now, _, _) = read_blocks(port)?;
        let kept = extras(Some(&now)).into_iter().find(|r| r.extra == e).and_then(|r| r.value);
        if kept != Some(v) {
            return Err(Error::BadAnswer(format!("the mouse kept {kept:?} instead of {v} ({})", e.label())));
        }
        Ok(())
    }

    /// setDpi / setPollingRate: read both blocks, the rewrite check, the one block write, the save, the read-back.
    fn write(&self, _m: &Matched, port: &mut dyn Port, s: Setting) -> Result<Changed> {
        let change = match s {
            Setting::Dpi(d) if caps().dpi.is_some_and(|c| c.allows(d)) => Change::Dpi(d),
            Setting::Dpi(d) => return Err(Error::range("DPI", format!("{d} DPI is not one this Orbital mouse can store (50 - 30000 in steps of 50)"))),
            Setting::Polling(hz) => match RATES_HZ.iter().position(|r| *r == hz) {
                Some(i) => Change::Rate(i),
                None => return Err(Error::range("polling rate", format!("{hz} Hz is not an Orbital polling rate"))),
            },
            Setting::LiftOff(_) => return Err(Error::range("lift-off distance", "this app can't set an Orbital mouse's lift-off (its levels have no millimetre figure)".to_string())),
        };
        let (cur, sensor, system) = read_blocks(port)?;
        let (sensor_ok, system_ok) = rewrite_is_lossless(&cur, &sensor, &system);
        let next = merge(&cur, change);
        let (packet, ok, before) = match change {
            Change::Dpi(_) => (sensor_write(&next, &cur), sensor_ok, Some(cur.dpi())),
            _ => (system_write(&next, &cur), system_ok, cur.polling_hz()),
        };
        if !ok {
            return Err(Error::BadAnswer("this Orbital mouse holds settings OpenMouse's way of saving would change (unknown bytes, or two different sleep times) - nothing was written".into()));
        }
        ask(port, &packet, 4, Some(packet[3]), TIMEOUT_MS)?;
        save_settings(port)?;
        let (now, _, _) = read_blocks(port)?;
        match change {
            Change::Dpi(d) if now.stages[cur.active_stage as usize] != (d as u16, d as u16) => {
                Err(Error::BadAnswer(format!("the mouse kept {} DPI instead of {d}", now.dpi())))
            }
            Change::Rate(i) if now.rate_index != i => Err(Error::BadAnswer(format!("the mouse kept {:?} Hz instead of {} Hz", now.polling_hz(), s.value()))),
            _ => Ok(Changed { now: s.value(), before }),
        }
    }
}

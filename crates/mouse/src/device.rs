//! "Your mouse" (DESIGN §3.4): identify the connected mouse by its USB ids; for a supported mouse read / set DPI, polling
//! rate and lift-off distance (+ battery) through its vendor HID protocol; any other mouse gets its brand's web-settings link.
//!
//! Supported now: the Pulsar X2 CrazyLight family ("cMouse" protocol, `crate::pulsar`) on VID 0x3710 with PID 0x5406 (8K
//! dongle) or 0x3414 (cable) — the two ids the sources confirm. The test PC (measured, read-only device list): 3710:5406.
//!
//! Calls made for what DESIGN left unclear (also in the report):
//! - The DPI chips / Custom field set the DPI of the mouse's ACTIVE stage (both axes). The other stages are untouched.
//! - Battery "about N days left" is not built: no source gives a drain rate (only percent / charging / millivolts).
//! - Lift-off shows 0.7 mm too when the mouse has it (the mouse's third value); the chips stay 1 mm / 2 mm (DESIGN).

use crate::error::{Error, Result};
use crate::os::{HidInfo, HidTransfer, MouseOs};
use crate::pulsar::{self as p, Link};
use crate::service::{Mouse, UndoKey, UndoValue};

/// DPI chips (DESIGN).
pub const DPI_CHIPS: [u32; 4] = [400, 800, 1600, 3200];
/// "Custom" field: any DPI the mouse can store, 50–26000 (Order 042: every step the mouse stores, e.g. 1230; the top stays the
/// X2-class sensors' 26000 - the codec goes to 32000, the sensor doesn't).
pub const DPI_RANGE: (u32, u32) = (50, 26000);
pub const DPI_STEP: u32 = 50;
/// Polling chips (DESIGN).
pub const POLLING_CHIPS: [u32; 4] = [1000, 2000, 4000, 8000];
/// Lift-off chips in tenths of a mm (DESIGN: 1 mm / 2 mm).
pub const LIFT_OFF_CHIPS: [u32; 2] = [10, 20];

/// Typed DPI → what the mouse gets: the nearest value the mouse can store (Pulsar's own steps, `pulsar::encode_dpi_stage`:
/// 10 DPI up to 10240, 50 up to 25600, 100 up to 32000), clamped to 50–26000 (1234 → 1230, 2400 → 2400, 12345 → 12350).
/// ↑/↓ and the wheel step ±50.
pub fn custom_dpi(typed: u32) -> u32 {
    let t = typed.clamp(DPI_RANGE.0, DPI_RANGE.1);
    let step = if t <= 10240 {
        10
    } else if t <= 25600 {
        50
    } else {
        100
    };
    ((t + step / 2) / step * step).clamp(DPI_RANGE.0, DPI_RANGE.1)
}

/// The lit DPI chip (a typed value lights no chip).
pub fn dpi_chip(dpi: u32) -> Option<usize> {
    DPI_CHIPS.iter().position(|c| *c == dpi)
}

/// A brand and its web-settings page (for the link on the right, and the whole row of an unsupported mouse).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Brand {
    pub name: &'static str,
    pub settings_url: &'static str,
}

/// Brand by USB vendor id — only ids that belong to ONE brand (shared chip / ODM ids like 0x093A, 0x258A, 0x1915, 0x3554,
/// 0x25A7 are left out: they would name the wrong brand). Source: the research notes (OpenMouse vendor list, usb.ids,
/// the brands' pages); pages marked unverified there are not used.
pub fn brand(vid: u16) -> Option<Brand> {
    let (name, settings_url) = match vid {
        0x3710 => ("Pulsar", "https://bbb.pulsar.gg/"),
        0x046D => ("Logitech", "https://www.logitechg.com/innovation/g-hub"),
        0x1532 => ("Razer", "https://synapse.razer.com/dashboard"),
        0x1038 => ("SteelSeries", "https://steelseries.com/gg"),
        0x373E | 0x37B0 => ("Lamzu", "https://lamzu.net/"),
        0x361D => ("Finalmouse", "https://xpanel.finalmouse.com/"),
        0x1B1C => ("Corsair", "https://www.corsair.com/us/en/s/icue"),
        0x3057 => ("VAXEE", "https://vcc.vaxee.cn/index.php"),
        0x373B => ("ATK", "https://hub.atk.pro/"),
        0x33E4 => ("G-Wolves", "https://www.mouse.fit/"),
        _ => return None,
    };
    Some(Brand { name, settings_url })
}

/// Which protocol a supported mouse speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    /// Pulsar cMouse 17-byte (X2 CrazyLight family)
    PulsarCmouse,
}

/// The supported (vid, pid) list → (protocol, name from the id, wired).
pub fn supported(vid: u16, pid: u16) -> Option<(Protocol, &'static str, bool)> {
    match (vid, pid) {
        (0x3710, 0x5406) => Some((Protocol::PulsarCmouse, "Pulsar X2 CrazyLight", false)),
        (0x3710, 0x3414) => Some((Protocol::PulsarCmouse, "Pulsar X2 CrazyLight", true)),
        _ => None,
    }
}

/// The connected mouse as found from the device list (nothing sent to it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct YourMouse {
    /// "Pulsar X2 CrazyLight", else the product string, else "Mouse (VID:PID)"
    pub name: String,
    pub vid: u16,
    pub pid: u16,
    pub brand: Option<Brand>,
    /// `Some` = DPI / polling / lift-off can be read and set
    pub protocol: Option<Protocol>,
    /// the interface the app talks to (vendor collection), for a supported mouse
    pub config_path: Option<String>,
    pub output_len: u16,
    pub input_len: u16,
    /// from the id: dongle = wireless, cable = wired (refined by the identify answer)
    pub wireless: Option<bool>,
}

impl YourMouse {
    /// The small line under the name.
    pub fn sub_line(&self) -> &'static str {
        match (self.protocol.is_some(), self.wireless) {
            (true, Some(true)) => "Wireless · saved on the mouse itself",
            (true, _) => "Saved on the mouse itself",
            (false, _) => "DPI and polling for this mouse aren't supported yet",
        }
    }

    /// The link on the right: "Open <Brand> web settings" (supported) / "open its web settings" (unsupported).
    pub fn link(&self) -> Option<(String, &'static str)> {
        let b = self.brand.as_ref()?;
        Some(if self.protocol.is_some() { (format!("Open {} web settings", b.name), b.settings_url) } else { ("open its web settings".into(), b.settings_url) })
    }
}

/// Every mouse in a device list (one entry per vid:pid that has a mouse collection, usage page 1 / usage 2), supported
/// mice first and among them the cable before the dongle (a dongle whose mouse is on its cable answers nothing).
pub fn find_mice(hid: &[HidInfo]) -> Vec<YourMouse> {
    let mut ids: Vec<(u16, u16)> = hid.iter().filter(|h| h.usage_page == 0x01 && h.usage == 0x02 && h.vid != 0).map(|h| (h.vid, h.pid)).collect();
    ids.sort();
    ids.dedup();
    let mut out: Vec<YourMouse> = ids
        .into_iter()
        .map(|(vid, pid)| {
            let sup = supported(vid, pid);
            let product = hid.iter().filter(|h| h.vid == vid && h.pid == pid).find_map(|h| h.product.clone());
            let config = sup.and_then(|_| config_interface(hid, vid, pid));
            YourMouse {
                name: sup.map(|s| s.1.to_string()).or(product).unwrap_or_else(|| format!("Mouse ({vid:04X}:{pid:04X})")),
                vid,
                pid,
                brand: brand(vid),
                protocol: config.and_then(|_| sup.map(|s| s.0)),
                config_path: config.map(|c| c.path.clone()),
                output_len: config.map(|c| c.output_len).unwrap_or(0),
                input_len: config.map(|c| c.input_len).unwrap_or(0),
                wireless: sup.map(|s| !s.2),
            }
        })
        .collect();
    out.sort_by_key(|m| (m.protocol.is_none(), m.wireless == Some(true)));
    out
}

/// The vendor collection the cMouse protocol uses: vendor usage page 0xFF02 with output AND input reports of ≥ 17 bytes
/// (Windows: interface 1, `col05`). Falls back to any 0xFFxx collection of interface 1 with ≥ 17-byte reports.
fn config_interface(hid: &[HidInfo], vid: u16, pid: u16) -> Option<&HidInfo> {
    let fits = |h: &&HidInfo| h.vid == vid && h.pid == pid && h.output_len as usize >= p::FRAME_LEN && h.input_len as usize >= p::FRAME_LEN;
    hid.iter().filter(fits).find(|h| h.usage_page == 0xFF02).or_else(|| hid.iter().filter(fits).find(|h| h.usage_page >= 0xFF00 && h.interface == Some(1)))
}

/// What a supported mouse reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OnMouse {
    /// from the identify answer, when the model code is known
    pub model: Option<&'static str>,
    pub family: u8,
    pub model_code: u8,
    pub link: Link,
    /// the mouse is awake and linked to the dongle
    pub online: bool,
    pub battery_percent: Option<u8>,
    pub charging: Option<bool>,
    pub battery_mv: Option<u16>,
    /// active DPI stage (0-based) and its DPI (x, y)
    pub stage: Option<u8>,
    pub dpi: Option<(u32, u32)>,
    pub polling_hz: Option<u32>,
    /// tenths of a millimetre (10 = 1 mm, 20 = 2 mm, 7 = 0.7 mm)
    pub lift_off: Option<u32>,
}

/// How hard to try reading a sleeping mouse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadOptions {
    /// how many times to ask "online?" ([`ONLINE_POLL_MS`] apart) before giving up (the proof run uses 1: A_009_01 "do
    /// not loop")
    pub online_tries: u32,
}

/// The pause between two online polls. With the default tries a waking mouse gets ~3 s to link (PulsarBattery waits 3 s,
/// 20 ms apart, before every settings session).
pub const ONLINE_POLL_MS: u64 = 20;
/// The longest the online poll (and the write hold) waits, answers or not.
pub const ONLINE_WAIT: std::time::Duration = std::time::Duration::from_millis(3000);

impl Default for ReadOptions {
    fn default() -> Self {
        Self { online_tries: 150 }
    }
}

/// Retries of one settings read / write over a wireless link that drops a frame now and then.
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

impl<O: MouseOs> Mouse<O> {
    /// The connected mice, best first (`[0]` is "Your mouse"). Read-only: nothing is sent to any device.
    pub fn mice(&self) -> Result<Vec<YourMouse>> {
        Ok(find_mice(&self.os.hid_devices()?))
    }

    fn send(&mut self, m: &YourMouse, req: [u8; p::FRAME_LEN]) -> Result<Vec<u8>> {
        let path = m.config_path.clone().ok_or_else(|| Error::UnsupportedMouse(m.name.clone()))?;
        // Windows wants exactly OutputReportByteLength bytes, zero-padded.
        let mut out = req.to_vec();
        out.resize((m.output_len as usize).max(p::FRAME_LEN), 0);
        let how = HidTransfer::OutputThenInput { reply_len: (m.input_len as usize).max(p::FRAME_LEN), max_reads: 64, timeout_ms: 1000, echo: Some((1, req[1])) };
        let a = self.os.hid_exchange(&path, &out, &how)?;
        p::check_answer(req[1], &a).map_err(Error::BadAnswer)?;
        Ok(a)
    }

    fn read_once(&mut self, m: &YourMouse, addr: u16, len: u8) -> Result<Vec<u8>> {
        let a = self.send(m, p::read_frame(addr, len))?;
        if a[2] != 0 || u16::from_be_bytes([a[3], a[4]]) != addr {
            return Err(Error::BadAnswer(format!("read {addr:#06x}: status {} / address {:#04x}{:02x}", a[2], a[3], a[4])));
        }
        Ok(a[6..6 + len.min(10) as usize].to_vec())
    }

    /// One settings read, asked again (up to [`TRIES`]) when a frame got lost or garbled on the way.
    fn read_mem(&mut self, m: &YourMouse, addr: u16, len: u8) -> Result<Vec<u8>> {
        let mut last = None;
        for _ in 0..TRIES {
            match self.read_once(m, addr, len) {
                Ok(v) => return Ok(v),
                Err(e @ Error::BadAnswer(_)) => last = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| Error::BadAnswer(format!("read {addr:#06x}"))))
    }

    /// Asks "is the mouse online?" (the QUERY form - the length-1 form is the write hold, whose answer says nothing about
    /// the mouse) until it is online and not busy, [`ONLINE_POLL_MS`] apart: a mouse that just woke up takes a moment to
    /// link to its dongle.
    fn online(&mut self, m: &YourMouse, tries: u32) -> Result<bool> {
        // and at most ONLINE_WAIT in all: a dongle that does not answer at all (1 s per try) must not hold the worker -
        // and its HID traffic after the tab closed - for minutes
        let t0 = std::time::Instant::now();
        for i in 0..tries.max(1) {
            if i > 0 {
                if t0.elapsed() >= ONLINE_WAIT {
                    break;
                }
                self.os.pause_ms(ONLINE_POLL_MS);
            }
            let a = match self.send(m, p::online_query_frame()) {
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

    /// Announce the app as the mouse's config software (0x02 `01`) or release it (`00`). False = not acknowledged (or
    /// refused by a read-only layer).
    fn driver(&mut self, m: &YourMouse, on: bool) -> bool {
        self.send(m, p::frame(p::CMD_DRIVER, 0, &[on as u8])).is_ok()
    }

    /// The settings the group shows: polling, active stage + its DPI, lift-off.
    fn read_settings(&mut self, m: &YourMouse, r: &mut OnMouse) -> Result<()> {
        r.polling_hz = p::scalar_from_pair(&self.read_mem(m, p::ADDR_POLLING, 2)?).and_then(p::polling_hz);
        let stage = p::scalar_from_pair(&self.read_mem(m, p::ADDR_ACTIVE_STAGE, 2)?);
        if let Some(s) = stage {
            r.stage = Some(s);
            r.dpi = p::decode_dpi_stage(&self.read_mem(m, p::ADDR_DPI_STAGE0 + 4 * s as u16, 4)?);
        }
        r.lift_off = p::scalar_from_pair(&self.read_mem(m, p::ADDR_LIFT_OFF, 2)?).and_then(p::lift_off_tenths);
        Ok(())
    }

    /// Reads everything the "Your mouse" group shows from a supported mouse: identify, battery, then (if the mouse is
    /// awake) polling, active stage + its DPI, lift-off. Only READ requests are sent - except
    /// when a dongle answers the settings reads only for its config software: then the app is announced (0x02 `01`) and
    /// released (`00`) around one more try. Never a write, reset or profile switch; the read-only layers refuse the
    /// announce, and then the first error stands.
    pub fn read_on_mouse(&mut self, m: &YourMouse, opt: ReadOptions) -> Result<OnMouse> {
        if m.protocol.is_none() {
            return Err(Error::UnsupportedMouse(m.name.clone()));
        }
        let id = self.send(m, p::identify_frame(challenge()))?;
        let (family, model_code, link) = (id[10], id[11], Link::from_code(id[12]));
        if family != p::FAMILY_CMOUSE {
            return Err(Error::UnsupportedMouse(format!("{}: family code {family:#04x}, not 0x57", m.name)));
        }
        let mut r = OnMouse {
            model: p::model_name(family, model_code),
            family,
            model_code,
            link,
            online: false,
            battery_percent: None,
            charging: None,
            battery_mv: None,
            stage: None,
            dpi: None,
            polling_hz: None,
            lift_off: None,
        };
        // A sleeping mouse behind the dongle answers nothing (measured on the real X2 CrazyLight: identify answered by the
        // dongle, battery timed out) — then battery stays unknown and the mouse counts as offline.
        match self.send(m, p::frame(p::CMD_BATTERY, 0, &[])) {
            Ok(b) if b[2] == 0 => {
                if let Some((pct, ch, mv)) = p::decode_battery(&b) {
                    r.battery_percent = Some(pct);
                    r.charging = Some(ch);
                    r.battery_mv = Some(mv);
                }
            }
            Ok(_) | Err(Error::BadAnswer(_)) => {}
            Err(e) => return Err(e),
        }
        r.online = self.online(m, opt.online_tries)?;
        if !r.online {
            return Ok(r);
        }
        // read requests only first; a dongle that answers settings reads only for its config software gets the app
        // announced (PulsarBattery's "active session") and is asked once more
        match self.read_settings(m, &mut r) {
            Ok(()) => {}
            Err(e @ Error::BadAnswer(_)) => {
                if !self.driver(m, true) {
                    // (its answer may be what got lost: released all the same)
                    self.driver(m, false);
                    return Err(e);
                }
                let again = self.read_settings(m, &mut r);
                self.driver(m, false);
                again?;
            }
            Err(e) => return Err(e),
        }
        Ok(r)
    }

    /// A change: the mouse awake and linked, the app announced for the session, released afterwards (also after a
    /// failure).
    fn session<T>(&mut self, m: &YourMouse, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        self.awake(m)?;
        // released also when the announce's answer was lost (the announce itself may have arrived)
        self.driver(m, true);
        let r = f(self);
        self.driver(m, false);
        r
    }

    /// Takes (`lock`) or releases the write hold: asked again until the mouse is not busy (and, for the lock, holds it).
    fn hold(&mut self, m: &YourMouse, lock: bool) -> Result<()> {
        let t0 = std::time::Instant::now();
        for i in 0..40 {
            if i > 0 {
                if t0.elapsed() >= ONLINE_WAIT {
                    break;
                }
                self.os.pause_ms(10);
            }
            match self.send(m, p::hold_frame(lock)) {
                Ok(a) if a[10] == 0 && (!lock || a[6] == 1) => return Ok(()),
                Ok(_) | Err(Error::BadAnswer(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Err(Error::BadAnswer(format!("the mouse did not {} its settings for writing", if lock { "lock" } else { "unlock" })))
    }

    /// The mouse must be awake and linked before a change (a sleeping mouse answers nothing behind the dongle).
    fn awake(&mut self, m: &YourMouse) -> Result<()> {
        if m.protocol.is_none() {
            return Err(Error::UnsupportedMouse(m.name.clone()));
        }
        if !self.online(m, ReadOptions::default().online_tries)? {
            return Err(Error::MouseGone(format!("{} is asleep or not linked — move it and try again", m.name)));
        }
        Ok(())
    }

    /// Lock → write → read back (the write asked again up to [`TRIES`] times) → unlock (always unlocks, also after a
    /// failure).
    fn write_mem(&mut self, m: &YourMouse, addr: u16, data: &[u8]) -> Result<()> {
        if let Err(e) = self.hold(m, true) {
            // the lock may have arrived with only its answers lost: never leave it held
            let _ = self.send(m, p::hold_frame(false));
            return Err(e);
        }
        let r = (|| {
            let mut last = String::new();
            for _ in 0..TRIES {
                let a = match self.send(m, p::frame(p::CMD_WRITE, addr, data)) {
                    Ok(a) => a,
                    Err(Error::BadAnswer(e)) => {
                        last = e;
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                if a[2] != 0 {
                    last = format!("write {addr:#06x}: status {}", a[2]);
                    continue;
                }
                let back = self.read_mem(m, addr, data.len() as u8)?;
                if back == data {
                    return Ok(());
                }
                last = format!("write {addr:#06x}: read back {back:02x?}, wrote {data:02x?}");
            }
            Err(Error::BadAnswer(last))
        })();
        let unlock = self.hold(m, false);
        r?;
        unlock
    }

    fn active_stage(&mut self, m: &YourMouse) -> Result<u8> {
        p::scalar_from_pair(&self.read_mem(m, p::ADDR_ACTIVE_STAGE, 2)?).ok_or_else(|| Error::BadAnswer("active stage".into()))
    }

    /// DPI chip or Custom value (`custom_dpi`: the mouse's own steps, 50–26000) on the active stage. Returns the toast.
    pub fn set_dpi(&mut self, m: &YourMouse, typed: u32) -> Result<String> {
        let dpi = custom_dpi(typed);
        let rec = p::encode_dpi_stage(dpi).ok_or_else(|| Error::range("DPI", format!("{dpi} can't be stored")))?;
        let old = self.session(m, |s| {
            let stage = s.active_stage(m)?;
            let addr = p::ADDR_DPI_STAGE0 + 4 * stage as u16;
            let old = s.read_mem(m, addr, 4)?;
            s.write_mem(m, addr, &rec)?;
            Ok(old)
        })?;
        self.remember(UndoKey::OnMouse("dpi"), UndoValue::OnMouse(u32::from_le_bytes([old[0], old[1], old[2], old[3]])));
        Ok(format!("DPI {dpi} · saved on the mouse"))
    }

    /// Polling chip. The link must allow it (e.g. 8000 Hz needs the 8K dongle).
    pub fn set_polling(&mut self, m: &YourMouse, hz: u32, link: Link) -> Result<()> {
        let code = p::polling_code(hz).ok_or_else(|| Error::range("polling rate", format!("{hz} Hz is not a polling rate")))?;
        if hz > link.max_polling_hz() {
            return Err(Error::range("polling rate", format!("{hz} Hz needs a faster receiver (this link allows {} Hz)", link.max_polling_hz())));
        }
        let old = self.session(m, |s| {
            let old = s.read_mem(m, p::ADDR_POLLING, 2)?;
            s.write_mem(m, p::ADDR_POLLING, &p::scalar_pair(code))?;
            Ok(old)
        })?;
        self.remember(UndoKey::OnMouse("polling"), UndoValue::OnMouse(old[0] as u32));
        Ok(())
    }

    /// Lift-off chip, in tenths of a mm (10 = 1 mm, 20 = 2 mm).
    pub fn set_lift_off(&mut self, m: &YourMouse, tenths_mm: u32) -> Result<()> {
        let code = p::lift_off_code(tenths_mm).ok_or_else(|| Error::range("lift-off distance", format!("{tenths_mm} tenths of a mm")))?;
        let old = self.session(m, |s| {
            let old = s.read_mem(m, p::ADDR_LIFT_OFF, 2)?;
            s.write_mem(m, p::ADDR_LIFT_OFF, &p::scalar_pair(code))?;
            Ok(old)
        })?;
        self.remember(UndoKey::OnMouse("lift_off"), UndoValue::OnMouse(old[0] as u32));
        Ok(())
    }

    /// Puts a mouse setting back exactly as it was before the last change made here ("dpi", "polling", "lift_off").
    pub fn undo_on_mouse(&mut self, m: &YourMouse, what: &'static str) -> Result<()> {
        let Some(UndoValue::OnMouse(old)) = self.take_undo(&UndoKey::OnMouse(what)) else {
            return Err(Error::NothingToUndo(what.into()));
        };
        self.session(m, |s| match what {
            "dpi" => {
                let stage = s.active_stage(m)?;
                s.write_mem(m, p::ADDR_DPI_STAGE0 + 4 * stage as u16, &old.to_le_bytes())
            }
            "polling" => s.write_mem(m, p::ADDR_POLLING, &p::scalar_pair(old as u8)),
            "lift_off" => s.write_mem(m, p::ADDR_LIFT_OFF, &p::scalar_pair(old as u8)),
            other => Err(Error::NothingToUndo(other.into())),
        })
    }
}

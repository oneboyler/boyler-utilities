//! "Your mouse" (DESIGN §3.4): identify the connected mouse by its USB ids; for a mouse with a driver (`crate::family`:
//! Pulsar, Logitech, Razer, SteelSeries - ported from OpenMouse) read / set DPI, polling rate and lift-off distance
//! (+ battery); any other mouse gets its brand's web-settings link.
//!
//! the owner's mouse (measured, read-only device list): Pulsar X2 CrazyLight, 3710:5406 (8K dongle).
//!
//! Calls made for what DESIGN left unclear:
//! - The DPI chips / Custom field set the DPI of the mouse's ACTIVE stage (both axes). The other stages are untouched.
//! - Battery "about N days left" is not built: no source gives a drain rate.
//! - Opening the tab sends READ requests only; a write only comes from a click.

use crate::error::{Error, Result};
use crate::family::{self, Caps, Changed, Family, Matched, Port, Setting};
use crate::os::{HidInfo, HidTransfer, MouseOs};
use crate::service::{Mouse, UndoKey, UndoValue};

pub use crate::family::{OnMouse, ReadOptions};

/// DPI chips (DESIGN).
pub const DPI_CHIPS: [u32; 4] = [400, 800, 1600, 3200];

/// The acceleration card's own DPI field (never sent to a mouse): a typed DPI rounded to the Pulsar cMouse steps,
/// 50–26000 (Order 042), the nearest value (1234 → 1230, 12345 → 12350).
pub fn custom_dpi(typed: u32) -> u32 {
    crate::pulsar::dpi_caps().nearest(typed)
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
        0x0B05 => ("ASUS", "https://rog.asus.com/armoury-crate/"),
        _ => return None,
    };
    Some(Brand { name, settings_url })
}

/// The brand of one mouse: by vendor id when the id belongs to one brand, else by the model (a Pulsar on the shared CompX id
/// 3554), else by the brand's own name in the mouse's product / manufacturer string. Never a guess from a shared id.
pub fn brand_of(vid: u16, pid: u16, product: Option<&str>, manufacturer: Option<&str>) -> Option<Brand> {
    if let Some(b) = brand(vid) {
        return Some(b);
    }
    let pulsar = brand(0x3710);
    if crate::models::PULSAR.iter().any(|e| e.0 == vid && e.1 == pid) {
        return pulsar;
    }
    let says_pulsar = |s: Option<&str>| s.map(|s| s.to_ascii_lowercase().contains("pulsar")).unwrap_or(false);
    if says_pulsar(product) || says_pulsar(manufacturer) {
        return pulsar;
    }
    None
}

/// "3710:5406" - the ids a user reports for a mouse the app does not know.
pub fn ids_text(vid: u16, pid: u16) -> String {
    format!("{vid:04X}:{pid:04X}")
}

/// The connected mouse as found from the device list (nothing sent to it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct YourMouse {
    /// "Pulsar X2 CrazyLight", else the product string, else "Mouse (VID:PID)"
    pub name: String,
    pub vid: u16,
    pub pid: u16,
    pub brand: Option<Brand>,
    /// `Some` = a driver can read and set DPI / polling / lift-off: its family, model and interface
    pub driver: Option<Matched>,
    /// the interface the app talks to, for a mouse with a driver
    pub config_path: Option<String>,
    /// from the id: dongle = wireless, cable = wired (refined by the mouse's answer)
    pub wireless: Option<bool>,
}

impl YourMouse {
    /// "3710:5406" (VID:PID, hex) - shown in the tab's details; `0000:0000` = Windows gave no USB id (built-in / virtual).
    pub fn ids(&self) -> String {
        ids_text(self.vid, self.pid)
    }

    /// The family whose driver talks to this mouse.
    pub fn family(&self) -> Option<Family> {
        self.driver.as_ref().map(|d| d.family)
    }

    /// Does the app read and set this mouse's settings?
    pub fn supported(&self) -> bool {
        self.driver.is_some()
    }

    /// The small line under the name.
    pub fn sub_line(&self) -> &'static str {
        match (self.supported(), self.wireless) {
            (true, Some(true)) => "Wireless · saved on the mouse itself",
            (true, _) => "Saved on the mouse itself",
            (false, _) => "DPI and polling for this mouse aren't supported yet",
        }
    }

    /// The link on the right: "Open <Brand> web settings" (supported) / "open its web settings" (unsupported).
    pub fn link(&self) -> Option<(String, &'static str)> {
        let b = self.brand.as_ref()?;
        Some(if self.supported() { (format!("Open {} web settings", b.name), b.settings_url) } else { ("open its web settings".into(), b.settings_url) })
    }
}

/// Every mouse in a device list (one entry per vid:pid that has a mouse collection, usage page 1 / usage 2), supported
/// mice first and among them the cable before the dongle (a dongle whose mouse is on its cable answers nothing).
pub fn find_mice(hid: &[HidInfo]) -> Vec<YourMouse> {
    let mut ids: Vec<(u16, u16)> = hid.iter().filter(|h| h.usage_page == 0x01 && h.usage == 0x02).map(|h| (h.vid, h.pid)).collect();
    ids.sort();
    ids.dedup();
    // Order 090: real mice only - a keyboard's extra mouse interface (Wooting 80HE's "mouse" for its analog keys) is no mouse
    ids.retain(|(vid, pid)| !is_keyboard(hid, *vid, *pid));
    // a mouse with no USB id (touchpad, PS/2, remote session) is only listed when no mouse with an id is there (it would
    // otherwise be a second row for the same hardware or a virtual device)
    if ids.iter().any(|i| i.0 != 0) {
        ids.retain(|i| i.0 != 0);
    }
    let mut out: Vec<YourMouse> = ids
        .into_iter()
        .map(|(vid, pid)| {
            let driver = if vid == 0 { None } else { family::find(hid, vid, pid) };
            let product = hid.iter().filter(|h| h.vid == vid && h.pid == pid).find_map(|h| h.product.clone());
            let manufacturer = hid.iter().filter(|h| h.vid == vid && h.pid == pid).find_map(|h| h.manufacturer.clone());
            let name = driver
                .as_ref()
                .and_then(|d| d.model)
                .or_else(|| crate::models::model(vid, pid))
                .map(str::to_string)
                .or(product.clone())
                .unwrap_or_else(|| if vid == 0 { "Mouse".to_string() } else { format!("Mouse ({vid:04X}:{pid:04X})") });
            YourMouse {
                name,
                vid,
                pid,
                brand: brand_of(vid, pid, product.as_deref(), manufacturer.as_deref()),
                config_path: driver.as_ref().map(|d| d.config.path.clone()),
                wireless: driver.as_ref().and_then(|d| d.wireless),
                driver,
            }
        })
        .collect();
    // supported first, then mice the tables know by model, then the rest; no-id mice last. The cable before the dongle only
    // within one brand (the same mouse on both): another brand's cabled mouse must not push the dongle's mouse off "Your
    // mouse" (Order 104 review)
    let wired_vids: Vec<u16> = out.iter().filter(|m| m.supported() && m.wireless == Some(false)).map(|m| m.vid).collect();
    out.sort_by_key(|m| {
        let named = crate::models::model(m.vid, m.pid).is_some() || m.driver.as_ref().and_then(|d| d.model).is_some();
        (!m.supported(), m.vid == 0, !named, m.wireless == Some(true) && wired_vids.contains(&m.vid))
    });
    out
}

/// Makers that make keyboards only: a mouse collection of theirs is a keyboard's extra interface (Wooting, Keychron).
const KEYBOARD_VENDORS: [u16; 2] = [0x31E3, 0x3434];

/// Is the device `vid:pid` a KEYBOARD that also shows a mouse collection (Order 090: the owner's Mouse tab listed "Wooting 80HE ·
/// Also connected · 31E3:1402")? Only by maker: the interfaces can't tell - his real Pulsar X2 MOUSE has its keyboard
/// collection on interface 0 and the mouse on interface 2, exactly like a keyboard with a mouse interface.
pub fn is_keyboard(_hid: &[HidInfo], vid: u16, pid: u16) -> bool {
    // Order 105: Keychron also makes mice with a driver (Nape Pro, M6 and its receiver) - those ids are mice
    KEYBOARD_VENDORS.contains(&vid) && !(vid == crate::keychron::VENDOR_ID && crate::keychron::MOUSE_PRODUCT_IDS.contains(&pid))
}

/// The mouse's interface, through the OS layer.
struct OsPort<'a, O: MouseOs> {
    os: &'a mut O,
    info: HidInfo,
}

impl<O: MouseOs> Port for OsPort<'_, O> {
    fn exchange(&mut self, out: &[u8], how: &HidTransfer) -> Result<Vec<u8>> {
        self.os.hid_exchange(&self.info.path, out, how)
    }
    fn pause_ms(&mut self, ms: u64) {
        self.os.pause_ms(ms)
    }
    fn info(&self) -> &HidInfo {
        &self.info
    }
}

impl<O: MouseOs> Mouse<O> {
    /// The connected mice, best first (`[0]` is "Your mouse"). Read-only: nothing is sent to any device.
    pub fn mice(&self) -> Result<Vec<YourMouse>> {
        Ok(find_mice(&self.os.hid_devices()?))
    }

    fn with_driver<T>(&mut self, m: &YourMouse, f: impl FnOnce(&dyn family::Driver, &Matched, &mut dyn Port) -> Result<T>) -> Result<T> {
        let d = m.driver.clone().ok_or_else(|| Error::UnsupportedMouse(m.name.clone()))?;
        let mut port = OsPort { os: &mut self.os, info: d.config.clone() };
        f(family::driver(d.family), &d, &mut port)
    }

    /// Reads everything the "Your mouse" group shows from a mouse with a driver - READ requests only (the read-only layers
    /// refuse anything else before a byte leaves).
    pub fn read_on_mouse(&mut self, m: &YourMouse, opt: ReadOptions) -> Result<OnMouse> {
        self.with_driver(m, |d, mt, p| d.read(mt, p, opt))
    }

    /// One setting from a click, through the mouse's driver; the value before is kept for undo.
    pub fn change(&mut self, m: &YourMouse, s: Setting) -> Result<Changed> {
        let c = self.with_driver(m, |d, mt, p| d.write(mt, p, s))?;
        if let Some(b) = c.before {
            self.remember(UndoKey::OnMouse(s.key()), UndoValue::OnMouse(b));
        }
        Ok(c)
    }

    /// DPI chip or Custom value, rounded to the nearest value the mouse can store (`caps` from its last read), on the active
    /// stage. Returns the toast.
    pub fn set_dpi(&mut self, m: &YourMouse, typed: u32, caps: &Caps) -> Result<String> {
        let dc = caps.dpi.as_ref().ok_or_else(|| Error::range("DPI", "this mouse's DPI can't be set here"))?;
        let c = self.change(m, Setting::Dpi(dc.nearest(typed)))?;
        Ok(format!("DPI {} · saved on the mouse", c.now))
    }

    /// Polling chip (the driver checks what the mouse / its receiver allows).
    pub fn set_polling(&mut self, m: &YourMouse, hz: u32) -> Result<()> {
        self.change(m, Setting::Polling(hz)).map(|_| ())
    }

    /// Lift-off chip, in tenths of a mm (10 = 1 mm, 20 = 2 mm).
    pub fn set_lift_off(&mut self, m: &YourMouse, tenths_mm: u32) -> Result<()> {
        self.change(m, Setting::LiftOff(tenths_mm)).map(|_| ())
    }

    /// Puts a mouse setting back exactly as it was before the last change made here ("dpi", "polling", "lift_off").
    pub fn undo_on_mouse(&mut self, m: &YourMouse, what: &'static str) -> Result<()> {
        let Some(UndoValue::OnMouse(old)) = self.take_undo(&UndoKey::OnMouse(what)) else {
            return Err(Error::NothingToUndo(what.into()));
        };
        let s = match what {
            "dpi" => Setting::Dpi(old),
            "polling" => Setting::Polling(old),
            "lift_off" => Setting::LiftOff(old),
            other => return Err(Error::NothingToUndo(other.into())),
        };
        self.with_driver(m, |d, mt, p| d.write(mt, p, s)).map(|_| ())
    }
}

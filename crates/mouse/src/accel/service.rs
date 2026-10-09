//! `Mouse<O>` methods for the acceleration card: is Raw Accel there, mirror what it runs, "Copy its curve", what should
//! run now (switch + per app), handing that to the driver (only when it changed), the header line, the graph.
//!
//! Writing: the user's own Raw Accel settings.json is the base (read only — its other profiles, devices, rotation, DPI
//! ratios… are kept); profile 0 gets the card's curve on both axes + Output DPI = sens × 1000; the whole config goes to
//! the driver as Raw Accel's own WRITE bytes (`bytes::to_bytes`) when the driver is v1.7.x (the layout this crate
//! knows), else through the installed Raw Accel's `writer.exe` with the same JSON. Raw Accel's settings.json is never
//! written. Validation = Raw Accel's own (`curves::validate`), so nothing invalid ever reaches the driver.

use super::args::DriverConfig;
use super::bytes;
use super::curves::{init_data, sensitivity, validate};
use super::panel::{from_args, Panel, Setting};
use super::switch::{AppEvent, PerApp, PresetId, Target};
use crate::error::{Error, Result};
use crate::os::{DriverVersion, MouseOs};
use crate::service::Mouse;
use std::path::{Path, PathBuf};

/// Official releases page (DESIGN: link only, never bundle the driver).
pub const RAWACCEL_RELEASES: &str = "https://github.com/RawAccelOfficial/rawaccel/releases";

/// The preset the first run makes from what Raw Accel runs (`mirror_rawaccel`).
pub const MIRROR_PRESET: &str = "Raw Accel";

/// The acceleration part of the service's state (saved with the app's settings: `panel`, `per_app`, `rawaccel_dir`).
#[derive(Clone, Debug, Default)]
pub struct AccelState {
    pub panel: Panel,
    pub per_app: PerApp,
    /// the folder holding rawaccel.exe / writer.exe / settings.json (found or picked)
    pub rawaccel_dir: Option<PathBuf>,
    /// the bytes last handed to the driver (a write is skipped when nothing changed)
    last_written: Option<Vec<u8>>,
    /// the saved card could not be read (damaged / newer version): it is never overwritten (`save_accel` refuses)
    pub(crate) load_failed: bool,
}

/// What the card shows in its place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawAccelStatus {
    /// The driver answers: the card, with "Runs on your Raw Accel <version>".
    Installed { version: DriverVersion, dir: Option<PathBuf> },
    /// No driver: the install card (DESIGN text) + the official link.
    NotInstalled,
}

/// What `start_accel` did at app start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartAccel {
    /// no saved card (the first run: the Mouse tab mirrors Raw Accel when it opens) — nothing written
    NothingSaved,
    /// the card was saved OFF — the driver is not touched
    LeftAlone,
    /// the card was ON but there is no Raw Accel driver
    NoDriver,
    /// the card was ON but the driver is not a 1.7 (its state can not be read, so nothing is written at start)
    Unreadable,
    /// the card was ON and the driver already ran it — nothing written
    AlreadyRunning,
    /// the card was ON and the driver ran something else — written
    Applied,
}

impl RawAccelStatus {
    pub fn footer(&self) -> Option<String> {
        match self {
            RawAccelStatus::Installed { version, .. } => Some(format!("Runs on your Raw Accel {version} · Copy its curve · Open Raw Accel")),
            RawAccelStatus::NotInstalled => None,
        }
    }
}

pub const INSTALL_TITLE: &str = "Install Raw Accel (free, open source) to use mouse acceleration";
pub const INSTALL_TEXT: &str =
    "It does the acceleration, inside games too. Its installer needs admin and one restart; then your curves, presets and the per-app switch show up here.";
pub const INSTALL_LINK: &str = "Get it from its official GitHub ↗";

/// A Raw Accel folder = one that holds rawaccel.exe and writer.exe. Looks at each root itself and at the folders directly
/// in it whose NAME starts with "RawAccel" (any case, spaces / dashes / underscores ignored: Desktop\RawAccel,
/// Downloads\RawAccel-1.7.0). Only the names in a root are listed; no other folder is opened or looked into (other folders are never touched). Read-only.
pub fn find_rawaccel_dir(roots: &[PathBuf]) -> Option<PathBuf> {
    let is_ra = |d: &Path| d.join("rawaccel.exe").is_file() && d.join("writer.exe").is_file();
    for r in roots {
        if is_ra(r) {
            return Some(r.clone());
        }
        if let Ok(rd) = std::fs::read_dir(r) {
            let mut named: Vec<PathBuf> = rd.flatten().filter(|e| is_rawaccel_name(&e.file_name().to_string_lossy())).map(|e| e.path()).collect();
            named.sort();
            if let Some(d) = named.into_iter().find(|d| is_ra(d)) {
                return Some(d);
            }
        }
    }
    None
}

/// "RawAccel", "rawaccel-1.7.0", "Raw Accel" … — the only folder names `find_rawaccel_dir` looks into.
pub fn is_rawaccel_name(name: &str) -> bool {
    let n: String = name.chars().filter(|c| !c.is_whitespace() && *c != '_' && *c != '-').collect::<String>().to_ascii_lowercase();
    n.starts_with("rawaccel")
}

/// The main preset's name for the header ("Fast"); the card's own values that are no saved preset show as "Custom".
pub fn main_name(panel: &Panel) -> String {
    panel.loaded.and_then(|id| panel.preset(id)).map(|p| p.name.clone()).unwrap_or_else(|| "Custom".into())
}

/// Header line when on: "Fast everywhere" - with games listed "VALORANT: Valorant · otherwise Fast" / "… · otherwise Off".
pub fn header_line(panel: &Panel, per_app: &PerApp) -> String {
    if !panel.on {
        return "Fast flicks go further, slow aim stays the same".into();
    }
    let name = |t: Target| match t {
        Target::Main => main_name(panel),
        Target::Off => "Off".to_string(),
        Target::Preset(id) => panel.preset(id).map(|p| p.name.clone()).unwrap_or_else(|| "Off".into()),
    };
    if per_app.rows().is_empty() {
        return format!("{} everywhere", main_name(panel));
    }
    let mut parts: Vec<String> = per_app.rows().iter().map(|r| format!("{}: {}", r.label, name(r.target))).collect();
    parts.push(format!("otherwise {}", name(per_app.everywhere_else())));
    parts.join(" · ")
}

/// The DPI line under the graph (DESIGN): known mouse → "at 1600 DPI · from Your mouse"; else "at [1600] DPI".
pub fn dpi_line(known: Option<u32>, typed: u32) -> String {
    match known {
        Some(d) => format!("at {d} DPI · from Your mouse"),
        None => format!("at [{typed}] DPI"),
    }
}

impl<O: MouseOs> Mouse<O> {
    pub fn accel(&self) -> &AccelState {
        &self.accel
    }
    pub fn accel_mut(&mut self) -> &mut AccelState {
        &mut self.accel
    }

    pub fn rawaccel_status(&self) -> Result<RawAccelStatus> {
        Ok(match self.os.rawaccel_driver_version()? {
            Some(version) => RawAccelStatus::Installed { version, dir: self.accel.rawaccel_dir.clone() },
            None => RawAccelStatus::NotInstalled,
        })
    }

    /// Raw Accel's own settings.json (read only). `None` when no folder is known or it has no settings.json yet.
    pub fn rawaccel_settings(&self) -> Result<Option<DriverConfig>> {
        let Some(dir) = &self.accel.rawaccel_dir else { return Ok(None) };
        match self.os.read_text(&dir.join("settings.json"))? {
            Some(t) => DriverConfig::from_json(&t).map(Some).map_err(Error::RawAccelSettings),
            None => Ok(None),
        }
    }

    /// What the driver runs right now (READ), `None` without a driver.
    pub fn driver_config_now(&self) -> Result<Option<Vec<super::args::Profile>>> {
        Ok(self.os.rawaccel_read()?.map(|b| bytes::read_profiles(&b)))
    }

    /// First run: the card mirrors the installed Raw Accel and changes NOTHING on the driver. When the driver runs a
    /// curve, that curve becomes a preset named "Raw Accel" (loaded in the card, collapsed), "Everywhere else" points at it
    /// and the switch shows on — so what should run equals what runs. Otherwise the card stays off (DESIGN: "for a new
    /// user it is off"). When the driver already holds exactly the bytes this crate would send, no write follows.
    pub fn mirror_rawaccel(&mut self) -> Result<bool> {
        // first run only: once the card has presets it is the user's, never overwritten
        if !self.accel.panel.presets.is_empty() {
            return Ok(false);
        }
        let Some(raw) = self.os.rawaccel_read()? else { return Ok(false) };
        let profiles = bytes::read_profiles(&raw);
        let Some(p) = profiles.first() else { return Ok(false) };
        let Some((c, v)) = from_args(&p.accel_x) else { return Ok(false) };
        let panel = &mut self.accel.panel;
        panel.curve = c;
        panel.values.insert(c, v);
        panel.sens = p.output_dpi / super::args::NORMALIZED_DPI;
        let (id, _) = panel.save_as_preset();
        let _ = panel.rename_preset(id, MIRROR_PRESET);
        panel.on = true;
        panel.expanded = false;
        // Same bytes already in the driver → remember them, so the next sync does not rewrite (no 1 s write for nothing).
        if let Ok(cfg) = self.config_for(&self.accel_target()) {
            let ours = bytes::to_bytes(&cfg);
            if bytes::same_effect(&ours, &raw) {
                self.accel.last_written = Some(ours);
            }
        }
        Ok(true)
    }

    /// "Copy its curve": Raw Accel's own settings.json (profile 0) into the card — any curve (a look-up table can't be
    /// shown: error). Returns the toast.
    pub fn copy_its_curve(&mut self) -> Result<String> {
        let cfg = self.rawaccel_settings()?.ok_or_else(|| Error::RawAccelMissing("no Raw Accel settings.json found".into()))?;
        let p = cfg.profiles.first().cloned().unwrap_or_default();
        match from_args(&p.accel_x) {
            Some((c, v)) => {
                self.accel.panel.curve = c;
                self.accel.panel.values.insert(c, v);
                self.accel.panel.sens = p.output_dpi / super::args::NORMALIZED_DPI;
                Ok(format!("Copied Raw Accel's {} curve", c.name()))
            }
            None => Err(Error::RawAccelSettings(format!("Raw Accel's curve is {:?} — not one of the six curves here", p.accel_x.mode))),
        }
    }

    /// What should run now: switch off → Off; else the per-app choice (a listed app that runs, else "Everywhere else").
    /// The loaded preset's LIVE card values are used while it is the one running (so tuning is felt at once).
    pub fn accel_target(&self) -> Setting {
        let a = &self.accel;
        if !a.panel.on {
            return Setting::off();
        }
        match a.per_app.current() {
            Target::Off => Setting::off(),
            Target::Main => a.panel.current_setting(),
            Target::Preset(id) if a.panel.loaded == Some(id) => a.panel.current_setting(),
            Target::Preset(id) => a.panel.preset_setting(id).unwrap_or_else(Setting::off),
        }
    }

    /// The whole config the driver should get for `s` (the user's settings.json as the base, else Raw Accel's defaults).
    pub fn config_for(&self, s: &Setting) -> Result<DriverConfig> {
        let mut cfg = self.rawaccel_settings()?.unwrap_or_default();
        let base = cfg.profiles.first().cloned().unwrap_or_default();
        cfg.profiles[0] = s.apply_to(&base);
        let errs = validate(&cfg.profiles[0]);
        if !errs.is_empty() {
            return Err(Error::RawAccelRefused(errs.join("; ")));
        }
        Ok(cfg)
    }

    /// Hands `accel_target()` to the driver unless it already runs exactly that. Returns true when it wrote.
    /// Call it after the settle delay for app events, and after any card change.
    /// With the 1.7 driver the driver's own READ decides (not what this service wrote last): another program — Raw Accel's
    /// own app re-sends its settings.json every time it opens — may have changed the driver since (Order 063).
    pub fn sync_driver(&mut self) -> Result<bool> {
        let version = self.os.rawaccel_driver_version()?.ok_or_else(|| Error::RawAccelMissing("the Raw Accel driver is not running".into()))?;
        let cfg = self.config_for(&self.accel_target())?;
        let b = bytes::to_bytes(&cfg);
        if version.major == 1 && version.minor == 7 {
            if self.os.rawaccel_read()?.is_some_and(|now| bytes::same_effect(&b, &now)) {
                self.accel.last_written = Some(b);
                return Ok(false);
            }
            self.os.rawaccel_write(&b)?;
        } else {
            if self.accel.last_written.as_ref() == Some(&b) {
                return Ok(false);
            }
            let dir = self.accel.rawaccel_dir.clone().ok_or_else(|| Error::RawAccelMissing(format!("driver {version}: need the Raw Accel folder for its writer.exe")))?;
            let file = self.dirs.rawaccel_settings();
            self.os.rawaccel_writer(&dir, &file, &cfg.to_json())?;
        }
        self.accel.last_written = Some(b);
        Ok(true)
    }

    /// Does the driver run what the card says now? `None` = no 1.7 driver to ask (nothing known). A read only.
    pub fn driver_matches_card(&self) -> Result<Option<bool>> {
        match self.os.rawaccel_driver_version()? {
            Some(v) if v.major == 1 && v.minor == 7 => {}
            _ => return Ok(None),
        }
        let want = bytes::to_bytes(&self.config_for(&self.accel_target())?);
        Ok(self.os.rawaccel_read()?.map(|now| bytes::same_effect(&want, &now)))
    }

    /// App start (Order 063): the saved card comes back, and the driver is touched ONLY when the user had acceleration ON
    /// and the driver doesn't already run what the card says now. A card that was off, or never saved, writes nothing.
    pub fn start_accel(&mut self) -> Result<StartAccel> {
        if !self.load_accel()? {
            return Ok(StartAccel::NothingSaved);
        }
        if !self.accel.panel.on {
            return Ok(StartAccel::LeftAlone);
        }
        match self.os.rawaccel_driver_version()? {
            None => return Ok(StartAccel::NoDriver),
            // a driver whose layout is unknown can not be read back: it is not written at start (writer.exe would run every time)
            Some(v) if !(v.major == 1 && v.minor == 7) => return Ok(StartAccel::Unreadable),
            Some(_) => {}
        }
        Ok(if self.sync_driver()? { StartAccel::Applied } else { StartAccel::AlreadyRunning })
    }

    /// The line for "another program also writes the driver" (shown on the card), `None` = nothing to say. Reads only:
    /// the process list (names), Raw Accel's `.config`, the driver. Two cases: Raw Accel's own app is open right now, or the
    /// card is on but the driver runs something else (the app wrote it after this one).
    pub fn other_writer_line(&self) -> Result<Option<String>> {
        if self.os.rawaccel_driver_version()?.is_none() {
            return Ok(None);
        }
        if self.os.process_running("rawaccel.exe")? || self.os.process_running("writer.exe")? {
            return Ok(Some("Your own Raw Accel app is open. It writes the driver too, and the last one to write wins. Close it to keep this card in charge.".into()));
        }
        if self.accel.panel.on && self.driver_matches_card()? == Some(false) {
            let auto = self.rawaccel_auto_writes();
            return Ok(Some(format!(
                "The driver is not running this card right now: another program wrote it after this app{}. It is set again at the next game start or change here.",
                if auto { " (your Raw Accel app sends its settings.json to the driver every time it opens)" } else { "" }
            )));
        }
        Ok(None)
    }

    /// Raw Accel's own app writes its settings.json to the driver whenever it starts (its `.config`:
    /// `AutoWriteToDriverOnStartup`). Read only; false when unknown.
    pub fn rawaccel_auto_writes(&self) -> bool {
        let Some(dir) = &self.accel.rawaccel_dir else { return false };
        let Ok(Some(t)) = self.os.read_text(&dir.join(".config")) else { return false };
        serde_json::from_str::<serde_json::Value>(&t).ok().and_then(|v| v.get("AutoWriteToDriverOnStartup").and_then(|b| b.as_bool())).unwrap_or(false)
    }

    /// The header switch. Returns the toast (DESIGN). The driver is synced at once (a direct click, not a game event).
    pub fn set_accel_on(&mut self, on: bool) -> Result<String> {
        self.accel.panel.on = on;
        self.accel.panel.expanded = on;
        self.sync_driver()?;
        Ok(self.accel_on_toast(on))
    }

    /// The toast after the header switch.
    pub fn accel_on_toast(&self, on: bool) -> String {
        if on {
            format!("Acceleration on · {}", header_line(&self.accel.panel, &self.accel.per_app))
        } else {
            "Acceleration off · plain 1:1 everywhere".into()
        }
    }

    /// An app started / stopped. Returns true when what should run may have changed — then settle and `sync_driver`.
    pub fn accel_app_event(&mut self, ev: &AppEvent) -> bool {
        self.accel.per_app.on_event(ev)
    }

    /// × on a preset chip: apps that used it turn Off. Returns the toast "Deleted <name> · <apps>, the other games now use the main preset".
    pub fn delete_accel_preset(&mut self, id: PresetId) -> Option<String> {
        let p = self.accel.panel.delete_preset(id)?;
        let (apps, ee) = self.accel.per_app.forget_preset(id);
        let mut parts = Vec::new();
        if !apps.is_empty() {
            parts.push(format!("{} now Off", apps.join(", ")));
        }
        if ee {
            parts.push("the other games now use the main preset".into());
        }
        Some(if parts.is_empty() { format!("Deleted {}", p.name) } else { format!("Deleted {} · {}", p.name, parts.join(", ")) })
    }

    /// The EPP warning (only while acceleration is on AND Windows' Enhance pointer precision is on).
    pub fn epp_warning(&self) -> Result<bool> {
        Ok(self.accel.panel.on && self.windows_mouse()?.precision)
    }

    /// The graph for the card's current values: (input speed, sensitivity × sens) at x = 0..=120 step `step`, computed with
    /// Raw Accel's own modifier on the user's profile — the numbers Raw Accel's Sensitivity chart shows.
    pub fn accel_graph(&self, step: f64) -> Result<Vec<(f64, f64)>> {
        let cfg = self.config_for(&self.accel.panel.current_setting())?;
        let p = &cfg.profiles[0];
        let d = init_data(p);
        let mut out = Vec::new();
        let mut x = 0.0;
        while x <= 120.0 + 1e-9 {
            out.push((x, sensitivity(p, &d, x)));
            x += step.max(0.01);
        }
        Ok(out)
    }
}

// ---- the app's change log (Order 036): what the driver ran before the app changed it, kept as a file, and put back

/// A driver state's bytes with the struct padding zeroed (padding never reaches the driver's maths; zeroed, two READs of
/// the same settings are the same bytes).
fn normalized(mut b: Vec<u8>) -> Vec<u8> {
    if let Some((n, m)) = bytes::read_header(&b) {
        for r in bytes::padding_ranges(n as usize, m as usize) {
            for i in r {
                if let Some(x) = b.get_mut(i) {
                    *x = 0;
                }
            }
        }
    }
    b
}

/// FNV-1a 64 of the bytes (names the kept file: the same state = the same file).
fn fnv(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, x| (h ^ *x as u64).wrapping_mul(0x0100_0000_01b3))
}

/// Does this driver state accelerate (its first profile has a curve on either axis)? The change log's "On" / "Off".
pub fn driver_state_on(b: &[u8]) -> bool {
    bytes::read_profiles(b).first().is_some_and(|p| p.accel_x.mode != super::args::AccelMode::Noaccel || p.accel_y.mode != super::args::AccelMode::Noaccel)
}

impl<O: MouseOs> Mouse<O> {
    /// What the driver runs now, as the change log keeps it (padding zeroed). `None` = no driver, or a driver version whose
    /// byte layout this crate does not know (then nothing of it can be kept or put back).
    pub fn driver_state(&self) -> Result<Option<Vec<u8>>> {
        match self.os.rawaccel_driver_version()? {
            Some(v) if v.major == 1 && v.minor == 7 => {}
            _ => return Ok(None),
        }
        Ok(self.os.rawaccel_read()?.map(normalized))
    }

    /// The file a driver state is kept in (`AppDirs::rawaccel_before`, named by its bytes) — the change log's value.
    pub fn driver_state_file(&self, b: &[u8]) -> PathBuf {
        self.dirs.rawaccel_before().join(format!("{:016x}.bin", fnv(b)))
    }

    /// Keeps a driver state in its file (written once; the same state is never written again). Returns the file.
    pub fn keep_driver_state(&mut self, b: &[u8]) -> Result<PathBuf> {
        let f = self.driver_state_file(b);
        if self.os.read_bytes(&f)?.as_deref() != Some(b) {
            self.os.write_bytes(&f, b)?;
        }
        Ok(f)
    }

    /// Puts a kept driver state back (only a file of `AppDirs::rawaccel_before`): the WRITE ioctl with its bytes, when the
    /// driver doesn't run them already. The card's next change is written again (`last_written` forgotten).
    /// The saved card goes OFF too (Order 063): else the next app start would write the card's curve back over what the
    /// user just put back. Its presets and games stay.
    pub fn restore_driver_state(&mut self, file: &Path) -> Result<()> {
        if file.parent() != Some(self.dirs.rawaccel_before().as_path()) {
            return Err(Error::NotFound(format!("{} is not a copy of Raw Accel's settings", file.display())));
        }
        let b = self.os.read_bytes(file)?.ok_or_else(|| Error::NotFound("the copy of Raw Accel's earlier settings".into()))?;
        let now = self.driver_state()?.ok_or_else(|| Error::RawAccelMissing("the Raw Accel 1.7 driver is not running".into()))?;
        if now != b {
            self.os.rawaccel_write(&b)?;
        }
        self.accel.last_written = None;
        // (a closed tab's service has not read the file yet: read it first, so saving never replaces it with an empty card)
        if self.load_accel().unwrap_or(false) && self.accel.panel.on {
            self.accel.panel.on = false;
            self.accel.panel.expanded = false;
            self.save_accel()?;
        }
        Ok(())
    }
}

/// Hover readout: "60 → 1.13×".
pub fn readout(x: f64, y: f64) -> String {
    format!("{} → {:.2}×", x.round() as i64, y)
}

impl<O: MouseOs> Mouse<O> {
    /// "Open Raw Accel": the GUI to start (the app layer launches it — never from a test). `None` = folder unknown.
    pub fn rawaccel_exe(&self) -> Option<PathBuf> {
        self.accel.rawaccel_dir.as_ref().map(|d| d.join("rawaccel.exe")).filter(|p| p.is_file())
    }
}

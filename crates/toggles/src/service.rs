//! The Toggles service: read every row's state from Windows, apply a change, read it back, run the after-step, and undo.
//! All logic lives here; the OS layer (`TogglesOs`) only reads and writes.

use std::collections::HashMap;

use crate::defaults::ChangeAction;
use crate::error::{Error, Result};
use crate::model::{Applied, Badge, FsoGame, Kind, RowState, Timeout, Value, TIMEOUT_CHOICES};
use crate::os::{Hive, PowerSetting, PowerValues, RegValue, SpiItem, TogglesOs};
use crate::rows::{self, Broadcast, Combine, Method, Row, LANG_PATH, LANG_VALUES};
use crate::{dxg, fso};

/// The Store page of the Copilot app (Microsoft Store product id 9NHT9RB2F4HD = "Microsoft Copilot").
pub const COPILOT_STORE_URI: &str = "ms-windows-store://pdp/?ProductId=9NHT9RB2F4HD";
/// Sleep timeout used when Sleep is switched on and no earlier timeout is known (the drawing shows "30 min").
pub const DEFAULT_SLEEP_SECONDS: u32 = 1800;
/// Grey line of Fast Startup while Hibernate is off (DESIGN §3.6).
pub const FAST_STARTUP_NEEDS_HIBERNATE: &str = "Needs Hibernate · Hibernate is off in Windows";
/// Grey line of "Sleep after" while Sleep is off.
pub const SLEEP_AFTER_SLEEP_OFF: &str = "Sleep is off";
/// Grey line of Bluetooth on a PC without a Bluetooth radio.
pub const NO_BLUETOOTH: &str = "No Bluetooth on this PC";
/// Grey line of GPU scheduling when the graphics card (driver) doesn't support it.
pub const NO_GPU_SCHEDULING: &str = "Your graphics card doesn't support it";

/// The old value of one row (or one game), kept so the change can be switched back.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Snapshot {
    Reg(Vec<(Hive, String, String, Option<RegValue>)>),
    Key { existed: bool },
    Spi(Vec<(SpiItem, u32)>),
    Power(PowerSetting, PowerValues),
    Bluetooth(bool),
    Copilot(bool),
}

/// The Toggles service over an OS layer (real or fake).
pub struct Toggles<O: TogglesOs> {
    os: O,
    undo: HashMap<String, Snapshot>,
    /// the sleep timeout before Sleep was switched off (for switching it on again)
    sleep_restore: Option<PowerValues>,
}

fn fso_undo_key(exe: &str) -> String {
    format!("fullscreen_optimizations_off|{}", exe.to_lowercase())
}

impl<O: TogglesOs> Toggles<O> {
    pub fn new(os: O) -> Self {
        Self { os, undo: HashMap::new(), sleep_restore: None }
    }

    pub fn os(&self) -> &O {
        &self.os
    }

    pub fn os_mut(&mut self) -> &mut O {
        &mut self.os
    }

    /// Every row, in page order.
    pub fn rows(&self) -> &'static [Row] {
        rows::ROWS
    }

    fn row(id: &str) -> Result<&'static Row> {
        rows::find(id).ok_or_else(|| Error::UnknownRow(id.to_string()))
    }

    /// Does changing this row need admin?
    pub fn needs_admin(&self, id: &str) -> Result<bool> {
        Ok(Self::row(id)?.needs_admin())
    }

    /// Is there an old value to switch back to?
    pub fn can_undo(&self, id: &str) -> bool {
        self.undo.contains_key(id)
    }

    // ------------------------------------------------------------------ read

    /// What Windows is set to NOW for every row (one failing row doesn't hide the others).
    pub fn read_all(&self) -> Vec<(&'static str, Result<RowState>)> {
        rows::ROWS.iter().map(|r| (r.id, self.read(r.id))).collect()
    }

    /// What Windows is set to NOW for one row.
    pub fn read(&self, id: &str) -> Result<RowState> {
        let row = Self::row(id)?;
        let mut st = RowState { id: row.id, value: Value::Switch(false), enabled: true, disabled_reason: None };
        match row.method {
            Method::Reg { .. } | Method::KeyExists { .. } | Method::Dxg { .. } | Method::SpiFlag { .. } | Method::Animations
            | Method::LangHotkeys | Method::PowerSwitch(_) | Method::Copilot => {
                st.value = Value::Switch(self.switch_state(row)?);
            }
            Method::PowerTimeout(setting) => {
                let v = self.os.power_read(setting)?;
                st.value = Value::Timeout(Timeout::from_seconds(v.ac));
                if setting == PowerSetting::Sleep && v.ac == 0 {
                    st.enabled = false;
                    st.disabled_reason = Some(SLEEP_AFTER_SLEEP_OFF);
                }
            }
            Method::SleepSwitch => {
                st.value = Value::Switch(self.os.power_read(PowerSetting::Sleep)?.ac != 0);
            }
            Method::FastStartup => {
                let hib = self.os.hibernate_on()?;
                st.value = Value::Switch(hib && self.hiberboot_on()?);
                if !hib {
                    st.enabled = false;
                    st.disabled_reason = Some(FAST_STARTUP_NEEDS_HIBERNATE);
                }
            }
            Method::Hags => {
                st.value = Value::Switch(self.switch_state(row)?);
                if !self.os.gpu_scheduling()?.supported {
                    st.enabled = false;
                    st.disabled_reason = Some(NO_GPU_SCHEDULING);
                }
            }
            Method::Bluetooth => match self.os.bluetooth()? {
                Some(on) => st.value = Value::Switch(on),
                None => {
                    st.enabled = false;
                    st.disabled_reason = Some(NO_BLUETOOTH);
                }
            },
            Method::FsoGames => st.value = Value::Games(self.fso_games()?),
        }
        Ok(st)
    }

    fn hiberboot_on(&self) -> Result<bool> {
        // missing = Windows' default (on)
        Ok(match self.os.reg_read(Hive::Hklm, rows::HIBERBOOT_PATH, rows::HIBERBOOT_VALUE)? {
            None => true,
            Some(v) => rows::RegData::D(1).matches(&v),
        })
    }

    fn dxg_list(&self) -> Result<Option<String>> {
        Ok(match self.os.reg_read(Hive::Hkcu, rows::DXG_PATH, rows::DXG_VALUE)? {
            Some(RegValue::Sz(s)) => Some(s),
            _ => None,
        })
    }

    /// The on/off state of a switch row.
    fn switch_state(&self, row: &Row) -> Result<bool> {
        Ok(match row.method {
            Method::Reg { values, combine } => {
                let mut states = Vec::with_capacity(values.len());
                for v in values {
                    states.push(v.state(self.os.reg_read(v.hive, v.path, v.name)?.as_ref()));
                }
                match combine {
                    Combine::Any => states.iter().any(|s| *s),
                    Combine::All => states.iter().all(|s| *s),
                }
            }
            Method::KeyExists { hive, key, .. } => self.os.reg_key_exists(hive, key)?,
            Method::Dxg { key, absent_on } => match self.dxg_list()?.as_deref().and_then(|l| dxg::get(l, key)) {
                None => absent_on,
                Some(v) => v.parse::<u32>().map(|n| n != 0).unwrap_or(absent_on),
            },
            Method::SpiFlag { item, bit } => self.os.spi_get(item)? & bit != 0,
            Method::Animations => {
                self.os.spi_get(SpiItem::ClientAreaAnimation)? != 0 || self.os.spi_get(SpiItem::MinimizeAnimation)? != 0
            }
            Method::LangHotkeys => {
                // on (= shortcuts stopped) when both the language and the layout hotkey are "3" (not assigned)
                let mut all = true;
                for (name, default) in LANG_VALUES.iter().skip(1) {
                    let v = match self.os.reg_read(Hive::Hkcu, LANG_PATH, name)? {
                        Some(RegValue::Sz(s)) => s,
                        _ => default.to_string(),
                    };
                    all &= v.trim_end_matches('\0').trim() == "3";
                }
                all
            }
            Method::PowerSwitch(s) => self.os.power_read(s)?.ac != 0,
            Method::SleepSwitch => self.os.power_read(PowerSetting::Sleep)?.ac != 0,
            Method::FastStartup => self.os.hibernate_on()? && self.hiberboot_on()?,
            Method::Hags => match self.os.reg_read(Hive::Hklm, rows::HAGS_PATH, rows::HAGS_VALUE)? {
                // what Settings shows: the set value (it takes effect after a restart) …
                Some(RegValue::Dword(2)) => true,
                Some(RegValue::Dword(1)) => false,
                // … else what is running now (measured on the test PC: on, with no HwSchMode and the driver's default bit off)
                _ => {
                    let g = self.os.gpu_scheduling()?;
                    g.supported && g.enabled_now
                }
            },
            Method::Bluetooth => self.os.bluetooth()?.unwrap_or(false),
            Method::Copilot => self.os.copilot_installed()?,
            Method::PowerTimeout(_) | Method::FsoGames => {
                return Err(Error::WrongKind { row: row.id.into(), why: "not an on/off switch" })
            }
        })
    }

    // ------------------------------------------------------------------ snapshot / restore

    fn snapshot(&self, row: &Row) -> Result<Snapshot> {
        Ok(match row.method {
            Method::Reg { values, .. } => {
                let mut v = Vec::new();
                for s in values {
                    v.push((s.hive, s.path.to_string(), s.name.to_string(), self.os.reg_read(s.hive, s.path, s.name)?));
                }
                Snapshot::Reg(v)
            }
            Method::KeyExists { hive, key, .. } => Snapshot::Key { existed: self.os.reg_key_exists(hive, key)? },
            Method::Dxg { .. } => Snapshot::Reg(vec![(
                Hive::Hkcu,
                rows::DXG_PATH.into(),
                rows::DXG_VALUE.into(),
                self.os.reg_read(Hive::Hkcu, rows::DXG_PATH, rows::DXG_VALUE)?,
            )]),
            Method::SpiFlag { item, .. } => Snapshot::Spi(vec![(item, self.os.spi_get(item)?)]),
            Method::Animations => Snapshot::Spi(vec![
                (SpiItem::ClientAreaAnimation, self.os.spi_get(SpiItem::ClientAreaAnimation)?),
                (SpiItem::MinimizeAnimation, self.os.spi_get(SpiItem::MinimizeAnimation)?),
            ]),
            Method::LangHotkeys => {
                let mut v = Vec::new();
                for (name, _) in LANG_VALUES {
                    v.push((Hive::Hkcu, LANG_PATH.to_string(), name.to_string(), self.os.reg_read(Hive::Hkcu, LANG_PATH, name)?));
                }
                Snapshot::Reg(v)
            }
            Method::PowerSwitch(s) | Method::PowerTimeout(s) => Snapshot::Power(s, self.os.power_read(s)?),
            Method::SleepSwitch => Snapshot::Power(PowerSetting::Sleep, self.os.power_read(PowerSetting::Sleep)?),
            Method::FastStartup => Snapshot::Reg(vec![(
                Hive::Hklm,
                rows::HIBERBOOT_PATH.into(),
                rows::HIBERBOOT_VALUE.into(),
                self.os.reg_read(Hive::Hklm, rows::HIBERBOOT_PATH, rows::HIBERBOOT_VALUE)?,
            )]),
            Method::Hags => Snapshot::Reg(vec![(
                Hive::Hklm,
                rows::HAGS_PATH.into(),
                rows::HAGS_VALUE.into(),
                self.os.reg_read(Hive::Hklm, rows::HAGS_PATH, rows::HAGS_VALUE)?,
            )]),
            Method::Bluetooth => Snapshot::Bluetooth(self.os.bluetooth()?.unwrap_or(false)),
            Method::Copilot => Snapshot::Copilot(self.os.copilot_installed()?),
            Method::FsoGames => return Err(Error::WrongKind { row: row.id.into(), why: "use fso_set / fso_undo" }),
        })
    }

    /// Puts a snapshot back. Returns something to open when Windows can't restore it silently (Copilot).
    fn restore(&mut self, row: &Row, snap: &Snapshot) -> Result<Option<ChangeAction>> {
        match snap {
            Snapshot::Reg(values) => {
                for (hive, path, name, old) in values {
                    match old {
                        Some(v) => self.os.reg_write(*hive, path, name, v)?,
                        None => self.os.reg_delete_value(*hive, path, name)?,
                    }
                }
            }
            Snapshot::Key { existed } => {
                if let Method::KeyExists { hive, key, delete } = row.method {
                    if *existed {
                        self.os.reg_create_key(hive, key)?;
                        self.os.reg_write(hive, key, "", &RegValue::Sz(String::new()))?;
                    } else {
                        self.os.reg_delete_tree(hive, delete)?;
                    }
                }
            }
            Snapshot::Spi(items) => {
                for (item, v) in items {
                    self.os.spi_set(*item, *v)?;
                }
            }
            Snapshot::Power(s, v) => self.os.power_write(*s, *v)?,
            Snapshot::Bluetooth(on) => self.os.set_bluetooth(*on)?,
            Snapshot::Copilot(installed) => {
                let now = self.os.copilot_installed()?;
                if *installed && !now {
                    return Ok(Some(ChangeAction::OpenUri(COPILOT_STORE_URI.into())));
                } else if !*installed && now {
                    self.os.remove_copilot()?;
                }
            }
        }
        Ok(None)
    }

    // ------------------------------------------------------------------ apply

    fn check_admin(&self, row: &Row) -> Result<()> {
        if row.needs_admin() && !self.os.is_elevated() {
            return Err(Error::NeedsAdmin { row: row.id.into() });
        }
        Ok(())
    }

    /// Any OS-level "access denied" becomes NeedsAdmin for this row.
    fn admin_err(row: &Row) -> impl Fn(Error) -> Error + '_ {
        move |e| match e {
            Error::NeedsAdmin { .. } => Error::NeedsAdmin { row: row.id.into() },
            other => other,
        }
    }

    /// Switches a row on or off. Reads the value back afterwards; a value that didn't stick is `BlockedByWindows`.
    pub fn set(&mut self, id: &str, on: bool) -> Result<Applied> {
        let row = Self::row(id)?;
        if row.kind != Kind::Switch {
            return Err(Error::WrongKind { row: row.id.into(), why: "not an on/off switch" });
        }
        if row.method == Method::Bluetooth && self.os.bluetooth()?.is_none() {
            return Err(Error::NotAvailable { row: row.id.into(), reason: NO_BLUETOOTH });
        }
        if row.method == Method::Hags && !self.os.gpu_scheduling()?.supported {
            return Err(Error::NotAvailable { row: row.id.into(), reason: NO_GPU_SCHEDULING });
        }
        if row.method == Method::FastStartup && !self.os.hibernate_on()? {
            return Err(Error::Disabled { row: row.id.into(), reason: FAST_STARTUP_NEEDS_HIBERNATE });
        }
        let before = self.switch_state(row)?;
        if before == on {
            // already so: nothing written, no Explorer restart
            return Ok(Applied { id: row.id, value: Value::Switch(on), toast: None, explorer_restarted: false, open: None });
        }
        self.check_admin(row)?;
        let snap = self.snapshot(row)?;

        let open = self.write_switch(row, on).map_err(Self::admin_err(row))?;
        let after = self.switch_state(row)?;
        if after != on && open.is_none() {
            return Err(Error::BlockedByWindows { row: row.id.into(), settings_uri: row.settings_uri });
        }
        self.undo.insert(row.id.to_string(), snap);
        let explorer_restarted = self.after_steps(row)?;

        let toast = self.toast(row, on);
        Ok(Applied { id: row.id, value: Value::Switch(after), toast, explorer_restarted, open })
    }

    fn write_switch(&mut self, row: &Row, on: bool) -> Result<Option<ChangeAction>> {
        match row.method {
            Method::Reg { values, .. } => {
                for v in values {
                    if on && v.on_deletes {
                        self.os.reg_delete_value(v.hive, v.path, v.name)?;
                    } else {
                        let data = if on { v.on } else { v.off };
                        self.os.reg_write(v.hive, v.path, v.name, &data.to_value())?;
                    }
                }
            }
            Method::KeyExists { hive, key, delete } => {
                if on {
                    self.os.reg_create_key(hive, key)?;
                    self.os.reg_write(hive, key, "", &RegValue::Sz(String::new()))?;
                } else {
                    self.os.reg_delete_tree(hive, delete)?;
                }
            }
            Method::Dxg { key, .. } => {
                let list = self.dxg_list()?.unwrap_or_default();
                let new = dxg::set(&list, key, if on { "1" } else { "0" });
                self.os.reg_write(Hive::Hkcu, rows::DXG_PATH, rows::DXG_VALUE, &RegValue::Sz(new))?;
            }
            Method::SpiFlag { item, bit } => {
                let v = self.os.spi_get(item)?;
                self.os.spi_set(item, if on { v | bit } else { v & !bit })?;
            }
            Method::Animations => {
                self.os.spi_set(SpiItem::ClientAreaAnimation, on as u32)?;
                self.os.spi_set(SpiItem::MinimizeAnimation, on as u32)?;
            }
            Method::LangHotkeys => {
                for (name, default) in LANG_VALUES {
                    let v = if on { "3" } else { default };
                    self.os.reg_write(Hive::Hkcu, LANG_PATH, name, &RegValue::Sz(v.into()))?;
                }
            }
            Method::PowerSwitch(s) => {
                let cur = self.os.power_read(s)?;
                let v = on as u32;
                let dc = if self.os.has_battery() { v } else { cur.dc };
                self.os.power_write(s, PowerValues { ac: v, dc })?;
            }
            Method::SleepSwitch => {
                let cur = self.os.power_read(PowerSetting::Sleep)?;
                if on {
                    let back = self.sleep_restore.take().unwrap_or(PowerValues { ac: DEFAULT_SLEEP_SECONDS, dc: DEFAULT_SLEEP_SECONDS });
                    let dc = if self.os.has_battery() { back.dc } else { cur.dc };
                    self.os.power_write(PowerSetting::Sleep, PowerValues { ac: back.ac, dc })?;
                } else {
                    self.sleep_restore = Some(cur);
                    let dc = if self.os.has_battery() { 0 } else { cur.dc };
                    self.os.power_write(PowerSetting::Sleep, PowerValues { ac: 0, dc })?;
                }
            }
            Method::FastStartup => {
                self.os.reg_write(Hive::Hklm, rows::HIBERBOOT_PATH, rows::HIBERBOOT_VALUE, &RegValue::Dword(on as u32))?;
            }
            Method::Hags => {
                self.os.reg_write(Hive::Hklm, rows::HAGS_PATH, rows::HAGS_VALUE, &RegValue::Dword(if on { 2 } else { 1 }))?;
            }
            Method::Bluetooth => self.os.set_bluetooth(on)?,
            Method::Copilot => {
                if on {
                    // Windows can't reinstall a Store app silently: the menu opens the Store page
                    return Ok(Some(ChangeAction::OpenUri(COPILOT_STORE_URI.into())));
                }
                self.os.remove_copilot()?;
            }
            Method::PowerTimeout(_) | Method::FsoGames => {
                return Err(Error::WrongKind { row: row.id.into(), why: "not an on/off switch" })
            }
        }
        Ok(None)
    }

    /// Broadcast, layout-hotkey reload, Explorer restart. Returns whether Explorer was restarted.
    fn after_steps(&mut self, row: &Row) -> Result<bool> {
        match row.broadcast {
            Broadcast::None => {}
            Broadcast::Plain => self.os.broadcast_setting_change(None)?,
            Broadcast::Area(a) => self.os.broadcast_setting_change(Some(a))?,
        }
        if row.method == Method::LangHotkeys {
            self.os.reload_language_hotkeys()?;
        }
        if row.refresh_shell {
            // the value is already written; a failed refresh only means it shows in the next Explorer window
            let _ = self.os.refresh_shell();
        }
        if row.restarts_explorer() {
            // the value is already written; a failed restart only means it shows after the next sign-in
            return Ok(self.os.restart_explorer().is_ok());
        }
        Ok(false)
    }

    fn toast(&self, row: &Row, on: bool) -> Option<String> {
        let own = if on { row.toast_on } else { row.toast_off };
        own.or_else(|| row.badges.iter().find_map(|b: &Badge| b.toast())).map(str::to_string)
    }

    /// Sets a timeout row (Screen off after, Sleep after) to one of [`TIMEOUT_CHOICES`].
    pub fn set_timeout(&mut self, id: &str, t: Timeout) -> Result<Applied> {
        let row = Self::row(id)?;
        let Method::PowerTimeout(_) = row.method else {
            return Err(Error::WrongKind { row: row.id.into(), why: "not a timeout row" });
        };
        if !TIMEOUT_CHOICES.contains(&t) {
            return Err(Error::WrongKind { row: row.id.into(), why: "timeout not in the list" });
        }
        self.set_seconds(id, t.seconds())
    }

    /// A timeout row's (or the Sleep switch's) power values as they are: plugged in AND on battery (the app's change log
    /// keeps both, so a reset puts a laptop's own battery time back too - Order 036 review).
    pub fn power_values(&self, id: &str) -> Result<PowerValues> {
        let row = Self::row(id)?;
        let setting = match row.method {
            Method::PowerTimeout(s) => s,
            Method::SleepSwitch => PowerSetting::Sleep,
            _ => return Err(Error::WrongKind { row: row.id.into(), why: "not a power row" }),
        };
        self.os.power_read(setting)
    }

    /// Puts a timeout row's power values back exactly as they were (plugged in + on battery; 0 = never), read back.
    pub fn set_power_values(&mut self, id: &str, v: PowerValues) -> Result<Applied> {
        let row = Self::row(id)?;
        let Method::PowerTimeout(setting) = row.method else {
            return Err(Error::WrongKind { row: row.id.into(), why: "not a timeout row" });
        };
        let cur = self.os.power_read(setting)?;
        self.check_admin(row)?;
        self.os.power_write(setting, v).map_err(Self::admin_err(row))?;
        let back = self.os.power_read(setting)?;
        if back != v {
            return Err(Error::BlockedByWindows { row: row.id.into(), settings_uri: row.settings_uri });
        }
        self.undo.insert(row.id.to_string(), Snapshot::Power(setting, cur));
        Ok(Applied { id: row.id, value: Value::Timeout(Timeout::from_seconds(back.ac)), toast: None, explorer_restarted: false, open: None })
    }

    /// Sets a timeout row to ANY number of seconds (0 = never): the app's reset puts back a value the user had before,
    /// which may not be one of [`TIMEOUT_CHOICES`] (Order 036). Same rules as [`Toggles::set_timeout`] otherwise.
    pub fn set_seconds(&mut self, id: &str, seconds: u32) -> Result<Applied> {
        let row = Self::row(id)?;
        let Method::PowerTimeout(setting) = row.method else {
            return Err(Error::WrongKind { row: row.id.into(), why: "not a timeout row" });
        };
        let t = Timeout::from_seconds(seconds);
        let cur = self.os.power_read(setting)?;
        if setting == PowerSetting::Sleep && cur.ac == 0 {
            return Err(Error::Disabled { row: row.id.into(), reason: SLEEP_AFTER_SLEEP_OFF });
        }
        self.check_admin(row)?;
        let s = t.seconds();
        let dc = if self.os.has_battery() { s } else { cur.dc };
        self.os.power_write(setting, PowerValues { ac: s, dc }).map_err(Self::admin_err(row))?;
        let back = self.os.power_read(setting)?;
        if back.ac != s {
            return Err(Error::BlockedByWindows { row: row.id.into(), settings_uri: row.settings_uri });
        }
        self.undo.insert(row.id.to_string(), Snapshot::Power(setting, cur));
        Ok(Applied { id: row.id, value: Value::Timeout(Timeout::from_seconds(back.ac)), toast: None, explorer_restarted: false, open: None })
    }

    /// Switches the row back to the value it had before its last change. Undo again = redo.
    pub fn undo(&mut self, id: &str) -> Result<Applied> {
        let row = Self::row(id)?;
        let snap = self.undo.get(row.id).cloned().ok_or_else(|| Error::NothingToUndo(row.id.into()))?;
        self.check_admin(row)?;
        let now = self.snapshot(row)?;
        let open = self.restore(row, &snap).map_err(Self::admin_err(row))?;
        self.undo.insert(row.id.to_string(), now);
        let explorer_restarted = self.after_steps(row)?;
        let st = self.read(row.id)?;
        let toast = match st.value {
            Value::Switch(on) => self.toast(row, on),
            _ => None,
        };
        Ok(Applied { id: row.id, value: st.value, toast, explorer_restarted, open })
    }

    // ------------------------------------------------------------------ fullscreen optimizations per game

    /// Games whose fullscreen optimizations are off (our flag is in their Layers entry). The menu keeps its own list for
    /// games switched back on — Windows forgets those.
    pub fn fso_games(&self) -> Result<Vec<FsoGame>> {
        let mut games: Vec<FsoGame> = self
            .os
            .reg_values(Hive::Hkcu, rows::LAYERS_PATH)?
            .into_iter()
            .filter_map(|(name, v)| match v {
                RegValue::Sz(s) if fso::has_flag(&s) => Some(FsoGame { exe: name, fso_off: true }),
                _ => None,
            })
            .collect();
        games.sort_by_key(|a| a.exe.to_lowercase());
        Ok(games)
    }

    /// Is fullscreen-optimizations-off set for this exe?
    pub fn fso_state(&self, exe: &str) -> Result<bool> {
        Ok(matches!(self.os.reg_read(Hive::Hkcu, rows::LAYERS_PATH, exe)?, Some(RegValue::Sz(s)) if fso::has_flag(&s)))
    }

    fn check_exe(exe: &str) -> Result<()> {
        let ok = exe.to_lowercase().ends_with(".exe")
            && (exe.len() > 3 && exe.as_bytes()[1] == b':' && (exe.as_bytes()[2] == b'\\' || exe.as_bytes()[2] == b'/')
                || exe.starts_with(r"\\"));
        if ok {
            Ok(())
        } else {
            Err(Error::WrongKind { row: "fullscreen_optimizations_off".into(), why: "needs a full path to an .exe" })
        }
    }

    /// Adds a game ("Add game") or flips its switch: `fso_off = true` sets our flag, `false` removes only our flag (other
    /// compatibility flags of that exe stay).
    pub fn fso_set(&mut self, exe: &str, fso_off: bool) -> Result<Applied> {
        Self::check_exe(exe)?;
        let old = self.os.reg_read(Hive::Hkcu, rows::LAYERS_PATH, exe)?;
        let cur = match &old {
            Some(RegValue::Sz(s)) => s.clone(),
            _ => String::new(),
        };
        if fso_off {
            self.os.reg_write(Hive::Hkcu, rows::LAYERS_PATH, exe, &RegValue::Sz(fso::add_flag(&cur)))?;
        } else {
            match fso::remove_flag(&cur) {
                Some(rest) => self.os.reg_write(Hive::Hkcu, rows::LAYERS_PATH, exe, &RegValue::Sz(rest))?,
                None if old.is_some() => self.os.reg_delete_value(Hive::Hkcu, rows::LAYERS_PATH, exe)?,
                None => {}
            }
        }
        if self.fso_state(exe)? != fso_off {
            return Err(Error::BlockedByWindows { row: "fullscreen_optimizations_off".into(), settings_uri: None });
        }
        self.undo.insert(fso_undo_key(exe), Snapshot::Reg(vec![(Hive::Hkcu, rows::LAYERS_PATH.into(), exe.into(), old)]));
        Ok(Applied {
            id: "fullscreen_optimizations_off",
            value: Value::Games(self.fso_games()?),
            toast: None,
            explorer_restarted: false,
            open: None,
        })
    }

    /// "×" on a game: removes our flag (same as switching it on again); the menu drops it from its list.
    pub fn fso_remove(&mut self, exe: &str) -> Result<Applied> {
        self.fso_set(exe, false)
    }

    /// Puts a game's Layers entry back as it was before its last change.
    pub fn fso_undo(&mut self, exe: &str) -> Result<Applied> {
        let key = fso_undo_key(exe);
        let snap = self.undo.get(&key).cloned().ok_or_else(|| Error::NothingToUndo(key.clone()))?;
        let now = Snapshot::Reg(vec![(
            Hive::Hkcu,
            rows::LAYERS_PATH.into(),
            exe.into(),
            self.os.reg_read(Hive::Hkcu, rows::LAYERS_PATH, exe)?,
        )]);
        let row = Self::row("fullscreen_optimizations_off")?;
        self.restore(row, &snap)?;
        self.undo.insert(key, now);
        Ok(Applied {
            id: "fullscreen_optimizations_off",
            value: Value::Games(self.fso_games()?),
            toast: None,
            explorer_restarted: false,
            open: None,
        })
    }
}


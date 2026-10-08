//! The FAKE OS layer: an in-memory registry, SPI values, power plan, hibernate, Bluetooth, Copilot and a log of every side
//! effect (broadcasts, Explorer restarts, opened pages). Every behaviour of the crate is tested against it. Public so the menu's
//! own tests can use it too.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::error::{Error, Result};
use crate::os::*;

fn norm(path: &str) -> String {
    path.trim_matches('\\').to_lowercase()
}

/// The fake. Fields are public: tests set the starting state and look at the result.
#[derive(Clone, Debug)]
pub struct FakeOs {
    /// (hive, lowercased key path) -> lowercased value name -> (name as written, value)
    pub reg: BTreeMap<(Hive, String), BTreeMap<String, (String, RegValue)>>,
    pub elevated: bool,
    /// value names whose writes "succeed" but don't stick (UCPD simulation)
    pub blocked_values: HashSet<String>,
    /// operations that fail with an OS error (e.g. "spi_set", "power_write", "reg_write")
    pub failing_ops: HashSet<&'static str>,
    pub spi: HashMap<SpiItem, u32>,
    pub power: HashMap<PowerSetting, PowerValues>,
    pub battery: bool,
    pub hibernate_on: bool,
    /// what the graphics driver reports about GPU scheduling
    pub gpu: GpuScheduling,
    /// None = no Bluetooth radio
    pub bluetooth: Option<bool>,
    pub copilot: bool,
    pub browsers: Vec<RegisteredBrowser>,
    pub default_browser_progid: Option<String>,
    pub assoc: HashMap<String, AssocApp>,
    /// every side effect, in order ("broadcast:ImmersiveColorSet", "restart_explorer", "open_uri:…", …)
    pub log: Vec<String>,
}

impl Default for FakeOs {
    /// A plain Windows 11 PC: nothing set (Windows' defaults), not elevated, desktop (no battery), hibernate on,
    /// Bluetooth on, Copilot installed, Sticky/Filter/Toggle Keys pop-ups on (Windows' default flags), animations on.
    fn default() -> Self {
        let mut spi = HashMap::new();
        spi.insert(SpiItem::StickyKeysFlags, 0x1FE); // 510: Windows default incl. HOTKEYACTIVE
        spi.insert(SpiItem::FilterKeysFlags, 0x7E); // 126
        spi.insert(SpiItem::ToggleKeysFlags, 0x3E); // 62
        spi.insert(SpiItem::ClientAreaAnimation, 1);
        spi.insert(SpiItem::MinimizeAnimation, 1);
        let mut power = HashMap::new();
        power.insert(PowerSetting::ScreenOff, PowerValues { ac: 600, dc: 300 });
        power.insert(PowerSetting::Sleep, PowerValues { ac: 1800, dc: 900 });
        power.insert(PowerSetting::UsbSelectiveSuspend, PowerValues { ac: 1, dc: 1 });
        Self {
            reg: BTreeMap::new(),
            elevated: false,
            blocked_values: HashSet::new(),
            failing_ops: HashSet::new(),
            spi,
            power,
            battery: false,
            hibernate_on: true,
            gpu: GpuScheduling { supported: true, enabled_now: false, enabled_by_default: false },
            bluetooth: Some(true),
            copilot: true,
            browsers: Vec::new(),
            default_browser_progid: None,
            assoc: HashMap::new(),
            log: Vec::new(),
        }
    }
}

impl FakeOs {
    pub fn new() -> Self {
        Self::default()
    }

    fn fail(&self, op: &'static str) -> Result<()> {
        if self.failing_ops.contains(op) {
            Err(Error::Os { op: op.into(), code: 0x8000_4005 })
        } else {
            Ok(())
        }
    }

    /// Writes under HKLM or `Software\Policies` need admin, like on Windows.
    fn needs_admin(hive: Hive, path: &str) -> bool {
        hive == Hive::Hklm || norm(path).starts_with(r"software\policies")
    }

    fn check_write(&self, hive: Hive, path: &str) -> Result<()> {
        if Self::needs_admin(hive, path) && !self.elevated {
            return Err(Error::NeedsAdmin { row: format!("{hive:?}\\{path}") });
        }
        Ok(())
    }

    /// Test helper: set a value directly (no admin check, no log).
    pub fn put(&mut self, hive: Hive, path: &str, name: &str, value: RegValue) {
        self.reg.entry((hive, norm(path))).or_default().insert(name.to_lowercase(), (name.to_string(), value));
    }

    /// Test helper: read a value directly.
    pub fn get(&self, hive: Hive, path: &str, name: &str) -> Option<RegValue> {
        self.reg.get(&(hive, norm(path))).and_then(|k| k.get(&name.to_lowercase())).map(|(_, v)| v.clone())
    }

    /// How many side effects of this kind were logged.
    pub fn count(&self, prefix: &str) -> usize {
        self.log.iter().filter(|l| l.starts_with(prefix)).count()
    }
}

impl TogglesOs for FakeOs {
    fn reg_read(&self, hive: Hive, path: &str, name: &str) -> Result<Option<RegValue>> {
        self.fail("reg_read")?;
        Ok(self.get(hive, path, name))
    }

    fn reg_write(&mut self, hive: Hive, path: &str, name: &str, value: &RegValue) -> Result<()> {
        self.fail("reg_write")?;
        self.check_write(hive, path)?;
        if self.blocked_values.contains(&name.to_lowercase()) {
            return Ok(()); // "succeeds", doesn't stick
        }
        self.put(hive, path, name, value.clone());
        Ok(())
    }

    fn reg_delete_value(&mut self, hive: Hive, path: &str, name: &str) -> Result<()> {
        self.fail("reg_delete_value")?;
        self.check_write(hive, path)?;
        if let Some(k) = self.reg.get_mut(&(hive, norm(path))) {
            k.remove(&name.to_lowercase());
        }
        Ok(())
    }

    fn reg_key_exists(&self, hive: Hive, path: &str) -> Result<bool> {
        let p = norm(path);
        let child = format!("{p}\\");
        Ok(self.reg.keys().any(|(h, k)| *h == hive && (*k == p || k.starts_with(&child))))
    }

    fn reg_create_key(&mut self, hive: Hive, path: &str) -> Result<()> {
        self.fail("reg_create_key")?;
        self.check_write(hive, path)?;
        self.reg.entry((hive, norm(path))).or_default();
        Ok(())
    }

    fn reg_delete_tree(&mut self, hive: Hive, path: &str) -> Result<()> {
        self.fail("reg_delete_tree")?;
        self.check_write(hive, path)?;
        let p = norm(path);
        let child = format!("{p}\\");
        self.reg.retain(|(h, k), _| !(*h == hive && (*k == p || k.starts_with(&child))));
        Ok(())
    }

    fn reg_values(&self, hive: Hive, path: &str) -> Result<Vec<(String, RegValue)>> {
        Ok(self
            .reg
            .get(&(hive, norm(path)))
            .map(|k| k.values().map(|(n, v)| (n.clone(), v.clone())).collect())
            .unwrap_or_default())
    }

    fn is_elevated(&self) -> bool {
        self.elevated
    }

    fn spi_get(&self, item: SpiItem) -> Result<u32> {
        self.fail("spi_get")?;
        Ok(*self.spi.get(&item).unwrap_or(&0))
    }

    fn spi_set(&mut self, item: SpiItem, value: u32) -> Result<()> {
        self.fail("spi_set")?;
        self.spi.insert(item, value);
        self.log.push(format!("spi_set:{item:?}={value:#x}"));
        Ok(())
    }

    fn reload_language_hotkeys(&mut self) -> Result<()> {
        self.fail("reload_language_hotkeys")?;
        self.log.push("reload_language_hotkeys".into());
        Ok(())
    }

    fn power_read(&self, setting: PowerSetting) -> Result<PowerValues> {
        self.fail("power_read")?;
        Ok(*self.power.get(&setting).unwrap_or(&PowerValues { ac: 0, dc: 0 }))
    }

    fn power_write(&mut self, setting: PowerSetting, values: PowerValues) -> Result<()> {
        self.fail("power_write")?;
        self.power.insert(setting, values);
        self.log.push(format!("power_write:{setting:?}={}/{}", values.ac, values.dc));
        Ok(())
    }

    fn has_battery(&self) -> bool {
        self.battery
    }

    fn hibernate_on(&self) -> Result<bool> {
        self.fail("hibernate")?;
        Ok(self.hibernate_on)
    }

    fn gpu_scheduling(&self) -> Result<GpuScheduling> {
        self.fail("gpu_scheduling")?;
        Ok(self.gpu)
    }

    fn bluetooth(&self) -> Result<Option<bool>> {
        self.fail("bluetooth")?;
        Ok(self.bluetooth)
    }

    fn set_bluetooth(&mut self, on: bool) -> Result<()> {
        self.fail("set_bluetooth")?;
        match self.bluetooth {
            None => Err(Error::os("no radio", 0x8007_0490u32 as i64)),
            Some(_) => {
                self.bluetooth = Some(on);
                self.log.push(format!("set_bluetooth:{on}"));
                Ok(())
            }
        }
    }

    fn copilot_installed(&self) -> Result<bool> {
        self.fail("copilot_installed")?;
        Ok(self.copilot)
    }

    fn remove_copilot(&mut self) -> Result<()> {
        self.fail("remove_copilot")?;
        self.copilot = false;
        self.log.push("remove_copilot".into());
        Ok(())
    }

    fn broadcast_setting_change(&mut self, area: Option<&str>) -> Result<()> {
        self.log.push(format!("broadcast:{}", area.unwrap_or("")));
        Ok(())
    }

    fn restart_explorer(&mut self) -> Result<()> {
        self.fail("restart_explorer")?;
        self.log.push("restart_explorer".into());
        Ok(())
    }

    fn refresh_shell(&mut self) -> Result<()> {
        self.fail("refresh_shell")?;
        self.log.push("refresh_shell".into());
        Ok(())
    }

    fn registered_browsers(&self) -> Result<Vec<RegisteredBrowser>> {
        Ok(self.browsers.clone())
    }

    fn default_browser_progid(&self) -> Result<Option<String>> {
        Ok(self.default_browser_progid.clone())
    }

    fn assoc_app(&self, what: &str) -> Result<Option<AssocApp>> {
        Ok(self.assoc.get(what).cloned())
    }

    fn open_uri(&mut self, uri: &str) -> Result<()> {
        self.log.push(format!("open_uri:{uri}"));
        Ok(())
    }

    fn open_with_dialog(&mut self, ext: &str) -> Result<()> {
        self.log.push(format!("open_with:{ext}"));
        Ok(())
    }
}

//! The REAL Windows layer. Every call is user-level (no admin). Each change has a test against the fake; on the real
//! PC only reads and `SDC_VALIDATE` checks were run in this order (SAFETY: never change the owner's real displays).

pub mod adl;
pub mod config;
pub mod ddc;
pub mod dpi;
pub mod edid;
pub mod modes;
pub mod nvapi;
pub mod watch;
pub mod wmi;

use crate::error::{DisplayError, Result};
use crate::os::DisplayOs;
use crate::picture::DdcCrashGuard;
use crate::types::*;
use std::path::PathBuf;
use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW};

pub(crate) fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|c| *c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

/// The GDI name (`\\.\DISPLAY1`) of an HMONITOR.
pub(crate) fn gdi_name_of(h: HMONITOR) -> Option<String> {
    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    unsafe { GetMonitorInfoW(h, &mut mi.monitorInfo as *mut MONITORINFO) }.as_bool().then(|| wide_to_string(&mi.szDevice))
}

/// The HMONITOR of a GDI name.
pub(crate) fn hmonitor_for(gdi_name: &str) -> Option<HMONITOR> {
    struct Find<'a> {
        name: &'a str,
        found: Option<HMONITOR>,
    }
    unsafe extern "system" fn cb(h: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> BOOL {
        let f = unsafe { &mut *(lp.0 as *mut Find) };
        if gdi_name_of(h).map(|n| n.eq_ignore_ascii_case(f.name)).unwrap_or(false) {
            f.found = Some(h);
            return BOOL(0);
        }
        BOOL(1)
    }
    let mut f = Find { name: gdi_name, found: None };
    let _ = unsafe { EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut f as *mut Find as isize)) };
    f.found
}

fn number_of(gdi_name: &str) -> u32 {
    let digits: String = gdi_name.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
    digits.parse().unwrap_or(0)
}

/// The real OS layer.
pub struct WinDisplayOs {
    guard: DdcCrashGuard,
    /// `read_only()`: every change is refused on its first line (proof programs; TECH_RULES).
    read_only: bool,
    /// Order 042: the modes this app set per monitor (their GPU scaling is sent again with every other change: `config::keep_scaling`)
    set_modes: Vec<(MonitorId, Mode)>,
}

impl WinDisplayOs {
    /// `ddc_guard_dir`: the folder for the DDC crash-guard markers (the app's settings folder); `None` = no guard.
    pub fn new(ddc_guard_dir: Option<PathBuf>) -> Self {
        Self { guard: DdcCrashGuard::new(ddc_guard_dir), read_only: false, set_modes: Vec::new() }
    }

    /// Real reads, every change refused with `DisplayError::ReadOnly` before anything is touched — what the proof
    /// programs (`display-show`, `display-validate`) run on. The `validate_*` checks stay available (SDC_VALIDATE only).
    pub fn read_only(ddc_guard_dir: Option<PathBuf>) -> Self {
        Self { guard: DdcCrashGuard::new(ddc_guard_dir), read_only: true, set_modes: Vec::new() }
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    fn refuse(&self, what: &'static str) -> Result<()> {
        if self.read_only { Err(DisplayError::ReadOnly(what)) } else { Ok(()) }
    }

    fn gdi(&self, id: &MonitorId) -> Result<String> {
        let s = config::query()?;
        let v = config::views(&s);
        Ok(config::find(&v, id)?.gdi_name.clone())
    }

    /// PROOF ONLY: asks Windows whether a mode change would work (`SDC_VALIDATE`) — changes nothing on screen.
    pub fn validate_mode(&self, id: &MonitorId, mode: &Mode) -> Result<()> {
        config::set_mode(id, mode, config::Submit::ValidateOnly, &[])
    }

    /// PROOF ONLY: the same check without SDC_ALLOW_CHANGES (Windows must accept the mode exactly) — changes nothing.
    pub fn validate_mode_strict(&self, id: &MonitorId, mode: &Mode) -> Result<()> {
        config::set_mode(id, mode, config::Submit::ValidateStrict, &[])
    }

    /// PROOF ONLY: asks Windows whether making `id` the main display would work (`SDC_VALIDATE`) — changes nothing.
    pub fn validate_main(&self, id: &MonitorId) -> Result<()> {
        config::set_main(id, config::Submit::ValidateOnly, &[])
    }

    /// PROOF ONLY: the main-display check without SDC_ALLOW_CHANGES — changes nothing.
    pub fn validate_main_strict(&self, id: &MonitorId) -> Result<()> {
        config::set_main(id, config::Submit::ValidateStrict, &[])
    }

    /// The GPU vendor driving a monitor (from its DXGI adapter).
    pub fn vendor(&self, id: &MonitorId) -> Option<GpuVendor> {
        modes::vendor(&self.gdi(id).ok()?)
    }

    /// A mode this app set on `id` (its scaling goes along with every later change of another monitor).
    fn remember(&mut self, id: &MonitorId, mode: &Mode) {
        self.set_modes.retain(|(m, _)| m != id);
        self.set_modes.push((id.clone(), *mode));
    }

    /// The DDC crash guard (the UI offers "turn DDC back on" with `clear`).
    pub fn ddc_guard(&self) -> &DdcCrashGuard {
        &self.guard
    }
}

impl DisplayOs for WinDisplayOs {
    fn monitors(&self) -> Result<Vec<MonitorInfo>> {
        let s = config::query()?;
        let mut out: Vec<MonitorInfo> = config::views(&s)
            .into_iter()
            .map(|v| MonitorInfo {
                id: MonitorId(v.device_path.clone()),
                number: number_of(&v.gdi_name),
                gdi_name: v.gdi_name.clone(),
                name: v.friendly_name.clone(),
                diagonal_inches: edid::read(&v.device_path).and_then(|e| edid::diagonal_inches(&e)),
                is_main: v.x == 0 && v.y == 0,
                rect: Rect { x: v.x, y: v.y, width: v.width, height: v.height },
                native: v.native,
                current: Mode { width: v.width, height: v.height, refresh: v.refresh, scaling: v.scaling },
                dpi: dpi::get(v.source_adapter, v.source_id),
                hdr: v.hdr,
            })
            .collect();
        out.sort_by_key(|m| m.number);
        Ok(out)
    }

    fn modes(&self, id: &MonitorId) -> Result<Vec<VideoMode>> {
        modes::list(&self.gdi(id)?)
    }

    fn apply_mode(&mut self, id: &MonitorId, mode: &Mode, save: bool) -> Result<()> {
        self.refuse("apply_mode")?;
        let how = if save { config::Submit::ApplyStrict } else { config::Submit::ApplyStrictTemporary };
        config::set_mode(id, mode, how, &self.set_modes)?;
        self.remember(id, mode);
        Ok(())
    }

    fn save_current(&mut self, id: &MonitorId, mode: &Mode) -> Result<()> {
        self.refuse("save_current")?;
        // the mode Apply set, submitted again with SDC_SAVE_TO_DATABASE (its scaling explicit, see the trait)
        config::set_mode(id, mode, config::Submit::ApplyStrict, &self.set_modes)?;
        self.remember(id, mode);
        Ok(())
    }

    fn set_main(&mut self, id: &MonitorId) -> Result<()> {
        self.refuse("set_main")?;
        config::set_main(id, config::Submit::ApplyStrict, &self.set_modes)
    }

    fn set_dpi_percent(&mut self, id: &MonitorId, percent: u32) -> Result<()> {
        self.refuse("set_dpi_percent")?;
        let s = config::query()?;
        let v = config::views(&s);
        let pv = config::find(&v, id)?;
        dpi::set(pv.source_adapter, pv.source_id, percent)
    }

    fn ddc_get(&mut self, id: &MonitorId, vcp: Vcp) -> Result<VcpValue> {
        if crate::picture::ddc_excluded(id) {
            return Err(DisplayError::DdcExcludedModel);
        }
        if self.guard.is_blocked(&id.0) {
            return Err(DisplayError::DdcBlockedAfterCrash);
        }
        let gdi = self.gdi(id)?;
        self.guard.run(&id.0, || ddc::get(&gdi, vcp))
    }

    fn ddc_set(&mut self, id: &MonitorId, vcp: Vcp, value: u32) -> Result<()> {
        self.refuse("ddc_set")?;
        if crate::picture::ddc_excluded(id) {
            return Err(DisplayError::DdcExcludedModel);
        }
        if self.guard.is_blocked(&id.0) {
            return Err(DisplayError::DdcBlockedAfterCrash);
        }
        let gdi = self.gdi(id)?;
        self.guard.run(&id.0, || ddc::set(&gdi, vcp, value))
    }

    fn vibrance_get(&mut self, id: &MonitorId) -> Result<VibranceRaw> {
        let gdi = self.gdi(id)?;
        match modes::vendor(&gdi) {
            Some(GpuVendor::Nvidia) => nvapi::get(&gdi),
            Some(GpuVendor::Amd) => adl::get(&gdi),
            v => Err(DisplayError::VibranceUnsupported(v)),
        }
    }

    fn vibrance_set(&mut self, id: &MonitorId, level: i32) -> Result<()> {
        self.refuse("vibrance_set")?;
        let gdi = self.gdi(id)?;
        match modes::vendor(&gdi) {
            Some(GpuVendor::Nvidia) => nvapi::set(&gdi, level),
            Some(GpuVendor::Amd) => adl::set(&gdi, level),
            v => Err(DisplayError::VibranceUnsupported(v)),
        }
    }

    fn needs_admin(&self, _kind: ChangeKind) -> bool {
        false
    }
}

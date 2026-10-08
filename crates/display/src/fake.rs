//! A fake OS layer: a scripted set of monitors held in memory. Every behaviour of the crate is tested against it;
//! nothing here touches Windows. Public so the menu app's own tests can use it too.

use crate::error::{DisplayError, Result};
use crate::os::DisplayOs;
use crate::types::*;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct FakeMonitor {
    pub info: MonitorInfo,
    pub modes: Vec<VideoMode>,
    /// `None` = the monitor doesn't answer DDC/CI.
    pub ddc: Option<HashMap<Vcp, VcpValue>>,
    /// `None` = no vibrance on this GPU.
    pub vibrance: Option<VibranceRaw>,
}

/// One recorded change call (tests check what was sent to the "OS").
#[derive(Clone, Debug, PartialEq)]
pub enum FakeCall {
    ApplyMode(MonitorId, Mode),
    SaveCurrent(MonitorId),
    SetMain(MonitorId),
    SetDpi(MonitorId, u32),
    DdcSet(MonitorId, Vcp, u32),
    VibranceSet(MonitorId, i32),
}

#[derive(Clone, Debug, Default)]
pub struct FakeDisplayOs {
    pub monitors: Vec<FakeMonitor>,
    pub calls: Vec<FakeCall>,
    /// Change kinds that answer "needs admin" (to test the admin path).
    pub admin_required: HashSet<ChangeKind>,
    /// When set, the next change call fails with this OS error text.
    pub fail_next_change: Option<String>,
    /// Windows' stored display setting per monitor (what a reboot / crash would come back to). Missing = the start mode.
    pub saved: HashMap<MonitorId, Mode>,
}

impl FakeDisplayOs {
    pub fn new(monitors: Vec<FakeMonitor>) -> Self {
        Self { monitors, ..Default::default() }
    }

    fn find(&self, id: &MonitorId) -> Result<usize> {
        self.monitors.iter().position(|m| &m.info.id == id).ok_or_else(|| DisplayError::MonitorNotFound(id.clone()))
    }

    fn gate(&mut self, kind: ChangeKind) -> Result<()> {
        if self.admin_required.contains(&kind) {
            return Err(DisplayError::NeedsAdmin(kind));
        }
        if let Some(e) = self.fail_next_change.take() {
            return Err(DisplayError::os("fake", e));
        }
        Ok(())
    }

    /// What Windows would come back to after a crash / reboot (for asserts).
    pub fn saved_mode(&self, id: &MonitorId) -> Mode {
        self.saved.get(id).copied().unwrap_or_else(|| self.current(id))
    }

    /// The fake's current mode of a monitor (for asserts).
    pub fn current(&self, id: &MonitorId) -> Mode {
        self.monitors[self.find(id).expect("fake monitor")].info.current
    }

    /// A ready-made two-monitor setup like the drawing: 1 · DELL 27″ (165 Hz, main) and 2 · LG 24″ (143.98 Hz),
    /// both with DDC, NVIDIA vibrance.
    pub fn two_monitors() -> Self {
        let r = RefreshRate::new;
        let dell_modes: Vec<VideoMode> = [(2560, 1440), (1920, 1080), (1440, 1080), (1280, 960), (1280, 720)]
            .iter()
            .flat_map(|(w, h)| {
                [r(164_950, 1000), r(144_000, 1000), r(120_000, 1000), r(119_880, 1000), r(60_000, 1000), r(60_000, 1001)]
                    .into_iter()
                    .map(move |rr| VideoMode { width: *w, height: *h, refresh: rr })
            })
            .collect();
        let lg_modes: Vec<VideoMode> = [(1920, 1080), (1440, 1080), (1280, 960), (1024, 768)]
            .iter()
            .flat_map(|(w, h)| {
                [r(143_981, 1000), r(119_982, 1000), r(60_000, 1000)]
                    .into_iter()
                    .map(move |rr| VideoMode { width: *w, height: *h, refresh: rr })
            })
            .collect();
        let ddc = |b, c| {
            let mut m = HashMap::new();
            m.insert(Vcp::Brightness, VcpValue { current: b, max: 100 });
            m.insert(Vcp::Contrast, VcpValue { current: c, max: 100 });
            Some(m)
        };
        let nv = Some(VibranceRaw { vendor: GpuVendor::Nvidia, current: 0, min: -63, max: 63, default: 0 });
        let dpi = |cur| Some(DpiScale { current_percent: cur, recommended_percent: 100, allowed_percent: vec![100, 125, 150, 175] });
        Self::new(vec![
            FakeMonitor {
                info: MonitorInfo {
                    id: MonitorId("fake-dell".into()),
                    number: 1,
                    gdi_name: r"\\.\DISPLAY1".into(),
                    name: "DELL S2721DGF".into(),
                    diagonal_inches: Some(27.0),
                    is_main: true,
                    rect: Rect { x: 0, y: 0, width: 2560, height: 1440 },
                    native: Some((2560, 1440)),
                    current: Mode { width: 2560, height: 1440, refresh: r(164_950, 1000), scaling: GpuScaling::KeepAspect },
                    dpi: dpi(100),
                    hdr: Some(HdrInfo { supported: true, enabled: false }),
                },
                modes: dell_modes,
                ddc: ddc(70, 75),
                vibrance: nv,
            },
            FakeMonitor {
                info: MonitorInfo {
                    id: MonitorId("fake-lg".into()),
                    number: 2,
                    gdi_name: r"\\.\DISPLAY2".into(),
                    name: "LG 24GL600F".into(),
                    diagonal_inches: Some(23.6),
                    is_main: false,
                    rect: Rect { x: 2560, y: 0, width: 1920, height: 1080 },
                    native: Some((1920, 1080)),
                    current: Mode { width: 1920, height: 1080, refresh: r(143_981, 1000), scaling: GpuScaling::Stretch },
                    dpi: dpi(100),
                    hdr: Some(HdrInfo { supported: false, enabled: false }),
                },
                modes: lg_modes,
                ddc: None,
                vibrance: nv,
            },
        ])
    }

    /// The menu drawing's sample (menu-v22 `MONS`, Order 019's pixel comparisons): 1 · DELL S2721DGF 27″ at
    /// 1920 × 1080 · 164.95 Hz Keep aspect (main; brightness 70 %, contrast 50 %, vibrance 50 %) and 2 · LG 24GL600F 24″ at
    /// 1920 × 1080 · 143.98 Hz (no DDC/CI; vibrance 50 %). Each monitor reports its drawing rates at every size.
    pub fn drawing_sample() -> Self {
        let sizes = [(1920, 1080), (1680, 1050), (1600, 900), (1440, 1080), (1280, 1024), (1280, 960), (1280, 720), (1024, 768)];
        let modes = |rates: &[u32]| -> Vec<VideoMode> {
            sizes
                .iter()
                .flat_map(|(w, h)| rates.iter().map(move |r| VideoMode { width: *w, height: *h, refresh: RefreshRate::new(*r, 1000) }))
                .collect()
        };
        let nv = Some(VibranceRaw { vendor: GpuVendor::Nvidia, current: 0, min: -63, max: 63, default: 0 });
        let dpi = Some(DpiScale { current_percent: 100, recommended_percent: 100, allowed_percent: vec![100, 125, 150, 175] });
        let mut ddc = HashMap::new();
        ddc.insert(Vcp::Brightness, VcpValue { current: 70, max: 100 });
        ddc.insert(Vcp::Contrast, VcpValue { current: 50, max: 100 });
        let mon = |id: &str, n: u32, name: &str, inch: f32, main: bool, x: i32, hz: u32| MonitorInfo {
            id: MonitorId(id.into()),
            number: n,
            gdi_name: format!(r"\.\DISPLAY{n}"),
            name: name.into(),
            diagonal_inches: Some(inch),
            is_main: main,
            rect: Rect { x, y: 0, width: 1920, height: 1080 },
            native: Some((1920, 1080)),
            current: Mode { width: 1920, height: 1080, refresh: RefreshRate::new(hz, 1000), scaling: GpuScaling::KeepAspect },
            dpi: dpi.clone(),
            hdr: Some(HdrInfo { supported: false, enabled: false }),
        };
        Self::new(vec![
            FakeMonitor {
                info: mon("fake-dell", 1, "DELL S2721DGF", 27.0, true, 0, 164_950),
                modes: modes(&[164_950, 143_970, 120_000, 119_880, 99_950, 60_000, 59_940]),
                ddc: Some(ddc),
                vibrance: nv,
            },
            FakeMonitor {
                info: mon("fake-lg", 2, "LG 24GL600F", 24.0, false, 1920, 143_980),
                modes: modes(&[143_980, 119_980, 99_950, 60_000, 59_940]),
                ddc: None,
                vibrance: nv,
            },
        ])
    }
}

impl DisplayOs for FakeDisplayOs {
    fn monitors(&self) -> Result<Vec<MonitorInfo>> {
        let mut v: Vec<MonitorInfo> = self.monitors.iter().map(|m| m.info.clone()).collect();
        v.sort_by_key(|m| m.number);
        Ok(v)
    }

    fn modes(&self, id: &MonitorId) -> Result<Vec<VideoMode>> {
        Ok(self.monitors[self.find(id)?].modes.clone())
    }

    fn apply_mode(&mut self, id: &MonitorId, mode: &Mode, save: bool) -> Result<()> {
        let i = self.find(id)?;
        let start = self.monitors[i].info.current;
        self.saved.entry(id.clone()).or_insert(start);
        self.gate(ChangeKind::Mode)?;
        let m = &mut self.monitors[i];
        if !m.modes.iter().any(|v| *v == mode.video()) {
            return Err(DisplayError::os("SetDisplayConfig (fake)", "mode not in the monitor's list"));
        }
        m.info.current = *mode;
        m.info.rect.width = mode.width;
        m.info.rect.height = mode.height;
        self.calls.push(FakeCall::ApplyMode(id.clone(), *mode));
        if save {
            self.saved.insert(id.clone(), *mode);
        }
        Ok(())
    }

    fn save_current(&mut self, id: &MonitorId, mode: &Mode) -> Result<()> {
        self.find(id)?;
        self.gate(ChangeKind::Mode)?;
        self.saved.insert(id.clone(), *mode);
        self.calls.push(FakeCall::SaveCurrent(id.clone()));
        Ok(())
    }

    fn set_main(&mut self, id: &MonitorId) -> Result<()> {
        let i = self.find(id)?;
        self.gate(ChangeKind::MainDisplay)?;
        let (dx, dy) = (self.monitors[i].info.rect.x, self.monitors[i].info.rect.y);
        for (j, m) in self.monitors.iter_mut().enumerate() {
            m.info.rect.x -= dx;
            m.info.rect.y -= dy;
            m.info.is_main = j == i;
        }
        self.calls.push(FakeCall::SetMain(id.clone()));
        Ok(())
    }

    fn set_dpi_percent(&mut self, id: &MonitorId, percent: u32) -> Result<()> {
        let i = self.find(id)?;
        self.gate(ChangeKind::DpiScale)?;
        let dpi = self.monitors[i].info.dpi.as_mut().ok_or(DisplayError::DpiNotOffered(percent))?;
        if !dpi.allowed_percent.contains(&percent) {
            return Err(DisplayError::DpiNotOffered(percent));
        }
        dpi.current_percent = percent;
        self.calls.push(FakeCall::SetDpi(id.clone(), percent));
        Ok(())
    }

    fn ddc_get(&mut self, id: &MonitorId, vcp: Vcp) -> Result<VcpValue> {
        let i = self.find(id)?;
        self.monitors[i].ddc.as_ref().and_then(|d| d.get(&vcp).copied()).ok_or(DisplayError::DdcNoAnswer)
    }

    fn ddc_set(&mut self, id: &MonitorId, vcp: Vcp, value: u32) -> Result<()> {
        let i = self.find(id)?;
        self.gate(ChangeKind::Ddc)?;
        let d = self.monitors[i].ddc.as_mut().ok_or(DisplayError::DdcNoAnswer)?;
        let v = d.get_mut(&vcp).ok_or(DisplayError::DdcNoAnswer)?;
        v.current = value.min(v.max);
        self.calls.push(FakeCall::DdcSet(id.clone(), vcp, value));
        Ok(())
    }

    fn vibrance_get(&mut self, id: &MonitorId) -> Result<VibranceRaw> {
        let i = self.find(id)?;
        self.monitors[i].vibrance.ok_or(DisplayError::VibranceUnsupported(None))
    }

    fn vibrance_set(&mut self, id: &MonitorId, level: i32) -> Result<()> {
        let i = self.find(id)?;
        self.gate(ChangeKind::Vibrance)?;
        let v = self.monitors[i].vibrance.as_mut().ok_or(DisplayError::VibranceUnsupported(None))?;
        v.current = crate::picture::clamp_any(level, v.min, v.max);
        self.calls.push(FakeCall::VibranceSet(id.clone(), level));
        Ok(())
    }

    fn needs_admin(&self, kind: ChangeKind) -> bool {
        self.admin_required.contains(&kind)
    }
}

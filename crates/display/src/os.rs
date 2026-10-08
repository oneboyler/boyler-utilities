//! The OS layer. `WinDisplayOs` (src/win) is the real one; `FakeDisplayOs` (src/fake.rs) is for tests.
//! The service never calls Windows directly — only through this trait.

use crate::error::Result;
use crate::types::{ChangeKind, MonitorId, MonitorInfo, Mode, Vcp, VcpValue, VibranceRaw, VideoMode};

pub trait DisplayOs {
    /// Every active monitor, in monitor-number order.
    fn monitors(&self) -> Result<Vec<MonitorInfo>>;

    /// Every mode the monitor reports (exact rates), any order, no duplicates needed.
    fn modes(&self, id: &MonitorId) -> Result<Vec<VideoMode>>;

    /// Sets W × H × Hz and scaling on one monitor (no keep/revert here — the service does that).
    /// `save` = also store it as Windows' saved display setting (survives a reboot / crash). The keep countdown and
    /// automatic per-app switches apply with `save = false`, so a crash in between leaves Windows on its stored mode.
    fn apply_mode(&mut self, id: &MonitorId, mode: &Mode, save: bool) -> Result<()>;

    /// Stores `mode` (what Apply set on this monitor, scaling included) as Windows' saved display setting (after Keep).
    /// Order 042: the mode is passed, not read back - Windows' read-back of the path does not carry the GPU scaling
    /// picked with Apply, so saving the read-back configuration put Stretch / Black bars back to Keep aspect (the owner's
    /// test 2).
    fn save_current(&mut self, id: &MonitorId, mode: &Mode) -> Result<()>;

    /// Makes this monitor the main display (the others keep their places relative to it).
    fn set_main(&mut self, id: &MonitorId) -> Result<()>;

    /// Sets the Windows scaling % (must be one of `DpiScale::allowed_percent`).
    fn set_dpi_percent(&mut self, id: &MonitorId, percent: u32) -> Result<()>;

    /// Reads a DDC/CI value from the monitor itself.
    fn ddc_get(&mut self, id: &MonitorId, vcp: Vcp) -> Result<VcpValue>;

    /// Writes a DDC/CI value to the monitor itself (`value` in the monitor's own 0..max range).
    fn ddc_set(&mut self, id: &MonitorId, vcp: Vcp, value: u32) -> Result<()>;

    /// Reads the driver's vibrance / saturation levels for this monitor.
    fn vibrance_get(&mut self, id: &MonitorId) -> Result<VibranceRaw>;

    /// Sets the driver level (within `VibranceRaw::min..=max`).
    fn vibrance_set(&mut self, id: &MonitorId, level: i32) -> Result<()>;

    /// Whether a change needs administrator rights on this PC. On Windows none of the Display changes do
    /// (SetDisplayConfig, DisplayConfigSetDeviceInfo, DDC/CI, NVAPI DVC, ADL all run as the user).
    fn needs_admin(&self, kind: ChangeKind) -> bool;
}

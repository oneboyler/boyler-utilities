//! Plain data shared by the whole crate (no Windows types here).

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

/// Stable id of one monitor: the Windows monitor device path
/// (e.g. `\\?\DISPLAY#DEL41B6#5&2a6e...&UID4352#{e6f07b5f-...}`). Same monitor on the same port = same id.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MonitorId(pub String);

impl fmt::Display for MonitorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An exact refresh rate as the monitor/driver reports it: `num / den` Hz (e.g. 60000/1001 = 59.94 Hz).
/// Two rates are equal when they agree to the millihertz.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct RefreshRate {
    pub num: u32,
    pub den: u32,
}

impl RefreshRate {
    pub fn new(num: u32, den: u32) -> Self {
        Self { num, den: den.max(1) }
    }
    /// A whole-number rate (e.g. `RefreshRate::whole(60)`).
    pub fn whole(hz: u32) -> Self {
        Self { num: hz, den: 1 }
    }
    /// The rate in Hz.
    pub fn hz(&self) -> f64 {
        self.num as f64 / self.den.max(1) as f64
    }
    /// The rate in millihertz, rounded (used for comparing).
    pub fn millihz(&self) -> u64 {
        ((self.num as u64) * 1000 + (self.den.max(1) as u64) / 2) / self.den.max(1) as u64
    }
}

impl PartialEq for RefreshRate {
    fn eq(&self, other: &Self) -> bool {
        self.millihz() == other.millihz()
    }
}
impl Eq for RefreshRate {}
impl Hash for RefreshRate {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.millihz().hash(state)
    }
}
impl PartialOrd for RefreshRate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for RefreshRate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.millihz().cmp(&other.millihz())
    }
}

/// How a picture smaller than the screen is shown (the "Scaling" row: Stretch | Black bars | Keep aspect).
/// Windows: `DISPLAYCONFIG_SCALING` on the display path (STRETCHED / CENTERED / ASPECTRATIOCENTEREDMAX).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GpuScaling {
    /// "Fills the whole screen".
    Stretch,
    /// "No fill: real size, black around it".
    BlackBars,
    /// "As big as it fits, same shape".
    KeepAspect,
    /// Windows reports "identity" or "preferred": no scaling chosen, the driver's own default is used.
    /// Read-only state; never offered in the segmented control.
    DriverDefault,
}

impl GpuScaling {
    /// Sort rank for presets: Stretch → Black bars → Keep aspect (DESIGN §3.2.4).
    pub fn rank(self) -> u8 {
        match self {
            GpuScaling::Stretch => 0,
            GpuScaling::BlackBars => 1,
            GpuScaling::KeepAspect => 2,
            GpuScaling::DriverDefault => 3,
        }
    }
    /// The word shown on chips / the segmented control.
    pub fn label(self) -> &'static str {
        match self {
            GpuScaling::Stretch => "Stretch",
            GpuScaling::BlackBars => "Black bars",
            GpuScaling::KeepAspect => "Keep aspect",
            GpuScaling::DriverDefault => "Driver default",
        }
    }
    /// The sub-line under "Scaling" (DESIGN §3.2.2).
    pub fn sub_line(self) -> &'static str {
        match self {
            GpuScaling::Stretch => "Fills the whole screen",
            GpuScaling::BlackBars => "No fill: real size, black around it",
            GpuScaling::KeepAspect => "As big as it fits, same shape",
            GpuScaling::DriverDefault => "The graphics driver decides",
        }
    }
}

/// One mode the monitor reports: width × height × exact refresh rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VideoMode {
    pub width: u32,
    pub height: u32,
    pub refresh: RefreshRate,
}

/// A full display setting: the video mode plus the scaling (what Apply sets and what Revert restores).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    pub refresh: RefreshRate,
    pub scaling: GpuScaling,
}

impl Mode {
    pub fn video(&self) -> VideoMode {
        VideoMode { width: self.width, height: self.height, refresh: self.refresh }
    }

    /// The mode as one line of text (the app's change log keeps it to put the mode back later, Order 036).
    pub fn to_raw(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// [`Mode::to_raw`] read back; None = not a mode.
    pub fn from_raw(raw: &str) -> Option<Mode> {
        serde_json::from_str(raw).ok()
    }
}

/// A rectangle on the Windows desktop (pixels; the main monitor's top-left is 0,0).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// The Windows scaling % (Settings → Display → Scale) of one monitor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DpiScale {
    /// The applied scaling, e.g. 125.
    pub current_percent: u32,
    /// What Windows recommends for this monitor, e.g. 100.
    pub recommended_percent: u32,
    /// Every value Windows offers for this monitor, smallest first (e.g. [100, 125, 150, 175]).
    pub allowed_percent: Vec<u32>,
}

/// HDR state — READ ONLY (an HDR toggle was rejected by the owner, Oct 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HdrInfo {
    pub supported: bool,
    pub enabled: bool,
}

/// Everything the selector, its tooltip and Identify need about one monitor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MonitorInfo {
    pub id: MonitorId,
    /// The Windows monitor number used for the selector and Identify (from `\\.\DISPLAYn`).
    pub number: u32,
    /// The GDI name, e.g. `\\.\DISPLAY1`.
    pub gdi_name: String,
    /// The monitor's own name from its EDID, e.g. "DELL S2721DGF" (may be empty for some built-in panels).
    pub name: String,
    /// Diagonal size in inches from the EDID, if the monitor reports it.
    pub diagonal_inches: Option<f32>,
    /// True for the main display.
    pub is_main: bool,
    /// Where the monitor sits on the desktop (Identify centres its number in this rectangle).
    pub rect: Rect,
    /// The monitor's native (preferred) resolution, if Windows knows it.
    pub native: Option<(u32, u32)>,
    /// The applied mode.
    pub current: Mode,
    /// Windows scaling %, if it could be read.
    pub dpi: Option<DpiScale>,
    /// HDR, read only.
    pub hdr: Option<HdrInfo>,
}

/// Which graphics vendor drives a monitor (decides how vibrance is done).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
    Other,
}

/// A monitor setting reached over DDC/CI (MCCS VCP codes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Vcp {
    /// VCP 0x10.
    Brightness,
    /// VCP 0x12.
    Contrast,
}

impl Vcp {
    pub fn code(self) -> u8 {
        match self {
            Vcp::Brightness => 0x10,
            Vcp::Contrast => 0x12,
        }
    }
}

/// A DDC/CI value as the monitor reports it (`current` of `max`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VcpValue {
    pub current: u32,
    pub max: u32,
}

impl VcpValue {
    /// As 0–100 % for the slider.
    pub fn percent(&self) -> u8 {
        if self.max == 0 {
            return 0;
        }
        ((self.current.min(self.max) as f64 * 100.0 / self.max as f64).round()) as u8
    }
}

/// Raw driver vibrance / saturation levels (NVIDIA DVC levels or AMD ADL saturation).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VibranceRaw {
    pub vendor: GpuVendor,
    pub current: i32,
    pub min: i32,
    pub max: i32,
    /// The driver's normal level (shown as 50 %).
    pub default: i32,
}

/// DDC/CI state of one monitor for the Picture group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DdcState {
    /// Brightness/contrast answered.
    Answers,
    /// The monitor did not answer (DDC/CI off in its menu, or a dock / TV / DisplayLink).
    NoAnswer,
    /// DDC was switched off for this monitor by the crash guard (a DDC call was in progress when the app last died).
    BlockedAfterCrash,
    /// This monitor model is on the DDC/CI exclusion list (`picture::DDC_EXCLUDED_EDID_IDS`): DDC is never used on it.
    ExcludedModel,
}

/// The Picture group of one monitor, read live.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PictureState {
    pub ddc: DdcState,
    pub brightness: Option<VcpValue>,
    pub contrast: Option<VcpValue>,
    /// Vibrance in % (50 = normal), `None` when the GPU offers none (Intel / other / unsupported driver).
    pub vibrance_percent: Option<u8>,
    pub vibrance_vendor: Option<GpuVendor>,
}

/// A change kind, for `needs_admin`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChangeKind {
    Mode,
    MainDisplay,
    DpiScale,
    Ddc,
    Vibrance,
}

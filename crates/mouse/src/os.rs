//! The OS layer behind a trait: the real Windows implementation is `crate::win::RealOs`, the fake is `crate::fake::FakeOs`.
//! All logic sits in the services (`settings`, `cursors`, `device`, `accel`); implementations only read and write.

use crate::error::Result;
use std::path::Path;

/// Which registry root a value lives under.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Hive {
    /// HKEY_CURRENT_USER
    Hkcu,
    /// HKEY_LOCAL_MACHINE (this crate only READS it: the system cursor schemes)
    Hklm,
}

/// A registry value as read from Windows. `Other` keeps any other type byte-exact so undo can put it back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegValue {
    Dword(u32),
    Sz(String),
    /// REG_EXPAND_SZ — cursor paths like `%SystemRoot%\cursors\aero_arrow.cur`
    ExpandSz(String),
    Other { kind: u32, bytes: Vec<u8> },
}

impl RegValue {
    /// The text of an SZ / EXPAND_SZ value.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            RegValue::Sz(s) | RegValue::ExpandSz(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_dword(&self) -> Option<u32> {
        match self {
            RegValue::Dword(d) => Some(*d),
            _ => None,
        }
    }
}

/// The Windows mouse settings (Control Panel › Mouse). Each is read and written through `SystemParametersInfo`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WinSetting {
    /// SPI_GET/SETMOUSESPEED, 1–20
    PointerSpeed,
    /// SPI_GET/SETMOUSE: the three ints {threshold1, threshold2, acceleration}; EPP on = {6,10,1}, off = {0,0,0}
    Precision,
    /// SPI_GET/SETWHEELSCROLLLINES (0xFFFFFFFF = "one screen at a time")
    ScrollLines,
    /// GetDoubleClickTime / SPI_SETDOUBLECLICKTIME, milliseconds
    DoubleClick,
    /// GetSystemMetrics(SM_SWAPBUTTON) / SPI_SETMOUSEBUTTONSWAP
    SwapButtons,
}

/// The raw value of a `WinSetting`, exactly as Windows holds it (undo puts back exactly this).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WinRaw {
    Num(u32),
    Mouse([i32; 3]),
    Bool(bool),
}

impl WinRaw {
    /// Its text form for the app's change log (`"10"`, `"6,10,1"`, `"on"` / `"off"`): [`WinRaw::from_text`] reads it back.
    pub fn to_text(&self) -> String {
        match self {
            WinRaw::Num(n) => n.to_string(),
            WinRaw::Mouse([a, b, c]) => format!("{a},{b},{c}"),
            WinRaw::Bool(b) => if *b { "on" } else { "off" }.into(),
        }
    }

    /// The value of `s` from its text form (None = not a value of that setting).
    pub fn from_text(s: WinSetting, t: &str) -> Option<WinRaw> {
        match s {
            WinSetting::Precision => {
                let v: Vec<i32> = t.split(',').map(|x| x.trim().parse().ok()).collect::<Option<_>>()?;
                <[i32; 3]>::try_from(v).ok().map(WinRaw::Mouse)
            }
            WinSetting::SwapButtons => match t {
                "on" => Some(WinRaw::Bool(true)),
                "off" => Some(WinRaw::Bool(false)),
                _ => None,
            },
            _ => t.trim().parse().ok().map(WinRaw::Num),
        }
    }
}

/// One HID interface (top-level collection) of a connected device, read without sending anything to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HidInfo {
    /// the interface path (`\\?\hid#vid_3710&pid_5406&mi_00#…`) — the handle to talk to it
    pub path: String,
    pub vid: u16,
    pub pid: u16,
    /// HIDD_ATTRIBUTES.VersionNumber
    pub version: u16,
    /// HIDP_CAPS: usage page 0x01 + usage 0x02 = a mouse; 0xFFxx = a vendor collection (where mouse software talks)
    pub usage_page: u16,
    pub usage: u16,
    pub input_len: u16,
    pub output_len: u16,
    pub feature_len: u16,
    /// USB interface number from the path (`mi_02` → 2), when the device has several
    pub interface: Option<u8>,
    pub product: Option<String>,
    pub manufacturer: Option<String>,
}

/// How a request goes to the mouse and how its answer comes back (vendor protocols differ).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HidTransfer {
    /// HidD_SetFeature(out) then HidD_GetFeature(report id = `reply_id`, `reply_len` bytes incl. the id)
    Feature { reply_id: u8, reply_len: usize },
    /// WriteFile(out) on the interface, then ReadFile input reports until one passes the protocol's check
    /// (at most `max_reads` reports, `timeout_ms` in total). The service checks the answer; the OS layer only moves bytes.
    /// `echo`: (byte index, value) that must also match in the answer (e.g. the echoed command) — other reports are skipped.
    OutputThenInput { reply_len: usize, max_reads: u32, timeout_ms: u32, echo: Option<(usize, u8)> },
}

/// Raw Accel driver version (GET_VERSION ioctl).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DriverVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl std::fmt::Display for DriverVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Everything the Mouse tab needs from Windows.
pub trait MouseOs {
    // ---- Windows mouse settings ----
    fn win_get(&self, s: WinSetting) -> Result<WinRaw>;
    /// Writes and persists (SPIF_UPDATEINIFILE | SPIF_SENDCHANGE).
    fn win_set(&mut self, s: WinSetting, v: WinRaw) -> Result<()>;

    // ---- registry (cursors) ----
    fn reg_read(&self, hive: Hive, path: &str, name: &str) -> Result<Option<RegValue>>;
    /// HKCU only — the crate never writes HKLM.
    fn reg_write(&mut self, path: &str, name: &str, value: &RegValue) -> Result<()>;
    /// All values of a key (name, value). A missing key gives an empty list.
    fn reg_values(&self, hive: Hive, path: &str) -> Result<Vec<(String, RegValue)>>;
    /// SPI_SETCURSORS: Windows reloads every cursor from `HKCU\Control Panel\Cursors`.
    fn reload_cursors(&mut self) -> Result<()>;
    /// SetSystemCursor(LoadCursorFromFile(file), id) — the Win11 stuck-scheme re-push, and hover previews.
    fn set_system_cursor(&mut self, file: &str, ocr_id: u32) -> Result<()>;
    /// `%SystemRoot%` etc. expanded.
    fn expand_env(&self, s: &str) -> String;

    // ---- the connected mouse (HID) ----
    /// Every HID interface present. Opening an interface for its attributes sends nothing to the device.
    fn hid_devices(&self) -> Result<Vec<HidInfo>>;
    /// Sends one vendor request and returns the answer's bytes. NEVER used on the real mouse in tests.
    fn hid_exchange(&mut self, path: &str, out: &[u8], how: &HidTransfer) -> Result<Vec<u8>>;

    // ---- Raw Accel ----
    /// The driver's version via `\\.\rawaccel` GET_VERSION, `None` when the driver is not installed / not running.
    fn rawaccel_driver_version(&self) -> Result<Option<DriverVersion>>;
    /// Reads a text file (Raw Accel's settings.json) — read only.
    fn read_text(&self, path: &Path) -> Result<Option<String>>;
    /// Reads one of the app's own files (`None` = there is none).
    fn read_bytes(&self, path: &Path) -> Result<Option<Vec<u8>>>;
    /// Writes one of the app's own files (only under `AppDirs::data`: the copy of what Raw Accel ran before the app
    /// changed it), its folder made first.
    fn write_bytes(&mut self, path: &Path, bytes: &[u8]) -> Result<()>;
    /// What the driver runs now: the READ ioctl's bytes (io_base + profiles + devices), `None` when no driver. A read.
    fn rawaccel_read(&self) -> Result<Option<Vec<u8>>>;
    /// Hands settings to the driver: the WRITE ioctl with `crate::accel::bytes::to_bytes` (the driver waits ~1 s per write —
    /// its anti-abuse delay — so this call blocks ~1 s). Never used on the real driver in tests.
    fn rawaccel_write(&mut self, bytes: &[u8]) -> Result<()>;
    /// Fallback for a driver version whose byte layout this crate does not know: the JSON goes into the app's own file
    /// `settings_file` and Raw Accel's `writer.exe <settings_file>` (from `rawaccel_dir`) is run. Raw Accel's own
    /// settings.json is never written.
    fn rawaccel_writer(&mut self, rawaccel_dir: &Path, settings_file: &Path, json: &str) -> Result<()>;

    // ---- process ----
    fn is_elevated(&self) -> bool;
    /// The pause between two "is the mouse online?" polls (a waking mouse links within ~1-3 s). The fake returns at once.
    fn pause_ms(&self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

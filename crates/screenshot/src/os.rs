//! The OS layer: everything the engine needs from Windows, behind one trait. [`crate::real::RealOs`] is Windows,
//! [`crate::fake::FakeOs`] is an in-memory stand-in every test runs against.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::Result;
use crate::geom::Monitor;
use crate::image::Image;
use crate::naming::LocalTime;

/// How the desktop is grabbed. Both give exact pixels, no sound, no border, no mouse cursor in the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// DXGI Desktop Duplication (IDXGIOutputDuplication). No border ever exists with this API.
    DesktopDuplication,
    /// Windows.Graphics.Capture (the API Snipping Tool / OBS use). Its yellow border is switched off with
    /// `GraphicsCaptureSession.IsBorderRequired = false` (Windows 11; Windows 10 always draws it).
    GraphicsCapture,
}

impl Method {
    pub fn name(self) -> &'static str {
        match self {
            Method::DesktopDuplication => "Desktop Duplication",
            Method::GraphicsCapture => "Windows.Graphics.Capture",
        }
    }
}

/// How a monitor's pixels were turned into 8-bit sRGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorPath {
    /// SDR monitor: the 8-bit desktop surface as is.
    Sdr,
    /// HDR monitor: captured as 16-bit float scRGB and converted by [`crate::image::HdrToSdr`] with the monitor's SDR white.
    HdrConverted,
    /// HDR monitor, but the 16-bit path was refused (e.g. the process is not per-monitor DPI aware for DuplicateOutput1);
    /// Windows' own 8-bit conversion was used instead. Reported so the menu can tell.
    HdrByWindows,
}

/// One monitor's frozen picture.
#[derive(Debug, Clone)]
pub struct MonitorFrame {
    /// The monitor's number ([`Monitor::number`]).
    pub monitor: usize,
    /// Exactly the monitor's resolution, turned the way the user sees it.
    pub image: Image,
    pub color: ColorPath,
}

/// Where the time of one capture went (microseconds, measured with `Instant`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CaptureTiming {
    /// Creating the GPU device(s) and the capture objects.
    pub setup_us: u64,
    /// From setup done until every monitor's frame was in hand (still on the GPU).
    pub frames_us: u64,
    /// Between the first and the last monitor's frame arriving — how "one instant" the All picture is.
    pub spread_us: u64,
    /// Copying the frames from the GPU into memory (incl. HDR conversion / rotation).
    pub readback_us: u64,
    /// The whole call.
    pub total_us: u64,
}

/// What one capture call returns.
#[derive(Debug, Clone)]
pub struct Capture {
    pub frames: Vec<MonitorFrame>,
    pub timing: CaptureTiming,
}

/// A running Live source: the picture keeps moving until the user snaps.
pub trait LiveFeed: Send {
    /// Blocks until a monitor shows a new frame or `timeout` passes (no timer runs inside; it waits on Windows).
    /// `Ok(true)` = something new arrived.
    fn wait(&mut self, timeout: Duration) -> Result<bool>;
    /// The newest picture of each monitor (the snap).
    fn latest(&mut self) -> Result<Vec<MonitorFrame>>;
}

/// Everything the engine asks Windows for.
pub trait ScreenshotOs {
    /// Every monitor attached to the desktop (numbered, physical pixels).
    fn monitors(&self) -> Result<Vec<Monitor>>;
    /// Grabs these monitors as close to one instant as the method allows.
    fn capture(&self, method: Method, monitors: &[Monitor]) -> Result<Capture>;
    /// Starts a Live source for these monitors. Dropping it stops it.
    fn live(&self, method: Method, monitors: &[Monitor]) -> Result<Box<dyn LiveFeed>>;

    fn local_time(&self) -> LocalTime;
    /// The wall-clock time of a moment (UTC ms) - the gallery's captions. Default: UTC (the fake); the real layer uses the
    /// PC's time zone with the daylight-saving rule of that date.
    fn local_time_of(&self, unix_ms: u64) -> LocalTime {
        crate::naming::civil(unix_ms, 0)
    }
    /// Milliseconds since 1970 (UTC).
    fn unix_ms(&self) -> u64;
    /// Windows' Screenshots folder (Known Folder "Screenshots", normally Pictures\Screenshots). Read only: never created here.
    fn default_save_dir(&self) -> Result<PathBuf>;

    /// `Ok(None)` if the file does not exist.
    fn read_file(&self, path: &Path) -> Result<Option<Vec<u8>>>;
    /// Writes the whole file (parent folders created; written to a temp name, then renamed, so a crash never leaves half a file).
    fn write_file(&self, path: &Path, bytes: &[u8]) -> Result<()>;
    /// Permanently removes one of the engine's OWN files (index temp, thumbnails). Missing = fine. Never used on screenshots.
    fn remove_file(&self, path: &Path) -> Result<()>;
    fn exists(&self, path: &Path) -> bool;
    fn is_dir(&self, path: &Path) -> bool;

    /// Puts the picture on the clipboard as PNG (registered format "PNG") + CF_DIB.
    fn set_clipboard(&self, png: &[u8], dib: &[u8]) -> Result<()>;
    /// Puts files on the clipboard the way Explorer's Copy does (CF_HDROP + "Preferred DropEffect" = copy).
    fn set_clipboard_files(&self, paths: &[PathBuf]) -> Result<()>;
    /// Moves files to the Recycle Bin, silently. Refuses ([`crate::Error::NoRecycleBin`]) instead of deleting for good.
    fn recycle(&self, paths: &[PathBuf]) -> Result<()>;
    /// Opens Explorer on each file's folder with the files selected.
    fn show_in_folder(&self, paths: &[PathBuf]) -> Result<()>;
    /// Opens a folder in Explorer ("Change path" › Open).
    fn open_folder(&self, dir: &Path) -> Result<()>;
    /// Windows' own folder picker ("Change path" › Change). [`crate::Error::Cancelled`] if closed without a pick.
    fn pick_folder(&self, start: Option<&Path>) -> Result<PathBuf>;
}

//! The real Windows OS layer.
//!
//! - [`monitors`]: DXGI outputs → [`Monitor`]s in physical pixels (DPI-correct), HDR state, SDR white level.
//! - [`gpu`]: D3D11 devices and the GPU → memory copy (incl. HDR conversion and rotation).
//! - [`dxgi`]: DXGI Desktop Duplication capture + Live.
//! - [`wgc`]: Windows.Graphics.Capture capture + Live (border and cursor off).
//! - [`shell`]: clipboard, Recycle Bin, Explorer, folder picker, the drag-out data object.
//!
//! Nothing here plays a sound: neither capture API makes one, and the engine never calls the shell's own screenshot path
//! (Win+PrtSc), which is where Windows' camera-shutter sound and screen dim come from.

pub mod dxgi;
pub mod gpu;
pub mod monitors;
pub mod shell;
pub mod wgc;

use std::path::{Path, PathBuf};
use std::time::Duration;

use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Shell::{FOLDERID_Pictures, FOLDERID_Screenshots, SHGetKnownFolderPath, KNOWN_FOLDER_FLAG};

use crate::error::{Error, Result};
use crate::geom::Monitor;
use crate::naming::LocalTime;
use crate::os::{Capture, LiveFeed, Method, ScreenshotOs};

pub use shell::drag_data_object;

/// Adds the name of the Windows call to an error.
pub(crate) trait Ctx<T> {
    fn ctx(self, op: &str) -> Result<T>;
}

impl<T> Ctx<T> for windows_core::Result<T> {
    fn ctx(self, op: &str) -> Result<T> {
        self.map_err(|e| Error::os(op, e.code().0))
    }
}

/// COM for the current thread, balanced: uninitialised on drop only if this guard's call initialised it.
pub(crate) struct ComGuard {
    owned: bool,
}

impl ComGuard {
    /// Single-threaded apartment (what the shell, the clipboard and drag-and-drop need). A thread already in the
    /// multi-threaded apartment is left as it is (`RPC_E_CHANGED_MODE`) and works for the calls used here.
    pub(crate) fn sta() -> Result<Self> {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        if hr == RPC_E_CHANGED_MODE {
            return Ok(ComGuard { owned: false });
        }
        hr.ok().ctx("CoInitializeEx")?;
        Ok(ComGuard { owned: true }) // S_OK and S_FALSE both need a matching CoUninitialize
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.owned {
            unsafe { CoUninitialize() };
        }
    }
}

/// What a [`RealOs`] may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Everything (the app; the scratch tests point all paths into the scratch folder).
    Full,
    /// No writes, no clipboard / Explorer / Recycle Bin, and NO screen capture (examples/show).
    ReadOnly,
    /// Screen capture allowed, file writes allowed (the proof run writes only into its scratch folder), but no clipboard /
    /// Explorer / Recycle Bin / folder picker.
    CaptureProof,
}

/// The real Windows layer.
#[derive(Debug, Clone)]
pub struct RealOs {
    mode: Mode,
}

impl RealOs {
    pub fn new() -> Self {
        RealOs { mode: Mode::Full }
    }

    /// Refuses every change AND every screen capture (the screen may show private things). For examples/show.
    pub fn read_only() -> Self {
        RealOs { mode: Mode::ReadOnly }
    }

    /// For the one-time timing proof: may capture and write files, refuses clipboard / Explorer / Recycle Bin / picker.
    pub fn capture_proof() -> Self {
        RealOs { mode: Mode::CaptureProof }
    }

    fn allow_write(&self, what: &'static str) -> Result<()> {
        if self.mode == Mode::ReadOnly {
            return Err(Error::ReadOnly(what));
        }
        Ok(())
    }

    fn allow_shell(&self, what: &'static str) -> Result<()> {
        if self.mode != Mode::Full {
            return Err(Error::ReadOnly(what));
        }
        Ok(())
    }

    fn allow_capture(&self) -> Result<()> {
        if self.mode == Mode::ReadOnly {
            return Err(Error::ReadOnly("screen capture"));
        }
        Ok(())
    }
}

impl Default for RealOs {
    fn default() -> Self {
        Self::new()
    }
}

/// `%LOCALAPPDATA%\BoylerUtilities\screenshots` — where the app keeps the engine's settings, gallery index and thumbnails.
/// (Local, not roaming: the index holds this PC's file paths.)
pub fn default_data_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("BoylerUtilities").join("screenshots"))
}

fn known_folder(id: &windows_core::GUID) -> Result<PathBuf> {
    // Flag 0 = do not create it (reading must not change the PC); the save creates it on the first shot.
    let p = unsafe { SHGetKnownFolderPath(id, KNOWN_FOLDER_FLAG(0), None) }.ctx("SHGetKnownFolderPath")?;
    let s = unsafe { p.to_string() };
    unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
    Ok(PathBuf::from(s.map_err(|_| Error::BadData("known folder path".into()))?))
}

impl ScreenshotOs for RealOs {
    fn monitors(&self) -> Result<Vec<Monitor>> {
        Ok(monitors::outputs()?.into_iter().map(|o| o.monitor).collect())
    }

    fn capture(&self, method: Method, wanted: &[Monitor]) -> Result<Capture> {
        self.allow_capture()?;
        match method {
            Method::DesktopDuplication => dxgi::capture(wanted),
            Method::GraphicsCapture => wgc::capture(wanted),
        }
    }

    fn live(&self, method: Method, wanted: &[Monitor]) -> Result<Box<dyn LiveFeed>> {
        self.allow_capture()?;
        match method {
            Method::DesktopDuplication => Ok(Box::new(dxgi::DdLive::start(wanted)?)),
            Method::GraphicsCapture => Ok(Box::new(wgc::WgcLive::start(wanted)?)),
        }
    }

    fn local_time(&self) -> LocalTime {
        let t = unsafe { GetLocalTime() };
        LocalTime {
            year: t.wYear,
            month: t.wMonth as u8,
            day: t.wDay as u8,
            hour: t.wHour as u8,
            minute: t.wMinute as u8,
            second: t.wSecond as u8,
        }
    }

    fn local_time_of(&self, unix_ms: u64) -> LocalTime {
        // UTC -> the PC's time zone with the daylight-saving rule of THAT date (a summer shot seen in winter keeps its time)
        let u = crate::naming::civil(unix_ms, 0);
        let utc = windows::Win32::Foundation::SYSTEMTIME {
            wYear: u.year,
            wMonth: u.month as u16,
            wDayOfWeek: 0,
            wDay: u.day as u16,
            wHour: u.hour as u16,
            wMinute: u.minute as u16,
            wSecond: u.second as u16,
            wMilliseconds: 0,
        };
        let mut t = windows::Win32::Foundation::SYSTEMTIME::default();
        match unsafe { windows::Win32::System::Time::SystemTimeToTzSpecificLocalTime(None, &utc, &mut t) } {
            Ok(()) => LocalTime { year: t.wYear, month: t.wMonth as u8, day: t.wDay as u8, hour: t.wHour as u8, minute: t.wMinute as u8, second: t.wSecond as u8 },
            Err(_) => u,
        }
    }

    fn unix_ms(&self) -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
    }

    fn default_save_dir(&self) -> Result<PathBuf> {
        known_folder(&FOLDERID_Screenshots).or_else(|_| known_folder(&FOLDERID_Pictures).map(|p| p.join("Screenshots")))
    }

    fn read_file(&self, path: &Path) -> Result<Option<Vec<u8>>> {
        match std::fs::read(path) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::io(path.display(), &e)),
        }
    }

    fn write_file(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.allow_write("write file")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir.display(), &e))?;
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".bu-tmp");
        let tmp = PathBuf::from(tmp);
        std::fs::write(&tmp, bytes).map_err(|e| Error::io(tmp.display(), &e))?;
        std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            Error::io(path.display(), &e)
        })
    }

    fn remove_file(&self, path: &Path) -> Result<()> {
        self.allow_write("remove file")?;
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::io(path.display(), &e)),
        }
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn set_clipboard(&self, png: &[u8], dib: &[u8]) -> Result<()> {
        self.allow_shell("clipboard")?;
        shell::set_clipboard_picture(png, dib)
    }

    fn set_clipboard_files(&self, paths: &[PathBuf]) -> Result<()> {
        self.allow_shell("clipboard")?;
        shell::set_clipboard_files(paths)
    }

    fn recycle(&self, paths: &[PathBuf]) -> Result<()> {
        self.allow_shell("Recycle Bin")?;
        shell::recycle(paths)
    }

    fn show_in_folder(&self, paths: &[PathBuf]) -> Result<()> {
        self.allow_shell("Explorer")?;
        shell::show_in_folder(paths)
    }

    fn open_folder(&self, dir: &Path) -> Result<()> {
        self.allow_shell("Explorer")?;
        shell::open_folder(dir)
    }

    fn pick_folder(&self, start: Option<&Path>) -> Result<PathBuf> {
        self.allow_shell("folder picker")?;
        shell::pick_folder(start)
    }
}

/// Microseconds since `t`.
pub(crate) fn us(t: std::time::Instant) -> u64 {
    t.elapsed().as_micros() as u64
}

/// How long a capture waits for each monitor's first frame.
pub(crate) const FRAME_TIMEOUT: Duration = Duration::from_millis(1000);

#[cfg(test)]
mod tests {
    //! The guards themselves, with no Windows call behind them (TECH_RULES: refusals are never proven with calls a broken guard
    //! would turn into a real change — clipboard, Explorer, picker, screen capture).
    use super::*;

    #[test]
    fn read_only_refuses_writes_shell_and_capture() {
        let os = RealOs::read_only();
        assert!(matches!(os.allow_write("w"), Err(Error::ReadOnly(_))));
        assert!(matches!(os.allow_shell("s"), Err(Error::ReadOnly(_))));
        assert!(matches!(os.allow_capture(), Err(Error::ReadOnly(_))));
    }

    #[test]
    fn capture_proof_allows_capture_and_files_but_no_shell() {
        let os = RealOs::capture_proof();
        assert!(os.allow_write("w").is_ok());
        assert!(os.allow_capture().is_ok());
        assert!(matches!(os.allow_shell("s"), Err(Error::ReadOnly(_))));
    }

    #[test]
    fn full_allows_everything() {
        let os = RealOs::new();
        assert!(os.allow_write("w").is_ok() && os.allow_shell("s").is_ok() && os.allow_capture().is_ok());
    }

    #[test]
    fn every_shell_and_capture_entry_point_checks_its_guard() {
        // Each trait method that reaches the clipboard / Explorer / Recycle Bin / picker / capture must start with its guard.
        let src = include_str!("mod.rs");
        for (f, guard) in [
            ("fn capture(", "self.allow_capture()?"),
            ("fn live(", "self.allow_capture()?"),
            ("fn write_file(", "self.allow_write("),
            ("fn remove_file(", "self.allow_write("),
            ("fn set_clipboard(", "self.allow_shell("),
            ("fn set_clipboard_files(", "self.allow_shell("),
            ("fn recycle(", "self.allow_shell("),
            ("fn show_in_folder(", "self.allow_shell("),
            ("fn open_folder(", "self.allow_shell("),
            ("fn pick_folder(", "self.allow_shell("),
        ] {
            let at = src.find(&format!("    {f}")).unwrap_or_else(|| panic!("{f} not found"));
            let body: String = src[at..].lines().take(3).collect();
            assert!(body.contains(guard), "{f} must begin with {guard}");
        }
    }
}
//! The REAL Windows implementation of `MouseOs` (the `windows` crate).
//!
//! Three modes:
//! - [`RealOs::new`] — everything real (the menu).
//! - [`RealOs::read_only`] — reads are real, EVERY change is refused (`examples/show`, proofs). Nothing is ever sent to the
//!   mouse in this mode (not even a "get" request) and Raw Accel's driver is only asked for its version.
//! - [`RealOs::scratch`] — HKCU registry reads/writes go under a scratch key (`HKCU\<base>\…`); every other change
//!   (SystemParametersInfo, SetSystemCursor, the mouse, Raw Accel) is refused or only recorded. Used by the scratch tests.

pub mod hid;
pub mod rawaccel_io;
pub mod watch;
pub mod wmi;

use windows::core::{HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::error::{Error, Result};
use crate::os::*;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Mode {
    Full,
    ReadOnly,
    /// read-only + READ requests to the mouse through `crate::pulsar::read_request_allowed` (A_009_01)
    ReadRequests,
    Scratch(String),
}

/// The real OS layer.
pub struct RealOs {
    mode: Mode,
    /// scratch mode: the Windows calls that were NOT made, in order
    recorded: Vec<String>,
}

pub(crate) fn win32(op: &str, e: WIN32_ERROR) -> Result<()> {
    match e {
        ERROR_SUCCESS => Ok(()),
        ERROR_ACCESS_DENIED => Err(Error::NeedsAdmin { what: op.into() }),
        other => Err(Error::os(op, other.0 as i64)),
    }
}

pub(crate) fn hr(op: &str, e: windows::core::Error) -> Error {
    if e.code() == ERROR_ACCESS_DENIED.to_hresult() {
        Error::NeedsAdmin { what: op.into() }
    } else {
        Error::os(op, e.code().0 as i64)
    }
}

pub(crate) fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|c| *c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

fn utf16_bytes(s: &str) -> Vec<u8> {
    s.encode_utf16().chain(std::iter::once(0)).flat_map(|c| c.to_le_bytes()).collect()
}

impl Default for RealOs {
    fn default() -> Self {
        Self::new()
    }
}

impl RealOs {
    /// Everything real — the menu uses this.
    pub fn new() -> Self {
        Self { mode: Mode::Full, recorded: Vec::new() }
    }

    /// Real reads; every change refused with `Error::ReadOnly`.
    pub fn read_only() -> Self {
        Self { mode: Mode::ReadOnly, recorded: Vec::new() }
    }

    /// Like `read_only`, but READ requests may go to the mouse: every frame is checked against the allow-list
    /// (`crate::pulsar::read_request_allowed`) BEFORE any byte leaves; writes, factory reset and profile switches are
    /// refused. Every request and answer is kept in `recorded` (A_009_01: logged byte for byte in the report).
    pub fn read_requests_only() -> Self {
        Self { mode: Mode::ReadRequests, recorded: Vec::new() }
    }

    /// HKCU registry under `HKCU\<base>` (must start with `Software\BoylerUtilities-test\`); everything else refused /
    /// recorded. Reads of HKLM stay real (read-only).
    pub fn scratch(base: &str) -> Result<Self> {
        if !base.starts_with(r"Software\BoylerUtilities-test\") || base.trim_end_matches('\\') == r"Software\BoylerUtilities-test" {
            return Err(Error::ReadOnly(format!("scratch base must be under Software\\BoylerUtilities-test\\<lane>, not {base}")));
        }
        Ok(Self { mode: Mode::Scratch(base.trim_end_matches('\\').to_string()), recorded: Vec::new() })
    }

    /// Scratch mode: the calls that were recorded instead of made.
    pub fn recorded(&self) -> &[String] {
        &self.recorded
    }

    fn refuse(&self, what: &str) -> Result<()> {
        match self.mode {
            Mode::Full => Ok(()),
            _ => Err(Error::ReadOnly(what.into())),
        }
    }

    /// Scratch: record instead of doing; read-only: refuse; full: do.
    fn record_or_refuse(&mut self, what: String) -> Result<bool> {
        match &self.mode {
            Mode::Full => Ok(true),
            Mode::ReadOnly | Mode::ReadRequests => Err(Error::ReadOnly(what)),
            Mode::Scratch(_) => {
                self.recorded.push(what);
                Ok(false)
            }
        }
    }

    fn map(&self, hive: Hive, path: &str) -> (HKEY, String) {
        match (&self.mode, hive) {
            (Mode::Scratch(base), Hive::Hkcu) => (HKEY_CURRENT_USER, format!(r"{base}\{path}")),
            (_, Hive::Hkcu) => (HKEY_CURRENT_USER, path.to_string()),
            (_, Hive::Hklm) => (HKEY_LOCAL_MACHINE, path.to_string()),
        }
    }

    fn open(&self, hive: Hive, path: &str) -> Result<Option<HKEY>> {
        let (root, p) = self.map(hive, path);
        let mut hk = HKEY::default();
        let e = unsafe { RegOpenKeyExW(root, &HSTRING::from(p.as_str()), None, KEY_READ | KEY_WOW64_64KEY, &mut hk) };
        match e {
            ERROR_SUCCESS => Ok(Some(hk)),
            ERROR_FILE_NOT_FOUND => Ok(None),
            other => win32(&format!("RegOpenKeyEx {p}"), other).map(|_| None),
        }
    }

    fn parse_value(kind: REG_VALUE_TYPE, buf: Vec<u8>) -> RegValue {
        let text = |b: &[u8]| {
            let w: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
            wide_to_string(&w)
        };
        match kind {
            REG_DWORD if buf.len() >= 4 => RegValue::Dword(u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]])),
            REG_SZ => RegValue::Sz(text(&buf)),
            REG_EXPAND_SZ => RegValue::ExpandSz(text(&buf)),
            other => RegValue::Other { kind: other.0, bytes: buf },
        }
    }

    fn spi_u32(action: SYSTEM_PARAMETERS_INFO_ACTION, op: &str) -> Result<u32> {
        let mut v = 0u32;
        unsafe { SystemParametersInfoW(action, 0, Some(&mut v as *mut u32 as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) }
            .map_err(|e| hr(op, e))?;
        Ok(v)
    }
}


/// Order 050: no SPIF_SENDCHANGE - that broadcast waits for every window on the caller's (the UI) thread, once per step of a
/// slider drag. The setting is saved at once; its WM_SETTINGCHANGE goes out on one worker ([`broadcast_later`]).
const PERSIST: SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS = SPIF_UPDATEINIFILE;

/// SPI codes waiting for their WM_SETTINGCHANGE, and whether the worker runs.
static PENDING: std::sync::Mutex<(Vec<u32>, bool)> = std::sync::Mutex::new((Vec::new(), false));

/// Order 050: WM_SETTINGCHANGE (wParam = the SPI_SET* code) to every window, on one worker thread: SMTO_ABORTIFHUNG skips a
/// hung window, 200 ms at most per window; a code asked again while it waits goes out once (a slider drag = one broadcast
/// per setting, not one per step).
fn broadcast_later(action: SYSTEM_PARAMETERS_INFO_ACTION) {
    let Ok(mut p) = PENDING.lock() else { return };
    if !p.0.contains(&action.0) {
        p.0.push(action.0);
    }
    if p.1 {
        return;
    }
    p.1 = true;
    let spawned = std::thread::Builder::new().name("bu-mouse-broadcast".into()).spawn(|| loop {
        let next = match PENDING.lock() {
            Ok(mut p) if p.0.is_empty() => {
                p.1 = false;
                return;
            }
            Ok(mut p) => p.0.remove(0),
            Err(_) => return,
        };
        unsafe {
            SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, WPARAM(next as usize), LPARAM(0), SMTO_ABORTIFHUNG | SMTO_NORMAL, 200, None);
        }
    });
    if spawned.is_err() {
        p.1 = false;
    }
}

/// Order 050: wait (at most `max`) until every WM_SETTINGCHANGE waiting for the worker went out - the app calls it before it
/// ends (Quit, the uninstaller's `--undo-windows`), so the last change is still announced. true = all sent.
pub fn wait_broadcasts(max: std::time::Duration) -> bool {
    let end = std::time::Instant::now() + max;
    while PENDING.lock().map(|p| p.1).unwrap_or(false) {
        if std::time::Instant::now() >= end {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    true
}

impl MouseOs for RealOs {
    fn win_get(&self, s: WinSetting) -> Result<WinRaw> {
        Ok(match s {
            WinSetting::PointerSpeed => WinRaw::Num(Self::spi_u32(SPI_GETMOUSESPEED, "SPI_GETMOUSESPEED")?),
            WinSetting::ScrollLines => WinRaw::Num(Self::spi_u32(SPI_GETWHEELSCROLLLINES, "SPI_GETWHEELSCROLLLINES")?),
            WinSetting::Precision => {
                let mut m = [0i32; 3];
                unsafe { SystemParametersInfoW(SPI_GETMOUSE, 0, Some(m.as_mut_ptr() as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) }
                    .map_err(|e| hr("SPI_GETMOUSE", e))?;
                WinRaw::Mouse(m)
            }
            WinSetting::DoubleClick => WinRaw::Num(unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() }),
            WinSetting::SwapButtons => WinRaw::Bool(unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0),
        })
    }

    fn win_set(&mut self, s: WinSetting, v: WinRaw) -> Result<()> {
        if !self.record_or_refuse(format!("win_set {s:?} {v:?}"))? {
            return Ok(());
        }
        let r = unsafe {
            match (s, v) {
                // SPI_SETMOUSESPEED takes the speed in pvParam itself (an integer, not a pointer).
                (WinSetting::PointerSpeed, WinRaw::Num(n)) => SystemParametersInfoW(SPI_SETMOUSESPEED, 0, Some(n as usize as *mut _), PERSIST),
                (WinSetting::ScrollLines, WinRaw::Num(n)) => SystemParametersInfoW(SPI_SETWHEELSCROLLLINES, n, None, PERSIST),
                (WinSetting::Precision, WinRaw::Mouse(mut m)) => SystemParametersInfoW(SPI_SETMOUSE, 0, Some(m.as_mut_ptr() as *mut _), PERSIST),
                (WinSetting::DoubleClick, WinRaw::Num(n)) => SystemParametersInfoW(SPI_SETDOUBLECLICKTIME, n, None, PERSIST),
                (WinSetting::SwapButtons, WinRaw::Bool(b)) => SystemParametersInfoW(SPI_SETMOUSEBUTTONSWAP, b as u32, None, PERSIST),
                (s, v) => return Err(Error::range(format!("{s:?}"), format!("wrong value kind {v:?}"))),
            }
        };
        r.map_err(|e| hr(&format!("SystemParametersInfo {s:?}"), e))?;
        broadcast_later(match s {
            WinSetting::PointerSpeed => SPI_SETMOUSESPEED,
            WinSetting::ScrollLines => SPI_SETWHEELSCROLLLINES,
            WinSetting::Precision => SPI_SETMOUSE,
            WinSetting::DoubleClick => SPI_SETDOUBLECLICKTIME,
            WinSetting::SwapButtons => SPI_SETMOUSEBUTTONSWAP,
        });
        Ok(())
    }

    fn reg_read(&self, hive: Hive, path: &str, name: &str) -> Result<Option<RegValue>> {
        let Some(hk) = self.open(hive, path)? else { return Ok(None) };
        let hname = HSTRING::from(name);
        let result = (|| unsafe {
            let mut kind = REG_VALUE_TYPE::default();
            let mut size = 0u32;
            match RegQueryValueExW(hk, &hname, None, Some(&mut kind), None, Some(&mut size)) {
                ERROR_SUCCESS => {}
                ERROR_FILE_NOT_FOUND => return Ok(None),
                e => return win32(&format!("RegQueryValueEx {name}"), e).map(|_| None),
            }
            let mut buf = vec![0u8; size as usize];
            let e = RegQueryValueExW(hk, &hname, None, Some(&mut kind), Some(buf.as_mut_ptr()), Some(&mut size));
            win32(&format!("RegQueryValueEx {name}"), e)?;
            buf.truncate(size as usize);
            Ok(Some(Self::parse_value(kind, buf)))
        })();
        unsafe {
            let _ = RegCloseKey(hk);
        }
        result
    }

    fn reg_write(&mut self, path: &str, name: &str, value: &RegValue) -> Result<()> {
        // Scratch mode writes for real, but only under its scratch key (`map`).
        if matches!(self.mode, Mode::ReadOnly | Mode::ReadRequests) {
            return Err(Error::ReadOnly(format!("reg_write {path}\\{name}")));
        }
        let (root, p) = self.map(Hive::Hkcu, path);
        let (kind, bytes) = match value {
            RegValue::Dword(d) => (REG_DWORD, d.to_le_bytes().to_vec()),
            RegValue::Sz(s) => (REG_SZ, utf16_bytes(s)),
            RegValue::ExpandSz(s) => (REG_EXPAND_SZ, utf16_bytes(s)),
            RegValue::Other { kind, bytes } => (REG_VALUE_TYPE(*kind), bytes.clone()),
        };
        unsafe {
            let mut hk = HKEY::default();
            let e = RegCreateKeyExW(root, &HSTRING::from(p.as_str()), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_SET_VALUE | KEY_WOW64_64KEY, None, &mut hk, None);
            win32(&format!("RegCreateKeyEx {p}"), e)?;
            let e = RegSetValueExW(hk, &HSTRING::from(name), None, kind, Some(&bytes));
            let _ = RegCloseKey(hk);
            win32(&format!("RegSetValueEx {name}"), e)
        }
    }

    fn reg_delete_value(&mut self, path: &str, name: &str) -> Result<()> {
        if matches!(self.mode, Mode::ReadOnly | Mode::ReadRequests) {
            return Err(Error::ReadOnly(format!("reg_delete {path}\\{name}")));
        }
        let (root, p) = self.map(Hive::Hkcu, path);
        unsafe {
            let mut hk = HKEY::default();
            let e = RegOpenKeyExW(root, &HSTRING::from(p.as_str()), None, KEY_SET_VALUE | KEY_WOW64_64KEY, &mut hk);
            if e == ERROR_FILE_NOT_FOUND {
                return Ok(());
            }
            win32(&format!("RegOpenKeyEx {p}"), e)?;
            let e = RegDeleteValueW(hk, &HSTRING::from(name));
            let _ = RegCloseKey(hk);
            if e == ERROR_FILE_NOT_FOUND {
                return Ok(());
            }
            win32(&format!("RegDeleteValue {name}"), e)
        }
    }

    fn reg_values(&self, hive: Hive, path: &str) -> Result<Vec<(String, RegValue)>> {
        let Some(hk) = self.open(hive, path)? else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        let mut index = 0u32;
        let result = loop {
            let mut name = vec![0u16; 16384];
            let mut name_len = name.len() as u32;
            let mut kind = 0u32;
            let mut data = vec![0u8; 4096];
            let mut data_len = data.len() as u32;
            let mut e = unsafe {
                RegEnumValueW(hk, index, Some(PWSTR(name.as_mut_ptr())), &mut name_len, None, Some(&mut kind), Some(data.as_mut_ptr()), Some(&mut data_len))
            };
            if e == ERROR_MORE_DATA {
                data = vec![0u8; data_len as usize];
                name_len = name.len() as u32;
                e = unsafe {
                    RegEnumValueW(hk, index, Some(PWSTR(name.as_mut_ptr())), &mut name_len, None, Some(&mut kind), Some(data.as_mut_ptr()), Some(&mut data_len))
                };
            }
            match e {
                ERROR_SUCCESS => {
                    data.truncate(data_len as usize);
                    out.push((String::from_utf16_lossy(&name[..name_len as usize]), Self::parse_value(REG_VALUE_TYPE(kind), data)));
                    index += 1;
                }
                ERROR_NO_MORE_ITEMS => break Ok(out),
                other => break win32("RegEnumValue", other).map(|_| Vec::new()),
            }
        };
        unsafe {
            let _ = RegCloseKey(hk);
        }
        result
    }

    fn reload_cursors(&mut self) -> Result<()> {
        if !self.record_or_refuse("SPI_SETCURSORS".into())? {
            return Ok(());
        }
        // Order 066: NO update flags. SPI_SETCURSORS with SPIF_UPDATEINIFILE returns FALSE (error 0) on Windows 11 - the old
        // call "failed" after every pick and the reload never happened. The cursors live in the registry already; the
        // WM_SETTINGCHANGE goes out on the worker below.
        unsafe { SystemParametersInfoW(SPI_SETCURSORS, 0, None, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) }.map_err(|e| hr("SPI_SETCURSORS", e))?;
        broadcast_later(SPI_SETCURSORS);
        Ok(())
    }

    fn set_system_cursor(&mut self, file: &str, ocr_id: u32) -> Result<()> {
        if !self.record_or_refuse(format!("SetSystemCursor {ocr_id} {file}"))? {
            return Ok(());
        }
        unsafe {
            // Order 066: at the size Windows' slider set (CursorBaseSize) - LoadCursorFromFile always loads 32 px, so the
            // re-push after a reload used to undo the size slider. A multi-size .cur gives its nearest picture, a single-size
            // one is scaled. (A file LoadImage refuses: the plain loader.)
            let px = self.reg_read(Hive::Hkcu, crate::cursors::CURSORS_KEY, "CursorBaseSize")?.and_then(|v| v.as_dword()).unwrap_or(32).clamp(32, 256) as i32;
            let h = match LoadImageW(None, &HSTRING::from(file), IMAGE_CURSOR, px, px, LR_LOADFROMFILE) {
                Ok(h) => HCURSOR(h.0),
                Err(_) => LoadCursorFromFileW(&HSTRING::from(file)).map_err(|e| hr(&format!("LoadCursorFromFile {file}"), e))?,
            };
            // SetSystemCursor takes ownership of the cursor (destroys it later), so no DestroyCursor here.
            SetSystemCursor(h, SYSTEM_CURSOR_ID(ocr_id)).map_err(|e| hr("SetSystemCursor", e))
        }
    }

    fn expand_env(&self, s: &str) -> String {
        use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
        if !s.contains('%') {
            return s.to_string();
        }
        let src = HSTRING::from(s);
        let n = unsafe { ExpandEnvironmentStringsW(&src, None) };
        if n == 0 {
            return s.to_string();
        }
        let mut buf = vec![0u16; n as usize];
        let n2 = unsafe { ExpandEnvironmentStringsW(&src, Some(&mut buf)) };
        if n2 == 0 || n2 > n {
            return s.to_string();
        }
        wide_to_string(&buf)
    }

    fn hid_devices(&self) -> Result<Vec<HidInfo>> {
        hid::list()
    }

    fn hid_exchange(&mut self, path: &str, out: &[u8], how: &HidTransfer) -> Result<Vec<u8>> {
        // Never in read-only or scratch mode — not even a "get" request reaches the mouse. In read-requests mode only
        // allow-listed READ frames go out, and every byte is recorded.
        if self.mode == Mode::ReadRequests {
            if !crate::pulsar::read_request_allowed(out) {
                self.recorded.push(format!("REFUSED (not a read request): {}", crate::fake::hex(out)));
                return Err(Error::ReadOnly(format!("not an allow-listed read request: {}", crate::fake::hex(out))));
            }
            self.recorded.push(format!("sent     {}", crate::fake::hex(out)));
            let r = hid::exchange(path, out, how);
            match &r {
                Ok(a) => self.recorded.push(format!("answered {}", crate::fake::hex(a))),
                Err(e) => self.recorded.push(format!("no answer: {e}")),
            }
            return r;
        }
        self.refuse("sending a request to the mouse")?;
        hid::exchange(path, out, how)
    }

    fn rawaccel_driver_version(&self) -> Result<Option<DriverVersion>> {
        rawaccel_io::driver_version()
    }

    fn read_text(&self, path: &Path) -> Result<Option<String>> {
        match std::fs::read_to_string(path) {
            Ok(t) => Ok(Some(t)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::io(format!("read {}", path.display()), e)),
        }
    }

    fn read_bytes(&self, path: &Path) -> Result<Option<Vec<u8>>> {
        match std::fs::read(path) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::io(format!("read {}", path.display()), e)),
        }
    }

    fn write_bytes(&mut self, path: &Path, bytes: &[u8]) -> Result<()> {
        // never in read-only mode; scratch mode records it
        if !self.record_or_refuse(format!("write {}", path.display()))? {
            return Ok(());
        }
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| Error::io(format!("create {}", d.display()), e))?;
        }
        std::fs::write(path, bytes).map_err(|e| Error::io(format!("write {}", path.display()), e))
    }

    fn rawaccel_read(&self) -> Result<Option<Vec<u8>>> {
        rawaccel_io::read()
    }

    fn rawaccel_write(&mut self, bytes: &[u8]) -> Result<()> {
        // Never in read-only or scratch mode — Raw Accel's driver is never written by a test.
        if !self.record_or_refuse(format!("Raw Accel WRITE {} bytes", bytes.len()))? {
            return Ok(());
        }
        rawaccel_io::write(bytes)
    }

    fn rawaccel_writer(&mut self, rawaccel_dir: &Path, settings_file: &Path, json: &str) -> Result<()> {
        if !self.record_or_refuse(format!("Raw Accel writer.exe {}", settings_file.display()))? {
            return Ok(());
        }
        rawaccel_io::run_writer(rawaccel_dir, settings_file, json)
    }

    fn process_running(&self, exe: &str) -> Result<bool> {
        use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.map_err(|e| hr("process list", e))?;
        let mut e = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut found = false;
        let mut ok = unsafe { Process32FirstW(snap, &mut e) }.is_ok();
        while ok {
            let len = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
            if String::from_utf16_lossy(&e.szExeFile[..len]).eq_ignore_ascii_case(exe) {
                found = true;
                break;
            }
            ok = unsafe { Process32NextW(snap, &mut e) }.is_ok();
        }
        let _ = unsafe { CloseHandle(snap) };
        Ok(found)
    }

    fn is_elevated(&self) -> bool {
        use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        unsafe {
            let mut token = HANDLE::default();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
                return false;
            }
            let mut elev = TOKEN_ELEVATION::default();
            let mut len = 0u32;
            let ok = GetTokenInformation(token, TokenElevation, Some(&mut elev as *mut _ as *mut _), size_of::<TOKEN_ELEVATION>() as u32, &mut len).is_ok();
            let _ = CloseHandle(token);
            ok && elev.TokenIsElevated != 0
        }
    }
}

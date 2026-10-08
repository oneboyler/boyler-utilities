//! The REAL Windows layer.
//!
//! - [`RealSteam`]: Steam's folder from the registry (`HKCU\Software\Valve\Steam\SteamPath`), the logged-in account
//!   (`…\ActiveProcess\ActiveUser`), plain file reads, and writes that are ALWAYS limited to allowed folders (Steam's
//!   `Steam Controller Configs` + the app's backup folder; a scratch folder in tests; nothing at all when read-only).
//!   A write goes to a temp file next to the target and is renamed over it, so Steam never reads half a file.
//!   `steam_running` only looks at the process list — the app never starts, closes or restarts Steam.
//! - [`RealPads`]: PlayStation pads through HID (SetupDi + HidD, the device opened for READING only — never an output or
//!   feature report), Xbox pads through XInput. Live: PlayStation = an overlapped `ReadFile` that wakes on each input
//!   report or on the stop event (no timer); Xbox = XInput has no events, so it is polled every 8 ms ONLY while the live
//!   view runs.

use crate::error::{Error, Result};
use crate::live;
use crate::os::{Battery, BatteryLevel, Connection, Entry, LiveEvent, LiveSource, PadInfo, PadOs, PadSource, SteamOs};
use crate::parts::PadKind;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use windows::core::{PCSTR, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE,
    DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
};
use windows::Win32::Devices::HumanInterfaceDevice::{
    HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetHidGuid, HidD_GetPreparsedData, HidD_GetProductString, HidP_GetCaps, HIDD_ATTRIBUTES, HIDP_CAPS,
    PHIDP_PREPARSED_DATA,
};
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_IO_PENDING, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Storage::FileSystem::{CreateFileW, ReadFile, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Registry::{RegGetValueW, HKEY, HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForMultipleObjects, WaitForSingleObject, INFINITE};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::UI::Input::XboxController::{
    XInputGetBatteryInformation, XInputGetState, BATTERY_DEVTYPE_GAMEPAD, BATTERY_TYPE_DISCONNECTED, BATTERY_TYPE_WIRED, XINPUT_BATTERY_INFORMATION,
    XINPUT_CAPABILITIES, XINPUT_STATE,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn reg_str(root: HKEY, key: &str, value: &str) -> Option<String> {
    let (k, v) = (wide(key), wide(value));
    let mut buf = vec![0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: RegGetValueW writes at most `len` bytes into `buf`.
    let r = unsafe { RegGetValueW(root, PCWSTR(k.as_ptr()), PCWSTR(v.as_ptr()), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut len)) };
    if r.is_err() {
        return None;
    }
    let n = (len as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..n.min(buf.len())]))
}

fn reg_dword(root: HKEY, key: &str, value: &str) -> Option<u32> {
    let (k, v) = (wide(key), wide(value));
    let mut d = 0u32;
    let mut len = 4u32;
    // SAFETY: a DWORD read into a 4-byte local.
    let r = unsafe { RegGetValueW(root, PCWSTR(k.as_ptr()), PCWSTR(v.as_ptr()), RRF_RT_REG_DWORD, None, Some((&mut d as *mut u32).cast()), Some(&mut len)) };
    r.is_ok().then_some(d)
}

/// Lower-case, `\`-separated, no trailing `\`, no `\\?\` — for "is this path inside that folder" checks.
fn norm(p: &Path) -> String {
    p.to_string_lossy().replace('/', "\\").trim_start_matches(r"\\?\").trim_end_matches('\\').to_ascii_lowercase()
}

/// Is `path` inside one of `roots` (no `..` parts allowed)?
pub fn inside(path: &Path, roots: &[PathBuf]) -> bool {
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return false;
    }
    let p = norm(path);
    roots.iter().any(|r| {
        let r = norm(r);
        !r.is_empty() && p.starts_with(&format!("{r}\\"))
    })
}

/// Steam's real files.
#[derive(Debug, Clone)]
pub struct RealSteam {
    /// `None` = read from the registry.
    steam_dir: Option<PathBuf>,
    /// Writes are refused outside these folders; empty = read-only.
    write_roots: Vec<PathBuf>,
    /// Report "no account logged in" (scratch Steam trees have none).
    no_active: bool,
}

impl RealSteam {
    /// The app's mode: writes only inside Steam's `Steam Controller Configs` folder and `backups`.
    pub fn new(backups: impl Into<PathBuf>) -> Self {
        let mut roots = vec![backups.into()];
        if let Some(d) = Self::registry_steam_dir() {
            roots.push(d.join("steamapps").join("common").join("Steam Controller Configs"));
        }
        RealSteam { steam_dir: None, write_roots: roots, no_active: false }
    }

    /// Read-only: every write / create / remove is refused (examples/show, tests on the real PC).
    pub fn read_only() -> Self {
        RealSteam { steam_dir: None, write_roots: Vec::new(), no_active: false }
    }

    /// Tests: a Steam-shaped tree inside a scratch folder. Refused unless `steam_dir` is inside `scratch_root` (which must
    /// exist); writes only inside `scratch_root`.
    pub fn scratch(steam_dir: impl Into<PathBuf>, scratch_root: &Path) -> Result<Self> {
        let steam_dir = steam_dir.into();
        let root = std::fs::canonicalize(scratch_root).map_err(|e| Error::io(format!("scratch root {}", scratch_root.display()), e))?;
        if !inside(&steam_dir, std::slice::from_ref(&root)) {
            return Err(Error::OutsideScratch(steam_dir));
        }
        Ok(RealSteam { steam_dir: Some(steam_dir), write_roots: vec![root], no_active: true })
    }

    fn registry_steam_dir() -> Option<PathBuf> {
        let s = reg_str(HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath")?;
        let s = s.trim();
        (!s.is_empty()).then(|| PathBuf::from(s.replace('/', "\\")))
    }

    fn check_write(&self, path: &Path, what: &str) -> Result<()> {
        if self.write_roots.is_empty() {
            return Err(Error::ReadOnly(format!("{what} {}", path.display())));
        }
        if !inside(path, &self.write_roots) {
            return Err(Error::OutsideScratch(path.to_path_buf()));
        }
        Ok(())
    }
}

impl SteamOs for RealSteam {
    fn steam_dir(&self) -> Option<PathBuf> {
        self.steam_dir.clone().or_else(Self::registry_steam_dir)
    }
    fn active_account(&self) -> Option<u32> {
        if self.no_active {
            return None;
        }
        reg_dword(HKEY_CURRENT_USER, r"Software\Valve\Steam\ActiveProcess", "ActiveUser").filter(|a| *a != 0)
    }
    fn read(&self, path: &Path) -> Result<Vec<u8>> {
        std::fs::read(path).map_err(|e| Error::io(format!("read {}", path.display()), e))
    }
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
    fn list(&self, dir: &Path) -> Result<Vec<Entry>> {
        let rd = std::fs::read_dir(dir).map_err(|e| Error::io(format!("list {}", dir.display()), e))?;
        let mut out = Vec::new();
        for e in rd.flatten() {
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            out.push(Entry { name: e.file_name().to_string_lossy().into_owned(), is_dir });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.check_write(path, "write")?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let tmp = path.with_file_name(format!("{name}.bu-tmp"));
        std::fs::write(&tmp, bytes).map_err(|e| Error::io(format!("write {}", tmp.display()), e))?;
        std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            Error::io(format!("replace {}", path.display()), e)
        })
    }
    fn create_dir_all(&self, dir: &Path) -> Result<()> {
        self.check_write(dir, "create")?;
        std::fs::create_dir_all(dir).map_err(|e| Error::io(format!("create {}", dir.display()), e))
    }
    fn remove(&self, path: &Path) -> Result<()> {
        self.check_write(path, "remove")?;
        std::fs::remove_file(path).map_err(|e| Error::io(format!("remove {}", path.display()), e))
    }
    fn steam_running(&self) -> bool {
        process_running("steam.exe")
    }
}

/// Is a process with this exe name running (read-only process list)?
pub fn process_running(exe: &str) -> bool {
    // SAFETY: a snapshot handle, walked and closed here.
    let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else { return false };
    let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut ok = unsafe { Process32FirstW(snap, &mut e) }.is_ok();
    let mut found = false;
    while ok {
        let end = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
        if String::from_utf16_lossy(&e.szExeFile[..end]).eq_ignore_ascii_case(exe) {
            found = true;
            break;
        }
        ok = unsafe { Process32NextW(snap, &mut e) }.is_ok();
    }
    let _ = unsafe { CloseHandle(snap) };
    found
}

// ------------------------------------------------------------------------------------------------ controllers

/// How the controller is connected, from its HID interface path: Bluetooth Classic HID devices carry the HID service
/// GUID `{00001124-…}`, USB ones `vid_xxxx&pid_xxxx` (Windows' own path formats).
pub fn connection_from_path(path: &str) -> Connection {
    let p = path.to_ascii_lowercase();
    if p.contains("{00001124-0000-1000-8000-00805f9b34fb}") || p.contains("bthenum") || p.contains("bthledevice") {
        Connection::Bluetooth
    } else if p.contains("#vid_") || p.contains("usb#") {
        Connection::Usb
    } else {
        Connection::Unknown
    }
}

/// A HANDLE that may cross threads (the stop event / the device are only used through Win32 calls that are thread-safe).
#[derive(Clone, Copy)]
struct SendHandle(isize);
unsafe impl Send for SendHandle {}
unsafe impl Sync for SendHandle {}
impl SendHandle {
    fn h(self) -> HANDLE {
        HANDLE(self.0 as *mut core::ffi::c_void)
    }
    fn of(h: HANDLE) -> Self {
        SendHandle(h.0 as isize)
    }
}

/// A stop event, closed when its last owner (the source or a stopper) goes.
struct Event(SendHandle);
impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: our own event handle, closed once.
        unsafe {
            let _ = CloseHandle(self.0.h());
        }
    }
}

/// Open a HID device. `read = false` opens it for queries only (no access rights at all), `true` for reading input
/// reports. Never for writing.
fn open_hid(path: &str, read: bool) -> Result<HANDLE> {
    let w = wide(path);
    // SAFETY: plain CreateFileW; the handle is closed by the caller.
    unsafe {
        CreateFileW(
            PCWSTR(w.as_ptr()),
            if read { FILE_GENERIC_READ.0 } else { 0 },
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            if read { FILE_FLAG_OVERLAPPED } else { Default::default() },
            None,
        )
    }
    .map_err(|e| Error::os(format!("open {path}"), e.code().0 as i64))
}

struct HidDev {
    path: String,
    kind: PadKind,
    name: String,
}

/// Every PlayStation gamepad HID interface (usage page 1 / usage 5 = game pad).
fn hid_pads() -> Vec<HidDev> {
    let mut out = Vec::new();
    // SAFETY: SetupDi enumeration with a list freed at the end; buffers sized as the API asks.
    unsafe {
        let guid = HidD_GetHidGuid();
        let Ok(set) = SetupDiGetClassDevsW(Some(&guid), PCWSTR::null(), None, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE) else { return out };
        for i in 0.. {
            let mut ifd = SP_DEVICE_INTERFACE_DATA { cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32, ..Default::default() };
            if SetupDiEnumDeviceInterfaces(set, None, &guid, i, &mut ifd).is_err() {
                break;
            }
            let mut need = 0u32;
            let _ = SetupDiGetDeviceInterfaceDetailW(set, &ifd, None, 0, Some(&mut need), None);
            if need < 8 {
                continue;
            }
            let mut buf = vec![0u8; need as usize + 8];
            let det = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*det).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            if SetupDiGetDeviceInterfaceDetailW(set, &ifd, Some(det), need, None, None).is_err() {
                continue;
            }
            let p = std::ptr::addr_of!((*det).DevicePath) as *const u16;
            let mut n = 0;
            while *p.add(n) != 0 {
                n += 1;
            }
            let path = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
            let Ok(h) = open_hid(&path, false) else { continue };
            let mut attr = HIDD_ATTRIBUTES { Size: std::mem::size_of::<HIDD_ATTRIBUTES>() as u32, ..Default::default() };
            let kind = if HidD_GetAttributes(h, &mut attr) { PadKind::from_ids(attr.VendorID, attr.ProductID) } else { None };
            // Xbox pads come from XInput (wired ones have no HID interface at all)
            let kind = kind.filter(|k| !k.is_xbox());
            let mut gamepad = false;
            let mut pp = PHIDP_PREPARSED_DATA::default();
            if kind.is_some() && HidD_GetPreparsedData(h, &mut pp) {
                let mut caps = HIDP_CAPS::default();
                if HidP_GetCaps(pp, &mut caps).is_ok() {
                    gamepad = caps.UsagePage == 0x01 && caps.Usage == 0x05;
                }
                let _ = HidD_FreePreparsedData(pp);
            }
            let mut name = [0u16; 128];
            let name = if HidD_GetProductString(h, name.as_mut_ptr().cast(), (name.len() * 2) as u32) {
                let e = name.iter().position(|c| *c == 0).unwrap_or(name.len());
                String::from_utf16_lossy(&name[..e])
            } else {
                String::new()
            };
            let _ = CloseHandle(h);
            if let (Some(kind), true) = (kind, gamepad) {
                out.push(HidDev { path, kind, name });
            }
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
    }
    out
}

/// XInput's undocumented `XInputGetCapabilitiesEx` (ordinal 108 of xinput1_4.dll, the one SDL uses) adds the vendor /
/// product id, which tells a real Xbox pad from Steam's own virtual one (vendor 0x28DE).
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct CapsEx {
    caps: XINPUT_CAPABILITIES,
    vendor: u16,
    product: u16,
    version: u16,
    unk1: u16,
    unk2: u32,
}
type CapsExFn = unsafe extern "system" fn(u32, u32, u32, *mut CapsEx) -> u32;

fn caps_ex() -> Option<CapsExFn> {
    // SAFETY: xinput1_4 ships with Windows 10/11; the function pointer is used only with its known signature.
    unsafe {
        let m = LoadLibraryW(PCWSTR(wide("xinput1_4.dll").as_ptr())).ok()?;
        let f = GetProcAddress(m, PCSTR(108usize as *const u8))?;
        Some(std::mem::transmute::<unsafe extern "system" fn() -> isize, CapsExFn>(f))
    }
}

/// Xbox pads in the four XInput slots: (slot, battery). Steam's virtual pads (vendor 0x28DE) are left out.
fn xinput_pads() -> Vec<(u32, Option<Battery>)> {
    let ex = caps_ex();
    let mut out = Vec::new();
    for slot in 0..4u32 {
        let mut st = XINPUT_STATE::default();
        // SAFETY: plain XInput reads.
        if unsafe { XInputGetState(slot, &mut st) } != 0 {
            continue;
        }
        if let Some(f) = ex {
            let mut c = CapsEx::default();
            if unsafe { f(1, slot, 0, &mut c) } == 0 && c.vendor == 0x28DE {
                continue;
            }
        }
        let mut b = XINPUT_BATTERY_INFORMATION::default();
        let battery = (unsafe { XInputGetBatteryInformation(slot, BATTERY_DEVTYPE_GAMEPAD, &mut b) } == 0 && b.BatteryType != BATTERY_TYPE_DISCONNECTED).then(|| {
            let wired = b.BatteryType == BATTERY_TYPE_WIRED;
            Battery {
                percent: None,
                level: if wired {
                        None
                    } else {
                        Some(match b.BatteryLevel.0 {
                            0 => BatteryLevel::Empty,
                            1 => BatteryLevel::Low,
                            2 => BatteryLevel::Medium,
                            _ => BatteryLevel::Full,
                        })
                    },
                charging: false,
                wired,
            }
        });
        out.push((slot, battery));
    }
    out
}

/// The real controllers.
#[derive(Debug, Default, Clone)]
pub struct RealPads {
    /// Read one input report per PlayStation pad while listing (for the battery). Read-only either way.
    pub read_battery: bool,
}

impl RealPads {
    pub fn new() -> Self {
        RealPads { read_battery: true }
    }
}

impl PadOs for RealPads {
    fn list_pads(&self) -> Result<Vec<PadInfo>> {
        let mut out = Vec::new();
        for d in hid_pads() {
            let battery = if self.read_battery {
                HidLive::open(&d.path).ok().and_then(|mut l| l.read_one(400)).and_then(|r| live::battery_in(d.kind, &r))
            } else {
                None
            };
            out.push(PadInfo {
                kind: d.kind,
                name: if d.name.is_empty() { d.kind.name().to_string() } else { d.name },
                connection: connection_from_path(&d.path),
                battery,
                source: PadSource::Hid(d.path),
            });
        }
        for (slot, battery) in xinput_pads() {
            let connection = match battery {
                Some(b) if b.wired => Connection::Usb,
                Some(_) => Connection::Unknown, // wireless adapter or Bluetooth: XInput doesn't say which
                None => Connection::Unknown,
            };
            out.push(PadInfo { kind: PadKind::Xbox, name: PadKind::Xbox.name().into(), connection, battery, source: PadSource::XInput(slot) });
        }
        Ok(out)
    }

    fn open_live(&self, pad: &PadInfo) -> Result<Box<dyn LiveSource>> {
        match &pad.source {
            PadSource::Hid(path) => Ok(Box::new(HidLive::open(path).map_err(|_| Error::PadGone(pad.name.clone()))?)),
            PadSource::XInput(slot) => Ok(Box::new(XLive::open(*slot)?)),
        }
    }
}

/// A PlayStation pad's input reports: overlapped reads that wake on a report or on the stop event.
struct HidLive {
    dev: SendHandle,
    read_ev: SendHandle,
    stop_ev: Arc<Event>,
    ov: Box<OVERLAPPED>,
    buf: Vec<u8>,
    pending: bool,
}

// SAFETY: the OVERLAPPED's event handle is a plain kernel handle; a HidLive is used by one thread at a time (it moves to
// the reader thread once), only the stop event is touched from elsewhere (SetEvent is thread-safe).
unsafe impl Send for HidLive {}

impl HidLive {
    fn open(path: &str) -> Result<HidLive> {
        let dev = open_hid(path, true)?;
        // SAFETY: two unnamed events, closed in Drop.
        let (read_ev, stop_ev) = unsafe {
            let r = CreateEventW(None, true, false, PCWSTR::null()).map_err(|e| Error::os("CreateEvent", e.code().0 as i64));
            let s = CreateEventW(None, true, false, PCWSTR::null()).map_err(|e| Error::os("CreateEvent", e.code().0 as i64));
            match (r, s) {
                (Ok(r), Ok(s)) => (r, s),
                (r, s) => {
                    for h in [r.ok(), s.ok()].into_iter().flatten().chain([dev]) {
                        let _ = CloseHandle(h);
                    }
                    return Err(Error::os("CreateEvent", 0));
                }
            }
        };
        Ok(HidLive {
            dev: SendHandle::of(dev),
            read_ev: SendHandle::of(read_ev),
            stop_ev: Arc::new(Event(SendHandle::of(stop_ev))),
            ov: Box::new(OVERLAPPED::default()),
            buf: vec![0u8; 128],
            pending: false,
        })
    }

    /// Start a read if none is running. Ok(Some(n)) = finished at once.
    fn start_read(&mut self) -> std::result::Result<Option<usize>, ()> {
        if self.pending {
            return Ok(None);
        }
        *self.ov = OVERLAPPED { hEvent: self.read_ev.h(), ..Default::default() };
        let mut n = 0u32;
        // SAFETY: `buf` and `ov` live in self (the OVERLAPPED is boxed, never moved) until the read finishes or is cancelled.
        let r = unsafe { ReadFile(self.dev.h(), Some(&mut self.buf), Some(&mut n), Some(&mut *self.ov)) };
        match r {
            Ok(()) => Ok(Some(n as usize)),
            Err(_) if unsafe { GetLastError() } == ERROR_IO_PENDING => {
                self.pending = true;
                Ok(None)
            }
            Err(_) => Err(()),
        }
    }

    fn finish(&mut self) -> std::result::Result<usize, ()> {
        let mut n = 0u32;
        // SAFETY: waits for the read started in start_read.
        let r = unsafe { GetOverlappedResult(self.dev.h(), &*self.ov, &mut n, true) };
        self.pending = false;
        r.map(|_| n as usize).map_err(|_| ())
    }

    /// One report within `ms` (the controller list's battery read).
    fn read_one(&mut self, ms: u32) -> Option<Vec<u8>> {
        if let Some(n) = self.start_read().ok()? {
            return Some(self.buf[..n].to_vec());
        }
        // SAFETY: waits on our own event.
        if unsafe { WaitForSingleObject(self.read_ev.h(), ms) } != WAIT_OBJECT_0 {
            return None; // Drop cancels the read
        }
        let n = self.finish().ok()?;
        Some(self.buf[..n].to_vec())
    }
}

impl LiveSource for HidLive {
    fn next(&mut self) -> Result<LiveEvent> {
        match self.start_read() {
            Ok(Some(n)) => return Ok(LiveEvent::Report(self.buf[..n].to_vec())),
            Ok(None) => {}
            Err(()) => return Ok(LiveEvent::Gone),
        }
        // SAFETY: waits on our own two events; no timeout (no polling).
        let w = unsafe { WaitForMultipleObjects(&[self.read_ev.h(), self.stop_ev.0.h()], false, INFINITE) };
        if w == WAIT_OBJECT_0 {
            return match self.finish() {
                Ok(n) => Ok(LiveEvent::Report(self.buf[..n].to_vec())),
                Err(()) => Ok(LiveEvent::Gone),
            };
        }
        Ok(LiveEvent::Stopped)
    }
    fn stopper(&self) -> Arc<dyn Fn() + Send + Sync> {
        let ev = self.stop_ev.clone();
        // SAFETY: the stopper holds its own Arc, so the event stays open for as long as it can be called.
        Arc::new(move || unsafe {
            let _ = SetEvent(ev.0.h());
        })
    }
}

impl Drop for HidLive {
    fn drop(&mut self) {
        // SAFETY: cancel a running read and wait for it before the buffer goes; then close our handles.
        unsafe {
            if self.pending {
                let _ = CancelIoEx(self.dev.h(), Some(&*self.ov));
                let mut n = 0u32;
                let _ = GetOverlappedResult(self.dev.h(), &*self.ov, &mut n, true);
            }
            let _ = CloseHandle(self.dev.h());
            let _ = CloseHandle(self.read_ev.h());
        }
    }
}

/// How often an Xbox pad is polled while the live view runs (XInput has no events). 8 ms = 125 per second.
pub const XINPUT_POLL_MS: u32 = 8;

/// An Xbox pad: XInput polled every [`XINPUT_POLL_MS`] while running; only changed packets are handed over.
struct XLive {
    slot: u32,
    stop_ev: Arc<Event>,
    last: Option<u32>,
}

impl XLive {
    fn open(slot: u32) -> Result<XLive> {
        // SAFETY: an unnamed event, closed in Drop.
        let ev = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(|e| Error::os("CreateEvent", e.code().0 as i64))?;
        Ok(XLive { slot, stop_ev: Arc::new(Event(SendHandle::of(ev))), last: None })
    }
}

impl LiveSource for XLive {
    fn next(&mut self) -> Result<LiveEvent> {
        loop {
            let mut st = XINPUT_STATE::default();
            // SAFETY: plain XInput read.
            if unsafe { XInputGetState(self.slot, &mut st) } != 0 {
                return Ok(LiveEvent::Gone);
            }
            if self.last != Some(st.dwPacketNumber) {
                self.last = Some(st.dwPacketNumber);
                let g = st.Gamepad;
                return Ok(LiveEvent::Report(live::xinput_report(g.wButtons.0, g.bLeftTrigger, g.bRightTrigger, g.sThumbLX, g.sThumbLY, g.sThumbRX, g.sThumbRY)));
            }
            // SAFETY: waits on our own event.
            let w = unsafe { WaitForSingleObject(self.stop_ev.0.h(), XINPUT_POLL_MS) };
            if w != WAIT_TIMEOUT {
                return Ok(LiveEvent::Stopped);
            }
        }
    }
    fn stopper(&self) -> Arc<dyn Fn() + Send + Sync> {
        let ev = self.stop_ev.clone();
        // SAFETY: as above, the Arc keeps the event open.
        Arc::new(move || unsafe {
            let _ = SetEvent(ev.0.h());
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_inside_roots() {
        let roots = vec![PathBuf::from(r"C:\Steam\steamapps\common\Steam Controller Configs"), PathBuf::from(r"D:\bu\backups")];
        assert!(inside(Path::new(r"c:/steam/steamapps/common/steam controller configs/1/config/252950/controller_ps5.vdf"), &roots));
        assert!(inside(Path::new(r"D:\bu\backups\1\x.original"), &roots));
        assert!(!inside(Path::new(r"C:\Steam\config\config.vdf"), &roots));
        assert!(!inside(Path::new(r"C:\Steam\steamapps\common\Steam Controller Configs\..\..\x.vdf"), &roots));
        assert!(!inside(Path::new(r"C:\Steam\steamapps\common\Steam Controller Configs"), &roots), "the root itself is not a file inside it");
        assert!(!inside(Path::new(r"C:\x"), &[]));
    }

    #[test]
    fn connection_kinds() {
        assert_eq!(connection_from_path(r"\\?\hid#vid_054c&pid_0df2&mi_03#8&1a2b&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}"), Connection::Usb);
        assert_eq!(
            connection_from_path(r"\\?\hid#{00001124-0000-1000-8000-00805f9b34fb}_vid&0002054c_pid&0ce6#9&2f&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}"),
            Connection::Bluetooth
        );
        assert_eq!(connection_from_path(r"\\?\hid#something"), Connection::Unknown);
    }

    #[test]
    fn read_only_refuses_every_change() {
        // a MISSING path inside the lane's scratch folder: even a broken guard could not write anywhere real
        let scratch = Path::new(r"C:\BoylerUtilities-scratch\L");
        if !scratch.is_dir() {
            eprintln!("scratch folder missing: skipped");
            return;
        }
        let s = RealSteam::read_only();
        let p = scratch.join("missing-dir-never-made").join("never-written.vdf");
        assert!(matches!(s.write(&p, b"x"), Err(Error::ReadOnly(_))));
        assert!(matches!(s.create_dir_all(&p), Err(Error::ReadOnly(_))));
        assert!(matches!(s.remove(&p), Err(Error::ReadOnly(_))));
        assert!(!p.exists());
    }
}

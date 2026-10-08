//! The real Windows implementation of [`PerfOs`]. Reads are safe on any PC. The calls that change something
//! (`end_process`, `set_priority`, `open_file_location`) are only reached through [`crate::processes`] — tests never
//! call them on the real OS.
//!
//! Processes: one `NtQuerySystemInformation(SystemProcessInformation)` call per refresh gives every process with its
//! CPU time, private working set, parent, session and base priority — without opening any process (what Task
//! Manager / Process Explorer use). Path, description and user are read once per process and cached.

mod live;
mod nvml;
mod specs;
pub mod wmi;

use crate::specs::PcSpecs;
use crate::{EndHow, Icon, LiveSource, PerfError, PerfOs, Priority, ProcessUser, RawProcess, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use windows::core::{PCWSTR, PWSTR};
use windows::Wdk::System::SystemInformation::{NtQuerySystemInformation, SystemProcessInformation};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::Security::{GetLengthSid, GetTokenInformation, TokenElevation, TokenUser, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Threading::{
    GetActiveProcessorCount, GetCurrentProcess, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
    SetPriorityClass, TerminateProcess, ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, HIGH_PRIORITY_CLASS,
    IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SET_INFORMATION, PROCESS_TERMINATE,
};
use windows::Win32::UI::Shell::{SHDefExtractIconW, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, EnumWindows, GetIconInfo, GetWindow, GetWindowTextLengthW, GetWindowThreadProcessId, IsWindowVisible,
    PostMessageW, GW_OWNER, HICON, ICONINFO, SW_SHOWNORMAL, WM_CLOSE,
};

pub use live::RealLive;

#[derive(Debug, Clone)]
struct StaticInfo {
    path: Option<PathBuf>,
    description: Option<String>,
    user: ProcessUser,
}

/// The real OS. Keeps a small cache of per-process facts that never change (path, description, user).
/// - [`RealOs::new`] — the app's one.
/// - [`RealOs::read_only`] — reads are real, EVERY change is refused (End, priority, opening Explorer) — `examples/show`.
#[derive(Default)]
pub struct RealOs {
    cache: Mutex<HashMap<(u32, u64), StaticInfo>>,
    read_only: bool,
}

impl RealOs {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn read_only() -> Self {
        RealOs { read_only: true, ..Self::default() }
    }
    fn refuse(&self, what: &str) -> Result<()> {
        if self.read_only {
            return Err(PerfError::Refused(format!("read-only OS layer refuses {what}")));
        }
        Ok(())
    }
}

struct H(HANDLE);
impl Drop for H {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn win_err(context: &str, e: windows::core::Error) -> PerfError {
    match e.code().0 as u32 {
        0x8007_0005 => PerfError::NeedsAdmin(context.to_string()),
        0x8007_0057 => PerfError::NotFound(context.to_string()), // ERROR_INVALID_PARAMETER: no such pid
        c => PerfError::Os { context: context.to_string(), code: c },
    }
}

pub(crate) fn elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let token = H(token);
        let mut e = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        GetTokenInformation(token.0, TokenElevation, Some(&mut e as *mut _ as *mut _), std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut len).is_ok()
            && e.TokenIsElevated != 0
    }
}

/// The user SID of a process handle, as its bytes (SIDs are canonical, so equal bytes = same user).
fn token_user(process: HANDLE) -> Option<Vec<u8>> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
        let token = H(token);
        let mut len = 0u32;
        let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut len);
        if len == 0 {
            return None;
        }
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        GetTokenInformation(token.0, TokenUser, Some(buf.as_mut_ptr() as *mut _), len, &mut len).ok()?;
        let sid = (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid;
        let n = GetLengthSid(sid) as usize;
        if n == 0 {
            return None;
        }
        Some(std::slice::from_raw_parts(sid.0 as *const u8, n).to_vec())
    }
}

fn same_user(a: &[u8], b: &[u8]) -> bool {
    a == b
}

/// FileDescription from the exe's version resource.
fn file_description(path: &Path) -> Option<String> {
    unsafe {
        let w = wide(&path.to_string_lossy());
        let size = GetFileVersionInfoSizeW(PCWSTR(w.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(PCWSTR(w.as_ptr()), None, size, data.as_mut_ptr() as *mut _).ok()?;
        let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        let tr = wide("\\VarFileInfo\\Translation");
        let langs: Vec<(u16, u16)> = if VerQueryValueW(data.as_ptr() as *const _, PCWSTR(tr.as_ptr()), &mut p, &mut len).as_bool() && len >= 4 {
            std::slice::from_raw_parts(p as *const u16, (len / 2) as usize).chunks(2).map(|c| (c[0], c[1])).collect()
        } else {
            vec![(0x0409, 0x04B0)]
        };
        for (lang, cp) in langs.into_iter().chain([(0x0409, 0x04B0), (0x0409, 0x04E4)]) {
            let key = wide(&format!("\\StringFileInfo\\{lang:04x}{cp:04x}\\FileDescription"));
            if VerQueryValueW(data.as_ptr() as *const _, PCWSTR(key.as_ptr()), &mut p, &mut len).as_bool() && len > 1 {
                let s = String::from_utf16_lossy(std::slice::from_raw_parts(p as *const u16, len as usize));
                let s = s.trim_end_matches('\0').trim().to_string();
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
        None
    }
}

fn static_info(pid: u32, session: u32, own_session: u32, own_user: &Option<Vec<u8>>) -> StaticInfo {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            let user = if session == 0 { ProcessUser::System } else if session == own_session { ProcessUser::Unknown } else { ProcessUser::OtherUser };
            return StaticInfo { path: None, description: None, user };
        };
        let h = H(h);
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let path = QueryFullProcessImageNameW(h.0, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
            .ok()
            .map(|_| PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])));
        let user = match (token_user(h.0), own_user) {
            (Some(t), Some(me)) if same_user(&t, me) => ProcessUser::Me,
            _ if session == 0 => ProcessUser::System,
            (Some(_), _) => ProcessUser::OtherUser,
            _ => ProcessUser::Unknown,
        };
        let description = path.as_deref().and_then(file_description);
        StaticInfo { path, description, user }
    }
}

/// Pids that own a visible, unowned, titled top-level window (a plain app), with those windows.
fn windowed_pids() -> HashMap<u32, Vec<isize>> {
    unsafe extern "system" fn cb(hwnd: HWND, lp: LPARAM) -> windows::core::BOOL {
        unsafe {
            let map = &mut *(lp.0 as *mut HashMap<u32, Vec<isize>>);
            if IsWindowVisible(hwnd).as_bool() && GetWindow(hwnd, GW_OWNER).map(|o| o.is_invalid()).unwrap_or(true) && GetWindowTextLengthW(hwnd) > 0 {
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                map.entry(pid).or_default().push(hwnd.0 as isize);
            }
        }
        true.into()
    }
    let mut map: HashMap<u32, Vec<isize>> = HashMap::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut map as *mut _ as isize));
    }
    map
}

fn read_u32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap_or_default())
}
fn read_u64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap_or_default())
}

/// The process snapshot as raw bytes (SYSTEM_PROCESS_INFORMATION list).
fn process_snapshot() -> Result<Vec<u8>> {
    let mut size = 512 * 1024u32;
    for _ in 0..8 {
        let mut buf = vec![0u8; size as usize];
        let mut needed = 0u32;
        let st = unsafe { NtQuerySystemInformation(SystemProcessInformation, buf.as_mut_ptr() as *mut _, size, &mut needed) };
        if st.0 >= 0 {
            buf.truncate(needed.max(1) as usize);
            return Ok(buf);
        }
        if st.0 as u32 != 0xC000_0004 {
            // not STATUS_INFO_LENGTH_MISMATCH
            return Err(PerfError::Os { context: "NtQuerySystemInformation".into(), code: st.0 as u32 });
        }
        size = needed.max(size) + 64 * 1024;
    }
    Err(PerfError::Os { context: "NtQuerySystemInformation (buffer)".into(), code: 0xC000_0004 })
}

impl PerfOs for RealOs {
    fn open_live(&self) -> Result<Box<dyn LiveSource>> {
        Ok(Box::new(RealLive::open()?))
    }

    #[cfg(target_pointer_width = "64")]
    fn processes(&self) -> Result<Vec<RawProcess>> {
        // SYSTEM_PROCESS_INFORMATION, x64 offsets (phnt / ntexapi.h): 0 NextEntryOffset · 32 CreateTime ·
        // 24 CycleTime · 40 UserTime · 48 KernelTime · 56 ImageName (Length u16, …, Buffer ptr at +8) · 72 BasePriority ·
        // 80 UniqueProcessId · 88 InheritedFromUniqueProcessId · 100 SessionId · 8 WorkingSetPrivateSize.
        let buf = process_snapshot()?;
        let windows = windowed_pids();
        let own_session = {
            let mut s = 0u32;
            unsafe {
                let _ = windows::Win32::System::RemoteDesktop::ProcessIdToSessionId(std::process::id(), &mut s);
            }
            s
        };
        let own_user = unsafe { token_user(GetCurrentProcess()) };
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        let mut off = 0usize;
        loop {
            if off + 136 > buf.len() {
                break;
            }
            let e = &buf[off..];
            let next = read_u32(e, 0) as usize;
            let create_time = read_u64(e, 32);
            let cpu_time = read_u64(e, 40) + read_u64(e, 48);
            let cycle_time = read_u64(e, 24);
            let name_len = u16::from_le_bytes([e[56], e[57]]) as usize;
            let name_ptr = read_u64(e, 64) as *const u16;
            let base_priority = read_u32(e, 72) as i32;
            let pid = read_u64(e, 80) as u32;
            let parent_pid = read_u64(e, 88) as u32;
            let session_id = read_u32(e, 100);
            let ram_bytes = read_u64(e, 8);
            // The name points into our own buffer (it is copied with the snapshot).
            let exe = if name_ptr.is_null() || name_len == 0 {
                if pid == 0 { "Idle".to_string() } else { "System".to_string() }
            } else {
                let base = buf.as_ptr() as usize;
                let p = name_ptr as usize;
                if p >= base && p + name_len <= base + buf.len() {
                    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(name_ptr, name_len / 2) })
                } else {
                    String::new()
                }
            };
            let key = (pid, create_time);
            seen.insert(key);
            let info = if pid == 0 || pid == 4 {
                StaticInfo { path: None, description: None, user: ProcessUser::System }
            } else {
                cache.entry(key).or_insert_with(|| static_info(pid, session_id, own_session, &own_user)).clone()
            };
            out.push(RawProcess {
                pid,
                parent_pid,
                exe,
                path: info.path,
                description: info.description,
                session_id,
                user: info.user,
                create_time,
                cpu_time,
                cycle_time,
                ram_bytes,
                priority: Priority::from_base(base_priority),
                has_window: windows.contains_key(&pid),
            });
            if next == 0 {
                break;
            }
            off += next;
        }
        cache.retain(|k, _| seen.contains(k));
        Ok(out)
    }

    #[cfg(not(target_pointer_width = "64"))]
    fn processes(&self) -> Result<Vec<RawProcess>> {
        Err(PerfError::Unavailable("process list needs the 64-bit build".into()))
    }

    fn cpu_count(&self) -> u32 {
        unsafe { GetActiveProcessorCount(0xFFFF) }.max(1)
    }

    fn specs(&self) -> Result<PcSpecs> {
        specs::read()
    }

    fn end_process(&self, pid: u32, how: EndHow) -> Result<()> {
        self.refuse("ending a process")?;
        match how {
            EndHow::Close => {
                let wins = windowed_pids().remove(&pid).unwrap_or_default();
                if wins.is_empty() {
                    return Err(PerfError::NotFound(format!("a window of pid {pid}")));
                }
                for w in wins {
                    unsafe {
                        PostMessageW(Some(HWND(w as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0)).map_err(|e| win_err("WM_CLOSE", e))?;
                    }
                }
                Ok(())
            }
            EndHow::Terminate => unsafe {
                let h = H(OpenProcess(PROCESS_TERMINATE, false, pid).map_err(|e| win_err(&format!("end pid {pid}"), e))?);
                TerminateProcess(h.0, 1).map_err(|e| win_err(&format!("end pid {pid}"), e))
            },
        }
    }

    fn set_priority(&self, pid: u32, p: Priority) -> Result<()> {
        self.refuse("a priority change")?;
        let class = match p {
            Priority::Realtime => return Err(PerfError::Refused("Realtime".into())),
            Priority::High => HIGH_PRIORITY_CLASS,
            Priority::AboveNormal => ABOVE_NORMAL_PRIORITY_CLASS,
            Priority::Normal => NORMAL_PRIORITY_CLASS,
            Priority::BelowNormal => BELOW_NORMAL_PRIORITY_CLASS,
            Priority::Low => IDLE_PRIORITY_CLASS,
        };
        unsafe {
            let h = H(OpenProcess(PROCESS_SET_INFORMATION, false, pid).map_err(|e| win_err(&format!("priority pid {pid}"), e))?);
            SetPriorityClass(h.0, class).map_err(|e| win_err(&format!("priority pid {pid}"), e))
        }
    }

    fn open_file_location(&self, path: &Path) -> Result<()> {
        self.refuse("opening Explorer")?;
        let args = wide(&format!("/select,\"{}\"", path.display()));
        let r = unsafe {
            ShellExecuteW(None, PCWSTR(wide("open").as_ptr()), PCWSTR(wide("explorer.exe").as_ptr()), PCWSTR(args.as_ptr()), PCWSTR::null(), SW_SHOWNORMAL)
        };
        if r.0 as isize > 32 {
            Ok(())
        } else {
            Err(PerfError::Os { context: "ShellExecute explorer".into(), code: r.0 as usize as u32 })
        }
    }

    fn icon_rgba(&self, path: &Path, size: u32) -> Result<Icon> {
        unsafe {
            let w = wide(&path.to_string_lossy());
            let mut icon = HICON::default();
            let hr = SHDefExtractIconW(PCWSTR(w.as_ptr()), 0, 0, Some(&mut icon), None, size);
            if hr.is_err() || icon.is_invalid() {
                return Err(PerfError::NotFound(format!("icon of {}", path.display())));
            }
            let mut info = ICONINFO::default();
            let got = GetIconInfo(icon, &mut info);
            let result = (|| {
                got.map_err(|e| win_err("GetIconInfo", e))?;
                let dc = GetDC(None);
                let mut bmi = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: size as i32,
                    biHeight: -(size as i32), // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                    ..Default::default()
                };
                let mut px = vec![0u8; (size * size * 4) as usize];
                let lines = GetDIBits(dc, info.hbmColor, 0, size, Some(px.as_mut_ptr() as *mut _), &mut bmi, DIB_RGB_COLORS);
                ReleaseDC(None, dc);
                if lines == 0 {
                    return Err(PerfError::Os { context: "GetDIBits".into(), code: 0 });
                }
                let has_alpha = px.chunks(4).any(|c| c[3] != 0);
                for c in px.chunks_mut(4) {
                    c.swap(0, 2); // BGRA → RGBA
                    if !has_alpha {
                        c[3] = 255;
                    }
                }
                Ok(Icon { width: size, height: size, rgba: px })
            })();
            if !info.hbmColor.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(info.hbmColor.0));
            }
            if !info.hbmMask.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(info.hbmMask.0));
            }
            let _ = DestroyIcon(icon);
            result
        }
    }

    fn is_elevated(&self) -> bool {
        elevated()
    }
}

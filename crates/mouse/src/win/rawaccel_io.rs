//! Raw Accel's driver through its own control device (`DEVICE`) — Raw Accel's source, MIT: `common/rawaccel-io-def.h`,
//! `common/rawaccel-io.hpp`, tag v1.7.0): GET_VERSION and READ (reads) and WRITE (the settings; the driver waits its ~1 s
//! anti-abuse delay). Fallback for an unknown driver version: Raw Accel's own `writer.exe <file>` (it converts the JSON to
//! THAT driver's layout; note it shows a message box on an error, so it is only used when the layout is unknown).

use crate::error::{Error, Result};
use crate::os::DriverVersion;
use std::path::Path;
use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, HANDLE};
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::System::IO::DeviceIoControl;

/// `CTL_CODE(DeviceType, Function, Method, Access)`.
pub const fn ctl_code(dev: u32, func: u32, method: u32, access: u32) -> u32 {
    (dev << 16) | (access << 14) | (func << 2) | method
}
pub const RA_DEV_TYPE: u32 = 0x8888;
const METHOD_BUFFERED: u32 = 0;
const FILE_ANY_ACCESS: u32 = 0;
/// RA_READ / RA_WRITE / RA_GET_VERSION (= 0x88882220 / 0x88882224 / 0x88882228)
pub const RA_READ: u32 = ctl_code(RA_DEV_TYPE, 0x888, METHOD_BUFFERED, FILE_ANY_ACCESS);
pub const RA_WRITE: u32 = ctl_code(RA_DEV_TYPE, 0x889, METHOD_BUFFERED, FILE_ANY_ACCESS);
pub const RA_GET_VERSION: u32 = ctl_code(RA_DEV_TYPE, 0x88a, METHOD_BUFFERED, FILE_ANY_ACCESS);
pub const DEVICE: &str = r"\\.\rawaccel";

/// Opens `\\.\rawaccel` the way Raw Accel does (dwDesiredAccess 0 — its ioctls are FILE_ANY_ACCESS; the device is
/// world read/write, so no admin). `None` when the device does not exist (driver not installed / not started / safe mode).
fn open() -> Result<Option<HANDLE>> {
    match unsafe { CreateFileW(&HSTRING::from(DEVICE), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) } {
        Ok(h) => Ok(Some(h)),
        Err(e) if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() || e.code() == ERROR_PATH_NOT_FOUND.to_hresult() => Ok(None),
        Err(e) => Err(super::hr("open the Raw Accel device", e)),
    }
}

/// The driver's version, `None` when the device does not exist.
pub fn driver_version() -> Result<Option<DriverVersion>> {
    let Some(h) = open()? else { return Ok(None) };
    let mut v = [0i32; 3];
    let mut n = 0u32;
    let r = unsafe { DeviceIoControl(h, RA_GET_VERSION, None, 0, Some(v.as_mut_ptr() as *mut _), size_of::<[i32; 3]>() as u32, Some(&mut n), None) };
    let _ = unsafe { CloseHandle(h) };
    r.map_err(|e| super::hr("Raw Accel GET_VERSION", e))?;
    if n as usize != size_of::<[i32; 3]>() {
        return Err(Error::RawAccelSettings(format!("GET_VERSION returned {n} bytes")));
    }
    Ok(Some(DriverVersion { major: v[0] as u32, minor: v[1] as u32, patch: v[2] as u32 }))
}

/// Writes the JSON to the app's own file and runs `<rawaccel_dir>\writer.exe <file>` hidden; its output is kept word for
/// word when it refuses.
pub fn run_writer(rawaccel_dir: &Path, settings_file: &Path, json: &str) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let writer = rawaccel_dir.join("writer.exe");
    if !writer.is_file() {
        return Err(Error::RawAccelMissing(format!("{} not found", writer.display())));
    }
    if let Some(d) = settings_file.parent() {
        std::fs::create_dir_all(d).map_err(|e| Error::io(format!("create {}", d.display()), e))?;
    }
    std::fs::write(settings_file, json).map_err(|e| Error::io(format!("write {}", settings_file.display()), e))?;
    let out = std::process::Command::new(&writer)
        .arg(settings_file)
        .current_dir(rawaccel_dir)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| Error::io(format!("run {}", writer.display()), e))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)).trim().to_string();
    if out.status.success() && text.is_empty() {
        Ok(())
    } else {
        Err(Error::RawAccelRefused(if text.is_empty() { format!("writer.exe exit code {:?}", out.status.code()) } else { text }))
    }
}

/// READ: the header first (40 bytes), then the whole buffer at the size the header gives — like `rawaccel::read`.
pub fn read() -> Result<Option<Vec<u8>>> {
    let Some(h) = open()? else { return Ok(None) };
    let call = |buf: &mut [u8]| -> Result<u32> {
        let mut n = 0u32;
        unsafe { DeviceIoControl(h, RA_READ, None, 0, Some(buf.as_mut_ptr() as *mut _), buf.len() as u32, Some(&mut n), None) }
            .map_err(|e| super::hr("Raw Accel READ", e))?;
        Ok(n)
    };
    let r = (|| {
        let mut head = vec![0u8; crate::accel::bytes::IO_BASE];
        call(&mut head)?;
        let (m, d) = crate::accel::bytes::read_header(&head).unwrap_or((0, 0));
        if m == 0 {
            return Ok(head);
        }
        let mut all = vec![0u8; crate::accel::bytes::write_size(m as usize, d as usize)];
        let n = call(&mut all)? as usize;
        if n != all.len() {
            return Err(Error::RawAccelSettings(format!("READ returned {n} bytes, the v1.7.0 layout says {}", all.len())));
        }
        Ok(all)
    })();
    let _ = unsafe { CloseHandle(h) };
    r.map(Some)
}

/// WRITE: the whole buffer; the driver checks the exact size and waits ~1 s before it applies it.
pub fn write(bytes: &[u8]) -> Result<()> {
    let Some(h) = open()? else { return Err(Error::RawAccelMissing("the Raw Accel driver is not running".into())) };
    let mut n = 0u32;
    let r = unsafe { DeviceIoControl(h, RA_WRITE, Some(bytes.as_ptr() as *const _), bytes.len() as u32, None, 0, Some(&mut n), None) };
    let _ = unsafe { CloseHandle(h) };
    r.map_err(|e| super::hr("Raw Accel WRITE", e))
}

//! Drives, physical disks and health through storage IOCTLs (no driver). All reads.
//!
//! Handles are opened with **0 access** wherever Windows allows it (no admin needed): `IOCTL_STORAGE_QUERY_PROPERTY`,
//! `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS` and `IOCTL_DISK_GET_DRIVE_GEOMETRY_EX` are FILE_ANY_ACCESS. SATA SMART
//! (`SMART_RCV_DRIVE_DATA`) needs read+write access to the disk, which needs admin — tried, and listed under
//! "needs admin" when refused. The SMART commands sent are READ DATA (0xD0) and READ THRESHOLDS (0xD1) only.

use super::wmi::Wmi;
use crate::{
    DriveInfo, DriveKind, HealthRaw, MediaKind, NvmeHealthLog, OsHealthStatus, PhysicalDisk, ReliabilityCounter, Result,
    SmartAttribute, StorageError, VolumeState,
};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW, FILE_FLAGS_AND_ATTRIBUTES,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::{
    IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, IOCTL_STORAGE_QUERY_PROPERTY,
    SMART_RCV_DRIVE_DATA,
};
use windows::Win32::System::IO::DeviceIoControl;

/// `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS` (winioctl.h; lives in the Storage_FileSystem feature of the windows crate).
const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: u32 = 0x0056_0000;

pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn open(path: &str, access: u32) -> windows::core::Result<Handle> {
    let w = wide(path);
    unsafe {
        CreateFileW(
            PCWSTR(w.as_ptr()),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
        .map(Handle)
    }
}

fn ioctl(h: &Handle, code: u32, input: &[u8], out: &mut [u8]) -> windows::core::Result<u32> {
    let mut got = 0u32;
    unsafe {
        DeviceIoControl(
            h.0,
            code,
            Some(input.as_ptr() as *const _),
            input.len() as u32,
            Some(out.as_mut_ptr() as *mut _),
            out.len() as u32,
            Some(&mut got),
            None,
        )?;
    }
    Ok(got)
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap_or([0; 4]))
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap_or([0; 8]))
}

/// STORAGE_PROPERTY_QUERY { PropertyId, QueryType = standard } (+ extra parameters).
fn property_query(property_id: i32, extra: &[u8]) -> Vec<u8> {
    let mut q = Vec::with_capacity(8 + extra.len());
    q.extend_from_slice(&property_id.to_le_bytes());
    q.extend_from_slice(&0i32.to_le_bytes());
    q.extend_from_slice(extra);
    if extra.is_empty() {
        q.extend_from_slice(&[0u8; 4]); // AdditionalParameters[1] + padding, as sizeof() in C
    }
    q
}

fn c_string(b: &[u8], off: u32) -> Option<String> {
    let off = off as usize;
    if off == 0 || off >= b.len() {
        return None;
    }
    let end = b[off..].iter().position(|&c| c == 0).map(|p| off + p).unwrap_or(b.len());
    let s = String::from_utf8_lossy(&b[off..end]).trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn bus_name(bus: u32) -> Option<String> {
    Some(
        match bus {
            1 => "SCSI",
            3 => "ATA",
            7 => "USB",
            8 => "RAID",
            10 => "SAS",
            11 => "SATA",
            12 => "SD",
            13 => "MMC",
            15 => "Virtual",
            16 => "Storage Spaces",
            17 => "NVMe",
            _ => return None,
        }
        .to_string(),
    )
}

/// Model, bus, SSD/HDD and size of `\\.\PhysicalDriveN` (no admin).
pub fn disk_identity(number: u32) -> Option<PhysicalDisk> {
    let h = open(&format!("\\\\.\\PhysicalDrive{number}"), 0).ok()?;
    let mut buf = vec![0u8; 1024];
    let q = property_query(0, &[]); // StorageDeviceProperty
    ioctl(&h, IOCTL_STORAGE_QUERY_PROPERTY, &q, &mut buf).ok()?;
    let vendor = c_string(&buf, u32_at(&buf, 12));
    let product = c_string(&buf, u32_at(&buf, 16)).unwrap_or_default();
    let bus = bus_name(u32_at(&buf, 28));
    let model = match vendor {
        Some(v) if !matches!(v.as_str(), "NVMe" | "ATA" | "SATA") && !product.starts_with(&v) => format!("{v} {product}"),
        _ => product,
    };
    let mut sp = [0u8; 16];
    let media = match ioctl(&h, IOCTL_STORAGE_QUERY_PROPERTY, &property_query(7, &[]), &mut sp) {
        Ok(n) if n >= 9 => {
            if sp[8] != 0 {
                MediaKind::Hdd
            } else {
                MediaKind::Ssd
            }
        }
        _ => MediaKind::Unknown,
    };
    let mut geo = [0u8; 256];
    let size_bytes = ioctl(&h, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, &[], &mut geo).map(|_| u64_at(&geo, 24)).unwrap_or(0);
    Some(PhysicalDisk { number, model, media, bus, size_bytes })
}

/// Physical disks 0–31 that open.
pub fn physical_disks() -> Vec<PhysicalDisk> {
    (0..32).filter_map(disk_identity).collect()
}

fn volume_disk_number(letter: char) -> Option<u32> {
    let h = open(&format!("\\\\.\\{letter}:"), 0).ok()?;
    let mut buf = [0u8; 256];
    ioctl(&h, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, &[], &mut buf).ok()?;
    (u32_at(&buf, 0) > 0).then(|| u32_at(&buf, 8))
}

const FVE_E_LOCKED_VOLUME: u32 = 0x8031_0000;
const ERROR_NOT_READY: u32 = 0x8007_0015;

pub fn drives() -> Result<Vec<DriveInfo>> {
    let mask = unsafe { GetLogicalDrives() };
    if mask == 0 {
        return Err(StorageError::Os { context: "GetLogicalDrives".into(), code: 0 });
    }
    let system = std::env::var("SystemDrive").ok().and_then(|s| s.chars().next()).map(|c| c.to_ascii_uppercase());
    let mut out = Vec::new();
    let mut identities: std::collections::HashMap<u32, Option<PhysicalDisk>> = Default::default();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = wide(&format!("{letter}:\\"));
        let kind = match unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } {
            2 => DriveKind::Removable,
            3 => DriveKind::Fixed,
            4 => DriveKind::Network,
            5 => DriveKind::CdRom,
            6 => DriveKind::RamDisk,
            _ => DriveKind::Unknown,
        };
        let mut info = DriveInfo {
            letter,
            kind,
            state: VolumeState::Ready,
            label: String::new(),
            file_system: String::new(),
            total_bytes: 0,
            free_bytes: 0,
            is_system: Some(letter) == system,
            model: None,
            media: MediaKind::Unknown,
            bus: None,
            disk_number: None,
        };
        // Network and CD drives are not shown; don't touch them (a dead network drive can hang for seconds).
        if matches!(kind, DriveKind::Network | DriveKind::CdRom | DriveKind::Unknown) {
            out.push(info);
            continue;
        }
        let mut label = [0u16; 261];
        let mut fs = [0u16; 261];
        match unsafe { GetVolumeInformationW(PCWSTR(root.as_ptr()), Some(&mut label), None, None, None, Some(&mut fs)) } {
            Ok(()) => {
                info.label = from_wide(&label);
                info.file_system = from_wide(&fs);
            }
            Err(e) => {
                let code = e.code().0 as u32;
                info.state = if code == FVE_E_LOCKED_VOLUME { VolumeState::Locked } else { VolumeState::NotReady };
                if code != ERROR_NOT_READY && code != FVE_E_LOCKED_VOLUME {
                    // Unknown refusal: treat as not ready, never guess "locked".
                    info.state = VolumeState::NotReady;
                }
            }
        }
        if info.state == VolumeState::Ready {
            let (mut free, mut total) = (0u64, 0u64);
            if unsafe { GetDiskFreeSpaceExW(PCWSTR(root.as_ptr()), None, Some(&mut total), Some(&mut free)) }.is_ok() {
                info.total_bytes = total;
                info.free_bytes = free;
            }
        }
        info.disk_number = volume_disk_number(letter);
        if let Some(n) = info.disk_number {
            if let Some(d) = identities.entry(n).or_insert_with(|| disk_identity(n)) {
                info.model = Some(d.model.clone()).filter(|m| !m.is_empty());
                info.media = d.media;
                info.bus = d.bus.clone();
            }
        }
        out.push(info);
    }
    Ok(out)
}

pub(crate) fn from_wide(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

/// Read-only health of one disk, from every source that answers. Never fails as a whole: what can't be read stays
/// `None` (and is named in `needs_admin` when admin would help). `allow_rw_open = false` (the read-only layer) never
/// opens the disk read+write, so SATA SMART is skipped and named like the non-admin case.
pub fn disk_health(number: u32, elevated: bool, allow_rw_open: bool) -> Result<HealthRaw> {
    let id = disk_identity(number).ok_or_else(|| StorageError::NotFound(format!("disk {number}")))?;
    let mut raw = HealthRaw::default();
    let path = format!("\\\\.\\PhysicalDrive{number}");

    // 1. Windows' own verdict (WMI, no admin).
    if let Ok(w) = Wmi::connect("root\\Microsoft\\Windows\\Storage") {
        if let Ok(rows) = w.query(&format!("SELECT DeviceId, HealthStatus FROM MSFT_PhysicalDisk WHERE DeviceId = '{number}'"), &["HealthStatus"]) {
            raw.os_status = rows.first().and_then(|r| r.get("HealthStatus")).and_then(|v| v.as_i64()).map(|s| match s {
                0 => OsHealthStatus::Healthy,
                1 => OsHealthStatus::Warning,
                2 => OsHealthStatus::Unhealthy,
                _ => OsHealthStatus::Unknown,
            });
        }
        // 4. Reliability counters (admin only).
        match w.query(
            &format!("SELECT DeviceId, Temperature, Wear, PowerOnHours FROM MSFT_StorageReliabilityCounter WHERE DeviceId = '{number}'"),
            &["Temperature", "Wear", "PowerOnHours"],
        ) {
            Ok(rows) => {
                if let Some(r) = rows.first() {
                    let n = |k: &str| r.get(k).and_then(|v| v.as_i64());
                    raw.reliability = Some(ReliabilityCounter {
                        temperature_c: n("Temperature").map(|v| v as u32),
                        wear_pct: n("Wear").map(|v| v as u32),
                        power_on_hours: n("PowerOnHours").map(|v| v as u64),
                    });
                }
            }
            Err(StorageError::NeedsAdmin(_)) => raw.needs_admin.push("Windows reliability counters".into()),
            Err(_) => {}
        }
    }

    if let Ok(h) = open(&path, 0) {
        // 2. Temperature property.
        let mut buf = [0u8; 512];
        if let Ok(n) = ioctl(&h, IOCTL_STORAGE_QUERY_PROPERTY, &property_query(52, &[]), &mut buf) {
            raw.temperature_c = parse_temperature_descriptor(&buf[..(n as usize).min(buf.len())]);
        }
        // 3a. NVMe health log (log page 0x02).
        if id.bus.as_deref() == Some("NVMe") {
            match nvme_health(&h) {
                Ok(log) => raw.nvme = Some(log),
                Err(e) if is_access_denied(&e) => raw.needs_admin.push("NVMe health log".into()),
                Err(_) => {}
            }
        }
    }

    // 3b. SATA / ATA SMART attributes — needs read+write access → admin.
    if matches!(id.bus.as_deref(), Some("SATA") | Some("ATA")) && !allow_rw_open {
        raw.needs_admin.push("SMART attributes (temperature, power-on hours, moved sectors)".into());
    } else if matches!(id.bus.as_deref(), Some("SATA") | Some("ATA")) {
        match open(&path, GENERIC_READ.0 | GENERIC_WRITE.0) {
            Ok(h) => {
                if let Ok(attrs) = sata_smart(&h) {
                    raw.smart = attrs;
                }
            }
            Err(e) if is_access_denied(&e) && !elevated => {
                raw.needs_admin.push("SMART attributes (temperature, power-on hours, moved sectors)".into())
            }
            Err(_) => {}
        }
    }
    Ok(raw)
}

fn is_access_denied(e: &windows::core::Error) -> bool {
    e.code().0 as u32 == 0x8007_0005
}

fn nvme_health(h: &Handle) -> windows::core::Result<NvmeHealthLog> {
    // STORAGE_PROTOCOL_SPECIFIC_DATA (40 bytes) then room for the 512-byte log.
    let mut spec = [0u8; 40];
    spec[0..4].copy_from_slice(&3u32.to_le_bytes()); // ProtocolTypeNvme
    spec[4..8].copy_from_slice(&2u32.to_le_bytes()); // NVMeDataTypeLogPage
    spec[8..12].copy_from_slice(&2u32.to_le_bytes()); // NVME_LOG_PAGE_HEALTH_INFO
    spec[12..16].copy_from_slice(&0u32.to_le_bytes()); // sub value
    spec[16..20].copy_from_slice(&40u32.to_le_bytes()); // ProtocolDataOffset = sizeof(STORAGE_PROTOCOL_SPECIFIC_DATA)
    spec[20..24].copy_from_slice(&512u32.to_le_bytes()); // ProtocolDataLength
    let mut extra = spec.to_vec();
    extra.extend_from_slice(&[0u8; 512]);
    let q = property_query(50, &extra); // StorageDeviceProtocolSpecificProperty
    let mut out = vec![0u8; q.len()];
    ioctl(h, IOCTL_STORAGE_QUERY_PROPERTY, &q, &mut out)?;
    parse_nvme_protocol_data(&out).ok_or_else(|| windows::core::Error::from_hresult(windows::core::HRESULT(0x8007_000Du32 as i32)))
}

/// `STORAGE_TEMPERATURE_DATA_DESCRIPTOR` → the first sensor's °C (InfoCount at byte 12, TemperatureInfo[0] at 24,
/// its Temperature at +2, signed). 0 or out-of-range values (−40 … 150) = not reported.
pub fn parse_temperature_descriptor(buf: &[u8]) -> Option<i32> {
    if buf.len() < 40 || u16_at(buf, 12) == 0 {
        return None;
    }
    let t = u16_at(buf, 24 + 2) as i16 as i32;
    (t > -40 && t < 150 && t != 0).then_some(t)
}

/// `STORAGE_PROTOCOL_DATA_DESCRIPTOR` (Version, Size, the 40-byte STORAGE_PROTOCOL_SPECIFIC_DATA) followed by the
/// NVMe SMART / health log at 8 + ProtocolDataOffset.
pub fn parse_nvme_protocol_data(out: &[u8]) -> Option<NvmeHealthLog> {
    if out.len() < 48 {
        return None;
    }
    let data_off = 8 + u32_at(out, 8 + 16) as usize;
    let len = u32_at(out, 8 + 20) as usize;
    if len < 176 || data_off + 176 > out.len() {
        return None;
    }
    parse_nvme_health_log(&out[data_off..])
}

/// The NVMe health log page (NVMe base spec, log 02h): byte 0 critical warning · 1–2 temperature (K) · 3 available
/// spare % · 4 its threshold · 5 percentage used · 128 power-on hours · 144 unsafe shutdowns · 160 media errors
/// (16-byte counters; the low 8 bytes are read).
pub fn parse_nvme_health_log(d: &[u8]) -> Option<NvmeHealthLog> {
    if d.len() < 176 {
        return None;
    }
    Some(NvmeHealthLog {
        critical_warning: d[0],
        temperature_kelvin: u16_at(d, 1),
        available_spare_pct: d[3],
        available_spare_threshold_pct: d[4],
        percentage_used: d[5],
        power_on_hours: u64_at(d, 128),
        unsafe_shutdowns: u64_at(d, 144),
        media_errors: u64_at(d, 160),
    })
}

/// SMART READ DATA (0xD0) + READ THRESHOLDS (0xD1) through `SMART_RCV_DRIVE_DATA`.
fn sata_smart(h: &Handle) -> windows::core::Result<Vec<SmartAttribute>> {
    let read = |feature: u8| -> windows::core::Result<Vec<u8>> {
        // SENDCMDINPARAMS (packed): cBufferSize u32 · IDEREGS (8) · bDriveNumber · 3 reserved · 4×u32 reserved.
        let mut input = [0u8; 32];
        input[0..4].copy_from_slice(&512u32.to_le_bytes());
        input[4..12].copy_from_slice(&[feature, 1, 1, 0x4F, 0xC2, 0xA0, 0xB0, 0]); // SMART_CMD, cylinder 0xC24F
        let mut out = vec![0u8; 16 + 512]; // SENDCMDOUTPARAMS: cBufferSize + DRIVERSTATUS (12) + 512 bytes
        ioctl(h, SMART_RCV_DRIVE_DATA, &input, &mut out)?;
        Ok(out[16..].to_vec())
    };
    let data = read(0xD0)?;
    let thresholds = read(0xD1).ok();
    Ok(parse_smart(&data, thresholds.as_deref()))
}

/// Parse the 30 × 12-byte SMART attribute table (starts at byte 2).
pub fn parse_smart(data: &[u8], thresholds: Option<&[u8]>) -> Vec<SmartAttribute> {
    let mut out = Vec::new();
    for i in 0..30 {
        let o = 2 + i * 12;
        if o + 12 > data.len() {
            break;
        }
        let id = data[o];
        if id == 0 {
            continue;
        }
        let mut raw = [0u8; 8];
        raw[..6].copy_from_slice(&data[o + 5..o + 11]);
        let threshold = thresholds.and_then(|t| {
            (0..30).map(|j| 2 + j * 12).filter(|&p| p + 2 <= t.len()).find(|&p| t[p] == id).map(|p| t[p + 1])
        });
        out.push(SmartAttribute { id, value: data[o + 3], worst: data[o + 4], raw: u64::from_le_bytes(raw), threshold });
    }
    out
}

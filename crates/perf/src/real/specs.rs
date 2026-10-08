//! "Your PC" for the real OS, read once (DESIGN §3.8 "How (guesses)", verified here):
//! WMI `Win32_Processor`, `Win32_VideoController` (driver) + DXGI (real VRAM), `Win32_PhysicalMemory`,
//! `Win32_BaseBoard`, `Win32_BIOS`, `MSFT_PhysicalDisk`; Windows from the registry (`CurrentVersion`);
//! displays from `QueryDisplayConfig` + the monitor's own name.

use super::wmi::{Value, Wmi};
use crate::specs::{memory_type_name, xmp_from_speeds, CpuSpec, DisplaySpec, DriveSpec, GpuSpec, NetSpec, PcSpecs, RamSpec, WindowsSpec};
use crate::Result;
use windows::core::PCWSTR;
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_TARGET_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn s(row: &super::wmi::Row, k: &str) -> Option<String> {
    row.get(k).and_then(Value::as_str).map(|x| x.trim().to_string()).filter(|x| !x.is_empty())
}
fn n(row: &super::wmi::Row, k: &str) -> Option<i64> {
    row.get(k).and_then(Value::as_i64)
}

const CV: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion";

fn reg_sz(key: &str, value: &str) -> Option<String> {
    let (k, v) = (wide(key), wide(value));
    let mut buf = vec![0u16; 512];
    let mut len = (buf.len() * 2) as u32;
    unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, PCWSTR(k.as_ptr()), PCWSTR(v.as_ptr()), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len)) }
        .is_ok()
        .then(|| String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(0)]))
        .filter(|x| !x.is_empty())
}

fn reg_dword(key: &str, value: &str) -> Option<u32> {
    let (k, v) = (wide(key), wide(value));
    let mut d = 0u32;
    let mut len = 4u32;
    unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, PCWSTR(k.as_ptr()), PCWSTR(v.as_ptr()), RRF_RT_REG_DWORD, None, Some(&mut d as *mut _ as *mut _), Some(&mut len)) }
        .is_ok()
        .then_some(d)
}

/// "2024-05-17" from a WMI datetime ("20240517000000.000000+000").
fn wmi_date(s: &str) -> Option<String> {
    (s.len() >= 8 && s[..8].bytes().all(|b| b.is_ascii_digit())).then(|| format!("{}-{}-{}", &s[..4], &s[4..6], &s[6..8]))
}

/// "2025-03-02" from Unix seconds.
pub fn unix_date(secs: u64) -> String {
    // Civil-from-days (Howard Hinnant), UTC.
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn read() -> Result<PcSpecs> {
    let mut out = PcSpecs::default();
    if let Ok(w) = Wmi::connect("root\\cimv2") {
        if let Some(r) = w
            .query(
                "SELECT Name, NumberOfCores, NumberOfEnabledCore, NumberOfLogicalProcessors, ThreadCount, MaxClockSpeed FROM Win32_Processor",
                &["Name", "NumberOfCores", "NumberOfEnabledCore", "NumberOfLogicalProcessors", "ThreadCount", "MaxClockSpeed"],
            )
            .ok()
            .and_then(|v| v.into_iter().next())
        {
            let in_use = n(&r, "NumberOfLogicalProcessors").map(|x| x as u32);
            out.cpu = CpuSpec {
                name: s(&r, "Name").unwrap_or_default(),
                cores: n(&r, "NumberOfEnabledCore").or_else(|| n(&r, "NumberOfCores")).map(|x| x as u32),
                threads: n(&r, "ThreadCount").map(|x| x as u32).or(in_use),
                threads_in_use: in_use,
                max_mhz: n(&r, "MaxClockSpeed").map(|x| x as u32),
            };
        }
        let drivers: Vec<(String, String)> = w
            .query("SELECT Name, DriverVersion FROM Win32_VideoController", &["Name", "DriverVersion"])
            .unwrap_or_default()
            .iter()
            .filter_map(|r| Some((s(r, "Name")?, s(r, "DriverVersion")?)))
            .collect();
        out.gpus = dxgi_gpus()
            .into_iter()
            .map(|(name, vram, integrated)| GpuSpec {
                driver: drivers.iter().find(|(n, _)| *n == name).map(|(_, d)| d.clone()),
                name,
                vram_bytes: Some(vram),
                integrated,
            })
            .collect();
        let sticks = w
            .query(
                "SELECT Capacity, Speed, ConfiguredClockSpeed, SMBIOSMemoryType FROM Win32_PhysicalMemory",
                &["Capacity", "Speed", "ConfiguredClockSpeed", "SMBIOSMemoryType"],
            )
            .unwrap_or_default();
        let configured = sticks.iter().filter_map(|r| n(r, "ConfiguredClockSpeed")).filter(|&x| x > 0).min().map(|x| x as u32);
        let rated = sticks.iter().filter_map(|r| n(r, "Speed")).filter(|&x| x > 0).min().map(|x| x as u32);
        out.ram = RamSpec {
            total_bytes: sticks.iter().filter_map(|r| n(r, "Capacity")).map(|x| x as u64).sum(),
            kind: sticks.iter().filter_map(|r| n(r, "SMBIOSMemoryType")).find_map(|t| memory_type_name(t as u32)).map(str::to_string),
            speed_mts: configured,
            rated_mts: rated,
            sticks: sticks.len() as u32,
            xmp_expo: xmp_from_speeds(configured, rated),
        };
        if let Some(r) = w.query("SELECT Manufacturer, Product FROM Win32_BaseBoard", &["Manufacturer", "Product"]).ok().and_then(|v| v.into_iter().next()) {
            out.board.maker = s(&r, "Manufacturer");
            out.board.model = s(&r, "Product");
        }
        if let Some(r) = w.query("SELECT SMBIOSBIOSVersion, ReleaseDate FROM Win32_BIOS", &["SMBIOSBIOSVersion", "ReleaseDate"]).ok().and_then(|v| v.into_iter().next()) {
            out.board.bios_version = s(&r, "SMBIOSBIOSVersion");
            out.board.bios_date = s(&r, "ReleaseDate").and_then(|d| wmi_date(&d));
        }
    }
    if let Ok(w) = Wmi::connect("root\\Microsoft\\Windows\\Storage") {
        let mut drives: Vec<(i64, DriveSpec)> = w
            .query("SELECT DeviceId, FriendlyName, Size, MediaType, BusType FROM MSFT_PhysicalDisk", &["DeviceId", "FriendlyName", "Size", "MediaType", "BusType"])
            .unwrap_or_default()
            .iter()
            .map(|r| {
                (
                    n(r, "DeviceId").unwrap_or(99),
                    DriveSpec {
                        model: s(r, "FriendlyName").unwrap_or_default(),
                        size_bytes: n(r, "Size").unwrap_or(0) as u64,
                        media: match n(r, "MediaType") {
                            Some(3) => Some("HDD".into()),
                            Some(4) => Some("SSD".into()),
                            _ => None,
                        },
                        bus: match n(r, "BusType") {
                            Some(17) => Some("NVMe".into()),
                            Some(11) => Some("SATA".into()),
                            Some(7) => Some("USB".into()),
                            Some(10) => Some("SAS".into()),
                            _ => None,
                        },
                    },
                )
            })
            .collect();
        drives.sort_by_key(|d| d.0);
        out.drives = drives.into_iter().map(|d| d.1).collect();
    }
    out.displays = displays();
    out.nets = nets();
    let build: Option<u32> = reg_sz(CV, "CurrentBuild").and_then(|b| b.parse().ok());
    let mut edition = reg_sz(CV, "ProductName").unwrap_or_else(|| "Windows".into());
    // Windows 11 still says "Windows 10" in ProductName; build 22000+ is Windows 11.
    if build.is_some_and(|b| b >= 22_000) {
        edition = edition.replacen("Windows 10", "Windows 11", 1);
    }
    out.windows = WindowsSpec {
        edition,
        version: reg_sz(CV, "DisplayVersion"),
        build: build.map(|b| match reg_dword(CV, "UBR") {
            Some(u) => format!("{b}.{u}"),
            None => b.to_string(),
        }),
        install_date: reg_dword(CV, "InstallDate").map(|t| unix_date(t as u64)),
    };
    Ok(out)
}

/// The physical network adapters (Order 021, the drawing's "Network" fact): WMI `Win32_NetworkAdapter` with
/// `PhysicalAdapter = TRUE` (virtual / VPN adapters are left out); `NetConnectionStatus` 2 = connected, `Speed` = the link in
/// bits per second, `AdapterTypeId` 9 = wireless (or the name says Wi-Fi / Wireless).
fn nets() -> Vec<NetSpec> {
    let Ok(w) = Wmi::connect("root\\cimv2") else { return Vec::new() };
    w.query(
        "SELECT Name, Speed, NetConnectionStatus, AdapterTypeId, PNPDeviceID FROM Win32_NetworkAdapter WHERE PhysicalAdapter = TRUE",
        &["Name", "Speed", "NetConnectionStatus", "AdapterTypeId", "PNPDeviceID"],
    )
    .unwrap_or_default()
    .iter()
    .filter_map(|r| {
        let name = s(r, "Name")?;
        let low = name.to_lowercase();
        // only real hardware: a card on the PCI / USB bus. WMI also calls VPN / TAP drivers (ROOT\NET\..., measured on this
        // PC: NordLynx, OpenVPN, TAP-NordVPN) and Bluetooth's PAN adapter "physical"
        let pnp = s(r, "PNPDeviceID").unwrap_or_default().to_uppercase();
        if !(pnp.starts_with("PCI\\") || pnp.starts_with("USB\\")) || low.contains("bluetooth") {
            return None;
        }
        let connected = n(r, "NetConnectionStatus") == Some(2);
        Some(NetSpec {
            wireless: n(r, "AdapterTypeId") == Some(9) || low.contains("wi-fi") || low.contains("wireless") || low.contains("wlan"),
            speed_bps: if connected { n(r, "Speed").filter(|&b| b > 0 && b < i64::MAX).map(|b| b as u64) } else { None },
            connected,
            name,
        })
    })
    .collect()
}

/// (name, dedicated VRAM, integrated) for every hardware GPU.
fn dxgi_gpus() -> Vec<(String, u64, bool)> {
    let mut out = Vec::new();
    let Ok(f) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else { return out };
    let mut luids = Vec::new();
    for i in 0.. {
        let Ok(a) = (unsafe { f.EnumAdapters1(i) }) else { break };
        let Ok(d) = (unsafe { a.GetDesc1() }) else { continue };
        if d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }
        let l = (d.AdapterLuid.HighPart, d.AdapterLuid.LowPart);
        if luids.contains(&l) {
            continue;
        }
        luids.push(l);
        let name = String::from_utf16_lossy(&d.Description[..d.Description.iter().position(|&c| c == 0).unwrap_or(128)]);
        let vram = d.DedicatedVideoMemory as u64;
        out.push((name, vram, vram <= 512 * 1024 * 1024));
    }
    out
}

fn displays() -> Vec<DisplaySpec> {
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm).is_err() {
            return Vec::new();
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        if QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None).is_err() {
            return Vec::new();
        }
        paths.truncate(np as usize);
        let mut out = Vec::new();
        for p in &paths {
            let mut name = DISPLAYCONFIG_TARGET_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                size: std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
                adapterId: p.targetInfo.adapterId,
                id: p.targetInfo.id,
            },
                ..Default::default()
            };
            let friendly = if DisplayConfigGetDeviceInfo(&mut name.header) == 0 {
                let f = &name.monitorFriendlyDeviceName;
                String::from_utf16_lossy(&f[..f.iter().position(|&c| c == 0).unwrap_or(f.len())])
            } else {
                String::new()
            };
            let idx = p.sourceInfo.Anonymous.modeInfoIdx as usize;
            let (w, h) = match modes.get(idx) {
                Some(m) if m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE => (m.Anonymous.sourceMode.width, m.Anonymous.sourceMode.height),
                _ => (0, 0),
            };
            let rr = p.targetInfo.refreshRate;
            let hz = if rr.Denominator == 0 { 0.0 } else { rr.Numerator as f64 / rr.Denominator as f64 };
            out.push(DisplaySpec { name: if friendly.is_empty() { "Display".into() } else { friendly }, width: w, height: h, hz });
        }
        out
    }
}

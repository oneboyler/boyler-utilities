//! The live source for the real OS. Opened when the Performance page opens, dropped when it closes.
//!
//! * CPU usage `\Processor Information(_Total)\% Processor Utility`; clock = `Processor Frequency` (base MHz) ×
//!   `% Processor Performance` ÷ 100 — what Task Manager does (research v3 §3). English counter names
//!   (`PdhAddEnglishCounterW`) so a Croatian / any-language Windows works.
//! * GPU usage `\GPU Engine(*)\Utilization Percentage` (busiest engine, any vendor), VRAM used
//!   `\GPU Adapter Memory(*)\Dedicated Usage`, VRAM total + names from DXGI.
//! * GPU temperature + fan RPM: `D3DKMTQueryAdapterInfo(KMTQAITYPE_ADAPTERPERFDATA)` — the WDDM 2.4+ data Task
//!   Manager shows; any vendor, no driver of ours. Fan % on NVIDIA from NVML (`nvml.dll`, ships with the driver),
//!   loaded only while the page is open.
//! * RAM `GlobalMemoryStatusEx`. Disk `\PhysicalDisk(*)\% Idle Time` (active = 100 − idle) + `Disk Bytes/sec`.
//! * Network: `GetIfTable2` byte counters of real adapters (hardware, not filters, up), difference per second.

use super::nvml::Nvml;
use crate::live::{aggregate_gpu, parse_disk_instance, parse_luid};
use crate::{DiskReading, GpuReading, LiveReading, LiveSource, PerfError, Result};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Wdk::Graphics::Direct3D::{
    D3DKMTCloseAdapter, D3DKMTOpenAdapterFromLuid, D3DKMTQueryAdapterInfo, D3DKMT_ADAPTER_PERFDATA, D3DKMT_CLOSEADAPTER,
    D3DKMT_OPENADAPTERFROMLUID, D3DKMT_QUERYADAPTERINFO, KMTQAITYPE_ADAPTERPERFDATA,
};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhGetFormattedCounterValue,
    PdhOpenQueryW, PDH_FMT, PDH_FMT_COUNTERVALUE, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
    PDH_MORE_DATA,
};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

const PDH_FMT_NOCAP100: u32 = 0x8000;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

struct Adapter {
    luid: (u32, u32),
    name: String,
    vram_total: u64,
    integrated: bool,
    kmt: Option<u32>,
    nvml_index: Option<u32>,
}

pub struct RealLive {
    query: PDH_HQUERY,
    cpu_util: PDH_HCOUNTER,
    cpu_perf: PDH_HCOUNTER,
    cpu_freq: PDH_HCOUNTER,
    gpu_engine: Option<PDH_HCOUNTER>,
    gpu_mem: Option<PDH_HCOUNTER>,
    disk_idle: PDH_HCOUNTER,
    disk_bytes: PDH_HCOUNTER,
    last_collect: Instant,
    adapters: Vec<Adapter>,
    nvml: Option<Nvml>,
    net_prev: Option<(u64, u64, Instant)>,
}

// The PDH / D3DKMT handles are plain numbers used from one thread at a time (the sampler thread).
unsafe impl Send for RealLive {}

fn pdh(context: &str, status: u32) -> Result<()> {
    if status == 0 {
        Ok(())
    } else {
        Err(PerfError::Os { context: context.to_string(), code: status })
    }
}

impl RealLive {
    pub fn open() -> Result<RealLive> {
        unsafe {
            let mut query = PDH_HQUERY::default();
            pdh("PdhOpenQuery", PdhOpenQueryW(PCWSTR::null(), 0, &mut query))?;
            let add = |path: &str| -> Result<PDH_HCOUNTER> {
                let mut c = PDH_HCOUNTER::default();
                let w = wide(path);
                pdh(&format!("PdhAddEnglishCounter {path}"), PdhAddEnglishCounterW(query, PCWSTR(w.as_ptr()), 0, &mut c))?;
                Ok(c)
            };
            let live = RealLive {
                cpu_util: add("\\Processor Information(_Total)\\% Processor Utility")?,
                cpu_perf: add("\\Processor Information(_Total)\\% Processor Performance")?,
                cpu_freq: add("\\Processor Information(_Total)\\Processor Frequency")?,
                // GPU counters exist only with a WDDM 2.x GPU; carry on without them.
                gpu_engine: add("\\GPU Engine(*)\\Utilization Percentage").ok(),
                gpu_mem: add("\\GPU Adapter Memory(*)\\Dedicated Usage").ok(),
                disk_idle: add("\\PhysicalDisk(*)\\% Idle Time")?,
                disk_bytes: add("\\PhysicalDisk(*)\\Disk Bytes/sec")?,
                query,
                last_collect: Instant::now(),
                adapters: Vec::new(),
                nvml: Nvml::load(),
                net_prev: None,
            };
            let _ = PdhCollectQueryData(query); // prime the rate counters
            let mut live = live;
            live.adapters = adapters(live.nvml.as_ref());
            live.net_prev = net_octets().map(|(i, o)| (i, o, Instant::now()));
            Ok(live)
        }
    }

    fn value(&self, c: PDH_HCOUNTER) -> f64 {
        unsafe {
            let mut v = PDH_FMT_COUNTERVALUE::default();
            if PdhGetFormattedCounterValue(c, PDH_FMT(PDH_FMT_DOUBLE.0 | PDH_FMT_NOCAP100), None, &mut v) == 0 {
                v.Anonymous.doubleValue
            } else {
                0.0
            }
        }
    }

    fn array(&self, c: PDH_HCOUNTER) -> Vec<(String, f64)> {
        unsafe {
            let fmt = PDH_FMT(PDH_FMT_DOUBLE.0 | PDH_FMT_NOCAP100);
            let (mut size, mut count) = (0u32, 0u32);
            if PdhGetFormattedCounterArrayW(c, fmt, &mut size, &mut count, None) != PDH_MORE_DATA || size == 0 {
                return Vec::new();
            }
            // u64-aligned buffer of `size` bytes.
            let mut buf = vec![0u64; (size as usize).div_ceil(8)];
            let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            if PdhGetFormattedCounterArrayW(c, fmt, &mut size, &mut count, Some(items)) != 0 {
                return Vec::new();
            }
            (0..count as usize)
                .filter_map(|i| {
                    let it = &*items.add(i);
                    if it.FmtValue.CStatus > 1 {
                        return None; // PDH_CSTATUS_VALID_DATA = 0, NEW_DATA = 1
                    }
                    Some((it.szName.to_string().unwrap_or_default(), it.FmtValue.Anonymous.doubleValue))
                })
                .collect()
        }
    }
}

impl LiveSource for RealLive {
    fn read(&mut self) -> Result<LiveReading> {
        // Rate counters need two collects at least a little apart (only matters right after open).
        let since = self.last_collect.elapsed();
        if since < Duration::from_millis(150) {
            std::thread::sleep(Duration::from_millis(150) - since);
        }
        unsafe {
            pdh("PdhCollectQueryData", PdhCollectQueryData(self.query))?;
        }
        self.last_collect = Instant::now();
        let mut r = LiveReading {
            cpu_usage_pct: self.value(self.cpu_util).clamp(0.0, 100.0),
            cpu_mhz: self.value(self.cpu_freq) * self.value(self.cpu_perf) / 100.0,
            ..Default::default()
        };
        // GPU
        let (by_adapter, by_pid) = match self.gpu_engine {
            Some(c) => aggregate_gpu(&self.array(c)),
            None => Default::default(),
        };
        r.gpu_by_pid = by_pid;
        let mut mem: HashMap<(u32, u32), f64> = HashMap::new();
        if let Some(c) = self.gpu_mem {
            for (name, v) in self.array(c) {
                if let Some(l) = parse_luid(&name) {
                    *mem.entry(l).or_default() += v;
                }
            }
        }
        for a in &self.adapters {
            let mut g = GpuReading {
                name: a.name.clone(),
                usage_pct: by_adapter.get(&a.luid).copied().unwrap_or(0.0),
                vram_used_bytes: mem.get(&a.luid).copied().unwrap_or(0.0) as u64,
                vram_total_bytes: a.vram_total,
                integrated: a.integrated,
                ..Default::default()
            };
            if let Some(h) = a.kmt {
                if let Some(p) = perf_data(h) {
                    // Temperature is in tenths of a degree Celsius.
                    if p.Temperature > 0 {
                        g.temperature_c = Some(p.Temperature as f64 / 10.0);
                    }
                    if p.FanRPM > 0 {
                        g.fan_rpm = Some(p.FanRPM);
                    }
                }
            }
            if let (Some(n), Some(i)) = (&self.nvml, a.nvml_index) {
                g.fan_pct = n.fan_pct(i);
                if g.temperature_c.is_none() {
                    g.temperature_c = n.temperature(i).map(|t| t as f64);
                }
            }
            r.gpus.push(g);
        }
        // RAM
        let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        if unsafe { GlobalMemoryStatusEx(&mut m) }.is_ok() {
            r.ram_total_bytes = m.ullTotalPhys;
            r.ram_used_bytes = m.ullTotalPhys.saturating_sub(m.ullAvailPhys);
        }
        // Disks
        let idle: HashMap<String, f64> = self.array(self.disk_idle).into_iter().collect();
        for (name, bps) in self.array(self.disk_bytes) {
            if let Some(letters) = parse_disk_instance(&name) {
                let active = 100.0 - idle.get(&name).copied().unwrap_or(100.0).clamp(0.0, 100.0);
                r.disks.push(DiskReading { instance: name, letters, active_pct: active, bytes_per_sec: bps });
            }
        }
        r.disks.sort_by(|a, b| a.instance.cmp(&b.instance));
        r.system_free_bytes = system_free();
        // Network
        if let Some((i, o)) = net_octets() {
            let now = Instant::now();
            if let Some((pi, po, pt)) = self.net_prev {
                let dt = now.duration_since(pt).as_secs_f64().max(0.001);
                r.net_down_bps = i.saturating_sub(pi) as f64 * 8.0 / dt;
                r.net_up_bps = o.saturating_sub(po) as f64 * 8.0 / dt;
            }
            self.net_prev = Some((i, o, now));
        }
        Ok(r)
    }
}

impl Drop for RealLive {
    fn drop(&mut self) {
        unsafe {
            let _ = PdhCloseQuery(self.query);
            for a in &self.adapters {
                if let Some(h) = a.kmt {
                    let _ = D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER { hAdapter: h });
                }
            }
        }
    }
}

/// Hardware GPUs from DXGI (software adapters skipped), each with a kernel handle for temperature.
fn adapters(nvml: Option<&Nvml>) -> Vec<Adapter> {
    let mut out = Vec::new();
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else { return out };
    let nvml_names = nvml.map(|n| n.names()).unwrap_or_default();
    let mut used_nvml: Vec<u32> = Vec::new();
    for i in 0.. {
        let Ok(a) = (unsafe { factory.EnumAdapters1(i) }) else { break };
        let Ok(d) = (unsafe { a.GetDesc1() }) else { continue };
        if d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }
        let luid = (d.AdapterLuid.HighPart as u32, d.AdapterLuid.LowPart);
        if out.iter().any(|x: &Adapter| x.luid == luid) {
            continue;
        }
        let name = String::from_utf16_lossy(&d.Description[..d.Description.iter().position(|&c| c == 0).unwrap_or(128)]);
        let mut open = D3DKMT_OPENADAPTERFROMLUID { AdapterLuid: d.AdapterLuid, hAdapter: 0 };
        let kmt = (unsafe { D3DKMTOpenAdapterFromLuid(&mut open) }.0 >= 0).then_some(open.hAdapter);
        let nvml_index = nvml_names
            .iter()
            .enumerate()
            .find(|(j, n)| **n == name && !used_nvml.contains(&(*j as u32)))
            .map(|(j, _)| j as u32);
        if let Some(j) = nvml_index {
            used_nvml.push(j);
        }
        out.push(Adapter {
            luid,
            name,
            vram_total: d.DedicatedVideoMemory as u64,
            // An integrated GPU has (almost) no memory of its own (≤ 512 MB carve-out) and shares system RAM.
            integrated: (d.DedicatedVideoMemory as u64) <= 512 * 1024 * 1024,
            kmt,
            nvml_index,
        });
    }
    out
}

fn perf_data(h: u32) -> Option<D3DKMT_ADAPTER_PERFDATA> {
    let mut p = D3DKMT_ADAPTER_PERFDATA::default();
    let mut q = D3DKMT_QUERYADAPTERINFO {
        hAdapter: h,
        Type: KMTQAITYPE_ADAPTERPERFDATA,
        pPrivateDriverData: &mut p as *mut _ as *mut _,
        PrivateDriverDataSize: std::mem::size_of::<D3DKMT_ADAPTER_PERFDATA>() as u32,
    };
    (unsafe { D3DKMTQueryAdapterInfo(&mut q) }.0 >= 0).then_some(p)
}

/// Total bytes in / out over real adapters: hardware, not a filter layer, up, not loopback.
fn net_octets() -> Option<(u64, u64)> {
    unsafe {
        let mut t: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut t).is_err() || t.is_null() {
            return None;
        }
        let n = (*t).NumEntries as usize;
        let rows = std::slice::from_raw_parts((*t).Table.as_ptr(), n);
        let (mut i, mut o) = (0u64, 0u64);
        for r in rows {
            let flags = r.InterfaceAndOperStatusFlags._bitfield;
            let hardware = flags & 0x01 != 0;
            let filter = flags & 0x02 != 0;
            if hardware && !filter && r.OperStatus.0 == 1 && r.Type != 24 {
                i += r.InOctets;
                o += r.OutOctets;
            }
        }
        FreeMibTable(t as *const _);
        Some((i, o))
    }
}

/// Free bytes on the Windows drive (`%SystemDrive%`, else C:) - `GetDiskFreeSpaceExW`, a cheap call (Order 021).
fn system_free() -> Option<u64> {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let root: Vec<u16> = format!("{}\\", drive.trim_end_matches('\\')).encode_utf16().chain(Some(0)).collect();
    let mut free = 0u64;
    unsafe { windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(PCWSTR(root.as_ptr()), Some(&mut free), None, None) }.ok().map(|_| free)
}

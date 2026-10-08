//! Live stats only while asked (DESIGN §3.8 tiles). The menu calls [`Sampler::start`] when the Performance page
//! opens and [`Sampler::stop`] (or drops it) when it closes. While stopped there is **no thread and no open counter**
//! — zero cost. While running, one thread reads every second and keeps the last 40 s for the sparklines; between
//! ticks it sleeps on a condition variable (stop wakes it at once).

use crate::{GpuReading, LiveReading, PerfError, PerfOs, Result};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// CPU temperature is not read: there is no reliable driverless way, and the drivers that can (WinRing0, PawnIO)
/// are flagged by Defender / blocked by anti-cheats (research v3 §3). UNSURE with the owner — the tile shows "— °C".
pub const CPU_TEMPERATURE_UNAVAILABLE: &str = "Needs a hardware sensor driver — see research";

/// Always `Unavailable` (see [`CPU_TEMPERATURE_UNAVAILABLE`]).
pub fn cpu_temperature() -> Result<f64> {
    Err(PerfError::Unavailable(CPU_TEMPERATURE_UNAVAILABLE.to_string()))
}

#[derive(Debug, Clone, Copy)]
pub struct SamplerOptions {
    /// DESIGN: every 1 s.
    pub interval: Duration,
    /// DESIGN: the last 40 s → 40 ticks.
    pub history_len: usize,
}

impl Default for SamplerOptions {
    fn default() -> Self {
        SamplerOptions { interval: Duration::from_secs(1), history_len: 40 }
    }
}

/// One reading with its time since the sampler started.
#[derive(Debug, Clone, PartialEq)]
pub struct Tick {
    pub since_start: Duration,
    pub reading: LiveReading,
}

struct Inner {
    stop: Mutex<bool>,
    wake: Condvar,
    history: Mutex<VecDeque<Tick>>,
    last_error: Mutex<Option<String>>,
    reads: AtomicU64,
}

/// The running sampler. Not running = this value doesn't exist.
pub struct Sampler {
    inner: Arc<Inner>,
    thread: Option<JoinHandle<()>>,
}

impl Sampler {
    /// Open the counters and start reading. The first reading is taken before this returns, so the tiles have
    /// numbers at once. (DESIGN wants the sparkline "already full when the page opens"; with zero cost while closed
    /// there is no history to show — the line fills over the first 40 s. Noted in the report.)
    pub fn start(os: Arc<dyn PerfOs>, opts: SamplerOptions) -> Result<Sampler> {
        let mut src = os.open_live()?;
        let started = Instant::now();
        let first = src.read()?;
        let inner = Arc::new(Inner {
            stop: Mutex::new(false),
            wake: Condvar::new(),
            history: Mutex::new(VecDeque::with_capacity(opts.history_len + 1)),
            last_error: Mutex::new(None),
            reads: AtomicU64::new(1),
        });
        inner.history.lock().unwrap_or_else(|p| p.into_inner()).push_back(Tick { since_start: Duration::ZERO, reading: first });
        let th = inner.clone();
        let thread = std::thread::Builder::new()
            .name("bu-perf-sampler".into())
            .spawn(move || {
                loop {
                    {
                        let stop = th.stop.lock().unwrap_or_else(|p| p.into_inner());
                        let (stop, _) = th.wake.wait_timeout_while(stop, opts.interval, |s| !*s).unwrap_or_else(|p| p.into_inner());
                        if *stop {
                            break;
                        }
                    }
                    th.reads.fetch_add(1, Ordering::Relaxed);
                    match src.read() {
                        Ok(r) => {
                            let mut h = th.history.lock().unwrap_or_else(|p| p.into_inner());
                            h.push_back(Tick { since_start: started.elapsed(), reading: r });
                            while h.len() > opts.history_len {
                                h.pop_front();
                            }
                        }
                        Err(e) => *th.last_error.lock().unwrap_or_else(|p| p.into_inner()) = Some(e.to_string()),
                    }
                }
                drop(src); // close counters / libraries on the sampler thread
            })
            .map_err(|e| PerfError::Os { context: format!("sampler thread: {e}"), code: 0 })?;
        Ok(Sampler { inner, thread: Some(thread) })
    }

    /// Stop reading and release everything. Returns once the thread has ended.
    pub fn stop(mut self) {
        self.halt();
    }

    fn halt(&mut self) {
        *self.inner.stop.lock().unwrap_or_else(|p| p.into_inner()) = true;
        self.inner.wake.notify_all();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    pub fn latest(&self) -> Option<Tick> {
        self.inner.history.lock().unwrap_or_else(|p| p.into_inner()).back().cloned()
    }
    /// Oldest first, at most `history_len` ticks.
    pub fn history(&self) -> Vec<Tick> {
        self.inner.history.lock().unwrap_or_else(|p| p.into_inner()).iter().cloned().collect()
    }
    /// How many readings were taken (tests use it to prove nothing runs after stop).
    pub fn reads(&self) -> u64 {
        self.inner.reads.load(Ordering::Relaxed)
    }
    pub fn last_error(&self) -> Option<String> {
        self.inner.last_error.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.halt();
    }
}

/// The GPU the big tile shows: the card with the most VRAM (an integrated GPU only if there is no card).
pub fn main_gpu(r: &LiveReading) -> Option<&GpuReading> {
    r.gpus.iter().max_by_key(|g| (!g.integrated, g.vram_total_bytes))
}

/// The disk tile: the disk holding C: (the system drive), else the first.
pub fn main_disk(r: &LiveReading, system_letter: char) -> Option<&crate::DiskReading> {
    r.disks.iter().find(|d| d.letters.contains(&system_letter)).or(r.disks.first())
}

/// GPU temperature pill (DESIGN: amber ≥ 75 °C, red ≥ 85 °C).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TempLevel {
    Ok,
    Warm,
    Hot,
}

pub fn temp_level(c: f64) -> TempLevel {
    if c >= 85.0 {
        TempLevel::Hot
    } else if c >= 75.0 {
        TempLevel::Warm
    } else {
        TempLevel::Ok
    }
}

/// "4.60 GHz".
pub fn format_ghz(mhz: f64) -> String {
    format!("{:.2} GHz", mhz / 1000.0)
}

/// "9.6 Mb/s" (megabits, decimal, like the drawing and Task Manager).
pub fn format_mbps(bits_per_sec: f64) -> String {
    let m = bits_per_sec / 1e6;
    if m >= 100.0 {
        format!("{m:.0} Mb/s")
    } else if m >= 0.1 || m == 0.0 {
        format!("{m:.1} Mb/s")
    } else {
        format!("{:.0} Kb/s", bits_per_sec / 1e3)
    }
}

/// "6 MB/s" disk throughput (decimal MB).
pub fn format_mb_per_s(bytes_per_sec: f64) -> String {
    let m = bytes_per_sec / 1e6;
    if m >= 10.0 || m == 0.0 {
        format!("{m:.0} MB/s")
    } else {
        format!("{m:.1} MB/s")
    }
}

/// RAM tile: big value "12.4 GB" and extra line "of 32 · 39 %" (GB = 1024³, like Task Manager).
pub fn ram_text(used: u64, total: u64) -> (String, String) {
    let gb = |b: u64| b as f64 / (1u64 << 30) as f64;
    let pct = if total == 0 { 0.0 } else { used as f64 * 100.0 / total as f64 };
    (format!("{:.1} GB", gb(used)), format!("of {:.0} · {:.0} %", gb(total), pct))
}

/// GPU extra line: "VRAM 5.2 / 12 GB · Fan 30 %" (fan part only when known).
pub fn gpu_extra(g: &GpuReading) -> String {
    let gb = |b: u64| b as f64 / (1u64 << 30) as f64;
    let mut s = format!("VRAM {:.1} / {:.0} GB", gb(g.vram_used_bytes), gb(g.vram_total_bytes));
    if let Some(f) = g.fan_pct {
        s.push_str(&format!(" · Fan {f} %"));
    } else if let Some(rpm) = g.fan_rpm.filter(|&r| r > 0) {
        s.push_str(&format!(" · Fan {rpm} rpm"));
    }
    s
}

/// One `GPU Engine` counter instance, e.g. `pid_1234_luid_0x00000000_0x0000D1F2_phys_0_eng_0_engtype_3D`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GpuEngineKey {
    pub pid: u32,
    /// Adapter LUID as (high, low).
    pub luid: (u32, u32),
    pub phys: u32,
    pub eng: u32,
}

fn hex(s: &str) -> Option<u32> {
    u32::from_str_radix(s.trim_start_matches("0x").trim_start_matches("0X"), 16).ok()
}

/// Parse a `GPU Engine` instance name.
pub fn parse_gpu_engine(name: &str) -> Option<GpuEngineKey> {
    let parts: Vec<&str> = name.split('_').collect();
    let at = |key: &str| parts.iter().position(|p| *p == key);
    let pid = parts.get(at("pid")? + 1)?.parse().ok()?;
    let l = at("luid")?;
    let luid = (hex(parts.get(l + 1)?)?, hex(parts.get(l + 2)?)?);
    let phys = parts.get(at("phys")? + 1)?.parse().ok()?;
    let eng = parts.get(at("eng")? + 1)?.parse().ok()?;
    Some(GpuEngineKey { pid, luid, phys, eng })
}

/// Parse a `GPU Adapter Memory` / `GPU Process Memory` instance name's LUID (`luid_0x00000000_0x0000D1F2_phys_0`).
pub fn parse_luid(name: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = name.split('_').collect();
    let l = parts.iter().position(|p| *p == "luid")?;
    Some((hex(parts.get(l + 1)?)?, hex(parts.get(l + 2)?)?))
}

/// Task Manager's GPU %: per engine, add up every process's share; an adapter's usage is its busiest engine.
/// Per process: its busiest engine. Returns (usage per adapter LUID, usage per pid), both 0–100.
pub fn aggregate_gpu(items: &[(String, f64)]) -> (std::collections::HashMap<(u32, u32), f64>, std::collections::HashMap<u32, f64>) {
    use std::collections::HashMap;
    let mut per_engine: HashMap<((u32, u32), u32, u32), f64> = HashMap::new();
    let mut per_pid_engine: HashMap<(u32, (u32, u32), u32, u32), f64> = HashMap::new();
    for (name, v) in items {
        if let Some(k) = parse_gpu_engine(name) {
            *per_engine.entry((k.luid, k.phys, k.eng)).or_default() += v;
            *per_pid_engine.entry((k.pid, k.luid, k.phys, k.eng)).or_default() += v;
        }
    }
    let mut adapters: HashMap<(u32, u32), f64> = HashMap::new();
    for ((luid, _, _), v) in per_engine {
        let e = adapters.entry(luid).or_default();
        *e = e.max(v.min(100.0));
    }
    let mut pids: HashMap<u32, f64> = HashMap::new();
    for ((pid, ..), v) in per_pid_engine {
        let e = pids.entry(pid).or_default();
        *e = e.max(v.min(100.0));
    }
    (adapters, pids)
}

/// Drive letters in a `PhysicalDisk` instance name ("0 C: G:" → [C, G]); `None` for "_Total".
pub fn parse_disk_instance(name: &str) -> Option<Vec<char>> {
    if name.starts_with('_') {
        return None;
    }
    Some(
        name.split_whitespace()
            .skip(1)
            .filter_map(|p| p.strip_suffix(':').and_then(|l| l.chars().next()).filter(|c| c.is_ascii_alphabetic()))
            .map(|c| c.to_ascii_uppercase())
            .collect(),
    )
}

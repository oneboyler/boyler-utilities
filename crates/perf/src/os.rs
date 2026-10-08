//! The OS layer: everything this crate asks Windows, behind one trait.

use crate::specs::PcSpecs;
use crate::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One GPU in one live reading.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GpuReading {
    pub name: String,
    /// Busiest engine, 0–100 (what Task Manager shows), from the `GPU Engine` counters.
    pub usage_pct: f64,
    pub vram_used_bytes: u64,
    pub vram_total_bytes: u64,
    /// °C, `None` when the driver doesn't report it.
    pub temperature_c: Option<f64>,
    /// Fan %, NVIDIA only (NVML). `None` elsewhere.
    pub fan_pct: Option<u32>,
    /// Fan RPM when the driver reports it (any vendor, WDDM 2.4+).
    pub fan_rpm: Option<u32>,
    /// Integrated (shares system RAM) or a card.
    pub integrated: bool,
}

/// One physical disk in one live reading.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DiskReading {
    /// PDH instance, e.g. "0 C:" — disk number + its letters.
    pub instance: String,
    pub letters: Vec<char>,
    /// "Active time" (100 − % idle time), 0–100.
    pub active_pct: f64,
    pub bytes_per_sec: f64,
}

/// Everything one 1-second tick reads. Rates are already per second (the OS layer keeps the previous reading).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LiveReading {
    pub cpu_usage_pct: f64,
    /// Current effective clock, MHz (base × % performance).
    pub cpu_mhz: f64,
    pub gpus: Vec<GpuReading>,
    pub ram_used_bytes: u64,
    pub ram_total_bytes: u64,
    pub disks: Vec<DiskReading>,
    /// Free space on the Windows drive (the Disk tile's "… free", Order 021), when Windows answers.
    pub system_free_bytes: Option<u64>,
    /// Bits per second over all real network adapters.
    pub net_down_bps: f64,
    pub net_up_bps: f64,
    /// GPU % per process id (busiest engine), for the process list's GPU column.
    pub gpu_by_pid: HashMap<u32, f64>,
}

/// A live source the sampler holds only while it runs. Dropping it releases every counter / library.
pub trait LiveSource: Send {
    fn read(&mut self) -> Result<LiveReading>;
}

/// Who a process runs as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessUser {
    /// The signed-in user (us).
    Me,
    /// SYSTEM / LOCAL SERVICE / NETWORK SERVICE (session 0 services).
    System,
    /// Another signed-in user.
    OtherUser,
    Unknown,
}

/// Priority classes. No Realtime (DESIGN; research: it can freeze the mouse).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Priority {
    Realtime,
    High,
    AboveNormal,
    Normal,
    BelowNormal,
    Low,
}

impl Priority {
    /// What the menu offers (no Realtime).
    pub const MENU: [Priority; 5] = [Priority::High, Priority::AboveNormal, Priority::Normal, Priority::BelowNormal, Priority::Low];
    pub fn name(self) -> &'static str {
        match self {
            Priority::Realtime => "Realtime",
            Priority::High => "High",
            Priority::AboveNormal => "Above normal",
            Priority::Normal => "Normal",
            Priority::BelowNormal => "Below normal",
            Priority::Low => "Low",
        }
    }
    /// From the base priority number Windows keeps per process (4 idle … 24 realtime).
    pub fn from_base(base: i32) -> Priority {
        match base {
            b if b >= 24 => Priority::Realtime,
            b if b >= 13 => Priority::High,
            b if b >= 10 => Priority::AboveNormal,
            b if b >= 8 => Priority::Normal,
            b if b >= 6 => Priority::BelowNormal,
            _ => Priority::Low,
        }
    }
}

/// One process as the OS reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct RawProcess {
    pub pid: u32,
    pub parent_pid: u32,
    /// Exe name ("chrome.exe"); "System", "Registry", "Idle" for kernel ones.
    pub exe: String,
    pub path: Option<PathBuf>,
    /// The exe's own description ("Google Chrome"), when it has one.
    pub description: Option<String>,
    pub session_id: u32,
    pub user: ProcessUser,
    /// Process start time (100 ns units) — with the pid it identifies a process even if the pid is reused.
    pub create_time: u64,
    /// User + kernel CPU time so far, 100 ns units.
    pub cpu_time: u64,
    /// CPU cycles used so far (all its threads; the Idle process: the idle cycles). 0 = not known (then CPU % comes
    /// from `cpu_time`). Windows charges `cpu_time` per clock tick (15.6 ms): a process that runs less than a tick at a
    /// time is often charged nothing, so most light apps read 0.0 % - cycles count every run (Task Manager, Process Explorer).
    pub cycle_time: u64,
    /// Private working set (what Task Manager's Memory column shows).
    pub ram_bytes: u64,
    pub priority: Priority,
    /// Has a visible top-level window (a plain app).
    pub has_window: bool,
}

/// How to end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndHow {
    /// Ask the windows to close (WM_CLOSE), like closing the app.
    Close,
    /// `TerminateProcess`: instant, the app can't save.
    Terminate,
}

/// Everything the Performance features ask Windows. `RealOs` is the Windows one; `FakeOs` is for tests.
pub trait PerfOs: Send + Sync {
    /// Open the live counters (PDH query, GPU adapters, NVML). Called when the page opens.
    fn open_live(&self) -> Result<Box<dyn LiveSource>>;
    /// Every running process, one snapshot.
    fn processes(&self) -> Result<Vec<RawProcess>>;
    /// Logical processors (for per-process CPU %).
    fn cpu_count(&self) -> u32;
    /// "Your PC".
    fn specs(&self) -> Result<PcSpecs>;
    /// End one process. Access denied → `NeedsAdmin`; gone → `NotFound`.
    fn end_process(&self, pid: u32, how: EndHow) -> Result<()>;
    fn set_priority(&self, pid: u32, p: Priority) -> Result<()>;
    /// Open Explorer with the file selected.
    fn open_file_location(&self, path: &Path) -> Result<()>;
    /// The exe's icon as 32-bit RGBA, `size` × `size`.
    fn icon_rgba(&self, path: &Path, size: u32) -> Result<Icon>;
    fn is_elevated(&self) -> bool;
}

/// An icon as RGBA pixels (row-major, top row first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Icon {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

//! The Performance page's link to `bu-perf`. A real copy runs ONE worker thread while the tab is open (the crate's live
//! sampler + a process refresh once a second + "Your PC" read once at the start); closing the tab stops it (zero cost
//! while closed). Test copies use the FAKE at the drawing's sample values (menu-v22 `PCS`, `PROCS[].c0`, `SPECS`),
//! built synchronously and frozen.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bu_perf::live::{Sampler, SamplerOptions};
use bu_perf::processes::{ProcessMonitor, ProcessRow};
use bu_perf::specs::{BoardSpec, CpuSpec, DisplaySpec, DriveSpec, GpuSpec, NetSpec, PcSpecs, RamSpec, WindowsSpec};
use bu_perf::{DiskReading, FakeOs, GpuReading, LiveReading, PerfOs, Priority, ProcessUser, RawProcess};

use crate::png::Pixels;

/// What the page shows; one per second from the worker (or once, frozen, in a test copy).
#[derive(Clone, Default)]
pub struct Snap {
    pub latest: Option<LiveReading>,
    /// oldest first, at most 40 (the sparklines)
    pub history: Vec<LiveReading>,
    pub rows: Vec<ProcessRow>,
    pub specs: Option<PcSpecs>,
    /// process icons by exe path (premultiplied BGRA 20 x 20), read once per path
    pub icons: HashMap<PathBuf, Arc<Pixels>>,
    pub err: Option<String>,
}

/// What the page asks the worker to do (the monitor that knows a row's helpers lives there).
pub enum Cmd {
    ShowWindows(bool),
    End { key: String, tree: bool },
    Priority { key: String, p: Priority },
    OpenLocation { key: String },
}

pub enum Msg {
    Snap(Box<Snap>),
    Toast(String),
}

pub struct Worker {
    tx: Sender<Option<Cmd>>,
    pub rx: Receiver<Msg>,
    /// Order 047: the tab closed - the thread ends at its next step (it is not waited for)
    stop: Arc<AtomicBool>,
}

impl Worker {
    /// `specs` = "Your PC" kept from the last visit (None = read it now: WMI, the slow part).
    pub fn start(os: Arc<dyn PerfOs>, waker: crate::services::Waker, specs: Option<PcSpecs>) -> Worker {
        let (ctx, crx) = channel::<Option<Cmd>>();
        let (mtx, mrx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let st = stop.clone();
        // (its handle is not kept: nobody waits for the thread)
        let _ = std::thread::Builder::new().name("bu-perf-page".into()).spawn(move || run(os, crx, mtx, waker, specs, st));
        Worker { tx: ctx, rx: mrx, stop }
    }
    pub fn send(&self, c: Cmd) {
        let _ = self.tx.send(Some(c));
    }
}

impl Drop for Worker {
    /// The tab closed: the worker is told to stop (its sampler and counters with it) and is NOT waited for - Order 047: it
    /// may be inside "Your PC"'s WMI read (1-3 s) or its first snapshot; it ends right after that, on its own thread, and
    /// the menu's thread goes on at once.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.tx.send(None);
    }
}

fn to_pixels(i: &bu_perf::Icon) -> Pixels {
    let mut data = Vec::with_capacity(i.rgba.len());
    for px in i.rgba.as_chunks::<4>().0 {
        let a = px[3] as u32;
        let pm = |c: u8| ((c as u32 * a + 127) / 255) as u8;
        data.extend_from_slice(&[pm(px[2]), pm(px[1]), pm(px[0]), px[3]]);
    }
    Pixels { w: i.width, h: i.height, data }
}

/// One snapshot: the sampler's numbers + a process refresh (+ icons of new paths).
pub fn snap(os: &dyn PerfOs, sampler: Option<&Sampler>, mon: &mut ProcessMonitor, show_windows: bool, icons: &mut HashMap<PathBuf, Arc<Pixels>>, specs: &Option<PcSpecs>) -> Snap {
    let history: Vec<LiveReading> = sampler.map(|s| s.history().into_iter().map(|t| t.reading).collect()).unwrap_or_default();
    let latest = history.last().cloned();
    let gpu = latest.as_ref().map(|l| l.gpu_by_pid.clone()).unwrap_or_default();
    let (rows, err) = match mon.refresh(os, &gpu, show_windows) {
        Ok(r) => (r, None),
        Err(e) => (Vec::new(), Some(e.to_string())),
    };
    for r in &rows {
        if let Some(p) = &r.path {
            if !icons.contains_key(p) {
                if let Ok(i) = os.icon_rgba(p, 20) {
                    icons.insert(p.clone(), Arc::new(to_pixels(&i)));
                }
            }
        }
    }
    Snap { latest, history, rows, specs: specs.clone(), icons: icons.clone(), err }
}

/// Every message wakes the menu (`waker`): it repaints once per snapshot, nothing in between.
fn run(os: Arc<dyn PerfOs>, rx: Receiver<Option<Cmd>>, tx: Sender<Msg>, waker: crate::services::Waker, kept: Option<PcSpecs>, stop: Arc<AtomicBool>) {
    let specs = kept.or_else(|| os.specs().ok());
    // Order 047: the tab closed while "Your PC" was read: nothing more starts (the closed tab did not wait for this)
    if stop.load(Ordering::Acquire) {
        return;
    }
    let sampler = Sampler::start(os.clone(), SamplerOptions::default()).ok();
    if stop.load(Ordering::Acquire) {
        if let Some(s) = sampler {
            s.stop();
        }
        return;
    }
    let mut mon = ProcessMonitor::for_this_pc();
    let mut show = false;
    let mut icons = HashMap::new();
    let mut rows: Vec<ProcessRow> = Vec::new();
    let mut next = Instant::now();
    loop {
        let wait = next.saturating_duration_since(Instant::now());
        match rx.recv_timeout(wait) {
            Ok(None) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(Some(c)) => {
                let find = |k: &str| rows.iter().find(|r| r.key == k).cloned();
                let toast = match c {
                    Cmd::ShowWindows(on) => {
                        show = on;
                        next = Instant::now();
                        None
                    }
                    Cmd::End { key, tree } => find(&key).map(|r| {
                        let res = if tree { mon.end_tree(os.as_ref(), &r, true) } else { mon.end_task(os.as_ref(), &r, true) };
                        next = Instant::now();
                        match res {
                            Ok(rep) => rep.toast(&r.name),
                            Err(e) => e.to_string(),
                        }
                    }),
                    Cmd::Priority { key, p } => find(&key).map(|r| match mon.set_priority(os.as_ref(), &r, p) {
                        Ok(_) => bu_perf::processes::PriorityUndo::toast(&r.name, p),
                        Err(e) => e.to_string(),
                    }),
                    Cmd::OpenLocation { key } => find(&key).and_then(|r| {
                        let f = r.path.as_ref()?.file_name()?.to_string_lossy().to_string();
                        Some(match mon.open_file_location(os.as_ref(), &r) {
                            Ok(()) => format!("Opens Explorer with {f} selected"),
                            Err(e) => e.to_string(),
                        })
                    }),
                };
                if let Some(t) = toast {
                    let _ = tx.send(Msg::Toast(t));
                    waker.wake();
                }
                continue;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        let s = snap(os.as_ref(), sampler.as_ref(), &mut mon, show, &mut icons, &specs);
        rows = s.rows.clone();
        let sent = tx.send(Msg::Snap(Box::new(s)));
        waker.wake();
        if sent.is_err() {
            break;
        }
        next = Instant::now() + Duration::from_secs(1);
    }
    if let Some(s) = sampler {
        s.stop();
    }
}

// ---------------------------------------------------------------- the drawing's sample (test copies)

const MB: u64 = 1024 * 1024;

/// The drawing's PC (`SPECS`).
pub fn sample_specs() -> PcSpecs {
    PcSpecs {
        cpu: CpuSpec { name: "AMD Ryzen 7 7800X3D 8-Core Processor".into(), cores: Some(8), threads: Some(16), threads_in_use: Some(16), max_mhz: Some(5000) },
        gpus: vec![GpuSpec { name: "NVIDIA GeForce RTX 4070 SUPER".into(), vram_bytes: Some(12 * 1024 * MB), integrated: false, driver: Some("32.0.15.8142".into()) }],
        ram: RamSpec { total_bytes: 32 * 1024 * MB, kind: Some("DDR5".into()), speed_mts: Some(6000), rated_mts: Some(4800), sticks: 2, xmp_expo: Some(true) },
        board: BoardSpec { maker: Some("ASUS".into()), model: Some("ROG STRIX B650E-F GAMING WIFI".into()), bios_version: Some("3263".into()), bios_date: None },
        drives: vec![
            DriveSpec { model: "Samsung 990 PRO".into(), size_bytes: 2_000_398_934_016, media: Some("SSD".into()), bus: Some("NVMe".into()) },
            DriveSpec { model: "WD_BLACK SN850X".into(), size_bytes: 2_000_398_934_016, media: Some("SSD".into()), bus: Some("NVMe".into()) },
            DriveSpec { model: "Seagate BarraCuda".into(), size_bytes: 4_000_787_030_016, media: Some("HDD".into()), bus: Some("SATA".into()) },
        ],
        displays: vec![
            DisplaySpec { name: "DELL S2721DGF".into(), width: 1920, height: 1080, hz: 165.0 },
            DisplaySpec { name: "LG 24GL600F".into(), width: 1920, height: 1080, hz: 144.0 },
        ],
        nets: vec![
            NetSpec { name: "Intel(R) Ethernet Controller I226-V".into(), speed_bps: Some(2_500_000_000), wireless: false, connected: true },
            NetSpec { name: "Intel(R) Wi-Fi 6E AX210 160MHz".into(), speed_bps: None, wireless: true, connected: false },
        ],
        windows: WindowsSpec { edition: "Windows 11 Pro".into(), version: Some("24H2".into()), build: Some("26100.6584".into()), install_date: Some("2026-02-03".into()) },
    }
}

/// The drawing's tiles (`PCS`): CPU 16 % at 4.60 GHz, GPU 38 % 57 °C VRAM 5.2 / 12 GB fan 30 %, RAM 12.6 of 32 GB, C: 2 %
/// 6 MB/s with 612 GB free (the Storage page's C: sample), network ↓ 9.6 ↑ 0.6 Mb/s.
pub fn sample_reading(gpu_by_pid: HashMap<u32, f64>) -> LiveReading {
    LiveReading {
        cpu_usage_pct: 16.0,
        cpu_mhz: 4600.0,
        gpus: vec![GpuReading {
            name: "NVIDIA GeForce RTX 4070 SUPER".into(),
            usage_pct: 38.0,
            vram_used_bytes: (5.2 * 1024.0) as u64 * MB,
            vram_total_bytes: 12 * 1024 * MB,
            temperature_c: Some(57.0),
            fan_pct: Some(30),
            fan_rpm: None,
            integrated: false,
        }],
        ram_used_bytes: (12.6 * 1024.0) as u64 * MB,
        ram_total_bytes: 32 * 1024 * MB,
        disks: vec![DiskReading { instance: "0 C:".into(), letters: vec!['C'], active_pct: 2.0, bytes_per_sec: 6e6 }],
        net_down_bps: 9.6e6,
        net_up_bps: 0.6e6,
        gpu_by_pid,
        system_free_bytes: Some(612 * 1024 * MB),
    }
}

/// The drawing's processes (`PROCS`): (description, exe, path, cpu %, RAM MB, GPU %, helpers, Windows' own).
pub const SAMPLE_PROCS: [(&str, &str, &str, f64, u64, f64, u32, bool); 17] = [
    ("VALORANT", "VALORANT-Win64-Shipping.exe", r"C:\Riot Games\VALORANT\live\VALORANT.exe", 9.4, 3150, 31.0, 1, false),
    ("Google Chrome", "chrome.exe", r"C:\Program Files\Google\Chrome\Application\chrome.exe", 2.6, 1720, 1.8, 14, false),
    ("Discord", "Discord.exe", r"C:\Users\someone\AppData\Local\Discord\app-1.0.9200\Discord.exe", 1.3, 880, 0.9, 5, false),
    ("OBS Studio", "obs64.exe", r"C:\Program Files\obs-studio\bin\64bit\obs64.exe", 3.2, 540, 5.6, 0, false),
    ("Spotify", "Spotify.exe", r"C:\Users\someone\AppData\Roaming\Spotify\Spotify.exe", 0.6, 420, 0.3, 4, false),
    ("Steam", "steam.exe", r"C:\Program Files (x86)\Steam\steam.exe", 0.3, 310, 0.0, 3, false),
    ("Windows Explorer", "explorer.exe", r"C:\Windows\explorer.exe", 0.4, 190, 0.2, 0, false),
    ("NVIDIA Container", "nvcontainer.exe", r"C:\Program Files\NVIDIA Corporation\NvContainer\nvcontainer.exe", 0.2, 120, 0.0, 2, false),
    ("Riot Vanguard tray", "vgtray.exe", r"C:\Program Files\Riot Vanguard\vgtray.exe", 0.0, 24, 0.0, 0, false),
    ("Wootility", "Wootility.exe", r"C:\Users\someone\AppData\Local\Programs\wootility\Wootility.exe", 0.1, 160, 0.1, 2, false),
    ("Desktop Window Manager", "dwm.exe", r"C:\Windows\System32\dwm.exe", 1.1, 150, 3.2, 0, true),
    ("Antimalware Service Executable", "MsMpEng.exe", r"C:\ProgramData\Microsoft\Windows Defender\Platform\MsMpEng.exe", 0.4, 260, 0.0, 0, true),
    ("Service Host: Windows Audio", "svchost.exe", r"C:\Windows\System32\svchost.exe", 0.3, 14, 0.0, 0, true),
    ("Client Server Runtime Process", "csrss.exe", r"C:\Windows\System32\csrss.exe", 0.1, 6, 0.1, 0, true),
    ("Local Security Authority Process", "lsass.exe", r"C:\Windows\System32\lsass.exe", 0.1, 28, 0.0, 0, true),
    ("Windows Logon Application", "winlogon.exe", r"C:\Windows\System32\winlogon.exe", 0.0, 9, 0.0, 0, true),
    ("System", "System", r"C:\Windows\System32\ntoskrnl.exe", 0.5, 4, 0.0, 0, true),
];

/// The fake with the drawing's processes; returns (os, gpu % by pid). `cpu_time` = 0 here; [`sample_rows`] adds the
/// second snapshot that gives each app its `c0` CPU %.
pub fn sample_os() -> (FakeOs, HashMap<u32, f64>) {
    let os = FakeOs::new();
    os.with_cpu_count(16).with_specs(sample_specs());
    let (procs, gpu) = sample_procs(0.0);
    os.with_processes(procs);
    (os, gpu)
}

fn sample_procs(secs: f64) -> (Vec<RawProcess>, HashMap<u32, f64>) {
    let mut out = Vec::new();
    let mut gpu = HashMap::new();
    let mut pid = 1000u32;
    for (desc, exe, path, cpu, ram, g, kids, win) in SAMPLE_PROCS {
        let main = pid;
        for k in 0..=kids {
            let p = if exe == "System" { 4 } else { pid };
            // the CPU time after `secs` seconds that makes `cpu` % of 16 logical processors
            let t = if k == 0 { (cpu / 100.0 * secs * 16.0 * 1e7) as u64 } else { 0 };
            out.push(RawProcess {
                pid: p,
                parent_pid: if k == 0 { 1 } else { main },
                exe: exe.into(),
                path: Some(path.into()),
                description: Some(desc.into()),
                session_id: if win { 0 } else { 1 },
                user: if win { ProcessUser::System } else { ProcessUser::Me },
                create_time: 1,
                cpu_time: t,
                cycle_time: 0,
                ram_bytes: if k == 0 { ram * MB } else { 0 },
                priority: Priority::Normal,
                has_window: !win && !matches!(exe, "nvcontainer.exe" | "vgtray.exe"),
            });
            if k == 0 && g > 0.0 {
                gpu.insert(p, g);
            }
            pid += 1;
        }
    }
    (out, gpu)
}

/// The rows at the drawing's sample values (two snapshots one second apart).
pub fn sample_rows(os: &FakeOs, mon: &mut ProcessMonitor, gpu: &HashMap<u32, f64>, show_windows: bool) -> Vec<ProcessRow> {
    let t0 = Instant::now();
    let (p0, _) = sample_procs(0.0);
    os.with_processes(p0);
    let _ = mon.refresh_at(os, gpu, show_windows, t0);
    let (p1, _) = sample_procs(1.0);
    os.with_processes(p1);
    mon.refresh_at(os, gpu, show_windows, t0 + Duration::from_secs(1)).unwrap_or_default()
}

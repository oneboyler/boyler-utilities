//! "Your PC" (DESIGN §3.8): CPU, GPU, RAM + speed, motherboard, drives, displays, Windows. Read once when the page
//! first opens; no live cost. Unknown values show "—".

use crate::{PerfOs, Result};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CpuSpec {
    pub name: String,
    /// The chip's own cores / threads (`NumberOfEnabledCore` / `ThreadCount`, falling back to the in-use numbers).
    pub cores: Option<u32>,
    pub threads: Option<u32>,
    /// Logical processors Windows actually runs on (`NumberOfLogicalProcessors`) — fewer than `threads` when a CCD is
    /// off in the BIOS or Windows is limited to fewer processors (measured on a 16-core PC: 16 of 32).
    pub threads_in_use: Option<u32>,
    /// `Win32_Processor.MaxClockSpeed` (MHz) — the rated base/max clock Windows knows.
    pub max_mhz: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct GpuSpec {
    pub name: String,
    /// Real VRAM from DXGI (WMI's AdapterRAM stops at 4 GB).
    pub vram_bytes: Option<u64>,
    /// Integrated (shares system RAM) or a card.
    pub integrated: bool,
    pub driver: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RamSpec {
    pub total_bytes: u64,
    /// "DDR5", "DDR4" … (SMBIOS memory type).
    pub kind: Option<String>,
    /// Running speed, MT/s (`ConfiguredClockSpeed`).
    pub speed_mts: Option<u32>,
    /// The sticks' own JEDEC speed, MT/s (`Speed`).
    pub rated_mts: Option<u32>,
    pub sticks: u32,
    /// `Some(true)` when running faster than the sticks' JEDEC speed (EXPO / XMP on). `None` = can't tell.
    pub xmp_expo: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BoardSpec {
    pub maker: Option<String>,
    pub model: Option<String>,
    pub bios_version: Option<String>,
    /// "2024-05-17".
    pub bios_date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DriveSpec {
    pub model: String,
    pub size_bytes: u64,
    /// "SSD" / "HDD".
    pub media: Option<String>,
    /// "NVMe" / "SATA" / "USB".
    pub bus: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DisplaySpec {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub hz: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WindowsSpec {
    /// "Windows 11 Pro".
    pub edition: String,
    /// "24H2".
    pub version: Option<String>,
    /// "26100.4061".
    pub build: Option<String>,
    /// "2025-03-02".
    pub install_date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PcSpecs {
    pub cpu: CpuSpec,
    pub gpus: Vec<GpuSpec>,
    pub ram: RamSpec,
    pub board: BoardSpec,
    pub drives: Vec<DriveSpec>,
    pub displays: Vec<DisplaySpec>,
    /// Network adapters (Order 021: the v21 drawing's 8th fact, so the grid is even)
    pub nets: Vec<NetSpec>,
    pub windows: WindowsSpec,
}

/// One physical network adapter (Order 021).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NetSpec {
    /// As Windows names it ("Intel(R) Ethernet Controller I226-V").
    pub name: String,
    /// Link speed, bits per second (when connected).
    pub speed_bps: Option<u64>,
    pub wireless: bool,
    pub connected: bool,
}

/// One cell: label, value, quiet line.
#[derive(Debug, Clone, PartialEq)]
pub struct SpecCell {
    pub label: &'static str,
    pub value: String,
    pub quiet: String,
}

const DASH: &str = "—";

fn gb(bytes: u64) -> String {
    let g = bytes as f64 / (1u64 << 30) as f64;
    if g >= 1000.0 {
        format!("{:.1} TB", g / 1024.0)
    } else if (g - g.round()).abs() < 0.05 {
        format!("{:.0} GB", g)
    } else {
        format!("{g:.1} GB")
    }
}

impl PcSpecs {
    /// The cells in the drawing's order and words (menu-v22 `SPECS`, Order 021): CPU · GPU · RAM · Motherboard · Drives ·
    /// Displays · Network · Windows, two columns, 8 facts. Facts Windows doesn't give are left out of a line (never made up):
    /// the boost clock ("up to"), the VRAM type (GDDR6X).
    pub fn cells(&self) -> Vec<SpecCell> {
        let join = |parts: Vec<String>| if parts.is_empty() { DASH.to_string() } else { parts.join(" · ") };
        let cpu_quiet = {
            let mut parts = Vec::new();
            if let Some(c) = self.cpu.cores {
                parts.push(format!("{c} cores"));
            }
            if let Some(t) = self.cpu.threads {
                parts.push(format!("{t} threads"));
            }
            if let Some(m) = self.cpu.max_mhz {
                parts.push(format!("{:.1} GHz", m as f64 / 1000.0));
            }
            if let (Some(u), Some(t)) = (self.cpu.threads_in_use, self.cpu.threads) {
                if u < t {
                    parts.push(format!("Windows uses {u} threads"));
                }
            }
            join(parts)
        };
        let gpu = self.gpus.iter().find(|g| !g.integrated).or(self.gpus.first());
        let gpu_quiet = gpu
            .map(|g| {
                let mut parts = Vec::new();
                if let Some(v) = g.vram_bytes {
                    parts.push(if g.integrated { format!("{} shared", gb(v)) } else { gb(v) });
                }
                if let Some(d) = &g.driver {
                    parts.push(format!("driver {}", driver_version(&g.name, d)));
                }
                if self.gpus.len() > 1 {
                    parts.push(format!("+{} more", self.gpus.len() - 1));
                }
                join(parts)
            })
            .unwrap_or_else(|| DASH.to_string());
        let r = &self.ram;
        let ram_value = {
            let mut s = if r.total_bytes > 0 { gb(r.total_bytes) } else { DASH.to_string() };
            if let Some(k) = &r.kind {
                s.push_str(&format!(" {k}"));
            }
            if let Some(m) = r.speed_mts {
                s.push_str(&format!(" · {m} MT/s"));
            }
            s
        };
        let ram_quiet = {
            // "2 × 16 GB · EXPO on" (AMD calls it EXPO, Intel XMP)
            let mut parts = Vec::new();
            if r.sticks > 0 && r.total_bytes > 0 {
                parts.push(format!("{} × {}", r.sticks, gb(r.total_bytes / u64::from(r.sticks))));
            }
            let name = if self.cpu.name.to_lowercase().contains("amd") { "EXPO" } else { "XMP" };
            match r.xmp_expo {
                Some(true) => parts.push(format!("{name} on")),
                Some(false) => parts.push(format!("{name} off")),
                None => {}
            }
            join(parts)
        };
        let b = &self.board;
        let board_value = match (&b.maker, &b.model) {
            (Some(m), Some(p)) => format!("{m} {p}"),
            (None, Some(p)) => p.clone(),
            (Some(m), None) => m.clone(),
            (None, None) => DASH.to_string(),
        };
        let board_quiet = b.bios_version.as_ref().map(|v| format!("BIOS {v}")).unwrap_or_else(|| DASH.to_string());
        // "Samsung 990 PRO · 2 TB NVMe", the others under it ("WD_BLACK SN850X 2 TB · Seagate BarraCuda 4 TB")
        let drive = |d: &DriveSpec| format!("{} {}", d.model, sold_size(d.size_bytes));
        let drives_value = self
            .drives
            .first()
            .map(|d| match &d.bus {
                Some(bus) => format!("{} · {} {bus}", d.model, sold_size(d.size_bytes)),
                None => format!("{} · {}", d.model, sold_size(d.size_bytes)),
            })
            .unwrap_or_else(|| DASH.to_string());
        let drives_quiet = join(self.drives.iter().skip(1).map(drive).collect());
        // "DELL S2721DGF · 1920 × 1080 · 165 Hz", the others under it
        let disp = |d: &DisplaySpec| format!("{} · {} × {} · {:.0} Hz", d.name, d.width, d.height, d.hz);
        let displays_value = self.displays.first().map(disp).unwrap_or_else(|| DASH.to_string());
        let displays_quiet = join(self.displays.iter().skip(1).map(disp).collect());
        // the adapter in use first (wired before Wi-Fi): "Intel Ethernet I226-V · 2.5 Gbps", the others under it
        let mut nets: Vec<&NetSpec> = self.nets.iter().collect();
        nets.sort_by_key(|n| (!n.connected, n.wireless));
        let net_value = nets
            .first()
            .map(|n| match (n.connected, n.speed_bps) {
                (true, Some(b)) => format!("{} · {}", net_name(n), link_speed(b)),
                _ => net_name(n),
            })
            .unwrap_or_else(|| DASH.to_string());
        let net_quiet = join(nets.iter().skip(1).map(|n| net_name(n)).collect());
        let w = &self.windows;
        let win_value = match &w.version {
            Some(v) => format!("{} · {v}", w.edition),
            None => w.edition.clone(),
        };
        let win_quiet = {
            let mut parts = Vec::new();
            if let Some(b) = &w.build {
                parts.push(format!("Build {b}"));
            }
            if let Some(d) = &w.install_date {
                parts.push(format!("installed {}", nice_date(d)));
            }
            join(parts)
        };
        let nz = |s: String| if s.trim().is_empty() { DASH.to_string() } else { s };
        vec![
            SpecCell { label: "CPU", value: nz(cpu_name(&self.cpu.name)), quiet: cpu_quiet },
            SpecCell { label: "GPU", value: nz(gpu.map(|g| g.name.clone()).unwrap_or_default()), quiet: gpu_quiet },
            SpecCell { label: "RAM", value: ram_value, quiet: ram_quiet },
            SpecCell { label: "Motherboard", value: board_value, quiet: board_quiet },
            SpecCell { label: "Drives", value: drives_value, quiet: drives_quiet },
            SpecCell { label: "Displays", value: displays_value, quiet: displays_quiet },
            SpecCell { label: "Network", value: net_value, quiet: net_quiet },
            SpecCell { label: "Windows", value: nz(win_value), quiet: win_quiet },
        ]
    }

    /// "Copy all": a plain-text list, one "Label: value (quiet)" line per cell.
    pub fn copy_text(&self) -> String {
        self.cells()
            .iter()
            .map(|c| if c.quiet == DASH { format!("{}: {}", c.label, c.value) } else { format!("{}: {} ({})", c.label, c.value, c.quiet) })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The CPU's name as the drawing writes it: Windows' name without the trademark marks and the "N-Core Processor" / "CPU"
/// tail ("AMD Ryzen 7 7800X3D 8-Core Processor" -> "AMD Ryzen 7 7800X3D"; "Intel(R) Core(TM) i7-14700K" ->
/// "Intel Core i7-14700K").
pub fn cpu_name(name: &str) -> String {
    let s = name.replace("(R)", "").replace("(TM)", "").replace("(r)", "").replace("(tm)", "");
    let words: Vec<&str> = s
        .split_whitespace()
        .filter(|w| !w.ends_with("-Core") && !w.ends_with("-core") && *w != "Processor" && *w != "CPU")
        .collect();
    words.join(" ")
}

/// A drive's size as the box says it (decimal units): 2,000,398,934,016 bytes = "2 TB", "500 GB", "1.5 TB".
pub fn sold_size(bytes: u64) -> String {
    let tb = bytes as f64 / 1e12;
    if tb >= 1.0 {
        if (tb - tb.round()).abs() < 0.05 {
            format!("{:.0} TB", tb.round())
        } else {
            format!("{tb:.1} TB")
        }
    } else {
        format!("{:.0} GB", bytes as f64 / 1e9)
    }
}

/// NVIDIA's own driver number from Windows' driver version ("32.0.15.8142" -> "581.42": the last five digits); others as
/// Windows says it.
pub fn driver_version(gpu: &str, v: &str) -> String {
    if gpu.to_lowercase().contains("nvidia") {
        let digits: String = v.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() >= 5 {
            let t = &digits[digits.len() - 5..];
            return format!("{}.{}", t[..3].trim_start_matches('0'), &t[3..]);
        }
    }
    v.to_string()
}

/// An adapter's name without the trademark noise ("Intel(R) Ethernet Controller I226-V" -> "Intel Ethernet I226-V"); a
/// Wi-Fi card leads with its Wi-Fi generation when its name has one ("Intel(R) Wi-Fi 6E AX210 160MHz" ->
/// "Wi-Fi 6E · Intel AX210").
pub fn net_name(n: &NetSpec) -> String {
    let mut s = n.name.replace("(R)", "").replace("(TM)", "").replace("(r)", "").replace("(tm)", "");
    for w in [" Controller", " Network Connection", " Adapter", " 160MHz", " 80MHz"] {
        s = s.replace(w, "");
    }
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some(i) = s.find("Wi-Fi ") {
        let rest: Vec<&str> = s[i + 6..].split_whitespace().collect();
        if let Some(gen) = rest.first().filter(|g| g.chars().next().is_some_and(|c| c.is_ascii_digit())) {
            let before = s[..i].trim();
            let after = rest[1..].join(" ");
            let model = [before, after.as_str()].iter().filter(|x| !x.is_empty()).copied().collect::<Vec<_>>().join(" ");
            return if model.is_empty() { format!("Wi-Fi {gen}") } else { format!("Wi-Fi {gen} · {model}") };
        }
    }
    s
}

/// "2.5 Gbps", "1 Gbps", "100 Mbps".
pub fn link_speed(bps: u64) -> String {
    let g = bps as f64 / 1e9;
    if g >= 1.0 {
        if (g - g.round()).abs() < 0.05 {
            format!("{:.0} Gbps", g.round())
        } else {
            format!("{g:.1} Gbps")
        }
    } else {
        format!("{:.0} Mbps", bps as f64 / 1e6)
    }
}

/// "2026-02-03" -> "3 Feb 2026".
pub fn nice_date(iso: &str) -> String {
    const M: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let p: Vec<&str> = iso.split('-').collect();
    match (p.first(), p.get(1).and_then(|m| m.parse::<usize>().ok()), p.get(2).and_then(|d| d.parse::<u32>().ok())) {
        (Some(y), Some(m), Some(d)) if (1..=12).contains(&m) => format!("{d} {} {y}", M[m - 1]),
        _ => iso.to_string(),
    }
}

/// Read "Your PC" once.
pub fn read(os: &dyn PerfOs) -> Result<PcSpecs> {
    os.specs()
}

/// SMBIOS memory type number → name (SMBIOS 3.x table 76).
pub fn memory_type_name(t: u32) -> Option<&'static str> {
    Some(match t {
        20 => "DDR",
        21 => "DDR2",
        24 => "DDR3",
        26 => "DDR4",
        27 => "LPDDR",
        28 => "LPDDR2",
        29 => "LPDDR3",
        30 => "LPDDR4",
        34 => "DDR5",
        35 => "LPDDR5",
        _ => return None,
    })
}

/// EXPO/XMP from speeds: running above the sticks' JEDEC speed means a profile is on. Equal speeds can't be told
/// apart (no profile, or a kit whose JEDEC speed equals its profile) → `None` unless clearly slower.
pub fn xmp_from_speeds(configured: Option<u32>, rated: Option<u32>) -> Option<bool> {
    match (configured, rated) {
        (Some(c), Some(r)) if c > r => Some(true),
        (Some(c), Some(r)) if c < r => Some(false),
        _ => None,
    }
}

//! Every Performance row against the fake OS (nothing real is touched; no real process is ended or re-prioritised).

use bu_perf::live::{self, Sampler, SamplerOptions, TempLevel};
use bu_perf::processes::{self, format_pct, format_ram, EndRule, PriorityUndo, ProcessMonitor, SortBy};
use bu_perf::specs::{self, CpuSpec, DisplaySpec, DriveSpec, GpuSpec, PcSpecs, RamSpec, WindowsSpec};
use bu_perf::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MB: u64 = 1024 * 1024;

// ---------------------------------------------------------------- live

fn reading(cpu: f64) -> LiveReading {
    LiveReading {
        cpu_usage_pct: cpu,
        cpu_mhz: 4600.0,
        gpus: vec![
            GpuReading { name: "iGPU".into(), integrated: true, vram_total_bytes: 512 * MB, ..Default::default() },
            GpuReading {
                name: "Card".into(),
                usage_pct: 40.0,
                vram_used_bytes: 5 * 1024 * MB + 205 * MB,
                vram_total_bytes: 12 * 1024 * MB,
                temperature_c: Some(76.0),
                fan_pct: Some(30),
                ..Default::default()
            },
        ],
        ram_used_bytes: 12 * 1024 * MB + 500 * MB,
        ram_total_bytes: 32 * 1024 * MB,
        disks: vec![
            DiskReading { instance: "0 D:".into(), letters: vec!['D'], active_pct: 1.0, bytes_per_sec: 0.0 },
            DiskReading { instance: "1 C:".into(), letters: vec!['C'], active_pct: 3.0, bytes_per_sec: 6e6 },
        ],
        net_down_bps: 9.6e6,
        net_up_bps: 0.6e6,
        gpu_by_pid: HashMap::new(),
        system_free_bytes: Some(812 * 1024 * MB),
    }
}

#[test]
fn sampler_runs_only_while_started_and_keeps_40_ticks() {
    let os = FakeOs::new();
    for i in 0..100 {
        os.push_reading(reading(i as f64));
    }
    assert_eq!(os.live_open(), 0, "nothing open before the page opens");
    let s = Sampler::start(Arc::new(os.clone()), SamplerOptions { interval: Duration::from_millis(5), history_len: 40 }).unwrap();
    assert_eq!(os.live_open(), 1);
    assert!(s.latest().is_some(), "first reading is there at once");
    let t0 = Instant::now();
    while s.reads() < 60 && t0.elapsed() < Duration::from_secs(120) {
        std::thread::sleep(Duration::from_millis(5));
    }
    let h = s.history();
    assert_eq!(h.len(), 40, "last 40 only");
    assert!(h.windows(2).all(|w| w[0].since_start <= w[1].since_start));
    assert!(h.last().unwrap().reading.cpu_usage_pct >= 58.0);
    s.stop();
    assert_eq!(os.live_open(), 0, "counters released on stop");
    let after = os.live_reads();
    std::thread::sleep(Duration::from_millis(60));
    assert_eq!(os.live_reads(), after, "zero reads while stopped");
}

#[test]
fn sampler_stops_on_drop_and_open_errors_surface() {
    let os = FakeOs::new();
    os.push_reading(reading(1.0));
    {
        let _s = Sampler::start(Arc::new(os.clone()), SamplerOptions { interval: Duration::from_millis(5), history_len: 3 }).unwrap();
        std::thread::sleep(Duration::from_millis(30));
    }
    assert_eq!(os.live_open(), 0);
    let bad = FakeOs::new();
    bad.with_open_live_failing();
    assert!(Sampler::start(Arc::new(bad), SamplerOptions::default()).is_err());
}

#[test]
fn stop_wakes_the_sampler_at_once() {
    let os = FakeOs::new();
    os.push_reading(reading(1.0));
    let s = Sampler::start(Arc::new(os), SamplerOptions { interval: Duration::from_secs(30), history_len: 40 }).unwrap();
    // The next tick is 30 s away, so this is exactly the reading `start` took itself (load-proof).
    assert_eq!(s.latest().unwrap().reading.cpu_usage_pct, 1.0, "first reading is taken before start returns");
    let t0 = Instant::now();
    s.stop();
    assert!(t0.elapsed() < Duration::from_secs(20), "stop doesn't wait for the 30 s tick (generous bound: load-proof)");
}

#[test]
fn tiles_text_and_picks() {
    let r = reading(39.0);
    let g = live::main_gpu(&r).unwrap();
    assert_eq!(g.name, "Card", "the card, not the integrated GPU");
    assert_eq!(live::gpu_extra(g), "VRAM 5.2 / 12 GB · Fan 30 %");
    assert_eq!(live::temp_level(g.temperature_c.unwrap()), TempLevel::Warm);
    assert_eq!(live::temp_level(74.9), TempLevel::Ok);
    assert_eq!(live::temp_level(85.0), TempLevel::Hot);
    assert_eq!(live::main_disk(&r, 'C').unwrap().instance, "1 C:");
    assert_eq!(live::format_ghz(4600.0), "4.60 GHz");
    assert_eq!(live::format_mbps(9.6e6), "9.6 Mb/s");
    assert_eq!(live::format_mbps(48e3), "48 Kb/s");
    assert_eq!(live::format_mb_per_s(6e6), "6.0 MB/s");
    assert_eq!(live::ram_text(r.ram_used_bytes, r.ram_total_bytes), ("12.5 GB".into(), "of 32 · 39 %".into()));
    let rpm = GpuReading { fan_rpm: Some(1056), ..Default::default() };
    assert!(live::gpu_extra(&rpm).ends_with("Fan 1056 rpm"));
    assert!(matches!(live::cpu_temperature(), Err(PerfError::Unavailable(_))), "no CPU temperature (no driver)");
}

#[test]
fn gpu_and_disk_counter_names() {
    let k = live::parse_gpu_engine("pid_1234_luid_0x00000000_0x0000D1F2_phys_0_eng_3_engtype_VideoDecode").unwrap();
    assert_eq!((k.pid, k.luid, k.phys, k.eng), (1234, (0, 0xD1F2), 0, 3));
    assert_eq!(live::parse_gpu_engine("garbage"), None);
    assert_eq!(live::parse_luid("luid_0x00000000_0x0000D1F2_phys_0"), Some((0, 0xD1F2)));
    let items = vec![
        ("pid_1_luid_0x0_0xA_phys_0_eng_0_engtype_3D".to_string(), 30.0),
        ("pid_2_luid_0x0_0xA_phys_0_eng_0_engtype_3D".to_string(), 25.0),
        ("pid_2_luid_0x0_0xA_phys_0_eng_1_engtype_Copy".to_string(), 40.0),
        ("pid_3_luid_0x0_0xB_phys_0_eng_0_engtype_3D".to_string(), 5.0),
    ];
    let (adapters, pids) = live::aggregate_gpu(&items);
    assert_eq!(adapters[&(0, 0xA)], 55.0, "3D engine = 30 + 25 > copy 40");
    assert_eq!(adapters[&(0, 0xB)], 5.0);
    assert_eq!(pids[&2], 40.0, "a process shows its busiest engine");
    assert_eq!(live::parse_disk_instance("2 E: F:"), Some(vec!['E', 'F']));
    assert_eq!(live::parse_disk_instance("_Total"), None);
}

// ---------------------------------------------------------------- specs

#[test]
fn your_pc_cells_and_copy_all() {
    let pc = PcSpecs {
        cpu: CpuSpec { name: "Ryzen".into(), cores: Some(16), threads: Some(32), threads_in_use: Some(32), max_mhz: Some(4200) },
        gpus: vec![
            GpuSpec { name: "iGPU".into(), vram_bytes: Some(512 * MB), integrated: true, driver: None },
            GpuSpec { name: "RTX".into(), vram_bytes: Some(24 * 1024 * MB), integrated: false, driver: Some("32.0".into()) },
        ],
        ram: RamSpec { total_bytes: 64 * 1024 * MB, kind: Some("DDR5".into()), speed_mts: Some(6000), rated_mts: Some(4800), sticks: 2, xmp_expo: Some(true) },
        drives: vec![DriveSpec { model: "SSD".into(), size_bytes: 1024 * 1024 * MB, media: Some("SSD".into()), bus: Some("NVMe".into()) }],
        displays: vec![DisplaySpec { name: "Mon".into(), width: 3440, height: 1440, hz: 239.96 }],
        windows: WindowsSpec { edition: "Windows 11 Pro".into(), version: Some("24H2".into()), build: Some("26100.1".into()), install_date: None },
        ..Default::default()
    };
    let cells = pc.cells();
    // Order 021: the drawing's 8 facts and words (menu-v22 SPECS)
    assert_eq!(cells.iter().map(|c| c.label).collect::<Vec<_>>(), ["CPU", "GPU", "RAM", "Motherboard", "Drives", "Displays", "Network", "Windows"]);
    assert_eq!(cells[0].quiet, "16 cores · 32 threads · 4.2 GHz");
    assert_eq!(cells[1].value, "RTX", "the card is shown, not the integrated GPU");
    assert_eq!(cells[1].quiet, "24 GB · driver 32.0 · +1 more");
    assert_eq!(cells[2].value, "64 GB DDR5 · 6000 MT/s");
    assert_eq!(cells[2].quiet, "2 × 32 GB · XMP on");
    assert_eq!(cells[3].value, "—", "unknown motherboard → —");
    assert_eq!(cells[4].value, "SSD · 1.1 TB NVMe", "sold (decimal) size: 1 TiB = 1.1 TB");
    assert_eq!(cells[5].value, "Mon · 3440 × 1440 · 240 Hz");
    assert_eq!(cells[6].value, "—", "no adapter read → —");
    assert_eq!(cells[7].value, "Windows 11 Pro · 24H2");
    assert_eq!(cells[7].quiet, "Build 26100.1");
    let text = pc.copy_text();
    assert!(text.starts_with("CPU: Ryzen (16 cores · 32 threads · 4.2 GHz)
GPU: RTX"));
    assert!(text.contains("
Motherboard: —
"));
    assert_eq!(PcSpecs::default().cells()[0].value, "—");
    let mut limited = pc.clone();
    limited.cpu.threads_in_use = Some(16);
    assert_eq!(limited.cells()[0].quiet, "16 cores · 32 threads · 4.2 GHz · Windows uses 16 threads", "a CCD off / processors limited is said");
    // the drawing's sample PC, word for word
    let d = PcSpecs {
        cpu: CpuSpec { name: "AMD Ryzen 7 7800X3D".into(), cores: Some(8), threads: Some(16), threads_in_use: Some(16), max_mhz: Some(5000) },
        gpus: vec![GpuSpec { name: "NVIDIA GeForce RTX 4070 SUPER".into(), vram_bytes: Some(12 * 1024 * MB), integrated: false, driver: Some("32.0.15.8142".into()) }],
        ram: RamSpec { total_bytes: 32 * 1024 * MB, kind: Some("DDR5".into()), speed_mts: Some(6000), rated_mts: Some(4800), sticks: 2, xmp_expo: Some(true) },
        board: specs::BoardSpec { maker: Some("ASUS".into()), model: Some("ROG STRIX B650E-F GAMING WIFI".into()), bios_version: Some("3263".into()), bios_date: Some("2025-01-10".into()) },
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
            specs::NetSpec { name: "Intel(R) Wi-Fi 6E AX210 160MHz".into(), speed_bps: None, wireless: true, connected: false },
            specs::NetSpec { name: "Intel(R) Ethernet Controller I226-V".into(), speed_bps: Some(2_500_000_000), wireless: false, connected: true },
        ],
        windows: WindowsSpec { edition: "Windows 11 Pro".into(), version: Some("24H2".into()), build: Some("26100.6584".into()), install_date: Some("2026-02-03".into()) },
    };
    let c: Vec<(String, String)> = d.cells().into_iter().map(|c| (c.value, c.quiet)).collect();
    assert_eq!(c[0], ("AMD Ryzen 7 7800X3D".into(), "8 cores · 16 threads · 5.0 GHz".into()));
    assert_eq!(specs::cpu_name("AMD Ryzen 7 7800X3D 8-Core Processor"), "AMD Ryzen 7 7800X3D");
    assert_eq!(specs::cpu_name("Intel(R) Core(TM) i7-14700K"), "Intel Core i7-14700K");
    assert_eq!(c[1], ("NVIDIA GeForce RTX 4070 SUPER".into(), "12 GB · driver 581.42".into()));
    assert_eq!(c[2], ("32 GB DDR5 · 6000 MT/s".into(), "2 × 16 GB · EXPO on".into()));
    assert_eq!(c[3], ("ASUS ROG STRIX B650E-F GAMING WIFI".into(), "BIOS 3263".into()));
    assert_eq!(c[4], ("Samsung 990 PRO · 2 TB NVMe".into(), "WD_BLACK SN850X 2 TB · Seagate BarraCuda 4 TB".into()));
    assert_eq!(c[5], ("DELL S2721DGF · 1920 × 1080 · 165 Hz".into(), "LG 24GL600F · 1920 × 1080 · 144 Hz".into()));
    assert_eq!(c[6], ("Intel Ethernet I226-V · 2.5 Gbps".into(), "Wi-Fi 6E · Intel AX210".into()));
    assert_eq!(c[7], ("Windows 11 Pro · 24H2".into(), "Build 26100.6584 · installed 3 Feb 2026".into()));
    assert_eq!(specs::xmp_from_speeds(Some(6000), Some(4800)), Some(true));
    assert_eq!(specs::xmp_from_speeds(Some(4800), Some(4800)), None);
    assert_eq!(specs::memory_type_name(34), Some("DDR5"));
    let os = FakeOs::new();
    os.with_specs(pc.clone());
    assert_eq!(specs::read(&os).unwrap(), pc);
}

// ---------------------------------------------------------------- processes

fn p(pid: u32, parent: u32, exe: &str, path: Option<&str>, user: ProcessUser, window: bool) -> RawProcess {
    RawProcess {
        pid,
        parent_pid: parent,
        exe: exe.into(),
        path: path.map(PathBuf::from),
        description: None,
        session_id: if user == ProcessUser::System { 0 } else { 1 },
        user,
        create_time: pid as u64 * 10,
        cpu_time: 0,
        cycle_time: 0,
        ram_bytes: 100 * MB,
        priority: Priority::Normal,
        has_window: window,
    }
}

fn table() -> Vec<RawProcess> {
    use ProcessUser::*;
    let mut v = vec![
        p(0, 0, "Idle", None, System, false),
        p(4, 0, "System", None, System, false),
        p(500, 4, "csrss.exe", None, System, false),
        p(900, 4, "svchost.exe", Some(r"C:\Windows\System32\svchost.exe"), System, false),
        p(950, 4, "MsMpEng.exe", Some(r"C:\ProgramData\Microsoft\Windows Defender\Platform\MsMpEng.exe"), System, false),
        p(960, 4, "AdobeUpdateService.exe", None, System, false),
        p(1000, 900, "explorer.exe", Some(r"C:\Windows\explorer.exe"), Me, true),
        p(2000, 1000, "chrome.exe", Some(r"C:\Apps\Chrome\chrome.exe"), Me, true),
        p(2001, 2000, "chrome.exe", Some(r"C:\Apps\Chrome\chrome.exe"), Me, false),
        p(2002, 2000, "chrome.exe", Some(r"C:\Apps\Chrome\chrome.exe"), Me, false),
        p(2100, 2001, "crashpad.exe", Some(r"C:\Apps\Chrome\crashpad.exe"), Me, false),
        p(3000, 1000, "notepad.exe", Some(r"C:\Apps\notepad.exe"), Me, true),
        p(3100, 1000, "nvcontainer.exe", Some(r"C:\Program Files\NVIDIA\nvcontainer.exe"), Me, false),
        p(3200, 1000, "vgtray.exe", Some(r"C:\Program Files\Riot Vanguard\vgtray.exe"), Me, true),
        p(3300, 1000, "tool.exe", Some(r"C:\Apps\tool.exe"), OtherUser, true),
        p(3400, 1000, "VALORANT-Win64-Shipping.exe", Some(r"C:\Riot Games\VALORANT-Win64-Shipping.exe"), Me, true),
    ];
    v[7].description = Some("Google Chrome".into());
    v
}

fn monitor() -> (FakeOs, ProcessMonitor) {
    let os = FakeOs::new();
    os.with_processes(table()).with_cpu_count(4);
    (os, ProcessMonitor::new(Path::new(r"C:\Windows")))
}

fn row<'a>(rows: &'a [processes::ProcessRow], name: &str) -> &'a processes::ProcessRow {
    rows.iter().find(|r| r.name == name).unwrap_or_else(|| panic!("no row {name}"))
}

#[test]
fn list_groups_apps_hides_windows_and_works_out_cpu() {
    let (os, mut mon) = monitor();
    let t0 = Instant::now();
    let rows = mon.refresh_at(&os, &HashMap::new(), false, t0).unwrap();
    assert!(rows.iter().all(|r| r.cpu_pct == 0.0), "first refresh: no CPU % yet");
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    for hidden in ["Idle", "System", "csrss", "svchost", "MsMpEng", "AdobeUpdateService"] {
        assert!(!names.contains(&hidden), "{hidden} must be hidden by default");
    }
    let chrome = row(&rows, "Google Chrome");
    assert_eq!(chrome.pids, vec![2000, 2001, 2002], "main process first");
    assert_eq!(chrome.display_name(), "Google Chrome (3)");
    assert_eq!(chrome.ram_bytes, 300 * MB);
    assert_eq!(row(&rows, "notepad").display_name(), "notepad");

    // One second later: chrome used 0.4 s + 0.4 s of CPU on 4 logical CPUs → 20 %.
    let mut t = table();
    for x in t.iter_mut().filter(|x| x.exe == "chrome.exe" && x.pid != 2002) {
        x.cpu_time += 4_000_000;
    }
    t.iter_mut().find(|x| x.pid == 3000).unwrap().cpu_time += 400_000; // notepad 1 %
    os.with_processes(t);
    let gpu = HashMap::from([(2001u32, 12.5), (2002, 3.0)]);
    let rows = mon.refresh_at(&os, &gpu, false, t0 + Duration::from_secs(1)).unwrap();
    let chrome = row(&rows, "Google Chrome");
    assert!((chrome.cpu_pct - 20.0).abs() < 1e-9, "{}", chrome.cpu_pct);
    assert_eq!(chrome.gpu_pct, 12.5, "busiest helper's GPU");
    assert!((row(&rows, "notepad").cpu_pct - 1.0).abs() < 1e-9);
    assert_eq!(rows[0].name, "Google Chrome", "default sort: CPU, highest first");
    assert_eq!(chrome.bright(), (true, false, true));

    // The switch shows Windows' own, locked.
    let all = mon.refresh_at(&os, &gpu, true, t0 + Duration::from_secs(2)).unwrap();
    let svc = row(&all, "svchost");
    assert!(svc.windows_own && svc.end_rule == EndRule::Locked);
    assert_eq!(svc.menu_title(), "Part of Windows · it can’t be ended");
    assert!(row(&all, "MsMpEng").windows_own, "Defender is Windows' own");
    assert!(row(&all, "csrss").windows_own);
    let adobe = row(&all, "AdobeUpdateService");
    assert!(adobe.protected && !adobe.windows_own && adobe.end_rule == EndRule::Locked);
    assert!(all.iter().all(|r| r.name != "Idle"), "Idle is never a row");
}

#[test]
fn end_rules_plain_app_ask_first_and_why_lines() {
    let (os, mut mon) = monitor();
    let rows = mon.refresh(&os, &HashMap::new(), false).unwrap();
    assert_eq!(row(&rows, "notepad").end_rule, EndRule::Instant);
    let why = |name: &str| match &row(&rows, name).end_rule {
        EndRule::AskFirst { why } => why.clone(),
        other => panic!("{name}: {other:?}"),
    };
    assert_eq!(why("explorer"), "Your taskbar and desktop vanish until Windows starts it again.");
    assert_eq!(why("nvcontainer"), "A background part of the graphics driver. Its overlay and recording stop until you restart.");
    assert_eq!(why("vgtray"), "Anti-cheat. Ending it only stops VALORANT from working until you restart the PC.");
    assert!(why("crashpad").contains("no window"));
    assert!(why("tool").contains("not as you"));
    assert!(row(&rows, "tool").needs_admin, "another user's process needs admin when we aren't");
    assert!(!row(&rows, "explorer").windows_own, "Explorer lives in C:\\Windows but is endable (asks first)");
}

#[test]
fn end_task_end_tree_locked_and_confirm() {
    let (os, mut mon) = monitor();
    let rows = mon.refresh(&os, &HashMap::new(), true).unwrap();
    // Plain app: closes its window politely.
    let r = mon.end_task(&os, row(&rows, "notepad"), false).unwrap();
    assert_eq!(r.ended, vec![3000]);
    assert_eq!(os.actions(), vec!["end 3000 Close"]);
    assert_eq!(r.toast("notepad"), "notepad closed");
    // Ask-first rows refuse without the confirm.
    assert!(matches!(mon.end_task(&os, row(&rows, "explorer"), false), Err(PerfError::Refused(_))));
    // Locked: never, even confirmed.
    assert!(matches!(mon.end_task(&os, row(&rows, "svchost"), true), Err(PerfError::Locked(_))));
    assert!(matches!(mon.end_tree(&os, row(&rows, "csrss"), true), Err(PerfError::Locked(_))));
    // Chrome: window closed on the main one, helpers terminated.
    let r = mon.end_task(&os, row(&rows, "Google Chrome"), false).unwrap();
    assert_eq!(r.toast("Google Chrome"), "Google Chrome and its 2 helper processes closed");
    assert!(os.actions().contains(&"end 2000 Close".to_string()));
    assert!(os.actions().contains(&"end 2001 Terminate".to_string()));
    assert!(!os.actions().iter().any(|a| a.starts_with("end 2100")), "End task leaves other apps' children");
}

#[test]
fn end_tree_children_first_never_windows_own_and_needs_admin() {
    let (os, mut mon) = monitor();
    os.with_protected(3300);
    let rows = mon.refresh(&os, &HashMap::new(), true).unwrap();
    let r = mon.end_tree(&os, row(&rows, "Google Chrome"), false).unwrap();
    let order: Vec<String> = os.actions();
    assert_eq!(order, vec!["end 2100 Terminate", "end 2002 Terminate", "end 2001 Terminate", "end 2000 Terminate"]);
    assert_eq!(r.ended.len(), 4);
    // Explorer's tree: its children (chrome is gone now), never svchost (its parent) or Windows' own.
    let rows = mon.refresh(&os, &HashMap::new(), true).unwrap();
    let r = mon.end_tree(&os, row(&rows, "explorer"), true).unwrap();
    assert!(r.needs_admin.contains(&3300), "other user's child: access denied → needs admin");
    assert!(!os.actions().iter().any(|a| a.contains(" 900 ") || a.contains(" 4 ")));
    assert_eq!(*os.actions().last().unwrap(), "end 1000 Terminate", "the main process last");
}

#[test]
fn priority_set_undo_and_refusals() {
    let (os, mut mon) = monitor();
    let rows = mon.refresh(&os, &HashMap::new(), true).unwrap();
    let chrome = row(&rows, "Google Chrome");
    let undo = mon.set_priority(&os, chrome, Priority::High).unwrap();
    assert_eq!(undo.old, vec![(2000, Priority::Normal), (2001, Priority::Normal), (2002, Priority::Normal)]);
    let rows = mon.refresh(&os, &HashMap::new(), true).unwrap();
    assert_eq!(row(&rows, "Google Chrome").priority, Priority::High, "the pill shows High");
    undo.undo(&os).unwrap();
    let rows = mon.refresh(&os, &HashMap::new(), true).unwrap();
    assert_eq!(row(&rows, "Google Chrome").priority, Priority::Normal, "undone");
    assert_eq!(PriorityUndo::toast("Google Chrome", Priority::AboveNormal), "Google Chrome · priority above normal until it closes");

    assert!(matches!(mon.set_priority(&os, row(&rows, "notepad"), Priority::Realtime), Err(PerfError::Refused(_))));
    assert!(matches!(mon.set_priority(&os, row(&rows, "svchost"), Priority::Low), Err(PerfError::Locked(_))));
    assert!(matches!(mon.set_priority(&os, row(&rows, "vgtray"), Priority::Low), Err(PerfError::Refused(_))));
    // VALORANT: allowed while Vanguard's vgc isn't running, refused when it is.
    let val = row(&rows, "VALORANT-Win64-Shipping").clone();
    assert!(mon.set_priority(&os, &val, Priority::AboveNormal).is_ok());
    let mut t = table();
    t.push(p(77, 4, "vgc.exe", Some(r"C:\Program Files\Riot Vanguard\vgc.exe"), ProcessUser::System, false));
    os.with_processes(t);
    let rows = mon.refresh(&os, &HashMap::new(), true).unwrap();
    assert!(matches!(mon.set_priority(&os, row(&rows, "VALORANT-Win64-Shipping"), Priority::High), Err(PerfError::Refused(_))));
    // Admin path: access denied → NeedsAdmin, nothing changed.
    os.with_protected(3000);
    assert!(matches!(mon.set_priority(&os, row(&rows, "notepad"), Priority::High), Err(PerfError::NeedsAdmin(_))));
    assert_eq!(Priority::MENU.len(), 5);
    assert!(!Priority::MENU.contains(&Priority::Realtime));
    assert_eq!(Priority::from_base(13), Priority::High);
    assert_eq!(Priority::from_base(4), Priority::Low);
}

#[test]
fn sort_search_formats_open_location_and_icon() {
    let (os, mut mon) = monitor();
    let mut rows = mon.refresh(&os, &HashMap::new(), false).unwrap();
    processes::sort(&mut rows, SortBy::Name, false);
    assert_eq!(rows[0].name, "crashpad");
    processes::sort(&mut rows, SortBy::Name, true);
    assert_eq!(rows[0].name, "vgtray");
    assert_eq!(processes::filter(&rows, "CHROME").len(), 1);
    assert_eq!(processes::filter(&rows, "nothing-like-this").len(), 0);
    assert_eq!(processes::filter(&rows, "").len(), rows.len());
    assert_eq!(format_pct(3.04), "3.0 %");
    assert_eq!(format_ram(512 * MB), "512 MB");
    assert_eq!(format_ram(1000 * MB), "1.0 GB");
    mon.open_file_location(&os, row(&rows, "notepad")).unwrap();
    assert_eq!(os.actions(), vec![r"open C:\Apps\notepad.exe"]);
    let icon = os.icon_rgba(Path::new(r"C:\Apps\notepad.exe"), 32).unwrap();
    assert_eq!(icon.rgba.len(), 32 * 32 * 4);
}

#[test]
fn cpu_share_counts_cycles_when_known_else_cpu_time() {
    // 1 s on 4 processors = 4e7 x 100 ns; cycles: idle 600, a 300, b 100 (of 1000) -> 30 % / 10 % even though
    // b was charged no CPU time (it ran between clock ticks)
    let s = processes::cpu_shares(&[(0, 0, 600), (10, 1_000_000, 300), (11, 0, 100)], 4e7, 4.0);
    assert!((s[&10] - 30.0).abs() < 1e-9 && (s[&11] - 10.0).abs() < 1e-9, "{s:?}");
    // no cycle counts (fake / unknown): CPU time over dt x processors
    let s = processes::cpu_shares(&[(10, 16_000_000, 0), (11, 0, 0)], 4e7, 4.0);
    assert!((s[&10] - 10.0).abs() < 1e-9 && s[&11] == 0.0, "{s:?}");
    // first refresh (no interval yet): nothing
    assert!(processes::cpu_shares(&[(10, 5, 5)], 0.0, 4.0).is_empty());
}

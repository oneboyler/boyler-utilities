//! Prints the REAL Performance state, READ-ONLY: "Your PC", 5 s of live tiles (sampler started then stopped), the
//! process list (top 15 by CPU), one icon read. Nothing is ended, no priority is changed.
//!   cargo run -p bu-perf --example perf-show
//! The user's folder name is printed as <user>.

use bu_perf::live::{self, Sampler, SamplerOptions};
use bu_perf::processes::{format_pct, format_ram, EndRule, ProcessMonitor};
use bu_perf::{specs, PerfOs, RealOs};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn anon(s: &str) -> String {
    match std::env::var("USERNAME") {
        Ok(u) if !u.is_empty() => s.replace(&u, "<user>"),
        _ => s.to_string(),
    }
}

fn main() {
    let os = Arc::new(RealOs::read_only());
    println!("elevated (admin): {}", os.is_elevated());

    println!("\n== Your PC");
    let t0 = Instant::now();
    let pc = specs::read(os.as_ref()).expect("specs");
    for c in pc.cells() {
        println!("{:<12} {:<52} {}", c.label, c.value, c.quiet);
    }
    println!("(read in {} ms)\n-- Copy all:\n{}", t0.elapsed().as_millis(), pc.copy_text());

    println!("\n== Live (sampler runs 5 s, then stops)");
    let t0 = Instant::now();
    let s = Sampler::start(os.clone(), SamplerOptions::default()).expect("sampler");
    println!("(started in {} ms)", t0.elapsed().as_millis());
    std::thread::sleep(Duration::from_millis(5200));
    let hist = s.history();
    for t in &hist {
        let r = &t.reading;
        let g = live::main_gpu(r);
        let d = live::main_disk(r, 'C');
        let (ram, ram_extra) = live::ram_text(r.ram_used_bytes, r.ram_total_bytes);
        println!(
            "t={:>4.1}s CPU {:>5.1} % {} | GPU {} {:>5.1} % {} {} | RAM {} {} | Disk {:?} {:>5.1} % {} | Net ↓ {} ↑ {}",
            t.since_start.as_secs_f64(),
            r.cpu_usage_pct,
            live::format_ghz(r.cpu_mhz),
            g.map(|g| g.name.as_str()).unwrap_or("-"),
            g.map(|g| g.usage_pct).unwrap_or(0.0),
            g.and_then(|g| g.temperature_c).map(|t| format!("{t:.0} °C")).unwrap_or("— °C".into()),
            g.map(live::gpu_extra).unwrap_or_default(),
            ram,
            ram_extra,
            d.map(|d| &d.letters),
            d.map(|d| d.active_pct).unwrap_or(0.0),
            live::format_mb_per_s(d.map(|d| d.bytes_per_sec).unwrap_or(0.0)),
            live::format_mbps(r.net_down_bps),
            live::format_mbps(r.net_up_bps),
        );
    }
    if let Some(last) = hist.last() {
        for g in &last.reading.gpus {
            println!(
                "   GPU {:<28} {:>5.1} %  temp {:?}  fan {:?} % / {:?} rpm  integrated {}  {}",
                g.name, g.usage_pct, g.temperature_c, g.fan_pct, g.fan_rpm, g.integrated, live::gpu_extra(g)
            );
        }
        for d in &last.reading.disks {
            println!("   disk {:<8} active {:>5.1} %  {}", d.instance, d.active_pct, live::format_mb_per_s(d.bytes_per_sec));
        }
    }
    println!("CPU temperature: {:?}", live::cpu_temperature());
    let reads = s.reads();
    let gpu_by_pid = hist.last().map(|t| t.reading.gpu_by_pid.clone()).unwrap_or_default();
    s.stop();
    println!("sampler stopped after {reads} readings (no thread left running)");

    println!("\n== Processes (top 15 by CPU, Windows' own hidden)");
    let mut mon = ProcessMonitor::for_this_pc();
    let t0 = Instant::now();
    let _ = mon.refresh(os.as_ref(), &gpu_by_pid, false).expect("processes");
    let first_ms = t0.elapsed().as_millis();
    std::thread::sleep(Duration::from_millis(1000));
    let t0 = Instant::now();
    let rows = mon.refresh(os.as_ref(), &gpu_by_pid, false).expect("processes");
    let second_ms = t0.elapsed().as_millis();
    let all = mon.refresh(os.as_ref(), &gpu_by_pid, true).expect("processes");
    println!(
        "{} app rows ({} with Windows' own); first refresh {first_ms} ms (reads names/paths once), next {second_ms} ms",
        rows.len(),
        all.len()
    );
    for r in rows.iter().take(15) {
        let rule = match &r.end_rule {
            EndRule::Instant => "End at once".to_string(),
            EndRule::AskFirst { why } => format!("asks first: {why}"),
            EndRule::Locked => "locked".to_string(),
        };
        println!(
            "  {:<34} {:>7} {:>8} {:>7}  {:?}{}  [{}]",
            anon(&r.display_name()),
            format_pct(r.cpu_pct),
            format_ram(r.ram_bytes),
            format_pct(r.gpu_pct),
            r.user,
            if r.priority != bu_perf::Priority::Normal { format!(" prio {}", r.priority.name()) } else { String::new() },
            rule
        );
    }
    println!("  Windows' own (shown with the switch), first 8:");
    for r in all.iter().filter(|r| r.windows_own).take(8) {
        println!("  {:<34} {:>7} {:>8}  locked: {}", r.display_name(), format_pct(r.cpu_pct), format_ram(r.ram_bytes), r.menu_title());
    }

    println!("\n== Icon (read-only)");
    let np = Path::new(r"C:\Windows\System32\notepad.exe");
    match os.icon_rgba(np, 32) {
        Ok(i) => println!("notepad.exe icon {}×{}, {} bytes RGBA, {} non-transparent pixels", i.width, i.height, i.rgba.len(), i.rgba.chunks(4).filter(|c| c[3] > 0).count()),
        Err(e) => println!("icon failed: {e}"),
    }
}

//! Prints the REAL Storage state, READ-ONLY: drive tiles, drive health, clean-up sizes (measured, nothing deleted).
//!   cargo run -p bu-storage --example storage-show
//!   cargo run -p bu-storage --release --example storage-show -- scan C [threads]   (also walks C: and prints time / CPU / RAM)
//! The user's folder name is printed as <user>.

use bu_storage::cleanup::{self, PartState};
use bu_storage::drives::{self, format_bytes};
use bu_storage::scan::{self, RowKind, ScanControl};
use bu_storage::{health, RealOs, StorageOs};
use std::time::Instant;

fn anon(s: &str) -> String {
    match std::env::var("USERNAME") {
        Ok(u) if !u.is_empty() => s.replace(&u, "<user>"),
        _ => s.to_string(),
    }
}

fn main() {
    let os = RealOs::read_only();
    println!("elevated (admin): {}", os.is_elevated());

    println!("\n== Drive tiles");
    let tiles = drives::list(&os).expect("drives");
    for t in &tiles {
        println!(
            "{:<18} {:>9} free of {:>9}  used {:>5.1} %{}  [{:?}]  hover: {}",
            t.title(),
            format_bytes(t.info.free_bytes),
            format_bytes(t.info.total_bytes),
            t.used_fraction * 100.0,
            if t.low_space { "  LOW (amber)" } else { "" },
            t.info.state,
            t.hover()
        );
    }
    println!("chosen on open: {:?}", drives::default_choice(&tiles));

    println!("\n== Drive health (read-only)");
    let t0 = Instant::now();
    for r in health::read_all(&os).expect("health") {
        println!(
            "disk {} {:<26} letters {:<8} temp {:>6} life {:>5} hours {:>7}  {}  os={:?}",
            r.disk_number,
            r.model,
            r.letters.iter().map(|l| format!("{l}:")).collect::<Vec<_>>().join(" "),
            r.temperature_c.map(|t| format!("{t} °C")).unwrap_or("—".into()),
            r.life_left_pct.map(|l| format!("{l} %")).unwrap_or("—".into()),
            r.power_on_hours.map(|h| h.to_string()).unwrap_or("—".into()),
            r.status_text(),
            r.os_status
        );
        for w in &r.warnings {
            println!("      warning: {w}");
        }
        if !r.admin_would_add.is_empty() {
            println!("      admin would add: {}", r.admin_would_add.join(", "));
        }
    }
    println!("(health read in {} ms)", t0.elapsed().as_millis());

    println!("\n== Clean up — sizes only (nothing is deleted)");
    let t0 = Instant::now();
    let plan = cleanup::measure(&os).expect("measure");
    for row in &plan.rows {
        println!("{:<16} {:>10}  ({} items){}", row.kind.name(), format_bytes(row.bytes), row.items, {
            let n = row.notes();
            if n.is_empty() { String::new() } else { format!("  — {}", n.join(", ")) }
        });
        for p in &row.parts {
            if p.state != PartState::Missing {
                println!("    {:<28} {:>10}  {:?}  {}", p.name, format_bytes(p.bytes), p.state, anon(&p.path.display().to_string()));
            }
        }
    }
    println!("\"Clean {}\" with the default ticks; measured in {} ms", format_bytes(plan.ticked_bytes(&plan.default_ticked())), t0.elapsed().as_millis());

    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("scan") {
        let letter = args.get(2).and_then(|s| s.chars().next()).unwrap_or('C');
        println!("\n== What's using {letter}: (walk, read-only)");
        let ctl = ScanControl::new();
        let t0 = Instant::now();
        let cpu0 = process_cpu();
        let threads = args.get(3).and_then(|s| s.parse().ok());
        let r = scan::scan_drive_with(&os, letter, threads, &ctl).expect("scan");
        let wall = t0.elapsed();
        let cpu = process_cpu() - cpu0;
        println!(
            "walked {} files in {} folders, {} unreadable folders, {} links not followed, {} threads",
            r.stats.files, r.stats.folders, r.stats.unreadable_folders, r.stats.links_skipped, r.stats.threads
        );
        println!(
            "TIME {:.1} s · CPU time {:.1} s (= {:.0} % of one core on average) · peak RAM {} MB",
            wall.as_secs_f64(),
            cpu,
            cpu / wall.as_secs_f64() * 100.0,
            peak_ram_mb()
        );
        println!("walked total {} · used (drive) {} · unseen {:?} bytes", format_bytes(r.types.walked_total()), format_bytes(r.types.used_bytes.unwrap_or(0)), r.types.unseen_bytes());
        for t in r.types.rows() {
            println!("  {:<16} {:>10} {:>5.1} %", t.ty.name(), format_bytes(t.bytes), t.share * 100.0);
        }
        println!("  Folders of {letter}: (biggest first)");
        for row in r.tree.rows(r.tree.root()).expect("rows").iter().take(12) {
            let tag = match row.kind {
                RowKind::Folder { windows_own: true, .. } => " [lock: Windows' own]",
                RowKind::Folder { unreadable: true, .. } => " [can't be read]",
                RowKind::Folder { has_subfolders: true, .. } => " ›",
                _ => "",
            };
            println!("    {:<34} {:>10} {:>5.1} %{tag}", anon(&row.name), format_bytes(row.bytes), row.share * 100.0);
        }
    }
}

/// This process's CPU time so far (user + kernel), seconds.
fn process_cpu() -> f64 {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe {
        let _ = GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u);
    }
    let f = |t: FILETIME| ((t.dwHighDateTime as u64) << 32 | t.dwLowDateTime as u64) as f64 / 1e7;
    f(k) + f(u)
}

/// Peak working set of this process, MB.
fn peak_ram_mb() -> u64 {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut m = PROCESS_MEMORY_COUNTERS { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
    unsafe {
        let _ = GetProcessMemoryInfo(GetCurrentProcess(), &mut m, m.cb);
    }
    m.PeakWorkingSetSize as u64 / (1024 * 1024)
}

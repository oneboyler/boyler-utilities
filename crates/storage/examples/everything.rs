//! `cargo run -p bu-storage --example storage-everything -- <drive letter> [--walk]` - Order 069: measure one drive the way the
//! app's Measure does: through our Everything when it is set up (it is started for the measure and quit again), else the walk.
//! `--walk` measures with the walk instead (the read-only layer has no listing) to compare. Reads only - the one thing it
//! starts is our own hidden Everything instance, which it quits again. Paths shown have the user name as `<user>`.

use bu_storage::scan::{self, ScanControl};
use bu_storage::{RealOs, StorageOs};
use std::time::Instant;

fn working_set_mb() -> f64 {
    use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut c = PROCESS_MEMORY_COUNTERS { cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
    unsafe {
        let _ = K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb);
    }
    c.WorkingSetSize as f64 / 1_048_576.0
}

fn anon(s: &str) -> String {
    match std::env::var("USERNAME") {
        Ok(u) if !u.is_empty() => s.replace(&u, "<user>"),
        _ => s.to_string(),
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let letter = a.first().and_then(|s| s.chars().next()).unwrap_or('C');
    let walk = a.iter().any(|x| x == "--walk");
    let os: Box<dyn StorageOs> = if walk { Box::new(RealOs::read_only()) } else { Box::new(RealOs::new()) };
    println!("measuring {letter}: with the {} (working set at start {:.0} MB)", if walk { "walk" } else { "listing when there is one" }, working_set_mb());
    if let Some(i) = a.iter().position(|x| x == "--orphans") {
        orphans(os.as_ref(), letter, a.get(i + 1).map(|s| s.as_str()).unwrap_or(""));
        return;
    }
    if let Some(i) = a.iter().position(|x| x == "--compare") {
        compare(os.as_ref(), letter, a.get(i + 1).map(|s| s.as_str()).unwrap_or(""));
        return;
    }
    let ctl = std::sync::Arc::new(ScanControl::new());
    // `--stop-after <ms>`: press Stop that long after the start (proves Everything is quit again)
    if let Some(ms) = a.iter().position(|x| x == "--stop-after").and_then(|i| a.get(i + 1)).and_then(|s| s.parse::<u64>().ok()) {
        let c = ctl.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(ms));
            c.cancel();
        });
    }
    let t = Instant::now();
    let r = match scan::scan_drive(os.as_ref(), letter, &ctl) {
        Ok(r) => r,
        Err(e) => {
            println!("failed after {:?}: {e}", t.elapsed());
            return;
        }
    };
    println!("done in {:?} (the scan itself said {:?}, {} thread(s)); working set now {:.0} MB", t.elapsed(), r.stats.elapsed, r.stats.threads, working_set_mb());
    println!("  {} files, {} folders, {} unreadable, tree {:.1} MB in memory", r.stats.files, r.stats.folders, r.stats.unreadable_folders, r.tree.heap_bytes() as f64 / 1_048_576.0);
    for t in r.types.rows() {
        println!("  {:<16} {:>10.1} GB", t.ty.name(), t.bytes as f64 / 1_073_741_824.0);
    }
    println!("  walked in all {:.1} GB; the biggest files:", r.types.walked_total() as f64 / 1_073_741_824.0);
    for b in r.biggest.iter().take(5) {
        println!("    {:>8.1} GB  {}", b.bytes as f64 / 1_073_741_824.0, anon(&b.path().display().to_string()));
    }
    println!("  top folders:");
    for row in r.tree.rows(r.tree.root()).unwrap_or_default().iter().take(8) {
        println!("    {:>8.1} GB  {}", row.bytes as f64 / 1_073_741_824.0, anon(&row.name));
    }
    let pruned = r.tree.pruned();
    println!("  pruned for the menu closing: {:.2} MB", pruned.heap_bytes() as f64 / 1_048_576.0);
}

/// `--compare <folder>`: every file of that folder as the listing gives it against what the disk says now (a sample of the
/// mismatches is shown).
#[allow(dead_code)]
pub fn compare(os: &dyn StorageOs, letter: char, prefix: &str) {
    let ctl = ScanControl::new();
    let Some(Ok(mut l)) = os.open_listing(letter, &ctl) else {
        println!("no listing");
        return;
    };
    let want = prefix.to_lowercase();
    let (mut n, mut diff, mut ev_sum, mut fs_sum) = (0u64, 0u64, 0u64, 0u64);
    let mut shown = 0;
    loop {
        let page = l.page(false, 100_000).unwrap();
        if page.is_empty() {
            break;
        }
        for e in page.into_iter().filter(|e| e.dir.to_lowercase().starts_with(&want)) {
            let p = std::path::Path::new(&e.dir).join(&e.name);
            let Ok(m) = std::fs::symlink_metadata(&p) else { continue };
            n += 1;
            ev_sum += e.size;
            fs_sum += m.len();
            if m.len() != e.size {
                diff += 1;
                if shown < 6 {
                    shown += 1;
                    println!("    {} everything {} disk {}", anon(&p.display().to_string()), e.size, m.len());
                }
            }
        }
    }
    println!("  {n} files under {prefix}: listing {:.2} GB, disk {:.2} GB, {diff} differ", ev_sum as f64 / 1e9, fs_sum as f64 / 1e9);
}

/// `--orphans <folder>`: files of that folder whose directory is not among the listing's folders.
#[allow(dead_code)]
pub fn orphans(os: &dyn StorageOs, letter: char, prefix: &str) {
    let ctl = ScanControl::new();
    let Some(Ok(mut l)) = os.open_listing(letter, &ctl) else {
        println!("no listing");
        return;
    };
    let norm = |s: &str| s.replace('/', "\\").to_lowercase().trim_end_matches('\\').to_string();
    let mut dirs = std::collections::HashSet::new();
    dirs.insert(norm(&format!("{letter}:")));
    loop {
        let page = l.page(true, 100_000).unwrap();
        if page.is_empty() {
            break;
        }
        for e in page {
            if !e.dir.is_empty() {
                dirs.insert(format!("{}\\{}", norm(&e.dir), norm(&e.name)));
            }
        }
    }
    println!("  {} folders", dirs.len());
    let want = prefix.to_lowercase();
    let (mut n, mut missing, mut bytes) = (0u64, 0u64, 0u64);
    let mut seen = std::collections::HashSet::new();
    loop {
        let page = l.page(false, 100_000).unwrap();
        if page.is_empty() {
            break;
        }
        for e in page.into_iter().filter(|e| e.dir.to_lowercase().starts_with(&want)) {
            n += 1;
            if !dirs.contains(&norm(&e.dir)) {
                missing += 1;
                bytes += e.size;
                if seen.len() < 5 && seen.insert(e.dir.clone()) {
                    println!("    missing dir: {}", anon(&e.dir));
                }
            }
        }
    }
    println!("  {n} files under {prefix}, {missing} in folders the listing does not have ({:.2} GB)", bytes as f64 / 1e9);
}

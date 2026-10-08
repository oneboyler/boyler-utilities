//! Prints the REAL Quick fixes state, READ-ONLY: admin or not, the display adapters, the Explorer cache files, System
//! Restore (frequency, newest point), the end of CBS.log. Then shows that every change is REFUSED by the read-only
//! layer (no key chord, no DISM/sfc/pnputil, no file deleted, Explorer not stopped, no restore point).
//!   cargo run -p bu-quickfix --example quickfix-show
//! The user's folder name is printed as <user>.

use bu_quickfix::real::RealOs;
use bu_quickfix::{cache, gfx, repair, restore, FixOs};
use std::sync::Arc;
use std::time::Instant;

fn anon(s: &str) -> String {
    match std::env::var("USERNAME") {
        Ok(u) if !u.is_empty() => s.replace(&u, "<user>"),
        _ => s.to_string(),
    }
}

fn main() {
    let os = Arc::new(RealOs::read_only());
    println!("elevated (admin): {}", os.is_elevated());
    let now = os.local(os.now());
    println!("now (local): {} ({:?})", restore::format_when(now), now);

    println!("\n== 1 Reset graphics driver — display adapters (read-only)");
    let t0 = Instant::now();
    match os.display_adapters() {
        Ok(list) => {
            for a in &list {
                println!("  {:<40} {}", a.name, a.instance_id);
            }
            println!("  ({} adapter(s), {} ms)", list.len(), t0.elapsed().as_millis());
        }
        Err(e) => println!("  error: {e}"),
    }
    println!("  our window in front: {}", os.foreground_is_ours());
    println!("  chord (must be refused): {:?}", gfx::reset_with_chord(os.as_ref()).err());
    println!("  chord send on the read-only layer (must be refused): {:?}", os.send_reset_chord().err());
    println!("  adapter restart (must be refused): {:?}", gfx::restart_adapters(os.as_ref()).err());

    println!("\n== 2 Repair Windows files");
    println!("  start (must be refused — needs admin / read-only): {:?}", repair::RepairRun::start(os.clone(), |_| {}).err());
    println!("  spawn dism on the read-only layer (must be refused): {:?}", os.spawn("dism.exe", &["/?"]).err().map(|e| e.to_string()));
    match os.cbs_log_tail() {
        Ok(t) => {
            let lines: Vec<&str> = t.lines().collect();
            println!("  CBS.log tail readable: {} lines in the last ≤4 MB; [SR] repair lines: {}", lines.len(), t.matches("[SR] Repairing corrupted file").count());
        }
        Err(e) => println!("  CBS.log: {e}"),
    }

    println!("\n== 3 Rebuild icon & thumbnail cache — files (read-only)");
    match os.explorer_cache_dir() {
        Ok(d) => println!("  folder: {}", anon(&d.display().to_string())),
        Err(e) => println!("  folder error: {e}"),
    }
    match cache::cache_files(os.as_ref()) {
        Ok(files) => {
            let total: u64 = files.iter().map(|f| f.bytes).sum();
            for f in &files {
                println!("  {:>12} B  {}", f.bytes, f.path.file_name().unwrap_or_default().to_string_lossy());
            }
            println!("  ({} cache files, {:.1} MB)", files.len(), total as f64 / 1_048_576.0);
        }
        Err(e) => println!("  error: {e}"),
    }
    println!("  confirm text: {}", cache::CONFIRM);
    println!("  rebuild on the read-only layer (must be refused, Explorer untouched): {:?}", cache::rebuild(os.as_ref()).err());

    println!("\n== 4 Make a restore point — status (read-only)");
    let t0 = Instant::now();
    match os.restore_status() {
        Ok(st) => {
            println!("  frequency: {} minutes (SystemRestorePointCreationFrequency; missing = 1440)", st.frequency_minutes);
            println!("  newest point known: {} ({} ms)", st.newest_known, t0.elapsed().as_millis());
            if let Some(p) = &st.newest {
                println!("  newest: #{} \"{}\" {}", p.sequence, p.description, restore::format_when(os.local(p.created)));
            }
            println!("  row sub-line: {}", restore::status_line(os.as_ref(), &st));
        }
        Err(e) => println!("  error: {e}"),
    }
    println!("  description it would use: {}", restore::description(now));
    println!("  create (must be refused — needs admin / read-only): {:?}", restore::make_restore_point(os.as_ref()).err());
    println!("  create on the read-only layer (must be refused): {:?}", os.create_restore_point("x").err());
}

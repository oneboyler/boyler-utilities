//! `cargo run -p bu-search --example search-show [text]` — prints the REAL Search state, READ-ONLY.
//! `... --example search-show -- --instance <name> [text]` — only that Everything instance (e.g. a scratch one).
//! Runs on `RealOs::read_only()`: nothing is opened, nothing goes on the clipboard. User names in paths are shown as `<user>`.

use bu_search::*;
use std::sync::Arc;
use std::time::Instant;

fn anon(s: &str) -> String {
    let Ok(u) = std::env::var("USERNAME") else { return s.to_string() };
    if u.is_empty() {
        return s.to_string();
    }
    // case-insensitive replace of the user name (ASCII lower-casing keeps byte positions)
    let (lower, needle) = (s.to_ascii_lowercase(), u.to_ascii_lowercase());
    let mut out = String::new();
    let mut at = 0;
    while let Some(i) = lower[at..].find(&needle) {
        out.push_str(&s[at..at + i]);
        out.push_str("<user>");
        at += i + needle.len();
    }
    out.push_str(&s[at..]);
    out
}

fn working_set_mb() -> f64 {
    use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut c = PROCESS_MEMORY_COUNTERS { cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
    unsafe { let _ = K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb); }
    c.WorkingSetSize as f64 / 1_048_576.0
}

/// Everything as its IPC window answers (read-only asks: version, index loaded, NTFS drives; one query, timed).
fn everything_report(instance: Option<&str>, text: &str) {
    use bu_search::real::everything;
    let classes: Vec<String> = match instance {
        Some(i) => vec![everything::class_of(Some(i))],
        None => vec![everything::class_of(None), everything::class_of(Some(bu_search::real::host::INSTANCE))],
    };
    println!("Everything.exe on this PC: {:?}", bu_search::real::host::exe().map(|p| p.display().to_string()));
    let Some((class, h)) = classes.iter().find_map(|c| everything::find(c).map(|h| (c.clone(), h))) else {
        println!("no Everything IPC window in this session (looked for {classes:?})");
        return;
    };
    println!("Everything IPC window: {class}");
    println!("  version: {:?}", everything::version(h));
    println!("  index loaded: {}", everything::db_loaded(h));
    println!("  NTFS drives in its index: [{}]", everything::ntfs_drives(h));
    for folders in [false, true] {
        let q = FileQuery { words: vec![text.to_lowercase()], folders, extensions: vec![], max: 5 };
        let t = Instant::now();
        match everything::query(h, &q, std::time::Duration::from_secs(5)) {
            Ok(hits) => {
                println!("  query {:?}: {} matches (total), {} sent back, in {:?}", q.everything_text(), hits.total.unwrap_or(0), hits.items.len(), t.elapsed());
                for it in &hits.items {
                    println!("    {}", anon(&it.path));
                }
            }
            Err(e) => println!("  query {:?} failed in {:?}: {e}", q.everything_text(), t.elapsed()),
        }
    }
}

fn main() {
    println!("working set at start: {:.1} MB", working_set_mb());
    // `--instance <name>`: only that Everything instance's window (read-only asks + one query), then stop
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `--instance <name> --raw <Everything search text>...`: counts of these searches only (e.g. `C:\ file:`)
    if let (Some(i), Some(r)) = (args.iter().position(|a| a == "--instance"), args.iter().position(|a| a == "--raw")) {
        let Some(h) = args.get(i + 1).and_then(|n| bu_search::real::everything::find(&bu_search::real::everything::class_of(Some(n)))) else {
            println!("no such instance");
            return;
        };
        for text in &args[r + 1..] {
            let t = Instant::now();
            match bu_search::real::everything::query_text(h, text, 3, std::time::Duration::from_secs(30)) {
                Ok(hits) => println!("  raw {:?}: {} matches in {:?} (first: {:?})", text, hits.total.unwrap_or(0), t.elapsed(), hits.items.first().map(|x| anon(&x.path))),
                Err(e) => println!("  raw {text:?} failed: {e}"),
            }
        }
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--instance") {
        everything_report(args.get(i + 1).map(|s| s.as_str()), args.get(i + 2).map(|s| s.as_str()).unwrap_or("note"));
        return;
    }
    everything_report(None, args.first().map(|s| s.as_str()).unwrap_or("note"));
    let svc = SearchService::new(Arc::new(RealOs::read_only()));
    let t0 = Instant::now();
    let rep = svc.backend_report();
    println!("backend check took {:?}", t0.elapsed());
    println!("Everything: {:?}", rep.everything);
    println!("Windows Search service: {:?}", rep.windows_search);
    println!("files and folders come from: {:?}", rep.files_from);
    let scope = svc.os().windows_search_scope();
    println!("Windows Search index covers ({} included, {} excluded rules):", scope.included.len(), scope.excluded.len());
    for p in &scope.included {
        println!("  + {}", anon(p));
    }
    for p in scope.excluded.iter().take(6) {
        println!("  - {}", anon(p));
    }

    let text = std::env::args().nth(1).unwrap_or_else(|| "note".to_string());
    for filter in [Filter::All, Filter::Apps] {
        let t = Instant::now();
        match svc.search(&Query::new(&text, filter), &Cancel::new()) {
            Ok(r) => {
                println!("\nsearch {:?} {:?}: {:?}{}", text, filter, t.elapsed(), if filter == Filter::All { " (first search loads the app list)" } else { " (app list cached)" });
                for g in &r.groups {
                    println!("  {} ({}{})", g.title, g.total, if g.total_is_lower_bound { "+" } else { "" });
                    for it in &g.items {
                        println!("    {:<34} {}", anon(&it.name), anon(&it.path));
                    }
                }
                if let Some(n) = &r.note {
                    println!("  note: {n}");
                }
            }
            Err(e) => println!("search error: {e}"),
        }
    }
    println!("\nworking set with the app list loaded: {:.1} MB", working_set_mb());
    let probe = FileQuery { words: vec!["boyler".into()], folders: false, extensions: vec![], max: 5 };
    let t = Instant::now();
    match svc.os().windows_search_query(&probe) {
        Ok(h) => println!("
raw index probe: {} rows in {:?}", h.items.len(), t.elapsed()),
        Err(e) => println!("
raw index probe failed in {:?}: {e}", t.elapsed()),
    }
    let t = Instant::now();
    let apps = svc.os().list_apps().map(|a| a.len());
    println!("\napp list: {:?} apps in {:?}", apps, t.elapsed());
    svc.release();
    println!("released (app list and Everything DLL dropped); working set now: {:.1} MB", working_set_mb());
}

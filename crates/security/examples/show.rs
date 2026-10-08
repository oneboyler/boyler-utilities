//! `cargo run -p bu-security --example security-show` — prints the REAL Defender state, READ-ONLY.
//! Runs on `RealOs::read_only()`: no scan, no change, nothing in Quarantine is touched. File names and folders under the user
//! profile are anonymised (`<user>`), threat names are Microsoft's own public names.

use bu_security::*;

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

fn row(r: &ThreatRow, now: &Stamp, verb: &str) -> String {
    let when = r.found.map(|f| f.short(now)).unwrap_or_default();
    format!("  {} [{}]  {} · {} · {verb} {when}", r.name, r.severity.label(), anon(&r.file), anon(r.folder_name()))
}

fn main() {
    let svc = SecurityService::new(std::sync::Arc::new(RealOs::read_only()));
    let t0 = std::time::Instant::now();
    let page = match svc.page() {
        Ok(p) => p,
        Err(e) => {
            println!("page: error: {e}");
            return;
        }
    };
    let now = svc.now();
    println!("read in {:?}; elevated: {}", t0.elapsed(), svc.os().is_elevated());
    println!("banner: {:?}", page.banner);
    println!("antivirus: {:?}", page.antivirus);
    let s = &page.status;
    println!(
        "defender: service {} · antivirus {} · real-time {} · tamper {} · mode {:?}",
        s.service_enabled, s.antivirus_enabled, s.realtime_enabled, s.tamper_protected, s.running_mode
    );
    println!(
        "definitions: {} · updated {}",
        s.definitions_version,
        s.definitions_updated.map(|d| format!("{} ({})", d.label(&now), d)).unwrap_or_else(|| "never".into())
    );
    match &page.last_scan {
        Some((t, k)) => println!("last scan: {} ({}) · {}", t.label(&now), t, k.title()),
        None => println!("last scan: none"),
    }
    println!("threats found: {}", page.threats.len());
    for r in &page.threats {
        println!("{}", row(r, &now, "found"));
    }
    println!("quarantine (from detection history: {}): {}", page.quarantine_from_history, page.quarantine.len());
    for r in &page.quarantine {
        println!("{}", row(r, &now, "found"));
    }
    println!("allowed in Defender (the reset line): {} {}", page.allowed.len(), if page.allowed.is_empty() { "(none, or not readable without admin: unproven)" } else { "" });
    for a in &page.allowed {
        println!("  {} ({})", a.name, a.files.iter().map(|f| anon(f)).collect::<Vec<_>>().join(", "));
    }
    println!("scan state: {:?}", svc.scan_state());
    for a in [Action::QuickScan, Action::FullScan, Action::ScanPath, Action::UpdateDefinitions, Action::OfflineScan, Action::RemoveThreat, Action::AllowThreat, Action::RestoreQuarantined, Action::RemoveAllow, Action::DeleteQuarantined] {
        println!("  {:?}: admin {:?}", a, svc.needs_admin(a));
    }
}

//! Prints the REAL installed apps of this PC. READ-ONLY: only `Apps::list()` (and, with `--measure`, folder sizes of the first
//! few Store apps) — never an uninstall. The user's profile folder / name are replaced by `%USERPROFILE%` / `<user>`.

#[cfg(windows)]
fn main() {
    use bu_apps::{real::RealOs, AppKind, Apps, SizeSource};

    let profile = std::env::var("USERPROFILE").unwrap_or_default();
    let user = std::env::var("USERNAME").unwrap_or_default();
    let anon = |s: &str| -> String {
        let mut s = s.to_string();
        if !profile.is_empty() {
            s = replace_ci(&s, &profile, "%USERPROFILE%");
        }
        if user.len() > 2 {
            s = replace_ci(&s, &user, "<user>");
        }
        s
    };
    let gb = |b: u64| format!("{:.2} GB", b as f64 / 1_073_741_824.0);

    let apps = Apps::new(RealOs::new());
    let t = std::time::Instant::now();
    let list = apps.list();
    println!("read {} apps in {} ms (measured)", list.apps.len(), t.elapsed().as_millis());
    for (src, e) in &list.problems {
        println!("problem reading {src}: {e}");
    }
    let desktop = list.apps.iter().filter(|a| a.kind == AppKind::Desktop).count();
    println!("desktop {desktop} · store {} · locked {}", list.apps.len() - desktop, list.apps.iter().filter(|a| a.lock.is_some()).count());
    for a in &list.apps {
        let size = match (a.size_bytes, a.size_source) {
            (Some(b), SizeSource::Estimated) => format!("{} (estimate)", gb(b)),
            (Some(b), _) => format!("{} (measured)", gb(b)),
            (None, _) => "—".into(),
        };
        println!(
            "{} | {}{} | {} | {} | {} | {}{}",
            anon(&a.name),
            if a.kind == AppKind::Store { "[Store] " } else { "" },
            a.publisher.as_deref().map(anon).unwrap_or_else(|| "-".into()),
            a.version.as_deref().unwrap_or("-"),
            size,
            a.install_date.map(|d| d.display()).unwrap_or_else(|| "—".into()),
            a.lock.map(|l| format!("LOCKED {l:?}")).unwrap_or_else(|| "uninstallable".into()),
            a.warning().map(|w| format!(" | warn: {w}")).unwrap_or_default(),
        );
        if let Some(c) = &a.uninstall_command {
            println!("      uninstall{}: {}", if a.quiet { " (quiet)" } else { "" }, anon(c));
        }
        if let Some(i) = &a.icon_path {
            println!("      icon: {},{}", anon(&i.to_string_lossy()), a.icon_index);
        }
    }
    if std::env::args().any(|a| a == "--measure") {
        println!("\nmeasured on demand (first 5 Store apps):");
        for a in list.apps.iter().filter(|a| a.kind == AppKind::Store).take(5) {
            let t = std::time::Instant::now();
            let m = apps.measure_size(a);
            println!("  {}: {} in {} ms", a.name, m.size_bytes.map(gb).unwrap_or_else(|| "unreadable".into()), t.elapsed().as_millis());
        }
    }
}

#[cfg(windows)]
fn replace_ci(hay: &str, needle: &str, with: &str) -> String {
    let lower = hay.to_lowercase();
    if lower.len() != hay.len() {
        return hay.to_string();
    }
    let n = needle.to_lowercase();
    let mut out = String::new();
    let mut i = 0;
    while let Some(p) = lower[i..].find(&n) {
        out.push_str(&hay[i..i + p]);
        out.push_str(with);
        i += p + n.len();
    }
    out.push_str(&hay[i..]);
    out
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows only");
}

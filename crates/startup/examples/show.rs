//! Prints the REAL startup list of this PC. READ-ONLY: it only calls `Startup::list()`, never a write.
//! The user's profile folder and user name are replaced by `%USERPROFILE%` / `<user>` so the output can go into a report.

#[cfg(windows)]
fn main() {
    use bu_startup::{real::RealOs, ImpactState, Kind, Startup, StartupOs, Switch, View};

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

    let t = std::time::Instant::now();
    let startup = Startup::new(RealOs::new());
    let list = startup.list();
    let took = t.elapsed();

    println!("admin: {}   impact source: {:?}   read in {} ms (measured)", startup.os().is_admin(), list.impact_source, took.as_millis());
    for (src, err) in &list.problems {
        println!("problem reading {src}: {err}");
    }
    for view in [View::All, View::Normal, View::Hidden] {
        let (on, shown) = list.counts(view);
        println!("{view:?}: {on} of {shown} on");
    }
    let mut last = None;
    for e in &list.entries {
        if last != Some(e.kind) {
            println!("\n== {:?} ==", e.kind);
            last = Some(e.kind);
        }
        let impact = match e.impact {
            ImpactState::Unknown => "impact ?".to_string(),
            ImpactState::NotMeasured => "not measured".to_string(),
            ImpactState::Measured(i, c) => format!("{i:?} ({} ms CPU, {} KB disk)", c.cpu_us / 1000, c.disk_bytes / 1024),
        };
        let switch = match e.switch {
            Switch::Free => "switch".to_string(),
            Switch::NeedsAdmin => "switch (admin)".to_string(),
            Switch::Settings(u) => format!("Settings: {u}"),
            Switch::Locked(r) => format!("locked: {r:?}"),
        };
        println!(
            "[{}] {}{} | {} | {} | {} | {}",
            if e.enabled { "on " } else { "off" },
            anon(&e.name),
            if e.windows_own && e.kind != Kind::Normal { " (Windows)" } else { "" },
            e.publisher.as_deref().map(anon).unwrap_or_else(|| "-".into()),
            anon(&e.location),
            impact,
            switch
        );
        println!("      cmd: {}", anon(&e.command));
        if let Some(i) = &e.icon_path {
            println!("      icon: {},{}", anon(&i.to_string_lossy()), e.icon_index);
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

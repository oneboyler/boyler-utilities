//! Prints what Windows is set to NOW for every Toggles row, the default apps and the per-game fullscreen list.
//! READ-ONLY: it runs on `RealOs::read_only()`, which refuses every change. Paths under the user's profile are shown as
//! `%USERPROFILE%`.
//!
//! `cargo run -p bu-toggles --example toggles-show`

#[cfg(windows)]
fn main() {
    use bu_toggles::model::{Badge, Group, Value};
    use bu_toggles::os::{Hive, RegValue, TogglesOs};
    use bu_toggles::real::RealOs;
    use bu_toggles::{defaults, Toggles};

    let profile = std::env::var("USERPROFILE").unwrap_or_default();
    let anon = |s: &str| if profile.is_empty() { s.to_string() } else { s.replace(&profile, "%USERPROFILE%") };

    let t = Toggles::new(RealOs::read_only());
    let cv = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let s = |n: &str| match t.os().reg_read(Hive::Hklm, cv, n) {
        Ok(Some(RegValue::Sz(v))) => v,
        Ok(Some(RegValue::Dword(d))) => d.to_string(),
        _ => "?".into(),
    };
    println!("Windows: {} {} build {}.{}  (admin: {})", s("EditionID"), s("DisplayVersion"), s("CurrentBuild"), s("UBR"),
        t.os().is_elevated());
    println!("battery: {}", t.os().has_battery());
    match t.os().gpu_scheduling() {
        Ok(g) => println!("GPU scheduling (driver): supported {}, on now {}, default {}", g.supported, g.enabled_now, g.enabled_by_default),
        Err(e) => println!("GPU scheduling (driver): READ ERROR {e}"),
    }

    for g in Group::ALL {
        println!("\n== {} ==", g.title());
        for row in t.rows().iter().filter(|r| r.group == g) {
            let badges: Vec<&str> = row
                .badges
                .iter()
                .map(|b| match b {
                    Badge::Admin => "admin",
                    Badge::Explorer => "Explorer",
                    Badge::SignOut => "sign-out",
                    Badge::Restart => "restart",
                    Badge::NextGame => "next game",
                })
                .collect();
            let state = match t.read(row.id) {
                Ok(st) => {
                    let v = match &st.value {
                        Value::Switch(true) => "ON".to_string(),
                        Value::Switch(false) => "off".to_string(),
                        Value::Timeout(x) => x.label(),
                        Value::Games(g) => format!("{} game(s): {}", g.len(),
                            g.iter().map(|x| anon(&x.exe)).collect::<Vec<_>>().join(" | ")),
                    };
                    let mut out = v;
                    if !st.enabled {
                        out.push_str(&format!("  [greyed: {}]", st.disabled_reason.unwrap_or("")));
                    }
                    out
                }
                Err(e) => format!("READ ERROR: {e}"),
            };
            println!("  {:<48} {:<34} {}", row.title, state,
                if badges.is_empty() { String::new() } else { format!("[{}]", badges.join(", ")) });
        }
    }

    println!("\n== Default apps ==");
    match defaults::read(t.os()) {
        Ok(d) => {
            let app = |a: &Option<bu_toggles::os::AssocApp>| match a {
                Some(a) => format!("{}{}", a.name, a.exe.as_deref().map(|e| format!("  ({})", anon(e))).unwrap_or_default()),
                None => "(none)".into(),
            };
            println!("  {:<8} {}", d.browser.label, app(&d.browser.app));
            for b in &d.browsers {
                let uri = match &b.change {
                    defaults::ChangeAction::OpenUri(u) | defaults::ChangeAction::OpenWith(u) => u.clone(),
                };
                println!("    {} {:<24} -> {}", if b.is_current { "✓" } else { " " }, b.name, uri);
            }
            for r in &d.file_types {
                println!("  {:<8} {}", r.label, app(&r.app));
            }
        }
        Err(e) => println!("  READ ERROR: {e}"),
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows only");
}

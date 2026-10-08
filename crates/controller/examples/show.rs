//! `cargo run -p bu-controller --example controller-show` — the REAL state, READ-ONLY: Steam, the account, the games with a
//! layout per controller type, what each layout says, the per-controller files, the connected controllers.
//! Account numbers and controller serials are shortened (anonymised). Nothing is written, nothing is sent to a controller.
//!
//! `--live <seconds>`: read the first connected PlayStation pad's input for that long (read-only) and print the report
//! rate + this process's CPU time (the live view's cost). `--xinput-cost <seconds>`: run the Xbox poll loop (one
//! `XInputGetState` every 8 ms) for that long and print its CPU time.

#[cfg(windows)]
fn main() {
    use bu_controller::os::{PadOs, PadSource};
    use bu_controller::real::{RealPads, RealSteam};
    use bu_controller::settings::{radius_to_pct, StickSetting};
    use bu_controller::{ControllerService, PadKind, Side};

    let mask = |s: &str| if s.len() > 4 { format!("{}…{}", &s[..2], &s[s.len() - 2..]) } else { "…".into() };
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<u64>().ok());

    println!("== Steam");
    let os = RealSteam::read_only();
    match ControllerService::new(os, std::env::temp_dir().join("bu-controller-show-never-written")) {
        Err(e) => println!("  not usable: {e}"),
        Ok(s) => {
            println!("  folder: {}", s.steam().dir.display());
            println!("  running: {}", if s.steam_running() { "yes" } else { "no (left as it is: the app never starts Steam)" });
            println!("  account: {} (logged-in one, else the first with controller configs)", mask(&s.steam().account));
            for kind in [PadKind::DualSenseEdge, PadKind::DualShock4, PadKind::Xbox] {
                let label = match kind {
                    PadKind::DualSenseEdge => "PS5 pads (DualSense / Edge)",
                    PadKind::DualShock4 => "PS4 pads",
                    _ => "Xbox pads",
                };
                match s.games(kind) {
                    Err(e) => println!("\n== {label}: {e}"),
                    Ok(games) => {
                        println!("\n== {label}: {} game(s) with a layout", games.len());
                        for g in games {
                            let src = match &g.source {
                                bu_controller::LayoutSource::Autosave => "the user's own copy".to_string(),
                                bu_controller::LayoutSource::Workshop(id) => format!("community/official layout {id}"),
                                bu_controller::LayoutSource::Template(t) => format!("Steam template {t}"),
                                bu_controller::LayoutSource::Other(k, v) => format!("{k} {v}"),
                            };
                            println!("  - {} ({}) · {src}{}", g.name, g.appid.map(|a| a.to_string()).unwrap_or_else(|| "shortcut".into()), if g.installed { "" } else { " · not installed" });
                            match s.open(&g.key, kind) {
                                Err(e) => println!("      can't read: {e}"),
                                Ok(o) => {
                                    let h = o.layout.header();
                                    let sets: Vec<String> = o.layout.action_sets().into_iter().map(|a| a.title).collect();
                                    println!("      layout \"{}\" · {} · action sets: {}", h.title, h.controller_type, sets.join(", "));
                                    let v = o.layout.pad_view(0, kind);
                                    for st in &v.sticks {
                                        let g = |k| st.settings.iter().find(|(x, _)| *x == k).and_then(|(_, v)| *v);
                                        let pc = |v: Option<i64>| v.map(|r| format!("{:.0} %", radius_to_pct(r))).unwrap_or_else(|| "Steam's".into());
                                        println!(
                                            "      {} stick: {} · dead zone {} · full at {}",
                                            if st.side == Side::Left { "left" } else { "right" },
                                            st.mode.label(),
                                            pc(g(StickSetting::DeadZone)),
                                            pc(g(StickSetting::FullAt))
                                        );
                                    }
                                    let remapped: Vec<String> = v
                                        .buttons
                                        .iter()
                                        .filter(|b| !b.fixed && b.presses.first().map(|p| p.1 != bu_controller::Action::Nothing).unwrap_or(false))
                                        .map(|b| format!("{} → {}", b.name, b.presses[0].1.label(kind.is_xbox())))
                                        .collect();
                                    println!("      buttons: {}", remapped.join(" · "));
                                    match s.changed_parts(&g.key, kind, 0) {
                                        Ok(c) if c.is_empty() => println!("      differs from Steam's layout: nothing"),
                                        Ok(c) => {
                                            println!("      differs from Steam's layout: {c:?}");
                                            // what exactly differs on the buttons (presses / press settings, the user's vs Steam's)
                                            if let Ok(sl) = s.steam_layout(&g.key, kind) {
                                                let sv = sl.pad_view(0, kind);
                                                for b in &v.buttons {
                                                    let Some(x) = sv.buttons.iter().find(|x| x.id == b.id) else { continue };
                                                    for (p, q) in b.presses.iter().zip(&x.presses) {
                                                        if p != q {
                                                            println!("        {} {:?}: {} (Steam's: {})", b.name, p.0, p.1.label(kind.is_xbox()), q.1.label(kind.is_xbox()));
                                                        }
                                                    }
                                                    for (p, q) in b.settings.iter().zip(&x.settings) {
                                                        if p != q {
                                                            println!("        {} {}: {:?} (Steam's: {:?})", b.name, p.0.def().label, p.1, q.1);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        Err(e) => println!("      Steam's layout: {e}"),
                                    }
                                }
                            }
                        }
                    }
                }
            }
            println!("\n== This controller · all games (preferences_<serial>.vdf)");
            match s.preferences() {
                Err(e) => println!("  can't read: {e}"),
                Ok(list) => {
                    for p in list {
                        let dz = |k| p.stick_deadzone_pct(k).map(|v| format!("{v:.0} %")).unwrap_or_else(|| "Steam's".into());
                        println!(
                            "  - {} [{}] · stick dead zones L {} / R {} · anti-drift {} · light {:?}",
                            p.name,
                            mask(&p.serial),
                            dz(bu_controller::PrefSetting::LeftStickDeadZone),
                            dz(bu_controller::PrefSetting::RightStickDeadZone),
                            p.get(bu_controller::PrefSetting::AntiDrift).unwrap_or("-"),
                            p.led()
                        );
                    }
                }
            }
        }
    }

    println!("\n== Controllers connected now");
    let pads = RealPads::new();
    let list = pads.list_pads().unwrap_or_default();
    if list.is_empty() {
        println!("  none");
    }
    for p in &list {
        let src = match &p.source {
            PadSource::Hid(_) => "HID".to_string(),
            PadSource::XInput(n) => format!("XInput slot {n}"),
        };
        println!("  - {} · {} · {:?} · {src} · battery {:?}", p.kind.name(), p.name, p.connection, p.battery);
    }

    if let Some(secs) = arg("--live") {
        match list.iter().find(|p| !p.kind.is_xbox()) {
            None => println!("\n--live: no PlayStation pad connected"),
            Some(p) => {
                let (c0, t0) = (cpu_ms(), std::time::Instant::now());
                match bu_controller::LiveView::start(&pads, p, None) {
                    Err(e) => println!("\n--live: {e}"),
                    Ok(v) => {
                        std::thread::sleep(std::time::Duration::from_secs(secs));
                        let n = v.reports();
                        let last = v.latest();
                        v.stop();
                        let (cpu, wall) = (cpu_ms() - c0, t0.elapsed().as_secs_f64());
                        println!(
                            "\n--live {secs} s on {}: {n} reports ({:.0}/s) · CPU {cpu:.0} ms = {:.2} % of one core · last state: {:?}",
                            p.name,
                            n as f64 / wall,
                            cpu / (wall * 10.0),
                            last.map(|s| (s.left, s.right, s.l2, s.r2, s.pressed))
                        );
                    }
                }
            }
        }
    }
    if let Some(secs) = arg("--xinput-cost") {
        use windows::Win32::UI::Input::XboxController::{XInputGetState, XINPUT_STATE};
        let (c0, t0) = (cpu_ms(), std::time::Instant::now());
        let mut calls = 0u64;
        while t0.elapsed().as_secs() < secs {
            let mut st = XINPUT_STATE::default();
            unsafe { XInputGetState(0, &mut st) };
            calls += 1;
            std::thread::sleep(std::time::Duration::from_millis(bu_controller::real::XINPUT_POLL_MS as u64));
        }
        let (cpu, wall) = (cpu_ms() - c0, t0.elapsed().as_secs_f64());
        println!("\n--xinput-cost {secs} s: {calls} polls · CPU {cpu:.0} ms = {:.2} % of one core", cpu / (wall * 10.0));
    }
    let _ = PadKind::ALL;
}

/// This process's CPU time (kernel + user) in ms.
#[cfg(windows)]
fn cpu_ms() -> f64 {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64 / 10_000.0;
    if unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) }.is_ok() {
        ft(k) + ft(u)
    } else {
        0.0
    }
}

#[cfg(not(windows))]
fn main() {
    println!("controller-show runs on Windows only");
}

//! `cargo run -p bu-activity --example activity-show` — REAL, READ-ONLY: uptime, the game launcher folders found, which
//! app is in front now (exe only — never a window title), and where the data would be kept. Writes nothing.

use bu_activity::views::fmt_uptime;
use bu_activity::{ActivityOs, Clock, RealOs, SystemClock};

fn anon(p: &str) -> String {
    match std::env::var("USERNAME") {
        Ok(u) if !u.is_empty() => p.replace(&u, "<user>"),
        _ => p.to_string(),
    }
}

fn main() {
    let mut os = RealOs;
    let now = SystemClock.now();
    let up = os.uptime_ms();
    println!("local offset now: {:+} min; uptime (GetTickCount64): {} ({} ms)", now.offset_min, fmt_uptime(up), up);
    let t = std::time::Instant::now();
    let roots = os.game_roots();
    println!("game launcher folders found: {} (read in {:.0} ms)", roots.len(), t.elapsed().as_secs_f64() * 1000.0);
    for r in &roots {
        let n = std::fs::read_dir(r).map(|d| d.count()).unwrap_or(0);
        println!("  {}  ({} entries)", anon(r), n);
    }
    let rules = bu_activity::games::GameRules::new(roots);
    // SAFETY: plain read of the front window.
    let hwnd = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    let path = bu_activity::real::exe_of_window(hwnd);
    let exe = path.rsplit('\\').next().unwrap_or("").to_string();
    println!("in front now: {} = \"{}\", counts as a game: {}", exe, os.app_name(&path), rules.is_game(&path));
    let dir = os.data_dir().map(|d| anon(&d.display().to_string())).unwrap_or("-".into());
    println!("data folder (not created by this example): {dir}");
}

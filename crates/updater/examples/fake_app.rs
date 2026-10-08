//! A stand-in for the real app, used ONLY by the tests (tests/real_swap.rs): the tests copy this exe into a scratch "install
//! folder", append a tag to the copy (`BU-TAG:good 2.0.0`) and let the updater replace it. What it does is decided by the
//! tag at the end of its own file:
//!   good <v>   confirms with `confirm_started`, then ends        crash <v>  ends at once with code 3 (never confirms)
//!   hang <v>   never confirms, sleeps 60 s                       late <v>   confirms after 1.5 s
//! Every start appends a line `<tag> | <args>` to `started.log` next to the exe, so a test can see who ran.
//! Special argument forms: `update <repo> <api_base> <current_version> <start_wait_ms>` = do check() + update() with the real
//! WinHTTP client against `api_base`, write `update-run.log`, then exit (as the real app would after "Restarting…").

use bu_updater::{confirm_started, run_helper_if_requested, CheckResult, Phase, Updater, UpdaterConfig};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::time::Duration;

fn own_tag(exe: &PathBuf) -> String {
    let mut f = match std::fs::File::open(exe) {
        Ok(f) => f,
        Err(_) => return "unknown".into(),
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let take = len.min(256);
    let mut buf = vec![0u8; take as usize];
    if f.seek(SeekFrom::Start(len - take)).is_err() || f.read_exact(&mut buf).is_err() {
        return "unknown".into();
    }
    let text = String::from_utf8_lossy(&buf).into_owned();
    match text.rfind("BU-TAG:") {
        Some(i) => text[i + 7..].lines().next().unwrap_or("").trim().to_string(),
        None => "untagged".into(),
    }
}

fn log(dir: &std::path::Path, file: &str, line: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(file)) {
        let _ = writeln!(f, "{line}");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = run_helper_if_requested(&args) {
        std::process::exit(code);
    }
    let exe = std::env::current_exe().expect("exe path");
    let dir = exe.parent().expect("dir").to_path_buf();
    let tag = own_tag(&exe);
    log(&dir, "started.log", &format!("{tag} | {}", args[1..].join(" ")));

    if args.get(1).map(String::as_str) == Some("update") && args.len() >= 6 {
        let mut cfg = UpdaterConfig::new(&args[2], &args[4], exe.clone());
        cfg.api_base = args[3].clone();
        cfg.start_wait = Duration::from_millis(args[5].parse().unwrap_or(3000));
        cfg.exit_wait = Duration::from_secs(20);
        cfg.temp_dir = dir.join("temp");
        std::fs::create_dir_all(&cfg.temp_dir).ok();
        let u = Updater::real(cfg);
        let release = match u.check() {
            Ok(CheckResult::Available(r)) => r,
            other => {
                log(&dir, "update-run.log", &format!("check: {other:?}"));
                std::process::exit(10);
            }
        };
        let mut last_phase = None;
        let r = u.update(&release, &mut |p| {
            if last_phase != Some(p.phase) {
                log(&dir, "update-run.log", &format!("phase {:?}", p.phase));
                last_phase = Some(p.phase);
            }
            if p.phase == Phase::Downloading && p.percent == Some(100) {
                log(&dir, "update-run.log", "downloaded 100%");
            }
        });
        log(&dir, "update-run.log", &format!("update: {r:?}"));
        std::process::exit(if r.is_ok() { 0 } else { 11 });
    }

    let behaviour = tag.split_whitespace().next().unwrap_or("").to_string();
    let updated = args.iter().any(|a| a == "--bu-updated");
    if updated {
        match behaviour.as_str() {
            "crash" => std::process::exit(3),
            "hang" => {
                let _ = std::fs::write(dir.join("hang.pid"), std::process::id().to_string());
                std::thread::sleep(Duration::from_secs(60));
                return;
            }
            "late" => std::thread::sleep(Duration::from_millis(1500)),
            _ => {}
        }
        let from = confirm_started(&args);
        log(&dir, "started.log", &format!("{tag} | confirmed, previous = {:?}", from.map(|f| f.previous_version)));
    }
}

//! End to end with REAL processes, in a scratch folder: a fake installed app (examples/fake_app.rs, tagged "old 1.0.0") asks a
//! local server playing GitHub for an update, downloads it over real WinHTTP, starts the real install step and exits; the install
//! step swaps the files, starts the new "app" and either keeps it (it confirms) or rolls back (it crashes / hangs). Nothing of
//! the real PC is touched; the fake app writes only into its own scratch folder.

mod common;

use bu_updater::*;
use common::*;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

fn fake_app_exe() -> PathBuf {
    // target/debug/deps/<this test>.exe -> target/debug/examples/updater-fake-app.exe
    let me = std::env::current_exe().unwrap();
    let p = me.parent().unwrap().parent().unwrap().join("examples").join("updater-fake-app.exe");
    assert!(p.exists(), "build the example first: {}", p.display());
    p
}

/// The fake app's bytes + a tag at the end (the tag decides what that "version" does).
fn app_bytes(tag: &str) -> Vec<u8> {
    let mut b = std::fs::read(fake_app_exe()).unwrap();
    b.extend_from_slice(format!("\nBU-TAG:{tag}\n").as_bytes());
    b
}

struct World {
    dir: Scratch,
    server: TestServer,
    exe: PathBuf,
    old: Vec<u8>,
    new: Vec<u8>,
}

fn world(name: &str, new_tag: &str, digest_override: Option<String>, listed_size_override: Option<u64>) -> World {
    let dir = Scratch::new(name);
    let server = TestServer::start();
    let exe = dir.join("BoylerUtilities.exe");
    let old = app_bytes("old 1.0.0");
    std::fs::write(&exe, &old).unwrap();
    let new = app_bytes(new_tag);
    server.body("/dl/BoylerUtilities.exe", new.clone());
    let digest = digest_override.unwrap_or_else(|| format!("sha256:{}", sha256_of(&new)));
    server.release("o/r", "v2.0.0", "BoylerUtilities.exe", "/dl/BoylerUtilities.exe", listed_size_override.unwrap_or(new.len() as u64), Some(&digest));
    World { dir, server, exe, old, new }
}

/// Starts the old app in "update" mode and waits until IT has ended (that is the moment the real app would exit).
fn run_update_app(w: &World, start_wait_ms: u64) -> i32 {
    let mut child = Command::new(&w.exe)
        .args(["update", "o/r", &w.server.base(), "1.0.0", &start_wait_ms.to_string()])
        .current_dir(w.dir.path())
        .spawn()
        .unwrap();
    child.wait().unwrap().code().unwrap_or(-1)
}

fn log(dir: &Path, file: &str) -> Vec<String> {
    std::fs::read_to_string(dir.join(file)).map(|t| t.lines().map(str::to_string).collect()).unwrap_or_default()
}

fn config(w: &World) -> UpdaterConfig {
    UpdaterConfig::new("o/r", "1.0.0", w.exe.clone())
}

#[test]
fn a_good_update_replaces_the_app_and_the_new_one_runs_and_confirms() {
    let w = world("real-good", "good 2.0.0", None, None);
    assert_eq!(run_update_app(&w, 8000), 0, "update-run.log: {:?}", log(w.dir.path(), "update-run.log"));
    let run_log = log(w.dir.path(), "update-run.log");
    assert!(run_log.iter().any(|l| l.contains("phase Downloading")), "{run_log:?}");
    assert!(run_log.iter().any(|l| l.contains("phase Restarting")), "{run_log:?}");
    assert!(run_log.iter().any(|l| l.contains("Sha256")), "verified with SHA-256: {run_log:?}");

    assert!(wait_until(Duration::from_secs(20), || log(w.dir.path(), "started.log").iter().any(|l| l.contains("confirmed"))), "started.log: {:?}", log(w.dir.path(), "started.log"));
    assert!(wait_until(Duration::from_secs(10), || !w.dir.join("BoylerUtilities.exe.update-old").exists()), "the old file must be deleted after the confirm");

    assert_eq!(std::fs::read(&w.exe).unwrap(), w.new, "the installed app is now exactly the downloaded file");
    let started = log(w.dir.path(), "started.log");
    assert!(started[0].starts_with("old 1.0.0 | update "), "{started:?}");
    assert!(started[1].starts_with("good 2.0.0 | --bu-updated Local\\BoylerUtilities.Update."), "{started:?}");
    assert!(started[1].ends_with(" 1.0.0"), "the new app is told the previous version: {started:?}");
    assert_eq!(started[2], "good 2.0.0 | confirmed, previous = Some(\"1.0.0\")");
    assert_eq!(started.len(), 3, "no other start happened: {started:?}");
    assert!(!w.dir.join("BoylerUtilities.exe.update-result").exists());
    assert!(!w.dir.join("BoylerUtilities.exe.update-new").exists());
    assert!(!w.dir.join("BoylerUtilities.exe.update-part").exists());
    // exactly the two questions the user's click caused
    assert_eq!(w.server.paths(), vec!["/repos/o/r/releases/latest", "/dl/BoylerUtilities.exe"]);
}

#[test]
fn a_new_app_that_confirms_late_is_still_accepted() {
    let w = world("real-late", "late 2.0.0", None, None);
    assert_eq!(run_update_app(&w, 15000), 0);
    assert!(wait_until(Duration::from_secs(25), || log(w.dir.path(), "started.log").iter().any(|l| l.contains("confirmed"))));
    assert!(wait_until(Duration::from_secs(10), || !w.dir.join("BoylerUtilities.exe.update-old").exists()));
    assert_eq!(std::fs::read(&w.exe).unwrap(), w.new);
}

#[test]
fn a_new_app_that_crashes_on_start_is_rolled_back_and_the_old_one_runs_again() {
    let w = world("real-crash", "crash 2.0.0", None, None);
    assert_eq!(run_update_app(&w, 8000), 0);
    let res = w.dir.join("BoylerUtilities.exe.update-result");
    assert!(wait_until(Duration::from_secs(20), || res.exists()), "started.log: {:?}", log(w.dir.path(), "started.log"));
    // the old app is started again by the install step, plain
    assert!(wait_until(Duration::from_secs(10), || log(w.dir.path(), "started.log").len() >= 3), "{:?}", log(w.dir.path(), "started.log"));
    assert_eq!(std::fs::read(&w.exe).unwrap(), w.old, "the old version is back in place");
    assert!(!w.dir.join("BoylerUtilities.exe.update-old").exists());
    assert!(!w.dir.join("BoylerUtilities.exe.update-new").exists());
    let started = log(w.dir.path(), "started.log");
    assert!(started[1].starts_with("crash 2.0.0 | --bu-updated"), "{started:?}");
    assert_eq!(started[2], "old 1.0.0 | ", "{started:?}");
    let r = take_last_result(&config(&w)).expect("the old app can read why");
    assert_eq!((r.status, r.from_version.as_str(), r.to_version.as_str()), (SwapStatus::RolledBack, "1.0.0", "2.0.0"));
    assert!(r.message.contains("closed right after starting"), "{}", r.message);
}

#[test]
fn a_new_app_that_hangs_without_confirming_is_stopped_and_rolled_back() {
    let w = world("real-hang", "hang 2.0.0", None, None);
    assert_eq!(run_update_app(&w, 1500), 0);
    let res = w.dir.join("BoylerUtilities.exe.update-result");
    assert!(wait_until(Duration::from_secs(25), || res.exists()), "started.log: {:?}", log(w.dir.path(), "started.log"));
    assert!(wait_until(Duration::from_secs(10), || log(w.dir.path(), "started.log").len() >= 3));
    assert_eq!(std::fs::read(&w.exe).unwrap(), w.old, "rollback needs the hung process stopped, or the file couldn't be replaced");
    let r = take_last_result(&config(&w)).unwrap();
    assert_eq!(r.status, SwapStatus::RolledBack);
    assert!(r.message.contains("did not finish starting"), "{}", r.message);
    // the hung process must really be gone (killed by the install step), not left running for a minute
    let pid: u32 = std::fs::read_to_string(w.dir.join("hang.pid")).unwrap().trim().parse().unwrap();
    assert!(wait_until(Duration::from_secs(10), || !process_alive(pid)), "the hung new app (pid {pid}) is still running");
}

#[test]
fn a_wrong_hash_never_gets_as_far_as_the_install_step() {
    let w = world("real-badhash", "good 2.0.0", Some(format!("sha256:{}", sha256_of(b"other"))), None);
    assert_eq!(run_update_app(&w, 3000), 11, "the app reports the update failed");
    let run_log = log(w.dir.path(), "update-run.log");
    assert!(run_log.last().unwrap().contains("HashMismatch"), "{run_log:?}");
    assert_eq!(std::fs::read(&w.exe).unwrap(), w.old);
    let mut files: Vec<String> = std::fs::read_dir(w.dir.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    files.sort();
    assert_eq!(files, vec!["BoylerUtilities.exe", "started.log", "temp", "update-run.log"], "no staged or partial file left");
    assert_eq!(log(w.dir.path(), "started.log").len(), 1, "nothing was started");
}

#[test]
fn a_wrong_listed_size_is_refused_over_real_http() {
    let w = world("real-badsize", "good 2.0.0", None, Some(1234));
    assert_eq!(run_update_app(&w, 3000), 11);
    assert!(log(w.dir.path(), "update-run.log").last().unwrap().contains("SizeMismatch"));
    assert_eq!(std::fs::read(&w.exe).unwrap(), w.old);
}

#[test]
fn cleanup_after_a_finished_update_leaves_only_the_app() {
    let w = world("real-cleanup", "good 2.0.0", None, None);
    assert_eq!(run_update_app(&w, 8000), 0);
    assert!(wait_until(Duration::from_secs(20), || log(w.dir.path(), "started.log").iter().any(|l| l.contains("confirmed"))));
    assert!(wait_until(Duration::from_secs(10), || !w.dir.join("BoylerUtilities.exe.update-old").exists()));
    let mut cfg = config(&w);
    cfg.temp_dir = w.dir.join("temp");
    // the install step's own copy may still be shutting down; give it a moment, then the next normal start tidies up
    assert!(wait_until(Duration::from_secs(15), || {
        cleanup_leftovers(&cfg, &["app.exe".into()]);
        std::fs::read_dir(w.dir.join("temp")).map(|d| d.count() == 0).unwrap_or(true)
    }));
    assert_eq!(std::fs::read(&w.exe).unwrap(), w.new);
}

fn process_alive(pid: u32) -> bool {
    let out = Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/NH"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
}

#[test]
fn a_folder_with_spaces_and_non_english_letters_works_like_program_files_would() {
    let w = world("real spaces č š ž & (x86)", "good 2.0.0", None, None);
    assert_eq!(run_update_app(&w, 8000), 0, "update-run.log: {:?}", log(w.dir.path(), "update-run.log"));
    assert!(wait_until(Duration::from_secs(20), || log(w.dir.path(), "started.log").iter().any(|l| l.contains("confirmed"))), "{:?}", log(w.dir.path(), "started.log"));
    assert!(wait_until(Duration::from_secs(10), || !w.dir.join("BoylerUtilities.exe.update-old").exists()));
    assert_eq!(std::fs::read(&w.exe).unwrap(), w.new);
}

#[test]
fn an_install_step_that_cannot_read_its_plan_starts_the_app_again_and_leaves_a_note() {
    let dir = Scratch::new("real-noplan");
    let exe = dir.join("BoylerUtilities.exe");
    let old = app_bytes("old 1.0.0");
    std::fs::write(&exe, &old).unwrap();
    // the install step = a copy of the app, as RealInstaller makes it; its plan file is garbage
    let helper = dir.join("install-step.exe");
    std::fs::write(&helper, &old).unwrap();
    let plan = dir.join("plan.json");
    std::fs::write(&plan, "{ this is not a plan").unwrap();
    let result = dir.join("BoylerUtilities.exe.update-result");
    // the "exiting app": a process that is already gone
    let mut gone = Command::new("cmd").args(["/C", "exit 0"]).spawn().unwrap();
    gone.wait().unwrap();
    let fb = Fallback { target: exe.clone(), result_file: result.clone(), old_pid: gone.id(), exit_wait_ms: 5000 };
    let mut args = vec![HELPER_ARG.to_string(), plan.to_string_lossy().into_owned()];
    args.extend(fb.to_args());
    let code = Command::new(&helper).args(&args).current_dir(dir.path()).status().unwrap().code();
    assert_eq!(code, Some(2), "the install step reports that it could not do its job");

    assert!(wait_until(Duration::from_secs(20), || !log(dir.path(), "started.log").is_empty()), "the app was not started again");
    assert_eq!(log(dir.path(), "started.log"), vec!["old 1.0.0 | ".to_string()], "started once, plain, no update arguments");
    assert_eq!(std::fs::read(&exe).unwrap(), old, "nothing was changed");
    let r = take_last_result(&config_for(&exe)).expect("the reason is left for the app's first look");
    assert_eq!(r.status, SwapStatus::Aborted);
    assert!(r.message.contains("could not be read"), "{}", r.message);
}

fn config_for(exe: &Path) -> UpdaterConfig {
    UpdaterConfig::new("o/r", "1.0.0", exe.to_path_buf())
}

#[test]
fn an_install_step_started_by_an_older_caller_without_fallback_values_just_ends() {
    let dir = Scratch::new("real-noplan-old");
    let exe = dir.join("BoylerUtilities.exe");
    std::fs::write(&exe, app_bytes("old 1.0.0")).unwrap();
    let helper = dir.join("install-step.exe");
    std::fs::write(&helper, app_bytes("old 1.0.0")).unwrap();
    let plan = dir.join("plan.json");
    std::fs::write(&plan, "garbage").unwrap();
    let code = Command::new(&helper).args([HELPER_ARG, &plan.to_string_lossy()]).current_dir(dir.path()).status().unwrap().code();
    assert_eq!(code, Some(2));
    std::thread::sleep(Duration::from_millis(500));
    assert!(log(dir.path(), "started.log").is_empty(), "nothing is started when the install step knows nothing");
}

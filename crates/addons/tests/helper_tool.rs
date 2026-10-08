//! The helper's tool runner on a stand-in tool (examples/fake_tool.rs - `cargo test` builds the crate's examples into
//! `target\<profile>\examples`). Nothing elevated, nothing of Raw Accel runs.

#![cfg(windows)]

use bu_addons::helper::{judge, run_tool};
use bu_addons::HelperAction;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn fake_tool() -> PathBuf {
    // target\<profile>\deps\helper_tool-xxxx.exe -> target\<profile>\examples\addons-fake-tool.exe
    let exe = std::env::current_exe().unwrap();
    let p = exe.parent().unwrap().parent().unwrap().join("examples").join("addons-fake-tool.exe");
    assert!(p.is_file(), "build the example first: cargo build -p bu-addons --examples ({})", p.display());
    p
}

#[test]
fn the_tool_runs_hidden_and_its_key_is_answered() {
    let t = Instant::now();
    let out = run_tool(&fake_tool(), &["ok"], Duration::from_secs(30)).unwrap();
    assert!(out.contains("Install complete"), "{out:?}");
    assert!(judge(HelperAction::RawAccelInstall, &out).is_ok());
    assert!(t.elapsed() < Duration::from_secs(10), "answered at once, not at the timeout");
}

#[test]
fn its_error_line_comes_back() {
    let out = run_tool(&fake_tool(), &["fail"], Duration::from_secs(30)).unwrap();
    assert_eq!(judge(HelperAction::RawAccelInstall, &out).unwrap_err(), "Raw Accel\u{2019}s installer: Error: copy_file failed: Access is denied. system:5");
}

#[test]
fn a_stuck_tool_is_ended() {
    let t = Instant::now();
    let r = run_tool(&fake_tool(), &["hang"], Duration::from_secs(2));
    assert!(r.unwrap_err().contains("did not finish in time"));
    assert!(t.elapsed() < Duration::from_secs(15));
}

#[test]
fn the_tool_runs_with_its_own_small_environment() {
    let win = bu_addons::helper::windows_dir().expect("Windows' folder");
    assert!(win.join("System32").is_dir());
    assert!(!bu_addons::helper::is_reparse(&win.join("Temp")), "Windows' Temp is a real folder here");
    let work = std::env::temp_dir();
    let env = bu_addons::helper::tool_env(&win, &work);
    assert!(env.starts_with(&format!("SystemRoot={}\0", win.display())));
    assert!(env.contains(&format!("PATH={}\\System32;{}\0", win.display(), win.display())));
    assert!(env.ends_with("\0\0"));
    assert!(!env.to_ascii_lowercase().contains("appdata\\local\\microsoft\\windowsapps"), "no user folders on its PATH");
    let out = bu_addons::helper::run_tool_env(&fake_tool(), &["ok"], Duration::from_secs(30), Some(&env)).unwrap();
    assert!(out.contains("Install complete"), "{out:?}");
}

#[test]
fn every_helper_code_has_a_line() {
    use bu_addons::helper::{message, Code};
    for c in [Code::BadArgs, Code::NotOfficial, Code::Prepare, Code::ToolError, Code::NotFinished, Code::NoStart] {
        assert!(!message(HelperAction::RawAccelInstall, c as u32).contains("code"), "{c:?}");
    }
    assert_eq!(message(HelperAction::RawAccelUninstall, 5), "Raw Accel\u{2019}s uninstaller reported an error");
    assert!(message(HelperAction::RawAccelInstall, 99).contains("code 99"));
}

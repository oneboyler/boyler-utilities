//! The REAL uninstaller runner, proven with harmless hidden commands only (cmd.exe / ping to 127.0.0.1) — never an uninstaller.
//! Shows: the exit code comes back, and the wait also covers a process the first one started and left running (the way NSIS /
//! Inno uninstallers hand over to a temp copy).
#![cfg(windows)]

use bu_apps::real::{run_and_wait, HIDDEN};
use std::time::{Duration, Instant};

#[test]
fn exit_code_comes_back() {
    assert_eq!(run_and_wait(r"C:\Windows\System32\cmd.exe /c exit 3010", HIDDEN), Ok(3010));
    assert_eq!(run_and_wait(r"C:\Windows\System32\cmd.exe /c exit 0", HIDDEN), Ok(0));
}

#[test]
fn waits_for_the_child_the_first_process_leaves_behind() {
    // cmd starts ping in the background and exits at once; ping -n 3 runs ~2 s.
    let t = Instant::now();
    let code = run_and_wait(r#"C:\Windows\System32\cmd.exe /c start "" /b C:\Windows\System32\PING.EXE -n 3 127.0.0.1"#, HIDDEN);
    let took = t.elapsed();
    assert_eq!(code, Ok(0));
    assert!(took >= Duration::from_millis(1500), "returned after {took:?}: did not wait for the child");
    println!("waited {} ms for the left-behind child (measured)", took.as_millis());
}

#[test]
fn missing_program_is_an_error_not_a_panic() {
    assert!(run_and_wait(r#""C:\does-not-exist\nope.exe" /S"#, HIDDEN).is_err());
}

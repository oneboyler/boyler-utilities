//! The install step's logic (run_swap) with scripted processes (FakeProcs) on REAL files in a scratch folder: swap, keep the old
//! one, roll back on every kind of failure. Locks are real Windows share-mode locks, so "the file can't be moved" is genuine.

mod common;

use bu_updater::*;
use common::*;
use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::rc::Rc;

struct Rig {
    dir: Scratch,
    plan: SwapPlan,
    old: Vec<u8>,
    new: Vec<u8>,
}

fn rig() -> Rig {
    let dir = Scratch::new("swap-logic");
    let old = fake_exe_bytes("old", 3000);
    let new = fake_exe_bytes("new", 5000);
    let target = dir.join("app.exe");
    let new_file = dir.join("app.exe.update-new");
    std::fs::write(&target, &old).unwrap();
    std::fs::write(&new_file, &new).unwrap();
    let plan = SwapPlan {
        target,
        new_file,
        old_file: dir.join("app.exe.update-old"),
        result_file: dir.join("app.exe.update-result"),
        old_pid: 4242,
        expected_size: new.len() as u64,
        expected_sha256: Some(sha256_of(&new)),
        from_version: "1.0.0".into(),
        to_version: "2.0.0".into(),
        event_name: "Local\\BoylerUtilities.Update.test".into(),
        exit_wait_ms: 10,
        start_wait_ms: 10,
        rename_retries: 2,
        rename_retry_ms: 10,
    };
    Rig { dir, plan, old, new }
}

fn result_file(r: &Rig) -> Option<SwapResult> {
    std::fs::read_to_string(&r.plan.result_file).ok().and_then(|t| serde_json::from_str(&t).ok())
}

/// Opens a file so it can be read but not renamed or deleted (a real sharing lock, like an antivirus scan holds).
fn lock(p: &PathBuf) -> File {
    OpenOptions::new().read(true).share_mode(1).open(p).unwrap()
}

#[test]
fn a_good_swap_installs_the_new_file_starts_it_and_deletes_the_old_one() {
    let r = rig();
    let mut procs = FakeProcs::new();
    let res = run_swap(&r.plan, &mut procs);
    assert_eq!(res.status, SwapStatus::Updated, "{res:?}");
    assert_eq!(std::fs::read(&r.plan.target).unwrap(), r.new);
    assert!(!r.plan.old_file.exists(), "the old file is deleted once the new one confirmed");
    assert!(!r.plan.new_file.exists());
    assert!(result_file(&r).is_none(), "nothing to report after a success");
    assert_eq!(procs.events, vec![r.plan.event_name.clone()], "the start event is created before the new app starts");
    assert_eq!(procs.launches.len(), 1);
    assert_eq!(procs.launches[0].0, vec!["--bu-updated".to_string(), r.plan.event_name.clone(), "1.0.0".to_string()]);
    assert_eq!(procs.launches[0].1, r.new, "what was started is the NEW file");
    assert!(procs.killed.is_empty());
}

#[test]
fn the_old_version_still_exists_while_the_new_one_is_starting() {
    // at the moment the new app is launched, the old file must be on disk (that is what makes the roll back possible)
    let r = rig();
    let seen = Rc::new(RefCell::new(None));
    let (old_file, s2) = (r.plan.old_file.clone(), seen.clone());
    let mut procs = FakeProcs::new();
    procs.on_launch = Some(Box::new(move |_| *s2.borrow_mut() = Some(std::fs::read(&old_file).ok())));
    run_swap(&r.plan, &mut procs);
    assert_eq!(seen.borrow().clone().unwrap().unwrap(), r.old);
}

#[test]
fn the_old_app_not_closing_changes_nothing_and_starts_no_second_copy() {
    let r = rig();
    let mut procs = FakeProcs { old_exits: false, ..FakeProcs::new() };
    let res = run_swap(&r.plan, &mut procs);
    assert_eq!(res.status, SwapStatus::Aborted);
    assert_eq!(std::fs::read(&r.plan.target).unwrap(), r.old);
    assert!(!r.plan.new_file.exists(), "the staged file is cleaned up");
    assert!(procs.launches.is_empty(), "the old app is still running - do not start another");
    assert_eq!(result_file(&r).unwrap().status, SwapStatus::Aborted);
}

#[test]
fn a_staged_file_that_changed_is_refused_and_the_old_app_is_started_again() {
    for what in ["hash", "size", "missing"] {
        let r = rig();
        match what {
            "hash" => {
                let mut b = r.new.clone();
                let n = b.len() / 2;
                b[n] ^= 0xFF;
                std::fs::write(&r.plan.new_file, b).unwrap();
            }
            "size" => std::fs::write(&r.plan.new_file, &r.new[..r.new.len() - 10]).unwrap(),
            _ => std::fs::remove_file(&r.plan.new_file).unwrap(),
        }
        let mut procs = FakeProcs::new();
        let res = run_swap(&r.plan, &mut procs);
        assert_eq!(res.status, SwapStatus::Aborted, "{what}: {res:?}");
        assert_eq!(std::fs::read(&r.plan.target).unwrap(), r.old, "{what}");
        assert!(!r.plan.new_file.exists(), "{what}");
        assert_eq!(procs.launches.len(), 1, "{what}");
        assert!(procs.launches[0].0.is_empty(), "{what}: plain start");
        assert_eq!(procs.launches[0].1, r.old, "{what}: it is the old app that is started");
        assert_eq!(result_file(&r).unwrap().status, SwapStatus::Aborted, "{what}");
    }
}

#[test]
fn an_app_file_that_cannot_be_moved_aside_changes_nothing() {
    let r = rig();
    let _held = lock(&r.plan.target);
    let mut procs = FakeProcs::new();
    let res = run_swap(&r.plan, &mut procs);
    assert_eq!(res.status, SwapStatus::Aborted, "{res:?}");
    drop(_held);
    assert_eq!(std::fs::read(&r.plan.target).unwrap(), r.old);
    assert!(!r.plan.old_file.exists());
    assert_eq!(procs.launches.len(), 1);
    assert_eq!(procs.launches[0].1, r.old);
}

#[test]
fn if_the_new_file_cannot_be_put_in_place_the_old_one_is_put_back() {
    let r = rig();
    let held = lock(&r.plan.new_file); // reading works (so the re-check passes) but it can't be renamed
    let mut procs = FakeProcs::new();
    let res = run_swap(&r.plan, &mut procs);
    drop(held);
    assert_eq!(res.status, SwapStatus::RolledBack, "{res:?}");
    assert_eq!(std::fs::read(&r.plan.target).unwrap(), r.old);
    assert!(!r.plan.old_file.exists(), "the old file went back to its place");
    assert_eq!(procs.launches.len(), 1);
    assert_eq!(procs.launches[0].1, r.old);
    assert_eq!(result_file(&r).unwrap().status, SwapStatus::RolledBack);
}

#[test]
fn a_new_app_that_closes_right_away_is_rolled_back() {
    let r = rig();
    let mut procs = FakeProcs { start_outcomes: vec![StartOutcome::Exited], ..FakeProcs::new() };
    let res = run_swap(&r.plan, &mut procs);
    assert_eq!(res.status, SwapStatus::RolledBack, "{res:?}");
    assert!(res.message.contains("closed right after starting"), "{}", res.message);
    assert_eq!(std::fs::read(&r.plan.target).unwrap(), r.old, "the old version is back");
    assert!(!r.plan.old_file.exists());
    assert!(!r.plan.new_file.exists());
    assert_eq!(procs.launches.len(), 2);
    assert_eq!(procs.launches[0].1, r.new);
    assert_eq!(procs.launches[1].1, r.old, "and it was started again");
    assert!(procs.launches[1].0.is_empty());
    let saved = result_file(&r).unwrap();
    assert_eq!((saved.status, saved.from_version.as_str(), saved.to_version.as_str()), (SwapStatus::RolledBack, "1.0.0", "2.0.0"));
}

#[test]
fn a_new_app_that_never_says_it_is_up_is_stopped_and_rolled_back() {
    let r = rig();
    let mut procs = FakeProcs { start_outcomes: vec![StartOutcome::TimedOut], ..FakeProcs::new() };
    let res = run_swap(&r.plan, &mut procs);
    assert_eq!(res.status, SwapStatus::RolledBack, "{res:?}");
    assert_eq!(procs.killed, vec![1], "the hanging new app is stopped before its file is replaced");
    assert_eq!(std::fs::read(&r.plan.target).unwrap(), r.old);
    assert_eq!(procs.launches.len(), 2);
    assert_eq!(procs.launches[1].1, r.old);
}

#[test]
fn a_new_app_that_cannot_be_started_is_rolled_back() {
    let r = rig();
    let mut procs = FakeProcs { fail_launch: Some(0), ..FakeProcs::new() };
    let res = run_swap(&r.plan, &mut procs);
    assert_eq!(res.status, SwapStatus::RolledBack, "{res:?}");
    assert_eq!(std::fs::read(&r.plan.target).unwrap(), r.old);
    assert_eq!(procs.launches.len(), 2);
    assert_eq!(procs.launches[1].1, r.old);
}

#[test]
fn if_even_the_roll_back_fails_the_old_file_is_kept_and_the_result_says_where() {
    let r = rig();
    let held: Rc<RefCell<Option<File>>> = Rc::default();
    let (old_file, h2) = (r.plan.old_file.clone(), held.clone());
    let mut procs = FakeProcs { start_outcomes: vec![StartOutcome::Exited], ..FakeProcs::new() };
    // at the moment the new app starts, something (antivirus...) takes a lock on the kept old file
    procs.on_launch = Some(Box::new(move |n| {
        if n == 0 {
            *h2.borrow_mut() = Some(lock(&old_file));
        }
    }));
    let res = run_swap(&r.plan, &mut procs);
    assert_eq!(res.status, SwapStatus::RollbackFailed, "{res:?}");
    assert!(res.message.contains("app.exe.update-old"), "{}", res.message);
    *held.borrow_mut() = None;
    assert_eq!(std::fs::read(&r.plan.old_file).unwrap(), r.old, "the old version is still on disk, nothing is lost");
    assert_eq!(result_file(&r).unwrap().status, SwapStatus::RollbackFailed);
}

#[test]
fn a_leftover_old_file_from_an_earlier_attempt_does_not_block_the_swap() {
    let r = rig();
    std::fs::write(&r.plan.old_file, b"MZ-stale-old").unwrap();
    let res = run_swap(&r.plan, &mut FakeProcs::new());
    assert_eq!(res.status, SwapStatus::Updated, "{res:?}");
    assert_eq!(std::fs::read(&r.plan.target).unwrap(), r.new);
    assert!(!r.plan.old_file.exists());
}

#[test]
fn without_a_listed_hash_the_size_alone_is_rechecked() {
    let mut r = rig();
    r.plan.expected_sha256 = None;
    assert_eq!(run_swap(&r.plan, &mut FakeProcs::new()).status, SwapStatus::Updated);
    let mut r = rig();
    r.plan.expected_sha256 = None;
    std::fs::write(&r.plan.new_file, &r.new[..100]).unwrap();
    assert_eq!(run_swap(&r.plan, &mut FakeProcs::new()).status, SwapStatus::Aborted);
}

#[test]
fn the_failure_result_is_what_the_app_reads_at_its_next_start() {
    let r = rig();
    let mut procs = FakeProcs { start_outcomes: vec![StartOutcome::Exited], ..FakeProcs::new() };
    let res = run_swap(&r.plan, &mut procs);
    // same naming as UpdaterConfig::side_file("result")
    let cfg = UpdaterConfig::new("o/r", "1.0.0", r.plan.target.clone());
    assert_eq!(take_last_result(&cfg), Some(res));
    assert_eq!(take_last_result(&cfg), None);
}

#[test]
fn the_plan_survives_the_json_file_it_travels_in() {
    let r = rig();
    let text = serde_json::to_string_pretty(&r.plan).unwrap();
    let back: SwapPlan = serde_json::from_str(&text).unwrap();
    assert_eq!(back, r.plan);
    let _ = &r.dir;
}

// ------------------------------------------------------------------------- the install step could not read its plan

fn fallback(r: &Rig) -> Fallback {
    Fallback { target: r.plan.target.clone(), result_file: r.plan.result_file.clone(), old_pid: 4242, exit_wait_ms: 10 }
}

#[test]
fn fallback_arguments_round_trip_and_bad_ones_are_refused() {
    let fb = Fallback { target: PathBuf::from(r"C:\Program Files\Boyler Utilities\BoylerUtilities.exe"), result_file: PathBuf::from(r"C:\x y\r.update-result"), old_pid: 77, exit_wait_ms: 60000 };
    assert_eq!(Fallback::from_args(&fb.to_args()), Some(fb.clone()));
    let mut with_more = fb.to_args();
    with_more.push("later".into());
    assert_eq!(Fallback::from_args(&with_more), Some(fb.clone()), "extra arguments are ignored");
    assert_eq!(Fallback::from_args(&[]), None);
    assert_eq!(Fallback::from_args(&fb.to_args()[..3]), None, "an older caller passed no fallback");
    let mut bad = fb.to_args();
    bad[2] = "not a pid".into();
    assert_eq!(Fallback::from_args(&bad), None);
    let mut empty = fb.to_args();
    empty[0] = String::new();
    assert_eq!(Fallback::from_args(&empty), None);
}

#[test]
fn an_unreadable_plan_leaves_the_app_untouched_writes_why_and_starts_the_app_again() {
    let r = rig();
    let mut procs = FakeProcs::new();
    let res = recover_without_plan(&fallback(&r), &mut procs);
    assert_eq!(res.status, SwapStatus::Aborted);
    assert_eq!(procs.launches.len(), 1, "the app is started again");
    assert_eq!(procs.launches[0].0, Vec::<String>::new(), "plain start, no update arguments");
    assert_eq!(procs.launches[0].1, r.old, "the installed (old) app is what starts");
    assert_eq!(std::fs::read(&r.plan.target).unwrap(), r.old, "nothing was changed");
    let written = result_file(&r).expect("the reason is written for the app's next start");
    assert_eq!(written.status, SwapStatus::Aborted);
    assert!(written.message.contains("could not be read") && written.message.contains("nothing was changed"), "{}", written.message);
}

#[test]
fn an_unreadable_plan_with_an_app_that_does_not_close_starts_no_second_copy() {
    let r = rig();
    let mut procs = FakeProcs { old_exits: false, ..FakeProcs::new() };
    let res = recover_without_plan(&fallback(&r), &mut procs);
    assert_eq!(res.status, SwapStatus::Aborted);
    assert!(procs.launches.is_empty(), "the old app is still running: no second copy");
    assert!(result_file(&r).unwrap().message.contains("did not close in time"));
    assert_eq!(std::fs::read(&r.plan.target).unwrap(), r.old);
}

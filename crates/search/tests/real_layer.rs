//! The real Windows layer, read-only: the app list, the Windows Search service and scope, and a graceful answer when the
//! index is off (it is OFF on the PC this was built on). Nothing is opened, nothing is copied.
#![cfg(windows)]

use bu_search::real::wsearch;
use bu_search::*;
use std::sync::{mpsc, Arc};
use std::time::Duration;

#[test]
fn rule_urls_become_folders() {
    assert_eq!(wsearch::parse_rule_url(r"file:///C:\[728cd639-f7da-4203-8ab7-d1ee68d040a9]\Users\").as_deref(), Some(r"C:\Users\"));
    assert_eq!(
        wsearch::parse_rule_url(r"file:///C:\[728cd639-f7da-4203-8ab7-d1ee68d040a9]\ProgramData\Microsoft\Windows\Start Menu\").as_deref(),
        Some(r"C:\ProgramData\Microsoft\Windows\Start Menu\")
    );
    assert_eq!(wsearch::parse_rule_url(r"file:///*\$RECYCLE.BIN\").as_deref(), Some(r"*\$RECYCLE.BIN\"));
    assert_eq!(wsearch::parse_rule_url(r"file:///D:\"), Some(r"D:\".to_string()));
    assert_eq!(wsearch::parse_rule_url("csc://{S-1-5-21-1}/"), None);
    assert_eq!(wsearch::parse_rule_url("winrt://{S-1-5-21-1}/"), None);
}

#[test]
fn the_app_list_is_real_and_complete_enough() {
    let apps = RealOs::read_only().list_apps().unwrap();
    assert!(apps.len() > 10, "{} apps", apps.len());
    assert!(apps.iter().all(|a| !a.name.is_empty() && !a.parsing_name.is_empty()));
    // something every Windows 11 has
    assert!(apps.iter().any(|a| a.name.to_lowercase().contains("notepad")), "no Notepad among {} apps", apps.len());
}

#[test]
fn searching_apps_on_the_real_pc() {
    let svc = SearchService::new(Arc::new(RealOs::read_only()));
    let r = svc.search(&Query::new("notepad", Filter::Apps), &Cancel::new()).unwrap();
    assert!(r.groups[0].items.iter().any(|i| i.name.to_lowercase().starts_with("notepad")));
    assert!(r.flat().iter().all(|i| matches!(i.open, OpenTarget::App(_))));
    svc.release();
}

#[test]
fn the_index_service_and_scope_read_without_admin() {
    let os = RealOs::read_only();
    let st = os.windows_search_status();
    assert!(matches!(st, WsStatus::Running | WsStatus::Stopped | WsStatus::Disabled | WsStatus::Missing), "{st:?}");
    let scope = os.windows_search_scope();
    // a normal Windows install has its crawl rules; when it does, the user's folders are among the included ones
    if !scope.included.is_empty() {
        assert!(scope.included.iter().any(|p| p.to_lowercase().contains(r"\users")), "{:?}", scope.included);
    }
}

/// Whatever the state of Windows Search on the test PC, a query comes back (rows, or an error) - it never hangs or panics.
#[test]
fn a_windows_search_query_comes_back_either_way() {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let fq = FileQuery { words: vec!["boyler".into()], folders: false, extensions: vec![], max: 20 };
        let _ = tx.send((RealOs::read_only().windows_search_status(), RealOs::read_only().windows_search_query(&fq)));
    });
    let (status, result) = rx.recv_timeout(Duration::from_secs(60)).expect("the query did not come back in 60 s");
    match (status, result) {
        (WsStatus::Running, Ok(h)) => assert!(h.items.len() <= 20),
        (WsStatus::Running, Err(e)) => panic!("the index runs but the query failed: {e}"),
        (_, Err(SearchError::Os { .. })) => {}   // service off: the provider says so
        (s, other) => panic!("service {s:?} but query gave {other:?}"),
    }
}

#[test]
fn without_everything_the_service_says_so_and_keeps_the_apps() {
    let os = Arc::new(RealOs::read_only());
    if matches!(os.everything_status(), EverythingStatus::Running { .. }) {
        return; // this test is about the case without Everything (this PC: not installed)
    }
    // the read-only layer never starts, stops or installs anything
    assert!(matches!(os.everything_start(), Err(SearchError::Refused(_))));
    assert!(matches!(os.everything_install(), Err(SearchError::Refused(_))));
    let svc = SearchService::new(os);
    let r = svc.search(&Query::new("notepad", Filter::All), &Cancel::new()).unwrap();
    assert_eq!(r.files_from, FilesFrom::Nothing);
    assert!(r.note.is_some());
    assert!(r.groups.iter().any(|g| g.kind == ItemKind::App));
}
#[test]
fn folder_counts_read_a_real_folder() {
    let dir = std::env::temp_dir().join(format!("bu-search-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "x").unwrap();
    std::fs::write(dir.join("b.txt"), "x").unwrap();
    assert_eq!(RealOs::read_only().dir_item_count(&dir.to_string_lossy()), Some(2));
    assert_eq!(RealOs::read_only().dir_item_count(r"C:\no\such\folder"), None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Harmless target only: `reveal` of a path that does not exist shows no window even if the guard were broken. `open`, `open with`
/// and the clipboard go through the same one-line `refuse` guard but are NOT called here - a broken guard would pop up a dialog
/// or overwrite the clipboard (TECH rule: refusals are proven only with harmless targets).
#[test]
fn read_only_layer_refuses_before_windows_is_asked() {
    let os = RealOs::read_only();
    let missing = r"C:\BoylerUtilities-scratch\lane-m\never-created\nothing.txt";
    assert!(!std::path::Path::new(missing).exists());
    assert_eq!(os.reveal(missing), Err(SearchError::Refused("reveal".into())));
}

/// Order 043: our Everything's index is kept only when it is whole and made with today's settings. Files in a scratch
/// folder this test creates and removes (never the app's own `%LOCALAPPDATA%` folder).
#[test]
fn our_index_is_kept_only_when_marked_whole() {
    use bu_search::real::host;
    let dir = std::path::PathBuf::from(r"C:\BoylerUtilities-scratch\043\search-index-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = host::db_file(&dir);
    // no index: a first build
    assert!(host::prepare_index(&dir));
    // an index from before the fix (no mark, e.g. a one-drive one) or one whose save was cut short: thrown away
    std::fs::write(&db, b"old").unwrap();
    assert!(host::prepare_index(&dir), "built again");
    assert!(!db.exists());
    // a mark of older settings: thrown away too
    std::fs::write(&db, b"old").unwrap();
    std::fs::write(host::mark_file(&dir), "BoylerUtilities index: v0").unwrap();
    assert!(host::prepare_index(&dir));
    assert!(!db.exists());
    // saved whole (a clean quit after it was loaded): loaded next time, and the mark is re-earned by each run
    std::fs::write(&db, b"whole").unwrap();
    host::mark_index(&dir);
    assert!(!host::prepare_index(&dir), "loads the saved one");
    assert!(db.exists());
    assert!(!host::mark_file(&dir).exists(), "this run must save cleanly again");
    // ... and if it does not (stopped while building, or ended mid-save), the next start builds
    assert!(host::prepare_index(&dir));
    assert!(!db.exists());
    // the settings point a named instance at the Everything service's pipe
    assert!(host::INI.contains("service_pipe_name=\\\\.\\PIPE\\Everything Service\r\n"), "{}", host::INI);
    std::fs::remove_dir_all(&dir).unwrap();
}

//! check() and update() against in-memory fakes and a scratch "install folder". No internet, no real app folder.

mod common;

use bu_updater::*;
use common::*;
use std::sync::Arc;

const API: &str = "https://api.example.test";
const LATEST: &str = "https://api.example.test/repos/o/r/releases/latest";
const ASSET_URL: &str = "https://dl.example.test/BoylerUtilities.exe";

struct Rig {
    dir: Scratch,
    http: Arc<FakeHttp>,
    installer: Arc<FakeInstaller>,
    updater: Updater,
    exe: std::path::PathBuf,
    old_bytes: Vec<u8>,
}

fn rig(repo: &str, current: &str) -> Rig {
    rig_with(repo, current, FakeHttp::new().with_chunk(1024), FakeInstaller::default())
}

fn rig_with(repo: &str, current: &str, http: FakeHttp, installer: FakeInstaller) -> Rig {
    let dir = Scratch::new("check-update");
    let exe = dir.join("BoylerUtilities.exe");
    let old_bytes = fake_exe_bytes("old", 4000);
    std::fs::write(&exe, &old_bytes).unwrap();
    let mut cfg = UpdaterConfig::new(repo, current, exe.clone());
    cfg.api_base = API.to_string();
    cfg.temp_dir = dir.join("temp");
    let http = Arc::new(http);
    let installer = Arc::new(installer);
    let updater = Updater::new(cfg, Box::new(http.clone()), Box::new(installer.clone()));
    Rig { dir, http, installer, updater, exe, old_bytes }
}

fn release_json(tag: &str, assets: &str) -> String {
    format!(r#"{{"tag_name":"{tag}","name":"{tag}","body":"- better\n- faster","html_url":"https://github.com/o/r/releases/tag/{tag}","draft":false,"prerelease":false,"assets":[{assets}]}}"#)
}

fn asset_json(name: &str, size: u64, digest: Option<&str>) -> String {
    let d = digest.map(|d| format!("\"{d}\"")).unwrap_or_else(|| "null".into());
    format!(r#"{{"name":"{name}","size":{size},"browser_download_url":"{ASSET_URL}","digest":{d}}}"#)
}

fn available(r: &Rig) -> ReleaseInfo {
    match r.updater.check().unwrap() {
        CheckResult::Available(i) => i,
        other => panic!("expected Available, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------------------------------------------- check

#[test]
fn empty_repo_is_not_set_up_and_uses_no_network() {
    for repo in ["", "   ", "\t"] {
        let r = rig(repo, "1.0.0");
        assert_eq!(r.updater.check().unwrap(), CheckResult::NotSetUp);
        assert!(r.http.requests().is_empty(), "no request may be made while the repo is empty");
    }
}

#[test]
fn building_the_updater_makes_no_request() {
    let r = rig("o/r", "1.0.0");
    assert!(r.http.requests().is_empty());
    drop(r.updater);
    assert!(r.http.requests().is_empty());
}

#[test]
fn a_bad_repo_setting_is_refused_before_any_request() {
    for repo in ["x", "a b/c", "a/b/c", "../x", "o/..", "/r", "o/", "o/r?x=1", "o/r#f", "o\\r", "https://github.com/o/r"] {
        let r = rig(repo, "1.0.0");
        assert!(matches!(r.updater.check(), Err(UpdateError::InvalidRepo(_))), "{repo:?}");
        assert!(r.http.requests().is_empty(), "{repo:?}");
    }
    assert_eq!(validate_repo(" owner/name.x-y_z ").unwrap(), "owner/name.x-y_z");
}

#[test]
fn check_asks_exactly_one_question_with_the_right_link() {
    let r = rig("o/r", "1.0.0");
    r.http.ok(LATEST, release_json("v1.0.0", &asset_json("BoylerUtilities.exe", 10, None)));
    assert!(matches!(r.updater.check().unwrap(), CheckResult::UpToDate { .. }));
    assert_eq!(r.http.requests(), vec![LATEST.to_string()]);
}

#[test]
fn same_or_older_release_is_up_to_date() {
    for tag in ["v1.0.0", "1.0", "0.9.9", "v0.1.0"] {
        let r = rig("o/r", "1.0.0");
        r.http.ok(LATEST, release_json(tag, &asset_json("BoylerUtilities.exe", 10, None)));
        match r.updater.check().unwrap() {
            CheckResult::UpToDate { current, latest } => {
                assert_eq!(current.to_string(), "1.0.0");
                assert_eq!(latest.to_string(), Version::parse(tag).unwrap().to_string());
            }
            other => panic!("{tag}: {other:?}"),
        }
    }
}

#[test]
fn a_newer_release_is_reported_with_its_file() {
    let r = rig("o/r", "1.0.0");
    let hex = sha256_of(b"abc");
    r.http.ok(LATEST, release_json("v1.10.0", &asset_json("BoylerUtilities.exe", 123, Some(&format!("sha256:{hex}")))));
    let info = available(&r);
    assert_eq!(info.version.to_string(), "1.10.0");
    assert_eq!(info.tag, "v1.10.0");
    assert_eq!(info.notes, "- better\n- faster");
    assert_eq!(info.asset.name, "BoylerUtilities.exe");
    assert_eq!(info.asset.size, 123);
    assert_eq!(info.asset.sha256.as_deref(), Some(hex.as_str()));
    assert_eq!(info.asset.url, ASSET_URL);
}

#[test]
fn a_prerelease_or_draft_is_never_offered() {
    for flag in ["prerelease", "draft"] {
        let r = rig("o/r", "1.0.0");
        let j = release_json("v9.0.0", &asset_json("BoylerUtilities.exe", 1, None)).replace(&format!("\"{flag}\":false"), &format!("\"{flag}\":true"));
        r.http.ok(LATEST, j);
        assert!(matches!(r.updater.check().unwrap(), CheckResult::UpToDate { .. }), "{flag}");
    }
}

#[test]
fn no_release_yet_is_a_result_not_an_error() {
    let r = rig("o/r", "1.0.0");
    r.http.status(LATEST, 404);
    assert_eq!(r.updater.check().unwrap(), CheckResult::NoReleases);
}

#[test]
fn network_and_server_trouble_are_typed_errors() {
    let r = rig("o/r", "1.0.0");
    r.http.route(LATEST, FakeReply::Offline("name not resolved".into()));
    assert!(matches!(r.updater.check(), Err(UpdateError::Network { .. })));
    r.http.status(LATEST, 403);
    assert_eq!(r.updater.check(), Err(UpdateError::RateLimited));
    r.http.status(LATEST, 429);
    assert_eq!(r.updater.check(), Err(UpdateError::RateLimited));
    r.http.status(LATEST, 500);
    assert!(matches!(r.updater.check(), Err(UpdateError::HttpStatus { status: 500, .. })));
    r.http.ok(LATEST, "<html>captive portal</html>");
    assert!(matches!(r.updater.check(), Err(UpdateError::BadResponse(_))));
}

#[test]
fn newer_release_without_a_windows_file_says_so() {
    let r = rig("o/r", "1.0.0");
    r.http.ok(LATEST, release_json("v2.0.0", &asset_json("source.zip", 10, None)));
    assert_eq!(r.updater.check(), Err(UpdateError::NoWindowsAsset { tag: "v2.0.0".into() }));
}

#[test]
fn a_current_version_that_is_not_a_version_is_an_error() {
    let r = rig("o/r", "dev");
    assert!(matches!(r.updater.check(), Err(UpdateError::BadVersion(_))));
}

// -------------------------------------------------------------------------------------------------------------- update

fn payload() -> Vec<u8> {
    fake_exe_bytes("new 2.0.0", 200 * 1024)
}

fn serve_release(r: &Rig, bytes: &[u8], listed_size: u64, digest: Option<String>) -> ReleaseInfo {
    r.http.ok(LATEST, release_json("v2.0.0", &asset_json("BoylerUtilities.exe", listed_size, digest.as_deref())));
    r.http.ok(ASSET_URL, bytes.to_vec());
    available(r)
}

fn leftovers(r: &Rig) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(r.dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != "BoylerUtilities.exe" && n != "temp")
        .collect();
    v.sort();
    v
}

fn assert_nothing_left_and_app_untouched(r: &Rig) {
    assert_eq!(leftovers(r), Vec::<String>::new(), "no half-downloaded or staged file may remain after a failure");
    assert_eq!(std::fs::read(&r.exe).unwrap(), r.old_bytes, "the running app's file must be untouched");
    assert!(r.installer.plans.lock().unwrap().is_empty(), "the install step must not be started after a failure");
}

#[test]
fn a_good_update_is_downloaded_verified_and_handed_to_the_install_step() {
    let r = rig("o/r", "1.0.0");
    let bytes = payload();
    let info = serve_release(&r, &bytes, bytes.len() as u64, Some(format!("sha256:{}", sha256_of(&bytes))));
    let mut events = Vec::new();
    let ready = r.updater.update(&info, &mut |p| events.push(p.clone())).unwrap();

    assert_eq!(ready, UpdateReady { from: "1.0.0".into(), to: "2.0.0".into(), bytes: bytes.len() as u64, verification: Verification::Sha256 });
    let staged = r.dir.join("BoylerUtilities.exe.update-new");
    assert_eq!(std::fs::read(&staged).unwrap(), bytes, "the staged file is exactly the download");
    assert!(!r.dir.join("BoylerUtilities.exe.update-part").exists());
    assert_eq!(std::fs::read(&r.exe).unwrap(), r.old_bytes, "update() itself never touches the running app");

    let plans = r.installer.plans.lock().unwrap();
    assert_eq!(plans.len(), 1);
    let p = &plans[0];
    assert_eq!(p.target, r.exe);
    assert_eq!(p.new_file, staged);
    assert_eq!(p.old_file, r.dir.join("BoylerUtilities.exe.update-old"));
    assert_eq!(p.result_file, r.dir.join("BoylerUtilities.exe.update-result"));
    assert_eq!(p.old_pid, std::process::id());
    assert_eq!((p.expected_size, p.expected_sha256.clone()), (bytes.len() as u64, Some(sha256_of(&bytes))));
    assert_eq!((p.from_version.as_str(), p.to_version.as_str()), ("1.0.0", "2.0.0"));
    assert!(p.event_name.starts_with("Local\\BoylerUtilities.Update."));

    // progress: phases in order, percent never goes back inside a phase, downloading reaches 100, last event = Restarting
    let phases: Vec<Phase> = events.iter().map(|e| e.phase).fold(Vec::new(), |mut v, p| {
        if v.last() != Some(&p) {
            v.push(p);
        }
        v
    });
    assert_eq!(phases, vec![Phase::Downloading, Phase::Verifying, Phase::Installing, Phase::Restarting]);
    for ph in [Phase::Downloading, Phase::Verifying] {
        let pcts: Vec<u8> = events.iter().filter(|e| e.phase == ph).filter_map(|e| e.percent).collect();
        assert!(pcts.windows(2).all(|w| w[0] <= w[1]), "{ph:?} percent went backwards: {pcts:?}");
        assert_eq!(pcts.first(), Some(&0), "{ph:?}");
        assert_eq!(pcts.last(), Some(&100), "{ph:?}");
    }
    let dl_bytes: Vec<u64> = events.iter().filter(|e| e.phase == Phase::Downloading).map(|e| e.bytes_done).collect();
    assert!(dl_bytes.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(dl_bytes.last(), Some(&(bytes.len() as u64)));
    assert_eq!(events.last().unwrap().phase, Phase::Restarting);
    assert!(events.len() >= 5 && events.len() <= 140, "events are throttled to about one per percent, got {}", events.len());
    assert_eq!(r.http.requests(), vec![LATEST.to_string(), ASSET_URL.to_string()]);
}

#[test]
fn without_a_listed_sha256_only_the_size_is_checked_and_it_says_so() {
    let r = rig("o/r", "1.0.0");
    let bytes = payload();
    let info = serve_release(&r, &bytes, bytes.len() as u64, None);
    let ready = r.updater.update(&info, &mut |_| {}).unwrap();
    assert_eq!(ready.verification, Verification::SizeOnly);
    // the install step still re-checks against the hash of what was downloaded
    assert_eq!(r.installer.plans.lock().unwrap()[0].expected_sha256, Some(sha256_of(&bytes)));
}

#[test]
fn with_neither_size_nor_hash_only_the_header_is_checked() {
    let r = rig("o/r", "1.0.0");
    let bytes = payload();
    let info = serve_release(&r, &bytes, 0, None);
    assert_eq!(r.updater.update(&info, &mut |_| {}).unwrap().verification, Verification::HeaderOnly);
}

#[test]
fn a_wrong_sha256_is_refused_and_nothing_is_left() {
    let r = rig("o/r", "1.0.0");
    let bytes = payload();
    let wrong = sha256_of(b"something else");
    let info = serve_release(&r, &bytes, bytes.len() as u64, Some(format!("sha256:{wrong}")));
    match r.updater.update(&info, &mut |_| {}) {
        Err(UpdateError::HashMismatch { expected, got }) => {
            assert_eq!(expected, wrong);
            assert_eq!(got, sha256_of(&bytes));
        }
        other => panic!("{other:?}"),
    }
    assert_nothing_left_and_app_untouched(&r);
}

#[test]
fn a_file_of_the_wrong_size_is_refused() {
    let bytes = payload();
    // shorter than listed, longer than listed (the safety stop), and a lying Content-Length
    for (listed, tag) in [(bytes.len() as u64 + 500, "shorter"), (bytes.len() as u64 - 500, "longer")] {
        let r = rig("o/r", "1.0.0");
        let info = serve_release(&r, &bytes, listed, None);
        assert!(matches!(r.updater.update(&info, &mut |_| {}), Err(UpdateError::SizeMismatch { .. })), "{tag}");
        assert_nothing_left_and_app_untouched(&r);
    }
    let r = rig("o/r", "1.0.0");
    let info = serve_release(&r, &bytes, 0, None);
    r.http.route(ASSET_URL, FakeReply::WrongLength { body: bytes.clone(), announced: bytes.len() as u64 + 9 });
    assert!(matches!(r.updater.update(&info, &mut |_| {}), Err(UpdateError::SizeMismatch { .. })));
    assert_nothing_left_and_app_untouched(&r);
}

#[test]
fn a_cut_download_is_a_network_error_and_leaves_nothing() {
    let r = rig("o/r", "1.0.0");
    let bytes = payload();
    let info = serve_release(&r, &bytes, bytes.len() as u64, None);
    r.http.route(ASSET_URL, FakeReply::Cut { body: bytes.clone(), sent: bytes.len() / 2 });
    assert!(matches!(r.updater.update(&info, &mut |_| {}), Err(UpdateError::Network { .. })));
    assert_nothing_left_and_app_untouched(&r);
}

#[test]
fn an_error_page_served_as_the_file_is_refused() {
    let r = rig("o/r", "1.0.0");
    let page = b"<html><body>Not a program, but status 200</body></html>".to_vec();
    let info = serve_release(&r, &page, page.len() as u64, None);
    assert_eq!(r.updater.update(&info, &mut |_| {}), Err(UpdateError::NotAnExecutable));
    assert_nothing_left_and_app_untouched(&r);
}

#[test]
fn a_download_error_status_is_reported_and_no_file_is_written() {
    let r = rig("o/r", "1.0.0");
    let bytes = payload();
    let info = serve_release(&r, &bytes, bytes.len() as u64, None);
    r.http.status(ASSET_URL, 404);
    assert!(matches!(r.updater.update(&info, &mut |_| {}), Err(UpdateError::HttpStatus { status: 404, .. })));
    assert_nothing_left_and_app_untouched(&r);
}

#[test]
fn an_insecure_download_link_is_refused_without_a_request() {
    let r = rig("o/r", "1.0.0");
    let bytes = payload();
    let mut info = serve_release(&r, &bytes, bytes.len() as u64, None);
    info.asset.url = "http://dl.example.test/BoylerUtilities.exe".into();
    assert!(matches!(r.updater.update(&info, &mut |_| {}), Err(UpdateError::InsecureDownload(_))));
    assert_eq!(r.http.requests().len(), 1, "only the check, nothing was fetched");
    assert_nothing_left_and_app_untouched(&r);
}

#[test]
fn a_file_over_the_limit_is_refused_without_a_request() {
    let r = rig("o/r", "1.0.0");
    let bytes = payload();
    let mut info = serve_release(&r, &bytes, bytes.len() as u64, None);
    info.asset.size = 301 * 1024 * 1024;
    assert!(matches!(r.updater.update(&info, &mut |_| {}), Err(UpdateError::TooLarge { .. })));
    assert_eq!(r.http.requests().len(), 1);
    assert_nothing_left_and_app_untouched(&r);
}

#[test]
fn an_unwritable_app_folder_stops_before_downloading() {
    let r = rig("o/r", "1.0.0");
    let bytes = payload();
    let info = serve_release(&r, &bytes, bytes.len() as u64, None);
    // a folder where the download file must go = the file can't be created (same effect as a read-only folder)
    std::fs::create_dir(r.dir.join("BoylerUtilities.exe.update-part")).unwrap();
    assert!(matches!(r.updater.update(&info, &mut |_| {}), Err(UpdateError::InstallDirNotWritable(_))));
    assert_eq!(r.http.requests().len(), 1, "no download was started");
    assert!(r.installer.plans.lock().unwrap().is_empty());
}

#[test]
fn a_missing_app_exe_is_an_error() {
    let r = rig("o/r", "1.0.0");
    let bytes = payload();
    let info = serve_release(&r, &bytes, bytes.len() as u64, None);
    std::fs::remove_file(&r.exe).unwrap();
    assert!(matches!(r.updater.update(&info, &mut |_| {}), Err(UpdateError::Io { .. })));
}

#[test]
fn if_the_install_step_cannot_start_the_staged_file_is_removed() {
    let r = rig_with("o/r", "1.0.0", FakeHttp::new().with_chunk(1024), FakeInstaller { fail_with: Some("no temp folder".into()), ..Default::default() });
    let bytes = payload();
    let info = serve_release(&r, &bytes, bytes.len() as u64, None);
    assert_eq!(r.updater.update(&info, &mut |_| {}), Err(UpdateError::HelperStart("no temp folder".into())));
    assert_nothing_left_and_app_untouched(&r);
}

#[test]
fn a_stale_staged_file_from_an_earlier_attempt_is_replaced() {
    let r = rig("o/r", "1.0.0");
    std::fs::write(r.dir.join("BoylerUtilities.exe.update-new"), b"MZ-stale").unwrap();
    let bytes = payload();
    let info = serve_release(&r, &bytes, bytes.len() as u64, None);
    r.updater.update(&info, &mut |_| {}).unwrap();
    assert_eq!(std::fs::read(r.dir.join("BoylerUtilities.exe.update-new")).unwrap(), bytes);
}

// ------------------------------------------------------------------------------------------- start-up helpers

#[test]
fn the_last_failed_result_is_read_once_and_deleted() {
    let r = rig("o/r", "1.0.0");
    let cfg = r.updater.config();
    assert_eq!(take_last_result(cfg), None);
    let res = SwapResult { status: SwapStatus::RolledBack, from_version: "1.0.0".into(), to_version: "2.0.0".into(), message: "the new version closed right after starting".into() };
    std::fs::write(r.dir.join("BoylerUtilities.exe.update-result"), serde_json::to_string(&res).unwrap()).unwrap();
    assert_eq!(take_last_result(cfg), Some(res));
    assert_eq!(take_last_result(cfg), None);
    std::fs::write(r.dir.join("BoylerUtilities.exe.update-result"), "garbage").unwrap();
    assert_eq!(take_last_result(cfg), None);
    assert!(!r.dir.join("BoylerUtilities.exe.update-result").exists());
}

#[test]
fn cleanup_removes_leftovers_but_never_the_app_or_a_running_update() {
    let r = rig("o/r", "1.0.0");
    let cfg = r.updater.config();
    let mk = || {
        for s in ["part", "new", "old"] {
            std::fs::write(r.dir.join(&format!("BoylerUtilities.exe.update-{s}")), b"x").unwrap();
        }
        std::fs::create_dir_all(r.dir.join("temp").join("BoylerUtilities-update-123")).unwrap();
        std::fs::create_dir_all(r.dir.join("temp").join("SomethingElse")).unwrap();
    };
    mk();
    // started by the install step: hands off
    cleanup_leftovers(cfg, &["app.exe".into(), "--bu-updated".into(), "ev".into(), "1.0.0".into()]);
    assert_eq!(leftovers(&r).len(), 3);
    assert!(r.dir.join("temp").join("BoylerUtilities-update-123").exists());
    // normal start: leftovers go
    cleanup_leftovers(cfg, &["app.exe".into()]);
    assert_eq!(leftovers(&r), Vec::<String>::new());
    assert!(!r.dir.join("temp").join("BoylerUtilities-update-123").exists());
    assert!(r.dir.join("temp").join("SomethingElse").exists(), "only the updater's own folders are removed");
    assert_eq!(std::fs::read(&r.exe).unwrap(), r.old_bytes);
    // the old file is the only copy when the app exe is missing: keep it
    std::fs::remove_file(&r.exe).unwrap();
    std::fs::write(r.dir.join("BoylerUtilities.exe.update-old"), b"only copy").unwrap();
    cleanup_leftovers(cfg, &["app.exe".into()]);
    assert!(r.dir.join("BoylerUtilities.exe.update-old").exists());
}

// ------------------------------------------------------------------------------------------- cancel + one at a time

fn good_release(r: &Rig) -> (Vec<u8>, ReleaseInfo) {
    let bytes = payload();
    let info = serve_release(r, &bytes, bytes.len() as u64, Some(format!("sha256:{}", sha256_of(&bytes))));
    (bytes, info)
}

#[test]
fn the_updater_can_be_shared_with_the_thread_that_has_the_cancel_button() {
    fn assert_sync<T: Send + Sync>() {}
    assert_sync::<Updater>();
}

#[test]
fn cancel_while_downloading_ends_with_cancelled_and_removes_everything() {
    let r = rig("o/r", "1.0.0");
    let (bytes, info) = good_release(&r);
    let mut last_pct = 0;
    let res = r.updater.update(&info, &mut |p| {
        if p.phase == Phase::Downloading {
            last_pct = p.percent.unwrap_or(0);
            if last_pct >= 30 {
                r.updater.cancel();
            }
        }
    });
    assert_eq!(res, Err(UpdateError::Cancelled));
    assert!(last_pct < 100, "the download must stop early, last percent {last_pct}");
    assert_nothing_left_and_app_untouched(&r);
    assert!(!r.updater.is_updating(), "the busy flag is released after a cancel");
    // and the very next update is not affected by the old click
    let res = r.updater.update(&info, &mut |_| {});
    assert_eq!(res.unwrap().bytes, bytes.len() as u64);
    assert_eq!(r.installer.plans.lock().unwrap().len(), 1);
}

#[test]
fn cancel_while_verifying_still_stops_before_the_install_step() {
    let r = rig("o/r", "1.0.0");
    let (_, info) = good_release(&r);
    let res = r.updater.update(&info, &mut |p| {
        if p.phase == Phase::Verifying {
            r.updater.cancel();
        }
    });
    assert_eq!(res, Err(UpdateError::Cancelled));
    assert_nothing_left_and_app_untouched(&r);
}

#[test]
fn cancel_after_the_install_step_is_started_is_too_late_and_the_update_goes_on() {
    let r = rig("o/r", "1.0.0");
    let (_, info) = good_release(&r);
    let res = r.updater.update(&info, &mut |p| {
        if p.phase == Phase::Restarting {
            r.updater.cancel();
        }
    });
    assert!(res.is_ok(), "{res:?}");
    assert_eq!(r.installer.plans.lock().unwrap().len(), 1);
}

#[test]
fn cancel_with_no_update_running_does_nothing_and_does_not_hit_the_next_one() {
    let r = rig("o/r", "1.0.0");
    let (_, info) = good_release(&r);
    r.updater.cancel();
    r.updater.cancel();
    assert!(!r.updater.is_updating());
    assert!(r.updater.update(&info, &mut |_| {}).is_ok());
}

#[test]
fn a_second_update_at_the_same_time_is_refused_and_the_first_is_not_harmed() {
    let r = rig("o/r", "1.0.0");
    let (bytes, info) = good_release(&r);
    let mut second: Vec<Result<UpdateReady>> = Vec::new();
    let mut busy_seen = Vec::new();
    let res = r.updater.update(&info, &mut |p| {
        if p.phase == Phase::Verifying && second.is_empty() {
            busy_seen.push(r.updater.is_updating());
            second.push(r.updater.update(&info, &mut |_| panic!("the refused call must not report progress")));
        }
    });
    assert_eq!(second.len(), 1);
    assert_eq!(second[0], Err(UpdateError::AlreadyRunning));
    assert_eq!(busy_seen, vec![true]);
    assert_eq!(res.unwrap().bytes, bytes.len() as u64, "the first update finished normally");
    assert_eq!(r.installer.plans.lock().unwrap().len(), 1, "one install step, not two");
    assert_eq!(r.http.requests(), vec![LATEST.to_string(), ASSET_URL.to_string()], "the refused call made no request");
    assert!(!r.updater.is_updating());
}

#[test]
fn the_busy_flag_is_released_after_every_kind_of_failure() {
    let r = rig("o/r", "1.0.0");
    let bytes = payload();
    let info = serve_release(&r, &bytes, bytes.len() as u64, Some(format!("sha256:{}", sha256_of(b"other"))));
    assert!(matches!(r.updater.update(&info, &mut |_| {}), Err(UpdateError::HashMismatch { .. })));
    assert!(!r.updater.is_updating());
    let info = serve_release(&r, &bytes, bytes.len() as u64, Some(format!("sha256:{}", sha256_of(&bytes))));
    assert!(r.updater.update(&info, &mut |_| {}).is_ok(), "a failed attempt does not lock the updater");
}

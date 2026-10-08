//! Get / Cancel / Remove of Raw Accel against the fakes: a pretend GitHub (FakeHttp) and the fake OS. The full Get path
//! needs the official release bytes (the pin is the real SHA-256): `BU_ADDONS_RELEASE_ZIP` = a scratch COPY of
//! RawAccel_v1.7.1.zip, served by the pretend GitHub. Without it those tests say so and pass on the parts they can prove.
//! Nothing is downloaded, installed or removed for real.

use bu_addons::fake::{Answer, FakeOs};
use bu_addons::rawaccel::{self, RaState, Step, PIN};
use bu_addons::{AddonError, HelperAction};
use bu_updater::http::{FakeHttp, FakeReply};
use std::path::PathBuf;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("bu-addons-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn never() -> bool {
    false
}

#[test]
fn state_follows_filter_and_driver() {
    let os = FakeOs::new();
    assert_eq!(rawaccel::state(&os), RaState::Absent);
    os.set_installed(true, false);
    assert_eq!(rawaccel::state(&os), RaState::InstallRestart);
    assert!(rawaccel::state(&os).got());
    os.set_installed(true, true);
    assert_eq!(rawaccel::state(&os), RaState::Installed);
    os.set_installed(false, true);
    assert_eq!(rawaccel::state(&os), RaState::RemoveRestart);
    assert!(!rawaccel::state(&os).got());
}

#[test]
fn the_size_reads_like_the_drawing() {
    assert_eq!(rawaccel::mb(PIN.size), "1.5 MB");
}

#[test]
fn a_changed_download_is_never_unpacked_or_installed() {
    let root = scratch("changed");
    let os = FakeOs::new();
    let http = FakeHttp::new().with_chunk(100_000);
    // right size, one byte different
    let mut body = vec![0u8; PIN.size as usize];
    body[0] = 1;
    http.ok(PIN.url, body);
    let mut steps = Vec::new();
    let r = rawaccel::get(&http, &os, &root, &mut |s| steps.push(s), &never);
    assert!(matches!(r, Err(AddonError::Verify(ref m)) if m.contains("SHA-256")), "{r:?}");
    assert!(os.runs().is_empty(), "nothing ran");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0, "nothing written");
    assert_eq!(steps.first(), Some(&Step::Download { got: 0, total: PIN.size }));
    assert!(steps.contains(&Step::Download { got: PIN.size, total: PIN.size }));
    assert_eq!(steps.last(), Some(&Step::Checking));
    assert_eq!(http.requests(), vec![PIN.url.to_string()]);
}

#[test]
fn a_wrong_size_is_refused() {
    let root = scratch("size");
    let os = FakeOs::new();
    let http = FakeHttp::new();
    http.ok(PIN.url, vec![0u8; 1000]);
    let r = rawaccel::get(&http, &os, &root, &mut |_| {}, &never);
    assert!(matches!(r, Err(AddonError::Verify(ref m)) if m.contains("wrong size")), "{r:?}");
    // bigger than the release: cut off while downloading
    http.ok(PIN.url, vec![0u8; PIN.size as usize + 10]);
    assert!(rawaccel::get(&http, &os, &root, &mut |_| {}, &never).is_err());
    assert!(os.runs().is_empty());
}

#[test]
fn offline_and_error_pages() {
    let root = scratch("offline");
    let os = FakeOs::new();
    let http = FakeHttp::new();
    http.route(PIN.url, FakeReply::Offline("no route".into()));
    assert!(matches!(rawaccel::get(&http, &os, &root, &mut |_| {}, &never), Err(AddonError::Network(_))));
    http.status(PIN.url, 404);
    assert!(matches!(rawaccel::get(&http, &os, &root, &mut |_| {}, &never), Err(AddonError::Network(ref m)) if m.contains("404")));
    assert!(os.runs().is_empty());
}

#[test]
fn cancel_stops_the_download() {
    let root = scratch("cancel");
    let os = FakeOs::new();
    let http = FakeHttp::new().with_chunk(64 * 1024);
    http.ok(PIN.url, vec![0u8; PIN.size as usize]);
    let seen = std::cell::Cell::new(0u64);
    let stop = || seen.get() > 300_000;
    let r = rawaccel::get(&http, &os, &root, &mut |s| {
        if let Step::Download { got, .. } = s {
            seen.set(got)
        }
    }, &stop);
    assert_eq!(r, Err(AddonError::Cancelled));
    assert!(seen.get() < PIN.size, "stopped part way: {}", seen.get());
    assert!(os.runs().is_empty());
}

/// The steps after the checks, on a pretend release whose unpacking is driven directly (the pinned SHA-256 can't be met by
/// a pretend zip): the driver hash check refuses a pretend driver, so this proves the order of refusals - a release with
/// a changed driver never reaches the install.
#[test]
fn a_changed_driver_inside_is_refused_and_cleaned_up() {
    let root = scratch("driver");
    let os = FakeOs::new();
    let zip = bu_addons::zip::build(&[("RawAccel/", b""), ("RawAccel/installer.exe", b"i"), ("RawAccel/uninstaller.exe", b"u"), ("RawAccel/driver/rawaccel.sys", b"changed")], true);
    let r = rawaccel::unpack(&os, &zip, &root);
    assert!(matches!(r, Err(AddonError::Verify(ref m)) if m.contains("driver")), "{r:?}");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0, "staging folder removed");
    let zip = bu_addons::zip::build(&[("RawAccel/installer.exe", b"i")], true);
    assert!(matches!(rawaccel::unpack(&os, &zip, &root), Err(AddonError::Verify(ref m)) if m.contains("no driver")));
    let zip = bu_addons::zip::build(&[("Other/installer.exe", b"i")], true);
    assert!(matches!(rawaccel::unpack(&os, &zip, &root), Err(AddonError::Files(_))));
}

fn release_zip() -> Option<Vec<u8>> {
    let p = std::env::var("BU_ADDONS_RELEASE_ZIP").ok()?;
    let b = std::fs::read(p).ok()?;
    (bu_addons::sha256_hex(&b) == PIN.sha256).then_some(b)
}

/// The whole Get on the official bytes: download (progress), size + SHA-256, unpack, the driver's hash + signature, one
/// elevated install of our folder; then Remove uses OUR uninstaller (no download) and deletes our folder.
#[test]
fn get_then_remove_with_the_official_release() {
    let Some(zip) = release_zip() else {
        eprintln!("BU_ADDONS_RELEASE_ZIP not set: the official-bytes path is not run here");
        return;
    };
    let root = scratch("get");
    let os = FakeOs::new();
    let http = FakeHttp::new().with_chunk(64 * 1024);
    http.ok(PIN.url, zip);
    let mut steps = Vec::new();
    let dir = rawaccel::get(&http, &os, &root, &mut |s| steps.push(s), &never).unwrap();
    assert_eq!(dir, root.join("RawAccel"));
    for f in ["installer.exe", "uninstaller.exe", "rawaccel.exe", "writer.exe", "driver/rawaccel.sys"] {
        assert!(dir.join(f).is_file(), "{f}");
    }
    assert!(!root.join("RawAccel.new").exists());
    assert_eq!(os.runs(), vec![(HelperAction::RawAccelInstall, dir.clone())]);
    let downloads = steps.iter().filter(|s| matches!(s, Step::Download { .. })).count();
    assert!(downloads > 20, "progress on every chunk: {downloads}");
    assert_eq!(&steps[steps.len() - 2..], &[Step::Checking, Step::Installing]);
    assert_eq!(rawaccel::state(&os), RaState::InstallRestart, "works after the restart");
    os.restart();
    assert_eq!(rawaccel::state(&os), RaState::Installed);

    let mut steps = Vec::new();
    rawaccel::remove(&http, &os, &root, None, &mut |s| steps.push(s), &never).unwrap();
    assert_eq!(steps, vec![Step::Removing], "our own official uninstaller: no download");
    assert_eq!(http.requests().len(), 1);
    assert_eq!(os.runs().last(), Some(&(HelperAction::RawAccelUninstall, dir.clone())));
    assert!(!dir.exists(), "our folder deleted");
    assert_eq!(rawaccel::state(&os), RaState::RemoveRestart);
}

/// Declined / failed install: our folder is deleted again, nothing left behind. A refused signature: never installed.
#[test]
fn declined_failed_and_unsigned_installs_leave_nothing() {
    let Some(zip) = release_zip() else {
        eprintln!("BU_ADDONS_RELEASE_ZIP not set: the official-bytes path is not run here");
        return;
    };
    let root = scratch("declined");
    let http = FakeHttp::new();
    http.ok(PIN.url, zip);
    let os = FakeOs::new();
    os.answer(Answer::Declined);
    assert_eq!(rawaccel::get(&http, &os, &root, &mut |_| {}, &never), Err(AddonError::Declined));
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    os.answer(Answer::Fails("Raw Accel\u{2019}s installer: Error: x".into()));
    assert!(matches!(rawaccel::get(&http, &os, &root, &mut |_| {}, &never), Err(AddonError::Tool(_))));
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    let os = FakeOs::new();
    os.signature_ok(false);
    assert!(matches!(rawaccel::get(&http, &os, &root, &mut |_| {}, &never), Err(AddonError::Verify(ref m)) if m.contains("signature")));
    assert!(os.runs().is_empty());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
}

/// Remove of a Raw Accel the user installed themselves (e.g. Desktop\RawAccel): its official uninstaller is used, no
/// download, the user's folder is kept. A changed uninstaller is never used (the release is fetched instead).
#[test]
fn remove_uses_the_users_official_uninstaller_and_keeps_his_folder() {
    let root = scratch("remove-root");
    let user = scratch("remove-user");
    let os = FakeOs::installed();
    let http = FakeHttp::new();
    http.route(PIN.url, FakeReply::Offline("no route".into()));
    std::fs::write(user.join("uninstaller.exe"), b"changed").unwrap();
    assert!(matches!(rawaccel::remove(&http, &os, &root, Some(&user), &mut |_| {}, &never), Err(AddonError::Network(_))));
    assert!(os.runs().is_empty());
    let Some(zip) = release_zip() else {
        eprintln!("BU_ADDONS_RELEASE_ZIP not set: the official-uninstaller path is not run here");
        return;
    };
    let e = bu_addons::zip::read(&zip, 16 << 20).unwrap();
    let un = e.iter().find(|x| x.name == "RawAccel/uninstaller.exe").unwrap();
    std::fs::write(user.join("uninstaller.exe"), &un.data).unwrap();
    let mut steps = Vec::new();
    rawaccel::remove(&http, &os, &root, Some(&user), &mut |s| steps.push(s), &never).unwrap();
    assert_eq!(http.requests().len(), 1, "only the refused try above");
    assert_eq!(os.runs(), vec![(HelperAction::RawAccelUninstall, user.clone())]);
    assert_eq!(steps, vec![Step::Removing]);
    assert!(user.join("uninstaller.exe").exists(), "the user's folder is kept");
    assert_eq!(rawaccel::state(&os), RaState::RemoveRestart);
    os.set_installed(true, true);
    os.answer(Answer::Declined);
    assert_eq!(rawaccel::remove(&http, &os, &root, Some(&user), &mut |_| {}, &never), Err(AddonError::Declined));
    assert_eq!(rawaccel::state(&os), RaState::Installed);
}

/// A second unpack (Get again) keeps the user's curves (Raw Accel's settings.json next to its window).
#[test]
fn a_new_unpack_keeps_raw_accels_settings() {
    let Some(zip) = release_zip() else {
        eprintln!("BU_ADDONS_RELEASE_ZIP not set: not run here");
        return;
    };
    let root = scratch("keep-settings");
    let os = FakeOs::new();
    let dir = rawaccel::unpack(&os, &zip, &root).unwrap();
    std::fs::write(dir.join("settings.json"), b"{\"curves\":1}").unwrap();
    let dir = rawaccel::unpack(&os, &zip, &root).unwrap();
    assert_eq!(std::fs::read(dir.join("settings.json")).unwrap(), b"{\"curves\":1}");
    assert!(!root.join("RawAccel.new").exists());
}

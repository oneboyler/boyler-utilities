//! Default apps against the fake: read the current defaults, list browsers with ✓, "Change" = Windows' own window.

use bu_toggles::defaults::{self, ChangeAction};
use bu_toggles::fake::FakeOs;
use bu_toggles::os::{AssocApp, RegisteredBrowser};

fn pc() -> FakeOs {
    let mut os = FakeOs {
        browsers: vec![
            RegisteredBrowser { reg_name: "Google Chrome".into(), display_name: "Google Chrome".into(), machine: true, https_progid: Some("ChromeHTML".into()) },
            RegisteredBrowser { reg_name: "Microsoft Edge".into(), display_name: "Microsoft Edge".into(), machine: true, https_progid: Some("MSEdgeHTM".into()) },
            RegisteredBrowser { reg_name: "Firefox-308046B0AF4A39CB".into(), display_name: "Firefox".into(), machine: false, https_progid: Some("FirefoxURL-308046B0AF4A39CB".into()) },
        ],
        default_browser_progid: Some("ChromeHTML".into()),
        ..FakeOs::default()
    };
    os.assoc.insert("https".into(), AssocApp { name: "Google Chrome".into(), exe: Some(r"C:\Program Files\Google\Chrome\Application\chrome.exe".into()) });
    os.assoc.insert(".png".into(), AssocApp { name: "Photos".into(), exe: None });
    os.assoc.insert(".mkv".into(), AssocApp { name: "VLC media player".into(), exe: Some(r"C:\Program Files\VideoLAN\VLC\vlc.exe".into()) });
    os
}

#[test]
fn reads_browser_installed_browsers_and_the_eight_types() {
    let os = pc();
    let d = defaults::read(&os).unwrap();
    assert_eq!(d.browser.label, "Browser");
    assert_eq!(d.browser.app.as_ref().unwrap().name, "Google Chrome");
    assert_eq!(d.browser.tip, "Pick another browser");
    let names: Vec<(&str, bool)> = d.browsers.iter().map(|b| (b.name.as_str(), b.is_current)).collect();
    assert_eq!(names, [("Firefox", false), ("Google Chrome", true), ("Microsoft Edge", false)]);
    assert_eq!(d.browsers[1].change, ChangeAction::OpenUri("ms-settings:defaultapps?registeredAppMachine=Google%20Chrome".into()));
    assert_eq!(d.browsers[0].change, ChangeAction::OpenUri("ms-settings:defaultapps?registeredAppUser=Firefox-308046B0AF4A39CB".into()));

    let labels: Vec<&str> = d.file_types.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(labels, [".png", ".jpg", ".mp4", ".mkv", ".mp3", ".pdf", ".txt", ".zip"]);
    assert_eq!(d.file_types[0].app.as_ref().unwrap().name, "Photos");
    assert!(d.file_types[1].app.is_none(), "no default -> None");
    assert_eq!(d.file_types[3].change, Some(ChangeAction::OpenWith(".mkv".into())));
    assert_eq!(d.file_types[3].tip, "Opens Windows’ “Open with” list for .mkv files");
    // nothing was opened or written by reading
    assert!(os.log.is_empty());
    assert!(os.reg.is_empty());
}

#[test]
fn change_opens_windows_own_window() {
    let mut os = pc();
    let d = defaults::read(&os).unwrap();
    defaults::perform(&mut os, &d.browsers[2].change).unwrap();
    defaults::perform(&mut os, d.file_types[0].change.as_ref().unwrap()).unwrap();
    assert_eq!(os.log, ["open_uri:ms-settings:defaultapps?registeredAppMachine=Microsoft%20Edge", "open_with:.png"]);
    assert!(os.reg.is_empty(), "never writes UserChoice (or anything)");
}

#[test]
fn toasts_are_design_words() {
    assert_eq!(defaults::browser_pick_toast("Firefox"), "Windows asks you to confirm · Firefox’s page in Settings opens");
    assert_eq!(defaults::browser_done_toast("Firefox"), "Firefox is your browser now");
    assert_eq!(defaults::file_type_toast(".mp4"), "Windows shows its “Open with” list for .mp4 · tick “Always”");
    assert_eq!(defaults::NOTE, "Windows 11 asks you to confirm each change in its own window.");
}

#[test]
fn pick_an_app_dialog_means_no_default() {
    // what the test PC answered for .mkv (examples/show, Oct 8): FRIENDLYAPPNAME "Pick an app", EXECUTABLE OpenWith.exe
    let mut os = pc();
    os.assoc.insert(".mkv".into(), AssocApp { name: "Pick an app".into(), exe: Some(r"C:\Windows\system32\OpenWith.exe".into()) });
    let d = defaults::read(&os).unwrap();
    assert!(d.file_types[3].app.is_none());
    assert_eq!(d.file_types[3].change, Some(ChangeAction::OpenWith(".mkv".into())), "Change still offered");
}

#[test]
fn no_browser_registered_is_fine() {
    let os = FakeOs::default();
    let d = defaults::read(&os).unwrap();
    assert!(d.browsers.is_empty());
    assert!(d.browser.app.is_none());
}

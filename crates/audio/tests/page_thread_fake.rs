//! The page worker on a real thread with the fake OS. Load-proof: every wait is "until it shows, at most 20 s";
//! ordering is proven through the command queue, never through sleeps.

use bu_audio::page::{AudioPage, PageSnapshot, Timing};
use bu_audio::*;
use std::time::{Duration, Instant};

fn wait(page: &AudioPage<SharedFake>, what: &str, ok: impl Fn(&PageSnapshot) -> bool) -> PageSnapshot {
    let t = Instant::now();
    loop {
        let s = page.snapshot();
        if ok(&s) {
            return s;
        }
        assert!(t.elapsed() < Duration::from_secs(20), "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn start(f: &SharedFake) -> AudioPage<SharedFake> {
    let f2 = f.clone();
    AudioPage::start(move || Ok(f2), Timing::default())
}

#[test]
fn page_fills_the_snapshot_and_runs_commands_with_undo() {
    let mut fake = FakeOs::drawing();
    fake.peaks.insert("arctis".into(), 0.42);
    fake.session_peaks.insert("discord-2".into(), 0.3);
    let f = SharedFake::new(fake);
    let page = start(&f);
    let s = wait(&page, "first read", |s| s.ready && s.output_level > 0.0);
    assert_eq!(s.outputs.len(), 4);
    assert_eq!(s.output.as_ref().map(|o| (o.0.as_str(), o.1.volume)), Some(("arctis", 0.74)));
    assert_eq!(s.input.as_ref().map(|o| o.0.as_str()), Some("mv7"));
    assert_eq!(s.apps.len(), 3);
    assert_eq!(s.output_level, 0.42);
    assert!(s.app_levels.iter().any(|(g, l)| g.ends_with("discord.exe") && *l == 0.3), "{:?}", s.app_levels);
    page.run(|svc| svc.set_device_volume("arctis", 0.5).map(Some));
    let s = wait(&page, "volume applied", |s| s.undo.len() == 1 && s.output.as_ref().is_some_and(|o| o.1.volume == 0.5));
    assert!(s.last_error.is_none());
    page.undo_last();
    wait(&page, "undo", |s| s.undo.is_empty() && s.output.as_ref().is_some_and(|o| o.1.volume == 0.74));
    page.run(|svc| svc.set_device_on(Flow::Output, "nope", false).map(Some));
    let s = wait(&page, "error shown", |s| s.last_error.is_some());
    assert_eq!(s.last_error, Some(AudioError::NotFound("nope".into())));
}

#[test]
fn levels_only_while_asked_and_nothing_after_drop() {
    let f = SharedFake::new(FakeOs::drawing());
    let page = start(&f);
    wait(&page, "levels running", |s| s.level_reads >= 3);
    page.set_levels(false);
    // commands run in order: once this one shows, levels are off
    page.run(|svc| svc.set_device_mute("mv7", true).map(Some));
    let s = wait(&page, "marker", |s| s.undo.len() == 1);
    let reads = s.level_reads;
    page.run(|svc| svc.set_device_mute("mv7", false).map(Some));
    let s = wait(&page, "second marker", |s| s.undo.len() == 2);
    assert_eq!(s.level_reads, reads, "no level reads while not asked");
    drop(page); // joins the worker
    let n = f.with(|x| x.reads);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(f.with(|x| x.reads), n, "nothing reads Windows after the page is dropped");
}

#[test]
fn page_without_core_audio_reports_and_ends() {
    let page: AudioPage<SharedFake> = AudioPage::start(|| Err(AudioError::Unavailable("no audio service".into())), Timing::default());
    let s = wait(&page, "error", |s| s.last_error.is_some());
    assert!(!s.ready);
}

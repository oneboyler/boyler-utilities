use std::sync::atomic::AtomicUsize;
use std::time::{Duration, Instant};

use super::*;

fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let t = Instant::now();
    while !ok() {
        assert!(t.elapsed() < Duration::from_secs(10), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn counting_runner() -> (JobRunner, Arc<AtomicUsize>) {
    let wakes = Arc::new(AtomicUsize::new(0));
    let w = wakes.clone();
    (JobRunner::new(move || {
        w.fetch_add(1, Ordering::SeqCst);
    }), wakes)
}

/// A page as the page API sees it: `open` gets the runner but no token; `on_click` gets the dispatch's token.
struct ScanPage {
    job: Option<JobId>,
}

impl ScanPage {
    fn open(runner: &mut JobRunner) -> ScanPage {
        // the open path has no `Pressed`, and can't make one: `Pressed`'s field is private to jobs and
        // `Clicks::take` was already used by the UI — so nothing here can call `runner.start`.
        let _ = runner.running();
        ScanPage { job: None }
    }

    fn on_click(&mut self, runner: &mut JobRunner, pressed: Pressed<'_>) {
        self.job = runner
            .start(pressed, "storage.scan", |ctx| {
                for i in 0..4 {
                    ctx.check()?;
                    ctx.progress(i as f32 / 4.0);
                }
                Ok("4 folders".into())
            })
            .ok();
    }
}

#[test]
fn never_starts_on_open_only_on_a_click() {
    let (mut runner, _) = counting_runner();
    let mut clicks = Clicks::for_test();
    let mut page = ScanPage::open(&mut runner);
    assert_eq!(runner.running(), 0);
    assert!(runner.view_key("storage.scan").is_none());
    page.on_click(&mut runner, clicks.press());
    let id = page.job.unwrap();
    wait_for("the scan to end", || runner.view(id).unwrap().end.is_some());
    let v = runner.view(id).unwrap();
    assert_eq!(v.end, Some(End::Done("4 folders".into())));
    assert_eq!(v.progress, Progress::Part(1.0));
}

#[test]
fn clicks_can_be_taken_once_per_process() {
    // the UI takes it at start-up; a page trying later gets nothing
    let first = Clicks::take();
    let second = Clicks::take();
    assert!(first.is_some());
    assert!(second.is_none());
}

#[test]
fn runs_off_the_calling_thread_and_progress_reaches_the_ui() {
    let (mut runner, wakes) = counting_runner();
    let mut clicks = Clicks::for_test();
    let gate = Arc::new(AtomicBool::new(false));
    let g = gate.clone();
    let ui = std::thread::current().id();
    let id = runner
        .start(clicks.press(), "net.ping", move |ctx| {
            if std::thread::current().id() == ui {
                return Err("ran on the UI thread".into());
            }
            ctx.status("Pinging Frankfurt");
            ctx.progress(0.5);
            while !g.load(Ordering::SeqCst) {
                ctx.check()?;
                std::thread::sleep(Duration::from_millis(1));
            }
            ctx.busy();
            Ok(String::new())
        })
        .unwrap();
    wait_for("progress 0.5", || runner.view(id).unwrap().progress == Progress::Part(0.5));
    let v = runner.view(id).unwrap();
    assert_eq!(v.status, "Pinging Frankfurt");
    assert_eq!(v.end, None);
    assert_eq!(runner.running(), 1);
    assert!(wakes.load(Ordering::SeqCst) >= 2, "the UI was woken");
    gate.store(true, Ordering::SeqCst);
    wait_for("the end", || runner.view(id).unwrap().end.is_some());
    assert_eq!(runner.view(id).unwrap().end, Some(End::Done(String::new())));
    assert_eq!(runner.running(), 0);
}

#[test]
fn stop_works() {
    let (mut runner, _) = counting_runner();
    let mut clicks = Clicks::for_test();
    let id = runner
        .start(clicks.press(), "speedtest", |ctx| loop {
            ctx.check()?;
            ctx.busy();
            std::thread::sleep(Duration::from_millis(1));
        })
        .unwrap();
    wait_for("running", || runner.view(id).unwrap().progress == Progress::Busy);
    runner.stop(id);
    assert!(runner.view(id).unwrap().stopping);
    wait_for("stopped", || runner.view(id).unwrap().end.is_some());
    assert_eq!(runner.view(id).unwrap().end, Some(End::Stopped));
    runner.forget(id);
    assert!(runner.view(id).is_none());
}

#[test]
fn a_second_press_finds_the_running_job() {
    let (mut runner, _) = counting_runner();
    let mut clicks = Clicks::for_test();
    let id = runner
        .start(clicks.press(), "scan", |ctx| loop {
            ctx.check()?;
            std::thread::sleep(Duration::from_millis(1));
        })
        .unwrap();
    assert_eq!(runner.start(clicks.press(), "scan", |_| Ok(String::new())), Err(StartError::AlreadyRunning(id)));
    // another key is fine
    let other = runner.start(clicks.press(), "other", |_| Ok("x".into())).unwrap();
    wait_for("other ends", || runner.view(other).unwrap().end.is_some());
    runner.stop(id);
    wait_for("scan stops", || runner.view(id).unwrap().end.is_some());
    // ended: the key can start again
    assert!(runner.start(clicks.press(), "scan", |_| Ok(String::new())).is_ok());
}

#[test]
fn failure_and_panic_end_as_failed() {
    let (mut runner, _) = counting_runner();
    let mut clicks = Clicks::for_test();
    let a = runner.start(clicks.press(), "a", |_| Err("No network".into())).unwrap();
    let b = runner.start(clicks.press(), "b", |_| -> Result<String, JobError> { panic!("boom") }).unwrap();
    wait_for("both end", || runner.running() == 0);
    assert_eq!(runner.view(a).unwrap().end, Some(End::Failed("No network".into())));
    assert_eq!(runner.view(b).unwrap().end, Some(End::Failed("The job crashed".into())));
}

#[test]
fn no_thread_left_after_drop() {
    let (mut runner, _) = counting_runner();
    let mut clicks = Clicks::for_test();
    let alive = Arc::new(());
    let exited = Arc::new(AtomicBool::new(false));
    for key in ["one", "two", "three"] {
        let a = alive.clone();
        let e = exited.clone();
        runner
            .start(clicks.press(), key, move |ctx| {
                let _hold = a;
                while !ctx.stopped() {
                    std::thread::sleep(Duration::from_millis(1));
                }
                e.store(true, Ordering::SeqCst);
                Err(JobError::Stopped)
            })
            .unwrap();
    }
    assert_eq!(runner.running(), 3);
    drop(runner);
    // every closure (and so every thread's work) is gone: only our own handle is left
    assert_eq!(Arc::strong_count(&alive), 1);
    assert!(exited.load(Ordering::SeqCst));
}

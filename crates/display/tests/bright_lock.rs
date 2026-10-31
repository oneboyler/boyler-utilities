//! Order 124: "Lock brightness" - the wake path against the FAKE layer only (no real monitor is read or changed).

use bu_display::brightlock::{LockWatch, Restore, WakeEvent};
use bu_display::fake::{FakeCall, FakeDisplayOs};
use bu_display::{DisplayService, MonitorId, Vcp, VcpValue};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

type Svc = Arc<Mutex<DisplayService<FakeDisplayOs>>>;

fn dell() -> MonitorId {
    MonitorId("fake-dell".into())
}

/// The drawing's sample: DELL answers DDC/CI at brightness 70 of 100, LG doesn't answer.
fn svc() -> Svc {
    Arc::new(Mutex::new(DisplayService::new(FakeDisplayOs::drawing_sample())))
}

fn set_brightness(s: &Svc, v: u32) {
    let mut g = s.lock().unwrap();
    let m = g.os_mut().monitors.iter_mut().find(|m| m.info.id == dell()).unwrap();
    m.ddc.as_mut().unwrap().insert(Vcp::Brightness, VcpValue { current: v, max: 100 });
}

fn brightness_writes(s: &Svc) -> Vec<u32> {
    s.lock().unwrap().os().calls.iter().filter_map(|c| if let FakeCall::DdcSet(_, Vcp::Brightness, v) = c { Some(*v) } else { None }).collect()
}

/// A watch on `svc` with DELL locked at `pct`, a short retry list, and a channel that gets every attempt's results.
fn watch(s: &Svc, locks: Arc<Mutex<Vec<(MonitorId, u8)>>>) -> (LockWatch, mpsc::Receiver<Vec<(MonitorId, Restore)>>) {
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let w = LockWatch::start(
        s.clone(),
        move || locks.lock().unwrap().clone(),
        Arc::new(AtomicU64::new(0)),
        vec![Duration::from_millis(5), Duration::from_millis(15), Duration::from_millis(30)],
        move |r: &[(MonitorId, Restore)]| {
            let _ = tx.lock().unwrap().send(r.to_vec());
        },
    );
    (w, rx)
}

/// Waits for `n` attempts (2 s at most each).
fn attempts(rx: &mpsc::Receiver<Vec<(MonitorId, Restore)>>, n: usize) -> Vec<Vec<(MonitorId, Restore)>> {
    (0..n).map(|_| rx.recv_timeout(Duration::from_secs(2)).expect("an attempt")).collect()
}

#[test]
fn each_wake_event_puts_the_locked_value_back_with_one_write() {
    for ev in [WakeEvent::DisplayOn, WakeEvent::Resume, WakeEvent::Unlock, WakeEvent::DisplayChange, WakeEvent::MonitorArrived] {
        let s = svc();
        let (w, rx) = watch(&s, Arc::new(Mutex::new(vec![(dell(), 70)])));
        // the monitor came back on at 0 %
        set_brightness(&s, 0);
        w.wake(ev);
        let all = attempts(&rx, 3);
        assert_eq!(brightness_writes(&s), [70], "{ev:?}: one write of the locked value");
        assert!(matches!(all[0][0].1, Restore::Written { before: VcpValue { current: 0, .. }, after: VcpValue { current: 70, .. } }), "{ev:?}: {all:?}");
        assert!(all[1..].iter().all(|a| matches!(a[0].1, Restore::Same)), "{ev:?}: later reads find it there: {all:?}");
    }
}

#[test]
fn the_same_value_is_never_written() {
    let s = svc();
    let (w, rx) = watch(&s, Arc::new(Mutex::new(vec![(dell(), 70)])));
    w.wake(WakeEvent::DisplayOn);
    let all = attempts(&rx, 3);
    assert!(brightness_writes(&s).is_empty());
    assert!(all.iter().all(|a| matches!(a[0].1, Restore::Same)), "{all:?}");
}

#[test]
fn nothing_is_written_without_an_event() {
    let s = svc();
    let (_w, rx) = watch(&s, Arc::new(Mutex::new(vec![(dell(), 70)])));
    // the user (or the monitor's own buttons) set another value: no event, so the lock does not fight it
    set_brightness(&s, 20);
    assert!(rx.recv_timeout(Duration::from_millis(200)).is_err(), "no attempt without an event");
    assert!(brightness_writes(&s).is_empty());
}

#[test]
fn a_monitor_that_answers_late_is_retried_until_it_answers() {
    let s = svc();
    {
        let mut g = s.lock().unwrap();
        // just woken: the first two DDC/CI calls get no answer
        g.os_mut().ddc_late.insert(dell(), 2);
    }
    set_brightness(&s, 0);
    let (w, rx) = watch(&s, Arc::new(Mutex::new(vec![(dell(), 70)])));
    w.wake(WakeEvent::Resume);
    let all = attempts(&rx, 3);
    assert!(matches!(all[0][0].1, Restore::NoAnswer), "{all:?}");
    assert!(matches!(all[1][0].1, Restore::NoAnswer), "{all:?}");
    assert!(matches!(all[2][0].1, Restore::Written { .. }), "{all:?}");
    assert_eq!(brightness_writes(&s), [70]);
}

#[test]
fn the_newest_locked_value_is_used_and_unlocked_monitors_are_left_alone() {
    let s = svc();
    let locks = Arc::new(Mutex::new(vec![(dell(), 70)]));
    let (w, rx) = watch(&s, locks.clone());
    // the slider moved while locked: the lock follows it
    *locks.lock().unwrap() = vec![(dell(), 40)];
    set_brightness(&s, 0);
    w.wake(WakeEvent::Unlock);
    attempts(&rx, 3);
    assert_eq!(brightness_writes(&s), [40]);
    // unlocked: an event changes nothing
    locks.lock().unwrap().clear();
    set_brightness(&s, 0);
    w.wake(WakeEvent::Unlock);
    let all = attempts(&rx, 3);
    assert!(all.iter().all(|a| a.is_empty()), "{all:?}");
    assert_eq!(brightness_writes(&s), [40]);
}

#[test]
fn a_new_event_during_the_retries_starts_them_again() {
    let s = svc();
    let (w, rx) = watch(&s, Arc::new(Mutex::new(vec![(dell(), 70)])));
    w.wake(WakeEvent::DisplayOn);
    w.wake(WakeEvent::Resume);
    w.wake(WakeEvent::Unlock);
    // events in a burst are one round of retries (3 attempts), not three
    attempts(&rx, 3);
    assert!(rx.recv_timeout(Duration::from_millis(150)).is_err(), "one round for a burst");
}

#[test]
fn dropping_the_watch_ends_its_thread() {
    let s = svc();
    let (w, rx) = watch(&s, Arc::new(Mutex::new(vec![(dell(), 70)])));
    drop(w);
    // the callback (and its sender) went with the thread
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Err(mpsc::RecvTimeoutError::Disconnected));
}

/// End review (Order 124): the user moved the slider between a try's read of the locked value and its DDC/CI read - the
/// try must not put the old locked value back over the user's new one.
#[test]
fn a_try_never_writes_over_a_slider_move_made_meanwhile() {
    let s = svc();
    set_brightness(&s, 0);
    let moved = Arc::new(AtomicU64::new(0));
    let m2 = moved.clone();
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let w = LockWatch::start(
        s.clone(),
        move || {
            // the slider moves right after the try read the (old) locked value
            m2.fetch_add(1, Ordering::SeqCst);
            vec![(dell(), 70)]
        },
        moved.clone(),
        vec![Duration::from_millis(5)],
        move |r: &[(MonitorId, Restore)]| {
            let _ = tx.lock().unwrap().send(r.to_vec());
        },
    );
    w.wake(WakeEvent::Resume);
    let all = attempts(&rx, 1);
    assert!(matches!(all[0][0].1, Restore::Moved), "{all:?}");
    assert!(brightness_writes(&s).is_empty());
}

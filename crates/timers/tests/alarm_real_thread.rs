//! The alarm thread with the real clock. Load-proof: it only checks that a ring never comes BEFORE its deadline, that
//! re-aiming / disarming works, and (with a generous 20 s upper bound) that it does ring.

use bu_timers::alarm::Alarm;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

#[test]
fn rings_once_never_early() {
    let (tx, rx) = channel();
    let a = Alarm::start(move || {
        let _ = tx.send(Instant::now());
    })
    .expect("alarm thread");
    let at = Instant::now() + Duration::from_millis(150);
    a.aim(Some(at));
    let rang = rx.recv_timeout(Duration::from_secs(20)).expect("the alarm rang");
    assert!(rang >= at, "never before the deadline");
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "rings once");
}

#[test]
fn disarm_and_re_aim() {
    let (tx, rx) = channel();
    let a = Alarm::start(move || {
        let _ = tx.send(Instant::now());
    })
    .expect("alarm thread");
    // aimed far ahead, so a stalled test thread can never let it ring before the disarm
    a.aim(Some(Instant::now() + Duration::from_secs(3600)));
    a.aim(None);
    assert!(rx.recv_timeout(Duration::from_millis(400)).is_err(), "disarmed: no ring");
    let later = Instant::now() + Duration::from_millis(200);
    a.aim(Some(Instant::now() + Duration::from_secs(3600)));
    a.aim(Some(later));
    let rang = rx.recv_timeout(Duration::from_secs(20)).expect("re-aimed alarm rang");
    assert!(rang >= later);
    drop(a); // the thread ends (join) — the test would hang here if it didn't
}

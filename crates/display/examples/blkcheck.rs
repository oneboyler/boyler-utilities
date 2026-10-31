//! REAL-PC brightness lock check (Order 124). Per monitor that answers DDC/CI: reads its brightness (exact raw value),
//! sets a value 10 % away through the app's path (DisplayService::set_ddc_percent) and locks it, moves the monitor 5 %
//! off that value (as if it woke at another value), fires a wake event through the app's lock thread (LockWatch, the
//! real ~10 s retry list) and reads back - then puts the EXACT starting raw value back and checks it. Also starts the
//! hidden wake-event window once (never shown) to prove Windows takes its registrations. Only brightness changes, for
//! ~12 s.
//!
//!   cargo run -p bu-display --example display-blkcheck -- <ddc-guard-scratch-folder>
//!   cargo run -p bu-display --example display-blkcheck -- <ddc-guard-scratch-folder> --idle 60   (idle cost only, changes nothing)

use bu_display::brightlock::{LockWatch, Restore, WakeEvent, RETRY_AT};
use bu_display::win::power::PowerEvents;
use bu_display::win::WinDisplayOs;
use bu_display::{DisplayOs, DisplayService, MonitorId, Vcp};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

type Svc = Arc<Mutex<DisplayService<WinDisplayOs>>>;

/// Puts every monitor's starting brightness back, also if the check panics half-way.
struct PutBack {
    svc: Svc,
    orig: Vec<(MonitorId, u32)>,
}

impl Drop for PutBack {
    fn drop(&mut self) {
        let mut s = self.svc.lock().unwrap_or_else(|e| e.into_inner());
        for (id, v) in &self.orig {
            let set = s.os_mut().ddc_set(id, Vcp::Brightness, *v);
            std::thread::sleep(Duration::from_millis(100));
            let back = s.os_mut().ddc_get(id, Vcp::Brightness).map(|x| x.current);
            println!("put back {id}: raw {v} -> set {set:?}, read back {back:?} {}", if back == Ok(*v) { "EXACT" } else { "MISMATCH" });
        }
    }
}

/// This process's CPU time (user + kernel) so far.
fn cpu() -> Duration {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let (mut a, mut b, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    let _ = unsafe { GetProcessTimes(GetCurrentProcess(), &mut a, &mut b, &mut k, &mut u) };
    let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    Duration::from_nanos((t(k) + t(u)) * 100)
}

/// `--idle <s>`: the lock's two parts idle with a lock on (read-only: no event = no DDC/CI call at all) - CPU used and
/// how often the window thread woke.
fn idle(dir: PathBuf, secs: u64) {
    let svc: Svc = Arc::new(Mutex::new(DisplayService::new(WinDisplayOs::read_only(Some(dir)))));
    let ev = PowerEvents::start(Arc::new(|e| println!("  (window event: {e:?})"))).expect("power event window");
    let tries = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let t2 = tries.clone();
    let w = LockWatch::start(svc, || vec![(MonitorId("idle-check".into()), 50)], Arc::new(std::sync::atomic::AtomicU64::new(0)), RETRY_AT.to_vec(), move |_: &[(MonitorId, Restore)]| {
        t2.fetch_add(1, Ordering::Relaxed);
    });
    std::thread::sleep(Duration::from_millis(500));
    let (c0, w0, t0) = (cpu(), ev.wakes.load(Ordering::Relaxed), Instant::now());
    std::thread::sleep(Duration::from_secs(secs));
    let (c1, w1, el) = (cpu(), ev.wakes.load(Ordering::Relaxed), t0.elapsed());
    println!(
        "IDLE {:.0} s: CPU {:.3} ms ({:.4} % of one core), window wakes {}, lock tries {}",
        el.as_secs_f64(),
        (c1 - c0).as_secs_f64() * 1000.0,
        (c1 - c0).as_secs_f64() / el.as_secs_f64() * 100.0,
        w1 - w0,
        tries.load(Ordering::Relaxed)
    );
    drop(w);
}

fn main() {
    let dir = std::env::args().nth(1).map(PathBuf::from).expect("a scratch folder for the DDC crash guard");
    if std::env::args().nth(2).as_deref() == Some("--idle") {
        let secs = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(60);
        return idle(dir, secs);
    }
    let svc: Svc = Arc::new(Mutex::new(DisplayService::new(WinDisplayOs::new(Some(dir)))));

    // the wake-event window: registered, then closed again
    let ev = PowerEvents::start(Arc::new(|e| println!("  (window event: {e:?})"))).expect("power event window");
    std::thread::sleep(Duration::from_millis(300));
    println!("wake-event window started; messages so far: {}", ev.wakes.load(Ordering::Relaxed));
    drop(ev);
    println!("wake-event window stopped");

    let mons = svc.lock().unwrap().monitors().expect("monitors");
    let mut orig = Vec::new();
    for m in &mons {
        match svc.lock().unwrap().os_mut().ddc_get(&m.id, Vcp::Brightness) {
            Ok(v) => {
                println!("{} · {}: brightness raw {} of {} ({} %)", m.number, m.name, v.current, v.max, v.percent());
                orig.push((m.id.clone(), v.current));
            }
            Err(e) => println!("{} · {}: no DDC/CI answer ({e}) - skipped", m.number, m.name),
        }
    }
    let put_back = PutBack { svc: svc.clone(), orig: orig.clone() };

    // set a value, lock it, then the monitor "wakes" 5 % off it
    let mut locks = Vec::new();
    for (id, raw) in &orig {
        let max = svc.lock().unwrap().os_mut().ddc_get(id, Vcp::Brightness).map(|v| v.max).unwrap_or(100);
        let pct = (*raw as f64 * 100.0 / max as f64).round() as u8;
        let locked = if pct <= 50 { pct + 10 } else { pct - 10 };
        let off = if locked <= 50 { locked + 5 } else { locked - 5 };
        // DDC/CI wants a pause between commands (a read right after a write got no answer on the first run)
        std::thread::sleep(Duration::from_millis(200));
        svc.lock().unwrap().set_ddc_percent(id, Vcp::Brightness, locked).expect("set the value to lock");
        std::thread::sleep(Duration::from_millis(300));
        locks.push((id.clone(), locked));
        svc.lock().unwrap().set_ddc_percent(id, Vcp::Brightness, off).expect("move it off the locked value");
        println!("{id}: locked at {locked} %, monitor now at {off} %");
    }

    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let l2 = locks.clone();
    let w = LockWatch::start(svc.clone(), move || l2.clone(), Arc::new(std::sync::atomic::AtomicU64::new(0)), RETRY_AT.to_vec(), move |r: &[(MonitorId, Restore)]| {
        let _ = tx.lock().unwrap().send(r.to_vec());
    });
    let t0 = Instant::now();
    w.wake(WakeEvent::Resume);
    for i in 0..RETRY_AT.len() {
        let r = rx.recv_timeout(Duration::from_secs(15)).expect("a try");
        println!("try {} at {:.1} s: {:?}", i + 1, t0.elapsed().as_secs_f64(), r);
    }
    drop(w);

    let mut ok = true;
    for (id, pct) in &locks {
        std::thread::sleep(Duration::from_millis(200));
        let v = svc.lock().unwrap().os_mut().ddc_get(id, Vcp::Brightness).expect("read back");
        let want = bu_display::picture::ddc_value_for_percent(v.max, *pct);
        println!("{id}: read back raw {} - locked value raw {want}: {}", v.current, if v.current == want { "OK" } else { "WRONG" });
        ok &= v.current == want;
    }
    drop(put_back);
    println!("{}", if ok { "BLKCHECK PASS" } else { "BLKCHECK FAIL" });
}

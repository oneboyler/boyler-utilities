//! Order 092's proof on the real PC: the REAL engine and stream, MUTED (nothing is heard), fed to the listening tracker the way
//! the app's glue feeds it: Play -> begin, Stop -> end, and the engine's own "the player left" call -> end. The ledger goes to a
//! scratch folder (the first argument), never to the real settings folder.
//!
//! `cargo run -p bu-noise --example noise-proof-listen -- <scratch folder>`

#[cfg(windows)]
fn main() {
    use bu_noise::listen::{local_now, Tracker};
    use bu_noise::{Kind, Noise};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("a scratch folder"));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("noise-listening.txt");
    let _ = std::fs::remove_file(&file);

    let t = Arc::new(Tracker::new());
    t.set_file(file.clone());
    let n = Arc::new(Noise::new());
    n.set_test_mute(true);
    let ends = Arc::new(AtomicU32::new(0));
    {
        let (t, n2, ends) = (t.clone(), n.clone(), ends.clone());
        n.set_on_change(Arc::new(move || {
            let s = n2.status();
            if t.is_open() && !s.playing {
                t.end(local_now(), s.played);
                ends.fetch_add(1, Ordering::SeqCst);
            }
        }));
    }

    // 1. Play 4 s, Stop by the button: the stretch ends at Stop
    t.begin(local_now(), n.status().played);
    n.play(Kind::Brown, 5, None);
    std::thread::sleep(Duration::from_secs(4));
    let mid = t.totals(local_now(), n.status().played).today;
    t.end(local_now(), n.status().played);
    n.stop();
    std::thread::sleep(Duration::from_millis(2500));
    let s = n.status();
    let after = t.totals(local_now(), s.played).today;
    println!("running figure after 4 s: {mid} s; after Stop: {after} s; player wrote {:.1} s in all; playing now: {}", s.played, s.playing);
    println!("the player-left call counted a second time: {} (must be 0: the Stop already closed the stretch)", ends.load(Ordering::SeqCst));

    // 2. Play 3 s, then the player is shut down without Stop (the app quits / the player ends by itself): the call ends it
    t.begin(local_now(), n.status().played);
    n.play(Kind::Pink, 5, None);
    std::thread::sleep(Duration::from_secs(3));
    n.stop(); // fade-out; no end() by hand: only the engine's call when its thread leaves
    std::thread::sleep(Duration::from_millis(2500));
    let all = t.totals(local_now(), n.status().played).all;
    println!("after a stretch ended only by the player-left call: all = {all} s (expect about 7), calls: {}", ends.load(Ordering::SeqCst));
    println!("file: {}", std::fs::read_to_string(&file).unwrap_or_default().replace('\n', " | "));
    let fresh = Tracker::new();
    fresh.set_file(file);
    println!("a new tracker reads back all = {} s", fresh.totals(local_now(), 0.0).all);
}

#[cfg(not(windows))]
fn main() {}

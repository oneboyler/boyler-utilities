//! Order 058's proof on the real PC: the key-sound path press -> samples in the output buffer, with the REAL output stream.
//! MUTED: the engine's own audio session is muted (`set_test_mute`), so the stream runs on the real default device and
//! nothing is heard. No key is pressed or read and NO key listening is registered (`enable_without_keys`): the "press" is the
//! engine's own preview call (the same mixer / stream path as a key press; only the raw-input read in front of it is missing,
//! which is a packet read on a thread that is already awake).
//!
//! `cargo run -p bu-keysound --example keysound-latency [-- --cold]` -> prints the stream (rate, period, Windows' own latency),
//! and the time from a press to its first samples being in the buffer (measured, µs) for: the very first press (the stream is
//! opened), 20 presses while it plays, the first press after it was parked (stopped, 3 s of silence: it only has to be filled
//! and started) and, with `--cold`, the first press after the parked stream was closed (60 s of silence).

#[cfg(windows)]
fn main() {
    use bu_keysound::{KeySounds, Kind, Pack, PackId, Settings};
    use std::time::Duration;

    let cold = std::env::args().any(|a| a == "--cold");
    let ks = KeySounds::new();
    ks.set_test_mute(true);
    let mut s = Settings::default();
    s.pack = Pack::Builtin(PackId::Linear);
    s.off_in_game = false;
    ks.enable_without_keys(s).expect("enable");
    let pack = Pack::Builtin(PackId::Linear);
    let ms = |us: u64| us as f32 / 1000.0;

    println!("first press (opens the stream):");
    ks.preview(&pack, Kind::Down);
    std::thread::sleep(Duration::from_millis(400));
    let st = ks.status();
    println!(
        "  stream: {} Hz, {} stream, period {:.2} ms, Windows' own latency {:.2} ms, opened {}x, press -> buffer {:.2} ms, error {:?}",
        st.rate,
        if st.low_latency { "low-latency" } else { "standard" },
        st.period_ms,
        st.stream_latency_ms,
        st.opens,
        ms(st.last_submit_us),
        st.error
    );
    let (first, lat) = (st.last_submit_us, st.stream_latency_ms);

    println!("20 presses while the stream plays (80 ms apart):");
    let mut v = Vec::new();
    for _ in 0..20 {
        ks.preview(&pack, Kind::Down);
        std::thread::sleep(Duration::from_millis(80));
        v.push(ks.status().last_submit_us);
    }
    v.sort_unstable();
    let median = v[v.len() / 2];
    println!("  press -> buffer: min {:.2} ms, median {:.2} ms, max {:.2} ms", ms(v[0]), ms(median), ms(v[v.len() - 1]));

    println!("waiting for the stream to park ({} ms silent) ...", bu_keysound::engine::IDLE_STOP.as_millis());
    std::thread::sleep(bu_keysound::engine::IDLE_STOP + Duration::from_millis(800));
    println!("  playing now: {} (parked = stopped but initialised: the render thread sleeps, 0 CPU)", ks.status().stream_open);
    ks.preview(&pack, Kind::Down);
    std::thread::sleep(Duration::from_millis(400));
    let st = ks.status();
    let parked = st.last_submit_us;
    println!("press after parking (fill + start): press -> buffer {:.2} ms, opened {}x", ms(parked), st.opens);
    let mut cold_us = None;
    if cold {
        println!("waiting {} s for the parked stream to close ...", bu_keysound::engine::PARK_FOR.as_secs() + 6);
        std::thread::sleep(bu_keysound::engine::PARK_FOR + Duration::from_secs(3 + 3));
        ks.preview(&pack, Kind::Down);
        std::thread::sleep(Duration::from_millis(600));
        let st = ks.status();
        cold_us = Some(st.last_submit_us);
        println!("press after the stream was closed (cold open): press -> buffer {:.2} ms, opened {}x", ms(st.last_submit_us), st.opens);
    }
    println!("press -> ear = press -> buffer + Windows' stream latency ({lat:.1} ms):");
    println!("  cold (first press, stream opened): {:.1} ms", ms(first) + lat);
    println!("  stream playing (median of 20):     {:.1} ms", ms(median) + lat);
    println!("  after parking (fill + start):      {:.1} ms", ms(parked) + lat);
    if let Some(c) = cold_us {
        println!("  after closing (cold open again):   {:.1} ms", ms(c) + lat);
    }
    ks.disable();
}

#[cfg(not(windows))]
fn main() {}

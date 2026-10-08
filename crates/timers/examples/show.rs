//! `cargo run -p bu-timers --example timers-show` — read-only proof on the real PC: the real clock (resolution,
//! monotonic), how typed times read, and the chime as a buffer. Plays NOTHING, changes nothing.

use bu_timers::parse::{format_countdown, parse_time, BareUnit};
use bu_timers::sound::{chime_samples, chime_wav, SAMPLE_RATE};
use bu_timers::stopwatch::Stopwatch;
use bu_timers::{Clock, MonoClock};
use std::time::Duration;

fn main() {
    let c = MonoClock::new();
    // the smallest step the real clock shows (Instant = QueryPerformanceCounter on Windows)
    let mut step = Duration::MAX;
    let mut last = c.now();
    let mut backwards = 0;
    for _ in 0..200_000 {
        let t = c.now();
        if t < last {
            backwards += 1;
        } else if t > last {
            step = step.min(t - last);
        }
        last = t;
    }
    println!("clock: std::time::Instant (QueryPerformanceCounter); smallest step seen {:?}; went backwards {} times in 200000 reads", step, backwards);
    let mut sw = Stopwatch::new(c);
    sw.start_stop();
    std::thread::sleep(Duration::from_millis(250));
    sw.start_stop();
    println!("stopwatch after a 250 ms sleep: {} ({:?})", sw.text(), sw.elapsed());
    for t in ["5", "1:30", "90s", "1h 20m", "2m", "45"] {
        println!(
            "typed {:>7}: countdown {:>8}  bar {:>8}",
            format!("{t:?}"),
            parse_time(t, BareUnit::Minutes).map(|s| format_countdown(Duration::from_secs(s))).unwrap_or("-".into()),
            parse_time(t, BareUnit::Seconds).map(|s| format_countdown(Duration::from_secs(s))).unwrap_or("-".into())
        );
    }
    let s = chime_samples();
    let peak = s.iter().fold(0f32, |m, x| m.max(x.abs()));
    println!(
        "chime: {} samples = {:.3} s at {} Hz, peak {:.3} (of 1.0), WAV {} bytes — not played",
        s.len(),
        s.len() as f64 / SAMPLE_RATE as f64,
        SAMPLE_RATE,
        peak,
        chime_wav().len()
    );
    // the World clock from Windows' own time zone rules (read-only)
    #[cfg(windows)]
    {
        let z = bu_timers::zones::RealZones;
        let cities: Vec<&str> = bu_timers::zones::PLACES.iter().map(|p| p.city).collect();
        let w = bu_timers::zones::world_view(&z, &cities);
        println!("world clock: {} {} ({})", w.home_name, w.home_time, w.home_line);
        for (c, t, l) in &w.places {
            println!("  {c:<12} {t}  {l}");
        }
    }
}

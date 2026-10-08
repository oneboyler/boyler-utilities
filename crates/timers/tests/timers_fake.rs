//! Every Timers row against the fake clock + fake sound: no test depends on real time, PC load or plays a sound.

use bu_timers::bars::{BarSize, Horiz, ShowOn, Spot, TimerBars, Vert, COLOURS, QUICK_SPOTS};
use bu_timers::countdown::{CdButton, Countdown};
use bu_timers::parse::{format_countdown, format_stopwatch, parse_time, BareUnit, MAX_SECS};
use bu_timers::sound::{chime_samples, chime_wav, SAMPLE_RATE};
use bu_timers::stopwatch::{Stopwatch, SwButton};
use bu_timers::{FakeClock, FakeSound, TimerError};
use std::time::Duration;

const S: fn(u64) -> Duration = Duration::from_secs;

// ------------------------------------------------------------------------------------------------ parsing / formats
#[test]
fn countdown_typing_the_drawing_examples() {
    let m = BareUnit::Minutes;
    assert_eq!(parse_time("5", m), Some(300));
    assert_eq!(parse_time("1:30", m), Some(90));
    assert_eq!(parse_time("90s", m), Some(90));
    assert_eq!(parse_time("1h 20m", m), Some(4800));
    assert_eq!(parse_time("1:02:03", m), Some(3723));
    assert_eq!(parse_time(" 2 min ", m), Some(120));
    assert_eq!(parse_time("1.5h", m), Some(5400));
    assert_eq!(parse_time("1,5m", m), Some(90));
    assert_eq!(parse_time("1h20m30s", m), Some(4830));
    assert_eq!(parse_time("10M", m), Some(600));
}

#[test]
fn bar_typing_bare_number_is_seconds() {
    let s = BareUnit::Seconds;
    assert_eq!(parse_time("45", s), Some(45));
    assert_eq!(parse_time("1:30", s), Some(90));
    assert_eq!(parse_time("2m", s), Some(120));
}

#[test]
fn not_a_time() {
    for bad in ["", "   ", "abc", "0", "0:00", "1:2:3:4", "1:300", "5x", "-5", "1::2", ":30", "0.2s"] {
        assert_eq!(parse_time(bad, BareUnit::Minutes), None, "{bad:?}");
    }
}

#[test]
fn capped_at_99_59_59() {
    assert_eq!(parse_time("1000h", BareUnit::Minutes), Some(MAX_SECS));
    assert_eq!(parse_time("99999999999999999999:00", BareUnit::Minutes), None); // too big to be a number at all
}

#[test]
fn formats() {
    assert_eq!(format_countdown(S(300)), "5:00");
    assert_eq!(format_countdown(Duration::from_millis(299_001)), "5:00"); // partial seconds round up
    assert_eq!(format_countdown(Duration::from_millis(1)), "0:01");
    assert_eq!(format_countdown(Duration::ZERO), "0:00");
    assert_eq!(format_countdown(S(3723)), "1:02:03");
    assert_eq!(format_stopwatch(Duration::from_millis(7_429)), "0:07.42"); // hundredths cut
    assert_eq!(format_stopwatch(Duration::from_millis(3_723_456)), "1:02:03.45");
}

// ------------------------------------------------------------------------------------------------ stopwatch
#[test]
fn stopwatch_start_stop_resume_reset() {
    let c = FakeClock::new();
    let mut sw = Stopwatch::new(c.clone());
    assert_eq!(sw.button(), SwButton::Start);
    assert!(!sw.lap_enabled() && !sw.reset_enabled());
    sw.start_stop();
    c.advance_ms(1_500);
    assert_eq!(sw.button(), SwButton::Stop);
    assert_eq!(sw.text(), "0:01.50");
    sw.start_stop();
    c.advance_ms(10_000); // stopped: time doesn't move
    assert_eq!(sw.elapsed(), Duration::from_millis(1_500));
    assert_eq!(sw.button(), SwButton::Resume);
    assert!(sw.reset_enabled() && !sw.lap_enabled());
    sw.start_stop();
    c.advance_ms(500);
    assert_eq!(sw.elapsed(), S(2));
    sw.reset();
    assert_eq!(sw.elapsed(), Duration::ZERO);
    assert!(!sw.is_running() && sw.laps().is_empty());
    assert_eq!(sw.button(), SwButton::Start);
}

#[test]
fn stopwatch_laps_newest_first_best_green() {
    let c = FakeClock::new();
    let mut sw = Stopwatch::new(c.clone());
    assert!(sw.lap().is_none(), "no lap while stopped");
    sw.start_stop();
    c.advance_ms(3_000);
    sw.lap();
    assert!(!sw.lap_rows()[0].best, "one lap: nothing marked");
    c.advance_ms(2_000);
    sw.lap();
    c.advance_ms(4_000);
    let l3 = sw.lap().unwrap();
    assert_eq!((l3.n, l3.lap, l3.total), (3, S(4), S(9)));
    let rows = sw.lap_rows();
    assert_eq!(rows.iter().map(|r| r.lap.n).collect::<Vec<_>>(), vec![3, 2, 1]);
    assert_eq!(rows.iter().map(|r| r.best).collect::<Vec<_>>(), vec![false, true, false]);
    assert_eq!((rows[1].label.as_str(), rows[1].lap_text.as_str(), rows[1].total_text.as_str()), ("Lap 2", "0:02.00", "0:05.00"));
}

#[test]
fn stopwatch_laps_across_a_pause() {
    let c = FakeClock::new();
    let mut sw = Stopwatch::new(c.clone());
    sw.start_stop();
    c.advance_ms(1_000);
    sw.start_stop();
    c.advance_ms(60_000);
    sw.start_stop();
    c.advance_ms(1_000);
    let l = sw.lap().unwrap();
    assert_eq!((l.lap, l.total), (S(2), S(2)), "paused time is not in the lap");
}

// ------------------------------------------------------------------------------------------------ countdown
#[test]
fn countdown_type_enter_runs_and_finishes_once_with_chime() {
    let c = FakeClock::new();
    let mut snd = FakeSound::default();
    let mut cd = Countdown::new(c.clone());
    assert!(!cd.sound_on, "the end sound is off by default");
    cd.sound_on = true;
    assert_eq!(cd.text(), "5:00");
    assert_eq!(cd.button(), CdButton::Start);
    assert_eq!(cd.line(), 0.0, "empty at rest");
    assert!(!cd.reset_enabled());
    assert_eq!(cd.enter("1:30").unwrap(), S(90));
    assert!(cd.is_running());
    assert_eq!(cd.button(), CdButton::Pause);
    assert_eq!(cd.deadline(), Some(S(90)));
    assert_eq!(cd.type_time("5"), Err(TimerError::Running), "digits read-only while running");
    c.advance_ms(45_000);
    assert_eq!(cd.text(), "0:45");
    assert!((cd.line() - 0.5).abs() < 1e-9);
    assert!(cd.check(&mut snd).is_none(), "not yet");
    c.advance_ms(44_999);
    assert!(cd.check(&mut snd).is_none(), "1 ms before zero");
    assert_eq!(cd.text(), "0:01");
    c.advance_ms(1);
    let done = cd.check(&mut snd).expect("done at zero");
    assert_eq!(done.toast, "Countdown done · 1:30");
    assert!(done.chimed);
    assert_eq!(snd.played, vec![chime_wav().len()]);
    assert!(cd.check(&mut snd).is_none(), "finishes once");
    assert_eq!(cd.button(), CdButton::Again);
    assert_eq!(cd.text(), "0:00");
    // Again = start from the set time
    cd.start_pause();
    assert_eq!(cd.left(), S(90));
    assert!(cd.is_running());
}

#[test]
fn countdown_pause_resume_reset() {
    let c = FakeClock::new();
    let mut cd = Countdown::new(c.clone());
    cd.start_pause();
    c.advance_ms(60_000);
    cd.start_pause();
    assert_eq!(cd.button(), CdButton::Resume);
    c.advance_ms(600_000); // paused: nothing moves
    assert_eq!(cd.left(), S(240));
    assert_eq!(cd.deadline(), None);
    cd.start_pause();
    assert_eq!(cd.deadline(), Some(S(660 + 240)));
    cd.reset();
    assert_eq!((cd.left(), cd.button(), cd.is_running()), (S(300), CdButton::Start, false));
    assert_eq!(cd.line(), 0.0);
}

#[test]
fn countdown_sound_off_and_failing_sound() {
    let c = FakeClock::new();
    let mut cd = Countdown::new(c.clone());
    let mut snd = FakeSound::default();
    cd.enter("90s").unwrap();
    c.advance_ms(90_000);
    let d = cd.check(&mut snd).unwrap();
    assert!(!d.chimed && snd.played.is_empty());
    assert_eq!(cd.preview_sound(&mut snd), Ok(false), "preview plays nothing while off");
    cd.sound_on = true;
    assert_eq!(cd.preview_sound(&mut snd), Ok(true));
    // a failing sound never blocks the end
    let mut bad = FakeSound { fail: true, ..Default::default() };
    cd.enter("1").unwrap();
    c.advance_ms(60_000);
    let d = cd.check(&mut bad).unwrap();
    assert!(!d.chimed);
    assert!(cd.preview_sound(&mut bad).is_err());
}

#[test]
fn countdown_bad_typing_keeps_the_old_time() {
    let c = FakeClock::new();
    let mut cd = Countdown::new(c);
    assert_eq!(cd.type_time("soon"), Err(TimerError::InvalidTime("soon".into())));
    assert_eq!(cd.set_time(), S(300));
    assert!(cd.enter("0").is_err());
    assert!(!cd.is_running());
}

// ------------------------------------------------------------------------------------------------ chime (measured from the buffer)
#[test]
fn chime_is_soft_short_and_the_drawing_tones() {
    let s = chime_samples();
    let secs = s.len() as f64 / SAMPLE_RATE as f64;
    assert!((0.69..0.71).contains(&secs), "{secs}");
    let peak = s.iter().fold(0f32, |m, x| m.max(x.abs()));
    assert!(peak > 0.15 && peak < 0.5, "soft, never clips: {peak}");
    assert!(s[0].abs() < 1e-3 && s[s.len() - 1].abs() < 1e-3, "starts and ends silent");
    // the main tone 1046.5 × 1.12 = 1172 Hz is there; a tone far off (500 Hz) is not
    let g = |f: f64| goertzel(&s[..8820], f);
    assert!(g(1172.08) > 20.0 * g(500.0), "{} vs {}", g(1172.08), g(500.0));
    let wav = chime_wav();
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(&wav[8..16], b"WAVEfmt ");
    assert_eq!(wav.len(), 44 + s.len() * 2);
}

#[test]
fn the_played_chime_is_quiet_5_percent_on_the_decibel_curve() {
    use bu_timers::sound::{volume_gain, CHIME_VOLUME};
    assert_eq!(CHIME_VOLUME, 5);
    let full = chime_samples().iter().fold(0f32, |m, x| m.max(x.abs()));
    let wav = chime_wav();
    let played = wav[44..].chunks_exact(2).map(|c| (i16::from_le_bytes([c[0], c[1]]) as f32).abs() / 32767.0).fold(0f32, f32::max);
    assert!(played > 0.0 && played < 0.01, "peak {played} of full scale (the recipe alone peaks at {full})");
    assert!((played / full - volume_gain(5)).abs() < 0.001);
    assert_eq!(volume_gain(0), 0.0);
    assert!((volume_gain(100) - 1.0).abs() < 1e-6);
}

fn goertzel(x: &[f32], f: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * f / SAMPLE_RATE as f64;
    let (mut s1, mut s2) = (0.0, 0.0);
    for &v in x {
        let s0 = v as f64 + 2.0 * w.cos() * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2).sqrt()
}

// ------------------------------------------------------------------------------------------------ timer bars
fn drawing_bars(c: &FakeClock) -> (TimerBars<FakeClock>, u32, u32) {
    let mut tb = TimerBars::new(c.clone());
    let u = tb.add("Ultimate", S(90));
    let d = tb.add("Dash", S(12));
    (tb, u, d)
}

#[test]
fn bars_off_by_default_and_off_starts_nothing() {
    let c = FakeClock::new();
    let (mut tb, u, _) = drawing_bars(&c);
    assert!(!tb.is_on() && !tb.open);
    assert_eq!(tb.start(u), Err(TimerError::BarsOff));
    assert!(tb.screen(true).is_empty(), "off: nothing on screen, not even the preview");
    tb.set_on(true);
    assert!(tb.open, "switching on unfolds the card");
    tb.start(u).unwrap();
    tb.set_on(false);
    assert!(!tb.is_running(u), "off clears running bars");
    assert!(!tb.open);
}

#[test]
fn bar_runs_turns_red_last_3s_and_fades_once() {
    let c = FakeClock::new();
    let (mut tb, _, d) = drawing_bars(&c);
    tb.set_on(true);
    tb.start(d).unwrap();
    c.advance_ms(6_000);
    let v = tb.screen(false);
    assert_eq!(v.pills.len(), 1, "only the running bar shows");
    let p = &v.pills[0];
    assert_eq!((p.name.as_str(), p.left_text.as_str(), p.low, p.running), ("Dash", "0:06", false, true));
    assert!((p.line - 0.5).abs() < 1e-9);
    assert_eq!(tb.deadline(), Some(S(9)), "the next change on its own: the red turn");
    c.advance_ms(3_000);
    assert!(tb.screen(false).pills[0].low, "red at 3 s left");
    assert_eq!(tb.deadline(), Some(S(12)), "then the end");
    c.advance_ms(2_999);
    assert!(tb.screen(false).pills[0].low);
    c.advance_ms(1);
    let v = tb.screen(false);
    assert!(v.is_empty());
    assert_eq!(v.finished, vec![d], "fades out");
    assert!(tb.screen(false).finished.is_empty(), "reported once");
    assert_eq!(tb.deadline(), None);
}

#[test]
fn pressing_again_starts_over() {
    let c = FakeClock::new();
    let (mut tb, u, _) = drawing_bars(&c);
    tb.set_on(true);
    tb.start(u).unwrap();
    c.advance_ms(80_000);
    tb.start(u).unwrap();
    assert_eq!(tb.screen(false).pills[0].left, S(90));
    assert_eq!(tb.screen(false).pills.len(), 1, "one pill per bar, not two");
}

#[test]
fn bars_stack_in_list_order_and_preview_shows_all_part_way() {
    let c = FakeClock::new();
    let (mut tb, u, d) = drawing_bars(&c);
    tb.set_on(true);
    tb.start(d).unwrap();
    tb.start(u).unwrap();
    assert_eq!(tb.screen(false).pills.iter().map(|p| p.id).collect::<Vec<_>>(), vec![u, d], "list order, not start order");
    tb.set_on(false);
    tb.set_on(true);
    let v = tb.screen(true);
    assert_eq!(v.pills.len(), 2, "card open on Timers: every bar shows");
    assert!(v.pills.iter().all(|p| !p.running && !p.low));
    assert!((v.pills[0].line - 0.7).abs() < 1e-9 && (v.pills[1].line - 0.35).abs() < 1e-9);
    assert_eq!(v.pills[0].left_text, "1:03");
    tb.open = false;
    assert!(tb.screen(true).is_empty(), "folded card: no preview");
    tb.set_moving(true);
    assert_eq!(tb.screen(false).pills.len(), 2, "Move bars shows them all");
}

#[test]
fn bar_editing_name_duration_colour_key_remove() {
    let c = FakeClock::new();
    let (mut tb, u, d) = drawing_bars(&c);
    assert_eq!(tb.bar(u).unwrap().colour_rgb(), COLOURS[1]);
    assert_eq!(tb.next_colour(u).unwrap(), COLOURS[2]);
    for _ in 0..4 {
        tb.next_colour(u).unwrap();
    }
    assert_eq!(tb.bar(u).unwrap().colour_rgb(), COLOURS[1], "wraps around");
    assert_eq!(tb.rename(u, "   ").unwrap(), "Ultimate", "empty keeps the old name");
    assert_eq!(tb.rename(u, "  A very long bar name here  ").unwrap(), "A very long bar na");
    assert_eq!(tb.set_duration_text(d, "45").unwrap(), S(45));
    assert_eq!(tb.set_duration_text(d, "1:30").unwrap(), S(90));
    assert_eq!(tb.set_duration_text(d, "2m").unwrap(), S(120));
    assert!(tb.set_duration_text(d, "never").is_err());
    assert_eq!(tb.bar(d).unwrap().duration, S(120), "bad text keeps the old duration");
    tb.set_key(u, Some("F7")).unwrap();
    assert_eq!(tb.key_clash(), None);
    tb.set_key(d, Some("f7")).unwrap();
    assert_eq!(tb.key_clash().as_deref(), Some("Already used by Timer bar · A very long bar na"));
    tb.set_key(d, None).unwrap();
    assert_eq!(tb.key_clash(), None);
    let n = tb.add_new();
    assert_eq!((tb.bar(n).unwrap().name.as_str(), tb.bar(n).unwrap().duration), ("Timer 3", S(30)));
    tb.set_on(true);
    tb.start(d).unwrap();
    tb.remove(d).unwrap();
    assert!(!tb.is_running(d));
    assert_eq!(tb.remove(d), Err(TimerError::NoSuchBar(d)));
    assert_eq!(tb.start(999), Err(TimerError::NoSuchBar(999)));
}

#[test]
fn bars_look_defaults_spots_sizes_opacity() {
    let c = FakeClock::new();
    let mut tb = TimerBars::new(c);
    assert_eq!(tb.look.spot, Spot { h: Horiz::Centre, v: Vert::Top, dx: 0, dy: 24 }, "top middle, 24 px");
    assert_eq!(tb.look.spot.quick_index(), Some(1));
    assert_eq!(tb.look.show_on, ShowOn::Main);
    assert_eq!(BarSize::M.pill_px(), (196, 32));
    assert_eq!(BarSize::S.pill_px(), (161, 26));
    assert_eq!(BarSize::L.pill_px(), (239, 39));
    for (i, (h, v)) in QUICK_SPOTS.iter().enumerate() {
        assert_eq!(Spot::quick(*h, *v).quick_index(), Some(i));
    }
    tb.look.spot = Spot { h: Horiz::Left, v: Vert::Top, dx: 300, dy: 24 };
    assert_eq!(tb.look.spot.quick_index(), None, "a dragged spot lights no quick spot");
    tb.set_opacity(10);
    assert_eq!(tb.look.opacity_pct, 30);
    tb.set_opacity(200);
    assert_eq!(tb.look.opacity_pct, 100);
}

#[test]
fn huge_durations_never_panic() {
    let c = FakeClock::new();
    let mut tb = TimerBars::new(c.clone());
    let id = tb.add("Big", Duration::MAX);
    assert_eq!(tb.bar(id).unwrap().duration, S(MAX_SECS), "capped at 99:59:59");
    tb.set_on(true);
    tb.start(id).unwrap();
    c.advance(Duration::from_secs(u64::MAX / 4));
    let mut cd = Countdown::new(c.clone());
    cd.start_pause();
    assert!(cd.deadline().is_some());
    assert!(bu_timers::MonoClock::new().instant_at(Duration::MAX).is_none());
}

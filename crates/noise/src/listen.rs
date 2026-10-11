//! The listening tracker (Order 092): how long the noise was on today, this week, this month, this year and all time.
//!
//! It counts ONLY at Play and at Stop: Play notes the start, Stop (or the sleep timer, the tray's "Stop noise", the app's exit)
//! adds the finished stretch to a per-day ledger and writes it. No timer, no thread, nothing runs in between; the sums are made
//! when the tab opens. A crash loses only the stretch that was still open.
//!
//! A stretch counts the shorter of "Stop time minus Play time" and the sound the player really wrote to the speakers
//! (`Status::played`), so a PC that slept with the noise on, or a missing output device, is not counted as listened.
//!
//! Days are the PC's local days (a stretch over midnight is split); the week starts on Monday. The ledger file holds one line
//! per day that had noise: `<day number>\t<seconds>` (day number = days since 1970-01-01, local).

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

const DAY: i64 = 86_400;

/// Days since 1970-01-01 of a civil date (proleptic Gregorian; H. Hinnant's `days_from_civil`).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date (year, month 1-12, day 1-31) of a day number.
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 0 = Monday ... 6 = Sunday (1970-01-01 was a Thursday).
fn weekday(day: i64) -> i64 {
    (day + 3).rem_euclid(7)
}

/// Local time now as seconds since 1970-01-01 00:00 local (the PC's clock, its time zone; not UTC).
#[cfg(windows)]
pub fn local_now() -> i64 {
    // SAFETY: a plain query with no arguments.
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    days_from_civil(i64::from(t.wYear), i64::from(t.wMonth), i64::from(t.wDay)) * DAY
        + i64::from(t.wHour) * 3600
        + i64::from(t.wMinute) * 60
        + i64::from(t.wSecond)
}

/// The five figures of the row, in seconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    pub today: u64,
    pub week: u64,
    pub month: u64,
    pub year: u64,
    pub all: u64,
}

/// "42m", "2h 05m", "132h" - short, so five of them sit in one row. Under a minute counts as "0m".
pub fn format(secs: u64) -> String {
    let mins = secs / 60;
    if mins < 60 {
        format!("{mins}m")
    } else if mins < 6000 {
        format!("{}h {:02}m", mins / 60, mins % 60)
    } else {
        format!("{}h", mins / 60)
    }
}

/// Seconds listened per local day.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ledger {
    days: BTreeMap<i64, u64>,
}

impl Ledger {
    /// Reads the file. Missing file or a broken line = nothing for that part (never an error).
    pub fn load(path: &Path) -> Ledger {
        let mut l = Ledger::default();
        let Ok(text) = std::fs::read_to_string(path) else { return l };
        for line in text.lines() {
            let mut it = line.split('\t');
            if let (Some(Ok(d)), Some(Ok(s)), None) = (it.next().map(|v| v.trim().parse::<i64>()), it.next().map(|v| v.trim().parse::<u64>()), it.next()) {
                *l.days.entry(d).or_insert(0) += s;
            }
        }
        l
    }

    /// Writes the file whole (a temporary file, then a rename: a crash never leaves half a ledger).
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let mut text = String::new();
        for (d, s) in &self.days {
            text.push_str(&format!("{d}\t{s}\n"));
        }
        let tmp: PathBuf = path.with_extension("tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)
    }

    /// Adds the stretch `start .. start + secs` (local seconds), split at every midnight it crosses.
    pub fn add(&mut self, start: i64, secs: u64) {
        let mut at = start;
        let end = start.saturating_add(secs.min(i64::MAX as u64 / 2) as i64);
        while at < end {
            let day = at.div_euclid(DAY);
            let next = (day + 1) * DAY;
            let part = end.min(next) - at;
            *self.days.entry(day).or_insert(0) += part as u64;
            at = next;
        }
    }

    /// The five sums as of `now` (local seconds); `open` = a stretch that is still running: (start, seconds so far).
    pub fn totals(&self, now: i64, open: Option<(i64, u64)>) -> Totals {
        let mut copy;
        let l = match open {
            Some((start, secs)) => {
                copy = self.clone();
                copy.add(start, secs);
                &copy
            }
            None => self,
        };
        let today = now.div_euclid(DAY);
        let (y, m, _) = civil_from_days(today);
        let week_start = today - weekday(today);
        let month_start = days_from_civil(y, m, 1);
        let year_start = days_from_civil(y, 1, 1);
        let mut t = Totals::default();
        for (d, s) in &l.days {
            if *d > today {
                continue; // a clock that was set back: those days are not "so far"
            }
            t.all += s;
            if *d >= year_start {
                t.year += s;
            }
            if *d >= month_start {
                t.month += s;
            }
            if *d >= week_start {
                t.week += s;
            }
            if *d == today {
                t.today += s;
            }
        }
        t
    }
}

/// How much the player has written so far: seconds of sound, `Status::played`.
pub type Played = f64;

#[derive(Default)]
struct Inner {
    path: Option<PathBuf>,
    ledger: Option<Ledger>,
    /// The running stretch: (start in local seconds, `played` at the start).
    open: Option<(i64, Played)>,
}

/// The tracker: one per process (the app keeps it in a static), callable from any thread. Without a file (`set_file` not
/// called: a test copy) it only counts in memory.
#[derive(Default)]
pub struct Tracker {
    inner: Mutex<Inner>,
}

fn lock(m: &Mutex<Inner>) -> MutexGuard<'_, Inner> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Tracker {
    pub const fn new() -> Tracker {
        Tracker { inner: Mutex::new(Inner { path: None, ledger: None, open: None }) }
    }

    /// Where the ledger lives. Nothing is read until it is needed (the app start costs nothing).
    pub fn set_file(&self, path: PathBuf) {
        let mut i = lock(&self.inner);
        i.path = Some(path);
        i.ledger = None;
    }

    fn ledger(i: &mut Inner) -> &mut Ledger {
        let path = i.path.clone();
        i.ledger.get_or_insert_with(|| path.map_or_else(Ledger::default, |p| Ledger::load(&p)))
    }

    /// Play: the stretch starts now (`now` = local seconds, `played` = the player's `Status::played`). A stretch that is
    /// already open stays as it is.
    pub fn begin(&self, now: i64, played: Played) {
        let mut i = lock(&self.inner);
        if i.open.is_none() {
            i.open = Some((now, played));
        }
    }

    /// Stop (any way the noise ends): the open stretch is added to its days and the file is written. Nothing open = nothing.
    pub fn end(&self, now: i64, played: Played) {
        let mut i = lock(&self.inner);
        let Some((start, p0)) = i.open.take() else { return };
        let secs = stretch(start, p0, now, played);
        if secs == 0 {
            return;
        }
        Self::ledger(&mut i).add(start, secs);
        if let (Some(p), Some(l)) = (i.path.clone(), i.ledger.as_ref()) {
            let _ = l.save(&p);
        }
    }

    pub fn is_open(&self) -> bool {
        lock(&self.inner).open.is_some()
    }

    /// The figures as of now, a running stretch included (up to now). Called when the tab opens and at Stop.
    pub fn totals(&self, now: i64, played: Played) -> Totals {
        let mut i = lock(&self.inner);
        let open = i.open.map(|(start, p0)| (start, stretch(start, p0, now, played)));
        Self::ledger(&mut i).totals(now, open)
    }
}

/// The seconds a stretch counts: the shorter of the clock and what the player wrote.
fn stretch(start: i64, p0: Played, now: i64, played: Played) -> u64 {
    let wall = (now - start).max(0) as u64;
    let wrote = (played - p0).max(0.0).floor() as u64;
    wall.min(wrote)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i64, m: i64, d: i64, h: i64, min: i64) -> i64 {
        days_from_civil(y, m, d) * DAY + h * 3600 + min * 60
    }

    #[test]
    fn the_date_maths_round_trips_and_knows_the_weekday() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(weekday(0), 3, "a Thursday");
        let d = days_from_civil(2026, 10, 10);
        assert_eq!(civil_from_days(d), (2026, 10, 10));
        assert_eq!(weekday(d), 5, "Saturday 10 Oct 2026");
        for z in [-800, -1, 0, 59, 60, 11_000, 20_000, 20_500] {
            let (y, m, dd) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, dd), z);
        }
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29), "leap day");
    }

    #[test]
    fn the_five_figures_cut_at_midnight_monday_the_1st_and_january() {
        let mut l = Ledger::default();
        // Sat 10 Oct 2026 is "today"; Mon 5 Oct starts its week; 1 Oct its month; 1 Jan its year
        l.add(at(2026, 10, 10, 8, 0), 600); // today 10 min
        l.add(at(2026, 10, 5, 9, 0), 1200); // Monday this week 20 min
        l.add(at(2026, 10, 4, 22, 0), 1800); // Sunday: last week, this month 30 min
        l.add(at(2026, 9, 30, 22, 0), 2400); // last month, this year 40 min
        l.add(at(2025, 12, 31, 22, 0), 3000); // last year 50 min
        let t = l.totals(at(2026, 10, 10, 12, 0), None);
        assert_eq!(t, Totals { today: 600, week: 1800, month: 3600, year: 6000, all: 9000 });
    }

    #[test]
    fn a_stretch_over_midnight_is_split_between_the_two_days() {
        let mut l = Ledger::default();
        l.add(at(2026, 10, 9, 23, 30), 3600); // 30 min before, 30 min after midnight
        let t = l.totals(at(2026, 10, 10, 1, 0), None);
        assert_eq!((t.today, t.all), (1800, 3600));
        // a 30 h stretch touches three days
        let mut l = Ledger::default();
        l.add(at(2026, 10, 8, 23, 0), 30 * 3600);
        assert_eq!(l.days.len(), 3);
        assert_eq!(l.days.values().sum::<u64>(), 30 * 3600);
    }

    #[test]
    fn a_running_stretch_counts_up_to_now_without_being_kept() {
        let mut l = Ledger::default();
        l.add(at(2026, 10, 10, 8, 0), 600);
        let now = at(2026, 10, 10, 12, 0);
        let t = l.totals(now, Some((now - 300, 300)));
        assert_eq!((t.today, t.all), (900, 900));
        assert_eq!(l.totals(now, None).today, 600, "the ledger itself did not change");
    }

    #[test]
    fn a_clock_set_back_never_shows_days_from_the_future() {
        let mut l = Ledger::default();
        l.add(at(2026, 12, 1, 8, 0), 600);
        assert_eq!(l.totals(at(2026, 10, 10, 12, 0), None), Totals::default());
    }

    #[test]
    fn the_file_round_trips_and_broken_lines_are_skipped() {
        let dir = std::env::temp_dir().join(format!("bu-listen-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("noise-listening.txt");
        let mut l = Ledger::default();
        l.add(at(2026, 10, 10, 8, 0), 600);
        l.add(at(2026, 10, 9, 8, 0), 120);
        l.save(&f).unwrap();
        assert_eq!(Ledger::load(&f), l);
        std::fs::write(&f, "20000\t60\nrubbish\n20001\tabc\n20002\t30\t9\n-5\t1\n20003\t10\n").unwrap();
        let b = Ledger::load(&f);
        assert_eq!(b.days.get(&20_000), Some(&60));
        assert_eq!(b.days.get(&20_003), Some(&10));
        assert_eq!(b.days.len(), 3, "20000, 20003 and the odd -5 day; the broken lines are gone: {:?}", b.days);
        assert_eq!(Ledger::load(&dir.join("missing.txt")), Ledger::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_words_are_short() {
        assert_eq!(format(0), "0m");
        assert_eq!(format(59), "0m");
        assert_eq!(format(60 * 42), "42m");
        assert_eq!(format(3600 + 300), "1h 05m");
        assert_eq!(format(2 * 3600 + 59 * 60 + 59), "2h 59m");
        assert_eq!(format(132 * 3600 + 20 * 60), "132h");
    }

    #[test]
    fn play_and_stop_are_all_that_is_counted() {
        let t = Tracker::new();
        let t0 = at(2026, 10, 10, 20, 0);
        assert!(!t.is_open());
        t.begin(t0, 100.0);
        t.begin(t0 + 5, 105.0); // a second Play while one is open changes nothing
        assert!(t.is_open());
        // 10 min later, the player wrote 10 min of sound
        assert_eq!(t.totals(t0 + 600, 700.0).today, 600, "a running stretch shows up to now");
        t.end(t0 + 600, 700.0);
        assert!(!t.is_open());
        assert_eq!(t.totals(t0 + 700, 700.0), Totals { today: 600, week: 600, month: 600, year: 600, all: 600 });
        t.end(t0 + 900, 900.0); // a Stop with nothing open adds nothing
        assert_eq!(t.totals(t0 + 900, 900.0).all, 600);
    }

    #[test]
    fn sleep_or_no_output_is_not_listening() {
        let t = Tracker::new();
        let t0 = at(2026, 10, 10, 20, 0);
        // the PC slept for 8 h with the noise on: the clock says 8 h 10 min, the player wrote 10 min
        t.begin(t0, 50.0);
        t.end(t0 + 8 * 3600 + 600, 650.0);
        assert_eq!(t.totals(t0 + 9 * 3600, 650.0).all, 600);
        // no output device the whole time: nothing was written, nothing counts
        t.begin(t0 + 10 * 3600, 650.0);
        t.end(t0 + 11 * 3600, 650.0);
        assert_eq!(t.totals(t0 + 12 * 3600, 650.0).all, 600);
    }

    #[test]
    fn the_stretch_is_written_to_the_file_at_stop_and_read_back_by_a_new_tracker() {
        let dir = std::env::temp_dir().join(format!("bu-listen-t-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("noise-listening.txt");
        let _ = std::fs::remove_file(&f);
        let t = Tracker::new();
        t.set_file(f.clone());
        let t0 = at(2026, 10, 10, 20, 0);
        t.begin(t0, 0.0);
        assert!(!f.exists(), "nothing is written at Play");
        t.end(t0 + 90, 90.0);
        assert!(f.exists());
        let again = Tracker::new();
        again.set_file(f);
        assert_eq!(again.totals(t0 + 100, 0.0).all, 90);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

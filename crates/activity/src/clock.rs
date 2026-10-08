//! Wall-clock time with the local time-zone offset, and local calendar days. Activity counts per LOCAL day (midnight
//! splits a span), so it needs the wall clock, not a monotonic one.

use std::sync::{Arc, Mutex};

/// A moment: Unix milliseconds (UTC) + the local offset at that moment (minutes east of UTC, e.g. +120 in summer CEST).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Stamp {
    pub unix_ms: i64,
    pub offset_min: i32,
}

pub const DAY_MS: i64 = 86_400_000;

impl Stamp {
    pub fn local_ms(&self) -> i64 {
        self.unix_ms + self.offset_min as i64 * 60_000
    }
    /// Local day number (days since 1970-01-01, local).
    pub fn day(&self) -> i64 {
        self.local_ms().div_euclid(DAY_MS)
    }
    /// Minutes since local midnight.
    pub fn local_minute(&self) -> u32 {
        (self.local_ms().rem_euclid(DAY_MS) / 60_000) as u32
    }
    /// UTC ms of the next local midnight after this moment (with this moment's offset).
    pub fn next_midnight_ms(&self) -> i64 {
        (self.day() + 1) * DAY_MS - self.offset_min as i64 * 60_000
    }
    pub fn plus_ms(&self, ms: i64) -> Stamp {
        Stamp { unix_ms: self.unix_ms.saturating_add(ms), offset_min: self.offset_min }
    }
}

/// A local calendar date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    pub y: i32,
    pub m: u32,
    pub d: u32,
}

impl Date {
    /// From a day number (Howard Hinnant's civil_from_days).
    pub fn from_day(z: i64) -> Date {
        let z = z + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        Date { y: (if m <= 2 { y + 1 } else { y }) as i32, m, d }
    }
    /// To a day number (days_from_civil).
    pub fn to_day(self) -> i64 {
        let y = if self.m <= 2 { self.y as i64 - 1 } else { self.y as i64 };
        let era = y.div_euclid(400);
        let yoe = y.rem_euclid(400);
        let m = self.m as i64;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + self.d as i64 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }
    /// "2026-10-08" (the day file's name).
    pub fn iso(self) -> String {
        format!("{:04}-{:02}-{:02}", self.y, self.m, self.d)
    }
    pub fn parse_iso(s: &str) -> Option<Date> {
        let mut it = s.split('-');
        let y = it.next()?.parse().ok()?;
        let m = it.next()?.parse().ok()?;
        let d = it.next()?.parse().ok()?;
        if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
            return None;
        }
        Some(Date { y, m, d })
    }
}

pub const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
pub const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// "Wed" for a day number (1970-01-01 was a Thursday).
pub fn weekday(day: i64) -> &'static str {
    WEEKDAYS[(day + 3).rem_euclid(7) as usize]
}

/// "Sat 3 Oct"
pub fn day_label(day: i64) -> String {
    let d = Date::from_day(day);
    format!("{} {} {}", weekday(day), d.d, MONTHS[(d.m - 1) as usize])
}

pub trait Clock: Send + Sync {
    fn now(&self) -> Stamp;
}

/// The real clock: `SystemTime` + the Windows local offset right now (`GetLocalTime` − `GetSystemTime`).
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Stamp {
        let unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        Stamp { unix_ms, offset_min: local_offset_min() }
    }
}

#[cfg(windows)]
fn local_offset_min() -> i32 {
    use windows::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};
    // SAFETY: plain reads of the clock.
    let (l, u) = unsafe { (GetLocalTime(), GetSystemTime()) };
    let mins = |t: &windows::Win32::Foundation::SYSTEMTIME| {
        Date { y: t.wYear as i32, m: t.wMonth as u32, d: t.wDay as u32 }.to_day() * 1440 + t.wHour as i64 * 60 + t.wMinute as i64
    };
    (mins(&l) - mins(&u)) as i32
}

#[cfg(not(windows))]
fn local_offset_min() -> i32 {
    0
}

/// A clock tests set by hand. Clones share the time.
#[derive(Debug, Clone)]
pub struct FakeClock(Arc<Mutex<Stamp>>);

impl FakeClock {
    pub fn at(s: Stamp) -> Self {
        FakeClock(Arc::new(Mutex::new(s)))
    }
    pub fn advance_ms(&self, ms: i64) {
        let mut g = self.0.lock().unwrap_or_else(|p| p.into_inner());
        *g = g.plus_ms(ms);
    }
    pub fn set(&self, s: Stamp) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = s;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Stamp {
        *self.0.lock().unwrap_or_else(|p| p.into_inner())
    }
}

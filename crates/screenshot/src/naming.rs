//! File names: `Screenshot YYYY-MM-DD HH-MM.png` (DESIGN §3.3). A second shot in the same minute gets ` (2)`, ` (3)` … the
//! way Windows names copies — DESIGN leaves this **unclear**; this is a call, written in the report.

use std::path::{Path, PathBuf};

/// Local wall-clock time (what the user's clock shows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct LocalTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

/// Days since 1970-01-01 of a calendar date (proleptic Gregorian; Howard Hinnant's `days_from_civil`).
pub fn days_from_civil(year: i64, month: u8, day: u8) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The wall-clock time of `unix_ms` (UTC milliseconds) at a fixed offset from UTC (`offset_min`, e.g. +120 for CEST).
pub fn civil(unix_ms: u64, offset_min: i32) -> LocalTime {
    let secs = (unix_ms / 1000) as i64 + offset_min as i64 * 60;
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    // Hinnant's civil_from_days
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let year = (yoe + era * 400 + if month <= 2 { 1 } else { 0 }) as u16;
    LocalTime { year, month, day, hour: (sod / 3600) as u8, minute: (sod % 3600 / 60) as u8, second: (sod % 60) as u8 }
}

const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"]; // 1970-01-01 was a Thursday
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// The gallery caption's time of a shot taken at `t`, seen at `now` (both wall-clock): today "21:34", the day before
/// "Yesterday", the 5 days before that the weekday ("Mon"), older "Sep 28" (another year: "Sep 28, 2025"). The drawing
/// shows the first three; the older form is a call (Order 019 report).
pub fn caption(now: &LocalTime, t: &LocalTime) -> String {
    let dn = days_from_civil(now.year as i64, now.month, now.day);
    let dt = days_from_civil(t.year as i64, t.month, t.day);
    let month = MONTHS[(t.month.clamp(1, 12) - 1) as usize];
    match dn - dt {
        d if d <= 0 => format!("{:02}:{:02}", t.hour, t.minute),
        1 => "Yesterday".into(),
        2..=6 => WEEKDAYS[dt.rem_euclid(7) as usize].into(),
        _ if t.year == now.year => format!("{} {}", month, t.day),
        _ => format!("{} {}, {}", month, t.day, t.year),
    }
}

/// `Screenshot 2026-10-08 01-36.png`
pub fn base_name(t: &LocalTime) -> String {
    format!("Screenshot {:04}-{:02}-{:02} {:02}-{:02}", t.year, t.month, t.day, t.hour, t.minute)
}

/// The first free name in `dir` for time `t`: the plain name, else ` (2)`, ` (3)` … `exists` answers whether a path is taken.
pub fn free_path(dir: &Path, t: &LocalTime, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let base = base_name(t);
    let first = dir.join(format!("{base}.png"));
    if !exists(&first) {
        return first;
    }
    let mut n: u64 = 2;
    loop {
        let p = dir.join(format!("{base} ({n}).png"));
        if !exists(&p) {
            return p;
        }
        n += 1;
    }
}

//! A calendar date (for "Installed": "2 Sep 2026").

/// Year, month 1-12, day 1-31. Orders by time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl Date {
    /// `20260902` → 2 Sep 2026. Anything else (empty, "2026-09-02" junk, month 13) → None.
    pub fn parse_yyyymmdd(s: &str) -> Option<Date> {
        let s = s.trim();
        if s.len() != 8 || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let d = Date { year: s[..4].parse().ok()?, month: s[4..6].parse().ok()?, day: s[6..].parse().ok()? };
        (d.year >= 1980 && (1..=12).contains(&d.month) && (1..=days_in_month(d.year, d.month)).contains(&d.day)).then_some(d)
    }

    /// FILETIME ticks (100 ns since 1601-01-01 UTC) → the UTC date.
    pub fn from_filetime(ticks: u64) -> Date {
        let days_since_1601 = (ticks / 10_000_000 / 86_400) as i64;
        // 1601-01-01 → 1970-01-01 is 134774 days.
        civil_from_days(days_since_1601 - 134_774)
    }

    /// "2 Sep 2026" (the drawing's format).
    pub fn display(&self) -> String {
        const M: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        format!("{} {} {}", self.day, M[(self.month.clamp(1, 12) - 1) as usize], self.year)
    }
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        _ => 28,
    }
}

/// Days since 1970-01-01 → date (Howard Hinnant's civil_from_days).
fn civil_from_days(z: i64) -> Date {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    Date { year: (if m <= 2 { y + 1 } else { y }) as i32, month: m, day: d }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        assert_eq!(Date::parse_yyyymmdd("20260902"), Some(Date { year: 2026, month: 9, day: 2 }));
        assert_eq!(Date::parse_yyyymmdd("20240229"), Some(Date { year: 2024, month: 2, day: 29 }));
        assert_eq!(Date::parse_yyyymmdd("20230229"), None);
        assert_eq!(Date::parse_yyyymmdd("20261301"), None);
        assert_eq!(Date::parse_yyyymmdd("2026-9-2"), None);
        assert_eq!(Date::parse_yyyymmdd(""), None);
    }

    #[test]
    fn filetime() {
        // 1970-01-01 = 116444736000000000 ticks.
        assert_eq!(Date::from_filetime(116_444_736_000_000_000), Date { year: 1970, month: 1, day: 1 });
        // 2026-10-06 00:00 UTC = 1791244800 s after 1970.
        assert_eq!(Date::from_filetime(116_444_736_000_000_000 + 1_791_244_800 * 10_000_000), Date { year: 2026, month: 10, day: 6 });
        assert_eq!(Date { year: 2026, month: 9, day: 2 }.display(), "2 Sep 2026");
    }
}

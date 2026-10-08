//! World clock (menu-v21/v22 Timers › World clock): your own time and the time in other places.
//!
//! * The places a user can add are the drawing's list ([`PLACES`]: city, country, Windows time zone key). Each place's
//!   time comes from **Windows' own time zone rules** (`EnumDynamicTimeZoneInformation` +
//!   `SystemTimeToTzSpecificLocalTimeEx`), so summer time and rule changes are Windows' — no table of offsets here.
//! * "Your time" = Windows' local time; its name = the city of your time zone that is the capital of the country set in
//!   Windows (Settings › Time & language › Region), e.g. "Zagreb" for "(UTC+01:00) Sarajevo, Skopje, Warsaw, Zagreb" + HR;
//!   when none of the zone's cities is that capital, the zone's first city.
//! * Everything goes through [`ZoneOs`]: [`RealZones`] (Windows, read-only) and [`FakeZones`] (tests: the drawing's
//!   "now" = Thursday 8 Oct 2026, 21:37 in Zagreb).

use std::time::Duration;

/// One place of the "Add a place" list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    pub city: &'static str,
    pub land: &'static str,
    /// The Windows time zone key (HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Time Zones\<key>).
    pub zone: &'static str,
}

/// The drawing's places, in its order (CITIES).
pub const PLACES: [Place; 10] = [
    Place { city: "New York", land: "USA", zone: "Eastern Standard Time" },
    Place { city: "Los Angeles", land: "USA", zone: "Pacific Standard Time" },
    Place { city: "London", land: "UK", zone: "GMT Standard Time" },
    Place { city: "Tokyo", land: "Japan", zone: "Tokyo Standard Time" },
    Place { city: "Sydney", land: "Australia", zone: "AUS Eastern Standard Time" },
    Place { city: "Dubai", land: "UAE", zone: "Arabian Standard Time" },
    Place { city: "São Paulo", land: "Brazil", zone: "E. South America Standard Time" },
    Place { city: "Seoul", land: "South Korea", zone: "Korea Standard Time" },
    Place { city: "Berlin", land: "Germany", zone: "W. Europe Standard Time" },
    Place { city: "Singapore", land: "Singapore", zone: "Singapore Standard Time" },
];

pub fn place(city: &str) -> Option<Place> {
    PLACES.iter().copied().find(|p| p.city == city)
}

/// A wall-clock time: date, time and weekday (0 = Sunday).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Civil {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub weekday: u32,
}

/// Where the world clock reads the time.
pub trait ZoneOs {
    /// Now, as seconds since 1970-01-01 UTC (with the fraction, for the next-minute wake-up).
    fn utc_now(&self) -> Duration;
    /// The offset from UTC in minutes of a Windows time zone at a moment; `None` = this PC has no such zone.
    fn offset_at(&self, zone: &str, utc_secs: i64) -> Option<i32>;
    /// The PC's own offset from UTC in minutes at a moment.
    fn local_offset_at(&self, utc_secs: i64) -> i32;
    /// The name shown for "your time".
    fn home_name(&self) -> String;
}

/// Days since 1970-01-01 → (year, month, day) (H. Hinnant's civil_from_days).
pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((y + if m <= 2 { 1 } else { 0 }) as i32, m, d)
}

/// (year, month, day) → days since 1970-01-01.
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = y as i64 - if m <= 2 { 1 } else { 0 };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// A UTC moment shifted by an offset (minutes) as a wall-clock time.
pub fn civil_at(utc_secs: i64, offset_min: i32) -> Civil {
    let t = utc_secs + offset_min as i64 * 60;
    let days = t.div_euclid(86_400);
    let s = t.rem_euclid(86_400) as u32;
    let (year, month, day) = civil_from_days(days);
    Civil { year, month, day, hour: s / 3600, minute: (s / 60) % 60, second: s % 60, weekday: (days + 4).rem_euclid(7) as u32 }
}

const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// "21:37"
pub fn hhmm(c: &Civil) -> String {
    format!("{:02}:{:02}", c.hour, c.minute)
}

/// The line under your time: "Thursday 8 Oct · your time".
pub fn home_line(c: &Civil) -> String {
    format!("{} {} {} · your time", DAYS[c.weekday as usize % 7], c.day, MONTHS[(c.month as usize + 11) % 12])
}

/// A place's small line: "USA · −6 h", "Japan · +7 h · tomorrow", "UK · same time". `diff_min` = the place's offset minus
/// yours; `day_diff` = its date minus yours (−1, 0, +1). Half-hour zones read "+5 h 30 m".
pub fn place_line(land: &str, diff_min: i32, day_diff: i64) -> String {
    let rel = if diff_min == 0 {
        "same time".to_string()
    } else {
        let sign = if diff_min > 0 { '+' } else { '\u{2212}' };
        let a = diff_min.unsigned_abs();
        if a.is_multiple_of(60) {
            format!("{sign}{} h", a / 60)
        } else {
            format!("{sign}{} h {} m", a / 60, a % 60)
        }
    };
    let day = match day_diff {
        d if d > 0 => " · tomorrow",
        d if d < 0 => " · yesterday",
        _ => "",
    };
    format!("{land} · {rel}{day}")
}

/// What the World clock shows now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldView {
    pub home_name: String,
    pub home_time: String,
    pub home_line: String,
    /// one per place asked for: (city, time "15:37", line "USA · −6 h")
    pub places: Vec<(String, String, String)>,
}

/// The world clock's view for a list of places (cities of [`PLACES`]); unknown cities / zones show "—".
pub fn world_view(os: &dyn ZoneOs, cities: &[&str]) -> WorldView {
    let utc = os.utc_now().as_secs() as i64;
    let my_off = os.local_offset_at(utc);
    let me = civil_at(utc, my_off);
    let my_days = days_from_civil(me.year, me.month, me.day);
    let places = cities
        .iter()
        .map(|c| match place(c).and_then(|p| os.offset_at(p.zone, utc).map(|o| (p, o))) {
            Some((p, off)) => {
                let t = civil_at(utc, off);
                let dd = days_from_civil(t.year, t.month, t.day) - my_days;
                (c.to_string(), hhmm(&t), place_line(p.land, off - my_off, dd))
            }
            None => (c.to_string(), "\u{2014}".into(), String::new()),
        })
        .collect();
    WorldView { home_name: os.home_name(), home_time: hhmm(&me), home_line: home_line(&me), places }
}

/// How long until the shown minute changes (the app wakes then; nothing ticks in between).
pub fn until_next_minute(os: &dyn ZoneOs) -> Duration {
    let now = os.utc_now();
    let into = Duration::new(now.as_secs() % 60, now.subsec_nanos());
    Duration::from_secs(60).saturating_sub(into).max(Duration::from_millis(1))
}

/// The capital of a country (ISO 3166 2-letter) as Windows writes it in time zone names - used to pick "your" city.
const CAPITALS: [(&str, &str); 48] = [
    ("HR", "Zagreb"), ("SI", "Ljubljana"), ("BA", "Sarajevo"), ("RS", "Belgrade"), ("ME", "Podgorica"), ("MK", "Skopje"),
    ("PL", "Warsaw"), ("CZ", "Prague"), ("SK", "Bratislava"), ("HU", "Budapest"), ("AT", "Vienna"), ("DE", "Berlin"),
    ("CH", "Bern"), ("IT", "Rome"), ("SE", "Stockholm"), ("NL", "Amsterdam"), ("BE", "Brussels"), ("DK", "Copenhagen"),
    ("ES", "Madrid"), ("FR", "Paris"), ("GB", "London"), ("IE", "Dublin"), ("PT", "Lisbon"), ("GR", "Athens"),
    ("RO", "Bucharest"), ("BG", "Sofia"), ("FI", "Helsinki"), ("UA", "Kyiv"), ("LV", "Riga"), ("LT", "Vilnius"),
    ("EE", "Tallinn"), ("TR", "Istanbul"), ("RU", "Moscow"), ("JP", "Tokyo"), ("KR", "Seoul"), ("CN", "Beijing"),
    ("IN", "New Delhi"), ("AU", "Canberra"), ("NZ", "Wellington"), ("AE", "Abu Dhabi"), ("SG", "Singapore"), ("BR", "Brasilia"),
    ("AR", "Buenos Aires"), ("MX", "Mexico City"), ("NO", "Oslo"), ("IL", "Jerusalem"), ("EG", "Cairo"), ("ZA", "Pretoria"),
];

/// "your" city from a Windows zone display name ("(UTC+01:00) Sarajevo, Skopje, Warsaw, Zagreb") and the country code.
pub fn home_city(display: &str, country: &str) -> Option<String> {
    let list = display.split_once(')').map(|(_, r)| r).unwrap_or(display);
    let cities: Vec<&str> = list.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    let cap = CAPITALS.iter().find(|(c, _)| c.eq_ignore_ascii_case(country)).map(|(_, n)| *n);
    if let Some(cap) = cap {
        if let Some(c) = cities.iter().find(|c| c.eq_ignore_ascii_case(cap)) {
            return Some(c.to_string());
        }
    }
    cities.first().map(|c| c.to_string())
}

/// Tests: a fixed "now" (the drawing's Thursday 8 Oct 2026, 21:37:00 in Zagreb = 19:37:00 UTC) that tests may move,
/// the offsets in force on that day, home = Zagreb (+2 h, summer time).
#[derive(Debug, Clone)]
pub struct FakeZones {
    pub now: Duration,
    pub home: String,
    pub local_off: i32,
}

/// 2026-10-08 19:37:00 UTC.
pub const DRAWING_NOW_UTC: u64 = 1_791_488_220;

impl Default for FakeZones {
    fn default() -> Self {
        FakeZones { now: Duration::from_secs(DRAWING_NOW_UTC), home: "Zagreb".into(), local_off: 120 }
    }
}

impl ZoneOs for FakeZones {
    fn utc_now(&self) -> Duration {
        self.now
    }
    fn offset_at(&self, zone: &str, _utc: i64) -> Option<i32> {
        Some(match zone {
            "Eastern Standard Time" => -240,
            "Pacific Standard Time" => -420,
            "GMT Standard Time" => 60,
            "Tokyo Standard Time" => 540,
            "AUS Eastern Standard Time" => 660,
            "Arabian Standard Time" => 240,
            "E. South America Standard Time" => -180,
            "Korea Standard Time" => 540,
            "W. Europe Standard Time" => 120,
            "Singapore Standard Time" => 480,
            _ => return None,
        })
    }
    fn local_offset_at(&self, _utc: i64) -> i32 {
        self.local_off
    }
    fn home_name(&self) -> String {
        self.home.clone()
    }
}

/// Windows' clock and time zones (read-only).
#[cfg(windows)]
#[derive(Debug, Default, Clone, Copy)]
pub struct RealZones;

#[cfg(windows)]
mod real {
    use super::*;
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::Time::{
        EnumDynamicTimeZoneInformation, FileTimeToSystemTime, GetDynamicTimeZoneInformation, SystemTimeToFileTime,
        SystemTimeToTzSpecificLocalTimeEx, DYNAMIC_TIME_ZONE_INFORMATION,
    };

    const EPOCH_100NS: u64 = 116_444_736_000_000_000;

    fn wstr(w: &[u16]) -> String {
        let n = w.iter().position(|&c| c == 0).unwrap_or(w.len());
        String::from_utf16_lossy(&w[..n])
    }

    fn st_of(utc_secs: i64) -> Option<SYSTEMTIME> {
        let v = (utc_secs as i128 * 10_000_000 + EPOCH_100NS as i128) as u64;
        let ft = FILETIME { dwLowDateTime: v as u32, dwHighDateTime: (v >> 32) as u32 };
        let mut st = SYSTEMTIME::default();
        // SAFETY: plain conversion between two stack structs.
        unsafe { FileTimeToSystemTime(&ft, &mut st) }.ok()?;
        Some(st)
    }

    fn secs_of(st: &SYSTEMTIME) -> Option<i64> {
        let mut ft = FILETIME::default();
        // SAFETY: as above.
        unsafe { SystemTimeToFileTime(st, &mut ft) }.ok()?;
        let v = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
        Some((v as i64 - EPOCH_100NS as i64) / 10_000_000)
    }

    fn offset_with(tz: Option<&DYNAMIC_TIME_ZONE_INFORMATION>, utc_secs: i64) -> Option<i32> {
        let u = st_of(utc_secs)?;
        let mut l = SYSTEMTIME::default();
        // SAFETY: tz (if any) is a valid struct read by Windows; u / l are stack structs.
        unsafe { SystemTimeToTzSpecificLocalTimeEx(tz.map(|t| t as *const _), &u, &mut l) }.ok()?;
        Some(((secs_of(&l)? - utc_secs) / 60) as i32)
    }

    fn find_zone(key: &str) -> Option<DYNAMIC_TIME_ZONE_INFORMATION> {
        let mut i = 0;
        loop {
            let mut d = DYNAMIC_TIME_ZONE_INFORMATION::default();
            // SAFETY: Windows fills the struct; ERROR_NO_MORE_ITEMS (259) ends the list.
            if unsafe { EnumDynamicTimeZoneInformation(i, &mut d) } != 0 {
                return None;
            }
            if wstr(&d.TimeZoneKeyName) == key {
                return Some(d);
            }
            i += 1;
        }
    }

    fn display_of(key: &str) -> Option<String> {
        use windows::core::{HSTRING, PCWSTR};
        use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
        let sub = HSTRING::from(format!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Time Zones\\{key}"));
        let mut buf = [0u16; 256];
        let mut len = (buf.len() * 2) as u32;
        // SAFETY: a read of one string value into a stack buffer of `len` bytes.
        let r = unsafe {
            RegGetValueW(HKEY_LOCAL_MACHINE, PCWSTR(sub.as_ptr()), windows::core::w!("Display"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len))
        };
        r.is_ok().then(|| wstr(&buf))
    }

    impl ZoneOs for RealZones {
        fn utc_now(&self) -> Duration {
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default()
        }
        fn offset_at(&self, zone: &str, utc_secs: i64) -> Option<i32> {
            let d = find_zone(zone)?;
            offset_with(Some(&d), utc_secs)
        }
        fn local_offset_at(&self, utc_secs: i64) -> i32 {
            offset_with(None, utc_secs).unwrap_or(0)
        }
        fn home_name(&self) -> String {
            let mut d = DYNAMIC_TIME_ZONE_INFORMATION::default();
            // SAFETY: Windows fills the struct.
            unsafe { GetDynamicTimeZoneInformation(&mut d) };
            let mut geo = [0u16; 16];
            // SAFETY: Windows writes at most geo.len() characters.
            let n = unsafe { windows::Win32::Globalization::GetUserDefaultGeoName(&mut geo) };
            let country = if n > 0 { wstr(&geo) } else { String::new() };
            display_of(&wstr(&d.TimeZoneKeyName)).and_then(|disp| home_city(&disp, &country)).unwrap_or_else(|| "Your time".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trip_and_the_drawings_now() {
        for d in [-1000i64, 0, 59, 20_000, 20_369] {
            let (y, m, dd) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
        let c = civil_at(DRAWING_NOW_UTC as i64, 120);
        assert_eq!((c.year, c.month, c.day, c.hour, c.minute, c.weekday), (2026, 10, 8, 21, 37, 4));
        assert_eq!(home_line(&c), "Thursday 8 Oct · your time");
    }

    #[test]
    fn the_drawings_places() {
        let v = world_view(&FakeZones::default(), &["New York", "Tokyo"]);
        assert_eq!(v.home_name, "Zagreb");
        assert_eq!(v.home_time, "21:37");
        assert_eq!(v.places[0], ("New York".into(), "15:37".into(), "USA · \u{2212}6 h".into()));
        assert_eq!(v.places[1], ("Tokyo".into(), "04:37".into(), "Japan · +7 h · tomorrow".into()));
        assert_eq!(place_line("UK", 0, 0), "UK · same time");
        assert_eq!(place_line("India", 210, 0), "India · +3 h 30 m");
        assert_eq!(place_line("USA", -540, -1), "USA · \u{2212}9 h · yesterday");
    }

    #[test]
    fn home_city_picks_the_countrys_capital() {
        let d = "(UTC+01:00) Sarajevo, Skopje, Warsaw, Zagreb";
        assert_eq!(home_city(d, "HR").as_deref(), Some("Zagreb"));
        assert_eq!(home_city(d, "PL").as_deref(), Some("Warsaw"));
        assert_eq!(home_city(d, "US").as_deref(), Some("Sarajevo"));
        assert_eq!(home_city("", "HR"), None);
    }

    #[test]
    fn next_minute_wake() {
        let mut z = FakeZones::default();
        assert_eq!(until_next_minute(&z), Duration::from_secs(60));
        z.now += Duration::from_millis(59_500);
        assert_eq!(until_next_minute(&z), Duration::from_millis(500));
    }

    #[cfg(windows)]
    #[test]
    fn real_zones_read_only_sanity() {
        // reads only: the PC's clock and zone rules; nothing is changed
        let z = RealZones;
        let t = z.utc_now().as_secs() as i64;
        let tokyo = z.offset_at("Tokyo Standard Time", t);
        assert_eq!(tokyo, Some(540)); // Japan has no summer time
        assert_eq!(z.offset_at("No Such Zone", t), None);
        assert!(!z.home_name().is_empty());
    }
}

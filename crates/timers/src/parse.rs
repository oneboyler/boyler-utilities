//! Typed times and how times are written — the drawing's `parseT` / `fmtCd` / `fmtSw` (menu-v18), made exact.

use std::time::Duration;

/// What a bare number means: the countdown reads "5" as 5 minutes, a timer bar reads "45" as 45 seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BareUnit {
    Minutes,
    Seconds,
}

/// The longest time accepted: 99:59:59 (the drawing's cap, 359 999 s).
pub const MAX_SECS: u64 = 359_999;

/// Reads a typed time. Accepts:
/// * `m:ss` / `h:mm:ss` ("1:30", "1:02:03"; the parts after the first have 1–2 digits),
/// * numbers with units `h` / `m` / `s` (any word starting with them: "1h 20m", "90s", "2 min", "1.5h", "1,5m"),
/// * a bare number in `bare` units ("5", "45").
///
/// Returns whole seconds (rounded), capped at [`MAX_SECS`]. `None` = not a time, or zero.
pub fn parse_time(text: &str, bare: BareUnit) -> Option<u64> {
    let s = text.trim().to_lowercase();
    if s.is_empty() {
        return None;
    }
    if let Some(v) = parse_colon(&s) {
        return (v > 0).then_some(v.min(MAX_SECS));
    }
    // "<number>[unit word]" repeated, e.g. "1h 20m", "90s", "5"
    let b = s.as_bytes();
    let mut i = 0;
    let mut total = 0f64;
    let mut any = false;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            // only spaces may sit between parts; anything else is not a time
            if b[i] == b' ' {
                i += 1;
                continue;
            }
            return None;
        }
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i + 1 < b.len() && (b[i] == b'.' || b[i] == b',') && b[i + 1].is_ascii_digit() {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        }
        let v: f64 = s[start..i].replace(',', ".").parse().ok()?;
        while i < b.len() && b[i] == b' ' {
            i += 1;
        }
        let unit = if i < b.len() && b[i].is_ascii_alphabetic() { Some(b[i]) } else { None };
        let mult = match unit {
            Some(b'h') => 3600.0,
            Some(b'm') => 60.0,
            Some(b's') => 1.0,
            Some(_) => return None,
            None => match bare {
                BareUnit::Minutes => 60.0,
                BareUnit::Seconds => 1.0,
            },
        };
        while i < b.len() && b[i].is_ascii_alphabetic() {
            i += 1;
        }
        total += v * mult;
        any = true;
    }
    if !any {
        return None;
    }
    let secs = total.round();
    if secs < 1.0 {
        return None;
    }
    Some((secs as u64).min(MAX_SECS))
}

fn parse_colon(s: &str) -> Option<u64> {
    if !s.contains(':') {
        return None;
    }
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() > 3 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|c| c.is_ascii_digit())) {
        return None;
    }
    if parts[1..].iter().any(|p| p.len() > 2) {
        return None;
    }
    let n: Vec<u64> = parts.iter().map(|p| p.parse::<u64>().ok()).collect::<Option<_>>()?;
    Some(match n.as_slice() {
        [m, s] => m.saturating_mul(60).saturating_add(*s),
        [h, m, s] => h.saturating_mul(3600).saturating_add(m.saturating_mul(60)).saturating_add(*s),
        _ => return None,
    })
}

/// A countdown time as shown: "4:59", "1:02:03". Partial seconds round UP (the drawing's `ceil`), so a countdown
/// shows "0:01" until it really reaches zero.
pub fn format_countdown(left: Duration) -> String {
    let mut sec = left.as_secs();
    if left.subsec_nanos() > 0 {
        sec += 1;
    }
    let (h, m, s) = (sec / 3600, (sec / 60) % 60, sec % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// A stopwatch time as shown: "m:ss.cc" ("0:07.42"), with hours "h:mm:ss.cc". Hundredths are cut, not rounded.
pub fn format_stopwatch(t: Duration) -> String {
    let cs = t.subsec_millis() / 10;
    let s = t.as_secs();
    let (h, m) = (s / 3600, (s / 60) % 60);
    if h > 0 {
        format!("{h}:{m:02}:{:02}.{cs:02}", s % 60)
    } else {
        format!("{m}:{:02}.{cs:02}", s % 60)
    }
}

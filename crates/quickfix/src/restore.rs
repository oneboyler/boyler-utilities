//! Row 4 — **Make a restore point** (admin). Only when the user presses it — never automatic (the owner's words).
//!
//! Windows' rule (research §2, Microsoft Learn): one restore point per `SystemRestorePointCreationFrequency` minutes
//! (value missing = 1440 = 24 h). Inside that window `SRSetRestorePointW` still says "yes" but makes NOTHING. So the
//! newest point is read before (no call when it is too soon) and after (did Windows really make one?), and the user
//! sees what happened: "Windows allows one restore point every 24 hours · the last one is from 2 Oct 2026, 18:04 ·
//! the next one from 3 Oct 2026, 18:04". The frequency value itself is never changed (**unclear** in DESIGN whether to
//! set it to 0 — not decided, not built).

use crate::os::{CreateCall, FixOs, LocalTime, RestorePoint, RestoreStatus, Stamp};
use crate::{FixError, Result};

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// "7 Oct 2026".
pub fn format_day(t: LocalTime) -> String {
    let m = MONTHS.get((t.month as usize).wrapping_sub(1)).copied().unwrap_or("?");
    format!("{} {} {}", t.day, m, t.year)
}

/// "2 Oct 2026, 18:04".
pub fn format_when(t: LocalTime) -> String {
    format!("{}, {:02}:{:02}", format_day(t), t.hour, t.minute)
}

/// "today, 21:37" when `t` is on `now`'s day, else "2 Oct 2026, 18:04".
pub fn format_relative(t: LocalTime, now: LocalTime) -> String {
    if (t.year, t.month, t.day) == (now.year, now.month, now.day) {
        format!("today, {:02}:{:02}", t.hour, t.minute)
    } else {
        format_when(t)
    }
}

/// Days since 1970-01-01 of a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// WMI's CIM datetime "yyyymmddHHMMSS.mmmmmmsUUU" (local time + offset in minutes) → a UTC [`Stamp`].
pub fn parse_cim_datetime(s: &str) -> Option<Stamp> {
    let b = s.as_bytes();
    if b.len() < 25 || b[14] != b'.' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (num(0..4)?, num(4..6)?, num(6..8)?, num(8..10)?, num(10..12)?, num(12..14)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let sign = match b[21] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let offset_min = sign * num(22..25)?;
    let local = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se;
    Some(Stamp(local - offset_min * 60))
}

/// What the row shows under the title: "Only when you press it · last one 2 Oct 2026, 18:04".
pub fn status_line(os: &dyn FixOs, st: &RestoreStatus) -> String {
    match (&st.newest, st.newest_known) {
        (Some(p), _) => format!("Only when you press it · last one {}", format_when(os.local(p.created))),
        (None, true) => "Only when you press it · none yet".into(),
        (None, false) => "Only when you press it".into(),
    }
}

/// How pressing [Create] ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreOutcome {
    /// A new point exists now.
    Made { at: LocalTime, description: String },
    /// Windows' rule: too soon after the last point — nothing was made.
    TooSoon { last: LocalTime, next_allowed: LocalTime, frequency_minutes: u32 },
    /// Windows accepted the call but the list can't be read to confirm (shouldn't happen when elevated).
    Accepted { description: String },
    /// System Protection is off (DESIGN: "Turn it on" link, not drawn).
    ProtectionOff,
}

impl RestoreOutcome {
    /// The row's end text. `now` decides "today".
    pub fn line(&self, now: LocalTime) -> String {
        match self {
            RestoreOutcome::Made { at, description } => {
                format!("✓ Made {} · \u{201c}{description}\u{201d}", format_relative(*at, now))
            }
            RestoreOutcome::TooSoon { last, next_allowed, frequency_minutes } => format!(
                "Windows allows one restore point every {} · the last one is from {} · the next one from {}",
                every(*frequency_minutes),
                format_when(*last),
                format_when(*next_allowed)
            ),
            RestoreOutcome::Accepted { description } => format!("Windows accepted \u{201c}{description}\u{201d}"),
            RestoreOutcome::ProtectionOff => "System Protection is off".into(),
        }
    }
}

fn every(minutes: u32) -> String {
    match minutes {
        1440 => "24 hours".into(),
        60 => "hour".into(),
        m if m % 60 == 0 => format!("{} hours", m / 60),
        1 => "minute".into(),
        m => format!("{m} minutes"),
    }
}

fn too_soon(os: &dyn FixOs, p: &RestorePoint, freq: u32, now: Stamp) -> Option<RestoreOutcome> {
    let next = Stamp(p.created.0 + i64::from(freq) * 60);
    (freq > 0 && now < next).then(|| RestoreOutcome::TooSoon {
        last: os.local(p.created),
        next_allowed: os.local(next),
        frequency_minutes: freq,
    })
}

/// The description Windows stores: "Boyler Utilities · 7 Oct 2026".
pub fn description(now: LocalTime) -> String {
    format!("Boyler Utilities · {}", format_day(now))
}

/// [Create]: makes a restore point now — or says plainly why Windows won't. Needs admin.
pub fn make_restore_point(os: &dyn FixOs) -> Result<RestoreOutcome> {
    if !os.is_elevated() {
        return Err(FixError::NeedsAdmin("making a restore point".into()));
    }
    let before = os.restore_status()?;
    let now = os.now();
    if let Some(p) = &before.newest {
        if let Some(o) = too_soon(os, p, before.frequency_minutes, now) {
            return Ok(o); // Windows would accept the call and make nothing — don't pretend
        }
    }
    let desc = description(os.local(now));
    if os.create_restore_point(&desc)? == CreateCall::ProtectionOff {
        return Ok(RestoreOutcome::ProtectionOff);
    }
    let after = os.restore_status()?;
    let newer = match (&after.newest, &before.newest) {
        (Some(a), Some(b)) => a.sequence != b.sequence || a.created > b.created,
        (Some(_), None) if before.newest_known => true,
        // the list was unreadable before: only a point stamped since we asked (1 min slack for clock rounding) is ours
        (Some(a), None) => a.created.0 >= now.0 - 60,
        _ => false,
    };
    match &after.newest {
        Some(a) if newer => Ok(RestoreOutcome::Made { at: os.local(a.created), description: a.description.clone() }),
        Some(a) => Ok(too_soon(os, a, after.frequency_minutes, now).unwrap_or(RestoreOutcome::Accepted { description: desc })),
        None => Ok(RestoreOutcome::Accepted { description: desc }),
    }
}

//! What the Activity tab shows, worked out from the counter (DESIGN §3.18, menu-v18 wording).

use crate::activity::Activity;
use crate::clock::{day_label, weekday, Date, Stamp, MONTHS};
use crate::store::Store;

/// The list shows this many, then "Show all N".
pub const MOST_USED_SHOWN: usize = 6;

/// "6 h 18 m", "42 m", "0 m".
pub fn fmt_hm(ms: u64) -> String {
    let m = ms / 60_000;
    if m >= 60 {
        format!("{} h {} m", m / 60, m % 60)
    } else {
        format!("{m} m")
    }
}

/// Uptime: "1 d 3 h" from a day on, else like [`fmt_hm`].
pub fn fmt_uptime(ms: u64) -> String {
    let h = ms / 3_600_000;
    if h >= 24 {
        format!("{} d {} h", h / 24, h % 24)
    } else {
        fmt_hm(ms)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseRow {
    pub path: String,
    pub name: String,
    pub ms: u64,
    /// The teal "Game" tag / bar.
    pub game: bool,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayCol {
    pub day: i64,
    pub label: String,
    pub total_ms: u64,
    /// Teal, at the bottom of the column.
    pub games_ms: u64,
    pub today: bool,
    /// "Sat 3 Oct · 8 h 20 m · games 4 h 10 m"
    pub hover: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub on: bool,
    /// "Most-used apps, game time and uptime · off" / "Counting since Wed 30 Sep"
    pub status: String,
    pub screen_today_ms: u64,
    /// "6 h 18 m · today · since 11:24"
    pub screen_text: String,
    pub games_today_ms: u64,
    pub games_week_ms: u64,
    /// "2 h 41 m · today · 17 h 31 m in 7 days"
    pub games_text: String,
    pub uptime_ms: u64,
    /// "1 d 3 h · since Mon 18:02"
    pub uptime_text: String,
    /// Most used today / in 7 days, most first (show 6, then "Show all N").
    pub today: Vec<UseRow>,
    pub week: Vec<UseRow>,
    /// Oldest first, today last.
    pub last7: Vec<DayCol>,
}

/// The always-shown line under the card.
pub const PRIVACY_LINE: &str = "Counts only which app is in front — nothing leaves your PC.";

fn hhmm(minute: u32) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

/// "Mon 18:02" (within the last week) or "30 Sep 18:02".
fn moment(at: Stamp, now: Stamp) -> String {
    let d = at.day();
    if now.day() - d < 7 {
        format!("{} {}", weekday(d), hhmm(at.local_minute()))
    } else {
        let dt = Date::from_day(d);
        format!("{} {} {}", dt.d, MONTHS[(dt.m - 1) as usize], hhmm(at.local_minute()))
    }
}

pub fn summary<S: Store>(a: &mut Activity<S>, now: Stamp, uptime_ms: u64) -> Summary {
    let days = a.days_until(now);
    let counted = |p: &str| a.is_counted(p);
    let mut week_apps: std::collections::BTreeMap<String, (String, u64)> = Default::default();
    let mut last7 = Vec::new();
    let mut games_week = 0u64;
    let today_n = now.day();
    let mut today_rows = Vec::new();
    let mut screen_today = 0u64;
    let mut games_today = 0u64;
    let mut first_today = None;
    for (d, data) in &days {
        let (mut total, mut games) = (0u64, 0u64);
        for (path, app) in &data.apps {
            if !counted(path) {
                continue;
            }
            total += app.ms;
            let g = a.rules.is_game(path);
            if g {
                games += app.ms;
            }
            let e = week_apps.entry(path.clone()).or_insert_with(|| (app.name.clone(), 0));
            e.0 = app.name.clone();
            e.1 += app.ms;
            if *d == today_n {
                today_rows.push(row(path, &app.name, app.ms, g));
            }
        }
        games_week += games;
        if *d == today_n {
            screen_today = total;
            games_today = games;
            first_today = data.first_minute;
        }
        let label = day_label(*d);
        last7.push(DayCol {
            day: *d,
            hover: format!("{label} · {} · games {}", fmt_hm(total), fmt_hm(games)),
            label,
            total_ms: total,
            games_ms: games,
            today: *d == today_n,
        });
    }
    let mut week: Vec<UseRow> = week_apps.into_iter().map(|(p, (n, ms))| row(&p, &n, ms, a.rules.is_game(&p))).collect();
    let order = |v: &mut Vec<UseRow>| v.sort_by(|x, y| y.ms.cmp(&x.ms).then_with(|| x.name.cmp(&y.name)));
    order(&mut week);
    order(&mut today_rows);
    let on = a.is_on();
    let status = match (on, a.settings().since) {
        (true, Some(s)) => format!("Counting since {}", day_label(s.to_day())),
        (true, None) => "Counting".into(),
        (false, _) => "Most-used apps, game time and uptime · off".into(),
    };
    let boot = now.plus_ms(-(uptime_ms.min(i64::MAX as u64) as i64));
    Summary {
        on,
        status,
        screen_today_ms: screen_today,
        screen_text: match first_today {
            Some(m) => format!("{} · today · since {}", fmt_hm(screen_today), hhmm(m)),
            None => format!("{} · today", fmt_hm(screen_today)),
        },
        games_today_ms: games_today,
        games_week_ms: games_week,
        games_text: format!("{} · today · {} in 7 days", fmt_hm(games_today), fmt_hm(games_week)),
        uptime_ms,
        uptime_text: format!("{} · since {}", fmt_uptime(uptime_ms), moment(boot, now)),
        today: today_rows,
        week,
        last7,
    }
}

fn row(path: &str, name: &str, ms: u64, game: bool) -> UseRow {
    UseRow { path: path.into(), name: name.into(), ms, game, text: fmt_hm(ms) }
}

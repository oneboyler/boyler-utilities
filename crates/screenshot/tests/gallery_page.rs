//! Order 019: what the Screenshots page asks of the engine besides Order 006's - caption times, the default folder and the
//! reset line's "forget the chosen folder". Fake OS only.

use std::path::{Path, PathBuf};

use bu_screenshot::fake::FakeOs;
use bu_screenshot::naming::{caption, civil, days_from_civil, LocalTime};
use bu_screenshot::{Image, ScreenshotOs, Screenshots};

fn lt(year: u16, month: u8, day: u8, hour: u8, minute: u8) -> LocalTime {
    LocalTime { year, month, day, hour, minute, second: 0 }
}

#[test]
fn civil_time_round_trips_known_dates() {
    assert_eq!(days_from_civil(1970, 1, 1), 0);
    assert_eq!(days_from_civil(2000, 3, 1), 11_017);
    // the fake's clock: 2026-10-07 23:36:05 UTC = 2026-10-08 01:36:05 CEST (+120 min, across midnight)
    assert_eq!(civil(1_791_416_165_000, 0), LocalTime { year: 2026, month: 10, day: 7, hour: 23, minute: 36, second: 5 });
    assert_eq!(civil(1_791_416_165_000, 120), LocalTime { year: 2026, month: 10, day: 8, hour: 1, minute: 36, second: 5 });
    assert_eq!(civil(1_791_416_165_000, -120).hour, 21);
    // leap day
    let d = days_from_civil(2028, 2, 29);
    assert_eq!(civil(d as u64 * 86_400_000, 0), lt(2028, 2, 29, 0, 0));
}

#[test]
fn caption_is_the_time_today_then_yesterday_then_the_weekday_then_the_date() {
    let now = lt(2026, 10, 8, 21, 37); // a Thursday
    assert_eq!(caption(&now, &lt(2026, 10, 8, 21, 34)), "21:34");
    assert_eq!(caption(&now, &lt(2026, 10, 8, 0, 5)), "00:05");
    assert_eq!(caption(&now, &lt(2026, 10, 7, 23, 59)), "Yesterday");
    assert_eq!(caption(&now, &lt(2026, 10, 5, 11, 40)), "Mon");
    assert_eq!(caption(&now, &lt(2026, 10, 2, 9, 0)), "Fri");
    assert_eq!(caption(&now, &lt(2026, 10, 1, 9, 0)), "Oct 1");
    assert_eq!(caption(&now, &lt(2025, 12, 31, 9, 0)), "Dec 31, 2025");
    // a clock set back (a shot "from the future") still shows its time
    assert_eq!(caption(&now, &lt(2026, 10, 9, 8, 0)), "08:00");
}

fn engine() -> (FakeOs, Screenshots<FakeOs>) {
    let os = FakeOs::two_monitors();
    let s = Screenshots::new(os.clone(), PathBuf::from(r"C:\scratch\engine"));
    (os, s)
}

#[test]
fn reset_save_dir_goes_back_to_windows_folder_and_keeps_old_shots() {
    let (os, s) = engine();
    let win = s.default_save_dir().unwrap();
    assert_eq!(win, PathBuf::from(r"C:\Users\test\Pictures\Screenshots"));
    os.add_dir(r"D:\Clips");
    s.set_save_dir(Path::new(r"D:\Clips")).unwrap();
    let img = Image { width: 4, height: 2, bgra: vec![200; 32] };
    let shot = s.save(&img).unwrap();
    assert!(shot.path.starts_with(r"D:\Clips"));
    s.reset_save_dir().unwrap();
    assert_eq!(s.saved_dir().unwrap(), None);
    assert_eq!(s.save_dir().unwrap(), win);
    // the older shot stays where it was and stays in the gallery
    assert!(os.exists(&shot.path));
    assert_eq!(s.gallery().unwrap().len(), 1);
    // nothing chosen: resetting writes nothing
    let (os2, s2) = engine();
    s2.reset_save_dir().unwrap();
    assert!(os2.state().files.is_empty());
}

#[test]
fn shot_time_uses_the_os_clock_rules() {
    let (_os, s) = engine();
    let img = Image { width: 2, height: 2, bgra: vec![9; 16] };
    let shot = s.save(&img).unwrap();
    // the fake's rule = UTC: the id is the save time
    assert_eq!(s.shot_time(&shot), civil(shot.id, 0));
    assert_eq!(s.now_local(), lt(2026, 10, 8, 1, 36).with_second(5));
}

trait WithSecond {
    fn with_second(self, s: u8) -> Self;
}
impl WithSecond for LocalTime {
    fn with_second(mut self, s: u8) -> Self {
        self.second = s;
        self
    }
}

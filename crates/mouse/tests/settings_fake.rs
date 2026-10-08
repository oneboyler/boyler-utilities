//! Windows mouse settings against the fake: read, change, exact undo, ranges, the admin path.

use bu_mouse::fake::FakeOs;
use bu_mouse::os::{WinRaw, WinSetting};
use bu_mouse::settings::*;
use bu_mouse::service::UndoValue;
use bu_mouse::{AppDirs, Error, Mouse, UndoKey};

fn m() -> Mouse<FakeOs> {
    Mouse::new(FakeOs::new(), AppDirs::new("unused"))
}

#[test]
fn reads_windows_values() {
    let m = m();
    let w = m.windows_mouse().unwrap();
    assert_eq!(w.pointer_speed, 10);
    assert!(w.precision);
    assert_eq!(w.precision_raw, [6, 10, 1]);
    assert_eq!(w.scroll_lines, ScrollLines::Lines(3));
    assert_eq!(w.double_click_ms, 500);
    assert!(!w.buttons_swapped);
}

#[test]
fn one_screen_scrolling_is_reported_as_such() {
    let mut m = m();
    m.os_mut().win.insert(WinSetting::ScrollLines, WinRaw::Num(u32::MAX));
    assert_eq!(m.windows_mouse().unwrap().scroll_lines, ScrollLines::OneScreen);
}

#[test]
fn every_setting_changes_and_undoes_exactly() {
    let mut m = m();
    m.set_pointer_speed(14).unwrap();
    m.set_scroll_lines(7).unwrap();
    m.set_double_click_step(14).unwrap();
    assert_eq!(m.set_buttons_swapped(true).unwrap(), "Right button is now your main button");
    let w = m.windows_mouse().unwrap();
    assert_eq!((w.pointer_speed, w.double_click_ms, w.buttons_swapped), (14, 200, true));
    assert_eq!(w.scroll_lines, ScrollLines::Lines(7));
    for s in [WinSetting::PointerSpeed, WinSetting::ScrollLines, WinSetting::DoubleClick, WinSetting::SwapButtons] {
        assert!(m.can_undo(&UndoKey::Windows(s)));
        m.undo_windows(s).unwrap();
    }
    let w = m.windows_mouse().unwrap();
    assert_eq!((w.pointer_speed, w.double_click_ms, w.buttons_swapped), (10, 500, false));
    assert_eq!(w.scroll_lines, ScrollLines::Lines(3));
    assert!(matches!(m.undo_windows(WinSetting::PointerSpeed), Err(Error::NothingToUndo(_))));
}

#[test]
fn precision_off_and_back_restores_the_users_own_thresholds() {
    let mut m = m();
    // a user with non-default thresholds (set by another tool) — undo must put exactly these back
    m.os_mut().win.insert(WinSetting::Precision, WinRaw::Mouse([4, 12, 2]));
    m.set_precision(false).unwrap();
    assert_eq!(m.os().win[&WinSetting::Precision], WinRaw::Mouse(PRECISION_OFF));
    m.undo_windows(WinSetting::Precision).unwrap();
    assert_eq!(m.os().win[&WinSetting::Precision], WinRaw::Mouse([4, 12, 2]));
    // turning it on from off uses Windows' own 6/10/1
    m.os_mut().win.insert(WinSetting::Precision, WinRaw::Mouse([0, 0, 0]));
    m.set_precision(true).unwrap();
    assert_eq!(m.os().win[&WinSetting::Precision], WinRaw::Mouse(PRECISION_ON));
}

#[test]
fn no_write_when_nothing_changes() {
    let mut m = m();
    m.set_pointer_speed(10).unwrap();
    m.set_precision(true).unwrap();
    assert!(m.os().log.is_empty());
}

#[test]
fn ranges_are_enforced() {
    let mut m = m();
    assert!(matches!(m.set_pointer_speed(0), Err(Error::OutOfRange { .. })));
    assert!(matches!(m.set_pointer_speed(21), Err(Error::OutOfRange { .. })));
    assert!(matches!(m.set_scroll_lines(0), Err(Error::OutOfRange { .. })));
    assert!(matches!(m.set_scroll_lines(101), Err(Error::OutOfRange { .. })));
    assert!(matches!(m.set_double_click_step(15), Err(Error::OutOfRange { .. })));
    assert!(m.os().log.is_empty());
}

#[test]
fn double_click_slider_steps() {
    assert_eq!(double_click_ms_for_step(0), 900);
    assert_eq!(double_click_ms_for_step(8), 500);
    assert_eq!(double_click_ms_for_step(14), 200);
    assert_eq!(double_click_step_for_ms(500), 8);
    assert_eq!(double_click_step_for_ms(333), 11, "nearest step to a value set elsewhere (350 ms)");
    assert_eq!(double_click_step_for_ms(5000), 0);
}

#[test]
fn access_denied_is_needs_admin_and_nothing_is_remembered() {
    let mut m = m();
    m.os_mut().deny_writes = 1;
    assert!(matches!(m.set_pointer_speed(12), Err(Error::NeedsAdmin { .. })));
    assert!(!m.can_undo(&UndoKey::Windows(WinSetting::PointerSpeed)));
    assert_eq!(m.windows_mouse().unwrap().pointer_speed, 10);
}

#[test]
fn swap_toasts() {
    assert_eq!(swap_toast(false), "Left button is your main button again");
}

#[test]
fn undo_entries_list_every_change_with_its_old_value_in_order() {
    let mut m = m();
    assert!(m.undo_entries().is_empty());
    m.set_buttons_swapped(true).unwrap();
    m.set_pointer_speed(14).unwrap();
    m.set_pointer_speed(16).unwrap();
    let e = m.undo_entries();
    // pointer speed first (Windows' order) with the value from before its last change (undo steps back once); swap after it
    assert_eq!(e.len(), 2);
    assert_eq!(e[0], (UndoKey::Windows(WinSetting::PointerSpeed), UndoValue::Windows(WinRaw::Num(14))));
    assert_eq!(e[1], (UndoKey::Windows(WinSetting::SwapButtons), UndoValue::Windows(WinRaw::Bool(false))));
    m.undo_windows(WinSetting::SwapButtons).unwrap();
    assert_eq!(m.undo_entries().len(), 1);
}

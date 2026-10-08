//! Mouse settings — Windows' own (DESIGN §3.4 "Mouse settings"): pointer speed, Enhance pointer precision, scroll lines,
//! double-click speed, swap primary button. Read live from Windows; every change remembers the exact old value (undo).
//! How: `SystemParametersInfo` (SPI_SETMOUSESPEED, SPI_SETMOUSE, SPI_SETWHEELSCROLLLINES, SPI_SETDOUBLECLICKTIME,
//! SPI_SETMOUSEBUTTONSWAP) — no admin, live, persisted for the user.

use crate::error::{Error, Result};
use crate::os::{MouseOs, WinRaw, WinSetting};
use crate::service::{Mouse, UndoKey, UndoValue};

/// EPP on, as Windows writes it ({threshold1, threshold2, acceleration}).
pub const PRECISION_ON: [i32; 3] = [6, 10, 1];
/// EPP off.
pub const PRECISION_OFF: [i32; 3] = [0, 0, 0];
/// `SPI_GETWHEELSCROLLLINES` value for "one screen at a time".
pub const WHEEL_PAGESCROLL: u32 = u32::MAX;

/// Pointer speed slider 1–20 (Windows default 10).
pub const POINTER_SPEED_RANGE: (u32, u32) = (1, 20);
/// Scroll lines slider 1–100 (default 3).
pub const SCROLL_LINES_RANGE: (u32, u32) = (1, 100);
/// Double-click slider: 900 → 200 ms in 15 steps (50 ms apart); right = faster; Windows default 500 ms.
pub const DOUBLE_CLICK_STEPS: usize = 15;
pub const DOUBLE_CLICK_SLOWEST_MS: u32 = 900;
pub const DOUBLE_CLICK_STEP_MS: u32 = 50;

/// Scroll lines as Windows holds them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollLines {
    Lines(u32),
    /// "One screen at a time" (set in Control Panel; outside the slider)
    OneScreen,
}

/// What the "Mouse settings" group shows — Windows' current values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowsMouse {
    pub pointer_speed: u32,
    /// Enhance pointer precision (acceleration "6/10/1" or any non-zero acceleration value)
    pub precision: bool,
    /// the exact three ints, kept so they can be put back exactly
    pub precision_raw: [i32; 3],
    pub scroll_lines: ScrollLines,
    pub double_click_ms: u32,
    pub buttons_swapped: bool,
}

/// Double-click time of slider step `i` (0 = slowest 900 ms … 14 = fastest 200 ms).
pub fn double_click_ms_for_step(i: usize) -> u32 {
    let i = i.min(DOUBLE_CLICK_STEPS - 1) as u32;
    DOUBLE_CLICK_SLOWEST_MS - i * DOUBLE_CLICK_STEP_MS
}

/// The slider step nearest to a time Windows reports (any value, e.g. 333 ms set elsewhere, maps to the nearest step;
/// the label still shows Windows' exact value).
pub fn double_click_step_for_ms(ms: u32) -> usize {
    (0..DOUBLE_CLICK_STEPS).min_by_key(|i| double_click_ms_for_step(*i).abs_diff(ms)).unwrap_or(0)
}

/// Toast text for the swap switch (DESIGN).
pub fn swap_toast(swapped: bool) -> &'static str {
    if swapped {
        "Right button is now your main button"
    } else {
        "Left button is your main button again"
    }
}

fn num(s: WinSetting, v: WinRaw) -> Result<u32> {
    match v {
        WinRaw::Num(n) => Ok(n),
        other => Err(Error::os(format!("{s:?}: unexpected value {other:?}"), 0)),
    }
}

impl<O: MouseOs> Mouse<O> {
    /// Windows' current mouse settings.
    pub fn windows_mouse(&self) -> Result<WindowsMouse> {
        let pointer_speed = num(WinSetting::PointerSpeed, self.os.win_get(WinSetting::PointerSpeed)?)?;
        let precision_raw = match self.os.win_get(WinSetting::Precision)? {
            WinRaw::Mouse(m) => m,
            other => return Err(Error::os(format!("Precision: unexpected value {other:?}"), 0)),
        };
        let lines = num(WinSetting::ScrollLines, self.os.win_get(WinSetting::ScrollLines)?)?;
        let double_click_ms = num(WinSetting::DoubleClick, self.os.win_get(WinSetting::DoubleClick)?)?;
        let buttons_swapped = match self.os.win_get(WinSetting::SwapButtons)? {
            WinRaw::Bool(b) => b,
            other => return Err(Error::os(format!("SwapButtons: unexpected value {other:?}"), 0)),
        };
        Ok(WindowsMouse {
            pointer_speed,
            // Windows treats the third int (acceleration level) as the switch: 0 = off.
            precision: precision_raw[2] != 0,
            precision_raw,
            scroll_lines: if lines == WHEEL_PAGESCROLL { ScrollLines::OneScreen } else { ScrollLines::Lines(lines) },
            double_click_ms,
            buttons_swapped,
        })
    }

    fn win_change(&mut self, s: WinSetting, new: WinRaw) -> Result<()> {
        let old = self.os.win_get(s)?;
        if old == new {
            return Ok(());
        }
        self.os.win_set(s, new)?;
        self.remember(UndoKey::Windows(s), UndoValue::Windows(old));
        Ok(())
    }

    /// Pointer speed 1–20.
    pub fn set_pointer_speed(&mut self, speed: u32) -> Result<()> {
        let (lo, hi) = POINTER_SPEED_RANGE;
        if !(lo..=hi).contains(&speed) {
            return Err(Error::range("pointer speed", format!("{speed} is outside {lo}–{hi}")));
        }
        self.win_change(WinSetting::PointerSpeed, WinRaw::Num(speed))
    }

    /// Enhance pointer precision on ({6,10,1}, Windows' own values) or off ({0,0,0}). Also the EPP warning's "Turn it off".
    pub fn set_precision(&mut self, on: bool) -> Result<()> {
        let cur = self.windows_mouse()?;
        if cur.precision == on {
            return Ok(());
        }
        self.win_change(WinSetting::Precision, WinRaw::Mouse(if on { PRECISION_ON } else { PRECISION_OFF }))
    }

    /// Scroll lines 1–100 per wheel notch.
    pub fn set_scroll_lines(&mut self, lines: u32) -> Result<()> {
        let (lo, hi) = SCROLL_LINES_RANGE;
        if !(lo..=hi).contains(&lines) {
            return Err(Error::range("scroll lines", format!("{lines} is outside {lo}–{hi}")));
        }
        self.win_change(WinSetting::ScrollLines, WinRaw::Num(lines))
    }

    /// Double-click speed by slider step (0 = 900 ms … 14 = 200 ms).
    pub fn set_double_click_step(&mut self, step: usize) -> Result<()> {
        if step >= DOUBLE_CLICK_STEPS {
            return Err(Error::range("double-click speed", format!("step {step} is outside 0–{}", DOUBLE_CLICK_STEPS - 1)));
        }
        self.win_change(WinSetting::DoubleClick, WinRaw::Num(double_click_ms_for_step(step)))
    }

    /// Swap primary button. Returns the toast text.
    pub fn set_buttons_swapped(&mut self, swapped: bool) -> Result<&'static str> {
        self.win_change(WinSetting::SwapButtons, WinRaw::Bool(swapped))?;
        Ok(swap_toast(swapped))
    }

    /// Puts one Windows mouse setting to an exact value it had (the app's change log, Order 036: "Back to how your PC
    /// was" / "Windows defaults" with no earlier state). Nothing is written when it already has it.
    pub fn restore_windows(&mut self, s: WinSetting, v: WinRaw) -> Result<()> {
        if self.os.win_get(s)? == v {
            return Ok(());
        }
        self.os.win_set(s, v)
    }

    /// Puts one Windows mouse setting back exactly as it was before the last change made here.
    pub fn undo_windows(&mut self, s: WinSetting) -> Result<()> {
        match self.take_undo(&UndoKey::Windows(s)) {
            Some(UndoValue::Windows(old)) => self.os.win_set(s, old),
            _ => Err(Error::NothingToUndo(format!("{s:?}"))),
        }
    }
}

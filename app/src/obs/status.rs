//! The optional status icon (status.c): small tiles in the very bottom-right corner of a chosen monitor while instant
//! replay and / or recording is on. Same window rules as the popups (topmost, click-through, never takes focus, no taskbar
//! button, layered, out of screen capture). Redrawn only when the state or the monitors change: no timers.

use std::cell::RefCell;

use bu_obs::monitors::{self, Mon};
use bu_obs::settings::{Settings, SI_MON1, SI_MON2, SI_OTHER, SI_REC_MON, W_MON1, W_MON2, W_NONE, W_OTHER, W_SAME};
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{GetDC, ReleaseDC, AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::gdi::{self, Dib};

#[derive(Default, PartialEq, Clone)]
struct Last {
    shown: bool,
    mon: Option<usize>,
    replay: bool,
    rec: bool,
    rc: (i32, i32, i32, i32),
    dpi: i32,
}

#[derive(Default)]
struct State {
    hwnd: Option<HWND>,
    last: Option<Last>,
    /// test copies: what would be shown
    pub note: String,
}

thread_local! {
    static S: RefCell<State> = RefCell::new(State::default());
}

unsafe extern "system" fn proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match m {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => DefWindowProcW(h, m, wp, lp),
    }
}

fn where_of(status: i32) -> i32 {
    match status {
        SI_REC_MON => W_SAME,
        SI_OTHER => W_OTHER,
        SI_MON1 => W_MON1,
        SI_MON2 => W_MON2,
        _ => W_NONE, // SI_OFF
    }
}

/// What a test copy's status icon shows ("" = nothing).
pub fn note() -> String {
    S.with(|s| s.borrow().note.clone())
}

/// Called whenever the state may have changed; does nothing unless something visible changed.
pub fn update(set: &Settings, mons: &[Mon], connected: bool, replay: bool, rec: bool, clipped: Option<usize>, hidden: bool) {
    let wh = where_of(set.status);
    let mon = if wh != W_NONE && connected && (replay || rec) { monitors::pick_monitor(mons, wh, clipped) } else { None };
    let shown = mon.is_some();
    let (replay, rec) = (shown && replay, shown && rec);
    S.with(|s| {
        let Ok(mut st) = s.try_borrow_mut() else { return };
        let dpi = st.hwnd.map(|h| unsafe { GetDpiForWindow(h) } as i32).unwrap_or(0);
        let rc = mon.map(|m| (mons[m].rc.left, mons[m].rc.top, mons[m].rc.right, mons[m].rc.bottom)).unwrap_or_default();
        let now = Last { shown, mon, replay, rec, rc, dpi };
        if st.last.as_ref() == Some(&now) {
            return;
        }
        st.last = Some(now.clone());
        let Some(m) = mon else {
            if let Some(h) = st.hwnd.take() {
                unsafe {
                    let _ = DestroyWindow(h);
                }
            }
            st.note.clear();
            return;
        };
        if hidden {
            st.note = format!("monitor {} tiles {}{}{}", mons[m].num, if rec { "recording" } else { "" }, if rec && replay { "," } else { "" }, if replay { "replay" } else { "" });
            return;
        }
        let r = mons[m].rc;
        let h = match st.hwnd {
            Some(h) => {
                unsafe {
                    let _ = SetWindowPos(h, None, r.right - 2, r.bottom - 2, 1, 1, SWP_NOZORDER | SWP_NOACTIVATE);
                }
                h
            }
            None => unsafe {
                static REG: std::sync::Once = std::sync::Once::new();
                REG.call_once(|| {
                    let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: GetModuleHandleW(None).unwrap_or_default().into(), lpszClassName: w!("BoylerObsStatus"), ..Default::default() };
                    RegisterClassW(&wc);
                });
                let ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
                let Ok(h) = CreateWindowExW(ex, w!("BoylerObsStatus"), w!(""), WS_POPUP, r.right - 2, r.bottom - 2, 1, 1, None, None, Some(GetModuleHandleW(None).unwrap_or_default().into()), None) else { return };
                super::popups::no_capture(h);
                st.hwnd = Some(h);
                h
            },
        };
        let dpi = (unsafe { GetDpiForWindow(h) } as i32).max(96);
        if let Some(l) = st.last.as_mut() {
            l.dpi = unsafe { GetDpiForWindow(h) } as i32;
        }
        let img = gdi::status_draw(replay, rec, dpi);
        let x = r.right - gdi::muldiv(gdi::S_MARGIN, dpi, 96) - img.w;
        let y = r.bottom - gdi::muldiv(gdi::S_MARGIN, dpi, 96) - img.h;
        let Some(mut dib) = Dib::new(img.w, img.h) else { return };
        dib.px().copy_from_slice(&img.px);
        let bf = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
        unsafe {
            let sdc = GetDC(None);
            let _ = UpdateLayeredWindow(h, Some(sdc), Some(&POINT { x, y }), Some(&SIZE { cx: img.w, cy: img.h }), Some(dib.dc), Some(&POINT::default()), COLORREF(0), Some(&bf), ULW_ALPHA);
            ReleaseDC(None, sdc);
            let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(h, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    });
}

/// The icon goes (feature off).
pub fn hide() {
    S.with(|s| {
        if let Ok(mut st) = s.try_borrow_mut() {
            if let Some(h) = st.hwnd.take() {
                unsafe {
                    let _ = DestroyWindow(h);
                }
            }
            st.last = None;
            st.note.clear();
        }
    });
}

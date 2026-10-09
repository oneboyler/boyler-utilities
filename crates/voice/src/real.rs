//! The real Windows layer (Order 060): Windows' own voice typing is opened with its shortcut, Win+H, sent with `SendInput` (the
//! documented way - no hook, no injection into any process) and **only while the foreground window belongs to this app**
//! ([`crate::send_if_ours`]): the menu in front, never another app, never a game. Win+H has its own permission in Windows, so
//! there is no "Online speech recognition" switch to read or ask for. Tests never use this layer (they use `fake`);
//! `examples/show.rs` only reads which window is in front.

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_H, VK_LWIN};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

use crate::error::{os, Result, VoiceError};
use crate::{send_if_ours, Clipboard, VoiceTyping};

/// The process that owns the foreground window (None = no foreground window).
pub fn foreground_pid() -> Option<u32> {
    // SAFETY: plain queries; a null window is handled.
    unsafe {
        let h = GetForegroundWindow();
        if h.is_invalid() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        (pid != 0).then_some(pid)
    }
}

/// Is the foreground window one of this app's own?
pub fn foreground_is_ours() -> bool {
    // SAFETY: a plain query.
    foreground_pid() == Some(unsafe { GetCurrentProcessId() })
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() }, time: 0, dwExtraInfo: 0 } },
    }
}

/// Win+H as one `SendInput` call (Win down, H down, H up, Win up).
fn send_win_h() -> Result<()> {
    let seq = [key(VK_LWIN, false), key(VK_H, false), key(VK_H, true), key(VK_LWIN, true)];
    // SAFETY: four well-formed INPUTs.
    let n = unsafe { SendInput(&seq, std::mem::size_of::<INPUT>() as i32) };
    if n as usize == seq.len() {
        Ok(())
    } else {
        Err(VoiceError::Os { context: "SendInput".into(), code: n })
    }
}

/// Windows' voice typing: Win+H to the foreground window when it is ours.
#[derive(Default)]
pub struct WinVoiceTyping;

impl VoiceTyping for WinVoiceTyping {
    fn toggle(&mut self) -> Result<bool> {
        send_if_ours(foreground_is_ours(), send_win_h)
    }
}

/// The Windows clipboard (Copy).
#[derive(Default)]
pub struct WinClipboard;

impl Clipboard for WinClipboard {
    fn set_text(&mut self, text: &str) -> Result<()> {
        let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        unsafe {
            OpenClipboard(Some(HWND::default())).map_err(|e| os("opening the clipboard", e))?;
            let r = (|| -> Result<()> {
                EmptyClipboard().map_err(|e| os("emptying the clipboard", e))?;
                let h = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).map_err(|e| os("clipboard memory", e))?;
                let p = GlobalLock(h) as *mut u16;
                if p.is_null() {
                    let _ = GlobalFree(Some(h));
                    return Err(VoiceError::Os { context: "clipboard memory".into(), code: 0 });
                }
                std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
                let _ = GlobalUnlock(h);
                if let Err(e) = SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(h.0))) {
                    let _ = GlobalFree(Some(HGLOBAL(h.0)));
                    return Err(os("setting the clipboard", e));
                }
                Ok(())
            })();
            let _ = CloseClipboard();
            r
        }
    }
}


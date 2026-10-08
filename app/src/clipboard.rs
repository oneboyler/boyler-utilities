//! Text onto the clipboard (Order 014 item 2, PIECES_WANTED: Performance › Your PC "Copy all", Voice to text's copy).
//! Pages reach it through `Cx::copy_text`. A test copy AND `cargo test` never touch the real clipboard (it is the user's): they
//! keep the text for the test hook / the test instead (Lane V 10:35: a page unit test clicking a copy button).

use std::sync::Mutex;

use windows::core::w;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HWND};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE};

/// What a test copy "copied" (the test hook's `state` shows it as `clip=<chars>`).
static TEST_CLIP: Mutex<Option<String>> = Mutex::new(None);

pub fn test_clip() -> Option<String> {
    TEST_CLIP.lock().ok().and_then(|c| c.clone())
}

/// The text as CF_UNICODETEXT wants it: UTF-16, line breaks as CR LF, ending in a 0.
pub fn unicode_bytes(text: &str) -> Vec<u8> {
    let crlf = text.replace("\r\n", "\n").replace('\n', "\r\n");
    crlf.encode_utf16().chain(std::iter::once(0)).flat_map(|u| u.to_le_bytes()).collect()
}

/// Put `text` on the clipboard (a test copy: kept for the test hook only). Err = why not (another app held it).
pub fn set_text(text: &str) -> Result<(), String> {
    if cfg!(test) || crate::testmode::on() {
        if let Ok(mut c) = TEST_CLIP.lock() {
            *c = Some(text.to_string());
        }
        return Ok(());
    }
    let bytes = unicode_bytes(text);
    unsafe {
        // a hidden message-only window owns the clipboard while we fill it (with no owner, SetClipboardData fails after
        // EmptyClipboard)
        let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, None, None)
            .map_err(|e| format!("CreateWindowExW {e}"))?;
        let r = fill(hwnd, &bytes);
        let _ = DestroyWindow(hwnd);
        r
    }
}

unsafe fn fill(hwnd: HWND, bytes: &[u8]) -> Result<(), String> {
    // another app may hold the clipboard for a moment: try for up to ~0.5 s
    let mut open = false;
    for _ in 0..25 {
        if OpenClipboard(Some(hwnd)).is_ok() {
            open = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    if !open {
        return Err("another app holds the clipboard".into());
    }
    let r = (|| {
        EmptyClipboard().map_err(|e| format!("EmptyClipboard {e}"))?;
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(|e| format!("GlobalAlloc {e}"))?;
        let p = GlobalLock(h);
        if p.is_null() {
            let _ = GlobalFree(Some(h));
            return Err("GlobalLock".to_string());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p as *mut u8, bytes.len());
        let _ = GlobalUnlock(h);
        if let Err(e) = SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(h.0))) {
            let _ = GlobalFree(Some(h)); // still ours when the call failed
            return Err(format!("SetClipboardData {e}"));
        }
        Ok(()) // the clipboard owns the memory now
    })();
    let _ = CloseClipboard();
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_utf16_with_crlf_and_a_final_zero() {
        let b = unicode_bytes("Č\nb");
        let u: Vec<u16> = b.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(u, vec![0x010C, 13, 10, b'b' as u16, 0]);
        assert_eq!(unicode_bytes("a\r\nb"), unicode_bytes("a\nb"));
    }

    /// `cargo test` never writes the real clipboard (testmode is off under cargo test).
    #[test]
    fn a_unit_test_never_touches_the_real_clipboard() {
        assert!(!crate::testmode::on());
        set_text("from a unit test").unwrap();
        assert_eq!(test_clip().as_deref(), Some("from a unit test"));
    }
}

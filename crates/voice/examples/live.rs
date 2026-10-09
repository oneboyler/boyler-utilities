//! Order 060's one real check (run BY HAND, only while the owner is away and no game runs - it shows a small window on screen):
//! a small window of this program takes the foreground, `WinVoiceTyping` sends Win+H to it (the guard lets it through because the
//! foreground window is ours), Windows' voice typing opens (new windows of other processes are listed), the program's window
//! must still be in front (the menu closes when it loses the focus, so this matters), and a second Win+H closes it again.
//! Nobody speaks, nothing is typed, no setting changes; every step is undone by the next. Aborts the moment there is any
//! keyboard / mouse input (GetLastInputInfo) and leaves.

#[cfg(windows)]
fn main() {
    use std::time::{Duration, Instant};
    use windows::core::w;
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EnumWindows, GetClassNameW, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
        PeekMessageW, RegisterClassW, SetForegroundWindow, ShowWindow, TranslateMessage, MSG, PM_REMOVE, SW_SHOW, WM_CHAR, WNDCLASSW, WS_CAPTION, WS_EX_TOPMOST, WS_VISIBLE,
    };

    use bu_voice::VoiceTyping;

    static CHARS: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

    unsafe extern "system" fn proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        if m == WM_CHAR {
            if let Some(c) = char::from_u32(w.0 as u32) {
                CHARS.lock().unwrap().push(c);
            }
        }
        DefWindowProcW(h, m, w, l)
    }

    fn windows_now() -> Vec<(u32, String, String)> {
        unsafe extern "system" fn each(h: HWND, l: LPARAM) -> BOOL {
            let v = &mut *(l.0 as *mut Vec<(u32, String, String)>);
            if IsWindowVisible(h).as_bool() {
                let (mut c, mut t) = ([0u16; 128], [0u16; 128]);
                let (cn, tn) = (GetClassNameW(h, &mut c), GetWindowTextW(h, &mut t));
                let mut pid = 0u32;
                GetWindowThreadProcessId(h, Some(&mut pid));
                v.push((pid, String::from_utf16_lossy(&c[..cn.max(0) as usize]), String::from_utf16_lossy(&t[..tn.max(0) as usize])));
            }
            true.into()
        }
        let mut v: Vec<(u32, String, String)> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(each), LPARAM(&mut v as *mut _ as isize));
        }
        v
    }

    fn tick() -> u32 {
        unsafe { windows::Win32::System::SystemInformation::GetTickCount() }
    }

    fn last_input() -> u32 {
        let mut i = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        unsafe {
            let _ = GetLastInputInfo(&mut i);
        }
        i.dwTime
    }

    fn idle_ms() -> u32 {
        let mut i = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        unsafe {
            let _ = GetLastInputInfo(&mut i);
            windows::Win32::System::SystemInformation::GetTickCount().wrapping_sub(i.dwTime)
        }
    }

    // only while he is away: no input for 2 minutes
    if idle_ms() < 120_000 {
        println!("not run: there was keyboard / mouse input in the last 2 minutes ({} s ago)", idle_ms() / 1000);
        return;
    }
    unsafe {
        let inst = GetModuleHandleW(None).unwrap();
        let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: inst.into(), lpszClassName: w!("BoylerVoiceLive"), ..Default::default() };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(WS_EX_TOPMOST, w!("BoylerVoiceLive"), w!("Boyler voice check"), WS_CAPTION | WS_VISIBLE, 200, 200, 420, 140, None, None, Some(inst.into()), None).unwrap();
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        // the last moment THIS program sent its own keys (its Win+H counts as input too): anything newer is his
        let mine = std::cell::Cell::new(tick());
        let opened = std::cell::Cell::new(false);
        let pump = |ms: u64| -> bool {
            let t0 = Instant::now();
            while t0.elapsed() < Duration::from_millis(ms) {
                let mut m = MSG::default();
                while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&m);
                    DispatchMessageW(&m);
                }
                if last_input().wrapping_sub(mine.get()) < 0x8000_0000 && last_input().wrapping_sub(mine.get()) > 50 {
                    return false; // someone touched the keyboard / mouse
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            true
        };
        let ok = (|| {
            if !pump(600) {
                return false;
            }
            let ours = bu_voice::real::foreground_is_ours();
            println!("our window in front before: {ours} (the guard sends Win+H only if true)");
            let before = windows_now();
            let mut vt = bu_voice::real::WinVoiceTyping;
            mine.set(tick());
            let r = vt.toggle();
            opened.set(r == Ok(true));
            println!("toggle 1 (open): {r:?}");
            mine.set(tick());
            if !pump(3000) {
                return false;
            }
            let mid = windows_now();
            println!("our window still in front after Windows' voice typing opened: {}", bu_voice::real::foreground_is_ours());
            println!("foreground window's class now: {}", {
                let h = GetForegroundWindow();
                let mut c = [0u16; 128];
                let n = GetClassNameW(h, &mut c);
                String::from_utf16_lossy(&c[..n.max(0) as usize])
            });
            println!("new visible windows while it is open:");
            for w in mid.iter().filter(|w| !before.contains(w)) {
                println!("  pid {} class {:?} title {:?}", w.0, w.1, w.2);
            }
            mine.set(tick());
            println!("toggle 2 (close): {:?}", vt.toggle());
            opened.set(false);
            mine.set(tick());
            if !pump(2000) {
                return false;
            }
            let after = windows_now();
            println!("windows left over from voice typing after the 2nd toggle:");
            for w in after.iter().filter(|w| !before.contains(w)) {
                println!("  pid {} class {:?} title {:?}", w.0, w.1, w.2);
            }
            println!("characters our window received: {:?} (nobody spoke - expected none)", CHARS.lock().unwrap());
            true
        })();
        if !ok {
            // (the guard sends it only if our window is still in front)
            let closed = opened.get() && bu_voice::real::WinVoiceTyping.toggle() == Ok(true);
            println!("ABORTED: input arrived; voice typing closed again: {closed} (if it is still open, press Win+H once)");
        }
        let _ = DestroyWindow(hwnd);
    }
}

#[cfg(not(windows))]
fn main() {}

//! Order 117: how long the overlay's opening grab of every monitor takes (the worker's `capture_all`, the Screenshot key's
//! first step) - 5 grabs, timings only. THE PICTURE NEVER LEAVES MEMORY: no file, no clipboard, nothing printed but sizes
//! and times (`RealOs::capture_proof`: no clipboard / Explorer / Recycle Bin; nothing here writes). Run:
//! `cargo run -p bu-screenshot --release --example screenshot-grabtime [-- <n>]` - with `<n>`, n extra windows are open during
//! the grabs: plain visible top-level windows parked far off every screen (x = -20000), never activated (no focus taken).

#[cfg(windows)]
fn main() {
    use std::time::Instant;

    use bu_screenshot::real::RealOs;
    use bu_screenshot::Screenshots;

    // the app is per-monitor DPI aware (its manifest): the same here
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    }
    let extra: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(0);
    let _wins = (0..extra).filter_map(|i| park_window(i as i32)).collect::<Vec<_>>();
    println!("{} extra windows open", _wins.len());
    let dir = std::env::temp_dir().join("BoylerUtilities-test").join("grabtime");
    let ms = |us: u64| us as f64 / 1000.0;
    for i in 0..5 {
        // a new engine per grab, as the worker makes one per job
        let s = Screenshots::new(RealOs::capture_proof(), &dir);
        let t = Instant::now();
        match s.capture_all() {
            Ok(f) => {
                let c = &f.timing;
                println!(
                    "grab {i}: {:.1} ms total (setup {:.1}, frames {:.1}, readback {:.1}) - {}x{} px, {} monitors, method {}",
                    t.elapsed().as_secs_f64() * 1000.0,
                    ms(c.setup_us),
                    ms(c.frames_us),
                    ms(c.readback_us),
                    f.image.width,
                    f.image.height,
                    f.monitors.len(),
                    s.method().name()
                );
                drop(f);
            }
            Err(e) => println!("grab {i}: failed: {e}"),
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
}

/// A visible top-level window off every screen, shown without taking the focus (`WS_EX_NOACTIVATE`, `SW_SHOWNOACTIVATE`).
#[cfg(windows)]
fn park_window(i: i32) -> Option<windows::Win32::Foundation::HWND> {
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::*;
    unsafe {
        let hwnd = CreateWindowExW(WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW, w!("STATIC"), w!("grabtime"), WS_POPUP, -20000 + i * 10, 0, 400, 300, None, None, None, None).ok()?;
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        Some(hwnd)
    }
}

#[cfg(not(windows))]
fn main() {}

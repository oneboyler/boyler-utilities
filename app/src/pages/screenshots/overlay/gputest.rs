//! Order 051, test copies only: the capture overlay over a made-up picture, so its drawing (GPU or CPU path) can be proven and
//! measured without ever grabbing the screen - bu-screenshot's test pattern (every pixel says where it came from) on made-up
//! monitors, the engine on the read-only OS layer (every capture, file write and clipboard call refused), and a drag made of
//! mouse messages posted to the overlay's own window (never SendInput, never the real pointer).

use bu_screenshot::fake::{mon, pattern};
use bu_screenshot::{CaptureTiming, Frozen, Image, Monitor, Rect};
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*; // (wParam 1 = MK_LBUTTON)

/// One made-up monitor `w` x `h` at (`x`, `y`) (off-screen in a plain test copy) and its frozen picture.
pub fn frozen(x: i32, y: i32, w: u32, h: u32) -> Frozen {
    let mut m: Monitor = mon(x, y, w, h, true, false);
    m.number = 1;
    let p = pattern(&m, 0);
    let img = Image { width: p.width, height: p.height, bgra: p.bgra };
    Frozen { area: Rect::new(x, y, w, h), image: img, monitors: vec![m], color: vec![(0, bu_screenshot::ColorPath::Sdr)], timing: CaptureTiming::default() }
}

fn lp(x: i32, y: i32) -> LPARAM {
    LPARAM((((y as u32 & 0xffff) << 16) | (x as u32 & 0xffff)) as isize)
}

/// A box drawn by dragging from `a` to `b` (window px) over `ms`, the pointer reporting at `hz` (a gaming mouse: 1000):
/// button down, moves, button up - posted from a helper thread, so the overlay's thread handles them as it would real input.
pub fn drag(hwnd: HWND, a: (i32, i32), b: (i32, i32), ms: u32, hz: u32) {
    let h = hwnd.0 as isize;
    std::thread::spawn(move || unsafe {
        let hwnd = HWND(h as *mut _);
        let _ = PostMessageW(Some(hwnd), WM_MOUSEMOVE, WPARAM(0), lp(a.0, a.1));
        let _ = PostMessageW(Some(hwnd), WM_LBUTTONDOWN, WPARAM(1), lp(a.0, a.1));
        let n = (ms as u64 * hz as u64 / 1000).max(1);
        let t0 = std::time::Instant::now();
        for i in 1..=n {
            let due = std::time::Duration::from_micros(i * 1_000_000 / hz as u64);
            // (std's sleep uses a high-resolution timer: no spinning core in the process being measured)
            if let Some(w) = due.checked_sub(t0.elapsed()) {
                std::thread::sleep(w);
            }
            let k = i as f32 / n as f32;
            let (x, y) = ((a.0 as f32 + (b.0 - a.0) as f32 * k).round() as i32, (a.1 as f32 + (b.1 - a.1) as f32 * k).round() as i32);
            let _ = PostMessageW(Some(hwnd), WM_MOUSEMOVE, WPARAM(1), lp(x, y));
        }
        let _ = PostMessageW(Some(hwnd), WM_LBUTTONUP, WPARAM(0), lp(b.0, b.1));
    });
}

/// Order 054: a click on `p` (window px): the pointer slides onto it from 30 px to the left, 1 px per message at 1000 Hz (one
/// coordinate changing, as a hand's last moves do), rests 20 ms, then the button goes down and up 30 ms later - posted from
/// a helper thread like `drag`.
pub fn click(hwnd: HWND, p: (i32, i32)) {
    let h = hwnd.0 as isize;
    std::thread::spawn(move || unsafe {
        let hwnd = HWND(h as *mut _);
        let ms = std::time::Duration::from_millis;
        for x in (p.0 - 30).max(0)..=p.0 {
            let _ = PostMessageW(Some(hwnd), WM_MOUSEMOVE, WPARAM(0), lp(x, p.1));
            std::thread::sleep(ms(1));
        }
        std::thread::sleep(ms(20));
        let _ = PostMessageW(Some(hwnd), WM_LBUTTONDOWN, WPARAM(1), lp(p.0, p.1));
        std::thread::sleep(ms(30));
        let _ = PostMessageW(Some(hwnd), WM_LBUTTONUP, WPARAM(0), lp(p.0, p.1));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_made_up_picture_covers_its_monitor() {
        let f = frozen(-20000, 0, 640, 360);
        assert_eq!((f.image.width, f.image.height), (640, 360));
        assert_eq!(f.image.bgra.len(), 640 * 360 * 4);
        assert_eq!(f.monitors[0].rect, Rect::new(-20000, 0, 640, 360));
        assert!(f.crop(&Rect::new(-19990, 10, 20, 20)).is_ok());
        assert_eq!(lp(3, 2).0, (2 << 16) | 3);
    }
}

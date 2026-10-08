//! Order 048 proof (item 2, "mouse moving"): what a fast mouse costs the app's UI thread, BEFORE and AFTER, on synthetic
//! packets - no Raw Input is registered, the real mouse is never touched (A_048_02).
//!
//! `cargo run --release -p bu-rawin --example rawin-bench -- [seconds]`
//! - a "UI" thread with a message-only window (like the app's message window) counts its wake-ups;
//! - a feeder thread makes 8000 packets a second (a gaming mouse moving the whole time) + 2 clicks a second (down + up);
//! - BEFORE = the old path: Windows posted one WM_INPUT per packet to the UI thread (every move woke it);
//! - AFTER = the new path: the packets go through bu-rawin's hub on the feeder ("raw") thread, the UI thread is woken
//!   once per batch only for packets that matter (buttons / wheel / keys).
//!
//! Reported per thread: wake-ups and CPU cycles (QueryThreadCycleTime) and the CPU time (GetThreadTimes).

use std::sync::atomic::{AtomicIsize, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bu_rawin::hub::{Hub, Posts};
use bu_rawin::RawPacket;
use windows::core::w;
use windows::Win32::Foundation::{FILETIME, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{GetCurrentThread, GetThreadTimes};
use windows::Win32::System::WindowsProgramming::QueryThreadCycleTime;
use windows::Win32::UI::WindowsAndMessaging::*;

const RATE: u64 = 8000;
const WM_PACKET: u32 = WM_APP + 1;
const WM_STOP: u32 = WM_APP + 2;

static WAKES: AtomicU64 = AtomicU64::new(0);
static TAKEN: AtomicU64 = AtomicU64::new(0);
static HUB: std::sync::Mutex<Hub> = std::sync::Mutex::new(Hub::new());

fn cpu(t: HANDLE) -> (u64, f64) {
    let mut cycles = 0u64;
    let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe {
        let _ = QueryThreadCycleTime(t, &mut cycles);
        let _ = GetThreadTimes(t, &mut c, &mut e, &mut k, &mut u);
    }
    let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64 / 10_000.0;
    (cycles, ft(k) + ft(u))
}

extern "system" fn proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match m {
        // BEFORE: one message per packet (the old WM_INPUT on the message window)
        WM_PACKET => {
            WAKES.fetch_add(1, Ordering::Relaxed);
            LRESULT(0)
        }
        // AFTER: the hub's wake-up (WM_RAWKEYS): take the waiting packets
        m if m == WM_APP + 8 => {
            WAKES.fetch_add(1, Ordering::Relaxed);
            let n = HUB.lock().unwrap().take().len();
            TAKEN.fetch_add(n as u64, Ordering::Relaxed);
            LRESULT(0)
        }
        WM_STOP => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(h, m, w, l) },
    }
}

fn run(after: bool, secs: u64) {
    WAKES.store(0, Ordering::Relaxed);
    TAKEN.store(0, Ordering::Relaxed);
    let ui_hwnd = Arc::new(AtomicIsize::new(0));
    let ui_cpu = Arc::new(std::sync::Mutex::new((0u64, 0.0f64)));
    let (h2, c2) = (ui_hwnd.clone(), ui_cpu.clone());
    let ui = std::thread::spawn(move || unsafe {
        let inst = GetModuleHandleW(None).unwrap();
        let class = w!("BoylerUtilities.RawinBench");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(proc), hInstance: inst.into(), lpszClassName: class, ..Default::default() });
        let h = CreateWindowExW(WINDOW_EX_STYLE(0), class, w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(inst.into()), None).unwrap();
        h2.store(h.0 as isize, Ordering::Release);
        let start = cpu(GetCurrentThread());
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
        let end = cpu(GetCurrentThread());
        *c2.lock().unwrap() = (end.0 - start.0, end.1 - start.1);
    });
    while ui_hwnd.load(Ordering::Acquire) == 0 {
        std::thread::sleep(Duration::from_millis(1));
    }
    let hwnd = HWND(ui_hwnd.load(Ordering::Acquire) as *mut _);
    let target = (hwnd.0 as isize, WM_APP + 8);
    *HUB.lock().unwrap() = Hub::new();
    // a mouse key is bound (mouse registered for the keys manager)
    HUB.lock().unwrap().set_keys(false, true, Some(target));
    let feeder_start = cpu(unsafe { GetCurrentThread() });
    let t0 = Instant::now();
    let mut sent = 0u64;
    let mut clicks = 0u64;
    while t0.elapsed() < Duration::from_secs(secs) {
        let due = (t0.elapsed().as_secs_f64() * RATE as f64) as u64;
        // one "batch" = everything due now (Windows hands GetRawInputBuffer what queued up meanwhile)
        let mut out = Posts::default();
        let mut hub = if after { Some(HUB.lock().unwrap()) } else { None };
        while sent < due {
            sent += 1;
            // 2 clicks a second (side button 4): a down, then its up 50 ms later; every other packet is a move
            let p = match sent % (RATE / 2) {
                0 => {
                    clicks += 1;
                    RawPacket::Mouse { buttons: 0x0040 }
                }
                x if x == RATE / 20 => RawPacket::Mouse { buttons: 0x0080 },
                _ => RawPacket::Mouse { buttons: 0 },
            };
            match hub.as_mut() {
                Some(h) => h.feed(p, &mut out),
                None => unsafe {
                    let _ = PostMessageW(Some(hwnd), WM_PACKET, WPARAM(0), LPARAM(0));
                },
            }
        }
        drop(hub);
        if let Some((h, m)) = out.keys {
            unsafe {
                let _ = PostMessageW(Some(HWND(h as *mut _)), m, WPARAM(0), LPARAM(0));
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let feeder_end = cpu(unsafe { GetCurrentThread() });
    std::thread::sleep(Duration::from_millis(200));
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_STOP, WPARAM(0), LPARAM(0));
    }
    ui.join().unwrap();
    let (uc, ut) = *ui_cpu.lock().unwrap();
    let st = HUB.lock().unwrap().stats;
    println!(
        "{}: {} s, {} packets ({} clicks): UI thread woken {} times ({:.1}/s), {:.1} M cycles, {:.1} ms CPU; raw/feeder thread {:.1} M cycles, {:.1} ms CPU{}",
        if after { "AFTER (bu-rawin hub)" } else { "BEFORE (WM_INPUT per packet on the UI thread)" },
        secs,
        sent,
        clicks,
        WAKES.load(Ordering::Relaxed),
        WAKES.load(Ordering::Relaxed) as f64 / secs as f64,
        uc as f64 / 1e6,
        ut,
        (feeder_end.0 - feeder_start.0) as f64 / 1e6,
        feeder_end.1 - feeder_start.1,
        if after { format!("; hub: seen {} forwarded {} wakes {}; packets taken by the UI {}", st.packets_seen, st.packets_forwarded, st.wakes_posted, TAKEN.load(Ordering::Relaxed)) } else { String::new() },
    );
}

fn main() {
    let secs = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(60);
    run(false, secs);
    run(true, secs);
}

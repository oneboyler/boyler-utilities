//! REAL-PC brightness / contrast test (Order 073). Per monitor: reads VCP 0x10 + 0x12 (and, read-only, the picture mode
//! 0xDC and colour preset 0x14), moves each slider alone a few steps through the app's own path (DisplayService ->
//! set_ddc_percent), reads both back after each step, then puts the EXACT starting raw values back and checks them.
//! The only thing changed on the monitor is brightness / contrast for a few seconds.
//!
//!   cargo run -p bu-display --example display-bricon -- <ddc-guard-scratch-folder>

use bu_display::win::WinDisplayOs;
use bu_display::{DisplayOs, DisplayService, MonitorId, Vcp};
use std::path::PathBuf;
use std::time::Duration;
use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::*;

fn hmon(gdi: &str) -> Option<HMONITOR> {
    struct F(String, Option<HMONITOR>);
    unsafe extern "system" fn cb(h: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> windows::core::BOOL {
        let f = unsafe { &mut *(lp.0 as *mut F) };
        let mut mi = MONITORINFOEXW::default();
        mi.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if unsafe { GetMonitorInfoW(h, &mut mi.monitorInfo) }.as_bool() {
            let end = mi.szDevice.iter().position(|c| *c == 0).unwrap_or(32);
            if String::from_utf16_lossy(&mi.szDevice[..end]).eq_ignore_ascii_case(&f.0) {
                f.1 = Some(h);
            }
        }
        windows::core::BOOL(1)
    }
    let mut f = F(gdi.to_string(), None);
    let _ = unsafe { EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut f as *mut F as isize)) };
    f.1
}

/// Raw read of any VCP code (read-only): (current, max) or None.
fn raw(gdi: &str, code: u8) -> Option<(u32, u32)> {
    let h = hmon(gdi)?;
    let mut n = 0u32;
    unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(h, &mut n) }.ok()?;
    let mut pm = vec![PHYSICAL_MONITOR::default(); n as usize];
    unsafe { GetPhysicalMonitorsFromHMONITOR(h, &mut pm) }.ok()?;
    let (mut c, mut m) = (0u32, 0u32);
    let ok = unsafe { GetVCPFeatureAndVCPFeatureReply(pm[0].hPhysicalMonitor, code, None, &mut c, Some(&mut m)) };
    let _ = unsafe { DestroyPhysicalMonitors(&pm) };
    if ok == 0 { None } else { Some((c, m)) }
}

fn show(v: Option<(u32, u32)>) -> String {
    v.map(|(c, m)| format!("{c}/{m}")).unwrap_or_else(|| "no answer".into())
}

fn main() {
    let scratch = std::env::args().nth(1).map(PathBuf::from).expect("pass a scratch folder for the DDC guard");
    let probe = WinDisplayOs::read_only(None);
    let mons = probe.monitors().expect("monitors");
    let mut os = WinDisplayOs::new(Some(scratch.clone())); // raw restore path
    let mut svc = DisplayService::new(WinDisplayOs::new(Some(scratch))); // the app's own path
    for m in &mons {
        let id: &MonitorId = &m.id;
        println!("\n=== monitor {} {:?} ({}x{} @ {:.1} Hz) gdi {}", m.number, m.name, m.current.width, m.current.height, m.current.refresh.hz(), m.gdi_name);
        println!("  picture mode 0xDC: {}   colour preset 0x14: {}   (read-only, raw)", show(raw(&m.gdi_name, 0xDC)), show(raw(&m.gdi_name, 0x14)));
        let (Ok(b0), Ok(c0)) = (os.ddc_get(id, Vcp::Brightness), os.ddc_get(id, Vcp::Contrast)) else {
            println!("  DDC/CI: bri {:?} con {:?} -> not both readable, skipped", os.ddc_get(id, Vcp::Brightness), os.ddc_get(id, Vcp::Contrast));
            continue;
        };
        println!("  START  bri 0x10 = {}/{}   con 0x12 = {}/{}", b0.current, b0.max, c0.current, c0.max);
        println!("  {:<28} | bri 0x10 | con 0x12", "step");
        let read = || -> (String, String) {
            std::thread::sleep(Duration::from_millis(900));
            (show(raw(&m.gdi_name, 0x10)), show(raw(&m.gdi_name, 0x12)))
        };
        for (vcp, name, v0) in [(Vcp::Brightness, "brightness", b0), (Vcp::Contrast, "contrast", c0)] {
            let start_pct = v0.percent() as i32;
            let dir = if start_pct >= 50 { -1 } else { 1 };
            for step in [10, 20] {
                let target = (start_pct + dir * step).clamp(0, 100) as u8;
                let r = svc.set_ddc_percent(id, vcp, target);
                let (b, c) = read();
                println!("  set {name} -> {target:>3} % {:<9}| {:<8} | {}   {:?}", "", b, c, r.err());
            }
            // put the exact starting raw value back and check it
            let _ = os.ddc_set(id, vcp, v0.current);
            let (b, c) = read();
            println!("  restore {name:<18} | {:<8} | {}", b, c);
        }
        let (b1, c1) = (os.ddc_get(id, Vcp::Brightness).ok(), os.ddc_get(id, Vcp::Contrast).ok());
        println!(
            "  END    bri {:?} con {:?}   back exactly as at START: {}",
            b1.map(|v| (v.current, v.max)), c1.map(|v| (v.current, v.max)), b1 == Some(b0) && c1 == Some(c0)
        );
    }
}

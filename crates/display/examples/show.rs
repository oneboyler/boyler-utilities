//! READ-ONLY proof on the real PC: prints every monitor, its real modes (exact DXGI rates next to the whole-Hz GDI
//! list), the applied mode, Windows scaling %, HDR, brightness / contrast over DDC/CI and vibrance. Changes NOTHING.
//!
//!   cargo run -p bu-display --example display-show [-- <ddc-guard-scratch-folder>]

use bu_display::fields::{all_rates, rate_label, rate_menu, rates_for};
use bu_display::win::{modes, WinDisplayOs};
use bu_display::{DisplayOs, DisplayService, Vcp};
use std::path::PathBuf;

fn main() {
    // Per-monitor DPI awareness so the documented GetDpiForMonitor cross-check returns real values.
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let guard = std::env::args().nth(1).map(PathBuf::from);
    let os = WinDisplayOs::read_only(guard); // proof program: every change refused (TECH_RULES)
    let vendors: Vec<_> = match os.monitors() {
        Ok(ms) => ms.iter().map(|m| (m.id.clone(), os.vendor(&m.id))).collect(),
        Err(e) => {
            println!("monitors: ERROR {e}");
            return;
        }
    };
    let mut svc = DisplayService::new(os);
    let mons = svc.monitors().expect("monitors");
    println!("monitors: {}", mons.len());
    for m in &mons {
        println!("\n=== {}   (tooltip: {})", DisplayService::<WinDisplayOs>::selector_label(m), svc.selector_tooltip(m));
        println!("  gdi name     : {}", m.gdi_name);
        println!("  name (EDID)  : {:?}   diagonal: {:?} in", m.name, m.diagonal_inches);
        println!("  main         : {}", m.is_main);
        println!("  rect         : x {} y {}  {} x {}", m.rect.x, m.rect.y, m.rect.width, m.rect.height);
        println!("  native       : {:?}", m.native);
        println!(
            "  applied mode : {} x {} @ {}/{} = {:.4} Hz  scaling {:?}",
            m.current.width, m.current.height, m.current.refresh.num, m.current.refresh.den, m.current.refresh.hz(), m.current.scaling
        );
        println!("  windows scale: {:?}", m.dpi);
        if let Some(h) = bu_display_hmon(&m.gdi_name) {
            let (mut x, mut y) = (0u32, 0u32);
            let ok = unsafe { windows::Win32::UI::HiDpi::GetDpiForMonitor(h, windows::Win32::UI::HiDpi::MDT_EFFECTIVE_DPI, &mut x, &mut y) };
            println!("  cross-check  : GetDpiForMonitor (documented) = {} dpi = {} %  ({:?})", x, x * 100 / 96, ok.is_ok());
        }
        println!("  hdr (read)   : {:?}", m.hdr);
        println!("  gpu vendor   : {:?}", vendors.iter().find(|(id, _)| *id == m.id).and_then(|(_, v)| *v));

        let list = svc.modes(&m.id).unwrap_or_default();
        let dx = modes::list_dxgi(&m.gdi_name);
        let gdi = modes::list_gdi(&m.gdi_name);
        println!("  modes        : {} (source: {}), GDI whole-Hz list has {}", list.len(), if dx.is_some() { "DXGI exact" } else { "GDI" }, gdi.len());
        let all = all_rates(&list);
        println!(
            "  all rates    : {}",
            all.iter().rev().map(|r| format!("{} [{}/{}={:.3}]", rate_label(*r, &all), r.num, r.den, r.hz())).collect::<Vec<_>>().join(", ")
        );
        let cur_rates = rates_for(&list, m.current.width, m.current.height);
        println!(
            "  Hz popup at {}x{}: {}",
            m.current.width,
            m.current.height,
            rate_menu(&cur_rates, m.current.refresh).iter().map(|(_, l, ck)| format!("{}{}", l, if *ck { " ✓" } else { "" })).collect::<Vec<_>>().join(" | ")
        );
        let mut sizes: Vec<(u32, u32)> = list.iter().map(|v| (v.width, v.height)).collect();
        sizes.dedup();
        println!("  sizes        : {}", sizes.iter().map(|(w, h)| format!("{w}x{h}")).collect::<Vec<_>>().join(" "));
        let gdi_rates: std::collections::BTreeSet<u32> = gdi.iter().map(|v| v.refresh.num).collect();
        println!("  GDI rates    : {:?}", gdi_rates);
        // Modes the whole-Hz GDI list has but the exact list doesn't (compared as W x H x truncated Hz, as GDI reports).
        let only_gdi: Vec<String> = gdi
            .iter()
            .filter(|g| !list.iter().any(|d| d.width == g.width && d.height == g.height && (d.refresh.hz() as u32 == g.refresh.num || d.refresh.hz().round() as u32 == g.refresh.num)))
            .map(|g| format!("{}x{}@{}", g.width, g.height, g.refresh.num))
            .collect();
        println!("  only in GDI  : {:?}", only_gdi);

        let b = svc.os_mut().ddc_get(&m.id, Vcp::Brightness);
        let c = svc.os_mut().ddc_get(&m.id, Vcp::Contrast);
        println!("  DDC brightness: {:?}", b);
        println!("  DDC contrast  : {:?}", c);
        let v = svc.os_mut().vibrance_get(&m.id);
        println!("  vibrance raw  : {:?}", v);
        match svc.picture(&m.id) {
            Ok(p) => println!("  picture       : {:?}", p),
            Err(e) => println!("  picture       : ERROR {e}"),
        }
    }
}

fn bu_display_hmon(gdi: &str) -> Option<windows::Win32::Graphics::Gdi::HMONITOR> {
    use windows::Win32::Foundation::{LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::*;
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

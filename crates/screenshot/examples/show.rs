//! Prints what the Screenshots engine sees NOW: the monitors (physical pixels, DPI, rotation, HDR, SDR white) cross-checked
//! against Windows' display modes, what the capture APIs support, the save folder and the gallery.
//! READ-ONLY and NO CAPTURE: it runs on `RealOs::read_only()`, which refuses every change and every screen capture (the screen
//! may show private things). Paths under the user's profile are shown as `%USERPROFILE%`.
//!
//! `cargo run -p bu-screenshot --example screenshot-show`

#[cfg(windows)]
fn main() {
    use bu_screenshot::real::{self, monitors, wgc, RealOs};
    use bu_screenshot::{geom, Screenshots};

    let profile = std::env::var("USERPROFILE").unwrap_or_default();
    let anon = |p: &std::path::Path| {
        let s = p.display().to_string();
        if profile.is_empty() { s } else { s.replace(&profile, "%USERPROFILE%") }
    };

    let data_dir = real::default_data_dir().unwrap_or_default();
    let s = Screenshots::new(RealOs::read_only(), &data_dir);

    println!("== Monitors (numbered left to right; physical pixels) ==");
    let mons = match s.monitors() {
        Ok(m) => m,
        Err(e) => {
            println!("READ ERROR {e}");
            return;
        }
    };
    for m in &mons {
        let (mode, info) = monitors::cross_check(m);
        let dpi_ok = mode.map(|(w, h)| w == m.rect.w && h == m.rect.h).unwrap_or(false) && info == Some(m.rect);
        println!(
            "Monitor {}: {}  {}x{} at ({}, {})  primary={}  scaling={}% ({} dpi)  rotation={:?}  HDR={}  SDR white={} nits",
            m.number, m.device, m.rect.w, m.rect.h, m.rect.x, m.rect.y, m.primary, m.dpi * 100 / 96, m.dpi, m.rotation, m.hdr,
            m.sdr_white_nits
        );
        println!(
            "   cross-check: display mode {:?}, GetMonitorInfo {:?} -> DXGI rectangle matches both: {}",
            mode,
            info.map(|r| (r.x, r.y, r.w, r.h)),
            dpi_ok
        );
    }
    let all = geom::desktop_bounds(&mons);
    println!("All monitors picture: {}x{} (desktop from ({}, {}))", all.w, all.h, all.x, all.y);

    let (wgc_ok, border_prop, cursor_prop) = wgc::support();
    println!("\n== Capture APIs ==");
    println!("Desktop Duplication: always on Windows 8+ (no border exists with it)");
    println!("Windows.Graphics.Capture supported: {wgc_ok}");
    println!("  IsBorderRequired present (yellow border can be switched off): {border_prop}");
    println!("  IsCursorCaptureEnabled present (cursor left out): {cursor_prop}");
    println!("Engine default method: {}", s.method().name());

    println!("\n== Save folder ==");
    match s.saved_dir() {
        Ok(Some(d)) => println!("Chosen with Change path: {}", anon(&d)),
        Ok(None) => println!("Chosen with Change path: (none yet)"),
        Err(e) => println!("Chosen with Change path: READ ERROR {e}"),
    }
    match s.save_dir() {
        Ok(d) => println!("New shots go to: {}  (exists: {})", anon(&d), d.is_dir()),
        Err(e) => println!("New shots go to: READ ERROR {e}"),
    }

    println!("\n== Gallery ==");
    println!("Engine data folder: {}  (exists: {})", anon(&data_dir), data_dir.is_dir());
    match s.gallery() {
        Ok(g) => println!("Shots in the gallery index: {}", g.len()),
        Err(e) => println!("Gallery: {e}"),
    }
}

#[cfg(not(windows))]
fn main() {}

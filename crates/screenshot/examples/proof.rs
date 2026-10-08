//! The ONE-TIME real-capture timing proof (Order 010 item 4). EVERY RUN CAPTURES THE REAL SCREEN — run it only when an order
//! approves a real capture (Order 010 allowed one run per method; that is used up). Build it with
//! `CARGO_PROFILE_RELEASE_PANIC=unwind` for release (the workspace release profile aborts on panic, which would skip the cleanup).
//! Run once per method:
//!   `cargo run -p bu-screenshot --example screenshot-proof -- dd`   (Desktop Duplication)
//!   `cargo run -p bu-screenshot --example screenshot-proof -- wgc`  (Windows.Graphics.Capture)
//!
//! The screen may show private things, so: the picture is never printed or shown — only sizes, timings and pixel COUNTS; it
//! is written only into `BoylerUtilities-board\scratch\lane-h\proof-<method>\` and that folder is deleted before the program
//! ends (also on errors). Runs on `RealOs::capture_proof()`: no clipboard, no Explorer, no Recycle Bin.

#[cfg(windows)]
fn main() {
    use std::path::PathBuf;
    use std::time::Instant;

    use bu_screenshot::encode::{self, PngLevel};
    use bu_screenshot::real::{wgc, RealOs};
    use bu_screenshot::{Method, Screenshots, Target};

    // The app will be per-monitor DPI aware (its manifest); the proof process is made so too, which DuplicateOutput1 (HDR)
    // requires.
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }

    let arg = std::env::args().nth(1).unwrap_or_default();
    let (method, tag) = match arg.as_str() {
        "dd" => (Method::DesktopDuplication, "dd"),
        "wgc" => (Method::GraphicsCapture, "wgc"),
        _ => {
            eprintln!("usage: screenshot-proof dd|wgc");
            return;
        }
    };
    let board_scratch = PathBuf::from(r"C:\BoylerUtilities-scratch");
    if !board_scratch.is_dir() {
        eprintln!("refusing: {} is missing (no other place is used)", board_scratch.display());
        return;
    }
    let scratch = board_scratch.join("lane-h");
    let root = scratch.join(format!("proof-{tag}"));
    if root.exists() {
        eprintln!("refusing: {} already exists (left from an earlier run?)", root.display());
        return;
    }
    let shots_dir = root.join("shots");
    let data_dir = root.join("data");
    let ms = |us: u64| us as f64 / 1000.0;

    let run = || -> bu_screenshot::Result<()> {
        std::fs::create_dir_all(&shots_dir).map_err(|e| bu_screenshot::Error::Io { what: "scratch".into(), why: e.to_string() })?;
        let s = Screenshots::new(RealOs::capture_proof(), &data_dir).with_method(method);
        s.set_save_dir(&shots_dir)?;
        println!("method: {}", method.name());

        // 1. Frozen capture of every monitor (cold: devices + capture objects created inside the call).
        let t = Instant::now();
        let frozen = s.capture(Target::All)?;
        let end_to_end = t.elapsed();
        let ti = frozen.timing;
        println!(
            "frozen All: {}x{}  end-to-end {:.1} ms  (inside: setup {:.1} ms, frames {:.1} ms, spread between monitors {:.2} ms, readback {:.1} ms, total {:.1} ms)",
            frozen.image.width, frozen.image.height, end_to_end.as_secs_f64() * 1000.0, ms(ti.setup_us), ms(ti.frames_us),
            ms(ti.spread_us), ms(ti.readback_us), ms(ti.total_us)
        );
        for m in &frozen.monitors {
            let img = frozen.monitor(m.number)?;
            let color = frozen.color.iter().find(|(n, _)| *n == m.number).map(|(_, c)| *c);
            println!(
                "  monitor {}: {}x{} (expected {}x{}: {})  colour path {:?}  non-black pixels {} of {}",
                m.number, img.width, img.height, m.rect.w, m.rect.h, img.width == m.rect.w && img.height == m.rect.h, color,
                img.non_black_pixels(), img.width as u64 * img.height as u64
            );
        }
        if method == Method::GraphicsCapture {
            println!("  yellow border: {:?}", wgc::border_state());
        }

        // 2. PNG encoding speed / size (in memory only).
        for level in [PngLevel::Fast, PngLevel::Balanced, PngLevel::High] {
            let t = Instant::now();
            let png = encode::png_bytes(&frozen.image, level)?;
            println!("  PNG {:?}: {:.1} ms, {} bytes", level, t.elapsed().as_secs_f64() * 1000.0, png.len());
        }
        let t = Instant::now();
        let dib = encode::dib_bytes(&frozen.image);
        println!("  DIB (clipboard bitmap): {:.1} ms, {} bytes (not put on the clipboard)", t.elapsed().as_secs_f64() * 1000.0, dib.len());

        // 3. Save through the engine (into the scratch folder), read it back, check the pixels are exact.
        let t = Instant::now();
        let shot = s.save(&frozen.image)?;
        println!(
            "  save: {:.1} ms -> {} bytes, name {:?}",
            t.elapsed().as_secs_f64() * 1000.0,
            std::fs::metadata(&shot.path).map(|m| m.len()).unwrap_or(0),
            shot.path.file_name().unwrap_or_default()
        );
        let back = s.load(shot.id)?;
        println!("  saved PNG decodes to exactly the captured pixels: {}", back == frozen.image);
        let thumb = s.thumbnail(shot.id)?;
        println!("  gallery: {} shot(s), thumbnail {}x{}", s.gallery()?.len(), thumb.width, thumb.height);

        // 4. Live: start, first frame, one snap, stop (nothing saved).
        let t = Instant::now();
        let mut live = s.live(Target::All)?;
        let started = t.elapsed();
        let t = Instant::now();
        let snap = live.snap()?;
        println!(
            "  live: started (first frames in hand) in {:.1} ms; snap {}x{} in {:.1} ms",
            started.as_secs_f64() * 1000.0, snap.image.width, snap.image.height, t.elapsed().as_secs_f64() * 1000.0
        );
        drop(live);
        Ok(())
    };
    // Even a panic must not skip the cleanup below.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run))
        .unwrap_or_else(|_| Err(bu_screenshot::Error::BadData("panic during the run".into())));

    // Always: delete everything this run wrote.
    let removed = std::fs::remove_dir_all(&root);
    match result {
        Ok(()) => println!("result: OK"),
        Err(e) => println!("result: ERROR {e}"),
    }
    println!(
        "cleanup: {} deleted: {}  (still exists: {})",
        root.display(),
        removed.is_ok(),
        root.exists()
    );
}

#[cfg(not(windows))]
fn main() {}

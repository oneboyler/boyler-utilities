//! `cargo run -p bu-mouse --example mouse-show` — prints the REAL Mouse-tab state, READ-ONLY.
//! Runs on `RealOs::read_only()`: every change is refused by the OS layer, nothing is sent to the mouse, Raw Accel's
//! driver is only asked for its version and what it runs (GET_VERSION + READ ioctls).

use bu_mouse::accel::bytes;
use bu_mouse::accel::curves::{init_data, sensitivity};
use bu_mouse::accel::panel::{from_args, value_text, visible_rows};
use bu_mouse::accel::service::find_rawaccel_dir;
use bu_mouse::cursors::Role;
use bu_mouse::settings::{double_click_step_for_ms, ScrollLines};
use bu_mouse::win::RealOs;
use bu_mouse::{AppDirs, Mouse, MouseOs};
use std::path::PathBuf;

fn main() {
    let scratch = PathBuf::from(r"C:\BoylerUtilities-scratch\lane-g\show-appdata");
    let mut m = Mouse::new(RealOs::read_only(), AppDirs::new(&scratch));

    println!("== Your mouse (device list only — nothing sent to any device) ==");
    match m.mice() {
        Ok(mice) if mice.is_empty() => println!("no mouse found"),
        Ok(mice) => {
            for (i, y) in mice.iter().enumerate() {
                println!(
                    "{} {} — VID {:04X} PID {:04X} · brand {} · {}",
                    if i == 0 { "*" } else { " " },
                    y.name,
                    y.vid,
                    y.pid,
                    y.brand.as_ref().map(|b| b.name).unwrap_or("unknown"),
                    if y.protocol.is_some() { "SUPPORTED (DPI / polling / lift-off / battery)" } else { "not supported" }
                );
                println!("    sub-line: {}", y.sub_line());
                if let Some((t, u)) = y.link() {
                    println!("    link: {t} -> {u}");
                }
                if let Some(p) = &y.config_path {
                    println!("    config interface: {p} (out {} / in {} bytes)", y.output_len, y.input_len);
                }
            }
        }
        Err(e) => println!("error: {e}"),
    }
    match m.os().hid_devices() {
        Ok(h) => {
            let mut v: Vec<_> = h.iter().filter(|d| d.vid == 0x3710).collect();
            v.sort_by_key(|d| (d.interface, d.usage_page, d.usage));
            println!("  Pulsar (VID 3710) HID interfaces: {}", v.len());
            for d in v {
                println!(
                    "    mi_{:02} usage page {:04X} usage {:04X} · in {} out {} feature {} · product {:?}",
                    d.interface.unwrap_or(0),
                    d.usage_page,
                    d.usage,
                    d.input_len,
                    d.output_len,
                    d.feature_len,
                    d.product
                );
            }
        }
        Err(e) => println!("  HID list error: {e}"),
    }

    println!("\n== Mouse settings (Windows) ==");
    match m.windows_mouse() {
        Ok(w) => {
            println!("Pointer speed: {} (1–20)", w.pointer_speed);
            println!("Enhance pointer precision: {} {:?}", if w.precision { "on" } else { "off" }, w.precision_raw);
            match w.scroll_lines {
                ScrollLines::Lines(n) => println!("Scroll lines: {n} lines"),
                ScrollLines::OneScreen => println!("Scroll lines: one screen at a time"),
            }
            println!("Double-click speed: {} ms (slider step {} of 0–14)", w.double_click_ms, double_click_step_for_ms(w.double_click_ms));
            println!("Swap primary button: {}", if w.buttons_swapped { "on" } else { "off" });
        }
        Err(e) => println!("error: {e}"),
    }

    println!("\n== Mouse acceleration = Raw Accel ==");
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let roots: Vec<PathBuf> = ["Desktop", "Downloads", "Documents"].iter().map(|d| PathBuf::from(&home).join(d)).collect();
    m.accel_mut().rawaccel_dir = find_rawaccel_dir(&roots);
    println!("folder: {:?}", m.accel().rawaccel_dir.as_ref().map(|p| p.display().to_string()));
    match m.rawaccel_status() {
        Ok(s) => println!("status: {s:?} · footer: {:?}", s.footer()),
        Err(e) => println!("status error: {e}"),
    }
    let cfg = match m.rawaccel_settings() {
        Ok(Some(c)) => Some(c),
        Ok(None) => {
            println!("settings.json: none");
            None
        }
        Err(e) => {
            println!("settings.json error: {e}");
            None
        }
    };
    if let Some(cfg) = &cfg {
        let p = &cfg.profiles[0];
        println!(
            "settings.json: {} profile(s), {} device entr(ies); profile 0 \"{}\": mode {:?}, gain {}, exponentClassic {}, acceleration {}, inputOffset {}, cap {:?} ({:?}), Output DPI {}",
            cfg.profiles.len(),
            cfg.devices.len(),
            p.name,
            p.accel_x.mode,
            p.accel_x.gain,
            p.accel_x.exponent_classic,
            p.accel_x.acceleration,
            p.accel_x.input_offset,
            (p.accel_x.cap.x, p.accel_x.cap.y),
            p.accel_x.cap_mode,
            p.output_dpi
        );
        if let Some((c, v)) = from_args(&p.accel_x) {
            print!("card: curve {} · Gain {} · Cap type {:?} ·", c.name(), v.gain, v.cap_type);
            for r in visible_rows(c, &v) {
                print!(" {} {} ·", r.label, value_text(&r, v.get(r.field)));
            }
            println!(" Sens multiplier {:.2}×", p.output_dpi / 1000.0);
        }
        let d = init_data(p);
        print!("graph (Raw Accel's sensitivity, exact maths):");
        for x in [20.0, 55.0, 60.0, 80.0, 100.0, 120.0] {
            print!("  {x} → {:.6}", sensitivity(p, &d, x));
        }
        println!();
    }
    match m.os().rawaccel_read() {
        Ok(Some(b)) => {
            let (n, dv) = bytes::read_header(&b).unwrap_or((0, 0));
            println!("driver READ: {} bytes = io_base + {n} profile(s) + {dv} device(s) (v1.7.0 layout predicts {})", b.len(), bytes::write_size(n as usize, dv as usize));
            for p in bytes::read_profiles(&b) {
                println!(
                    "  driver runs \"{}\": mode {:?}, gain {}, acceleration {}, inputOffset {}, cap {:?} {:?}, Output DPI {}",
                    p.name,
                    p.accel_x.mode,
                    p.accel_x.gain,
                    p.accel_x.acceleration,
                    p.accel_x.input_offset,
                    (p.accel_x.cap.x, p.accel_x.cap.y),
                    p.accel_x.cap_mode,
                    p.output_dpi
                );
            }
            if let Some(cfg) = &cfg {
                let ours = bytes::to_bytes(cfg);
                let diffs = bytes::diff(&ours, &b, cfg.profiles.len(), cfg.devices.len(), 20);
                if ours.len() == b.len() && diffs.is_empty() {
                    println!("  BYTE-EXACT: our bytes for settings.json ({} bytes, incl. the precomputed curve data) == the driver's bytes (struct padding excluded)", ours.len());
                } else {
                    println!("  DIFFERENT: ours {} bytes vs driver {} bytes; first differences (offset ours driver): {diffs:?}", ours.len(), b.len());
                }
                println!("  SAME EFFECT (padding and an Off axis' unused bytes ignored): {}", bytes::same_effect(&ours, &b));
                let card = m.accel_target();
                println!("  card state: on {} | would write {}", m.accel().panel.on, if card.args.mode == bu_mouse::accel::args::AccelMode::Noaccel { "Off".to_string() } else { format!("{:?}", card.args.mode) });
                let le_i32 = |at: usize| i32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
                let le_f64 = |at: usize| f64::from_le_bytes(b[at..at + 8].try_into().unwrap_or([0; 8]));
                println!(
                    "  driver default device: disable {} | extra-info {} | constant-poll-time {} | DPI {} | polling {} Hz | clamp {}..{}   settings.json: disable {} | constant-poll-time {} | DPI {} | polling {} Hz",
                    b[0] != 0,
                    b[1] != 0,
                    b[2] != 0,
                    le_i32(4),
                    le_i32(8),
                    le_f64(16),
                    le_f64(24),
                    cfg.default_device_config.disable,
                    cfg.default_device_config.poll_time_lock,
                    cfg.default_device_config.dpi,
                    cfg.default_device_config.polling_rate
                );
                if let (Some(dp), Some(sp)) = (bytes::read_profiles(&b).first(), cfg.profiles.first()) {
                    println!("  driver profile 0 == settings.json profile 0 (every field, Off axes' unused arguments included): {}", dp == sp);
                    println!("  driver: ratios y/x {} l/r {} u/d {} | rotation {} | snap {} | speed cap {}..{} | whole {} | lp-norm {} | halflives {}/{}/{} | domain {:?} | range {:?}", dp.yx_output_dpi_ratio, dp.lr_output_dpi_ratio, dp.ud_output_dpi_ratio, dp.degrees_rotation, dp.degrees_snap, dp.speed_min, dp.speed_max, dp.speed.whole, dp.speed.lp_norm, dp.speed.input_speed_smooth_halflife, dp.speed.scale_smooth_halflife, dp.speed.output_speed_smooth_halflife, dp.domain_weights, dp.range_weights);
                    println!("  json  : ratios y/x {} l/r {} u/d {} | rotation {} | snap {} | speed cap {}..{} | whole {} | lp-norm {} | halflives {}/{}/{} | domain {:?} | range {:?}", sp.yx_output_dpi_ratio, sp.lr_output_dpi_ratio, sp.ud_output_dpi_ratio, sp.degrees_rotation, sp.degrees_snap, sp.speed_min, sp.speed_max, sp.speed.whole, sp.speed.lp_norm, sp.speed.input_speed_smooth_halflife, sp.speed.scale_smooth_halflife, sp.speed.output_speed_smooth_halflife, sp.domain_weights, sp.range_weights);
                }
                let pad = bytes::padding_ranges(cfg.profiles.len(), cfg.devices.len());
                let pad_bytes: Vec<u8> = pad.iter().flat_map(|r| r.clone()).filter_map(|i| b.get(i).copied()).collect();
                println!("  (padding bytes in the driver copy: {} of them, non-zero: {})", pad_bytes.len(), pad_bytes.iter().filter(|x| **x != 0).count());
            }
        }
        Ok(None) => println!("driver READ: no driver"),
        Err(e) => println!("driver READ error: {e}"),
    }
    let saved = std::env::var("APPDATA").map(|a| std::path::PathBuf::from(a).join("Boyler Utilities").join("mouse").join("accel.json")).unwrap_or_default();
    println!("saved card file {}: {}", saved.display(), if saved.is_file() { "exists" } else { "none" });
    println!("Raw Accel app writes the driver when it opens: {} | its app running now: {} | other-writer line: {:?}", m.rawaccel_auto_writes(), m.os().process_running("rawaccel.exe").unwrap_or(false), m.other_writer_line());
    match m.mirror_rawaccel() {
        Ok(true) => println!("mirror: the card would start ON (driver runs a curve): {}", bu_mouse::accel::service::header_line(&m.accel().panel, &m.accel().per_app)),
        Ok(false) => println!("mirror: the card would start OFF"),
        Err(e) => println!("mirror error: {e}"),
    }
    match m.epp_warning() {
        Ok(w) => println!("EPP warning shown: {w}"),
        Err(e) => println!("EPP error: {e}"),
    }
    // Proof that this layer refuses changes — only with HARMLESS targets (TECH_RULES, Order 007): calls that would do
    // nothing even if the guard were broken (a writer.exe that does not exist, a HID path that does not exist, a scratch-key
    // registry path) — never a real driver WRITE.
    let gone = PathBuf::from(r"C:\BoylerUtilities-scratch\lane-g\no-such-folder");
    println!("refused: writer.exe in a missing folder -> {:?}", m.os_mut().rawaccel_writer(&gone, &gone.join("s.json"), "{}"));
    println!("refused: a request to a missing HID path -> {:?}", m.os_mut().hid_exchange(r"\\?\hid#no-such-device", &[0u8; 17], &bu_mouse::os::HidTransfer::Feature { reply_id: 8, reply_len: 17 }));
    println!("refused: a registry write -> {:?}", m.os_mut().reg_write(r"Software\BoylerUtilities-test\G\never", "x", &bu_mouse::os::RegValue::Dword(1)));

    println!("\n== Cursors ==");
    match m.cursors() {
        Ok(c) => {
            println!("scheme: {:?} (Scheme Source {:?}) · size {} ({} px)", c.scheme, c.scheme_source, c.size, c.size_px);
            for r in &c.roles {
                println!("  {:<8} {} · set: {}", r.role.name(), if r.file.is_empty() { "(Windows built-in)".to_string() } else { r.file.replace(&home, "%USERPROFILE%") }, r.set.label());
            }
        }
        Err(e) => println!("error: {e}"),
    }
    match m.schemes() {
        Ok(s) => println!("schemes Windows knows: {}", s.iter().map(|s| format!("{}{}", s.name, if s.system { "" } else { " (yours)" })).collect::<Vec<_>>().join(", ")),
        Err(e) => println!("schemes error: {e}"),
    }
    for role in [Role::Normal, Role::Link] {
        println!("suggestion for {}: {:?}", role.name(), m.suggestion(role));
    }
}

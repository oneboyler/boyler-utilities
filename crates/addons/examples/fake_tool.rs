//! Test stand-in for Raw Accel's installer.exe / uninstaller.exe (tests/helper_tool.rs): prints like them through a
//! buffered writer (as their `std::cout` into a non-console would), then waits for one key with `_getwch()` exactly as
//! they do. Args: `ok` (default) / `fail` / `hang` (never asks for the key).

use std::io::Write;

extern "C" {
    fn _getwch() -> u16;
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "ok".into());
    let mut out = std::io::BufWriter::new(std::io::stdout());
    match mode.as_str() {
        "fail" => {
            let _ = writeln!(out, "Driver already installed. Removing previous installation.");
            let _ = writeln!(out, "Error: copy_file failed: Access is denied. system:5");
        }
        "hang" => {
            let _ = out.flush();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
        _ => {
            let _ = writeln!(out, "Install complete, change will take effect after restart.");
        }
    }
    let _ = writeln!(out, "Press any key to close this window . . .");
    let _ = out.flush();
    // SAFETY: the CRT's console read, as in Raw Accel's tools.
    let k = unsafe { _getwch() };
    std::process::exit(if k == 13 { 0 } else { 3 });
}

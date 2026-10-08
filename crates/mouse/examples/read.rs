//! `cargo run -p bu-mouse --example mouse-read` — the ONE real read of the mouse approved in A_009_01.
//! Runs on `RealOs::read_requests_only()`: only allow-listed READ requests (identify 0x01, online 0x03 with the lock byte
//! 0, battery 0x04, read settings memory 0x08) can leave; a write / reset / profile command is refused before any byte
//! is sent. Every request and answer is printed byte for byte. If the mouse answers "offline": one more try after 60 s,
//! then stop (no loop).

use bu_mouse::device::ReadOptions;
use bu_mouse::pulsar;
use bu_mouse::win::RealOs;
use bu_mouse::{AppDirs, Mouse};

fn main() {
    // The allow-list itself, shown before anything is sent: these are refused.
    for (name, f) in [
        ("write 0x07", pulsar::frame(pulsar::CMD_WRITE, 0, &[1, 0x54])),
        ("factory reset 0x09", pulsar::frame(pulsar::CMD_RESET, 0, &[])),
        ("set profile 0x0F", pulsar::frame(pulsar::CMD_SET_PROFILE, 0, &[1])),
        ("write lock 0x03/1", pulsar::frame(pulsar::CMD_ONLINE, 0, &[1])),
    ] {
        println!("allow-list check: {name} -> {}", if pulsar::read_request_allowed(&f) { "ALLOWED (bug!)" } else { "refused" });
    }
    let scratch = std::path::PathBuf::from(r"C:\BoylerUtilities-scratch\lane-g\read-appdata");
    let mut m = Mouse::new(RealOs::read_requests_only(), AppDirs::new(scratch));
    let mice = match m.mice() {
        Ok(v) => v,
        Err(e) => {
            println!("device list error: {e}");
            return;
        }
    };
    let Some(y) = mice.into_iter().find(|y| y.protocol.is_some()) else {
        println!("no supported mouse connected");
        return;
    };
    println!("mouse: {} (VID {:04X} PID {:04X})", y.name, y.vid, y.pid);
    // `--once`: a single attempt (used for the approved retry after a first run that got no full answer)
    let attempts = if std::env::args().any(|a| a == "--once") { 1 } else { 2 };
    let mut shown = 0;
    for attempt in 1..=attempts {
        let r = m.read_on_mouse(&y, ReadOptions { online_tries: 1 });
        let lines = m.os().recorded()[shown..].to_vec();
        shown += lines.len();
        for line in lines {
            println!("  {line}");
        }
        match r {
            Ok(o) => {
                println!(
                    "attempt {attempt}: model {:?} (family {:#04x}, model code {}), link {:?}, online {}, battery {:?} % charging {:?} ({:?} mV), stage {:?}, DPI {:?}, polling {:?} Hz, lift-off {:?} (tenths of a mm)",
                    o.model, o.family, o.model_code, o.link, o.online, o.battery_percent, o.charging, o.battery_mv, o.stage, o.dpi, o.polling_hz, o.lift_off
                );
                if o.online || attempt == attempts {
                    break;
                }
                println!("mouse offline (asleep?) — one more try in 60 s, then stop");
                std::thread::sleep(std::time::Duration::from_secs(60));
            }
            Err(e) => {
                println!("attempt {attempt}: error: {e}");
                if attempt == attempts {
                    break;
                }
                println!("no full answer — one more try in 60 s, then stop");
                std::thread::sleep(std::time::Duration::from_secs(60));
            }
        }
    }
}

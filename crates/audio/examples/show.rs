//! `cargo run -p bu-audio --example audio-show` — the REAL audio state, READ-ONLY: the OS layer is
//! `RealOs::read_only()` (every change refused). Lists devices, defaults per role, volumes, a 1 s level sample and the
//! app mixer. Plays nothing, changes nothing.

use bu_audio::*;
use std::time::{Duration, Instant};

fn main() {
    let os = match RealOs::read_only() {
        Ok(o) => o,
        Err(e) => {
            println!("no Core Audio: {e}");
            return;
        }
    };
    let mut svc = AudioService::new(os);
    for flow in FLOWS {
        let t = Instant::now();
        let rows = svc.device_rows(flow).unwrap_or_default();
        let defs = svc.defaults(flow).unwrap_or_default();
        println!("== {flow:?} devices ({} listed, read in {:.0} ms)", rows.len(), t.elapsed().as_secs_f64() * 1000.0);
        for r in &rows {
            let roles: Vec<&str> = ROLES
                .iter()
                .filter(|x| defs.get(**x) == Some(&r.device.id))
                .map(|x| match x {
                    Role::Console => "default",
                    Role::Multimedia => "multimedia",
                    Role::Communications => "communications",
                })
                .collect();
            let vol = svc
                .device_volume(&r.device.id)
                .map(|v| format!("{:>3.0} %{}", v.volume * 100.0, if v.muted { " muted" } else { "" }))
                .unwrap_or("  -".into());
            println!(
                "  {} {:<5} {:<55} {:<4} {}{}",
                if r.current { "*" } else { " " },
                r.device.kind.glyph(),
                r.device.name,
                if r.on { "on" } else { "OFF" },
                vol,
                if roles.is_empty() { String::new() } else { format!("  [{}]", roles.join(", ")) }
            );
        }
    }
    let out = svc.current(Flow::Output).ok().flatten();
    let inp = svc.current(Flow::Input).ok().flatten();
    // levels: sample 1 s at 16 ms (only here, while asked)
    let (mut po, mut pi, mut n) = (0f32, 0f32, 0);
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(1) {
        if let Some(d) = &out {
            po = po.max(svc.os_mut().peak(&d.id).unwrap_or(0.0));
        }
        if let Some(d) = &inp {
            pi = pi.max(svc.os_mut().peak(&d.id).unwrap_or(0.0));
        }
        n += 1;
        std::thread::sleep(Duration::from_millis(16));
    }
    println!("== levels over 1 s ({n} reads): output peak {po:.3}, input peak {pi:.3}");
    if let Some(d) = &out {
        // the first call starts the icon reads; the second (a moment later) has them
        let _ = svc.apps(&d.id);
        std::thread::sleep(Duration::from_millis(1500));
        let t = Instant::now();
        let apps = svc.apps(&d.id).unwrap_or_default();
        println!("== mixer on \"{}\" ({} apps making sound, read in {:.0} ms)", d.name, apps.len(), t.elapsed().as_secs_f64() * 1000.0);
        for a in apps {
            println!(
                "  {:<28} {:>3.0} %{} sessions {} pids {:?} colour #{:06x}/#{:06x} icon {}",
                a.look.name,
                a.volume * 100.0,
                if a.muted { " muted" } else { "" },
                a.sessions.len(),
                a.pids,
                a.look.colour,
                a.look.colour2,
                a.look.icon.as_ref().map(|i| format!("{}x{}", i.w, i.h)).unwrap_or("-".into())
            );
        }
        // the read-only layer refuses a change — proven on a device that doesn't exist, so even a broken guard could
        // change nothing real (it would answer NotFound instead of ReadOnly)
        println!("== read-only check: set volume on a made-up device -> {:?}", svc.os_mut().set_volume("{bu-audio-no-such-device}", 0.5).err());
    }
}

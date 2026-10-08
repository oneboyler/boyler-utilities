//! VALIDATE-ONLY proof on the real PC: builds the exact SetDisplayConfig data our Apply / Main-display code would send
//! and submits it with SDC_VALIDATE (Windows checks it; nothing is applied). Then confirms the real state is unchanged.
//!
//!   cargo run -p bu-display --example display-validate

use bu_display::fields::{rates_for, resolve_fields};
use bu_display::win::WinDisplayOs;
use bu_display::{DisplayOs, GpuScaling, Mode, RefreshRate};

fn main() {
    let os = WinDisplayOs::read_only(None); // proof program: every change refused; validate_* = SDC_VALIDATE only
    let before = os.monitors().expect("monitors");
    for m in &before {
        let modes = os.modes(&m.id).expect("modes");
        println!("\n=== monitor {} {:?}", m.number, m.name);
        let mut checks: Vec<(String, Mode)> = vec![("the applied mode itself".into(), m.current)];
        let rates = rates_for(&modes, m.current.width, m.current.height);
        if let Some(r) = rates.iter().rev().find(|r| **r != m.current.refresh) {
            checks.push((format!("same size, other reported rate {:.3}", r.hz()), Mode { refresh: *r, ..m.current }));
        }
        if let Ok(md) = resolve_fields(&modes, 1280, 960, 1000.0, GpuScaling::KeepAspect) {
            checks.push(("1280x960 fastest rate, Keep aspect".into(), md));
        }
        if let Ok(md) = resolve_fields(&modes, 1440, 1080, 1000.0, GpuScaling::Stretch) {
            checks.push(("1440x1080 fastest rate, Stretch".into(), md));
        }
        if let Ok(md) = resolve_fields(&modes, 1280, 960, 60.0, GpuScaling::BlackBars) {
            checks.push(("1280x960 typed 60 Hz (snapped), Black bars".into(), md));
        }
        checks.push((
            "NOT reported: native size at 500 Hz (fields never send this)".into(),
            Mode { refresh: RefreshRate::whole(500), ..m.current },
        ));
        for (what, md) in checks {
            let r = os.validate_mode(&m.id, &md);
            let strict = os.validate_mode_strict(&m.id, &md);
            println!(
                "  {:<62} {}x{} @ {:.3} {:?}\n      allow-changes -> {:?}   strict (what Apply uses) -> {:?}",
                what, md.width, md.height, md.refresh.hz(), md.scaling, r, strict
            );
        }
        let what = if m.is_main { "make this (already main) the main display" } else { "make this the main display" };
        println!("  {what}\n      allow-changes -> {:?}   strict (what Apply uses) -> {:?}", os.validate_main(&m.id), os.validate_main_strict(&m.id));
    }
    let after = os.monitors().expect("monitors");
    println!("\nreal state unchanged after all validations: {}", before == after);
}

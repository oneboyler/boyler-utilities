//! The read-only OS layer the proof programs run on: every change is refused on its first line. Proven only with a
//! monitor id that cannot exist, so even a broken guard could not change anything real.
#![cfg(windows)]

use bu_display::win::WinDisplayOs;
use bu_display::*;

fn nowhere() -> MonitorId {
    MonitorId(r"\\?\DISPLAY#BU-TEST-NO-SUCH-MONITOR#0#{00000000-0000-0000-0000-000000000000}".into())
}

#[test]
fn read_only_layer_refuses_every_change() {
    let mut os = WinDisplayOs::read_only(None);
    assert!(os.is_read_only());
    let id = nowhere();
    let mode = Mode { width: 1920, height: 1080, refresh: RefreshRate::whole(60), scaling: GpuScaling::DriverDefault };
    assert_eq!(os.apply_mode(&id, &mode, false), Err(DisplayError::ReadOnly("apply_mode")));
    assert_eq!(os.apply_mode(&id, &mode, true), Err(DisplayError::ReadOnly("apply_mode")));
    assert_eq!(os.save_current(&id, &bu_display::Mode { width: 1, height: 1, refresh: bu_display::RefreshRate::new(60, 1), scaling: bu_display::GpuScaling::Stretch }), Err(DisplayError::ReadOnly("save_current")));
    assert_eq!(os.set_main(&id), Err(DisplayError::ReadOnly("set_main")));
    assert_eq!(os.set_dpi_percent(&id, 125), Err(DisplayError::ReadOnly("set_dpi_percent")));
    assert_eq!(os.ddc_set(&id, Vcp::Brightness, 50), Err(DisplayError::ReadOnly("ddc_set")));
    assert_eq!(os.vibrance_set(&id, 50), Err(DisplayError::ReadOnly("vibrance_set")));
}

#[test]
fn read_only_layer_still_reads() {
    let os = WinDisplayOs::read_only(None);
    // Reading the real monitor list is allowed (and changes nothing).
    let mons = os.monitors().expect("monitors");
    assert!(!mons.is_empty());
    // The full layer is not read-only.
    assert!(!WinDisplayOs::new(None).is_read_only());
}

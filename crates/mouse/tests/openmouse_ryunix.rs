//! Order 105 check, Ryunix KYU Pro MX1 (held back): OpenMouse's telemetry report decodes to the same values (decode), and the
//! driver claims no mouse and sends nothing. Expected values are worked out by hand from OpenMouse (@openmouse/protocol 0.40.1:
//! `dist/ryunix/kyu-pro-mx1.js`, `dist/drivers/ryunix/kyu-pro-mx1-hid.js`).

use bu_mouse::family::{self, Driver, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::ryunix as rx;
use bu_mouse::Error;

#[test]
fn the_telemetry_report_decodes_like_openmouse() {
    // input report 0x04: [active, dpi stage, polling code, battery, charging, LED mode]
    let t = rx::decode_telemetry(0x04, &[1, 2, 0x01, 87, 0, 3]).unwrap();
    assert_eq!((t.active, t.dpi_stage, t.polling_hz, t.battery_percent, t.charging, t.led_mode), (true, 2, 1000, 87, false, 3));
    // POLLING_RATES: 0x08 = 125, 0x04 = 250, 0x02 = 500, 0x01 = 1000
    for (code, hz) in [(0x08u8, 125u32), (0x04, 250), (0x02, 500), (0x01, 1000)] {
        assert_eq!(rx::decode_telemetry(0x04, &[0, 0, code, 50, 1, 0]).unwrap().polling_hz, hz, "{code:#x}");
    }
    let t = rx::decode_telemetry(0x04, &[0, 0, 0x02, 100, 1, 7]).unwrap();
    assert_eq!((t.active, t.battery_percent, t.charging, t.led_mode), (false, 100, true, 7));
    // any failed check makes the whole report invalid: another report id, fewer than six bytes, an active byte above 1,
    // an unknown polling code, a battery above 100, a charging byte above 1
    assert!(rx::decode_telemetry(0x05, &[1, 2, 0x01, 87, 0, 3]).is_none());
    assert!(rx::decode_telemetry(0x04, &[1, 2, 0x01, 87, 0]).is_none());
    assert!(rx::decode_telemetry(0x04, &[2, 2, 0x01, 87, 0, 3]).is_none());
    assert!(rx::decode_telemetry(0x04, &[1, 2, 0x03, 87, 0, 3]).is_none());
    assert!(rx::decode_telemetry(0x04, &[1, 2, 0x01, 101, 0, 3]).is_none());
    assert!(rx::decode_telemetry(0x04, &[1, 2, 0x01, 87, 2, 3]).is_none());
    // extra bytes are fine (a longer report)
    assert!(rx::decode_telemetry(0x04, &[1, 2, 0x01, 87, 0, 3, 9, 9]).is_some());
}

#[test]
fn no_mouse_is_claimed_and_nothing_is_sent() {
    // supported-devices-data.ts: KYU Pro MX1 pids [0x26e, 0x26f] - listed, but the driver is held back (see the module header)
    for pid in [0x026Eu16, 0x026F] {
        let c = hid_info("ryunix", 0x04F3, pid, 0x000A, 0x00C7, (7, 0, 8));
        assert!(rx::matches(&[&c], 0x04F3, pid).is_none(), "{pid:04x}");
        assert!(family::find(&[c], 0x04F3, pid).is_none(), "{pid:04x}: no family claims it");
    }
    assert!(!rx::read_request_allowed(&[0x05, 0, 0, 0]) && !rx::read_request_allowed(&[]) && !rx::read_request_allowed(&[0x04]));
    // the driver itself, called by hand, sends nothing
    let m = bu_mouse::family::Matched {
        family: family::Family::Ryunix,
        model: None,
        wireless: None,
        config: hid_info("ryunix", 0x04F3, 0x026E, 0x000A, 0x00C7, (7, 0, 8)),
        variant: 0,
    };
    let mut p = ScriptPort::new(m.config.clone(), |_, _| panic!("nothing may be sent"));
    assert!(matches!(rx::Ryunix.read(&m, &mut p, ReadOptions::default()), Err(Error::UnsupportedMouse(_))));
    for s in [Setting::Dpi(800), Setting::Polling(1000), Setting::LiftOff(10)] {
        assert!(matches!(rx::Ryunix.write(&m, &mut p, s), Err(Error::UnsupportedMouse(_))), "{s:?}");
    }
    assert!(p.sent.is_empty());
}

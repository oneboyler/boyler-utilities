//! Order 104 check, Logitech HID++ 2.0: our requests = the bytes OpenMouse sends for the same setting (encode), and answers
//! shaped the way OpenMouse reads them decode to the same values (decode). The expected bytes are worked out by hand from
//! OpenMouse (@openmouse/protocol 0.40.1: `dist/drivers/logitech/hidpp.js`, `dist/logitech/index.js`,
//! `dist/drivers/logitech/onboard-profiles.js`), never from our own encoder.
//!
//! OpenMouse's requestLong builds `new Uint8Array(19)` = `[device, feature index, withSoftwareId(function), params…]` and
//! sends it as report 0x11; Windows wants the report id in front: OpenMouse's byte i = our byte i + 1. withSoftwareId(f)
//! = (f & 0xF0) | 5, so function 0x00 → 0x05, 0x10 → 0x15, 0x20 → 0x25, 0x30 → 0x35, 0x50 → 0x55, 0x60 → 0x65.
//! OpenMouse's `reply` / `report` is an input report WITHOUT the id; the decoders take the same, so the indices below are
//! OpenMouse's own.

use bu_mouse::family::{Driver, Family, Matched, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::logitech::{self as lg, Transport};
use bu_mouse::os::{HidInfo, HidTransfer};
use std::cell::RefCell;
use std::rc::Rc;

/// OpenMouse's requestLong bytes `[device, feature, function | 5, params…]` as Windows sends them: 0x11 first, 20 bytes.
fn om_long(b: &[u8]) -> Vec<u8> {
    let mut v = vec![0x11];
    v.extend_from_slice(b);
    v.resize(20, 0);
    v
}

/// An OpenMouse `reply` (no report id), 19 bytes, from its first bytes.
fn reply(b: &[u8]) -> Vec<u8> {
    let mut v = b.to_vec();
    v.resize(19, 0);
    v
}

// ---------------------------------------------------------------- encode

#[test]
fn feature_lookups_are_openmouses_getfeature_bytes() {
    // getFeature(featureId) = request(0x00, 0x00, featureId >> 8, featureId & 0xff)
    assert_eq!(lg::get_feature(0x01, 0x0003).to_vec(), om_long(&[0x01, 0x00, 0x05, 0x00, 0x03]), "the probe on slot 1");
    assert_eq!(lg::get_feature(0xFF, 0x2202).to_vec(), om_long(&[0xFF, 0x00, 0x05, 0x22, 0x02]));
    assert_eq!(lg::get_feature(0x02, 0x8100).to_vec(), om_long(&[0x02, 0x00, 0x05, 0x81, 0x00]));
    assert_eq!(lg::get_feature(0x03, 0x1F20).to_vec(), om_long(&[0x03, 0x00, 0x05, 0x1F, 0x20]));
}

#[test]
fn dpi_requests_are_openmouses() {
    // readDpiConfiguration: request(idx, 0x50)
    assert_eq!(lg::read_dpi(0xFF, 0x09).to_vec(), om_long(&[0xFF, 0x09, 0x55]));
    // readDpiCapabilities: request(idx, 0x10, 0x00)
    assert_eq!(lg::dpi_capabilities(0xFF, 0x09).to_vec(), om_long(&[0xFF, 0x09, 0x15, 0x00]));
    // getDpiOptions page 2: request(idx, 0x20, 0x00, 0x00, page)
    assert_eq!(lg::dpi_list_page(0x01, 0x09, 2).to_vec(), om_long(&[0x01, 0x09, 0x25, 0x00, 0x00, 0x02]));
    // setDpi(1600): requestLong(idx, 0x60, [0x00, 1600 >> 8 = 0x06, 1600 & 0xff = 0x40, 0x06, 0x40, current.lod = 2])
    assert_eq!(lg::set_dpi(0x01, 0x09, 1600, 1600, 2).to_vec(), om_long(&[0x01, 0x09, 0x65, 0x00, 0x06, 0x40, 0x06, 0x40, 0x02]));
    // the same function with another lift-off byte (setLiftOffDistance "High" on format 7 = 3: lodCodes(7)[2])
    assert_eq!(lg::set_dpi(0x01, 0x09, 800, 800, lg::lod_codes(Some(7))[2]).to_vec(), om_long(&[0x01, 0x09, 0x65, 0x00, 0x03, 0x20, 0x03, 0x20, 0x03]));
    // legacy 0x2201: getSensorDpi request(idx, 0x20, 0x00), list request(idx, 0x10, 0x00), set request(idx, 0x30, 0, hi, lo)
    assert_eq!(lg::legacy_read_dpi(0xFF, 0x0B).to_vec(), om_long(&[0xFF, 0x0B, 0x25, 0x00]));
    assert_eq!(lg::legacy_dpi_list(0xFF, 0x0B).to_vec(), om_long(&[0xFF, 0x0B, 0x15, 0x00]));
    assert_eq!(lg::legacy_set_dpi(0xFF, 0x0B, 2400).to_vec(), om_long(&[0xFF, 0x0B, 0x35, 0x00, 0x09, 0x60]));
}

#[test]
fn rate_requests_are_openmouses() {
    // 0x8061: list request(idx, 0x10), current request(idx, 0x20), set request(idx, 0x30, REPORT_RATE_HZ.indexOf(hz))
    assert_eq!(lg::rate_list(0x01, 0x0C).to_vec(), om_long(&[0x01, 0x0C, 0x15]));
    assert_eq!(lg::read_rate(0x01, 0x0C).to_vec(), om_long(&[0x01, 0x0C, 0x25]));
    assert_eq!(lg::set_rate(0x01, 0x0C, 8000).unwrap().to_vec(), om_long(&[0x01, 0x0C, 0x35, 0x06]));
    assert_eq!(lg::set_rate(0x01, 0x0C, 1000).unwrap().to_vec(), om_long(&[0x01, 0x0C, 0x35, 0x03]));
    assert!(lg::set_rate(0x01, 0x0C, 333).is_none(), "Unsupported polling rate.");
    // 0x8060: list request(idx, 0x00), current request(idx, 0x10), set request(idx, 0x20, round(1000 / hz))
    assert_eq!(lg::legacy_rate_list(0xFF, 0x0D).to_vec(), om_long(&[0xFF, 0x0D, 0x05]));
    assert_eq!(lg::legacy_read_rate(0xFF, 0x0D).to_vec(), om_long(&[0xFF, 0x0D, 0x15]));
    assert_eq!(lg::legacy_set_rate(0xFF, 0x0D, 500).unwrap().to_vec(), om_long(&[0xFF, 0x0D, 0x25, 0x02]));
    assert_eq!(lg::legacy_set_rate(0xFF, 0x0D, 125).unwrap().to_vec(), om_long(&[0xFF, 0x0D, 0x25, 0x08]));
    // Math.round(1000 / 2000) = Math.round(0.5) = 1 in JS (halves up): OpenMouse would send 1 ms (its read-back then fails)
    assert_eq!(lg::legacy_set_rate(0xFF, 0x0D, 2000).unwrap().to_vec(), om_long(&[0xFF, 0x0D, 0x25, 0x01]));
}

#[test]
fn legacy_rate_ms_is_openmouses_rounding() {
    // rateMs = Math.round(1000 / hz), refused outside 1..8
    assert_eq!(lg::legacy_rate_ms(1000), Some(1));
    assert_eq!(lg::legacy_rate_ms(250), Some(4));
    assert_eq!(lg::legacy_rate_ms(333), Some(3));
    assert_eq!(lg::legacy_rate_ms(100), None, "10 ms > 8");
}

#[test]
fn battery_identity_and_profile_requests_are_openmouses() {
    // readBattery (0x1004): request(idx, 0x10); 0x1000 / 0x1001 / 0x1f20: request(idx, 0x00)
    assert_eq!(lg::battery_request(0x1004, 0x02, 0x06).to_vec(), om_long(&[0x02, 0x06, 0x15]));
    assert_eq!(lg::battery_request(0x1000, 0x02, 0x06).to_vec(), om_long(&[0x02, 0x06, 0x05]));
    assert_eq!(lg::battery_request(0x1001, 0x02, 0x06).to_vec(), om_long(&[0x02, 0x06, 0x05]));
    assert_eq!(lg::battery_request(0x1F20, 0x02, 0x06).to_vec(), om_long(&[0x02, 0x06, 0x05]));
    // readIdentity: request(idx, 0x00)
    assert_eq!(lg::identity(0xFF, 0x02).to_vec(), om_long(&[0xFF, 0x02, 0x05]));
    // 0x8100: getInfo request(idx, PROFILE_FN.getInfo = 0x00), getMode 0x20, ensureHostControl's request(idx, 0x10, 0x02)
    assert_eq!(lg::profiles_info(0x01, 0x0E).to_vec(), om_long(&[0x01, 0x0E, 0x05]));
    assert_eq!(lg::get_onboard_mode(0x01, 0x0E).to_vec(), om_long(&[0x01, 0x0E, 0x25]));
    assert_eq!(lg::set_host_mode(0x01, 0x0E).to_vec(), om_long(&[0x01, 0x0E, 0x15, 0x02]));
}

// ---------------------------------------------------------------- decode

#[test]
fn answers_and_errors_are_matched_like_openmouse() {
    // the waiter: report[0] = device, report[1] = feature, report[2] = withSoftwareId(function)
    assert_eq!(lg::classify(&reply(&[0x01, 0x09, 0x55, 0x00]), 0x01, 0x09, 0x55), lg::Answer::Reply);
    // hidppErrorForRequest: [device, 0xff, feature, function | id, error] / [device, 0x8f, …]
    assert_eq!(lg::classify(&reply(&[0x01, 0xFF, 0x09, 0x65, 0x02]), 0x01, 0x09, 0x65), lg::Answer::Rejected { hidpp10: false, code: 2 });
    assert_eq!(lg::classify(&reply(&[0x03, 0x8F, 0x00, 0x05, 0x08]), 0x03, 0x00, 0x05), lg::Answer::Rejected { hidpp10: true, code: 8 });
    // a notification (software id 0) or another device is not the answer
    assert_eq!(lg::classify(&reply(&[0x01, 0x09, 0x50]), 0x01, 0x09, 0x55), lg::Answer::Other);
    assert_eq!(lg::classify(&reply(&[0x02, 0x09, 0x55]), 0x01, 0x09, 0x55), lg::Answer::Other);
    assert_eq!(lg::error_words(false, 0x02), "The mouse rejected that setting (invalid argument).");
    assert_eq!(lg::error_words(true, 0x0A), "The mouse rejected that setting (HID++ 1.0: request unavailable).");
    assert_eq!(lg::error_words(false, 0x42), "The mouse rejected that setting (HID++ 2.0 error 0x42).");
    // the transfer's prefixes: the reply, the 2.0 error, the 1.0 error (report id included)
    let p = lg::answer_prefixes(&lg::read_dpi(0x01, 0x09));
    assert_eq!(p, vec![vec![0x11, 0x01, 0x09, 0x55], vec![0x11, 0x01, 0xFF, 0x09, 0x55], vec![0x11, 0x01, 0x8F, 0x09, 0x55]]);
}

#[test]
fn feature_answers_decode_like_openmouse() {
    // getFeature: { index: reply[3], version: reply[6] }
    assert_eq!(lg::decode_feature(&reply(&[0xFF, 0x00, 0x05, 0x0A, 0x00, 0x00, 0x03])), (0x0A, 3));
    assert_eq!(lg::decode_feature(&reply(&[0xFF, 0x00, 0x05, 0x00])).0, 0, "index 0 = not there");
}

#[test]
fn dpi_answers_decode_like_openmouse() {
    // readDpiConfiguration: x = reply[4] << 8 | reply[5], y = reply[8..9], lod = reply[12]
    let r = reply(&[0x01, 0x09, 0x55, 0x00, 0x06, 0x40, 0x00, 0x00, 0x03, 0x20, 0x00, 0x00, 0x02]);
    assert_eq!(lg::decode_dpi_configuration(&r), (1600, 800, 2));
    // readDpiCapabilities: flags = reply[5]; bit 0 separate axes, bit 1 lift-off
    assert_eq!(lg::decode_dpi_capabilities(&reply(&[0x01, 0x09, 0x15, 0x00, 0x00, 0x02])), (false, true));
    assert_eq!(lg::decode_dpi_capabilities(&reply(&[0x01, 0x09, 0x15, 0x00, 0x00, 0x01])), (true, false));
    // readLegacyDpi: reply[4] << 8 | reply[5]
    assert_eq!(lg::decode_legacy_dpi(&reply(&[0xFF, 0x0B, 0x25, 0x00, 0x0C, 0x80])), 3200);
}

#[test]
fn dpi_lists_parse_like_openmouses_getdpioptions() {
    // 100, then 0xE000 | 50 (a range in steps of 50) up to 25600 = 0x6400, then 0 ends it
    let b = [0x00, 0x64, 0xE0, 0x32, 0x64, 0x00, 0x00, 0x00];
    let v = lg::parse_dpi_list(&b, true).unwrap();
    assert_eq!((v.len(), v[0], v[1], *v.last().unwrap()), ((25600 - 100) / 50 + 1, 100, 150, 25600));
    assert_eq!(lg::dpi_caps(&v).unwrap().bands, vec![(100, 25600, 50)]);
    // 100..1000 in 10s (0xE00A), then 1000..10000 in 100s (0xE064): the second range starts after the first one's end
    let b = [0x00, 0x64, 0xE0, 0x0A, 0x03, 0xE8, 0xE0, 0x64, 0x27, 0x10, 0x00, 0x00];
    let v = lg::parse_dpi_list(&b, true).unwrap();
    assert_eq!(lg::dpi_caps(&v).unwrap().bands, vec![(100, 1000, 10), (1100, 10000, 100)]);
    // plain values (no range): 400, 800, 1600, 3200
    let b = [0x01, 0x90, 0x03, 0x20, 0x06, 0x40, 0x0C, 0x80, 0x00, 0x00];
    assert_eq!(lg::parse_dpi_list(&b, true).unwrap(), vec![400, 800, 1600, 3200]);
    // a range whose end is not above its start: 0x2202 throws "invalid DPI range", 0x2201 stops the list there
    let b = [0x03, 0x20, 0xE0, 0x32, 0x01, 0x00, 0x00, 0x00];
    assert!(lg::parse_dpi_list(&b, true).is_err());
    assert_eq!(lg::parse_dpi_list(&b, false).unwrap(), vec![800]);
    // the page loop stops once two zero bytes follow each other
    assert!(lg::dpi_list_done(&[0x00, 0x64, 0x00, 0x00]) && !lg::dpi_list_done(&[0x00, 0x64, 0xE0, 0x32]));
}

#[test]
fn rate_answers_decode_like_openmouse() {
    // readSupportedPollingRates: flags = reply[3] << 8 | reply[4]; bit i → REPORT_RATE_HZ[i]
    assert_eq!(lg::decode_supported_rates(&reply(&[0x01, 0x0C, 0x15, 0x00, 0x7F])), vec![125, 250, 500, 1000, 2000, 4000, 8000]);
    assert_eq!(lg::decode_supported_rates(&reply(&[0x01, 0x0C, 0x15, 0x00, 0x0F])), vec![125, 250, 500, 1000]);
    // readPollingRate: REPORT_RATE_HZ[reply[3]]
    assert_eq!(lg::decode_rate(&reply(&[0x01, 0x0C, 0x25, 0x06])), Some(8000));
    assert_eq!(lg::decode_rate(&reply(&[0x01, 0x0C, 0x25, 0x07])), None, "unknown report-rate value");
    // decodeReportRateBitmap(0x8b): bits 0, 1, 3, 7 = 1, 2, 4, 8 ms = 1000, 500, 250, 125 Hz (index.js's own G402 example)
    assert_eq!(lg::decode_report_rate_bitmap(0x8B), vec![125, 250, 500, 1000]);
    assert_eq!(lg::decode_legacy_rates(&reply(&[0xFF, 0x0D, 0x05, 0x8B])), vec![125, 250, 500, 1000]);
    // readLegacyReportRate: round(1000 / reply[3])
    assert_eq!(lg::decode_legacy_rate(&reply(&[0xFF, 0x0D, 0x15, 0x02])), Some(500));
    assert_eq!(lg::decode_legacy_rate(&reply(&[0xFF, 0x0D, 0x15, 0x03])), Some(333));
    assert_eq!(lg::decode_legacy_rate(&reply(&[0xFF, 0x0D, 0x15, 0x00])), None);
    // the "rate changed" notification: report[0] = device, [1] = the 0x8061 index, [2] = 0x00, [3] = 0x01, [4] = index
    assert_eq!(lg::decode_rate_notification(&reply(&[0x01, 0x0C, 0x00, 0x01, 0x04]), 0x01, 0x0C), Some(2000));
    assert_eq!(lg::decode_rate_notification(&reply(&[0x01, 0x0C, 0x00, 0x02, 0x04]), 0x01, 0x0C), None);
}

#[test]
fn battery_answers_decode_like_openmouse() {
    // 0x1004: percent = reply[3] (≤ 100), state = decodeUnifiedBatteryState(reply[5])
    let b = lg::decode_unified_battery(&reply(&[0x02, 0x06, 0x15, 80, 0x00, 0x01]));
    assert_eq!((b.percent, b.charging), (Some(80), Some(true)));
    assert_eq!(lg::decode_unified_battery(&reply(&[0x02, 0x06, 0x15, 80, 0, 0x02])).charging, Some(true), "2 = charging slowly");
    assert_eq!(lg::decode_unified_battery(&reply(&[0x02, 0x06, 0x15, 100, 0, 0x03])).charging, Some(false), "3 = full");
    assert_eq!(lg::decode_unified_battery(&reply(&[0x02, 0x06, 0x15, 50, 0, 0x04])).charging, None, "4 = a charging fault");
    assert_eq!(lg::decode_unified_battery(&reply(&[0x02, 0x06, 0x15, 0xFF])).percent, None);
    // 0x1000: percent = reply[3], state = decodeBatteryLevelState(reply[5]) - 2 "almost full" and 4 "charging slowly"
    let b = lg::decode_battery_level_status(&reply(&[0x02, 0x06, 0x05, 50, 0x00, 0x02]));
    assert_eq!((b.percent, b.charging), (Some(50), Some(true)));
    assert_eq!(lg::decode_battery_level_status(&reply(&[0x02, 0x06, 0x05, 0, 0, 0x00])).percent, Some(0), "0 with a status is a real reading");
    assert_eq!(lg::decode_battery_level_status(&reply(&[0x02, 0x06, 0x05, 50, 0, 0x05])).charging, None, "5 = invalid battery");
    // 0x1001: mV = reply[3] << 8 | reply[4] = 0x0F64 = 3940; flags reply[5]. Curve: 3922 → 70, 3989 → 80:
    // 70 + 10 × (3940 − 3922) / (3989 − 3922) = 70 + 180 / 67 = 72.69 → 73
    let b = lg::decode_battery_voltage(&reply(&[0x02, 0x06, 0x05, 0x0F, 0x64, 0x80]));
    assert_eq!((b.percent, b.charging, b.voltage_mv), (Some(73), Some(true), Some(3940)));
    // charging with flags & 3 == 2 = "Full"; 0x10 = "Charging slowly"; no flag = discharging
    assert_eq!(lg::decode_battery_voltage(&reply(&[0x02, 0x06, 0x05, 0x10, 0x5A, 0x82])).charging, Some(false));
    assert_eq!(lg::decode_battery_voltage(&reply(&[0x02, 0x06, 0x05, 0x0F, 0x64, 0x10])).charging, Some(true));
    assert_eq!(lg::decode_battery_voltage(&reply(&[0x02, 0x06, 0x05, 0x0F, 0x64, 0x00])).charging, Some(false));
    assert_eq!(lg::decode_battery_voltage(&reply(&[0x02, 0x06, 0x05])).percent, None, "0 mV = no reading");
    // estimateBatteryPercent's ends: ≥ 4186 → 100, ≤ 3500 → 0; 3811 is a curve point (50)
    assert_eq!((lg::estimate_battery_percent(4200), lg::estimate_battery_percent(3500), lg::estimate_battery_percent(3811)), (100, 0, 50));
    // 0x1f20: flags bit 0 = valid, bit 1 = charging; 0x0EE2 = 3810 mV → between 3778 (40) and 3811 (50):
    // 40 + 10 × 32 / 33 = 49.7 → 50
    let b = lg::decode_adc_measurement(&reply(&[0x02, 0x06, 0x05, 0x0E, 0xE2, 0x03]));
    assert_eq!((b.percent, b.charging), (Some(50), Some(true)));
    assert_eq!(lg::decode_adc_measurement(&reply(&[0x02, 0x06, 0x05, 0x0E, 0xE2, 0x02])).percent, None, "not valid");
}

#[test]
fn identity_and_profile_answers_decode_like_openmouse() {
    // readIdentity: payload = reply.slice(3); payload[6] transport flags; model bytes payload[7..13] hand out ids in the
    // order Bluetooth, BLE, Wireless, USB. Flags 0x0C = Wireless + USB: Wireless = 40 9F, USB = C0 9B
    // reply[3] = payload[0], unit id reply[4..8], payload[5] = reply[8], flags payload[6] = reply[9], model reply[10..16]
    let r = reply(&[0xFF, 0x02, 0x05, 0x03, 0x11, 0x22, 0x33, 0x44, 0x00, 0x0C, 0x40, 0x9F, 0xC0, 0x9B, 0x00, 0x00]);
    let ids = lg::decode_identity(&r);
    assert_eq!(ids, vec![(Transport::Wireless, 0x409F), (Transport::Usb, 0xC09B)]);
    // isWiredHidppConnection: the active pid matches the USB id → wired; a receiver's pid matches none → the direct index
    assert!(lg::is_wired_connection(0xC09B, &ids, false));
    assert!(!lg::is_wired_connection(0xC547, &ids, false));
    assert!(lg::is_wired_connection(0xC547, &[], true));
    // parseProfilesInfo: profileFormatId = reply[4]
    assert_eq!(lg::decode_profile_format(&reply(&[0x01, 0x0E, 0x05, 0x01, 0x07])), 7);
}

#[test]
fn format_limits_are_onboard_profiles_js() {
    // reportRates: formats 1-4 { 1000, 1000 }, 7 and 8 { wireless 8000, wired 1000 }; C0A9's cable gets the wireless one
    assert_eq!((lg::rate_ceiling(Some(7), None, false), lg::rate_ceiling(Some(7), None, true)), (Some(8000), Some(1000)));
    assert_eq!(lg::rate_ceiling(Some(8), Some(0xC0A9), true), Some(8000));
    assert_eq!(lg::rate_ceiling(Some(2), None, false), Some(1000));
    assert_eq!((lg::rate_ceiling(None, None, true), lg::rate_ceiling(Some(5), None, true)), (None, None));
    // profileRateWritable: WRITABLE_FORMATS {2, 3, 4, 7, 8} with a reportRates entry
    assert!([2, 3, 4, 7, 8].iter().all(|f| lg::profile_rate_writable(Some(*f))));
    assert!(!lg::profile_rate_writable(Some(1)) && !lg::profile_rate_writable(Some(5)) && !lg::profile_rate_writable(None));
    // LOD_ENCODING { Low: 1, Medium: 2, High: 3 } on formats 7 / 8; the 0x2202 override { Low: 0, Medium: 1, High: 2 } else
    assert_eq!((lg::lod_codes(Some(7)), lg::lod_codes(Some(8))), ([1, 2, 3], [1, 2, 3]));
    assert_eq!((lg::lod_codes(None), lg::lod_codes(Some(2)), lg::lod_codes(Some(4))), ([0, 1, 2], [0, 1, 2], [0, 1, 2]));
    // the legacy grid: format 2 { 100, 25600, 50 }
    assert_eq!(lg::legacy_dpi_grid(Some(2)), Some((100, 25600, 50)));
    assert_eq!(lg::legacy_dpi_grid(Some(6)), None);
}

// ---------------------------------------------------------------- device table, interface

fn long_col(pid: u16) -> HidInfo {
    hid_info("long", 0x046D, pid, 0xFF00, 0x02, (20, 20, 0))
}

#[test]
fn table_is_the_supported_list() {
    let short = hid_info("short", 0x046D, 0xC08B, 0xFF00, 0x01, (7, 7, 0));
    let long = long_col(0xC08B);
    let m = lg::matches(&[&short, &long], 0x046D, 0xC08B).unwrap();
    assert_eq!((m.family, m.model, m.wireless, m.config.path.as_str()), (Family::Logitech, Some("Logitech G502 HERO (wired)"), Some(false), "long"));
    assert!(lg::matches(&[&short], 0x046D, 0xC08B).is_none(), "a short-only collection is not claimed");
    // shared receivers name no model
    for pid in [0xC548, 0xC539, 0xC547, 0xC54D] {
        let l = long_col(pid);
        assert_eq!(lg::matches(&[&l], 0x046D, pid).map(|m| (m.model, m.wireless)), Some((None, Some(true))), "{pid:04x}");
    }
    // a pid-less SUPPORTED entry filled from LOGITECH_PRODUCTS for that exact model
    let l = long_col(0xC097);
    assert_eq!(lg::matches(&[&l], 0x046D, 0xC097).unwrap().model, Some("Logitech G303 Shroud Edition"));
    // not "supported": G302 (pending, c07f), G900 (pending, c081), G403 Wireless c082, Unifying c52b; and not Logitech
    for pid in [0xC07F, 0xC081, 0xC082, 0xC52B, 0xC0A9] {
        let l = long_col(pid);
        assert!(lg::matches(&[&l], 0x046D, pid).is_none(), "{pid:04x}");
    }
    let l = long_col(0xC08B);
    assert!(lg::matches(&[&l], 0x3710, 0xC08B).is_none());
    // the Bluetooth page (the PRO X 3 Superstrike's interface) counts, and marks the variant
    let bt = hid_info("bt", 0x046D, 0xC54F, 0xFF43, 0x0202, (20, 20, 0));
    let m = lg::matches(&[&bt], 0x046D, 0xC54F).unwrap();
    assert_eq!((m.model, m.variant), (Some("Logitech PRO X3 SUPERSTRIKE"), lg::VARIANT_BLUETOOTH_PAGE));
    // and the family layer finds it
    assert_eq!(bu_mouse::family::find(&[short.clone(), long.clone()], 0x046D, 0xC08B).map(|m| m.family), Some(Family::Logitech));
}

#[test]
fn index_candidates_are_openmouses() {
    assert_eq!(lg::index_candidates(0xC547), vec![1, 2, 3, 4, 5, 6, 0xFF], "a receiver: every slot first");
    assert_eq!(lg::index_candidates(0xC548), vec![1, 2, 3, 4, 5, 6, 0xFF]);
    assert_eq!(lg::index_candidates(0xC08B), vec![0xFF, 1], "a cabled mouse: itself, then slot 1");
}

// ---------------------------------------------------------------- allow-list

#[test]
fn allow_list_takes_reads_on_learned_features_only() {
    lg::forget_learned();
    // root getFeature always passes
    assert!(lg::read_request_allowed(&lg::get_feature(0x01, 0x2202)));
    // before the mouse named index 9, nothing on it passes - not even a read
    assert!(!lg::read_request_allowed(&lg::read_dpi(0x01, 9)));
    lg::learn_from_answer(&lg::get_feature(0x01, 0x2202), &om_long(&[0x01, 0x00, 0x05, 0x09]));
    lg::learn_from_answer(&lg::get_feature(0x01, 0x8100), &om_long(&[0x01, 0x00, 0x05, 0x0E]));
    lg::learn_from_answer(&lg::get_feature(0x01, 0x8061), &om_long(&[0x01, 0x00, 0x05, 0x0C]));
    lg::learn_from_answer(&lg::get_feature(0x01, 0x8060), &om_long(&[0x01, 0x00, 0x05, 0x00]));
    for r in [lg::read_dpi(1, 9), lg::dpi_capabilities(1, 9), lg::dpi_list_page(1, 9, 3), lg::profiles_info(1, 0x0E), lg::get_onboard_mode(1, 0x0E), lg::rate_list(1, 0x0C), lg::read_rate(1, 0x0C)] {
        assert!(lg::read_request_allowed(&r), "{r:02x?}");
    }
    // every write is refused
    assert!(!lg::read_request_allowed(&lg::set_dpi(1, 9, 800, 800, 2)));
    assert!(!lg::read_request_allowed(&lg::set_host_mode(1, 0x0E)), "setMode is function 1 on 0x8100");
    assert!(!lg::read_request_allowed(&lg::set_rate(1, 0x0C, 1000).unwrap()));
    // the same function on another feature: 0x2202 function 1 (caps) is a read only with sensor 0
    assert!(!lg::read_request_allowed(&om_long(&[0x01, 0x09, 0x15, 0x02])));
    // 0x8060 had index 0 (absent) - not learned; the same bytes on another device index are not learned either
    assert!(!lg::read_request_allowed(&lg::read_dpi(0x02, 9)));
    // malformed: short report, wrong software id, bad device index, too short, trailing bytes
    assert!(!lg::read_request_allowed(&[0x10, 0x01, 0x00, 0x05, 0x22, 0x02, 0x00]));
    assert!(!lg::read_request_allowed(&om_long(&[0x01, 0x09, 0x51])));
    assert!(!lg::read_request_allowed(&om_long(&[0x07, 0x00, 0x05, 0x22, 0x02])));
    assert!(!lg::read_request_allowed(&lg::read_dpi(1, 9)[..19]));
    let mut t = lg::read_dpi(1, 9);
    t[19] = 1;
    assert!(!lg::read_request_allowed(&t));
    // an index two answers named differently is refused
    lg::learn_from_answer(&lg::get_feature(0x01, 0x1004), &om_long(&[0x01, 0x00, 0x05, 0x09]));
    assert!(!lg::read_request_allowed(&lg::read_dpi(1, 9)));
    lg::forget_learned();
}

// ---------------------------------------------------------------- a scripted HID++ mouse

/// One HID++ device behind an index: its features (index = position; 0 = root) and state.
#[derive(Clone, Default)]
struct Dev {
    features: Vec<u16>,
    dpi: (u16, u16, u8),
    caps_flags: u8,
    /// 0x2202 list bytes (as the pages carry them) / 0x2201 list bytes
    dpi_list: Vec<u8>,
    rates: u16,
    rate: u8,
    legacy_bitmap: u8,
    legacy_ms: u8,
    legacy_rate_rejects: bool,
    mode: u8,
    refuses_host: bool,
    /// the 0x2202 set DPI is rejected (invalid argument)
    dpi_set_rejects: bool,
    format: u8,
    battery: (u8, u8),
    /// 0x0003 identity payload after reply[3]
    identity: Vec<u8>,
}

struct FakeHidpp {
    devs: Vec<(u8, Dev)>,
}

impl FakeHidpp {
    fn answer(&mut self, out: &[u8], how: &HidTransfer) -> bu_mouse::error::Result<Vec<u8>> {
        assert!(matches!(how, HidTransfer::OutputThenMatch { .. }), "{how:?}");
        assert_eq!((out.len(), out[0], out[3] & 0x0F), (20, 0x11, 0x05), "a long request with software id 5");
        let (dev, idx, f, p) = (out[1], out[2], out[3] & 0xF0, out[4..].to_vec());
        // an empty slot: the receiver's HID++ 1.0 error comes as a short report on the other collection - nothing here
        let Some(d) = self.devs.iter_mut().find(|d| d.0 == dev).map(|d| &mut d.1) else {
            return Err(bu_mouse::error::Error::BadAnswer("the mouse did not answer in time".into()));
        };
        let mut a = vec![0u8; 20];
        a[..4].copy_from_slice(&out[..4]);
        let err = |code: u8| {
            let mut e = vec![0u8; 20];
            e[..6].copy_from_slice(&[0x11, dev, 0xFF, idx, out[3], code]);
            Ok(e)
        };
        if idx == 0 {
            let id = u16::from_be_bytes([p[0], p[1]]);
            a[4] = d.features.iter().position(|x| *x == id).unwrap_or(0) as u8;
            return Ok(a);
        }
        let Some(&id) = d.features.get(idx as usize) else { return err(0x06) };
        match (id, f) {
            (0x0003, 0x00) => a[4..4 + d.identity.len()].copy_from_slice(&d.identity),
            (0x1004, 0x10) => (a[4], a[6]) = d.battery,
            (0x2202, 0x50) => {
                a[5..7].copy_from_slice(&d.dpi.0.to_be_bytes());
                a[9..11].copy_from_slice(&d.dpi.1.to_be_bytes());
                a[13] = d.dpi.2;
            }
            (0x2202, 0x10) => a[6] = d.caps_flags,
            (0x2202, 0x20) => {
                let page = p[2] as usize;
                for (i, b) in d.dpi_list.iter().skip(page * 13).take(13).enumerate() {
                    a[7 + i] = *b;
                }
            }
            (0x2202, 0x60) if d.dpi_set_rejects => return err(0x02),
            (0x2202, 0x60) => d.dpi = (u16::from_be_bytes([p[1], p[2]]), u16::from_be_bytes([p[3], p[4]]), p[5]),
            (0x2201, 0x20) => a[5..7].copy_from_slice(&d.dpi.0.to_be_bytes()),
            (0x2201, 0x10) => a[5..5 + d.dpi_list.len()].copy_from_slice(&d.dpi_list),
            (0x2201, 0x30) => d.dpi.0 = u16::from_be_bytes([p[1], p[2]]),
            (0x8061, 0x10) => a[4..6].copy_from_slice(&d.rates.to_be_bytes()),
            (0x8061, 0x20) => a[4] = d.rate,
            (0x8061, 0x30) => {
                d.rate = p[0];
                // the "rate changed" notification (software id 0); the acknowledgement is skipped by the transfer
                return Ok([0x11, dev, idx, 0x00, 0x01, p[0]].iter().copied().chain(std::iter::repeat(0)).take(20).collect());
            }
            (0x8060, 0x00) => a[4] = d.legacy_bitmap,
            (0x8060, 0x10) => a[4] = d.legacy_ms,
            (0x8060, 0x20) if d.legacy_rate_rejects => return err(0x02),
            (0x8060, 0x20) => d.legacy_ms = p[0],
            (0x8100, 0x00) => a[5] = d.format,
            (0x8100, 0x20) => a[4] = d.mode,
            (0x8100, 0x10) if d.refuses_host => return err(0x09),
            (0x8100, 0x10) => d.mode = p[0],
            _ => return err(0x07),
        }
        Ok(a)
    }
}

fn port_for(info: HidInfo, devs: Vec<(u8, Dev)>) -> (ScriptPort, Rc<RefCell<FakeHidpp>>) {
    let fake = Rc::new(RefCell::new(FakeHidpp { devs }));
    let f2 = fake.clone();
    (ScriptPort::new(info, move |out, how| f2.borrow_mut().answer(out, how)), fake)
}

fn sent(port: &ScriptPort) -> Vec<Vec<u8>> {
    port.sent.iter().map(|s| s.0.clone()).collect()
}

/// A PRO X SUPERLIGHT 2 on its cable (c09b, format 7): 0x2202 with lift-off, 0x8061 to 8 kHz (1 kHz on the cable).
fn superlight2() -> Dev {
    Dev {
        // 0 root, 1 feature set, 2 firmware, 3 name, 4 unified battery, 5 extended DPI, 6 extended rate, 7 onboard profiles
        features: vec![0x0000, 0x0001, 0x0003, 0x0005, 0x1004, 0x2202, 0x8061, 0x8100],
        dpi: (800, 800, 2),
        caps_flags: 0x02,
        dpi_list: vec![0x00, 0x64, 0xE0, 0x32, 0x7D, 0x00, 0x00, 0x00],
        rates: 0x7F,
        rate: 3,
        mode: 1,
        format: 7,
        battery: (64, 1),
        // payload[0..13]: [0], unit id [1..5], [5], flags [6] = 0x0C (Wireless + USB), model [7..13]: 40 9F | C0 9B | 00 00
        identity: vec![0x03, 0x11, 0x22, 0x33, 0x44, 0x00, 0x0C, 0x40, 0x9F, 0xC0, 0x9B, 0x00, 0x00],
        ..Default::default()
    }
}

/// The cabled Superlight 2: a DPI change ends in exactly OpenMouse's setDpi (no host control on a direct mouse).
#[test]
fn cabled_extended_dpi_change_is_openmouses_setdpi() {
    lg::forget_learned();
    let info = long_col(0xC09B);
    let mt = lg::matches(&[&info], 0x046D, 0xC09B).unwrap();
    let (mut port, fake) = port_for(info, vec![(0xFF, superlight2())]);
    let r = lg::Logitech.read(&mt, &mut port, ReadOptions::default()).unwrap();
    // lift-off is not offered (Logitech names levels only); the identity's USB id is the connected c09b → wired
    assert_eq!((r.online, r.dpi, r.lift_off, r.battery_percent, r.charging, r.wireless), (true, Some(800), None, Some(64), Some(true), Some(false)));
    // format 7 on its cable: the 1 kHz ceiling
    assert_eq!(r.polling_hz, Some(1000));
    assert_eq!(r.caps.polling, vec![125, 250, 500, 1000], "format 7 is profile-writable, so the cabled rate is offered (live write first)");
    assert!(r.caps.lift_off.is_empty(), "no millimetres from Logitech: no lift-off chips");
    assert_eq!(r.caps.dpi.as_ref().map(|d| (d.min(), d.max())), Some((100, 32000)));
    assert!(port.sent.iter().all(|s| lg::read_request_allowed(&s.0)), "reading sends read requests only");
    port.sent.clear();
    let c = lg::Logitech.write(&mt, &mut port, Setting::Dpi(1600)).unwrap();
    assert_eq!((c.now, c.before), (1600, Some(800)));
    let s = sent(&port);
    let n = s.len();
    // OpenMouse's setDpi on a direct mouse: (ensureHostControl returns at once) getFeature(0x2202), readDpiConfiguration,
    // requestLong(0x60, [0, 0x06, 0x40, 0x06, 0x40, current.lod = 2]), readDpiConfiguration
    assert_eq!(
        s[n - 4..].to_vec(),
        vec![om_long(&[0xFF, 0x00, 0x05, 0x22, 0x02]), om_long(&[0xFF, 0x05, 0x55]), om_long(&[0xFF, 0x05, 0x65, 0x00, 0x06, 0x40, 0x06, 0x40, 0x02]), om_long(&[0xFF, 0x05, 0x55])]
    );
    assert!(s[..n - 4].iter().all(|q| lg::read_request_allowed(q)), "what comes before is finding the mouse and its DPI list - reads");
    assert!(!lg::read_request_allowed(&s[n - 2]));
    assert_eq!(fake.borrow().devs[0].1.dpi, (1600, 1600, 2), "the lift-off byte is kept");
    // the DPI list: 100..32000 in 50s; 1610 is not on it
    assert!(lg::Logitech.write(&mt, &mut port, Setting::Dpi(1610)).is_err());
    lg::forget_learned();
}

/// A Lightspeed receiver (c547) with a keyboard on slot 1 and the mouse on slot 2, running its onboard profile: a polling
/// change = OpenMouse's ensureHostControl (getFeature 0x8100, getMode, setMode 2, getMode) + setExtendedReportRate.
#[test]
fn receiver_polling_change_takes_host_control_then_sets_the_rate() {
    lg::forget_learned();
    let info = long_col(0xC547);
    let mt = lg::matches(&[&info], 0x046D, 0xC547).unwrap();
    let keyboard = Dev { features: vec![0x0000, 0x0001, 0x0003, 0x1B04], ..Default::default() };
    let (mut port, fake) = port_for(info, vec![(0x01, keyboard), (0x02, superlight2())]);
    let r = lg::Logitech.read(&mt, &mut port, ReadOptions::default()).unwrap();
    // the receiver's pid matches no transport id and slot 2 is not the direct index → wireless; format 7 wireless = 8 kHz
    assert_eq!((r.wireless, r.polling_hz), (Some(true), Some(1000)));
    assert_eq!(r.caps.polling, vec![125, 250, 500, 1000, 2000, 4000, 8000]);
    // the probe: slot 1 answers but has no DPI feature (a keyboard), slot 2 does
    assert_eq!(
        sent(&port)[..5].to_vec(),
        vec![
            om_long(&[0x01, 0x00, 0x05, 0x00, 0x03]),
            om_long(&[0x01, 0x00, 0x05, 0x22, 0x02]),
            om_long(&[0x01, 0x00, 0x05, 0x22, 0x01]),
            om_long(&[0x02, 0x00, 0x05, 0x00, 0x03]),
            om_long(&[0x02, 0x00, 0x05, 0x22, 0x02]),
        ]
    );
    assert!(port.sent.iter().all(|s| lg::read_request_allowed(&s.0)));
    port.sent.clear();
    let c = lg::Logitech.write(&mt, &mut port, Setting::Polling(4000)).unwrap();
    assert_eq!((c.now, c.before), (4000, Some(1000)));
    let s = sent(&port);
    let n = s.len();
    assert_eq!(
        s[n - 5..].to_vec(),
        vec![
            om_long(&[0x02, 0x00, 0x05, 0x81, 0x00]),
            om_long(&[0x02, 0x07, 0x25]),
            om_long(&[0x02, 0x07, 0x15, 0x02]),
            om_long(&[0x02, 0x07, 0x25]),
            // request(feature, 0x30, REPORT_RATE_HZ.indexOf(4000) = 5)
            om_long(&[0x02, 0x06, 0x35, 0x05]),
        ]
    );
    assert!(s[..n - 5].iter().all(|q| lg::read_request_allowed(q)));
    // the rate write waits for the notification [0x11, 2, 6, 0x00, 0x01, 5] or an error - not the plain acknowledgement
    match &port.sent[n - 1].1 {
        HidTransfer::OutputThenMatch { prefixes, .. } => assert_eq!(prefixes[0], vec![0x11, 0x02, 0x06, 0x00, 0x01, 0x05]),
        o => panic!("{o:?}"),
    }
    assert_eq!(fake.borrow().devs[1].1.mode, 2);
    // lift-off is not offered for Logitech (levels only, no millimetres): refused before anything is written
    port.sent.clear();
    assert!(lg::Logitech.write(&mt, &mut port, Setting::LiftOff(7)).is_err());
    assert!(port.sent.iter().all(|s| lg::read_request_allowed(&s.0)), "only the reads that find the mouse");
    lg::forget_learned();
}

/// Order 104 review: a receiver mouse switched to host mode for a DPI write that it then rejects is put back into onboard
/// mode (OpenMouse's setOnboardMode("Onboard"): setMode 1) - never left half-changed. Undo's DPI is the one read before
/// the switch.
#[test]
fn a_failed_write_puts_a_switched_mouse_back_into_onboard_mode() {
    lg::forget_learned();
    let info = long_col(0xC547);
    let mt = lg::matches(&[&info], 0x046D, 0xC547).unwrap();
    let mut mouse = superlight2();
    mouse.dpi_set_rejects = true;
    let (mut port, fake) = port_for(info, vec![(0x01, mouse)]);
    assert_eq!(fake.borrow().devs[0].1.mode, 1, "running its onboard profile");
    assert!(lg::Logitech.write(&mt, &mut port, Setting::Dpi(1600)).is_err());
    assert_eq!(fake.borrow().devs[0].1.mode, 1, "back in onboard mode");
    let writes: Vec<Vec<u8>> = sent(&port).into_iter().filter(|q| !lg::read_request_allowed(q)).collect();
    // setMode 2 (index 7, function 1, sw id 5), the rejected set DPI, setMode 1
    assert_eq!(writes.first(), Some(&om_long(&[0x01, 0x07, 0x15, 0x02])));
    assert_eq!(writes.last(), Some(&om_long(&[0x01, 0x07, 0x15, 0x01])));
    assert_eq!(writes.len(), 3);
    assert_eq!(fake.borrow().devs[0].1.dpi.0, 800);
    // a write that goes through leaves it in host mode, as OpenMouse does
    fake.borrow_mut().devs[0].1.dpi_set_rejects = false;
    let c = lg::Logitech.write(&mt, &mut port, Setting::Dpi(1600)).unwrap();
    assert_eq!((c.now, c.before, fake.borrow().devs[0].1.mode), (1600, Some(800), 2));
    lg::forget_learned();
}

/// A receiver mouse that will not hand control to software: setMode is refused, and nothing else is written.
#[test]
fn onboard_only_mouse_gets_no_write() {
    lg::forget_learned();
    let info = long_col(0xC539);
    let mt = lg::matches(&[&info], 0x046D, 0xC539).unwrap();
    let mut mouse = superlight2();
    mouse.refuses_host = true;
    let (mut port, fake) = port_for(info, vec![(0x01, mouse)]);
    let e = lg::Logitech.write(&mt, &mut port, Setting::Dpi(1600)).unwrap_err();
    assert!(e.to_string().contains("onboard memory"), "{e}");
    let writes: Vec<Vec<u8>> = sent(&port).into_iter().filter(|q| !lg::read_request_allowed(q)).collect();
    assert_eq!(writes, vec![om_long(&[0x01, 0x07, 0x15, 0x02])], "only the refused setMode");
    assert_eq!(fake.borrow().devs[0].1.dpi.0, 800);
    lg::forget_learned();
}

/// A cabled G502 HERO (c08b, format 2): legacy 0x2201 DPI and 0x8060 rate, written live (OpenMouse's setLegacyDpi /
/// setLegacyReportRate; no host control on a direct mouse); no lift-off control.
#[test]
fn cabled_legacy_dpi_and_rate_changes_are_openmouses() {
    lg::forget_learned();
    let info = long_col(0xC08B);
    let mt = lg::matches(&[&info], 0x046D, 0xC08B).unwrap();
    let g502 = Dev {
        // 0 root, 1 feature set, 2 firmware, 3 adjustable DPI, 4 report rate, 5 onboard profiles
        features: vec![0x0000, 0x0001, 0x0003, 0x2201, 0x8060, 0x8100],
        dpi: (2400, 2400, 0),
        // 100, 0xE000 | 50, 25600
        dpi_list: vec![0x00, 0x64, 0xE0, 0x32, 0x64, 0x00],
        legacy_bitmap: 0x8B,
        legacy_ms: 1,
        mode: 1,
        format: 2,
        identity: vec![0x02, 0, 0, 0, 0, 0, 0x08, 0xC0, 0x8B, 0, 0, 0, 0],
        ..Default::default()
    };
    let (mut port, fake) = port_for(info, vec![(0xFF, g502)]);
    let r = lg::Logitech.read(&mt, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((r.dpi, r.polling_hz, r.lift_off, r.wireless, r.caps.battery), (Some(2400), Some(1000), None, Some(false), false));
    assert_eq!((r.caps.polling.clone(), r.caps.lift_off.clone()), (vec![125, 250, 500, 1000], vec![]));
    assert!(port.sent.iter().all(|s| lg::read_request_allowed(&s.0)));
    port.sent.clear();
    let c = lg::Logitech.write(&mt, &mut port, Setting::Dpi(3200)).unwrap();
    assert_eq!((c.now, c.before), (3200, Some(2400)));
    let s = sent(&port);
    let n = s.len();
    // setLegacyDpi: request(idx, 0x30, 0x00, 3200 >> 8 = 0x0C, 0x80), readLegacyDpi request(idx, 0x20, 0x00)
    assert_eq!(s[n - 2..].to_vec(), vec![om_long(&[0xFF, 0x03, 0x35, 0x00, 0x0C, 0x80]), om_long(&[0xFF, 0x03, 0x25, 0x00])]);
    assert!(s[..n - 2].iter().all(|q| lg::read_request_allowed(q)));
    port.sent.clear();
    let c = lg::Logitech.write(&mt, &mut port, Setting::Polling(500)).unwrap();
    assert_eq!((c.now, c.before), (500, Some(1000)));
    let s = sent(&port);
    let n = s.len();
    // setLegacyReportRate: request(idx, 0x20, round(1000 / 500) = 2), readLegacyReportRate request(idx, 0x10)
    assert_eq!(s[n - 2..].to_vec(), vec![om_long(&[0xFF, 0x04, 0x25, 0x02]), om_long(&[0xFF, 0x04, 0x15])]);
    assert!(s[..n - 2].iter().all(|q| lg::read_request_allowed(q)));
    assert!(lg::Logitech.write(&mt, &mut port, Setting::LiftOff(10)).is_err(), "legacy DPI has no lift-off");
    // a mouse that rejects the live rate write: OpenMouse would fall back to a profile-flash write - here it is an error
    fake.borrow_mut().devs[0].1.legacy_rate_rejects = true;
    let e = lg::Logitech.write(&mt, &mut port, Setting::Polling(250)).unwrap_err();
    assert!(e.to_string().contains("onboard profile"), "{e}");
    assert_eq!(fake.borrow().devs[0].1.legacy_ms, 2);
    lg::forget_learned();
}

/// A cabled mouse on a profile format OpenMouse does not write: its polling rate is read, not offered.
#[test]
fn cabled_rate_on_an_unwritten_format_is_read_only() {
    lg::forget_learned();
    let info = long_col(0xC094);
    let mt = lg::matches(&[&info], 0x046D, 0xC094).unwrap();
    let mut d = superlight2();
    d.format = 5;
    let (mut port, _fake) = port_for(info, vec![(0xFF, d)]);
    let r = lg::Logitech.read(&mt, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((r.polling_hz, r.caps.polling.len()), (Some(1000), 0));
    port.sent.clear();
    assert!(lg::Logitech.write(&mt, &mut port, Setting::Polling(500)).is_err());
    assert!(port.sent.iter().all(|s| lg::read_request_allowed(&s.0)), "the refused write sent reads only");
    assert_eq!(r.lift_off, None);
    lg::forget_learned();
}

/// An MX Master 3S on a Bolt receiver's slot 3: battery and DPI, no report-rate feature and no lift-off.
#[test]
fn bolt_mouse_reads_battery_and_dpi() {
    lg::forget_learned();
    let info = long_col(0xC548);
    let mt = lg::matches(&[&info], 0x046D, 0xC548).unwrap();
    let mx = Dev {
        features: vec![0x0000, 0x0001, 0x0003, 0x1004, 0x2202],
        dpi: (1000, 1000, 0),
        caps_flags: 0x00,
        // 200, 0xE000 | 50, 8000
        dpi_list: vec![0x00, 0xC8, 0xE0, 0x32, 0x1F, 0x40, 0x00, 0x00],
        battery: (45, 0),
        ..Default::default()
    };
    let (mut port, _fake) = port_for(info, vec![(0x03, mx)]);
    let r = lg::Logitech.read(&mt, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((r.dpi, r.battery_percent, r.charging, r.polling_hz, r.lift_off), (Some(1000), Some(45), Some(false), None, None));
    assert!(r.caps.polling.is_empty() && r.caps.lift_off.is_empty() && r.caps.battery);
    assert_eq!(r.caps.dpi.unwrap().bands, vec![(200, 8000, 50)]);
    // slots 1 and 2 were asked first and stayed silent
    assert_eq!(sent(&port)[..3].to_vec(), vec![om_long(&[0x01, 0x00, 0x05, 0x00, 0x03]), om_long(&[0x02, 0x00, 0x05, 0x00, 0x03]), om_long(&[0x03, 0x00, 0x05, 0x00, 0x03])]);
    assert!(port.sent.iter().all(|s| lg::read_request_allowed(&s.0)));
    assert!(lg::Logitech.write(&mt, &mut port, Setting::Polling(1000)).is_err());
    lg::forget_learned();
}

/// Nobody answers (the mouse is asleep): offline after one probe per index; a write says so.
#[test]
fn a_silent_receiver_reads_as_offline() {
    lg::forget_learned();
    let info = long_col(0xC539);
    let mt = lg::matches(&[&info], 0x046D, 0xC539).unwrap();
    let (mut port, _fake) = port_for(info, vec![]);
    let r = lg::Logitech.read(&mt, &mut port, ReadOptions { online_tries: 1 }).unwrap();
    assert!(!r.online && r.dpi.is_none());
    assert_eq!(port.sent.len(), 7);
    assert!(port.sent.iter().all(|s| lg::read_request_allowed(&s.0)));
    assert!(matches!(lg::Logitech.write(&mt, &mut port, Setting::Dpi(800)), Err(bu_mouse::error::Error::MouseGone(_))));
}

/// A cabled id whose direct index answers without a sensor (the PRO X Superlight's pass-through endpoint): slot 1 is the
/// mouse; a device that answers everywhere without a sensor is not a mouse.
#[test]
fn a_sensorless_direct_index_falls_through_to_slot_one() {
    lg::forget_learned();
    let info = long_col(0xC094);
    let mt: Matched = lg::matches(&[&info], 0x046D, 0xC094).unwrap();
    let admin = Dev { features: vec![0x0000, 0x0001, 0x0003], ..Default::default() };
    let (mut port, _fake) = port_for(info.clone(), vec![(0xFF, admin.clone()), (0x01, superlight2())]);
    let r = lg::Logitech.read(&mt, &mut port, ReadOptions::default()).unwrap();
    assert_eq!(r.dpi, Some(800));
    // slot 1 is not the direct index → treated like a receiver mouse (identity: 0xC094 is none of its ids)
    assert_eq!(r.wireless, Some(true));
    let (mut port, _fake) = port_for(info, vec![(0xFF, admin.clone()), (0x01, admin)]);
    assert!(matches!(lg::Logitech.read(&mt, &mut port, ReadOptions::default()), Err(bu_mouse::error::Error::UnsupportedMouse(_))));
    lg::forget_learned();
}

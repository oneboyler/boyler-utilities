//! The byte parsers behind drive health, on made-up buffers laid out the way Windows / the drives return them.
#![cfg(windows)]

use bu_storage::real::{parse_nvme_health_log, parse_nvme_protocol_data, parse_smart, parse_temperature_descriptor};

#[test]
fn nvme_health_log_fields() {
    let mut log = [0u8; 512];
    log[0] = 0x04; // reliability degraded
    log[1..3].copy_from_slice(&(273u16 + 51).to_le_bytes());
    log[3] = 100;
    log[4] = 10;
    log[5] = 23; // 23 % used
    log[128..136].copy_from_slice(&33_335u64.to_le_bytes());
    log[144..152].copy_from_slice(&120u64.to_le_bytes());
    log[160..168].copy_from_slice(&453u64.to_le_bytes());
    let h = parse_nvme_health_log(&log).unwrap();
    assert_eq!((h.critical_warning, h.temperature_kelvin, h.percentage_used), (0x04, 324, 23));
    assert_eq!((h.available_spare_pct, h.available_spare_threshold_pct), (100, 10));
    assert_eq!((h.power_on_hours, h.unsafe_shutdowns, h.media_errors), (33_335, 120, 453));
    assert!(parse_nvme_health_log(&log[..100]).is_none());

    // Wrapped in STORAGE_PROTOCOL_DATA_DESCRIPTOR: 8 bytes header + 40-byte spec (offset at +16, length at +20).
    let mut out = vec![0u8; 8 + 40 + 512];
    out[8 + 16..8 + 20].copy_from_slice(&40u32.to_le_bytes());
    out[8 + 20..8 + 24].copy_from_slice(&512u32.to_le_bytes());
    out[48..].copy_from_slice(&log);
    assert_eq!(parse_nvme_protocol_data(&out), Some(h));
    out[8 + 20..8 + 24].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(parse_nvme_protocol_data(&out), None, "no data returned");
}

#[test]
fn temperature_descriptor() {
    let mut b = [0u8; 64];
    b[12..14].copy_from_slice(&1u16.to_le_bytes()); // InfoCount
    b[26..28].copy_from_slice(&37i16.to_le_bytes()); // TemperatureInfo[0].Temperature
    assert_eq!(parse_temperature_descriptor(&b), Some(37));
    b[26..28].copy_from_slice(&0i16.to_le_bytes());
    assert_eq!(parse_temperature_descriptor(&b), None, "0 = not reported");
    b[26..28].copy_from_slice(&500i16.to_le_bytes());
    assert_eq!(parse_temperature_descriptor(&b), None, "out of range");
    b[12..14].copy_from_slice(&0u16.to_le_bytes());
    assert_eq!(parse_temperature_descriptor(&b), None, "no sensors");
    assert_eq!(parse_temperature_descriptor(&b[..20]), None);
}

#[test]
fn sata_smart_table_and_thresholds() {
    let mut data = [0u8; 512];
    let mut put = |slot: usize, id: u8, value: u8, worst: u8, raw: u64| {
        let o = 2 + slot * 12;
        data[o] = id;
        data[o + 3] = value;
        data[o + 4] = worst;
        data[o + 5..o + 11].copy_from_slice(&raw.to_le_bytes()[..6]);
    };
    put(0, 5, 100, 100, 8);
    put(1, 9, 90, 90, 10_000);
    put(3, 194, 63, 40, 0x0014_0000_0029); // slot 2 empty (id 0) is skipped
    let mut th = [0u8; 512];
    th[2] = 5;
    th[3] = 10;
    th[2 + 12] = 194;
    th[2 + 12 + 1] = 0;
    let attrs = parse_smart(&data, Some(&th));
    assert_eq!(attrs.len(), 3);
    assert_eq!((attrs[0].id, attrs[0].value, attrs[0].raw, attrs[0].threshold), (5, 100, 8, Some(10)));
    assert_eq!((attrs[1].id, attrs[1].raw, attrs[1].threshold), (9, 10_000, None));
    assert_eq!((attrs[2].id, attrs[2].worst, attrs[2].raw & 0xFF, attrs[2].threshold), (194, 40, 0x29, Some(0)));
    assert_eq!(parse_smart(&data, None)[0].threshold, None);
}

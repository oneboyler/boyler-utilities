//! Order 105 check, Ninjutso (Sora V2 / Sora V3 / TEN): our requests = the bytes OpenMouse sends for the same setting (encode),
//! and answers shaped the way OpenMouse reads them decode to the same values (decode). The expected bytes are worked out by
//! hand from OpenMouse (@openmouse/protocol 0.40.1: `dist/ninjutso/index.js`, `dist/drivers/ninjutso/hid.js`), never from our
//! own encoder.
//!
//! Current protocol: OpenMouse's `ninjutsoBuildRequest(command, profile, args)` is a 15-byte payload
//! `[command, 0, 0, 1, 0, args.length, profile, ...args]` sent with `sendFeatureReport(6, payload)`; Windows puts the report id in
//! front: our byte i + 1 = OpenMouse's byte i, 16 bytes. An answer is good when `bytes[1] === command`; its value is `bytes.slice(8)`
//! (the report id stays in byte 0 of what `receiveFeatureReport` returns).
//! Legacy protocol: report 5 = 31-byte payload `[command, 0, 0, 1, 0, profile, 22, 0, value]`, report 4 = 703 (73) byte payload
//! `[38, 0, 0, 1, 0, profile]`.

use bu_mouse::error::Error;
use bu_mouse::family::{self, Driver, Family, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::ninjutso as n;
use bu_mouse::os::{HidInfo, HidTransfer};
use std::cell::RefCell;
use std::rc::Rc;

/// A request/answer buffer with the given (index, value) bytes set.
fn buf(len: usize, set: &[(usize, u8)]) -> Vec<u8> {
    let mut v = vec![0u8; len];
    for (i, b) in set {
        v[*i] = *b;
    }
    v
}

fn cfg(vid: u16, pid: u16, feature_len: u16, product: Option<&str>) -> HidInfo {
    let mut h = hid_info(&format!("cfg{pid:04x}"), vid, pid, 0xFF00, 0x01, (0, 0, feature_len));
    h.product = product.map(|s| s.to_string());
    h
}

// ---------------------------------------------------------------- encode: current protocol

#[test]
fn current_requests_are_openmouses() {
    // read profile: ninjutsoBuildRequest(16, 0, []) = [16, 0, 0, 1, 0, 0, 0]
    assert_eq!(n::read_request(16, 1, &[]), buf(16, &[(0, 6), (1, 16), (4, 1)]), "16 is not a profile command: profile byte 0");
    // battery 18 / charging 17: no profile
    assert_eq!(n::read_request(18, 3, &[]), buf(16, &[(0, 6), (1, 18), (4, 1)]));
    assert_eq!(n::read_request(17, 3, &[]), buf(16, &[(0, 6), (1, 17), (4, 1)]));
    // active stage 28 and polling 6 carry the profile: [28, 0, 0, 1, 0, 0, profile]
    assert_eq!(n::read_request(28, 2, &[]), buf(16, &[(0, 6), (1, 28), (4, 1), (7, 2)]));
    assert_eq!(n::read_request(6, 1, &[]), buf(16, &[(0, 6), (1, 6), (4, 1), (7, 1)]));
    // DPI of a stage: [4, 0, 0, 1, 0, 1, profile, stage]
    assert_eq!(n::read_request(4, 1, &[2]), buf(16, &[(0, 6), (1, 4), (4, 1), (6, 1), (7, 1), (8, 2)]));
    // setDpi: [3, 0, 0, 1, 0, 4, profile, stage, lo, hi, 0]; TEN 1600: 1600 / 50 - 1 = 31 = 0x1f
    assert_eq!(n::encode_dpi(1600, false), Some([0x1F, 0, 0]));
    assert_eq!(n::current_request(3, 1, &[0, 0x1F, 0, 0]), buf(16, &[(0, 6), (1, 3), (4, 1), (6, 4), (7, 1), (9, 0x1F)]));
    // Sora V3 sends the DPI as it is: 1600 = 0x0640; 45 000 = 0xafc8
    assert_eq!(n::encode_dpi(1600, true), Some([0x40, 0x06, 0]));
    assert_eq!(n::encode_dpi(45_000, true), Some([0xC8, 0xAF, 0]));
    assert_eq!(n::encode_dpi(1, true), Some([1, 0, 0]));
    assert_eq!(n::encode_dpi(45_001, true), None);
    assert_eq!(n::encode_dpi(0, true), None);
    // TEN: 50 -> index 0; 30 000 -> 599 = 0x0257; not a multiple of 50 / too big -> refused
    assert_eq!(n::encode_dpi(50, false), Some([0, 0, 0]));
    assert_eq!(n::encode_dpi(30_000, false), Some([0x57, 0x02, 0]));
    for d in [0, 25, 825, 30_050] {
        assert_eq!(n::encode_dpi(d, false), None, "{d}");
    }
    // setPollingRate: [5, 0, 0, 1, 0, 1, profile, index + 1]: 1000 -> 1, 2000 -> 2, 4000 -> 3, 8000 -> 4
    for (hz, code) in [(1000, 1u8), (2000, 2), (4000, 3), (8000, 4)] {
        assert_eq!(n::encode_polling(hz), Some(code));
        assert_eq!(n::decode_polling(code), Some(hz));
    }
    assert_eq!(n::encode_polling(500), None);
    assert_eq!(n::decode_polling(0), None);
    assert_eq!(n::decode_polling(5), None);
    assert_eq!(n::current_request(5, 1, &[3]), buf(16, &[(0, 6), (1, 5), (4, 1), (6, 1), (7, 1), (8, 3)]));
    // setControl(false) = report 3, [27, 26, 0, 0, 1] in a 63-byte payload; setControl(true) = [28, 27, 0, 0, 1]
    assert_eq!(n::control_request(false), buf(64, &[(0, 3), (1, 27), (2, 26), (5, 1)]));
    assert_eq!(n::control_request(true), buf(64, &[(0, 3), (1, 28), (2, 27), (5, 1)]));
}

#[test]
fn current_answers_decode_like_openmouse() {
    // ninjutsoResponseValue: at least 9 bytes, byte 1 echoes the command, value = bytes.slice(8)
    let a = buf(64, &[(0, 6), (1, 18), (8, 87)]);
    assert_eq!(n::response_value(&a, 18).map(|v| v[0]), Some(87));
    assert_eq!(n::response_value(&a, 17), None, "another command's answer");
    assert_eq!(n::response_value(&a[..8], 18), None, "shorter than 9 bytes");
    assert_eq!(n::response_value(&a[..9], 18).map(|v| v.to_vec()), Some(vec![87]));
    // ninjutsoDecodeDpi: TEN (0x1f, 0) -> (31 + 1) * 50 = 1600; (0x57, 0x02) -> (599 + 1) * 50 = 30 000; Sora V3 (0x40, 0x06) -> 1600
    assert_eq!(n::decode_dpi(0x1F, 0, false), 1600);
    assert_eq!(n::decode_dpi(0x57, 0x02, false), 30_000);
    assert_eq!(n::decode_dpi(0x40, 0x06, true), 1600);
    assert_eq!(n::decode_dpi(0xC8, 0xAF, true), 45_000);
}

// ---------------------------------------------------------------- encode / decode: legacy protocol

#[test]
fn legacy_requests_are_openmouses() {
    // ninjutsoLegacyRequest(13): [13, 0, 0, 1, 0, 0, 22] in 31 bytes (profile query)
    assert_eq!(n::legacy_request(13, 0, None), buf(32, &[(0, 5), (1, 13), (4, 1), (7, 22)]));
    // with a profile and a value: [5, 0, 0, 1, 0, profile, 22, 0, value]
    assert_eq!(n::legacy_request(11, 2, Some(7)), buf(32, &[(0, 5), (1, 11), (4, 1), (6, 2), (7, 22), (9, 7)]));
    // settings dump: [38, 0, 0, 1, 0, profile] in 703 (73) bytes on report 4
    assert_eq!(n::legacy_data_request(2, 703), buf(704, &[(0, 4), (1, 38), (4, 1), (6, 2)]));
    assert_eq!(n::legacy_data_request(1, 73), buf(74, &[(0, 4), (1, 38), (4, 1), (6, 1)]));
    assert_eq!((n::legacy_data_len(None), n::legacy_data_len(Some("Ninjutso Sora V2")), n::legacy_data_len(Some("Ninjutso Sora V2 8K"))), (703, 703, 73));
    // ninjutsoLegacyEncodeDpi: 800 -> 15, 2400 -> 47, 30 000 -> 599 = 0x0257; decode (v + 1) * 50
    assert_eq!(n::legacy_encode_dpi(800), Some([15, 0]));
    assert_eq!(n::legacy_encode_dpi(2400), Some([47, 0]));
    assert_eq!(n::legacy_encode_dpi(30_000), Some([0x57, 0x02]));
    assert_eq!(n::legacy_encode_dpi(49), None);
    assert_eq!(n::legacy_encode_dpi(30_050), None);
    assert_eq!(n::legacy_decode_dpi(15, 0), 800);
    assert_eq!(n::legacy_decode_dpi(0x57, 0x02), 30_000);
    assert_eq!(n::legacy_decode_dpi(0, 0), 50);
}

/// A settings dump as Windows hands it over (the report id 4 in byte 0, OpenMouse's `slice(9)` data from byte 9): battery data[7],
/// stage count data[11], four DPI slots data[12..20] (raw = (dpi - 50) / 50), active stage data[20]; the checksum is the sum of
/// the first `sum_end` bytes as a little-endian u16 at `sum_end` (701 / 71).
fn dump(len: usize, battery: u8, count: u8, raw: [u8; 4], active: u8) -> Vec<u8> {
    let mut v = buf(len, &[(0, 4), (9 + 7, battery), (9 + 11, count), (9 + 20, active)]);
    for (i, r) in raw.iter().enumerate() {
        v[9 + 12 + i * 2] = *r;
    }
    let sum_end = len - 3;
    let sum: u32 = v[..sum_end].iter().map(|b| *b as u32).sum();
    v[sum_end + 1] = sum as u8;
    v[sum_end + 2] = (sum >> 8) as u8;
    v
}

#[test]
fn the_legacy_checksum_and_fields_are_openmouses() {
    // report id 4 + battery 80 + count 3 + slots 15 / 31 / 63 / 127 + active 1 = 4 + 80 + 3 + 15 + 31 + 63 + 127 + 1 = 324 = 0x0144
    let d = dump(704, 80, 3, [15, 31, 63, 127], 1);
    assert_eq!((d[702], d[703]), (0x44, 0x01));
    let data = n::legacy_settings(&d, 703).unwrap();
    let st = n::LegacyState::from_data(2, &data);
    // stages (v + 1) * 50: 800, 1600, 3200, 6400; count 3, active stage 1 -> 1600; battery 80
    assert_eq!((st.stage_count, st.stage, st.stages, st.dpi(), st.battery, st.profile), (3, 1, [800, 1600, 3200, 6400], 1600, 80, 2));
    // a wrong sum is refused
    let mut bad = d.clone();
    bad[702] = 196;
    assert!(matches!(n::legacy_settings(&bad, 703), Err(Error::BadAnswer(_))));
    // the 8K's 73-byte dump: sum of the first 71 bytes at 72 / 73
    let c = dump(74, 50, 1, [15, 0, 0, 0], 0);
    assert_eq!((c[72], c[73]), (4 + 50 + 1 + 15, 0));
    assert!(n::legacy_settings(&c, 73).is_ok());
    // count 0 -> 1, count 9 -> 4, an active stage past the count is pulled back (Math.min(data[20], count - 1)); battery above 100 -> 100
    let st = n::LegacyState::from_data(1, &n::legacy_settings(&dump(704, 150, 9, [1, 2, 3, 4], 7), 703).unwrap());
    assert_eq!((st.stage_count, st.stage, st.battery), (4, 3, 100));
    let st = n::LegacyState::from_data(1, &n::legacy_settings(&dump(704, 10, 0, [1, 2, 3, 4], 2), 703).unwrap());
    assert_eq!((st.stage_count, st.stage), (1, 0));
}

#[test]
fn the_legacy_dpi_write_is_openmouses() {
    // setLegacyDpi: settings (count 3, slots 800 / 1600 / 3200 / 6400, active 1), the active slot becomes 2400 (raw 47); the
    // request ninjutsoLegacyRequest(5, profile 2) gets [8] = 3 and, from [9], every slot below the count encoded (lo, hi) and 0, 0
    // for the others: [8] = 3, [9..17] = 15 0 | 47 0 | 63 0 | 0 0
    let st = n::LegacyState::from_data(2, &n::legacy_settings(&dump(704, 80, 3, [15, 31, 63, 127], 1), 703).unwrap());
    let w = n::legacy_dpi_write(&st, 2400).unwrap();
    assert_eq!(w, buf(32, &[(0, 5), (1, 5), (4, 1), (6, 2), (7, 22), (9, 3), (10, 15), (12, 47), (14, 63)]));
    assert_eq!(n::legacy_dpi_write(&st, 2410), None);
}

// ---------------------------------------------------------------- the device table

#[test]
fn the_device_table_is_the_supported_list() {
    assert_eq!(n::PRODUCTS.len(), 9);
    // Sora V2 0xae11-0xae16 on 0x1915: report 4 makes the collection's feature report 704 bytes
    for pid in 0xAE11u16..=0xAE16 {
        let info = cfg(0x1915, pid, 704, Some("Ninjutso Sora V2"));
        let m = n::matches(&[&info], 0x1915, pid).unwrap_or_else(|| panic!("{pid:04x}"));
        assert_eq!((m.family, m.model, m.wireless), (Family::Ninjutso, Some("Ninjutso Sora V2"), Some(false)));
        let c = n::caps(n::product(0x1915, pid).unwrap());
        assert_eq!((c.dpi.clone().map(|d| (d.min(), d.max())), c.polling.clone(), c.lift_off.clone(), c.battery), (None, vec![], vec![], true), "a Sora V2's DPI is read, not set");
        assert_eq!(family::find(std::slice::from_ref(&info), 0x1915, pid).unwrap().family, Family::Ninjutso);
    }
    // the "8K" product has the short dump
    assert!(n::matches(&[&cfg(0x1915, 0xAE11, 74, Some("Ninjutso Sora V2 8K"))], 0x1915, 0xAE11).is_some());
    assert!(n::matches(&[&cfg(0x1915, 0xAE11, 704, Some("Ninjutso Sora V2 8K"))], 0x1915, 0xAE11).is_none(), "length and name disagree");
    assert!(n::matches(&[&cfg(0x1915, 0xAE11, 16, None)], 0x1915, 0xAE11).is_none(), "no report 4");
    // Sora V3 0xe010 and TEN 0xe020 / 0xea01 on 0x093a
    for (pid, model, dpi_max, step) in [(0xE010u16, "Ninjutso Sora V3", 45_000u32, 1u32), (0xE020, "Ninjutso TEN", 30_000, 50), (0xEA01, "Ninjutso TEN", 30_000, 50)] {
        let info = cfg(0x093A, pid, 64, None);
        let m = n::matches(&[&info], 0x093A, pid).unwrap_or_else(|| panic!("{pid:04x}"));
        assert_eq!((m.family, m.model, m.wireless), (Family::Ninjutso, Some(model), Some(false)));
        let c = n::caps(n::product(0x093A, pid).unwrap());
        let d = c.dpi.clone().unwrap();
        assert_eq!((d.max(), d.min(), c.polling.clone(), c.lift_off.clone(), c.battery), (dpi_max, step, vec![1000, 2000, 4000, 8000], vec![], true));
        assert!(d.allows(step * 2) && d.allows(dpi_max) && !d.allows(dpi_max + step));
        assert_eq!(family::find(std::slice::from_ref(&info), 0x093A, pid).unwrap().family, Family::Ninjutso);
    }
    // not matched: the receivers the driver also knows, the unlisted legacy ids, another vendor, wrong vendor for an id, a collection
    // without a feature report long enough for report 6
    for (vid, pid) in [(0x093Au16, 0xEB01u16), (0x093A, 0xEB02), (0x1915, 0xAE1C), (0x1915, 0xAE8C), (0x1915, 0xAE8A), (0x1915, 0xAE17), (0x093A, 0xE011), (0x093A, 0xAE11), (0x1915, 0xE010), (0x258A, 0xE010)] {
        assert!(n::matches(&[&cfg(vid, pid, 704, None)], vid, pid).is_none(), "{vid:04x}:{pid:04x}");
    }
    assert!(n::matches(&[&cfg(0x093A, 0xE010, 0, None)], 0x093A, 0xE010).is_none());
    // the vendor collection is picked before a plain mouse collection that happens to carry feature reports
    let mouse = hid_info("mi_00", 0x093A, 0xE010, 0x01, 0x02, (8, 0, 20));
    let m = n::matches(&[&mouse, &cfg(0x093A, 0xE010, 16, None)], 0x093A, 0xE010).unwrap();
    assert_eq!(m.config.usage_page, 0xFF00);
}

#[test]
fn no_other_family_claims_a_ninjutso_id() {
    for p in n::PRODUCTS {
        let info = cfg(p.vid, p.pid, 704, None);
        let own = [&info];
        assert!(bu_mouse::pulsar::matches_cmouse(&own, p.vid, p.pid).is_none());
        assert!(bu_mouse::pulsar::matches_xs1(&own, p.vid, p.pid).is_none());
        assert!(bu_mouse::logitech::matches(&own, p.vid, p.pid).is_none());
        assert!(bu_mouse::razer::matches(&own, p.vid, p.pid).is_none());
        assert!(bu_mouse::steelseries::matches(&own, p.vid, p.pid).is_none());
    }
}

// ---------------------------------------------------------------- the driver: current protocol

/// A fake Sora V3 / TEN answering the report-6 protocol.
struct Cur {
    profile: u8,
    stage: u8,
    battery: u8,
    charging: u8,
    dpi: [u32; 4],
    polling_code: u8,
    direct: bool,
    /// answer every request with an all-zero report (a mouse that is not there)
    silent: bool,
    /// the next N answers echo another command
    stale: u32,
    /// writes are ignored
    deaf: bool,
    seen: Vec<Vec<u8>>,
    waits: Vec<u32>,
}

fn cur(direct: bool) -> Rc<RefCell<Cur>> {
    Rc::new(RefCell::new(Cur { profile: 1, stage: 0, battery: 87, charging: 0, dpi: [800, 1600, 3200, 6400], polling_code: 1, direct, silent: false, stale: 0, deaf: false, seen: Vec::new(), waits: Vec::new() }))
}

fn cur_port(info: &HidInfo, m: Rc<RefCell<Cur>>) -> ScriptPort {
    ScriptPort::new(info.clone(), move |out, how| {
        let mut m = m.borrow_mut();
        m.seen.push(out.to_vec());
        match how {
            HidTransfer::Feature { reply_id: 6, wait_ms, .. } => {
                m.waits.push(*wait_ms);
                if m.silent {
                    return Ok(vec![0u8; 16]);
                }
                let cmd = out[1];
                if m.stale > 0 {
                    m.stale -= 1;
                    return Ok(buf(64, &[(0, 6), (1, cmd.wrapping_add(1)), (8, 9)]));
                }
                let value: Vec<u8> = match cmd {
                    16 => vec![m.profile],
                    28 => vec![m.stage],
                    18 => vec![m.battery],
                    17 => vec![m.charging],
                    4 => {
                        let d = m.dpi[out[8] as usize];
                        let e = if m.direct { d } else { d / 50 - 1 };
                        vec![e as u8, (e >> 8) as u8, 0]
                    }
                    6 => vec![m.polling_code],
                    _ => panic!("unexpected read {cmd}"),
                };
                let mut v = buf(64, &[(0, 6), (1, cmd)]);
                v[8..8 + value.len()].copy_from_slice(&value);
                Ok(v)
            }
            HidTransfer::FeatureOut => {
                match (out[0], out[1]) {
                    (6, 3) if !m.deaf => {
                        let e = out[9] as u32 | ((out[10] as u32) << 8);
                        let d = if m.direct { e } else { (e + 1) * 50 };
                        m.dpi[out[8] as usize] = d;
                    }
                    (6, 5) if !m.deaf => m.polling_code = out[8],
                    (6, 3) | (6, 5) | (3, _) => {}
                    other => panic!("unexpected write {other:?}"),
                }
                Ok(Vec::new())
            }
            other => panic!("unexpected transfer {other:?}"),
        }
    })
}

fn current_mouse(pid: u16) -> (HidInfo, bu_mouse::family::Matched) {
    let info = cfg(0x093A, pid, 64, None);
    let m = n::matches(&[&info], 0x093A, pid).unwrap();
    (info, m)
}

#[test]
fn a_ten_reads_in_openmouses_order() {
    let (info, m) = current_mouse(0xE020);
    let mouse = cur(false);
    {
        let mut c = mouse.borrow_mut();
        c.charging = 1;
        c.stage = 1;
        c.polling_code = 3;
        c.profile = 2;
    }
    let mut port = cur_port(&info, mouse.clone());
    let s = n::Ninjutso.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((s.online, s.dpi, s.polling_hz, s.battery_percent, s.charging, s.lift_off, s.wireless), (true, Some(1600), Some(4000), Some(87), Some(true), None, Some(false)));
    // readCurrentStatus: profile, active stage, battery, charging, DPI of the active stage, polling
    assert_eq!(
        mouse.borrow().seen,
        vec![
            buf(64, &[(0, 6), (1, 16), (4, 1)]),
            buf(64, &[(0, 6), (1, 28), (4, 1), (7, 2)]),
            buf(64, &[(0, 6), (1, 18), (4, 1)]),
            buf(64, &[(0, 6), (1, 17), (4, 1)]),
            buf(64, &[(0, 6), (1, 4), (4, 1), (6, 1), (7, 2), (8, 1)]),
            buf(64, &[(0, 6), (1, 6), (4, 1), (7, 2)]),
        ]
    );
    assert!(port.sent.iter().all(|s| n::read_request_allowed(&s.0)));
    assert_eq!(port.sent[0].1, HidTransfer::Feature { reply_id: 6, reply_len: 64, wait_ms: 30 });
}

#[test]
fn a_sora_v3_dpi_is_read_directly() {
    let (info, m) = current_mouse(0xE010);
    let mouse = cur(true);
    mouse.borrow_mut().dpi[0] = 1234;
    let mut port = cur_port(&info, mouse);
    let s = n::Ninjutso.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!(s.dpi, Some(1234));
}

#[test]
fn a_wrong_echo_is_asked_again_up_to_four_times() {
    let (info, m) = current_mouse(0xE020);
    let mouse = cur(false);
    mouse.borrow_mut().stale = 2;
    let mut port = cur_port(&info, mouse.clone());
    let s = n::Ninjutso.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert!(s.online);
    // the first request (profile) was sent three times: waits 30, 60, 60
    assert_eq!(&mouse.borrow().waits[..4], &[30, 60, 60, 30]);
    assert_eq!(&mouse.borrow().seen[..3], &[buf(64, &[(0, 6), (1, 16), (4, 1)]), buf(64, &[(0, 6), (1, 16), (4, 1)]), buf(64, &[(0, 6), (1, 16), (4, 1)])]);
}

#[test]
fn a_mouse_that_does_not_answer_is_offline() {
    let (info, m) = current_mouse(0xE020);
    let mouse = cur(false);
    mouse.borrow_mut().silent = true;
    let mut port = cur_port(&info, mouse.clone());
    let s = n::Ninjutso.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((s.online, s.dpi, s.battery_percent), (false, None, None));
    assert_eq!(mouse.borrow().seen.len(), 4, "four tries of the first request, then it gives up");
    assert_eq!(mouse.borrow().waits, vec![30, 60, 60, 60]);
    // the read-only proof asks once
    mouse.borrow_mut().seen.clear();
    let s = n::Ninjutso.read(&m, &mut port, ReadOptions { online_tries: 1 }).unwrap();
    assert!(!s.online);
    assert_eq!(mouse.borrow().seen.len(), 1);
}

#[test]
fn setting_dpi_wraps_the_write_in_the_control_reports_and_reads_back() {
    let (info, m) = current_mouse(0xE020);
    let mouse = cur(false);
    let mut port = cur_port(&info, mouse.clone());
    let c = n::Ninjutso.write(&m, &mut port, Setting::Dpi(1600)).unwrap();
    assert_eq!((c.now, c.before), (1600, Some(800)));
    // profile, stage, DPI before (ours, for undo), control off, setDpi(stage 0, 0x1f, 0, 0), control on, DPI read-back
    assert_eq!(
        mouse.borrow().seen,
        vec![
            buf(64, &[(0, 6), (1, 16), (4, 1)]),
            buf(64, &[(0, 6), (1, 28), (4, 1), (7, 1)]),
            buf(64, &[(0, 6), (1, 4), (4, 1), (6, 1), (7, 1), (8, 0)]),
            buf(64, &[(0, 3), (1, 27), (2, 26), (5, 1)]),
            buf(64, &[(0, 6), (1, 3), (4, 1), (6, 4), (7, 1), (9, 0x1F)]),
            buf(64, &[(0, 3), (1, 28), (2, 27), (5, 1)]),
            buf(64, &[(0, 6), (1, 4), (4, 1), (6, 1), (7, 1), (8, 0)]),
        ]
    );
    // Sora V3 on stage 2, profile 3: 3200 = 0x0c80
    let (info, m) = current_mouse(0xE010);
    let mouse = cur(true);
    {
        let mut c = mouse.borrow_mut();
        c.stage = 2;
        c.profile = 3;
    }
    let mut port = cur_port(&info, mouse.clone());
    let c = n::Ninjutso.write(&m, &mut port, Setting::Dpi(3300)).unwrap();
    assert_eq!((c.now, c.before), (3300, Some(3200)));
    assert_eq!(mouse.borrow().seen[4], buf(64, &[(0, 6), (1, 3), (4, 1), (6, 4), (7, 3), (8, 2), (9, 0xE4), (10, 0x0C)]), "3300 = 0x0ce4");
}

#[test]
fn a_dpi_the_model_cannot_store_sends_nothing() {
    let (info, m) = current_mouse(0xE020);
    let mouse = cur(false);
    let mut port = cur_port(&info, mouse.clone());
    for d in [0, 825, 30_050] {
        assert!(matches!(n::Ninjutso.write(&m, &mut port, Setting::Dpi(d)), Err(Error::OutOfRange { .. })), "{d}");
    }
    assert!(mouse.borrow().seen.iter().all(|s| s[0] == 6 && n::read_request_allowed(s)), "reads only: no control report, no write");
}

#[test]
fn a_mouse_that_keeps_the_old_dpi_is_an_error_and_resumes() {
    let (info, m) = current_mouse(0xE020);
    let mouse = cur(false);
    mouse.borrow_mut().deaf = true;
    let mut port = cur_port(&info, mouse.clone());
    let e = n::Ninjutso.write(&m, &mut port, Setting::Dpi(1600)).unwrap_err();
    assert!(matches!(&e, Error::BadAnswer(t) if t.contains("kept 800 DPI instead of 1600")), "{e}");
    let seen = mouse.borrow().seen.clone();
    assert!(seen.contains(&n::control_request(true)), "the control resume is sent anyway");
}

#[test]
fn setting_polling_reads_back() {
    let (info, m) = current_mouse(0xE020);
    let mouse = cur(false);
    let mut port = cur_port(&info, mouse.clone());
    let c = n::Ninjutso.write(&m, &mut port, Setting::Polling(4000)).unwrap();
    assert_eq!((c.now, c.before), (4000, Some(2000 - 1000)));
    // profile, polling before, setPollingRate(3), polling read-back
    assert_eq!(
        mouse.borrow().seen,
        vec![
            buf(64, &[(0, 6), (1, 16), (4, 1)]),
            buf(64, &[(0, 6), (1, 6), (4, 1), (7, 1)]),
            buf(64, &[(0, 6), (1, 5), (4, 1), (6, 1), (7, 1), (8, 3)]),
            buf(64, &[(0, 6), (1, 6), (4, 1), (7, 1)]),
        ]
    );
    assert_eq!(port.sent[2].1, HidTransfer::FeatureOut);
    assert!(matches!(n::Ninjutso.write(&m, &mut port, Setting::Polling(500)), Err(Error::OutOfRange { .. })));
    // a deaf mouse
    let deaf = cur(false);
    deaf.borrow_mut().deaf = true;
    let mut port = cur_port(&info, deaf);
    let e = n::Ninjutso.write(&m, &mut port, Setting::Polling(8000)).unwrap_err();
    assert!(matches!(&e, Error::BadAnswer(t) if t.contains("kept 1000 Hz instead of 8000 Hz")), "{e}");
    // lift-off is not offered
    assert!(matches!(n::Ninjutso.write(&m, &mut port, Setting::LiftOff(10)), Err(Error::OutOfRange { .. })));
}

// ---------------------------------------------------------------- the driver: legacy protocol

/// A fake Sora V2: profile query on report 5, settings dump on report 4, the DPI write on report 5 command 5.
struct Leg {
    profile: u8,
    len: usize,
    battery: u8,
    count: u8,
    raw: [u8; 4],
    active: u8,
    silent: bool,
    deaf: bool,
    corrupt: bool,
    seen: Vec<Vec<u8>>,
}

fn leg(len: usize) -> Rc<RefCell<Leg>> {
    Rc::new(RefCell::new(Leg { profile: 2, len, battery: 80, count: 3, raw: [15, 31, 63, 127], active: 1, silent: false, deaf: false, corrupt: false, seen: Vec::new() }))
}

fn leg_port(info: &HidInfo, m: Rc<RefCell<Leg>>) -> ScriptPort {
    ScriptPort::new(info.clone(), move |out, how| {
        let mut m = m.borrow_mut();
        m.seen.push(out.to_vec());
        match (out[0], how) {
            (5, HidTransfer::FeatureOut) => {
                assert_eq!(out[1], 5, "only the DPI table is written");
                if !m.deaf {
                    m.count = out[9];
                    for i in 0..4 {
                        m.raw[i] = out[10 + i * 2];
                    }
                }
                Ok(Vec::new())
            }
            (5, HidTransfer::Feature { reply_id: 5, wait_ms: 15, .. }) => {
                if m.silent {
                    return Err(Error::BadAnswer("no answer".into()));
                }
                assert_eq!(out[1], 13);
                Ok(buf(32, &[(0, 5), (9, m.profile)]))
            }
            (4, HidTransfer::Feature { reply_id: 4, wait_ms: 90, .. }) => {
                assert_eq!((out.len(), out[1], out[6]), (m.len, 38, m.profile));
                let mut d = dump(m.len, m.battery, m.count, m.raw, m.active);
                if m.corrupt {
                    d[m.len - 2] ^= 1;
                }
                Ok(d)
            }
            other => panic!("unexpected {other:?}"),
        }
    })
}

fn legacy_mouse(len: u16, product: &str) -> (HidInfo, bu_mouse::family::Matched) {
    let info = cfg(0x1915, 0xAE12, len, Some(product));
    let m = n::matches(&[&info], 0x1915, 0xAE12).unwrap();
    (info, m)
}

#[test]
fn a_sora_v2_reads_the_settings_dump() {
    let (info, m) = legacy_mouse(704, "Ninjutso Sora V2");
    let mouse = leg(704);
    let mut port = leg_port(&info, mouse.clone());
    let s = n::Ninjutso.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((s.online, s.dpi, s.battery_percent, s.polling_hz, s.charging, s.lift_off), (true, Some(1600), Some(80), None, None, None));
    // the report-5 query is only 32 bytes: it goes out zero-padded to the collection's whole 704-byte feature report
    let mut query = n::legacy_request(13, 0, None);
    query.resize(704, 0);
    assert_eq!(mouse.borrow().seen, vec![query, n::legacy_data_request(2, 703)]);
    assert!(port.sent.iter().all(|s| n::read_request_allowed(&s.0)));
    assert_eq!(port.sent[1].1, HidTransfer::Feature { reply_id: 4, reply_len: 704, wait_ms: 90 });
}

#[test]
fn a_zero_profile_is_asked_once_more() {
    let (info, m) = legacy_mouse(704, "Ninjutso Sora V2");
    let mouse = leg(704);
    mouse.borrow_mut().profile = 0;
    let mut port = leg_port(&info, mouse.clone());
    let s = n::Ninjutso.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert!(s.online);
    assert_eq!(mouse.borrow().seen.len(), 3, "query, query again, dump");
    assert_eq!(mouse.borrow().seen[2][6], 0);
}

#[test]
fn the_8k_uses_the_short_dump() {
    let (info, m) = legacy_mouse(74, "Ninjutso Sora V2 8K");
    let mouse = leg(74);
    let mut port = leg_port(&info, mouse.clone());
    let s = n::Ninjutso.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!(s.dpi, Some(1600));
    assert_eq!(mouse.borrow().seen[1].len(), 74);
}

#[test]
fn a_silent_or_garbled_sora_v2() {
    let (info, m) = legacy_mouse(704, "Ninjutso Sora V2");
    let mouse = leg(704);
    mouse.borrow_mut().silent = true;
    let mut port = leg_port(&info, mouse);
    let s = n::Ninjutso.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((s.online, s.dpi), (false, None));
    let mouse = leg(704);
    mouse.borrow_mut().corrupt = true;
    let mut port = leg_port(&info, mouse);
    let e = n::Ninjutso.read(&m, &mut port, ReadOptions::default()).unwrap_err();
    assert!(matches!(&e, Error::BadAnswer(t) if t.contains("checksum")), "{e}");
}

#[test]
fn a_sora_v2_dpi_write_is_held_back_and_sends_nothing() {
    let (info, m) = legacy_mouse(704, "Ninjutso Sora V2");
    let mouse = leg(704);
    let mut port = leg_port(&info, mouse.clone());
    // OpenMouse's setLegacyDpi replaces the whole stage table (zeroing unused slots): this app refuses it before a byte leaves
    assert!(matches!(n::Ninjutso.write(&m, &mut port, Setting::Dpi(2400)), Err(Error::OutOfRange { .. })));
    assert!(mouse.borrow().seen.is_empty());
    // polling and lift-off are not offered on a Sora V2
    assert!(matches!(n::Ninjutso.write(&m, &mut port, Setting::Polling(1000)), Err(Error::OutOfRange { .. })));
    assert!(matches!(n::Ninjutso.write(&m, &mut port, Setting::LiftOff(10)), Err(Error::OutOfRange { .. })));
}

// ---------------------------------------------------------------- the allow-list

#[test]
fn only_the_reads_pass_the_allow_list() {
    for r in [
        n::read_request(16, 1, &[]),
        n::read_request(28, 2, &[]),
        n::read_request(18, 1, &[]),
        n::read_request(17, 1, &[]),
        n::read_request(4, 2, &[3]),
        n::read_request(6, 4, &[]),
        n::legacy_request(13, 0, None),
        n::legacy_data_request(2, 703),
        n::legacy_data_request(1, 73),
    ] {
        assert!(n::read_request_allowed(&r), "{r:02x?}");
        assert!(family::read_request_allowed(Family::Ninjutso, &r));
    }
    // the writes: setDpi (3), setPolling (5), the control reports, the legacy table write (5), other commands
    for w in [
        n::current_request(3, 1, &[0, 0x1F, 0, 0]),
        n::current_request(5, 1, &[3]),
        n::control_request(false),
        n::control_request(true),
        n::legacy_request(5, 2, None),
        n::legacy_request(11, 2, Some(1)),
        n::legacy_request(14, 1, Some(1)),
        n::current_request(13, 1, &[1]),
        n::current_request(27, 1, &[1]),
        n::current_request(167, 0, &[]),
        n::read_request(4, 1, &[4]),
        n::read_request(4, 1, &[]),
        n::read_request(21, 0, &[0]),
        vec![],
        vec![6],
    ] {
        assert!(!n::read_request_allowed(&w), "{w:02x?}");
    }
    // a read with a stray byte, and a dump request with a stray byte
    let mut odd = n::read_request(18, 0, &[]);
    odd[12] = 1;
    assert!(!n::read_request_allowed(&odd));
    let mut odd = n::legacy_data_request(1, 703);
    odd[100] = 1;
    assert!(!n::read_request_allowed(&odd));
    assert!(!n::read_request_allowed(&n::legacy_data_request(1, 100)));
}

// ---------------------------------------------------------------- review fixes: padding to the feature-report length, dead buffer

#[test]
fn every_set_feature_buffer_is_as_long_as_the_collections_feature_report() {
    // Windows' HidD_SetFeature rejects a buffer shorter than FeatureReportByteLength: the 16-byte report-6 requests, the report-5
    // query and the writes go out zero-padded to it, and the bytes before the padding are OpenMouse's
    let (info, m) = current_mouse(0xE020);
    let mouse = cur(false);
    let mut port = cur_port(&info, mouse.clone());
    n::Ninjutso.read(&m, &mut port, ReadOptions::default()).unwrap();
    n::Ninjutso.write(&m, &mut port, Setting::Dpi(1600)).unwrap();
    n::Ninjutso.write(&m, &mut port, Setting::Polling(4000)).unwrap();
    assert!(!port.sent.is_empty());
    for (out, _) in &port.sent {
        assert_eq!(out.len(), 64, "{out:02x?}");
    }
    let set_dpi = port.sent.iter().find(|(o, h)| *h == HidTransfer::FeatureOut && o[0] == 6 && o[1] == 3).unwrap();
    assert_eq!(&set_dpi.0[..16], n::current_request(3, 1, &[0, 0x1F, 0, 0]).as_slice());
    assert!(set_dpi.0[16..].iter().all(|b| *b == 0));
    let set_polling = port.sent.iter().find(|(o, h)| *h == HidTransfer::FeatureOut && o[0] == 6 && o[1] == 5).unwrap();
    assert_eq!(&set_polling.0[..16], n::current_request(5, 1, &[3]).as_slice());
    assert!(set_polling.0[16..].iter().all(|b| *b == 0));
    // a collection with a longer feature report: the same, longer
    let info = cfg(0x093A, 0xE020, 130, None);
    let m = n::matches(&[&info], 0x093A, 0xE020).unwrap();
    let mut port = cur_port(&info, cur(false));
    n::Ninjutso.write(&m, &mut port, Setting::Polling(2000)).unwrap();
    assert!(port.sent.iter().all(|(o, _)| o.len() == 130));
    // the Sora V2: the report-5 query (32 bytes by itself) and the dump request fill the 704 / 74 bytes
    for (len, product) in [(704u16, "Ninjutso Sora V2"), (74, "Ninjutso Sora V2 8K")] {
        let (info, m) = legacy_mouse(len, product);
        let mut port = leg_port(&info, leg(len as usize));
        n::Ninjutso.read(&m, &mut port, ReadOptions::default()).unwrap();
        assert!(port.sent.iter().all(|(o, _)| o.len() == len as usize), "{len}");
        assert_eq!(&port.sent[0].0[..32], n::legacy_request(13, 0, None).as_slice());
        assert!(port.sent[0].0[32..].iter().all(|b| *b == 0));
    }
}

#[test]
fn the_allow_list_takes_the_exact_request_and_zero_padding_only() {
    for r in [n::read_request(16, 1, &[]), n::read_request(28, 2, &[]), n::read_request(4, 2, &[3]), n::read_request(6, 4, &[]), n::legacy_request(13, 0, None)] {
        for len in [r.len(), 64, 704] {
            let mut padded = r.clone();
            padded.resize(len, 0);
            assert!(n::read_request_allowed(&padded), "{len}: {r:02x?}");
            // a stray byte in the padding, or anywhere, refuses it
            if len > r.len() {
                let mut odd = padded.clone();
                odd[len - 1] = 1;
                assert!(!n::read_request_allowed(&odd), "{len}");
            }
        }
    }
    // the writes stay refused when padded
    for w in [n::current_request(3, 1, &[0, 0x1F, 0, 0]), n::current_request(5, 1, &[3]), n::legacy_request(5, 2, None), n::legacy_request(11, 2, Some(1))] {
        let mut padded = w.clone();
        padded.resize(704, 0);
        assert!(!n::read_request_allowed(&padded), "{w:02x?}");
    }
    let mut short = n::read_request(16, 1, &[]);
    short.truncate(15);
    assert!(!n::read_request_allowed(&short));
    let mut short = n::legacy_request(13, 0, None);
    short.truncate(31);
    assert!(!n::read_request_allowed(&short));
}

#[test]
fn an_all_zero_settings_answer_is_not_a_sora_v2_online() {
    // an all-zero report sums to 0 and "matches" the 0 behind it; without the report id (byte 0 = 4) and a non-zero sum it is a
    // dead buffer, not a mouse at 50 DPI and 0 %
    assert!(matches!(n::legacy_settings(&[0u8; 704], 703), Err(Error::BadAnswer(_))));
    assert!(matches!(n::legacy_settings(&[0u8; 74], 73), Err(Error::BadAnswer(_))));
    // a report that is not number 4 but whose checksum is consistent
    let mut other = dump(704, 80, 3, [15, 31, 63, 127], 1);
    other[0] = 5;
    let sum: u32 = other[..701].iter().map(|b| *b as u32).sum();
    other[702] = sum as u8;
    other[703] = (sum >> 8) as u8;
    assert!(matches!(n::legacy_settings(&other, 703), Err(Error::BadAnswer(_))));
    // through the driver: the zero buffer is an error, nothing is shown online
    let (info, m) = legacy_mouse(704, "Ninjutso Sora V2");
    let mut port = ScriptPort::new(info, |out, _| match out[0] {
        5 => Ok(buf(704, &[(0, 5), (9, 2)])),
        4 => Ok(vec![0u8; 704]),
        other => panic!("{other}"),
    });
    assert!(n::Ninjutso.read(&m, &mut port, ReadOptions::default()).is_err());
    // the real dump still passes
    assert!(n::legacy_settings(&dump(704, 80, 3, [15, 31, 63, 127], 1), 703).is_ok());
}

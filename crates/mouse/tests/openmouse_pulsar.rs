//! Order 104 check, Pulsar: our requests = the bytes OpenMouse sends for the same setting (encode), and answers shaped the
//! way OpenMouse reads them decode to the same values (decode). The expected bytes are worked out by hand from OpenMouse
//! (@openmouse/protocol 0.40.1: `dist/pulsar/index.js`, `dist/drivers/pulsar/pulsar-hid.js`, `pulsar-xs1-hid.js`), never
//! from our own encoder. OpenMouse's report-8 packet has no report id (its byte i = our byte i + 1); its checksum counts
//! the id 0x08.

use bu_mouse::family::{Driver, Family, Matched, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::os::HidTransfer;
use bu_mouse::pulsar::{self as p, Link};

/// A 17-byte report-8 frame from its first bytes (rest zero) and its checksum.
fn r8(head: &[u8], cks: u8) -> [u8; 17] {
    let mut f = [0u8; 17];
    f[..head.len()].copy_from_slice(head);
    f[16] = cks;
    f
}

// ---------------------------------------------------------------- cMouse (X2 CrazyLight, Tenz Signature, X2h, X2 V3 ES Mini, X2F)

#[test]
fn cmouse_reads_are_openmouses_readflash_bytes() {
    // readFlash(FLASH.reportRate = 0, 2): [08 | 08 00 00 00 02 … cks], cks = 0x55 − (8 + 8 + 2)
    assert_eq!(p::read_frame(p::ADDR_POLLING, 2), r8(&[0x08, 0x08, 0x00, 0x00, 0x00, 0x02], 0x43));
    // readFlash(FLASH.currentDpi = 4, 2)
    assert_eq!(p::read_frame(p::ADDR_ACTIVE_STAGE, 2), r8(&[0x08, 0x08, 0x00, 0x00, 0x04, 0x02], 0x3F));
    // readFlash(FLASH.dpiValues + 1 * 4 = 16, 4) - stage 1
    assert_eq!(p::read_frame(p::ADDR_DPI_STAGE0 + 4, 4), r8(&[0x08, 0x08, 0x00, 0x00, 0x10, 0x04], 0x31));
    // readFlash(FLASH.liftOffDistance = 10, 2)
    assert_eq!(p::read_frame(p::ADDR_LIFT_OFF, 2), r8(&[0x08, 0x08, 0x00, 0x00, 0x0A, 0x02], 0x39));
    // query(batteryLevel): no parameters
    assert_eq!(p::frame(p::CMD_BATTERY, 0, &[]), r8(&[0x08, 0x04], 0x49));
    // query(encryptionData, [c0..c3, 0, 0, 0, 0]): length 8
    assert_eq!(p::identify_frame([1, 2, 3, 4]), r8(&[0x08, 0x01, 0x00, 0x00, 0x00, 0x08, 1, 2, 3, 4], 0x3A));
}

#[test]
fn cmouse_host_control_is_openmouses_setdeviceonline() {
    // packet[4] (length) stays 0, packet[5] = enabled
    assert_eq!(p::online_frame(true), r8(&[0x08, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01], 0x49));
    assert_eq!(p::online_frame(false), r8(&[0x08, 0x03], 0x4A));
    assert_eq!(p::online_query_frame(), p::online_frame(false));
}

#[test]
fn cmouse_writes_are_openmouses_writeflash_bytes() {
    // setPollingRate(8000): writeCheckedByte(0, 0x40) = [0x40, 0x55 − 0x40]
    assert_eq!(p::frame(p::CMD_WRITE, p::ADDR_POLLING, &p::scalar_pair(p::polling_code(8000).unwrap())), r8(&[0x08, 0x07, 0x00, 0x00, 0x00, 0x02, 0x40, 0x15], 0xEF));
    assert_eq!(p::write_byte_frame(p::ADDR_POLLING, 0x40), r8(&[0x08, 0x07, 0x00, 0x00, 0x00, 0x02, 0x40, 0x15], 0xEF));
    // setLiftOffDistance("Low") = 3 at 10
    assert_eq!(p::write_byte_frame(p::ADDR_LIFT_OFF, p::lift_off_code(7).unwrap()), r8(&[0x08, 0x07, 0x00, 0x00, 0x0A, 0x02, 0x03, 0x52], 0xE5));
    // setDpi(800) on stage 1: writeFlash(16, pulsarEncodeDpi(800) = [79, 79, 0, 0x55 − 158])
    assert_eq!(p::frame(p::CMD_WRITE, p::ADDR_DPI_STAGE0 + 4, &p::encode_dpi_stage(800).unwrap()), r8(&[0x08, 0x07, 0x00, 0x00, 0x10, 0x04, 0x4F, 0x4F, 0x00, 0xB7], 0xDD));
}

#[test]
fn cmouse_polling_and_lift_off_codes_are_openmouses() {
    // pulsarEncodePollingRate: ≤ 1000 → 1000 / rate, else rate / 2000 × 16
    for (hz, code) in [(125, 8), (250, 4), (500, 2), (1000, 1), (2000, 16), (4000, 32), (8000, 64)] {
        assert_eq!(p::polling_code(hz), Some(code), "{hz}");
        assert_eq!(p::polling_hz(code), Some(hz), "{code}");
    }
    assert_eq!(p::polling_code(3000), None, "not one of OpenMouse's rates");
    // { Low: 3, Medium: 1, High: 2 }
    assert_eq!((p::lift_off_code(7), p::lift_off_code(10), p::lift_off_code(20)), (Some(3), Some(1), Some(2)));
}

#[test]
fn cmouse_dpi_encoding_is_openmouses_pulsarencodedpi() {
    // raw = dpi/10 − 1 below 10050; (dpi − 10050)/50 with dpiEx 0x22 from 10050; (dpi/2 − 10050)/50 with 0x33 from 30100;
    // flags = (high << 2) | (high << 6) | dpiEx | (dpiEx << 4) cut to a byte; last byte = 0x55 − sum of the three
    for (dpi, rec) in [
        (800, [0x4F, 0x4F, 0x00, 0xB7]),
        (2400, [0xEF, 0xEF, 0x00, 0x77]),
        (3200, [0x3F, 0x3F, 0x44, 0x55u8.wrapping_sub(0x3F + 0x3F + 0x44)]),
        (10000, [0xE7, 0xE7, 0xCC, 0x55u8.wrapping_sub(0xE7u8.wrapping_add(0xE7).wrapping_add(0xCC))]),
        (10050, [0x00, 0x00, 0x22, 0x33]),
        (12000, [0x27, 0x27, 0x22, 0xE5]),
        (31000, [0x6D, 0x6D, 0x33, 0x48]),
    ] {
        assert_eq!(p::encode_dpi_stage(dpi), Some(rec), "{dpi}");
        // pulsarDecodeDpi of the same record
        assert_eq!(p::decode_dpi_stage(&rec), Some(dpi), "{dpi}");
    }
    // pulsarDpiOptions has no 10010-10040 and no odd 50s above
    for dpi in [10010, 10040, 10075, 30050, 32100] {
        assert_eq!(p::encode_dpi_stage(dpi), None, "{dpi}");
    }
    // decode refuses what OpenMouse refuses: low != duplicate, a bad sum
    assert_eq!(p::decode_dpi_stage(&[0x4F, 0x4E, 0x00, 0xB8]), None);
    assert_eq!(p::decode_dpi_stage(&[0x4F, 0x4F, 0x00, 0xB8]), None);
    // flag 0x01 alone doubles (pulsarDecodeDpi): raw 79 → 800 × 2
    assert_eq!(p::decode_dpi_stage(&[0x4F, 0x4F, 0x11, 0x55u8.wrapping_sub(0x4F + 0x4F + 0x11)]), Some(1600));
}

#[test]
fn cmouse_answers_decode_like_openmouse() {
    // readDeviceInfo: cid = response[9], mid = [10], type = [11] (our 10, 11, 12); maximumPollingRateHz by type
    for (t, max, wireless) in [(0, 1000, Some(true)), (1, 4000, Some(true)), (2, 1000, Some(false)), (3, 8000, Some(false)), (4, 2000, Some(true)), (5, 8000, Some(true)), (9, 1000, None)] {
        assert_eq!(Link::from_code(t).max_polling_hz(), max, "type {t}");
        assert_eq!(Link::from_code(t).wireless(), wireless, "type {t}");
    }
    // battery: percent = battery[5] (≤ 100, else none), charging = battery[6] === 1 (our 6, 7)
    let b = r8(&[0x08, 0x04, 0x00, 0x00, 0x00, 0x00, 78, 1, 0x0F, 0x6E], 0);
    assert_eq!(p::decode_battery(&b), Some((Some(78), true, 0x0F6E)));
    let b = r8(&[0x08, 0x04, 0x00, 0x00, 0x00, 0x00, 0xFF, 0], 0);
    assert_eq!(p::decode_battery(&b).map(|x| x.0), Some(None), "over 100 = no reading");
}

/// The whole cMouse driver against a scripted dongle: a DPI change is exactly OpenMouse's sequence.
#[test]
fn cmouse_driver_sends_openmouses_sequence_for_a_dpi_change() {
    use std::cell::RefCell;
    use std::rc::Rc;
    let model = Rc::new(RefCell::new(bu_mouse::fake::CmouseModel::new()));
    let m2 = model.clone();
    let info = hid_info("cfg", 0x3710, 0x5406, 0xFF02, 0x02, (17, 17, 0));
    let mut port = ScriptPort::new(info.clone(), move |out, _| Ok(m2.borrow_mut().answer(out)));
    let mt = Matched { family: Family::PulsarCmouse, model: None, wireless: Some(true), config: info, variant: 0 };
    let c = p::Cmouse.write(&mt, &mut port, Setting::Dpi(1600)).unwrap();
    assert_eq!((c.now, c.before), (1600, Some(800)));
    let sent: Vec<Vec<u8>> = port.sent.iter().map(|s| s.0.clone()).collect();
    assert_eq!(
        sent,
        vec![
            p::online_frame(true).to_vec(),
            p::read_frame(p::ADDR_ACTIVE_STAGE, 2).to_vec(),
            p::read_frame(p::ADDR_DPI_STAGE0 + 4, 4).to_vec(),
            r8(&[0x08, 0x07, 0x00, 0x00, 0x10, 0x04, 0x9F, 0x9F, 0x00, 0x17], 0xDD).to_vec(),
            p::read_frame(p::ADDR_DPI_STAGE0 + 4, 4).to_vec(),
            p::online_frame(false).to_vec(),
        ]
    );
    assert!(port.sent.iter().all(|s| matches!(s.1, HidTransfer::OutputThenInput { .. })));
    // reading sends read requests only
    port.sent.clear();
    let r = p::Cmouse.read(&mt, &mut port, ReadOptions::default()).unwrap();
    assert_eq!(r.dpi, Some(1600));
    assert!(port.sent.iter().all(|s| p::read_request_allowed(&s.0)));
}

// ---------------------------------------------------------------- cMouse: Order 110, the five settings under Lift-off

use bu_mouse::family::{Change, Extra, Kind};

/// A scripted cMouse whose settings memory holds the values OpenMouse reads (PULSAR_FLASH), stored as the firmware stores
/// them: `[v, 0x55 − v]` pairs.
fn x2_with_extras() -> bu_mouse::fake::CmouseModel {
    let mut m = bu_mouse::fake::CmouseModel::new();
    for (a, v) in [(169usize, 4u8), (171, 1), (175, 0), (177, 1), (181, 1)] {
        m.mem[a] = v;
        m.mem[a + 1] = 0x55u8.wrapping_sub(v);
    }
    m
}

fn cmouse_port(model: bu_mouse::fake::CmouseModel) -> (std::rc::Rc<std::cell::RefCell<bu_mouse::fake::CmouseModel>>, ScriptPort, Matched) {
    let model = std::rc::Rc::new(std::cell::RefCell::new(model));
    let m2 = model.clone();
    let info = hid_info("cfg", 0x3710, 0x5406, 0xFF02, 0x02, (17, 17, 0));
    let port = ScriptPort::new(info.clone(), move |out, _| Ok(m2.borrow_mut().answer(out)));
    let mt = Matched { family: Family::PulsarCmouse, model: None, wireless: Some(true), config: info, variant: 0 };
    (model, port, mt)
}

#[test]
fn cmouse_read_shows_the_five_settings_as_openmouse_reads_them() {
    let (_, mut port, mt) = cmouse_port(x2_with_extras());
    let r = p::Cmouse.read(&mt, &mut port, ReadOptions::default()).unwrap();
    let got: Vec<(Extra, Option<i32>)> = r.extras.iter().map(|e| (e.extra, e.value)).collect();
    // debounceMs = flash[169]; motionSync / angleSnapping / rippleControl / performanceMode = flash[171 / 175 / 177 / 181] === 1
    assert_eq!(got, vec![(Extra::Debounce, Some(4)), (Extra::MotionSync, Some(1)), (Extra::AngleSnap, Some(0)), (Extra::RippleControl, Some(1)), (Extra::Turbo, Some(1))]);
    // getDebounceOptions: 0..15 ms
    assert_eq!(r.extras[0].kind, Kind::Range { min: 0, max: 15, step: 1, unit: "ms" });
    assert!(r.extras[1..].iter().all(|e| e.kind == Kind::Switch));
    // the row 169..183 that readStatus takes these from, in two reads (readFlash's 10-byte limit): 10 at 169 (0xA9), 4 at 179 (0xB3)
    assert!(port.sent.iter().any(|s| s.0 == r8(&[0x08, 0x08, 0x00, 0x00, 0xA9, 0x0A], 0x55u8.wrapping_sub(0x08 + 0x08 + 0xA9 + 0x0A)).to_vec()));
    assert!(port.sent.iter().any(|s| s.0 == r8(&[0x08, 0x08, 0x00, 0x00, 0xB3, 0x04], 0x55u8.wrapping_sub(0x08 + 0x08 + 0xB3 + 0x04)).to_vec()));
    // + the 0.7 mm lift-off (OpenMouse's "Low" = 3); reading = read requests only, the DPI chips' range as before
    assert_eq!(r.caps.lift_off, vec![7, 10, 20]);
    assert!(port.sent.iter().all(|s| p::read_request_allowed(&s.0)));
    assert_eq!((r.caps.dpi.as_ref().unwrap().min(), r.caps.dpi.as_ref().unwrap().max()), (50, 26000));
}

/// The write frames (0x07 writeFlash) a change sent.
fn writes(port: &ScriptPort) -> Vec<Vec<u8>> {
    port.sent.iter().map(|s| s.0.clone()).filter(|f| f[1] == 0x07).collect()
}

#[test]
fn cmouse_changes_are_openmouses_writecheckedbyte_frames() {
    let (model, mut port, mt) = cmouse_port(x2_with_extras());
    let mut go = |c: Change| -> Vec<Vec<u8>> {
        port.sent.clear();
        p::Cmouse.change(&mt, &mut port, c).unwrap();
        // inside host control (setVerifiedByte → withDeviceControl): enter first, leave last
        assert_eq!(port.sent.iter().find(|s| s.0[1] == 0x03).map(|s| s.0[6]), Some(1), "{c:?}");
        assert_eq!(port.sent.last().map(|s| s.0.clone()), Some(p::online_frame(false).to_vec()), "{c:?}");
        writes(&port)
    };
    // frame [08 07 00 | addr hi lo | 02 | v, 0x55 − v | … | 0x55 − sum], sums worked out by hand:
    // setDebounceTime(8) at 169 = 0xA9: 8+7+0xA9+2+8+0x4D = 271 → 15 → 0x46
    assert_eq!(go(Change::Extra(Extra::Debounce, 8)), vec![r8(&[0x08, 0x07, 0x00, 0x00, 0xA9, 0x02, 0x08, 0x4D], 0x46).to_vec()]);
    // setMotionSync(false) at 171 = 0xAB: 8+7+0xAB+2+0+0x55 = 273 → 17 → 0x44
    assert_eq!(go(Change::Extra(Extra::MotionSync, 0)), vec![r8(&[0x08, 0x07, 0x00, 0x00, 0xAB, 0x02, 0x00, 0x55], 0x44).to_vec()]);
    // setAngleSnapping(true) at 175 = 0xAF: 8+7+0xAF+2+1+0x54 = 277 → 21 → 0x40
    assert_eq!(go(Change::Extra(Extra::AngleSnap, 1)), vec![r8(&[0x08, 0x07, 0x00, 0x00, 0xAF, 0x02, 0x01, 0x54], 0x40).to_vec()]);
    // setRippleControl(false) at 177 = 0xB1: 8+7+0xB1+2+0+0x55 = 279 → 23 → 0x3E
    assert_eq!(go(Change::Extra(Extra::RippleControl, 0)), vec![r8(&[0x08, 0x07, 0x00, 0x00, 0xB1, 0x02, 0x00, 0x55], 0x3E).to_vec()]);
    // setPerformanceMode(false) at 181 = 0xB5: 8+7+0xB5+2+0+0x55 = 283 → 27 → 0x3A
    assert_eq!(go(Change::Extra(Extra::Turbo, 0)), vec![r8(&[0x08, 0x07, 0x00, 0x00, 0xB5, 0x02, 0x00, 0x55], 0x3A).to_vec()]);
    // the fake mouse holds what was written; the next read shows it
    let (_, mut port2, mt2) = cmouse_port(model.borrow().clone());
    let r = p::Cmouse.read(&mt2, &mut port2, ReadOptions::default()).unwrap();
    let got: Vec<Option<i32>> = r.extras.iter().map(|e| e.value).collect();
    assert_eq!(got, vec![Some(8), Some(0), Some(1), Some(0), Some(0)]);
}

#[test]
fn cmouse_refuses_what_openmouse_does_not_take_before_a_byte_leaves() {
    let (_, mut port, mt) = cmouse_port(x2_with_extras());
    for c in [Change::Extra(Extra::Debounce, 16), Change::Extra(Extra::Debounce, -1), Change::Extra(Extra::MotionSync, 2), Change::Extra(Extra::Turbo, -1)] {
        port.sent.clear();
        assert!(p::Cmouse.change(&mt, &mut port, c).is_err(), "{c:?}");
        assert!(port.sent.is_empty(), "{c:?}: nothing sent");
    }
}

// ---------------------------------------------------------------- XS-1 (X3 Medium)

/// An XS-1 report as Windows sends it: report id 0, then OpenMouse's 64-byte packet `[0x00, command…, cks_lo, cks_hi]`.
fn xs1(cmd: &[u8], cks: u16) -> Vec<u8> {
    let mut v = vec![0u8; 65];
    v[2..2 + cmd.len()].copy_from_slice(cmd);
    v[63] = cks as u8;
    v[64] = (cks >> 8) as u8;
    v
}

#[test]
fn xs1_packets_are_openmouses_encoderequest_bytes() {
    // QUERY.battery [08 81 01]: checksum = 0x08 + 0x81 + 0x01
    let mut q = vec![0u8];
    q.extend_from_slice(&p::xs1_packet(&p::XS1_Q_BATTERY));
    assert_eq!(q, xs1(&[0x08, 0x81, 0x01], 0x008A));
    let mut q = vec![0u8];
    q.extend_from_slice(&p::xs1_packet(&p::XS1_Q_DPI));
    assert_eq!(q, xs1(&[0x05, 0x82, 0x05], 0x008C));
    // WRITE.dpi(1600 = 0x0640): [05 02 05 00 00 01 40 06 40 06]
    assert_eq!(p::xs1_dpi_write(1600), [0x05, 0x02, 0x05, 0x00, 0x00, 0x01, 0x40, 0x06, 0x40, 0x06]);
    let mut w = vec![0u8];
    w.extend_from_slice(&p::xs1_packet(&p::xs1_dpi_write(1600)));
    assert_eq!(w, xs1(&[0x05, 0x02, 0x05, 0x00, 0x00, 0x01, 0x40, 0x06, 0x40, 0x06], 0x0099));
    // WRITE.liftOffDistance(High = 0x14): [07 02 03 00 00 01 02 14]
    assert_eq!(p::xs1_lift_off_write(0x14), [0x07, 0x02, 0x03, 0x00, 0x00, 0x01, 0x02, 0x14]);
    // a checksum over 255 keeps its high byte (16-bit little-endian)
    let big = p::xs1_packet(&[0xFF, 0xFF]);
    assert_eq!((big[62], big[63]), (0xFE, 0x01));
}

#[test]
fn xs1_answers_decode_like_openmouse() {
    // polling query answer byte 7: {240: 125, 120: 250, 60: 500, 30: 1000, 15: 2000, 8: 4000, 4: 8000}
    for (v, hz) in [(240, 125), (120, 250), (60, 500), (30, 1000), (15, 2000), (8, 4000), (4, 8000)] {
        assert_eq!(p::xs1_polling_hz(v), Some(hz));
    }
    assert_eq!(p::xs1_polling_hz(31), None);
    // lift-off answer byte 8: 0x07 Low, 0x0A Medium, 0x14 High
    assert_eq!((p::xs1_lift_off_tenths(0x07), p::xs1_lift_off_tenths(0x0A), p::xs1_lift_off_tenths(0x14), p::xs1_lift_off_tenths(9)), (Some(7), Some(10), Some(20), None));
}

#[test]
fn xs1_driver_reads_and_writes_the_openmouse_way() {
    use std::cell::RefCell;
    use std::rc::Rc;
    // a scripted X3 Medium: DPI 800, lift-off Medium, polling 1000, battery 64 %
    let dpi = Rc::new(RefCell::new(800u16));
    let d2 = dpi.clone();
    let info = hid_info("xs1", 0x3710, 0x5402, 0xFFFF, 0x01, (0, 0, 65));
    let mut port = ScriptPort::new(info.clone(), move |out, how| {
        let cmd = &out[2..5];
        if matches!(how, HidTransfer::FeatureOut) {
            if cmd == [0x05, 0x02, 0x05] {
                *d2.borrow_mut() = u16::from_le_bytes([out[8], out[9]]);
            }
            return Ok(Vec::new());
        }
        let mut a = vec![0u8; 65];
        a[1..4].copy_from_slice(cmd);
        match cmd {
            [0x08, 0x81, 0x01] => a[7] = 64,
            [0x05, 0x82, 0x05] => {
                let b = d2.borrow().to_le_bytes();
                a[8..12].copy_from_slice(&[b[0], b[1], b[0], b[1]]);
            }
            [0x07, 0x82, 0x03] => a[9] = 0x0A,
            [0x08, 0x85, 0x03] => a[8] = 30,
            _ => {}
        }
        Ok(a)
    });
    let mt = bu_mouse::pulsar::matches_xs1(&[&info], 0x3710, 0x5402).unwrap();
    assert_eq!((mt.family, mt.model), (Family::PulsarXs1, Some("Pulsar X3 Medium")));
    let r = p::Xs1.read(&mt, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((r.battery_percent, r.dpi, r.lift_off, r.polling_hz), (Some(64), Some(800), Some(10), Some(1000)));
    assert!(r.caps.polling.is_empty(), "OpenMouse never sets an XS-1 polling rate");
    assert!(port.sent.iter().all(|s| p::xs1_read_request_allowed(&s.0)), "reading = queries only");
    assert!(port.sent.iter().all(|s| s.1 == HidTransfer::Feature { reply_id: 0, reply_len: 65, wait_ms: 25 }));
    port.sent.clear();
    let c = p::Xs1.write(&mt, &mut port, Setting::Dpi(1600)).unwrap();
    assert_eq!((c.now, c.before), (1600, Some(800)));
    assert_eq!(port.sent[1].0, xs1(&[0x05, 0x02, 0x05, 0x00, 0x00, 0x01, 0x40, 0x06, 0x40, 0x06], 0x0099));
    assert_eq!(port.sent[1].1, HidTransfer::FeatureOut);
    assert!(!p::xs1_read_request_allowed(&port.sent[1].0), "a write is not a read request");
    assert!(p::Xs1.write(&mt, &mut port, Setting::Polling(1000)).is_err());
    assert!(p::Xs1.write(&mt, &mut port, Setting::Dpi(1610)).is_err(), "50-DPI steps");
}

#[test]
fn xs1_extras_are_openmouses_queries_and_writes() {
    use std::cell::RefCell;
    use std::rc::Rc;
    // a scripted X3: a store of the four flags, answered at packet byte 7 (our 8), set by the writes' last byte
    let st: Rc<RefCell<std::collections::HashMap<[u8; 2], u8>>> = Rc::new(RefCell::new([([0x07, 0x05], 1), ([0x07, 0x03], 0), ([0x07, 0x04], 0), ([0x04, 0x03], 4)].into_iter().collect()));
    let s2 = st.clone();
    let info = hid_info("xs1", 0x3710, 0x5402, 0xFFFF, 0x01, (0, 0, 65));
    let mut port = ScriptPort::new(info.clone(), move |out, how| {
        let cmd = &out[2..5];
        if matches!(how, HidTransfer::FeatureOut) {
            s2.borrow_mut().insert([cmd[0], cmd[1]], out[8]);
            return Ok(Vec::new());
        }
        let mut a = vec![0u8; 65];
        a[1..4].copy_from_slice(cmd);
        if let Some(v) = s2.borrow().get(&[cmd[0], cmd[1] & 0x7F]) {
            a[8] = *v;
        }
        if cmd == [0x07, 0x82, 0x03] {
            a[9] = 0x07;
        }
        Ok(a)
    });
    let mt = bu_mouse::pulsar::matches_xs1(&[&info], 0x3710, 0x5402).unwrap();
    let r = p::Xs1.read(&mt, &mut port, ReadOptions::default()).unwrap();
    // QUERY.motionSync [07 85 02], rippleControl [07 83 02], angleSnapping [07 84 02], debounce [04 83 03]
    for (q, cks) in [([0x07, 0x85, 0x02], 0x8E), ([0x07, 0x83, 0x02], 0x8C), ([0x07, 0x84, 0x02], 0x8D), ([0x04, 0x83, 0x03], 0x8A)] {
        assert!(port.sent.iter().any(|s| s.0 == xs1(&q, cks)), "{q:02x?}");
    }
    assert!(port.sent.iter().all(|s| p::xs1_read_request_allowed(&s.0)));
    let got: Vec<(Extra, Option<i32>)> = r.extras.iter().map(|e| (e.extra, e.value)).collect();
    assert_eq!(got, vec![(Extra::Debounce, Some(4)), (Extra::MotionSync, Some(1)), (Extra::AngleSnap, Some(0)), (Extra::RippleControl, Some(0))], "the card's order; no turbo on XS-1");
    assert_eq!((r.lift_off, r.caps.lift_off.clone()), (Some(7), vec![7, 10, 20]), "Low = 0x07 = 0.7 mm");
    // WRITE.motionSync(false) [07 05 02 00 00 01 00], debounce(9) [04 03 03 00 00 01 09], rippleControl(true), angleSnapping(true)
    for (c, w, cks) in [
        (Change::Extra(Extra::MotionSync, 0), vec![0x07, 0x05, 0x02, 0x00, 0x00, 0x01, 0x00], 0x0F),
        (Change::Extra(Extra::Debounce, 9), vec![0x04, 0x03, 0x03, 0x00, 0x00, 0x01, 0x09], 0x14),
        (Change::Extra(Extra::RippleControl, 1), vec![0x07, 0x03, 0x02, 0x00, 0x00, 0x01, 0x01], 0x0E),
        (Change::Extra(Extra::AngleSnap, 1), vec![0x07, 0x04, 0x02, 0x00, 0x00, 0x01, 0x01], 0x0F),
    ] {
        port.sent.clear();
        p::Xs1.change(&mt, &mut port, c).unwrap();
        let out: Vec<&Vec<u8>> = port.sent.iter().filter(|s| s.1 == HidTransfer::FeatureOut).map(|s| &s.0).collect();
        assert_eq!(out, vec![&xs1(&w, cks)], "{c:?}");
    }
    assert_eq!(*st.borrow().get(&[0x04, 0x03]).unwrap(), 9);
    port.sent.clear();
    assert!(p::Xs1.change(&mt, &mut port, Change::Extra(Extra::Debounce, 16)).is_err() && port.sent.is_empty(), "0-15 ms");
    assert!(p::Xs1.change(&mt, &mut port, Change::Extra(Extra::Turbo, 1)).is_err() && port.sent.is_empty(), "no turbo on XS-1");
}

#[test]
fn xs1_is_only_for_its_ids_and_never_shadows_a_cmouse() {
    let xs = hid_info("xs1", 0x3710, 0x5402, 0xFFFF, 0x01, (0, 0, 65));
    let r8c = hid_info("r8", 0x3710, 0x5402, 0xFF02, 0x02, (17, 17, 0));
    assert!(p::matches_xs1(&[&xs, &r8c], 0x3710, 0x5402).is_none(), "a report-8 collection makes it a cMouse");
    assert!(p::matches_xs1(&[&xs], 0x3710, 0x3404).is_none(), "X2A v3 is not on the SUPPORTED list");
    assert_eq!(p::matches_xs1(&[&xs], 0x3710, 0x5403).map(|m| m.model), Some(None), "the shared 8K receiver names no model");
    let all = [xs.clone(), r8c.clone()];
    assert_eq!(bu_mouse::family::find(&all, 0x3710, 0x5402).map(|m| m.family), Some(Family::PulsarCmouse));
}

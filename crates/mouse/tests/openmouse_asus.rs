//! Order 105 check, ASUS ROG Gladius II P502: our packets = the bytes OpenMouse sends for the same setting (encode), and
//! answers shaped the way OpenMouse reads them decode to the same values (decode). Expected bytes are worked out by hand from
//! OpenMouse (@openmouse/protocol 0.40.1: `dist/asus/index.js`, `dist/asus/devices.js`, `dist/drivers/asus/hid.js`).
//!
//! OpenMouse sends `sendReport(0, 64 bytes)`; Windows puts report id 0 in front: OpenMouse's byte i is our byte i + 1.

use bu_mouse::asus as a;
use bu_mouse::family::{self, Driver, Family, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::os::{HidInfo, HidTransfer};
use bu_mouse::Error;
use std::cell::RefCell;
use std::rc::Rc;

/// A 65-byte report (id 0 first) from OpenMouse's leading bytes.
fn rep(bytes: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 65];
    v[1..1 + bytes.len()].copy_from_slice(bytes);
    v
}

fn cfg() -> HidInfo {
    hid_info("asus-cfg", 0x0B05, 0x1845, 0xFF01, 0x0001, (65, 65, 0))
}

// ---------------------------------------------------------------- encode

#[test]
fn reads_are_openmouses_bytes() {
    // asusReadSettingsRequest = request(0x12, 0x04, 0x00); asusReadProfileRequest = request(0x12, 0x00)
    assert_eq!(a::wire(&a::read_settings_request(), 65), rep(&[0x12, 0x04, 0x00]));
    assert_eq!(a::wire(&a::read_profile_request(), 65), rep(&[0x12, 0x00]));
    assert_eq!(a::wire(&a::save_request(), 65), rep(&[0x50, 0x03]));
}

#[test]
fn the_dpi_write_is_openmouses_request() {
    // asusSetDpiRequest(model, stage, dpi, undefined): request(0x51, 0x31, stage, 0x00, enc & 0xff, enc >> 8),
    // enc = dpi / 100 − 1 (the Gladius II has no DPI colours, so none are appended)
    // stage 0, 800: enc 7 · stage 1, 1600: enc 15 = 0x0F · 12000: enc 119 = 0x77 · 100: enc 0
    assert_eq!(a::wire(&a::set_dpi_request(0, 800).unwrap(), 65), rep(&[0x51, 0x31, 0x00, 0x00, 0x07, 0x00]));
    assert_eq!(a::wire(&a::set_dpi_request(1, 1600).unwrap(), 65), rep(&[0x51, 0x31, 0x01, 0x00, 0x0F, 0x00]));
    assert_eq!(a::wire(&a::set_dpi_request(1, 12000).unwrap(), 65), rep(&[0x51, 0x31, 0x01, 0x00, 0x77, 0x00]));
    assert_eq!(a::wire(&a::set_dpi_request(0, 100).unwrap(), 65), rep(&[0x51, 0x31, 0x00, 0x00, 0x00, 0x00]));
    // out of the range / the step / the stage count
    for (stage, dpi) in [(0u8, 50u32), (0, 150), (0, 12100), (2, 800), (0, 0)] {
        assert!(a::set_dpi_request(stage, dpi).is_none(), "{stage} {dpi}");
    }
}

#[test]
fn the_polling_write_is_openmouses_request() {
    // asusSetPollingRateRequest: request(0x51, 0x31, settingField(model, 0) = dpiStages + 0 = 2, 0x00, index in
    // [125, 250, 500, 1000, 2000, 4000, 8000])
    for (hz, idx) in [(125, 0u8), (250, 1), (500, 2), (1000, 3)] {
        assert_eq!(a::wire(&a::set_polling_request(hz).unwrap(), 65), rep(&[0x51, 0x31, 0x02, 0x00, idx]), "{hz}");
    }
    assert!(a::set_polling_request(3000).is_none());
    // the longer collection (e.g. 66 bytes) is zero padded
    assert_eq!(a::wire(&a::save_request(), 66).len(), 66);
}

// ---------------------------------------------------------------- decode

/// A settings answer the way OpenMouse reads it: `12 04 00`, stage 1 = 800 (enc 7), stage 2 = 1600 (enc 15) from byte 4, the
/// rate byte at 8 (4 + 2 stages × 2), debounce at 10, angle snapping at 12.
fn settings_answer(rate: u8) -> Vec<u8> {
    let mut v = vec![0u8; 64];
    v[..3].copy_from_slice(&[0x12, 0x04, 0x00]);
    v[4..8].copy_from_slice(&[0x07, 0x00, 0x0F, 0x00]);
    v[8] = rate;
    v[10] = 4;
    v[12] = 1;
    v
}

fn profile_answer(profile: u8, stage: u8) -> Vec<u8> {
    let mut v = vec![0u8; 64];
    v[..3].copy_from_slice(&[0x12, 0x00, 0x00]);
    v[10] = profile;
    v[11] = stage;
    v
}

#[test]
fn answers_decode_like_openmouse() {
    let s = a::decode_settings(&settings_answer(3)).unwrap();
    assert_eq!((s.dpi_stages, s.polling_hz, s.rate_byte), ([800, 1600], 1000, 3));
    // (raw + 1) × 100 for each stage; the rate byte & 7 indexes [125, 250, 500, 1000, 2000, 4000, 8000]
    for (raw, hz) in [(0u8, 125u32), (1, 250), (2, 500), (3, 1000), (4, 2000), (6, 8000)] {
        assert_eq!(a::decode_settings(&settings_answer(raw)).unwrap().polling_hz, hz, "{raw}");
    }
    // the high nibble is the booster value, ignored without the booster: 0x13 → & 7 = 3
    assert_eq!(a::decode_settings(&settings_answer(0x13)).unwrap().polling_hz, 1000);
    assert!(a::decode_settings(&settings_answer(7)).is_none(), "index 7: OpenMouse throws");
    // not the settings block
    let mut other = settings_answer(3);
    other[1] = 0x05;
    assert!(a::decode_settings(&other).is_none());
    assert!(a::decode_settings(&settings_answer(3)[..12]).is_none(), "shorter than the angle-snapping byte");
    // the profile: byte 10 = profile (zero-based) → 1-based, byte 11 = DPI stage (1-based) → zero-based
    assert_eq!(a::decode_profile(&profile_answer(0, 1)), Some((1, 0)));
    assert_eq!(a::decode_profile(&profile_answer(2, 2)), Some((3, 1)));
    assert_eq!(a::decode_profile(&profile_answer(0, 0)), None);
    assert_eq!(a::decode_profile(&profile_answer(0, 3)), None, "the Gladius II has two stages");
    assert_eq!(a::decode_profile(&profile_answer(0, 1)[..11]), None);
    // a profile byte of 0xFF must not overflow (u8 + 1): not a profile answer
    assert_eq!(a::decode_profile(&profile_answer(0xFF, 1)), None);
    assert_eq!(a::decode_profile(&profile_answer(0xFE, 1)), Some((0xFF, 0)));
}

// ---------------------------------------------------------------- allow-list + table

#[test]
fn only_the_two_reads_pass_the_allow_list() {
    assert!(a::read_request_allowed(&rep(&[0x12, 0x04, 0x00])));
    assert!(a::read_request_allowed(&rep(&[0x12, 0x00])));
    let mut longer = rep(&[0x12, 0x00]);
    longer.extend_from_slice(&[0, 0]);
    assert!(a::read_request_allowed(&longer), "zero padding");
    // writes, the save, lighting reads (12 03), battery (12 07), lift-off (12 06), a profile switch, the lift-off write
    for p in [
        a::set_dpi_request(0, 800).unwrap().to_vec(),
        a::set_polling_request(500).unwrap().to_vec(),
        a::save_request().to_vec(),
        vec![0x12, 0x03, 0x00],
        vec![0x12, 0x07],
        vec![0x12, 0x06],
        vec![0x12, 0x04, 0x02],
        vec![0x50, 0x02, 0x01],
        vec![0x51, 0x35, 0xFF, 0x00, 0xFF, 0x01],
    ] {
        assert!(!a::read_request_allowed(&rep(&p)), "{p:02x?}");
    }
    let mut junk = rep(&[0x12, 0x00]);
    junk[30] = 1;
    assert!(!a::read_request_allowed(&junk));
    let mut id = rep(&[0x12, 0x00]);
    id[0] = 1;
    assert!(!a::read_request_allowed(&id));
    assert!(!a::read_request_allowed(&rep(&[0x12, 0x00])[..64]));
}

#[test]
fn the_table_is_openmouses_supported_list() {
    // supported-devices-data.ts: ROG Gladius II P502 [0x1845]; the other ASUS entries are "driver" / "pending"
    let c = cfg();
    let m = a::matches(&[&c], 0x0B05, 0x1845).unwrap();
    assert_eq!((m.family, m.model, m.wireless, m.variant), (Family::Asus, Some("ASUS ROG Gladius II P502"), Some(false), 0x1845));
    assert_eq!(m.config.path, c.path);
    assert_eq!(family::find(std::slice::from_ref(&c), 0x0B05, 0x1845).map(|m| m.family), Some(Family::Asus));
    // devices.js knows more ids (Gladius II Core 0x18dd, TUF M5 0x1898, TUF M3 0x1910, Strix Impact III 0x1a88, Harpe Ace
    // 0x1a92 / 0x1a94, the Omni receiver 0x1ace) - the list does not
    for pid in [0x18DDu16, 0x1898, 0x1910, 0x1A88, 0x1A92, 0x1A94, 0x1ACE, 0x0000] {
        let h = hid_info("x", 0x0B05, pid, 0xFF01, 1, (65, 65, 0));
        assert!(a::matches(&[&h], 0x0B05, pid).is_none(), "{pid:04x}");
    }
    assert!(a::matches(&[&c], 0x0B06, 0x1845).is_none());
    // the config collection: usage page 0xFF01 / usage 1 with 64-byte reports both ways
    for (page, usage, lens) in [(0xFF02u16, 1u16, (65u16, 65u16)), (0xFF01, 2, (65, 65)), (0xFF01, 1, (9, 9)), (0xFF01, 1, (65, 0)), (0x01, 0x02, (8, 0))] {
        let h = hid_info("x", 0x0B05, 0x1845, page, usage, (lens.0, lens.1, 0));
        assert!(a::matches(&[&h], 0x0B05, 0x1845).is_none(), "{page:04x}:{usage} {lens:?}");
    }
    let mouse = hid_info("mouse", 0x0B05, 0x1845, 0x01, 0x02, (8, 0, 0));
    assert_eq!(a::matches(&[&mouse, &c], 0x0B05, 0x1845).map(|m| m.config.path), Some(c.path.clone()));
}

#[test]
fn caps_are_what_openmouse_offers() {
    let c = a::caps();
    let d = c.dpi.unwrap();
    assert_eq!((d.min(), d.max()), (100, 12000));
    assert!(d.allows(100) && d.allows(12000) && !d.allows(150) && !d.allows(12100));
    assert_eq!(c.polling, vec![125, 250, 500, 1000], "2000 Hz and up need the Omni polling booster");
    assert!(c.lift_off.is_empty() && !c.battery);
}

// ---------------------------------------------------------------- the driver against a scripted mouse

struct Fake {
    dpi: [u32; 2],
    rate_byte: u8,
    stage: u8,
    asleep: bool,
    silent: bool,
    /// ignore writes
    ignore: bool,
    /// every request (64 bytes), in order
    seen: Vec<Vec<u8>>,
}

fn fake() -> Rc<RefCell<Fake>> {
    Rc::new(RefCell::new(Fake { dpi: [800, 1600], rate_byte: 3, stage: 2, asleep: false, silent: false, ignore: false, seen: Vec::new() }))
}

fn port(st: &Rc<RefCell<Fake>>) -> ScriptPort {
    let st = st.clone();
    ScriptPort::new(cfg(), move |out, how| {
        let HidTransfer::OutputThenMatch { prefixes, reply_len, .. } = how else { panic!("ASUS uses output + matched input reports, not {how:?}") };
        assert_eq!((out.len(), out[0], *reply_len), (65, 0, 65));
        let q = out[1..].to_vec();
        let mut s = st.borrow_mut();
        s.seen.push(q.clone());
        // the accepted answers: the FF AA refusal, or the request's first 3 bytes (2 for the save), behind id 0
        let n = prefixes[1].len() - 1;
        assert_eq!(prefixes[0], vec![0, 0xFF, 0xAA]);
        assert_eq!(prefixes[1], [&[0u8][..], &q[..n]].concat());
        assert_eq!(n, if q[0] == 0x50 { 2 } else { 3 }, "the save matches 2 bytes, everything else 3");
        if s.silent {
            return Err(Error::BadAnswer("no answer in time".into()));
        }
        let mut data = vec![0u8; 64];
        if s.asleep {
            data[0] = 0xFF;
            data[1] = 0xAA;
        } else {
            match (q[0], q[1], q[2]) {
                (0x12, 0x04, 0x00) => {
                    data = settings_answer(s.rate_byte);
                    for i in 0..2 {
                        let enc = s.dpi[i] / 100 - 1;
                        data[4 + i * 2] = enc as u8;
                        data[5 + i * 2] = (enc >> 8) as u8;
                    }
                }
                (0x12, 0x00, 0x00) => data = profile_answer(0, s.stage),
                (0x51, 0x31, field) => {
                    if !s.ignore {
                        if field < 2 {
                            s.dpi[field as usize] = ((q[4] as u32) | (q[5] as u32) << 8) * 100 + 100;
                        } else {
                            s.rate_byte = q[4];
                        }
                    }
                    data[..3].copy_from_slice(&q[..3]);
                }
                (0x50, 0x03, _) => data[..2].copy_from_slice(&[0x50, 0x03]),
                other => panic!("unexpected request {other:02x?}"),
            }
        }
        let mut v = vec![0u8];
        v.extend(data);
        Ok(v)
    })
}

fn matched() -> bu_mouse::family::Matched {
    a::matches(&[&cfg()], 0x0B05, 0x1845).unwrap()
}

fn kinds(st: &Rc<RefCell<Fake>>) -> Vec<(u8, u8)> {
    st.borrow().seen.iter().map(|q| (q[0], q[1])).collect()
}

#[test]
fn reading_asks_settings_and_profile() {
    let st = fake();
    let mut p = port(&st);
    let r = a::Asus.read(&matched(), &mut p, ReadOptions::default()).unwrap();
    assert!(p.sent.iter().all(|s| a::read_request_allowed(&s.0)), "read requests only");
    assert_eq!(kinds(&st), vec![(0x12, 0x04), (0x12, 0x00)]);
    // stage 2 is active → 1600 DPI; 1000 Hz
    assert_eq!((r.online, r.dpi, r.polling_hz, r.battery_percent, r.lift_off, r.wireless), (true, Some(1600), Some(1000), None, None, Some(false)));
    assert_eq!(r.caps, a::caps());
    st.borrow_mut().stage = 1;
    assert_eq!(a::Asus.read(&matched(), &mut port(&st), ReadOptions::default()).unwrap().dpi, Some(800));
}

#[test]
fn an_asleep_or_silent_mouse_is_offline() {
    let st = fake();
    st.borrow_mut().asleep = true;
    let r = a::Asus.read(&matched(), &mut port(&st), ReadOptions::default()).unwrap();
    assert_eq!((r.online, r.dpi, r.polling_hz), (false, None, None));
    assert_eq!(kinds(&st), vec![(0x12, 0x04)], "the FF AA refusal ends it");
    let st = fake();
    st.borrow_mut().silent = true;
    assert!(!a::Asus.read(&matched(), &mut port(&st), ReadOptions::default()).unwrap().online);
    let mut gone = ScriptPort::new(cfg(), |_, _| Err(Error::MouseGone("a".into())));
    assert!(matches!(a::Asus.read(&matched(), &mut gone, ReadOptions::default()), Err(Error::MouseGone(_))));
}

#[test]
fn a_dpi_change_is_profile_read_write_save_read() {
    let st = fake();
    let mut p = port(&st);
    let c = a::Asus.write(&matched(), &mut p, Setting::Dpi(3200)).unwrap();
    assert_eq!((c.now, c.before), (3200, Some(1600)));
    // profile (the active stage), the settings (before), the stage-2 write, the save, the settings (confirm)
    assert_eq!(kinds(&st), vec![(0x12, 0x00), (0x12, 0x04), (0x51, 0x31), (0x50, 0x03), (0x12, 0x04)]);
    // stage index 1, 3200 → enc 31 = 0x1F
    assert_eq!(p.sent[2].0, rep(&[0x51, 0x31, 0x01, 0x00, 0x1F, 0x00]));
    assert_eq!(p.sent[3].0, rep(&[0x50, 0x03]));
    assert!(!a::read_request_allowed(&p.sent[2].0) && !a::read_request_allowed(&p.sent[3].0));
    assert_eq!(st.borrow().dpi, [800, 3200], "only the active stage changed");
    // the first stage when it is the active one
    st.borrow_mut().stage = 1;
    a::Asus.write(&matched(), &mut port(&st), Setting::Dpi(400)).unwrap();
    assert_eq!(st.borrow().dpi, [400, 3200]);
}

#[test]
fn a_polling_change_is_write_save_read() {
    let st = fake();
    let mut p = port(&st);
    let c = a::Asus.write(&matched(), &mut p, Setting::Polling(500)).unwrap();
    assert_eq!((c.now, c.before), (500, Some(1000)));
    assert_eq!(kinds(&st), vec![(0x12, 0x04), (0x51, 0x31), (0x50, 0x03), (0x12, 0x04)]);
    assert_eq!(p.sent[1].0, rep(&[0x51, 0x31, 0x02, 0x00, 0x02]));
    assert_eq!(st.borrow().rate_byte, 2);
    assert_eq!(a::Asus.write(&matched(), &mut port(&st), Setting::Polling(125)).unwrap().before, Some(500));
    assert_eq!(st.borrow().rate_byte, 0);
}

#[test]
fn a_rate_byte_with_more_than_the_rate_is_not_overwritten() {
    let st = fake();
    st.borrow_mut().rate_byte = 0x13; // booster nibble set: the write would clear it
    let mut p = port(&st);
    let e = a::Asus.write(&matched(), &mut p, Setting::Polling(500)).unwrap_err();
    assert!(matches!(e, Error::BadAnswer(ref m) if m.contains("nothing was written")), "{e:?}");
    assert_eq!(kinds(&st), vec![(0x12, 0x04)]);
}

#[test]
fn a_change_the_mouse_does_not_keep_is_an_error() {
    let st = fake();
    st.borrow_mut().ignore = true;
    let e = a::Asus.write(&matched(), &mut port(&st), Setting::Dpi(3200)).unwrap_err();
    assert!(matches!(e, Error::BadAnswer(ref m) if m.contains("kept 1600")), "{e:?}");
    let e = a::Asus.write(&matched(), &mut port(&st), Setting::Polling(500)).unwrap_err();
    assert!(matches!(e, Error::BadAnswer(ref m) if m.contains("kept 1000")), "{e:?}");
    // a sleeping mouse refuses the write
    let st = fake();
    st.borrow_mut().asleep = true;
    assert!(a::Asus.write(&matched(), &mut port(&st), Setting::Dpi(3200)).is_err());
}

#[test]
fn values_the_mouse_cannot_store_are_refused_before_a_byte_leaves() {
    let st = fake();
    let mut p = port(&st);
    for s in [Setting::Dpi(50), Setting::Dpi(150), Setting::Dpi(12100), Setting::Polling(2000), Setting::Polling(8000), Setting::Polling(0), Setting::LiftOff(10)] {
        assert!(a::Asus.write(&matched(), &mut p, s).is_err(), "{s:?}");
    }
    assert!(p.sent.is_empty());
}

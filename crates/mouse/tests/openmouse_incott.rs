//! Order 105 check, Incott: our requests = the bytes OpenMouse sends for the same setting (encode), and answers shaped the
//! way OpenMouse reads them decode to the same values (decode). The expected bytes are worked out by hand from OpenMouse
//! (@openmouse/protocol 0.40.1: `dist/incott/index.js` - the codec - and `dist/drivers/incott/hid.js`), never from our own
//! encoder. Never run on a real unit.
//!
//! How OpenMouse counts: `sendFeatureReport(9, payload)` - the payload is 8 bytes WITHOUT the report id; an answer
//! (`receiveFeatureReport(9)`) comes WITH the id at byte 0 (the codec checks `frame[0] === 0x09`). Windows' feature buffer is
//! the report id + the payload, zero-padded to the collection's feature length (65 here).

use bu_mouse::family::{self, Driver, Family, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::incott as inc;
use bu_mouse::os::{HidInfo, HidTransfer};
use bu_mouse::{Error, Result};
use std::cell::RefCell;
use std::rc::Rc;

/// The report as Windows sends it: report id 9, OpenMouse's payload, zeros up to the 65-byte feature length.
fn rep(payload: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 65];
    v[0] = 0x09;
    v[1..1 + payload.len()].copy_from_slice(payload);
    v
}

fn frame(bytes: &[u8]) -> [u8; 64] {
    let mut f = [0u8; 64];
    f[..bytes.len()].copy_from_slice(bytes);
    f
}

/// The vendor collection of an Incott mouse as Windows lists it.
fn col(pid: u16) -> HidInfo {
    hid_info(&format!("inc{pid:04x}"), 0x093A, pid, 0xFF05, 0x01, (0, 0, 65))
}

// ---------------------------------------------------------------- encode: OpenMouse's bytes, by hand

#[test]
fn set_dpi_bytes_are_openmouses() {
    // incottEncodeSetDpi(stage, dpi, "both") = payload(0x02, stage, wire & 0xff, wire >> 8, 0, 0, 0, axis 0), wire = dpi / 50 - 1
    // 800: 800 / 50 - 1 = 15 = 0x0f
    assert_eq!(inc::encode_set_dpi(2, 800), Some([0x02, 0x02, 0x0F, 0x00, 0, 0, 0, 0]));
    // 45000: 899 = 0x0383 -> lo 0x83, hi 0x03
    assert_eq!(inc::encode_set_dpi(0, 45000), Some([0x02, 0x00, 0x83, 0x03, 0, 0, 0, 0]));
    // 50: wire 0, stage 5
    assert_eq!(inc::encode_set_dpi(5, 50), Some([0x02, 0x05, 0x00, 0x00, 0, 0, 0, 0]));
    // 12000: 239 = 0xef
    assert_eq!(inc::encode_set_dpi(1, 12000), Some([0x02, 0x01, 0xEF, 0x00, 0, 0, 0, 0]));
    // incottValidateDpi: 50..45000, multiples of 50; stages 0..5
    assert_eq!(inc::encode_set_dpi(0, 49), None);
    assert_eq!(inc::encode_set_dpi(0, 45050), None);
    assert_eq!(inc::encode_set_dpi(0, 810), None);
    assert_eq!(inc::encode_set_dpi(6, 800), None);
    // the feature report Windows sends
    assert_eq!(inc::report(&inc::encode_set_dpi(2, 800).unwrap(), 65), rep(&[0x02, 0x02, 0x0F]));
}

#[test]
fn set_polling_bytes_are_openmouses() {
    // incottEncodeSetPollingRate = payload(0x01, wire); INCOTT_POLLING_HZ_TO_WIRE: 1000:0 500:1 250:2 125:3 8000:4 4000:5 2000:6
    for (hz, wire) in [(1000, 0), (500, 1), (250, 2), (125, 3), (8000, 4), (4000, 5), (2000, 6)] {
        assert_eq!(inc::encode_set_polling(hz), Some([0x01, wire, 0, 0, 0, 0, 0, 0]), "{hz}");
        assert_eq!(inc::polling_hz(wire), Some(hz), "{hz} wire -> Hz");
    }
    assert_eq!(inc::encode_set_polling(750), None);
    assert_eq!(inc::polling_hz(7), None);
}

#[test]
fn set_lift_off_bytes_are_openmouses() {
    // incottEncodeSetLiftOff = payload(0x04, SUB_LOD 0x01, wire); INCOTT_LOD_TENTHS_TO_WIRE: 10 -> 0, 20 -> 1, 7 -> 2
    assert_eq!(inc::encode_set_lift_off(7), Some([0x04, 0x01, 0x02, 0, 0, 0, 0, 0]));
    assert_eq!(inc::encode_set_lift_off(10), Some([0x04, 0x01, 0x00, 0, 0, 0, 0, 0]));
    assert_eq!(inc::encode_set_lift_off(20), Some([0x04, 0x01, 0x01, 0, 0, 0, 0, 0]));
    assert_eq!(inc::encode_set_lift_off(15), None);
}

#[test]
fn query_bytes_are_openmouses() {
    // incottEncodeQuery(cmd, sub) = payload(cmd, sub)
    assert_eq!(inc::query(0x8F, 0), [0x8F, 0, 0, 0, 0, 0, 0, 0], "identity");
    assert_eq!(inc::query(0x83, 0), [0x83, 0, 0, 0, 0, 0, 0, 0], "DPI cycle");
    assert_eq!(inc::query(0x82, 3), [0x82, 3, 0, 0, 0, 0, 0, 0], "DPI stage 3's value");
    assert_eq!(inc::query(0x81, 0), [0x81, 0, 0, 0, 0, 0, 0, 0], "polling");
    assert_eq!(inc::query(0x84, 1), [0x84, 1, 0, 0, 0, 0, 0, 0], "lift-off");
}

// ---------------------------------------------------------------- decode: answers as OpenMouse reads them

#[test]
fn answers_decode_the_openmouse_way() {
    // the DPI cycle: "09 83 00 answers 09 83 06 01" (hid.js / index.js comments, hardware): 6 stages, stage 1 active
    assert_eq!(inc::decode_dpi_cycle(&frame(&[0x09, 0x83, 0x06, 0x01])), Some((6, 1)));
    // a four-stage cycle: byte 2 is the COUNT, not an echo
    assert_eq!(inc::decode_dpi_cycle(&frame(&[0x09, 0x83, 0x04, 0x03])), Some((4, 3)));
    assert_eq!(inc::decode_dpi_cycle(&frame(&[0x09, 0x83, 0x04, 0x04])), None, "active past the count");
    assert_eq!(inc::decode_dpi_cycle(&frame(&[0x09, 0x83, 0x00, 0x00])), None, "no stages");
    assert_eq!(inc::decode_dpi_cycle(&frame(&[0x09, 0x83, 0x07, 0x00])), None, "more than six");
    assert_eq!(inc::decode_dpi_cycle(&frame(&[0x09, 0x82, 0x06, 0x01])), None, "another command's answer");
    assert_eq!(inc::decode_dpi_cycle(&frame(&[0x0A, 0x83, 0x06, 0x01])), None, "another report id");
    // a stage's value: "wire 0x07/0x0f/0x1f/0x2f/0x3f/0x7f -> 400/800/1600/2400/3200/6400" (index.js comment, hardware)
    for (stage, wire, dpi) in [(0u8, 0x07u8, 400u32), (1, 0x0F, 800), (2, 0x1F, 1600), (3, 0x2F, 2400), (4, 0x3F, 3200), (5, 0x7F, 6400)] {
        assert_eq!(inc::decode_dpi_stage(&frame(&[0x09, 0x82, stage, wire, 0x00]), stage), Some(dpi), "stage {stage}");
    }
    // wire 0x0383 = 899 -> (899 + 1) * 50 = 45000; wire 0x0384 -> 45050 is out of range
    assert_eq!(inc::decode_dpi_stage(&frame(&[0x09, 0x82, 0x00, 0x83, 0x03]), 0), Some(45000));
    assert_eq!(inc::decode_dpi_stage(&frame(&[0x09, 0x82, 0x00, 0x84, 0x03]), 0), None);
    // the sub-command echo (byte 2) must be the stage asked for
    assert_eq!(inc::decode_dpi_stage(&frame(&[0x09, 0x82, 0x02, 0x0F, 0x00]), 3), None, "stage 2's answer to stage 3's request");
    // polling: byte 2 is data; 1000 Hz = 0 ... 125 Hz = 3 ... 2000 Hz = 6
    assert_eq!(inc::decode_polling(&frame(&[0x09, 0x81, 0x00])), Some(1000));
    assert_eq!(inc::decode_polling(&frame(&[0x09, 0x81, 0x03])), Some(125));
    assert_eq!(inc::decode_polling(&frame(&[0x09, 0x81, 0x04])), Some(8000));
    assert_eq!(inc::decode_polling(&frame(&[0x09, 0x81, 0x09])), None);
    // lift-off: "09 84 01 02" after setting 0.7 mm = hardware 2 = 0.7 mm (index.js comment); 0 = 1 mm, 1 = 2 mm
    assert_eq!(inc::decode_lift_off(&frame(&[0x09, 0x84, 0x01, 0x02])), Some(7));
    assert_eq!(inc::decode_lift_off(&frame(&[0x09, 0x84, 0x01, 0x00])), Some(10));
    assert_eq!(inc::decode_lift_off(&frame(&[0x09, 0x84, 0x01, 0x01])), Some(20));
    assert_eq!(inc::decode_lift_off(&frame(&[0x09, 0x84, 0x01, 0x03])), None);
    assert_eq!(inc::decode_lift_off(&frame(&[0x09, 0x84, 0x02, 0x02])), None, "another sensor sub-command (ripple)");
    // identity: guard [2] = 1, model code [3] (0x08 / 0x0e = G23V2), sensor id 0xf1 in [5] or [6] = PAW3950 = "Pro"
    let id = inc::decode_identity(&frame(&[0x09, 0x8F, 0x01, 0x08, 0x02, 0xF1])).unwrap();
    assert_eq!((id.name.as_deref(), id.paw3950), (Some("G23V2 Pro"), Some(true)));
    let id = inc::decode_identity(&frame(&[0x09, 0x8F, 0x01, 0x0E, 0x00, 0x00, 0xF1])).unwrap();
    assert_eq!((id.name.as_deref(), id.paw3950), (Some("G23V2 Pro"), Some(true)));
    let id = inc::decode_identity(&frame(&[0x09, 0x8F, 0x01, 0x0E, 0x00, 0x00, 0x00])).unwrap();
    assert_eq!((id.name.as_deref(), id.paw3950), (Some("G23V2"), Some(false)));
    let id = inc::decode_identity(&frame(&[0x09, 0x8F, 0x01, 0x63])).unwrap();
    assert_eq!(id.name, None, "a model code outside OpenMouse's table has no name");
    let id = inc::decode_identity(&frame(&[0x09, 0x8F, 0x00, 0x08])).unwrap();
    assert_eq!((id.name, id.paw3950), (None, None), "no guard byte = the shape is unknown");
    assert!(inc::decode_identity(&frame(&[0x09, 0x83, 0x01, 0x08])).is_none());
    assert!(inc::decode_identity(&frame(&[])).is_none(), "a silent (all-zero) buffer is not an identity");
}

#[test]
fn caps_follow_the_connection_and_the_sensor() {
    let wired = inc::caps(false, Some(true));
    assert_eq!(wired.polling, vec![125, 250, 500, 1000], "the cable only reaches 1000 Hz");
    assert_eq!(wired.lift_off, vec![7, 10, 20]);
    assert!(!wired.battery, "battery is an unsolicited input report - no request to send");
    let d = wired.dpi.unwrap();
    assert_eq!((d.min(), d.max()), (50, 45000));
    assert!(d.allows(50) && d.allows(45000) && !d.allows(75));
    let air = inc::caps(true, Some(false));
    assert_eq!(air.polling, vec![125, 250, 500, 1000, 2000, 4000, 8000]);
    assert_eq!(air.dpi.unwrap().max(), 32000, "the PAW3395 models stop at 32000");
    assert_eq!(inc::caps(true, None).dpi.unwrap().max(), 45000, "unknown sensor: the higher ceiling");
}

// ---------------------------------------------------------------- the allow-list

#[test]
fn only_read_queries_pass_the_allow_list() {
    for p in [inc::query(0x8F, 0), inc::query(0x83, 0), inc::query(0x81, 0), inc::query(0x84, 1), inc::query(0x82, 0), inc::query(0x82, 5)] {
        assert!(inc::read_request_allowed(&inc::report(&p, 65)), "{p:02x?}");
        assert!(inc::read_request_allowed(&inc::report(&p, 9)), "{p:02x?} unpadded");
    }
    // every write this driver sends is refused
    for p in [inc::encode_set_dpi(0, 800).unwrap(), inc::encode_set_polling(1000).unwrap(), inc::encode_set_lift_off(10).unwrap()] {
        assert!(!inc::read_request_allowed(&inc::report(&p, 65)), "{p:02x?}");
    }
    // the other commands of the protocol (set cycle, set button, receiver LED, set timing, macro chunk) and other reads
    for first in [0x03u8, 0x05, 0x06, 0x07, 0x08, 0x85, 0x86, 0x88, 0x89, 0x8E, 0x00, 0xFF] {
        assert!(!inc::read_request_allowed(&rep(&[first, 0x01, 0x02])), "{first:02x}");
    }
    // a read with something extra set is not one of OpenMouse's reads
    assert!(!inc::read_request_allowed(&rep(&[0x84, 0x02])), "sensor query for another sub-command");
    assert!(!inc::read_request_allowed(&rep(&[0x84, 0x01, 0x05])));
    assert!(!inc::read_request_allowed(&rep(&[0x82, 0x06])), "a stage that does not exist");
    assert!(!inc::read_request_allowed(&rep(&[0x82, 0x01, 0x02])), "an axis query");
    assert!(!inc::read_request_allowed(&rep(&[0x81, 0x01])));
    let mut tail = rep(&[0x81, 0x00]);
    tail[40] = 1;
    assert!(!inc::read_request_allowed(&tail), "data in the padding");
    // wrong report id / too short
    let mut other = rep(&[0x81, 0x00]);
    other[0] = 0x0A;
    assert!(!inc::read_request_allowed(&other));
    assert!(!inc::read_request_allowed(&[0x09, 0x81]));
    assert!(!inc::read_request_allowed(&[]));
}

// ---------------------------------------------------------------- the device table

type Matcher = fn(&[&HidInfo], u16, u16) -> Option<bu_mouse::family::Matched>;

/// Every other family's matcher (the finished ones and the other Order 105 modules).
fn others() -> Vec<(&'static str, Matcher)> {
    use bu_mouse::*;
    vec![
        ("pulsar cmouse", pulsar::matches_cmouse as Matcher),
        ("pulsar xs1", pulsar::matches_xs1),
        ("logitech", logitech::matches),
        ("razer", razer::matches),
        ("steelseries", steelseries::matches),
        ("atk", atk::matches),
        ("endgame", endgame::matches),
        ("lamzu", lamzu::matches),
        ("wlmouse", wlmouse::matches),
        ("redragon", redragon::matches),
        ("attackshark", attackshark::matches),
        ("gearhub", gearhub::matches),
        ("vaxee", vaxee::matches),
        ("corsair", corsair::matches),
        ("glorious", glorious::matches),
        ("ninjutso", ninjutso::matches),
        ("finalmouse", finalmouse::matches),
        ("zaunkoenig", zaunkoenig::matches),
        ("orbital", orbital::matches),
        ("rawm", rawm::matches),
        ("mchose", mchose::matches),
        ("delux", delux::matches),
        ("dareu", dareu::matches),
        ("keychron", keychron::matches),
        ("ksnake", ksnake::matches),
        ("microsoft", microsoft::matches),
        ("moddo", moddo::matches),
        ("teevolution", teevolution::matches),
        ("wallhack", wallhack::matches),
        ("vgn", vgn::matches),
        ("asus", asus::matches),
        ("ryunix", ryunix::matches),
        ("rapoo", rapoo::matches),
    ]
}

#[test]
fn both_supported_ids_match_and_nothing_else_does() {
    // OpenMouse's SUPPORTED list: "G23 V2 (SE / Pro)", pids 0x522c (the 2.4 GHz dongle) and 0x622c (the cable)
    for (pid, wireless) in [(0x522C, true), (0x622C, false)] {
        let c = col(pid);
        let m = inc::matches(&[&c], 0x093A, pid).unwrap_or_else(|| panic!("{pid:04x} must match"));
        assert_eq!((m.family, m.model, m.wireless, m.variant), (Family::Incott, None, Some(wireless), pid));
        assert_eq!(m.config.path, c.path);
        // no other family claims it, and the registry picks Incott
        for (name, f) in others() {
            assert!(f(&[&c], 0x093A, pid).is_none(), "{name} must not claim {pid:04x}");
        }
        assert_eq!(family::find(std::slice::from_ref(&c), 0x093A, pid).map(|m| m.family), Some(Family::Incott));
    }
    // a wrong id / vendor, or a collection that is not the 0xFF05 one, does not match
    assert!(inc::matches(&[&col(0x522D)], 0x093A, 0x522D).is_none(), "another product id");
    assert!(inc::matches(&[&col(0x522C)], 0x3710, 0x522C).is_none(), "another vendor");
    let mut other_page = col(0x522C);
    other_page.usage_page = 0xFF00;
    assert!(inc::matches(&[&other_page], 0x093A, 0x522C).is_none(), "OpenMouse's isSupported wants usage page 0xFF05");
    let mut mouse = col(0x522C);
    mouse.usage_page = 0x01;
    mouse.usage = 0x02;
    assert!(inc::matches(&[&mouse], 0x093A, 0x522C).is_none(), "the mouse collection is no config channel");
    let mut no_feature = col(0x522C);
    no_feature.feature_len = 0;
    assert!(inc::matches(&[&no_feature], 0x093A, 0x522C).is_none(), "no feature report 9");
    // of a mouse collection plus the 0xFF05 one, the 0xFF05 one
    let mut m_col = col(0x622C);
    m_col.usage_page = 0x01;
    m_col.usage = 0x02;
    m_col.path = "mouse".into();
    let c = col(0x622C);
    assert_eq!(inc::matches(&[&m_col, &c], 0x093A, 0x622C).map(|m| m.config.path), Some(c.path.clone()));
}

// ---------------------------------------------------------------- the driver against a scripted mouse

/// A fake Incott mouse: one shared answer buffer (like the real one), six DPI stages, a polling code, a lift-off code.
struct Model {
    stages: [u32; 6],
    count: u8,
    active: u8,
    polling_wire: u8,
    lod_wire: u8,
    identity: Vec<u8>,
    latched: [u8; 64],
    /// ignore every write (a mouse that keeps its value)
    refuse_writes: bool,
    /// answer nothing (asleep): the buffer keeps whatever it held
    silent: bool,
    /// the cable's dead twin: HidD_SetFeature fails
    dead: bool,
    /// the first N fetches of a fresh answer return the previous (stale) frame
    stale_reads: u32,
    /// the next N fetches after a request fail with an OS error (HidD_GetFeature failing)
    fetch_errors: u32,
    /// the frame that was latched before the last request (what a stale fetch returns)
    prev: [u8; 64],
    /// a request was sent and its first fetch has not happened yet
    fresh: bool,
    log: Vec<(Vec<u8>, HidTransfer)>,
}

impl Model {
    fn new() -> Self {
        Model {
            stages: [400, 800, 1600, 2400, 3200, 6400],
            count: 6,
            active: 1,
            polling_wire: 0,
            lod_wire: 0,
            identity: vec![0x09, 0x8F, 0x01, 0x08, 0x02, 0xF1],
            latched: [0; 64],
            refuse_writes: false,
            silent: false,
            dead: false,
            stale_reads: 0,
            fetch_errors: 0,
            prev: [0; 64],
            fresh: false,
            log: Vec::new(),
        }
    }

    fn answer(&mut self, req: &[u8]) {
        let p = &req[1..];
        let f = &mut self.latched;
        *f = [0; 64];
        f[0] = 0x09;
        f[1] = p[0];
        match p[0] {
            0x8F => f[..self.identity.len()].copy_from_slice(&self.identity),
            0x83 => {
                f[2] = self.count;
                f[3] = self.active;
            }
            0x82 => {
                f[2] = p[1];
                let wire = self.stages[p[1] as usize] / 50 - 1;
                f[3] = wire as u8;
                f[4] = (wire >> 8) as u8;
            }
            0x81 => f[2] = self.polling_wire,
            0x84 => {
                f[2] = p[1];
                f[3] = self.lod_wire;
            }
            _ => {}
        }
    }

    fn write(&mut self, req: &[u8]) {
        if self.refuse_writes {
            return;
        }
        let p = &req[1..];
        match p[0] {
            0x02 => self.stages[p[1] as usize] = ((p[2] as u32 | (p[3] as u32) << 8) + 1) * 50,
            0x01 => self.polling_wire = p[1],
            0x04 if p[1] == 0x01 => self.lod_wire = p[2],
            other => panic!("the driver sent command {other:02x}, which no setting needs"),
        }
    }
}

fn port(info: HidInfo, m: Rc<RefCell<Model>>) -> ScriptPort {
    ScriptPort::new(info, move |out, how| {
        let mut m = m.borrow_mut();
        m.log.push((out.to_vec(), how.clone()));
        match how {
            HidTransfer::FeatureOut => {
                if m.dead {
                    return Err(Error::Os { op: "HidD_SetFeature".into(), code: 1 });
                }
                if out[1] >= 0x80 {
                    // a query: the answer is latched in the shared buffer, fetched by the next HidD_GetFeature
                    m.prev = m.latched;
                    m.fresh = true;
                    if !m.silent {
                        m.answer(out);
                    }
                } else if !m.silent {
                    m.write(out);
                }
                Ok(Vec::new())
            }
            HidTransfer::FeatureIn { reply_len, .. } => {
                let mut frame = m.latched;
                if m.fresh {
                    if m.fetch_errors > 0 {
                        m.fetch_errors -= 1;
                        return Err(Error::Os { op: "HidD_GetFeature".into(), code: 31 });
                    }
                    m.fresh = false;
                    if m.stale_reads > 0 {
                        // the buffer still holds the previous answer for this fetch
                        m.stale_reads -= 1;
                        frame = m.prev;
                    }
                }
                let mut v = frame.to_vec();
                v.resize(*reply_len, 0);
                Ok(v)
            }
            other => panic!("Incott talks in feature reports, not {other:?}"),
        }
    })
}

fn setup(pid: u16) -> (bu_mouse::family::Matched, Rc<RefCell<Model>>, ScriptPort) {
    let info = col(pid);
    let m = inc::matches(&[&info], 0x093A, pid).unwrap();
    let model = Rc::new(RefCell::new(Model::new()));
    let p = port(info, model.clone());
    (m, model, p)
}

/// The writes the driver made, in order (the FeatureOut reports).
fn writes(model: &Rc<RefCell<Model>>) -> Vec<Vec<u8>> {
    model.borrow().log.iter().filter(|(o, h)| matches!(h, HidTransfer::FeatureOut) && o[1] < 0x80).map(|(o, _)| o.clone()).collect()
}

#[test]
fn reading_asks_only_what_openmouse_asks() {
    let (m, model, mut p) = setup(0x522C);
    let r = inc::Incott.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(r.online);
    assert_eq!((r.dpi, r.polling_hz, r.lift_off), (Some(800), Some(1000), Some(10)));
    assert_eq!((r.battery_percent, r.charging), (None, None), "battery needs a passive read: not ported");
    assert_eq!(r.model.as_deref(), Some("Incott G23V2 Pro"));
    assert_eq!(r.wireless, Some(true));
    assert_eq!(r.caps.polling.last(), Some(&8000));
    assert_eq!(r.caps.dpi.as_ref().unwrap().max(), 45000);
    assert!(!r.caps.battery);
    // nothing was written and every request passes the read-only allow-list
    assert!(writes(&model).is_empty());
    assert!(model.borrow().log.iter().all(|(o, _)| inc::read_request_allowed(o)));
    // the requests OpenMouse sends, in order: identity, cycle, the ACTIVE stage's value, polling, lift-off - each as
    // "discard fetch, then set, then fetch after 50 ms"
    let sent: Vec<Vec<u8>> = model.borrow().log.iter().filter(|(_, h)| matches!(h, HidTransfer::FeatureOut)).map(|(o, _)| o.clone()).collect();
    assert_eq!(sent, vec![rep(&[0x8F]), rep(&[0x83]), rep(&[0x82, 1]), rep(&[0x81]), rep(&[0x84, 1])]);
    let first_three: Vec<HidTransfer> = model.borrow().log.iter().take(3).map(|(_, h)| h.clone()).collect();
    assert_eq!(
        first_three,
        vec![HidTransfer::FeatureIn { reply_id: 9, reply_len: 65, wait_ms: 0 }, HidTransfer::FeatureOut, HidTransfer::FeatureIn { reply_id: 9, reply_len: 65, wait_ms: 50 }]
    );
}

#[test]
fn a_stale_frame_is_not_taken_for_the_answer() {
    // the shared buffer still holds the previous answer: OpenMouse's incottFrameMatches rejects it and reads again
    let (m, model, mut p) = setup(0x522C);
    model.borrow_mut().stale_reads = 3;
    let r = inc::Incott.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(r.online);
    assert_eq!((r.dpi, r.polling_hz, r.lift_off), (Some(800), Some(1000), Some(10)));
}

#[test]
fn a_silent_mouse_is_offline_and_nothing_is_invented() {
    let (m, model, mut p) = setup(0x522C);
    model.borrow_mut().silent = true;
    let r = inc::Incott.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(!r.online);
    assert_eq!((r.dpi, r.polling_hz, r.lift_off, r.battery_percent, r.model), (None, None, None, None, None));
    // one request (10 tries at most), then it gives up
    let fetches = model.borrow().log.len();
    assert!(fetches <= 12, "gave up after {fetches} transfers");
    // a write to it is refused, nothing written
    for s in [Setting::Dpi(800), Setting::Polling(500), Setting::LiftOff(10)] {
        assert!(inc::Incott.write(&m, &mut p, s).is_err());
    }
    assert!(writes(&model).is_empty());
}

#[test]
fn the_cables_dead_twin_collection_is_offline() {
    // HidD_SetFeature "Incorrect function" on the look-alike collection: OpenMouse's open() probe marks it dead
    let (m, model, mut p) = setup(0x622C);
    model.borrow_mut().dead = true;
    let r = inc::Incott.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(!r.online);
    assert_eq!(r.wireless, Some(false));
    assert!(inc::Incott.write(&m, &mut p, Setting::Dpi(800)).is_err());
}

#[test]
fn a_mouse_that_is_gone_is_an_error_not_offline() {
    let info = col(0x522C);
    let m = inc::matches(&[&info], 0x093A, 0x522C).unwrap();
    let mut p = ScriptPort::new(info, |_, _| -> Result<Vec<u8>> { Err(Error::MouseGone("inc".into())) });
    assert!(matches!(inc::Incott.read(&m, &mut p, ReadOptions::default()), Err(Error::MouseGone(_))));
}

#[test]
fn dpi_change_writes_the_active_stage_then_reads_it_back() {
    let (m, model, mut p) = setup(0x522C);
    let c = inc::Incott.write(&m, &mut p, Setting::Dpi(1600)).unwrap();
    assert_eq!((c.now, c.before), (1600, Some(800)));
    // the one write: stage 1 (active), 1600 -> 1600 / 50 - 1 = 31 = 0x1f
    assert_eq!(writes(&model), vec![rep(&[0x02, 0x01, 0x1F])]);
    assert_eq!(model.borrow().stages, [400, 1600, 1600, 2400, 3200, 6400], "only the active stage changed");
    assert_eq!(model.borrow().active, 1);
    assert!(model.borrow().log.iter().filter(|(_, h)| !matches!(h, HidTransfer::FeatureOut)).all(|(o, _)| inc::read_request_allowed(o)));
    // an out-of-range / off-step value is refused before a byte leaves
    model.borrow_mut().log.clear();
    for bad in [49, 810, 45050, 0] {
        assert!(inc::Incott.write(&m, &mut p, Setting::Dpi(bad)).is_err(), "{bad}");
    }
    assert!(model.borrow().log.is_empty());
    // the top of the range (a Pro): 45000 -> 0x0383
    inc::Incott.write(&m, &mut p, Setting::Dpi(45000)).unwrap();
    assert_eq!(writes(&model), vec![rep(&[0x02, 0x01, 0x83, 0x03])]);
}

#[test]
fn polling_change_writes_the_wire_code_then_reads_it_back() {
    let (m, model, mut p) = setup(0x522C);
    let c = inc::Incott.write(&m, &mut p, Setting::Polling(4000)).unwrap();
    assert_eq!((c.now, c.before), (4000, Some(1000)));
    assert_eq!(writes(&model), vec![rep(&[0x01, 0x05])]);
    assert_eq!(model.borrow().polling_wire, 5);
    assert!(inc::Incott.write(&m, &mut p, Setting::Polling(3000)).is_err());
    // the cable stops at 1000 Hz: 2000 is refused before a byte leaves, 500 goes through
    let (m, model, mut p) = setup(0x622C);
    assert!(inc::Incott.write(&m, &mut p, Setting::Polling(2000)).is_err());
    assert!(model.borrow().log.is_empty());
    inc::Incott.write(&m, &mut p, Setting::Polling(500)).unwrap();
    assert_eq!(writes(&model), vec![rep(&[0x01, 0x01])]);
}

#[test]
fn lift_off_change_writes_the_wire_code_then_reads_it_back() {
    let (m, model, mut p) = setup(0x522C);
    let c = inc::Incott.write(&m, &mut p, Setting::LiftOff(7)).unwrap();
    assert_eq!((c.now, c.before), (7, Some(10)));
    assert_eq!(writes(&model), vec![rep(&[0x04, 0x01, 0x02])]);
    inc::Incott.write(&m, &mut p, Setting::LiftOff(20)).unwrap();
    assert_eq!(writes(&model).last(), Some(&rep(&[0x04, 0x01, 0x01])));
    model.borrow_mut().log.clear();
    assert!(inc::Incott.write(&m, &mut p, Setting::LiftOff(15)).is_err());
    assert!(model.borrow().log.is_empty());
}

#[test]
fn a_mouse_that_keeps_its_old_value_is_an_error() {
    // OpenMouse: "The mouse kept X DPI instead of Y" - the change is not reported as done
    let (m, model, mut p) = setup(0x522C);
    model.borrow_mut().refuse_writes = true;
    assert!(matches!(inc::Incott.write(&m, &mut p, Setting::Dpi(1600)), Err(Error::BadAnswer(_))));
    assert!(matches!(inc::Incott.write(&m, &mut p, Setting::Polling(500)), Err(Error::BadAnswer(_))));
    assert!(matches!(inc::Incott.write(&m, &mut p, Setting::LiftOff(20)), Err(Error::BadAnswer(_))));
    assert_eq!(model.borrow().stages[1], 800);
}

#[test]
fn a_four_stage_cycle_changes_the_active_stage_only() {
    let (m, model, mut p) = setup(0x522C);
    {
        let mut mm = model.borrow_mut();
        mm.count = 4;
        mm.active = 3;
    }
    let c = inc::Incott.write(&m, &mut p, Setting::Dpi(3000)).unwrap();
    assert_eq!((c.now, c.before), (3000, Some(2400)));
    assert_eq!(writes(&model), vec![rep(&[0x02, 0x03, 0x3B])], "stage 3: 3000 / 50 - 1 = 59 = 0x3b");
}

#[test]
fn a_failing_first_fetch_is_just_a_try_only_a_failing_send_ends_the_exchange() {
    // OpenMouse's read() catches an error from the GetFeature and the loop reads again (up to 10 times); only the SEND failing
    // (the dead twin) ends the exchange. The request goes out as a plain SetFeature (FeatureOut), the answer is fetched after it.
    let (m, model, mut p) = setup(0x522C);
    model.borrow_mut().fetch_errors = 3;
    let r = inc::Incott.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(r.online, "the first request's fetch failed once, then answered");
    assert_eq!((r.dpi, r.polling_hz, r.lift_off), (Some(800), Some(1000), Some(10)));
    // the identity request: discard fetch, ONE send, then three failing fetches and the fourth answers
    let log = model.borrow().log.clone();
    assert_eq!(log[1].1, HidTransfer::FeatureOut);
    assert_eq!(log[1].0, rep(&[0x8F]));
    assert_eq!(log[2].1, HidTransfer::FeatureIn { reply_id: 9, reply_len: 65, wait_ms: 50 });
    assert_eq!(log.iter().take(6).filter(|(_, h)| matches!(h, HidTransfer::FeatureOut)).count(), 1, "the request is sent once, not again");
    assert!(log[2..6].iter().all(|(_, h)| matches!(h, HidTransfer::FeatureIn { wait_ms: 50, .. })));
    // a mouse whose every fetch fails: ten tries, then offline - not an error, and nothing is written
    let (m, model, mut p) = setup(0x522C);
    model.borrow_mut().fetch_errors = 1000;
    let r = inc::Incott.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(!r.online);
    assert!(model.borrow().log.len() <= 12);
    assert!(writes(&model).is_empty());
    // a send that fails stops at once: discard fetch + the one send
    let (m, model, mut p) = setup(0x622C);
    model.borrow_mut().dead = true;
    let r = inc::Incott.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(!r.online);
    assert_eq!(model.borrow().log.len(), 2);
    // every transfer the driver makes passes the read-only gate: fetches send nothing, the sends are allow-listed reads
    let (m, model, mut p) = setup(0x522C);
    inc::Incott.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(model.borrow().log.iter().all(|(o, h)| matches!(h, HidTransfer::FeatureIn { .. }) || inc::read_request_allowed(o)));
}

#[test]
fn the_cables_two_look_alike_collections_are_picked_the_same_way_in_any_order() {
    // two 0xFF05 collections with feature report 9 on the cable; which one answers is unclear from here, so the pick must not
    // depend on the order Windows lists them in: lowest interface, then the larger feature report, then usage, then path
    let mut a = col(0x622C);
    a.path = "a".into();
    a.interface = Some(1);
    let mut b = col(0x622C);
    b.path = "b".into();
    b.interface = Some(2);
    let pick = |list: &[&HidInfo]| inc::matches(list, 0x093A, 0x622C).map(|m| m.config.path);
    assert_eq!(pick(&[&a, &b]), Some("a".to_string()));
    assert_eq!(pick(&[&b, &a]), Some("a".to_string()));
    // same interface: the larger feature report wins
    b.interface = Some(1);
    b.feature_len = 129;
    assert_eq!(pick(&[&a, &b]), Some("b".to_string()));
    assert_eq!(pick(&[&b, &a]), Some("b".to_string()));
    // and a full tie falls to the path
    b.feature_len = 65;
    assert_eq!(pick(&[&a, &b]), Some("a".to_string()));
    assert_eq!(pick(&[&b, &a]), Some("a".to_string()));
    // a collection with no interface number sorts after the numbered ones
    let mut c = col(0x622C);
    c.path = "0".into();
    c.interface = None;
    assert_eq!(pick(&[&c, &b, &a]), Some("a".to_string()));
}

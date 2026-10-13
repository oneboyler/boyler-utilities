//! Order 105 check, K-snake X11 / Noir Gear M2-NEX: our requests = the bytes OpenMouse sends (encode), and answers shaped the
//! way OpenMouse reads them decode to the same values (decode). The expected bytes are worked out by hand from OpenMouse
//! (@openmouse/protocol 0.40.1: `dist/ksnake/index.js` - the codec - and `dist/drivers/ksnake/hid.js`), never from our own
//! encoder. Never run on a real unit.
//!
//! How OpenMouse counts: `sendReport(0, body)` - a 64-byte body WITHOUT the report id; an answer is `event.data`, also without
//! the id. Windows' buffer is the report id 0 + the body (65 bytes): OpenMouse's byte i is our byte i + 1.

use bu_mouse::family::{self, Driver, Family, Matched, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::ksnake as ks;
use bu_mouse::os::{HidInfo, HidTransfer};
use bu_mouse::Error;
use std::cell::RefCell;
use std::rc::Rc;

/// The output report as Windows sends it: report id 0, OpenMouse's body, zeros to 65 bytes.
fn rep(body: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 65];
    v[1..1 + body.len()].copy_from_slice(body);
    v
}

/// The control collection as Windows lists it: vendor page 0xFF01, usage 0x10, 65-byte reports.
fn col(vid: u16, product: Option<&str>) -> HidInfo {
    let mut h = hid_info(&format!("ks{vid:04x}"), vid, 0x2255, 0xFF01, 0x10, (65, 65, 0));
    h.product = product.map(str::to_string);
    h
}

// ---------------------------------------------------------------- encode: OpenMouse's bytes, by hand

#[test]
fn read_requests_are_openmouses() {
    // ksnakeGetConfigRequest: [0x55, CMD.GET_CONFIG 0x0e, then GET_CONFIG_TAIL a5 0b 2f 01 01 00 00 00]
    assert_eq!(ks::report(&ks::get_config_request(), 65), rep(&[0x55, 0x0E, 0xA5, 0x0B, 0x2F, 0x01, 0x01, 0x00, 0x00, 0x00]));
    // ksnakeGetBatteryRequest: [0x55, CMD.GET_BATTERY 0x30, a5 0b 2e 01 01 00 00 00]
    assert_eq!(ks::report(&ks::get_battery_request(), 65), rep(&[0x55, 0x30, 0xA5, 0x0B, 0x2E, 0x01, 0x01, 0x00, 0x00, 0x00]));
}

/// OpenMouse's KSNAKE_DEFAULT_CONFIG as a decoded config.
fn default_like() -> ks::Config {
    ks::Config {
        light_mode: 2,
        report_rate: 3,
        dpi_index: 2,
        dpi_count: 6,
        stages: [800, 1200, 1600, 3200, 5000, 12000],
        scroll_flag: 0,
        lod_value: 1,
        sensor_flag: 53,
        key_respond: 2,
        sleep_light: 10,
        highspeed_mode: 0,
        wakeup_flag: 1,
        move_light_flag: 1,
        blank: false,
    }
}

#[test]
fn set_config_bytes_are_openmouses() {
    // ksnakeEncodeSetConfig: [0]=0x55 [1]=0x0f [2..9]=SET_CONFIG_HEAD ae 0a 2f 01 01 00 00; [9]=lightMode 2;
    // [10]=reportRate+1 = 3+1 = 4 (1000 Hz); [11]=dpiCount 6; [12]=dpiIndex+1 = 2+1 = 3; stages little-endian from [13]:
    // 800=0x0320 -> 20 03, 1200=0x04b0 -> b0 04, 1600=0x0640 -> 40 06, 3200=0x0c80 -> 80 0c, 5000=0x1388 -> 88 13,
    // 12000=0x2ee0 -> e0 2e; [48]=scroll 0 [49]=lod 1 [50]=sensor 53=0x35 [51]=key respond 2 [52]=sleep 10=0x0a [53]=0;
    // [54]=(wakeup 1 << 4) | move 1 = 0x11; the rest 0
    let mut want = [0u8; 64];
    want[..13].copy_from_slice(&[0x55, 0x0F, 0xAE, 0x0A, 0x2F, 0x01, 0x01, 0x00, 0x00, 0x02, 0x04, 0x06, 0x03]);
    want[13..25].copy_from_slice(&[0x20, 0x03, 0xB0, 0x04, 0x40, 0x06, 0x80, 0x0C, 0x88, 0x13, 0xE0, 0x2E]);
    want[48..55].copy_from_slice(&[0x00, 0x01, 0x35, 0x02, 0x0A, 0x00, 0x11]);
    assert_eq!(ks::encode_set_config(&default_like()), want);
    // setDpi(2000) on stage 2: 2000 = 0x07d0 -> d0 07 at [17], [18]
    let mut c = default_like();
    c.stages[2] = 2000;
    let mut w2 = want;
    w2[17] = 0xD0;
    w2[18] = 0x07;
    assert_eq!(ks::encode_set_config(&c), w2);
    // setPollingRate(500): index 2 -> [10] = 3
    let mut c = default_like();
    c.report_rate = ks::polling_index(500).unwrap();
    let mut w3 = want;
    w3[10] = 0x03;
    assert_eq!(ks::encode_set_config(&c), w3);
    // 125 -> 1, 250 -> 2, 500 -> 3, 1000 -> 4 in the byte; other rates have no index
    for (hz, byte) in [(125, 1u8), (250, 2), (500, 3), (1000, 4)] {
        let c = ks::Config { report_rate: ks::polling_index(hz).unwrap(), ..default_like() };
        assert_eq!(ks::encode_set_config(&c)[10], byte, "{hz}");
    }
    assert_eq!(ks::polling_index(2000), None);
    // the wake-up / move-light nibbles go out swapped relative to how the decode reads them: wakeup 2, move 1 -> 0x21
    let c = ks::Config { wakeup_flag: 2, move_light_flag: 1, ..default_like() };
    assert_eq!(ks::encode_set_config(&c)[54], 0x21);
}

// ---------------------------------------------------------------- decode: answers as OpenMouse reads them

/// A GET_CONFIG answer as the mouse sends it (OpenMouse's reply, 64 bytes): fields at the offsets ksnakeDecodeConfig reads.
fn config_reply(stages: [u16; 6], rate_byte: u8, index_byte: u8) -> Vec<u8> {
    let mut r = vec![0u8; 64];
    r[0] = 0x55;
    r[9] = 3; // light mode
    r[10] = rate_byte;
    r[11] = 6;
    r[12] = index_byte;
    for (i, s) in stages.iter().enumerate() {
        r[13 + 2 * i] = *s as u8;
        r[14 + 2 * i] = (*s >> 8) as u8;
    }
    r[49] = 2; // lod
    r[50] = 53;
    r[55] = 0x21; // high nibble 2 = move light, low nibble 1 = wakeup (as the vendor decode reads it)
    r
}

#[test]
fn config_answers_decode_the_openmouse_way() {
    // reportRate = reply[10] - 1 (4 -> index 3 = 1000 Hz), dpiIndex = reply[12] - 1 (3 -> stage 2), stages LE from [13]
    let c = ks::decode_config(&config_reply([800, 1200, 1600, 3200, 5000, 12000], 4, 3)).unwrap();
    assert_eq!((c.report_rate, c.dpi_index, c.dpi_count, c.light_mode, c.lod_value), (3, 2, 6, 3, 2));
    assert_eq!(c.stages, [800, 1200, 1600, 3200, 5000, 12000]);
    assert_eq!((c.wakeup_flag, c.move_light_flag), (1, 2), "wakeup from the LOW nibble of reply[55]");
    assert_eq!((ks::polling_hz(c.report_rate), c.active_dpi(), c.blank), (Some(1000), Some(1600), false));
    // 0x0640 = 1600: the LE pair 40 06
    assert_eq!(config_reply([0x0640, 0, 0, 0, 0, 0x2EE0], 1, 1)[13..15], [0x40, 0x06]);
    // rate bytes 1..4 = 125 / 250 / 500 / 1000
    for (b, hz) in [(1u8, 125u32), (2, 250), (3, 500), (4, 1000)] {
        let c = ks::decode_config(&config_reply([800; 6], b, 1)).unwrap();
        assert_eq!(ks::polling_hz(c.report_rate), Some(hz));
    }
    // 0 (garbled) -> index -1 -> no rate; index 4 -> none
    assert_eq!(ks::polling_hz(ks::decode_config(&config_reply([800; 6], 0, 1)).unwrap().report_rate), None);
    assert_eq!(ks::polling_hz(4), None);
    // an all-zero or all-0xff first stage = blank / erased: OpenMouse substitutes its defaults (flagged here)
    let mut blank = config_reply([0, 0, 0, 0, 0, 0], 2, 1);
    let c = ks::decode_config(&blank).unwrap();
    assert!(c.blank);
    assert_eq!((c.stages, c.report_rate, c.dpi_index), ([800, 1200, 1600, 3200, 5000, 12000], 3, 2));
    blank[13..16].copy_from_slice(&[255, 255, 255]);
    assert!(ks::decode_config(&blank).unwrap().blank);
    // shorter than 56 bytes is no config
    assert!(ks::decode_config(&[0x55; 55]).is_none());
    // the battery: reply[8] percent, reply[9] charging
    let mut b = vec![0u8; 64];
    b[8] = 87;
    b[9] = 1;
    assert_eq!(ks::decode_battery(&b), Some((87, true)));
    b[9] = 0;
    assert_eq!(ks::decode_battery(&b), Some((87, false)));
    assert_eq!(ks::decode_battery(&b[..9]), None);
}

#[test]
fn dpi_and_polling_limits() {
    assert!(ks::dpi_valid(200) && ks::dpi_valid(12000) && ks::dpi_valid(850) && !ks::dpi_valid(199) && !ks::dpi_valid(12001));
    let c = ks::full_caps();
    assert_eq!(c.polling, vec![125, 250, 500, 1000]);
    let d = c.dpi.unwrap();
    assert_eq!((d.min(), d.max()), (200, 12000));
    assert!(d.allows(200) && d.allows(12000) && d.allows(1600) && !d.allows(250));
    assert!(c.lift_off.is_empty(), "no millimetre figure for the two stops");
    assert!(c.battery);
}

// ---------------------------------------------------------------- the allow-list

#[test]
fn only_the_two_read_requests_pass_the_allow_list() {
    assert!(ks::read_request_allowed(&ks::report(&ks::get_config_request(), 65)));
    assert!(ks::read_request_allowed(&ks::report(&ks::get_battery_request(), 65)));
    // SET_CONFIG, the version query, the light command, key map reads / writes, macro commands
    assert!(!ks::read_request_allowed(&ks::report(&ks::encode_set_config(&default_like()), 65)));
    let mut other = [0u8; 64];
    other[0] = 0x55;
    for cmd in [0x03u8, 0x08, 0x09, 0x21, 0x0C, 0x0F] {
        other[1] = cmd;
        assert!(!ks::read_request_allowed(&ks::report(&other, 65)), "{cmd:02x}");
    }
    // a read with a changed tail, a wrong magic or report id, stuff in the padding
    let mut cfg = ks::get_config_request();
    cfg[4] = 0x2E;
    assert!(!ks::read_request_allowed(&ks::report(&cfg, 65)));
    let mut cfg = ks::get_config_request();
    cfg[0] = 0x56;
    assert!(!ks::read_request_allowed(&ks::report(&cfg, 65)));
    let mut r = ks::report(&ks::get_config_request(), 65);
    r[0] = 6;
    assert!(!ks::read_request_allowed(&r));
    let mut r = ks::report(&ks::get_config_request(), 80);
    r[70] = 1;
    assert!(!ks::read_request_allowed(&r));
    assert!(!ks::read_request_allowed(&[0, 0x55, 0x0E]));
}

// ---------------------------------------------------------------- the device table

type Matcher = fn(&[&HidInfo], u16, u16) -> Option<Matched>;

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
        ("incott", incott::matches),
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
fn the_x11_and_the_m2_nex_match_on_both_vendor_ids() {
    for (vid, wireless) in [(0xA8A4u16, false), (0xA8A5, true)] {
        // the X11 (OpenMouse's list: K-snake X11, pid 0x2255)
        let c = col(vid, Some("K-snake X11"));
        let m = ks::matches(&[&c], vid, 0x2255).unwrap_or_else(|| panic!("{vid:04x}"));
        assert_eq!((m.family, m.model, m.wireless, m.variant), (Family::Ksnake, Some("K-snake X11"), Some(wireless), ks::VARIANT_X11));
        assert_eq!(m.config.path, c.path);
        // no product string at all: still the X11
        assert_eq!(ks::matches(&[&col(vid, None)], vid, 0x2255).unwrap().variant, ks::VARIANT_X11);
        // the Noir Gear M2-NEX: same ids, told apart by the product string (isNoirM2NexDevice)
        for name in ["M2-NEX", "m2 nex", " M2_NEX ", "m2nex"] {
            let n = col(vid, Some(name));
            let m = ks::matches(&[&n], vid, 0x2255).unwrap();
            assert_eq!((m.model, m.variant, m.wireless), (Some("Noir Gear M2-NEX"), ks::VARIANT_M2_NEX, Some(wireless)), "{name}");
        }
        for name in ["M2-NEX Pro", "M1-NEX", "Noir M2-NEX"] {
            assert_eq!(ks::matches(&[&col(vid, Some(name))], vid, 0x2255).unwrap().variant, ks::VARIANT_X11, "{name}");
        }
        for (name, f) in others() {
            assert!(f(&[&c], vid, 0x2255).is_none(), "{name} must not claim {vid:04x}:2255");
        }
        assert_eq!(family::find(std::slice::from_ref(&c), vid, 0x2255).map(|m| m.family), Some(Family::Ksnake));
    }
    // the product string of another interface of the same mouse names the M2-NEX too
    let c = col(0xA8A5, None);
    let mut other = hid_info("mouse", 0xA8A5, 0x2255, 0x01, 0x02, (8, 0, 0));
    other.product = Some("M2-NEX".into());
    assert_eq!(ks::matches(&[&other, &c], 0xA8A5, 0x2255).unwrap().variant, ks::VARIANT_M2_NEX);
}

#[test]
fn nothing_else_matches() {
    let c = col(0xA8A4, Some("K-snake X11"));
    assert!(ks::matches(&[&c], 0xA8A4, 0x2256).is_none(), "another product id (AJAZZ owns the others)");
    assert!(ks::matches(&[&c], 0xA8A6, 0x2255).is_none(), "another vendor id");
    assert!(ks::matches(&[&c], 0x3710, 0x2255).is_none());
    let mut other_page = col(0xA8A4, None);
    other_page.usage_page = 0xFF00;
    assert!(ks::matches(&[&other_page], 0xA8A4, 0x2255).is_none());
    let mut other_usage = col(0xA8A4, None);
    other_usage.usage = 0x01;
    assert!(ks::matches(&[&other_usage], 0xA8A4, 0x2255).is_none());
    let mut short = col(0xA8A4, None);
    short.output_len = 33;
    assert!(ks::matches(&[&short], 0xA8A4, 0x2255).is_none(), "no 64-byte output report");
    let mut no_in = col(0xA8A4, None);
    no_in.input_len = 0;
    assert!(ks::matches(&[&no_in], 0xA8A4, 0x2255).is_none(), "the answer needs an input report");
}

// ---------------------------------------------------------------- the driver against a scripted mouse

struct Model {
    /// the config answer, OpenMouse's reply bytes
    cfg: Vec<u8>,
    battery: (u8, u8),
    /// answer nothing (asleep)
    silent: bool,
    /// the next N answers are a stray report (all zero) instead of the answer
    stray: u32,
    /// ignore SET_CONFIG
    refuse_writes: bool,
    /// every config answer changes a stage value behind it (a second read never equals the first)
    drift: bool,
    /// after SET_CONFIG the stage table reads blank (all zero)
    blank_after_write: bool,
    log: Vec<Vec<u8>>,
}

impl Model {
    fn new() -> Self {
        let mut cfg = config_reply([800, 1200, 1600, 3200, 5000, 12000], 4, 3);
        cfg[48] = 1; // scroll flag
        cfg[51] = 2;
        cfg[52] = 10;
        Model { cfg, battery: (64, 0), silent: false, stray: 0, refuse_writes: false, drift: false, blank_after_write: false, log: Vec::new() }
    }
}

fn port(info: HidInfo, m: Rc<RefCell<Model>>) -> ScriptPort {
    ScriptPort::new(info, move |out, how| {
        let mut m = m.borrow_mut();
        m.log.push(out.to_vec());
        assert!(matches!(how, HidTransfer::OutputThenInput { reply_len: 65, max_reads: 1, timeout_ms: 800, echo: None }), "{how:?}");
        if m.silent {
            return Err(Error::BadAnswer("no answer in time".into()));
        }
        let body = &out[1..];
        let reply = match body[1] {
            0x0E => {
                let c = m.cfg.clone();
                if m.drift {
                    m.cfg[24] ^= 1;
                }
                c
            }
            0x30 => {
                let mut b = vec![0u8; 64];
                b[8] = m.battery.0;
                b[9] = m.battery.1;
                b
            }
            0x0F => {
                if !m.refuse_writes {
                    // the mouse stores what it is sent (a model written independently of the encoder: by hand offsets)
                    for i in (9..25).chain(48..54) {
                        m.cfg[i] = body[i];
                    }
                    if m.blank_after_write {
                        m.cfg[13..16].copy_from_slice(&[0, 0, 0]);
                    }
                }
                let mut a = vec![0u8; 64];
                a[0] = 0x55;
                a[1] = 0x0F;
                a
            }
            other => panic!("unexpected command {other:02x}"),
        };
        if m.stray > 0 {
            m.stray -= 1;
            return Ok(vec![0u8; 65]);
        }
        let mut v = vec![0u8];
        v.extend_from_slice(&reply);
        Ok(v)
    })
}

fn setup(vid: u16, product: Option<&str>) -> (Matched, Rc<RefCell<Model>>, ScriptPort) {
    let info = col(vid, product);
    let m = ks::matches(&[&info], vid, 0x2255).unwrap();
    let model = Rc::new(RefCell::new(Model::new()));
    let p = port(info, model.clone());
    (m, model, p)
}

#[test]
fn reading_asks_only_the_config_and_the_battery() {
    let (m, model, mut p) = setup(0xA8A5, Some("M2-NEX"));
    let r = ks::Ksnake.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(r.online);
    assert_eq!((r.dpi, r.polling_hz, r.battery_percent, r.charging), (Some(1600), Some(1000), Some(64), Some(false)));
    assert_eq!((r.lift_off, r.wireless), (None, Some(true)));
    assert_eq!(r.model.as_deref(), Some("Noir Gear M2-NEX"));
    // requests: the config, then the battery - exactly OpenMouse's two requests, both on the allow-list
    let log = model.borrow().log.clone();
    assert_eq!(log, vec![ks::report(&ks::get_config_request(), 65), ks::report(&ks::get_battery_request(), 65)]);
    assert!(log.iter().all(|o| ks::read_request_allowed(o)));
    // writes are held back, so the caps offer only the battery
    assert!(r.caps.dpi.is_none() && r.caps.polling.is_empty() && r.caps.lift_off.is_empty() && r.caps.battery);
}

#[test]
fn charging_and_the_cable() {
    let (m, model, mut p) = setup(0xA8A4, Some("K-snake X11"));
    model.borrow_mut().battery = (100, 1);
    let r = ks::Ksnake.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.battery_percent, r.charging, r.wireless), (Some(100), Some(true), Some(false)));
    assert_eq!(r.model.as_deref(), Some("K-snake X11"));
}

#[test]
fn a_stray_report_is_asked_again() {
    // OpenMouse readValidated: the first answer fails validation (a stray all-zero report), one more ask
    let (m, model, mut p) = setup(0xA8A5, None);
    model.borrow_mut().stray = 1;
    let r = ks::Ksnake.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.dpi, r.polling_hz, r.battery_percent), (Some(1600), Some(1000), Some(64)));
    assert_eq!(model.borrow().log.len(), 3);
}

#[test]
fn a_sleeping_mouse_is_offline() {
    let (m, model, mut p) = setup(0xA8A5, None);
    model.borrow_mut().silent = true;
    let r = ks::Ksnake.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(!r.online);
    assert_eq!((r.dpi, r.polling_hz, r.battery_percent, r.charging), (None, None, None, None));
    // two asks each, no loop
    assert_eq!(model.borrow().log.len(), 4);
}

#[test]
fn a_blank_config_shows_no_values() {
    let (m, model, mut p) = setup(0xA8A5, None);
    model.borrow_mut().cfg[13..16].copy_from_slice(&[0, 0, 0]);
    let r = ks::Ksnake.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(r.online);
    assert_eq!((r.dpi, r.polling_hz), (None, None), "OpenMouse's defaults are not the mouse's values");
    assert_eq!(r.battery_percent, Some(64));
}

#[test]
fn a_gone_mouse_is_an_error() {
    let info = col(0xA8A5, None);
    let m = ks::matches(&[&info], 0xA8A5, 0x2255).unwrap();
    let mut p = ScriptPort::new(info, |_, _| Err(Error::MouseGone("ks".into())));
    assert!(matches!(ks::Ksnake.read(&m, &mut p, ReadOptions::default()), Err(Error::MouseGone(_))));
}

#[test]
fn writes_are_held_back_through_the_driver() {
    if !ks::HOLD_WRITES {
        return; // (the lead switched writes on: this test is about the held-back state)
    }
    let (m, model, mut p) = setup(0xA8A5, None);
    for s in [Setting::Dpi(1600), Setting::Polling(500), Setting::LiftOff(10)] {
        assert!(matches!(ks::Ksnake.write(&m, &mut p, s), Err(Error::UnsupportedMouse(_))));
    }
    assert!(model.borrow().log.is_empty(), "not a byte leaves");
}

// the ported write flow (switched off by HOLD_WRITES) is proven here through `apply`

fn sets(model: &Rc<RefCell<Model>>) -> Vec<Vec<u8>> {
    model.borrow().log.iter().filter(|o| o[2] == 0x0F).cloned().collect()
}

#[test]
fn the_ported_dpi_flow_sends_openmouses_set_config() {
    let (m, model, mut p) = setup(0xA8A5, None);
    let c = ks::apply(&m, &mut p, Setting::Dpi(2000)).unwrap();
    assert_eq!((c.now, c.before), (2000, Some(1600)));
    // read config twice (the same both times), SET_CONFIG, read config back
    let log = model.borrow().log.clone();
    assert_eq!(log.len(), 4);
    assert_eq!(log[0], ks::report(&ks::get_config_request(), 65));
    assert_eq!(log[1], ks::report(&ks::get_config_request(), 65));
    assert_eq!(log[3], ks::report(&ks::get_config_request(), 65));
    // the SET_CONFIG: the mouse's own block (light 3, rate byte 4, count 6, index byte 3, scroll 1 ...), stage 2 = 2000 = d0 07,
    // the wake-up/move flags (reply[55] = 0x21: wakeup 1, move 2) written to byte 54 swapped: (1 << 4) | 2 = 0x12
    let mut want = vec![0u8; 65];
    want[1..14].copy_from_slice(&[0x55, 0x0F, 0xAE, 0x0A, 0x2F, 0x01, 0x01, 0x00, 0x00, 0x03, 0x04, 0x06, 0x03]);
    want[14..26].copy_from_slice(&[0x20, 0x03, 0xB0, 0x04, 0xD0, 0x07, 0x80, 0x0C, 0x88, 0x13, 0xE0, 0x2E]);
    want[49..56].copy_from_slice(&[0x01, 0x02, 0x35, 0x02, 0x0A, 0x00, 0x12]);
    assert_eq!(sets(&model), vec![want]);
    assert_eq!(model.borrow().cfg[17..19], [0xD0, 0x07]);
}

#[test]
fn the_ported_polling_flow_sends_openmouses_set_config() {
    let (m, model, mut p) = setup(0xA8A4, None);
    let c = ks::apply(&m, &mut p, Setting::Polling(500)).unwrap();
    assert_eq!((c.now, c.before), (500, Some(1000)));
    let s = sets(&model);
    assert_eq!(s.len(), 1);
    assert_eq!(s[0][1 + 10], 0x03, "report rate byte: index 2 + 1");
    assert_eq!(s[0][1 + 12], 0x03, "the active stage is kept");
}

#[test]
fn the_ported_flow_refuses_bad_values_a_blank_config_and_a_mouse_that_keeps_its_value() {
    let (m, model, mut p) = setup(0xA8A5, None);
    for s in [Setting::Dpi(199), Setting::Dpi(12001), Setting::Polling(2000), Setting::LiftOff(10)] {
        assert!(ks::apply(&m, &mut p, s).is_err());
    }
    assert!(model.borrow().log.is_empty(), "refused before a byte leaves");
    model.borrow_mut().refuse_writes = true;
    assert!(matches!(ks::apply(&m, &mut p, Setting::Dpi(2000)), Err(Error::BadAnswer(_))));
    model.borrow_mut().refuse_writes = false;
    model.borrow_mut().cfg[13..16].copy_from_slice(&[0, 0, 0]);
    model.borrow_mut().log.clear();
    assert!(ks::apply(&m, &mut p, Setting::Dpi(2000)).is_err());
    assert!(sets(&model).is_empty(), "a blank config is never overwritten with defaults");
    // a silent mouse: the read fails after 3 tries, nothing is written
    model.borrow_mut().silent = true;
    model.borrow_mut().log.clear();
    assert!(ks::apply(&m, &mut p, Setting::Dpi(2000)).is_err());
    assert_eq!(model.borrow().log.len(), 3);
    assert!(sets(&model).is_empty());
}

#[test]
fn the_ported_flow_writes_only_from_a_real_config_seen_twice() {
    // a stray report that happens to carry stage bytes and a stage index but no valid polling index (reply[10] = 0 reads as -1):
    // not a config, nothing is written
    let (m, model, mut p) = setup(0xA8A5, None);
    model.borrow_mut().cfg[10] = 0;
    assert!(matches!(ks::apply(&m, &mut p, Setting::Dpi(2000)), Err(Error::BadAnswer(_))));
    assert!(sets(&model).is_empty());
    // a stage index outside the table (reply[12] = 7 reads as 6): not plausible either
    let (m, model, mut p) = setup(0xA8A5, None);
    model.borrow_mut().cfg[12] = 7;
    assert!(ks::apply(&m, &mut p, Setting::Polling(500)).is_err());
    assert!(sets(&model).is_empty());
    // an erased table (0xFF) is as blank as a zero one
    let (m, model, mut p) = setup(0xA8A5, None);
    model.borrow_mut().cfg[13..16].copy_from_slice(&[255, 255, 255]);
    assert!(ks::apply(&m, &mut p, Setting::Dpi(2000)).is_err());
    assert!(sets(&model).is_empty());
    // two reads that disagree: not written
    let (m, model, mut p) = setup(0xA8A5, None);
    model.borrow_mut().drift = true;
    assert!(matches!(ks::apply(&m, &mut p, Setting::Dpi(2000)), Err(Error::BadAnswer(_))));
    assert!(sets(&model).is_empty());
    assert_eq!(model.borrow().log.len(), 2, "two reads, then it stops");
}

#[test]
fn the_ported_flow_does_not_take_a_blank_read_back_for_a_confirmation() {
    // polling 1000 is already set; a read-back that comes blank decodes as OpenMouse's defaults (also 1000 Hz, 1600 DPI) and
    // would pass for "confirmed" - it must not
    let (m, model, mut p) = setup(0xA8A5, None);
    model.borrow_mut().blank_after_write = true;
    let e = ks::apply(&m, &mut p, Setting::Polling(1000)).unwrap_err();
    assert!(matches!(e, Error::BadAnswer(_)), "{e}");
    assert_eq!(sets(&model).len(), 1);
    // the same with a DPI of 1600 (the active stage already holds it)
    let (m, model, mut p) = setup(0xA8A5, None);
    model.borrow_mut().blank_after_write = true;
    assert!(ks::apply(&m, &mut p, Setting::Dpi(1600)).is_err());
}

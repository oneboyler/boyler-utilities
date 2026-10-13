//! Order 105 check, Mchose (A7 V2 Ultra+ and A5 Pro Max): our requests = the bytes OpenMouse sends for the same setting
//! (encode), and answers shaped the way OpenMouse reads them decode to the same values (decode). The expected bytes are
//! worked out by hand from OpenMouse (@openmouse/protocol 0.40.1: `dist/mchose/index.js`, `dist/mchose/a5-gen1.js`,
//! `dist/drivers/mchose/hid.js`, `dist/drivers/mchose/a5-gen1-hid.js`), never from our own encoder.
//!
//! A7 V2: a feature report is `[report id] ++ body(64)`; the body is the command and its arguments BIT-INVERTED, tokens beyond
//! the given ones inverted zeros (0xFF) up to the channel's token count (short 0x11: 20, long 0x12: 64), then zeros. Windows
//! pads the buffer to the collection's feature-report length (65 here). An answer is `[report id, ~cmd, ~payload...]`.
//! The configuration write 0x57 echoes the payload that was read: payload byte j goes to token j + 1, so our byte j + 2 is
//! `!payload[j]`; our byte 1 is `!0x57 = 0xA8`.
//! A5: the 64-byte body is `[0, 0, route 2, length, page, command, data...]`, not inverted, behind the feature report id.

use bu_mouse::family::{Change, Driver, Extra, Family, Kind, Matched, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::mchose::{self as mc, a5, v2};
use bu_mouse::os::{HidInfo, HidTransfer};
use std::cell::RefCell;
use std::rc::Rc;

// ---------------------------------------------------------------- shared helpers

/// `n` copies of `b`.
fn rep(b: u8, n: usize) -> Vec<u8> {
    vec![b; n]
}

fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

fn v2_col(pid: u16, feature_len: u16) -> HidInfo {
    hid_info(&format!("v2cfg{pid:04x}"), 0x3837, pid, 0xFF01, 0x01, (65, 65, feature_len))
}

/// The inverted-token request: `[id, ~cmd, 0xFF x (tokens - 1), zeros]` padded to 65 bytes.
fn v2_req(id: u8, cmd: u8, tokens: usize) -> Vec<u8> {
    cat(&[&[id, !cmd], &rep(0xFF, tokens - 1), &rep(0, 65 - 1 - tokens)])
}

fn req_identity() -> Vec<u8> {
    // short channel 0x11, 20 tokens: id, ~0x03 = 0xFC, 19 x 0xFF, 44 zeros
    cat(&[&[0x11, 0xFC], &rep(0xFF, 19), &rep(0, 44)])
}

fn req_battery() -> Vec<u8> {
    // ~0x06 = 0xF9
    cat(&[&[0x11, 0xF9], &rep(0xFF, 19), &rep(0, 44)])
}

fn req_config() -> Vec<u8> {
    // long channel 0x12, 64 tokens: id, ~0x67 = 0x98, 63 x 0xFF
    cat(&[&[0x12, 0x98], &rep(0xFF, 63)])
}

/// The configuration payload of a scripted A7 V2 (63 bytes, as OpenMouse's reply decodes: raw.length - 2).
/// Stages 400 / 800 / 1600 / 3200 / 6400 / 12800 as little-endian words; the wired link at 1000 Hz (rate index 2) on stage 1,
/// the 2.4 GHz link at 500 Hz (rate index 1) on stage 0; stage count 6; some "stale scratch" bytes that must survive a write.
fn config_payload() -> Vec<u8> {
    let mut p = vec![0u8; 63];
    p[0] = 0; // profile 1
    p[1] = 0x21; // wired: rate index 2 << 4 | stage 1
    p[2] = 0x10; // wireless: rate index 1 << 4 | stage 0
    for (i, dpi) in [400u16, 800, 1600, 3200, 6400, 12800].iter().enumerate() {
        p[4 + i * 2] = (dpi & 0xFF) as u8;
        p[5 + i * 2] = (dpi >> 8) as u8;
    }
    p[16] = 6;
    p[17] = 0x91;
    p[18] = 4;
    p[19] = 5;
    p[30] = 0x5A;
    p[49] = 0xE2;
    p[62] = 0x77;
    p
}

/// What the mouse puts in the long answer for `payload` (bit-inverted behind the command echo).
fn raw_reply(id: u8, cmd: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![id, !cmd];
    v.extend(payload.iter().map(|b| !*b));
    v.resize(65, 0xFF);
    v
}

/// The write frame OpenMouse sends for `payload` (the 0x57 write): `[0x12, 0xA8] ++ !payload[j]`.
fn write_frame(payload: &[u8]) -> Vec<u8> {
    cat(&[&[0x12, 0xA8], &payload.iter().map(|b| !*b).collect::<Vec<_>>()])
}

// ---------------------------------------------------------------- A7 V2: encode

#[test]
fn the_a7_read_requests_are_openmouses_inverted_frames() {
    // mchoseEncodeCommand([0x03], 20): body[0] = ~0x03 = 0xFC, body[1..20] = 0xFF, body[20..64] = 0
    assert_eq!(v2::feature_report(v2::SHORT_ID, &v2::encode_command(&[v2::CMD_IDENTITY], v2::SHORT_TOKENS), 65).unwrap(), req_identity());
    assert_eq!(v2::feature_report(v2::SHORT_ID, &v2::encode_command(&[v2::CMD_BATTERY], v2::SHORT_TOKENS), 65).unwrap(), req_battery());
    assert_eq!(v2::feature_report(v2::LONG_ID, &v2::encode_command(&[v2::CMD_CONFIG], v2::LONG_TOKENS), 65).unwrap(), req_config());
    // and the helper above agrees with the hand-written ones
    assert_eq!(v2_req(0x11, 0x03, 20), req_identity());
    assert_eq!(v2_req(0x12, 0x67, 64), req_config());
    // Windows pads to a longer collection with zeros
    let long = v2::feature_report(v2::SHORT_ID, &v2::encode_command(&[0x06], 20), 80).unwrap();
    assert_eq!((long.len(), &long[..65], &long[65..]), (80, &req_battery()[..], &[0u8; 15][..]));
    // a collection shorter than id + 64 can't carry OpenMouse's body
    assert!(v2::feature_report(0x12, &[0u8; 64], 64).is_none());
}

#[test]
fn the_configuration_write_echoes_the_read_payload_byte_for_byte() {
    let p = config_payload();
    // no change: tokens = [0x57] ++ payload, the two link bytes re-packed to themselves
    let none = v2::encode_config_write(&p, &v2::Changes::default()).unwrap();
    assert_eq!(none, cat(&[&[0x57], &p]));
    // wired rate index 4 (4000 Hz): out[2] = 4 << 4 | stage 1 = 0x41 (token index = payload index + 1)
    let t = v2::encode_config_write(&p, &v2::Changes { wired_rate_index: Some(4), ..Default::default() }).unwrap();
    let mut want = cat(&[&[0x57], &p]);
    want[2] = 0x41;
    assert_eq!(t, want);
    // wireless rate index 3 (2000 Hz): out[3] = 3 << 4 | stage 0 = 0x30
    let t = v2::encode_config_write(&p, &v2::Changes { wireless_rate_index: Some(3), ..Default::default() }).unwrap();
    let mut want = cat(&[&[0x57], &p]);
    want[3] = 0x30;
    assert_eq!(t, want);
    // stage 1 = 1000 = 0x03E8: out[1 + 4 + 2] = 0xE8, out[1 + 4 + 3] = 0x03
    let t = v2::encode_config_write(&p, &v2::Changes { dpi_stages: Some([400, 1000, 1600, 3200, 6400, 12800]), ..Default::default() }).unwrap();
    let mut want = cat(&[&[0x57], &p]);
    want[7] = 0xE8;
    want[8] = 0x03;
    assert_eq!(t, want);
    // as sent: ~0x57 = 0xA8, then the inverted payload; token 64 and beyond (none here: 1 + 63 = 64) are all there is
    let body = v2::encode_command(&t, v2::LONG_TOKENS);
    assert_eq!(body[0], 0xA8);
    assert_eq!(body[1 + 30], !0x5A, "stale scratch byte 30 survives");
    assert_eq!(body[1 + 62], !0x77, "and the last byte too");
    assert!(v2::encode_config_write(&p[..10], &v2::Changes::default()).is_none(), "a payload too short to decode is not written");
}

#[test]
fn dpi_rounding_and_the_model_table() {
    // Math.round(dpi / 50) * 50: halves go up
    for (d, r) in [(1000, 1000), (1010, 1000), (1025, 1050), (1024, 1000), (42000, 42000), (50, 50)] {
        assert_eq!(v2::round_dpi(d), r, "{d}");
    }
    // MCHOSE_PRODUCTS: dpiMax 42000 for the Ultra / Ultra+, 26000 for the Pro / Pro+; every model reaches 8000 Hz
    let ultra_plus = v2::product_by_id(0x4021).unwrap();
    assert_eq!((ultra_plus.name, ultra_plus.dpi_max, ultra_plus.max_polling), ("A7 V2 Ultra+", 42000, 8000));
    assert_eq!(v2::product_by_id(0x4018).unwrap().dpi_max, 26000);
    assert!(v2::product_by_id(0x100b).is_none(), "the receiver id is a link, not a model");
    // MCHOSE_POLLING_RATES[8000]
    assert_eq!(v2::polling_rates(8000), vec![125, 500, 1000, 2000, 4000, 8000]);
    assert_eq!(v2::polling_rates(1000), vec![125, 500, 1000]);
}

// ---------------------------------------------------------------- A7 V2: decode

#[test]
fn the_a7_answers_decode_like_openmouse() {
    // mchoseDecodeReply: raw = [report id, ~cmd, ~payload...]
    let (cmd, payload) = v2::decode_reply(&[0x11, 0xF9, 0xFE, 0x00, 0xFF]).unwrap();
    assert_eq!((cmd, payload), (0x06, vec![0x01, 0xFF, 0x00]));
    assert!(v2::decode_reply(&[0x11, 0xF9]).is_none());
    // mchoseDecodeIdentity: [bonded, vid lo, vid hi, pid lo, pid hi, connected, gameMode]
    let i = v2::decode_identity(&[1, 0x37, 0x38, 0x0B, 0x10, 1, 2]).unwrap();
    assert_eq!((i.bonded, i.vendor_id, i.product_id, i.connected, i.game_mode), (true, 0x3837, 0x100B, true, 2));
    assert!(v2::decode_identity(&[1, 2, 3]).is_none());
    // mchoseDecodeBattery: [vid(2), pid(2), fw(4), flags, percent, charging]; connected = (flags >> 4) & 1
    let b = v2::decode_battery(&[0x37, 0x38, 0x21, 0x40, 1, 0, 2, 0, 0x10, 87, 1]).unwrap();
    assert_eq!((b.product_id, b.connected, b.percent, b.charging), (0x4021, true, 87, true));
    let b = v2::decode_battery(&[0x37, 0x38, 0x21, 0x40, 1, 0, 2, 0, 0x00, 5, 0]).unwrap();
    assert_eq!((b.connected, b.percent, b.charging), (false, 5, false));
    assert!(v2::decode_battery(&[0; 10]).is_none());
    // mchoseDecodeConfig: wired byte first, rate = high nibble, stage = low nibble; stages from offset 4
    let c = v2::decode_config(&config_payload()).unwrap();
    assert_eq!((c.profile_index, c.wired_rate_index, c.wired_dpi_index, c.wireless_rate_index, c.wireless_dpi_index, c.stage_count), (0, 2, 1, 1, 0, 6));
    assert_eq!(c.dpi_stages, [400, 800, 1600, 3200, 6400, 12800]);
    assert!(v2::decode_config(&[0; 15]).is_none());
    // looksLikeConfig: 20+ bytes and a plausible first stage (50..42000)
    assert!(v2::looks_like_config(&config_payload()));
    assert!(!v2::looks_like_config(&[0; 63]), "a zero first stage");
    assert!(!v2::looks_like_config(&config_payload()[..19]));
    let mut huge = config_payload();
    huge[4] = 0x11;
    huge[5] = 0xA4; // 0xA411 = 42001
    assert!(!v2::looks_like_config(&huge));
}

// ---------------------------------------------------------------- A7 V2: allow-list

#[test]
fn only_a7_read_requests_pass_the_allow_list() {
    for r in [req_identity(), req_battery(), req_config()] {
        assert!(mc::read_request_allowed(&r));
    }
    // a longer collection pads with zeros
    let mut padded = req_config();
    padded.extend([0u8; 9]);
    assert!(mc::read_request_allowed(&padded));
    // the configuration write 0x57 (any payload), the performance command 0x42, sleep 0x0a, profile 0x58, button 0x52
    assert!(!mc::read_request_allowed(&write_frame(&config_payload())));
    for cmd in [0x42u8, 0x0a, 0x58, 0x52, 0x57] {
        assert!(!mc::read_request_allowed(&v2_req(0x11, cmd, 20)), "short {cmd:02x}");
        assert!(!mc::read_request_allowed(&v2_req(0x12, cmd, 64)), "long {cmd:02x}");
    }
    // OpenMouse also reads version 0x04, profile names 0x68, button names 0x63: this driver sends none of them
    for cmd in [0x04u8, 0x68, 0x63] {
        assert!(!mc::read_request_allowed(&v2_req(0x11, cmd, 20)) && !mc::read_request_allowed(&v2_req(0x12, cmd, 64)), "{cmd:02x}");
    }
    // the right command on the wrong channel, an extra argument, a short buffer
    assert!(!mc::read_request_allowed(&v2_req(0x12, 0x03, 20)));
    assert!(!mc::read_request_allowed(&v2_req(0x11, 0x67, 64)));
    let mut arg = req_identity();
    arg[30] = 1;
    assert!(!mc::read_request_allowed(&arg));
    assert!(!mc::read_request_allowed(&req_config()[..64]));
    assert!(!mc::read_request_allowed(&[]));
}

// ---------------------------------------------------------------- device table

#[test]
fn the_table_is_openmouses_supported_list() {
    // A7 V2 Ultra+: 0x4021 (cable) and 0x100b (the A7 V2 receiver)
    for (pid, wireless) in [(0x4021u16, false), (0x100B, true)] {
        let col = v2_col(pid, 65);
        let m = mc::matches(&[&col], 0x3837, pid).unwrap();
        assert_eq!((m.family, m.model, m.wireless, m.variant), (Family::Mchose, Some("MCHOSE A7 V2 Ultra+"), Some(wireless), mc::CLIENT_V2), "{pid:04x}");
        assert_eq!(m.config.path, col.path);
        // the dongle / cable also lists other collections: the vendor 0xFF01 one is picked
        let mouse = hid_info("mouse", 0x3837, pid, 0x01, 0x02, (8, 0, 0));
        let decoy = hid_info("decoy", 0x3837, pid, 0xFF00, 0x01, (65, 65, 65));
        assert_eq!(mc::matches(&[&mouse, &decoy, &col], 0x3837, pid).unwrap().config.path, col.path);
    }
    // not on the list: other A7 V2 models, the Bluetooth link, the A7 V3 ids, the MagDock (not a mouse), another vendor
    for pid in [0x4019u16, 0x4018, 0x4023, 0x100A, 0x1020, 0x4033, 0x1014, 0x1018, 0x4032, 0x4031, 0x4030, 0x1012] {
        let col = v2_col(pid, 65);
        assert!(mc::matches(&[&col], 0x3837, pid).is_none(), "{pid:04x}");
    }
    let col = v2_col(0x4021, 65);
    assert!(mc::matches(&[&col], 0x3838, 0x4021).is_none());
    // the config collection must be the 0xFF01 / usage 1 one with a feature report that holds OpenMouse's 64-byte body
    assert!(mc::matches(&[&v2_col(0x4021, 33)], 0x3837, 0x4021).is_none());
    assert!(mc::matches(&[&hid_info("x", 0x3837, 0x4021, 0xFF00, 0x01, (65, 65, 65))], 0x3837, 0x4021).is_none());
    assert!(mc::matches(&[&hid_info("x", 0x3837, 0x4021, 0xFF01, 0x02, (65, 65, 65))], 0x3837, 0x4021).is_none());
    assert!(mc::matches(&[&hid_info("x", 0x3837, 0x4021, 0x01, 0x02, (8, 0, 0))], 0x3837, 0x4021).is_none(), "no config collection");
    // the A5 Pro Max (0x2023: f019 / f013 / f015) is held back: its feature report id is not on HidInfo
    let a5col = hid_info("a5", 0x2023, 0xF019, 0xFFFF, 0x01, (65, 65, 65));
    for pid in [0xF019u16, 0xF013, 0xF015] {
        assert!(mc::matches(&[&a5col], 0x2023, pid).is_none(), "{pid:04x}");
        let m = mc::a5_matches(&[&a5col], 0x2023, pid, 0x04).unwrap();
        assert_eq!((m.family, m.model, m.variant), (Family::Mchose, Some("MCHOSE A5 Pro Max"), mc::CLIENT_A5 | 0x0400));
        assert_eq!(m.wireless, Some(pid != 0xF019));
    }
    assert!(mc::a5_matches(&[&a5col], 0x2023, 0xF016, 0x04).is_none());
    assert!(mc::a5_matches(&[&a5col], 0x3837, 0xF019, 0x04).is_none());
    assert!(mc::a5_matches(&[&hid_info("n", 0x2023, 0xF019, 0xFFFF, 0x01, (65, 65, 0))], 0x2023, 0xF019, 0x04).is_none(), "no feature report");
    // no overlap with the other families (find() walks them in OpenMouse's registry order)
    let cols = [v2_col(0x4021, 65)];
    assert_eq!(bu_mouse::family::find(&cols, 0x3837, 0x4021).map(|m| m.family), Some(Family::Mchose));
    for (vid, pid) in [(0x1532u16, 0x4021u16), (0x046d, 0x100B), (0x1038, 0x4021)] {
        assert!(mc::matches(&[&v2_col(pid, 65)], vid, pid).is_none());
    }
    // the MagDock charging base has only a lighting read / write: it is NOT in the table at all
    assert!(mc::PRODUCTS.iter().all(|p| p.pid != 0x1012));
    assert_eq!(mc::PRODUCTS.len(), 5);
}

#[test]
fn caps_are_what_openmouse_offers() {
    let up = v2::product_by_id(0x4021);
    let c = mc::v2_caps(up);
    let dpi = c.dpi.as_ref().unwrap();
    assert_eq!(dpi.bands, vec![(50, 42000, 50)], "the stage editor: 50..dpiMax by 50");
    assert_eq!(c.polling, vec![125, 500, 1000, 2000, 4000, 8000]);
    assert!(c.lift_off.is_empty() && c.battery);
    // a model OpenMouse can't name: its 26000 ceiling and no polling control
    let u = mc::v2_caps(None);
    assert_eq!((u.dpi.unwrap().max(), u.polling.len()), (26000, 0));
    // A5 Pro Max: 50..26000 by 50, wired 125/500/1000, receivers 125/250/500/1000
    assert_eq!(mc::a5_caps(true).polling, vec![125, 500, 1000]);
    assert_eq!(mc::a5_caps(false).polling, vec![125, 250, 500, 1000]);
    assert_eq!(mc::a5_caps(true).dpi.unwrap().bands, vec![(50, 26000, 50)]);
}

// ---------------------------------------------------------------- A7 V2: the driver against a scripted mouse

/// A scripted A7 V2: answers the three reads from its state (the way the firmware does: command echo inverted, payload
/// inverted), takes the 0x57 write, and can be told to go quiet, to ignore writes, or to answer a stale reply first.
struct V2Mouse {
    pid: u16,
    config: Vec<u8>,
    connected: bool,
    battery_pid: u16,
    percent: u8,
    charging: bool,
    quiet: bool,
    ignore_writes: bool,
    stale_config_first: bool,
    sent: Vec<Vec<u8>>,
}

impl V2Mouse {
    fn new(pid: u16) -> Self {
        V2Mouse { pid, config: config_payload(), connected: true, battery_pid: 0x4021, percent: 87, charging: false, quiet: false, ignore_writes: false, stale_config_first: false, sent: Vec::new() }
    }

    fn payload_for(&mut self, cmd: u8) -> Option<Vec<u8>> {
        let mut p = vec![0u8; 63];
        match cmd {
            0x03 => p[..7].copy_from_slice(&[1, 0x37, 0x38, (self.pid & 0xFF) as u8, (self.pid >> 8) as u8, self.connected as u8, 0]),
            0x06 => p[..11].copy_from_slice(&[0x37, 0x38, (self.battery_pid & 0xFF) as u8, (self.battery_pid >> 8) as u8, 1, 0, 2, 0, 0x10, self.percent, self.charging as u8]),
            0x67 => p = self.config.clone(),
            _ => return None,
        }
        Some(p)
    }
}

fn v2_setup(mouse: Rc<RefCell<V2Mouse>>) -> (Matched, ScriptPort) {
    let pid = mouse.borrow().pid;
    let col = v2_col(pid, 65);
    let m = mc::matches(&[&col], 0x3837, pid).unwrap();
    let p = ScriptPort::new(col, move |out, how| {
        let mut mo = mouse.borrow_mut();
        mo.sent.push(out.to_vec());
        match how {
            HidTransfer::Feature { reply_id, reply_len, wait_ms } => {
                assert_eq!((*reply_len, *wait_ms), (65, 90), "OpenMouse waits POLL_DELAY_MS = 90 and reads the whole report");
                assert_eq!(*reply_id, out[0]);
                if mo.quiet {
                    return Err(bu_mouse::Error::BadAnswer("no answer".into()));
                }
                let mut cmd = !out[1];
                if cmd == 0x67 && mo.stale_config_first {
                    mo.stale_config_first = false;
                    cmd = 0x06; // a stale battery answer left in the shared reply buffer
                }
                let payload = mo.payload_for(cmd).unwrap();
                Ok(raw_reply(out[0], cmd, &payload))
            }
            // Order 110: the performance command on the short channel: tokens [0x42, lod, ripple, line, motionSync, ...] inverted;
            // a switch 1 = on, 2 = off, 0 = unchanged (bits 0x04 / 0x08 / 0x10 of config byte 17, lift-off in bits 0-1)
            HidTransfer::FeatureOut if out[..2] == [0x11, 0xBD] => {
                if !mo.ignore_writes {
                    let t: Vec<u8> = out[1..21].iter().map(|b| !*b).collect();
                    let mut s = (mo.config[17] & !0x03) | (t[1] & 0x03);
                    for (tok, bit) in [(t[2], 0x04u8), (t[3], 0x08), (t[4], 0x10)] {
                        match tok {
                            1 => s |= bit,
                            2 => s &= !bit,
                            _ => {}
                        }
                    }
                    mo.config[17] = s;
                }
                Ok(Vec::new())
            }
            HidTransfer::FeatureOut => {
                assert_eq!(out[..2], [0x12, 0xA8], "only the configuration write is ever sent");
                if !mo.ignore_writes {
                    mo.config = out[2..65].iter().map(|b| !*b).collect();
                }
                Ok(Vec::new())
            }
            other => panic!("the A7 V2 driver uses feature reports only, not {other:?}"),
        }
    });
    (m, p)
}

fn sent_of(p: &ScriptPort) -> Vec<Vec<u8>> {
    p.sent.iter().map(|s| s.0.clone()).collect()
}

#[test]
fn reading_asks_only_what_openmouse_asks() {
    // the cable: identity, battery, configuration - each polled until the same answer comes twice
    let mouse = Rc::new(RefCell::new(V2Mouse::new(0x4021)));
    let (m, mut p) = v2_setup(mouse);
    let r = mc::Mchose.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(p.sent.iter().all(|s| mc::read_request_allowed(&s.0)), "read requests only");
    assert_eq!(sent_of(&p), vec![req_identity(), req_identity(), req_battery(), req_battery(), req_config(), req_config()]);
    assert_eq!((r.online, r.battery_percent, r.charging, r.dpi, r.polling_hz, r.lift_off, r.wireless), (true, Some(87), Some(false), Some(800), Some(1000), None, Some(false)));
    assert_eq!(r.model.as_deref(), Some("MCHOSE A7 V2 Ultra+"));
    assert_eq!(r.caps.polling, vec![125, 500, 1000, 2000, 4000, 8000]);
    assert_eq!(r.caps.dpi.as_ref().unwrap().max(), 42000);
    // the receiver: the 2.4 GHz link's own stage (0 = 400 DPI) and rate index (1 = 500 Hz), charging shown
    let mouse = Rc::new(RefCell::new(V2Mouse::new(0x100B)));
    mouse.borrow_mut().charging = true;
    let (m, mut p) = v2_setup(mouse);
    let r = mc::Mchose.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.online, r.battery_percent, r.charging, r.dpi, r.polling_hz, r.wireless), (true, Some(87), Some(true), Some(400), Some(500), Some(true)));
}

#[test]
fn the_mouse_behind_a_shared_receiver_is_named_by_its_own_answer() {
    // an A7 V2 Pro behind the receiver (battery answer names 0x4018): DPI ceiling 26000, same rate list
    let mouse = Rc::new(RefCell::new(V2Mouse::new(0x100B)));
    mouse.borrow_mut().battery_pid = 0x4018;
    let (m, mut p) = v2_setup(mouse.clone());
    let r = mc::Mchose.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!(r.model.as_deref(), Some("MCHOSE A7 V2 Pro"));
    assert_eq!(r.caps.dpi.as_ref().unwrap().max(), 26000);
    // a model OpenMouse has never heard of: no model, 26000, no polling rate (the index can't be turned into Hz)
    mouse.borrow_mut().battery_pid = 0x4999;
    let (m, mut p) = v2_setup(mouse);
    let r = mc::Mchose.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.model, r.polling_hz, r.dpi, r.caps.polling.len()), (None, None, Some(400), 0));
    assert_eq!(r.caps.dpi.unwrap().max(), 26000);
}

#[test]
fn a_stale_answer_from_another_command_is_not_taken_for_the_config() {
    let mouse = Rc::new(RefCell::new(V2Mouse::new(0x4021)));
    mouse.borrow_mut().stale_config_first = true;
    let (m, mut p) = v2_setup(mouse);
    let r = mc::Mchose.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.dpi, r.polling_hz), (Some(800), Some(1000)));
    // identity x2, battery x2, then the config asked three times: the stale battery reply, then the config twice
    assert_eq!(p.sent.len(), 7);
}

#[test]
fn a_receiver_that_says_the_mouse_is_not_linked_gets_no_more_questions() {
    let mouse = Rc::new(RefCell::new(V2Mouse::new(0x100B)));
    mouse.borrow_mut().connected = false;
    let (m, mut p) = v2_setup(mouse);
    let r = mc::Mchose.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.online, r.battery_percent, r.dpi, r.polling_hz), (false, None, None, None));
    assert_eq!(sent_of(&p), vec![req_identity(), req_identity()]);
}

#[test]
fn a_silent_mouse_is_offline_and_the_proof_does_not_loop() {
    let mouse = Rc::new(RefCell::new(V2Mouse::new(0x4021)));
    mouse.borrow_mut().quiet = true;
    let (m, mut p) = v2_setup(mouse.clone());
    let r = mc::Mchose.read(&m, &mut p, ReadOptions { online_tries: 1 }).unwrap();
    assert_eq!((r.online, r.battery_percent, r.dpi, r.polling_hz), (false, None, None, None));
    assert_eq!(p.sent.len(), 12, "the read-only proof: four tries for each of the three reads");
    assert!(p.sent.iter().all(|s| mc::read_request_allowed(&s.0)));
    // the normal read polls like OpenMouse (20 attempts each)
    let (m, mut p) = v2_setup(mouse);
    let r = mc::Mchose.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(!r.online);
    assert_eq!(p.sent.len(), 60);
}

#[test]
fn a_dpi_change_rewrites_the_configuration_the_openmouse_way() {
    // wired: stage 1 (800) is the one in use; set 1000 → stage 1 = 0x03E8
    let mouse = Rc::new(RefCell::new(V2Mouse::new(0x4021)));
    let (m, mut p) = v2_setup(mouse.clone());
    let c = mc::Mchose.write(&m, &mut p, Setting::Dpi(1000)).unwrap();
    assert_eq!((c.now, c.before), (1000, Some(800)));
    let mut want = config_payload();
    want[6] = 0xE8;
    want[7] = 0x03;
    // the cable's model is known by its id: no battery question; read twice, write once, read twice
    assert_eq!(sent_of(&p), vec![req_config(), req_config(), write_frame(&want), req_config(), req_config()]);
    assert_eq!(mouse.borrow().config, want, "only the two bytes of stage 1 changed - everything else, scratch bytes included, came back exactly");
    // 3200 on the receiver (stage 0 for the 2.4 GHz link): the model comes from the battery answer first
    let mouse = Rc::new(RefCell::new(V2Mouse::new(0x100B)));
    let (m, mut p) = v2_setup(mouse.clone());
    let c = mc::Mchose.write(&m, &mut p, Setting::Dpi(3200)).unwrap();
    assert_eq!((c.now, c.before), (3200, Some(400)));
    let mut want = config_payload();
    want[4] = 0x80;
    want[5] = 0x0C;
    assert_eq!(sent_of(&p), vec![req_battery(), req_battery(), req_config(), req_config(), write_frame(&want), req_config(), req_config()]);
    // OpenMouse rounds to the 50 grid, and its limits are 50 .. the model's ceiling
    let mouse = Rc::new(RefCell::new(V2Mouse::new(0x4021)));
    let (m, mut p) = v2_setup(mouse);
    assert_eq!(mc::Mchose.write(&m, &mut p, Setting::Dpi(42000)).unwrap().now, 42000);
    assert_eq!(mc::Mchose.write(&m, &mut p, Setting::Dpi(1010)).unwrap().now, 1000, "rounded, as Math.round(dpi / 50) * 50");
    p.sent.clear();
    for bad in [49, 42001, 0] {
        assert!(mc::Mchose.write(&m, &mut p, Setting::Dpi(bad)).is_err(), "{bad}");
    }
    assert!(p.sent.is_empty(), "refused before a byte leaves");
}

#[test]
fn a_polling_change_rewrites_the_link_byte_the_openmouse_way() {
    // wired 4000 Hz = rate index 4: payload[1] = 4 << 4 | stage 1 = 0x41
    let mouse = Rc::new(RefCell::new(V2Mouse::new(0x4021)));
    let (m, mut p) = v2_setup(mouse.clone());
    let c = mc::Mchose.write(&m, &mut p, Setting::Polling(4000)).unwrap();
    assert_eq!((c.now, c.before), (4000, Some(1000)));
    let mut want = config_payload();
    want[1] = 0x41;
    assert_eq!(sent_of(&p), vec![req_config(), req_config(), write_frame(&want), req_config(), req_config()]);
    assert_eq!(mouse.borrow().config, want);
    // receiver, 2000 Hz = rate index 3: payload[2] = 3 << 4 | stage 0 = 0x30
    let mouse = Rc::new(RefCell::new(V2Mouse::new(0x100B)));
    let (m, mut p) = v2_setup(mouse.clone());
    let c = mc::Mchose.write(&m, &mut p, Setting::Polling(2000)).unwrap();
    assert_eq!((c.now, c.before), (2000, Some(500)));
    let mut want = config_payload();
    want[2] = 0x30;
    assert_eq!(mouse.borrow().config, want);
    // a rate the model does not list is refused before a byte leaves
    let mouse = Rc::new(RefCell::new(V2Mouse::new(0x4021)));
    let (m, mut p) = v2_setup(mouse);
    for bad in [250, 750, 3000, 0] {
        assert!(mc::Mchose.write(&m, &mut p, Setting::Polling(bad)).is_err(), "{bad}");
    }
    assert!(mc::Mchose.write(&m, &mut p, Setting::LiftOff(10)).is_err(), "lift-off is not set here");
    assert!(p.sent.is_empty());
}

#[test]
fn a_mouse_that_ignores_the_write_is_reported_not_believed() {
    let mouse = Rc::new(RefCell::new(V2Mouse::new(0x4021)));
    mouse.borrow_mut().ignore_writes = true;
    let (m, mut p) = v2_setup(mouse);
    let e = mc::Mchose.write(&m, &mut p, Setting::Polling(4000)).unwrap_err();
    assert!(matches!(e, bu_mouse::Error::BadAnswer(ref s) if s.contains("polling rate")), "{e}");
    let e = mc::Mchose.write(&m, &mut p, Setting::Dpi(1000)).unwrap_err();
    assert!(matches!(e, bu_mouse::Error::BadAnswer(ref s) if s.contains("DPI")), "{e}");
    // and one that never returns its configuration is not written to at all
    let mouse = Rc::new(RefCell::new(V2Mouse::new(0x4021)));
    mouse.borrow_mut().quiet = true;
    let (m, mut p) = v2_setup(mouse);
    assert!(mc::Mchose.write(&m, &mut p, Setting::Dpi(1000)).is_err());
    assert!(p.sent.iter().all(|s| mc::read_request_allowed(&s.0)), "nothing written");
}

// ---------------------------------------------------------------- A5 Pro Max (held back from the table; proven by hand-made Matched)

const A5_ID: u8 = 0x04;

/// The request OpenMouse sends: the feature report id, then `[0, 0, 2, length, page, command, data...]`, padded to 65 bytes.
fn a5_req(length: u8, page: u8, cmd: u8, data: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 65];
    v[0] = A5_ID;
    v[3] = 2;
    v[4] = length;
    v[5] = page;
    v[6] = cmd;
    v[7..7 + data.len()].copy_from_slice(data);
    v
}

struct A5Mouse {
    polling: u8,
    profile: u8,
    stages: Vec<(u16, u16)>,
    active: u8,
    battery: (u8, u8),
    ignore_writes: bool,
    /// answer this many requests with an unrelated report before the right one
    noise: u32,
    hows: Vec<HidTransfer>,
    sent: Vec<Vec<u8>>,
}

impl A5Mouse {
    fn new() -> Self {
        A5Mouse {
            polling: 0x01,
            profile: 1,
            stages: vec![(400, 400), (800, 800), (1600, 1600), (3200, 3200), (6400, 6400), (12800, 12800)],
            active: 2,
            battery: (64, 0),
            ignore_writes: false,
            noise: 0,
            hows: Vec::new(),
            sent: Vec::new(),
        }
    }

    fn answer(&mut self, out: &[u8]) -> Vec<u8> {
        let (page, cmd) = (out[5], out[6]);
        let mut a = vec![0u8; 65];
        a[0] = A5_ID;
        a[1] = 0xA1;
        a[5] = page;
        a[6] = cmd;
        match (page, cmd) {
            (0, 0x83) => {
                a[7] = self.battery.1;
                a[8] = self.battery.0;
            }
            (0, 0x85) => a[7] = self.profile,
            (1, 0x80) => a[7] = self.polling,
            (1, 0x81) => {
                a[8] = self.stages.len() as u8;
                for (i, (x, y)) in self.stages.iter().enumerate() {
                    a[9 + i * 4..13 + i * 4].copy_from_slice(&[(x >> 8) as u8, (x & 0xFF) as u8, (y >> 8) as u8, (y & 0xFF) as u8]);
                }
            }
            (1, 0x82) => a[8] = self.active,
            (1, 0x00) => {
                if !self.ignore_writes {
                    self.polling = out[7];
                }
            }
            (1, 0x01) => {
                if !self.ignore_writes {
                    let n = out[8] as usize;
                    self.stages = (0..n).map(|i| (((out[9 + i * 4] as u16) << 8) | out[10 + i * 4] as u16, ((out[11 + i * 4] as u16) << 8) | out[12 + i * 4] as u16)).collect();
                }
            }
            (1, 0x02) => {
                if !self.ignore_writes {
                    self.active = out[8];
                }
            }
            other => panic!("a command the A5 driver must not send: {other:?}"),
        }
        a
    }
}

fn a5_setup(pid: u16, mouse: Rc<RefCell<A5Mouse>>) -> (Matched, ScriptPort) {
    let col = hid_info("a5cfg", 0x2023, pid, 0xFFFF, 0x01, (65, 65, 65));
    let m = mc::a5_matches(&[&col], 0x2023, pid, A5_ID).unwrap();
    let p = ScriptPort::new(col, move |out, how| {
        let mut mo = mouse.borrow_mut();
        mo.hows.push(how.clone());
        mo.sent.push(out.to_vec());
        if mo.noise > 0 {
            mo.noise -= 1;
            let mut a = vec![0u8; 65];
            a[0] = A5_ID;
            a[1] = 0xA1;
            a[5] = 9; // another page's answer
            return Ok(a);
        }
        Ok(mo.answer(out))
    });
    (m, p)
}

#[test]
fn the_a5_requests_are_openmouses() {
    // mchoseA5Gen1EncodeRequest: payload[2] = route (2), [3] = length, [4] = page, [5] = command, data from [6]
    let b = a5::encode_request(10, 1, 0x81, &[1, 6], 2);
    let mut want = [0u8; 64];
    want[2..8].copy_from_slice(&[2, 10, 1, 0x81, 1, 6]);
    assert_eq!(b, want);
    assert_eq!(v2::feature_report(A5_ID, &b, 65).unwrap(), a5_req(10, 1, 0x81, &[1, 6]));
    // normalised DPI + the table write: 1000 DPI = 0x03E8 → (03 E8 03 E8); [profile, count, 4 bytes per stage]
    let data = a5::encode_dpi(1, &[400, 1000, 1600, 3200, 6400, 12800]);
    assert_eq!(data, vec![1, 6, 0x01, 0x90, 0x01, 0x90, 0x03, 0xE8, 0x03, 0xE8, 0x06, 0x40, 0x06, 0x40, 0x0C, 0x80, 0x0C, 0x80, 0x19, 0x00, 0x19, 0x00, 0x32, 0x00, 0x32, 0x00]);
    assert_eq!(a5::normalize_dpi(1010), 1000);
    assert_eq!(a5::normalize_dpi(30), 50);
    assert_eq!(a5::normalize_dpi(99999), 26000);
    // polling codes: 125 → 8, 250 → 4, 500 → 2, 1000 → 1
    for (hz, code) in [(125, 8), (250, 4), (500, 2), (1000, 1)] {
        assert_eq!((a5::polling_code(hz), a5::polling_hz(code)), (Some(code), Some(hz)));
    }
    assert_eq!(a5::polling_code(2000), None);
    // answers: a 64-byte body, the report id cut off; replies echo 0xA1 / page / command
    let mut raw = vec![A5_ID; 1];
    raw.extend(vec![0xA1, 0, 0, 0, 1, 0x80, 0x02]);
    raw.resize(65, 0);
    let body = a5::decode_reply(&raw, A5_ID);
    assert_eq!((body.len(), body[0], body[6]), (64, 0xA1, 0x02));
    assert!(a5::reply_matches(&body, 1, 0x80) && !a5::reply_matches(&body, 1, 0x81) && !a5::reply_matches(&body, 0, 0x80));
    // mchoseA5Gen1DecodeDpi: count at [7] (0 → 6), then x as a big-endian word at 8 + 4 i
    let mut reply = vec![0u8; 64];
    reply[7] = 3;
    reply[8..20].copy_from_slice(&[0x01, 0x90, 0x01, 0x90, 0x03, 0x20, 0x03, 0x20, 0x06, 0x40, 0x06, 0x40]);
    let t = a5::decode_dpi(&reply);
    assert_eq!((t.count, t.stages.clone(), t.second.clone()), (3, vec![400, 800, 1600], vec![400, 800, 1600]));
    reply[7] = 0;
    assert_eq!(a5::decode_dpi(&reply).count, 6);
}

#[test]
fn only_a5_read_requests_pass_the_allow_list() {
    for r in [a5_req(2, 0, 0x83, &[]), a5_req(1, 0, 0x85, &[]), a5_req(1, 1, 0x80, &[]), a5_req(10, 1, 0x81, &[1, 6]), a5_req(2, 1, 0x82, &[2])] {
        assert!(mc::read_request_allowed(&r), "{r:02x?}");
    }
    // the setters: polling 0x00, DPI table 0x01, active stage 0x02, profile 0x05, sleep 0x07, debounce 0x08, lift-off 0x08, ...
    for (len, page, cmd, data) in [(1u8, 1u8, 0x00u8, vec![1u8]), (26, 1, 0x01, vec![1, 6]), (2, 1, 0x02, vec![1, 2]), (1, 0, 0x05, vec![2]), (2, 0, 0x07, vec![0, 60]), (2, 0, 0x08, vec![1, 5]), (1, 1, 0x08, vec![2]), (1, 1, 0x09, vec![1]), (1, 1, 0x04, vec![1]), (1, 1, 0x0a, vec![1])] {
        assert!(!mc::read_request_allowed(&a5_req(len, page, cmd, &data)), "{page}/{cmd:02x}");
    }
    // the firmware read (0x81 on page 0) and the other optional reads are OpenMouse's but not this driver's
    for (len, page, cmd) in [(16u8, 0u8, 0x81u8), (1, 0, 0x85 + 1), (2, 0, 0x87), (1, 1, 0x88), (1, 1, 0x89)] {
        assert!(!mc::read_request_allowed(&a5_req(len, page, cmd, &[])), "{page}/{cmd:02x}");
    }
    // a read with a wrong length byte or a stray data byte
    assert!(!mc::read_request_allowed(&a5_req(3, 0, 0x83, &[])));
    assert!(!mc::read_request_allowed(&a5_req(2, 0, 0x83, &[1])));
}

#[test]
fn the_a5_reads_ask_only_what_openmouse_asks() {
    let mouse = Rc::new(RefCell::new(A5Mouse::new()));
    let (m, mut p) = a5_setup(0xF013, mouse.clone());
    let r = mc::Mchose.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(p.sent.iter().all(|s| mc::read_request_allowed(&s.0)));
    // battery, profile, polling, the DPI table (profile 1, 6 stages), the active stage
    assert_eq!(sent_of(&p), vec![a5_req(2, 0, 0x83, &[]), a5_req(1, 0, 0x85, &[]), a5_req(1, 1, 0x80, &[]), a5_req(10, 1, 0x81, &[1, 6]), a5_req(2, 1, 0x82, &[1])]);
    assert_eq!(p.sent[0].1, HidTransfer::Feature { reply_id: A5_ID, reply_len: 65, wait_ms: 34 }, "send, 34 ms, read");
    assert_eq!((r.online, r.battery_percent, r.charging, r.dpi, r.polling_hz, r.wireless), (true, Some(64), Some(false), Some(800), Some(1000), Some(true)));
    assert_eq!(r.caps.polling, vec![125, 250, 500, 1000], "receivers: 125 / 250 / 500 / 1000");
    mouse.borrow_mut().battery = (101, 1);
    let (m, mut p) = a5_setup(0xF019, mouse);
    let r = mc::Mchose.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.battery_percent, r.charging, r.wireless), (Some(100), Some(true), Some(false)));
    assert_eq!(r.caps.polling, vec![125, 500, 1000], "the wired firmware: 125 / 500 / 1000");
}

#[test]
fn an_a5_request_is_resent_after_the_fourth_read_like_openmouse() {
    let mouse = Rc::new(RefCell::new(A5Mouse::new()));
    mouse.borrow_mut().noise = 5; // five unrelated answers, then the real one on the sixth read
    let (m, mut p) = a5_setup(0xF019, mouse.clone());
    let r = mc::Mchose.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(r.battery_percent.is_some());
    let f = |w| HidTransfer::Feature { reply_id: A5_ID, reply_len: 65, wait_ms: w };
    let i = |w| HidTransfer::FeatureIn { reply_id: A5_ID, reply_len: 65, wait_ms: w };
    // attempt 0: send + 34 ms + read; attempts 1-3: read after 30 ms; attempt 4: the request goes out again (30 ms), then reads
    let how: Vec<HidTransfer> = mouse.borrow().hows.iter().take(6).cloned().collect();
    assert_eq!(how, vec![f(34), i(30), i(30), i(30), f(30), i(30)]);
    // a mouse that never answers: eight tries, then the read is left out (offline), not an error
    let mouse = Rc::new(RefCell::new(A5Mouse::new()));
    mouse.borrow_mut().noise = 1000;
    let (m, mut p) = a5_setup(0xF019, mouse);
    let r = mc::Mchose.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.online, r.battery_percent, r.dpi, r.polling_hz), (false, None, None, None));
    assert_eq!(p.sent.len(), 4 * 8, "four reads, eight tries each");
}

#[test]
fn an_a5_polling_change_is_openmouses_write_and_read_back() {
    let mouse = Rc::new(RefCell::new(A5Mouse::new()));
    let (m, mut p) = a5_setup(0xF013, mouse.clone());
    let c = mc::Mchose.write(&m, &mut p, Setting::Polling(500)).unwrap();
    assert_eq!((c.now, c.before), (500, Some(1000)));
    // [read for undo], setPollingRate: request(1, 1, 0x00, [code 2]), then request(1, 1, 0x80)
    assert_eq!(sent_of(&p), vec![a5_req(1, 1, 0x80, &[]), a5_req(1, 1, 0x00, &[2]), a5_req(1, 1, 0x80, &[])]);
    assert_eq!(mouse.borrow().polling, 2);
    // the wired firmware has no 250; the receivers have no 2000
    let (m, mut p) = a5_setup(0xF019, mouse.clone());
    assert!(mc::Mchose.write(&m, &mut p, Setting::Polling(250)).is_err());
    assert!(mc::Mchose.write(&m, &mut p, Setting::Polling(2000)).is_err());
    assert!(p.sent.is_empty());
    // a mouse that keeps its rate is reported
    mouse.borrow_mut().ignore_writes = true;
    let (m, mut p) = a5_setup(0xF013, mouse);
    assert!(mc::Mchose.write(&m, &mut p, Setting::Polling(125)).is_err());
}

#[test]
fn an_a5_dpi_change_rewrites_the_stage_table_the_openmouse_way() {
    let mouse = Rc::new(RefCell::new(A5Mouse::new()));
    let (m, mut p) = a5_setup(0xF019, mouse.clone());
    let c = mc::Mchose.write(&m, &mut p, Setting::Dpi(1000)).unwrap();
    assert_eq!((c.now, c.before), (1000, Some(800)));
    let table = vec![1u8, 6, 0x01, 0x90, 0x01, 0x90, 0x03, 0xE8, 0x03, 0xE8, 0x06, 0x40, 0x06, 0x40, 0x0C, 0x80, 0x0C, 0x80, 0x19, 0x00, 0x19, 0x00, 0x32, 0x00, 0x32, 0x00];
    // profile, the table, the active stage, then writeDpi: the table (26 bytes), the active stage [profile, 2], and the read-back
    assert_eq!(
        sent_of(&p),
        vec![
            a5_req(1, 0, 0x85, &[]),
            a5_req(10, 1, 0x81, &[1, 6]),
            a5_req(2, 1, 0x82, &[1]),
            a5_req(26, 1, 0x01, &table),
            a5_req(2, 1, 0x02, &[1, 2]),
            a5_req(10, 1, 0x81, &[1, 6]),
            a5_req(2, 1, 0x82, &[1]),
        ]
    );
    assert_eq!(mouse.borrow().stages[1], (1000, 1000));
    // off the 50 grid / out of range: refused before a byte leaves
    p.sent.clear();
    for bad in [75, 49, 26050, 0] {
        assert!(mc::Mchose.write(&m, &mut p, Setting::Dpi(bad)).is_err(), "{bad}");
    }
    assert!(p.sent.is_empty());
}

#[test]
fn an_a5_table_with_different_y_values_is_left_alone() {
    let mouse = Rc::new(RefCell::new(A5Mouse::new()));
    mouse.borrow_mut().stages[3] = (3200, 1600);
    let (m, mut p) = a5_setup(0xF019, mouse.clone());
    assert!(mc::Mchose.write(&m, &mut p, Setting::Dpi(1000)).is_err());
    assert!(p.sent.iter().all(|s| mc::read_request_allowed(&s.0)), "nothing was written");
    assert_eq!(mouse.borrow().stages[3], (3200, 1600));
    // a mouse that ignores the table write is reported
    let mouse = Rc::new(RefCell::new(A5Mouse::new()));
    mouse.borrow_mut().ignore_writes = true;
    let (m, mut p) = a5_setup(0xF019, mouse);
    assert!(mc::Mchose.write(&m, &mut p, Setting::Dpi(1000)).is_err());
}

#[test]
fn an_a5_table_that_openmouses_write_would_change_is_left_alone() {
    // OpenMouse rewrites the table from what it decoded: a raw count of 0 becomes 6, a count above 6 is cut to 6, the active stage
    // is clamped, every stage goes through normalize_dpi. Each of these would silently change stored data, so nothing is written.
    type Case<'a> = (&'a str, Box<dyn Fn(&mut A5Mouse)>);
    let cases: Vec<Case> = vec![
        ("stage count 0", Box::new(|m| m.stages.clear())),
        ("stage count 7", Box::new(|m| m.stages = vec![(400, 400); 7])),
        ("active stage 0", Box::new(|m| m.active = 0)),
        ("active stage past the count", Box::new(|m| m.active = 7)),
        ("a stage off the 50 grid (26100 would be written back as 26000)", Box::new(|m| m.stages[4] = (26100, 26100))),
        ("a stage off the 50 grid (1010 would become 1000)", Box::new(|m| m.stages[2] = (1010, 1010))),
        ("a stage of 0", Box::new(|m| m.stages[0] = (0, 0))),
    ];
    for (what, change) in cases {
        let mouse = Rc::new(RefCell::new(A5Mouse::new()));
        change(&mut mouse.borrow_mut());
        let before = mouse.borrow().stages.clone();
        let (m, mut p) = a5_setup(0xF019, mouse.clone());
        assert!(mc::Mchose.write(&m, &mut p, Setting::Dpi(1000)).is_err(), "{what}");
        assert!(p.sent.iter().all(|s| mc::read_request_allowed(&s.0)), "{what}: nothing was written");
        assert_eq!(mouse.borrow().stages, before, "{what}");
    }
    // the clean table is still written
    let (m, mut p) = a5_setup(0xF019, Rc::new(RefCell::new(A5Mouse::new())));
    assert!(mc::Mchose.write(&m, &mut p, Setting::Dpi(1000)).is_ok());
    // a count of 1 and 6, and an active stage equal to the count, are fine
    let mouse = Rc::new(RefCell::new(A5Mouse::new()));
    {
        let mut mo = mouse.borrow_mut();
        mo.stages = vec![(800, 800)];
        mo.active = 1;
    }
    let (m, mut p) = a5_setup(0xF019, mouse.clone());
    assert!(mc::Mchose.write(&m, &mut p, Setting::Dpi(1000)).is_ok());
    assert_eq!(mouse.borrow().stages, vec![(1000, 1000)]);
}

#[test]
fn a5_shaped_frames_under_the_a7_v2_report_ids_do_not_pass_the_allow_list() {
    // the A5 read bytes are fine under the A5's own report id, but 0x11 / 0x12 belong to the A7 V2 (a write channel there)
    for r in [a5_req(2, 0, 0x83, &[]), a5_req(10, 1, 0x81, &[1, 6]), a5_req(2, 1, 0x82, &[2]), a5_req(1, 1, 0x80, &[])] {
        assert!(mc::read_request_allowed(&r));
        for rid in [0x11u8, 0x12] {
            let mut other = r.clone();
            other[0] = rid;
            assert!(!mc::read_request_allowed(&other), "{rid:02x} {r:02x?}");
        }
    }
}

#[test]
fn an_a5_answer_is_cut_behind_the_report_id_whatever_the_collection_length() {
    // the Windows buffer is the collection's feature-report length (here 70 and 129), the id byte first
    for len in [65usize, 70, 129] {
        let mut raw = vec![0u8; len];
        raw[0] = A5_ID;
        raw[1] = 0xA1;
        raw[5] = 1;
        raw[6] = 0x80;
        raw[7] = 0x02;
        let body = a5::decode_reply(&raw, A5_ID);
        assert_eq!(body.len(), 64);
        assert_eq!((body[0], body[4], body[5], body[6]), (0xA1, 1, 0x80, 0x02), "len {len}");
        assert!(a5::reply_matches(&body, 1, 0x80));
    }
    // an id that is itself 0xA1 does not shift the body
    let mut raw = vec![0u8; 70];
    raw[0] = 0xA1;
    raw[1] = 0xA1;
    raw[5] = 1;
    raw[6] = 0x80;
    assert!(a5::reply_matches(&a5::decode_reply(&raw, 0xA1), 1, 0x80));
    // an empty buffer is an all-zero body, which matches nothing
    assert_eq!(a5::decode_reply(&[], A5_ID), vec![0u8; 64]);
}

#[test]
fn an_a5_driver_works_on_a_collection_longer_than_65() {
    let mouse = Rc::new(RefCell::new(A5Mouse::new()));
    let col = hid_info("a5cfg", 0x2023, 0xF019, 0xFFFF, 0x01, (70, 70, 70));
    let m = mc::a5_matches(&[&col], 0x2023, 0xF019, A5_ID).unwrap();
    let mo = mouse.clone();
    let mut p = ScriptPort::new(col, move |out, _| {
        let mut mo = mo.borrow_mut();
        let mut a = mo.answer(&out[..65]);
        a.resize(70, 0);
        Ok(a)
    });
    let r = mc::Mchose.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.online, r.dpi, r.polling_hz), (true, Some(800), Some(1000)));
}

// ---------------------------------------------------------------- Order 110 (A7 V2): debounce, Motion Sync, angle snapping, ripple

/// The performance frame OpenMouse sends for these tokens: `[0x11, ~tokens (20, zeros inverted), zeros up to 65]`.
fn perf_frame(tokens: &[u8]) -> Vec<u8> {
    let mut t = tokens.to_vec();
    t.resize(20, 0);
    cat(&[&[0x11], &t.iter().map(|b| !*b).collect::<Vec<_>>(), &rep(0, 44)])
}

#[test]
fn a7_extras_read_like_openmouse() {
    // config byte 17 = 0x91: Motion Sync (0x10) on, linear correction / angle snapping (0x08) off, ripple (0x04) off, lift-off
    // step 1; keyDebounce byte 18 = 4
    let mouse = Rc::new(RefCell::new(V2Mouse::new(0x4021)));
    let (m, mut p) = v2_setup(mouse);
    let r = mc::Mchose.read(&m, &mut p, ReadOptions::default()).unwrap();
    let got: Vec<(Extra, Option<i32>)> = r.extras.iter().map(|e| (e.extra, e.value)).collect();
    assert_eq!(got, vec![(Extra::Debounce, Some(4)), (Extra::MotionSync, Some(1)), (Extra::AngleSnap, Some(0)), (Extra::RippleControl, Some(0))]);
    // MCHOSE_DEBOUNCE_MAX_MS = 20
    assert_eq!(r.extras[0].kind, Kind::Range { min: 0, max: 20, step: 1, unit: "ms" });
    assert!(p.sent.iter().all(|s| mc::read_request_allowed(&s.0)));
}

#[test]
fn a7_extras_are_openmouses_frames() {
    let mouse = Rc::new(RefCell::new(V2Mouse::new(0x4021)));
    let (m, mut p) = v2_setup(mouse.clone());
    let writes = |p: &ScriptPort| -> Vec<Vec<u8>> { p.sent.iter().filter(|s| s.1 == HidTransfer::FeatureOut).map(|s| s.0.clone()).collect() };
    // setMotionSync(false): writePerformance(lift-off step 1, { motionSync: false }) = [0x42, 1, 0, 0, 2, 0, 0, 0, 0, 0]
    mc::Mchose.change(&m, &mut p, Change::Extra(Extra::MotionSync, 0)).unwrap();
    assert_eq!(writes(&p), vec![perf_frame(&[0x42, 1, 0, 0, 2])]);
    assert_eq!(perf_frame(&[0x42, 1, 0, 0, 2])[..6], [0x11, 0xBD, 0xFE, 0xFF, 0xFF, 0xFD]);
    assert!(!mc::read_request_allowed(&writes(&p)[0]));
    // setAngleSnapping(true): line = 1 → [0x42, 1, 0, 1, 0, ...]
    p.sent.clear();
    mc::Mchose.change(&m, &mut p, Change::Extra(Extra::AngleSnap, 1)).unwrap();
    assert_eq!(writes(&p), vec![perf_frame(&[0x42, 1, 0, 1, 0])]);
    // setRippleControl(true): ripple = 1 → [0x42, 1, 1, 0, 0, ...]
    p.sent.clear();
    mc::Mchose.change(&m, &mut p, Change::Extra(Extra::RippleControl, 1)).unwrap();
    assert_eq!(writes(&p), vec![perf_frame(&[0x42, 1, 1, 0, 0])]);
    // setDebounceTime(20): the 0x57 write of the payload just read, byte 18 = 20, every other byte echoed
    p.sent.clear();
    let mut want = mouse.borrow().config.clone();
    want[18] = 20;
    mc::Mchose.change(&m, &mut p, Change::Extra(Extra::Debounce, 20)).unwrap();
    assert_eq!(writes(&p), vec![write_frame(&want)]);
    let r = mc::Mchose.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!(r.extras.iter().map(|e| e.value).collect::<Vec<_>>(), vec![Some(20), Some(0), Some(1), Some(1)]);
    assert_eq!(mouse.borrow().config[17], 0x8D, "mode bits and lift-off kept");
    // a mouse that ignores the performance command: sent 3 times (PERFORMANCE_ATTEMPTS), then an error
    mouse.borrow_mut().ignore_writes = true;
    p.sent.clear();
    assert!(mc::Mchose.change(&m, &mut p, Change::Extra(Extra::MotionSync, 1)).is_err());
    assert_eq!(writes(&p).len(), 3);
    // refused before a byte leaves
    p.sent.clear();
    for c in [Change::Extra(Extra::Debounce, 21), Change::Extra(Extra::MotionSync, 2), Change::Extra(Extra::Turbo, 1)] {
        assert!(mc::Mchose.change(&m, &mut p, c).is_err(), "{c:?}");
    }
    assert!(p.sent.is_empty());
}

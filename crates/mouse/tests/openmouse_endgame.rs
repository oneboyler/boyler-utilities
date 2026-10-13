//! Order 105 check, Endgame Gear: our requests = the bytes OpenMouse sends for the same setting (encode), and answers shaped
//! the way OpenMouse reads them decode to the same values (decode). The expected bytes are worked out by hand from OpenMouse
//! (@openmouse/protocol 0.40.1: `dist/endgame-gear/op1.js`, `ot.js`, `dist/drivers/endgame/egg-op1-hid.js`,
//! `egg-ot-hid.js`, `dist/drivers/registry.js`), never from our own encoder. Never run on a real unit.
//!
//! OP1 config protocol, OpenMouse's indexes (report id at 0): polling divider 21 (Hz = 8000 / divider), filter flags 22, lift-off
//! 25, motion sync 28, CPI stage count 30, CPI stage n at 51 + 5n (split flag, X u16 LE, Y u16 LE), glass mode 127. A command is
//! `[0xA1, op, header..]` with its payload at byte 16; the 8K store is `[0xA0, 0x11, config[2..]]`. The 4K block writes read
//! the config at `16 + o` (op1.js eggBlockWrites): sensor = config[23, 24, 25, 26, 27, 17, 30, 29] + config[51..71); power =
//! config[28, 21, 22, 20, 77, 84, 91, 98, 105, 19] (+ config[127] with glass mode); buttons = config[71..127) in two chunks.
//!
//! OT platform: a 64-byte packet `[0, comtype, packet id, payload..]`; query 51 = `[0, 51, 0, block, profile]`, set 52 =
//! `[0, 52, 0, block, effect[1..4), slice]` (main block: `[0, 52, 0, effect[0..50)]`). Effect offsets: CPI stages 4 (4 x u16
//! LE), active stage 16, polling index 17, lift-off 23, cpi-Y 103. Blocks: keyboard 50 (21), name 71 (32), cpi-Y 103 (8),
//! wireless 111 (1), rgb 112 (6), main 0 (50); sent in that order.

use bu_mouse::endgame as e;
use bu_mouse::error::Error;
use bu_mouse::family::{self, Driver, Family, Matched, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::os::{HidInfo, HidTransfer};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

const VID: u16 = 0x3367;
const LEN: usize = 1041;
const ONCE: ReadOptions = ReadOptions { online_tries: 1 };

fn info(pid: u16, feature_len: u16) -> HidInfo {
    hid_info("vendor", VID, pid, 0xFF00, 1, (64, 64, feature_len))
}

fn matched(pid: u16, feature_len: u16) -> Matched {
    e::matches(&[&info(pid, feature_len)], VID, pid).unwrap_or_else(|| panic!("{pid:#06x} did not match"))
}

/// The requests that really go to the mouse (a feature SET, alone or with its GET), not the GETs that follow.
fn sends(p: &ScriptPort) -> Vec<Vec<u8>> {
    p.sent.iter().filter(|(_, how)| matches!(how, HidTransfer::FeatureOut | HidTransfer::Feature { .. })).map(|(o, _)| o.clone()).collect()
}

fn gets(p: &ScriptPort) -> usize {
    p.sent.iter().filter(|(_, how)| matches!(how, HidTransfer::FeatureIn { .. } | HidTransfer::Feature { .. })).count()
}

// ======================================================================================== the OP1 fake mouse

#[derive(Clone, Copy)]
enum Last {
    Nothing,
    Load,
    Info,
    Ack,
}

struct Op1Mouse {
    cfg: Vec<u8>,
    glass_in_power: bool,
    /// the PID the mouse-info answer names (`None` = the mouse does not relay: status 8)
    paired: Option<u16>,
    asleep: bool,
    /// the status the acknowledgement gives (1 OK, 7 rejected)
    ack_status: u8,
    busy_left: u32,
    ignore_store: bool,
    last: Last,
    stores: usize,
    /// GET 0xA1 after a load answers this (a short command reply whose first bytes look like a config, zero-padded)
    a1_decoy: Option<Vec<u8>>,
    /// GET 0xA0 answers nothing but zeros (no config there)
    a0_dead: bool,
}

impl Op1Mouse {
    fn new(cfg: Vec<u8>) -> Rc<RefCell<Op1Mouse>> {
        Rc::new(RefCell::new(Op1Mouse { cfg, glass_in_power: false, paired: None, asleep: false, ack_status: 1, busy_left: 0, ignore_store: false, last: Last::Nothing, stores: 0, a1_decoy: None, a0_dead: false }))
    }

    fn set(&mut self, out: &[u8]) {
        match (out[0], out[1]) {
            (0xA1, 0x12) => self.last = Last::Load,
            (0xA1, 0x0e) => self.last = Last::Info,
            (0xA1, op @ 0x14..=0x16) => {
                self.block(op, out);
                self.last = Last::Ack;
            }
            (0xA0, 0x11) => {
                self.stores += 1;
                if !self.ignore_store {
                    self.cfg[2..out.len()].copy_from_slice(&out[2..]);
                }
                self.last = Last::Ack;
            }
            other => panic!("the fake mouse got a request it should never see: {other:02x?}"),
        }
    }

    /// The inverse of op1.js eggBlockWrites, from the vendor blob offsets (config index = 16 + offset).
    fn block(&mut self, op: u8, out: &[u8]) {
        let p = &out[16..];
        let c = &mut self.cfg;
        match op {
            0x14 => {
                for (i, at) in [0x07usize, 0x08, 0x09, 0x0a, 0x0b, 0x01, 0x0e, 0x0d].iter().enumerate() {
                    c[16 + at] = p[i];
                }
                for i in 0..20 {
                    c[16 + 0x23 + i] = p[8 + i];
                }
            }
            0x15 => {
                for (i, at) in [0x0cusize, 0x05, 0x06, 0x04].iter().enumerate() {
                    c[16 + at] = p[i];
                }
                for b in 0..5 {
                    c[16 + 0x3d + b * 7] = p[4 + b];
                }
                c[16 + 0x03] = p[9];
                if self.glass_in_power {
                    c[16 + 0x6f] = p[10];
                }
            }
            _ => {
                let chunk = out[6] as usize;
                let at = 16 + 0x37 + 28 * (chunk - 1);
                c[at..at + 28].copy_from_slice(&p[..28]);
            }
        }
    }

    fn get(&mut self, id: u8, len: usize) -> Vec<u8> {
        let mut v = vec![0u8; len];
        v[0] = id;
        if self.asleep {
            v[1] = 8;
            return v;
        }
        match id {
            0xA0 if self.a0_dead => {}
            0xA0 => {
                v.copy_from_slice(&self.cfg[..len]);
                v[0] = 0xA0;
            }
            _ => match self.last {
                Last::Load if self.a1_decoy.is_some() => {
                    let d = self.a1_decoy.as_ref().unwrap();
                    v[..d.len()].copy_from_slice(d);
                    v[0] = 0xA1;
                }
                Last::Info => match self.paired {
                    Some(pid) => {
                        v[1] = 1;
                        v[16..18].copy_from_slice(&VID.to_le_bytes());
                        v[18..20].copy_from_slice(&pid.to_le_bytes());
                    }
                    None => v[1] = 8,
                },
                Last::Ack if self.busy_left > 0 => {
                    self.busy_left -= 1;
                    v[1] = 3;
                }
                Last::Ack => v[1] = self.ack_status,
                _ => v[1] = 1,
            },
        }
        v
    }
}

fn op1_port(pid: u16, m: &Rc<RefCell<Op1Mouse>>) -> ScriptPort {
    let m = m.clone();
    ScriptPort::new(info(pid, LEN as u16), move |out, how| {
        let mut s = m.borrow_mut();
        if matches!(how, HidTransfer::FeatureOut | HidTransfer::Feature { .. }) {
            s.set(out);
        }
        match how {
            HidTransfer::Feature { reply_id, reply_len, .. } | HidTransfer::FeatureIn { reply_id, reply_len, .. } => Ok(s.get(*reply_id, *reply_len)),
            _ => Ok(Vec::new()),
        }
    })
}

/// A config as an OP1 8K hands it over: 1000 Hz, 1.0 mm, two CPI stages (800 / 1600), the rest a pattern to prove that every
/// other byte is copied back unchanged.
fn base() -> Vec<u8> {
    let mut c: Vec<u8> = (0..LEN).map(|i| (i * 7 + 3) as u8).collect();
    c[0] = 0xA0;
    c[1] = 1;
    c[21] = 8;
    c[22] = 0x01;
    c[25] = 1;
    c[28] = 0;
    c[30] = 2;
    c[127] = 0;
    c[51..56].copy_from_slice(&[0, 0x20, 0x03, 0x20, 0x03]);
    c[56..61].copy_from_slice(&[0, 0x40, 0x06, 0x40, 0x06]);
    c
}

fn load_report() -> Vec<u8> {
    let mut v = vec![0u8; LEN];
    v[0] = 0xA1;
    v[1] = 0x12;
    v
}

fn read(pid: u16, m: &Rc<RefCell<Op1Mouse>>) -> (bu_mouse::family::OnMouse, ScriptPort) {
    let mut port = op1_port(pid, m);
    let r = e::Endgame.read(&matched(pid, LEN as u16), &mut port, ReadOptions::default()).unwrap();
    (r, port)
}

fn write(pid: u16, m: &Rc<RefCell<Op1Mouse>>, s: Setting) -> (Result<family::Changed, Error>, ScriptPort) {
    let mut port = op1_port(pid, m);
    let r = e::Endgame.write(&matched(pid, LEN as u16), &mut port, s);
    (r, port)
}

// ======================================================================================== OP1: decode / read

#[test]
fn op1_8k_read_decodes_like_openmouse() {
    let m = Op1Mouse::new(base());
    let (r, port) = read(0x1964, &m);
    // readStatus: dpi = stage 1 X (c[52..54] = 0x0320 = 800); polling = 8000 / c[21] = 8000 / 8; lift-off option c[25] = 1 of
    // ["0.7 mm", "1 mm", "2 mm"] (8K v1, base 0) = 1 mm
    assert_eq!((r.online, r.dpi, r.polling_hz, r.lift_off, r.model, r.wireless), (true, Some(800), Some(1000), Some(10), None, Some(false)));
    assert_eq!(r.battery_percent, None);
    // the caps: PAW3395 50..26000 in 50s; every rate to 8000 (Motion Sync is off); lift-off 0.7 / 1 / 2 mm; no battery
    let caps = r.caps;
    assert_eq!(caps.dpi.as_ref().map(|d| d.bands.clone()), Some(vec![(50, 26_000, 50)]));
    assert_eq!(caps.polling, vec![125, 250, 500, 1000, 2000, 4000, 8000]);
    assert_eq!((caps.lift_off, caps.battery), (vec![7, 10, 20], false));
    // readConfigRaw: command 0x12 padded to the feature length, then GET 0xA1 and GET 0xA0 on that same request
    assert_eq!(sends(&port), vec![load_report()]);
    assert_eq!(port.sent.len(), 3);
    assert_eq!(port.sent[1].1, HidTransfer::FeatureIn { reply_id: 0xA1, reply_len: LEN, wait_ms: 0 });
    assert_eq!(port.sent[2].1, HidTransfer::FeatureIn { reply_id: 0xA0, reply_len: LEN, wait_ms: 0 });
    assert!(port.sent.iter().all(|(o, _)| e::read_request_allowed(o)), "every read request passes the allow-list");
}

#[test]
fn op1_polling_dividers_decode_like_openmouse() {
    // decodePollingRate: 8000 / divider; 0 is invalid; the 4K mice's 0x80 is 1000 Hz with power saving
    let p8 = e::op1_profile(0x1964).unwrap();
    for (div, hz) in [(1u8, 8000u32), (2, 4000), (4, 2000), (8, 1000), (16, 500), (32, 250), (64, 125)] {
        assert_eq!(p8.decode_polling(div), Some(hz), "divider {div}");
    }
    assert_eq!((p8.decode_polling(0), p8.decode_polling(0x80), p8.decode_polling(3)), (None, None, None));
    let p4 = e::op1_profile(0x1984).unwrap();
    assert_eq!((p4.decode_polling(0x80), p4.decode_polling(2), p4.decode_polling(0x40)), (Some(1000), Some(4000), Some(125)));
}

#[test]
fn op1_cpi_rules_are_openmouses() {
    // eggClampCpi / eggIsValidCpi: v1 steps of 50; v2 steps of 10 up to 10000, then 50
    let v1 = e::op1_profile(0x1964).unwrap();
    assert!(v1.is_valid_cpi(50) && v1.is_valid_cpi(26_000) && !v1.is_valid_cpi(26_050) && !v1.is_valid_cpi(1610) && !v1.is_valid_cpi(25));
    let v2 = e::op1_profile(0x1978).unwrap();
    // 10005: step(10005) = 50 -> floor((10005 + 25) / 50) * 50 = 10000 != 10005; 9995: step 10 -> floor(10000 / 10) * 10 = 10000
    assert!(v2.is_valid_cpi(10) && v2.is_valid_cpi(1610) && v2.is_valid_cpi(10_000) && v2.is_valid_cpi(10_050) && v2.is_valid_cpi(30_000));
    assert!(!v2.is_valid_cpi(10_005) && !v2.is_valid_cpi(9995) && !v2.is_valid_cpi(15) && !v2.is_valid_cpi(30_050) && !v2.is_valid_cpi(5));
    // eggDpiOptions: 10, 20 ... 10000, 10050, 10100 ... 30000
    assert_eq!(v2.dpi_caps().bands, vec![(10, 10_000, 10), (10_050, 30_000, 50)]);
    assert_eq!(e::op1_profile(0x1976).unwrap().dpi_caps().bands, vec![(50, 30_000, 50)], "Purple Frost: PAW3950 limits on the v1 protocol");
    assert_eq!(e::op1_profile(0x1972).unwrap().dpi_caps().bands, vec![(50, 26_000, 50)], "OP1w 4K v1: PAW3395");
}

#[test]
fn op1_models_have_their_openmouse_rates_and_lift_off() {
    let rates = |pid| e::op1_profile(pid).unwrap().polling_rates();
    assert_eq!(rates(0x1964), vec![125, 250, 500, 1000, 2000, 4000, 8000]);
    assert_eq!(rates(0x1978), vec![125, 250, 500, 1000, 2000, 4000, 8000]);
    assert_eq!(rates(0x1972), vec![1000, 2000, 4000], "4K v1: EGG_4K_V1_POLLING_RATES");
    assert_eq!(rates(0x1984), vec![125, 1000, 2000, 4000], "4K v2: EGG_4K_POLLING_RATES");
    let lod = |pid, glass| e::op1_profile(pid).unwrap().lod_options(glass).to_vec();
    assert_eq!(lod(0x1964, false), vec![7, 10, 20]);
    assert_eq!(lod(0x1978, false), (7..=17).collect::<Vec<u32>>());
    assert_eq!(lod(0x1978, true), vec![10, 20]);
    assert_eq!(lod(0x1964, true), vec![7, 10, 20], "no glass mode on the OP1 8K");
    assert_eq!(lod(0x1972, false), vec![10, 20], "4K v1: LOD_4K_V1");
    // lodBase: the 4K v1 (and the 4K v2 in glass mode) store whole millimetres, option 0 = wire value 1
    let base = |pid, glass| e::op1_profile(pid).unwrap().lod_base(glass);
    assert_eq!((base(0x1964, false), base(0x1972, false), base(0x1984, false), base(0x1984, true), base(0x1978, true)), (0, 1, 0, 1, 0));
}

#[test]
fn op1_feature_replies_normalise_like_openmouse() {
    // eggNormalizeFeatureReport(raw, 0xA1, 64, payload 8): raw = 8 bytes with the wire header first and a status behind it:
    // [A1, 01, ..] -> result[0] = report id, the rest stays: [A1, 01, 02, 03, 04, 05, 06, 07] padded to 64
    let w = e::normalize_feature(&[0xA1, 0x01, 2, 3, 4, 5, 6, 7], 0xA1, 64, 8);
    assert_eq!((w.len(), &w[..8]), (64, &[0xA1, 0x01, 2, 3, 4, 5, 6, 7][..]));
    // status 7 (rejected) and 8 (unreachable) are framing too; so is a leading 0
    assert_eq!(&e::normalize_feature(&[0, 8, 9, 9, 9, 9, 9, 9], 0xA1, 64, 8)[..3], &[0xA1, 8, 9]);
    assert_eq!(&e::normalize_feature(&[0xA0, 7, 9, 9, 9, 9, 9, 9], 0xA0, 64, 8)[..3], &[0xA0, 7, 9]);
    // no header: payload length == raw length and raw[0] is the status: the id goes in front
    let n = e::normalize_feature(&[0x01, 0x42, 3, 4, 5, 6, 7, 8], 0xA0, 64, 8);
    assert_eq!(&n[..4], &[0xA0, 0x01, 0x42, 3]);
    // raw one byte longer than the payload, starting with the id: it already has it
    let i = e::normalize_feature(&[0xA1, 0x01, 0x42, 3, 4, 5, 6, 7, 8], 0xA1, 64, 8);
    assert_eq!(&i[..4], &[0xA1, 0x01, 0x42, 3]);
    // a short raw is padded to the expected total
    assert_eq!(e::normalize_feature(&[1, 2], 0xA0, 1041, 8).len(), 1041);
}

// ======================================================================================== OP1: encode (8K, whole-config store)

#[test]
fn op1_8k_polling_write_is_openmouses_store() {
    let m = Op1Mouse::new(base());
    let (r, port) = write(0x1964, &m, Setting::Polling(4000));
    assert_eq!(r, Ok(family::Changed { now: 4000, before: Some(1000) }));
    // setPollingRate(4000): divider 8000 / 4000 = 2 -> config[21]; writeConfigRaw: [0xA0, 0x11 (store), config[2..]] - every
    // other byte of the config just read, unchanged
    let mut want = base();
    want[1] = 0x11;
    want[21] = 2;
    let s = sends(&port);
    assert_eq!(s.len(), 3, "load, store, load (the read-back)");
    assert_eq!((&s[0], &s[2]), (&load_report(), &load_report()));
    assert_eq!(s[1], want);
    // the acknowledgement is fetched with GET 0xA1 after the 300 ms pause
    assert!(port.sent.iter().any(|(o, how)| o[1] == 0x11 && *how == HidTransfer::FeatureIn { reply_id: 0xA1, reply_len: LEN, wait_ms: 0 }));
    assert_eq!(m.borrow().cfg[21], 2);
}

#[test]
fn op1_8k_dpi_write_is_openmouses_set_cpi_stage_for_stage_1() {
    let m = Op1Mouse::new(base());
    let (r, port) = write(0x1964, &m, Setting::Dpi(1600));
    assert_eq!(r, Ok(family::Changed { now: 1600, before: Some(800) }));
    // setCpiStage(0, 1600, 1600): config[51] = 0 (x == y), 1600 = 0x0640 little-endian = 40 06 at 52 and 54. Stage 2 and
    // everything else stay as read.
    let mut want = base();
    want[1] = 0x11;
    want[51..56].copy_from_slice(&[0x00, 0x40, 0x06, 0x40, 0x06]);
    assert_eq!(sends(&port)[1], want);
}

#[test]
fn op1_dpi_write_refuses_a_split_stage_and_sends_nothing() {
    // OpenMouse's setCpiStage(0, d, d) would turn a split stage (flag byte 1, X 800 / Y 600) into d / d with the flag 0;
    // undo remembers X only and could not bring Y or the flag back, so the write is refused before anything is sent.
    // (flag byte, X, Y): flag set with X != Y, flag set with X == Y, no flag but X != Y
    for (flag, x, y) in [(1u8, 800u16, 600u16), (1, 800, 800), (0, 800, 600)] {
        for pid in [0x1964u16, 0x1984] {
            let mut c = base();
            c[51] = flag;
            c[52..54].copy_from_slice(&x.to_le_bytes());
            c[54..56].copy_from_slice(&y.to_le_bytes());
            let m = Op1Mouse::new(c.clone());
            let (r, port) = write(pid, &m, Setting::Dpi(1600));
            assert!(matches!(r, Err(Error::OutOfRange { .. })), "{pid:#06x} flag {flag} {x}/{y}: {r:?}");
            assert_eq!(sends(&port), vec![load_report()], "only the read request went out");
            assert_eq!(m.borrow().cfg, c, "the mouse is unchanged");
            assert_eq!(m.borrow().stores, 0);
        }
    }
    // the same stage still READS (stage 1's X is the DPI), and polling / lift-off writes keep the split stage as it was
    let mut c = base();
    c[51..56].copy_from_slice(&[1, 0x20, 0x03, 0x58, 0x02]);
    let m = Op1Mouse::new(c.clone());
    assert_eq!(read(0x1964, &m).0.dpi, Some(800));
    let (r, port) = write(0x1964, &m, Setting::Polling(2000));
    assert!(r.is_ok());
    assert_eq!(&sends(&port)[1][51..56], &[1, 0x20, 0x03, 0x58, 0x02]);
}

#[test]
fn op1_a_write_never_builds_on_a_short_command_reply() {
    // OpenMouse tells the 0xA1 command reply from the config by its raw length (>= 131); Windows' answer is always the full
    // feature length, so the test cannot fire. A decoy: GET 0xA1 answers 64 bytes that look like a config (status 1, 2 stages,
    // divider 8, stage 1 = 1600), zero behind them, while GET 0xA0 gives no config. Storing it would zero stage 2.
    let mut decoy = base()[..64].to_vec();
    decoy[52..56].copy_from_slice(&[0x40, 0x06, 0x40, 0x06]);
    for pid in [0x1964u16, 0x1984] {
        for setting in [Setting::Dpi(1000), Setting::Polling(2000), Setting::LiftOff(20)] {
            let m = Op1Mouse::new(base());
            m.borrow_mut().a1_decoy = Some(decoy.clone());
            m.borrow_mut().a0_dead = true;
            let (r, port) = write(pid, &m, setting);
            assert!(matches!(r, Err(Error::BadAnswer(ref s)) if s.contains("valid configuration")), "{pid:#06x} {setting:?}: {r:?}");
            // three load rounds, only GET 0xA0 asked (3 rounds x 6 attempts), nothing stored or block-written
            assert_eq!(sends(&port), vec![load_report(); 3]);
            assert_eq!(gets(&port), 18);
            assert!(port.sent.iter().all(|(_, how)| !matches!(how, HidTransfer::FeatureIn { reply_id: 0xA1, .. })));
            assert_eq!(m.borrow().cfg, base());
            assert_eq!(m.borrow().stores, 0);
        }
    }
    // the same decoy next to a real config on GET 0xA0: the write uses the 0xA0 one (stage 2 stays 1600 = 0x0640)
    let m = Op1Mouse::new(base());
    m.borrow_mut().a1_decoy = Some(decoy.clone());
    let (r, port) = write(0x1964, &m, Setting::Polling(2000));
    assert_eq!(r, Ok(family::Changed { now: 2000, before: Some(1000) }));
    let mut want = base();
    want[1] = 0x11;
    want[21] = 4;
    assert_eq!(sends(&port)[1], want);
    // a READ for display may still use the short reply, as OpenMouse's order (0xA1 first) does
    let m = Op1Mouse::new(base());
    m.borrow_mut().a1_decoy = Some(decoy);
    m.borrow_mut().a0_dead = true;
    assert!(read(0x1964, &m).0.online);
}

#[test]
fn op1_4k_power_saving_polling_has_no_undo_value() {
    // 4K polling byte 0x80 = "1000 Hz with power saving": it reads as 1000 Hz, but a write of 1000 Hz stores divider 8, so
    // "undo to 1000" would turn power saving off. No before value then; any other byte keeps its before value.
    let mut c = base();
    c[21] = 0x80;
    let m = Op1Mouse::new(c);
    m.borrow_mut().glass_in_power = true;
    assert_eq!(read(0x1984, &m).0.polling_hz, Some(1000));
    let (r, _) = write(0x1984, &m, Setting::Polling(2000));
    assert_eq!(r, Ok(family::Changed { now: 2000, before: None }));
    assert_eq!(m.borrow().cfg[21], 4);
    let (r, _) = write(0x1984, &m, Setting::Polling(1000));
    assert_eq!(r, Ok(family::Changed { now: 1000, before: Some(2000) }));
    assert_eq!(m.borrow().cfg[21], 8);
    // the 8K models have no such byte: 0x80 is just divider 128 = 62.5 Hz, no rate
    let mut c = base();
    c[21] = 0x80;
    let m = Op1Mouse::new(c);
    let (r, _) = write(0x1964, &m, Setting::Polling(2000));
    assert_eq!(r, Ok(family::Changed { now: 2000, before: None }));
}

#[test]
fn op1_8k_lift_off_write_is_openmouses_lod_index() {
    // setEggLodIndex(index): config[25] = index + lodBase. OP1 8K ["0.7", "1", "2"] base 0: 2 mm = index 2
    let m = Op1Mouse::new(base());
    let (r, port) = write(0x1964, &m, Setting::LiftOff(20));
    assert_eq!(r, Ok(family::Changed { now: 20, before: Some(10) }));
    let mut want = base();
    want[1] = 0x11;
    want[25] = 2;
    assert_eq!(sends(&port)[1], want);
    // 8K v2: 1.0 mm is the fourth of 0.7 ... 1.7 = index 3
    let m = Op1Mouse::new(base());
    let (r, port) = write(0x1978, &m, Setting::LiftOff(10));
    // (config[25] = 1 is the second v2 option, 0.8 mm)
    assert_eq!(r.map(|c| c.before), Ok(Some(8)));
    let mut want = base();
    want[1] = 0x11;
    want[25] = 3;
    assert_eq!(sends(&port)[1], want);
    // glass mode (config[127] != 0): the options are ["1.0", "2.0"]; 2 mm = index 1; the OP1 8K Purple Frost: 1 mm = index 0
    let mut g = base();
    g[127] = 1;
    let m = Op1Mouse::new(g.clone());
    let (r, port) = write(0x1978, &m, Setting::LiftOff(20));
    assert!(r.is_ok());
    let mut want = g.clone();
    want[1] = 0x11;
    want[25] = 1;
    assert_eq!(sends(&port)[1], want);
    let m = Op1Mouse::new(g.clone());
    let (_, port) = write(0x1976, &m, Setting::LiftOff(10));
    assert_eq!(sends(&port)[1][25], 0);
    // an option the mode does not list is refused before anything is stored (2 mm outside glass on the v2 list)
    let m = Op1Mouse::new(base());
    let (r, port) = write(0x1978, &m, Setting::LiftOff(20));
    assert!(matches!(r, Err(Error::OutOfRange { .. })), "{r:?}");
    assert_eq!(m.borrow().stores, 0);
    assert_eq!(sends(&port), vec![load_report()]);
}

#[test]
fn op1_8k_refuses_what_openmouse_refuses_before_writing() {
    // assertCpi: a CPI the profile can't store
    let m = Op1Mouse::new(base());
    let (r, _) = write(0x1964, &m, Setting::Dpi(1610));
    assert!(matches!(r, Err(Error::OutOfRange { .. })));
    let (r, _) = write(0x1964, &m, Setting::Dpi(26_050));
    assert!(matches!(r, Err(Error::OutOfRange { .. })));
    // the v2 takes 1610
    let m2 = Op1Mouse::new(base());
    let (r, _) = write(0x1978, &m2, Setting::Dpi(1610));
    assert_eq!(r, Ok(family::Changed { now: 1610, before: Some(800) }));
    // a polling rate the model doesn't list (3000), and 8000 on a 4K
    let (r, _) = write(0x1964, &m, Setting::Polling(3000));
    assert!(matches!(r, Err(Error::OutOfRange { .. })));
    let m4 = Op1Mouse::new(base());
    let (r, _) = write(0x1984, &m4, Setting::Polling(8000));
    assert!(matches!(r, Err(Error::OutOfRange { .. })));
    assert_eq!(m.borrow().stores + m4.borrow().stores, 0, "nothing was stored");
    // glass mode: "Polling rate is controlled by firmware while Glass Mode is active."
    let mut g = base();
    g[127] = 1;
    let mg = Op1Mouse::new(g);
    let (r, port) = write(0x1978, &mg, Setting::Polling(2000));
    assert!(matches!(r, Err(Error::OutOfRange { .. })));
    assert_eq!(sends(&port), vec![load_report()]);
    // ... and the read offers no polling rate then
    let (r, _) = read(0x1978, &mg);
    assert!(r.caps.polling.is_empty());
}

#[test]
fn op1_8000_hz_never_switches_motion_sync_off_behind_your_back() {
    // OpenMouse: setPollingRate(8000) on the OP1 8K / XM2 8K also writes config[28] = 0 (Motion Sync off). This port writes
    // 8000 Hz there only while Motion Sync is already off, so the bytes are OpenMouse's and nothing else changes.
    let off = Op1Mouse::new(base());
    let (r, port) = write(0x1964, &off, Setting::Polling(8000));
    assert_eq!(r, Ok(family::Changed { now: 8000, before: Some(1000) }));
    let mut want = base();
    want[1] = 0x11;
    want[21] = 1;
    assert_eq!(sends(&port)[1], want);
    // Motion Sync on: refused, nothing sent but the load; and 8000 is not offered
    let mut on = base();
    on[28] = 1;
    let m = Op1Mouse::new(on.clone());
    let (r, port) = write(0x1966, &m, Setting::Polling(8000));
    assert!(matches!(r, Err(Error::OutOfRange { .. })));
    assert_eq!((sends(&port), m.borrow().stores), (vec![load_report()], 0));
    let (rd, _) = read(0x1966, &m);
    assert_eq!(rd.caps.polling, vec![125, 250, 500, 1000, 2000, 4000]);
    // a model whose Motion Sync works at 8 kHz (the 8K v2): 8000 Hz with Motion Sync on writes divider 1 and leaves c[28] alone
    let m = Op1Mouse::new(on.clone());
    let (r, port) = write(0x1978, &m, Setting::Polling(8000));
    assert!(r.is_ok());
    let mut want = on;
    want[1] = 0x11;
    want[21] = 1;
    assert_eq!(sends(&port)[1], want);
    let (rd, _) = read(0x1978, &m);
    assert!(rd.caps.polling.contains(&8000));
}

#[test]
fn op1_mouse_refusals_and_busy_acknowledgements() {
    // the store is rejected (status 7 on the third try = "not busy, not OK"): an error, and no read-back
    let m = Op1Mouse::new(base());
    m.borrow_mut().ack_status = 7;
    let (r, port) = write(0x1964, &m, Setting::Polling(2000));
    assert!(matches!(r, Err(Error::BadAnswer(_))), "{r:?}");
    assert_eq!(sends(&port).len(), 2, "load, store - no confirming load");
    // pollCommandOk: attempt 0 and 1 may be anything but OK and keep asking; the third non-OK, non-busy answer stops it
    let acks = port.sent.iter().filter(|(o, how)| o[1] == 0x11 && matches!(how, HidTransfer::FeatureIn { reply_id: 0xA1, .. })).count();
    assert_eq!(acks, 3);
    // busy (status 3) twice, then OK: the write goes through
    let m = Op1Mouse::new(base());
    m.borrow_mut().busy_left = 2;
    let (r, _) = write(0x1964, &m, Setting::Polling(2000));
    assert_eq!(r.map(|c| c.now), Ok(2000));
    // the mouse acknowledges but keeps its old value: "The mouse kept 1000 Hz instead of 2000 Hz."
    let m = Op1Mouse::new(base());
    m.borrow_mut().ignore_store = true;
    let (r, _) = write(0x1964, &m, Setting::Polling(2000));
    assert!(matches!(r, Err(Error::BadAnswer(ref s)) if s.contains("kept")), "{r:?}");
}

#[test]
fn op1_asleep_mouse_reads_as_offline() {
    let m = Op1Mouse::new(base());
    m.borrow_mut().asleep = true;
    // the read-only proof asks once: one load, one GET of each report
    let mut port = op1_port(0x1964, &m);
    let r = e::Endgame.read(&matched(0x1964, LEN as u16), &mut port, ONCE).unwrap();
    assert!(!r.online && r.dpi.is_none() && r.polling_hz.is_none());
    assert_eq!((sends(&port).len(), gets(&port)), (1, 2));
    assert!(r.caps.dpi.is_some(), "a known cable id still says what the model can do");
    // the full budget is OpenMouse's: 3 rounds of 6 attempts x 2 reports
    let mut port = op1_port(0x1964, &m);
    let r = e::Endgame.read(&matched(0x1964, LEN as u16), &mut port, ReadOptions::default()).unwrap();
    assert!(!r.online);
    assert_eq!((sends(&port).len(), gets(&port)), (3, 36));
    // the shared dongle: nothing known about the mouse behind it
    let mut port = op1_port(0x1970, &m);
    let r = e::Endgame.read(&matched(0x1970, LEN as u16), &mut port, ONCE).unwrap();
    assert!(!r.online && r.model.is_none() && r.caps == Default::default() && r.wireless == Some(true));
    // a write to a sleeping mouse is an error, not a guess
    let (w, _) = write(0x1964, &m, Setting::Polling(2000));
    assert!(matches!(w, Err(Error::BadAnswer(_))));
}

// ======================================================================================== OP1: encode (4K, block writes)

fn hdr(op: u8, declared: u8, chunk: u8, payload: &[u8]) -> Vec<u8> {
    // [A1, op, 0x0f (target mouse), declared length, 0, 0, chunk, zeros up to 16, payload]
    let mut v = vec![0u8; LEN];
    v[0] = 0xA1;
    v[1] = op;
    v[2] = 0x0f;
    v[3] = declared;
    v[6] = chunk;
    v[16..16 + payload.len()].copy_from_slice(payload);
    v
}

#[test]
fn op1_4k_v2_polling_write_sends_only_the_power_block() {
    let b = base();
    let m = Op1Mouse::new(b.clone());
    m.borrow_mut().glass_in_power = true;
    let (r, port) = write(0x1984, &m, Setting::Polling(2000));
    assert_eq!(r, Ok(family::Changed { now: 2000, before: Some(1000) }));
    // 2000 Hz = divider 4 (config[21]). Power block (op 0x15, declared 10, chunk 0) payload, 11 bytes with glass mode:
    // config[28, 21 (= 4), 22, 20, 77, 84, 91, 98, 105, 19, 127]
    let power = [b[28], 4, b[22], b[20], b[77], b[84], b[91], b[98], b[105], b[19], b[127]];
    let s = sends(&port);
    assert_eq!(s.len(), 3, "load, power block, load - the sensor and button blocks did not change");
    assert_eq!(s[1], hdr(0x15, 10, 0, &power));
    assert_eq!(m.borrow().cfg[21], 4);
    assert_eq!(m.borrow().stores, 0, "the 4K mice never get the whole-config store");
    // the block's acknowledgement: send, 150 ms, GET 0xA1
    assert!(port.sent.iter().any(|(o, how)| o[1] == 0x15 && matches!(how, HidTransfer::FeatureIn { reply_id: 0xA1, .. })));
}

#[test]
fn op1_4k_v2_dpi_write_sends_only_the_sensor_block() {
    let b = base();
    let m = Op1Mouse::new(b.clone());
    m.borrow_mut().glass_in_power = true;
    let (r, port) = write(0x1984, &m, Setting::Dpi(1000));
    assert_eq!(r, Ok(family::Changed { now: 1000, before: Some(800) }));
    // 1000 = 0x03E8 -> E8 03 on both axes of stage 1; sensor block (op 0x14, declared 28, chunk 0): config[23, 24, 25, 26, 27,
    // 17, 30, 29] then config[51..71) with stage 1 replaced
    let mut stages = b[51..71].to_vec();
    stages[..5].copy_from_slice(&[0, 0xE8, 0x03, 0xE8, 0x03]);
    let mut sensor = vec![b[23], b[24], b[25], b[26], b[27], b[17], b[30], b[29]];
    sensor.extend(stages);
    assert_eq!(sensor.len(), 28);
    let s = sends(&port);
    assert_eq!(s.len(), 3);
    assert_eq!(s[1], hdr(0x14, 28, 0, &sensor));
}

#[test]
fn op1_4k_v1_lift_off_write_stores_whole_millimetres() {
    // 4K v1 (OP1w 4K cable): options ["1 mm", "2 mm"], wire value = index + 1. config[25] = 1 reads as 1 mm; 2 mm = wire 2
    let b = base();
    let m = Op1Mouse::new(b.clone());
    let (r, port) = write(0x1972, &m, Setting::LiftOff(20));
    assert_eq!(r, Ok(family::Changed { now: 20, before: Some(10) }));
    let mut sensor = vec![b[23], b[24], 2, b[26], b[27], b[17], b[30], b[29]];
    sensor.extend_from_slice(&b[51..71]);
    let s = sends(&port);
    assert_eq!(s.len(), 3);
    assert_eq!(s[1], hdr(0x14, 28, 0, &sensor));
    // the 4K v1 has no 0.7 mm option
    let (r, _) = write(0x1972, &m, Setting::LiftOff(7));
    assert!(matches!(r, Err(Error::OutOfRange { .. })));
    // 4K v2 glass mode: wire = index + 1 as well: config[25] = 2 reads as the second glass option, 2.0 mm
    let mut g = base();
    g[127] = 1;
    g[25] = 2;
    let m = Op1Mouse::new(g);
    let (r, _) = read(0x1984, &m);
    assert_eq!(r.lift_off, Some(20));
    assert!(r.caps.polling.is_empty() && r.caps.lift_off == vec![10, 20]);
}

#[test]
fn op1_4k_buttons_chunks_go_together() {
    // eggBlockWrites on a config whose byte i is i: sensor, power (+ glass), buttons 71..127 in two chunks of 28
    let cfg: Vec<u8> = (0..LEN).map(|i| i as u8).collect();
    let w = e::block_writes(&cfg, true);
    assert_eq!(w.len(), 4);
    let mut sensor = vec![23u8, 24, 25, 26, 27, 17, 30, 29];
    sensor.extend(51u8..71);
    assert_eq!((w[0].command, w[0].declared_length, w[0].chunk), (0x14, 28, 0));
    assert_eq!(w[0].payload, sensor);
    assert_eq!((w[1].command, w[1].declared_length, w[1].chunk), (0x15, 10, 0));
    assert_eq!(w[1].payload, vec![28u8, 21, 22, 20, 77, 84, 91, 98, 105, 19, 127], "11 bytes behind a declared 10");
    assert_eq!(e::block_writes(&cfg, false)[1].payload, vec![28u8, 21, 22, 20, 77, 84, 91, 98, 105, 19], "the v1 never sends glass mode");
    assert_eq!((w[2].command, w[2].declared_length, w[2].chunk), (0x16, 28, 1));
    assert_eq!(w[2].payload, (71u8..99).collect::<Vec<u8>>());
    assert_eq!((w[3].command, w[3].declared_length, w[3].chunk), (0x16, 28, 2));
    assert_eq!(w[3].payload, (99u8..127).collect::<Vec<u8>>());
    // sendCommand: [op, 0x0f, declared, 0, 0, chunk] after the report id, payload at wire byte 16
    let r = e::block_write_report(&w[3], 64);
    assert_eq!(&r[..8], &[0xA1, 0x16, 0x0f, 28, 0, 0, 2, 0]);
    assert_eq!(&r[16..44], &w[3].payload[..]);
}

// ======================================================================================== OP1: the shared 4K dongle

fn dongle_base() -> Vec<u8> {
    let mut c = base();
    c[25] = 2;
    c
}

#[test]
fn the_4k_dongle_is_named_by_the_mouse_behind_it() {
    // config[25] = 2: a v1 mouse reads it as the second of ["1 mm", "2 mm"] = 2 mm; a v2 mouse as the third of 0.7 ... 1.7 = 0.9 mm
    let v1 = Op1Mouse::new(dongle_base());
    v1.borrow_mut().paired = Some(0x1972);
    let (r, port) = read(0x1970, &v1);
    assert_eq!((r.online, r.model.as_deref(), r.lift_off, r.polling_hz, r.wireless), (true, Some("Endgame Gear OP1w 4K"), Some(20), Some(1000), Some(true)));
    assert_eq!(r.caps.polling, vec![1000, 2000, 4000]);
    // the requests: load, then mouse-info (op 0x0e): both reads
    let s = sends(&port);
    assert_eq!(s, vec![load_report(), { let mut v = vec![0u8; LEN]; v[0] = 0xA1; v[1] = 0x0e; v }]);
    assert!(s.iter().all(|o| e::read_request_allowed(o)));
    let v2 = Op1Mouse::new(dongle_base());
    v2.borrow_mut().paired = Some(0x1984);
    let (r, _) = read(0x1970, &v2);
    assert_eq!((r.model.as_deref(), r.lift_off, r.caps.polling.clone()), (Some("Endgame Gear OP1w 4K v2"), Some(9), vec![125, 1000, 2000, 4000]));
    let xm = Op1Mouse::new(dongle_base());
    xm.borrow_mut().paired = Some(0x1982);
    assert_eq!(read(0x1970, &xm).0.model.as_deref(), Some("Endgame Gear XM2w 4K v2"));
    xm.borrow_mut().paired = Some(0x1968);
    assert_eq!(read(0x1970, &xm).0.model.as_deref(), Some("Endgame Gear XM2w 4K"));
}

#[test]
fn the_4k_dongle_shows_no_lift_off_until_the_mouse_names_itself() {
    // mouse-info answers status 8 (the mouse does not relay): still online (the config read), but v1 or v2 is not known
    let m = Op1Mouse::new(dongle_base());
    let (r, _) = read(0x1970, &m);
    assert_eq!((r.online, r.model.clone(), r.lift_off, r.dpi, r.polling_hz), (true, None, None, Some(800), Some(1000)));
    assert_eq!(r.caps, Default::default(), "no rows: the dongle alone doesn't say what the mouse can do");
    // a write is refused (OpenMouse: "never write blind"), and nothing but the mouse-info request was sent
    let (w, port) = write(0x1970, &m, Setting::Polling(2000));
    assert!(matches!(w, Err(Error::BadAnswer(ref s)) if s.contains("identified")), "{w:?}");
    assert_eq!(sends(&port).len(), 1);
    assert_eq!(m.borrow().cfg, dongle_base());
    // a mouse-info answer that names no 4K mouse leaves the neutral profile: still no writes
    m.borrow_mut().paired = Some(0x1964);
    let (w, _) = write(0x1970, &m, Setting::Polling(2000));
    assert!(w.is_err());
}

#[test]
fn the_4k_dongle_writes_once_the_mouse_is_known() {
    // paired OP1w 4K v2: polling 4000 = divider 2 in the power block, same bytes as the cable id
    let b = dongle_base();
    let m = Op1Mouse::new(b.clone());
    {
        let mut s = m.borrow_mut();
        s.paired = Some(0x1984);
        s.glass_in_power = true;
    }
    let (r, port) = write(0x1970, &m, Setting::Polling(4000));
    assert_eq!(r, Ok(family::Changed { now: 4000, before: Some(1000) }));
    let power = [b[28], 2, b[22], b[20], b[77], b[84], b[91], b[98], b[105], b[19], b[127]];
    let s = sends(&port);
    // mouse-info, load, power block, load
    assert_eq!(s.len(), 4);
    assert_eq!(s[0][..2], [0xA1, 0x0e]);
    assert_eq!(s[2], hdr(0x15, 10, 0, &power));
}

// ======================================================================================== OT

struct OtMouse {
    eff: Vec<u8>,
    queue: VecDeque<Vec<u8>>,
    /// answers nothing at all
    deaf: bool,
    /// acknowledges a query but never sends the response
    ack_only: bool,
    /// the set packets seen
    sets: Vec<Vec<u8>>,
    /// (asked block, answered block): a query for the first is answered with the second's reply (a late / stale response)
    swap: Option<(u8, u8)>,
    /// the swap only applies once a setProfileData packet has been seen (it hits the read-back, not the first load)
    swap_after_sets: bool,
    /// the response comes with this comtype instead of 21
    response_comtype: u8,
}

const BLOCKS: [(u8, usize, usize); 5] = [(1, 50, 21), (2, 71, 32), (3, 103, 8), (4, 111, 1), (6, 112, 6)];

fn pkt(comtype: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 64];
    v[1] = comtype;
    v[3..3 + payload.len()].copy_from_slice(payload);
    v
}

impl OtMouse {
    fn new(eff: Vec<u8>) -> Rc<RefCell<OtMouse>> {
        Rc::new(RefCell::new(OtMouse { eff, queue: VecDeque::new(), deaf: false, ack_only: false, sets: Vec::new(), swap: None, swap_after_sets: false, response_comtype: 21 }))
    }

    fn set(&mut self, out: &[u8]) {
        if self.deaf {
            return;
        }
        match out[1] {
            51 => {
                let mut block = out[3];
                if let Some((asked, answered)) = self.swap {
                    if asked == block && (!self.swap_after_sets || !self.sets.is_empty()) {
                        block = answered;
                    }
                }
                self.queue.push_back(pkt(20, &[]));
                if !self.ack_only {
                    let payload = if block == 0 {
                        self.eff[..50].to_vec()
                    } else {
                        let (_, off, len) = *BLOCKS.iter().find(|b| b.0 == block).expect("a block the driver asks for");
                        let mut p = vec![block, self.eff[1], self.eff[2], self.eff[3]];
                        p.extend_from_slice(&self.eff[off..off + len]);
                        p
                    };
                    self.queue.push_back(pkt(self.response_comtype, &payload));
                }
            }
            52 => {
                self.sets.push(out[..64].to_vec());
                if out[3] == 0 {
                    self.eff[..50].copy_from_slice(&out[3..53]);
                } else {
                    let (_, off, len) = *BLOCKS.iter().find(|b| b.0 == out[3]).expect("a block the driver writes");
                    self.eff[off..off + len].copy_from_slice(&out[7..7 + len]);
                }
                self.queue.push_back(pkt(20, &[]));
            }
            other => panic!("the OT fake got comtype {other}, which no read or setting sends"),
        }
    }
}

fn ot_port(pid: u16, m: &Rc<RefCell<OtMouse>>, feature_len: u16) -> ScriptPort {
    let m = m.clone();
    ScriptPort::new(info(pid, feature_len), move |out, how| {
        let mut s = m.borrow_mut();
        if matches!(how, HidTransfer::FeatureOut | HidTransfer::Feature { .. }) {
            s.set(out);
        }
        match how {
            HidTransfer::Feature { reply_len, .. } | HidTransfer::FeatureIn { reply_len, .. } => {
                let mut a = s.queue.pop_front().unwrap_or_else(|| vec![0u8; 64]);
                // a 65-byte report (64 payload bytes): the id, then the whole packet with its own leading 0
                if *reply_len > 64 {
                    a.insert(0, 0);
                }
                a.resize(*reply_len, 0);
                Ok(a)
            }
            _ => Ok(Vec::new()),
        }
    })
}

/// An OT profile: stages 800 / 1600 / 3200 / 6400 (stage 2 active), 1000 Hz (index 0), lift-off raw 1; every other byte distinct.
fn ot_effect() -> Vec<u8> {
    let mut e = vec![0u8; 118];
    e[3] = 4;
    e[4..12].copy_from_slice(&[0x20, 0x03, 0x40, 0x06, 0x80, 0x0c, 0x00, 0x19]);
    e[12] = 0x48;
    e[16] = 1;
    e[18..21].copy_from_slice(&[0, 1, 2]);
    e[21..24].copy_from_slice(&[1, 1, 1]);
    e[27..39].copy_from_slice(&[0x0a, 0x14, 0x1e, 0x28, 0x0b, 0x15, 0x1f, 0x29, 0x0c, 0x16, 0x20, 0x2a]);
    e[39..46].copy_from_slice(&[0, 1, 2, 3, 4, 5, 6]);
    e[50] = 0x04;
    e[52] = 0x05;
    e[70] = 2;
    e[71..77].copy_from_slice(&[0x4f, 0, 0x50, 0, 0x31, 0]);
    e[103..111].copy_from_slice(&[0x20, 0x03, 0x40, 0x06, 0x80, 0x0c, 0x00, 0x19]);
    e[112..118].copy_from_slice(&[3, 0xff, 0x80, 0, 0x4d, 5]);
    e
}

/// The set packet of a block, by the layout in the header: `[0, 52, 0, block, effect[1..4), slice]` (main: `[0, 52, 0, effect[0..50)]`).
fn want_set(eff: &[u8], block: u8) -> Vec<u8> {
    let mut v = vec![0u8; 64];
    v[1] = 52;
    if block == 0 {
        v[3..53].copy_from_slice(&eff[..50]);
        return v;
    }
    let (_, off, len) = *BLOCKS.iter().find(|b| b.0 == block).unwrap();
    v[3] = block;
    v[4..7].copy_from_slice(&eff[1..4]);
    v[7..7 + len].copy_from_slice(&eff[off..off + len]);
    v
}

fn ot_read(pid: u16, m: &Rc<RefCell<OtMouse>>, opt: ReadOptions) -> (bu_mouse::family::OnMouse, ScriptPort) {
    let mut port = ot_port(pid, m, 64);
    let r = e::Endgame.read(&matched(pid, 64), &mut port, opt).unwrap();
    (r, port)
}

fn ot_write(pid: u16, m: &Rc<RefCell<OtMouse>>, s: Setting) -> (Result<family::Changed, Error>, ScriptPort) {
    let mut port = ot_port(pid, m, 64);
    let r = e::Endgame.write(&matched(pid, 64), &mut port, s);
    (r, port)
}

fn query(block: u8) -> Vec<u8> {
    let mut v = vec![0u8; 64];
    v[1] = 51;
    v[3] = block;
    v
}

#[test]
fn ot_read_decodes_like_openmouse() {
    let m = OtMouse::new(ot_effect());
    let (r, port) = ot_read(0x1906, &m, ReadOptions::default());
    // readStatus: dpi = cpi[cpiIndex] = cpi[1] = 0x0640 = 1600; polling index 0 = 1000 Hz; dcuThreshold 1 = "1 mm"
    assert_eq!((r.online, r.dpi, r.polling_hz, r.lift_off, r.model, r.wireless), (true, Some(1600), Some(1000), Some(10), None, Some(false)));
    // one query: the main block of profile 0
    assert_eq!(sends(&port), vec![query(0)]);
    assert!(port.sent.iter().all(|(o, _)| e::read_request_allowed(o)));
    // the caps: 50..10000 in 50s, 10100.. in 100s; XM1r to 19000, OP1 / OP1 RGB to 26000; 125 .. 1000 Hz; 1 / 2 mm; no battery
    assert_eq!(r.caps.dpi.as_ref().map(|d| d.bands.clone()), Some(vec![(50, 10_000, 50), (10_100, 26_000, 100)]));
    assert_eq!((r.caps.polling.clone(), r.caps.lift_off.clone(), r.caps.battery), (vec![125, 250, 500, 1000], vec![10, 20], false));
    let (x, _) = ot_read(0x1903, &m, ReadOptions::default());
    assert_eq!(x.caps.dpi.map(|d| d.max()), Some(19_000));
    // polling indexes 0..3 = 1000 / 500 / 250 / 125; lift-off raw 2 = 2 mm, anything else = unknown
    for (idx, hz) in [(0u8, 1000u32), (1, 500), (2, 250), (3, 125)] {
        let mut eff = ot_effect();
        eff[17] = idx;
        assert_eq!(e::ot_effect_polling(&eff), Some(hz), "index {idx}");
    }
    let mut eff = ot_effect();
    eff[23] = 2;
    assert_eq!(e::ot_effect_lift_off(&eff), Some(20));
    eff[23] = 3;
    assert_eq!(e::ot_effect_lift_off(&eff), None);
    // an active stage outside the four is no DPI
    eff[16] = 9;
    assert_eq!(e::ot_effect_dpi(&eff), None);
}

#[test]
fn ot_dpi_write_is_openmouses_set_cpi_stage_on_the_active_stage() {
    let before = ot_effect();
    let m = OtMouse::new(before.clone());
    let (r, port) = ot_write(0x1906, &m, Setting::Dpi(2400));
    assert_eq!(r, Ok(family::Changed { now: 2400, before: Some(1600) }));
    // active stage 2 (cpiIndex 1): cpi[1] at 6..8 and cpiY[1] at 105..107 = 2400 = 0x0960 -> 60 09
    let mut after = before.clone();
    after[6..8].copy_from_slice(&[0x60, 0x09]);
    after[105..107].copy_from_slice(&[0x60, 0x09]);
    assert_eq!(m.borrow().eff, after);
    // the requests: six queries (main first, then 1 2 3 4 6), six packets in vendor order, six queries again
    let s = sends(&port);
    let q: Vec<Vec<u8>> = [0u8, 1, 2, 3, 4, 6].iter().map(|b| query(*b)).collect();
    assert_eq!(s.len(), 18);
    assert_eq!(&s[..6], &q[..]);
    assert_eq!(&s[12..], &q[..]);
    let order: Vec<u8> = s[6..12].iter().map(|p| p[3]).collect();
    assert_eq!(order, vec![1, 2, 3, 4, 6, 0], "keyboard, name, cpi-Y, wireless, rgb, main");
    for (i, block) in [1u8, 2, 3, 4, 6, 0].iter().enumerate() {
        assert_eq!(s[6 + i], want_set(&after, *block), "block {block}");
    }
    // by hand: the main packet - report id 0, comtype 52, packet id 0, effect[0..50), zeros
    let mut main = vec![0x00, 0x34, 0x00];
    main.extend_from_slice(&[0x00, 0x00, 0x00, 0x04]); // 0..4
    main.extend_from_slice(&[0x20, 0x03, 0x60, 0x09, 0x80, 0x0c, 0x00, 0x19]); // CPI stages, stage 2 now 2400
    main.extend_from_slice(&[0x48, 0x00, 0x00, 0x00]); // profile flags
    main.extend_from_slice(&[0x01, 0x00, 0x00, 0x01, 0x02, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00]); // 16..27
    main.extend_from_slice(&[0x0a, 0x14, 0x1e, 0x28, 0x0b, 0x15, 0x1f, 0x29, 0x0c, 0x16, 0x20, 0x2a]); // LEDs
    main.extend_from_slice(&[0, 1, 2, 3, 4, 5, 6]); // button remap
    main.extend_from_slice(&[0, 0, 0, 0]); // 46..50
    main.resize(64, 0);
    assert_eq!(s[11], main);
    // and the cpi-Y block: block 3, effect[1..4) = 00 00 04, then the four Y values
    let mut y = vec![0x00, 0x34, 0x00, 0x03, 0x00, 0x00, 0x04, 0x20, 0x03, 0x60, 0x09, 0x80, 0x0c, 0x00, 0x19];
    y.resize(64, 0);
    assert_eq!(s[8], y);
    // the query goes out with SetFeature + GetFeature at once, the other reads only GET
    assert!(port.sent.iter().any(|(o, how)| o[1] == 51 && matches!(how, HidTransfer::Feature { reply_id: 0, reply_len: 64, .. })));
    assert!(port.sent.iter().any(|(_, how)| matches!(how, HidTransfer::FeatureIn { reply_id: 0, reply_len: 64, .. })));
    assert!(port.sent.iter().all(|(o, how)| o.len() == 64 && o[0] == 0 && (o[1] == 51 || o[1] == 52) && !matches!(how, HidTransfer::OutputOnly)));
}

#[test]
fn ot_dpi_write_refuses_a_split_stage_and_sends_nothing() {
    // the active stage (2) has X 1600 but Y 1200: OpenMouse's setCpiStage would set both to the new value; undo remembers X
    // only, so the write is refused before any packet goes out
    let mut eff = ot_effect();
    eff[105..107].copy_from_slice(&[0xB0, 0x04]);
    let m = OtMouse::new(eff.clone());
    let (r, port) = ot_write(0x1906, &m, Setting::Dpi(2400));
    assert!(matches!(r, Err(Error::OutOfRange { .. })), "{r:?}");
    assert_eq!(m.borrow().sets.len(), 0);
    assert_eq!(m.borrow().eff, eff);
    assert!(sends(&port).iter().all(|p| p[1] == 51), "only the six read queries went out");
    // a split on a stage that is NOT active does not matter: polling still writes, and the DPI write on the active stage works
    let mut eff = ot_effect();
    eff[107..109].copy_from_slice(&[0xB0, 0x04]);
    let m = OtMouse::new(eff);
    assert!(ot_write(0x1906, &m, Setting::Dpi(2400)).0.is_ok());
}

#[test]
fn ot_a_reply_to_another_block_is_never_spliced_into_the_profile() {
    // a stale response: the keyboard query (block 1) is answered with the name block's reply (block 2), or the main query
    // with the keyboard reply. OpenMouse would take either as the block asked for and write it back.
    for (asked, answered) in [(1u8, 2u8), (2, 1), (3, 4), (6, 3), (0, 1)] {
        let m = OtMouse::new(ot_effect());
        m.borrow_mut().swap = Some((asked, answered));
        let (r, port) = ot_write(0x1906, &m, Setting::Polling(500));
        assert!(matches!(r, Err(Error::BadAnswer(ref s)) if s.contains("did not answer")), "{asked}<-{answered}: {r:?}");
        assert_eq!(m.borrow().sets.len(), 0, "nothing was written");
        assert!(sends(&port).iter().all(|p| p[1] == 51));
        assert_eq!(m.borrow().eff, ot_effect());
        // and the DPI write refuses the same way
        assert!(ot_write(0x1906, &m, Setting::Dpi(2400)).0.is_err());
        assert_eq!(m.borrow().sets.len(), 0);
    }
    // a reply that is not a response packet (comtype 21) is no answer: a write errors, a read says offline
    let m = OtMouse::new(ot_effect());
    m.borrow_mut().response_comtype = 99;
    let (r, _) = ot_write(0x1906, &m, Setting::LiftOff(20));
    assert!(matches!(r, Err(Error::BadAnswer(_))));
    assert_eq!(m.borrow().sets.len(), 0);
    assert!(!ot_read(0x1906, &m, ONCE).0.online);
    // the read-back after the six packets is checked too: a write whose confirmation is another block's reply is an error
    let m = OtMouse::new(ot_effect());
    {
        let mut s = m.borrow_mut();
        s.swap = Some((2, 1));
        s.swap_after_sets = true;
    }
    let (r, _) = ot_write(0x1906, &m, Setting::Polling(500));
    assert!(matches!(r, Err(Error::BadAnswer(_))), "{r:?}");
}

#[test]
fn ot_polling_and_lift_off_writes_are_openmouses() {
    // setPollingRate(500): OT_POLL_RATE_HZ index 1 -> effect[17] = 1
    let m = OtMouse::new(ot_effect());
    let (r, port) = ot_write(0x1905, &m, Setting::Polling(500));
    assert_eq!(r, Ok(family::Changed { now: 500, before: Some(1000) }));
    let mut want = ot_effect();
    want[17] = 1;
    assert_eq!(m.borrow().eff, want);
    assert_eq!(sends(&port)[11], want_set(&want, 0));
    assert_eq!(sends(&port)[11][3 + 17], 1);
    // 125 Hz = index 3, 250 Hz = index 2
    let m = OtMouse::new(ot_effect());
    assert!(ot_write(0x1905, &m, Setting::Polling(125)).0.is_ok());
    assert_eq!(m.borrow().eff[17], 3);
    let m = OtMouse::new(ot_effect());
    assert!(ot_write(0x1905, &m, Setting::Polling(250)).0.is_ok());
    assert_eq!(m.borrow().eff[17], 2);
    assert!(matches!(ot_write(0x1905, &m, Setting::Polling(2000)).0, Err(Error::OutOfRange { .. })));
    // setLiftOffDistance("High"): raw 2 at effect[23]; 1 mm = raw 1; nothing else exists
    let m = OtMouse::new(ot_effect());
    let (r, _) = ot_write(0x1906, &m, Setting::LiftOff(20));
    assert_eq!(r, Ok(family::Changed { now: 20, before: Some(10) }));
    let mut want = ot_effect();
    want[23] = 2;
    assert_eq!(m.borrow().eff, want);
    assert!(matches!(ot_write(0x1906, &m, Setting::LiftOff(7)).0, Err(Error::OutOfRange { .. })));
}

#[test]
fn ot_every_byte_not_changed_is_sent_back_as_read() {
    // a profile with a name followed by a stray byte after its terminator: OpenMouse's re-encode would zero it, this port
    // keeps it (documented in the file header); everything else is identical
    let mut eff = ot_effect();
    eff[79] = 0xAA;
    let m = OtMouse::new(eff.clone());
    let (r, port) = ot_write(0x1906, &m, Setting::Polling(250));
    assert!(r.is_ok());
    let mut after = eff.clone();
    after[17] = 2;
    assert_eq!(m.borrow().eff, after);
    assert_eq!(sends(&port)[7], want_set(&after, 2), "the name block still carries the byte");
    assert_eq!(m.borrow().sets.len(), 6);
}

#[test]
fn ot_cpi_rules_are_openmouses() {
    // otIsValidCpi: 50 steps up to 10000, 100 steps above, to the model's maximum
    assert!(e::ot_is_valid_cpi(26_000, 50) && e::ot_is_valid_cpi(26_000, 10_000) && e::ot_is_valid_cpi(26_000, 10_100) && e::ot_is_valid_cpi(26_000, 26_000));
    assert!(!e::ot_is_valid_cpi(26_000, 10_050) && !e::ot_is_valid_cpi(26_000, 10_001) && !e::ot_is_valid_cpi(26_000, 26_100) && !e::ot_is_valid_cpi(26_000, 1625) && !e::ot_is_valid_cpi(26_000, 25));
    assert!(e::ot_is_valid_cpi(19_000, 19_000) && !e::ot_is_valid_cpi(19_000, 19_100));
    let m = OtMouse::new(ot_effect());
    for bad in [10_050u32, 19_100, 1625] {
        let (r, _) = ot_write(0x1903, &m, Setting::Dpi(bad));
        assert!(matches!(r, Err(Error::OutOfRange { .. })), "{bad}");
    }
    assert_eq!(m.borrow().sets.len(), 0, "nothing was written");
    // 20000 is fine on the OP1, not on the XM1r
    assert!(ot_write(0x1906, &m, Setting::Dpi(20_000)).0.is_ok());
    assert!(matches!(ot_write(0x1903, &m, Setting::Dpi(20_000)).0, Err(Error::OutOfRange { .. })));
}

#[test]
fn ot_silent_mouse_reads_as_offline() {
    let m = OtMouse::new(ot_effect());
    m.borrow_mut().deaf = true;
    // asking once: one send, ten GETs (OT_READ_RETRIES)
    let (r, port) = ot_read(0x1906, &m, ONCE);
    assert!(!r.online && r.dpi.is_none());
    assert_eq!((sends(&port).len(), gets(&port)), (1, 10));
    assert!(r.caps.dpi.is_some());
    // OpenMouse's budget: ten sends (OT_WRITE_RETRIES)
    let (r, port) = ot_read(0x1906, &m, ReadOptions::default());
    assert!(!r.online);
    assert_eq!((sends(&port).len(), gets(&port)), (10, 100));
    // an ACK without the response behind it is no answer either
    let m = OtMouse::new(ot_effect());
    m.borrow_mut().ack_only = true;
    assert!(!ot_read(0x1906, &m, ONCE).0.online);
    // a write to a silent mouse is an error
    let (w, _) = ot_write(0x1906, &m, Setting::Polling(500));
    assert!(matches!(w, Err(Error::BadAnswer(_))));
}

#[test]
fn ot_mouse_that_never_acknowledges_a_packet_is_an_error() {
    // reads fine, but a setProfileData is never answered: sendPacket gives up after OT_WRITE_RETRIES sends
    let m = OtMouse::new(ot_effect());
    let mm = m.clone();
    let mut port = ScriptPort::new(info(0x1906, 64), move |out, how| {
        let mut s = mm.borrow_mut();
        if matches!(how, HidTransfer::FeatureOut | HidTransfer::Feature { .. }) && out[1] == 51 {
            s.set(out);
        }
        match how {
            HidTransfer::Feature { reply_len, .. } | HidTransfer::FeatureIn { reply_len, .. } => {
                let mut a = s.queue.pop_front().unwrap_or_else(|| vec![0u8; 64]);
                a.resize(*reply_len, 0);
                Ok(a)
            }
            _ => Ok(Vec::new()),
        }
    });
    let r = e::Endgame.write(&matched(0x1906, 64), &mut port, Setting::Polling(500));
    assert!(matches!(r, Err(Error::BadAnswer(_))), "{r:?}");
    // 6 queries, then ten sends of the first packet (10 GETs shared by all of them: only the first send reads)
    let s = sends(&port);
    assert_eq!(s.iter().filter(|o| o[1] == 52).count(), 10);
    assert_eq!(m.borrow().eff, ot_effect(), "nothing changed on the mouse");
}

#[test]
fn ot_feature_replies_normalise_like_openmouse() {
    // otNormalizeFeatureReport: 63 bytes (no report id) get a 0 in front; 64 with a leading 0 stay; 65 with a leading 0 lose it
    let raw63: Vec<u8> = (1..64).collect();
    let n = e::ot_normalize(&raw63);
    assert_eq!((n.len(), n[0], n[1], n[63]), (64, 0, 1, 63));
    let mut raw64 = vec![0u8; 64];
    raw64[1] = 21;
    assert_eq!(e::ot_normalize(&raw64), raw64);
    let mut raw65 = vec![0u8; 65];
    raw65[2] = 20;
    assert_eq!(e::ot_normalize(&raw65)[..4], [0, 20, 0, 0], "the leading 0 is dropped: byte 2 moves to 1");
    // a longer feature report (65 bytes on Windows): the packets still carry report id 0 and are padded
    let m = OtMouse::new(ot_effect());
    let mut port = ot_port(0x1906, &m, 65);
    let r = e::Endgame.read(&matched(0x1906, 65), &mut port, ONCE).unwrap();
    assert_eq!(r.dpi, Some(1600));
    assert_eq!(sends(&port)[0].len(), 65);
}

// ======================================================================================== allow-list

#[test]
fn only_read_requests_pass_the_allow_list() {
    let pad = |b: &[u8], n: usize| {
        let mut v = b.to_vec();
        v.resize(n, 0);
        v
    };
    // OP1 reads: load (0x12) and mouse-info (0x0e) on report 0xA1
    assert!(e::read_request_allowed(&pad(&[0xA1, 0x12], LEN)));
    assert!(e::read_request_allowed(&pad(&[0xA1, 0x0e], 64)));
    // OT reads: queryProfileData (51) of block 0 / 1 / 2 / 3 / 4 / 6 of profile 0
    for b in [0u8, 1, 2, 3, 4, 6] {
        assert!(e::read_request_allowed(&pad(&[0, 51, 0, b], 64)), "block {b}");
    }
    // everything that writes or changes the mouse
    for w in [
        pad(&[0xA0, 0x11], LEN),                  // whole-config store
        pad(&[0xA1, 0x11], LEN),                  // store as a command
        pad(&[0xA1, 0x13], LEN),                  // factory reset
        pad(&[0xA1, 0x14, 0x0f, 28], LEN),        // sensor block
        pad(&[0xA1, 0x15, 0x0f, 10], LEN),        // power block
        pad(&[0xA1, 0x16, 0x0f, 28], LEN),        // button block
        pad(&[0xA1, 0x02], LEN),                  // firmware version (not sent by this port)
        pad(&[0xA1, 0x12, 0, 0, 0, 0, 1], LEN),   // a load with something set behind it
        pad(&[0, 52, 0, 0], 64),                  // setProfileData
        pad(&[0, 18, 0, 1, 0x0b, 0xad, 0xf0, 0x0d], 64), // resetFirmware
        pad(&[0, 8, 0, 1], 64),                   // firmwareDlSetup
        pad(&[0, 9, 0], 64),                      // firmwareDataPacket
        pad(&[0, 37, 0], 64),                     // resetMouse
        pad(&[0, 51, 0, 5], 64),                  // a block this port never asks for
        pad(&[0, 51, 0, 0, 1], 64),               // another profile
        pad(&[0, 51, 0, 0, 0, 7], 64),            // something set behind a query
        pad(&[0x08, 0x07], 17),                   // the WE series' WriteEEPROM
        vec![],
    ] {
        assert!(!e::read_request_allowed(&w), "{w:02x?}");
    }
}

#[test]
fn a_write_sends_nothing_outside_its_own_known_requests() {
    // every request of a whole OP1 8K change is one of: the load (allowed), the store (refused by the allow-list)
    let m = Op1Mouse::new(base());
    let (r, port) = write(0x1964, &m, Setting::Dpi(1600));
    assert!(r.is_ok());
    let kinds: Vec<(u8, u8, bool)> = sends(&port).iter().map(|o| (o[0], o[1], e::read_request_allowed(o))).collect();
    assert_eq!(kinds, vec![(0xA1, 0x12, true), (0xA0, 0x11, false), (0xA1, 0x12, true)]);
    // an OT change: only query 51 (allowed) and set 52 (refused)
    let m = OtMouse::new(ot_effect());
    let (r, port) = ot_write(0x1906, &m, Setting::Polling(500));
    assert!(r.is_ok());
    for o in sends(&port) {
        assert_eq!(e::read_request_allowed(&o), o[1] == 51);
        assert!(o[1] == 51 || o[1] == 52);
    }
}

// ======================================================================================== the device table

#[test]
fn every_supported_model_matches_with_its_vendor_interface() {
    // OpenMouse's SUPPORTED list, status "supported", Endgame Gear: 14 models. 12 are served; the OP1we and XM2we are the held-back WE series.
    let table: [(u16, Option<&str>, bool); 13] = [
        (0x1906, Some("Endgame Gear OP1"), false),
        (0x1905, Some("Endgame Gear OP1 RGB"), false),
        (0x1903, Some("Endgame Gear XM1r"), false),
        (0x1964, Some("Endgame Gear OP1 8K"), false),
        (0x1976, Some("Endgame Gear OP1 8K Purple Frost"), false),
        (0x1978, Some("Endgame Gear OP1 8K v2"), false),
        (0x1966, Some("Endgame Gear XM2 8K"), false),
        (0x1980, Some("Endgame Gear XM2 8K v2"), false),
        (0x1972, Some("Endgame Gear OP1w 4K"), false),
        (0x1968, Some("Endgame Gear XM2w 4K"), false),
        (0x1984, Some("Endgame Gear OP1w 4K v2"), false),
        (0x1982, Some("Endgame Gear XM2w 4K v2"), false),
        (0x1970, None, true),
    ];
    assert_eq!(e::PRODUCTS.len(), table.len());
    for (pid, model, wireless) in table {
        let feat = if matches!(pid, 0x1906 | 0x1905 | 0x1903) { 64 } else { 1041 };
        // the mouse's own collections (usage page 1, no feature report) next to the vendor one
        let mouse = hid_info("mouse", VID, pid, 0x01, 0x02, (8, 0, 0));
        let vendor = hid_info("vendor", VID, pid, 0xFF00, 0x01, (64, 64, feat));
        let m = e::matches(&[&mouse, &vendor], VID, pid).unwrap_or_else(|| panic!("{pid:#06x}"));
        assert_eq!((m.family, m.model, m.wireless, m.variant), (Family::Endgame, model, Some(wireless), pid), "{pid:#06x}");
        assert_eq!(m.config.path, "vendor", "{pid:#06x}: the vendor interface, not the mouse collection");
        // the registry sees the same
        let found = family::find(&[mouse.clone(), vendor.clone()], VID, pid).expect("family::find");
        assert_eq!((found.family, found.config.path.as_str()), (Family::Endgame, "vendor"));
        // a wrong vendor id, a device with no long-enough feature report, a collection list without the vendor one
        assert!(e::matches(&[&vendor], 0x1532, pid).is_none());
        assert!(e::matches(&[&mouse], VID, pid).is_none(), "{pid:#06x}: no vendor interface");
        let short = hid_info("short", VID, pid, 0xFF02, 1, (17, 17, 17));
        assert!(e::matches(&[&short], VID, pid).is_none(), "{pid:#06x}: a 16-byte WE feature report is not the OP1 / OT interface");
    }
    // an id that is not in the table
    let v = info(0x1964, 1041);
    assert!(e::matches(&[&v], VID, 0x1965).is_none());
    assert!(e::matches(&[&v], VID, 0x0000).is_none());
}

#[test]
fn the_held_back_we_series_matches_nothing() {
    // OP1we 0x1961 / 0x1962 and XM2we 0x1960: NEEDS TRANSFER (a feature SET on one collection, the answer as an INPUT report on another)
    for pid in [0x1961u16, 0x1962, 0x1960] {
        for feat in [17u16, 64, 1041] {
            let i = info(pid, feat);
            assert!(e::matches(&[&i], VID, pid).is_none(), "{pid:#06x} / {feat}");
            assert!(family::find(&[i], VID, pid).is_none());
        }
    }
    // the OT interface length is not enough for an OP1 id (and the other way round the OP1 one would do)
    let i = info(0x1964, 64);
    assert!(e::matches(&[&i], VID, 0x1964).is_none());
    assert!(e::matches(&[&info(0x1906, 1041)], VID, 0x1906).is_some());
    // the vendor usage page is preferred when two collections are long enough
    let a = hid_info("generic", VID, 0x1964, 0x01, 0x06, (8, 0, 1041));
    let b = hid_info("vendor", VID, 0x1964, 0xFF01, 0x01, (64, 64, 1041));
    assert_eq!(e::matches(&[&a, &b], VID, 0x1964).map(|m| m.config.path), Some("vendor".into()));
}

#[test]
fn endgame_ids_belong_to_no_other_family() {
    use bu_mouse::{asus, atk, attackshark, corsair, dareu, delux, finalmouse, gearhub, glorious, incott, keychron, ksnake, lamzu, logitech, mchose, microsoft, moddo, ninjutso, orbital, pulsar, rapoo, rawm, razer, redragon, ryunix, steelseries, teevolution, vaxee, vgn, wallhack, wlmouse, zaunkoenig};
    type Matcher = fn(&[&HidInfo], u16, u16) -> Option<Matched>;
    let others: Vec<(&str, Matcher)> = vec![
        ("pulsar cmouse", pulsar::matches_cmouse),
        ("pulsar xs1", pulsar::matches_xs1),
        ("logitech", logitech::matches),
        ("razer", razer::matches),
        ("steelseries", steelseries::matches),
        ("atk", atk::matches),
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
        ("ksnake", ksnake::matches),
        ("microsoft", microsoft::matches),
        ("moddo", moddo::matches),
        ("teevolution", teevolution::matches),
        ("wallhack", wallhack::matches),
        ("vgn", vgn::matches),
        ("asus", asus::matches),
        ("ryunix", ryunix::matches),
        ("rapoo", rapoo::matches),
    ];
    for p in e::PRODUCTS {
        let i = hid_info("vendor", VID, p.pid, 0xFF00, 1, (64, 64, 1041));
        for (name, f) in &others {
            assert!(f(&[&i], VID, p.pid).is_none(), "{name} claims Endgame Gear {:#06x}", p.pid);
        }
    }
    // and Endgame claims nothing of the others' vendors
    for (vid, pid) in [(0x1532u16, 0x00c0u16), (0x046d, 0xc08b), (0x1038, 0x1850), (0x3710, 0x5406)] {
        let i = hid_info("vendor", vid, pid, 0xFF00, 1, (64, 64, 1041));
        assert!(e::matches(&[&i], vid, pid).is_none());
    }
}

// ======================================================================================== Order 110: Motion Sync, angle snapping, ripple

use bu_mouse::family::{Change, Extra};

fn extras(r: &bu_mouse::family::OnMouse) -> Vec<(Extra, Option<i32>)> {
    r.extras.iter().map(|x| (x.extra, x.value)).collect()
}

fn change_op1(pid: u16, m: &Rc<RefCell<Op1Mouse>>, c: Change) -> (Result<(), Error>, ScriptPort) {
    let mut port = op1_port(pid, m);
    let r = e::Endgame.change(&matched(pid, LEN as u16), &mut port, c);
    (r, port)
}

fn change_ot(pid: u16, m: &Rc<RefCell<OtMouse>>, c: Change) -> (Result<(), Error>, ScriptPort) {
    let mut port = ot_port(pid, m, 64);
    let r = e::Endgame.change(&matched(pid, 64), &mut port, c);
    (r, port)
}

#[test]
fn order_110_op1_read_shows_motion_sync_angle_snapping_and_ripple() {
    // readStatus: motionSync = config[28] !== 0, angleSnapping = config[26] !== 0, rippleControl = config[27] !== 0
    let mut c = base();
    c[26] = 1;
    c[27] = 0;
    c[28] = 1;
    let (r, port) = read(0x1978, &Op1Mouse::new(c));
    assert_eq!(extras(&r), vec![(Extra::MotionSync, Some(1)), (Extra::AngleSnap, Some(1)), (Extra::RippleControl, Some(0))]);
    assert_eq!(sends(&port), vec![load_report()], "the same config read, nothing more");
    assert!(port.sent.iter().all(|(o, _)| e::read_request_allowed(o)));
}

#[test]
fn order_110_op1_changes_are_openmouses_setboolean_bytes() {
    // 8K: writeConfigRaw [0xA0, 0x11, config[2..]] with the one byte changed, every other byte as read
    for (c, offset, value) in [(Change::Extra(Extra::MotionSync, 1), 28usize, 1u8), (Change::Extra(Extra::AngleSnap, 0), 26, 0), (Change::Extra(Extra::RippleControl, 1), 27, 1)] {
        let mut b = base();
        b[26] = 1;
        b[27] = 0;
        let m = Op1Mouse::new(b.clone());
        let (r, port) = change_op1(0x1978, &m, c);
        r.unwrap_or_else(|x| panic!("{c:?}: {x}"));
        let mut want = b.clone();
        want[1] = 0x11;
        want[offset] = value;
        let s = sends(&port);
        assert_eq!(s.len(), 3, "{c:?}: load, store, load");
        assert_eq!((&s[0], &s[1], &s[2]), (&load_report(), &want, &load_report()), "{c:?}");
        assert_eq!(m.borrow().cfg[offset], value);
    }
    // 4K v2: Motion Sync (config[28] = blob 0x0c) is the first byte of the power block (op 0x15, declared 10, chunk 0); angle
    // snapping / ripple (config[26] / [27] = blob 0x0a / 0x0b) are bytes 3 / 4 of the sensor block (op 0x14, declared 28)
    let b = base();
    let m = Op1Mouse::new(b.clone());
    m.borrow_mut().glass_in_power = true;
    let (r, port) = change_op1(0x1984, &m, Change::Extra(Extra::MotionSync, 1));
    r.unwrap();
    let power = [1, b[21], b[22], b[20], b[77], b[84], b[91], b[98], b[105], b[19], b[127]];
    let s = sends(&port);
    assert_eq!(s.len(), 3, "load, power block, load");
    assert_eq!(s[1], hdr(0x15, 10, 0, &power));
    let m = Op1Mouse::new(b.clone());
    let (r, port) = change_op1(0x1984, &m, Change::Extra(Extra::RippleControl, 0));
    r.unwrap();
    let mut sensor = vec![b[23], b[24], b[25], b[26], 0, b[17], b[30], b[29]];
    sensor.extend_from_slice(&b[51..71]);
    assert_eq!(sends(&port)[1], hdr(0x14, 28, 0, &sensor));
    // a mouse that keeps the old value is an error
    let m = Op1Mouse::new(base());
    m.borrow_mut().ignore_store = true;
    assert!(matches!(change_op1(0x1978, &m, Change::Extra(Extra::MotionSync, 1)).0, Err(Error::BadAnswer(_))));
}

#[test]
fn order_110_op1_refuses_what_openmouse_does_not_take_before_a_byte_leaves() {
    // OP1 8K (no Motion Sync at 8 kHz) running at 8000 Hz: setMotionSync(true) throws before writing
    let mut b = base();
    b[21] = 1;
    let m = Op1Mouse::new(b);
    let (r, port) = change_op1(0x1964, &m, Change::Extra(Extra::MotionSync, 1));
    assert!(matches!(r, Err(Error::OutOfRange { .. })));
    assert!(sends(&port).iter().all(|o| e::read_request_allowed(o)) && m.borrow().stores == 0);
    // switches take 0 / 1; nothing else is offered (no debounce setter in OpenMouse's OP1 client)
    let m = Op1Mouse::new(base());
    for c in [Change::Extra(Extra::MotionSync, 2), Change::Extra(Extra::Debounce, 4), Change::Extra(Extra::Turbo, 1)] {
        let (r, port) = change_op1(0x1978, &m, c);
        assert!(r.is_err(), "{c:?}");
        assert!(port.sent.is_empty(), "{c:?}: nothing sent");
    }
}

#[test]
fn order_110_ot_reads_and_writes_the_profile_flags_like_openmouse() {
    // profileFlags (u32 LE at 12): angleSnapping = 1 << 3, rippleControl = 1 << 6; ot_effect() holds 0x48 = both on
    let m = OtMouse::new(ot_effect());
    let (r, port) = ot_read(0x1906, &m, ReadOptions::default());
    assert_eq!(extras(&r), vec![(Extra::AngleSnap, Some(1)), (Extra::RippleControl, Some(1))]);
    assert_eq!(sends(&port), vec![query(0)]);
    // setAngleSnapping(false): flags 0x48 & !0x08 = 0x40; the main block (packet byte 3 + 12) carries it, everything else as read
    let (r, port) = change_ot(0x1906, &m, Change::Extra(Extra::AngleSnap, 0));
    r.unwrap();
    let mut want = ot_effect();
    want[12] = 0x40;
    assert_eq!(m.borrow().eff, want);
    assert_eq!(sends(&port)[11], want_set(&want, 0));
    assert_eq!(sends(&port)[11][3 + 12], 0x40);
    // setRippleControl(false) then (true): 0x40 -> 0x00 -> 0x40
    change_ot(0x1906, &m, Change::Extra(Extra::RippleControl, 0)).0.unwrap();
    assert_eq!(m.borrow().eff[12], 0x00);
    change_ot(0x1906, &m, Change::Extra(Extra::RippleControl, 1)).0.unwrap();
    assert_eq!(m.borrow().eff[12], 0x40);
    // refused before a byte leaves: Motion Sync (two OT flags, no plain switch), anything else, a value a switch does not take
    for c in [Change::Extra(Extra::MotionSync, 1), Change::Extra(Extra::AngleSnap, 2), Change::Extra(Extra::Debounce, 2), Change::Extra(Extra::Turbo, 1)] {
        let (r, port) = change_ot(0x1906, &m, c);
        assert!(r.is_err(), "{c:?}");
        assert!(port.sent.is_empty(), "{c:?}");
    }
}

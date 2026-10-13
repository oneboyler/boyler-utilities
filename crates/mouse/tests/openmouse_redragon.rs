//! Order 105 check, Redragon: our frames = the bytes OpenMouse sends for the same setting (encode), and answers shaped the way
//! OpenMouse reads them decode to the same values (decode). The expected bytes are worked out by hand from OpenMouse
//! (@openmouse/protocol 0.40.1: `dist/redragon/index.js`, `dist/drivers/redragon/hid.js`), never from our own encoder.
//!
//! OpenMouse's frame is 16 bytes WITH the report id 2 in front (it sends `frame.slice(1)` as report 2): our byte i = its byte i.
//! Nothing here ran on a real mouse: the fake mouse below is a model of what OpenMouse's code expects.

use bu_mouse::error::Error;
use bu_mouse::family::{Driver, Family, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::os::{HidInfo, HidTransfer};
use bu_mouse::redragon as r;
use std::cell::RefCell;
use std::rc::Rc;

/// A frame the way OpenMouse builds it: `02` then the bytes given, zero-padded to 16.
fn fr(bytes: &[u8]) -> Vec<u8> {
    let mut v = vec![2u8];
    v.extend_from_slice(bytes);
    v.resize(16, 0);
    v
}

fn hello() -> Vec<u8> {
    fr(&[0xf5, 0x00])
}
fn close() -> Vec<u8> {
    fr(&[0xf5, 0x01])
}
fn commit(code: u8) -> Vec<u8> {
    fr(&[0xf1, 0x02, code])
}
fn commits() -> Vec<Vec<u8>> {
    [0x04, 0x01, 0x02, 0x08, 0x10].iter().map(|c| commit(*c)).collect()
}

// ---------------------------------------------------------------- encode

#[test]
fn session_and_commit_frames_are_openmouses() {
    // redragonSession(true): frame[1] = 0xF5 (REDRAGON_CMD_HELLO), frame[2] = 0
    assert_eq!(r::session_frame(true).to_vec(), fr(&[0xf5, 0x00]));
    // redragonSession(false): frame[2] = 1
    assert_eq!(r::session_frame(false).to_vec(), fr(&[0xf5, 0x01]));
    // redragonEncodeCommit(code): frame[1] = 0xF1, frame[2] = 2 (COMMIT_SECTION), frame[3] = code
    for code in [0x04u8, 0x01, 0x02, 0x08, 0x10] {
        assert_eq!(r::commit_frame(code).to_vec(), fr(&[0xf1, 0x02, code]));
    }
    // REDRAGON_COMMIT_CODES, in order
    assert_eq!(r::COMMIT_CODES, [0x04, 0x01, 0x02, 0x08, 0x10]);
}

#[test]
fn read_and_write_frames_are_openmouses() {
    // redragonEncodeRead(0x002c, 1): [02, F2, addr & 0xff = 2c, addr >> 8 = 00, len = 01]
    assert_eq!(r::read_frame(0x002c, 1).unwrap().to_vec(), fr(&[0xf2, 0x2c, 0x00, 0x01]));
    // redragonEncodeRead(0x0262, 5): [02, F2, 62, 02, 05]
    assert_eq!(r::read_frame(0x0262, 5).unwrap().to_vec(), fr(&[0xf2, 0x62, 0x02, 0x05]));
    // redragonEncodeWrite(0x0032, [02 00 02 00 02 00]): [02, F3, 32, 00, len 06, 00 00 00, data from byte 8]
    assert_eq!(r::write_frame(0x0032, &[2, 0, 2, 0, 2, 0]).unwrap().to_vec(), fr(&[0xf3, 0x32, 0x00, 0x06, 0, 0, 0, 2, 0, 2, 0, 2, 0]));
    // redragonEncodeWrite(0x0110, [01 6a 00 6a 00]): [02, F3, 0e, 01, 05, 00 00 00, 01 6a 00 6a 00]
    assert_eq!(r::write_frame(0x0110, &[1, 0x6a, 0, 0x6a, 0]).unwrap().to_vec(), fr(&[0xf3, 0x10, 0x01, 0x05, 0, 0, 0, 1, 0x6a, 0, 0x6a, 0]));
    // checkTransfer: 1-8 bytes only
    assert!(r::read_frame(0x0032, 0).is_err() && r::read_frame(0x0032, 9).is_err());
    assert!(r::write_frame(0x0032, &[]).is_err() && r::write_frame(0x0032, &[0; 9]).is_err());
    assert!(r::write_frame(0x0032, &[0; 8]).is_ok());
}

#[test]
fn m724_polling_frame_is_openmouses() {
    // redragonEncodePollingRate: [02, F3, sub 32, profile 00, section 06, 00 00 00, code at 8, 00, 01 at 10, 00, 01 at 12]
    // code = 1000 / Hz: 1000 -> 01, 500 -> 02, 250 -> 04, 125 -> 08
    for (hz, code) in [(1000, 0x01), (500, 0x02), (250, 0x04), (125, 0x08)] {
        assert_eq!(r::m724_polling_frame(hz).unwrap().to_vec(), fr(&[0xf3, 0x32, 0x00, 0x06, 0, 0, 0, code, 0x00, 0x01, 0x00, 0x01]), "{hz}");
        assert_eq!((r::polling_code(hz), r::polling_hz(code)), (Some(code), Some(hz)));
    }
    assert!(r::m724_polling_frame(2000).is_none() && r::m724_polling_frame(300).is_none());
    assert_eq!((r::polling_code(0), r::polling_hz(0x03)), (None, None));
}

#[test]
fn m612_dpi_codes_are_openmouses() {
    // redragonM612DpiCodeAt: position p < 93: value 14 + p, range 0; p >= 93: value p - 39, range 1
    // 1200 is label 18 (labels: 500 570 600 640 680 700 760 800 | 830 870 900 950 980 1000 1060 1100 | 1140 1170 1200):
    // value 14 + 18 = 32 = 0x20
    assert_eq!(r::m612_encode_dpi(1200), Some((1200, 0x20, 0)));
    // 500 is position 0: value 14 = 0x0e
    assert_eq!(r::m612_encode_dpi(500), Some((500, 0x0e, 0)));
    // 1000 is position 13 (the factory push "13"): value 27 = 0x1b
    assert_eq!(r::m612_encode_dpi(1000), Some((1000, 0x1b, 0)));
    // 4000 is position 92 (the factory push "92", last of range 0): value 14 + 92 = 106 = 0x6a
    assert_eq!(r::m612_encode_dpi(4000), Some((4000, 0x6a, 0)));
    // 4100 is position 93, the first of range 1: value 93 - 39 = 54 = 0x36
    assert_eq!(r::m612_encode_dpi(4100), Some((4100, 0x36, 1)));
    // 8000 is position 145, the last: value 145 - 39 = 106 = 0x6a, range 1
    assert_eq!(r::m612_encode_dpi(8000), Some((8000, 0x6a, 1)));
    // the table has 146 labels: 18 rows of 8 + 2
    assert_eq!(r::M612_DPI_LABELS.len(), 146);
    assert_eq!((r::M612_DPI_LABELS[0], r::M612_DPI_LABELS[92], r::M612_DPI_LABELS[93], r::M612_DPI_LABELS[145]), (500, 4000, 4100, 8000));
    // redragonM612EncodeDpi snaps to the nearest label, a tie going to the LOWER one (strict <, first wins)
    assert_eq!(r::m612_encode_dpi(1234).map(|c| c.0), Some(1250), "1250 is 16 away, 1200 is 34");
    assert_eq!(r::m612_encode_dpi(585).map(|c| c.0), Some(570), "570 and 600 are both 15 away: the lower");
    assert_eq!(r::m612_encode_dpi(4050).map(|c| c.0), Some(4000), "4000 and 4100 are both 50 away: the lower");
    // outside 500..8000 is refused (OpenMouse throws before sending)
    assert_eq!((r::m612_encode_dpi(499), r::m612_encode_dpi(8001), r::m612_encode_dpi(0)), (None, None, None));
    // every label survives encode -> decode
    for (p, label) in r::M612_DPI_LABELS.iter().enumerate() {
        let (v, range) = r::m612_code_at(p).unwrap();
        assert_eq!(r::m612_decode_dpi(v, range), *label, "position {p}");
        assert_eq!(r::m612_encode_dpi(*label), Some((*label, v, range)));
    }
    assert_eq!(r::m612_code_at(146), None);
}

#[test]
fn m612_dpi_decoding_is_openmouses() {
    // redragonM612DecodeDpi: range 0, value 32 -> position 18 -> label 1200
    assert_eq!(r::m612_decode_dpi(32, 0), 1200);
    // range 1, value 54 -> position 93 -> 4100
    assert_eq!(r::m612_decode_dpi(54, 1), 4100);
    // a code RDCfg never writes falls back to 4000 DPI per 106 counts, rounded to 10:
    // range 0 value 120: position 106 is not < 93 -> linear: 120 * 4000 / 106 = 4528.3 -> /10 = 452.8 -> 453 -> 4530
    assert_eq!(r::m612_decode_dpi(120, 0), 4530);
    // range 1 value 5: position 44 < 93 is out of range 1 -> 5 * 2 * 4000 / 106 = 377.36 -> 37.7 -> 38 -> 380
    assert_eq!(r::m612_decode_dpi(5, 1), 380);
    // range 2 is no range: value 10 -> 10 * 4000 / 106 = 377.36 -> 380
    assert_eq!(r::m612_decode_dpi(10, 2), 380);
}

#[test]
fn m612_slot_addresses_are_openmouses() {
    // slot n of a profile: base + 2 + 6n; bases 0x42, 0x102, 0x1b2, 0x262, 0x312
    assert_eq!(r::m612_slot_address(0, 0), Some(0x0044));
    assert_eq!(r::m612_slot_address(0, 4), Some(0x0042 + 2 + 24));
    assert_eq!(r::m612_slot_address(1, 2), Some(0x0102 + 2 + 12));
    assert_eq!(r::m612_slot_address(4, 4), Some(0x0312 + 2 + 24));
    assert_eq!((r::m612_slot_address(5, 0), r::m612_slot_address(0, 5)), (None, None));
}

// ---------------------------------------------------------------- decode

#[test]
fn read_answers_decode_like_openmouse() {
    // redragonDecodeRead: body = [08, addrLo, 32 + addrHi, len, ??, FA, FA, data..] (id 02 in front or not)
    let ans = |addr: u16, data: &[u8]| {
        let mut v = vec![2u8, 0x08, addr as u8, 0x32 + (addr >> 8) as u8, data.len() as u8, 0, 0xfa, 0xfa];
        v.extend_from_slice(data);
        v.resize(16, 0);
        v
    };
    let a = ans(0x0110, &[1, 0x20, 0, 0x20, 0]);
    assert_eq!(a[3], 0x33, "0x32 + 0x01");
    assert_eq!(r::decode_read(&a, 0x0110, 5).unwrap(), vec![1, 0x20, 0, 0x20, 0]);
    assert_eq!(r::decode_read(&a[1..], 0x0110, 5).unwrap(), vec![1, 0x20, 0, 0x20, 0], "without the report id");
    // a wrong echo anywhere is refused: tag, address low, address high, length, the FA FA
    for (i, name) in [(1usize, "tag"), (2, "address low"), (3, "address high"), (4, "length"), (6, "first FA"), (7, "second FA")] {
        let mut bad = a.clone();
        bad[i] ^= 0x01;
        assert!(matches!(r::decode_read(&bad, 0x0110, 5), Err(Error::BadAnswer(_))), "{name}");
    }
    // too short for the asked length
    assert!(r::decode_read(&a[..10], 0x0110, 5).is_err());
    // probeConfigChannel: the last two bytes are FA FA
    assert!(r::has_config_marker(&[2, 0, 0, 0xfa, 0xfa]) && r::has_config_marker(&[0xfa, 0xfa]));
    assert!(!r::has_config_marker(&[2, 0xfa, 0x00]) && !r::has_config_marker(&[0xfa]) && !r::has_config_marker(&[]));
}

// ---------------------------------------------------------------- the device table

fn config(pid: u16) -> HidInfo {
    hid_info("mi_02", 0x04d9, pid, 0xffa0, 0x01, (16, 16, 16))
}

#[test]
fn the_supported_models_match_on_their_vendor_interface() {
    let m = r::matches(&[&config(0xfc61)], 0x04d9, 0xfc61).unwrap();
    assert_eq!((m.family, m.model, m.wireless, m.variant), (Family::Redragon, Some("Redragon PREDATOR M612"), Some(false), r::M612));
    assert_eq!(m.config.path, "mi_02");
    // the table is the M612 only: the other SUPPORTED-list model, the M724 K1NG 1K (04d9:fc7a), is held back (see the last test)
    assert!(r::matches(&[&config(0xfc7a)], 0x04d9, 0xfc7a).is_none());
    assert_eq!(r::PRODUCTS.iter().map(|p| p.0).collect::<Vec<_>>(), vec![0xfc61]);
    // the vendor collection is picked among the mouse's other interfaces
    let kb = hid_info("mi_00", 0x04d9, 0xfc61, 0x01, 0x06, (8, 1, 0));
    let mouse = hid_info("mi_01", 0x04d9, 0xfc61, 0x01, 0x02, (8, 0, 0));
    let cfg = config(0xfc61);
    assert_eq!(r::matches(&[&kb, &mouse, &cfg], 0x04d9, 0xfc61).unwrap().config.path, "mi_02");
}

#[test]
fn other_ids_and_other_interfaces_do_not_match() {
    // a model OpenMouse's driver knows of but the list does not (none for Redragon: the catalog is exactly these two) and
    // other Holtek ids
    assert!(r::matches(&[&config(0xfc62)], 0x04d9, 0xfc62).is_none());
    assert!(r::matches(&[&config(0xfc61)], 0x04d9, 0xfc7b).is_none());
    // another vendor id
    assert!(r::matches(&[&config(0xfc61)], 0x258a, 0xfc61).is_none());
    // the keyboard / mouse interfaces (no feature report 2), a vendor page that is not 0xFFA0, another usage, no feature report
    for h in [
        hid_info("mi_00", 0x04d9, 0xfc61, 0x01, 0x06, (8, 1, 0)),
        hid_info("mi_01", 0x04d9, 0xfc61, 0x01, 0x02, (8, 0, 0)),
        hid_info("x", 0x04d9, 0xfc61, 0xff00, 0x01, (16, 16, 16)),
        hid_info("x", 0x04d9, 0xfc61, 0xffa0, 0x02, (16, 16, 16)),
        hid_info("x", 0x04d9, 0xfc61, 0xffa0, 0x01, (16, 16, 0)),
    ] {
        assert!(r::matches(&[&h], 0x04d9, 0xfc61).is_none(), "{h:?}");
    }
    assert!(r::matches(&[], 0x04d9, 0xfc61).is_none());
}

#[test]
fn no_other_family_claims_the_redragon_ids() {
    {
        let pid = 0xfc61u16;
        let cfg = config(pid);
        let found = bu_mouse::family::find(std::slice::from_ref(&cfg), 0x04d9, pid).unwrap();
        assert_eq!(found.family, Family::Redragon, "{pid:#06x}");
        let own = [&cfg];
        assert!(bu_mouse::pulsar::matches_cmouse(&own, 0x04d9, pid).is_none() && bu_mouse::pulsar::matches_xs1(&own, 0x04d9, pid).is_none());
        assert!(bu_mouse::logitech::matches(&own, 0x04d9, pid).is_none());
        assert!(bu_mouse::razer::matches(&own, 0x04d9, pid).is_none());
        assert!(bu_mouse::steelseries::matches(&own, 0x04d9, pid).is_none());
    }
}

// ---------------------------------------------------------------- the read-only allow-list

#[test]
fn only_openmouses_read_requests_pass_the_allow_list() {
    // the probe's blank frame is only ever FETCHED (GET_FEATURE, nothing sent): as a request it is not a read and does not pass
    assert!(!r::read_request_allowed(&fr(&[])));
    // the F2 reads of the settings memory
    for read in [
        fr(&[0xf2, 0x2c, 0x00, 0x01]),  // active profile
        fr(&[0xf2, 0x32, 0x00, 0x01]),  // polling code
        fr(&[0xf2, 0x32, 0x00, 0x06]),  // polling block (before a polling write)
        fr(&[0xf2, 0x42, 0x00, 0x01]),  // profile 1 base: active stage
        fr(&[0xf2, 0x02, 0x01, 0x01]),  // profile 2 base
        fr(&[0xf2, 0x10, 0x01, 0x05]),  // profile 2, stage 3 slot (0x102 + 2 + 12)
        fr(&[0xf2, 0x14, 0x03, 0x05]),  // profile 5, stage 1 slot (0x312 + 2)
    ] {
        assert!(r::read_request_allowed(&read), "{read:02x?}");
    }
    // zero padding up to a longer report is fine, anything else is not
    let mut long = fr(&[0xf2, 0x2c, 0x00, 0x01]);
    long.extend_from_slice(&[0, 0, 0]);
    assert!(r::read_request_allowed(&long));
    long[17] = 1;
    assert!(!r::read_request_allowed(&long));
    // every write frame is refused
    let mut writes = vec![hello(), close(), fr(&[0xf3, 0x32, 0x00, 0x06, 0, 0, 0, 2, 0, 2, 0, 2, 0]), fr(&[0xf3, 0x10, 0x01, 0x05, 0, 0, 0, 1, 0x6a, 0, 0x6a, 0])];
    writes.extend(commits());
    writes.push(r::m724_polling_frame(500).unwrap().to_vec());
    for w in writes {
        assert!(!r::read_request_allowed(&w), "{w:02x?}");
    }
    // a read of memory this app never reads (lighting selector 0x446, buttons 0x82), a wrong length, a stray byte, the wrong id
    assert!(!r::read_request_allowed(&fr(&[0xf2, 0x46, 0x04, 0x01])));
    assert!(!r::read_request_allowed(&fr(&[0xf2, 0x82, 0x00, 0x08])));
    assert!(!r::read_request_allowed(&fr(&[0xf2, 0x2c, 0x00, 0x02])));
    assert!(!r::read_request_allowed(&fr(&[0xf2, 0x2c, 0x00, 0x01, 0x01])));
    let mut id = fr(&[0xf2, 0x2c, 0x00, 0x01]);
    id[0] = 3;
    assert!(!r::read_request_allowed(&id));
    assert!(!r::read_request_allowed(&fr(&[0xf2, 0x2c, 0x00, 0x01])[..15]));
    assert!(!r::read_request_allowed(&[]));
}

// ---------------------------------------------------------------- a scripted Redragon mouse

/// A fake Holtek mouse: a settings memory that answers `F2` reads, takes `F3` writes only inside an `F5` session, and
/// echoes `FA FA` on a bare GET_FEATURE.
struct Mouse {
    mem: Vec<u8>,
    session: bool,
    sessions_closed: u32,
    commits: Vec<u8>,
    /// the M724 never answers a read request usefully; it only echoes its FA FA marker
    write_only: bool,
    /// no FA FA marker on a bare GET_FEATURE (not the config channel)
    no_marker: bool,
    /// the interface STALLs every F3 write (a malformed write)
    stall_writes: bool,
    /// the mouse takes the write frames and keeps its old bytes
    ignore_writes: bool,
}

impl Mouse {
    /// An M612 at profile 2 (index 1), active stage 3 (index 2) of 1200 DPI, 1000 Hz.
    fn m612() -> Self {
        let mut mem = vec![0u8; 0x500];
        mem[0x2c] = 1;
        mem[0x32..0x38].copy_from_slice(&[0x01, 0x00, 0x02, 0x00, 0x02, 0x00]);
        mem[0x102] = 2;
        for (n, label_pos) in [13usize, 39, 18, 65, 92].iter().enumerate() {
            let a = 0x102 + 2 + 6 * n;
            let (v, range) = r::m612_code_at(*label_pos).unwrap();
            mem[a..a + 5].copy_from_slice(&[1, v, range, v, range]);
        }
        Mouse { mem, session: false, sessions_closed: 0, commits: Vec::new(), write_only: false, no_marker: false, stall_writes: false, ignore_writes: false }
    }

    fn k1ng() -> Self {
        Mouse { write_only: true, ..Mouse::m612() }
    }

    fn answer(&mut self, out: &[u8], how: &HidTransfer) -> Result<Vec<u8>, Error> {
        assert_eq!(out.len(), 16);
        match how {
            HidTransfer::FeatureIn { .. } => {
                let mut v = vec![0u8; 16];
                v[0] = 2;
                if !self.no_marker {
                    v[14] = 0xfa;
                    v[15] = 0xfa;
                }
                Ok(v)
            }
            HidTransfer::FeatureOut => {
                match out[1] {
                    0xf5 => {
                        self.session = out[2] == 0;
                        if out[2] == 1 {
                            self.sessions_closed += 1;
                        }
                    }
                    0xf3 => {
                        if self.stall_writes {
                            return Err(Error::Os { op: "HidD_SetFeature".into(), code: 1117 });
                        }
                        assert!(self.session, "a write outside the F5 session");
                        let (addr, len) = (out[2] as usize | (out[3] as usize) << 8, out[4] as usize);
                        if !self.ignore_writes {
                            self.mem[addr..addr + len].copy_from_slice(&out[8..8 + len]);
                        }
                    }
                    0xf1 => {
                        assert!(self.session, "a commit outside the F5 session");
                        assert_eq!(out[2], 2);
                        self.commits.push(out[3]);
                    }
                    other => panic!("the fake does not know {other:#04x}"),
                }
                Ok(Vec::new())
            }
            HidTransfer::Feature { reply_id, reply_len, wait_ms } => {
                assert_eq!((*reply_id, *reply_len, *wait_ms), (2, 16, 30), "OpenMouse: send, 30 ms, GET_FEATURE of report 2");
                assert_eq!(out[1], 0xf2, "only reads are answered");
                let (addr, len) = (out[2] as usize | (out[3] as usize) << 8, out[4] as usize);
                let mut v = vec![2u8, 0x08, out[2], 0x32 + out[3], out[4], 0, 0xfa, 0xfa];
                if !self.write_only {
                    v.extend_from_slice(&self.mem[addr..addr + len]);
                }
                v.resize(16, 0);
                Ok(v)
            }
            other => panic!("unexpected transfer {other:?}"),
        }
    }
}

fn port_for(info: &HidInfo, mouse: Rc<RefCell<Mouse>>) -> ScriptPort {
    ScriptPort::new(info.clone(), move |out, how| mouse.borrow_mut().answer(out, how))
}

fn sent(port: &ScriptPort) -> Vec<Vec<u8>> {
    port.sent.iter().map(|s| s.0.clone()).collect()
}

// ---------------------------------------------------------------- driver: the Predator M612

#[test]
fn m612_reads_the_active_stage_and_polling_like_openmouse() {
    let info = config(0xfc61);
    let m = r::matches(&[&info], 0x04d9, 0xfc61).unwrap();
    let mouse = Rc::new(RefCell::new(Mouse::m612()));
    let mut port = port_for(&info, mouse);
    let s = r::Redragon.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((s.online, s.dpi, s.polling_hz, s.wireless, s.battery_percent, s.lift_off), (true, Some(1200), Some(1000), Some(false), None, None));
    // probe (a bare GET_FEATURE, the blank frame is only its name), active profile, its active stage, that stage's slot, polling
    assert_eq!(sent(&port), vec![fr(&[]), fr(&[0xf2, 0x2c, 0x00, 0x01]), fr(&[0xf2, 0x02, 0x01, 0x01]), fr(&[0xf2, 0x10, 0x01, 0x05]), fr(&[0xf2, 0x32, 0x00, 0x01])]);
    assert_eq!(port.sent[0].1, HidTransfer::FeatureIn { reply_id: 2, reply_len: 16, wait_ms: 0 });
    assert!(port.sent[1..].iter().all(|s| s.1 == HidTransfer::Feature { reply_id: 2, reply_len: 16, wait_ms: 30 }));
    // the probe is a bare GET_FEATURE (nothing sent); every other request is an allow-listed read
    assert!(port.sent[1..].iter().all(|s| r::read_request_allowed(&s.0)), "reading = read requests only");
    assert!(!r::read_request_allowed(&port.sent[0].0), "the blank probe frame is not on the list");
    // caps: the stage editor's 500..8000 in 10s, the four polling rates, no battery, no lift-off
    assert_eq!((s.caps.dpi.as_ref().map(|d| (d.min(), d.max())), s.caps.polling.clone(), s.caps.battery, s.caps.lift_off.clone()), (Some((500, 8000)), vec![125, 250, 500, 1000], false, Vec::<u32>::new()));
}

#[test]
fn m612_set_dpi_is_the_stage_write_with_the_commit_block_and_a_readback() {
    let info = config(0xfc61);
    let m = r::matches(&[&info], 0x04d9, 0xfc61).unwrap();
    let mouse = Rc::new(RefCell::new(Mouse::m612()));
    let mut port = port_for(&info, mouse.clone());
    // 4000 DPI = label 92: value 0x6a, range 0, both axes; the slot's enabled byte (01) is kept
    let c = r::Redragon.write(&m, &mut port, Setting::Dpi(4000)).unwrap();
    assert_eq!((c.now, c.before), (4000, Some(1200)));
    let mut want = vec![fr(&[0xf2, 0x2c, 0x00, 0x01]), fr(&[0xf2, 0x02, 0x01, 0x01]), fr(&[0xf2, 0x10, 0x01, 0x05]), hello(), fr(&[0xf3, 0x10, 0x01, 0x05, 0, 0, 0, 0x01, 0x6a, 0x00, 0x6a, 0x00])];
    want.extend(commits());
    want.push(close());
    want.push(fr(&[0xf2, 0x10, 0x01, 0x05]));
    assert_eq!(sent(&port), want);
    // writes go as plain feature reports (no answer), the reads as send + 30 ms + get
    assert_eq!(port.sent[3].1, HidTransfer::FeatureOut);
    assert_eq!(&mouse.borrow().mem[0x110..0x115], &[1, 0x6a, 0, 0x6a, 0]);
    assert_eq!(mouse.borrow().commits, vec![0x04, 0x01, 0x02, 0x08, 0x10]);
    assert!(!mouse.borrow().session && mouse.borrow().sessions_closed == 1, "the session is closed");
    // the other stages are untouched
    assert_eq!(&mouse.borrow().mem[0x104..0x109], &[1, 0x1b, 0, 0x1b, 0], "stage 1 is still 1000");
    // 1234 is snapped to 1250 (label 19, value 33) and reported as 1250
    let c = r::Redragon.write(&m, &mut port, Setting::Dpi(1234)).unwrap();
    assert_eq!((c.now, c.before), (1250, Some(4000)));
    assert_eq!(&mouse.borrow().mem[0x110..0x115], &[1, 33, 0, 33, 0]);
    // a high one: 8000 = value 0x6a, range 1
    let c = r::Redragon.write(&m, &mut port, Setting::Dpi(8000)).unwrap();
    assert_eq!(c.now, 8000);
    assert_eq!(&mouse.borrow().mem[0x110..0x115], &[1, 0x6a, 1, 0x6a, 1]);
    // out of range: refused before a byte leaves
    port.sent.clear();
    assert!(matches!(r::Redragon.write(&m, &mut port, Setting::Dpi(499)), Err(Error::OutOfRange { .. })));
    assert!(matches!(r::Redragon.write(&m, &mut port, Setting::Dpi(8001)), Err(Error::OutOfRange { .. })));
    assert!(matches!(r::Redragon.write(&m, &mut port, Setting::LiftOff(10)), Err(Error::OutOfRange { .. })));
    assert!(port.sent.is_empty());
}

#[test]
fn m612_set_polling_rewrites_the_block_with_its_other_bytes_kept() {
    let info = config(0xfc61);
    let m = r::matches(&[&info], 0x04d9, 0xfc61).unwrap();
    let mouse = Rc::new(RefCell::new(Mouse::m612()));
    mouse.borrow_mut().mem[0x32..0x38].copy_from_slice(&[0x01, 0xaa, 0x02, 0xbb, 0x02, 0xcc]);
    let mut port = port_for(&info, mouse.clone());
    let c = r::Redragon.write(&m, &mut port, Setting::Polling(500)).unwrap();
    assert_eq!((c.now, c.before), (500, Some(1000)));
    // read the 6-byte block, write it back with byte 0 = 1000 / 500 = 02 and the other five exactly as read
    let mut want = vec![fr(&[0xf2, 0x32, 0x00, 0x06]), hello(), fr(&[0xf3, 0x32, 0x00, 0x06, 0, 0, 0, 0x02, 0xaa, 0x02, 0xbb, 0x02, 0xcc])];
    want.extend(commits());
    want.push(close());
    want.push(fr(&[0xf2, 0x32, 0x00, 0x06]));
    assert_eq!(sent(&port), want);
    assert_eq!(&mouse.borrow().mem[0x32..0x38], &[0x02, 0xaa, 0x02, 0xbb, 0x02, 0xcc]);
    let s = r::Redragon.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!(s.polling_hz, Some(500));
    for (hz, code) in [(125u32, 8u8), (250, 4), (1000, 1)] {
        let c = r::Redragon.write(&m, &mut port, Setting::Polling(hz)).unwrap();
        assert_eq!(c.now, hz);
        assert_eq!(mouse.borrow().mem[0x32], code);
    }
    port.sent.clear();
    assert!(matches!(r::Redragon.write(&m, &mut port, Setting::Polling(2000)), Err(Error::OutOfRange { .. })));
    assert!(port.sent.is_empty());
}

#[test]
fn m612_refusals_and_a_mouse_that_keeps_its_old_value() {
    let info = config(0xfc61);
    let m = r::matches(&[&info], 0x04d9, 0xfc61).unwrap();
    // the interface STALLs the write: the error comes back and the session is still closed (an open one hangs the mouse)
    let mouse = Rc::new(RefCell::new(Mouse::m612()));
    mouse.borrow_mut().stall_writes = true;
    let mut port = port_for(&info, mouse.clone());
    assert!(matches!(r::Redragon.write(&m, &mut port, Setting::Dpi(2000)), Err(Error::Os { .. })));
    assert_eq!(port.sent.last().unwrap().0, close());
    assert_eq!(mouse.borrow().sessions_closed, 1);
    assert_eq!(&mouse.borrow().mem[0x110..0x115], &[1, 32, 0, 32, 0], "nothing changed");
    // the mouse takes the frames and keeps its value: the read-back catches it
    let mouse = Rc::new(RefCell::new(Mouse::m612()));
    mouse.borrow_mut().ignore_writes = true;
    let mut port = port_for(&info, mouse);
    assert!(matches!(r::Redragon.write(&m, &mut port, Setting::Dpi(2000)), Err(Error::BadAnswer(_))));
    assert!(matches!(r::Redragon.write(&m, &mut port, Setting::Polling(250)), Err(Error::BadAnswer(_))));
}

#[test]
fn m612_wrong_channel_and_garbled_answers_are_errors() {
    let info = config(0xfc61);
    let m = r::matches(&[&info], 0x04d9, 0xfc61).unwrap();
    // no FA FA marker: not the config channel (OpenMouse throws)
    let mouse = Rc::new(RefCell::new(Mouse::m612()));
    mouse.borrow_mut().no_marker = true;
    let mut port = port_for(&info, mouse);
    assert!(matches!(r::Redragon.read(&m, &mut port, ReadOptions::default()), Err(Error::BadAnswer(_))));
    // a profile index outside 0-4
    let mouse = Rc::new(RefCell::new(Mouse::m612()));
    mouse.borrow_mut().mem[0x2c] = 9;
    let mut port = port_for(&info, mouse);
    assert!(matches!(r::Redragon.read(&m, &mut port, ReadOptions::default()), Err(Error::BadAnswer(_))));
    // an active stage outside 0-4
    let mouse = Rc::new(RefCell::new(Mouse::m612()));
    mouse.borrow_mut().mem[0x102] = 7;
    let mut port = port_for(&info, mouse);
    assert!(matches!(r::Redragon.read(&m, &mut port, ReadOptions::default()), Err(Error::BadAnswer(_))));
    // an unknown polling code shows no polling rate (OpenMouse would refuse the whole status)
    let mouse = Rc::new(RefCell::new(Mouse::m612()));
    mouse.borrow_mut().mem[0x32] = 0x03;
    let mut port = port_for(&info, mouse);
    let s = r::Redragon.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((s.dpi, s.polling_hz), (Some(1200), None));
    // the mouse is unplugged
    let mut gone = ScriptPort::new(info.clone(), |_, _| Err(Error::MouseGone("mi_02".into())));
    assert!(matches!(r::Redragon.read(&m, &mut gone, ReadOptions::default()), Err(Error::MouseGone(_))));
}

// ---------------------------------------------------------------- the M724 K1NG 1K (held back)

#[test]
fn the_k1ng_is_held_back_nothing_claims_it_and_nothing_is_sent_to_it() {
    // OpenMouse can only write the M724 blind (it never answers a read); its polling frame would replace five bytes this code
    // does not understand and its DPI write the whole saved table: no control, so the table does not claim 04d9:fc7a
    let cfg = config(0xfc7a);
    assert!(r::matches(&[&cfg], 0x04d9, 0xfc7a).is_none());
    assert!(bu_mouse::family::find(std::slice::from_ref(&cfg), 0x04d9, 0xfc7a).is_none_or(|f| f.family != Family::Redragon));
    // a Matched built by hand for it is refused by the driver, read and write, before a byte is sent
    let m12 = r::matches(&[&config(0xfc61)], 0x04d9, 0xfc61).unwrap();
    let held = bu_mouse::family::Matched { variant: r::M724, model: Some("Redragon M724 K1NG 1K"), config: cfg.clone(), ..m12 };
    let mouse = Rc::new(RefCell::new(Mouse::k1ng()));
    let mut port = port_for(&cfg, mouse.clone());
    assert!(matches!(r::Redragon.read(&held, &mut port, ReadOptions::default()), Err(Error::UnsupportedMouse(_))));
    for s in [Setting::Polling(500), Setting::Dpi(1600), Setting::LiftOff(10)] {
        assert!(matches!(r::Redragon.write(&held, &mut port, s), Err(Error::UnsupportedMouse(_))), "{s:?}");
    }
    assert!(port.sent.is_empty());
    assert!(mouse.borrow().commits.is_empty());
}

#[test]
fn m612_dpi_write_does_not_flatten_a_slot_with_a_separate_y() {
    let info = config(0xfc61);
    let m = r::matches(&[&info], 0x04d9, 0xfc61).unwrap();
    // the active slot (profile 2, stage 3 at 0x110) holds X = label 13 (value 0x1b) but Y = label 39 (value 0x35): setDpiStageValue
    // writes one code on both axes, which would overwrite the Y
    for slot in [[1u8, 0x1b, 0, 0x35, 0], [1, 0x1b, 0, 0x1b, 1]] {
        let mouse = Rc::new(RefCell::new(Mouse::m612()));
        mouse.borrow_mut().mem[0x110..0x115].copy_from_slice(&slot);
        let mut port = port_for(&info, mouse.clone());
        let e = r::Redragon.write(&m, &mut port, Setting::Dpi(4000)).unwrap_err();
        assert!(matches!(e, Error::OutOfRange { .. }), "{e:?}");
        // active profile, active stage, the slot - three reads; no hello, no write, no commit
        assert_eq!(sent(&port), vec![fr(&[0xf2, 0x2c, 0x00, 0x01]), fr(&[0xf2, 0x02, 0x01, 0x01]), fr(&[0xf2, 0x10, 0x01, 0x05])]);
        assert!(port.sent.iter().all(|s| r::read_request_allowed(&s.0)));
        assert!(mouse.borrow().commits.is_empty());
        assert_eq!(&mouse.borrow().mem[0x110..0x115], &slot, "the slot is untouched");
    }
}

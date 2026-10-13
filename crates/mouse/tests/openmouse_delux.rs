//! Order 105 check, Delux M800 Mini: our requests = the bytes OpenMouse sends for the same setting (encode), and answers
//! shaped the way OpenMouse reads them decode to the same values (decode). The expected bytes are worked out by hand from
//! OpenMouse (@openmouse/protocol 0.40.1: `dist/drivers/delux/hid.js`, `dist/delux/index.js`, `dist/compx/x11-dpi.js`),
//! never from our own encoder.
//!
//! OpenMouse's DPI report (buildX11DpiReport), index 0 = the report id 0x04: `[1]` 0x38, `[2]` 0x01, `[3]` angle snap, `[4]`
//! ripple, `[5]` 0x3F, `[6]` `[7]` stage mask (bit i = stage i is above 12000 DPI), `[8..14]` the six DPI bytes, `[16..22]` the
//! six "high" flags (10100..12000 or 20100..22000), `[24]` the active stage (1-based), 0xFF at 25, 29, 33-35, 38-40, 42, 43,
//! 46-48, 0x40 at 44, 0x02 at 49, `[50]` `[51]` the sum of bytes 3..49 (big endian); 56 bytes in all (52 over the cable).

use bu_mouse::delux as dx;
use bu_mouse::family::{Change, Driver, Extra, Family, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::os::{HidInfo, HidTransfer};
use std::cell::RefCell;
use std::rc::Rc;

/// The factory table by hand: stages 800 / 1600 / 2400 / 3200 / 5000 / 22000, stage 2 active, angle snapping off, ripple on.
/// DPI bytes: 800→0x12, 1600→0x25, 2400→0x38, 3200→0x4b, 5000→0x75, 22000→0x81. Only stage 6 is above 12000: mask 0x20; it is
/// also inside 20100..22000, so its high flag ([21]) is 1.
/// Checksum, bytes 3..49: 0 + 1 + 0x3f(63) = 64; + mask 32 + 32 = 128; + (18 + 37 + 56 + 75 + 117 + 129 = 432) = 560; + flag 1 = 561;
/// + active 2 = 563; + 0xff at 25 and 29 = 1073; + 0xff x3 at 33-35 = 1838; + 0xff x3 at 38-40 = 2603; + 0xff x2 at 42, 43 = 3113;
/// + 0x40 (64) = 3177; + 0xff x3 at 46-48 = 3942; + 0x02 at 49 = 3944 = 0x0F68.
fn factory() -> Vec<u8> {
    let mut v = vec![0u8; 56];
    v[..9].copy_from_slice(&[0x04, 0x38, 0x01, 0x00, 0x01, 0x3f, 0x20, 0x20, 0x12]);
    v[9..14].copy_from_slice(&[0x25, 0x38, 0x4b, 0x75, 0x81]);
    v[21] = 0x01;
    v[24] = 0x02;
    for i in [25, 29, 33, 34, 35, 38, 39, 40, 42, 43, 46, 47, 48] {
        v[i] = 0xff;
    }
    v[44] = 0x40;
    v[49] = 0x02;
    v[50] = 0x0f;
    v[51] = 0x68;
    v
}

fn state(stages: [u32; 6], active: u8) -> dx::DpiState {
    dx::DpiState { stages, active, angle_snap: false, ripple: true }
}

const FACTORY: [u32; 6] = [800, 1600, 2400, 3200, 5000, 22000];

// ---------------------------------------------------------------- the DPI byte map

#[test]
fn the_dpi_byte_map_is_openmouses() {
    // X11_DPI_STEP_MAP spot values, read off the JS table
    for (dpi, b) in [(50, 0x01), (100, 0x02), (350, 0x08), (800, 0x12), (1600, 0x25), (3200, 0x4b), (5000, 0x75), (10000, 0xeb), (10100, 0x76), (11000, 0x81), (12000, 0x8d), (12100, 0x8e), (20000, 0xeb), (20100, 0xeb), (20200, 0x76), (20300, 0x76), (21900, 0x80), (22000, 0x81)] {
        assert_eq!(dx::dpi_byte(dpi), b, "{dpi}");
    }
    // nearestX11Dpi: the value at or above, clamped to 50..22000
    for (typed, v) in [(51, 100), (800, 800), (801, 850), (10050, 10100), (10101, 10200), (22001, 22000), (0, 50), (49, 50), (21950, 22000)] {
        assert_eq!(dx::nearest_up(typed), v, "{typed}");
    }
    // 320 entries: 200 at 50-steps up to 10000 and 120 at 100-steps from 10100 to 22000
    let n50 = (50..=10000).step_by(50).count();
    let n100 = (10100..=22000).step_by(100).count();
    assert_eq!((n50, n100), (200, 120));
    let mut last = 0;
    for d in (50..=10000).step_by(50).chain((10100..=22000).step_by(100)) {
        assert_eq!(dx::nearest_up(d), d, "{d} is on the map");
        assert!(d > last);
        last = d;
    }
}

// ---------------------------------------------------------------- encode

#[test]
fn the_dpi_report_is_openmouses_buildx11dpireport() {
    assert_eq!(dx::build_dpi_report(&state(FACTORY, 2), false), factory());
    // over the cable only the first 52 bytes go out
    assert_eq!(dx::build_dpi_report(&state(FACTORY, 2), true), factory()[..52].to_vec());
    // stage 2 = 1000 DPI → byte 0x17 (was 0x25 = 37, now 23): sum 3944 - 14 = 3930 = 0x0F5A
    let mut want = factory();
    want[9] = 0x17;
    want[50] = 0x0f;
    want[51] = 0x5a;
    assert_eq!(dx::build_dpi_report(&state([800, 1000, 2400, 3200, 5000, 22000], 2), false), want);
    // stage 2 = 11000 DPI → byte 0x81, high flag [17] = 1, no mask bit (11000 <= 12000): sum 3944 - 37 + 129 + 1 = 4037 = 0x0FC5
    let mut want = factory();
    want[9] = 0x81;
    want[17] = 0x01;
    want[50] = 0x0f;
    want[51] = 0xc5;
    assert_eq!(dx::build_dpi_report(&state([800, 11000, 2400, 3200, 5000, 22000], 2), false), want);
    // stage 2 = 15000 DPI → byte 0xb0, mask bit 1 (0x02) on top of 0x20 = 0x22 in [6] and [7], no high flag:
    // sum 3944 - 37 + 176 + 2 x 2 (both mask bytes) = 4087 = 0x0FF7
    let mut want = factory();
    want[9] = 0xb0;
    want[6] = 0x22;
    want[7] = 0x22;
    want[50] = 0x0f;
    want[51] = 0xf7;
    assert_eq!(dx::build_dpi_report(&state([800, 15000, 2400, 3200, 5000, 22000], 2), false), want);
    // angle snapping on, active stage 6: [3] = 1 and [24] = 6: sum 3944 + 1 + 4 = 3949 = 0x0F6D
    let mut want = factory();
    want[3] = 1;
    want[24] = 6;
    want[50] = 0x0f;
    want[51] = 0x6d;
    let mut s = state(FACTORY, 6);
    s.angle_snap = true;
    assert_eq!(dx::build_dpi_report(&s, false), want);
}

#[test]
fn the_polling_report_is_openmouses() {
    // setPollingRate: [0x09, 0x01, rate, 0xff - rate, 0, 0, 0, 0] on report 0x06
    assert_eq!(dx::polling_request(125), Some(vec![0x06, 0x09, 0x01, 0x08, 0xf7, 0, 0, 0, 0]));
    assert_eq!(dx::polling_request(250), Some(vec![0x06, 0x09, 0x01, 0x04, 0xfb, 0, 0, 0, 0]));
    assert_eq!(dx::polling_request(500), Some(vec![0x06, 0x09, 0x01, 0x02, 0xfd, 0, 0, 0, 0]));
    assert_eq!(dx::polling_request(1000), Some(vec![0x06, 0x09, 0x01, 0x01, 0xfe, 0, 0, 0, 0]));
    for hz in [2000, 750, 0] {
        assert_eq!(dx::polling_request(hz), None);
    }
    // Windows pads to the collection's feature-report length
    let p = dx::padded(&dx::polling_request(500).unwrap(), 20).unwrap();
    assert_eq!((p.len(), &p[..9], &p[9..]), (20, &[0x06, 0x09, 0x01, 0x02, 0xfd, 0, 0, 0, 0][..], &[0u8; 11][..]));
    assert!(dx::padded(&[0u8; 56], 20).is_none(), "a report that does not fit is not cut");
}

// ---------------------------------------------------------------- decode

#[test]
fn the_dpi_report_decodes_like_openmouse() {
    let s = dx::decode_dpi_report(&factory()).unwrap();
    assert_eq!((s.stages, s.active, s.angle_snap, s.ripple), (FACTORY, 2, false, true));
    // over the cable: 52 bytes are enough
    assert_eq!(dx::decode_dpi_report(&factory()[..52]).unwrap().stages, FACTORY);
    // the form with the report id cut off (starts 0x38 0x01) is accepted too
    assert_eq!(dx::decode_dpi_report(&factory()[1..]).unwrap().stages, FACTORY);
    // a bad checksum, a wrong signature, too short
    let mut bad = factory();
    bad[51] ^= 1;
    assert!(dx::decode_dpi_report(&bad).is_none());
    let mut bad = factory();
    bad[1] = 0x39;
    assert!(dx::decode_dpi_report(&bad).is_none());
    assert!(dx::decode_dpi_report(&factory()[..40]).is_none());
    // a DPI byte off the map: 0x00 is nobody's byte
    let mut off = factory();
    off[8] = 0x00;
    off[51] = 0x56; // 0x68 - 0x12: the checksum stays valid, only the byte is unknown
    assert!(dx::decode_dpi_report(&off).is_none());
    // the repeated bytes: 20200 and 20300 both write 0x76, the decoder keeps the lower value (reverseRange)
    let mut s = state([800, 20300, 2400, 3200, 5000, 22000], 2);
    let d = dx::decode_dpi_report(&dx::build_dpi_report(&s, false)).unwrap();
    assert_eq!(d.stages[1], 20200);
    // the mask / high flags pick the page: 10100 → 0x76 with the high flag → 10100, 5050 → 0x76 without → 5050
    s.stages = [5050, 10100, 12000, 12100, 20000, 20100];
    let d = dx::decode_dpi_report(&dx::build_dpi_report(&s, false)).unwrap();
    assert_eq!(d.stages, [5050, 10100, 12000, 12100, 20000, 20100]);
}

// ---------------------------------------------------------------- allow-list

#[test]
fn nothing_passes_the_byte_allow_list_the_get_feature_read_passes_the_gate_by_itself() {
    // The Delux read is a GetFeature of report 4 (HidTransfer::FeatureIn): it sends nothing, so the read-only gate lets every
    // FeatureIn through. The allow-list sees only bytes, and `[04, 00 ...]` sent as a SetFeature would be a DPI-table write
    // (report 4 with a valid checksum, in the mouse's eyes), so it must NOT pass; no read goes out as anything but a FeatureIn.
    let mut blank = vec![0u8; 262];
    blank[0] = 0x04;
    assert!(!dx::read_request_allowed(&blank));
    assert!(!dx::read_request_allowed(&dx::read_dpi_request(56)));
    assert!(!dx::read_request_allowed(&dx::read_dpi_request(1)));
    // every write: the DPI table (byte 1 is 0x38), the polling report, anything else
    assert!(!dx::read_request_allowed(&factory()));
    for hz in [125, 250, 500, 1000] {
        assert!(!dx::read_request_allowed(&dx::polling_request(hz).unwrap()));
        assert!(!dx::read_request_allowed(&dx::padded(&dx::polling_request(hz).unwrap(), 262).unwrap()));
    }
    assert!(!dx::read_request_allowed(&[0x06]), "report 0x06 is the polling report");
    assert!(!dx::read_request_allowed(&[0x03, 0x55, 0x40, 0x01, 0x20]));
    assert!(!dx::read_request_allowed(&[]));
}

// ---------------------------------------------------------------- device table

fn cfg(pid: u16, product: Option<&str>) -> HidInfo {
    let mut h = hid_info(&format!("cfg{pid:04x}"), 0x1d57, pid, 0x0b, 0x01, (0, 0, 262));
    h.interface = Some(2);
    h.product = product.map(String::from);
    h
}

fn mouse_col(pid: u16, product: Option<&str>) -> HidInfo {
    let mut h = hid_info(&format!("mouse{pid:04x}"), 0x1d57, pid, 0x01, 0x02, (8, 0, 0));
    h.interface = Some(0);
    h.product = product.map(String::from);
    h
}

#[test]
fn the_table_is_openmouses_supported_list() {
    // supported-devices-data.ts: Delux M800 Mini, pids 0xfa60 (receiver) and 0xfa55 (cable)
    for (pid, wireless) in [(0xfa60u16, true), (0xfa55, false)] {
        let (c, mo) = (cfg(pid, Some("Delux M800 Mini")), mouse_col(pid, Some("Delux M800 Mini")));
        let m = dx::matches(&[&mo, &c], 0x1d57, pid).unwrap();
        assert_eq!((m.family, m.model, m.wireless), (Family::Delux, Some("Delux M800 Mini"), Some(wireless)));
        assert_eq!((m.config.usage_page, m.config.path.as_str()), (0x0b, c.path.as_str()));
    }
    // the ids are shared with other mice (the Attack Shark X11): OpenMouse needs "Delux" in the product string
    for product in [None, Some("Attack Shark X11"), Some("2.4G Wireless Device"), Some("USB Gaming Mouse"), Some("Deluxe Mouse"), Some("")] {
        let (c, mo) = (cfg(0xfa60, product), mouse_col(0xfa60, product));
        assert!(dx::matches(&[&mo, &c], 0x1d57, 0xfa60).is_none(), "{product:?}");
    }
    let (c, mo) = (cfg(0xfa60, Some("USB Delux Wireless Mouse")), mouse_col(0xfa60, Some("USB Delux Wireless Mouse")));
    assert!(dx::matches(&[&mo, &c], 0x1d57, 0xfa60).is_some(), "the word Delux anywhere in the string, any case");
    // the M600 Pro (0xfa71) is not on the supported list; other ids / vendors do not match
    let c = cfg(0xfa71, Some("Delux M600 Pro"));
    assert!(dx::matches(&[&c], 0x1d57, 0xfa71).is_none());
    let c = cfg(0xfa60, Some("Delux M800 Mini"));
    assert!(dx::matches(&[&c], 0x1d58, 0xfa60).is_none());
    assert!(dx::matches(&[&c], 0x25a7, 0xfa60).is_none());
    // no config sub-collection (usage page 0x0b with a feature report) = nothing to talk to
    let mo = mouse_col(0xfa60, Some("Delux M800 Mini"));
    assert!(dx::matches(&[&mo], 0x1d57, 0xfa60).is_none());
    let mut nofeat = cfg(0xfa60, Some("Delux M800 Mini"));
    nofeat.feature_len = 0;
    assert!(dx::matches(&[&nofeat], 0x1d57, 0xfa60).is_none());
    // find(): the Delux family is found for a Delux-named mouse on the shared ids
    let all = [cfg(0xfa55, Some("Delux M800 Mini")), mouse_col(0xfa55, Some("Delux M800 Mini"))];
    assert_eq!(bu_mouse::family::find(&all, 0x1d57, 0xfa55).map(|m| m.family), Some(Family::Delux));
}

#[test]
fn caps_are_what_openmouse_offers() {
    let w = dx::caps(false);
    assert_eq!(w.dpi.as_ref().unwrap().bands, vec![(50, 10000, 50), (10100, 20200, 100), (20400, 22000, 200)], "getDpiOptions' values minus the nine that read back as the value 100 below");
    let d = w.dpi.as_ref().unwrap();
    // 20300, 20500 ... 21900 write the same byte + flags as 20200, 20400 ... 21800: not offered
    for v in [20300, 20500, 20700, 20900, 21100, 21300, 21500, 21700, 21900] {
        assert!(!d.allows(v), "{v}");
    }
    for v in [20100, 20200, 20400, 20600, 20800, 21000, 21200, 21400, 21600, 21800, 22000] {
        assert!(d.allows(v), "{v}");
    }
    // every offered value survives encode + decode unchanged (the mouse reads back what was written)
    for (min, max, step) in &d.bands {
        let mut v = *min;
        while v <= *max {
            assert!(dx::dpi_is_exact(v), "{v}");
            let back = dx::decode_dpi_report(&dx::build_dpi_report(&state([v; 6], 1), false)).unwrap();
            assert_eq!(back.stages[0], v);
            v += step;
        }
    }
    assert!(!dx::dpi_is_exact(20300) && !dx::dpi_is_exact(21900) && !dx::dpi_is_exact(10050));
    assert!(d.allows(50) && d.allows(10000) && d.allows(10100) && d.allows(22000) && !d.allows(10050) && !d.allows(10150));
    assert_eq!(w.polling, vec![125, 250, 500, 1000]);
    assert!(w.lift_off.is_empty() && !w.battery, "the battery only arrives unasked");
    let c = dx::caps(true);
    assert!(c.dpi.is_none(), "the cable's DPI is held back");
    assert_eq!(c.polling, vec![125, 250, 500, 1000]);
}

// ---------------------------------------------------------------- the driver against a scripted mouse

struct Mouse {
    table: Vec<u8>,
    ignore_writes: bool,
    unreadable: bool,
}

fn setup(pid: u16, mouse: Rc<RefCell<Mouse>>) -> (bu_mouse::family::Matched, ScriptPort) {
    let c = cfg(pid, Some("Delux M800 Mini"));
    let m = dx::matches(&[&c], 0x1d57, pid).unwrap();
    let p = ScriptPort::new(c, move |out, how| {
        let mut mo = mouse.borrow_mut();
        match how {
            HidTransfer::FeatureIn { reply_id, reply_len, wait_ms } => {
                assert_eq!((*reply_id, *reply_len, *wait_ms), (0x04, 262, 0));
                assert!(out.iter().skip(1).all(|b| *b == 0) && out[0] == 4, "the read is a GetFeature of report 4: nothing else in it");
                if mo.unreadable {
                    return Err(bu_mouse::Error::BadAnswer("no answer".into()));
                }
                let mut r = mo.table.clone();
                r.resize(262, 0);
                Ok(r)
            }
            HidTransfer::FeatureOut => {
                assert_eq!(out.len(), 262, "padded to the collection's feature-report length");
                if out[0] == 0x04 && !mo.ignore_writes {
                    mo.table = out[..56].to_vec();
                }
                Ok(Vec::new())
            }
            other => panic!("the Delux driver uses feature reports only, not {other:?}"),
        }
    });
    (m, p)
}

fn new_mouse() -> Rc<RefCell<Mouse>> {
    Rc::new(RefCell::new(Mouse { table: factory(), ignore_writes: false, unreadable: false }))
}

#[test]
fn reading_asks_only_what_openmouse_asks() {
    // the receiver: GetFeature of report 4, the active stage (2) is 1600 DPI
    let mouse = new_mouse();
    let (m, mut p) = setup(0xfa60, mouse);
    let r = d_read(&m, &mut p);
    assert!(p.sent.iter().all(|s| matches!(s.1, HidTransfer::FeatureIn { reply_id: 4, .. })), "read requests only: GetFeature fetches, nothing written");
    assert_eq!(p.sent.len(), 1);
    assert_eq!((r.online, r.dpi, r.polling_hz, r.battery_percent, r.lift_off, r.wireless), (true, Some(1600), None, None, None, Some(true)));
    assert_eq!(r.caps.polling, vec![125, 250, 500, 1000]);
    // the cable: OpenMouse reads nothing - nothing is asked, nothing is known
    let (m, mut p) = setup(0xfa55, new_mouse());
    let r = d_read(&m, &mut p);
    assert!(p.sent.is_empty());
    assert_eq!((r.online, r.dpi, r.wireless), (true, None, Some(false)));
    assert!(r.caps.dpi.is_none());
    // a receiver that does not hand over its table: offline, nothing invented
    let mouse = new_mouse();
    mouse.borrow_mut().unreadable = true;
    let (m, mut p) = setup(0xfa60, mouse.clone());
    let r = d_read(&m, &mut p);
    assert_eq!((r.online, r.dpi), (false, None));
    // a table with a bad checksum is the same
    mouse.borrow_mut().unreadable = false;
    mouse.borrow_mut().table[51] ^= 1;
    let (m, mut p) = setup(0xfa60, mouse);
    assert!(!d_read(&m, &mut p).online);
}

fn d_read(m: &bu_mouse::family::Matched, p: &mut ScriptPort) -> bu_mouse::family::OnMouse {
    dx::Delux.read(m, p, ReadOptions::default()).unwrap()
}

#[test]
fn a_polling_change_is_openmouses_report_6() {
    for pid in [0xfa60u16, 0xfa55] {
        for (hz, rate) in [(125u32, 0x08u8), (250, 0x04), (500, 0x02), (1000, 0x01)] {
            let mouse = new_mouse();
            let (m, mut p) = setup(pid, mouse);
            let c = dx::Delux.write(&m, &mut p, Setting::Polling(hz)).unwrap();
            assert_eq!((c.now, c.before), (hz, None), "the mouse can't say what it had");
            let mut want = vec![0u8; 262];
            want[..9].copy_from_slice(&[0x06, 0x09, 0x01, rate, 0xff - rate, 0, 0, 0, 0]);
            assert_eq!(p.sent, vec![(want, HidTransfer::FeatureOut)], "{pid:04x} {hz}");
        }
        let (m, mut p) = setup(pid, new_mouse());
        assert!(dx::Delux.write(&m, &mut p, Setting::Polling(2000)).is_err());
        assert!(dx::Delux.write(&m, &mut p, Setting::LiftOff(10)).is_err());
        assert!(p.sent.is_empty());
    }
}

#[test]
fn a_dpi_change_rewrites_the_table_the_openmouse_way_and_reads_it_back() {
    let mouse = new_mouse();
    let (m, mut p) = setup(0xfa60, mouse.clone());
    let c = dx::Delux.write(&m, &mut p, Setting::Dpi(1000)).unwrap();
    assert_eq!((c.now, c.before), (1000, Some(1600)));
    // read, write, read back
    let mut want = factory();
    want[9] = 0x17;
    want[50] = 0x0f;
    want[51] = 0x5a;
    let mut padded = want.clone();
    padded.resize(262, 0);
    assert_eq!(p.sent.len(), 3);
    assert_eq!((p.sent[0].1.clone(), p.sent[1].clone(), p.sent[2].1.clone()), (HidTransfer::FeatureIn { reply_id: 4, reply_len: 262, wait_ms: 0 }, (padded, HidTransfer::FeatureOut), HidTransfer::FeatureIn { reply_id: 4, reply_len: 262, wait_ms: 0 }));
    assert_eq!(mouse.borrow().table, want);
    // an off-map value goes to the next one up, as nearestX11Dpi does
    let c = dx::Delux.write(&m, &mut p, Setting::Dpi(1010)).unwrap();
    assert_eq!(c.now, 1050);
    // out of range: refused before a byte leaves
    p.sent.clear();
    for bad in [49, 22001, 0] {
        assert!(dx::Delux.write(&m, &mut p, Setting::Dpi(bad)).is_err(), "{bad}");
    }
    assert!(p.sent.is_empty());
}

#[test]
fn a_table_that_holds_more_than_openmouse_models_is_left_alone() {
    // a byte after the checksum (the receiver's 56-byte tail) that OpenMouse would zero
    let mouse = new_mouse();
    mouse.borrow_mut().table[53] = 0x07;
    let (m, mut p) = setup(0xfa60, mouse.clone());
    assert!(dx::Delux.write(&m, &mut p, Setting::Dpi(1000)).is_err());
    // a byte inside the checksummed range that differs from OpenMouse's constant (the checksum fixed up to stay valid)
    let mouse = new_mouse();
    {
        let mut mo = mouse.borrow_mut();
        mo.table[26] = 0x55;
        mo.table[51] = 0x68 + 0x55;
    }
    let (m, mut p2) = setup(0xfa60, mouse.clone());
    assert!(dx::Delux.read(&m, &mut p2, ReadOptions::default()).unwrap().dpi.is_some(), "it still reads fine");
    assert!(dx::Delux.write(&m, &mut p2, Setting::Dpi(1000)).is_err());
    assert!(p.sent.iter().chain(p2.sent.iter()).all(|s| s.1 != HidTransfer::FeatureOut), "nothing was written");
    // polling does not touch the table at all, so it is still fine on such a mouse
    assert!(dx::Delux.write(&m, &mut p2, Setting::Polling(500)).is_ok());
}

#[test]
fn the_cable_dpi_is_held_back_and_a_deaf_mouse_is_reported() {
    let (m, mut p) = setup(0xfa55, new_mouse());
    assert!(dx::Delux.write(&m, &mut p, Setting::Dpi(1000)).is_err());
    assert!(p.sent.is_empty(), "OpenMouse never reads the cable's table, so nothing is sent");
    let mouse = new_mouse();
    mouse.borrow_mut().ignore_writes = true;
    let (m, mut p) = setup(0xfa60, mouse);
    let e = dx::Delux.write(&m, &mut p, Setting::Dpi(1000)).unwrap_err();
    assert!(matches!(e, bu_mouse::Error::BadAnswer(ref s) if s.contains("did not confirm")), "{e}");
    // a table that can't be read is not written
    let mouse = new_mouse();
    mouse.borrow_mut().unreadable = true;
    let (m, mut p) = setup(0xfa60, mouse);
    assert!(dx::Delux.write(&m, &mut p, Setting::Dpi(1000)).is_err());
    assert!(p.sent.iter().all(|s| s.1 != HidTransfer::FeatureOut));
}

#[test]
fn a_byte_stored_behind_the_first_56_is_never_zeroed() {
    // the write is padded to the whole 262-byte feature report; a nonzero byte behind byte 56 would be overwritten with a zero
    // that was never compared, so such a table is left alone (nothing written)
    for at in [56, 57, 100, 261] {
        let mouse = new_mouse();
        {
            let mut mo = mouse.borrow_mut();
            mo.table.resize(262, 0);
            mo.table[at] = 0x07;
        }
        let (m, mut p) = setup(0xfa60, mouse.clone());
        assert!(dx::Delux.read(&m, &mut p, ReadOptions::default()).unwrap().dpi.is_some(), "it still reads fine");
        assert!(dx::Delux.write(&m, &mut p, Setting::Dpi(1000)).is_err(), "{at}");
        assert!(p.sent.iter().all(|s| s.1 != HidTransfer::FeatureOut), "{at}: nothing was written");
        assert_eq!(mouse.borrow().table[at], 0x07);
    }
    // the same table with zeros behind byte 56 is written as before
    let (m, mut p) = setup(0xfa60, new_mouse());
    assert!(dx::Delux.write(&m, &mut p, Setting::Dpi(1000)).is_ok());
}

#[test]
fn a_dpi_the_mouse_would_read_back_as_another_is_refused_and_nothing_is_sent() {
    // 20300 writes the byte of 20200 (0x76 with the mask and high flags): the mouse would take it, read back 20200, and the click
    // would fail with no undo - so none of the nine such values is sent
    for bad in [20300, 20500, 20700, 20900, 21100, 21300, 21500, 21700, 21900] {
        let mouse = new_mouse();
        let (m, mut p) = setup(0xfa60, mouse.clone());
        assert!(dx::Delux.write(&m, &mut p, Setting::Dpi(bad)).is_err(), "{bad}");
        // 20250 is not on the map: it would snap up to 20300, which is one of them
        assert!(p.sent.is_empty(), "{bad}: nothing is sent, not even the read");
        assert_eq!(mouse.borrow().table, factory());
    }
    let (m, mut p) = setup(0xfa60, new_mouse());
    assert!(dx::Delux.write(&m, &mut p, Setting::Dpi(20250)).is_err());
    assert!(p.sent.is_empty());
    // a neighbour that is fine still works
    assert_eq!(dx::Delux.write(&m, &mut p, Setting::Dpi(20400)).unwrap().now, 20400);
}

// ---------------------------------------------------------------- Order 110: angle snapping and ripple control

#[test]
fn angle_snapping_and_ripple_are_the_tables_bytes_3_and_4() {
    // readStatus: angleSnapping = table[3] === 1 (off), rippleControl = table[4] === 1 (on)
    let mouse = new_mouse();
    let (m, mut p) = setup(0xfa60, mouse.clone());
    let r = d_read(&m, &mut p);
    assert_eq!(r.extras.iter().map(|e| (e.extra, e.value)).collect::<Vec<_>>(), vec![(Extra::AngleSnap, Some(0)), (Extra::RippleControl, Some(1))]);
    // setAngleSnapping(true): writeDpiTable with [3] = 1 → checksum 3944 + 1 = 3945 = 0x0F69; read, write, read back
    p.sent.clear();
    dx::Delux.change(&m, &mut p, Change::Extra(Extra::AngleSnap, 1)).unwrap();
    let mut want = factory();
    want[3] = 1;
    want[50] = 0x0f;
    want[51] = 0x69;
    let mut padded = want.clone();
    padded.resize(262, 0);
    assert_eq!(p.sent.len(), 3);
    assert_eq!(p.sent[1], (padded, HidTransfer::FeatureOut));
    assert_eq!(mouse.borrow().table, want);
    // setRippleControl(false): [4] = 0 → 3945 − 1 = 3944 = 0x0F68
    dx::Delux.change(&m, &mut p, Change::Extra(Extra::RippleControl, 0)).unwrap();
    want[4] = 0;
    want[51] = 0x68;
    assert_eq!(mouse.borrow().table, want);
    let r = d_read(&m, &mut p);
    assert_eq!((r.extra(Extra::AngleSnap), r.extra(Extra::RippleControl), r.dpi), (Some(1), Some(0), Some(1600)));
    // a mouse that ignores it
    mouse.borrow_mut().ignore_writes = true;
    assert!(dx::Delux.change(&m, &mut p, Change::Extra(Extra::RippleControl, 1)).is_err());
    // refused before a byte leaves: the cable (its table is held back), values and settings OpenMouse does not take here
    let (m, mut p) = setup(0xfa55, new_mouse());
    assert!(d_read(&m, &mut p).extras.is_empty(), "the cable reads nothing");
    for c in [Change::Extra(Extra::AngleSnap, 1), Change::Extra(Extra::RippleControl, 0), Change::Extra(Extra::AngleSnap, 2), Change::Extra(Extra::MotionSync, 1), Change::Extra(Extra::Debounce, 4)] {
        assert!(dx::Delux.change(&m, &mut p, c).is_err(), "{c:?}");
    }
    assert!(p.sent.is_empty());
}

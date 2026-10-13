//! Order 105 check, WLMouse: our requests = the bytes OpenMouse sends for the same setting (encode), and answers shaped the way
//! OpenMouse reads them decode to the same values (decode). The expected bytes are worked out by hand from OpenMouse
//! (@openmouse/protocol 0.40.1: `dist/compx/codec.js`, `dist/wlmouse/index.js`, `dist/drivers/wlmouse/hid.js`,
//! `dist/drivers/vendors.js`), never from our own encoder.
//!
//! OpenMouse's feature report (id 0) is 64 bytes without the id:
//!   [0] status 0 · [1] 0 · [2] target (2 = the mouse) · [3] length · [4] page · [5] command · [6..] arguments
//! (compaxEncodeRequest). Windows puts the report id 0 in front: OpenMouse's byte i = our byte i + 1, so in the 65-byte reports
//! below the target is byte 3, the length byte 4, the page byte 5, the command byte 6 and the arguments start at byte 7.
//! An answer is [0] status 0xa1 · [3] payload length · [4] page · [5] command · [6..] payload (again without the id).

use bu_mouse::family::{Driver, Family, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::os::{HidInfo, HidTransfer};
use bu_mouse::wlmouse as wl;
use bu_mouse::Error;
use std::cell::RefCell;
use std::rc::Rc;

/// A request as Windows sends it to the mouse (target 2): length, page, command, arguments, by hand.
fn rq(len: u8, page: u8, cmd: u8, args: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 65];
    v[3] = 0x02;
    v[4] = len;
    v[5] = page;
    v[6] = cmd;
    v[7..7 + args.len()].copy_from_slice(args);
    v
}

/// An answer as GetFeature hands it over on Windows: report id 0, then OpenMouse's 64 bytes.
fn answer(status: u8, page: u8, cmd: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 65];
    v[1] = status;
    v[3] = 0x02;
    v[4] = payload.len() as u8;
    v[5] = page;
    v[6] = cmd;
    v[7..7 + payload.len()].copy_from_slice(payload);
    v
}

fn cfg(pid: u16, product: Option<&str>) -> HidInfo {
    let mut h = hid_info(&format!("cfg{pid:04x}"), 0x36a7, pid, 0xFF02, 0x01, (0, 0, 65));
    h.product = product.map(|s| s.to_string());
    h
}

// ---------------------------------------------------------------- encode: reads

#[test]
fn reads_are_openmouses_compax_requests() {
    // READ.firmware {target 2, page 0, 0x81, length 0x10, no args}
    assert_eq!(wl::report(&wl::read_firmware(), 65), rq(0x10, 0x00, 0x81, &[]));
    // READ.battery {2, 0, 0x83, length 2}
    assert_eq!(wl::report(&wl::read_battery(), 65), rq(0x02, 0x00, 0x83, &[]));
    // READ.activeProfile {2, 0, 0x85, length 1}
    assert_eq!(wl::report(&wl::read_active_profile(), 65), rq(0x01, 0x00, 0x85, &[]));
    // PROFILE_READ.dpiStages(1): page 1, 0x81, length 0x0a, args [profile, DPI_STAGE_MAX = 6]
    assert_eq!(wl::report(&wl::read_dpi_stages(1), 65), rq(0x0a, 0x01, 0x81, &[0x01, 0x06]));
    assert_eq!(wl::report(&wl::read_dpi_stages(3), 65), rq(0x0a, 0x01, 0x81, &[0x03, 0x06]));
    // activeStage 0x82, pollingRate 0x80, liftOffDistance 0x88: page 1, length 2, args [profile]
    assert_eq!(wl::report(&wl::read_active_stage(2), 65), rq(0x02, 0x01, 0x82, &[0x02]));
    assert_eq!(wl::report(&wl::read_polling_rate(2), 65), rq(0x02, 0x01, 0x80, &[0x02]));
    assert_eq!(wl::report(&wl::read_lift_off(2), 65), rq(0x02, 0x01, 0x88, &[0x02]));
    // a longer feature report (a collection that is bigger than the packet) is padded with zeros
    let long = wl::report(&wl::read_battery(), 91);
    assert_eq!((long.len(), &long[..65]), (91, &rq(0x02, 0x00, 0x83, &[])[..]));
    assert!(long[65..].iter().all(|b| *b == 0));
}

// ---------------------------------------------------------------- encode: writes

#[test]
fn polling_writes_are_openmouses_codes() {
    // write(PAGE.profile, WRITE.pollingRate = 0x00, profile, [code]): args [profile, code], length 2.
    // POLLING_RATES.find(hz) takes the FIRST code of a rate: 125 08, 250 04, 500 02, 1000 01 (not 10), 2000 20, 4000 40, 8000 80
    for (hz, code) in [(125, 0x08), (250, 0x04), (500, 0x02), (1000, 0x01), (2000, 0x20), (4000, 0x40), (8000, 0x80)] {
        assert_eq!(wl::polling_code(hz), Some(code), "{hz}");
        assert_eq!(wl::report(&wl::write_spec(1, 0x00, 1, &[code]), 65), rq(0x02, 0x01, 0x00, &[0x01, code]), "{hz}");
    }
    assert_eq!(wl::polling_code(3000), None);
    // a profile other than 1: args [3, code]
    assert_eq!(wl::report(&wl::write_spec(1, 0x00, 3, &[0x40]), 65), rq(0x02, 0x01, 0x00, &[0x03, 0x40]));
}

#[test]
fn lift_off_writes_are_openmouses_codes() {
    // LIFT_OFF_DISTANCES: Low 0x87, Medium 0x01, High 0x02; WRITE.liftOffDistance = 0x08 on the profile page.
    // compaxDecodeLiftOff: 0x87 has bit 0x80 -> (0x87 & 0x7f) / 10 = 0.7 mm (Low); 0x01 = 1 mm (Medium); 0x02 = 2 mm (High)
    assert_eq!((wl::lift_off_code(7), wl::lift_off_code(10), wl::lift_off_code(20)), (Some(0x87), Some(0x01), Some(0x02)));
    assert_eq!(wl::lift_off_code(15), None);
    assert_eq!(wl::report(&wl::write_spec(1, 0x08, 1, &[0x87]), 65), rq(0x02, 0x01, 0x08, &[0x01, 0x87]));
    assert_eq!(wl::report(&wl::write_spec(1, 0x08, 2, &[0x02]), 65), rq(0x02, 0x01, 0x08, &[0x02, 0x02]));
}

#[test]
fn the_dpi_table_is_written_back_whole_big_endian() {
    // writeStages: write(PAGE.profile, WRITE.dpiStages = 0x01, profile, [count, x>>8, x&ff, y>>8, y&ff, ...]).
    // 800 = 0x0320, 2400 = 0x0960, 3200 = 0x0c80: three stages (800, 2400, 3200), profile 1:
    // args = [01 | 03 | 03 20 03 20 | 09 60 09 60 | 0c 80 0c 80] = 14 bytes -> length 0x0e
    let table = wl::stage_table_bytes(&[(800, 800), (2400, 2400), (3200, 3200)]);
    assert_eq!(table, vec![0x03, 0x03, 0x20, 0x03, 0x20, 0x09, 0x60, 0x09, 0x60, 0x0c, 0x80, 0x0c, 0x80]);
    assert_eq!(wl::report(&wl::write_spec(1, 0x01, 1, &table), 65), rq(0x0e, 0x01, 0x01, &[0x01, 0x03, 0x03, 0x20, 0x03, 0x20, 0x09, 0x60, 0x09, 0x60, 0x0c, 0x80, 0x0c, 0x80]));
    // separate axes survive as read: (800, 1600): 03 20 06 40
    assert_eq!(wl::stage_table_bytes(&[(800, 1600)]), vec![0x01, 0x03, 0x20, 0x06, 0x40]);
}

// ---------------------------------------------------------------- decode

#[test]
fn answers_decode_like_openmouse() {
    let spec = wl::read_polling_rate(1);
    // OpenMouse's data: [0xa1 ok, 0, 2, length 2, page 1, command 0x80, payload: profile, code]
    let ok = [0xa1, 0x00, 0x02, 0x02, 0x01, 0x80, 0x01, 0x04];
    assert_eq!(wl::decode_reply(&spec, &ok), wl::Reply::Ok(vec![0x01, 0x04]));
    // the payload is cut to the declared length (the zero padding after it is not part of it)
    let mut padded = ok.to_vec();
    padded.resize(64, 0);
    assert_eq!(wl::decode_reply(&spec, &padded), wl::Reply::Ok(vec![0x01, 0x04]));
    // the frame shifted by one byte (FRAME_OFFSETS [0, 1]): status at 1, length at 4, page 5, command 6, payload from 7
    let shifted = [0x00, 0xa1, 0x00, 0x02, 0x02, 0x01, 0x80, 0x01, 0x04];
    assert_eq!(wl::decode_reply(&spec, &shifted), wl::Reply::Ok(vec![0x01, 0x04]));
    // status 0xa2 = unsupported; 0xa0 pending / 0xa3 busy / 0 = ask again; another command's answer = ask again
    let mut un = ok;
    un[0] = 0xa2;
    assert_eq!(wl::decode_reply(&spec, &un), wl::Reply::Unsupported);
    for s in [0xa0, 0xa3, 0x00] {
        let mut b = ok;
        b[0] = s;
        assert_eq!(wl::decode_reply(&spec, &b), wl::Reply::Skip, "status {s:#x}");
    }
    let mut other = ok;
    other[5] = 0x88;
    assert_eq!(wl::decode_reply(&spec, &other), wl::Reply::Skip);
    assert_eq!(wl::decode_reply(&spec, &[]), wl::Reply::Skip);
}

#[test]
fn values_decode_like_openmouse() {
    // polling table: 08 125, 04 250, 02 500, 01 1000, 10 1000, 20 2000, 40 4000, 80 8000; anything else unknown
    for (c, hz) in [(0x08, 125), (0x04, 250), (0x02, 500), (0x01, 1000), (0x10, 1000), (0x20, 2000), (0x40, 4000), (0x80, 8000)] {
        assert_eq!(wl::polling_hz(c), Some(hz), "{c:#x}");
    }
    assert_eq!(wl::polling_hz(0x03), None);
    // compaxDecodeLiftOff: 0 -> null; 0x87 -> 0.7 mm Low; 0x01 -> 1 mm Medium; 0x02 -> 2 mm High. OpenMouse knows only the level, so a
    // code that is not one of the three it writes (0x03 = 3 mm, 0x85 = 0.5 mm, 0x8f = 1.5 mm, 0x80 = 0 mm) is not rounded into a level: None
    for (c, t) in [(0x87, Some(7)), (0x01, Some(10)), (0x02, Some(20)), (0x03, None), (0x85, None), (0x8f, None), (0x80, None), (0x00, None)] {
        assert_eq!(wl::lift_off_tenths(c), t, "{c:#x}");
    }
    // compaxDecodeDpiStages: payload [profile, count, x hi, x lo, y hi, y lo, ...]
    let p = [0x01, 0x03, 0x03, 0x20, 0x03, 0x20, 0x06, 0x40, 0x06, 0x40, 0x0c, 0x80, 0x0f, 0xa0];
    assert_eq!(wl::decode_dpi_stages(&p), vec![(800, 800), (1600, 1600), (3200, 4000)]);
    // a payload cut short ends the list (offset + 3 >= length): count 3 but only two whole stages
    assert_eq!(wl::decode_dpi_stages(&p[..10]), vec![(800, 800), (1600, 1600)]);
    assert_eq!(wl::declared_stage_count(&p[..10]), 3);
    assert_eq!(wl::decode_dpi_stages(&[]), vec![]);
    // stageIndex: min(max(reported, 1), max(count, 1)) - 1
    assert_eq!((wl::stage_index(0, 3), wl::stage_index(1, 3), wl::stage_index(3, 3), wl::stage_index(9, 3), wl::stage_index(2, 0)), (0, 0, 2, 2, 0));
}

#[test]
fn the_product_name_cuts_the_polling_rates() {
    // maxPollingRateHz: /(\d+)\s*K\b/i in productName, times 1000; no marker = every rate
    assert_eq!(wl::max_polling_from_name("WLMouse Beast X 4K"), Some(4000));
    assert_eq!(wl::max_polling_from_name("wlmouse 8k receiver"), Some(8000));
    assert_eq!(wl::max_polling_from_name("Beast 1 K"), Some(1000));
    assert_eq!(wl::max_polling_from_name("WLMouse Beast Mini"), None);
    assert_eq!(wl::max_polling_from_name("Model 4Kb"), None, "no word boundary after the K");
    // a number too big for u32 once times 1000 (the old `n * 1000` panicked in a debug build): no limit, not a crash
    assert_eq!(wl::max_polling_from_name("Beast 9999999K"), None);
    assert_eq!(wl::max_polling_from_name("Beast 4294968K"), None);
    assert_eq!(wl::max_polling_from_name("Beast 4294967K"), Some(4_294_967_000));
    assert_eq!(wl::max_polling_from_name(""), None);
    assert_eq!(wl::supported_polling(None), vec![125, 250, 500, 1000, 2000, 4000, 8000]);
    assert_eq!(wl::supported_polling(Some(1000)), vec![125, 250, 500, 1000]);
    assert_eq!(wl::supported_polling(Some(4000)), vec![125, 250, 500, 1000, 2000, 4000]);
    let c = wl::caps(Some(4000));
    assert_eq!((c.polling.last().copied(), c.battery, c.lift_off.clone()), (Some(4000), true, vec![7, 10, 20]));
    // DPI: OpenMouse's options 50..30000 in steps of 50
    let d = c.dpi.unwrap();
    assert_eq!((d.min(), d.max()), (50, 30000));
    assert!(d.allows(50) && d.allows(30000) && d.allows(850) && !d.allows(849) && !d.allows(30050) && !d.allows(0));
}

// ---------------------------------------------------------------- the allow-list

#[test]
fn only_read_requests_pass_the_allow_list() {
    for r in [
        wl::read_firmware(),
        wl::read_battery(),
        wl::read_active_profile(),
        wl::read_dpi_stages(1),
        wl::read_dpi_stages(5),
        wl::read_active_stage(2),
        wl::read_polling_rate(3),
        wl::read_lift_off(1),
    ] {
        assert!(wl::read_request_allowed(&wl::report(&r, 65)), "{r:?}");
        assert!(wl::read_request_allowed(&wl::report(&r, 91)), "{r:?} on a longer report");
    }
    // every write OpenMouse sends for these settings is refused
    for hz in [125, 250, 500, 1000, 2000, 4000, 8000] {
        assert!(!wl::read_request_allowed(&wl::report(&wl::write_spec(1, 0x00, 1, &[wl::polling_code(hz).unwrap()]), 65)), "{hz}");
    }
    for t in [7, 10, 20] {
        assert!(!wl::read_request_allowed(&wl::report(&wl::write_spec(1, 0x08, 1, &[wl::lift_off_code(t).unwrap()]), 65)), "{t}");
    }
    assert!(!wl::read_request_allowed(&wl::report(&wl::write_spec(1, 0x01, 1, &wl::stage_table_bytes(&[(800, 800)])), 65)));
    // the other writes the client has (sleep 0x07 / debounce 0x08 on page 0, active stage 0x02, flags): refused
    for (page, cmd) in [(0, 0x07), (0, 0x08), (1, 0x02), (1, 0x04), (1, 0x09), (1, 0x0a), (1, 0x0b), (1, 0x13), (1, 0x14), (3, 0x01)] {
        assert!(!wl::read_request_allowed(&wl::report(&wl::write_spec(page, cmd, 1, &[1]), 65)), "{page}/{cmd:#x}");
    }
    // a read of the right command but to the dongle target, with junk after it, a profile 0, another report id, too short
    let mut dongle = wl::report(&wl::read_battery(), 65);
    dongle[3] = 0;
    assert!(!wl::read_request_allowed(&dongle));
    let mut junk = wl::report(&wl::read_battery(), 91);
    junk[80] = 1;
    assert!(!wl::read_request_allowed(&junk));
    let mut junk2 = wl::report(&wl::read_battery(), 65);
    junk2[8] = 1;
    assert!(!wl::read_request_allowed(&junk2), "an argument the read does not have");
    let mut id1 = wl::report(&wl::read_battery(), 65);
    id1[0] = 1;
    assert!(!wl::read_request_allowed(&id1));
    assert!(!wl::read_request_allowed(&wl::report(&wl::read_polling_rate(0), 65)), "profile 0 does not exist");
    assert!(!wl::read_request_allowed(&[0, 0, 0]));
    assert!(!wl::read_request_allowed(&[]));
}

// ---------------------------------------------------------------- the device table

#[test]
fn the_table_is_openmouses_supported_list() {
    // supported-devices-data.ts, status "supported": Beast X a883/a884, Beast Mini a885/a886, Beast X Pro a870/a871,
    // Sword X a878/a879, Beast Max a880/a881; receiver / cable from vendors.js WLMOUSE_PRODUCTS (wireless true for the first of each)
    let want: [(u16, &str, bool); 10] = [
        (0xa883, "WLmouse Beast X", true),
        (0xa884, "WLmouse Beast X", false),
        (0xa885, "WLmouse Beast Mini", true),
        (0xa886, "WLmouse Beast Mini", false),
        (0xa870, "WLmouse Beast X Pro", true),
        (0xa871, "WLmouse Beast X Pro", false),
        (0xa878, "WLmouse Sword X", true),
        (0xa879, "WLmouse Sword X", false),
        (0xa880, "WLmouse Beast Max", true),
        (0xa881, "WLmouse Beast Max", false),
    ];
    assert_eq!(wl::PRODUCTS.len(), want.len());
    for (pid, model, wireless) in want {
        let c = cfg(pid, None);
        let mouse = hid_info("mouse", 0x36a7, pid, 0x01, 0x02, (8, 0, 0));
        let m = wl::matches(&[&mouse, &c], 0x36a7, pid).unwrap_or_else(|| panic!("{pid:04x}"));
        assert_eq!((m.family, m.model, m.wireless, m.variant), (Family::WlMouse, Some(model), Some(wireless), pid), "{pid:04x}");
        assert_eq!(m.config.path, c.path, "{pid:04x}: the collection with the 64-byte feature report");
        assert_eq!(bu_mouse::family::find(&[mouse, c.clone()], 0x36a7, pid).map(|m| m.family), Some(Family::WlMouse), "{pid:04x}");
    }
}

#[test]
fn the_matcher_claims_nothing_else() {
    // driver knows, list does not: Beast X 4K (a887, "pr", another protocol), the 1K receiver (a882), Beast G / Huan / Beast Miao /
    // Beast Mini Pro / Strider / Ying
    for pid in [0xa887, 0xa882, 0xa860, 0xa861, 0xa863, 0xa864, 0xa866, 0xa867, 0xa868, 0xa869, 0xa872, 0xa873, 0xa874, 0xa875, 0x0001] {
        assert!(wl::matches(&[&cfg(pid, None)], 0x36a7, pid).is_none(), "{pid:04x}");
    }
    assert!(wl::matches(&[&cfg(0xa883, None)], 0x3710, 0xa883).is_none(), "another vendor");
    // OpenMouse's isSupported wants a feature report on id 0: no feature report, or one too short, is not the channel
    let mouse_only = hid_info("m", 0x36a7, 0xa883, 0x01, 0x02, (8, 0, 0));
    assert!(wl::matches(&[&mouse_only], 0x36a7, 0xa883).is_none());
    let short = hid_info("s", 0x36a7, 0xa883, 0xFF02, 0x01, (0, 0, 33));
    assert!(wl::matches(&[&short], 0x36a7, 0xa883).is_none());
    // of two fitting collections, the vendor-defined one
    let mut generic = cfg(0xa884, None);
    generic.usage_page = 0x01;
    generic.path = "generic".into();
    let vendor = cfg(0xa884, None);
    assert_eq!(wl::matches(&[&generic, &vendor], 0x36a7, 0xa884).map(|m| m.config.path), Some(vendor.path.clone()));
    // no overlap with the other new tables
    for pid in [0xa883u16, 0xa870, 0xa881] {
        let c = cfg(pid, None);
        assert!(bu_mouse::razer::matches(&[&c], 0x36a7, pid).is_none() && bu_mouse::steelseries::matches(&[&c], 0x36a7, pid).is_none());
        assert!(bu_mouse::vaxee::matches(&[&c], 0x36a7, pid).is_none() && bu_mouse::corsair::matches(&[&c], 0x36a7, pid).is_none());
    }
}

// ---------------------------------------------------------------- the driver against a scripted mouse

/// A WLMouse as OpenMouse's client sees it: a profile with a stage table, polling, lift-off, battery.
#[derive(Default)]
struct Mouse {
    awake: bool,
    profile: u8,
    stages: Vec<(u16, u16)>,
    /// the 1-based active stage
    active: u8,
    poll: u8,
    lift: u8,
    charging: u8,
    percent: u8,
    /// every write seen (OpenMouse's byte i = index i + 1)
    writes: Vec<Vec<u8>>,
    /// polling writes are ignored (the mouse keeps its rate)
    ignore_polling: bool,
    /// commands (page, command) answered "unsupported"
    unsupported: Vec<(u8, u8)>,
    /// answer "pending" this many times first
    pending: u32,
    /// Order 110: debounce ms; the flags Motion Sync, angle snapping, ripple, turbo; their writes have no effect
    debounce: u8,
    flags: [u8; 4],
    ignore_extra: bool,
}

impl Mouse {
    fn new() -> Self {
        Mouse { awake: true, profile: 1, stages: vec![(800, 800), (1600, 1600), (3200, 3200)], active: 2, poll: 0x01, lift: 0x01, charging: 0, percent: 77, debounce: 4, flags: [1, 0, 1, 0], ..Default::default() }
    }
    fn reply(&mut self, req: &[u8]) -> Vec<u8> {
        if !self.awake {
            return vec![0u8; 65];
        }
        let (target, len, page, cmd) = (req[3], req[4], req[5], req[6]);
        assert_eq!(target, 0x02, "every request goes to the mouse");
        let args = &req[7..7 + 8.max(len as usize)];
        if self.pending > 0 {
            self.pending -= 1;
            return answer(0xa0, page, cmd, &[]);
        }
        if self.unsupported.contains(&(page, cmd)) {
            return answer(0xa2, page, cmd, &[]);
        }
        let p = self.profile;
        match (page, cmd) {
            (0, 0x81) => answer(0xa1, page, cmd, &[0, 0, 1, 2]),
            (0, 0x83) => answer(0xa1, page, cmd, &[self.charging, self.percent]),
            (0, 0x85) => answer(0xa1, page, cmd, &[p]),
            (1, 0x81) => {
                let mut pl = vec![p, self.stages.len() as u8];
                for (x, y) in &self.stages {
                    pl.extend_from_slice(&x.to_be_bytes());
                    pl.extend_from_slice(&y.to_be_bytes());
                }
                answer(0xa1, page, cmd, &pl)
            }
            (1, 0x82) => answer(0xa1, page, cmd, &[p, self.active]),
            (1, 0x80) => answer(0xa1, page, cmd, &[p, self.poll]),
            (1, 0x88) => answer(0xa1, page, cmd, &[p, self.lift]),
            (0, 0x88) => answer(0xa1, page, cmd, &[p, self.debounce]),
            (1, 0x89) => answer(0xa1, page, cmd, &[p, self.flags[0]]),
            (1, 0x84) => answer(0xa1, page, cmd, &[p, self.flags[1]]),
            (1, 0x8a) => answer(0xa1, page, cmd, &[p, self.flags[2]]),
            (1, 0x93) => answer(0xa1, page, cmd, &[p, self.flags[3]]),
            (0, 0x08) | (1, 0x09) | (1, 0x04) | (1, 0x0a) | (1, 0x13) => {
                self.writes.push(req[..7 + len as usize].to_vec());
                if args[0] == p && !self.ignore_extra {
                    match (page, cmd) {
                        (0, _) => self.debounce = args[1],
                        (_, 0x09) => self.flags[0] = args[1],
                        (_, 0x04) => self.flags[1] = args[1],
                        (_, 0x0a) => self.flags[2] = args[1],
                        _ => self.flags[3] = args[1],
                    }
                }
                answer(0xa1, page, cmd, &[])
            }
            // writes: the mouse applies them to the active profile only
            (1, 0x00) | (1, 0x08) | (1, 0x01) => {
                self.writes.push(req[..7 + len as usize].to_vec());
                if args[0] == p {
                    match cmd {
                        0x00 if !self.ignore_polling => self.poll = args[1],
                        0x08 => self.lift = args[1],
                        0x01 => {
                            self.stages = (0..args[1] as usize).map(|i| (u16::from_be_bytes([args[2 + i * 4], args[3 + i * 4]]), u16::from_be_bytes([args[4 + i * 4], args[5 + i * 4]]))).collect();
                        }
                        _ => {}
                    }
                }
                answer(0xa1, page, cmd, &[])
            }
            other => panic!("a request OpenMouse never sends here: {other:?}"),
        }
    }
}

fn port(info: HidInfo, mouse: &Rc<RefCell<Mouse>>) -> ScriptPort {
    let m = mouse.clone();
    ScriptPort::new(info, move |out, how| match how {
        HidTransfer::Feature { reply_id: 0, reply_len, wait_ms: 30 } => {
            assert_eq!((out.len(), *reply_len, out[0]), (65, 65, 0));
            let r = m.borrow_mut().reply(out);
            Ok(r)
        }
        other => panic!("WLmouse sends feature reports only, not {other:?}"),
    })
}

fn setup(pid: u16, product: Option<&str>, mouse: &Rc<RefCell<Mouse>>) -> (bu_mouse::family::Matched, ScriptPort) {
    let info = cfg(pid, product);
    let m = wl::matches(&[&info], 0x36a7, pid).unwrap();
    let p = port(info, mouse);
    (m, p)
}

#[test]
fn reading_asks_only_what_openmouse_asks() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    let (m, mut p) = setup(0xa883, Some("WLmouse Beast X 4K"), &mouse);
    let r = wl::WlMouse.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(p.sent.iter().all(|s| wl::read_request_allowed(&s.0)), "read requests only");
    // firmware (is it answering), battery, active profile, stage table, active stage, polling, lift-off - OpenMouse's order
    let want = vec![
        rq(0x10, 0, 0x81, &[]),
        rq(0x02, 0, 0x83, &[]),
        rq(0x01, 0, 0x85, &[]),
        rq(0x0a, 1, 0x81, &[1, 6]),
        rq(0x02, 1, 0x82, &[1]),
        rq(0x02, 1, 0x80, &[1]),
        rq(0x02, 1, 0x88, &[1]),
        // Order 110: debounce (device page 0x88), angle snapping 0x84, Motion Sync 0x89, ripple 0x8a, turbo 0x93
        rq(0x02, 0, 0x88, &[1]),
        rq(0x02, 1, 0x84, &[1]),
        rq(0x02, 1, 0x89, &[1]),
        rq(0x02, 1, 0x8a, &[1]),
        rq(0x02, 1, 0x93, &[1]),
    ];
    assert_eq!(p.sent.iter().map(|s| s.0.clone()).collect::<Vec<_>>(), want);
    assert!(p.sent.iter().all(|s| s.1 == HidTransfer::Feature { reply_id: 0, reply_len: 65, wait_ms: 30 }));
    // stage 2 is active: 1600 DPI; polling code 01 = 1000 Hz; lift code 01 = 1 mm (Medium); battery [0 = not charging, 77]
    assert_eq!((r.online, r.dpi, r.polling_hz, r.lift_off, r.battery_percent, r.charging, r.wireless), (true, Some(1600), Some(1000), Some(10), Some(77), Some(false), Some(true)));
    assert_eq!(r.caps.polling, vec![125, 250, 500, 1000, 2000, 4000], "the 4K in the product name");
    assert_eq!(r.caps.lift_off, vec![7, 10, 20]);
    assert!(r.caps.battery && r.caps.dpi.is_some());
    // a charging cable mouse on another profile, 0x87 lift-off, 0x10 polling (the second 1000 Hz code)
    {
        let mut mm = mouse.borrow_mut();
        mm.profile = 3;
        mm.charging = 1;
        mm.percent = 255;
        mm.lift = 0x87;
        mm.poll = 0x10;
    }
    let (m, mut p) = setup(0xa884, None, &mouse);
    let r = wl::WlMouse.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!(p.sent[3].0, rq(0x0a, 1, 0x81, &[3, 6]), "profile commands carry the ACTIVE profile");
    assert_eq!((r.charging, r.battery_percent, r.lift_off, r.polling_hz, r.wireless), (Some(true), None, Some(7), Some(1000), Some(false)));
    assert_eq!(r.caps.polling.last(), Some(&8000), "no K marker: every rate");
}

#[test]
fn a_command_the_mouse_does_not_know_leaves_that_value_out() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    mouse.borrow_mut().unsupported = vec![(1, 0x88), (0, 0x83)];
    let (m, mut p) = setup(0xa871, None, &mouse);
    let r = wl::WlMouse.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.online, r.lift_off, r.battery_percent, r.dpi, r.polling_hz), (true, None, None, Some(1600), Some(1000)));
}

#[test]
fn a_lift_off_code_that_is_not_one_of_the_three_levels_is_not_shown() {
    // 0x03 = 3 mm and 0x85 = 0.5 mm are not 2 mm / 0.7 mm: the lift-off row stays empty rather than showing a rounded figure
    for code in [0x03, 0x85, 0x80] {
        let mouse = Rc::new(RefCell::new(Mouse::new()));
        mouse.borrow_mut().lift = code;
        let (m, mut p) = setup(0xa883, None, &mouse);
        let r = wl::WlMouse.read(&m, &mut p, ReadOptions::default()).unwrap();
        assert_eq!((r.online, r.lift_off, r.dpi), (true, None, Some(1600)), "{code:#x}");
    }
}

#[test]
fn a_sleeping_mouse_is_offline_and_online_tries_is_honoured() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    mouse.borrow_mut().awake = false;
    let (m, mut p) = setup(0xa883, None, &mouse);
    // the read-only proof asks once
    let r = wl::WlMouse.read(&m, &mut p, ReadOptions { online_tries: 1 }).unwrap();
    assert_eq!((r.online, r.battery_percent, r.dpi, r.polling_hz, r.lift_off), (false, None, None, None, None));
    assert_eq!(p.sent.len(), 1);
    assert_eq!(p.sent[0].0, rq(0x10, 0, 0x81, &[]));
    assert!(r.caps.dpi.is_some(), "the caps are the model's even when it sleeps");
    // the normal read is OpenMouse's 12 tries, then gives up
    let (m, mut p) = setup(0xa883, None, &mouse);
    let r = wl::WlMouse.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.online, p.sent.len()), (false, 12));
    // it wakes during the tries: a "pending" answer is asked again
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    mouse.borrow_mut().pending = 2;
    let (m, mut p) = setup(0xa883, None, &mouse);
    let r = wl::WlMouse.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(r.online && r.dpi == Some(1600));
    assert_eq!(p.sent.iter().filter(|s| s.0 == rq(0x10, 0, 0x81, &[])).count(), 3, "two pending answers, then ok");
    // a write to a sleeping mouse fails (nothing was written)
    mouse.borrow_mut().awake = false;
    let (m, mut p) = setup(0xa883, None, &mouse);
    assert!(matches!(wl::WlMouse.write(&m, &mut p, Setting::Polling(500)), Err(Error::BadAnswer(_))));
    assert!(mouse.borrow().writes.is_empty());
}

#[test]
fn polling_is_changed_the_openmouse_way() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    let (m, mut p) = setup(0xa883, Some("WLmouse Beast X 4K"), &mouse);
    let c = wl::WlMouse.write(&m, &mut p, Setting::Polling(4000)).unwrap();
    assert_eq!((c.now, c.before), (4000, Some(1000)));
    // currentProfile (read), [the read for undo], the write, the read-back
    let sent: Vec<Vec<u8>> = p.sent.iter().map(|s| s.0.clone()).collect();
    assert_eq!(sent, vec![rq(0x01, 0, 0x85, &[]), rq(0x02, 1, 0x80, &[1]), rq(0x02, 1, 0x00, &[1, 0x40]), rq(0x02, 1, 0x80, &[1])]);
    assert_eq!(mouse.borrow().poll, 0x40);
    // the active profile is 4: the write goes to profile 4 (a write to profile 1 would be silently ignored)
    mouse.borrow_mut().profile = 4;
    let (m, mut p) = setup(0xa883, Some("WLmouse Beast X 4K"), &mouse);
    wl::WlMouse.write(&m, &mut p, Setting::Polling(250)).unwrap();
    assert_eq!(p.sent[2].0, rq(0x02, 1, 0x00, &[4, 0x04]));
    assert_eq!(mouse.borrow().poll, 0x04);
    // 8000 Hz is above the "4K" in the product name: refused before a byte is sent
    let (m, mut p) = setup(0xa883, Some("WLmouse Beast X 4K"), &mouse);
    assert!(matches!(wl::WlMouse.write(&m, &mut p, Setting::Polling(8000)), Err(Error::OutOfRange { .. })));
    assert!(wl::WlMouse.write(&m, &mut p, Setting::Polling(750)).is_err());
    assert!(p.sent.is_empty());
}

#[test]
fn a_mouse_that_keeps_its_rate_is_an_error() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    mouse.borrow_mut().ignore_polling = true;
    let (m, mut p) = setup(0xa883, None, &mouse);
    // OpenMouse: "The mouse kept 1000 Hz instead of 500 Hz."
    let e = wl::WlMouse.write(&m, &mut p, Setting::Polling(500)).unwrap_err();
    assert!(matches!(e, Error::BadAnswer(_)), "{e:?}");
    // a command the mouse calls unsupported
    mouse.borrow_mut().unsupported = vec![(1, 0x00)];
    let (m, mut p) = setup(0xa883, None, &mouse);
    assert!(matches!(wl::WlMouse.write(&m, &mut p, Setting::Polling(500)), Err(Error::BadAnswer(_))));
}

#[test]
fn lift_off_is_changed_the_openmouse_way() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    let (m, mut p) = setup(0xa885, None, &mouse);
    let c = wl::WlMouse.write(&m, &mut p, Setting::LiftOff(7)).unwrap();
    assert_eq!((c.now, c.before), (7, Some(10)));
    let sent: Vec<Vec<u8>> = p.sent.iter().map(|s| s.0.clone()).collect();
    assert_eq!(sent, vec![rq(0x01, 0, 0x85, &[]), rq(0x02, 1, 0x88, &[1]), rq(0x02, 1, 0x08, &[1, 0x87]), rq(0x02, 1, 0x88, &[1])]);
    assert_eq!(mouse.borrow().lift, 0x87);
    let (m, mut p) = setup(0xa885, None, &mouse);
    assert_eq!(wl::WlMouse.write(&m, &mut p, Setting::LiftOff(20)).unwrap().before, Some(7));
    assert_eq!(mouse.borrow().lift, 0x02);
    // a distance that is not one of the three levels: refused, nothing sent
    let (m, mut p) = setup(0xa885, None, &mouse);
    assert!(wl::WlMouse.write(&m, &mut p, Setting::LiftOff(15)).is_err());
    assert!(p.sent.is_empty());
}

#[test]
fn dpi_rewrites_the_table_with_only_the_active_stage_changed() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    let (m, mut p) = setup(0xa883, None, &mouse);
    // setDpi(2400): stages (800, 1600, 3200), active 2 -> (800, 2400, 3200), written whole
    let c = wl::WlMouse.write(&m, &mut p, Setting::Dpi(2400)).unwrap();
    assert_eq!((c.now, c.before), (2400, Some(1600)));
    let sent: Vec<Vec<u8>> = p.sent.iter().map(|s| s.0.clone()).collect();
    // currentProfile, readStages, activeStage, [write], readStages. 800 = 03 20, 2400 = 09 60, 3200 = 0c 80
    let table = [0x01, 0x03, 0x03, 0x20, 0x03, 0x20, 0x09, 0x60, 0x09, 0x60, 0x0c, 0x80, 0x0c, 0x80];
    assert_eq!(sent, vec![rq(0x01, 0, 0x85, &[]), rq(0x0a, 1, 0x81, &[1, 6]), rq(0x02, 1, 0x82, &[1]), rq(0x0e, 1, 0x01, &table), rq(0x0a, 1, 0x81, &[1, 6])]);
    assert_eq!(mouse.borrow().stages, vec![(800, 800), (2400, 2400), (3200, 3200)]);
    assert_eq!(mouse.borrow().active, 2, "the active stage is not touched");
    // the active stage is the first and the table has six stages: 50 DPI (00 32) and 30000 (75 30) at the ends
    {
        let mut mm = mouse.borrow_mut();
        mm.stages = vec![(400, 400), (800, 800), (1200, 1200), (1600, 1600), (2000, 2000), (2400, 2400)];
        mm.active = 1;
    }
    let (m, mut p) = setup(0xa883, None, &mouse);
    wl::WlMouse.write(&m, &mut p, Setting::Dpi(50)).unwrap();
    assert_eq!(mouse.borrow().stages[0], (50, 50));
    assert_eq!(p.sent[3].0[5..9], [0x01, 0x01, 0x01, 0x06], "page 1, command 1, profile 1, count 6");
    assert_eq!(p.sent[3].0[9..13], [0x00, 0x32, 0x00, 0x32]);
    wl::WlMouse.write(&m, &mut p, Setting::Dpi(30000)).unwrap();
    assert_eq!(mouse.borrow().stages[0], (30000, 30000));
    // 7530 hex = 30000
    let last_write = p.sent.iter().rev().find(|s| s.0[5] == 0x01 && s.0[6] == 0x01).unwrap();
    assert_eq!(last_write.0[9..13], [0x75, 0x30, 0x75, 0x30]);
}

#[test]
fn dpi_is_held_back_when_the_table_cannot_be_copied_back() {
    // separate X / Y on the active stage: OpenMouse's setDpi would flatten Y -> refused, nothing written
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    mouse.borrow_mut().stages = vec![(800, 800), (1600, 3200), (3200, 3200)];
    let (m, mut p) = setup(0xa883, None, &mouse);
    assert!(matches!(wl::WlMouse.write(&m, &mut p, Setting::Dpi(2400)), Err(Error::OutOfRange { .. })));
    assert!(mouse.borrow().writes.is_empty());
    // another stage having separate axes is copied back as read: fine
    mouse.borrow_mut().stages = vec![(800, 1600), (1600, 1600), (3200, 3200)];
    let (m, mut p) = setup(0xa883, None, &mouse);
    wl::WlMouse.write(&m, &mut p, Setting::Dpi(2400)).unwrap();
    assert_eq!(mouse.borrow().stages, vec![(800, 1600), (2400, 2400), (3200, 3200)]);
    // an answer holding fewer stages than its count (cut short): a write would shrink the table -> refused
    struct Cut;
    impl Cut {
        fn port(info: HidInfo) -> ScriptPort {
            ScriptPort::new(info, |out, _| {
                let (page, cmd) = (out[5], out[6]);
                Ok(match (page, cmd) {
                    (0, 0x85) => answer(0xa1, page, cmd, &[1]),
                    // count 6 but only 10 payload bytes: two whole stages
                    (1, 0x81) => answer(0xa1, page, cmd, &[1, 6, 0x03, 0x20, 0x03, 0x20, 0x06, 0x40, 0x06, 0x40]),
                    (1, 0x82) => answer(0xa1, page, cmd, &[1, 1]),
                    other => panic!("nothing may be written: {other:?}"),
                })
            })
        }
    }
    let info = cfg(0xa883, None);
    let m = wl::matches(&[&info], 0x36a7, 0xa883).unwrap();
    let mut p = Cut::port(info);
    assert!(matches!(wl::WlMouse.write(&m, &mut p, Setting::Dpi(2400)), Err(Error::BadAnswer(_))));
    // values the table cannot hold
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    let (m, mut p) = setup(0xa883, None, &mouse);
    for bad in [0, 49, 75, 30050, 100000] {
        assert!(matches!(wl::WlMouse.write(&m, &mut p, Setting::Dpi(bad)), Err(Error::OutOfRange { .. })), "{bad}");
    }
    assert!(p.sent.is_empty());
}

#[test]
fn a_mouse_that_does_not_keep_the_dpi_is_an_error() {
    // the mouse answers the write but its table is unchanged afterwards (profile mismatch: the write went to profile 1 while
    // the mouse reads profile 2's table) - OpenMouse: "The mouse kept ... DPI instead of ..."
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    let mm = mouse.clone();
    let info = cfg(0xa883, None);
    let m = wl::matches(&[&info], 0x36a7, 0xa883).unwrap();
    let mut flip = 0;
    let mut p = ScriptPort::new(info, move |out, _| {
        let r = mm.borrow_mut().reply(out);
        // a stale answer to the active-profile read the second time: the client then writes to a profile the mouse ignores
        if out[5] == 0 && out[6] == 0x85 {
            flip += 1;
            if flip == 1 {
                mm.borrow_mut().profile = 2;
                return Ok(answer(0xa1, 0, 0x85, &[1]));
            }
        }
        Ok(r)
    });
    let e = wl::WlMouse.write(&m, &mut p, Setting::Dpi(2400)).unwrap_err();
    assert!(matches!(e, Error::BadAnswer(_)), "{e:?}");
}

// ---------------------------------------------------------------- Order 110: debounce, Motion Sync, angle snapping, ripple, turbo

use bu_mouse::family::{Change, Extra, Kind};

#[test]
fn order_110_read_shows_debounce_and_the_flags_like_openmouse() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    mouse.borrow_mut().profile = 2;
    let (m, mut p) = setup(0xa883, None, &mouse);
    let r = wl::WlMouse.read(&m, &mut p, ReadOptions::default()).unwrap();
    // readStatus: debounceMs = debounce[1]; angleSnapping / motionSync / rippleControl / turboMode = reply[1] === 1
    let sent: Vec<Vec<u8>> = p.sent.iter().map(|s| s.0.clone()).collect();
    assert_eq!(sent[7..], [rq(0x02, 0, 0x88, &[2]), rq(0x02, 1, 0x84, &[2]), rq(0x02, 1, 0x89, &[2]), rq(0x02, 1, 0x8a, &[2]), rq(0x02, 1, 0x93, &[2])]);
    let got: Vec<(Extra, Option<i32>)> = r.extras.iter().map(|x| (x.extra, x.value)).collect();
    assert_eq!(got, vec![(Extra::Debounce, Some(4)), (Extra::MotionSync, Some(1)), (Extra::AngleSnap, Some(0)), (Extra::RippleControl, Some(1)), (Extra::Turbo, Some(0))]);
    assert_eq!(r.extras[0].kind, Kind::Range { min: 0, max: 15, step: 1, unit: "ms" });
    assert!(p.sent.iter().all(|s| wl::read_request_allowed(&s.0)));
    // an unsupported read leaves its row without a value
    mouse.borrow_mut().unsupported = vec![(1, 0x93)];
    let (m, mut p) = setup(0xa883, None, &mouse);
    assert_eq!(wl::WlMouse.read(&m, &mut p, ReadOptions::default()).unwrap().extra(Extra::Turbo), None);
}

#[test]
fn order_110_changes_are_openmouses_setflag_and_setdebouncetime_bytes() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    mouse.borrow_mut().profile = 2;
    let (m, mut p) = setup(0xa883, None, &mouse);
    // currentProfile (0x85), write(page, command, profile, [value]) = length 2 `[profile, value]`, then the read-back
    for (c, write, read) in [
        (Change::Extra(Extra::Debounce, 15), rq(0x02, 0, 0x08, &[2, 15]), rq(0x02, 0, 0x88, &[2])),
        (Change::Extra(Extra::MotionSync, 0), rq(0x02, 1, 0x09, &[2, 0]), rq(0x02, 1, 0x89, &[2])),
        (Change::Extra(Extra::AngleSnap, 1), rq(0x02, 1, 0x04, &[2, 1]), rq(0x02, 1, 0x84, &[2])),
        (Change::Extra(Extra::RippleControl, 0), rq(0x02, 1, 0x0a, &[2, 0]), rq(0x02, 1, 0x8a, &[2])),
        (Change::Extra(Extra::Turbo, 1), rq(0x02, 1, 0x13, &[2, 1]), rq(0x02, 1, 0x93, &[2])),
    ] {
        p.sent.clear();
        wl::WlMouse.change(&m, &mut p, c).unwrap_or_else(|e| panic!("{c:?}: {e}"));
        assert_eq!(p.sent.iter().map(|s| s.0.clone()).collect::<Vec<_>>(), vec![rq(0x01, 0, 0x85, &[]), write.clone(), read], "{c:?}");
        assert!(!wl::read_request_allowed(&write), "{c:?}");
    }
    assert_eq!((mouse.borrow().debounce, mouse.borrow().flags), (15, [0, 1, 0, 1]));
    // turbo needs high-speed mode on the mouse: one that leaves it off is an error, as in OpenMouse
    mouse.borrow_mut().ignore_extra = true;
    assert!(matches!(wl::WlMouse.change(&m, &mut p, Change::Extra(Extra::Turbo, 0)), Err(Error::BadAnswer(_))));
    // refused before a byte leaves
    p.sent.clear();
    for c in [Change::Extra(Extra::Debounce, 16), Change::Extra(Extra::Turbo, 2), Change::Extra(Extra::MotionSync, -1)] {
        assert!(wl::WlMouse.change(&m, &mut p, c).is_err(), "{c:?}");
    }
    assert!(p.sent.is_empty());
}

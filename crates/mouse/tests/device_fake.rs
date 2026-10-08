//! "Your mouse" against the fake: identification from a device list, the cMouse protocol on a byte-level fake mouse
//! (read, set DPI / polling / lift-off with lock → write → read back → unlock, undo), errors, the allow-list.

use bu_mouse::device::*;
use bu_mouse::fake::{CmouseModel, FakeOs};
use bu_mouse::os::HidInfo;
use bu_mouse::pulsar::{self as p, Link};
use bu_mouse::{AppDirs, Error, Mouse};
use std::cell::RefCell;
use std::rc::Rc;

const CFG: &str = r"\\?\hid#vid_3710&pid_5406&mi_01&col05#x";

fn hid(path: &str, vid: u16, pid: u16, page: u16, usage: u16, inl: u16, outl: u16) -> HidInfo {
    HidInfo {
        path: path.into(),
        vid,
        pid,
        version: 0x0305,
        usage_page: page,
        usage,
        input_len: inl,
        output_len: outl,
        feature_len: 0,
        interface: bu_mouse::win::hid::interface_number(path),
        product: Some(if vid == 0x3710 { "Pulsar 8K Dongle".into() } else { "Other Mouse".into() }),
        manufacturer: None,
    }
}

/// The interfaces measured on the real dongle (mouse-show), plus another brand's mouse.
fn pulsar_list() -> Vec<HidInfo> {
    vec![
        hid(r"\\?\hid#vid_3710&pid_5406&mi_00#a", 0x3710, 0x5406, 0x01, 0x02, 8, 0),
        hid(r"\\?\hid#vid_3710&pid_5406&mi_01&col08#b", 0x3710, 0x5406, 0x01, 0x02, 8, 0),
        hid(r"\\?\hid#vid_3710&pid_5406&mi_01&col06#c", 0x3710, 0x5406, 0xFF06, 0x02, 49, 49),
        hid(CFG, 0x3710, 0x5406, 0xFF02, 0x02, 17, 17),
        hid(r"\\?\hid#vid_046d&pid_c547&mi_00#d", 0x046D, 0xC547, 0x01, 0x02, 8, 0),
    ]
}

fn setup() -> (Mouse<FakeOs>, Rc<RefCell<CmouseModel>>, YourMouse) {
    let mut os = FakeOs::new();
    os.hid = pulsar_list();
    let model = Rc::new(RefCell::new(CmouseModel::new()));
    let m2 = model.clone();
    os.mice.insert(CFG.into(), Box::new(move |req| Ok(m2.borrow_mut().answer(req))));
    let m = Mouse::new(os, AppDirs::new("unused"));
    let y = m.mice().unwrap().remove(0);
    (m, model, y)
}

#[test]
fn identifies_the_pulsar_dongle_and_the_other_brand() {
    let (m, _, y) = setup();
    assert_eq!(y.name, "Pulsar X2 CrazyLight");
    assert_eq!((y.vid, y.pid), (0x3710, 0x5406));
    assert_eq!(y.protocol, Some(Protocol::PulsarCmouse));
    assert_eq!(y.config_path.as_deref(), Some(CFG), "the FF02 collection, not FF06");
    assert_eq!(y.sub_line(), "Wireless · saved on the mouse itself");
    assert_eq!(y.link(), Some(("Open Pulsar web settings".into(), "https://bbb.pulsar.gg/")));
    let all = m.mice().unwrap();
    assert_eq!(all.len(), 2);
    let other = &all[1];
    assert_eq!(other.protocol, None);
    assert_eq!(other.name, "Other Mouse");
    assert_eq!(other.sub_line(), "DPI and polling for this mouse aren't supported yet");
    assert_eq!(other.link(), Some(("open its web settings".into(), "https://www.logitechg.com/innovation/g-hub")));
}

#[test]
fn unknown_brand_has_no_link_and_shared_vids_name_no_brand() {
    assert_eq!(brand(0x093A), None, "PixArt id is shared by many brands");
    assert_eq!(brand(0x3554), None, "CompX id is shared");
    let list = vec![hid(r"\\?\hid#vid_1234&pid_0001#z", 0x1234, 0x0001, 0x01, 0x02, 8, 0)];
    let y = find_mice(&list).remove(0);
    assert_eq!(y.link(), None);
    assert_eq!(y.brand, None);
}

#[test]
fn a_dongle_without_its_config_collection_is_not_called_supported() {
    let list = vec![hid(r"\\?\hid#vid_3710&pid_5406&mi_00#a", 0x3710, 0x5406, 0x01, 0x02, 8, 0)];
    let y = find_mice(&list).remove(0);
    assert_eq!(y.protocol, None);
    assert_eq!(y.name, "Pulsar X2 CrazyLight");
}

#[test]
fn the_cable_is_preferred_over_the_dongle() {
    let mut list = pulsar_list();
    list.push(hid(r"\\?\hid#vid_3710&pid_3414&mi_00#w", 0x3710, 0x3414, 0x01, 0x02, 8, 0));
    list.push(hid(r"\\?\hid#vid_3710&pid_3414&mi_01&col05#w2", 0x3710, 0x3414, 0xFF02, 0x02, 17, 17));
    let y = find_mice(&list).remove(0);
    assert_eq!(y.pid, 0x3414);
    assert_eq!(y.wireless, Some(false));
    assert_eq!(y.sub_line(), "Saved on the mouse itself");
}

#[test]
fn reads_everything_with_read_requests_only() {
    let (mut m, model, y) = setup();
    let r = m.read_on_mouse(&y, ReadOptions::default()).unwrap();
    assert_eq!(r.model, Some("Pulsar X2 CrazyLight"));
    assert_eq!(r.link, Link::Wireless8k);
    assert!(r.online);
    assert_eq!((r.battery_percent, r.charging, r.battery_mv), (Some(78), Some(false), Some(3950)));
    assert_eq!(r.stage, Some(1));
    assert_eq!(r.dpi, Some((800, 800)));
    assert_eq!(r.polling_hz, Some(1000));
    assert_eq!(r.lift_off, Some(10));
    // every frame that went out passes the read allow-list, and none changed the mouse
    for req in &model.borrow().seen {
        assert!(p::read_request_allowed(req), "not a read request: {req:02x?}");
    }
    assert_eq!(model.borrow().mem, CmouseModel::new().mem);
}

#[test]
fn offline_mouse_gives_battery_but_no_settings() {
    let (mut m, model, y) = setup();
    model.borrow_mut().online = false;
    let r = m.read_on_mouse(&y, ReadOptions { online_tries: 1 }).unwrap();
    assert!(!r.online);
    assert_eq!(r.battery_percent, Some(78));
    assert_eq!(r.dpi, None);
    let onlines = model.borrow().seen.iter().filter(|s| s[1] == p::CMD_ONLINE).count();
    assert_eq!(onlines, 1, "online_tries 1 = asked once, no loop (A_009_01)");
}

#[test]
fn set_dpi_rounds_writes_the_active_stage_and_undo_puts_it_back() {
    let (mut m, model, y) = setup();
    let before = model.borrow().mem.clone();
    assert_eq!(m.set_dpi(&y, 1234).unwrap(), "DPI 1230 · saved on the mouse");
    let rec = p::encode_dpi_stage(1230).unwrap();
    assert_eq!(&model.borrow().mem[0x10..0x14], &rec, "stage 1 (active) at 0x0C + 4");
    assert_eq!(&model.borrow().mem[0x0C..0x10], &before[0x0C..0x10], "other stages untouched");
    assert!(!model.borrow().locked, "unlocked again");
    // the sequence: online? (query) → announce → read active stage → read old record → lock → write → read back →
    // unlock → release
    let cmds: Vec<(u8, u8, u8)> = model.borrow().seen.iter().map(|s| (s[1], s[5], s[6])).collect();
    let tail: Vec<(u8, u8, u8)> = cmds[cmds.len() - 9..].to_vec();
    assert_eq!(
        tail.iter().map(|c| c.0).collect::<Vec<_>>(),
        vec![p::CMD_ONLINE, p::CMD_DRIVER, p::CMD_READ, p::CMD_READ, p::CMD_ONLINE, p::CMD_WRITE, p::CMD_READ, p::CMD_ONLINE, p::CMD_DRIVER]
    );
    assert_eq!((tail[0].1, tail[4], tail[7]), (0, (p::CMD_ONLINE, 1, 1), (p::CMD_ONLINE, 1, 0)), "query (length 0), lock (1), unlock (0)");
    assert_eq!((tail[1].2, tail[8].2), (1, 0), "announced, then released");
    assert!(!model.borrow().driver_on);
    m.undo_on_mouse(&y, "dpi").unwrap();
    assert_eq!(model.borrow().mem, before);
    assert!(matches!(m.undo_on_mouse(&y, "dpi"), Err(Error::NothingToUndo(_))));
}

#[test]
fn dpi_custom_rules() {
    // Order 042: every value the mouse can store (10-DPI steps up to 10240, 50 up to 25600, 100 up to 32000)
    assert_eq!(custom_dpi(2400), 2400);
    assert_eq!(custom_dpi(1234), 1230);
    assert_eq!(custom_dpi(1236), 1240);
    assert_eq!(custom_dpi(12345), 12350);
    assert_eq!(custom_dpi(26049), 26000);
    assert_eq!(custom_dpi(10), 50);
    assert_eq!(custom_dpi(99999), 26000);
    for d in [50, 2400, 1230, 10240, 12350, 25600, 26000] {
        assert!(bu_mouse::pulsar::encode_dpi_stage(custom_dpi(d)).is_some(), "{d} can be stored");
        assert_eq!(bu_mouse::pulsar::decode_dpi_stage(&bu_mouse::pulsar::encode_dpi_stage(d).unwrap()), Some((d, d)), "{d} reads back");
    }
    assert_eq!(dpi_chip(1600), Some(2));
    assert_eq!(dpi_chip(1250), None, "a typed value lights no chip");
}

#[test]
fn polling_respects_the_link_and_undo_restores() {
    let (mut m, model, y) = setup();
    m.set_polling(&y, 8000, Link::Wireless8k).unwrap();
    assert_eq!(&model.borrow().mem[0..2], &p::scalar_pair(0x40));
    assert!(matches!(m.set_polling(&y, 8000, Link::Wireless4k), Err(Error::OutOfRange { .. })));
    assert!(matches!(m.set_polling(&y, 3000, Link::Wireless8k), Err(Error::OutOfRange { .. })));
    m.undo_on_mouse(&y, "polling").unwrap();
    assert_eq!(&model.borrow().mem[0..2], &p::scalar_pair(0x01));
}

#[test]
fn lift_off_and_undo() {
    let (mut m, model, y) = setup();
    m.set_lift_off(&y, 20).unwrap();
    assert_eq!(&model.borrow().mem[0x0A..0x0C], &p::scalar_pair(0x02));
    assert!(matches!(m.set_lift_off(&y, 15), Err(Error::OutOfRange { .. })));
    m.undo_on_mouse(&y, "lift_off").unwrap();
    assert_eq!(&model.borrow().mem[0x0A..0x0C], &p::scalar_pair(0x01));
}

#[test]
fn bad_answers_and_asleep_mouse_are_errors_and_the_lock_is_always_released() {
    let (mut m, model, y) = setup();
    model.borrow_mut().corrupt_next = true;
    assert!(matches!(m.read_on_mouse(&y, ReadOptions::default()), Err(Error::BadAnswer(_))));
    model.borrow_mut().online = false;
    assert!(matches!(m.set_dpi(&y, 800), Err(Error::MouseGone(_))));
    model.borrow_mut().online = true;
    // a write the mouse refuses (status 1) → error, but the unlock still goes out
    model.borrow_mut().refuse_writes = true;
    let before = model.borrow().mem.clone();
    assert!(matches!(m.set_dpi(&y, 1600), Err(Error::BadAnswer(_))));
    assert_eq!(model.borrow().mem, before);
    assert!(!model.borrow().locked, "unlock sent after the failed write");
    let seen = model.borrow().seen.clone();
    let last_write = seen.iter().rposition(|s| s[1] == p::CMD_WRITE).unwrap();
    assert!(seen[last_write..].iter().any(|s| s[1] == p::CMD_ONLINE && s[5] == 1 && s[6] == 0), "the unlock follows the last write");
    assert_eq!(seen.last().map(|s| (s[1], s[6])), Some((p::CMD_DRIVER, 0)), "the app released at the end");
    // a broken stored stage pair is refused before anything is written
    model.borrow_mut().refuse_writes = false;
    model.borrow_mut().mem[0x04] = 7;
    assert!(m.set_dpi(&y, 800).is_err());
}

#[test]
fn unsupported_mouse_is_refused_without_sending() {
    let mut os = FakeOs::new();
    os.hid = vec![hid(r"\\?\hid#vid_046d&pid_c547&mi_00#d", 0x046D, 0xC547, 0x01, 0x02, 8, 0)];
    let mut m = Mouse::new(os, AppDirs::new("unused"));
    let y = m.mice().unwrap().remove(0);
    assert!(matches!(m.read_on_mouse(&y, ReadOptions::default()), Err(Error::UnsupportedMouse(_))));
    assert!(m.set_dpi(&y, 800).is_err());
    assert!(m.os().log.iter().all(|l| !l.starts_with("hid_exchange")));
}

#[test]
fn admin_or_access_denied_comes_back_typed() {
    let (mut m, _, y) = setup();
    m.os_mut().deny_writes = 1;
    assert!(matches!(m.read_on_mouse(&y, ReadOptions::default()), Err(Error::NeedsAdmin { .. })));
}

#[test]
fn protocol_codecs_match_the_published_samples() {
    // pulsar-mouse-linux verified samples: [x_lo, y_lo, flags]
    for (dpi, s) in [(400, [0x27, 0x27, 0x00]), (800, [0x4F, 0x4F, 0x00]), (1600, [0x9F, 0x9F, 0x00]), (3200, [0x3F, 0x3F, 0x44]), (6400, [0x7F, 0x7F, 0x88]), (10000, [0xE7, 0xE7, 0xCC]), (12800, [0x37, 0x37, 0x22]), (20000, [0xC7, 0xC7, 0x22]), (32000, [0x77, 0x77, 0x33])] {
        let r = p::encode_dpi_stage(dpi).unwrap();
        assert_eq!(&r[..3], &s, "{dpi}");
        assert_eq!(r[3], p::checksum(&s));
        assert_eq!(p::decode_dpi_stage(&r), Some((dpi, dpi)));
    }
    // battery request worked example: 08 04 00 … 00 49
    let f = p::frame(p::CMD_BATTERY, 0, &[]);
    assert_eq!(f[16], 0x49);
    for hz in [125, 250, 500, 1000, 2000, 4000, 8000] {
        assert_eq!(p::polling_hz(p::polling_code(hz).unwrap()), Some(hz));
    }
    assert_eq!(p::lift_off_tenths(0x03), Some(7));
    assert_eq!(Link::from_code(1).max_polling_hz(), 4000);
}

#[test]
fn allow_list_refuses_every_changing_command() {
    assert!(!p::read_request_allowed(&p::frame(p::CMD_WRITE, 0, &[1, 0x54])));
    assert!(!p::read_request_allowed(&p::frame(p::CMD_RESET, 0, &[])));
    assert!(!p::read_request_allowed(&p::frame(p::CMD_SET_PROFILE, 0, &[1])));
    assert!(!p::read_request_allowed(&p::frame(p::CMD_ONLINE, 0, &[1])), "the write lock");
    assert!(!p::read_request_allowed(&p::frame(0x02, 0, &[1])), "anything not listed");
    let mut bad = p::frame(p::CMD_BATTERY, 0, &[]);
    bad[16] ^= 1;
    assert!(!p::read_request_allowed(&bad), "a broken checksum");
    assert!(p::read_request_allowed(&p::frame(p::CMD_ONLINE, 0, &[0])));
    assert!(p::read_request_allowed(&p::read_frame(0x0C, 4)));
    assert!(p::read_request_allowed(&p::identify_frame([1, 2, 3, 4])));
}

/// Test feedback: the polling rate was not shown for a detected mouse, and changing settings only worked on wireless
/// with an error. The app asked "online?" with the HOLD form
/// (length 1), which the real dongle only ACKs (status 1, `[6]` = 0 - measured, Order 009) - so the mouse always counted
/// as asleep. Now: the QUERY form, polled while the mouse links.
#[test]
fn online_uses_the_query_form_and_waits_for_a_waking_mouse() {
    let (mut m, model, y) = setup();
    // the dongle's answer to the hold form says nothing about the mouse
    let hold = model.borrow_mut().answer(&p::hold_frame(false));
    assert_eq!((hold[2], hold[6]), (1, 0));
    model.borrow_mut().seen.clear();
    // a mouse that links after 40 polls (~0.8 s) and is busy twice more
    model.borrow_mut().waking = 40;
    model.borrow_mut().busy = 42;
    let r = m.read_on_mouse(&y, ReadOptions::default()).unwrap();
    assert!(r.online);
    assert_eq!(r.polling_hz, Some(1000));
    let queries: Vec<Vec<u8>> = model.borrow().seen.iter().filter(|s| s[1] == p::CMD_ONLINE).cloned().collect();
    assert_eq!(queries.len(), 43);
    assert!(queries.iter().all(|q| q[5] == 0), "every online question is the query form (length 0)");
    // a change on the waking wireless mouse works too
    model.borrow_mut().waking = 30;
    m.set_polling(&y, 2000, Link::Wireless8k).unwrap();
    assert_eq!(&model.borrow().mem[0..2], &p::scalar_pair(p::polling_code(2000).unwrap()));
}

#[test]
fn a_mouse_that_stays_asleep_is_one_clear_error() {
    let (mut m, model, y) = setup();
    model.borrow_mut().waking = u32::MAX;
    let r = m.read_on_mouse(&y, ReadOptions::default()).unwrap();
    assert!(!r.online && r.polling_hz.is_none());
    match m.set_dpi(&y, 800) {
        Err(Error::MouseGone(t)) => assert!(t.contains("asleep"), "{t}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_dongle_that_wants_the_app_announced_gets_it_for_reads_and_is_released() {
    let (mut m, model, y) = setup();
    model.borrow_mut().needs_driver = true;
    let r = m.read_on_mouse(&y, ReadOptions::default()).unwrap();
    assert_eq!((r.polling_hz, r.dpi, r.lift_off), (Some(1000), Some((800, 800)), Some(10)));
    assert!(!model.borrow().driver_on, "released after the read");
    assert_eq!(model.borrow().mem, CmouseModel::new().mem, "nothing written");
}

#[test]
fn the_query_and_hold_frames() {
    assert_eq!(p::online_query_frame()[5], 0);
    assert_eq!((p::hold_frame(true)[5], p::hold_frame(true)[6]), (1, 1));
    assert!(p::read_request_allowed(&p::online_query_frame()));
    assert!(!p::read_request_allowed(&p::hold_frame(true)));
    assert!(!p::read_request_allowed(&p::frame(p::CMD_DRIVER, 0, &[1])), "the announce is not a read request");
}

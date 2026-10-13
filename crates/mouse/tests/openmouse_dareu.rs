//! Order 105 check, Dareu A950 PRO Mg: our requests = the bytes OpenMouse sends for the same setting (encode), and answers
//! shaped the way OpenMouse reads them decode to the same values (decode). The expected bytes are worked out by hand from
//! OpenMouse (@openmouse/protocol 0.40.1: `dist/dareu/index.js`, `dist/drivers/dareu/hid.js`), never from our own encoder.
//!
//! OpenMouse's frame is `sendReport(8, body)` with a 16-byte body; Windows adds the report id 8 in front, so OpenMouse's body
//! byte i is our byte i + 1: `[1]` command, `[3..5]` address high / low, `[5]` length, `[6..]` data, `[16]` checksum.
//! The checksum (dareuChecksum) is the sum of bytes 0..15 (report id included): `85 - sum` when the sum is under 55, else
//! `341 - sum`, low byte. Every checksum below is worked out in its comment.

use bu_mouse::dareu as d;
use bu_mouse::family::{Driver, Family, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::os::{HidInfo, HidTransfer};
use std::cell::RefCell;
use std::rc::Rc;

/// A full 17-byte frame: id 8, `mid` from byte 1, zeros, the checksum (by hand) in byte 16.
fn fr(mid: &[u8], chk: u8) -> Vec<u8> {
    let mut v = vec![0u8; 17];
    v[0] = 8;
    v[1..1 + mid.len()].copy_from_slice(mid);
    v[16] = chk;
    v
}

fn command_col(pid: u16) -> HidInfo {
    hid_info(&format!("cmd{pid:04x}"), 0x260d, pid, 0xFF02, 0x02, (17, 17, 0))
}

fn service_col(pid: u16) -> HidInfo {
    hid_info(&format!("svc{pid:04x}"), 0x260d, pid, 0xFF05, 0x00, (0, 0, 0))
}

// ---------------------------------------------------------------- the checksum

#[test]
fn the_jm_checksum_follows_the_two_branches() {
    // sum under 55: 85 - sum
    assert_eq!(d::checksum(&[8, 3, 0, 0]), 85 - 11);
    assert_eq!(d::checksum(&[1, 0]), 84, "polling id 1: 85 - 1");
    // sum 55 and over: 341 - sum (low byte)
    assert_eq!(d::checksum(&[15, 15, 0, 0]), 55, "800 DPI record: 85 - 30");
    assert_eq!(d::checksum(&[0x3f, 0x3f, 0, 0]), 215, "3200 DPI record: sum 126 → 341 - 126");
    assert_eq!(d::checksum(&[0x8f, 0x8f, 0x44, 0]), 243, "20000 DPI record: sum 354 → 341 - 354 = -13 → 243");
    // dareuWithMemoryChecksum / dareuHasValidMemoryChecksum
    assert_eq!(d::with_memory_checksum(&[8]), vec![8, 77]);
    assert!(d::has_valid_memory_checksum(&[1, 84]) && !d::has_valid_memory_checksum(&[1, 85]) && !d::has_valid_memory_checksum(&[1]));
}

// ---------------------------------------------------------------- encode: reads

#[test]
fn reads_are_openmouses_frames() {
    // dareuCheckActiveRequest: 8 + 3 = 11 → 85 - 11 = 74 = 0x4a
    assert_eq!(d::check_active_request(), fr(&[3], 0x4a));
    // dareuGetBatteryRequest: 8 + 4 = 12 → 73 = 0x49
    assert_eq!(d::battery_request(), fr(&[4], 0x49));
    // readBufferRequest(0, 2) (polling): 8 + 8 + 2 = 18 → 67 = 0x43
    assert_eq!(d::read_buffer_requests(0, 2), vec![fr(&[8, 0, 0, 0, 2], 0x43)]);
    // readBufferRequest(2, 4) (DPI header): 8 + 8 + 2 + 4 = 22 → 63 = 0x3f
    assert_eq!(d::read_buffer_requests(2, 4), vec![fr(&[8, 0, 0, 2, 4], 0x3f)]);
    // 5 stages = 20 bytes at 12: two ten-byte reads. (12,10): 8+8+12+10 = 38 → 47 = 0x2f. (22,10): 48 → 37 = 0x25
    assert_eq!(d::read_buffer_requests(12, 20), vec![fr(&[8, 0, 0, 12, 10], 0x2f), fr(&[8, 0, 0, 22, 10], 0x25)]);
    // 3 stages = 12 bytes: (12,10) then (22,2): 8+8+22+2 = 40 → 45 = 0x2d
    assert_eq!(d::read_buffer_requests(12, 12), vec![fr(&[8, 0, 0, 12, 10], 0x2f), fr(&[8, 0, 0, 22, 2], 0x2d)]);
}

// ---------------------------------------------------------------- encode: writes

#[test]
fn writes_are_openmouses_frames() {
    // setPollingRate: writeBuffer(0, dareuWithMemoryChecksum([id])): the record [id, 85 - id], then the frame
    // [07 00 | 00 00 | len 2 | id, 85 - id]: the sum is 8 + 7 + 2 + id + (85 - id) = 102 → 341 - 102 = 239 = 0xef for every id
    for (hz, id) in [(125u32, 0x08u8), (250, 0x04), (500, 0x02), (1000, 0x01), (2000, 0x10), (4000, 0x20)] {
        let rec = d::with_memory_checksum(&[d::polling_id(hz, true).unwrap()]);
        assert_eq!(rec, vec![id, 85 - id], "{hz} Hz record");
        assert_eq!(d::write_buffer_requests(0, &rec), vec![fr(&[7, 0, 0, 0, 2, id, 85 - id], 0xef)], "{hz} Hz");
    }
    // writeDpiStage: data [id & 0xff, id & 0xff, id >> 8, checksum] at 12 + stage * 4
    // 800 DPI = 800/50 - 1 = 15 = 0x0f: record sum 30 → 85 - 30 = 55 = 0x37; stage 1 → address 16:
    //   frame sum 8 + 7 + 16 + 4 + 15 + 15 + 0 + 55 = 120 → 341 - 120 = 221 = 0xdd
    assert_eq!(d::dpi_stage_record(800), Some(vec![0x0f, 0x0f, 0x00, 0x37]));
    assert_eq!(d::write_buffer_requests(16, &[0x0f, 0x0f, 0x00, 0x37]), vec![fr(&[7, 0, 0, 16, 4, 0x0f, 0x0f, 0x00, 0x37], 0xdd)]);
    // 3200 DPI = 63 = 0x3f: record sum 126 → 341 - 126 = 215 = 0xd7; stage 0 → address 12:
    //   frame sum 8 + 7 + 12 + 4 + 63 + 63 + 0 + 215 = 372 → 341 - 372 = -31 → 225 = 0xe1
    assert_eq!(d::dpi_stage_record(3200), Some(vec![0x3f, 0x3f, 0x00, 0xd7]));
    assert_eq!(d::write_buffer_requests(12, &[0x3f, 0x3f, 0x00, 0xd7]), vec![fr(&[7, 0, 0, 12, 4, 0x3f, 0x3f, 0x00, 0xd7], 0xe1)]);
    // 20000 DPI = index 399 → (399 - 256) | 0x4400 = 0x448f: record [8f 8f 44], sum 354 → -13 → 243 = 0xf3
    assert_eq!(d::dpi_stage_record(20000), Some(vec![0x8f, 0x8f, 0x44, 0xf3]));
    // 26000 DPI = index 519 → (519 - 512) | 0x8800 = 0x8807: record [07 07 88], sum 150 → 191 = 0xbf
    assert_eq!(d::dpi_stage_record(26000), Some(vec![0x07, 0x07, 0x88, 0xbf]));
    // 100 DPI = index 1: record [01 01 00], sum 2 → 83 = 0x53
    assert_eq!(d::dpi_stage_record(100), Some(vec![0x01, 0x01, 0x00, 0x53]));
    // a write longer than ten bytes would be cut into ten-byte frames (dareuWriteBufferRequests): 12 bytes at 12
    let long = d::write_buffer_requests(12, &[1u8; 12]);
    assert_eq!(long.len(), 2);
    assert_eq!((long[0][4], long[0][5]), (12, 10));
    assert_eq!((long[1][4], long[1][5]), (22, 2));
}

#[test]
fn the_dpi_id_map_is_openmouses() {
    // dareuDpiId: index = dpi / 50 - 1; < 256 as is; < 512 → (index - 256) | 0x4400; else (index - 512) | 0x8800
    assert_eq!(d::dpi_id(100), Some(1));
    assert_eq!(d::dpi_id(12800), Some(0x00ff), "index 255 is the last plain id");
    assert_eq!(d::dpi_id(12850), Some(0x4400), "index 256");
    assert_eq!(d::dpi_id(25600), Some(0x44ff), "index 511");
    assert_eq!(d::dpi_id(25650), Some(0x8800), "index 512");
    for off in [0u32, 99, 125, 26050, 26001] {
        assert_eq!(d::dpi_id(off), None, "{off} is not a Dareu DPI");
    }
    // dareuDpiFromId is its inverse
    for dpi in (100..=26000).step_by(50) {
        assert_eq!(d::dpi_from_id(d::dpi_id(dpi).unwrap()), Some(dpi));
    }
    assert_eq!(d::dpi_from_id(0x0000), None, "index 0 = 50 DPI is below the minimum");
    assert_eq!(d::dpi_from_id(0x5500), None, "an unknown high byte");
    // polling ids: the wired link has no 2000 / 4000
    assert_eq!(d::polling_id(2000, false), None);
    assert_eq!(d::polling_id(4000, true), Some(32));
    assert_eq!((d::polling_hz(16, true), d::polling_hz(16, false), d::polling_hz(1, false)), (Some(2000), None, Some(1000)));
}

// ---------------------------------------------------------------- decode

#[test]
fn answers_decode_like_openmouse() {
    // dareuDecodeBattery: body[0] = 4, body[5] = percent (<= 100), body[6] = raw status
    let mut b = vec![0u8; 16];
    b[0] = 4;
    b[5] = 87;
    b[6] = 1;
    assert_eq!(d::decode_battery(&b), Some((87, 1)));
    b[5] = 101;
    assert_eq!(d::decode_battery(&b), None, "over 100 is invalid");
    b[5] = 50;
    b[0] = 3;
    assert_eq!(d::decode_battery(&b), None, "not a battery answer");
    // the status words (batteryState): 1 and 2 charging, 0 not, the rest unlabelled
    assert_eq!((d::charging_of(1), d::charging_of(2), d::charging_of(0), d::charging_of(7)), (Some(true), Some(true), Some(false), None));
    // dareuMatchesReply: same command, not an event (10); a memory read also echoes address and length
    let req = &fr(&[8, 0, 0, 2, 4], 0x3f)[1..];
    let mut rep = vec![0u8; 16];
    rep[0] = 8;
    rep[2] = 0;
    rep[3] = 2;
    rep[4] = 4;
    rep[5..9].copy_from_slice(&[5, 80, 1, 84]);
    assert!(d::matches_reply(req, &rep));
    assert_eq!(d::decode_read_buffer_reply(req, &rep), Some(vec![5, 80, 1, 84]));
    let mut other_addr = rep.clone();
    other_addr[3] = 12;
    assert!(!d::matches_reply(req, &other_addr), "a reply for another address is not ours");
    let mut event = rep.clone();
    event[0] = 10;
    assert!(!d::matches_reply(req, &event), "event 10 is skipped");
    // a stage record: [x low, y low, high byte, checksum]
    assert_eq!(d::decode_stage_record(&[0x0f, 0x0f, 0x00, 0x37]), Some((800, 800)));
    assert_eq!(d::decode_stage_record(&[0x8f, 0x8f, 0x44, 0xf3]), Some((20000, 20000)));
    assert_eq!(d::decode_stage_record(&[0x0f, 0x1f, 0x00, 0x00]), None, "bad checksum");
    // x and y can differ (the vendor panel's legacy record): 800 / 1600
    let mut rec = vec![0x0f, 0x1f, 0x00, 0];
    rec[3] = d::checksum(&rec);
    assert_eq!(d::decode_stage_record(&rec), Some((800, 1600)));
}

// ---------------------------------------------------------------- allow-list

#[test]
fn only_read_requests_pass_the_allow_list() {
    for req in [d::check_active_request(), d::battery_request(), d::read_buffer_request(0, 2), d::read_buffer_request(2, 4), d::read_buffer_request(12, 10), d::read_buffer_request(22, 10), d::read_buffer_request(0xF6, 10)] {
        assert!(d::read_request_allowed(&req), "{req:02x?}");
    }
    // every write
    for w in [d::write_buffer_requests(0, &d::with_memory_checksum(&[1])), d::write_buffer_requests(12, &[0x0f, 0x0f, 0x00, 0x37]), d::write_buffer_requests(2, &[3, 0x51])] {
        for req in w {
            assert!(!d::read_request_allowed(&req), "{req:02x?}");
        }
    }
    // commands OpenMouse also has (active profile 14, set profile 15, firmware 18 / 29) are not read requests of this driver
    for cmd in [1u8, 2, 5, 7, 14, 15, 18, 29, 0xff] {
        let mut f = vec![0u8; 17];
        f[0] = 8;
        f[1] = cmd;
        // the checksum by hand: 8 + cmd under 55 → 85 - sum, else 341 - sum
        let s = 8 + cmd as u32;
        let c = if s < 55 { 85 - s } else { 341u32.wrapping_sub(s) };
        f[16] = (c & 0xff) as u8;
        assert!(!d::read_request_allowed(&f), "command {cmd}");
    }
    // a read past the first 256 bytes of memory, a bad checksum, a wrong length / report id, bytes after the request
    assert!(!d::read_request_allowed(&d::read_buffer_request(0xFB, 10)));
    assert!(!d::read_request_allowed(&d::read_buffer_request(0x2000, 4)));
    let mut bad = d::battery_request();
    bad[16] ^= 1;
    assert!(!d::read_request_allowed(&bad));
    let mut tail = d::battery_request();
    tail[9] = 1;
    tail[16] = 72; // 8 + 4 + 1 = 13 → 85 - 13: a valid checksum, so only the stray byte is what refuses it
    assert!(!d::read_request_allowed(&tail));
    assert!(!d::read_request_allowed(&d::battery_request()[1..]));
    let mut wrong_id = d::battery_request();
    wrong_id[0] = 0;
    assert!(!d::read_request_allowed(&wrong_id));
    assert!(!d::read_request_allowed(&[]));
}

// ---------------------------------------------------------------- device table

#[test]
fn the_table_is_openmouses_supported_list() {
    // supported-devices-data.ts: Dareu A950 PRO Mg, pids 0x1114 (TM265 receiver) and 0x1117 (wired TM271F)
    for (pid, wireless) in [(0x1114u16, true), (0x1117, false)] {
        let (svc, cmd) = (service_col(pid), command_col(pid));
        let m = d::matches(&[&svc, &cmd], 0x260d, pid).unwrap();
        assert_eq!((m.family, m.model, m.wireless), (Family::Dareu, Some("Dareu A950 PRO Mg"), Some(wireless)));
        assert_eq!((m.config.usage_page, m.config.usage), (0xFF02, 0x02), "the report-8 command collection");
        assert_eq!(m.config.path, cmd.path);
    }
    let (svc, cmd) = (service_col(0x1114), command_col(0x1114));
    // wrong vendor / unknown id
    assert!(d::matches(&[&svc, &cmd], 0x260e, 0x1114).is_none());
    assert!(d::matches(&[&svc, &cmd], 0x260d, 0x1115).is_none());
    // OpenMouse needs BOTH collections: the service one and the 16-byte report-8 one
    assert!(d::matches(&[&cmd], 0x260d, 0x1114).is_none());
    assert!(d::matches(&[&svc], 0x260d, 0x1114).is_none());
    let short = hid_info("short", 0x260d, 0x1114, 0xFF02, 0x02, (9, 9, 0));
    assert!(d::matches(&[&svc, &short], 0x260d, 0x1114).is_none(), "a report-8 collection that is not 16 bytes");
    let other_usage = hid_info("other", 0x260d, 0x1114, 0xFF02, 0x03, (17, 17, 0));
    assert!(d::matches(&[&svc, &other_usage], 0x260d, 0x1114).is_none());
    let mouse = hid_info("mouse", 0x260d, 0x1114, 0x01, 0x02, (8, 0, 0));
    assert!(d::matches(&[&svc, &mouse], 0x260d, 0x1114).is_none(), "the mouse collection is not the command channel");
    // no overlap with another family's table (find() walks the families in OpenMouse's registry order)
    let all = [svc.clone(), cmd.clone()];
    assert_eq!(bu_mouse::family::find(&all, 0x260d, 0x1114).map(|m| m.family), Some(Family::Dareu));
    for (vid, pid) in [(0x1532u16, 0x1114u16), (0x046d, 0x1117), (0x1038, 0x1114)] {
        assert!(d::matches(&[&svc, &cmd], vid, pid).is_none());
    }
}

#[test]
fn caps_are_what_openmouse_offers() {
    let wired = d::caps(false);
    let dpi = wired.dpi.as_ref().unwrap();
    assert_eq!((dpi.min(), dpi.max(), dpi.bands.clone()), (100, 26000, vec![(100, 26000, 50)]), "getDpiOptions: 100..26000 by 50");
    assert_eq!(wired.polling, vec![125, 250, 500, 1000], "DAREU_WIRED_POLLING_RATES");
    assert_eq!(d::caps(true).polling, vec![125, 250, 500, 1000, 2000, 4000], "DAREU_WIRELESS_POLLING_RATES");
    assert!(wired.lift_off.is_empty() && wired.battery);
}

// ---------------------------------------------------------------- the driver against a scripted mouse

/// A scripted Dareu mouse: 256 bytes of memory (reads and writes work like the Jm channel), battery, online flag.
struct Mouse {
    mem: Vec<u8>,
    online: bool,
    battery: (u8, u8),
    silent: bool,
    refuse_writes: bool,
    seen: Vec<Vec<u8>>,
}

impl Mouse {
    fn new() -> Self {
        let mut mem = vec![0u8; 0x100];
        mem[0..2].copy_from_slice(&[0x01, 0x54]); // 1000 Hz
        mem[2..6].copy_from_slice(&[5, 80, 1, 84]); // 5 stages, stage 2 active (85 - 5, 85 - 1)
        let stages: [[u8; 4]; 5] = [[0x07, 0x07, 0, 0x47], [0x0f, 0x0f, 0, 0x37], [0x1f, 0x1f, 0, 0x17], [0x3f, 0x3f, 0, 0xd7], [0x7f, 0x7f, 0, 0x57]];
        for (i, s) in stages.iter().enumerate() {
            mem[12 + i * 4..16 + i * 4].copy_from_slice(s);
        }
        Mouse { mem, online: true, battery: (73, 0), silent: false, refuse_writes: false, seen: Vec::new() }
    }

    fn answer(&mut self, out: &[u8]) -> Vec<u8> {
        self.seen.push(out.to_vec());
        let mut a = vec![0u8; 17];
        a[0] = 8;
        a[1] = out[1];
        match out[1] {
            3 => a[6] = self.online as u8,
            4 => {
                a[6] = self.battery.0;
                a[7] = self.battery.1;
            }
            8 => {
                a[3] = out[3];
                a[4] = out[4];
                a[5] = out[5];
                let (addr, len) = ((out[3] as usize) << 8 | out[4] as usize, out[5] as usize);
                a[6..6 + len].copy_from_slice(&self.mem[addr..addr + len]);
            }
            7 => {
                let (addr, len) = ((out[3] as usize) << 8 | out[4] as usize, out[5] as usize);
                if !self.refuse_writes {
                    self.mem[addr..addr + len].copy_from_slice(&out[6..6 + len]);
                }
            }
            c => panic!("a command the Dareu driver must not send: {c}"),
        }
        a
    }
}

fn setup(pid: u16, mouse: Rc<RefCell<Mouse>>) -> (bu_mouse::family::Matched, ScriptPort) {
    let (svc, cmd) = (service_col(pid), command_col(pid));
    let m = d::matches(&[&svc, &cmd], 0x260d, pid).unwrap();
    let p = ScriptPort::new(cmd, move |out, how| {
        let mut mo = mouse.borrow_mut();
        assert_eq!(*how, HidTransfer::OutputThenInput { reply_len: 17, max_reads: 16, timeout_ms: 800, echo: Some((1, out[1])) });
        if mo.silent {
            return Err(bu_mouse::Error::BadAnswer("no answer in time".into()));
        }
        Ok(mo.answer(out))
    });
    (m, p)
}

#[test]
fn reading_asks_only_what_openmouse_asks() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    let (m, mut p) = setup(0x1114, mouse.clone());
    let r = d::Dareu.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(p.sent.iter().all(|s| d::read_request_allowed(&s.0)), "read requests only");
    assert_eq!((r.online, r.battery_percent, r.charging, r.dpi, r.polling_hz, r.lift_off, r.wireless), (true, Some(73), Some(false), Some(800), Some(1000), None, Some(true)));
    assert_eq!(r.caps.polling, vec![125, 250, 500, 1000, 2000, 4000]);
    // the order: online?, battery, DPI header, the 5 stages (two reads), polling
    let asked: Vec<Vec<u8>> = p.sent.iter().map(|s| s.0.clone()).collect();
    assert_eq!(
        asked,
        vec![
            fr(&[3], 0x4a),
            fr(&[4], 0x49),
            fr(&[8, 0, 0, 2, 4], 0x3f),
            fr(&[8, 0, 0, 12, 10], 0x2f),
            fr(&[8, 0, 0, 22, 10], 0x25),
            fr(&[8, 0, 0, 0, 2], 0x43),
        ]
    );
    // the wired id: the same reads, the wired polling list, charging shown
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    mouse.borrow_mut().battery = (100, 1);
    mouse.borrow_mut().mem[0..2].copy_from_slice(&[0x02, 0x53]); // 500 Hz
    let (m, mut p) = setup(0x1117, mouse);
    let r = d::Dareu.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.battery_percent, r.charging, r.polling_hz, r.wireless), (Some(100), Some(true), Some(500), Some(false)));
    assert_eq!(r.caps.polling, vec![125, 250, 500, 1000]);
}

#[test]
fn a_broken_value_is_left_out_not_invented() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    mouse.borrow_mut().mem[1] = 0x00; // the polling checksum is wrong
    mouse.borrow_mut().mem[5] = 0x00; // the DPI header checksum is wrong
    let (m, mut p) = setup(0x1114, mouse);
    let r = d::Dareu.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.online, r.battery_percent, r.dpi, r.polling_hz), (true, Some(73), None, None));
}

#[test]
fn a_mouse_that_is_not_linked_is_offline_and_nothing_else_is_asked() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    mouse.borrow_mut().online = false;
    let (m, mut p) = setup(0x1114, mouse.clone());
    let r = d::Dareu.read(&m, &mut p, ReadOptions { online_tries: 1 }).unwrap();
    assert_eq!((r.online, r.battery_percent, r.dpi, r.polling_hz), (false, None, None, None));
    assert_eq!(p.sent.len(), 1, "one online? question, nothing more");
    // no answer at all (a receiver with nothing behind it) is the same
    mouse.borrow_mut().silent = true;
    let (m, mut p) = setup(0x1114, mouse.clone());
    let r = d::Dareu.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(!r.online);
    assert_eq!(p.sent.len(), 1);
    // and a change on an offline mouse is refused before any write
    mouse.borrow_mut().silent = false;
    let (m, mut p) = setup(0x1114, mouse);
    assert!(d::Dareu.write(&m, &mut p, Setting::Polling(500)).is_err());
    assert!(d::Dareu.write(&m, &mut p, Setting::Dpi(800)).is_err());
    assert!(p.sent.iter().all(|s| d::read_request_allowed(&s.0)), "nothing written");
}

#[test]
fn a_dpi_change_rewrites_the_one_stage_record_the_openmouse_way() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    let (m, mut p) = setup(0x1117, mouse.clone());
    // active stage is index 1 (800); set 3200: online?, header + stages, the 4-byte record at 16, header + stages again
    let c = d::Dareu.write(&m, &mut p, Setting::Dpi(3200)).unwrap();
    assert_eq!((c.now, c.before), (3200, Some(800)));
    let asked: Vec<Vec<u8>> = p.sent.iter().map(|s| s.0.clone()).collect();
    // writeDpiStage(stage 1, 3200): record [3f 3f 00 d7] at 12 + 4 = 16: frame sum 8+7+16+4+63+63+0+215 = 376 → 341 - 376 = -35 → 221 = 0xdd
    let write = fr(&[7, 0, 0, 16, 4, 0x3f, 0x3f, 0x00, 0xd7], 0xdd);
    assert_eq!(asked.iter().filter(|a| a[1] == 7).collect::<Vec<_>>(), vec![&write], "exactly one write");
    assert_eq!(&asked[..4], &[fr(&[3], 0x4a), fr(&[8, 0, 0, 2, 4], 0x3f), fr(&[8, 0, 0, 12, 10], 0x2f), fr(&[8, 0, 0, 22, 10], 0x25)]);
    assert_eq!(asked[4], write, "the write comes right after the first read");
    assert_eq!(&asked[5..], &[fr(&[8, 0, 0, 2, 4], 0x3f), fr(&[8, 0, 0, 12, 10], 0x2f), fr(&[8, 0, 0, 22, 10], 0x25)], "then the read-back");
    // the memory changed in those four bytes only
    let mem = mouse.borrow().mem.clone();
    assert_eq!(&mem[16..20], &[0x3f, 0x3f, 0x00, 0xd7]);
    let fresh = Mouse::new().mem;
    assert_eq!(&mem[..16], &fresh[..16]);
    assert_eq!(&mem[20..], &fresh[20..]);
    // a value off the grid, or out of range, is refused before a byte leaves
    p.sent.clear();
    for bad in [75, 50, 26050, 0] {
        assert!(d::Dareu.write(&m, &mut p, Setting::Dpi(bad)).is_err(), "{bad}");
    }
    assert!(d::Dareu.write(&m, &mut p, Setting::LiftOff(10)).is_err());
    assert!(p.sent.is_empty());
}

#[test]
fn a_polling_change_writes_the_two_bytes_the_openmouse_way() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    let (m, mut p) = setup(0x1114, mouse.clone());
    let c = d::Dareu.write(&m, &mut p, Setting::Polling(4000)).unwrap();
    assert_eq!((c.now, c.before), (4000, Some(1000)));
    let asked: Vec<Vec<u8>> = p.sent.iter().map(|s| s.0.clone()).collect();
    // online?, read the rate, write [20 35] (id 32, 85 - 32 = 53), read it back
    assert_eq!(asked, vec![fr(&[3], 0x4a), fr(&[8, 0, 0, 0, 2], 0x43), fr(&[7, 0, 0, 0, 2, 0x20, 0x35], 0xef), fr(&[8, 0, 0, 0, 2], 0x43)]);
    assert_eq!(&mouse.borrow().mem[0..2], &[0x20, 0x35]);
    // the wired link has no 4000 Hz: refused, nothing sent
    let (m, mut p) = setup(0x1117, Rc::new(RefCell::new(Mouse::new())));
    assert!(d::Dareu.write(&m, &mut p, Setting::Polling(4000)).is_err());
    assert!(d::Dareu.write(&m, &mut p, Setting::Polling(750)).is_err());
    assert!(p.sent.is_empty());
    let c = d::Dareu.write(&m, &mut p, Setting::Polling(125)).unwrap();
    assert_eq!(c.now, 125);
}

#[test]
fn a_mouse_that_keeps_its_value_is_reported_not_believed() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    mouse.borrow_mut().refuse_writes = true;
    let (m, mut p) = setup(0x1114, mouse.clone());
    let e = d::Dareu.write(&m, &mut p, Setting::Polling(500)).unwrap_err();
    assert!(matches!(e, bu_mouse::Error::BadAnswer(ref s) if s.contains("kept 1000 Hz")), "{e}");
    let e = d::Dareu.write(&m, &mut p, Setting::Dpi(1600)).unwrap_err();
    assert!(matches!(e, bu_mouse::Error::BadAnswer(ref s) if s.contains("kept 800 DPI")), "{e}");
}

#[test]
fn a_stage_with_different_x_and_y_is_left_alone() {
    let mouse = Rc::new(RefCell::new(Mouse::new()));
    // stage 2 (active) = X 800 / Y 1600: [0f 1f 00 chk]; chk: sum 46 → 85 - 46 = 39 = 0x27
    mouse.borrow_mut().mem[16..20].copy_from_slice(&[0x0f, 0x1f, 0x00, 0x27]);
    let (m, mut p) = setup(0x1114, mouse.clone());
    let r = d::Dareu.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!(r.dpi, Some(800), "X is what OpenMouse shows");
    p.sent.clear();
    assert!(d::Dareu.write(&m, &mut p, Setting::Dpi(1600)).is_err());
    assert!(p.sent.iter().all(|s| s.0[1] != 7), "no write was sent");
    assert_eq!(&mouse.borrow().mem[16..20], &[0x0f, 0x1f, 0x00, 0x27]);
}

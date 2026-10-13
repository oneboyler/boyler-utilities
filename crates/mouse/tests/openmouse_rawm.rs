//! Order 105 check, RAWM: our messages = the bytes OpenMouse sends (encode), and answers shaped the way OpenMouse reads them
//! decode to the same values (decode). The expected bytes are worked out by hand from OpenMouse (@openmouse/protocol 0.40.1:
//! `dist/rawm/index.js`, `dist/drivers/rawm/hid.js`), never from our own encoder.
//!
//! OpenMouse sends `sendReport(0, frame)`: a 64-byte frame `[0x80 | n, n payload bytes, zeros]` (n <= 63; longer messages in
//! 63-byte pieces); Windows puts the report id 0 in front (65 bytes). The status answer is a JSON text spread over several
//! input frames: `FF FF FF FF, (length >> 4 & 0xF0) | 2, length & 0xFF, JSON..., 0`, the length counting those two header
//! bytes, the JSON and the trailing zero.
//!
//! This family is HELD BACK in the device table (it needs a transfer that collects several input reports): the driver is
//! proven here against a scripted link; the device table's own matcher is proven too, and the switch stays off.

use bu_mouse::family::{Driver, Family, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::os::HidInfo;
use bu_mouse::rawm as rw;
use bu_mouse::Error;

const NOW: u64 = 1_700_000_000; // 0x6553F100

const JSON: &str = r#"{"battery":80,"chr":0,"cpi":1600,"polling":1000,"cpi_l":[400,800,1600],"cpi_l_c":[1,2,3],"kd":[5,5],"lod":1,"ms":1,"at":0,"as":0,"rctrl":1,"pm":2,"wired":0,"dn":"RAWM ML01","crc":0,"light":48,"ob":0,"top":255,"bl":[4100,4000,3950,3900,3850,3800,3750,3700,3500,3300,3100],"atp":1,"ocs":[132,130,131,129],"gm":[0,0]}"#;

/// The settings record OpenMouse builds for JSON above (no CRC), by hand, 65 bytes (the JSON carries all eight optional fields, with
/// the values OpenMouse would have defaulted: light 48 = 0x30, ob 0, top 255, bl = the eleven words in decimal, atp 1, ocs, gm [0,0]):
/// header `03 41 15` (0x41 = the length 65) · DPI `40 06` (1600) · polling `E8 03` (1000) · light `30` (default) ·
/// stages `03 | 90 01 | 20 03 | 40 06` · onboard `00` · power mode `02` · four zero bytes (no wide DPI) · `00` (no wide
/// stages) · lift-off `01` · key delays `02 | 05 05` · motion sync `01` · angle tuning `00` · angle snapping `00` · ripple
/// `01` · colours `03 | 01 02 03` · tx power `FF` (default) · battery levels `0B |` the eleven default words little endian ·
/// auto tx power `01` · onboard status `04 | 84 82 83 81` · glass mode `00`.
fn record() -> Vec<u8> {
    let mut v = vec![0x03, 0x41, 0x15, 0x40, 0x06, 0xE8, 0x03, 0x30, 0x03, 0x90, 0x01, 0x20, 0x03, 0x40, 0x06, 0x00, 0x02, 0, 0, 0, 0, 0x00, 0x01, 0x02, 0x05, 0x05, 0x01, 0x00, 0x00, 0x01, 0x03, 0x01, 0x02, 0x03, 0xFF, 0x0B];
    v.extend([0x04, 0x10, 0xA0, 0x0F, 0x6E, 0x0F, 0x3C, 0x0F, 0x0A, 0x0F, 0xD8, 0x0E, 0xA6, 0x0E, 0x74, 0x0E, 0xAC, 0x0D, 0xE4, 0x0C, 0x1C, 0x0C]);
    v.extend([0x01, 0x04, 0x84, 0x82, 0x83, 0x81, 0x00]);
    assert_eq!(v.len(), 65);
    v
}

/// The two frames a 65-byte message goes out in: 63 bytes behind `0xBF`, then 2 bytes behind `0x82`.
fn two_frames(msg: &[u8]) -> Vec<Vec<u8>> {
    let mut a = vec![0u8; 64];
    a[0] = 0x80 | 63;
    a[1..64].copy_from_slice(&msg[..63]);
    let mut b = vec![0u8; 64];
    b[0] = 0x80 | (msg.len() - 63) as u8;
    b[1..1 + msg.len() - 63].copy_from_slice(&msg[63..]);
    vec![a, b]
}

fn query_frame() -> Vec<u8> {
    // [0x80 | 13, 01, 0D, 03, 00, 00, time 0x6553F100 little endian: 00 F1 53 65 00 00 00 00, zeros]
    let mut f = vec![0u8; 64];
    f[..14].copy_from_slice(&[0x8D, 0x01, 0x0D, 0x03, 0x00, 0x00, 0x00, 0xF1, 0x53, 0x65, 0x00, 0x00, 0x00, 0x00]);
    f
}

// ---------------------------------------------------------------- the CRC

#[test]
fn the_crc_is_crc16_ccitt_false() {
    // the published check value of CRC-16/CCITT-FALSE (poly 0x1021, start 0xFFFF) for "123456789"
    assert_eq!(rw::crc16(b"123456789"), 0x29B1);
    // OpenMouse's byte-swap steps run by hand for the single byte 0x00 from 0xFFFF:
    //   swap → FFFF · ^ 00 → FFFF · ^ (FF >> 4 = 0F) → FFF0 · ^ ((FFF0 << 12) & FFFF = 0000) → FFF0 · ^ ((F0 << 5) & FFFF = 1E00) → E1F0
    assert_eq!(rw::crc16(&[0x00]), 0xE1F0);
    assert_eq!(rw::crc16(&[]), 0xFFFF);
    // and the same as a plain bit-by-bit CCITT-FALSE on a record-sized input
    let bitwise = |data: &[u8]| {
        let mut crc: u16 = 0xFFFF;
        for b in data {
            crc ^= (*b as u16) << 8;
            for _ in 0..8 {
                crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
            }
        }
        crc
    };
    assert_eq!(rw::crc16(&record()), bitwise(&record()));
}

// ---------------------------------------------------------------- framing + query

#[test]
fn frames_are_openmouses_event_framing() {
    let f = rw::frame_event(&[1, 2, 3]).unwrap();
    assert_eq!(&f[..5], &[0x83, 1, 2, 3, 0]);
    assert_eq!(f.len(), 64);
    assert_eq!(rw::frame_event(&[7; 63]).unwrap()[0], 0xBF);
    assert!(rw::frame_event(&[7; 64]).is_none(), "one frame holds 63 bytes");
    // 130 bytes = 63 + 63 + 4
    let m: Vec<u8> = (0..130u8).collect();
    let frames = rw::frame_events(&m);
    assert_eq!(frames.iter().map(|f| f[0]).collect::<Vec<_>>(), vec![0xBF, 0xBF, 0x84]);
    assert_eq!(&frames[2][1..5], &[126, 127, 128, 129]);
    // an input frame: header 0x80 | n
    assert_eq!(rw::event_payload(&f), Some(vec![1, 2, 3]));
    // a header with the top bits 11 skips one byte first (rawmEventPayload: the receiver channel byte)
    assert_eq!(rw::event_payload(&[0xC1, 0x82, 9, 8, 0]), Some(vec![9, 8]));
    // neither: not a payload frame
    assert_eq!(rw::event_payload(&[0x03, 1, 2, 3]), None);
    assert_eq!(rw::event_payload(&[]), None);
}

#[test]
fn the_status_query_is_openmouses() {
    assert_eq!(rw::build_query(NOW), vec![0x01, 0x0D, 0x03, 0x00, 0x00, 0x00, 0xF1, 0x53, 0x65, 0x00, 0x00, 0x00, 0x00]);
    assert_eq!(rw::frame_event(&rw::build_query(NOW)).unwrap().to_vec(), query_frame());
    let mut out = vec![0u8];
    out.extend(query_frame());
    assert!(rw::read_request_allowed(&out));
}

// ---------------------------------------------------------------- decode

/// A scripted status answer: junk telemetry, then `FF FF FF FF | hdr | JSON | 0`, cut into input frames of at most 63 bytes.
fn answer_frames(json: &str) -> Vec<Vec<u8>> {
    let len = 2 + json.len() + 1;
    let mut s = vec![0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x00, 0x7F, 0x00]; // telemetry: a sync followed by a non-result nibble
    s.extend([0xFF, 0xFF, 0xFF, 0xFF, (((len >> 4) & 0xF0) | 2) as u8, (len & 0xFF) as u8]);
    s.extend(json.as_bytes());
    s.push(0);
    rw::frame_events(&s).iter().map(|f| f.to_vec()).collect()
}

#[test]
fn the_status_answer_decodes_like_openmouse() {
    let frames = answer_frames(JSON);
    assert!(frames.len() >= 3, "the JSON spans several input reports: {}", frames.len());
    let stream: Vec<u8> = frames.iter().flat_map(|f| rw::event_payload(f).unwrap()).collect();
    let id = rw::parse_query_result(&stream).unwrap();
    assert_eq!((id.battery, id.charging, id.dpi, id.polling, id.lod, id.wired, id.crc_supported), (Some(80), Some(false), Some(1600), Some(1000), Some(1), Some(false), Some(false)));
    assert_eq!(id.dpi_levels, Some(vec![400, 800, 1600]));
    assert_eq!(id.key_delay, Some(vec![5, 5]));
    assert_eq!((id.motion_sync, id.angle_snapping, id.ripple_control, id.angle_tuning), (Some(true), Some(false), Some(true), Some(0)));
    assert_eq!(id.device_name.as_deref(), Some("RAWM ML01"));
    // an answer still missing its tail is not an answer yet
    assert!(rw::parse_query_result(&stream[..stream.len() - 20]).is_none());
    // a signed angle byte (0xF1 = -15) and a field of the wrong JSON type is simply absent
    let id = rw::parse_identity(r#"{"at":241,"battery":"x","chr":1,"gm":[3,1]}"#).unwrap();
    assert_eq!((id.angle_tuning, id.battery, id.charging, id.glass_mode_enabled), (Some(-15), None, Some(true), Some(1)));
    assert!(rw::parse_identity("not json").is_none());
}

// ---------------------------------------------------------------- encode: the settings record

fn identity() -> rw::Identity {
    rw::parse_identity(JSON).unwrap()
}

#[test]
fn the_settings_record_is_openmouses_buildrawmmouseparams() {
    assert_eq!(rw::build_mouse_params(&identity()).unwrap(), record());
    // setPollingRate(4000): the record with polling 0x0FA0 → `A0 0F`
    let mut id = identity();
    id.polling = Some(4000);
    let mut want = record();
    want[5] = 0xA0;
    want[6] = 0x0F;
    assert_eq!(rw::build_mouse_params(&id).unwrap(), want);
    // setDpi(800): `20 03`
    let mut id = identity();
    id.dpi = Some(800);
    let mut want = record();
    want[3] = 0x20;
    want[4] = 0x03;
    assert_eq!(rw::build_mouse_params(&id).unwrap(), want);
    // fields the mouse did report are echoed instead of OpenMouse's defaults: light 0x11, onboard 1, power mode 1, tx power 0x10,
    // auto tx 0, glass 1, and two battery levels
    let mut id = identity();
    id.light = Some(0x11);
    id.onboard = Some(1);
    id.power_mode = Some(1);
    id.tx_output_power = Some(0x10);
    id.auto_tx_power = Some(0);
    id.glass_mode_enabled = Some(1);
    id.battery_levels = Some(vec![0x1004, 0x0fa0]);
    id.onboard_status = Some(vec![0x84]);
    let got = rw::build_mouse_params(&id).unwrap();
    // 3 + 2 + 3 → light at 7; levels 8..15; onboard at 15, power mode at 16
    assert_eq!((got[7], got[15], got[16]), (0x11, 1, 1));
    // behind the colours (index 30: count 3, 31..34 = 1 2 3): tx power, battery level count 2, the two words, auto tx, status, glass
    assert_eq!(&got[34..], &[0x10, 0x02, 0x04, 0x10, 0xA0, 0x0F, 0x00, 0x01, 0x84, 0x01]);
    assert_eq!(got[1] as usize, got.len(), "the length byte counts the record");
}

#[test]
fn a_wide_dpi_moves_to_the_four_byte_slot() {
    // two axes packed X | Y << 16: 1600 | 800 << 16 = 0x03200640 → the 2-byte DPI slot is zero, the 4-byte slot after the
    // power mode holds `40 06 20 03`
    let mut id = identity();
    id.dpi = Some(0x0320_0640);
    let got = rw::build_mouse_params(&id).unwrap();
    assert_eq!((got[3], got[4]), (0, 0));
    assert_eq!(&got[17..21], &[0x40, 0x06, 0x20, 0x03]);
    // wide stages: a stage above 16 bits moves all stages to 4 bytes each (count in the later slot, 0 in the first)
    let mut id = identity();
    id.dpi_levels = Some(vec![400, 0x0001_0000]);
    let got = rw::build_mouse_params(&id).unwrap();
    assert_eq!(got[8], 0, "no 2-byte stage count");
    // header 3 + dpi 2 + polling/light 3 + count 1 = 9; onboard, power mode = 11; 4 DPI bytes = 15; stage count 2 at 15; then 8 bytes
    assert_eq!(got[15], 2);
    assert_eq!(&got[16..24], &[0x90, 0x01, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00]);
}

#[test]
fn the_crc_wrapper_is_openmouses() {
    let mut id = identity();
    id.crc_supported = Some(true);
    let got = rw::build_mouse_params(&id).unwrap();
    // 65 + 5 = 70 bytes: `03 46 24 crc-lo crc-hi` then the record; the CRC is over the record (length byte already set)
    assert_eq!(got.len(), 70);
    assert_eq!(&got[..3], &[0x03, 0x46, 0x24]);
    assert_eq!(&got[5..], &record()[..]);
    let crc = {
        let mut crc: u16 = 0xFFFF;
        for b in record() {
            crc ^= (b as u16) << 8;
            for _ in 0..8 {
                crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
            }
        }
        crc
    };
    assert_eq!((got[3], got[4]), ((crc & 0xFF) as u8, (crc >> 8) as u8));
}

#[test]
fn a_partial_identity_is_never_written() {
    // each required field missing → "refusing a partial configuration write"
    let cut = |key: &str| {
        let mut v: serde_json::Value = serde_json::from_str(JSON).unwrap();
        v.as_object_mut().unwrap().remove(key);
        rw::parse_identity(&v.to_string()).unwrap()
    };
    for key in ["cpi", "polling", "cpi_l", "cpi_l_c", "kd", "lod", "ms", "at", "as", "rctrl"] {
        let e = rw::build_mouse_params(&cut(key)).unwrap_err();
        assert!(matches!(e, Error::BadAnswer(ref s) if s.contains("refusing a partial configuration write")), "{key}: {e}");
    }
    // the eight OpenMouse would default (made-up battery table, power mode ...) are refused too: nothing may be written that the
    // mouse did not report
    for key in ["light", "ob", "pm", "top", "bl", "atp", "ocs", "gm"] {
        let e = rw::build_mouse_params(&cut(key)).unwrap_err();
        assert!(matches!(e, Error::BadAnswer(ref s) if s.contains("refusing a partial configuration write")), "{key}: {e}");
    }
}

// ---------------------------------------------------------------- allow-list

#[test]
fn only_the_status_query_passes_the_allow_list() {
    let rep = |f: &[u8]| {
        let mut v = vec![0u8];
        v.extend_from_slice(f);
        v
    };
    assert!(rw::read_request_allowed(&rep(&query_frame())));
    // a later time is the same question
    let mut later = query_frame();
    later[6..14].copy_from_slice(&1_800_000_000u64.to_le_bytes());
    assert!(rw::read_request_allowed(&rep(&later)));
    // every settings record (any of its pieces) is refused
    for piece in two_frames(&record()) {
        assert!(!rw::read_request_allowed(&rep(&piece)));
    }
    let mut id = identity();
    id.crc_supported = Some(true);
    for f in rw::frame_events(&rw::build_mouse_params(&id).unwrap()) {
        assert!(!rw::read_request_allowed(&rep(&f)));
    }
    // a query with an extra byte behind it, a wrong report id, a wrong length byte, a short buffer
    let mut extra = query_frame();
    extra[30] = 1;
    assert!(!rw::read_request_allowed(&rep(&extra)));
    let mut wrong_id = rep(&query_frame());
    wrong_id[0] = 1;
    assert!(!rw::read_request_allowed(&wrong_id));
    let mut other = query_frame();
    other[0] = 0x8C;
    assert!(!rw::read_request_allowed(&rep(&other)));
    assert!(!rw::read_request_allowed(&rep(&query_frame())[..60]));
    // the sleep command (0x03 .. 0x21) is not a read either
    let sleep = rw::frame_event(&[0x03, 0x07, 0x21, 0x2c, 0x01]).unwrap();
    assert!(!rw::read_request_allowed(&rep(&sleep)));
}

// ---------------------------------------------------------------- device table

fn vendor_col(pid: u16) -> HidInfo {
    hid_info(&format!("rawm{pid:04x}"), 0x1915, pid, 0xFF00, 0x01, (65, 65, 0))
}

#[test]
fn the_table_is_openmouses_supported_list() {
    // supported-devices-data.ts: Blade 1 2328, ML01 2329, MH01 232c, SL01 232d, SH01 232e/232f, SH01 Pro 2330/2332,
    // MH01 Pro 2331, ES21 2333, ES21 Pro 2334, ES21M 2337, ER21 Pro 2338, ER21M 2339
    let want = [
        (0x2328u16, "Blade 1"),
        (0x2329, "ML01"),
        (0x232c, "MH01"),
        (0x232d, "SL01"),
        (0x232e, "SH01"),
        (0x232f, "SH01"),
        (0x2330, "SH01 Pro"),
        (0x2332, "SH01 Pro"),
        (0x2331, "MH01 Pro"),
        (0x2333, "ES21"),
        (0x2334, "ES21 Pro"),
        (0x2337, "ES21M"),
        (0x2338, "ER21 Pro"),
        (0x2339, "ER21M"),
    ];
    assert_eq!(rw::MODELS.len(), 14);
    for (pid, name) in want {
        let col = vendor_col(pid);
        let m = rw::table_matches(&[&col], 0x1915, pid).unwrap();
        assert_eq!((m.family, m.model), (Family::Rawm, Some(name)), "{pid:04x}");
        assert_eq!(m.config.path, col.path);
    }
    // the receivers the driver knows and the Leviathan are not on the supported list
    for pid in [0x232au16, 0x232b, 0x2335, 0x2336, 0x2327, 0x233a] {
        assert!(rw::table_matches(&[&vendor_col(pid)], 0x1915, pid).is_none(), "{pid:04x}");
    }
    // wrong vendor; the collection must be 0xFF00 / usage 1 with report 0 of 64 bytes both ways
    assert!(rw::table_matches(&[&vendor_col(0x2329)], 0x1916, 0x2329).is_none());
    assert!(rw::table_matches(&[&hid_info("x", 0x1915, 0x2329, 0xFF01, 0x01, (65, 65, 0))], 0x1915, 0x2329).is_none());
    assert!(rw::table_matches(&[&hid_info("x", 0x1915, 0x2329, 0xFF00, 0x02, (65, 65, 0))], 0x1915, 0x2329).is_none());
    assert!(rw::table_matches(&[&hid_info("x", 0x1915, 0x2329, 0xFF00, 0x01, (9, 9, 0))], 0x1915, 0x2329).is_none());
    // the driver reads and writes 65-byte frames: a longer collection is not claimed
    assert!(rw::table_matches(&[&hid_info("x", 0x1915, 0x2329, 0xFF00, 0x01, (129, 129, 0))], 0x1915, 0x2329).is_none());
    assert!(rw::table_matches(&[&hid_info("x", 0x1915, 0x2329, 0xFF00, 0x01, (65, 129, 0))], 0x1915, 0x2329).is_none());
    assert!(rw::table_matches(&[&hid_info("x", 0x1915, 0x2329, 0x01, 0x02, (8, 0, 0))], 0x1915, 0x2329).is_none());
    // ON since the multi-report transfer (HidTransfer::OutputThenStream) exists
    const { assert!(rw::TRANSPORT_READY) };
    for (pid, name) in want {
        assert!(rw::matches(&[&vendor_col(pid)], 0x1915, pid).is_some(), "{pid:04x}");
        let f = bu_mouse::family::find(&[vendor_col(pid)], 0x1915, pid).unwrap_or_else(|| panic!("{pid:04x} {name}"));
        assert_eq!(f.family, Family::Rawm);
    }
}

#[test]
fn caps_are_what_openmouse_offers() {
    let c = rw::caps(true);
    assert_eq!(c.dpi.as_ref().unwrap().bands, vec![(50, 42000, 50)], "getDpiOptions: 50..42000 by 50");
    assert_eq!(c.polling, vec![125, 250, 500, 1000, 2000, 4000, 8000]);
    assert!(c.lift_off.is_empty(), "Low / High only: no millimetres");
    assert!(c.battery && !rw::caps(false).battery);
}

// ---------------------------------------------------------------- the driver against a scripted link

struct Fake {
    dpi: i64,
    polling: i64,
    battery: Option<i64>,
    crc: bool,
    quiet: bool,
    ignore_writes: bool,
    drop_field: Option<&'static str>,
    queries: Vec<Vec<u8>>,
    sent: Vec<Vec<u8>>,
    pauses: Vec<u64>,
    pending: Vec<u8>,
}

impl Fake {
    fn new() -> Self {
        Fake { dpi: 1600, polling: 1000, battery: Some(80), crc: false, quiet: false, ignore_writes: false, drop_field: None, queries: Vec::new(), sent: Vec::new(), pauses: Vec::new(), pending: Vec::new() }
    }

    fn json(&self) -> String {
        let mut v: serde_json::Value = serde_json::from_str(JSON).unwrap();
        let o = v.as_object_mut().unwrap();
        o.insert("cpi".into(), self.dpi.into());
        o.insert("polling".into(), self.polling.into());
        o.insert("crc".into(), (self.crc as i64).into());
        match self.battery {
            Some(b) => o.insert("battery".into(), b.into()),
            None => o.remove("battery"),
        };
        if let Some(k) = self.drop_field {
            o.remove(k);
        }
        v.to_string()
    }
}

impl rw::Link for Fake {
    fn send(&mut self, frame: &[u8; 64]) -> bu_mouse::Result<()> {
        self.sent.push(frame.to_vec());
        self.pending.extend(rw::event_payload(frame).unwrap());
        if self.pending.len() >= self.pending[1] as usize {
            let msg = std::mem::take(&mut self.pending);
            if !self.ignore_writes {
                let inner = if msg[2] == 0x24 { &msg[5..] } else { &msg[..] };
                self.dpi = inner[3] as i64 | (inner[4] as i64) << 8;
                self.polling = inner[5] as i64 | (inner[6] as i64) << 8;
            }
        }
        Ok(())
    }
    fn query_frames(&mut self, frame: &[u8; 64]) -> bu_mouse::Result<Vec<Vec<u8>>> {
        self.queries.push(frame.to_vec());
        if self.quiet {
            return Err(Error::BadAnswer("no answer".into()));
        }
        Ok(answer_frames(&self.json()))
    }
    fn pause_ms(&mut self, ms: u64) {
        self.pauses.push(ms);
    }
    fn now_secs(&self) -> u64 {
        NOW
    }
}

#[test]
fn reading_asks_only_what_openmouse_asks() {
    let mut f = Fake::new();
    let r = rw::read_with(&mut f, Some("ML01"), ReadOptions::default()).unwrap();
    assert_eq!(f.queries, vec![query_frame()], "one status query");
    assert!(f.sent.is_empty());
    assert_eq!((r.online, r.battery_percent, r.charging, r.dpi, r.polling_hz, r.lift_off, r.wireless), (true, Some(80), Some(false), Some(1600), Some(1000), None, Some(true)));
    assert_eq!(r.model.as_deref(), Some("RAWM ML01"), "the name the mouse itself gives");
    assert!(r.caps.battery);
    // no battery in the answer = no battery row; a wired USB answer says wired
    let mut f = Fake::new();
    f.battery = None;
    let r = rw::read_with(&mut f, None, ReadOptions::default()).unwrap();
    assert_eq!((r.battery_percent, r.charging, r.caps.battery), (None, None, false));
    // a packed X|Y DPI shows X: (packed & 0xffff)
    let mut f = Fake::new();
    f.dpi = 0x0320_0640;
    assert_eq!(rw::read_with(&mut f, None, ReadOptions::default()).unwrap().dpi, Some(1600));
    // a mouse that does not answer is offline, nothing invented
    let mut f = Fake::new();
    f.quiet = true;
    let r = rw::read_with(&mut f, Some("ML01"), ReadOptions { online_tries: 1 }).unwrap();
    assert_eq!((r.online, r.battery_percent, r.dpi, r.polling_hz), (false, None, None, None));
    assert_eq!(r.model.as_deref(), Some("ML01"));
}

#[test]
fn a_polling_change_sends_the_rebuilt_record_the_openmouse_way() {
    let mut f = Fake::new();
    let c = rw::write_with(&mut f, Setting::Polling(4000)).unwrap();
    assert_eq!((c.now, c.before), (4000, Some(1000)));
    let mut want = record();
    want[5] = 0xA0;
    want[6] = 0x0F;
    assert_eq!(f.sent, two_frames(&want), "two pieces: 63 + 2 bytes");
    // query, [send, 25 ms, send], 100 ms, query
    assert_eq!(f.queries.len(), 2);
    assert_eq!(f.pauses, vec![25, 100]);
}

#[test]
fn a_dpi_change_sends_the_rebuilt_record_the_openmouse_way() {
    let mut f = Fake::new();
    let c = rw::write_with(&mut f, Setting::Dpi(800)).unwrap();
    assert_eq!((c.now, c.before), (800, Some(1600)));
    let mut want = record();
    want[3] = 0x20;
    want[4] = 0x03;
    assert_eq!(f.sent, two_frames(&want));
    // with a CRC-speaking mouse the record is wrapped (70 bytes: 63 + 7)
    let mut f = Fake::new();
    f.crc = true;
    rw::write_with(&mut f, Setting::Dpi(800)).unwrap();
    let mut id = identity();
    id.dpi = Some(800);
    id.crc_supported = Some(true);
    let wrapped = rw::build_mouse_params(&id).unwrap();
    assert_eq!(wrapped.len(), 70);
    assert_eq!(f.sent.iter().map(|s| s[0]).collect::<Vec<_>>(), vec![0xBF, 0x87]);
    assert_eq!(f.sent, rw::frame_events(&wrapped).iter().map(|x| x.to_vec()).collect::<Vec<_>>());
    // values OpenMouse refuses are refused before anything is asked
    let mut f = Fake::new();
    for bad in [49, 42001, 0] {
        assert!(rw::write_with(&mut f, Setting::Dpi(bad)).is_err(), "{bad}");
    }
    for bad in [100, 3000, 0] {
        assert!(rw::write_with(&mut f, Setting::Polling(bad)).is_err(), "{bad}");
    }
    assert!(rw::write_with(&mut f, Setting::LiftOff(10)).is_err());
    assert!(f.queries.is_empty() && f.sent.is_empty());
    assert_eq!(rw::write_with(&mut f, Setting::Dpi(42000)).unwrap().now, 42000);
}

#[test]
fn a_mouse_that_keeps_its_value_or_a_partial_status_is_reported() {
    let mut f = Fake::new();
    f.ignore_writes = true;
    let e = rw::write_with(&mut f, Setting::Polling(500)).unwrap_err();
    assert!(matches!(e, Error::BadAnswer(ref s) if s.contains("did not confirm the requested polling rate")), "{e}");
    let e = rw::write_with(&mut f, Setting::Dpi(800)).unwrap_err();
    assert!(matches!(e, Error::BadAnswer(ref s) if s.contains("did not confirm the requested DPI")), "{e}");
    // a status without the stage list: nothing is sent at all
    let mut f = Fake::new();
    f.drop_field = Some("cpi_l");
    assert!(rw::write_with(&mut f, Setting::Dpi(800)).is_err());
    assert!(f.sent.is_empty(), "OpenMouse refuses a partial write and so do we");
    // a firmware whose status has no battery table (`bl`) or power mode (`pm`): one click sends nothing, not a made-up record
    for key in ["bl", "pm", "light", "ob", "top", "atp", "ocs", "gm"] {
        for setting in [Setting::Dpi(800), Setting::Polling(500)] {
            let mut f = Fake::new();
            f.drop_field = Some(key);
            assert!(rw::write_with(&mut f, setting).is_err(), "{key}");
            assert!(f.sent.is_empty(), "{key}: nothing is sent");
        }
    }
    // a mouse that is silent is not written to
    let mut f = Fake::new();
    f.quiet = true;
    assert!(rw::write_with(&mut f, Setting::Polling(500)).is_err());
    assert!(f.sent.is_empty());
    // separate X / Y DPI is not collapsed
    let mut f = Fake::new();
    f.dpi = 0x0320_0640;
    assert!(rw::write_with(&mut f, Setting::Dpi(800)).is_err());
    assert!(f.sent.is_empty());
}

#[test]
fn over_a_real_port_the_status_answer_comes_in_one_stream() {
    use bu_mouse::os::HidTransfer;
    let col = vendor_col(0x2329);
    let m = rw::table_matches(&[&col], 0x1915, 0x2329).unwrap();
    // the mouse answers with the input reports back to back, each with the unnumbered report id 0 in front
    let frames = answer_frames(JSON);
    let mut stream = Vec::new();
    for f in &frames {
        stream.push(0u8);
        stream.extend_from_slice(f);
    }
    let mut p = ScriptPort::new(col, move |_, how| {
        assert!(matches!(how, HidTransfer::OutputThenStream { reply_len: 65, .. }), "{how:?}");
        Ok(stream.clone())
    });
    let r = rw::Rawm.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(r.online);
    assert_eq!((r.dpi, r.polling_hz, r.battery_percent), (Some(1600), Some(1000), Some(80)));
    // exactly one request went out (the status query), and it passes the allow-list
    assert_eq!(p.sent.len(), 1);
    assert!(rw::read_request_allowed(&p.sent[0].0));
    // a mouse that sends no report is reported offline, not an error
    let col = vendor_col(0x2329);
    let mut p = ScriptPort::new(col, |_, _| Err(Error::BadAnswer("the mouse did not answer in time".into())));
    let r = rw::Rawm.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(!r.online);
}

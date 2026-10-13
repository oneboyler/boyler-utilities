//! Order 105 check, Keychron (Nape Pro + M6): our requests = the bytes OpenMouse sends for the same setting (encode), and
//! answers shaped the way OpenMouse reads them decode to the same values (decode). The expected bytes are worked out by hand
//! from OpenMouse (@openmouse/protocol 0.40.1: `dist/keychron/index.js`, `dist/drivers/keychron/launcher-mouse.js`,
//! `nape-hid.js`, `mouse-8k-hid.js`), never from our own encoder. Never run on a real unit.
//!
//! How OpenMouse counts: `sendReport(reportId, packet)` - the packet WITHOUT the report id; an answer is `event.data`, also
//! without the id. Windows' buffer is the report id + the packet, zero-padded to the collection's report length: OpenMouse's
//! byte i is our byte i + 1.

use bu_mouse::family::{self, Change, Driver, Extra, Family, Kind, Matched, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::keychron as kc;
use bu_mouse::os::{HidInfo, HidTransfer};
use bu_mouse::Error;
use std::cell::RefCell;
use std::rc::Rc;

// ---------------------------------------------------------------- helpers

/// A Nape report as Windows sends it: report id 0, the command, zeros to 33 bytes.
fn nrep(cmd: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 33];
    v[1..1 + cmd.len()].copy_from_slice(cmd);
    v
}

/// An M6 report: the report id, the packet, zeros to 64 bytes.
fn mrep(id: u8, packet: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 64];
    v[0] = id;
    v[1..1 + packet.len()].copy_from_slice(packet);
    v
}

fn nape_col() -> HidInfo {
    hid_info("nape", 0x3434, 0x0440, 0xFF60, 0x61, (33, 33, 0))
}

fn m6_col(pid: u16) -> HidInfo {
    hid_info(&format!("m6{pid:04x}"), 0x3434, pid, 0xFFC1, 0x01, (64, 64, 0))
}

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
    ]
}

// ---------------------------------------------------------------- Nape: encode, by hand

#[test]
fn nape_read_requests_are_openmouses() {
    // keychronPacket([CMD.miscGroup 167 = 0xa7, NAPE.x]) zero-padded to 32 bytes; getDpiStage 33, getDpiValue 36 + stage,
    // getBattery 49, MISC.getPolling 13, getCustomDpi 54
    assert_eq!(kc::nape_report(&kc::nape_get_dpi_stage(), 33), nrep(&[0xA7, 0x21]));
    assert_eq!(kc::nape_report(&kc::nape_get_dpi_value(3), 33), nrep(&[0xA7, 0x24, 0x03]));
    assert_eq!(kc::nape_report(&kc::nape_get_battery(), 33), nrep(&[0xA7, 0x31]));
    assert_eq!(kc::nape_report(&kc::nape_get_polling(), 33), nrep(&[0xA7, 0x0D]));
    assert_eq!(kc::nape_report(&kc::nape_get_custom_dpi(), 33), nrep(&[0xA7, 0x36]));
}

#[test]
fn nape_write_requests_are_openmouses() {
    // writeDpiStageValue: [miscGroup, setDpiValue 35 = 0x23, stage, dpi & 0xff, dpi >> 8]; 1600 = 0x0640 -> 40 06
    assert_eq!(kc::nape_set_dpi_value(2, 1600).to_vec(), vec![0xA7, 0x23, 0x02, 0x40, 0x06]);
    // 4000 = 0x0fa0; 50 = 0x0032
    assert_eq!(kc::nape_set_dpi_value(4, 4000).to_vec(), vec![0xA7, 0x23, 0x04, 0xA0, 0x0F]);
    assert_eq!(kc::nape_set_dpi_value(0, 50).to_vec(), vec![0xA7, 0x23, 0x00, 0x32, 0x00]);
    // setActiveDpiStage: [miscGroup, setDpiStage 34 = 0x22, stage]
    assert_eq!(kc::nape_set_dpi_stage(2).to_vec(), vec![0xA7, 0x22, 0x02]);
    // the custom-DPI fallback: [miscGroup, setCustomDpi 55 = 0x37, lo, hi]
    assert_eq!(kc::nape_set_custom_dpi(1600).to_vec(), vec![0xA7, 0x37, 0x40, 0x06]);
    // setPollingRate: [miscGroup, MISC.setPolling 14 = 0x0e, shift, shift], shift = index in [8000, 4000, 2000, 1000, 500, 250, 125]
    for (hz, shift) in [(8000, 0u8), (4000, 1), (2000, 2), (1000, 3), (500, 4), (250, 5), (125, 6)] {
        assert_eq!(kc::nape_set_polling(hz).unwrap().to_vec(), vec![0xA7, 0x0E, shift, shift], "{hz}");
    }
    assert_eq!(kc::nape_set_polling(750), None);
}

#[test]
fn nape_answers_decode_the_openmouse_way() {
    // getDpiStage: min(response[2], 4)
    assert_eq!(kc::nape_decode_dpi_stage(&[0xA7, 0x21, 2]), 2);
    assert_eq!(kc::nape_decode_dpi_stage(&[0xA7, 0x21, 9]), 4);
    // getDpiValue: response[2] | response[3] << 8
    assert_eq!(kc::nape_decode_dpi(&[0xA7, 0x24, 0x40, 0x06]), 1600);
    assert_eq!(kc::nape_decode_dpi(&[0xA7, 0x24, 0xA0, 0x0F]), 4000);
    // keychronDecodeBattery: percent response[2] (0xff when missing), status response[3]: 1 charging, 2 full
    assert_eq!(kc::nape_decode_battery(&[0xA7, 0x31, 87, 1]), (Some(87), true));
    assert_eq!(kc::nape_decode_battery(&[0xA7, 0x31, 100, 2]), (Some(100), false));
    assert_eq!(kc::nape_decode_battery(&[0xA7, 0x31, 55, 0]), (Some(55), false));
    assert_eq!(kc::nape_decode_battery(&[0xA7, 0x31, 255, 0]).0, None, "over 100 = unknown");
    // keychronDecodePolling: supported = table entries whose bit is set in response[5], ascending; current = table[response[6]]
    // mask 0x58 = bits 3, 4, 6 = 1000, 500, 125; index 3 = 1000
    assert_eq!(kc::nape_decode_polling(&[0xA7, 0x0D, 0, 0, 0, 0x58, 0x03]), Some((1000, vec![125, 500, 1000])));
    // mask 0x7f = all seven; index 0 = 8000
    assert_eq!(kc::nape_decode_polling(&[0xA7, 0x0D, 0, 0, 0, 0x7F, 0x00]), Some((8000, vec![125, 250, 500, 1000, 2000, 4000, 8000])));
    // index 6 = 125, mask 0 -> supported = [the current rate]
    assert_eq!(kc::nape_decode_polling(&[0xA7, 0x0D, 1, 0, 0, 0x00, 0x06]), Some((125, vec![125])));
    // an index past the table is clamped to its last entry (125)
    assert_eq!(kc::nape_decode_polling(&[0xA7, 0x0D, 0, 0, 0, 0x40, 0x09]).map(|p| p.0), Some(125));
    // all zero from byte 2: OpenMouse claims 1000 Hz out of thin air; here: no polling data
    assert_eq!(kc::nape_decode_polling(&[0xA7, 0x0D, 0, 0, 0, 0, 0]), None);
}

// ---------------------------------------------------------------- M6: encode, by hand

#[test]
fn m6_read_requests_are_openmouses() {
    // the status request: query(0xb3, 63, [STATUS_COMMAND 6]); the version query: querySettings(SET.version 0x02, [mode]) on
    // 0xb5 (20 bytes); the connected-mouse list SET.receiverState 0x03; readXy 0x49 on 0xb3; readPollingSets 0x4b on 0xb5
    assert_eq!(kc::m6_report(0xB3, &kc::m6_status_request(), 64), mrep(0xB3, &[0x06]));
    assert_eq!(kc::m6_report(0xB5, &kc::m6_version_request(0), 64), mrep(0xB5, &[0x02, 0x00]));
    assert_eq!(kc::m6_report(0xB5, &kc::m6_version_request(1), 64), mrep(0xB5, &[0x02, 0x01]));
    assert_eq!(kc::m6_report(0xB5, &kc::m6_receiver_state_request(), 64), mrep(0xB5, &[0x03]));
    assert_eq!(kc::m6_report(0xB3, &kc::m6_read_xy_request(), 64), mrep(0xB3, &[0x49]));
    assert_eq!(kc::m6_report(0xB5, &kc::m6_read_polling_sets_request(), 64), mrep(0xB5, &[0x4B]));
}

fn status_of(f: impl FnOnce(&mut [u8; 63])) -> kc::Status {
    let mut b = [0u8; 63];
    b[0] = 6;
    f(&mut b);
    kc::m6_decode_status(&b, &kc::m6_decode_version(None, 0)).unwrap()
}

#[test]
fn m6_dpi_packet_is_openmouses() {
    // keychronEncodeDpi(activeStage, stages, stageCount): [0]=0x40, [1..4]=stage x3, slots LE from [4], [14]=count.
    // stage 2, stages 400 (0x0190), 800 (0x0320), 3200 (0x0c80), 3200, 6400 (0x1900), count 5
    let p = kc::m6_encode_dpi(2, &[400, 800, 3200, 3200, 6400], 5);
    assert_eq!(p.to_vec(), vec![0x40, 2, 2, 2, 0x90, 0x01, 0x20, 0x03, 0x80, 0x0C, 0x80, 0x0C, 0x00, 0x19, 5, 0, 0, 0, 0, 0]);
    assert_eq!(kc::m6_report(0xB5, &p, 64), mrep(0xB5, &[0x40, 2, 2, 2, 0x90, 0x01, 0x20, 0x03, 0x80, 0x0C, 0x80, 0x0C, 0x00, 0x19, 5]));
}

#[test]
fn m6_sensor_packet_is_openmouses() {
    // keychronEncodeSensor (0x42): [1]=lod code, [2]=ripple 1/2, [3]=angle snapping 1/2, [4]=motion sync 1/2, [6]=scroll
    // reversed 2/1, [8]=max speed (INVERTED vs the others: on = 2, off = 1; always sent), [11]=lod level; toggles: on = 1, off = 2. Status: ripple on, motion sync
    // on, the rest off: flags byte [15] = 0x04 | 0x10 | lod 1 = 0x15
    let st = status_of(|b| b[15] = 0x15);
    assert_eq!((st.lod, st.ripple_control, st.angle_snapping, st.motion_sync, st.scroll_reversed, st.max_speed), (1, true, false, true, false, false));
    let p = kc::m6_encode_sensor(&st, false, 2);
    assert_eq!(p.to_vec(), vec![0x42, 2, 1, 2, 1, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    // the level form (firmware that flags lift-off levels): [1] = 0, [11] = the level
    assert_eq!(kc::m6_encode_sensor(&st, true, 3).to_vec(), vec![0x42, 0, 1, 2, 1, 0, 1, 0, 1, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0]);
    // scroll reversed (bit 6) and 20K FPS (reply [52] bit 0) are re-sent as read
    let st = status_of(|b| {
        b[15] = 0x08 | 0x40;
        b[52] = 1;
    });
    assert_eq!(kc::m6_encode_sensor(&st, false, 1).to_vec(), vec![0x42, 1, 2, 1, 2, 0, 2, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn m6_polling_packets_are_openmouses() {
    // pollingPacket without sets: [0]=0x41, [1]=[2]=level, [3..]=table[0..count], [9]=count
    let st = status_of(|b| {
        b[2] = 0x22;
        b[43..49].copy_from_slice(&[0, 1, 2, 0, 0, 0]);
        b[49] = 3;
    });
    let cur = kc::m6_gears_from_status(&st, 0);
    assert_eq!((cur.level, cur.count, cur.rate()), (2, 3, Some(1000)));
    let next = kc::Gears { level: 1, ..cur.clone() };
    assert_eq!(kc::m6_encode_polling(&cur, &next).to_vec(), vec![0x41, 1, 1, 0, 1, 2, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    // with sets: [0]=0x4a, [1+i]=level, [3+i]=count, set i's table from [5 + 6i]; the active set replaced
    let b: Vec<u8> = [vec![0x4B, 2, 0, 3, 3], vec![0, 1, 2, 0, 0, 0], vec![0, 1, 2, 0, 0, 0]].concat();
    let ident = kc::m6_decode_version(None, 0);
    let cur = kc::m6_gears_from_sets(&b, &ident);
    assert_eq!((cur.level, cur.count, cur.set, cur.rate()), (2, 3, 0, Some(1000)));
    let next = kc::Gears { level: 1, ..cur.clone() };
    assert_eq!(kc::m6_encode_polling(&cur, &next).to_vec(), vec![0x4A, 1, 0, 3, 3, 0, 1, 2, 0, 0, 0, 0, 1, 2, 0, 0, 0, 0]
        .into_iter().chain([0, 0]).collect::<Vec<u8>>());
    // the 2.4 GHz link (work mode 1) uses set 1
    let ident1 = kc::Identity { work_mode: 1, ..ident };
    assert_eq!(kc::m6_gears_from_sets(&b, &ident1).set, 1);
}

#[test]
fn m6_separate_xy_packet_is_openmouses() {
    // writeDpi with separate X/Y (protocol 6): [0]=0x48, [1..4]=active stage, [4]=count, X LE from [5], Y LE from [21] (an
    // unchanged slot keeps its Y, a changed one gets Y = X), [37] = the enable bits
    let ident = kc::Identity { work_mode: 0, version: 6, product_id: None, flags: [0, 0, 1, 0] };
    let mut b = vec![0u8; 63];
    b[0] = 0x49;
    b[1] = 2;
    b[4] = 5;
    for (i, x) in [400u16, 800, 1600, 3200, 6400].iter().enumerate() {
        b[5 + 2 * i] = *x as u8;
        b[6 + 2 * i] = (*x >> 8) as u8;
    }
    for (i, y) in [400u16, 800, 1500, 3200, 6000].iter().enumerate() {
        b[21 + 2 * i] = *y as u8;
        b[22 + 2 * i] = (*y >> 8) as u8;
    }
    b[37] = 0x1F;
    let cur = kc::m6_dpi_from_xy(&b, &ident);
    assert_eq!((cur.active, cur.count, cur.stages), (2, 5, [400, 800, 1600, 3200, 6400]));
    let mut next = cur.clone();
    next.stages[2] = 3200;
    let p = kc::m6_encode_dpi_xy(&cur, &next, &ident).unwrap();
    // X: 400 90 01 | 800 20 03 | 3200 80 0c | 3200 80 0c | 6400 00 19; Y: 400, 800, 3200 (changed), 3200 (kept), 6000 = 0x1770 (kept)
    let mut want = vec![0u8; 63];
    want[..5].copy_from_slice(&[0x48, 2, 2, 2, 5]);
    want[5..15].copy_from_slice(&[0x90, 0x01, 0x20, 0x03, 0x80, 0x0C, 0x80, 0x0C, 0x00, 0x19]);
    want[21..31].copy_from_slice(&[0x90, 0x01, 0x20, 0x03, 0x80, 0x0C, 0x80, 0x0C, 0x70, 0x17]);
    want[37] = 0x1F;
    assert_eq!(p.to_vec(), want);
    // before protocol 6 each connection byte carries the stage in both nibbles: 2 << 4 | 2 = 0x22; the enable flags are 8 bytes
    let old = kc::Identity { version: 5, ..ident };
    let mut b5 = b.clone();
    b5[37..45].copy_from_slice(&[1, 1, 1, 1, 1, 0, 0, 0]);
    let cur5 = kc::m6_dpi_from_xy(&b5, &old);
    let p5 = kc::m6_encode_dpi_xy(&cur5, &cur5, &old).unwrap();
    assert_eq!(p5[1..4], [0x22, 0x22, 0x22]);
    assert_eq!(p5[37..45], [1, 1, 1, 1, 1, 0, 0, 0]);
    // no X/Y read = no 0x48 form
    assert!(kc::m6_encode_dpi_xy(&kc::m6_dpi_from_status(&status_of(|_| {}), 0), &cur, &ident).is_none());
}

// ---------------------------------------------------------------- M6: decode, by hand

#[test]
fn m6_status_decodes_the_openmouse_way() {
    let st = status_of(|b| {
        b[2] = 0x32; // USB: stage 2, gear 3
        b[3] = 0x11;
        b[4] = 0x00;
        for (i, d) in [400u16, 800, 1600, 3200, 6400].iter().enumerate() {
            b[5 + 2 * i] = *d as u8;
            b[6 + 2 * i] = (*d >> 8) as u8;
        }
        b[15] = 0x03 | 0x04; // lod code 3 (0.7 mm), ripple control
        b[16] = 4; // four stages in use
        b[19] = 0x80 | 75; // charging, 75 %
        b[40] = 0x30;
        b[41] = 0x75; // 0x7530 = 30000
        b[42] = 100;
        b[43..49].copy_from_slice(&[0, 1, 2, 3, 4, 5]);
        b[49] = 0; // 0 = all six
        b[61] = 0x32; // level 2 of 3
    });
    assert_eq!(st.levels, [0x32, 0x11, 0x00]);
    assert_eq!(st.dpi_stages, [400, 800, 1600, 3200, 6400]);
    assert_eq!((st.stage_count, st.lod, st.ripple_control), (4, 3, true));
    assert_eq!((st.battery_percent, st.charging), (75, true));
    assert_eq!((st.dpi_max, st.dpi_step, st.polling_count), (30000, 100, 6));
    assert_eq!((st.lod_level, st.lod_count), (2, 3));
    assert_eq!(st.polling_table, vec![0, 1, 2, 3, 4, 5]);
    // keychronActiveStage / ActiveGear: USB = connection 0: low nibble 2 = stage 2, high nibble 3 = gear 3
    let d = kc::m6_dpi_from_status(&st, 0);
    assert_eq!((d.active, d.count, d.stages[d.active]), (2, 4, 1600));
    // 2.4 GHz = connection 1: 0x11 = stage 1; Bluetooth = connection 2: stage 0; the stage is kept below the count
    assert_eq!(kc::m6_dpi_from_status(&st, 1).active, 1);
    assert_eq!(kc::m6_dpi_from_status(&st, 2).active, 0);
    assert_eq!(kc::m6_gears_from_status(&st, 0).level, 3);
    assert_eq!(kc::m6_gears_from_status(&st, 1).level, 1);
    // a status that is too short or does not start with 6 is no status
    let ident = kc::m6_decode_version(None, 0);
    assert!(kc::m6_decode_status(&[6u8; 50], &ident).is_none());
    assert!(kc::m6_decode_status(&[7u8; 63], &ident).is_none());
}

#[test]
fn m6_feature_flags_come_from_the_version_answer_from_protocol_6() {
    // 0x02 answer: [1..3] = version LE, [5..7] = the paired mouse's product id, [9] & 7 = work mode, flags at [11] [12] [13] [15]
    let mut v = vec![0u8; 20];
    v[0] = 0x02;
    v[1] = 6;
    v[5] = 0x60;
    v[6] = 0xD0;
    v[9] = 0x01;
    v[11] = 0x18; // DPI limits + polling gears
    v[13] = 0x03; // separate DPI + separate polling
    v[15] = 0x10; // lift-off levels
    let id = kc::m6_decode_version(Some(&v), 0);
    assert_eq!((id.version, id.product_id, id.work_mode, id.flags), (6, Some(0xD060), 1, [0x18, 0, 0x03, 0x10]));
    let mut b = [0u8; 63];
    b[0] = 6;
    b[26] = 0x80; // an OLD flag position: ignored from protocol 6
    let f = kc::m6_decode_status(&b, &id).unwrap().features;
    assert!(f.dpi_limits && f.polling_gears && f.separate_dpi && f.separate_polling && f.lod_level && !f.fps20k);
    // before protocol 6 the flags are status bytes [26], [53], [60]
    let old = kc::m6_decode_version(Some(&[0x02, 5, 0, 0, 0, 0, 0, 0, 0, 0]), 0);
    b[26] = 0x80;
    b[53] = 0x04;
    b[60] = 0x01;
    let f = kc::m6_decode_status(&b, &old).unwrap().features;
    assert!(f.fps20k && f.separate_dpi && !f.polling_gears && !f.lod_level);
    // no version answer: the work mode falls back to the id's (1 behind a receiver), no flags
    let none = kc::m6_decode_version(None, 1);
    assert_eq!((none.work_mode, none.version, none.product_id), (1, 0, None));
    // keychronDecodeConnectedMouse: [1] = count, then 5 bytes per mouse: vid LE, pid LE, state (1 = connected)
    let list = [0x03, 2, 0x34, 0x34, 0x33, 0xD0, 0, 0x34, 0x34, 0x60, 0xD0, 1];
    assert_eq!(kc::m6_connected_mouse(&list), Some(0xD060));
    assert_eq!(kc::m6_connected_mouse(&[0x03, 1, 0x34, 0x34, 0x60, 0xD0, 0]), None);
    assert_eq!(kc::m6_connected_mouse(&[0x03, 0]), None);
}

#[test]
fn m6_limits_follow_the_model_and_the_firmware() {
    // the M6 row: DPI 100..26000, step 50, lift-off [[3, 0.7], [1, 1], [2, 2]], polling up to 1000 Hz
    let plain = status_of(|_| {});
    assert_eq!(kc::m6_dpi_limits(&plain), (100, 26000, 50));
    assert!(kc::m6_dpi_ok(100, &plain) && kc::m6_dpi_ok(26000, &plain) && kc::m6_dpi_ok(850, &plain));
    assert!(!kc::m6_dpi_ok(99, &plain) && !kc::m6_dpi_ok(26050, &plain) && !kc::m6_dpi_ok(810, &plain));
    // firmware that flags its own limits: ceiling and step from [40..41] / [42] (when above the floor / non-zero)
    let ident = kc::Identity { work_mode: 0, version: 6, product_id: None, flags: [0x08, 0, 0, 0] };
    let mut b = [0u8; 63];
    b[0] = 6;
    b[40] = 0xB8;
    b[41] = 0x0B; // 3000
    b[42] = 100;
    let lim = kc::m6_decode_status(&b, &ident).unwrap();
    assert_eq!(kc::m6_dpi_limits(&lim), (100, 3000, 100));
    assert!(!kc::m6_dpi_ok(850, &lim) && kc::m6_dpi_ok(900, &lim) && !kc::m6_dpi_ok(3100, &lim));
    // lift-off: the 2-bit code, 0.7 / 1 / 2 mm
    let c = kc::m6_lift_off_choices(&plain);
    assert!(!c.level_mode);
    assert_eq!(c.tenths(), vec![7, 10, 20]);
    assert_eq!((c.code_for(7), c.code_for(10), c.code_for(20), c.code_for(15)), (Some(3), Some(1), Some(2), None));
    for (lod, t) in [(1u8, 10u32), (2, 20), (3, 7)] {
        assert_eq!(c.current(&status_of(|b| b[15] = lod)), Some(t));
    }
    assert_eq!(c.current(&status_of(|b| b[15] = 0)), None);
    // rates: gears fixed -> the rates the gears hold (3 of 6 in use); rewritable gears -> the model's up to 1000 Hz
    let fixed = status_of(|b| {
        b[43..49].copy_from_slice(&[2, 1, 0, 5, 5, 5]);
        b[49] = 3;
    });
    assert_eq!(kc::m6_supported_rates(&fixed, &kc::m6_gears_from_status(&fixed, 0)), vec![125, 500, 1000]);
    let gears = kc::m6_decode_status(&b, &kc::Identity { flags: [0x10, 0, 0, 0], ..ident }).unwrap();
    assert_eq!(kc::m6_supported_rates(&gears, &kc::m6_gears_from_status(&gears, 0)), vec![125, 500, 1000]);
}

// ---------------------------------------------------------------- the allow-list

#[test]
fn only_read_requests_pass_the_allow_list() {
    // Nape
    for c in [nrep(&[0xA7, 0x21]), nrep(&[0xA7, 0x31]), nrep(&[0xA7, 0x0D]), nrep(&[0xA7, 0x36]), nrep(&[0xA7, 0x24, 0]), nrep(&[0xA7, 0x24, 4])] {
        assert!(kc::read_request_allowed(&c), "{c:02x?}");
    }
    for c in [
        nrep(&kc::nape_set_dpi_value(1, 800)),
        nrep(&kc::nape_set_dpi_stage(1)),
        nrep(&kc::nape_set_custom_dpi(800)),
        nrep(&kc::nape_set_polling(1000).unwrap()),
        nrep(&[0xA7, 0x24, 5]),
        nrep(&[0xA7, 0x20]),
        nrep(&[0xA7, 0x2D, 1]),
        nrep(&[0xA1]),
        nrep(&[0x04, 0x00]),
        nrep(&[0xA7, 0x21, 1]),
    ] {
        assert!(!kc::read_request_allowed(&c), "{c:02x?}");
    }
    // M6
    for c in [mrep(0xB3, &[0x06]), mrep(0xB3, &[0x49]), mrep(0xB5, &[0x02, 0x00]), mrep(0xB5, &[0x02, 0x01]), mrep(0xB5, &[0x03]), mrep(0xB5, &[0x4B])] {
        assert!(kc::read_request_allowed(&c), "{c:02x?}");
    }
    let st = status_of(|_| {});
    let gears = kc::m6_gears_from_status(&st, 0);
    let writes = [
        mrep(0xB5, &kc::m6_encode_dpi(0, &[800; 5], 5)),
        mrep(0xB5, &kc::m6_encode_polling(&gears, &gears)),
        mrep(0xB5, &kc::m6_encode_sensor(&st, false, 1)),
        mrep(0xB3, &[0x48, 1, 1, 1, 5]),
        mrep(0xB3, &[0x52, 1]),
        mrep(0xB3, &[0x04, 0]),
        mrep(0xB3, &[0x62, 1]),
        mrep(0xB5, &[0x0A, 1, 5]),
        mrep(0xB5, &[0x0E, 1]),
        mrep(0xB5, &[0x24, 1, 2, 3]),
        mrep(0xB5, &[0x02, 0x05]),
        mrep(0xB5, &[0x03, 0x00, 0x01]),
        mrep(0xB3, &[0x06, 0x00, 0x01]),
        mrep(0xB4, &[0x06]),
        mrep(0xB7, &[0x06]),
        vec![],
        vec![0xB3, 0x06],
    ];
    for c in writes {
        assert!(!kc::read_request_allowed(&c), "{c:02x?}");
    }
}

// ---------------------------------------------------------------- the device table

#[test]
fn the_supported_ids_match_and_nothing_else_does() {
    // OpenMouse's SUPPORTED list: Keychron Nape Pro (0x440), Keychron M6 (0xd060, 0xd029)
    let n = nape_col();
    let m = kc::matches(&[&n], 0x3434, 0x0440).unwrap();
    assert_eq!((m.family, m.model, m.wireless, m.variant), (Family::Keychron, Some("Keychron Nape Pro"), Some(false), 0x0440));
    for (pid, wireless) in [(0xD060u16, false), (0xD029, true)] {
        let c = m6_col(pid);
        let m = kc::matches(&[&c], 0x3434, pid).unwrap_or_else(|| panic!("{pid:04x}"));
        assert_eq!((m.family, m.model, m.wireless, m.variant), (Family::Keychron, Some("Keychron M6"), Some(wireless), pid));
        assert_eq!(m.config.path, c.path);
        for (name, f) in others() {
            assert!(f(&[&c], 0x3434, pid).is_none(), "{name} must not claim {pid:04x}");
        }
        assert_eq!(family::find(std::slice::from_ref(&c), 0x3434, pid).map(|m| m.family), Some(Family::Keychron));
    }
    for (name, f) in others() {
        assert!(f(&[&n], 0x3434, 0x0440).is_none(), "{name} must not claim the Nape Pro");
    }
    assert_eq!(kc::MOUSE_PRODUCT_IDS, [0x0440, 0xD060, 0xD029]);
    // the other 51 Keychron rows are status "pr": a Keychron M3 (0xd033), M5 8K (0xd048) ... do not match
    for pid in [0xD033u16, 0xD048, 0xD049, 0xD03F, 0xD064, 0xD077, 0xD026, 0xD05A, 0xD040] {
        assert!(kc::matches(&[&m6_col(pid)], 0x3434, pid).is_none(), "{pid:04x}");
    }
    // wrong vendor
    assert!(kc::matches(&[&m6_col(0xD060)], 0x3710, 0xD060).is_none());
    // the wrong collection: the M6 needs 0xFFC1/1, the Nape 0xFF60/0x61
    let mut keymap = m6_col(0xD060);
    keymap.usage_page = 0xFF60;
    keymap.usage = 0x61;
    assert!(kc::matches(&[&keymap], 0x3434, 0xD060).is_none(), "the VIA raw collection is not the M6's");
    assert!(kc::matches(&[&m6_col(0x0440)], 0x3434, 0x0440).is_none(), "the Nape wants 0xFF60/0x61");
    // the Nape behind a receiver is not on the list: the receiver with only the VIA collection does not match
    let mut via = hid_info("recv", 0x3434, 0xD029, 0xFF60, 0x61, (33, 33, 0));
    via.interface = Some(1);
    assert!(kc::matches(&[&via], 0x3434, 0xD029).is_none());
    // report lengths too short for a 63-byte command report / a 32-byte raw report
    let mut short = m6_col(0xD060);
    short.output_len = 21;
    assert!(kc::matches(&[&short], 0x3434, 0xD060).is_none());
    let mut short_n = nape_col();
    short_n.input_len = 0;
    assert!(kc::matches(&[&short_n], 0x3434, 0x0440).is_none());
    // of a mouse collection + the config one, the config one
    let mouse = hid_info("mouse", 0x3434, 0xD060, 0x01, 0x02, (8, 0, 0));
    let c = m6_col(0xD060);
    assert_eq!(kc::matches(&[&mouse, &c], 0x3434, 0xD060).map(|m| m.config.path), Some(c.path.clone()));
}

// ---------------------------------------------------------------- the Nape against a scripted mouse

struct NapeModel {
    active: u8,
    values: [u32; 5],
    custom: u32,
    battery: (u8, u8),
    /// the answer to the polling read, bytes 2.. (OpenMouse's numbering)
    polling: Vec<u8>,
    silent: bool,
    /// ignore every write
    refuse_writes: bool,
    /// the stage value only follows the custom DPI command (a firmware that ignores setDpiValue)
    stage_ignores_value: bool,
    log: Vec<(Vec<u8>, HidTransfer)>,
}

impl NapeModel {
    fn new() -> Self {
        NapeModel { active: 1, values: [400, 800, 1600, 2400, 4000], custom: 800, battery: (64, 0), polling: vec![0, 0, 0, 0x58, 0x03], silent: false, refuse_writes: false, stage_ignores_value: false, log: Vec::new() }
    }
}

fn nape_port(m: Rc<RefCell<NapeModel>>) -> ScriptPort {
    ScriptPort::new(nape_col(), move |out, how| {
        let mut m = m.borrow_mut();
        m.log.push((out.to_vec(), how.clone()));
        let c = &out[1..];
        match how {
            HidTransfer::OutputOnly => {
                if c[0] != 0xA7 {
                    panic!("not a misc command: {c:02x?}");
                }
                if !m.refuse_writes {
                    match c[1] {
                        0x23 if !m.stage_ignores_value => m.values[c[2] as usize] = c[3] as u32 | (c[4] as u32) << 8,
                        0x23 => {}
                        0x22 => m.active = c[2],
                        0x37 => {
                            m.custom = c[2] as u32 | (c[3] as u32) << 8;
                            if m.stage_ignores_value {
                                let a = m.active as usize;
                                let v = m.custom;
                                m.values[a] = v;
                            }
                        }
                        0x0E => {
                            let shift = c[2];
                            m.polling[3] = 0x58;
                            m.polling[4] = shift;
                        }
                        other => panic!("the driver sent command {other:02x}"),
                    }
                }
                Ok(Vec::new())
            }
            HidTransfer::OutputThenMatch { reply_len, prefixes, .. } => {
                if m.silent {
                    return Err(Error::BadAnswer("no matching answer".into()));
                }
                let mut a = vec![0u8; 33];
                a[1] = 0xA7;
                a[2] = c[1];
                match c[1] {
                    0x21 => a[3] = m.active,
                    0x24 => {
                        let v = m.values[c[2] as usize];
                        a[3] = v as u8;
                        a[4] = (v >> 8) as u8;
                    }
                    0x31 => {
                        a[3] = m.battery.0;
                        a[4] = m.battery.1;
                    }
                    0x0D => {
                        for (i, b) in m.polling.iter().enumerate() {
                            a[3 + i] = *b;
                        }
                    }
                    0x36 => {
                        a[3] = m.custom as u8;
                        a[4] = (m.custom >> 8) as u8;
                    }
                    other => panic!("the driver asked command {other:02x}"),
                }
                assert!(prefixes.iter().any(|p| a.starts_with(p)), "the answer must match the request's prefix: {prefixes:02x?}");
                a.resize(*reply_len, 0);
                Ok(a)
            }
            other => panic!("the Nape talks in output reports, not {other:?}"),
        }
    })
}

fn nape_setup() -> (Matched, Rc<RefCell<NapeModel>>, ScriptPort) {
    let info = nape_col();
    let m = kc::matches(&[&info], 0x3434, 0x0440).unwrap();
    let model = Rc::new(RefCell::new(NapeModel::new()));
    (m, model.clone(), nape_port(model))
}

fn nape_writes(m: &Rc<RefCell<NapeModel>>) -> Vec<Vec<u8>> {
    m.borrow().log.iter().filter(|(_, h)| matches!(h, HidTransfer::OutputOnly)).map(|(o, _)| o.clone()).collect()
}

#[test]
fn the_nape_reads_what_openmouse_reads() {
    let (m, model, mut p) = nape_setup();
    let r = kc::Keychron.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(r.online);
    assert_eq!((r.dpi, r.polling_hz, r.battery_percent, r.charging, r.lift_off), (Some(800), Some(1000), Some(64), Some(false), None));
    assert_eq!(r.model.as_deref(), Some("Keychron Nape Pro"));
    assert_eq!(r.wireless, Some(false));
    assert_eq!(r.caps.polling, vec![125, 500, 1000]);
    let d = r.caps.dpi.clone().unwrap();
    assert_eq!((d.min(), d.max()), (50, 4000));
    assert!(d.allows(4000) && !d.allows(4050) && !d.allows(75));
    assert!(r.caps.lift_off.is_empty() && r.caps.battery);
    // the requests, in order: the active stage, that stage's value, the battery, the polling - all reads, all on the allow-list
    let sent: Vec<Vec<u8>> = model.borrow().log.iter().map(|(o, _)| o.clone()).collect();
    assert_eq!(sent, vec![nrep(&[0xA7, 0x21]), nrep(&[0xA7, 0x24, 1]), nrep(&[0xA7, 0x31]), nrep(&[0xA7, 0x0D])]);
    assert!(sent.iter().all(|o| kc::read_request_allowed(o)));
    let how = model.borrow().log[0].1.clone();
    assert_eq!(how, HidTransfer::OutputThenMatch { reply_len: 33, max_reads: 32, timeout_ms: 1200, prefixes: vec![vec![0x00, 0xA7, 0x21]] });
}

#[test]
fn a_silent_or_firmwareless_nape_shows_only_what_it_answers() {
    let (m, model, mut p) = nape_setup();
    model.borrow_mut().silent = true;
    let r = kc::Keychron.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(!r.online);
    assert_eq!((r.dpi, r.polling_hz, r.battery_percent), (None, None, None));
    assert!(r.caps.polling.is_empty(), "no polling row for a mouse that told nothing");
    // a polling answer of all zeros (firmware without it): no polling data, the rest as usual
    let (m, model, mut p) = nape_setup();
    model.borrow_mut().polling = vec![0; 5];
    let r = kc::Keychron.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(r.online);
    assert_eq!((r.dpi, r.polling_hz, r.battery_percent), (Some(800), None, Some(64)));
    assert!(r.caps.polling.is_empty());
    // a battery byte over 100 is no percentage
    let (m, model, mut p) = nape_setup();
    model.borrow_mut().battery = (255, 0);
    let r = kc::Keychron.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!((r.battery_percent, r.charging), (None, None));
}

#[test]
fn the_nape_dpi_change_is_openmouses_write_sequence() {
    let (m, model, mut p) = nape_setup();
    let c = kc::Keychron.write(&m, &mut p, Setting::Dpi(1600)).unwrap();
    assert_eq!((c.now, c.before), (1600, Some(800)));
    // writeDpiStageValue: setDpiValue(stage 1, 1600), (the stage is the active one) setDpiStage(1) - the only writes
    assert_eq!(nape_writes(&model), vec![nrep(&[0xA7, 0x23, 0x01, 0x40, 0x06]), nrep(&[0xA7, 0x22, 0x01])]);
    assert_eq!(model.borrow().values, [400, 1600, 1600, 2400, 4000]);
    // reads in between: the stage, its value (before), [write], the stage, [write], the value (confirm)
    let kinds: Vec<u8> = model.borrow().log.iter().map(|(o, _)| o[2]).collect();
    assert_eq!(kinds, vec![0x21, 0x24, 0x23, 0x21, 0x22, 0x24]);
    assert!(model.borrow().log.iter().filter(|(_, h)| !matches!(h, HidTransfer::OutputOnly)).all(|(o, _)| kc::read_request_allowed(o)));
    // out of range / off-step values are refused before a byte leaves
    model.borrow_mut().log.clear();
    for bad in [49, 4050, 810, 0] {
        assert!(kc::Keychron.write(&m, &mut p, Setting::Dpi(bad)).is_err(), "{bad}");
    }
    assert!(model.borrow().log.is_empty());
}

#[test]
fn the_nape_dpi_falls_back_to_the_custom_command_like_openmouse() {
    // a firmware whose stage value only follows the custom DPI command: OpenMouse writes setCustomDpi and reads it back
    let (m, model, mut p) = nape_setup();
    model.borrow_mut().stage_ignores_value = true;
    let c = kc::Keychron.write(&m, &mut p, Setting::Dpi(1600)).unwrap();
    assert_eq!(c.now, 1600);
    assert_eq!(nape_writes(&model), vec![nrep(&[0xA7, 0x23, 0x01, 0x40, 0x06]), nrep(&[0xA7, 0x22, 0x01]), nrep(&[0xA7, 0x37, 0x40, 0x06])]);
    // a mouse that ignores everything: the change is an error, not a silent pass
    let (m, model, mut p) = nape_setup();
    model.borrow_mut().refuse_writes = true;
    assert!(matches!(kc::Keychron.write(&m, &mut p, Setting::Dpi(1600)), Err(Error::BadAnswer(_))));
    assert_eq!(model.borrow().values[1], 800);
}

#[test]
fn the_nape_polling_change_is_openmouses_write() {
    let (m, model, mut p) = nape_setup();
    let c = kc::Keychron.write(&m, &mut p, Setting::Polling(500)).unwrap();
    assert_eq!((c.now, c.before), (500, Some(1000)));
    assert_eq!(nape_writes(&model), vec![nrep(&[0xA7, 0x0E, 0x04, 0x04])]);
    // a rate this connection does not offer: refused before a write; no lift-off on the Nape
    model.borrow_mut().log.clear();
    assert!(kc::Keychron.write(&m, &mut p, Setting::Polling(8000)).is_err());
    assert!(kc::Keychron.write(&m, &mut p, Setting::LiftOff(10)).is_err());
    assert!(nape_writes(&model).is_empty());
    // a mouse that keeps its rate
    let (m, model, mut p) = nape_setup();
    model.borrow_mut().refuse_writes = true;
    assert!(matches!(kc::Keychron.write(&m, &mut p, Setting::Polling(500)), Err(Error::BadAnswer(_))));
    // a firmware without polling data cannot be written
    let (m, model, mut p) = nape_setup();
    model.borrow_mut().polling = vec![0; 5];
    assert!(kc::Keychron.write(&m, &mut p, Setting::Polling(500)).is_err());
    assert!(nape_writes(&model).is_empty());
}

#[test]
fn a_gone_nape_is_an_error_not_offline() {
    let info = nape_col();
    let m = kc::matches(&[&info], 0x3434, 0x0440).unwrap();
    let mut p = ScriptPort::new(info, |_, _| Err(Error::MouseGone("nape".into())));
    assert!(matches!(kc::Keychron.read(&m, &mut p, ReadOptions::default()), Err(Error::MouseGone(_))));
}

// ---------------------------------------------------------------- the M6 against a scripted mouse

#[derive(Clone)]
struct M6Model {
    // the 0x02 answer
    version: u16,
    work_mode: u8,
    product_id: u16,
    flags: [u8; 4],
    version_silent: bool,
    // the status
    levels: [u8; 3],
    stages: [u16; 5],
    f15: u8,
    stage_count: u8,
    /// Order 110: debounce ms (status byte 17)
    debounce: u8,
    battery_byte: u8,
    dpi_max: u16,
    dpi_step: u8,
    table: [u8; 6],
    poll_count: u8,
    b52: u8,
    b61: u8,
    old_flags: [u8; 3],
    status_silent: bool,
    // the separate X/Y read
    y: [u16; 5],
    enable: u8,
    // the separate polling sets: (level, count, table)
    sets: [(u8, u8, [u8; 6]); 2],
    // the 0x03 answer
    list: Vec<u8>,
    ack_code: u8,
    refuse_writes: bool,
    log: Vec<(Vec<u8>, HidTransfer)>,
}

impl M6Model {
    fn new() -> Self {
        M6Model {
            version: 5,
            work_mode: 0,
            product_id: 0xD060,
            flags: [0; 4],
            version_silent: false,
            levels: [0x22, 0x11, 0x00],
            stages: [400, 800, 1600, 3200, 6400],
            f15: 0x15,
            stage_count: 5,
            debounce: 4,
            battery_byte: 0x80 | 75,
            dpi_max: 0,
            dpi_step: 0,
            table: [0, 1, 2, 0, 0, 0],
            poll_count: 3,
            b52: 0,
            b61: 0,
            old_flags: [0; 3],
            status_silent: false,
            y: [400, 800, 1500, 3200, 6000],
            enable: 0x1F,
            sets: [(2, 3, [0, 1, 2, 0, 0, 0]), (0, 3, [0, 1, 2, 0, 0, 0])],
            list: vec![0x03, 0],
            ack_code: 0,
            refuse_writes: false,
            log: Vec::new(),
        }
    }

    fn v6(mut self, flags: [u8; 4]) -> Self {
        self.version = 6;
        self.flags = flags;
        self
    }

    fn status(&self) -> Vec<u8> {
        let mut b = vec![0u8; 63];
        b[0] = 6;
        b[2..5].copy_from_slice(&self.levels);
        for (i, s) in self.stages.iter().enumerate() {
            b[5 + 2 * i] = *s as u8;
            b[6 + 2 * i] = (*s >> 8) as u8;
        }
        b[15] = self.f15;
        b[16] = self.stage_count;
        b[17] = self.debounce;
        b[19] = self.battery_byte;
        b[26] = self.old_flags[0];
        b[40] = self.dpi_max as u8;
        b[41] = (self.dpi_max >> 8) as u8;
        b[42] = self.dpi_step;
        b[43..49].copy_from_slice(&self.table);
        b[49] = self.poll_count;
        b[52] = self.b52;
        b[53] = self.old_flags[1];
        b[60] = self.old_flags[2];
        b[61] = self.b61;
        b
    }

    fn version_answer(&self) -> Vec<u8> {
        let mut v = vec![0u8; 20];
        v[0] = 0x02;
        v[1] = self.version as u8;
        v[2] = (self.version >> 8) as u8;
        v[5] = self.product_id as u8;
        v[6] = (self.product_id >> 8) as u8;
        v[9] = self.work_mode;
        v[11] = self.flags[0];
        v[12] = self.flags[1];
        v[13] = self.flags[2];
        v[15] = self.flags[3];
        v
    }

    fn xy_answer(&self) -> Vec<u8> {
        let mut b = vec![0u8; 63];
        b[0] = 0x49;
        b[1..4].copy_from_slice(&self.levels.map(|l| l & 0x0F));
        b[4] = self.stage_count;
        for i in 0..5 {
            b[5 + 2 * i] = self.stages[i] as u8;
            b[6 + 2 * i] = (self.stages[i] >> 8) as u8;
            b[21 + 2 * i] = self.y[i] as u8;
            b[22 + 2 * i] = (self.y[i] >> 8) as u8;
        }
        b[37] = self.enable;
        b
    }

    fn sets_answer(&self) -> Vec<u8> {
        let mut b = vec![0u8; 20];
        b[0] = 0x4B;
        for s in 0..2 {
            b[1 + s] = self.sets[s].0;
            b[3 + s] = self.sets[s].1;
            b[5 + 6 * s..11 + 6 * s].copy_from_slice(&self.sets[s].2);
        }
        b
    }

    /// Apply a write the way the mouse would (independently of the driver's encoder: by the byte offsets of the packets).
    fn apply(&mut self, id: u8, p: &[u8]) {
        if self.refuse_writes {
            return;
        }
        let wm = (self.work_mode as usize).min(2);
        match (id, p[0]) {
            (0xB5, 0x40) => {
                for i in 0..3 {
                    self.levels[i] = (self.levels[i] & 0xF0) | p[1 + i];
                }
                for i in 0..5 {
                    self.stages[i] = p[4 + 2 * i] as u16 | (p[5 + 2 * i] as u16) << 8;
                }
                self.stage_count = p[14];
            }
            (0xB3, 0x48) => {
                for i in 0..3 {
                    self.levels[i] = (self.levels[i] & 0xF0) | (p[1 + i] & 0x0F);
                }
                self.stage_count = p[4];
                for i in 0..5 {
                    self.stages[i] = p[5 + 2 * i] as u16 | (p[6 + 2 * i] as u16) << 8;
                    self.y[i] = p[21 + 2 * i] as u16 | (p[22 + 2 * i] as u16) << 8;
                }
                self.enable = p[37];
            }
            (0xB5, 0x41) => {
                self.levels[wm] = (p[1] << 4) | (self.levels[wm] & 0x0F);
                self.poll_count = p[9];
                self.table[..p[9] as usize].copy_from_slice(&p[3..3 + p[9] as usize]);
            }
            (0xB5, 0x4A) => {
                for s in 0..2 {
                    self.sets[s] = (p[1 + s], p[3 + s], p[5 + 6 * s..11 + 6 * s].try_into().unwrap());
                }
            }
            (0xB5, 0x42) => {
                if self.b61 == 0 && self.flags[3] & 0x10 == 0 {
                    self.f15 = (self.f15 & !0x03) | (p[1] & 0x03);
                } else {
                    self.b61 = (self.b61 & 0xF0) | (p[11] & 0x0F);
                }
                // Order 110: the switches (1 on, 2 off) into the flags byte: ripple 0x04, angle snapping 0x08, Motion Sync 0x10
                for (tok, bit) in [(p[2], 0x04u8), (p[3], 0x08), (p[4], 0x10)] {
                    match tok {
                        1 => self.f15 |= bit,
                        2 => self.f15 &= !bit,
                        _ => {}
                    }
                }
            }
            (0xB5, 0x43) => self.debounce = p[1],
            other => panic!("unexpected write {other:02x?}"),
        }
    }
}

fn m6_port(pid: u16, model: Rc<RefCell<M6Model>>) -> ScriptPort {
    ScriptPort::new(m6_col(pid), move |out, how| {
        let mut m = model.borrow_mut();
        m.log.push((out.to_vec(), how.clone()));
        assert_eq!(out.len(), 64, "Windows wants the collection's full output length");
        let HidTransfer::OutputThenMatch { reply_len, prefixes, .. } = how else { panic!("the M6 uses output reports + input matches, not {how:?}") };
        let (id, p) = (out[0], &out[1..]);
        let (answer_id, data): (u8, Vec<u8>) = match (id, p[0]) {
            (0xB5, 0x02) => {
                if m.version_silent {
                    return Err(Error::BadAnswer("no matching answer".into()));
                }
                (0xB6, m.version_answer())
            }
            (0xB5, 0x03) => (0xB6, m.list.clone()),
            (0xB5, 0x4B) => (0xB6, m.sets_answer()),
            (0xB3, 0x06) => {
                if m.status_silent {
                    return Err(Error::BadAnswer("no matching answer".into()));
                }
                (0xB4, m.status())
            }
            (0xB3, 0x49) => (0xB4, m.xy_answer()),
            (0xB5, c @ (0x40 | 0x41 | 0x42 | 0x43 | 0x4A)) | (0xB3, c @ 0x48) => {
                m.apply(id, p);
                let ack = vec![0xE4, m.ack_code, c];
                (if id == 0xB3 { 0xB4 } else { 0xB6 }, ack)
            }
            other => panic!("unexpected request {other:02x?}"),
        };
        let mut a = vec![answer_id];
        a.extend_from_slice(&data);
        assert!(prefixes.iter().any(|pre| a.starts_with(pre)), "the answer {:02x?} must match one of the request's prefixes", &a[..4]);
        a.resize(*reply_len, 0);
        Ok(a)
    })
}

fn m6_setup(pid: u16, model: M6Model) -> (Matched, Rc<RefCell<M6Model>>, ScriptPort) {
    let info = m6_col(pid);
    let m = kc::matches(&[&info], 0x3434, pid).unwrap();
    let model = Rc::new(RefCell::new(model));
    (m, model.clone(), m6_port(pid, model))
}

fn m6_writes(m: &Rc<RefCell<M6Model>>) -> Vec<Vec<u8>> {
    m.borrow().log.iter().filter(|(o, _)| !kc::read_request_allowed(o)).map(|(o, _)| o.clone()).collect()
}

#[test]
fn the_wired_m6_reads_what_openmouse_reads() {
    let (m, model, mut p) = m6_setup(0xD060, M6Model::new());
    let r = kc::Keychron.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(r.online);
    // levels[0] = 0x22: stage 2 (1600), gear 2 (table[2] = 2 -> 1000 Hz); flags 0x15: lift-off code 1 = 1 mm; 75 %, charging
    assert_eq!((r.dpi, r.polling_hz, r.lift_off), (Some(1600), Some(1000), Some(10)));
    assert_eq!((r.battery_percent, r.charging, r.wireless), (Some(75), Some(true), Some(false)));
    assert_eq!(r.model.as_deref(), Some("Keychron M6"));
    assert_eq!(r.caps.polling, vec![125, 500, 1000]);
    assert_eq!(r.caps.lift_off, vec![7, 10, 20]);
    let d = r.caps.dpi.clone().unwrap();
    assert_eq!((d.min(), d.max()), (100, 26000));
    assert!(d.allows(100) && d.allows(26000) && d.allows(850) && !d.allows(810));
    assert!(r.caps.battery);
    // two reads: the version / connection (0x02 on 0xb5) and the status (0x06 on 0xb3); both on the allow-list
    let sent: Vec<Vec<u8>> = model.borrow().log.iter().map(|(o, _)| o.clone()).collect();
    assert_eq!(sent, vec![mrep(0xB5, &[0x02, 0x00]), mrep(0xB3, &[0x06])]);
    assert!(sent.iter().all(|o| kc::read_request_allowed(o)));
    // the answer may come on either answer report id: the 0x02 query accepts [0xb4, 2] and [0xb6, 2]
    let how = model.borrow().log[0].1.clone();
    let HidTransfer::OutputThenMatch { reply_len, max_reads, timeout_ms, prefixes } = how else { panic!() };
    assert_eq!((reply_len, max_reads, timeout_ms), (64, 32, 1200));
    assert_eq!(prefixes, vec![vec![0xB4, 0x02], vec![0xB6, 0x02]]);
}

#[test]
fn the_m6_dpi_change_is_openmouses_0x40_packet() {
    let (m, model, mut p) = m6_setup(0xD060, M6Model::new());
    let c = kc::Keychron.write(&m, &mut p, Setting::Dpi(3200)).unwrap();
    assert_eq!((c.now, c.before), (3200, Some(1600)));
    // stage 2 (levels[0] = 0x22), the five slots with slot 2 = 3200 = 0x0c80, five stages
    assert_eq!(m6_writes(&model), vec![mrep(0xB5, &[0x40, 2, 2, 2, 0x90, 0x01, 0x20, 0x03, 0x80, 0x0C, 0x80, 0x0C, 0x00, 0x19, 5])]);
    assert_eq!(model.borrow().stages, [400, 800, 3200, 3200, 6400]);
    // the answer to a 0x40 write is accepted as the command's ack on the settings answer report (any ack code is a prefix)
    let how = model.borrow().log.iter().find(|(o, _)| o[1] == 0x40).unwrap().1.clone();
    let HidTransfer::OutputThenMatch { prefixes, .. } = how else { panic!() };
    assert!(prefixes.contains(&vec![0xB6, 0x40]) && prefixes.contains(&vec![0xB6, 0xE4, 0x00, 0x40]) && prefixes.contains(&vec![0xB4, 0xE4, 0x07, 0x40]));
    // refused before a byte leaves
    model.borrow_mut().log.clear();
    for bad in [99, 26050, 810] {
        assert!(kc::Keychron.write(&m, &mut p, Setting::Dpi(bad)).is_err(), "{bad}");
    }
    assert!(m6_writes(&model).is_empty());
}

#[test]
fn the_m6_polling_change_is_openmouses_0x41_packet() {
    let (m, model, mut p) = m6_setup(0xD060, M6Model::new());
    let c = kc::Keychron.write(&m, &mut p, Setting::Polling(500)).unwrap();
    assert_eq!((c.now, c.before), (500, Some(1000)));
    // gear 1 holds 500 Hz: level 1, the three gears in use [0, 1, 2], count 3
    assert_eq!(m6_writes(&model), vec![mrep(0xB5, &[0x41, 1, 1, 0, 1, 2, 0, 0, 0, 3])]);
    // 2000 Hz is in no gear of this connection and the gears cannot be rewritten: refused, nothing written
    model.borrow_mut().log.clear();
    assert!(kc::Keychron.write(&m, &mut p, Setting::Polling(2000)).is_err());
    assert!(m6_writes(&model).is_empty());
}

#[test]
fn the_m6_lift_off_change_is_openmouses_0x42_packet() {
    let (m, model, mut p) = m6_setup(0xD060, M6Model::new());
    let c = kc::Keychron.write(&m, &mut p, Setting::LiftOff(20)).unwrap();
    assert_eq!((c.now, c.before), (20, Some(10)));
    // lift-off code 2 (2 mm); ripple + motion sync on (flags 0x15) are re-sent as on, angle snapping off; scroll normal (1);
    // 20K FPS off (1: maxSpeed ? 2 : 1)
    assert_eq!(m6_writes(&model), vec![mrep(0xB5, &[0x42, 2, 1, 2, 1, 0, 1, 0, 1])]);
    // 0.7 mm = code 3
    kc::Keychron.write(&m, &mut p, Setting::LiftOff(7)).unwrap();
    assert_eq!(m6_writes(&model).last(), Some(&mrep(0xB5, &[0x42, 3, 1, 2, 1, 0, 1, 0, 1])));
    model.borrow_mut().log.clear();
    assert!(kc::Keychron.write(&m, &mut p, Setting::LiftOff(15)).is_err());
    assert!(m6_writes(&model).is_empty());
}

#[test]
fn m6_firmware_with_lift_off_levels_writes_the_level_byte() {
    // feature byte 4 bit 4 (version answer [15]) = levels: choices = the model's codes sorted by code, cut to the count the
    // mouse reports (high nibble of [61]); the current level is the low nibble
    let mut model = M6Model::new().v6([0, 0, 0, 0x10]);
    model.b61 = (3 << 4) | 1;
    let (m, model, mut p) = m6_setup(0xD060, model);
    let r = kc::Keychron.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!(r.lift_off, Some(10), "level 1 = code 1 = 1 mm");
    assert_eq!(r.caps.lift_off, vec![7, 10, 20]);
    kc::Keychron.write(&m, &mut p, Setting::LiftOff(7)).unwrap();
    // [1] = 0 and [11] = the level (code 3 = 0.7 mm)
    assert_eq!(m6_writes(&model), vec![mrep(0xB5, &[0x42, 0, 1, 2, 1, 0, 1, 0, 1, 0, 0, 3])]);
    // only two levels in use: the 0.7 mm code 3 is cut
    model.borrow_mut().b61 = (2 << 4) | 1;
    let r = kc::Keychron.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!(r.caps.lift_off, vec![10, 20]);
}

#[test]
fn m6_firmware_with_rewritable_polling_gears_rewrites_the_active_gear() {
    // version 6, feature byte 1 bit 4 = polling gears: a rate no gear holds is written into the ACTIVE gear's slot
    let mut model = M6Model::new().v6([0x10, 0, 0, 0]);
    model.levels[0] = 0x00; // gear 0, stage 0
    model.table = [0, 1, 3, 0, 0, 0]; // 125, 500, 2000
    let (m, model, mut p) = m6_setup(0xD060, model);
    let r = kc::Keychron.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!(r.polling_hz, Some(125));
    assert_eq!(r.caps.polling, vec![125, 500, 1000], "the model's rates up to 1000 Hz");
    let c = kc::Keychron.write(&m, &mut p, Setting::Polling(1000)).unwrap();
    assert_eq!((c.now, c.before), (1000, Some(125)));
    // the active gear 0 now holds index 2 (1000 Hz): table [2, 1, 3]
    assert_eq!(m6_writes(&model), vec![mrep(0xB5, &[0x41, 0, 0, 2, 1, 3, 0, 0, 0, 3])]);
    // 8000 Hz is beyond the M6 row
    assert!(kc::Keychron.write(&m, &mut p, Setting::Polling(8000)).is_err());
}

#[test]
fn m6_firmware_with_separate_polling_sets_writes_0x4a() {
    let model = M6Model::new().v6([0, 0, 0x02, 0]);
    let (m, model, mut p) = m6_setup(0xD060, model);
    let r = kc::Keychron.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!(r.polling_hz, Some(1000));
    // the read asks 0x4b on the settings report
    assert!(model.borrow().log.iter().any(|(o, _)| o[..2] == [0xB5, 0x4B]));
    let c = kc::Keychron.write(&m, &mut p, Setting::Polling(500)).unwrap();
    assert_eq!((c.now, c.before), (500, Some(1000)));
    // both sets sent, the USB set (set 0) at level 1: [4a, level0, level1, count0, count1, table0, table1]
    assert_eq!(m6_writes(&model), vec![mrep(0xB5, &[0x4A, 1, 0, 3, 3, 0, 1, 2, 0, 0, 0, 0, 1, 2, 0, 0, 0])]);
}

#[test]
fn m6_firmware_with_separate_xy_dpi_writes_0x48() {
    let model = M6Model::new().v6([0, 0, 0x01, 0]);
    let (m, model, mut p) = m6_setup(0xD060, model);
    let r = kc::Keychron.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!(r.dpi, Some(1600));
    assert!(model.borrow().log.iter().any(|(o, _)| o[..2] == [0xB3, 0x49]), "the X/Y read is on the command report");
    let c = kc::Keychron.write(&m, &mut p, Setting::Dpi(3200)).unwrap();
    assert_eq!((c.now, c.before), (3200, Some(1600)));
    let mut want = vec![0x48, 2, 2, 2, 5];
    want.resize(63, 0);
    want[5..15].copy_from_slice(&[0x90, 0x01, 0x20, 0x03, 0x80, 0x0C, 0x80, 0x0C, 0x00, 0x19]);
    want[21..31].copy_from_slice(&[0x90, 0x01, 0x20, 0x03, 0x80, 0x0C, 0x80, 0x0C, 0x70, 0x17]);
    want[37] = 0x1F;
    assert_eq!(m6_writes(&model), vec![mrep(0xB3, &want)]);
    // its answer is the ack on the command-answer report
    let how = model.borrow().log.iter().find(|(o, _)| o[..2] == [0xB3, 0x48]).unwrap().1.clone();
    let HidTransfer::OutputThenMatch { prefixes, .. } = how else { panic!() };
    assert!(prefixes.contains(&vec![0xB4, 0xE4, 0x00, 0x48]) && !prefixes.contains(&vec![0xB4, 0x48]));
    assert_eq!(model.borrow().y, [400, 800, 3200, 3200, 6000]);
}

#[test]
fn a_rejected_write_and_a_mouse_that_keeps_its_value_are_errors() {
    // the mouse acknowledges with code 7 = "not supported on this connection"
    let (m, model, mut p) = m6_setup(0xD060, M6Model::new());
    model.borrow_mut().ack_code = 7;
    assert!(matches!(kc::Keychron.write(&m, &mut p, Setting::Dpi(3200)), Err(Error::BadAnswer(_))));
    assert!(matches!(kc::Keychron.write(&m, &mut p, Setting::Polling(500)), Err(Error::BadAnswer(_))));
    assert!(matches!(kc::Keychron.write(&m, &mut p, Setting::LiftOff(20)), Err(Error::BadAnswer(_))));
    // acknowledged but not kept
    let (m, model, mut p) = m6_setup(0xD060, M6Model::new());
    model.borrow_mut().refuse_writes = true;
    for s in [Setting::Dpi(3200), Setting::Polling(500), Setting::LiftOff(20)] {
        assert!(matches!(kc::Keychron.write(&m, &mut p, s), Err(Error::BadAnswer(_))), "{s:?}");
    }
    assert_eq!(model.borrow().stages, [400, 800, 1600, 3200, 6400]);
}

#[test]
fn a_silent_m6_is_offline() {
    let (m, model, mut p) = m6_setup(0xD060, M6Model::new());
    model.borrow_mut().status_silent = true;
    let r = kc::Keychron.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(!r.online);
    assert_eq!((r.dpi, r.polling_hz, r.lift_off, r.battery_percent), (None, None, None, None));
    assert!(kc::Keychron.write(&m, &mut p, Setting::Dpi(3200)).is_err());
    assert!(m6_writes(&model).is_empty());
}

// ---------------------------------------------------------------- the M6 behind the receiver

#[test]
fn the_receiver_drives_an_m6_over_2_4_ghz() {
    // the 0x02 answer names the paired mouse (product id 0xd060) and the link (work mode 1): the 2.4 GHz connection's level
    // byte (levels[1] = 0x11: stage 1, gear 1) is the one in use
    let mut model = M6Model::new();
    model.work_mode = 1;
    let (m, model, mut p) = m6_setup(0xD029, model);
    assert_eq!(m.wireless, Some(true));
    let r = kc::Keychron.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(r.online);
    assert_eq!((r.dpi, r.polling_hz, r.wireless), (Some(800), Some(500), Some(true)));
    let sent: Vec<Vec<u8>> = model.borrow().log.iter().map(|(o, _)| o.clone()).collect();
    assert_eq!(sent, vec![mrep(0xB5, &[0x02, 0x01]), mrep(0xB3, &[0x06])]);
    // a DPI change writes the 2.4 GHz stage (1) into all three connection bytes, slot 1 changed
    let c = kc::Keychron.write(&m, &mut p, Setting::Dpi(1000)).unwrap();
    assert_eq!((c.now, c.before), (1000, Some(800)));
    assert_eq!(m6_writes(&model), vec![mrep(0xB5, &[0x40, 1, 1, 1, 0x90, 0x01, 0xE8, 0x03, 0x40, 0x06, 0x80, 0x0C, 0x00, 0x19, 5])]);
}

#[test]
fn the_receiver_names_the_mouse_from_the_connected_list_when_the_id_is_missing() {
    // the 0x02 answer has no product id: OpenMouse asks the 0x03 list (vid 0x3434 LE, pid LE, state 1)
    let mut model = M6Model::new();
    model.work_mode = 1;
    model.product_id = 0;
    model.list = vec![0x03, 1, 0x34, 0x34, 0x60, 0xD0, 1];
    let (m, model, mut p) = m6_setup(0xD029, model);
    let r = kc::Keychron.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(r.online);
    let sent: Vec<Vec<u8>> = model.borrow().log.iter().map(|(o, _)| o.clone()).collect();
    assert_eq!(sent, vec![mrep(0xB5, &[0x02, 0x01]), mrep(0xB5, &[0x03]), mrep(0xB3, &[0x06])]);
    assert!(sent.iter().all(|o| kc::read_request_allowed(o)));
    // no version answer at all (asleep): the work mode falls back to 1 behind a receiver, the list still names it
    let mut model = M6Model::new();
    model.version_silent = true;
    model.list = vec![0x03, 1, 0x34, 0x34, 0x60, 0xD0, 1];
    let (m, _model, mut p) = m6_setup(0xD029, model);
    let r = kc::Keychron.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(r.online, "the status still answers");
}

#[test]
fn a_receiver_with_nothing_paired_or_another_mouse_is_not_an_m6() {
    // nothing paired: the product id is 0 and the list is empty = cannot tell = offline, nothing written
    let mut model = M6Model::new();
    model.work_mode = 1;
    model.product_id = 0;
    let (m, model, mut p) = m6_setup(0xD029, model);
    let r = kc::Keychron.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert!(!r.online);
    assert!(kc::Keychron.write(&m, &mut p, Setting::Dpi(1000)).is_err());
    assert!(m6_writes(&model).is_empty());
    // another Keychron mouse paired (an M3, 0xd033): not on the supported list - refused, no status read
    let mut model = M6Model::new();
    model.work_mode = 1;
    model.product_id = 0xD033;
    let (m, model, mut p) = m6_setup(0xD029, model);
    assert!(matches!(kc::Keychron.read(&m, &mut p, ReadOptions::default()), Err(Error::UnsupportedMouse(_))));
    assert!(matches!(kc::Keychron.write(&m, &mut p, Setting::Dpi(1000)), Err(Error::UnsupportedMouse(_))));
    assert!(model.borrow().log.iter().all(|(o, _)| o[1] != 0x06), "no status read from a mouse that is not an M6");
    assert!(m6_writes(&model).is_empty());
}

// ---------------------------------------------------------------- Order 110 (M6): debounce, Motion Sync, angle snapping, ripple

#[test]
fn the_m6_extras_are_openmouses_0x42_and_0x43_packets() {
    let (m, model, mut p) = m6_setup(0xD060, M6Model::new());
    // keychronDecodeSettings: debounceMs = status [17] = 4; flags [15] = 0x15: ripple (0x04) on, angle snapping (0x08) off,
    // Motion Sync (0x10) on
    let r = kc::Keychron.read(&m, &mut p, ReadOptions::default()).unwrap();
    let got: Vec<(Extra, Option<i32>)> = r.extras.iter().map(|e| (e.extra, e.value)).collect();
    assert_eq!(got, vec![(Extra::Debounce, Some(4)), (Extra::MotionSync, Some(1)), (Extra::AngleSnap, Some(0)), (Extra::RippleControl, Some(1))]);
    // KEYCHRON_DEBOUNCE_MAX_MS = 20
    assert_eq!(r.extras[0].kind, Kind::Range { min: 0, max: 20, step: 1, unit: "ms" });
    // writeSensorFlag("motionSync", false): keychronEncodeSensor of the status with motionSync off - [1] lod code 1, [2] ripple
    // 1 (on), [3] angle 2 (off), [4] motion sync 2 (off), [6] scroll 1, [8] max speed 1
    model.borrow_mut().log.clear();
    kc::Keychron.change(&m, &mut p, Change::Extra(Extra::MotionSync, 0)).unwrap();
    assert_eq!(m6_writes(&model), vec![mrep(0xB5, &[0x42, 1, 1, 2, 2, 0, 1, 0, 1])]);
    // setAngleSnapping(true): [3] = 1
    model.borrow_mut().log.clear();
    kc::Keychron.change(&m, &mut p, Change::Extra(Extra::AngleSnap, 1)).unwrap();
    assert_eq!(m6_writes(&model), vec![mrep(0xB5, &[0x42, 1, 1, 1, 2, 0, 1, 0, 1])]);
    // setRippleControl(false): [2] = 2
    model.borrow_mut().log.clear();
    kc::Keychron.change(&m, &mut p, Change::Extra(Extra::RippleControl, 0)).unwrap();
    assert_eq!(m6_writes(&model), vec![mrep(0xB5, &[0x42, 1, 2, 1, 2, 0, 1, 0, 1])]);
    // setDebounceTime(20): keychronEncodeDebounce = [0x43, 20]
    model.borrow_mut().log.clear();
    kc::Keychron.change(&m, &mut p, Change::Extra(Extra::Debounce, 20)).unwrap();
    assert_eq!(m6_writes(&model), vec![mrep(0xB5, &[0x43, 20])]);
    let r = kc::Keychron.read(&m, &mut p, ReadOptions::default()).unwrap();
    assert_eq!(r.extras.iter().map(|e| e.value).collect::<Vec<_>>(), vec![Some(20), Some(0), Some(1), Some(0)]);
    assert_eq!(r.lift_off, Some(10), "the lift-off was re-sent as read");
    // a mouse that keeps its value
    model.borrow_mut().refuse_writes = true;
    assert!(kc::Keychron.change(&m, &mut p, Change::Extra(Extra::MotionSync, 1)).is_err());
    // refused before a byte leaves
    model.borrow_mut().log.clear();
    for c in [Change::Extra(Extra::Debounce, 21), Change::Extra(Extra::AngleSnap, 2), Change::Extra(Extra::Turbo, 1)] {
        assert!(kc::Keychron.change(&m, &mut p, c).is_err(), "{c:?}");
    }
    assert!(model.borrow().log.is_empty());
    // the Nape: OpenMouse's client throws for all four - nothing is sent
    let (nm, nmodel, mut np) = nape_setup();
    for c in [Change::Extra(Extra::Debounce, 4), Change::Extra(Extra::MotionSync, 1)] {
        assert!(kc::Keychron.change(&nm, &mut np, c).is_err());
    }
    assert!(nape_writes(&nmodel).is_empty() && np.sent.is_empty());
}

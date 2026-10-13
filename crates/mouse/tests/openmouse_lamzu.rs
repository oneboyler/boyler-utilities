//! Order 105 check, Lamzu (LamzuHidClient: Maya X, Inca 8K, CRDRAKO KO-ONE, LunaFury LUNA33, Attack Shark R5 Ultra): our
//! requests = the bytes OpenMouse sends for the same setting (encode), and answers shaped the way OpenMouse reads them decode
//! to the same values (decode). The expected bytes are worked out by hand from OpenMouse (@openmouse/protocol 0.40.1:
//! `dist/compx/codec.js`, `dist/lamzu/index.js`, `dist/drivers/lamzu/hid.js`), never from our own encoder.
//!
//! compaxEncodeRequest builds 64 bytes (no report id): `[0]` 0 · `[1]` 0 · `[2]` target · `[3]` length · `[4]` page · `[5]`
//! command · `[6..]` the arguments. Windows adds report id 0 in front: OpenMouse's byte i = our byte i + 1 (target 3, length 4,
//! page 5, command 6, arguments 7..). Nothing here needs a crc. The constants of hid.js: TARGET mouse 0x02 / dongle 0x00,
//! PAGE device 0x00 / profile 0x01; READ firmware 0x81 (len 0x10), battery 0x83 (2), activeProfile 0x85 (1); PROFILE_READ
//! dpiStages 0x81 (0x0a, [profile, 6]), activeStage 0x82 (2), pollingRate 0x80 (2), liftOffDistance 0x88 (2); WRITE
//! pollingRate 0x00, liftOffDistance 0x08, dpiStages 0x01 (all on the profile page, `write()` sets length = args.length).

use bu_mouse::error::Error;
use bu_mouse::family::{Driver, Family, Matched, ReadOptions, Setting};
use bu_mouse::fake::{hid_info, ScriptPort};
use bu_mouse::lamzu as lz;
use bu_mouse::os::{HidInfo, HidTransfer};
use std::cell::RefCell;
use std::rc::Rc;

/// A request as Windows sends it: report id 0, then OpenMouse's fields (65 bytes).
fn rq(target: u8, length: u8, page: u8, command: u8, args: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 65];
    v[3] = target;
    v[4] = length;
    v[5] = page;
    v[6] = command;
    v[7..7 + args.len()].copy_from_slice(args);
    v
}

/// An answer the way receiveFeatureReport returns it (64 bytes, no report id): status, data length, echoed page + command, data.
fn reply(status: u8, page: u8, command: u8, data: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 64];
    v[0] = status;
    v[3] = data.len() as u8;
    v[4] = page;
    v[5] = command;
    v[6..6 + data.len()].copy_from_slice(data);
    v
}

fn get(report: &[u8]) -> Vec<u8> {
    report.to_vec()
}

// ---------------------------------------------------------------- encode: reads

#[test]
fn reads_are_openmouses_compaxencoderequest_bytes() {
    // READ.firmware {target 2, page 0, command 0x81, length 0x10, no args}
    assert_eq!(get(&lz::report(&lz::read_firmware(), 2, 65)), rq(0x02, 0x10, 0x00, 0x81, &[]));
    // READ.battery {2, 0, 0x83, 2}
    assert_eq!(get(&lz::report(&lz::read_battery(), 2, 65)), rq(0x02, 0x02, 0x00, 0x83, &[]));
    // READ.activeProfile {2, 0, 0x85, 1}
    assert_eq!(get(&lz::report(&lz::read_active_profile(), 2, 65)), rq(0x02, 0x01, 0x00, 0x85, &[]));
    // PROFILE_READ.dpiStages(profile 1, maxStages 6) {2, page 1, 0x81, length 0x0a, args [1, 6]}
    assert_eq!(get(&lz::report(&lz::read_dpi_stages(1, 6), 2, 65)), rq(0x02, 0x0a, 0x01, 0x81, &[1, 6]));
    // activeStage(3) {2, 1, 0x82, 2, [3]}
    assert_eq!(get(&lz::report(&lz::read_active_stage(3), 2, 65)), rq(0x02, 0x02, 0x01, 0x82, &[3]));
    // pollingRate(1) {2, 1, 0x80, 2, [1]}
    assert_eq!(get(&lz::report(&lz::read_polling(1), 2, 65)), rq(0x02, 0x02, 0x01, 0x80, &[1]));
    // liftOffDistance(2) {2, 1, 0x88, 2, [2]}
    assert_eq!(get(&lz::report(&lz::read_lift_off(2), 2, 65)), rq(0x02, 0x02, 0x01, 0x88, &[2]));
    // a model with `mouseTarget: 0x00` (KO-ONE and LUNA33 on the cable): exchange() swaps the mouse target, byte 3 becomes 0
    assert_eq!(get(&lz::report(&lz::read_battery(), 0, 65)), rq(0x00, 0x02, 0x00, 0x83, &[]));
    // Windows wants the collection's FeatureReportByteLength: zero-padded behind the 65 bytes
    let long = lz::report(&lz::read_firmware(), 2, 70);
    assert_eq!((long.len(), &long[..65], &long[65..]), (70, &rq(0x02, 0x10, 0x00, 0x81, &[])[..], &[0u8; 5][..]));
}

// ---------------------------------------------------------------- encode: writes

#[test]
fn writes_are_openmouses_compaxencoderequest_bytes() {
    // write(PAGE.profile, WRITE.pollingRate = 0x00, profile 1, [code]): args = [1, code], length 2.
    // POLLING_RATES.find(hz): 125 -> 0x08, 250 -> 0x04, 500 -> 0x02, 1000 -> 0x01 (the FIRST 1000 entry), 2000 -> 0x20,
    // 4000 -> 0x40, 8000 -> 0x80
    for (hz, code) in [(125, 0x08), (250, 0x04), (500, 0x02), (1000, 0x01), (2000, 0x20), (4000, 0x40), (8000, 0x80)] {
        let c = lz::polling_code(hz).unwrap();
        assert_eq!(c, code, "{hz} Hz");
        assert_eq!(get(&lz::report(&lz::write_polling(1, c), 2, 65)), rq(0x02, 0x02, 0x01, 0x00, &[1, code]), "{hz} Hz");
    }
    assert_eq!(lz::polling_code(3000), None);
    // the profile in the arguments is the active one: profile 3
    assert_eq!(get(&lz::report(&lz::write_polling(3, 0x40), 2, 65)), rq(0x02, 0x02, 0x01, 0x00, &[3, 0x40]));
    // write(PAGE.profile, WRITE.liftOffDistance = 0x08, profile, [code]): Low 0x87, Medium 0x01, High 0x02. The codes mean
    // tenths of a mm through compaxDecodeLiftOff: 0x87 -> (0x87 & 0x7f) = 7 tenths, 0x01 -> 1 mm, 0x02 -> 2 mm
    for (tenths, code) in [(7, 0x87), (10, 0x01), (20, 0x02)] {
        let c = lz::lift_off_code(tenths).unwrap();
        assert_eq!(c, code, "{tenths}");
        assert_eq!(get(&lz::report(&lz::write_lift_off(1, c), 2, 65)), rq(0x02, 0x02, 0x01, 0x08, &[1, code]), "{tenths}");
    }
    assert_eq!(lz::lift_off_code(15), None);
    // setDpi: write(PAGE.profile, WRITE.dpiStages = 0x01, profile 1, [stages.length, per stage: x hi, x lo, y hi, y lo]).
    // two stages 800 (0x0320) and 3200 (0x0c80): args [1, 2, 03 20 03 20, 0c 80 0c 80] = 10 bytes -> length 0x0a
    assert_eq!(
        get(&lz::report(&lz::write_dpi_stages(1, &[(800, 800), (3200, 3200)]), 2, 65)),
        rq(0x02, 0x0a, 0x01, 0x01, &[1, 2, 0x03, 0x20, 0x03, 0x20, 0x0c, 0x80, 0x0c, 0x80])
    );
    // six stages on profile 2, x != y on the second: 400 (0x0190), (1000 = 0x03e8, 1200 = 0x04b0), 1600 (0x0640), 3200 (0x0c80),
    // 6400 (0x1900), 30000 (0x7530): args = 2 + 6 * 4 = 26 = 0x1a
    let six = [(400, 400), (1000, 1200), (1600, 1600), (3200, 3200), (6400, 6400), (30000, 30000)];
    let args = [2, 6, 0x01, 0x90, 0x01, 0x90, 0x03, 0xe8, 0x04, 0xb0, 0x06, 0x40, 0x06, 0x40, 0x0c, 0x80, 0x0c, 0x80, 0x19, 0x00, 0x19, 0x00, 0x75, 0x30, 0x75, 0x30];
    assert_eq!(get(&lz::report(&lz::write_dpi_stages(2, &six), 2, 65)), rq(0x02, 0x1a, 0x01, 0x01, &args));
}

// ---------------------------------------------------------------- decode

#[test]
fn answers_decode_like_openmouse() {
    // compaxDecodeDpiStages: count at [1], stage i at 2 + 4i: x = p[o] << 8 | p[o+1], y = p[o+2] << 8 | p[o+3]
    let p = [1, 2, 0x03, 0x20, 0x03, 0x20, 0x0c, 0x80, 0x0f, 0xa0];
    assert_eq!(lz::decode_dpi_stages(&p), vec![(800, 800), (3200, 4000)]);
    // the loop stops where the data ends: count 3 but only 2 stages' bytes (offset 10 + 3 >= 10)
    let mut short = p.to_vec();
    short[1] = 3;
    assert_eq!(lz::decode_dpi_stages(&short), vec![(800, 800), (3200, 4000)]);
    // a stage cut off after 3 bytes is not decoded: offset 2 + 3 >= 5
    assert_eq!(lz::decode_dpi_stages(&[1, 1, 0x03, 0x20, 0x03]), vec![]);
    assert_eq!(lz::decode_dpi_stages(&[1]), vec![]);
    // stageIndex: min(max(reported, 1), max(count, 1)) - 1
    assert_eq!((lz::stage_index(0, 4), lz::stage_index(1, 4), lz::stage_index(3, 4), lz::stage_index(9, 4), lz::stage_index(2, 0)), (0, 0, 2, 3, 0));
    // currentProfile: Math.max(1, reply[0])
    assert_eq!((lz::profile_of(&[0]), lz::profile_of(&[1]), lz::profile_of(&[3]), lz::profile_of(&[])), (1, 1, 3, 1));
    // compaxDecodePollingRate over LAMZU_POLLING_RATES (both 1000 Hz codes)
    for (code, hz) in [(0x08, 125), (0x04, 250), (0x02, 500), (0x01, 1000), (0x10, 1000), (0x20, 2000), (0x40, 4000), (0x80, 8000)] {
        assert_eq!(lz::polling_hz(code), Some(hz), "{code:#04x}");
    }
    assert_eq!((lz::polling_hz(0x03), lz::polling_hz(0)), (None, None), "OpenMouse throws on an unknown code");
    // compaxDecodeLiftOff: 0 -> none; bit 7 set -> (v & 0x7f) / 10 mm; else v mm. 0x87 -> 0.7, 0x85 -> 0.5, 0x01 -> 1, 0x03 -> 3
    assert_eq!((lz::lift_off_tenths(0x87), lz::lift_off_tenths(0x85), lz::lift_off_tenths(0x01), lz::lift_off_tenths(0x02), lz::lift_off_tenths(0x03)), (Some(7), Some(5), Some(10), Some(20), Some(30)));
    assert_eq!(lz::lift_off_tenths(0), None);
    // 0x80: bit 7 set and a value of 0 = "0 mm", which OpenMouse would call Low; it is shown as no distance, not as 0 mm
    assert_eq!(lz::lift_off_tenths(0x80), None);
    // battery: percent = battery[1] when <= 100 (else null), charging = battery[0] === 1
    assert_eq!(lz::battery(&[1, 87]), (Some(87), Some(true)));
    assert_eq!(lz::battery(&[0, 100]), (Some(100), Some(false)));
    assert_eq!(lz::battery(&[0, 101]), (None, Some(false)));
    assert_eq!(lz::battery(&[0xff]), (None, Some(false)));
    assert_eq!(lz::battery(&[]), (None, None));
}

#[test]
fn replies_are_judged_like_the_exchange_loop() {
    let polling = lz::read_polling(1);
    // status 0xa1 + echoed page 1 / command 0x80: the data is bytes 6 .. 6 + length
    assert_eq!(lz::decode_reply(&reply(0xa1, 0x01, 0x80, &[1, 0x40]), &polling), lz::Reply::Data(vec![1, 0x40]));
    // an ok answer to ANOTHER command (the stale answer to the lift-off read), pending 0xa0 and busy 0xa3: fetch again
    assert_eq!(lz::decode_reply(&reply(0xa1, 0x01, 0x88, &[1, 0x01]), &polling), lz::Reply::Again);
    assert_eq!(lz::decode_reply(&reply(0xa1, 0x00, 0x80, &[1, 0x40]), &polling), lz::Reply::Again, "the page counts too");
    assert_eq!(lz::decode_reply(&reply(0xa0, 0x01, 0x80, &[]), &polling), lz::Reply::Again);
    assert_eq!(lz::decode_reply(&reply(0xa3, 0x01, 0x80, &[]), &polling), lz::Reply::Again);
    // 0xa2: "is not supported by this mouse"; any other status: "unexpected status"
    assert_eq!(lz::decode_reply(&reply(0xa2, 0x01, 0x80, &[]), &polling), lz::Reply::Unsupported);
    assert_eq!(lz::decode_reply(&reply(0x00, 0x01, 0x80, &[]), &polling), lz::Reply::Status(0x00));
    assert_eq!(lz::decode_reply(&reply(0xa4, 0x01, 0x80, &[]), &polling), lz::Reply::Status(0xa4));
    // Math.min(reply[3], 64 - 6): a length byte of 200 reads 58 bytes
    let mut long = reply(0xa1, 0x01, 0x80, &[]);
    long[3] = 200;
    long[6] = 9;
    assert_eq!(lz::decode_reply(&long, &polling), lz::Reply::Data({
        let mut d = vec![0u8; 58];
        d[0] = 9;
        d
    }));
}

// ---------------------------------------------------------------- the read-only allow-list

#[test]
fn only_openmouses_read_requests_pass_the_allow_list() {
    for read in [
        rq(0x02, 0x10, 0x00, 0x81, &[]),
        rq(0x02, 0x02, 0x00, 0x83, &[]),
        rq(0x02, 0x01, 0x00, 0x85, &[]),
        rq(0x02, 0x0a, 0x01, 0x81, &[1, 6]),
        rq(0x02, 0x0a, 0x01, 0x81, &[255, 6]),
        rq(0x02, 0x02, 0x01, 0x82, &[2]),
        rq(0x02, 0x02, 0x01, 0x80, &[1]),
        rq(0x02, 0x02, 0x01, 0x88, &[3]),
        // the models that answer on target 0
        rq(0x00, 0x10, 0x00, 0x81, &[]),
        rq(0x00, 0x02, 0x00, 0x83, &[]),
        rq(0x00, 0x0a, 0x01, 0x81, &[1, 6]),
        rq(0x00, 0x02, 0x01, 0x88, &[1]),
    ] {
        assert!(lz::read_request_allowed(&read), "{read:02x?}");
    }
    // every request this driver builds as a read passes, on both targets and with a padded collection
    for t in [0, 2] {
        for r in [lz::read_firmware(), lz::read_battery(), lz::read_active_profile(), lz::read_dpi_stages(4, 6), lz::read_active_stage(4), lz::read_polling(4), lz::read_lift_off(4)] {
            assert!(lz::read_request_allowed(&lz::report(&r, t, 65)) && lz::read_request_allowed(&lz::report(&r, t, 72)), "{r:?}");
        }
    }
    // every write is refused
    for write in [
        rq(0x02, 0x02, 0x01, 0x00, &[1, 0x40]),
        rq(0x02, 0x02, 0x01, 0x08, &[1, 0x87]),
        rq(0x02, 0x0a, 0x01, 0x01, &[1, 2, 0x03, 0x20, 0x03, 0x20, 0x0c, 0x80, 0x0c, 0x80]),
        // writes (sleep timeout 0x07, debounce 0x08 on the device page, angle snapping 0x04, motion
        // sync 0x09, competitive 0x13, hyper 0x0b, ripple 0x0a, the dongle LED 0x04 on page 2) - never allowed
        rq(0x02, 0x03, 0x00, 0x07, &[1, 0x01, 0x2c]),
        rq(0x02, 0x02, 0x00, 0x08, &[1, 4]),
        rq(0x02, 0x02, 0x01, 0x04, &[1, 1]),
        rq(0x02, 0x02, 0x01, 0x09, &[1, 1]),
        rq(0x02, 0x02, 0x01, 0x13, &[1, 1]),
        rq(0x02, 0x02, 0x01, 0x0b, &[1, 1]),
        rq(0x02, 0x02, 0x01, 0x0a, &[1, 1]),
        rq(0x00, 0x02, 0x02, 0x04, &[1, 1]),
    ] {
        assert!(!lz::read_request_allowed(&write), "{write:02x?}");
    }
    // reads this driver never sends: sleep 0x87, competitive 0x93 and other flags, the LunaFury / magnetic reads (Order 110: the
    // debounce 0x88 and Motion Sync / angle snapping / ripple reads are allowed now - see order_110_reads_and_writes)
    for other in [
        rq(0x02, 0x03, 0x00, 0x87, &[1]),
        rq(0x02, 0x02, 0x01, 0x93, &[1]),
        rq(0x02, 0x02, 0x01, 0x94, &[1]),
        rq(0x02, 0x04, 0x00, 0x98, &[1]),
        rq(0x02, 0x06, 0x03, 0x86, &[]),
        rq(0x00, 0x02, 0x02, 0x84, &[1]),
    ] {
        assert!(!lz::read_request_allowed(&other), "{other:02x?}");
    }
    // malformed: a changed argument, profile 0, another stage count, a status byte, an unknown target, report id, short, trailing bytes
    assert!(!lz::read_request_allowed(&rq(0x02, 0x10, 0x00, 0x81, &[1])));
    assert!(!lz::read_request_allowed(&rq(0x02, 0x0a, 0x01, 0x81, &[0, 6])));
    assert!(!lz::read_request_allowed(&rq(0x02, 0x0a, 0x01, 0x81, &[1, 5])));
    assert!(!lz::read_request_allowed(&rq(0x02, 0x0a, 0x01, 0x81, &[1, 6, 1])));
    assert!(!lz::read_request_allowed(&rq(0x01, 0x10, 0x00, 0x81, &[])));
    let mut st = rq(0x02, 0x10, 0x00, 0x81, &[]);
    st[1] = 1;
    assert!(!lz::read_request_allowed(&st));
    let mut id = rq(0x02, 0x10, 0x00, 0x81, &[]);
    id[0] = 8;
    assert!(!lz::read_request_allowed(&id));
    assert!(!lz::read_request_allowed(&rq(0x02, 0x10, 0x00, 0x81, &[])[..64]));
    let mut padded = rq(0x02, 0x10, 0x00, 0x81, &[]);
    padded.extend_from_slice(&[0, 0, 0]);
    assert!(lz::read_request_allowed(&padded));
    padded[67] = 1;
    assert!(!lz::read_request_allowed(&padded));
}

// ---------------------------------------------------------------- the device table and the matcher

fn ctl(vid: u16, pid: u16) -> HidInfo {
    // the control collection: vendor page 0xffff, feature report id 0 of 64 bytes = 65 with the id
    hid_info("mi_02", vid, pid, 0xFFFF, 0x0000, (0, 0, 65))
}

#[test]
fn the_table_is_openmouses_supported_lamzu_client_list() {
    // (vid, pid, model, wireless) for every "supported" entry of supported-devices-data.ts that LamzuHidClient drives:
    // Maya X [0x1c, 0x1d, 0x1e], Inca [0x9, 0xf, 0x10] (under Lamzu's 0x37b0), CRDRAKO KO-ONE [0x6a, 0x6b], LunaFury LUNA33
    // [0x32, 0x33], Attack Shark R5 Ultra [0x46, 0x47] (its catalog row is in lamzu/index.js, first in the registry)
    let table = [
        (0x373e, 0x001c, "Lamzu Maya X", false),
        (0x373e, 0x001d, "Lamzu Maya X", true),
        (0x373e, 0x001e, "Lamzu Maya X", true),
        (0x37b0, 0x0009, "Lamzu Inca 8K", false),
        (0x37b0, 0x000f, "Lamzu Inca 8K", true),
        (0x37b0, 0x0010, "Lamzu Inca 8K", true),
        (0x373e, 0x006a, "CRDRAKO KO-ONE", false),
        (0x373e, 0x006b, "CRDRAKO KO-ONE", true),
        (0x373e, 0x0032, "LunaFury LUNA33", false),
        (0x373e, 0x0033, "LunaFury LUNA33", true),
        (0x373e, 0x0046, "Attack Shark R5 Ultra", false),
        (0x373e, 0x0047, "Attack Shark R5 Ultra", true),
    ];
    assert_eq!(lz::PRODUCTS.len(), table.len());
    for (vid, pid, model, wireless) in table {
        let m = lz::matches(&[&ctl(vid, pid)], vid, pid).unwrap_or_else(|| panic!("{vid:#06x}:{pid:#06x}"));
        assert_eq!((m.family, m.model, m.wireless), (Family::Lamzu, Some(model), Some(wireless)), "{vid:#06x}:{pid:#06x}");
        assert_eq!(m.config.path, "mi_02");
        let p = lz::product_of_variant(m.variant).unwrap();
        assert_eq!((p.vid, p.pid, p.model), (vid, pid, model));
    }
    // per-model overrides of lamzu/index.js: mouseTarget 0x00 on the KO-ONE cable and the LUNA33 cable, else 0x02;
    // maxDpi 42000 on the R5 Ultra, else the default 30000; 6 stages everywhere
    let t = |pid: u16| lz::product(0x373e, pid).unwrap().mouse_target;
    assert_eq!((t(0x6a), t(0x6b), t(0x32), t(0x33), t(0x1c), t(0x46)), (0, 2, 0, 2, 2, 2));
    assert_eq!((lz::product(0x373e, 0x46).unwrap().max_dpi, lz::product(0x373e, 0x1c).unwrap().max_dpi, lz::product(0x37b0, 0x10).unwrap().max_dpi), (42000, 30000, 30000));
    assert!(lz::PRODUCTS.iter().all(|p| p.max_stages == 6));
    // "likely" / not listed but in the driver's catalog (driver knows, list doesn't): LunaFury TYPE33 0x54 / 0x84, Attack Shark
    // R6 0x21 / 0x22, R8 0x3a / 0x3b, RAWM Leviathan V4 GT 0x98 / 0x99, Vancer Akitsu 0x66 / 0x67, Paro Aurora 0x37b0:0x7 / 0xd / 0xe
    for pid in [0x0054, 0x0084, 0x0021, 0x0022, 0x003a, 0x003b, 0x0098, 0x0099, 0x0066, 0x0067] {
        assert!(lz::matches(&[&ctl(0x373e, pid)], 0x373e, pid).is_none(), "{pid:#06x}");
    }
    for pid in [0x0007, 0x000d, 0x000e] {
        assert!(lz::matches(&[&ctl(0x37b0, pid)], 0x37b0, pid).is_none(), "{pid:#06x}");
    }
    // the same product id under the other vendor id is another device (lamzuProduct): Maya X's id under Lamzu's vendor id,
    // the Inca's under the CompX one; and a foreign vendor id finds nothing
    assert!(lz::matches(&[&ctl(0x37b0, 0x001c)], 0x37b0, 0x001c).is_none());
    assert!(lz::matches(&[&ctl(0x373e, 0x0009)], 0x373e, 0x0009).is_none());
    assert!(lz::matches(&[&ctl(0x373f, 0x001c)], 0x373f, 0x001c).is_none());
}

#[test]
fn the_interface_is_the_one_with_the_report_id_0_feature_report() {
    // OpenMouse: any collection that declares feature report id 0. On Windows: a collection whose feature report is 65 bytes
    let mouse = hid_info("mi_00", 0x373e, 0x001d, 0x01, 0x02, (8, 0, 0));
    let consumer = hid_info("mi_00&col02", 0x373e, 0x001d, 0x0C, 0x01, (3, 0, 0));
    let vendor_no_feature = hid_info("mi_01", 0x373e, 0x001d, 0xFFFF, 0x01, (64, 64, 0));
    let control = ctl(0x373e, 0x001d);
    assert_eq!(lz::matches(&[&mouse, &consumer, &vendor_no_feature, &control], 0x373e, 0x001d).map(|m| m.config.path), Some("mi_02".into()));
    // a non-vendor interface of the device only: nothing to talk to
    assert!(lz::matches(&[&mouse, &consumer, &vendor_no_feature], 0x373e, 0x001d).is_none());
    assert!(lz::matches(&[], 0x373e, 0x001d).is_none());
    // a feature report shorter than 65 bytes is not the 64-byte control channel
    let small = hid_info("mi_02", 0x373e, 0x001d, 0xFFFF, 0x00, (0, 0, 33));
    assert!(lz::matches(&[&small], 0x373e, 0x001d).is_none());
    // if several carry one, the vendor-defined collection wins over a generic one
    let generic = hid_info("mi_03", 0x373e, 0x001d, 0x01, 0x80, (0, 0, 65));
    assert_eq!(lz::matches(&[&generic, &control], 0x373e, 0x001d).map(|m| m.config.path), Some("mi_02".into()));
    // the whole-device lookup lands on the Lamzu family, with the model's row as the variant
    let all = [mouse.clone(), control.clone()];
    let found = bu_mouse::family::find(&all, 0x373e, 0x001d).unwrap();
    assert_eq!((found.family, found.config.path.as_str()), (Family::Lamzu, "mi_02"));
    assert_eq!(lz::product_of_variant(found.variant).map(|p| p.pid), Some(0x001d));
}

#[test]
fn no_other_family_claims_a_lamzu_id() {
    use bu_mouse::{asus, atk, attackshark, corsair, dareu, delux, endgame, finalmouse, gearhub, glorious, incott, keychron, ksnake, logitech, mchose, microsoft, moddo, ninjutso, orbital, pulsar, rapoo, rawm, razer, redragon, ryunix, steelseries, teevolution, vaxee, vgn, wallhack, wlmouse, zaunkoenig};
    type M = fn(&[&HidInfo], u16, u16) -> Option<Matched>;
    let others: [(&str, M); 33] = [
        ("pulsar cmouse", pulsar::matches_cmouse),
        ("pulsar xs1", pulsar::matches_xs1),
        ("logitech", logitech::matches),
        ("razer", razer::matches),
        ("steelseries", steelseries::matches),
        ("atk", atk::matches),
        ("endgame", endgame::matches),
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
    for p in lz::PRODUCTS {
        let ifs = [
            ctl(p.vid, p.pid),
            hid_info("mi_00", p.vid, p.pid, 0x01, 0x02, (8, 0, 0)),
            hid_info("mi_01", p.vid, p.pid, 0xFF00, 0x01, (64, 64, 65)),
            hid_info("mi_03", p.vid, p.pid, 0xFFFF, 0x01, (64, 64, 65)),
        ];
        let refs: Vec<&HidInfo> = ifs.iter().collect();
        for (name, f) in others {
            assert!(f(&refs, p.vid, p.pid).is_none(), "{name} also claims {:#06x}:{:#06x} ({})", p.vid, p.pid, p.model);
        }
    }
}

// ---------------------------------------------------------------- a scripted Lamzu mouse

/// A fake Lamzu mouse answering the 64-byte page-command protocol from a little state. It answers status 0xa1 with the
/// request's page + command echoed and the data length set; `busy` answers 0xa3 that many fetches first; `stale` answers
/// the NEXT request first with the previous request's answer; `asleep` never leaves status 0xa0; `unsupported` lists
/// (page, command) it answers 0xa2; `keep_polling` / `keep_lift` / `keep_dpi` make a write have no effect.
struct Mouse {
    target: u8,
    profile: u8,
    charging: u8,
    percent: u8,
    stages: Vec<(u16, u16)>,
    active: u8,
    poll: u8,
    lift: u8,
    busy: u32,
    stale: u32,
    asleep: bool,
    unsupported: Vec<(u8, u8)>,
    keep_polling: bool,
    keep_lift: bool,
    keep_dpi: bool,
    claim_stages: Option<u8>,
    pending: Vec<u8>,
    /// Order 110: debounce ms, the flags (Motion Sync, angle snapping, ripple), a write of them has no effect
    debounce: u8,
    flags: [u8; 3],
    keep_extra: bool,
    last: Vec<u8>,
}

impl Mouse {
    fn new(target: u8) -> Self {
        Mouse {
            target,
            profile: 1,
            charging: 1,
            percent: 87,
            stages: vec![(800, 800), (1600, 1600)],
            active: 2,
            poll: 0x01,
            lift: 0x01,
            busy: 0,
            stale: 0,
            asleep: false,
            unsupported: Vec::new(),
            keep_polling: false,
            keep_lift: false,
            keep_dpi: false,
            claim_stages: None,
            pending: Vec::new(),
            debounce: 4,
            flags: [1, 0, 1],
            keep_extra: false,
            last: Vec::new(),
        }
    }

    fn answer(&mut self, out: &[u8], how: &HidTransfer) -> Vec<u8> {
        let mut early = None;
        if let HidTransfer::Feature { .. } = how {
            assert_eq!(out.len(), 65);
            assert_eq!(out[0], 0, "report id 0");
            let p = &out[1..65];
            assert_eq!(p[2], self.target, "a wrong target: a real mouse would never answer");
            let (len, page, cmd) = (p[3] as usize, p[4], p[5]);
            let args = &p[6..];
            let mut status = 0xa1;
            let data: Vec<u8> = match (page, cmd) {
                _ if self.unsupported.contains(&(page, cmd)) => {
                    status = 0xa2;
                    vec![]
                }
                (0, 0x81) => vec![0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14],
                (0, 0x83) => vec![self.charging, self.percent],
                (0, 0x85) => vec![self.profile],
                (1, 0x81) => {
                    assert_eq!(args[..2], [self.profile, 6], "the stage table read names the active profile and 6 stages");
                    let mut d = vec![self.profile, self.claim_stages.unwrap_or(self.stages.len() as u8)];
                    for (x, y) in &self.stages {
                        d.extend_from_slice(&x.to_be_bytes());
                        d.extend_from_slice(&y.to_be_bytes());
                    }
                    d
                }
                (1, 0x82) => vec![args[0], self.active],
                (1, 0x80) => vec![args[0], self.poll],
                (1, 0x88) => vec![args[0], self.lift],
                (0, 0x88) => vec![args[0], self.debounce],
                (1, 0x89) => vec![args[0], self.flags[0]],
                (1, 0x84) => vec![args[0], self.flags[1]],
                (1, 0x8a) => vec![args[0], self.flags[2]],
                (0, 0x08) | (1, 0x09) | (1, 0x04) | (1, 0x0a) => {
                    assert_eq!((len, args[0]), (2, self.profile), "a write names the ACTIVE profile");
                    if !self.keep_extra {
                        match (page, cmd) {
                            (0, _) => self.debounce = args[1],
                            (_, 0x09) => self.flags[0] = args[1],
                            (_, 0x04) => self.flags[1] = args[1],
                            _ => self.flags[2] = args[1],
                        }
                    }
                    args[..2].to_vec()
                }
                (1, 0x00) => {
                    assert_eq!((len, args[0]), (2, self.profile), "a write names the ACTIVE profile");
                    if !self.keep_polling {
                        self.poll = args[1];
                    }
                    args[..2].to_vec()
                }
                (1, 0x08) => {
                    assert_eq!((len, args[0]), (2, self.profile), "a write names the ACTIVE profile");
                    if !self.keep_lift {
                        self.lift = args[1];
                    }
                    args[..2].to_vec()
                }
                (1, 0x01) => {
                    assert_eq!((len, args[0]), (2 + 4 * args[1] as usize, self.profile));
                    if !self.keep_dpi {
                        self.stages = (0..args[1] as usize).map(|i| (u16::from_be_bytes([args[2 + i * 4], args[3 + i * 4]]), u16::from_be_bytes([args[4 + i * 4], args[5 + i * 4]]))).collect();
                    }
                    args[..len].to_vec()
                }
                _ => panic!("the fake does not know page {page:#04x} command {cmd:#04x}"),
            };
            let mut q = reply(status, page, cmd, &data);
            if self.asleep {
                q = reply(0xa0, 0, 0, &[]);
            }
            let previous = std::mem::replace(&mut self.last, q.clone());
            if self.stale > 0 && !previous.is_empty() {
                self.stale -= 1;
                early = Some(previous);
            }
            self.pending = q;
        }
        let mut q = early.unwrap_or_else(|| self.pending.clone());
        if self.busy > 0 {
            self.busy -= 1;
            q[0] = 0xa3;
        }
        let mut v = vec![0u8];
        v.extend_from_slice(&q);
        v
    }
}

fn port_for(info: &HidInfo, mouse: Rc<RefCell<Mouse>>) -> ScriptPort {
    ScriptPort::new(info.clone(), move |out, how| Ok(mouse.borrow_mut().answer(out, how)))
}

fn sent(port: &ScriptPort) -> Vec<Vec<u8>> {
    port.sent.iter().map(|s| s.0.clone()).collect()
}

fn setup(vid: u16, pid: u16) -> (Matched, Rc<RefCell<Mouse>>, ScriptPort) {
    let info = ctl(vid, pid);
    let m = lz::matches(&[&info], vid, pid).unwrap();
    let target = lz::product(vid, pid).unwrap().mouse_target;
    let mouse = Rc::new(RefCell::new(Mouse::new(target)));
    let port = port_for(&info, mouse.clone());
    (m, mouse, port)
}

fn feature0() -> HidTransfer {
    HidTransfer::Feature { reply_id: 0, reply_len: 65, wait_ms: 0 }
}

// ---------------------------------------------------------------- driver: reading

#[test]
fn reads_with_the_requests_openmouses_readstatus_sends() {
    let (m, _mouse, mut port) = setup(0x373e, 0x001d);
    let r = lz::Lamzu.read(&m, &mut port, ReadOptions::default()).unwrap();
    // firmware (is it answering), battery, active profile, stage table, active stage (2 = 1600), polling (0x01), lift-off (0x01)
    assert_eq!(
        sent(&port),
        vec![
            rq(0x02, 0x10, 0x00, 0x81, &[]),
            rq(0x02, 0x02, 0x00, 0x83, &[]),
            rq(0x02, 0x01, 0x00, 0x85, &[]),
            rq(0x02, 0x0a, 0x01, 0x81, &[1, 6]),
            rq(0x02, 0x02, 0x01, 0x82, &[1]),
            rq(0x02, 0x02, 0x01, 0x80, &[1]),
            rq(0x02, 0x02, 0x01, 0x88, &[1]),
            // Order 110: PROFILE_READ.debounce (device page 0x88), angleSnapping 0x84, motionSync 0x89, rippleControl 0x8a
            rq(0x02, 0x02, 0x00, 0x88, &[1]),
            rq(0x02, 0x02, 0x01, 0x84, &[1]),
            rq(0x02, 0x02, 0x01, 0x89, &[1]),
            rq(0x02, 0x02, 0x01, 0x8a, &[1]),
        ]
    );
    assert!(port.sent.iter().all(|s| s.1 == feature0()), "SetFeature, then GetFeature at once (OpenMouse fetches before it waits)");
    assert!(port.sent.iter().all(|s| lz::read_request_allowed(&s.0)), "every read passes the allow-list");
    assert_eq!((r.online, r.battery_percent, r.charging, r.dpi, r.polling_hz, r.lift_off, r.wireless), (true, Some(87), Some(true), Some(1600), Some(1000), Some(10), Some(true)));
    // Maya X 0x1d: DPI 50 - 30000 in 50s, polling 125 - 1000 (RATES_1K), lift-off 0.7 / 1 / 2 mm, battery
    assert_eq!(r.caps.dpi.as_ref().map(|d| (d.min(), d.max(), d.bands.clone())), Some((50, 30000, vec![(50, 30000, 50)])));
    assert_eq!((r.caps.polling.clone(), r.caps.lift_off.clone(), r.caps.battery), (vec![125, 250, 500, 1000], vec![7, 10, 20], true));
}

#[test]
fn reads_address_the_profile_the_mouse_is_running() {
    let (m, mouse, mut port) = setup(0x373e, 0x001e);
    mouse.borrow_mut().profile = 3;
    mouse.borrow_mut().poll = 0x40;
    mouse.borrow_mut().lift = 0x87;
    mouse.borrow_mut().active = 1;
    mouse.borrow_mut().charging = 0;
    mouse.borrow_mut().percent = 101;
    let r = lz::Lamzu.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!(sent(&port)[3..7], [rq(0x02, 0x0a, 0x01, 0x81, &[3, 6]), rq(0x02, 0x02, 0x01, 0x82, &[3]), rq(0x02, 0x02, 0x01, 0x80, &[3]), rq(0x02, 0x02, 0x01, 0x88, &[3])]);
    // 0x40 = 4000 Hz, lift-off 0x87 = 0.7 mm, stage 1 = 800, battery 101 = unknown (OpenMouse: null), not charging
    assert_eq!((r.dpi, r.polling_hz, r.lift_off, r.battery_percent, r.charging), (Some(800), Some(4000), Some(7), None, Some(false)));
    // the 8K receiver: polling 500 - 8000
    assert_eq!(r.caps.polling, vec![500, 1000, 2000, 4000, 8000]);
}

#[test]
fn a_mouse_that_does_not_answer_is_offline_not_an_error() {
    let (m, mouse, mut port) = setup(0x373e, 0x001d);
    mouse.borrow_mut().asleep = true;
    let r = lz::Lamzu.read(&m, &mut port, ReadOptions { online_tries: 1 }).unwrap();
    assert!(!r.online);
    assert_eq!((r.battery_percent, r.dpi, r.polling_hz, r.lift_off, r.wireless), (None, None, None, None, Some(true)));
    assert!(r.caps.dpi.is_some() && r.caps.battery, "the caps still say what the model can do");
    // one firmware request, fetched 12 times (RESPONSE_ATTEMPTS) without sending again: SetFeature + 11 GetFeature, the waits
    // 30 ms after each of the first three attempts, 300 ms after that (RESPONSE_DELAY_MS / WAKE_DELAY_MS / QUICK_ATTEMPTS)
    assert_eq!(port.sent.len(), 12);
    assert!(port.sent.iter().all(|s| s.0 == rq(0x02, 0x10, 0x00, 0x81, &[])));
    let waits: Vec<u32> = port
        .sent
        .iter()
        .map(|s| match s.1 {
            HidTransfer::Feature { wait_ms, .. } | HidTransfer::FeatureIn { wait_ms, .. } => wait_ms,
            _ => panic!("{:?}", s.1),
        })
        .collect();
    assert_eq!(waits, vec![0, 30, 30, 30, 300, 300, 300, 300, 300, 300, 300, 300]);
    assert!(matches!(port.sent[0].1, HidTransfer::Feature { .. }) && port.sent[1..].iter().all(|s| matches!(s.1, HidTransfer::FeatureIn { reply_id: 0, reply_len: 65, .. })));
}

#[test]
fn the_exchange_waits_out_busy_and_skips_a_stale_answer() {
    let (m, mouse, mut port) = setup(0x373e, 0x001d);
    // two busy fetches, then the answer: fetched again without sending again
    mouse.borrow_mut().busy = 2;
    lz::Lamzu.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!(port.sent.len(), 11 + 2, "two extra fetches, no re-send (7 Order 104 reads + 4 Order 110 reads)");
    assert_eq!((&port.sent[0].1, &port.sent[1].1, &port.sent[2].1), (&feature0(), &HidTransfer::FeatureIn { reply_id: 0, reply_len: 65, wait_ms: 30 }, &HidTransfer::FeatureIn { reply_id: 0, reply_len: 65, wait_ms: 30 }));
    assert_eq!(port.sent[1].0, port.sent[0].0);
    // a stale answer (ok, but to the previous command) is skipped, not taken for this one's
    port.sent.clear();
    mouse.borrow_mut().stale = 1;
    let r = lz::Lamzu.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((r.dpi, r.polling_hz, r.lift_off, r.battery_percent), (Some(1600), Some(1000), Some(10), Some(87)));
    assert_eq!(port.sent.len(), 11 + 1);
}

#[test]
fn a_request_the_mouse_does_not_support_is_dropped_not_fatal_for_a_read() {
    let (m, mouse, mut port) = setup(0x373e, 0x001d);
    mouse.borrow_mut().unsupported = vec![(0, 0x83), (1, 0x88)];
    let r = lz::Lamzu.read(&m, &mut port, ReadOptions::default()).unwrap();
    // 0xa2 ends that request at once (no further fetches); the rest still reads
    assert_eq!(port.sent.len(), 11);
    assert_eq!((r.online, r.battery_percent, r.charging, r.dpi, r.polling_hz, r.lift_off), (true, None, None, Some(1600), Some(1000), None));
}

// ---------------------------------------------------------------- driver: writing

#[test]
fn writes_dpi_on_the_active_stage_and_copies_the_other_stages_back() {
    let (m, mouse, mut port) = setup(0x373e, 0x001d);
    {
        let mut ms = mouse.borrow_mut();
        ms.stages = vec![(400, 400), (1000, 1200), (1600, 1600), (3200, 3200)];
        ms.active = 3; // 1600
        ms.profile = 2;
    }
    let c = lz::Lamzu.write(&m, &mut port, Setting::Dpi(2400)).unwrap();
    assert_eq!((c.now, c.before), (2400, Some(1600)));
    // active profile, stage table, active stage, the write (profile 2; stage 3 replaced by 2400 = 0x0960 on both axes, the others
    // as read - including the 1000 x 1200 stage), the table again
    let args = [2, 4, 0x01, 0x90, 0x01, 0x90, 0x03, 0xe8, 0x04, 0xb0, 0x09, 0x60, 0x09, 0x60, 0x0c, 0x80, 0x0c, 0x80];
    assert_eq!(
        sent(&port),
        vec![
            rq(0x02, 0x01, 0x00, 0x85, &[]),
            rq(0x02, 0x0a, 0x01, 0x81, &[2, 6]),
            rq(0x02, 0x02, 0x01, 0x82, &[2]),
            rq(0x02, 0x12, 0x01, 0x01, &args),
            rq(0x02, 0x0a, 0x01, 0x81, &[2, 6]),
        ]
    );
    assert_eq!(mouse.borrow().stages, vec![(400, 400), (1000, 1200), (2400, 2400), (3200, 3200)]);
    // the four reads pass the allow-list, the write does not
    let allowed: Vec<bool> = port.sent.iter().map(|s| lz::read_request_allowed(&s.0)).collect();
    assert_eq!(allowed, vec![true, true, true, false, true]);
}

#[test]
fn writes_dpi_limits_and_refusals() {
    let (m, mouse, mut port) = setup(0x373e, 0x001d);
    // OpenMouse's setDpi: whole multiples of 50, 50 .. maxDpi (30000)
    for bad in [0, 49, 75, 30050, 100000] {
        assert!(matches!(lz::Lamzu.write(&m, &mut port, Setting::Dpi(bad)), Err(Error::OutOfRange { .. })), "{bad}");
    }
    assert!(port.sent.is_empty(), "nothing is sent for a value the mouse cannot store");
    // the mouse keeps its old DPI: OpenMouse throws "kept ... instead of ..."
    mouse.borrow_mut().keep_dpi = true;
    let e = lz::Lamzu.write(&m, &mut port, Setting::Dpi(3200)).unwrap_err();
    assert!(matches!(&e, Error::BadAnswer(t) if t.contains("1600") && t.contains("3200")), "{e:?}");
    // R5 Ultra: ceiling 42000 (and 41950 / 42000 are stored)
    let (r5, _mouse5, mut port5) = setup(0x373e, 0x0047);
    assert!(lz::Lamzu.write(&r5, &mut port5, Setting::Dpi(42050)).is_err());
    assert_eq!(lz::Lamzu.write(&r5, &mut port5, Setting::Dpi(42000)).map(|c| c.now), Ok(42000));
}

#[test]
fn a_stage_table_that_did_not_read_completely_is_not_written_back() {
    let (m, mouse, mut port) = setup(0x373e, 0x001d);
    // the answer says 6 stages but carries the bytes of 2: OpenMouse's decoder stops there, and a write would send count 2
    // and lose stages 3 - 6. Refused after the reads, nothing written.
    {
        let mut ms = mouse.borrow_mut();
        ms.stages = vec![(800, 800), (1600, 1600)];
        ms.claim_stages = Some(6);
    }
    let e = lz::Lamzu.write(&m, &mut port, Setting::Dpi(2400)).unwrap_err();
    assert!(matches!(&e, Error::BadAnswer(t) if t.contains("incomplete")), "{e:?}");
    assert!(port.sent.iter().all(|s| lz::read_request_allowed(&s.0)), "nothing but reads went out");
    assert_eq!(mouse.borrow().stages, vec![(800, 800), (1600, 1600)]);
    // a count above the model's 6 stages, and a count of 0, are refused too
    for claim in [7, 0] {
        port.sent.clear();
        mouse.borrow_mut().claim_stages = Some(claim);
        assert!(lz::Lamzu.write(&m, &mut port, Setting::Dpi(2400)).is_err(), "{claim}");
        assert!(port.sent.iter().all(|s| lz::read_request_allowed(&s.0)), "{claim}");
    }
}

#[test]
fn an_active_stage_with_a_separate_y_is_not_flattened() {
    let (m, mouse, mut port) = setup(0x373e, 0x001d);
    {
        let mut ms = mouse.borrow_mut();
        ms.stages = vec![(800, 800), (1600, 900)];
        ms.active = 2;
    }
    let e = lz::Lamzu.write(&m, &mut port, Setting::Dpi(2400)).unwrap_err();
    assert!(matches!(e, Error::OutOfRange { .. }), "{e:?}");
    // profile, stage table, active stage were read; no write went out; the mouse is unchanged
    assert_eq!(port.sent.len(), 3);
    assert!(port.sent.iter().all(|s| lz::read_request_allowed(&s.0)));
    assert_eq!(mouse.borrow().stages, vec![(800, 800), (1600, 900)]);
}
#[test]
fn writes_polling_the_openmouse_way() {
    let (m, mouse, mut port) = setup(0x373e, 0x001e);
    let c = lz::Lamzu.write(&m, &mut port, Setting::Polling(4000)).unwrap();
    assert_eq!((c.now, c.before), (4000, Some(1000)));
    // active profile, polling read (for undo), the write [profile 1, 0x40], polling read-back
    assert_eq!(
        sent(&port),
        vec![rq(0x02, 0x01, 0x00, 0x85, &[]), rq(0x02, 0x02, 0x01, 0x80, &[1]), rq(0x02, 0x02, 0x01, 0x00, &[1, 0x40]), rq(0x02, 0x02, 0x01, 0x80, &[1])]
    );
    assert_eq!(mouse.borrow().poll, 0x40);
    // 1000 Hz is written as 0x01 (the first LAMZU_POLLING_RATES entry), and 0x10 is still read as 1000
    port.sent.clear();
    lz::Lamzu.write(&m, &mut port, Setting::Polling(1000)).unwrap();
    assert_eq!(port.sent[2].0, rq(0x02, 0x02, 0x01, 0x00, &[1, 0x01]));
    mouse.borrow_mut().poll = 0x10;
    let r = lz::Lamzu.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!(r.polling_hz, Some(1000));
    // a rate this model cannot set (the 8K receiver's list starts at 500; 3000 has no code): refused before any request
    port.sent.clear();
    for hz in [125, 250, 3000] {
        assert!(matches!(lz::Lamzu.write(&m, &mut port, Setting::Polling(hz)), Err(Error::OutOfRange { .. })), "{hz}");
    }
    assert!(port.sent.is_empty());
    // the mouse keeps its rate: "The mouse kept ... Hz instead of ..."
    mouse.borrow_mut().keep_polling = true;
    let e = lz::Lamzu.write(&m, &mut port, Setting::Polling(8000)).unwrap_err();
    assert!(matches!(&e, Error::BadAnswer(t) if t.contains("8000")), "{e:?}");
}

#[test]
fn writes_lift_off_in_the_three_openmouse_steps() {
    let (m, mouse, mut port) = setup(0x37b0, 0x0010);
    let c = lz::Lamzu.write(&m, &mut port, Setting::LiftOff(7)).unwrap();
    assert_eq!((c.now, c.before), (7, Some(10)));
    assert_eq!(
        sent(&port),
        vec![rq(0x02, 0x01, 0x00, 0x85, &[]), rq(0x02, 0x02, 0x01, 0x88, &[1]), rq(0x02, 0x02, 0x01, 0x08, &[1, 0x87]), rq(0x02, 0x02, 0x01, 0x88, &[1])]
    );
    assert_eq!(mouse.borrow().lift, 0x87);
    let c = lz::Lamzu.write(&m, &mut port, Setting::LiftOff(20)).unwrap();
    assert_eq!((c.now, c.before), (20, Some(7)));
    assert_eq!(mouse.borrow().lift, 0x02);
    // a distance OpenMouse cannot write
    assert!(matches!(lz::Lamzu.write(&m, &mut port, Setting::LiftOff(15)), Err(Error::OutOfRange { .. })));
    // the mouse keeps its distance
    mouse.borrow_mut().keep_lift = true;
    let e = lz::Lamzu.write(&m, &mut port, Setting::LiftOff(10)).unwrap_err();
    assert!(matches!(&e, Error::BadAnswer(t) if t.contains("20") && t.contains("10")), "{e:?}");
}

#[test]
fn a_refused_or_unanswered_write_is_an_error() {
    let (m, mouse, mut port) = setup(0x373e, 0x001d);
    // the mouse answers "not supported" (0xa2) to the polling write: OpenMouse throws, so do we
    mouse.borrow_mut().unsupported = vec![(1, 0x00)];
    let e = lz::Lamzu.write(&m, &mut port, Setting::Polling(500)).unwrap_err();
    assert!(matches!(&e, Error::BadAnswer(t) if t.contains("not supported")), "{e:?}");
    // asleep: no profile answer, the change stops before any write goes out
    mouse.borrow_mut().asleep = true;
    port.sent.clear();
    for s in [Setting::Dpi(1600), Setting::Polling(500), Setting::LiftOff(10)] {
        assert!(lz::Lamzu.write(&m, &mut port, s).is_err(), "{s:?}");
    }
    assert!(port.sent.iter().all(|s| lz::read_request_allowed(&s.0)), "only the profile read went out");
    assert_eq!(port.sent.len(), 3 * 12);
}

#[test]
fn the_kone_and_luna33_cables_answer_on_target_zero() {
    // CRDRAKO KO-ONE 0x6a and LunaFury LUNA33 0x32 (mouseTarget 0x00): the mouse requests go out with target 0
    for (pid, rates) in [(0x006a, vec![125, 250, 500, 1000, 2000, 4000, 8000]), (0x0032, vec![125, 250, 500, 1000])] {
        let (m, _mouse, mut port) = setup(0x373e, pid);
        let r = lz::Lamzu.read(&m, &mut port, ReadOptions::default()).unwrap();
        assert!(r.online, "{pid:#06x}");
        assert!(port.sent.iter().all(|s| s.0[3] == 0x00), "{pid:#06x}");
        assert_eq!((sent(&port)[0].clone(), sent(&port)[3].clone()), (rq(0x00, 0x10, 0x00, 0x81, &[]), rq(0x00, 0x0a, 0x01, 0x81, &[1, 6])));
        assert_eq!((r.caps.polling, r.wireless), (rates, Some(false)));
        assert!(port.sent.iter().all(|s| lz::read_request_allowed(&s.0)));
        let c = lz::Lamzu.write(&m, &mut port, Setting::Polling(500)).unwrap();
        assert_eq!((c.now, c.before), (500, Some(1000)));
    }
    // and their receivers: KO-ONE 0x6b target 2 with all seven rates, LUNA33 0x33 target 2
    for pid in [0x006b, 0x0033] {
        let (m, _mouse, mut port) = setup(0x373e, pid);
        let r = lz::Lamzu.read(&m, &mut port, ReadOptions::default()).unwrap();
        assert!(r.online && port.sent.iter().all(|s| s.0[3] == 0x02), "{pid:#06x}");
        assert_eq!((r.caps.polling, r.wireless), (vec![125, 250, 500, 1000, 2000, 4000, 8000], Some(true)));
    }
}

#[test]
fn every_model_reads_its_own_polling_list() {
    // (vid, pid) -> polling rates OpenMouse can set (lamzu/index.js RATES_1K / RATES_8K / RATES_8K_FULL)
    let rates = |vid, pid| {
        let (m, _mouse, mut port) = setup(vid, pid);
        lz::Lamzu.read(&m, &mut port, ReadOptions::default()).unwrap().caps.polling
    };
    let k1 = vec![125, 250, 500, 1000];
    let k8 = vec![500, 1000, 2000, 4000, 8000];
    let full = vec![125, 250, 500, 1000, 2000, 4000, 8000];
    assert_eq!((rates(0x373e, 0x1c), rates(0x373e, 0x1d), rates(0x373e, 0x1e)), (k1.clone(), k1.clone(), k8.clone()));
    assert_eq!((rates(0x37b0, 0x9), rates(0x37b0, 0xf), rates(0x37b0, 0x10)), (k1.clone(), k1.clone(), k8));
    assert_eq!((rates(0x373e, 0x6a), rates(0x373e, 0x6b), rates(0x373e, 0x32), rates(0x373e, 0x33)), (full.clone(), full.clone(), k1.clone(), full.clone()));
    assert_eq!((rates(0x373e, 0x46), rates(0x373e, 0x47)), (k1, full));
}

// ---------------------------------------------------------------- Order 110: debounce, Motion Sync, angle snapping, ripple

use bu_mouse::family::{Change, Extra, Kind};

#[test]
fn order_110_read_shows_debounce_and_the_flags_like_openmouse() {
    let (m, mouse, mut port) = setup(0x373e, 0x001d);
    mouse.borrow_mut().profile = 3;
    let r = lz::Lamzu.read(&m, &mut port, ReadOptions::default()).unwrap();
    // readStatus: debounceMs = debounce[1]; angleSnapping / motionSync / rippleControl = reply[1] === 1
    assert_eq!(sent(&port)[7..], [rq(0x02, 0x02, 0x00, 0x88, &[3]), rq(0x02, 0x02, 0x01, 0x84, &[3]), rq(0x02, 0x02, 0x01, 0x89, &[3]), rq(0x02, 0x02, 0x01, 0x8a, &[3])]);
    let got: Vec<(Extra, Option<i32>)> = r.extras.iter().map(|x| (x.extra, x.value)).collect();
    assert_eq!(got, vec![(Extra::Debounce, Some(4)), (Extra::MotionSync, Some(1)), (Extra::AngleSnap, Some(0)), (Extra::RippleControl, Some(1))]);
    assert_eq!(r.extras[0].kind, Kind::Range { min: 0, max: 15, step: 1, unit: "ms" });
    assert!(port.sent.iter().all(|s| lz::read_request_allowed(&s.0)));
    // each read is optional (.catch(() => null)): an unsupported one leaves its row without a value
    let (m, mouse, mut port) = setup(0x373e, 0x001d);
    mouse.borrow_mut().unsupported = vec![(0, 0x88), (1, 0x89)];
    let r = lz::Lamzu.read(&m, &mut port, ReadOptions::default()).unwrap();
    assert_eq!((r.extra(Extra::Debounce), r.extra(Extra::MotionSync), r.extra(Extra::AngleSnap)), (None, None, Some(0)));
}

#[test]
fn order_110_changes_are_openmouses_setflag_and_setdebouncetime_bytes() {
    let (m, mouse, mut port) = setup(0x373e, 0x001d);
    mouse.borrow_mut().profile = 3;
    // currentProfile (device 0x85), then write(page, command, profile, [value]) = length 2, [profile, value], then the read
    for (c, write, read) in [
        (Change::Extra(Extra::Debounce, 8), rq(0x02, 0x02, 0x00, 0x08, &[3, 8]), rq(0x02, 0x02, 0x00, 0x88, &[3])),
        (Change::Extra(Extra::MotionSync, 0), rq(0x02, 0x02, 0x01, 0x09, &[3, 0]), rq(0x02, 0x02, 0x01, 0x89, &[3])),
        (Change::Extra(Extra::AngleSnap, 1), rq(0x02, 0x02, 0x01, 0x04, &[3, 1]), rq(0x02, 0x02, 0x01, 0x84, &[3])),
        (Change::Extra(Extra::RippleControl, 0), rq(0x02, 0x02, 0x01, 0x0a, &[3, 0]), rq(0x02, 0x02, 0x01, 0x8a, &[3])),
    ] {
        port.sent.clear();
        lz::Lamzu.change(&m, &mut port, c).unwrap_or_else(|e| panic!("{c:?}: {e}"));
        assert_eq!(sent(&port), vec![rq(0x02, 0x01, 0x00, 0x85, &[]), write.clone(), read], "{c:?}");
        assert!(!lz::read_request_allowed(&write), "{c:?}: a write never passes the allow-list");
    }
    assert_eq!((mouse.borrow().debounce, mouse.borrow().flags), (8, [0, 1, 0]));
    // a mouse that keeps the old value is an error
    mouse.borrow_mut().keep_extra = true;
    assert!(matches!(lz::Lamzu.change(&m, &mut port, Change::Extra(Extra::Debounce, 2)), Err(Error::BadAnswer(_))));
    // refused before a byte leaves: debounce 0-15, switches 0 / 1, no turbo (Lamzu's "competitive mode" is not turbo)
    port.sent.clear();
    for c in [Change::Extra(Extra::Debounce, 16), Change::Extra(Extra::MotionSync, 2), Change::Extra(Extra::Turbo, 1)] {
        assert!(lz::Lamzu.change(&m, &mut port, c).is_err(), "{c:?}");
    }
    assert!(port.sent.is_empty());
}

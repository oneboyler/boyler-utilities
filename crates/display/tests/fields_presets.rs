//! DESIGN §3.2.2 field rules and §3.2.4 presets, against plain data.

use bu_display::fields::*;
use bu_display::presets::PresetList;
use bu_display::{DisplayError, GpuScaling, RefreshRate, VideoMode};

fn r(mhz: u32) -> RefreshRate {
    RefreshRate::new(mhz, 1000)
}

fn lg_144() -> Vec<RefreshRate> {
    vec![r(143_981), r(119_982), r(60_000)]
}

#[test]
fn width_height_clamp_and_parse() {
    assert_eq!(clamp_width(100), 640);
    assert_eq!(clamp_width(9999), 7680);
    assert_eq!(clamp_height(10), 480);
    assert_eq!(clamp_height(5000), 4320);
    assert_eq!(parse_size_field("19x20", true), Some(1920));
    assert_eq!(parse_size_field("123456", true), Some(1234)); // up to 4 digits
    assert_eq!(parse_size_field("99", false), Some(480));
    assert_eq!(parse_size_field("", true), None);
    assert_eq!(parse_hz_field("143.98"), Some(143.98));
    assert_eq!(parse_hz_field("1a4b4"), Some(144.0));
    assert_eq!(parse_hz_field("1234567"), Some(123456.0)); // up to 6 characters
    assert_eq!(parse_hz_field("."), None);
}

#[test]
fn hz_snaps_to_reported_rates_and_caps_at_fastest() {
    // DESIGN: 240 typed on a 144 Hz LG → 143.98, shown "144".
    let rates = lg_144();
    let s = snap_hz(240.0, &rates).unwrap();
    assert_eq!(s, r(143_981));
    assert_eq!(rate_label(s, &rates), "144");
    assert_eq!(snap_hz(143.98, &rates).unwrap(), r(143_981));
    assert_eq!(snap_hz(100.0, &rates).unwrap(), r(119_982));
    assert_eq!(snap_hz(10.0, &rates).unwrap(), r(60_000));
    // A tie goes to the faster rate.
    assert_eq!(snap_hz(90.0, &[r(60_000), r(120_000)]).unwrap(), r(120_000));
    assert_eq!(snap_hz(60.0, &[]), None);
}

#[test]
fn rate_labels_like_the_nvidia_panel() {
    // DESIGN example list.
    let list: Vec<RefreshRate> =
        [165_000, 144_000, 120_000, 119_880, 100_000, 60_000, 59_940].iter().map(|m| r(*m)).collect();
    let labels: Vec<String> = list.iter().map(|x| rate_label(*x, &list)).collect();
    assert_eq!(labels, ["165", "144", "120", "119.88", "100", "60", "59.94"]);
    // A lone 239.76 shows as "240".
    assert_eq!(rate_label(r(239_760), &[r(239_760), r(60_000)]), "240");
    // 60000/1001 (exact NTSC rate) next to 60.
    let ntsc = RefreshRate::new(60_000, 1001);
    assert_eq!(rate_label(ntsc, &[ntsc, RefreshRate::whole(60)]), "59.94");
    assert_eq!(rate_label(RefreshRate::whole(60), &[ntsc, RefreshRate::whole(60)]), "60");
    // Measured on a real 360 Hz monitor: 360, 359.999, 359.998 in one list — 3 decimals keep them apart.
    let z = [RefreshRate::whole(360), r(359_999), r(359_998)];
    let zl: Vec<String> = z.iter().map(|x| rate_label(*x, &z)).collect();
    assert_eq!(zl, ["360", "359.999", "359.998"]);
    assert_eq!(exact_label(r(164_950)), "164.95");
    assert_eq!(exact_label(RefreshRate::whole(165)), "165");
}

#[test]
fn hz_popup_fastest_first_with_check() {
    let rates = lg_144();
    let menu = rate_menu(&rates, r(119_982));
    let labels: Vec<&str> = menu.iter().map(|(_, l, _)| l.as_str()).collect();
    assert_eq!(labels, ["144 Hz", "120 Hz", "60 Hz"]);
    assert_eq!(menu.iter().filter(|(_, _, c)| *c).count(), 1);
    assert!(menu[1].2);
}

#[test]
fn arrow_and_wheel_stepping_stops_at_ends() {
    assert_eq!(step_through(1920, &COMMON_WIDTHS, true), 2560);
    assert_eq!(step_through(1920, &COMMON_WIDTHS, false), 1680);
    assert_eq!(step_through(2000, &COMMON_WIDTHS, true), 2560); // off-list value steps to the next stop
    assert_eq!(step_through(3840, &COMMON_WIDTHS, true), 3840);
    assert_eq!(step_through(1024, &COMMON_WIDTHS, false), 1024);
    assert_eq!(step_through(1080, &COMMON_HEIGHTS, true), 1200);
    assert_eq!(step_through(720, &COMMON_HEIGHTS, false), 720);
    let rates = lg_144();
    assert_eq!(step_hz(r(119_982), &rates, true), r(143_981));
    assert_eq!(step_hz(r(143_981), &rates, true), r(143_981));
    assert_eq!(step_hz(r(60_000), &rates, false), r(60_000));
}

fn modes() -> Vec<VideoMode> {
    let mut v = vec![];
    for (w, h) in [(1920, 1080), (1280, 960)] {
        for m in [143_981, 60_000] {
            v.push(VideoMode { width: w, height: h, refresh: r(m) });
        }
    }
    v.push(VideoMode { width: 1024, height: 768, refresh: r(60_000) });
    v
}

#[test]
fn resolve_fields_snaps_per_size_and_refuses_unsupported_sizes() {
    let m = resolve_fields(&modes(), 1920, 1080, 240.0, GpuScaling::Stretch).unwrap();
    assert_eq!((m.width, m.height, m.refresh), (1920, 1080, r(143_981)));
    // 1024x768 only has 60 here: 144 typed snaps to 60.
    let m = resolve_fields(&modes(), 1024, 768, 144.0, GpuScaling::BlackBars).unwrap();
    assert_eq!(m.refresh, r(60_000));
    match resolve_fields(&modes(), 1600, 900, 144.0, GpuScaling::Stretch) {
        Err(DisplayError::ModeNotSupported { width: 1600, height: 900, nearest: Some(n) }) => {
            assert_eq!((n.width, n.height), (1920, 1080))
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(resolve_fields(&[], 1920, 1080, 60.0, GpuScaling::Stretch), Err(DisplayError::NoModes));
}

#[test]
fn presets_sorted_no_names_every_one_has_scaling() {
    let mut p = PresetList::new();
    let md = modes();
    p.add(1280, 960, 144.0, GpuScaling::BlackBars, &md).unwrap();
    p.add(1920, 1080, 60.0, GpuScaling::KeepAspect, &md).unwrap();
    p.add(1920, 1080, 144.0, GpuScaling::KeepAspect, &md).unwrap();
    p.add(1920, 1080, 144.0, GpuScaling::Stretch, &md).unwrap();
    let order: Vec<(u32, u32, u64, GpuScaling)> = p.items().iter().map(|x| (x.width, x.height, x.refresh.millihz(), x.scaling)).collect();
    assert_eq!(
        order,
        [
            (1920, 1080, 143_981, GpuScaling::Stretch),
            (1920, 1080, 143_981, GpuScaling::KeepAspect),
            (1920, 1080, 60_000, GpuScaling::KeepAspect),
            (1280, 960, 143_981, GpuScaling::BlackBars),
        ]
    );
}

#[test]
fn duplicate_preset_refused_with_existing_index() {
    let mut p = PresetList::new();
    let md = modes();
    p.add(1920, 1080, 144.0, GpuScaling::Stretch, &md).unwrap();
    p.add(1280, 960, 60.0, GpuScaling::Stretch, &md).unwrap();
    // 240 typed snaps to 143.98 → same as the first preset.
    assert_eq!(p.add(1920, 1080, 240.0, GpuScaling::Stretch, &md), Err(DisplayError::DuplicatePreset(0)));
    // Different scaling is a different preset.
    assert!(p.add(1920, 1080, 240.0, GpuScaling::BlackBars, &md).is_ok());
}

#[test]
fn preset_delete_restore_and_match() {
    let mut p = PresetList::new();
    let md = modes();
    let a = p.add(1920, 1080, 144.0, GpuScaling::Stretch, &md).unwrap();
    let b = p.add(1280, 960, 60.0, GpuScaling::BlackBars, &md).unwrap();
    let applied = p.get(b).unwrap().resolve(&md).unwrap();
    assert_eq!(p.matching(&applied, &md), Some(b));
    let removed = p.remove(a).unwrap();
    assert_eq!(p.items().len(), 1);
    assert_eq!(p.remove(a), Err(DisplayError::PresetNotFound));
    p.restore(removed);
    assert_eq!(p.items()[0].id, a);
    // A preset saved at 165 Hz on another monitor snaps to this one's fastest.
    let mut q = PresetList::new();
    let id = q.add(1920, 1080, 165.0, GpuScaling::Stretch, &[VideoMode { width: 1920, height: 1080, refresh: r(165_000) }]).unwrap();
    assert_eq!(q.get(id).unwrap().resolve(&md).unwrap().refresh, r(143_981));
}

#[test]
fn presets_serialize_round_trip() {
    let mut p = PresetList::new();
    p.add(1920, 1080, 144.0, GpuScaling::Stretch, &modes()).unwrap();
    let s = serde_json::to_string(&p).unwrap();
    let back: PresetList = serde_json::from_str(&s).unwrap();
    assert_eq!(back, p);
    assert!(s.contains("Stretch"));
}

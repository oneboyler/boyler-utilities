//! Raw Accel: the maths (worked check + independent gain/legacy derivative checks), validation, settings.json,
//! the driver bytes, the card (curves, values, presets), mirroring, syncing the (fake) driver, per app, header line.

use bu_mouse::accel::args::*;
use bu_mouse::accel::bytes;
use bu_mouse::accel::curves::*;
use bu_mouse::accel::panel::*;
use bu_mouse::accel::service::*;
use bu_mouse::accel::switch::*;
use bu_mouse::fake::FakeOs;
use bu_mouse::os::{DriverVersion, WinRaw, WinSetting};
use bu_mouse::{AppDirs, Error, Mouse};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Raw Accel 1.7.0's settings.json in the shape it writes (values = the drawing's sample settings: Linear, Gain,
/// acceleration 2.8, offset 55, output cap 2.6).
const SETTINGS: &str = r####"{
  "### Accel modes ###": "classic | jump | natural | synchronous | power | lut | noaccel",
  "### Cap modes ###": "in_out | input | output",
  "version": "1.7.0",
  "defaultDeviceConfig": { "disable": false, "Use constant time interval based on polling rate": false,
    "DPI (normalizes input speed unit: counts/ms -> in/s)": 0, "Polling rate Hz (keep at 0 for automatic adjustment)": 0 },
  "profiles": [ { "name": "default",
      "Stretches domain for horizontal vs vertical inputs": { "x": 1.0, "y": 1.0 },
      "Stretches accel range for horizontal vs vertical inputs": { "x": 1.0, "y": 1.0 },
      "Whole or horizontal accel parameters": { "mode": "classic", "Gain / Velocity": true, "inputOffset": 55.0, "outputOffset": 0.0,
        "acceleration": 2.8, "decayRate": 0.1, "gamma": 1.0, "motivity": 1.5, "exponentClassic": 2.0, "scale": 1.0, "exponentPower": 0.05,
        "limit": 1.5, "syncSpeed": 5.0, "smooth": 0.5, "Cap / Jump": { "x": 15.0, "y": 2.6 }, "Cap mode": "output", "data": [] },
      "Vertical accel parameters": { "mode": "classic", "Gain / Velocity": true, "inputOffset": 55.0, "outputOffset": 0.0,
        "acceleration": 2.8, "decayRate": 0.1, "gamma": 1.0, "motivity": 1.5, "exponentClassic": 2.0, "scale": 1.0, "exponentPower": 0.05,
        "limit": 1.5, "syncSpeed": 5.0, "smooth": 0.5, "Cap / Jump": { "x": 15.0, "y": 2.6 }, "Cap mode": "output", "data": [] },
      "Input speed calculation parameters": { "Whole/combined accel (set false for 'by component' mode)": true, "lpNorm": 2.0,
        "Time in ms after which an input is weighted at half its original value.": 0.0,
        "Time in ms after which scale is weighted at half its original value.": 0.0,
        "Time in ms after which an output is weighted at half its original value.": 0.0 },
      "Output DPI": 1000.0, "Y/X output DPI ratio (vertical sens multiplier)": 1.0, "L/R output DPI ratio (left sens multiplier)": 1.0,
      "U/D output DPI ratio (up sens multiplier)": 1.0, "Degrees of rotation": 0.0, "Degrees of angle snapping": 0.0, "Input Speed Cap": 0.0 } ],
  "devices": []
}"####;

fn his_cfg() -> DriverConfig {
    DriverConfig::from_json(SETTINGS).unwrap()
}

fn sens_of(p: &Profile, x: f64) -> f64 {
    sensitivity(p, &init_data(p), x)
}

fn profile_with(a: AccelArgs) -> Profile {
    Profile { accel_x: a.clone(), accel_y: a, ..Profile::default() }
}

// ---------------------------------------------------------------- maths

#[test]
fn his_curve_matches_raw_accels_window() {
    let p = his_cfg().profiles[0].clone();
    // DESIGN: "flat 1.0 to 55, then 60 → 1.13, 80 → 1.50, 100 → 1.72, 120 → 1.86 — the same as the sample's Raw Accel window"
    for x in [1.0, 20.0, 54.0, 55.0] {
        assert_eq!(sens_of(&p, x), 1.0, "flat at {x}");
    }
    for (x, want) in [(60.0, 1.13), (80.0, 1.50), (100.0, 1.72), (120.0, 1.86)] {
        let got = sens_of(&p, x);
        assert_eq!(format!("{got:.2}"), format!("{want:.2}"), "at {x}: {got}");
    }
    // exact constants from the source formulas (research worked check)
    let Accel::ClassicGain { accel_raised, cap, constant, sign } = Accel::new(&p.accel_x) else { panic!() };
    assert_eq!((accel_raised, sign), (2.8, 1.0));
    assert!((cap.x - 55.285_714_285_714_29).abs() < 1e-12);
    assert!((cap.y - 1.6).abs() < 1e-15);
    assert!((constant + 88.228_571_428_571_4).abs() < 1e-9);
    assert!((sens_of(&p, 60.0) - 1.129_523_809_5).abs() < 1e-9);
}

#[test]
fn legacy_output_cap_is_a_sensitivity_cap() {
    let mut a = his_cfg().profiles[0].accel_x.clone();
    a.gain = false;
    let p = profile_with(a);
    // legacy: sens = 1 + min(a·(x−off)²/x, cap.y − 1): at 60 still under the cap, capped at 2.6 from ~61.6 on
    assert!((sens_of(&p, 60.0) - (1.0 + 2.8 * 25.0 / 60.0)).abs() < 1e-12);
    assert_eq!(sens_of(&p, 62.0), 2.6);
    assert_eq!(sens_of(&p, 120.0), 2.6);
    assert!((sens_of(&p, 56.0) - (1.0 + 2.8 * 1.0 / 56.0)).abs() < 1e-12);
}

/// d/dx [x · s(x)] by central difference.
fn gain_of(p: &Profile, x: f64) -> f64 {
    let h = 1e-4;
    ((x + h) * sens_of(p, x + h) - (x - h) * sens_of(p, x - h)) / (2.0 * h)
}

#[test]
fn gain_modes_are_the_integral_of_their_legacy_curve() {
    // An independent check of the port: in Raw Accel, "Gain" applies the legacy shape to the slope of the output
    // velocity, so d(x·s_gain)/dx must equal the legacy sensitivity where the shapes are defined that way.
    let jump = AccelArgs { mode: AccelMode::Jump, cap: Vec2 { x: 15.0, y: 2.6 }, smooth: 0.5, ..AccelArgs::default() };
    let sync = AccelArgs { mode: AccelMode::Synchronous, sync_speed: 5.0, motivity: 1.5, gamma: 1.0, smooth: 0.5, ..AccelArgs::default() };
    let power = AccelArgs { mode: AccelMode::Power, scale: 1.0, exponent_power: 0.05, cap: Vec2 { x: 0.0, y: 0.0 }, ..AccelArgs::default() };
    for (a, tol) in [(jump, 1e-6), (sync, 2e-2), (power, 1e-6)] {
        let g = profile_with(AccelArgs { gain: true, ..a.clone() });
        let l = profile_with(AccelArgs { gain: false, ..a.clone() });
        for x in [3.0, 7.5, 12.0, 20.0, 45.0, 90.0] {
            let want = if a.mode == AccelMode::Power { (a.exponent_power + 1.0) * (a.scale * x).powf(a.exponent_power) } else { sens_of(&l, x) };
            let got = gain_of(&g, x);
            assert!((got - want).abs() < tol, "{:?} at {x}: gain slope {got} vs {want}", a.mode);
        }
    }
    // classic with no cap: legacy and gain are the same curve (research §5)
    let c = AccelArgs { mode: AccelMode::Classic, acceleration: 0.02, exponent_classic: 2.5, input_offset: 20.0, cap: Vec2 { x: 0.0, y: 0.0 }, ..AccelArgs::default() };
    for x in [10.0, 30.0, 77.0] {
        assert_eq!(sens_of(&profile_with(AccelArgs { gain: true, ..c.clone() }), x), sens_of(&profile_with(AccelArgs { gain: false, ..c.clone() }), x));
    }
    // classic gain with an output cap: the slope is flat at the cap above cap.x
    let p = his_cfg().profiles[0].clone();
    assert!((gain_of(&p, 90.0) - 2.6).abs() < 1e-6);
    assert!((gain_of(&p, 55.2) - (1.0 + 2.0 * 2.8 * 0.2)).abs() < 1e-3, "below the cap the gain line is 1 + 2a(x - off)");
}

#[test]
fn natural_and_caps_behave() {
    let n = profile_with(AccelArgs { mode: AccelMode::Natural, decay_rate: 0.1, limit: 1.5, input_offset: 0.0, gain: false, ..AccelArgs::default() });
    assert!(sens_of(&n, 20.0) > 1.3 && sens_of(&n, 20.0) < 1.5, "legacy natural rises towards the limit");
    assert_eq!(sens_of(&n, 1000.0), 1.5, "and reaches it");
    let ng = profile_with(AccelArgs { gain: true, ..n.accel_x.clone() });
    assert!((gain_of(&ng, 200.0) - 1.5).abs() < 1e-4, "natural gain's slope approaches the limit");
    // input cap (classic, gain): flat gain above cap.x
    let ci = profile_with(AccelArgs { mode: AccelMode::Classic, acceleration: 0.05, input_offset: 10.0, cap: Vec2 { x: 40.0, y: 0.0 }, cap_mode: CapMode::Input, ..AccelArgs::default() });
    let g40 = 1.0 + 2.0 * 0.05 * 30.0;
    assert!((gain_of(&ci, 80.0) - g40).abs() < 1e-6);
    // both caps (classic, gain): the curve passes through gain (cap.x, cap.y)
    let cb = profile_with(AccelArgs { mode: AccelMode::Classic, input_offset: 10.0, cap: Vec2 { x: 40.0, y: 2.0 }, cap_mode: CapMode::InOut, ..AccelArgs::default() });
    assert!((gain_of(&cb, 39.9) - 2.0).abs() < 1e-2);
    assert!((gain_of(&cb, 60.0) - 2.0).abs() < 1e-6);
    // sens multiplier scales everything (Output DPI / 1000)
    let mut p = his_cfg().profiles[0].clone();
    p.output_dpi = 1500.0;
    assert!((sens_of(&p, 30.0) - 1.5).abs() < 1e-12);
}

#[test]
fn synchronous_gain_table_has_97_floats() {
    let a = AccelArgs { mode: AccelMode::Synchronous, ..AccelArgs::default() };
    let Accel::SyncGain { table, range, x_start, .. } = Accel::new(&a) else { panic!() };
    assert_eq!(table.len(), 97);
    assert_eq!(range.size(), 97);
    assert_eq!(x_start, 0.125);
    assert!(table.windows(2).all(|w| w[1] >= w[0]), "an integral grows");
}

#[test]
fn validation_is_raw_accels() {
    let mut p = his_cfg().profiles[0].clone();
    assert!(validate(&p).is_empty());
    p.accel_x.acceleration = 0.0;
    assert_eq!(validate(&p), vec!["acceleration must be positive"]);
    p.accel_x.acceleration = 2.8;
    p.accel_x.cap_mode = CapMode::Input;
    p.accel_x.cap.x = 30.0;
    assert_eq!(validate(&p), vec!["cap < offset"]);
    p.accel_x.cap_mode = CapMode::Output;
    p.accel_x.motivity = 1.0;
    assert_eq!(validate(&p), vec!["motivity must be greater than 1"]);
    p.accel_x.motivity = 1.5;
    p.output_dpi = 0.0;
    p.degrees_snap = 50.0;
    assert_eq!(validate(&p), vec!["snap angle must be between 0 and 45 degrees", "output DPI is 0"]);
}

// ---------------------------------------------------------------- settings.json + bytes

#[test]
fn settings_json_reads_like_the_wrapper_and_writes_back_equal() {
    let c = his_cfg();
    let p = &c.profiles[0];
    assert_eq!((p.accel_x.mode, p.accel_x.gain, p.accel_x.cap_mode), (AccelMode::Classic, true, CapMode::Output));
    assert_eq!((p.accel_x.acceleration, p.accel_x.input_offset, p.accel_x.cap.y), (2.8, 55.0, 2.6));
    assert_eq!(c.default_device_config.clamp_min, DEFAULT_TIME_MIN, "missing key → C++ default (Populate)");
    let back = DriverConfig::from_json(&c.to_json()).unwrap();
    assert_eq!(back, c);
    assert!(c.to_json().starts_with("{\n  \"### Accel modes ###\""));
    // empty JSON → one default profile
    let d = DriverConfig::from_json("{}").unwrap();
    assert_eq!(d.profiles.len(), 1);
    assert_eq!(d.profiles[0].accel_x.mode, AccelMode::Noaccel);
    assert!(DriverConfig::from_json("{ nope").is_err());
}

#[test]
fn driver_bytes_layout_and_roundtrip() {
    assert_eq!(bytes::MODIFIER_SETTINGS, 5184);
    assert_eq!(bytes::write_size(1, 0), 5224);
    assert_eq!(bytes::write_size(2, 1), 40 + 2 * 5184 + 1456);
    let mut c = his_cfg();
    c.devices.push(DeviceSettings { name: "Pulsar".into(), profile: "default".into(), id: r"HID\VID_3710&PID_5406&MI_00".into(), config: DeviceConfig { dpi: 1600, ..DeviceConfig::default() } });
    let b = bytes::to_bytes(&c);
    assert_eq!(b.len(), bytes::write_size(1, 1));
    assert_eq!(bytes::read_header(&b), Some((1, 1)));
    let back = bytes::read_profiles(&b);
    assert_eq!(back, c.profiles);
    // classic<GAIN> union at data_t + 24: accel_raised, cap.x, cap.y, constant, sign
    let u = 40 + bytes::PROFILE + 24;
    assert_eq!(f64::from_le_bytes(b[u..u + 8].try_into().unwrap()), 2.8);
    assert_eq!(f64::from_le_bytes(b[u + 32..u + 40].try_into().unwrap()), 1.0);
    // device id + dpi
    let d0 = 40 + 5184;
    assert_eq!(u16::from_le_bytes([b[d0 + 1024], b[d0 + 1025]]), 'H' as u16);
    assert_eq!(i32::from_le_bytes(b[d0 + 1428..d0 + 1432].try_into().unwrap()), 1600);
    // the synchronous table travels in the profile's accel_args.data (offset 544 + 128)
    let mut s = his_cfg();
    s.profiles[0].accel_x = AccelArgs { mode: AccelMode::Synchronous, ..AccelArgs::default() };
    let sb = bytes::to_bytes(&s);
    let t0 = 40 + 544 + 128;
    assert_ne!(f32::from_le_bytes(sb[t0..t0 + 4].try_into().unwrap()), 0.0);
    assert_eq!(i32::from_le_bytes(sb[40 + 544 + 124..40 + 544 + 128].try_into().unwrap()), 0, "length stays the JSON length");
    assert!(bytes::diff(&b, &b, 1, 1, 5).is_empty());
}

// ---------------------------------------------------------------- the card

#[test]
fn card_defaults_rows_and_mapping_like_raw_accels_gui() {
    let panel = Panel::default();
    assert!(!panel.on, "a new user starts off");
    assert_eq!(panel.curve, Curve::Linear);
    assert_eq!(panel.sens, 1.0);
    // the Linear defaults ARE the sample Raw Accel args, field for field (GUI rule: defaults + shown fields; exponent 2)
    assert_eq!(to_args(Curve::Linear, &panel.current_values()), his_cfg().profiles[0].accel_x);
    let v = CurveValues::defaults(Curve::Linear);
    let labels: Vec<&str> = visible_rows(Curve::Linear, &v).iter().map(|r| r.label).collect();
    assert_eq!(labels, vec!["Acceleration", "Input offset", "Cap: output"]);
    let both = CurveValues { cap_type: CapType::Both, ..v.clone() };
    let labels: Vec<&str> = visible_rows(Curve::Linear, &both).iter().map(|r| r.label).collect();
    assert_eq!(labels, vec!["Input offset", "Cap: input", "Cap: output"], "Both hides the rate row");
    let input = CurveValues { cap_type: CapType::Input, ..v };
    let labels: Vec<&str> = visible_rows(Curve::Power, &CurveValues { cap_type: CapType::Input, ..CurveValues::defaults(Curve::Power) }).iter().map(|r| r.label).collect();
    assert_eq!(labels, vec!["Scale", "Exponent", "Output offset", "Cap: input"]);
    assert!(visible_rows(Curve::Linear, &input).iter().all(|r| r.label != "Cap: output"));
    // reverse mapping: exponent exactly 2 = Linear, else Classic; noaccel / lut = none
    let a = his_cfg().profiles[0].accel_x.clone();
    assert_eq!(from_args(&a).map(|x| x.0), Some(Curve::Linear));
    assert_eq!(from_args(&AccelArgs { exponent_classic: 2.5, ..a.clone() }).map(|x| x.0), Some(Curve::Classic));
    assert_eq!(from_args(&AccelArgs { mode: AccelMode::Lut, ..a }), None);
    for c in Curve::ALL {
        let v = CurveValues::defaults(c);
        let (c2, v2) = from_args(&to_args(c, &v)).unwrap();
        assert_eq!((c2, v2), (c, v), "{c:?} roundtrip");
        assert!(validate(&profile_with(to_args(c, &CurveValues::defaults(c)))).is_empty(), "{c:?} defaults are valid for Raw Accel");
    }
}

#[test]
fn every_slider_extreme_is_valid_for_raw_accel() {
    for c in Curve::ALL {
        for spec in rows(c) {
            for v in [spec.min, spec.max] {
                let mut vals = CurveValues::defaults(c);
                vals.values.insert(spec.field, v);
                for t in [CapType::Output, CapType::Input, CapType::Both] {
                    let mut p = Panel::default();
                    p.curve = c;
                    p.values.insert(c, vals.clone());
                    if c.has_cap() {
                        p.set_cap_type(t);
                    }
                    let a = to_args(c, &p.current_values());
                    assert!(validate(&profile_with(a)).is_empty(), "{c:?} {} = {v} ({t:?})", spec.label);
                }
            }
        }
    }
}

#[test]
fn sliders_snap_curves_keep_their_values_cap_rule() {
    let mut p = Panel::default();
    assert_eq!(p.set_value(Field::Acceleration, 2.83), Some(2.85));
    assert_eq!(p.set_value(Field::Acceleration, 99.0), Some(5.0));
    assert_eq!(p.set_value(Field::Gamma, 1.0), None, "not a Linear row");
    assert_eq!(p.set_sens(0.27), 0.25);
    p.set_curve(Curve::Natural);
    p.set_value(Field::Limit, 2.0);
    p.set_gain(false);
    p.set_curve(Curve::Linear);
    assert_eq!(p.current_values().get(Field::Acceleration), 5.0, "switching curves resets nothing");
    assert!(p.current_values().gain, "each curve keeps its own Gain");
    assert!(!p.values[&Curve::Natural].gain);
    // Cap type Input with Cap: input (15) ≤ offset (55) → moves to min(120, 55 + 30) = 85
    p.set_cap_type(CapType::Input);
    assert_eq!(p.current_values().get(Field::CapInput), 85.0);
    p.set_value(Field::InputOffset, 100.0);
    assert_eq!(p.current_values().get(Field::CapInput), 120.0);
    assert_eq!(value_text(&rows(Curve::Linear)[3], 2.6), "2.60×");
}

#[test]
fn presets_load_update_rename_delete() {
    let mut p = Panel::default();
    let (a, toast) = p.save_as_preset();
    assert_eq!(toast, "Saved as Preset 1 · pick it for an app below");
    assert_eq!(p.loaded, Some(a));
    assert!(!p.changed_since_loaded());
    p.set_value(Field::Acceleration, 3.0);
    assert!(p.changed_since_loaded(), "amber dot + Update link");
    assert_eq!(p.update_loaded(), Some("Saved · Preset 1".into()));
    assert!(!p.changed_since_loaded());
    p.rename_preset(a, "  Valorant ").unwrap();
    assert_eq!(p.preset(a).unwrap().name, "Valorant");
    let (b, _) = p.save_as_preset();
    assert_eq!(p.rename_preset(b, "valorant"), Err("another preset has this name"));
    assert_eq!(p.rename_preset(b, &"x".repeat(21)), Err("at most 20 characters"));
    assert_eq!(p.rename_preset(b, "   "), Err("a preset needs a name"));
    p.set_curve(Curve::Jump);
    p.set_sens(2.0);
    assert_eq!(p.load_preset(a), Some("Loaded Valorant".into()));
    assert_eq!((p.curve, p.sens, p.current_values().get(Field::Acceleration)), (Curve::Linear, 1.0, 3.0));
    assert!(p.delete_preset(a).is_some());
    assert_eq!(p.loaded, None);
    assert_eq!(p.presets.len(), 1);
}

// ---------------------------------------------------------------- the service with a fake driver

const RA_DIR: &str = r"C:\fake\RawAccel";

fn with_driver(version: (u32, u32, u32), settings: Option<&str>) -> Mouse<FakeOs> {
    let mut os = FakeOs::new();
    os.rawaccel_version = Some(DriverVersion { major: version.0, minor: version.1, patch: version.2 });
    if let Some(s) = settings {
        os.files.insert(PathBuf::from(RA_DIR).join("settings.json"), s.into());
        // the driver currently runs exactly these settings (Raw Accel's GUI wrote them at start-up)
        os.rawaccel_driver = bytes::to_bytes(&DriverConfig::from_json(s).unwrap());
    }
    os.files.insert(PathBuf::from(RA_DIR).join("writer.exe"), String::new());
    let mut m = Mouse::new(os, AppDirs::new(r"C:\fake\appdata"));
    m.accel_mut().rawaccel_dir = Some(PathBuf::from(RA_DIR));
    m
}

#[test]
fn not_installed_shows_the_install_card() {
    let mut m = Mouse::new(FakeOs::new(), AppDirs::new("x"));
    assert_eq!(m.rawaccel_status().unwrap(), RawAccelStatus::NotInstalled);
    assert_eq!(RawAccelStatus::NotInstalled.footer(), None);
    assert!(matches!(m.sync_driver(), Err(Error::RawAccelMissing(_))));
    assert!(!m.mirror_rawaccel().unwrap());
    assert_eq!(RAWACCEL_RELEASES, "https://github.com/RawAccelOfficial/rawaccel/releases");
}

#[test]
fn first_run_mirrors_raw_accel_and_writes_nothing() {
    let mut m = with_driver((1, 7, 0), Some(SETTINGS));
    let st = m.rawaccel_status().unwrap();
    assert_eq!(st.footer(), Some("Runs on your Raw Accel 1.7.0 · Copy its curve · Open Raw Accel".into()));
    assert!(m.mirror_rawaccel().unwrap());
    let a = m.accel();
    assert!(a.panel.on && !a.panel.expanded, "on but collapsed");
    assert_eq!(a.panel.presets[0].name, "Raw Accel");
    assert_eq!(a.per_app.everywhere_else(), Target::Main, "the mirrored preset is the main one: it runs everywhere");
    assert_eq!(header_line(&a.panel, &a.per_app), "Raw Accel everywhere");
    assert!(!m.sync_driver().unwrap(), "same bytes already in the driver → no write");
    assert_eq!(m.os().rawaccel_byte_writes, 0);
    assert!(!m.mirror_rawaccel().unwrap(), "only on the first run");
    assert_eq!(m.accel().panel.presets.len(), 1);
}

#[test]
fn switch_off_writes_plain_1_to_1_and_on_puts_his_curve_back_byte_exact() {
    let mut m = with_driver((1, 7, 0), Some(SETTINGS));
    let original = m.os().rawaccel_driver.clone();
    m.mirror_rawaccel().unwrap();
    assert_eq!(m.set_accel_on(false).unwrap(), "Acceleration off · plain 1:1 everywhere");
    let off = bytes::read_profiles(&m.os().rawaccel_driver);
    assert_eq!(off[0].accel_x.mode, AccelMode::Noaccel);
    assert_eq!(off[0].output_dpi, 1000.0);
    assert_eq!(m.set_accel_on(true).unwrap(), "Acceleration on · Raw Accel everywhere");
    assert_eq!(m.os().rawaccel_driver, original, "back to exactly what Raw Accel ran");
    assert_eq!(m.os().rawaccel_byte_writes, 2);
    assert!(m.os().rawaccel_writes.is_empty(), "v1.7 → the WRITE ioctl, never writer.exe");
    assert!(!m.os().files.keys().any(|k| k.ends_with("settings.json") && k.starts_with(r"C:\fake\appdata")));
}

#[test]
fn per_app_switches_on_process_start_and_stop() {
    let mut m = with_driver((1, 7, 0), Some(SETTINGS));
    m.mirror_rawaccel().unwrap();
    // a second preset "Fast" for VALORANT, everywhere else Off
    let (fast, _) = m.accel_mut().panel.save_as_preset();
    m.accel_mut().panel.rename_preset(fast, "Fast").unwrap();
    m.accel_mut().panel.set_value(Field::Acceleration, 1.0);
    m.accel_mut().panel.update_loaded();
    let row = m.accel_mut().per_app.add_row(r"C:\Riot Games\VALORANT\live\ShooterGame\Binaries\Win64\VALORANT-Win64-Shipping.exe", Target::Preset(fast));
    m.accel_mut().per_app.set_row_label(row, "VALORANT");
    m.accel_mut().per_app.set_everywhere_else(Target::Off);
    assert_eq!(header_line(&m.accel().panel, &m.accel().per_app), "VALORANT: Fast · otherwise Off");
    assert_eq!(m.accel().per_app.watched_names(), vec!["valorant-win64-shipping.exe".to_string()]);
    m.sync_driver().unwrap();
    assert_eq!(bytes::read_profiles(&m.os().rawaccel_driver)[0].accel_x.mode, AccelMode::Noaccel);
    // the game starts (no window yet) → Fast
    assert!(m.accel_app_event(&AppEvent::Started { pid: 42, exe: "VALORANT-Win64-Shipping.exe".into(), has_window: false }));
    m.sync_driver().unwrap();
    let run = bytes::read_profiles(&m.os().rawaccel_driver);
    assert_eq!((run[0].accel_x.mode, run[0].accel_x.acceleration), (AccelMode::Classic, 1.0));
    // other processes change nothing
    assert!(!m.accel_app_event(&AppEvent::Started { pid: 7, exe: "notepad.exe".into(), has_window: false }));
    // it stops → Off again
    assert!(m.accel_app_event(&AppEvent::Stopped { pid: 42 }));
    m.sync_driver().unwrap();
    assert_eq!(bytes::read_profiles(&m.os().rawaccel_driver)[0].accel_x.mode, AccelMode::Noaccel);
    // too late (already has a window): no switch, a note
    assert!(!m.accel_app_event(&AppEvent::Started { pid: 43, exe: "VALORANT-Win64-Shipping.exe".into(), has_window: true }));
    assert_eq!(m.accel_mut().per_app.take_notes(), vec![SwitchNote::TooLate { row, exe: "VALORANT-Win64-Shipping.exe".into() }]);
    // deleting the preset turns the row Off
    assert_eq!(m.delete_accel_preset(fast), Some("Deleted Fast · VALORANT-Win64-Shipping.exe now Off".into()));
    assert_eq!(m.accel().per_app.rows()[0].target, Target::Off);
}

#[test]
fn two_listed_apps_last_started_wins() {
    let mut pa = PerApp::new();
    let a = pa.add_row("a.exe", Target::Preset(PresetId(1)));
    let _b = pa.add_row(r"C:\Games\b.exe", Target::Preset(PresetId(2)));
    pa.set_everywhere_else(Target::Preset(PresetId(9)));
    assert_eq!(pa.current(), Target::Preset(PresetId(9)));
    pa.on_event(&AppEvent::Started { pid: 1, exe: r"C:\x\A.EXE".into(), has_window: false });
    pa.on_event(&AppEvent::Started { pid: 2, exe: r"C:\Games\b.exe".into(), has_window: false });
    assert_eq!(pa.current(), Target::Preset(PresetId(2)));
    pa.on_event(&AppEvent::Stopped { pid: 2 });
    assert_eq!(pa.current(), Target::Preset(PresetId(1)));
    assert_eq!(pa.deciding_row().map(|r| r.id), Some(a));
    pa.on_event(&AppEvent::Stopped { pid: 1 });
    assert_eq!(pa.current(), Target::Preset(PresetId(9)));
    // a full path in the row must match the full path of the process
    assert!(!exe_matches(r"C:\Games\b.exe", r"D:\Other\b.exe"));
    assert!(exe_matches("b.exe", r"D:\Other\B.EXE"));
    assert_eq!(pa.forget_preset(PresetId(9)), (vec![], true));
    assert_eq!(pa.everywhere_else(), Target::Main, "a deleted preset falls back to the main preset");
}

#[test]
fn settle_groups_events_into_one_write() {
    let t0 = Instant::now();
    let mut s = Settle::new(Duration::from_millis(1000));
    assert_eq!(s.until_due(t0), None);
    s.poke(t0);
    s.poke(t0 + Duration::from_millis(400));
    assert!(!s.take_due(t0 + Duration::from_millis(1000)), "re-armed by the second event");
    assert_eq!(s.until_due(t0 + Duration::from_millis(1000)), Some(Duration::from_millis(400)));
    assert!(s.take_due(t0 + Duration::from_millis(1400)));
    assert!(!s.take_due(t0 + Duration::from_millis(5000)), "once");
}

#[test]
fn live_tuning_reaches_the_driver_while_the_loaded_preset_runs() {
    let mut m = with_driver((1, 7, 0), Some(SETTINGS));
    m.mirror_rawaccel().unwrap();
    m.accel_mut().panel.set_value(Field::Acceleration, 2.0);
    assert!(m.sync_driver().unwrap());
    assert_eq!(bytes::read_profiles(&m.os().rawaccel_driver)[0].accel_x.acceleration, 2.0);
    assert!(!m.sync_driver().unwrap(), "nothing changed → no second write");
}

#[test]
fn other_driver_version_goes_through_writer_exe_with_the_same_json() {
    let mut m = with_driver((1, 6, 1), Some(SETTINGS));
    m.mirror_rawaccel().unwrap();
    m.set_accel_on(false).unwrap();
    assert_eq!(m.os().rawaccel_byte_writes, 0);
    let (file, json) = m.os().rawaccel_writes[0].clone();
    assert_eq!(file, PathBuf::from(r"C:\fake\appdata\rawaccel\settings.json"), "the app's own file, never Raw Accel's");
    let c = DriverConfig::from_json(&json).unwrap();
    assert_eq!(c.profiles[0].accel_x.mode, AccelMode::Noaccel);
    m.os_mut().rawaccel_refuse.push_back("Bad settings: ...".into());
    m.accel_mut().panel.on = true;
    assert!(matches!(m.sync_driver(), Err(Error::RawAccelRefused(_))));
}

#[test]
fn copy_its_curve_and_the_keeps_of_his_profile() {
    let mut odd = his_cfg();
    odd.profiles[0].degrees_rotation = 2.5;
    odd.profiles[0].accel_x.mode = AccelMode::Power;
    odd.profiles[0].accel_x.scale = 1.2;
    let mut m = with_driver((1, 7, 0), Some(&odd.to_json()));
    assert_eq!(m.copy_its_curve().unwrap(), "Copied Raw Accel's Power curve");
    assert_eq!(m.accel().panel.curve, Curve::Power);
    assert_eq!(m.accel().panel.current_values().get(Field::Scale), 1.2);
    // writing keeps everything else of the profile (rotation)
    m.accel_mut().panel.on = true;
    m.accel_mut().per_app.set_everywhere_else(Target::Off);
    m.sync_driver().unwrap();
    assert_eq!(bytes::read_profiles(&m.os().rawaccel_driver)[0].degrees_rotation, 2.5);
    // a look-up table can't be shown
    let mut lut = his_cfg();
    lut.profiles[0].accel_x.mode = AccelMode::Lut;
    lut.profiles[0].accel_x.data = vec![1.0, 1.0, 2.0, 2.0];
    let mut m = with_driver((1, 7, 0), Some(&lut.to_json()));
    assert!(matches!(m.copy_its_curve(), Err(Error::RawAccelSettings(_))));
}

#[test]
fn epp_warning_graph_readout_dpi_line() {
    let mut m = with_driver((1, 7, 0), Some(SETTINGS));
    m.mirror_rawaccel().unwrap();
    assert!(m.epp_warning().unwrap(), "fake Windows has EPP on");
    m.os_mut().win.insert(WinSetting::Precision, WinRaw::Mouse([0, 0, 0]));
    assert!(!m.epp_warning().unwrap());
    let g = m.accel_graph(1.0).unwrap();
    assert_eq!(g.len(), 121);
    assert_eq!(g[55].1, 1.0);
    assert_eq!(readout(g[60].0, g[60].1), "60 → 1.13×");
    assert_eq!(readout(g[120].0, g[120].1), "120 → 1.86×");
    assert_eq!(dpi_line(Some(1600), 800), "at 1600 DPI · from Your mouse");
    assert_eq!(dpi_line(None, 800), "at [800] DPI");
    assert!(header_line(&Panel::default(), &PerApp::new()).starts_with("Fast flicks go further"));
}

#[test]
fn a_denied_driver_write_is_typed() {
    let mut m = with_driver((1, 7, 0), Some(SETTINGS));
    m.mirror_rawaccel().unwrap();
    m.os_mut().deny_writes = 1;
    assert!(matches!(m.set_accel_on(false), Err(Error::NeedsAdmin { .. })));
}

// ---------------------------------------------------------------- Order 063: the card is kept, the driver is re-checked

/// The same profile with both axes Off (what Raw Accel's "Off" / noaccel writes).
fn off_cfg() -> DriverConfig {
    let mut c = his_cfg();
    c.profiles[0].accel_x = off_args();
    c.profiles[0].accel_y = off_args();
    c
}

#[test]
fn same_effect_ignores_padding_and_the_stale_bytes_of_an_off_axis_only() {
    let off = bytes::to_bytes(&off_cfg());
    // stale numbers in an Off axis' union + its unused arguments (measured on the real driver, Oct 9: a live Off profile
    // held an old curve's numbers there — Raw Accel's own GUI leaves them): same behaviour
    let mut stale = off.clone();
    let at = bytes::IO_BASE + bytes::PROFILE + 24;
    stale[at..at + 8].copy_from_slice(&0.001f64.to_le_bytes());
    stale[at + 16..at + 24].copy_from_slice(&1.6f64.to_le_bytes());
    let args = bytes::IO_BASE + 544;
    stale[args + 8 + 16..args + 8 + 24].copy_from_slice(&7.5f64.to_le_bytes());
    assert!(!bytes::diff(&off, &stale, 1, 0, 1).is_empty(), "the plain diff does see them");
    assert!(bytes::same_effect(&off, &stale));
    // padding bytes never count
    let mut pad = off.clone();
    pad[3] = 9;
    assert!(bytes::same_effect(&off, &pad));
    // a real difference does: the sens multiplier, the mode, a curve number of an ON axis
    let mut sens = off.clone();
    let tail = bytes::IO_BASE + 4952;
    sens[tail..tail + 8].copy_from_slice(&1200.0f64.to_le_bytes());
    assert!(!bytes::same_effect(&off, &sens));
    let on = bytes::to_bytes(&his_cfg());
    assert!(!bytes::same_effect(&off, &on) && !bytes::same_effect(&on, &off));
    let mut curve = on.clone();
    let u = bytes::IO_BASE + bytes::PROFILE + 24;
    curve[u] ^= 0x10;
    assert!(!bytes::same_effect(&on, &curve), "stale bytes in an axis that is ON are a real difference");
    // sizes / a driver that was never written (header only)
    assert!(!bytes::same_effect(&on, &on[..bytes::IO_BASE]));
    assert!(!bytes::same_effect(&on, &[]));
}

#[test]
fn the_card_is_saved_and_comes_back_with_the_switch_the_presets_and_the_rows() {
    let mut m = with_driver((1, 7, 0), Some(SETTINGS));
    assert!(m.mirror_rawaccel().unwrap());
    let (fast, _) = m.accel_mut().panel.save_as_preset();
    m.accel_mut().panel.rename_preset(fast, "Fast").unwrap();
    let row = m.accel_mut().per_app.add_row("VALORANT-Win64-Shipping.exe", Target::Preset(fast));
    m.accel_mut().per_app.set_row_label(row, "VALORANT");
    assert!(m.save_accel().unwrap());
    assert!(!m.save_accel().unwrap(), "unchanged → no second write");
    // "a restart": a new service on the same files
    let mut again = with_driver((1, 7, 0), Some(SETTINGS));
    again.os_mut().byte_files = m.os().byte_files.clone();
    assert!(again.load_accel().unwrap());
    assert_eq!(again.accel().panel, m.accel().panel);
    assert_eq!(again.accel().per_app.rows(), m.accel().per_app.rows());
    assert_eq!(again.accel().per_app.everywhere_else(), m.accel().per_app.everywhere_else());
    assert!(again.accel().panel.on);
    // damaged file: an error, the card stays empty and the file is not replaced
    let file = again.dirs().accel_file();
    again.os_mut().byte_files.insert(file.clone(), b"{ nope".to_vec());
    let mut fresh = with_driver((1, 7, 0), Some(SETTINGS));
    fresh.os_mut().byte_files = again.os().byte_files.clone();
    assert!(fresh.load_accel().is_err());
    assert!(fresh.accel().panel.presets.is_empty());
    assert_eq!(fresh.os().byte_files.get(&file).map(|b| b.as_slice()), Some(&b"{ nope"[..]));
}

#[test]
fn app_start_writes_the_driver_only_when_the_user_had_it_on_and_the_driver_differs() {
    // nothing saved: nothing written (the Mouse tab mirrors Raw Accel when it opens)
    let mut m = with_driver((1, 7, 0), Some(SETTINGS));
    assert_eq!(m.start_accel().unwrap(), StartAccel::NothingSaved);
    assert_eq!(m.os().rawaccel_byte_writes, 0);
    // saved ON, and the driver already runs it: nothing written
    m.mirror_rawaccel().unwrap();
    m.save_accel().unwrap();
    let saved = m.os().byte_files.clone();
    let mut on = with_driver((1, 7, 0), Some(SETTINGS));
    on.os_mut().byte_files = saved.clone();
    assert_eq!(on.start_accel().unwrap(), StartAccel::AlreadyRunning);
    assert_eq!(on.os().rawaccel_byte_writes, 0);
    // saved ON, but Raw Accel's app wrote Off meanwhile: written once
    let mut reset = with_driver((1, 7, 0), Some(SETTINGS));
    reset.os_mut().byte_files = saved.clone();
    reset.os_mut().rawaccel_driver = bytes::to_bytes(&off_cfg());
    assert_eq!(reset.start_accel().unwrap(), StartAccel::Applied);
    assert_eq!(reset.os().rawaccel_byte_writes, 1);
    assert_eq!(bytes::read_profiles(&reset.os().rawaccel_driver)[0].accel_x.mode, AccelMode::Classic);
    let mut header_only = with_driver((1, 7, 0), Some(SETTINGS));
    header_only.os_mut().byte_files = saved.clone();
    header_only.os_mut().rawaccel_driver = vec![0; bytes::IO_BASE];
    assert_eq!(header_only.start_accel().unwrap(), StartAccel::Applied, "a driver nobody has written since the PC started");
    // saved OFF: never written, whatever the driver runs
    let mut off = with_driver((1, 7, 0), Some(SETTINGS));
    off.os_mut().byte_files = saved;
    off.load_accel().unwrap();
    off.accel_mut().panel.on = false;
    off.save_accel().unwrap();
    let mut left = with_driver((1, 7, 0), Some(SETTINGS));
    left.os_mut().byte_files = off.os().byte_files.clone();
    assert_eq!(left.start_accel().unwrap(), StartAccel::LeftAlone);
    assert_eq!(left.os().rawaccel_byte_writes, 0);
    // saved ON but no driver at all
    let mut none = Mouse::new(FakeOs::new(), AppDirs::new(r"C:\fake\appdata"));
    none.os_mut().byte_files = on.os().byte_files.clone();
    assert_eq!(none.start_accel().unwrap(), StartAccel::NoDriver);
}

#[test]
fn a_game_start_sets_the_driver_again_after_another_program_wrote_it() {
    let mut m = with_driver((1, 7, 0), Some(SETTINGS));
    m.mirror_rawaccel().unwrap();
    let (fast, _) = m.accel_mut().panel.save_as_preset();
    m.accel_mut().per_app.add_row("VALORANT-Win64-Shipping.exe", Target::Preset(fast));
    m.accel_mut().per_app.set_everywhere_else(Target::Off);
    m.sync_driver().unwrap();
    let writes = m.os().rawaccel_byte_writes;
    assert!(!m.sync_driver().unwrap(), "nothing changed → no write");
    // Raw Accel's own app opens and sends its settings.json (Off) to the driver
    m.os_mut().rawaccel_driver = bytes::to_bytes(&off_cfg());
    assert_eq!(m.driver_matches_card().unwrap(), Some(true), "everywhere else = Off = what it wrote");
    m.accel_app_event(&AppEvent::Started { pid: 5, exe: "VALORANT-Win64-Shipping.exe".into(), has_window: false });
    assert_eq!(m.driver_matches_card().unwrap(), Some(false));
    assert!(m.sync_driver().unwrap(), "the game started → the driver is set to the game's preset again");
    assert_eq!(m.os().rawaccel_byte_writes, writes + 1);
    assert_eq!(bytes::read_profiles(&m.os().rawaccel_driver)[0].accel_x.mode, AccelMode::Classic);
}

#[test]
fn another_writer_is_said_plainly() {
    let mut m = with_driver((1, 7, 0), Some(SETTINGS));
    m.mirror_rawaccel().unwrap();
    assert_eq!(m.other_writer_line().unwrap(), None, "the driver runs the card");
    // Raw Accel's app wrote Off after this app
    m.os_mut().rawaccel_driver = bytes::to_bytes(&off_cfg());
    let line = m.other_writer_line().unwrap().unwrap();
    assert!(line.starts_with("The driver is not running this card right now: another program wrote it"), "{line}");
    assert!(!line.contains("settings.json"), "no .config known → no claim about it");
    m.os_mut().files.insert(PathBuf::from(RA_DIR).join(".config"), r#"{"DPI":2400,"AutoWriteToDriverOnStartup":true}"#.into());
    assert!(m.other_writer_line().unwrap().unwrap().contains("sends its settings.json to the driver every time it opens"));
    // its app is open right now
    m.os_mut().running.push("rawaccel.exe".into());
    assert!(m.other_writer_line().unwrap().unwrap().starts_with("Your own Raw Accel app is open. It writes the driver too"));
    // card off + no app open: nothing to say even though the driver differs
    m.os_mut().running.clear();
    m.accel_mut().panel.on = false;
    assert_eq!(m.other_writer_line().unwrap(), None);
    // no driver → nothing
    assert_eq!(Mouse::new(FakeOs::new(), AppDirs::new("x")).other_writer_line().unwrap(), None);
}

#[test]
fn a_game_that_was_already_open_is_marked_late_until_its_next_launch() {
    let mut m = with_driver((1, 7, 0), Some(SETTINGS));
    m.mirror_rawaccel().unwrap();
    let (fast, _) = m.accel_mut().panel.save_as_preset();
    let row = m.accel_mut().per_app.add_row("VALORANT-Win64-Shipping.exe", Target::Preset(fast));
    assert!(!m.accel().per_app.is_late(row));
    assert!(!m.accel_app_event(&AppEvent::Started { pid: 9, exe: "VALORANT-Win64-Shipping.exe".into(), has_window: true }));
    assert!(m.accel().per_app.is_late(row) && !m.accel().per_app.is_active(row));
    assert!(m.accel_app_event(&AppEvent::Started { pid: 10, exe: "VALORANT-Win64-Shipping.exe".into(), has_window: false }));
    assert!(!m.accel().per_app.is_late(row) && m.accel().per_app.is_active(row));
}

/// "Same numbers in = same driver values out": every curve of the card, with its defaults and with each row at its
/// extremes, goes into the driver bytes and comes back from them as exactly the args Raw Accel's own GUI would have made —
/// and the sens multiplier is Raw Accel's "Output DPI" (× 1000) on the user's own profile.
#[test]
fn what_the_card_hands_over_is_what_the_driver_reads_back() {
    let base = his_cfg();
    let picks: [fn(&RowSpec) -> f64; 3] = [|r| r.default, |r| r.min, |r| r.max];
    for c in Curve::ALL {
        for pick in picks {
            let mut v = CurveValues::defaults(c);
            for r in rows(c) {
                v.values.insert(r.field, pick(&r));
            }
            if matches!(c, Curve::Linear | Curve::Classic) {
                let off = v.get(Field::InputOffset);
                if v.get(Field::CapInput) <= off {
                    v.values.insert(Field::CapInput, (off + 30.0).min(120.0));
                }
            }
            for sens in [0.1, 1.0, 1.37, 3.0] {
                let s = Setting { args: to_args(c, &v), sens };
                let mut cfg = base.clone();
                cfg.profiles[0] = s.apply_to(&base.profiles[0]);
                if !validate(&cfg.profiles[0]).is_empty() {
                    continue;
                }
                let raw = bytes::to_bytes(&cfg);
                let back = &bytes::read_profiles(&raw)[0];
                assert_eq!(back.accel_x, s.args, "{c:?} x");
                assert_eq!(back.accel_y, s.args, "{c:?} y (whole mode: same args)");
                assert_eq!(back.output_dpi, sens * 1000.0, "{c:?} sens {sens}");
                assert_eq!(back.domain_weights, base.profiles[0].domain_weights, "his own profile fields are kept");
            }
        }
    }
}

#[test]
fn putting_the_earlier_driver_state_back_also_switches_the_saved_card_off() {
    let mut m = with_driver((1, 7, 0), Some(SETTINGS));
    m.mirror_rawaccel().unwrap();
    let (fast, _) = m.accel_mut().panel.save_as_preset();
    m.accel_mut().per_app.add_row("VALORANT-Win64-Shipping.exe", Target::Preset(fast));
    m.save_accel().unwrap();
    // the kept earlier state: Off
    let before = m.os().rawaccel_driver.clone();
    let off = bytes::to_bytes(&off_cfg());
    m.os_mut().rawaccel_driver = off.clone();
    let kept = m.keep_driver_state(&off).unwrap();
    m.os_mut().rawaccel_driver = before; // the card's curve runs now
    // "a closed tab's service": nothing read yet
    let mut cold = with_driver((1, 7, 0), Some(SETTINGS));
    cold.os_mut().byte_files = m.os().byte_files.clone();
    cold.os_mut().rawaccel_driver = m.os().rawaccel_driver.clone();
    cold.restore_driver_state(&kept).unwrap();
    assert!(bytes::same_effect(&off, &cold.os().rawaccel_driver), "the earlier state is back");
    let mut next_start = with_driver((1, 7, 0), Some(SETTINGS));
    next_start.os_mut().byte_files = cold.os().byte_files.clone();
    next_start.os_mut().rawaccel_driver = cold.os().rawaccel_driver.clone();
    assert_eq!(next_start.start_accel().unwrap(), StartAccel::LeftAlone, "the next app start does not write the card's curve over it");
    assert_eq!(next_start.os().rawaccel_byte_writes, 0);
    assert_eq!(next_start.accel().panel.presets.len(), 2, "presets and games are kept");
    assert_eq!(next_start.accel().per_app.rows().len(), 1);
}

#[test]
fn a_game_row_with_no_game_chosen_is_not_kept() {
    let mut m = with_driver((1, 7, 0), Some(SETTINGS));
    m.accel_mut().per_app.add_row("", Target::Main);
    m.accel_mut().per_app.add_row("VALORANT-Win64-Shipping.exe", Target::Off);
    assert_eq!(m.accel().per_app.watched_names(), vec!["valorant-win64-shipping.exe".to_string()], "a blank row listens for nothing");
    m.save_accel().unwrap();
    let mut again = with_driver((1, 7, 0), Some(SETTINGS));
    again.os_mut().byte_files = m.os().byte_files.clone();
    again.load_accel().unwrap();
    assert_eq!(again.accel().per_app.rows().len(), 1);
    assert_eq!(m.accel().per_app.rows().len(), 2, "the open card keeps its blank row until a game is picked");
}

#[test]
fn a_damaged_saved_card_is_never_overwritten_and_an_unknown_driver_is_not_written_at_start() {
    let mut m = with_driver((1, 7, 0), Some(SETTINGS));
    let file = m.dirs().accel_file();
    m.os_mut().byte_files.insert(file.clone(), b"{ torn".to_vec());
    assert!(m.load_accel().is_err());
    m.accel_mut().panel.on = true;
    let e = m.save_accel().unwrap_err().to_string();
    assert!(e.contains("not overwritten"), "{e}");
    assert_eq!(m.os().byte_files.get(&file).map(|b| b.as_slice()), Some(&b"{ torn"[..]));
    // a good file again (deleted by the user): saving works
    m.os_mut().byte_files.remove(&file);
    assert!(!m.load_accel().unwrap());
    assert!(m.save_accel().unwrap());
    // a driver that is not a 1.7: its state can't be read, so nothing is written at start (no writer.exe on every start)
    let saved = m.os().byte_files.clone();
    let mut old = with_driver((1, 6, 0), Some(SETTINGS));
    old.os_mut().byte_files = saved;
    assert_eq!(old.start_accel().unwrap(), StartAccel::Unreadable);
    assert!(old.os().rawaccel_writes.is_empty() && old.os().rawaccel_byte_writes == 0);
}

#[test]
fn the_tab_counts_the_games_the_engine_says_run() {
    let mut m = with_driver((1, 7, 0), Some(SETTINGS));
    m.mirror_rawaccel().unwrap();
    let (fast, _) = m.accel_mut().panel.save_as_preset();
    let row = m.accel_mut().per_app.add_row("VALORANT-Win64-Shipping.exe", Target::Preset(fast));
    m.accel_mut().per_app.set_everywhere_else(Target::Off);
    m.accel_mut().per_app.set_active_rows(&[row]);
    assert!(m.accel().per_app.is_active(row));
    assert_eq!(m.accel().per_app.current(), Target::Preset(fast));
    m.accel_mut().per_app.set_active_rows(&[RowId(999)]);
    assert_eq!(m.accel().per_app.current(), Target::Off, "an unknown row counts for nothing");
}

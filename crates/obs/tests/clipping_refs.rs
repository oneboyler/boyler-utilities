//! The sounds are byte-for-byte ClipPing's: compared with the .wav files ClipPing itself wrote (its test renders,
//! C:\src\notifications-for-obs\tests\renders\sounds, read-only; at 100 % volume = -18 dBFS peak). Skipped where that
//! folder is missing (another PC).

use bu_obs::settings::Settings;
use bu_obs::sound::{build, opt_count, opt_name, Sound};

const DIR: &str = "C:/src/notifications-for-obs/tests/renders/sounds";

#[test]
fn every_built_in_sound_is_byte_identical_to_clippings() {
    if !std::path::Path::new(DIR).is_dir() {
        eprintln!("ClipPing's renders not here: skipped");
        return;
    }
    let mut n = 0;
    for (e, ev) in [Sound::Saved, Sound::Failed, Sound::Changed, Sound::Warning].into_iter().enumerate() {
        for i in 1..=opt_count(ev) as i32 {
            let mut s = Settings { vol: 100, ..Settings::default() };
            s.snd[e] = i;
            let (mine, _) = build(&s, ev, None).unwrap();
            let theirs = std::fs::read(format!("{DIR}/{e}_{i}_{}.wav", opt_name(ev, i))).unwrap();
            assert_eq!(mine.len(), theirs.len(), "{e}_{i}");
            let diff = mine.iter().zip(&theirs).filter(|(a, b)| a != b).count();
            assert_eq!(diff, 0, "{e}_{i} {}: {diff} bytes differ", opt_name(ev, i));
            n += 1;
        }
    }
    assert_eq!(n, 14);
}

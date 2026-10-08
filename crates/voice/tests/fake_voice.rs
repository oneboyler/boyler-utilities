//! Voice to text against the FAKE engine, fed a WAV this test writes into the lane's scratch folder (made in code: a quiet
//! part, then three "syllable" bursts). The microphone is never recorded; nothing on the PC changes; the clipboard is fake.

use std::path::PathBuf;
use std::time::Duration;

use bu_voice::fake::{FakeClipboard, FakeEngine};
use bu_voice::wav::{self, Pcm};
use bu_voice::{Dictation, Heard, Language, State, VoiceError};

/// The tests' own scratch folder (`%TEMP%\BoylerUtilities-test\voice-test`), never anywhere else.
fn scratch() -> PathBuf {
    let d = std::env::temp_dir().join("BoylerUtilities-test").join("voice-test");
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn speech_like_wav() -> Pcm {
    let rate = 16000u32;
    let mut s = vec![0i16; (rate / 5) as usize]; // 200 ms quiet
    for burst in 0..3 {
        let amp = 2500.0 + 1500.0 * burst as f32;
        s.extend((0..rate / 10).map(|i| ((i as f32 * 0.21).sin() * amp) as i16)); // 100 ms "syllable"
        s.extend(vec![0i16; (rate / 20) as usize]); // 50 ms gap
    }
    Pcm { rate, samples: s }
}

fn wait_end(d: &mut Dictation) -> Vec<f32> {
    let mut levels = Vec::new();
    for _ in 0..3000 {
        d.poll();
        levels.push(d.level());
        if matches!(d.state(), State::Idle | State::Done) {
            return levels;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("the fake never ended");
}

#[test]
fn a_scratch_wav_drives_the_level_and_the_words_arrive() {
    let path = scratch().join("speech-like.wav");
    std::fs::write(&path, wav::encode(&speech_like_wav())).unwrap();
    let eng = FakeEngine::from_wav(&path, vec![Heard::Guess("hello".into()), Heard::Sentence("Hello there.".into())])
        .unwrap()
        .with_pace(Duration::from_millis(3), false);
    let starts = eng.starts.clone();
    let (clip, got) = FakeClipboard::new();
    let mut d = Dictation::new(Box::new(eng), Box::new(clip));
    let en = d.languages().unwrap().remove(0);
    assert_eq!(en.tag, "en-US");
    d.toggle(&en).unwrap();
    assert_eq!(d.state(), State::Listening);
    let seen = wait_end(&mut d);
    assert_eq!(*starts.lock().unwrap(), 1);
    // the bursts reached the level while listening (the quiet part: 0), and the level is 0 once it stopped
    let peak = seen.iter().cloned().fold(0.0f32, f32::max);
    assert!(peak > 0.3, "peak level {peak}");
    assert_eq!(d.level(), 0.0);
    assert_eq!(d.state(), State::Done);
    assert_eq!(d.text(), "Hello there.");
    assert!(d.copy().unwrap());
    assert_eq!(got.lock().unwrap().as_str(), "Hello there.");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn stop_while_speaking_keeps_the_guess_and_the_key_toggles() {
    // a held (mic-like) fake: it listens until stopped
    let eng = FakeEngine::scripted(vec![Heard::Level(0.5), Heard::Guess("quick note".into())]).with_pace(Duration::from_millis(5), true);
    let (clip, _) = FakeClipboard::new();
    let mut d = Dictation::new(Box::new(eng), Box::new(clip));
    let en = d.languages().unwrap().remove(0);
    d.toggle(&en).unwrap();
    for _ in 0..200 {
        d.poll();
        if d.guess() == "quick note" {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(d.guess(), "quick note");
    assert!(d.listening());
    d.toggle(&en).unwrap(); // the key again = stop
    assert_eq!(d.state(), State::Finishing);
    wait_end(&mut d);
    assert_eq!(d.state(), State::Done);
    assert_eq!(d.firm(), "quick note");
}

#[test]
fn only_the_languages_windows_has_are_offered() {
    let langs = vec![
        Language { tag: "en-US".into(), name: "English (US)".into(), engine: "a".into() },
        Language { tag: "de-DE".into(), name: "Deutsch".into(), engine: "b".into() },
    ];
    let eng = FakeEngine::scripted(vec![]).with_languages(langs.clone());
    let (clip, _) = FakeClipboard::new();
    let mut d = Dictation::new(Box::new(eng), Box::new(clip));
    assert_eq!(d.languages().unwrap(), langs);
    let hr = Language { tag: "hr-HR".into(), name: "Hrvatski".into(), engine: "".into() };
    assert_eq!(d.start(&hr), Err(VoiceError::NoLanguage("hr-HR".into())));
    d.start(&langs[1]).unwrap();
    wait_end(&mut d);
    assert_eq!(d.state(), State::Idle); // nothing heard
}

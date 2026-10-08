//! Fakes for tests and the app's test copies: an engine that "hears" a script (optionally with the voice level of a WAV the
//! test wrote into its scratch folder), and a clipboard that is a string. Nothing here records, plays or touches Windows.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::error::{Result, VoiceError};
use crate::lang::Language;
use crate::{wav, Clipboard, Engine, Heard, Session};

/// Hears `script` (in order), then ends. `pace` = the pause between two events (0 in unit tests).
pub struct FakeEngine {
    langs: Vec<Language>,
    script: Vec<Heard>,
    pace: Duration,
    /// keep listening after the script until `stop` (like a real mic); false = end when the script is done
    hold: bool,
    /// what stops every dictation (a Windows switch off, no microphone): `check` and `start` return it; tests change it
    /// while the page is open (the user flipped the switch in Windows)
    pub blocked: Arc<Mutex<Option<VoiceError>>>,
    /// how many times `start` was called (tests)
    pub starts: Arc<Mutex<u32>>,
}

impl FakeEngine {
    /// English (US) only, like the PC this was built on.
    pub fn scripted(script: Vec<Heard>) -> FakeEngine {
        FakeEngine {
            langs: vec![Language { tag: "en-US".into(), name: "English (US)".into(), engine: "fake".into() }],
            script,
            pace: Duration::ZERO,
            hold: false,
            blocked: Arc::new(Mutex::new(None)),
            starts: Arc::new(Mutex::new(0)),
        }
    }

    /// The drawing's made-up recording (its VSAY sentences, word by word), paced like speech, held until stopped.
    pub fn drawing() -> FakeEngine {
        const SAY: [&str; 4] = [
            "Hey, are you on tonight?",
            "I was thinking we queue a few ranked games around nine.",
            "Bring your headset, your mic was really quiet last time.",
            "And send me that clip from yesterday, the triple kill with the Operator.",
        ];
        let mut script = Vec::new();
        for s in SAY {
            let words: Vec<&str> = s.split(' ').collect();
            for i in 1..=words.len() {
                script.push(Heard::Level(0.38 + 0.11 * (i % 5) as f32));
                script.push(Heard::Guess(words[..i].join(" ")));
            }
            script.push(Heard::Level(0.0));
            script.push(Heard::Sentence(s.to_string()));
        }
        FakeEngine { pace: Duration::from_millis(160), hold: true, ..FakeEngine::scripted(script) }
    }

    /// Feeds a WAV (written by the test into its scratch folder) as the voice level, 50 ms slices, then the script.
    pub fn from_wav(path: &std::path::Path, script: Vec<Heard>) -> Result<FakeEngine> {
        let bytes = std::fs::read(path).map_err(|e| VoiceError::File(format!("{}: {e}", path.display())))?;
        let pcm = wav::decode(&bytes)?;
        let mut all: Vec<Heard> = wav::levels(&pcm, 50).into_iter().map(Heard::Level).collect();
        all.extend(script);
        Ok(FakeEngine::scripted(all))
    }

    /// Other languages (to test the picker); the first one is the default.
    pub fn with_languages(mut self, langs: Vec<Language>) -> FakeEngine {
        self.langs = langs;
        self
    }
    pub fn with_pace(mut self, pace: Duration, hold: bool) -> FakeEngine {
        self.pace = pace;
        self.hold = hold;
        self
    }

    /// Like Windows with "Online speech recognition" off / the microphone blocked / no microphone (`e`).
    pub fn with_blocker(self, e: Option<VoiceError>) -> FakeEngine {
        *self.blocked.lock().unwrap() = e;
        self
    }
}

struct FakeSession {
    stop: Arc<AtomicBool>,
}

impl Session for FakeSession {
    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Engine for FakeEngine {
    fn languages(&self) -> Result<Vec<Language>> {
        Ok(self.langs.clone())
    }

    fn check(&self) -> Result<()> {
        self.blocked.lock().unwrap().clone().map_or(Ok(()), Err)
    }

    fn start(&mut self, lang: &Language, out: Sender<Heard>) -> Result<Box<dyn Session>> {
        if !self.langs.iter().any(|l| l.tag == lang.tag) {
            return Err(VoiceError::NoLanguage(lang.tag.clone()));
        }
        self.check()?;
        *self.starts.lock().unwrap() += 1;
        let stop = Arc::new(AtomicBool::new(false));
        let (script, pace, hold, s2) = (self.script.clone(), self.pace, self.hold, stop.clone());
        std::thread::spawn(move || {
            for h in script {
                if s2.load(Ordering::SeqCst) {
                    break;
                }
                let end = matches!(h, Heard::Failed(_));
                if out.send(h).is_err() || end {
                    return;
                }
                if !pace.is_zero() {
                    std::thread::sleep(pace);
                }
            }
            while hold && !s2.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(20));
            }
            let _ = out.send(Heard::Ended);
        });
        Ok(Box::new(FakeSession { stop }))
    }
}

/// A clipboard that is a string (`new` also returns the string to look at).
pub struct FakeClipboard {
    text: Arc<Mutex<String>>,
}

impl FakeClipboard {
    pub fn new() -> (FakeClipboard, Arc<Mutex<String>>) {
        let text = Arc::new(Mutex::new(String::new()));
        (FakeClipboard { text: text.clone() }, text)
    }
}

impl Clipboard for FakeClipboard {
    fn set_text(&mut self, text: &str) -> Result<()> {
        *self.text.lock().unwrap() = text.to_string();
        Ok(())
    }
}

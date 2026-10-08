//! `bu-voice` — Voice to text (menu-v22 page `vtt`), no UI, no keys.
//!
//! * [`Dictation`] — the page's whole state: start / stop listening, the words as they come (the firm ones and the newest,
//!   still-guessed ones Windows firms up at the end of each sentence), the voice level for the pulsing mic, clear, copy,
//!   fixing a word once stopped. The app's keys manager maps the Voice to text key to [`Dictation::toggle`].
//! * The engine is **Windows' online dictation** (Order 043; the owner: "Win+H perfect, ours wrong" with the old on-device SAPI
//!   recognizer): `Windows.Media.SpeechRecognition` with the dictation topic, a continuous session on the default microphone
//!   — Microsoft's online speech service, the one Windows' voice typing (Win+H) uses, behind the same "Online speech
//!   recognition" switch. [`Engine::languages`] lists ONLY the dictation languages Windows offers on this PC; [`Engine::check`]
//!   says (read-only) whether a Windows switch or a missing microphone stops it.
//! * [`Clipboard`] — Copy (real: the Windows clipboard; fake: a string).
//!
//! Every Windows call goes through the [`Engine`] / [`Clipboard`] traits: real ones in `real` (Windows) and fakes in [`fake`]
//! (tests). Tests never record the microphone: the fake engine is fed a WAV the test writes into its scratch folder.

mod error;
pub mod fake;
pub mod lang;
#[cfg(windows)]
pub mod real;
pub mod wav;

pub use error::{Result, VoiceError};
pub use lang::Language;

use std::sync::mpsc::{Receiver, Sender, TryRecvError};

/// Windows Settings › Time & language › Speech ("More languages…").
pub const SETTINGS_SPEECH: &str = "ms-settings:speech";
/// Windows Settings › Privacy & security › Speech ("Online speech recognition").
pub const SETTINGS_PRIVACY_SPEECH: &str = "ms-settings:privacy-speech";
/// Windows Settings › Privacy & security › Microphone.
pub const SETTINGS_PRIVACY_MIC: &str = "ms-settings:privacy-microphone";

/// What the engine hears, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Heard {
    /// the voice level now, 0..1 (the mic pulses with it)
    Level(f32),
    /// the sentence being spoken, as Windows guesses it so far (replaces the previous guess)
    Guess(String),
    /// Windows firmed a sentence up (replaces the guess)
    Sentence(String),
    /// the engine stopped (after `stop`, or the input ended)
    Ended,
    /// the engine could not go on
    Failed(VoiceError),
}

/// A running dictation; `stop` asks it to end (the last sentence still arrives, then `Heard::Ended`).
pub trait Session: Send {
    fn stop(&mut self);
}

/// Windows' speech recognition (or a fake).
pub trait Engine: Send {
    /// The languages Windows can take dictation in on this PC (read-only, quick, no audio).
    fn languages(&self) -> Result<Vec<Language>>;
    /// Read-only, quick, no audio: Ok, or what stops every dictation now (`OnlineOff`, `MicDenied`, `NoMic`).
    fn check(&self) -> Result<()>;
    /// Start listening in `lang`; what is heard goes to `out`.
    fn start(&mut self, lang: &Language, out: Sender<Heard>) -> Result<Box<dyn Session>>;
}

/// Where Copy puts the words.
pub trait Clipboard: Send {
    fn set_text(&mut self, text: &str) -> Result<()>;
}

/// The page's state (the drawing's `V.st`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// nothing said yet (or cleared)
    Idle,
    /// the mic is on
    Listening,
    /// stopped, the last sentence is still coming (≤ a second)
    Finishing,
    /// stopped with words: click the text to fix a word
    Done,
}

/// Voice to text: one dictation at a time.
pub struct Dictation {
    engine: Box<dyn Engine>,
    clip: Box<dyn Clipboard>,
    session: Option<Box<dyn Session>>,
    rx: Option<Receiver<Heard>>,
    state: State,
    /// the firm words (editable once stopped)
    firm: String,
    /// the newest, still-guessed words of the sentence being spoken
    guess: String,
    level: f32,
    error: Option<VoiceError>,
}

impl Dictation {
    pub fn new(engine: Box<dyn Engine>, clip: Box<dyn Clipboard>) -> Dictation {
        Dictation { engine, clip, session: None, rx: None, state: State::Idle, firm: String::new(), guess: String::new(), level: 0.0, error: None }
    }

    /// The languages to offer (only what works on this PC).
    pub fn languages(&self) -> Result<Vec<Language>> {
        self.engine.languages()
    }

    /// What stops every dictation now (a Windows switch, no microphone), read-only; Ok = ready.
    pub fn check(&self) -> Result<()> {
        self.engine.check()
    }

    pub fn state(&self) -> State {
        self.state
    }
    pub fn listening(&self) -> bool {
        self.state == State::Listening
    }
    /// The firm words.
    pub fn firm(&self) -> &str {
        &self.firm
    }
    /// The newest guessed words (shown lighter).
    pub fn guess(&self) -> &str {
        &self.guess
    }
    /// Everything, as Copy takes it (single spaces, trimmed).
    pub fn text(&self) -> String {
        let all = format!("{} {}", self.firm, self.guess);
        all.split_whitespace().collect::<Vec<_>>().join(" ")
    }
    pub fn words(&self) -> usize {
        self.text().split_whitespace().count()
    }
    pub fn level(&self) -> f32 {
        self.level
    }
    /// The last engine error (shown as a toast by the page), taken once.
    pub fn take_error(&mut self) -> Option<VoiceError> {
        self.error.take()
    }

    /// Start listening (talking again adds on to the words already there).
    pub fn start(&mut self, lang: &Language) -> Result<()> {
        if matches!(self.state, State::Listening | State::Finishing) {
            return Err(VoiceError::Busy);
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let session = self.engine.start(lang, tx)?;
        if !self.firm.is_empty() && !self.firm.ends_with(' ') {
            self.firm.push(' ');
        }
        self.session = Some(session);
        self.rx = Some(rx);
        self.guess.clear();
        self.state = State::Listening;
        Ok(())
    }

    /// Stop listening; the last sentence still arrives through `poll`.
    pub fn stop(&mut self) {
        if self.state != State::Listening {
            return;
        }
        if let Some(s) = self.session.as_mut() {
            s.stop();
        }
        self.state = State::Finishing;
        self.level = 0.0;
    }

    /// The key / the mic button.
    pub fn toggle(&mut self, lang: &Language) -> Result<()> {
        match self.state {
            State::Listening => {
                self.stop();
                Ok(())
            }
            State::Finishing => Ok(()),
            _ => self.start(lang),
        }
    }

    /// Take what the engine heard since the last call. True = something changed (repaint).
    pub fn poll(&mut self) -> bool {
        let Some(rx) = self.rx.as_ref() else { return false };
        let mut changed = false;
        let mut ended = false;
        loop {
            match rx.try_recv() {
                Ok(h) => {
                    changed = true;
                    match h {
                        Heard::Level(l) => {
                            if self.state == State::Listening {
                                self.level = l.clamp(0.0, 1.0);
                            }
                        }
                        Heard::Guess(g) => self.guess = g.trim().to_string(),
                        Heard::Sentence(s) => {
                            self.guess.clear();
                            let s = s.trim();
                            if !s.is_empty() {
                                if !self.firm.is_empty() && !self.firm.ends_with(' ') {
                                    self.firm.push(' ');
                                }
                                self.firm.push_str(s);
                                self.firm.push(' ');
                            }
                        }
                        Heard::Ended => {
                            ended = true;
                            break;
                        }
                        Heard::Failed(e) => {
                            self.error = Some(e);
                            ended = true;
                            break;
                        }
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    ended = true;
                    changed = true;
                    break;
                }
            }
        }
        if ended {
            self.finish();
        }
        changed
    }

    /// The engine is gone: what was still a guess becomes firm (the drawing's vStop), state Done / Idle.
    fn finish(&mut self) {
        self.session = None;
        self.rx = None;
        if !self.guess.trim().is_empty() {
            if !self.firm.is_empty() && !self.firm.ends_with(' ') {
                self.firm.push(' ');
            }
            self.firm.push_str(self.guess.trim());
        }
        self.guess.clear();
        self.firm = self.firm.trim_end().to_string();
        self.level = 0.0;
        self.state = if self.firm.is_empty() { State::Idle } else { State::Done };
    }

    /// Clear: no words (listening goes on).
    pub fn clear(&mut self) {
        self.firm.clear();
        self.guess.clear();
        if self.state == State::Done {
            self.state = State::Idle;
        }
    }

    /// Copy the words to the clipboard. False = nothing to copy.
    pub fn copy(&mut self) -> Result<bool> {
        let t = self.text();
        if t.is_empty() {
            return Ok(false);
        }
        self.clip.set_text(&t)?;
        Ok(true)
    }

    /// Fixing a word once stopped: the page edits the firm text (only in Done / Idle).
    pub fn set_text(&mut self, text: &str) {
        if matches!(self.state, State::Done | State::Idle) {
            self.firm = text.to_string();
            self.state = if self.firm.trim().is_empty() { State::Idle } else { State::Done };
        }
    }
}

impl Drop for Dictation {
    fn drop(&mut self) {
        if let Some(s) = self.session.as_mut() {
            s.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{FakeClipboard, FakeEngine};

    fn en() -> Language {
        Language { tag: "en-US".into(), name: "English (US)".into(), engine: "fake".into() }
    }

    fn drain(d: &mut Dictation) {
        for _ in 0..2000 {
            d.poll();
            if matches!(d.state(), State::Idle | State::Done) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("the fake never ended");
    }

    #[test]
    fn sentences_become_firm_and_the_last_guess_too_on_stop() {
        let eng = FakeEngine::scripted(vec![
            Heard::Guess("hey are".into()),
            Heard::Guess("hey are you on".into()),
            Heard::Sentence("Hey, are you on tonight?".into()),
            Heard::Guess("bring your".into()),
        ]);
        let (clip, got) = FakeClipboard::new();
        let mut d = Dictation::new(Box::new(eng), Box::new(clip));
        d.start(&en()).unwrap();
        assert!(d.listening());
        drain(&mut d);
        assert_eq!(d.state(), State::Done);
        assert_eq!(d.text(), "Hey, are you on tonight? bring your");
        assert!(d.copy().unwrap());
        assert_eq!(got.lock().unwrap().as_str(), "Hey, are you on tonight? bring your");
        d.clear();
        assert_eq!(d.state(), State::Idle);
        assert!(!d.copy().unwrap());
    }

    #[test]
    fn talking_again_adds_on_and_a_second_start_is_refused() {
        let eng = FakeEngine::scripted(vec![Heard::Sentence("one".into())]);
        let (clip, _) = FakeClipboard::new();
        let mut d = Dictation::new(Box::new(eng), Box::new(clip));
        d.start(&en()).unwrap();
        assert_eq!(d.start(&en()), Err(VoiceError::Busy));
        drain(&mut d);
        d.start(&en()).unwrap();
        drain(&mut d);
        assert_eq!(d.text(), "one one");
        d.set_text("one two");
        assert_eq!(d.text(), "one two");
    }

    #[test]
    fn an_engine_error_ends_listening_and_is_reported_once() {
        let eng = FakeEngine::scripted(vec![Heard::Guess("half".into()), Heard::Failed(VoiceError::NoMic)]);
        let (clip, _) = FakeClipboard::new();
        let mut d = Dictation::new(Box::new(eng), Box::new(clip));
        d.start(&en()).unwrap();
        drain(&mut d);
        assert_eq!(d.take_error(), Some(VoiceError::NoMic));
        assert_eq!(d.take_error(), None);
        assert_eq!(d.text(), "half");
    }

    #[test]
    fn a_language_windows_does_not_have_is_refused() {
        let eng = FakeEngine::scripted(vec![]);
        let (clip, _) = FakeClipboard::new();
        let mut d = Dictation::new(Box::new(eng), Box::new(clip));
        let xx = Language { tag: "hr-HR".into(), name: "Hrvatski".into(), engine: "none".into() };
        assert!(matches!(d.start(&xx), Err(VoiceError::NoLanguage(_))));
        assert_eq!(d.state(), State::Idle);
    }

    /// Order 043: "Online speech recognition" off, the microphone blocked, no microphone - `check` says it before any click,
    /// `start` refuses, and once the user changed it in Windows the next start works (nothing restarts the page).
    #[test]
    fn a_windows_switch_or_no_mic_blocks_until_changed() {
        for e in [VoiceError::OnlineOff, VoiceError::MicDenied, VoiceError::NoMic] {
            let eng = FakeEngine::scripted(vec![Heard::Sentence("back on".into())]).with_blocker(Some(e.clone()));
            let blocked = eng.blocked.clone();
            let starts = eng.starts.clone();
            let (clip, _) = FakeClipboard::new();
            let mut d = Dictation::new(Box::new(eng), Box::new(clip));
            assert!(e.blocks());
            assert_eq!(d.check(), Err(e.clone()));
            assert_eq!(d.start(&en()), Err(e.clone()));
            assert_eq!(d.toggle(&en()), Err(e.clone()));
            assert_eq!(d.state(), State::Idle);
            assert_eq!(*starts.lock().unwrap(), 0, "nothing listened");
            *blocked.lock().unwrap() = None;
            assert_eq!(d.check(), Ok(()));
            d.start(&en()).unwrap();
            drain(&mut d);
            assert_eq!(d.text(), "back on");
        }
    }

    /// The service refusing once the session runs (0x80045509 at start, the network gone, the mic taken away) ends the
    /// dictation, keeps what was heard and reports why once.
    #[test]
    fn the_service_refusing_while_listening_reports_why() {
        for e in [VoiceError::OnlineOff, VoiceError::Offline, VoiceError::MicDenied, VoiceError::NoMic] {
            let eng = FakeEngine::scripted(vec![Heard::Guess("half a".into()), Heard::Failed(e.clone())]);
            let (clip, _) = FakeClipboard::new();
            let mut d = Dictation::new(Box::new(eng), Box::new(clip));
            d.start(&en()).unwrap();
            drain(&mut d);
            assert_eq!(d.take_error(), Some(e));
            assert_eq!(d.take_error(), None);
            assert_eq!(d.text(), "half a");
            assert_eq!(d.state(), State::Done);
        }
    }

    /// A sentence Windows rejects (noise) arrives empty: its guess goes, nothing is added.
    #[test]
    fn a_rejected_sentence_clears_its_guess() {
        let eng = FakeEngine::scripted(vec![Heard::Sentence("Hello.".into()), Heard::Guess("uh".into()), Heard::Sentence(String::new())]);
        let (clip, _) = FakeClipboard::new();
        let mut d = Dictation::new(Box::new(eng), Box::new(clip));
        d.start(&en()).unwrap();
        drain(&mut d);
        assert_eq!(d.text(), "Hello.");
    }

    #[test]
    fn windows_codes_become_the_three_blockers() {
        use crate::error::from_code;
        assert_eq!(from_code("x", 0x8004_5509), VoiceError::OnlineOff);
        assert_eq!(from_code("x", 0x8007_0005), VoiceError::MicDenied);
        assert_eq!(from_code("x", 0xC00D_ABE0), VoiceError::NoMic);
        assert_eq!(from_code("starting", 0x8000_4005), VoiceError::Os { context: "starting".into(), code: 0x8000_4005 });
        assert!(!VoiceError::Offline.blocks());
        assert!(!from_code("x", 1).blocks());
    }
}

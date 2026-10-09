//! Fakes for tests and the app's test copies: a voice typing that counts the shortcuts it would have sent, and a clipboard that
//! is a string. Nothing here sends a key, opens a window or touches Windows.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::error::Result;
use crate::{send_if_ours, Clipboard, VoiceTyping};

/// Counts the Win+H shortcuts that would go out. `ours` = whether the foreground window is one of ours (tests flip it).
pub struct FakeVoiceTyping {
    pub ours: Arc<AtomicBool>,
    pub sent: Arc<AtomicU32>,
}

impl FakeVoiceTyping {
    /// (the fake, its "foreground is ours" switch, its sent counter) - our window in front at first.
    pub fn new() -> (FakeVoiceTyping, Arc<AtomicBool>, Arc<AtomicU32>) {
        let (ours, sent) = (Arc::new(AtomicBool::new(true)), Arc::new(AtomicU32::new(0)));
        (FakeVoiceTyping { ours: ours.clone(), sent: sent.clone() }, ours, sent)
    }
}

impl VoiceTyping for FakeVoiceTyping {
    fn toggle(&mut self) -> Result<bool> {
        let sent = self.sent.clone();
        send_if_ours(self.ours.load(Ordering::SeqCst), move || {
            sent.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

/// The clipboard as a string the test can read.
pub struct FakeClipboard(Arc<Mutex<String>>);

impl FakeClipboard {
    pub fn new() -> (FakeClipboard, Arc<Mutex<String>>) {
        let s = Arc::new(Mutex::new(String::new()));
        (FakeClipboard(s.clone()), s)
    }
}

impl Clipboard for FakeClipboard {
    fn set_text(&mut self, text: &str) -> Result<()> {
        *self.0.lock().unwrap() = text.to_string();
        Ok(())
    }
}

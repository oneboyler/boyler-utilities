//! `bu-voice` — Voice to text (menu-v22 page `vtt`), no UI, no keys of its own.
//!
//! * [`VoiceTyping`] — **Windows' own voice typing** (Win+H; Order 060, the owner: "why does voice to text asks for some online speech
//!   recognition shit to be turned on in settings? ... i dont even have that setting on yet i can use win + h"). Win+H has its own
//!   permission, so no privacy switch of ours is needed and the page asks about none. `toggle` sends the shortcut with
//!   `SendInput` **only while one of OUR OWN windows is the foreground window** (see [`send_if_ours`]) — never to another app and
//!   never to a game. The words land in our focused text box as ordinary typed characters.
//! * [`Clipboard`] — Copy (real: the Windows clipboard; fake: a string).
//!
//! Every Windows call goes through the [`VoiceTyping`] / [`Clipboard`] traits: real ones in `real` (Windows) and fakes in [`fake`]
//! (tests send no key and touch no window). The old `Windows.Media.SpeechRecognition` dictation path (needed the "Online speech
//! recognition" privacy switch) is removed, not kept as a fallback: Win+H exists on every Windows 10/11 this app runs on.

mod error;
pub mod fake;
#[cfg(windows)]
pub mod real;

pub use error::{Result, VoiceError};

/// Windows' voice typing (or a fake).
pub trait VoiceTyping: Send {
    /// Open Windows' voice typing for the focused text box of our own window (pressed again, Windows closes it). `Ok(true)` =
    /// the shortcut was sent; `Ok(false)` = refused because the foreground window is not one of ours (nothing was sent).
    fn toggle(&mut self) -> Result<bool>;
}

/// Where Copy puts the words.
pub trait Clipboard: Send {
    fn set_text(&mut self, text: &str) -> Result<()>;
}

/// The one rule of the shortcut: `send` runs only when the foreground window belongs to this app. `Ok(false)` = not sent.
pub fn send_if_ours(foreground_is_ours: bool, send: impl FnOnce() -> Result<()>) -> Result<bool> {
    if !foreground_is_ours {
        return Ok(false);
    }
    send()?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shortcut_goes_out_only_for_our_own_foreground_window() {
        let mut sent = 0;
        assert_eq!(send_if_ours(false, || { sent += 1; Ok(()) }), Ok(false));
        assert_eq!(sent, 0, "another app (or a game) in front: nothing sent");
        assert_eq!(send_if_ours(true, || { sent += 1; Ok(()) }), Ok(true));
        assert_eq!(sent, 1);
    }

    #[test]
    fn a_failing_send_is_an_error_not_a_silent_success() {
        let e = VoiceError::Os { context: "SendInput".into(), code: 5 };
        assert_eq!(send_if_ours(true, || Err(e.clone())), Err(e));
    }
}

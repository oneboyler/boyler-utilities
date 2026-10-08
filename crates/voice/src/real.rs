//! The real Windows layer (Order 043): Windows' online dictation - `Windows.Media.SpeechRecognition`'s `SpeechRecognizer` with
//! the dictation topic constraint and a continuous recognition session on the default microphone. That is Microsoft's online
//! speech service, the one Windows' own voice typing (Win+H) uses, behind the same switch: Settings › Privacy & security ›
//! Speech › "Online speech recognition". It works from our unpackaged exe (no package identity); the microphone follows
//! Windows' "Let desktop apps access your microphone". The words as they come: `HypothesisGenerated` (the sentence so far)
//! and `ResultGenerated` (the sentence firmed up). The mic's pulse: the default microphone's peak meter
//! (`IAudioMeterInformation`, read only - no second recording).
//!
//! The old on-device SAPI recognizer (Order 024) is gone: it got the owner's words wrong ("Win+H perfect, ours wrong", test
//! build 2, Oct 8). Tests never use this layer (they use `fake`); `examples/show.rs` only creates the recognizer and compiles
//! the dictation constraint - it never listens.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Foundation::{TimeSpan, TypedEventHandler};
use windows::Globalization::Language as WinLanguage;
use windows::Media::SpeechRecognition::{
    SpeechContinuousRecognitionCompletedEventArgs, SpeechContinuousRecognitionResultGeneratedEventArgs, SpeechContinuousRecognitionSession,
    SpeechRecognitionConfidence, SpeechRecognitionHypothesisGeneratedEventArgs, SpeechRecognitionResultStatus, SpeechRecognitionScenario,
    SpeechRecognitionTopicConstraint, SpeechRecognizer,
};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::{eCapture, eConsole, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{CoCreateInstance, CoIncrementMTAUsage, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::Registry::{RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::error::{os, Result, VoiceError};
use crate::lang::{short_name, Language};
use crate::{Clipboard, Engine, Heard, Session};

/// Settings › Privacy & security › Speech › "Online speech recognition" (`HasAccepted` 1 = on, 0 = off, missing = never chosen).
const ONLINE_KEY: &str = r"Software\Microsoft\Speech_OneCore\Settings\OnlineSpeechPrivacy";
/// Settings › Privacy & security › Microphone (`Value` "Allow" / "Deny"; HKLM = the whole PC, HKCU = this user,
/// `NonPackaged` = "Let desktop apps access your microphone").
const MIC_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";
/// Silence this long ends the session by itself (Windows' default is 20 s, which would cut a dictation at a pause; the mic
/// button / the key stops it normally).
const AUTO_STOP: Duration = Duration::from_secs(5 * 60);

/// Windows' online dictation.
#[derive(Default)]
pub struct WinSpeech;

impl WinSpeech {
    pub fn new() -> WinSpeech {
        WinSpeech
    }
}

/// COM on this thread for the scope (MTA; harmless if it already is). The process's MTA is kept alive once for good
/// (`CoIncrementMTAUsage`): the windows crate caches WinRT factories process-wide, and a thread's CoUninitialize tearing the
/// MTA down would leave the cached SpeechRecognizer factory dead - the next call crashed (measured: the show example
/// segfaulted on its second WinRT call, Oct 8).
struct Com(bool);
impl Com {
    fn init() -> Com {
        static KEEP_MTA: std::sync::Once = std::sync::Once::new();
        KEEP_MTA.call_once(|| {
            let _ = unsafe { CoIncrementMTAUsage() };
        });
        Com(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// A REG_DWORD value, read-only.
fn reg_dword(root: HKEY, key: &str, value: &str) -> Option<u32> {
    let (k, v) = (wide(key), wide(value));
    let mut d = 0u32;
    let mut len = 4u32;
    // SAFETY: RegGetValueW writes at most `len` (4) bytes into `d`.
    let r = unsafe { RegGetValueW(root, PCWSTR(k.as_ptr()), PCWSTR(v.as_ptr()), RRF_RT_REG_DWORD, None, Some(&mut d as *mut u32 as *mut _), Some(&mut len)) };
    r.is_ok().then_some(d)
}

/// A REG_SZ value, read-only.
fn reg_sz(root: HKEY, key: &str, value: &str) -> Option<String> {
    let (k, v) = (wide(key), wide(value));
    let mut buf = vec![0u16; 256];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: RegGetValueW writes at most `len` bytes into `buf`.
    let r = unsafe { RegGetValueW(root, PCWSTR(k.as_ptr()), PCWSTR(v.as_ptr()), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len)) };
    if r.is_err() {
        return None;
    }
    let n = (len as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..n.min(buf.len())]))
}

/// Windows' default recording device (the one the recognizer listens to).
fn default_mic() -> Result<IMMDevice> {
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| os("the audio devices", e))?;
        en.GetDefaultAudioEndpoint(eCapture, eConsole).map_err(|_| VoiceError::NoMic)
    }
}

/// What the switches say, read-only: "Online speech recognition" off / the microphone blocked for desktop apps.
pub fn switches() -> (Option<u32>, Vec<(&'static str, String)>) {
    let online = reg_dword(HKEY_CURRENT_USER, ONLINE_KEY, "HasAccepted");
    let np = format!(r"{MIC_KEY}\NonPackaged");
    let mic = [("PC", HKEY_LOCAL_MACHINE, MIC_KEY), ("user", HKEY_CURRENT_USER, MIC_KEY), ("desktop apps", HKEY_CURRENT_USER, np.as_str())]
        .into_iter()
        .filter_map(|(n, root, k)| reg_sz(root, k, "Value").map(|v| (n, v)))
        .collect();
    (online, mic)
}

/// A dictation status Windows reports -> Ok (it worked / it was stopped) or why not.
fn status(s: SpeechRecognitionResultStatus, tag: &str) -> Result<()> {
    match s {
        SpeechRecognitionResultStatus::Success | SpeechRecognitionResultStatus::UserCanceled | SpeechRecognitionResultStatus::TimeoutExceeded => Ok(()),
        SpeechRecognitionResultStatus::TopicLanguageNotSupported => Err(VoiceError::NoLanguage(tag.to_string())),
        SpeechRecognitionResultStatus::NetworkFailure => Err(VoiceError::Offline),
        SpeechRecognitionResultStatus::MicrophoneUnavailable => Err(VoiceError::NoMic),
        s => Err(VoiceError::Os { context: format!("dictation (status {})", s.0), code: s.0 as u32 }),
    }
}

/// A recognizer for `lang` with the dictation topic, compiled (no audio yet).
fn recognizer(lang: &Language) -> Result<SpeechRecognizer> {
    let wl = WinLanguage::CreateLanguage(&HSTRING::from(lang.tag.as_str())).map_err(|e| os("the language", e))?;
    let reco = SpeechRecognizer::Create(&wl).map_err(|e| os("the speech recognizer", e))?;
    let topic = SpeechRecognitionTopicConstraint::Create(SpeechRecognitionScenario::Dictation, &HSTRING::from("dictation")).map_err(|e| os("the dictation topic", e))?;
    reco.Constraints().and_then(|c| c.Append(&topic)).map_err(|e| os("adding the dictation topic", e))?;
    let comp = reco.CompileConstraintsAsync().and_then(|op| op.join()).map_err(|e| os("preparing dictation", e))?;
    status(comp.Status().map_err(|e| os("preparing dictation", e))?, &lang.tag)?;
    Ok(reco)
}

/// The read-only proof (`examples/show.rs`): create the recognizer for `lang` and compile the dictation constraint - never
/// listens. Ok = the language the recognizer took.
pub fn prepare(lang: &Language) -> Result<String> {
    let _com = Com::init();
    let reco = recognizer(lang)?;
    let tag = reco.CurrentLanguage().and_then(|l| l.LanguageTag()).map(|t| t.to_string()).unwrap_or_default();
    let _ = reco.Close();
    Ok(tag)
}

/// Windows' speech language (Settings › Time & language › Speech), read-only.
pub fn system_language() -> Option<String> {
    let _com = Com::init();
    SpeechRecognizer::SystemSpeechLanguage().and_then(|l| l.LanguageTag()).map(|t| t.to_string()).ok()
}

impl Engine for WinSpeech {
    fn languages(&self) -> Result<Vec<Language>> {
        let _com = Com::init();
        let list = SpeechRecognizer::SupportedTopicLanguages().map_err(|e| os("listing the dictation languages", e))?;
        let n = list.Size().map_err(|e| os("listing the dictation languages", e))?;
        let mut out: Vec<Language> = Vec::new();
        for i in 0..n {
            let Ok(l) = list.GetAt(i) else { continue };
            let Ok(tag) = l.LanguageTag() else { continue };
            let tag = tag.to_string();
            if out.iter().any(|x| x.tag.eq_ignore_ascii_case(&tag)) {
                continue;
            }
            let mut name = short_name(&tag);
            if name == tag {
                if let Ok(nn) = l.NativeName() {
                    if !nn.is_empty() {
                        name = nn.to_string();
                    }
                }
            }
            out.push(Language { tag, name, engine: "online".into() });
        }
        // Windows' speech language first: the page's default
        if let Ok(sys) = SpeechRecognizer::SystemSpeechLanguage().and_then(|l| l.LanguageTag()) {
            let sys = sys.to_string();
            if let Some(p) = out.iter().position(|l| l.tag.eq_ignore_ascii_case(&sys)) {
                let l = out.remove(p);
                out.insert(0, l);
            }
        }
        Ok(out)
    }

    fn check(&self) -> Result<()> {
        let (online, mic) = switches();
        // 0 = switched off; missing = never chosen - then the service's own answer at start decides (0x80045509 = off)
        if online == Some(0) {
            return Err(VoiceError::OnlineOff);
        }
        if mic.iter().any(|(_, v)| v.eq_ignore_ascii_case("Deny")) {
            return Err(VoiceError::MicDenied);
        }
        let _com = Com::init();
        default_mic().map(|_| ())
    }

    fn start(&mut self, lang: &Language, out: Sender<Heard>) -> Result<Box<dyn Session>> {
        // returns at once: preparing the online dictation takes a moment and the mic click runs on the UI thread. A failure
        // to open arrives as `Heard::Failed`.
        let stop = Arc::new(AtomicBool::new(false));
        let (s2, want) = (stop.clone(), lang.clone());
        std::thread::Builder::new()
            .name("bu-voice".into())
            .spawn(move || {
                let _com = Com::init();
                match open(&want, &out) {
                    Err(e) => {
                        let _ = out.send(Heard::Failed(e));
                    }
                    Ok(rig) => listen(rig, &want.tag, &s2, &out),
                }
            })
            .map_err(|e| VoiceError::Os { context: format!("starting the speech thread: {e}"), code: 0 })?;
        Ok(Box::new(WinSession { stop }))
    }
}

struct WinSession {
    stop: Arc<AtomicBool>,
}

impl Session for WinSession {
    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

struct Rig {
    reco: SpeechRecognizer,
    sess: SpeechContinuousRecognitionSession,
    /// the session's Completed status (it ended: stopped by us, or by Windows - silence, network, the mic gone)
    done: Receiver<SpeechRecognitionResultStatus>,
    meter: Option<IAudioMeterInformation>,
    hooks: (i64, i64, i64),
}

/// The recognizer listening on the default microphone; guesses and sentences go straight to `out`.
fn open(lang: &Language, out: &Sender<Heard>) -> Result<Rig> {
    let reco = recognizer(lang)?;
    let sess = reco.ContinuousRecognitionSession().map_err(|e| os("the dictation session", e))?;
    let _ = sess.SetAutoStopSilenceTimeout(TimeSpan { Duration: (AUTO_STOP.as_nanos() / 100) as i64 });
    let tx = out.clone();
    let h1 = reco
        .HypothesisGenerated(&TypedEventHandler::<SpeechRecognizer, SpeechRecognitionHypothesisGeneratedEventArgs>::new(move |_, a| {
            if let Some(a) = a.as_ref() {
                let _ = tx.send(Heard::Guess(a.Hypothesis()?.Text()?.to_string()));
            }
            Ok(())
        }))
        .map_err(|e| os("the dictation events", e))?;
    let tx = out.clone();
    let h2 = sess
        .ResultGenerated(&TypedEventHandler::<SpeechContinuousRecognitionSession, SpeechContinuousRecognitionResultGeneratedEventArgs>::new(move |_, a| {
            if let Some(a) = a.as_ref() {
                let r = a.Result()?;
                let ok = r.Status()? == SpeechRecognitionResultStatus::Success && r.Confidence()? != SpeechRecognitionConfidence::Rejected;
                // a rejected sentence (noise) still clears its guess
                let _ = tx.send(Heard::Sentence(if ok { r.Text()?.to_string() } else { String::new() }));
            }
            Ok(())
        }))
        .map_err(|e| os("the dictation events", e))?;
    let (dtx, done) = mpsc::channel();
    let h3 = sess
        .Completed(&TypedEventHandler::<SpeechContinuousRecognitionSession, SpeechContinuousRecognitionCompletedEventArgs>::new(move |_, a| {
            let s = a.as_ref().and_then(|a| a.Status().ok()).unwrap_or(SpeechRecognitionResultStatus::Unknown);
            let _ = dtx.send(s);
            Ok(())
        }))
        .map_err(|e| os("the dictation events", e))?;
    sess.StartAsync().and_then(|a| a.join()).map_err(|e| os("starting the microphone", e))?;
    let meter = default_mic().ok().and_then(|d| unsafe { d.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None) }.ok());
    Ok(Rig { reco, sess, done, meter, hooks: (h1, h2, h3) })
}

/// The mic's level from the microphone's peak (0..1): the square root lifts normal speech (peaks ~0.1-0.4) to ~0.3-0.6
/// (guessed, like the fake's WAV levels; never measured on a real voice).
fn level(peak: f32) -> f32 {
    peak.max(0.0).sqrt().clamp(0.0, 1.0)
}

/// Levels every 50 ms until `stop` (or Windows ends the session); then StopAsync - the sentence being spoken still arrives -
/// and Ended (or Failed with why Windows ended it).
fn listen(rig: Rig, tag: &str, stop: &AtomicBool, out: &Sender<Heard>) {
    let mut ended: Option<SpeechRecognitionResultStatus> = None;
    while !stop.load(Ordering::SeqCst) {
        match rig.done.recv_timeout(Duration::from_millis(50)) {
            Ok(s) => {
                ended = Some(s);
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        let peak = rig.meter.as_ref().and_then(|m| unsafe { m.GetPeakValue() }.ok()).unwrap_or(0.0);
        if out.send(Heard::Level(level(peak))).is_err() {
            break;
        }
    }
    let _ = out.send(Heard::Level(0.0));
    let r = match ended {
        // Windows ended it by itself (silence, the network, the mic gone)
        Some(s) => status(s, tag),
        None => {
            let _ = rig.sess.StopAsync().and_then(|a| a.join());
            let _ = rig.done.recv_timeout(Duration::from_millis(2000));
            Ok(())
        }
    };
    let _ = rig.reco.RemoveHypothesisGenerated(rig.hooks.0);
    let _ = rig.sess.RemoveResultGenerated(rig.hooks.1);
    let _ = rig.sess.RemoveCompleted(rig.hooks.2);
    let _ = rig.reco.Close();
    let _ = out.send(match r {
        Ok(()) => Heard::Ended,
        Err(e) => Heard::Failed(e),
    });
}

/// The Windows clipboard (Copy).
#[derive(Default)]
pub struct WinClipboard;

impl Clipboard for WinClipboard {
    fn set_text(&mut self, text: &str) -> Result<()> {
        let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        unsafe {
            OpenClipboard(Some(HWND::default())).map_err(|e| os("opening the clipboard", e))?;
            let r = (|| -> Result<()> {
                EmptyClipboard().map_err(|e| os("emptying the clipboard", e))?;
                let h = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).map_err(|e| os("clipboard memory", e))?;
                let p = GlobalLock(h) as *mut u16;
                if p.is_null() {
                    let _ = GlobalFree(Some(h));
                    return Err(VoiceError::Os { context: "clipboard memory".into(), code: 0 });
                }
                std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
                let _ = GlobalUnlock(h);
                if let Err(e) = SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(h.0))) {
                    let _ = GlobalFree(Some(HGLOBAL(h.0)));
                    return Err(os("setting the clipboard", e));
                }
                Ok(())
            })();
            let _ = CloseClipboard();
            r
        }
    }
}

/// A Windows Settings page (`crate::SETTINGS_*`), only on the user's click. Changes nothing by itself.
pub fn open_settings(uri: &str) -> Result<()> {
    let u = wide(uri);
    let r = unsafe { ShellExecuteW(None, w!("open"), PCWSTR(u.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    if r.0 as isize > 32 {
        Ok(())
    } else {
        Err(VoiceError::Os { context: "opening Windows Settings".into(), code: r.0 as isize as u32 })
    }
}

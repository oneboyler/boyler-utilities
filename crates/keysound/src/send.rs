//! Sending: macros (SendInput on ONE worker thread at a time) and the preset actions (Windows only). The rules and the plan
//! are in [`crate::macros`] / [`crate::binds`]; here is only the Windows shell. Input goes out through `SendInput` — the
//! documented way, no hook and no injection into any process — and never while a game / full-screen window or an
//! administrator window is in front ([`crate::guard::input_blocked`]), checked before every event.

use crate::binds::Preset;
use crate::guard;
use crate::macros::{plan, run, Macro, Out, Outcome, Safety};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC,
    VIRTUAL_KEY,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

static RUNNING: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);
static LAST: Mutex<Option<String>> = Mutex::new(None);

const VK_MEDIA_NEXT_TRACK: u16 = 0xB0;
const VK_MEDIA_PREV_TRACK: u16 = 0xB1;
const VK_MEDIA_STOP: u16 = 0xB2;
const VK_MEDIA_PLAY_PAUSE: u16 = 0xB3;
const VK_VOLUME_MUTE: u16 = 0xAD;
const VK_VOLUME_DOWN: u16 = 0xAE;
const VK_VOLUME_UP: u16 = 0xAF;

/// Keys that need the "extended key" flag.
fn extended(vk: u16) -> bool {
    matches!(vk, 0x21..=0x28 | 0x2C | 0x2D | 0x2E | 0x5B..=0x5D | 0x6F | 0x90 | 0xA3 | 0xA5 | 0xA6..=0xB7)
}

fn send(input: INPUT) {
    // SAFETY: one well-formed INPUT.
    unsafe {
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

fn kbd(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } } }
}

fn send_key(vk: u16, down: bool) {
    // SAFETY: a plain lookup.
    let scan = unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_VSC) } as u16;
    let mut f = KEYBD_EVENT_FLAGS(0);
    if extended(vk) {
        f |= KEYEVENTF_EXTENDEDKEY;
    }
    if !down {
        f |= KEYEVENTF_KEYUP;
    }
    send(kbd(vk, scan, f));
}

fn send_unit(unit: u16, down: bool) {
    let mut f = KEYEVENTF_UNICODE;
    if !down {
        f |= KEYEVENTF_KEYUP;
    }
    send(kbd(0, unit, f));
}

/// Opens an app / file / folder / website the way Explorer does.
pub fn shell_open(target: &str) -> Result<(), String> {
    let wide: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: NUL-terminated strings that outlive the call.
    let r = unsafe { ShellExecuteW(None, w!("open"), PCWSTR(wide.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    if r.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!("Windows couldn't open {target}"))
    }
}

struct SendOut;
impl Out for SendOut {
    fn key(&mut self, vk: u16, down: bool) {
        send_key(vk, down);
    }
    fn unit(&mut self, unit: u16, down: bool) {
        send_unit(unit, down);
    }
    fn open(&mut self, target: &str) -> Result<(), String> {
        shell_open(target)
    }
    fn sleep(&mut self, ms: u32) {
        std::thread::sleep(Duration::from_millis(u64::from(ms)));
    }
}

/// The real safety question, remembered for 25 ms so typing a long text doesn't ask Windows for every character.
struct WinSafety {
    at: Option<Instant>,
    cached: Option<&'static str>,
}
impl Safety for WinSafety {
    fn blocked(&mut self) -> Option<String> {
        if self.at.is_none_or(|t| t.elapsed() > Duration::from_millis(25)) {
            self.cached = guard::input_blocked();
            self.at = Some(Instant::now());
        }
        self.cached.map(str::to_string)
    }
}

fn set_last(s: String) {
    *LAST.lock().unwrap_or_else(|p| p.into_inner()) = Some(s);
}

/// Starts macro `m` on a worker thread and returns at once. Err (nothing sent): the macro is wrong or empty, one is running
/// already, or input may not go out now (a game / full-screen window or an administrator window is in front).
pub fn run_macro(m: &Macro) -> Result<(), String> {
    m.check()?;
    if m.steps.is_empty() {
        return Err("This macro has no steps yet".into());
    }
    if let Some(why) = guard::input_blocked() {
        set_last(format!("not run: {why}"));
        return Err(format!("Not run: {why}"));
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("A macro is already running".into());
    }
    CANCEL.store(false, Ordering::SeqCst);
    let events = plan(m);
    let spawned = std::thread::Builder::new().name("bu-macro".into()).spawn(move || {
        let out = run(&events, &mut SendOut, &mut WinSafety { at: None, cached: None }, &CANCEL);
        set_last(match out {
            Outcome::Done => "done".to_string(),
            Outcome::Stopped(why) => format!("stopped: {why}"),
        });
        RUNNING.store(false, Ordering::SeqCst);
    });
    if spawned.is_err() {
        RUNNING.store(false, Ordering::SeqCst);
        return Err("Windows couldn't start the macro".into());
    }
    Ok(())
}

/// Stops a running macro (it lets go of any key it holds).
pub fn cancel_macro() {
    CANCEL.store(true, Ordering::SeqCst);
}

pub fn macro_running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

/// How the last macro ended ("done", "stopped: a game or full-screen window is in front", …).
pub fn last_macro_outcome() -> Option<String> {
    LAST.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Runs a preset action: the media / volume keys go out as media keys (they are for Windows, not for the window in front, so
/// they work over a game too); opening an app / folder / website never happens over a game / full-screen window.
pub fn run_preset(p: &Preset) -> Result<(), String> {
    if !p.ready() {
        return Err("This action has nothing to open yet".into());
    }
    let tap = |vk: u16| {
        send_key(vk, true);
        send_key(vk, false);
        Ok(())
    };
    match p {
        Preset::PlayPause => tap(VK_MEDIA_PLAY_PAUSE),
        Preset::NextTrack => tap(VK_MEDIA_NEXT_TRACK),
        Preset::PrevTrack => tap(VK_MEDIA_PREV_TRACK),
        Preset::StopMedia => tap(VK_MEDIA_STOP),
        Preset::VolumeUp => tap(VK_VOLUME_UP),
        Preset::VolumeDown => tap(VK_VOLUME_DOWN),
        Preset::VolumeMute => tap(VK_VOLUME_MUTE),
        Preset::NextOutput => Err("Switching the audio output is done by the app itself".into()),
        Preset::LockPc => {
            // SAFETY: a plain call, no arguments.
            unsafe { windows::Win32::System::Shutdown::LockWorkStation() }.map_err(|e| format!("Windows didn't lock the PC: {}", e.message()))
        }
        _ if p.combo().is_some() => press_combo(p.combo().unwrap_or(&[])),
        Preset::OpenApp(t) | Preset::OpenFolder(t) | Preset::OpenWeb(t) => {
            if guard::game_in_front() {
                return Err("Not opened: a game or full-screen window is in front".into());
            }
            shell_open(t)
        }
        _ => Err("This action has nothing to run".into()),
    }
}

/// Presses a key combination (modifiers first, released in reverse) into the window in front - never while a game /
/// full-screen window or an administrator window is in front (the same guard a macro has).
fn press_combo(vks: &[u16]) -> Result<(), String> {
    if let Some(why) = guard::input_blocked() {
        return Err(format!("Not run: {why}"));
    }
    for v in vks {
        send_key(*v, true);
    }
    for v in vks.iter().rev() {
        send_key(*v, false);
    }
    Ok(())
}

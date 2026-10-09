//! The Windows shell around [`Hub`]: the "bu-rawin" thread, its message-only window, the registration and the reads.

use std::ffi::c_void;
use std::mem::size_of;
use std::sync::{Mutex, MutexGuard, OnceLock};

use windows::core::{w, HRESULT, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::{
    GetRawInputBuffer, GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
    RIDEV_INPUTSINK, RIDEV_REMOVE, RID_INPUT, RIM_TYPEKEYBOARD, RIM_TYPEMOUSE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostMessageW, RegisterClassW, SendMessageTimeoutW,
    HWND_MESSAGE, MSG, SMTO_BLOCK, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_INPUT, WNDCLASSW,
};

use crate::hub::{Hub, KeysNeed, MouseSoundEvent, Posts, RawPacket, SoundEvent, Stats, Target, USAGE_PAGE_GENERIC};

/// Who plays the key sounds (Order 058): called on the raw thread, right after a batch is read, once per key going down
/// or up. It gets a [`SoundEvent`] (class + up / down) and nothing else - never the key.
pub type SoundSink = std::sync::Arc<dyn Fn(SoundEvent) + Send + Sync>;
static SINK: Mutex<Option<SoundSink>> = Mutex::new(None);

fn sink() -> MutexGuard<'static, Option<SoundSink>> {
    SINK.lock().unwrap_or_else(|p| p.into_inner())
}

/// The key sounds (Order 058): Some = deliver every key down / up to `sink` as a class-only event (the keyboard is
/// registered for it, listen-only); None = stop listening (nothing is registered for it, nothing is kept). Err = Windows
/// refused the registration (its own text); the old setting stays.
pub fn set_key_sound(new: Option<SoundSink>) -> Result<(), String> {
    let on = new.is_some();
    let old_sink = std::mem::replace(&mut *sink(), new);
    let old_on = hub().set_sound(on);
    if old_on == on {
        return Ok(());
    }
    request().inspect_err(|_| {
        *sink() = old_sink;
        hub().set_sound(old_on);
    })
}

/// Who plays the mouse button sounds (Order 064): called on the raw thread, once per button going down or up, with a
/// [`MouseSoundEvent`] (button class + up / down) and nothing else.
pub type MouseSink = std::sync::Arc<dyn Fn(MouseSoundEvent) + Send + Sync>;
static MSINK: Mutex<Option<MouseSink>> = Mutex::new(None);

fn msink() -> MutexGuard<'static, Option<MouseSink>> {
    MSINK.lock().unwrap_or_else(|p| p.into_inner())
}

/// The mouse button sounds (Order 064): Some = deliver every button down / up to `sink` as a class-only event (the mouse is
/// registered for it, listen-only; moves and wheel turns never leave the raw thread); None = stop listening (nothing is
/// registered for it, nothing is kept). Err = Windows refused the registration (its own text); the old setting stays.
pub fn set_mouse_sound(new: Option<MouseSink>) -> Result<(), String> {
    let on = new.is_some();
    let old_sink = std::mem::replace(&mut *msink(), new);
    let old_on = hub().set_mouse_sound(on);
    if old_on == on {
        return Ok(());
    }
    request().inspect_err(|_| {
        *msink() = old_sink;
        hub().set_mouse_sound(old_on);
    })
}

/// "Ignore repeats within `ms`" for the key sounds (Order 059; 0 = off, at most 80): a key that comes down twice inside the
/// window plays one sound. Works inside the sound client only - no input is blocked or changed, the typed keys are
/// untouched. The filter keeps a few bytes of state while it is on and none while it is off.
pub fn set_sound_chatter(ms: u32) {
    hub().set_chatter(ms);
}

/// Another thread asks the raw thread to bring the registration up to date (sent; LRESULT 0 = done, else the HRESULT
/// of Windows' refusal).
const WM_APPLY: u32 = WM_APP + 1;
/// E_FAIL, for a refusal that came without an error code.
const E_FAIL: i32 = 0x8000_4005_u32 as i32;

/// One batch read holds up to this many packets; one RAWINPUT more of room stays unused at the end so the last block
/// can always be read as a whole RAWINPUT (a keyboard block is shorter).
const BATCH: usize = 64;
const RAW_SIZE: usize = size_of::<RAWINPUT>();
const BUF_WORDS: usize = (BATCH + 1) * RAW_SIZE / 8 + 1;

static HUB: Mutex<Hub> = Mutex::new(Hub::new());
/// The raw thread's window (started on first use), or why it couldn't start.
static WINDOW: OnceLock<Result<isize, String>> = OnceLock::new();

fn hub() -> MutexGuard<'static, Hub> {
    HUB.lock().unwrap_or_else(|p| p.into_inner())
}

/// The keys manager's needs: raw keyboard / mouse, and where to post its wake-up (window, message). Packets that matter
/// are kept for [`take_packets`]; one message is posted per batch. (false, false, None) = it needs nothing. Err = Windows
/// refused the registration (its own text); the old needs stay.
pub fn set_keys(keyboard: bool, mouse: bool, notify: Option<Target>) -> Result<(), String> {
    let (old, change) = {
        let mut h = hub();
        let old = h.set_keys(keyboard, mouse, notify);
        // (Order 048 review) asked whenever the NEEDS changed, not when `registered` looks different: `registered` may be
        // in the middle of a change on the raw thread; this request is applied after it, so it always sees the final state
        (old, old != KeysNeed { keyboard, mouse, target: notify })
    };
    if !change {
        return Ok(());
    }
    request().inspect_err(|_| hub().restore_keys(old))
}

/// The activity watcher: Some = post that message once, on the first input of any kind (moves too), then disarm;
/// None = disarm. Err = Windows refused the registration; it stays disarmed.
pub fn notify_on_input(target: Option<Target>) -> Result<(), String> {
    let (old, change) = {
        let mut h = hub();
        let old = h.set_activity(target);
        (old, old != target)
    };
    if !change {
        return Ok(());
    }
    request().inspect_err(|_| {
        hub().set_activity(old);
    })
}

/// The packets that matter for the keys (key packets; mouse buttons / wheel) since the last call, oldest first. The next
/// such packet posts a new wake-up.
pub fn take_packets() -> Vec<RawPacket> {
    hub().take()
}

/// The counters (packets read, forwarded, wake-ups posted, batches).
pub fn stats() -> Stats {
    hub().stats
}

/// Ask the raw thread to register / remove what changed, and wait for its answer. SMTO_BLOCK: while it waits, the
/// calling thread handles no other sent message - so a caller that holds its own state borrowed (the UI thread inside
/// `services::with`, the activity thread inside its context) is never re-entered. The raw thread never sends to anyone,
/// so this can't deadlock; the time-out is only a guard.
fn request() -> Result<(), String> {
    let hwnd = window()?;
    let mut answer = 0usize;
    // SAFETY: our own window; the raw thread answers it from its message loop.
    let sent = unsafe { SendMessageTimeoutW(hwnd, WM_APPLY, WPARAM(0), LPARAM(0), SMTO_BLOCK, 5000, Some(&mut answer)) };
    if sent.0 == 0 {
        return Err("the raw input thread didn't answer".into());
    }
    if answer == 0 {
        Ok(())
    } else {
        Err(HRESULT(answer as i32).message())
    }
}

fn window() -> Result<HWND, String> {
    WINDOW.get_or_init(start).clone().map(|h| HWND(h as *mut c_void))
}

fn start() -> Result<isize, String> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<isize, String>>();
    std::thread::Builder::new()
        .name("bu-rawin".into())
        .spawn(move || run(tx))
        .map_err(|e| format!("raw input thread: {e}"))?;
    rx.recv().unwrap_or_else(|_| Err("raw input thread ended".into()))
}

fn run(ready: std::sync::mpsc::Sender<Result<isize, String>>) {
    unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let class = w!("BoylerUtilitiesRawInput");
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: class, ..Default::default() };
        RegisterClassW(&wc);
        // message-only (parent HWND_MESSAGE): never shown, gets no broadcasts, only what is sent / posted to it + WM_INPUT
        let hwnd = match CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            PCWSTR::null(),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(inst.into()),
            None,
        ) {
            Ok(h) => h,
            Err(e) => {
                let _ = ready.send(Err(format!("raw input window: {}", e.message())));
                return;
            }
        };
        let _ = ready.send(Ok(hwnd.0 as isize));
        // sleeps here (0 CPU) between packets / requests; lives as long as the process
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_INPUT => {
            on_input(hwnd, lp);
            // Windows wants WM_INPUT passed on (it frees that packet)
            unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
        }
        WM_APPLY => match apply(hwnd) {
            Ok(()) => LRESULT(0),
            Err(e) => {
                let code = if e.code().0 == 0 { E_FAIL } else { e.code().0 };
                LRESULT(code as isize)
            }
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

/// Register / remove what changed (on the raw thread only). The lock is not held during the Windows call; only this
/// thread changes `registered`, so a request that comes meanwhile is simply applied after this one.
fn apply(hwnd: HWND) -> windows::core::Result<()> {
    let (changes, want) = {
        let h = hub();
        (h.changes(), h.wanted())
    };
    if changes.is_empty() {
        return Ok(());
    }
    let list: Vec<RAWINPUTDEVICE> = changes
        .into_iter()
        .map(|(usage, on)| RAWINPUTDEVICE {
            usUsagePage: USAGE_PAGE_GENERIC,
            usUsage: usage,
            dwFlags: if on { RIDEV_INPUTSINK } else { RIDEV_REMOVE },
            hwndTarget: if on { hwnd } else { HWND(std::ptr::null_mut()) },
        })
        .collect();
    // SAFETY: a plain registration for our own window.
    unsafe { RegisterRawInputDevices(&list, size_of::<RAWINPUTDEVICE>() as u32) }?;
    hub().registered = want;
    Ok(())
}

/// One RAWINPUT → a packet (None = neither keyboard nor mouse; never registered).
#[inline]
fn parse(raw: &RAWINPUT) -> Option<RawPacket> {
    let t = raw.header.dwType;
    if t == RIM_TYPEMOUSE.0 {
        // SAFETY: dwType says the union holds a RAWMOUSE.
        Some(RawPacket::Mouse { buttons: unsafe { raw.data.mouse.Anonymous.Anonymous.usButtonFlags } })
    } else if t == RIM_TYPEKEYBOARD.0 {
        // SAFETY: dwType says the union holds a RAWKEYBOARD.
        let k = unsafe { raw.data.keyboard };
        Some(RawPacket::Key { vk: k.VKey, make: k.MakeCode, flags: k.Flags })
    } else {
        None
    }
}

/// WM_INPUT: this message's own packet (GetRawInputData), then every packet still queued, in batches
/// (GetRawInputBuffer takes their WM_INPUT messages off the queue too, so they never pile up and nothing is read twice:
/// the message being handled is already off the queue). One lock for the whole batch; the posts go after it.
fn on_input(hwnd: HWND, lp: LPARAM) {
    let mut out = Posts::default();
    {
        let mut h = hub();
        h.stats.batches += 1;
        let mut raw = RAWINPUT::default();
        let mut size = RAW_SIZE as u32;
        // SAFETY: lParam is this WM_INPUT's handle; the buffer is a whole RAWINPUT and `size` says so.
        let n = unsafe {
            GetRawInputData(
                HRAWINPUT(lp.0 as *mut c_void),
                RID_INPUT,
                Some(&mut raw as *mut RAWINPUT as *mut c_void),
                &mut size,
                size_of::<RAWINPUTHEADER>() as u32,
            )
        };
        if n != 0 && n != u32::MAX {
            if let Some(p) = parse(&raw) {
                h.feed(p, &mut out);
            }
        }
        drain(&mut h, &mut out);
    }
    // the key sounds go FIRST (the tightest path press -> sound): straight from this thread, no message, no queue
    if !out.sounds.is_empty() {
        let s = sink().clone();
        if let Some(s) = s {
            for e in out.sounds.drain(..) {
                s(e);
            }
        }
    }
    if !out.mouse_sounds.is_empty() {
        let s = msink().clone();
        if let Some(s) = s {
            for e in out.mouse_sounds.drain(..) {
                s(e);
            }
        }
    }
    if let Some(t) = out.keys {
        if !post(t) {
            hub().post_failed();
        }
    }
    if let Some(t) = out.activity {
        let _ = post(t);
        // the activity watcher is disarmed: drop what only it needed
        let _ = apply(hwnd);
    }
}

/// Every queued packet, BATCH at a time, until the queue is empty.
fn drain(h: &mut Hub, out: &mut Posts) {
    // u64 words: Windows wants the buffer 8-byte aligned (native 64-bit blocks are)
    let mut buf = [0u64; BUF_WORDS];
    let base = buf.as_mut_ptr() as *mut u8;
    let room = BATCH * RAW_SIZE;
    loop {
        let mut size = room as u32;
        // SAFETY: `base` points at BUF_WORDS * 8 >= room bytes, 8-byte aligned.
        let n = unsafe { GetRawInputBuffer(Some(base as *mut RAWINPUT), &mut size, size_of::<RAWINPUTHEADER>() as u32) };
        if n == 0 || n == u32::MAX {
            break;
        }
        let mut off = 0usize;
        for _ in 0..n {
            if off >= room {
                break;
            }
            // SAFETY: a block Windows wrote starts at `off` (< room); a whole RAWINPUT from there stays inside `buf`
            // (one RAWINPUT of spare room) and every byte is initialised (zeroed above).
            let raw = unsafe { &*(base.add(off) as *const RAWINPUT) };
            if let Some(p) = parse(raw) {
                h.feed(p, out);
            }
            // NEXTRAWINPUTBLOCK: the next block starts on the next 8-byte boundary after this one
            let len = (raw.header.dwSize as usize).max(size_of::<RAWINPUTHEADER>());
            off = (off + len + 7) & !7;
        }
    }
}

fn post(t: Target) -> bool {
    // SAFETY: posting a plain message (no pointers) to the client's window.
    unsafe { PostMessageW(Some(HWND(t.0 as *mut c_void)), t.1, WPARAM(0), LPARAM(0)) }.is_ok()
}

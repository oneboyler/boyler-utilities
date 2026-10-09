//! "A game / full-screen window came to the front, or left" (Windows only; Order 058). Keys that carry a macro or an action are
//! swallowed by the keys manager (RegisterHotKey) — in a game that would take the key from the game. So while a game is in
//! front the app lets those keys go (the keys manager's `set_active(false)`) and takes them back when it leaves. This is the
//! watcher that says so.
//!
//! One thread with a message loop that sleeps in `GetMessageW`; it wakes only when Windows says the front window changed
//! (`SetWinEventHook(EVENT_SYSTEM_FOREGROUND)`, an accessibility event — nothing is injected, no input hook) and then twice
//! more, 0.7 s and 2.7 s later, because a game's window is in front BEFORE it goes full-screen. It exists only while some key
//! carries a macro or an action (the app starts it then and stops it when the last one goes): nothing runs otherwise.

use crate::guard::game_in_front;
use std::cell::RefCell;
use std::sync::Arc;
use std::thread::JoinHandle;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, KillTimer, PostThreadMessageW, SetTimer, EVENT_SYSTEM_FOREGROUND, MSG, OBJID_WINDOW, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_QUIT,
};

type Callback = Arc<dyn Fn(bool) + Send + Sync>;

struct State {
    cb: Callback,
    last: bool,
    /// Re-checks still to make after the last front-window change.
    again: u8,
    timer: usize,
}

thread_local! {
    static ST: RefCell<Option<State>> = const { RefCell::new(None) };
}

pub struct GameWatch {
    thread: u32,
    join: Option<JoinHandle<()>>,
}

impl GameWatch {
    /// Starts watching; `cb(true)` when a game / full-screen window is in front, `cb(false)` when none is (only on a change,
    /// plus once at the start). Called on the watcher's thread: keep it short.
    pub fn start(cb: Callback) -> Result<GameWatch, String> {
        let (tx, rx) = std::sync::mpsc::channel::<Result<u32, String>>();
        let join = std::thread::Builder::new()
            .name("bu-gamewatch".into())
            .spawn(move || run(cb, tx))
            .map_err(|e| format!("game watcher thread: {e}"))?;
        match rx.recv() {
            Ok(Ok(thread)) => Ok(GameWatch { thread, join: Some(join) }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("game watcher ended".into()),
        }
    }

    pub fn stop(mut self) {
        self.shut();
    }

    fn shut(&mut self) {
        if let Some(j) = self.join.take() {
            // SAFETY: a plain thread message to our own thread.
            unsafe {
                let _ = PostThreadMessageW(self.thread, WM_QUIT, WPARAM(0), LPARAM(0));
            }
            let _ = j.join();
        }
    }
}

impl Drop for GameWatch {
    fn drop(&mut self) {
        self.shut();
    }
}

fn run(cb: Callback, ready: std::sync::mpsc::Sender<Result<u32, String>>) {
    // SAFETY: the hook and timer calls are the documented ones; everything is released before the thread ends.
    unsafe {
        let hook = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_foreground), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);
        if hook.is_invalid() {
            let _ = ready.send(Err("Windows refused the front-window watcher".into()));
            return;
        }
        ST.with(|s| *s.borrow_mut() = Some(State { cb, last: false, again: 0, timer: 0 }));
        let _ = ready.send(Ok(GetCurrentThreadId()));
        refresh(true);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
        let _ = UnhookWinEvent(hook);
        ST.with(|s| {
            if let Some(st) = s.borrow().as_ref() {
                if st.timer != 0 {
                    let _ = KillTimer(None, st.timer);
                }
            }
            *s.borrow_mut() = None;
        });
    }
}

/// Reads the answer; tells the callback when it changed (or on `force`, the first time).
fn refresh(force: bool) {
    let now = game_in_front();
    let cb = ST.with(|s| {
        let mut b = s.borrow_mut();
        let st = b.as_mut()?;
        if now != st.last || force {
            st.last = now;
            Some(st.cb.clone())
        } else {
            None
        }
    });
    if let Some(cb) = cb {
        cb(now);
    }
}

unsafe extern "system" fn on_foreground(_h: HWINEVENTHOOK, _ev: u32, _hwnd: HWND, id_object: i32, _child: i32, _thread: u32, _time: u32) {
    if id_object != OBJID_WINDOW.0 {
        return;
    }
    refresh(false);
    // the window may go full-screen a moment later: look again at 0.7 s and 2.7 s
    ST.with(|s| {
        if let Some(st) = s.borrow_mut().as_mut() {
            st.again = 2;
            // SAFETY: a thread timer without a window; `on_timer` is its callback.
            st.timer = unsafe { SetTimer(None, st.timer, 700, Some(on_timer)) };
        }
    });
}

unsafe extern "system" fn on_timer(_hwnd: HWND, _msg: u32, id: usize, _time: u32) {
    refresh(false);
    ST.with(|s| {
        if let Some(st) = s.borrow_mut().as_mut() {
            if st.again > 0 {
                st.again -= 1;
                // SAFETY: re-arm the same timer.
                st.timer = unsafe { SetTimer(None, id, 2000, Some(on_timer)) };
            } else {
                // SAFETY: stop it.
                unsafe {
                    let _ = KillTimer(None, id);
                }
                st.timer = 0;
            }
        }
    });
}

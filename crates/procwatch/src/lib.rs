//! Boyler Utilities - one shared "a watched game started / ended" signal for the per-app switchers (the Display tab's
//! auto-switch and the Mouse tab's per-game acceleration). Order 048, A_048_01.
//!
//! START = a new TOP-LEVEL WINDOW appears anywhere (`SetWinEventHook(EVENT_OBJECT_CREATE)`, out of context: an
//! accessibility event, nothing is injected, no admin). A game's first window is created before it is shown or goes
//! fullscreen, so the switch still lands before the game is on screen (NOTE_004_01: switching mid-game glitches).
//! The hook callback only filters (a top-level window, its process id); a process id already seen in the last process
//! snapshot is dropped at once. An unseen one makes the thread take ONE Toolhelp process snapshot (no process handle is
//! opened) and report EVERY process new since the last snapshot whose exe name someone listens for - also a game that
//! has no window yet. The first snapshot only records what already runs (that is not a start).
//! Replaces the WMI `__InstanceCreationEvent WITHIN 1` subscriptions, which made Windows' WMI service re-read the whole
//! process list every second for each watcher (~1.3 % of a core in WmiPrvSE).
//!
//! STOP = `OpenProcess(SYNCHRONIZE)` only (A_004_01) + `RegisterWaitForSingleObject`: the Windows thread pool waits,
//! no thread of ours per game. A process that refuses even that handle is checked by snapshot instead: at every
//! snapshot, and when one of its windows is destroyed (`EVENT_OBJECT_DESTROY`, hooked only while such a process is
//! waited for). Already gone when the wait starts = reported ended at once.
//!
//! One thread ("bu-procwatch", a message loop) runs only while someone listens for at least one name or waits for an
//! exit that way; with nothing to do it unhooks and ends. The pure part (seen list, who gets which start, which
//! fallback exits ended) is [`state`], tested without Windows.

pub mod state;

#[cfg(windows)]
mod win;

#[cfg(windows)]
pub use win::{is_elevated, problem, rescan, subscribe, wait_exit, ExitWait, OnExit, StartFn, Subscription};

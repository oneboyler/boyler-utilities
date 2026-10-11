//! The driver: one thread runs the engine (ClipPing's UI thread), sleeping in a channel wait until an input arrives or
//! its next timer is due - nothing else runs. The WebSocket reader, the folder watcher and an "OBS exited" wait are
//! threads of their own that only post inputs. Stopping (the feature switched off, app exit) ends them all.

use std::path::PathBuf;
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::engine::{Engine, Host, Input, KeysView, PopMsg, View};
use crate::monitors::Mon;
use crate::os::{ObsOs, Waiter};
use crate::settings::Settings;
use crate::sound::{self, Sound};
use crate::ws::{WsClient, WsEvent};

/// What the engine shows / asks of the app (implemented by the app: popups, the page, the keys manager).
pub trait Ui: Send {
    fn popup(&mut self, m: &PopMsg, clipped: Option<usize>);
    fn retract_clip_failed(&mut self) {}
    fn publish(&mut self, v: &View);
    fn dialog(&mut self, text: &str);
    fn keys(&mut self, k: &KeysView);
    fn log(&mut self, _line: &str) {}
}

enum Msg {
    In(Box<Input>),
    Stop,
}

struct H {
    os: Box<dyn ObsOs>,
    ws: WsClient,
    tx: Sender<Msg>,
    ui: Box<dyn Ui>,
    t0: Instant,
    waiter: Arc<Mutex<Waiter>>,
    watch: bool,
    #[cfg(windows)]
    watcher: Option<crate::watch::Watcher>,
    exe_dir: Option<PathBuf>,
    view: Arc<Mutex<View>>,
}

impl Host for H {
    fn now_ms(&self) -> u64 {
        self.t0.elapsed().as_millis() as u64 + 1_000_000 // never 0 (0 = "not set" in the engine)
    }
    fn os(&mut self) -> &mut dyn ObsOs {
        &mut *self.os
    }
    fn ws_start(&mut self, port: u16) -> u32 {
        let tx = self.tx.clone();
        self.ws.start(port, move |e: WsEvent| {
            let _ = tx.send(Msg::In(Box::new(Input::Ws(e))));
        })
    }
    fn ws_busy(&self) -> bool {
        self.ws.busy()
    }
    fn ws_send(&mut self, text: &str) {
        self.ws.send(text);
    }
    fn ws_abort(&mut self) {
        self.ws.abort();
    }
    fn popup(&mut self, m: &PopMsg, clipped: Option<usize>) {
        self.ui.popup(m, clipped);
    }
    fn retract_clip_failed(&mut self) {
        self.ui.retract_clip_failed();
    }
    fn sound(&mut self, ev: Sound, set: &Settings) {
        if let Some((w, _)) = sound::build(set, ev, self.exe_dir.as_deref()) {
            self.os.play(w);
        }
    }
    fn publish(&mut self, v: &View) {
        if let Ok(mut g) = self.view.lock() {
            *g = v.clone();
        }
        self.ui.publish(v);
    }
    fn dialog(&mut self, text: &str) {
        self.ui.dialog(text);
    }
    fn wait_exit(&mut self, pid: u32, timeout_ms: u32) {
        let tx = self.tx.clone();
        let w = self.waiter.clone();
        let _ = std::thread::Builder::new().name("obs-exit".into()).stack_size(64 * 1024).spawn(move || {
            let ok = w.lock().map(|w| w(pid, timeout_ms)).unwrap_or(false);
            let _ = tx.send(Msg::In(Box::new(Input::ObsExited(ok))));
        });
    }
    fn watch(&mut self, dirs: Vec<PathBuf>) {
        if !self.watch {
            return;
        }
        #[cfg(windows)]
        {
            self.watcher = None;
            let tx = self.tx.clone();
            self.watcher = crate::watch::Watcher::start(dirs, move || {
                let _ = tx.send(Msg::In(Box::new(Input::FilesChanged)));
            });
        }
    }
    fn keys(&mut self, k: &KeysView) {
        self.ui.keys(k);
    }
    fn log(&mut self, line: &str) {
        self.ui.log(line);
    }
}

/// The running feature. Dropping it stops everything.
pub struct Service {
    tx: Sender<Msg>,
    view: Arc<Mutex<View>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

pub struct Options {
    /// watch OBS's folders for changes (real runs)
    pub watch: bool,
    /// saved.wav etc. next to the app's exe replace the built-in sounds (ClipPing's rule)
    pub exe_dir: Option<PathBuf>,
}

impl Service {
    pub fn start(os: Box<dyn ObsOs>, set: Settings, mons: Vec<Mon>, ui: Box<dyn Ui>, opt: Options) -> Service {
        let (tx, rx) = channel::<Msg>();
        let view = Arc::new(Mutex::new(View::default()));
        let v2 = view.clone();
        let tx2 = tx.clone();
        let thread = std::thread::Builder::new()
            .name("obs-engine".into())
            .stack_size(512 * 1024)
            .spawn(move || {
                let waiter = Arc::new(Mutex::new(os.waiter()));
                let mut h = H {
                    os,
                    ws: WsClient::new(),
                    tx: tx2,
                    ui,
                    t0: Instant::now(),
                    waiter,
                    watch: opt.watch,
                    #[cfg(windows)]
                    watcher: None,
                    exe_dir: opt.exe_dir,
                    view: v2,
                };
                let mut e = Engine::new(set, mons);
                e.start(&mut h);
                loop {
                    let msg = match e.next_due() {
                        Some(d) => {
                            let now = h.now_ms();
                            rx.recv_timeout(Duration::from_millis(d.saturating_sub(now)))
                        }
                        None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
                    };
                    match msg {
                        Ok(Msg::In(i)) => e.input(&mut h, *i),
                        Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                        Err(RecvTimeoutError::Timeout) => {}
                    }
                    e.fire_timers(&mut h);
                }
                e.finish_on_stop(&mut h);
                h.ws.abort();
                #[cfg(windows)]
                {
                    h.watcher = None;
                }
            })
            .expect("obs engine thread");
        Service { tx, view, thread: Some(thread) }
    }

    pub fn send(&self, i: Input) {
        let _ = self.tx.send(Msg::In(Box::new(i)));
    }

    /// The last published view.
    pub fn view(&self) -> View {
        self.view.lock().map(|v| v.clone()).unwrap_or_default()
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

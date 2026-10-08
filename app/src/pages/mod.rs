//! The page API (Order 014): every tab of the top row is one module here, registered ONCE in `all()` (the top row's
//! order = the drawing's PAGES list). A tab lane adds or changes only its own page file (+ its crate's dependency line in
//! app/Cargo.toml). How to write a page: app/PAGES.md.
//!
//! Life of a page: `open` when its tab is shown (create the feature crate's service here - the FAKE one in tests),
//! `build` whenever it must be painted again (returns the boxes of the drawing's `.pg`, header first), `event` for
//! clicks / drags / keys on its elements, `close` when the tab is left or the menu closes (drop the service and every
//! cached value: closed menu = ~0 % CPU, small RAM). Nothing slow starts in `open` or `build` - only a button starts a
//! job (the job runner, Order 014 item 2).

use crate::ui::cx::{Cx, Ev};
use crate::ui::el::El;
use crate::ui::LegacyPage;

pub mod activity;
pub mod addons;
pub mod apps;
pub mod audio;
pub mod controller;
pub mod display;
pub mod mouse;
pub mod network;
pub mod notifications;
pub mod performance;
pub mod screenshots;
pub mod search;
pub mod security;
pub mod settings;
pub mod srch_data;
pub mod startup;
pub mod storage;
pub mod timers;
pub mod tweaks;
pub mod voice;

/// What the app hands every page.
#[derive(Clone, Debug, Default)]
pub struct Env {
    /// a test copy (testmode.rs): FAKE services only, nothing on the PC is ever changed
    pub test: bool,
    /// a test copy that reads the real services but changes nothing (measure.ps1)
    pub real_read: bool,
    /// test pictures: live values frozen to the drawing's sample values
    pub frozen: bool,
    /// Windows' "show animations" is off (the drawing's reduced motion): transitions .01 s
    pub rm: bool,
    /// What the page remembers while the APP runs (keep.rs, feedback F2): its last result + when, across tab switches
    /// and window closes. The app's one store in the menu; `Env::default()` = a fresh one (each unit test its own).
    pub keep: crate::keep::Keep,
}

impl Env {
    /// Use the feature crate's FAKE layer (every test copy except --real-read).
    pub fn fake(&self) -> bool {
        self.test && !self.real_read
    }
    /// The page's waker for its background threads: `let w = env.waker(); thread::spawn(move || { …; w.wake(); })` -
    /// the menu then builds the page again (Send + Sync + Copy; harmless after the menu closed).
    pub fn waker(&self) -> crate::services::Waker {
        crate::services::Waker
    }
}

/// A page's part that runs for the app's whole life (`Page::background`). Dropped at app exit.
pub trait Background {
    /// test hook text (what a test checks), after `bg <page id>: `
    fn describe(&self) -> String {
        String::new()
    }
}

/// One tab of the menu.
pub trait Page {
    /// the drawing's page id (`aud`, `dsp`, ... - also the test hook's `--tab` name)
    fn id(&self) -> &'static str;
    /// the title and the top row's hover name
    fn name(&self) -> &'static str;
    /// the top row's icon (a name of the drawing's ICON table, icons.rs)
    fn icon(&self) -> &'static str;
    /// the tab is shown: create the crate's service (fake in tests)
    fn open(&mut self, _env: &Env, _now: f64) {}
    /// the tab is left or the menu closes: drop everything it holds
    fn close(&mut self) {}
    /// Its real values are in (a page whose service reads on a worker thread: false until the first answer). While false,
    /// right after `open`, the frame keeps showing the old page / holds the open motion (0.4 s at most), so the page
    /// never shows defaults first and then snaps to the real settings (the owner Oct 8). Transitions jump for 0.5 s after an
    /// open anyway.
    fn ready(&self) -> bool {
        true
    }
    /// the page's boxes (the children of the drawing's `.pg`, the header `.ph` first)
    fn build(&mut self, cx: &mut Cx) -> Vec<El>;
    /// input on one of its elements
    fn event(&mut self, _ev: &Ev, _cx: &mut Cx) {}
    /// called every frame while shown; true = live content is moving (meters), keep painting
    fn tick(&mut self, _now: f64) -> bool {
        false
    }
    /// a popup of this page (small window, review list...) drawn above everything, in window coordinates
    fn popup(&mut self, _cx: &mut Cx) -> Option<El> {
        None
    }
    /// the popup was dismissed from outside (a click outside it, Esc)
    fn popup_dismiss(&mut self) {}
    /// A NON-modal layer of this page, fixed to the window (window coordinates, not scrolled with the page) and above
    /// it: a selection bar (`.selbar`), a note. Only its own boxes take clicks; a click beside it is not "outside" and
    /// closes nothing. (A small note is `cx.toast(text)`.)
    fn overlay(&mut self, _cx: &mut Cx) -> Option<El> {
        None
    }
    /// The overlay's selection bar is shown (`.selbar.on`): the frame's toast then sits above it (the drawing raises
    /// `#toast` while a `.selbar.on` exists).
    fn bar_shown(&self) -> bool {
        false
    }
    /// Another page sent the menu here with a target (`cx.show_tab(id, Some(target))`, the drawing's `jump(id, el)`):
    /// open that popup / mark that row. Called right after `open`.
    fn jump(&mut self, _target: &str) {}
    /// test hook text after `tab=<id> open=<bool> ` (values a test checks)
    fn describe(&self) -> String {
        String::new()
    }
    /// a hand-painted page (only Audio, test A's code): the frame calls it instead of `build`
    fn legacy(&mut self) -> Option<&mut dyn LegacyPage> {
        None
    }
    fn legacy_ref(&self) -> Option<&dyn LegacyPage> {
        None
    }
    /// Once at app start, kept until the app ends (NOT with the menu): a background part of this page's feature that must
    /// run while the menu is closed - Display's per-game watcher and its 10 s keep / revert countdown, Mouse's per-app
    /// switch, the Screenshot key's capture. The app holds the returned box and drops it at exit (its `Drop` stops it).
    /// It talks to the page through the page module's own shared state; `env.waker()` repaints an open menu. Idle cost
    /// rule: it must sleep (events / waits), never poll.
    fn background(&self, _env: &Env) -> Option<Box<dyn Background>> {
        None
    }
    /// Once at app start (the menu may never open): register this page's keys and what they do -
    /// `s.add_action(keys::Action::new("mic.mute", "Mic mute", "aud"), move |down| …)`. Keys work with the menu closed, so
    /// the handler uses its own always-on part of the crate (never the page's UI state).
    fn start(&self, _s: &mut crate::services::Services) {}
    /// The page's items for the reset line ("Back to how your PC was" / "Windows defaults"): None = the page changes
    /// nothing on the PC.
    fn resettable(&mut self) -> Option<&mut dyn crate::undo::Resettable> {
        None
    }
}

/// Every tab, in the top row's order (the drawing's PAGES list, menu-v22).
pub fn all() -> Vec<Box<dyn Page>> {
    vec![
        Box::new(audio::Audio::new()),
        Box::new(display::Display::default()),
        Box::new(screenshots::Screenshots::default()),
        Box::new(mouse::Mouse::default()),
        Box::new(controller::Controller::default()),
        Box::new(tweaks::Tweaks::default()),
        Box::new(startup::Startup::default()),
        Box::new(performance::Performance::default()),
        Box::new(network::Network::default()),
        Box::new(storage::Storage::default()),
        Box::new(apps::Apps::default()),
        Box::new(security::Security::default()),
        Box::new(search::Search::default()),
        Box::new(voice::Voice::default()),
        Box::new(timers::Timers::default()),
        Box::new(activity::Activity::default()),
        // Order 035: Notifications for OBS (addons-v1: a page add-on, in the top row only while on - `crate::addons::tab_visible`)
        Box::new(notifications::Notifications::default()),
        // Order 037: Add-ons (addons-v1: the puzzle, just before Settings)
        Box::new(addons::Addons::default()),
        Box::new(settings::Settings::default()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::Gfx;
    use crate::ui::cx::State;
    use crate::ui::lay::Laid;

    #[test]
    fn the_tabs_in_the_drawings_order() {
        let ids: Vec<&str> = all().iter().map(|p| p.id()).collect();
        assert_eq!(ids, ["aud", "dsp", "shot", "cur", "pad", "tgl", "sup", "pc", "net", "sto", "apps", "sec", "srch", "vtt", "tmr", "act", "ntf", "add", "set"]);
        assert_eq!(crate::ui::tab_index("Voice to text"), Some(13));
        assert_eq!(crate::ui::tab_index("tmr"), Some(14));
    }

    /// A page's boxes land where Chromium lays out the drawing (menu-v22, tools/ref/dom_dump.js): the header `.ph` at
    /// window (28, 58) 544 x 32, the title at (28, 62); Startup's `.seg.fit` at (402.64, 60) 169.36 x 28.
    #[test]
    fn header_boxes_match_the_drawing() {
        let g = Gfx::new(1.0);
        let mut st = State::default();
        let env = Env { test: true, ..Env::default() };
        for (tab, want) in [(1usize, None), (6, Some((402.64f32, 60.0f32, 169.36f32, 28.0f32)))] {
            let mut pages = all();
            pages[tab].open(&env, 0.0);
            let mut cx = Cx::new(0.0, false, &g, &mut st);
            let kids = pages[tab].build(&mut cx);
            let root = El::block().w(600.0).pad(2.0, 26.0, 18.0, 26.0).children(kids);
            let laid = Laid::new(&g, root, 600.0, None);
            let ph = laid.nodes[1].rect;
            assert_eq!((ph.0, ph.1 + 56.0, ph.2, ph.3), (28.0, 58.0, 544.0, 32.0));
            let h2 = laid.nodes[2].rect;
            assert_eq!((h2.0, h2.1 + 56.0, h2.3), (28.0, 62.0, 24.0));
            if let Some(w) = want {
                let s = laid.nodes[3].rect;
                let near = |a: f32, b: f32| (a - b).abs() < 0.02;
                assert!(near(s.0, w.0) && near(s.1 + 56.0, w.1) && near(s.2, w.2) && near(s.3, w.3), "seg at {:?}", s);
            }
        }
    }
}

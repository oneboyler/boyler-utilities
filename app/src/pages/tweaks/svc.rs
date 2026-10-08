//! The Tweaks page's link to `bu-toggles`: the real service (Windows), the read-only one (measure.ps1 copies) or the
//! FAKE one (every test copy) seeded with the drawing's sample values (menu-v22 `TGL[].on`, `v`, `FSO`, `DEFS`).

use bu_toggles::defaults::{self, ChangeAction, DefaultApps};
use bu_toggles::fake::FakeOs;
use bu_toggles::os::{AssocApp, PowerValues, RegisteredBrowser};
use bu_toggles::{Applied, FsoGame, Result, RowState, Timeout, Toggles};

use super::rows::ROWS;

pub enum Svc {
    Fake(Box<Toggles<FakeOs>>),
    #[cfg(windows)]
    Real(Toggles<crate::admin::proxy::TweaksOs<bu_toggles::real::RealOs>>),
}

macro_rules! with {
    ($s:expr, $t:ident => $e:expr) => {
        match $s {
            Svc::Fake($t) => $e,
            #[cfg(windows)]
            Svc::Real($t) => $e,
        }
    };
}

impl Svc {
    #[cfg(windows)]
    pub fn real(read_only: bool) -> Svc {
        let os = if read_only { bu_toggles::real::RealOs::read_only() } else { bu_toggles::real::RealOs::new() };
        // the admin rows go to the app's elevated copy behind Windows' admin prompt (Order 039)
        Svc::Real(Toggles::new(crate::admin::proxy::TweaksOs::new(os, crate::admin::client::admin())))
    }

    /// The fake at the drawing's sample values (switches as `TGL[].on`, Screen off 10 min, Sleep after 30 min,
    /// Rocket League in the games list, Chrome + the file types of `DEFS`). Not elevated afterwards, like a normal start.
    pub fn sample() -> Svc {
        let mut os = FakeOs::new();
        os.elevated = true;
        let progid = |n: &str| format!("{n}HTML");
        for (reg, name) in [("Google Chrome", "Chrome"), ("Microsoft Edge", "Edge"), ("Firefox", "Firefox")] {
            os.browsers.push(RegisteredBrowser { reg_name: reg.into(), display_name: name.into(), machine: true, https_progid: Some(progid(name)) });
        }
        os.default_browser_progid = Some(progid("Chrome"));
        let app = |n: &str| Some(AssocApp { name: n.into(), exe: None });
        for (what, n) in [("https", "Chrome"), (".png", "Photos"), (".jpg", "Photos"), (".mp4", "Media Player"), (".mkv", "VLC"), (".mp3", "Media Player"), (".pdf", "Edge"), (".txt", "Notepad"), (".zip", "File Explorer")] {
            if let Some(a) = app(n) {
                os.assoc.insert(what.into(), a);
            }
        }
        let mut t = Toggles::new(os);
        for r in ROWS.iter() {
            match r.sample {
                Sample::On(on) => {
                    if let Ok(st) = t.read(r.crate_id) {
                        if st.value != bu_toggles::Value::Switch(on) {
                            let _ = t.set(r.crate_id, on);
                        }
                    }
                }
                Sample::Time(s) => {
                    let _ = t.set_timeout(r.crate_id, Timeout::from_seconds(s));
                }
                Sample::Games => {
                    let _ = t.fso_set(SAMPLE_GAME, true);
                }
            }
        }
        t.os_mut().elevated = false;
        t.os_mut().log.clear();
        Svc::Fake(Box::new(t))
    }

    pub fn read(&self, id: &str) -> Result<RowState> {
        with!(self, t => t.read(id))
    }
    pub fn set(&mut self, id: &str, on: bool) -> Result<Applied> {
        with!(self, t => t.set(id, on))
    }
    pub fn set_timeout(&mut self, id: &str, v: Timeout) -> Result<Applied> {
        with!(self, t => t.set_timeout(id, v))
    }
    /// Any number of seconds (the reset puts back an old timeout that may not be in the list).
    pub fn set_seconds(&mut self, id: &str, secs: u32) -> Result<Applied> {
        with!(self, t => t.set_seconds(id, secs))
    }
    pub fn power_values(&self, id: &str) -> Result<PowerValues> {
        with!(self, t => t.power_values(id))
    }
    pub fn set_power_values(&mut self, id: &str, v: PowerValues) -> Result<Applied> {
        with!(self, t => t.set_power_values(id, v))
    }
    pub fn fso_state(&self, exe: &str) -> Result<bool> {
        with!(self, t => t.fso_state(exe))
    }
    pub fn undo(&mut self, id: &str) -> Result<Applied> {
        with!(self, t => t.undo(id))
    }
    pub fn needs_admin(&self, id: &str) -> bool {
        with!(self, t => t.needs_admin(id).unwrap_or(false))
    }
    pub fn fso_games(&self) -> Result<Vec<FsoGame>> {
        with!(self, t => t.fso_games())
    }
    pub fn fso_set(&mut self, exe: &str, off: bool) -> Result<Applied> {
        with!(self, t => t.fso_set(exe, off))
    }
    pub fn fso_remove(&mut self, exe: &str) -> Result<Applied> {
        with!(self, t => t.fso_remove(exe))
    }
    pub fn fso_undo(&mut self, exe: &str) -> Result<Applied> {
        with!(self, t => t.fso_undo(exe))
    }
    pub fn defaults(&self) -> Result<DefaultApps> {
        with!(self, t => defaults::read(t.os()))
    }
    pub fn open(&mut self, a: &ChangeAction) -> Result<()> {
        with!(self, t => defaults::perform(t.os_mut(), a))
    }
    /// test copies: what the fake was asked to do
    pub fn fake(&self) -> Option<&FakeOs> {
        match self {
            Svc::Fake(t) => Some(t.os()),
            #[cfg(windows)]
            Svc::Real(_) => None,
        }
    }
    pub fn fake_mut(&mut self) -> Option<&mut FakeOs> {
        match self {
            Svc::Fake(t) => Some(t.os_mut()),
            #[cfg(windows)]
            Svc::Real(_) => None,
        }
    }
}

/// The drawing's one sample game (`FSO[0]`: Rocket League, RocketLeague.exe).
pub const SAMPLE_GAME: &str = r"C:\Program Files\Epic Games\rocketleague\Binaries\Win64\RocketLeague.exe";
/// The drawing's file picker answers (`FSO_PICK`), for test copies: what "Add game" picks.
pub const SAMPLE_PICKS: [&str; 2] = [r"C:\Program Files (x86)\Steam\steamapps\common\Counter-Strike Global Offensive\game\bin\win64\cs2.exe", r"C:\Program Files\Epic Games\Fortnite\FortniteGame\Binaries\Win64\FortniteClient-Win64-Shipping.exe"];

/// A row's drawing sample value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
    On(bool),
    /// a timeout in seconds
    Time(u32),
    Games,
}

//! Mouse acceleration = Raw Accel (free, open source; github.com/RawAccelOfficial/rawaccel). Get = download the PINNED
//! official release, check it, unpack it into the add-ons folder, run its own installer.exe through the elevated helper
//! (one admin prompt; the driver works after a restart). Remove = its own uninstaller.exe the same way (the driver file
//! goes at the restart), then our folder is deleted. A Raw Accel the user installed themselves shows as got and is removed
//! with the uninstaller of the user's own folder when that file is an official one (else ours is downloaded for it).

use crate::error::{AddonError, Result};
use crate::os::{AddonOs, Elevated, HelperAction};
use bu_updater::http::{Http, Request};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The one release we install. Measured Oct 8 2026 from GitHub's release API (asset size + its published SHA-256 digest)
/// and from that zip's files (the same zip, byte for byte, as a copy downloaded from the release page).
pub struct Pin {
    pub version: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    pub installer_sha256: &'static str,
    pub driver_sha256: &'static str,
}

pub const PIN: Pin = Pin {
    version: "1.7.1",
    url: "https://github.com/RawAccelOfficial/rawaccel/releases/download/v1.7.1/RawAccel_v1.7.1.zip",
    size: 1_542_808,
    sha256: "770fe3ae0919ca3c4d412f58c985eb27f5434decad809f7e8206de4e8852eec4",
    installer_sha256: "b3b4948de9470a5727390ed016507b59d2c628463e42aaf1b75e906ee04f58a9",
    driver_sha256: "8a62c4deef2774b43a7363b352eda79897533a1080c9c26ffeff0559e43358d7",
};

/// uninstaller.exe of the official releases 1.6.1, 1.7.0, 1.7.1 (measured from their release zips): only these are ever
/// run to remove the driver.
pub const OFFICIAL_UNINSTALLERS: [&str; 3] = [
    "944946946b3e853dff5ad058dedbe7fb81d0c5aa2c45a39b0e2b47fd3b42f561",
    "32013a0b17355c680f573d30d6dc5f34acbdfbbb0aa0568052ca283b53625e7a",
    "62090da1ecd73aa18c2119650a4384d80691fcbabb90716cf5f731ed084a51ca",
];

/// The folder inside the zip.
pub const ZIP_PREFIX: &str = "RawAccel/";
/// Our folder's name inside the add-ons folder.
pub const DIR_NAME: &str = "RawAccel";
const STAGE_NAME: &str = "RawAccel.new";

/// Where Raw Accel stands on this PC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaState {
    /// Not installed.
    Absent,
    /// Installed and running.
    Installed,
    /// Installed; works after the restart.
    InstallRestart,
    /// Removed; the driver still runs until the restart.
    RemoveRestart,
}

impl RaState {
    /// Shown as got (the tile's "On this PC", the Mouse tab's card).
    pub fn got(self) -> bool {
        matches!(self, RaState::Installed | RaState::InstallRestart)
    }
}

pub fn state(os: &dyn AddonOs) -> RaState {
    match (os.rawaccel_filter_set(), os.rawaccel_running()) {
        (true, true) => RaState::Installed,
        (true, false) => RaState::InstallRestart,
        (false, true) => RaState::RemoveRestart,
        (false, false) => RaState::Absent,
    }
}

/// What the work is doing now (for the progress bar and its line).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    Download { got: u64, total: u64 },
    /// Size, SHA-256, unpacking, the driver's signature.
    Checking,
    /// Windows' admin prompt + Raw Accel's installer.
    Installing,
    /// Windows' admin prompt + Raw Accel's uninstaller.
    Removing,
}

/// "1.5 MB" (one decimal, MB = 10^6 bytes as Windows' download sizes on the web; 1,542,808 B = 1.5 either way).
pub fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

struct Sink<'a> {
    buf: Vec<u8>,
    max: usize,
    stop: &'a dyn Fn() -> bool,
}

impl Write for Sink<'_> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if (self.stop)() {
            return Err(std::io::Error::other("cancelled"));
        }
        if self.buf.len() + data.len() > self.max {
            return Err(std::io::Error::other("bigger than the official release"));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Download the pinned zip into memory and check its size + SHA-256. Cancel = `stop()` true (checked on every chunk).
pub fn download(http: &dyn Http, step: &mut dyn FnMut(Step), stop: &dyn Fn() -> bool) -> Result<Vec<u8>> {
    let total = PIN.size;
    step(Step::Download { got: 0, total });
    let mut sink = Sink { buf: Vec::with_capacity(total as usize), max: total as usize, stop };
    let r = http.get(&Request { url: PIN.url, accept: "application/octet-stream" }, &mut sink, &mut |got, _| step(Step::Download { got: got.min(total), total }));
    if stop() {
        return Err(AddonError::Cancelled);
    }
    let r = r.map_err(|e| AddonError::Network(e.to_string()))?;
    if r.status != 200 {
        return Err(AddonError::Network(format!("GitHub answered {}", r.status)));
    }
    step(Step::Checking);
    let not_official = |what: &str| AddonError::Verify(format!("The download was not Raw Accel\u{2019}s official {} release ({what})", PIN.version));
    if sink.buf.len() as u64 != total {
        return Err(not_official("wrong size"));
    }
    if crate::sha256_hex(&sink.buf) != PIN.sha256 {
        return Err(not_official("its SHA-256 did not match"));
    }
    Ok(sink.buf)
}

/// Unpack the checked zip into `<root>\RawAccel` (through a staging folder; the old one is replaced) and let Windows
/// check the driver's signature. Returns the folder.
pub fn unpack(os: &dyn AddonOs, zip: &[u8], root: &Path) -> Result<PathBuf> {
    let io = |e: std::io::Error| AddonError::Files(e.to_string());
    let stage = root.join(STAGE_NAME);
    if stage.exists() {
        std::fs::remove_dir_all(&stage).map_err(io)?;
    }
    let entries = crate::zip::read(zip, 16 * 1024 * 1024)?;
    crate::zip::extract_under(&entries, ZIP_PREFIX, &stage)?;
    let checked = (|| {
        let driver = stage.join("driver").join("rawaccel.sys");
        let bytes = std::fs::read(&driver).map_err(|_| AddonError::Verify("The release has no driver".into()))?;
        if crate::sha256_hex(&bytes) != PIN.driver_sha256 {
            return Err(AddonError::Verify("The driver was not the official one".into()));
        }
        os.verify_signature(&driver)
    })();
    if let Err(e) = checked {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(e);
    }
    let dir = root.join(DIR_NAME);
    if dir.exists() {
        // Raw Accel's own window keeps the user's curves in settings.json next to it: carried over, never lost
        let old = dir.join("settings.json");
        if old.is_file() {
            let _ = std::fs::copy(&old, stage.join("settings.json"));
        }
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            let _ = std::fs::remove_dir_all(&stage);
            return Err(AddonError::Files(format!("{e} (is Raw Accel’s window open?)")));
        }
    }
    std::fs::rename(&stage, &dir).map_err(io)?;
    Ok(dir)
}

fn elevated(r: Elevated) -> Result<()> {
    match r {
        Elevated::Done => Ok(()),
        Elevated::Declined => Err(AddonError::Declined),
        Elevated::Failed(s) => Err(AddonError::Tool(s)),
    }
}

/// Get: download, check, unpack, install (admin prompt). Cancel works until the install step starts. On a failed or
/// declined install our folder is deleted again (nothing left behind).
pub fn get(http: &dyn Http, os: &dyn AddonOs, root: &Path, step: &mut dyn FnMut(Step), stop: &dyn Fn() -> bool) -> Result<PathBuf> {
    let zip = download(http, step, stop)?;
    let dir = unpack(os, &zip, root)?;
    // the last chance to cancel - checked after the page shows "Installing" (its Cancel goes away then), so a Cancel
    // pressed before is never ignored
    step(Step::Installing);
    if stop() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(AddonError::Cancelled);
    }
    match elevated(os.run_elevated(HelperAction::RawAccelInstall, &dir, stop)) {
        Ok(()) => Ok(dir),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            Err(e)
        }
    }
}

/// The uninstaller.exe in `dir` is an official one.
pub fn official_uninstaller(dir: &Path) -> bool {
    std::fs::read(dir.join("uninstaller.exe")).map(|b| OFFICIAL_UNINSTALLERS.contains(&crate::sha256_hex(&b).as_str())).unwrap_or(false)
}

/// Remove: Raw Accel's own uninstaller (ours, else the official one in the user's own Raw Accel folder `user_dir`, else
/// the pinned release downloaded for it), admin prompt; then our folder is deleted (the user's own folder never).
pub fn remove(http: &dyn Http, os: &dyn AddonOs, root: &Path, user_dir: Option<&Path>, step: &mut dyn FnMut(Step), stop: &dyn Fn() -> bool) -> Result<()> {
    let ours = root.join(DIR_NAME);
    let tool_dir = if official_uninstaller(&ours) {
        ours.clone()
    } else if let Some(u) = user_dir.filter(|u| official_uninstaller(u)) {
        u.to_path_buf()
    } else {
        let zip = download(http, step, stop)?;
        unpack(os, &zip, root)?
    };
    step(Step::Removing);
    elevated(os.run_elevated(HelperAction::RawAccelUninstall, &tool_dir, stop))?;
    if ours.exists() {
        let _ = std::fs::remove_dir_all(&ours);
    }
    Ok(())
}

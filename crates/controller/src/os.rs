//! The OS layer behind traits: [`SteamOs`] (Steam's files, read and written) and [`PadOs`] (controllers, read only).
//! Real Windows code is in `real/`, the fake for tests in [`crate::fake`].

use crate::error::Result;
use crate::parts::PadKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// One directory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
}

/// Steam's files. Writes are whole-file replaces (write a temp file next to it, then rename over it), so Steam never
/// sees half a file.
pub trait SteamOs: Send + Sync {
    /// Steam's install folder (Windows: `HKCU\Software\Valve\Steam\SteamPath`), `None` = no Steam.
    fn steam_dir(&self) -> Option<PathBuf>;
    /// The Steam account that is logged in now (`HKCU\Software\Valve\Steam\ActiveProcess\ActiveUser`), if any.
    fn active_account(&self) -> Option<u32>;
    fn read(&self, path: &Path) -> Result<Vec<u8>>;
    fn exists(&self, path: &Path) -> bool;
    fn list(&self, dir: &Path) -> Result<Vec<Entry>>;
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<()>;
    fn create_dir_all(&self, dir: &Path) -> Result<()>;
    /// Remove a file the app created itself (undo of a "your own copy" of a community layout).
    fn remove(&self, path: &Path) -> Result<()>;
    /// Is Steam running (read-only check; the app never starts, closes or restarts Steam).
    fn steam_running(&self) -> bool;
}

/// How a controller is connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connection {
    Usb,
    Bluetooth,
    Unknown,
}

/// Battery as the controller reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    /// 0–100 (PlayStation pads report steps of 10 %).
    pub percent: Option<u8>,
    /// Xbox pads only report four steps.
    pub level: Option<BatteryLevel>,
    pub charging: bool,
    /// On a cable / wired (no battery in use).
    pub wired: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryLevel {
    Empty,
    Low,
    Medium,
    Full,
}

/// Where the live data of a controller comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PadSource {
    /// A HID device (PlayStation pads): its interface path.
    Hid(String),
    /// An XInput slot 0–3 (Xbox pads).
    XInput(u32),
}

/// One connected controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PadInfo {
    pub kind: PadKind,
    /// The name the controller reports (e.g. "DualSense Edge Wireless Controller").
    pub name: String,
    pub connection: Connection,
    pub battery: Option<Battery>,
    pub source: PadSource,
}

/// What a live source hands over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveEvent {
    /// One input report (PlayStation: the raw HID report, byte 0 = report id; Xbox: [`crate::live::xinput_report`]).
    Report(Vec<u8>),
    /// Stopped on purpose.
    Stopped,
    /// The controller went away.
    Gone,
}

/// Reads a controller's input while the page is open. `next` blocks until the controller sends something or the
/// stopper is called (PlayStation: an overlapped read + a stop event — no timer; Xbox: XInput has no events, so it
/// polls only while running).
pub trait LiveSource: Send {
    fn next(&mut self) -> Result<LiveEvent>;
    /// Called from another thread to end a blocked `next`.
    fn stopper(&self) -> Arc<dyn Fn() + Send + Sync>;
}

/// Controllers: list them, read their input. Never sends anything TO a controller (no rumble, no LED, no output report).
pub trait PadOs: Send + Sync {
    fn list_pads(&self) -> Result<Vec<PadInfo>>;
    fn open_live(&self, pad: &PadInfo) -> Result<Box<dyn LiveSource>>;
}

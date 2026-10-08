//! Row 1 — **Reset graphics driver** (= Win + Ctrl + Shift + B).
//!
//! Research E §3: no documented API triggers Windows' graphics reset; sending the chord with `SendInput` *might* work
//! (nobody documents it — one AutoHotkey report says the synthetic chord did nothing). The reliable fallback is
//! restarting the display adapter(s) (`pnputil /restart-device`, Windows 10 2004+, admin): screens go black 2–5 s and
//! a running game will likely crash or lose its 3D device.
//!
//! So: [`reset_with_chord`] (no admin) and [`restart_adapters`] (admin), and the app layer decides which the button
//! uses after the owner's 10-second test on a real PC (**unclear** until then).

use crate::os::{DisplayAdapter, FixOs};
use crate::{FixError, Result};
use std::io::Read;

/// Sends Win + Ctrl + Shift + B. Refused unless this app's own window is in front — the chord is never sent into a
/// game or another app (the project rules: never send fake input to games). Whether Windows acts on a synthetic chord is
/// **unclear** (Windows marks it "injected"); `Ok` only means the keys were sent.
pub fn reset_with_chord(os: &dyn FixOs) -> Result<()> {
    if !os.foreground_is_ours() {
        return Err(FixError::Refused("the key chord is only sent while the Boyler Utilities menu is in front".into()));
    }
    os.send_reset_chord()
}

/// One adapter's restart result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterRestart {
    pub adapter: DisplayAdapter,
    /// pnputil's exit code: 0 = restarted; 3010 = restarted but Windows wants a reboot (guess: the standard Win32
    /// ERROR_SUCCESS_REBOOT_REQUIRED — no source documents pnputil's exit codes).
    pub exit_code: u32,
    /// pnputil's own words (last non-empty line).
    pub message: String,
}

impl AdapterRestart {
    pub fn ok(&self) -> bool {
        self.exit_code == 0 || self.exit_code == 3010
    }
    pub fn needs_reboot(&self) -> bool {
        self.exit_code == 3010
    }
}

/// Restarts every present display adapter with `pnputil /restart-device "<instance id>"`. Needs admin.
pub fn restart_adapters(os: &dyn FixOs) -> Result<Vec<AdapterRestart>> {
    if !os.is_elevated() {
        return Err(FixError::NeedsAdmin("restarting the display adapter".into()));
    }
    let adapters = os.display_adapters()?;
    if adapters.is_empty() {
        return Err(FixError::Unavailable("no display adapter found".into()));
    }
    let mut out = Vec::new();
    for a in adapters {
        let mut p = os.spawn("pnputil.exe", &["/restart-device", &a.instance_id])?;
        let mut bytes = Vec::new();
        let _ = p.output.read_to_end(&mut bytes);
        let code = p.ctl.wait()?;
        let text = crate::repair::decode_all(&bytes);
        let message = text.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("").to_string();
        out.push(AdapterRestart { adapter: a, exit_code: code, message });
    }
    Ok(out)
}

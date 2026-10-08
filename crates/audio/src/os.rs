//! The OS layer: everything this crate asks Windows, behind one trait. Implementations only read and write; all
//! rules (last device on, unmute on slider move, grouping, Keep my devices, New apps volume) live in the crate.

use crate::model::*;
use crate::Result;

pub trait AudioOs {
    /// Every device of a flow that is on, switched off or unplugged (not ones Windows removed for good).
    fn devices(&mut self, flow: Flow) -> Result<Vec<Device>>;
    fn defaults(&mut self, flow: Flow) -> Result<Defaults>;
    /// Sets the default for one role (undocumented `IPolicyConfig::SetDefaultEndpoint` in the real layer).
    fn set_default(&mut self, id: &str, role: Role) -> Result<()>;
    fn volume(&mut self, id: &str) -> Result<VolumeMute>;
    fn set_volume(&mut self, id: &str, volume: f32) -> Result<()>;
    fn set_mute(&mut self, id: &str, muted: bool) -> Result<()>;
    /// The device's level right now, 0.0 … 1.0 (`IAudioMeterInformation::GetPeakValue`).
    fn peak(&mut self, id: &str) -> Result<f32>;
    /// Device on/off (undocumented `IPolicyConfig::SetEndpointVisibility`, the Sound panel's Disable / Enable).
    fn set_enabled(&mut self, id: &str, on: bool) -> Result<()>;
    /// The sessions on one output device (not expired ones).
    fn sessions(&mut self, device_id: &str) -> Result<Vec<SessionInfo>>;
    fn set_session_volume(&mut self, key: &str, volume: f32) -> Result<()>;
    fn set_session_mute(&mut self, key: &str, muted: bool) -> Result<()>;
    /// A session's level right now, 0.0 … 1.0.
    fn session_peak(&mut self, key: &str) -> Result<f32>;
    /// The app's name, icon and colour (may be slow the first time: the shell reads the icon).
    fn app_look(&mut self, s: &SessionInfo) -> AppLook;
}

/// The drawing's grey for apps without a colourful icon (and System sounds).
pub const GREY: (u32, u32) = (0x8d96a8, 0xc6ccd8);

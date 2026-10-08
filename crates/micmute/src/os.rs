//! The OS layer: everything this crate asks Windows, behind two small traits — the mic ([`MicOs`]) and the speaker for
//! the mute sounds ([`SoundOut`]). Real = `crate::real`, fake = `crate::fake`.

use crate::Result;
use std::sync::Arc;

/// One active capture endpoint (a microphone), as Windows lists it in Sound settings › Input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicDevice {
    /// The endpoint id (`{0.0.1.00000000}.{guid}`) — stable across restarts, what the shared mic choice stores.
    pub id: String,
    /// The friendly name, e.g. "Microphone (Shure MV7)".
    pub name: String,
    /// Windows' default input device (the "Default Device" role, `eConsole`).
    pub is_default: bool,
}

/// Something that changed outside our own calls (or because of them). Delivered on a Windows audio thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MicEvent {
    /// The watched mic's mute flag is now `muted`. `by_us` = the change came from this crate's own `set_muted`
    /// (Windows hands our event-context GUID back), `false` = another app (Discord, Sound settings, a headset button …).
    Mute { device_id: String, muted: bool, by_us: bool },
    /// Windows' default input device changed (`None` = no input device left).
    DefaultChanged { device_id: Option<String> },
    /// A capture device was added, removed, enabled or disabled.
    DevicesChanged,
}

/// Where a watcher delivers its events.
pub type EventSink = Arc<dyn Fn(MicEvent) + Send + Sync>;

/// A running watch; dropping it unregisters from Windows (nothing keeps running).
pub trait Watch: Send {}

/// Everything Mic mute needs from Windows' audio stack (Core Audio). All logic sits in [`crate::MicMute`].
pub trait MicOs: Send + Sync {
    /// Active capture devices (plugged in and enabled), default first is NOT promised — order as Windows lists them.
    fn capture_devices(&self) -> Result<Vec<MicDevice>>;
    /// The id of Windows' default input device (`eCapture`, `eConsole`), `None` when there is no input device.
    fn default_capture(&self) -> Result<Option<String>>;
    /// The endpoint mute flag (`IAudioEndpointVolume::GetMute`).
    fn is_muted(&self, device_id: &str) -> Result<bool>;
    /// Sets the endpoint mute flag (`IAudioEndpointVolume::SetMute`), tagged with our own event context so the
    /// change event can tell it apart from other apps' changes.
    fn set_muted(&self, device_id: &str, muted: bool) -> Result<()>;
    /// Watches one mic's mute flag (`RegisterControlChangeNotify`) — event-driven, no polling.
    fn watch_mute(&self, device_id: &str, sink: EventSink) -> Result<Box<dyn Watch>>;
    /// Watches the device list + the default input device (`IMMNotificationClient`) — event-driven, no polling.
    fn watch_devices(&self, sink: EventSink) -> Result<Box<dyn Watch>>;
}

/// The speaker for the mute / unmute sounds: plays one in-memory WAV, asynchronously (a new sound cuts the old one).
/// Tests use a fake that only records — no sound ever reaches a real speaker in a test.
pub trait SoundOut: Send + Sync {
    fn play_wav(&self, wav: Vec<u8>) -> Result<()>;
    fn stop(&self);
}

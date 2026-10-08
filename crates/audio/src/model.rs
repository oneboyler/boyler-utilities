//! The plain data the Audio tab shows.

/// Output (playback, "render") or Input (recording, "capture").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Flow {
    Output,
    Input,
}

/// Windows' three default-device roles. "Set as default" in the Sound panel sets all three; so does this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Role {
    /// "Default device" (games, system sounds).
    Console,
    /// Music / movies.
    Multimedia,
    /// "Default communication device" (Discord, Teams).
    Communications,
}

pub const ROLES: [Role; 3] = [Role::Console, Role::Multimedia, Role::Communications];
pub const FLOWS: [Flow; 2] = [Flow::Output, Flow::Input];

/// The 16 px glyph in the device popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    Speakers,
    Headphones,
    /// A monitor / TV over HDMI or DisplayPort.
    Monitor,
    Microphone,
    Webcam,
    /// A game controller (PS5 DualSense, Xbox …).
    Controller,
}

impl DeviceKind {
    /// The glyph name the drawing uses (spk, hp, mon, mic, wcam, pad).
    pub fn glyph(self) -> &'static str {
        match self {
            DeviceKind::Speakers => "spk",
            DeviceKind::Headphones => "hp",
            DeviceKind::Monitor => "mon",
            DeviceKind::Microphone => "mic",
            DeviceKind::Webcam => "wcam",
            DeviceKind::Controller => "pad",
        }
    }

    /// From the device's name and Windows' `PKEY_AudioEndpoint_FormFactor` (1 Speakers, 2 LineLevel, 3 Headphones,
    /// 4 Microphone, 5 Headset, 6 Handset, 7 UnknownDigitalPassthrough, 8 SPDIF, 9 DigitalAudioDisplayDevice). The name
    /// wins for controllers and webcams (Windows reports them as Headset / Microphone).
    pub fn classify(name: &str, form_factor: u32, flow: Flow) -> DeviceKind {
        let n = name.to_lowercase();
        if ["controller", "dualsense", "dualshock", "xbox", "gamepad"].iter().any(|w| n.contains(w)) {
            return DeviceKind::Controller;
        }
        if ["webcam", "camera", "c920", "c922", "brio"].iter().any(|w| n.contains(w)) {
            return DeviceKind::Webcam;
        }
        match (form_factor, flow) {
            (3 | 5 | 6, _) => DeviceKind::Headphones,
            (9, Flow::Output) => DeviceKind::Monitor,
            (_, Flow::Input) => DeviceKind::Microphone,
            _ if n.contains("headphone") || n.contains("headset") => DeviceKind::Headphones,
            _ if ["nvidia", "monitor", "display", "hdmi", "displayport"].iter().any(|w| n.contains(w)) => DeviceKind::Monitor,
            _ => DeviceKind::Speakers,
        }
    }
}

/// Where a device is: on (usable), switched off (the Sound panel's "Disable"), or not plugged in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceState {
    On,
    Off,
    Unplugged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// Windows' endpoint id (opaque).
    pub id: String,
    /// "Headphones (Arctis Nova)" — Windows' friendly name.
    pub name: String,
    pub kind: DeviceKind,
    pub flow: Flow,
    pub state: DeviceState,
}

/// One device row in the popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRow {
    pub device: Device,
    /// The accent ✓: the current default (Console role).
    pub current: bool,
    /// Its switch.
    pub on: bool,
    /// The last device still on: its switch is disabled ("One device always stays on").
    pub switch_locked: bool,
}

/// Windows' default device per role for one flow.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Defaults {
    pub console: Option<String>,
    pub multimedia: Option<String>,
    pub communications: Option<String>,
}

impl Defaults {
    pub fn get(&self, r: Role) -> Option<&String> {
        match r {
            Role::Console => self.console.as_ref(),
            Role::Multimedia => self.multimedia.as_ref(),
            Role::Communications => self.communications.as_ref(),
        }
    }
    pub fn set(&mut self, r: Role, id: Option<String>) {
        match r {
            Role::Console => self.console = id,
            Role::Multimedia => self.multimedia = id,
            Role::Communications => self.communications = id,
        }
    }
}

/// A device's volume and mute.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumeMute {
    /// 0.0 … 1.0 (Windows' scalar, what its own slider shows).
    pub volume: f32,
    pub muted: bool,
}

/// Windows' session state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// A stream is open and running — "making sound" (the mixer shows it).
    Active,
    /// Open but stopped.
    Inactive,
    /// Gone.
    Expired,
}

/// One Core Audio session (an app can have several).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionInfo {
    /// Windows' session instance id (unique per session).
    pub key: String,
    pub pid: u32,
    /// When the process started (FILETIME, 100 ns). With the pid it names one process run (pids get reused).
    pub process_started: u64,
    /// The exe's full path ("" when it can't be read).
    pub exe_path: String,
    /// The name the app gave the session ("" or "@resource" when none).
    pub display_name: String,
    /// Windows' "System sounds" session.
    pub system: bool,
    pub state: SessionState,
    pub volume: f32,
    pub muted: bool,
}

/// An app's icon as 32-bit premultiplied BGRA, row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Icon {
    pub w: u32,
    pub h: u32,
    pub bgra: Vec<u8>,
}

/// What the mixer shows for one app: name, icon, colour.
#[derive(Debug, Clone, PartialEq)]
pub struct AppLook {
    /// The exe's description (like Windows' own mixer), else the session's name, else the exe name.
    pub name: String,
    pub icon: Option<Icon>,
    /// The slider / level colour (0xRRGGBB) and its lighter tint for the level bar's end.
    pub colour: u32,
    pub colour2: u32,
}

/// One mixer row: every session of one app, grouped.
#[derive(Debug, Clone, PartialEq)]
pub struct AppRow {
    /// The group: the exe path (lower case), or "pid:<n>" when the path can't be read, or "system".
    pub group: String,
    pub look: AppLook,
    pub sessions: Vec<String>,
    pub pids: Vec<u32>,
    /// The loudest session's volume (Windows' mixer shows one slider per app; the crate sets every session to it).
    pub volume: f32,
    /// Muted = every session muted.
    pub muted: bool,
    pub system: bool,
}

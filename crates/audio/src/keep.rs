//! "Keep my devices" — stop Windows switching to a newly plugged-in device (e.g. a PS5 controller). Pure logic, fed
//! with Windows' device events (the real watcher gets them from `IMMNotificationClient`: no polling).
//!
//! The rule (both Output and Input, all three roles):
//! * The **chosen** default per flow + role is the last default that the user picked (in this app or in Windows).
//! * A default change TO a device that arrived (plugged in / switched on) less than [`ARRIVAL_WINDOW`] ago is Windows'
//!   doing → put the chosen device back (or, if that one isn't plugged in, the previous default).
//! * A default change right after the chosen device left (unplugged) is Windows' fallback → accepted, but the chosen
//!   device stays chosen, so when it comes back and Windows switches to it, that is kept.
//! * Any other default change counts as the user's own choice → it becomes the chosen one.
//! * Our own put-back makes Windows report a change to the chosen device → nothing to do.
//!
//! Known limit: picking the just-plugged device by hand within those seconds is undone too.

use crate::model::{Flow, Role};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

/// How long after a device arrives a default change to it counts as Windows' automatic switch (guess: Windows switches
/// within about a second of the plug-in; 5 s leaves room on a busy PC).
pub const ARRIVAL_WINDOW: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceEvent {
    /// A device became usable: added, plugged back in, or switched on (`OnDeviceAdded` / state → ACTIVE).
    Present { id: String, flow: Flow },
    /// A device left: unplugged, removed, switched off (state ≠ ACTIVE / `OnDeviceRemoved`).
    Gone { id: String },
    /// `OnDefaultDeviceChanged` (`None` = no device left for that role).
    DefaultChanged { flow: Flow, role: Role, id: Option<String> },
}

/// What to do: make `id` the default for `role` again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutBack {
    pub id: String,
    pub role: Role,
}

#[derive(Debug, Clone, Default)]
pub struct KeepDevices {
    on: bool,
    present: HashSet<String>,
    arrived: HashMap<String, Duration>,
    gone: HashMap<String, Duration>,
    chosen: HashMap<(Flow, Role), String>,
    current: HashMap<(Flow, Role), String>,
    /// Our own put-backs not yet reported back by Windows.
    expect: HashMap<(Flow, Role), String>,
}

impl KeepDevices {
    /// Starts from what is plugged in now and the current defaults (they count as chosen).
    pub fn new(on: bool, present: impl IntoIterator<Item = String>, defaults: impl IntoIterator<Item = ((Flow, Role), String)>) -> Self {
        let current: HashMap<_, _> = defaults.into_iter().collect();
        KeepDevices { on, present: present.into_iter().collect(), chosen: current.clone(), current, ..Default::default() }
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    /// The switch. Turning it on takes today's defaults as the chosen ones.
    pub fn set_on(&mut self, on: bool) {
        if on && !self.on {
            self.chosen = self.current.clone();
        }
        self.on = on;
    }

    pub fn chosen(&self, flow: Flow, role: Role) -> Option<&String> {
        self.chosen.get(&(flow, role))
    }

    /// One Windows event at time `now` (any monotonic clock). Returns what to put back (empty = nothing).
    pub fn on_event(&mut self, ev: DeviceEvent, now: Duration) -> Vec<PutBack> {
        match ev {
            DeviceEvent::Present { id, .. } => {
                if self.present.insert(id.clone()) {
                    self.arrived.insert(id, now);
                }
                Vec::new()
            }
            DeviceEvent::Gone { id } => {
                if self.present.remove(&id) {
                    self.gone.insert(id, now);
                }
                Vec::new()
            }
            DeviceEvent::DefaultChanged { flow, role, id: None } => {
                self.current.remove(&(flow, role));
                Vec::new()
            }
            DeviceEvent::DefaultChanged { flow, role, id: Some(id) } => self.default_changed(flow, role, id, now),
        }
    }

    fn within(&self, t: Option<&Duration>, now: Duration) -> bool {
        t.is_some_and(|t| now.saturating_sub(*t) < ARRIVAL_WINDOW)
    }

    fn default_changed(&mut self, flow: Flow, role: Role, id: String, now: Duration) -> Vec<PutBack> {
        let k = (flow, role);
        let prev = self.current.insert(k, id.clone());
        if !self.on {
            self.chosen.insert(k, id);
            return Vec::new();
        }
        if self.expect.get(&k) == Some(&id) {
            // Windows reporting our own put-back
            self.expect.remove(&k);
            return Vec::new();
        }
        let chosen = self.chosen.get(&k).cloned();
        if chosen.as_deref() == Some(id.as_str()) {
            return Vec::new();
        }
        // the default event can come before the device's own "present" event: then it is arriving right now
        if !self.present.contains(&id) {
            self.present.insert(id.clone());
            self.arrived.insert(id.clone(), now);
        }
        if self.within(self.arrived.get(&id), now) {
            let back = chosen
                .filter(|c| self.present.contains(c))
                .or_else(|| prev.filter(|p| *p != id && self.present.contains(p)));
            return match back {
                Some(b) => {
                    // our own put-back will report this device: keep `current` in step so it isn't taken as a choice
                    self.current.insert(k, b.clone());
                    self.expect.insert(k, b.clone());
                    vec![PutBack { id: b, role }]
                }
                None => Vec::new(),
            };
        }
        if let Some(c) = &chosen {
            if !self.present.contains(c) && self.within(self.gone.get(c), now) {
                // Windows' fallback after the chosen device left: accept it, keep the chosen one
                return Vec::new();
            }
        }
        self.chosen.insert(k, id);
        Vec::new()
    }
}

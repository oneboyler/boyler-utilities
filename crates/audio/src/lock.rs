//! "Lock the input volume" (Order 118) - Windows or an app keeps moving the mic's volume; a locked value is put back the
//! moment the default input device's volume changes (Windows' volume notification, no polling). Pure logic, fed with
//! Windows' events by the [`crate::engine::Engine`]; input only.
//!
//! The lock is a value (0.0 ... 1.0, whole percent), not a device: it holds on whichever device is the default input
//! (Console role - the one the Audio page's Input row shows), so a change of default device carries it along.

/// Two volumes this close are the same (Windows reports the scalar as an f32; 0.04 % is far below a whole percent).
const SAME: f32 = 0.0004;

#[derive(Debug, Clone, Default)]
pub struct MicLock {
    locked: Option<f32>,
    /// The default input device (Console role) now.
    input: Option<String>,
    /// What the device read back right after our last put-back, when it differs from the locked value (a device whose
    /// volume steps are coarse cannot hold it exactly): reported again, it is not put back again - no loop.
    settled: Option<f32>,
}

impl MicLock {
    pub fn new(locked: Option<f32>, input: Option<String>) -> Self {
        MicLock { locked: locked.map(whole_percent), input, settled: None }
    }

    pub fn locked(&self) -> Option<f32> {
        self.locked
    }

    pub fn is_on(&self) -> bool {
        self.locked.is_some()
    }

    /// The default input device the lock holds on (None while the lock is off or there is no input device).
    pub fn watching(&self) -> Option<&str> {
        self.locked.and(self.input.as_deref())
    }

    pub fn set(&mut self, locked: Option<f32>) {
        self.locked = locked.map(whole_percent);
        self.settled = None;
    }

    /// The default input device changed. Returns true when the lock now has a device to hold.
    pub fn set_input(&mut self, id: Option<String>) -> bool {
        if self.input != id {
            self.input = id;
            self.settled = None;
        }
        self.watching().is_some()
    }

    /// A volume notification from `id` (any device we listen to): the value to put back, or None.
    pub fn on_volume(&self, id: &str, volume: f32) -> Option<f32> {
        let locked = self.locked?;
        if self.input.as_deref() != Some(id) {
            return None;
        }
        self.must_put(volume).then_some(locked)
    }

    /// Does a device at `volume` need the locked value put back?
    pub fn must_put(&self, volume: f32) -> bool {
        match self.locked {
            None => false,
            Some(l) => (volume - l).abs() > SAME && self.settled.is_none_or(|s| (volume - s).abs() > SAME),
        }
    }

    /// After a put-back: what the device reads now (equal to the locked value, or the nearest it can hold).
    pub fn note_settled(&mut self, read_back: f32) {
        self.settled = self.locked.filter(|l| (read_back - l).abs() > SAME).map(|_| read_back);
    }
}

fn whole_percent(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        (v * 100.0).round().clamp(0.0, 100.0) / 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_never_puts_back() {
        let l = MicLock::new(None, Some("mic".into()));
        assert_eq!(l.on_volume("mic", 0.3), None);
        assert!(!l.is_on() && l.watching().is_none());
    }

    #[test]
    fn a_change_of_the_default_input_is_put_back_and_the_same_value_is_not() {
        let l = MicLock::new(Some(0.62), Some("mic".into()));
        assert_eq!(l.on_volume("mic", 0.30), Some(0.62));
        assert_eq!(l.on_volume("mic", 0.62), None);
        assert_eq!(l.on_volume("mic", 0.6201), None);
        assert_eq!(l.on_volume("mic", 0.64), Some(0.62), "1 % off is off");
    }

    #[test]
    fn only_the_default_input_counts() {
        let l = MicLock::new(Some(0.5), Some("mic".into()));
        assert_eq!(l.on_volume("other", 0.1), None);
    }

    #[test]
    fn follows_the_default_device() {
        let mut l = MicLock::new(Some(0.5), Some("a".into()));
        assert_eq!(l.watching(), Some("a"));
        assert!(l.set_input(Some("b".into())));
        assert_eq!(l.on_volume("a", 0.1), None);
        assert_eq!(l.on_volume("b", 0.1), Some(0.5));
        assert!(!l.set_input(None));
    }

    #[test]
    fn a_device_that_cannot_hold_the_value_is_not_fought_forever() {
        let mut l = MicLock::new(Some(0.50), Some("mic".into()));
        assert_eq!(l.on_volume("mic", 0.2), Some(0.50));
        l.note_settled(0.49); // the device's nearest step
        assert_eq!(l.on_volume("mic", 0.49), None, "its own echo of our put-back");
        assert_eq!(l.on_volume("mic", 0.2), Some(0.50), "a real change is still put back");
    }

    #[test]
    fn values_are_whole_percent_and_clamped() {
        assert_eq!(MicLock::new(Some(0.4149), None).locked(), Some(0.41));
        assert_eq!(MicLock::new(Some(1.7), None).locked(), Some(1.0));
        assert_eq!(MicLock::new(Some(f32::NAN), None).locked(), Some(0.0));
    }
}

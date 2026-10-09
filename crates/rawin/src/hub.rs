//! The pure part (no Windows calls): which devices must be registered, which packets go to which client, and the
//! "one wake-up per batch" rule. The Windows shell (`win.rs`) keeps one [`Hub`] behind a mutex.

use std::collections::VecDeque;

/// Where a client wants its message: (window handle as an integer, message number).
pub type Target = (isize, u32);

/// HID usage page "generic desktop" and its two usages we register.
pub const USAGE_PAGE_GENERIC: u16 = 1;
pub const USAGE_MOUSE: u16 = 2;
pub const USAGE_KEYBOARD: u16 = 6;

/// The most packets kept for the keys client between two reads (the oldest go first when it is full).
pub const QUEUE_CAP: usize = 256;

/// One raw packet, parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawPacket {
    /// RAWKEYBOARD: VKey, MakeCode, Flags.
    Key { vk: u16, make: u16, flags: u16 },
    /// RAWMOUSE.usButtonFlags: buttons going down / up and the wheels (0 = the mouse only moved).
    Mouse { buttons: u16 },
}

/// RAWKEYBOARD.Flags: the key went up (0 = down).
pub const RI_KEY_BREAK: u16 = 1;
/// RAWKEYBOARD.Flags: an extended (E0) key; E1 is the Pause key's own sequence.
pub const RI_KEY_E0: u16 = 2;
pub const RI_KEY_E1: u16 = 4;

/// What the key SOUNDS may hear of a key (Order 058): one of four classes, never the key itself. Space, Enter and
/// Backspace sound different from the rest; every other key is `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundClass {
    Other,
    Space,
    Enter,
    Backspace,
}

/// One key going down or up, as the sounds hear it. By design this holds NO key identity (no VKey, no scan code): the
/// key is used to pick the class and forgotten at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundEvent {
    pub class: SoundClass,
    pub down: bool,
}

/// The one thing the sound client's repeat filter keeps: WHICH of the 256 scan codes are held down right now (a 32-byte
/// bit map, so a held key's auto-repeat doesn't machine-gun the sound). It is a state, never a history: a release clears
/// the bit, nothing is ever appended, it never leaves the raw thread (see [`Hub::held_count`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Held([u64; 4]);

impl Held {
    const fn new() -> Self {
        Held([0; 4])
    }
    fn bit(code: u8) -> (usize, u64) {
        ((code >> 6) as usize, 1u64 << (code & 63))
    }
    /// Sets the bit; true when it was clear (a new press).
    fn press(&mut self, code: u8) -> bool {
        let (w, m) = Self::bit(code);
        let fresh = self.0[w] & m == 0;
        self.0[w] |= m;
        fresh
    }
    /// Clears the bit.
    fn release(&mut self, code: u8) {
        let (w, m) = Self::bit(code);
        self.0[w] &= !m;
    }
    fn count(&self) -> u32 {
        self.0.iter().map(|w| w.count_ones()).sum()
    }
}

/// The class of a virtual key (Space / Enter / Backspace, else Other).
pub fn sound_class(vk: u16) -> SoundClass {
    match vk {
        0x20 => SoundClass::Space,
        0x0D => SoundClass::Enter,
        0x08 => SoundClass::Backspace,
        _ => SoundClass::Other,
    }
}

/// Counters for the proof that mouse moves stay on the raw thread.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Every packet read (moves too).
    pub packets_seen: u64,
    /// Packets handed to the keys client (keys; mouse buttons / wheel).
    pub packets_forwarded: u64,
    /// Wake-ups posted to the keys client (at most one per batch).
    pub wakes_posted: u64,
    /// WM_INPUT messages handled (each one reads a whole batch).
    pub batches: u64,
}

/// What a batch asks the Windows shell to post once the lock is let go.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Posts {
    /// Wake the keys client (one per batch, none while the last one wasn't read yet).
    pub keys: Option<Target>,
    /// The activity watcher's one-shot "input came" (it is disarmed already; the registration must be redone).
    pub activity: Option<Target>,
    /// Key sounds to play (Order 058), in the order they happened; the shell hands them to the sound client once the lock
    /// is let go. Empty (and never allocated) while nobody listens for sounds.
    pub sounds: Vec<SoundEvent>,
}

/// What the keys client asked for last (put back if Windows refuses the new one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeysNeed {
    pub keyboard: bool,
    pub mouse: bool,
    pub target: Option<Target>,
}

#[derive(Debug)]
pub struct Hub {
    keys: KeysNeed,
    /// The activity watcher waits for the first input (armed) → where to post it.
    activity: Option<Target>,
    /// The key-sound client listens (Order 058): key packets become [`SoundEvent`]s, nothing else is kept.
    sound: bool,
    /// Repeat filter of the sound client (see [`Held`]); empty while nobody listens.
    held: Held,
    /// What is registered with Windows now (keyboard, mouse).
    pub registered: (bool, bool),
    queue: VecDeque<RawPacket>,
    /// A wake-up was posted to the keys client and it hasn't taken the packets since.
    posted: bool,
    pub stats: Stats,
}

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}

impl Hub {
    pub const fn new() -> Self {
        Hub {
            keys: KeysNeed { keyboard: false, mouse: false, target: None },
            activity: None,
            sound: false,
            held: Held::new(),
            registered: (false, false),
            queue: VecDeque::new(),
            posted: false,
            stats: Stats { packets_seen: 0, packets_forwarded: 0, wakes_posted: 0, batches: 0 },
        }
    }

    /// (keyboard, mouse) Windows must deliver: what the keys need, the keyboard while the key sounds listen, and both
    /// while the activity watcher is armed.
    pub fn wanted(&self) -> (bool, bool) {
        let a = self.activity.is_some();
        (self.keys.keyboard || self.sound || a, self.keys.mouse || a)
    }

    /// The registration calls that bring Windows to [`Hub::wanted`]: (usage, true = register / false = remove).
    pub fn changes(&self) -> Vec<(u16, bool)> {
        let want = self.wanted();
        let mut out = Vec::new();
        for (usage, w, have) in [(USAGE_KEYBOARD, want.0, self.registered.0), (USAGE_MOUSE, want.1, self.registered.1)] {
            if w != have {
                out.push((usage, w));
            }
        }
        out
    }

    /// The keys client's needs (a target to wake, or None). Returns the old ones. Nothing needed any more: its queue
    /// is dropped.
    pub fn set_keys(&mut self, keyboard: bool, mouse: bool, target: Option<Target>) -> KeysNeed {
        let old = self.keys;
        self.keys = KeysNeed { keyboard, mouse, target };
        if !keyboard && !mouse {
            self.queue.clear();
            self.posted = false;
        }
        old
    }

    pub fn restore_keys(&mut self, k: KeysNeed) {
        self.set_keys(k.keyboard, k.mouse, k.target);
    }

    /// Arm (Some) / disarm (None) the activity watcher's one-shot message. Returns the old setting.
    pub fn set_activity(&mut self, target: Option<Target>) -> Option<Target> {
        std::mem::replace(&mut self.activity, target)
    }

    /// The key-sound client listens (true) or lets go (false). Returns the old setting. Either way the repeat filter
    /// starts empty: nothing about keys survives a change.
    pub fn set_sound(&mut self, on: bool) -> bool {
        self.held = Held::new();
        std::mem::replace(&mut self.sound, on)
    }

    /// How many keys the repeat filter thinks are held (0 when nothing is held; the proof that nothing piles up).
    pub fn held_count(&self) -> u32 {
        self.held.count()
    }

    /// How many packets wait for the keys client (the sounds never add to this).
    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    /// A key packet for the sound client: the sound event, or None (a repeat of a held key, a fake key, the Pause key's
    /// E1 sequence). The key is looked at here and forgotten: only the class and up / down leave.
    #[inline]
    fn sound_of(&mut self, vk: u16, make: u16, flags: u16) -> Option<SoundEvent> {
        if vk == 0 || vk >= 0xFF || flags & RI_KEY_E1 != 0 {
            return None;
        }
        let code = (make & 0x7F) as u8 | if flags & RI_KEY_E0 != 0 { 0x80 } else { 0 };
        if flags & RI_KEY_BREAK != 0 {
            self.held.release(code);
            Some(SoundEvent { class: sound_class(vk), down: false })
        } else if self.held.press(code) {
            Some(SoundEvent { class: sound_class(vk), down: true })
        } else {
            None
        }
    }

    /// One packet of a batch. Pure moves are only counted; key packets / mouse buttons and wheel go to the keys client
    /// while it needs that device; the first packet of any kind fires (and disarms) the activity watcher.
    #[inline]
    pub fn feed(&mut self, p: RawPacket, out: &mut Posts) {
        self.stats.packets_seen += 1;
        if let Some(t) = self.activity.take() {
            out.activity = Some(t);
        }
        if self.sound {
            if let RawPacket::Key { vk, make, flags } = p {
                if let Some(e) = self.sound_of(vk, make, flags) {
                    out.sounds.push(e);
                }
            }
        }
        let for_keys = match p {
            RawPacket::Key { .. } => self.keys.keyboard,
            RawPacket::Mouse { buttons } => buttons != 0 && self.keys.mouse,
        };
        if !for_keys {
            return;
        }
        if self.queue.len() >= QUEUE_CAP {
            self.queue.pop_front();
        }
        self.queue.push_back(p);
        self.stats.packets_forwarded += 1;
        if !self.posted {
            if let Some(t) = self.keys.target {
                self.posted = true;
                self.stats.wakes_posted += 1;
                out.keys = Some(t);
            }
        }
    }

    /// The keys client's read: every packet waiting, oldest first; the next packet posts a new wake-up.
    pub fn take(&mut self) -> Vec<RawPacket> {
        self.posted = false;
        self.queue.drain(..).collect()
    }

    /// The wake-up couldn't be posted (the client's queue was full): the next packet tries again.
    pub fn post_failed(&mut self) {
        self.posted = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYS: Target = (100, 0x8008);
    const ACT: Target = (200, 0x8002);

    fn key(vk: u16) -> RawPacket {
        RawPacket::Key { vk, make: 0, flags: 0 }
    }
    fn mouse(buttons: u16) -> RawPacket {
        RawPacket::Mouse { buttons }
    }
    fn batch(h: &mut Hub, packets: &[RawPacket]) -> Posts {
        let mut out = Posts::default();
        for p in packets {
            h.feed(*p, &mut out);
        }
        out
    }

    #[test]
    fn moves_stay_buttons_wheel_and_keys_go() {
        let mut h = Hub::new();
        h.set_keys(true, true, Some(KEYS));
        let out = batch(&mut h, &[mouse(0), mouse(0), mouse(0)]);
        assert_eq!(out, Posts::default(), "a move never wakes anyone");
        assert!(h.take().is_empty());
        let p = [mouse(0), mouse(0x0040), mouse(0), mouse(0x0400), key(0x41), mouse(0)];
        let out = batch(&mut h, &p);
        assert_eq!(out.keys, Some(KEYS));
        assert_eq!(h.take(), vec![mouse(0x0040), mouse(0x0400), key(0x41)]);
        assert_eq!(h.stats, Stats { packets_seen: 9, packets_forwarded: 3, wakes_posted: 1, batches: 0 });
    }

    #[test]
    fn only_the_devices_the_keys_need_are_forwarded() {
        let mut h = Hub::new();
        h.set_keys(false, true, Some(KEYS));
        batch(&mut h, &[key(0x41), mouse(0x0010)]);
        assert_eq!(h.take(), vec![mouse(0x0010)]);
        h.set_keys(true, false, Some(KEYS));
        batch(&mut h, &[key(0x41), mouse(0x0010)]);
        assert_eq!(h.take(), vec![key(0x41)]);
    }

    #[test]
    fn one_wake_per_batch_until_the_client_reads() {
        let mut h = Hub::new();
        h.set_keys(true, true, Some(KEYS));
        assert_eq!(batch(&mut h, &[key(1), key(2)]).keys, Some(KEYS));
        assert_eq!(batch(&mut h, &[key(3)]).keys, None, "not read yet: no second wake-up");
        assert_eq!(h.take().len(), 3);
        assert_eq!(batch(&mut h, &[key(4)]).keys, Some(KEYS), "read: the next packet wakes it again");
        h.post_failed();
        assert_eq!(batch(&mut h, &[key(5)]).keys, Some(KEYS), "a failed post is tried again");
        assert_eq!(h.stats.wakes_posted, 3);
    }

    #[test]
    fn the_queue_keeps_the_newest_packets() {
        let mut h = Hub::new();
        h.set_keys(true, false, Some(KEYS));
        let p: Vec<RawPacket> = (0..QUEUE_CAP as u16 + 10).map(key).collect();
        batch(&mut h, &p);
        let got = h.take();
        assert_eq!(got.len(), QUEUE_CAP);
        assert_eq!(got[0], key(10));
        // nothing needed any more: the queue goes
        batch(&mut h, &[key(1)]);
        h.set_keys(false, false, None);
        assert!(h.take().is_empty());
    }

    #[test]
    fn registration_is_the_union_of_both_clients() {
        let mut h = Hub::new();
        assert_eq!(h.wanted(), (false, false));
        assert!(h.changes().is_empty());
        h.set_keys(false, true, Some(KEYS));
        assert_eq!(h.changes(), vec![(USAGE_MOUSE, true)]);
        h.registered = h.wanted();
        h.set_activity(Some(ACT));
        assert_eq!(h.wanted(), (true, true));
        assert_eq!(h.changes(), vec![(USAGE_KEYBOARD, true)], "the mouse is registered already");
        h.registered = h.wanted();
        // the keys let go: the activity watcher still needs both, nothing is removed
        h.set_keys(false, false, None);
        assert!(h.changes().is_empty());
        h.set_activity(None);
        assert_eq!(h.changes(), vec![(USAGE_KEYBOARD, false), (USAGE_MOUSE, false)]);
    }

    #[test]
    fn activity_fires_once_on_any_packet_then_disarms() {
        let mut h = Hub::new();
        h.set_keys(false, true, Some(KEYS));
        h.registered = (false, true);
        h.set_activity(Some(ACT));
        h.registered = h.wanted();
        let out = batch(&mut h, &[mouse(0), mouse(0), key(0x41)]);
        assert_eq!(out, Posts { keys: None, activity: Some(ACT), sounds: vec![] }, "a move is enough; the key isn't the keys' device");
        assert_eq!(batch(&mut h, &[mouse(0)]).activity, None, "once only");
        // the keyboard is dropped again, the keys' mouse stays
        assert_eq!(h.changes(), vec![(USAGE_KEYBOARD, false)]);
    }

    #[test]
    fn a_refused_change_puts_the_old_needs_back() {
        let mut h = Hub::new();
        h.set_keys(false, true, Some(KEYS));
        h.registered = h.wanted();
        let old = h.set_keys(true, true, Some(KEYS));
        h.restore_keys(old);
        assert!(h.changes().is_empty());
        assert_eq!(h.set_activity(Some(ACT)), None);
        assert_eq!(h.set_activity(None), Some(ACT));
    }

    fn kdown(vk: u16, make: u16) -> RawPacket {
        RawPacket::Key { vk, make, flags: 0 }
    }
    fn kup(vk: u16, make: u16) -> RawPacket {
        RawPacket::Key { vk, make, flags: RI_KEY_BREAK }
    }
    fn ev(class: SoundClass, down: bool) -> SoundEvent {
        SoundEvent { class, down }
    }

    #[test]
    fn sounds_hear_class_and_direction_only() {
        let mut h = Hub::new();
        h.set_sound(true);
        let out = batch(&mut h, &[kdown(0x41, 0x1E), kup(0x41, 0x1E), kdown(0x20, 0x39), kup(0x20, 0x39), kdown(0x0D, 0x1C), kdown(0x08, 0x0E)]);
        assert_eq!(
            out.sounds,
            vec![
                ev(SoundClass::Other, true),
                ev(SoundClass::Other, false),
                ev(SoundClass::Space, true),
                ev(SoundClass::Space, false),
                ev(SoundClass::Enter, true),
                ev(SoundClass::Backspace, true)
            ]
        );
        assert_eq!(out.keys, None, "the sounds never wake the keys client");
    }

    #[test]
    fn a_held_keys_auto_repeat_is_one_sound() {
        let mut h = Hub::new();
        h.set_sound(true);
        let out = batch(&mut h, &[kdown(0x41, 0x1E), kdown(0x41, 0x1E), kdown(0x41, 0x1E), kdown(0x42, 0x30), kup(0x41, 0x1E), kdown(0x41, 0x1E)]);
        let downs = out.sounds.iter().filter(|e| e.down).count();
        assert_eq!(downs, 3, "A once, B once, A again after its release; the repeats are silent");
        // an extended key has its own scan code: the right Ctrl isn't the left one
        let out = batch(&mut h, &[kdown(0x11, 0x1D), RawPacket::Key { vk: 0x11, make: 0x1D, flags: RI_KEY_E0 }]);
        assert_eq!(out.sounds.len(), 2);
    }

    #[test]
    fn fake_keys_and_the_pause_sequence_are_ignored() {
        let mut h = Hub::new();
        h.set_sound(true);
        let out = batch(&mut h, &[kdown(0xFF, 0), kdown(0, 0), RawPacket::Key { vk: 0x13, make: 0x1D, flags: RI_KEY_E1 }]);
        assert!(out.sounds.is_empty());
        assert_eq!(h.held_count(), 0);
    }

    /// Order 058's promise: the key is used to pick the sound and forgotten. After a whole text was typed (and every key
    /// released) the sound client's state is empty, no packet waited in any queue, and the event type has no key in it.
    #[test]
    fn nothing_keeps_the_keys() {
        let mut h = Hub::new();
        h.set_sound(true); // the sounds ONLY: no keys client at all
        let text = "the quick brown fox - hunter2";
        let mut seen = 0usize;
        for (i, c) in text.chars().enumerate() {
            let vk = 0x41 + (c as u16 % 26);
            let make = 0x10 + (i as u16 % 40);
            let out = batch(&mut h, &[kdown(vk, make), kup(vk, make)]);
            seen += out.sounds.len();
        }
        assert_eq!(seen, text.chars().count() * 2);
        assert_eq!(h.held_count(), 0, "every key was released: nothing is left in the repeat filter");
        assert_eq!(h.queued(), 0, "the sounds never put a packet in the keys queue");
        assert!(h.take().is_empty());
        // held right now (a state, not a history) ...
        batch(&mut h, &[kdown(0x41, 0x1E), kdown(0x42, 0x30)]);
        assert_eq!(h.held_count(), 2);
        // ... and gone the moment the sounds stop listening
        h.set_sound(false);
        assert_eq!(h.held_count(), 0);
        let out = batch(&mut h, &[kdown(0x41, 0x1E)]);
        assert!(out.sounds.is_empty(), "off = nothing is heard");
        assert_eq!(h.held_count(), 0, "off = nothing is kept");
        // the event is exactly (class, down): two bytes of information and no key
        assert_eq!(std::mem::size_of::<SoundEvent>(), 2);
    }

    #[test]
    fn the_sounds_need_the_keyboard_registered_and_let_go_when_off() {
        let mut h = Hub::new();
        assert_eq!(h.wanted(), (false, false));
        h.set_sound(true);
        assert_eq!(h.wanted(), (true, false));
        assert_eq!(h.changes(), vec![(USAGE_KEYBOARD, true)]);
        h.registered = h.wanted();
        // the keys client shares the keyboard: removing the sounds keeps it registered
        h.set_keys(true, false, Some(KEYS));
        h.set_sound(false);
        assert!(h.changes().is_empty());
        h.set_keys(false, false, None);
        assert_eq!(h.changes(), vec![(USAGE_KEYBOARD, false)]);
    }
}

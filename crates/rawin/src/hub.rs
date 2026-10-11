//! The pure part (no Windows calls): which devices must be registered, which packets go to which client, and the
//! "one wake-up per batch" rule. The Windows shell (`win.rs`) keeps one [`Hub`] behind a mutex.

use std::collections::VecDeque;

/// Where a client wants its message: (window handle as an integer, message number).
pub type Target = (isize, u32);

/// HID usage page "generic desktop" and its two usages we register.
pub const USAGE_PAGE_GENERIC: u16 = 1;
pub const USAGE_MOUSE: u16 = 2;
pub const USAGE_KEYBOARD: u16 = 6;

/// The longest "ignore repeats" window the sounds accept (ms).
pub const MAX_CHATTER_MS: u32 = 400;

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

/// The class of a key for the key SOUNDS (Order 058): Space, Enter and Backspace sound different from the rest of a pack;
/// every other key is `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundClass {
    Other,
    Space,
    Enter,
    Backspace,
}

/// Which mouse button a click sound is for (Order 064): the two main buttons, the wheel click, and "side" (X1 and X2 alike).
/// Wheel TURNS are not clicks and are never heard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButtonClass {
    Left,
    Right,
    Middle,
    Side,
}

/// One mouse button going down or up, as the mouse sounds hear it: which class of button and up / down - nothing else
/// (no position, no time, no device).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseSoundEvent {
    pub button: MouseButtonClass,
    pub down: bool,
    /// Which of the five buttons (Order 090: a button's own sound): [`MOUSE_LEFT`] .. [`MOUSE_X2`].
    pub index: u8,
}

/// The mouse buttons' numbers in [`MouseSoundEvent::index`] (Order 090).
pub const MOUSE_LEFT: u8 = 0;
pub const MOUSE_RIGHT: u8 = 1;
pub const MOUSE_MIDDLE: u8 = 2;
/// Back (side).
pub const MOUSE_X1: u8 = 3;
/// Forward (side).
pub const MOUSE_X2: u8 = 4;

/// RAWMOUSE.usButtonFlags of the five buttons: (down flag, up flag, chatter code, class). The wheels (0x0400 / 0x0800) are
/// not here on purpose.
/// The chatter code is also the button's number ([`MOUSE_LEFT`] .. [`MOUSE_X2`]).
const MOUSE_BUTTONS: [(u16, u16, u8, MouseButtonClass); 5] = [
    (0x0001, 0x0002, 0, MouseButtonClass::Left),
    (0x0004, 0x0008, 1, MouseButtonClass::Right),
    (0x0010, 0x0020, 2, MouseButtonClass::Middle),
    (0x0040, 0x0080, 3, MouseButtonClass::Side),
    (0x0100, 0x0200, 4, MouseButtonClass::Side),
];

/// HID usages (page "generic desktop") of the two kinds of game controller Windows lists.
pub const USAGE_JOYSTICK: u16 = 4;
pub const USAGE_GAMEPAD: u16 = 5;

/// What a controller sound is for (Order 081): any button (face, bumpers, D-pad, stick clicks, the rest) or one of the two
/// triggers. Never which button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadSoundClass {
    Button,
    TriggerLeft,
    TriggerRight,
}

/// One controller button / trigger going down or up, as the controller sounds hear it: the class and up / down - nothing else
/// (no button number, no device, no time).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PadSoundEvent {
    pub class: PadSoundClass,
    pub down: bool,
    /// Which button (Order 090: a button's own sound), one of the [`pad`] numbers; [`pad::UNKNOWN`] on a controller whose layout
    /// is not known.
    pub button: u8,
}

/// The controller buttons' numbers in [`PadSoundEvent::button`] (Order 090), by place on the pad - the same on a PlayStation and
/// an Xbox controller (Cross = A).
pub mod pad {
    pub const SOUTH: u8 = 0;
    pub const EAST: u8 = 1;
    pub const WEST: u8 = 2;
    pub const NORTH: u8 = 3;
    pub const LB: u8 = 4;
    pub const RB: u8 = 5;
    pub const LT: u8 = 6;
    pub const RT: u8 = 7;
    /// Share / Create / View.
    pub const BACK: u8 = 8;
    /// Options / Menu.
    pub const START: u8 = 9;
    pub const LS: u8 = 10;
    pub const RS: u8 = 11;
    /// PS / Xbox button.
    pub const HOME: u8 = 12;
    pub const TOUCHPAD: u8 = 13;
    pub const DPAD_UP: u8 = 14;
    pub const DPAD_RIGHT: u8 = 15;
    pub const DPAD_DOWN: u8 = 16;
    pub const DPAD_LEFT: u8 = 17;
    /// The DualSense mic button.
    pub const MIC: u8 = 18;
    pub const UNKNOWN: u8 = 255;
    pub const VID_SONY: u16 = 0x054C;
    pub const VID_MICROSOFT: u16 = 0x045E;

    /// The button number of HID button bit `bit` (0 = HID button 1; 56..=59 = the D-pad) on a pad made by `vendor`.
    pub fn of(vendor: u16, bit: u32) -> u8 {
        match bit {
            56 => return DPAD_UP,
            57 => return DPAD_RIGHT,
            58 => return DPAD_DOWN,
            59 => return DPAD_LEFT,
            _ => {}
        }
        let table: &[u8] = match vendor {
            // DualShock 4 / DualSense / DualSense Edge (DirectInput order)
            VID_SONY => &[WEST, SOUTH, EAST, NORTH, LB, RB, LT, RT, BACK, START, LS, RS, HOME, TOUCHPAD, MIC],
            // Xbox controllers over Bluetooth / HID
            VID_MICROSOFT => &[SOUTH, EAST, WEST, NORTH, LB, RB, BACK, START, LS, RS, HOME],
            _ => &[],
        };
        table.get(bit as usize).copied().unwrap_or(UNKNOWN)
    }
}

/// One decoded input report of one controller (made by the Windows shell from the HID report; this part is pure).
/// `keys`: bit n = button n+1 is held (bits 0..=55), bits 56..=59 = D-pad up / right / down / left. `trig`: bit 0 = left
/// trigger, bit 1 = right trigger held, for pads whose triggers are buttons. `analog`: the two triggers as 0..=255 for pads
/// whose triggers are axes (then `trig` is unused).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PadFrame {
    /// The device (its raw-input handle; never 0).
    pub dev: usize,
    /// Its maker's USB vendor id (which button is which: [`pad::of`]).
    pub vendor: u16,
    pub keys: u64,
    pub trig: u8,
    pub analog: Option<(u8, u8)>,
}

/// A trigger axis counts as pressed from this value up and as released below the lower one (so a trigger resting on the
/// threshold can't chatter).
pub const TRIGGER_DOWN: u8 = 96;
pub const TRIGGER_UP: u8 = 64;
/// How many controllers are told apart at once (a fifth takes the place of the one seen longest ago).
pub const MAX_PADS: usize = 4;

/// What the controller sounds know about one pad: which buttons are held right now (a state, never a history).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct PadHeld {
    dev: usize,
    vendor: u16,
    keys: u64,
    trig: u8,
}

/// Counters for the check "can the app see the controller's presses" (nothing about WHICH button).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PadStats {
    /// Controller reports read since the sounds listen.
    pub packets: u64,
    /// ... of which the shell could decode (a report it does not know is counted in `packets` only).
    pub decoded: u64,
    /// Different controllers that sent a report since the sounds listen.
    pub devices: u32,
    /// Button / trigger events handed out.
    pub events: u64,
}

/// One key going down or up, as the sounds hear it: its class, up / down and (Order 090: a key's own sound, a pack made from
/// one sound) its place on the keyboard - the scan code as the remap writes it (0xE000 added for an extended key). The sound
/// engine uses it at once to pick the sound; it is never stored, logged or sent anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundEvent {
    pub class: SoundClass,
    pub down: bool,
    pub key: u16,
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

/// The sound client's chatter filter (Order 059, "Ignore repeats within __ ms"): a key that comes down twice within the
/// window - a worn switch bouncing - plays one sound. It keeps the last [`Chatter::SLOTS`] key presses that PLAYED as
/// (scan code, millisecond) and only while the window is on (0 = off = nothing kept at all); an entry older than the window
/// counts for nothing and is overwritten. A press that was silenced also silences its release (bit map `quiet`). Like
/// [`Held`] it is a few bytes of state, never a log, never leaves the raw thread, and is wiped when the sounds stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Chatter {
    ms: u32,
    slots: [(u8, u32); Chatter::SLOTS],
    quiet: Held,
}

impl Chatter {
    const SLOTS: usize = 6;
    const fn new() -> Self {
        Chatter { ms: 0, slots: [(0, 0); Chatter::SLOTS], quiet: Held::new() }
    }
    /// A fresh press at `now`: false = a repeat inside the window (silence it).
    fn down(&mut self, code: u8, now: u32) -> bool {
        // time 0 = an empty slot (the clock starts at 1)
        let now = now.max(1);
        if let Some(i) = self.slots.iter().position(|&(c, t)| c == code && t != 0) {
            if now.wrapping_sub(self.slots[i].1) < self.ms {
                self.quiet.press(code);
                return false;
            }
            self.slots[i].1 = now;
            return true;
        }
        // the slot that is empty (time 0) or the oldest
        let i = (0..Self::SLOTS).min_by_key(|&i| self.slots[i].1).unwrap_or(0);
        self.slots[i] = (code, now);
        true
    }
    /// A release: false = its press was silenced, so it is too.
    fn up(&mut self, code: u8) -> bool {
        let (w, m) = Held::bit(code);
        let was = self.quiet.0[w] & m != 0;
        self.quiet.release(code);
        !was
    }
}

/// Milliseconds on a clock that starts at the first call (only asked while the chatter window is on).
fn now_ms() -> u32 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u32
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
    /// Mouse button sounds to play (Order 064), same rules as `sounds`.
    pub mouse_sounds: Vec<MouseSoundEvent>,
    /// Controller sounds to play (Order 081), same rules as `sounds`.
    pub pad_sounds: Vec<PadSoundEvent>,
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
    /// The chatter filter (see [`Chatter`]); off (0 ms) = nothing is kept.
    chatter: Chatter,
    /// The mouse-sound client listens (Order 064): button packets become [`MouseSoundEvent`]s. Moves and wheel turns are only
    /// counted; nothing is kept but the chatter filter below.
    msound: bool,
    /// The chatter filter of the mouse buttons (same window as the keys', buttons coded 0-4); empty while nobody listens.
    mchatter: Chatter,
    /// What is registered with Windows now (keyboard, mouse).
    pub registered: (bool, bool),
    /// The controller-sound client listens (Order 081): controller reports become [`PadSoundEvent`]s.
    psound: bool,
    /// What is registered with Windows for the controllers now (joystick + gamepad usage).
    pub registered_pad: bool,
    /// Which buttons each known controller holds right now (empty while nobody listens).
    pads: [PadHeld; MAX_PADS],
    /// Next slot a new controller takes when all are in use.
    pad_next: usize,
    pad_stats: PadStats,
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
            chatter: Chatter::new(),
            msound: false,
            mchatter: Chatter::new(),
            registered: (false, false),
            psound: false,
            registered_pad: false,
            pads: [PadHeld { dev: 0, vendor: 0, keys: 0, trig: 0 }; MAX_PADS],
            pad_next: 0,
            pad_stats: PadStats { packets: 0, decoded: 0, devices: 0, events: 0 },
            queue: VecDeque::new(),
            posted: false,
            stats: Stats { packets_seen: 0, packets_forwarded: 0, wakes_posted: 0, batches: 0 },
        }
    }

    /// (keyboard, mouse) Windows must deliver: what the keys need, the keyboard while the key sounds listen, the mouse while
    /// the mouse sounds listen, and both while the activity watcher is armed.
    pub fn wanted(&self) -> (bool, bool) {
        let a = self.activity.is_some();
        (self.keys.keyboard || self.sound || a, self.keys.mouse || self.msound || a)
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
        if self.psound != self.registered_pad {
            out.push((USAGE_GAMEPAD, self.psound));
            out.push((USAGE_JOYSTICK, self.psound));
        }
        out
    }

    /// The controller-sound client listens (true) or lets go (false). Returns the old setting. Either way what is known
    /// about the pads' buttons starts empty: nothing about them survives a change.
    pub fn set_pad_sound(&mut self, on: bool) -> bool {
        self.pads = [PadHeld::default(); MAX_PADS];
        self.pad_next = 0;
        self.pad_stats = PadStats::default();
        std::mem::replace(&mut self.psound, on)
    }

    /// The controller sounds listen right now.
    pub fn pad_listening(&self) -> bool {
        self.psound
    }

    pub fn pad_stats(&self) -> PadStats {
        self.pad_stats
    }

    /// A controller report that was read but could not be decoded (counted, nothing else).
    pub fn pad_undecoded(&mut self, _dev: usize) {
        // (no slot is taken: a device nobody can read must not push a real pad out)
        self.pad_stats.packets += 1;
    }

    fn pad_slot(&mut self, dev: usize, vendor: u16) -> usize {
        if let Some(i) = self.pads.iter().position(|p| p.dev == dev) {
            return i;
        }
        let i = self.pads.iter().position(|p| p.dev == 0).unwrap_or_else(|| {
            let i = self.pad_next;
            self.pad_next = (i + 1) % MAX_PADS;
            i
        });
        self.pads[i] = PadHeld { dev, vendor, keys: 0, trig: 0 };
        self.pad_stats.devices += 1;
        i
    }

    /// One decoded report: the buttons / triggers that changed since this pad's last report become events (a release
    /// before a press, in button order). Reports that change nothing - a pad streams them all the time - add nothing.
    #[inline]
    pub fn feed_pad(&mut self, f: PadFrame, out: &mut Posts) {
        if !self.psound || f.dev == 0 {
            return;
        }
        self.pad_stats.packets += 1;
        self.pad_stats.decoded += 1;
        let i = self.pad_slot(f.dev, f.vendor);
        let was = self.pads[i];
        let trig = match f.analog {
            Some((l, r)) => {
                let mut t = was.trig;
                for (bit, v) in [(1u8, l), (2u8, r)] {
                    if v >= TRIGGER_DOWN {
                        t |= bit;
                    } else if v <= TRIGGER_UP {
                        t &= !bit;
                    }
                }
                t
            }
            None => f.trig & 3,
        };
        if f.keys == was.keys && trig == was.trig {
            return;
        }
        self.pads[i].keys = f.keys;
        self.pads[i].trig = trig;
        let before = out.pad_sounds.len();
        let (up, down) = (was.keys & !f.keys, f.keys & !was.keys);
        let vendor = self.pads[i].vendor;
        let bits = |m: u64| (0..64u32).filter(move |b| m & (1u64 << b) != 0);
        for b in bits(up) {
            out.pad_sounds.push(PadSoundEvent { class: PadSoundClass::Button, down: false, button: pad::of(vendor, b) });
        }
        const TRIGS: [(u8, PadSoundClass, u8); 2] = [(1, PadSoundClass::TriggerLeft, pad::LT), (2, PadSoundClass::TriggerRight, pad::RT)];
        for (bit, class, button) in TRIGS {
            if was.trig & bit != 0 && trig & bit == 0 {
                out.pad_sounds.push(PadSoundEvent { class, down: false, button });
            }
        }
        for (bit, class, button) in TRIGS {
            if was.trig & bit == 0 && trig & bit != 0 {
                out.pad_sounds.push(PadSoundEvent { class, down: true, button });
            }
        }
        for b in bits(down) {
            out.pad_sounds.push(PadSoundEvent { class: PadSoundClass::Button, down: true, button: pad::of(vendor, b) });
        }
        self.pad_stats.events += (out.pad_sounds.len() - before) as u64;
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
        self.chatter = Chatter { ms: self.chatter.ms, ..Chatter::new() };
        std::mem::replace(&mut self.sound, on)
    }

    /// "Ignore repeats within `ms`" for the sounds (0 = off, at most [`MAX_CHATTER_MS`]). Starts the filter empty.
    pub fn set_chatter(&mut self, ms: u32) {
        let ms = ms.min(MAX_CHATTER_MS);
        if ms != self.chatter.ms {
            self.chatter = Chatter { ms, ..Chatter::new() };
        }
        if ms != self.mchatter.ms {
            self.mchatter = Chatter { ms, ..Chatter::new() };
        }
    }

    /// The mouse-sound client listens (true) or lets go (false). Returns the old setting. Either way its repeat filter starts
    /// empty: nothing about clicks survives a change.
    pub fn set_mouse_sound(&mut self, on: bool) -> bool {
        self.mchatter = Chatter { ms: self.mchatter.ms, ..Chatter::new() };
        std::mem::replace(&mut self.msound, on)
    }

    /// The button events in one mouse packet (a down before its up, in button order). A very fast click is a down and an up
    /// in ONE packet; a pure move or a wheel turn has none. With the chatter window on, a button that goes down again
    /// inside it is silent, and so is its release.
    #[inline]
    fn mouse_sounds_of(&mut self, buttons: u16, now: u32, out: &mut Vec<MouseSoundEvent>) {
        for (down, up, code, button) in MOUSE_BUTTONS {
            if buttons & down != 0 && (self.mchatter.ms == 0 || self.mchatter.down(code, now)) {
                out.push(MouseSoundEvent { button, down: true, index: code });
            }
            if buttons & up != 0 && (self.mchatter.ms == 0 || self.mchatter.up(code)) {
                out.push(MouseSoundEvent { button, down: false, index: code });
            }
        }
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
    /// E1 sequence). Only the class, up / down and the key's scan code leave (for the key's own sound).
    #[inline]
    fn sound_of(&mut self, vk: u16, make: u16, flags: u16, now: u32) -> Option<SoundEvent> {
        if vk == 0 || vk >= 0xFF || flags & RI_KEY_E1 != 0 {
            return None;
        }
        let code = (make & 0x7F) as u8 | if flags & RI_KEY_E0 != 0 { 0x80 } else { 0 };
        let key = (make & 0x7F) | if flags & RI_KEY_E0 != 0 { 0xE000 } else { 0 };
        if flags & RI_KEY_BREAK != 0 {
            self.held.release(code);
            if self.chatter.ms > 0 && !self.chatter.up(code) {
                return None;
            }
            Some(SoundEvent { class: sound_class(vk), down: false, key })
        } else if self.held.press(code) {
            if self.chatter.ms > 0 && !self.chatter.down(code, now) {
                return None;
            }
            Some(SoundEvent { class: sound_class(vk), down: true, key })
        } else {
            None
        }
    }

    /// One packet of a batch. Pure moves are only counted; key packets / mouse buttons and wheel go to the keys client
    /// while it needs that device; the first packet of any kind fires (and disarms) the activity watcher.
    #[inline]
    pub fn feed(&mut self, p: RawPacket, out: &mut Posts) {
        let now = if (self.sound || self.msound) && self.chatter.ms > 0 { now_ms() } else { 0 };
        self.feed_at(p, out, now);
    }

    /// [`Hub::feed`] with the clock (ms) given: what the tests use.
    #[inline]
    pub fn feed_at(&mut self, p: RawPacket, out: &mut Posts, now: u32) {
        self.stats.packets_seen += 1;
        if let Some(t) = self.activity.take() {
            out.activity = Some(t);
        }
        if self.sound {
            if let RawPacket::Key { vk, make, flags } = p {
                if let Some(e) = self.sound_of(vk, make, flags, now) {
                    out.sounds.push(e);
                }
            }
        }
        if self.msound {
            if let RawPacket::Mouse { buttons } = p {
                if buttons != 0 {
                    self.mouse_sounds_of(buttons, now, &mut out.mouse_sounds);
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
        assert_eq!(out, Posts { keys: None, activity: Some(ACT), ..Posts::default() }, "a move is enough; the key isn't the keys' device");
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
    /// A key's event with its scan code (`make` as the test packets give it, no E0).
    fn evk(class: SoundClass, down: bool, key: u16) -> SoundEvent {
        SoundEvent { class, down, key }
    }
    fn ev(class: SoundClass, down: bool) -> SoundEvent {
        evk(class, down, 0x1E)
    }

    /// Order 090: class, direction and the key's place (its scan code, 0xE000 for an extended key) - for the key's own sound.
    #[test]
    fn sounds_hear_class_direction_and_the_keys_place() {
        let mut h = Hub::new();
        h.set_sound(true);
        let out = batch(&mut h, &[kdown(0x41, 0x1E), kup(0x41, 0x1E), kdown(0x20, 0x39), kup(0x20, 0x39), kdown(0x0D, 0x1C), kdown(0x08, 0x0E)]);
        assert_eq!(
            out.sounds,
            vec![
                evk(SoundClass::Other, true, 0x1E),
                evk(SoundClass::Other, false, 0x1E),
                evk(SoundClass::Space, true, 0x39),
                evk(SoundClass::Space, false, 0x39),
                evk(SoundClass::Enter, true, 0x1C),
                evk(SoundClass::Backspace, true, 0x0E)
            ]
        );
        assert_eq!(out.keys, None, "the sounds never wake the keys client");
        // numpad Enter (E0 1C) is told apart from Enter
        let out = batch(&mut h, &[RawPacket::Key { vk: 0x0D, make: 0x1C, flags: RI_KEY_E0 }]);
        assert_eq!(out.sounds, vec![evk(SoundClass::Enter, true, 0xE01C)]);
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
        // the event is (class, down, scan code): nothing more of the key, and nothing of the text it typed
        assert_eq!(std::mem::size_of::<SoundEvent>(), 4);
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

    fn batch_at(h: &mut Hub, packets: &[(RawPacket, u32)]) -> Vec<SoundEvent> {
        let mut out = Posts::default();
        for (p, t) in packets {
            h.feed_at(*p, &mut out, *t);
        }
        out.sounds
    }

    /// Order 059: a bouncing key (down, up, down, up within the window) plays ONE down and ONE up; another key meanwhile and
    /// the same key after the window both play; 0 = off keeps nothing.
    #[test]
    fn the_chatter_window_silences_a_bouncing_key_only() {
        let mut h = Hub::new();
        h.set_sound(true);
        h.set_chatter(40);
        let s = batch_at(&mut h, &[(kdown(0x41, 0x1E), 100), (kup(0x41, 0x1E), 108), (kdown(0x41, 0x1E), 120), (kup(0x41, 0x1E), 126)]);
        assert_eq!(s, vec![ev(SoundClass::Other, true), ev(SoundClass::Other, false)], "the bounce is silent, down and up");
        // B inside A's window is another key: it plays
        let s = batch_at(&mut h, &[(kdown(0x42, 0x30), 130), (kup(0x42, 0x30), 135)]);
        assert_eq!(s.len(), 2);
        // A again after the window (measured from its last played press at 100): plays
        let s = batch_at(&mut h, &[(kdown(0x41, 0x1E), 150), (kup(0x41, 0x1E), 155)]);
        assert_eq!(s.len(), 2);
        // a real second press 30 ms after the last played one is the chatter too (that is what the window means)
        let s = batch_at(&mut h, &[(kdown(0x41, 0x1E), 170), (kup(0x41, 0x1E), 172)]);
        assert!(s.is_empty());
        assert_eq!(h.held_count(), 0);
    }

    /// A mouse event without its button number (the class tests); [`mouse_index_batch`] keeps it.
    fn me(button: MouseButtonClass, down: bool) -> MouseSoundEvent {
        MouseSoundEvent { button, down, index: 0 }
    }
    fn mouse_index_batch(h: &mut Hub, packets: &[(u16, u32)]) -> Vec<MouseSoundEvent> {
        let mut out = Posts::default();
        for (b, t) in packets {
            h.feed_at(mouse(*b), &mut out, *t);
        }
        assert!(out.sounds.is_empty(), "a mouse packet is never a key sound");
        out.mouse_sounds
    }
    fn mouse_batch(h: &mut Hub, packets: &[(u16, u32)]) -> Vec<MouseSoundEvent> {
        mouse_index_batch(h, packets).into_iter().map(|e| MouseSoundEvent { index: 0, ..e }).collect()
    }

    /// Order 090: every mouse event says which of the five buttons it is (Back and Forward told apart).
    #[test]
    fn mouse_sounds_say_which_button() {
        let mut h = Hub::new();
        h.set_mouse_sound(true);
        let s = mouse_index_batch(&mut h, &[(0x0001, 0), (0x0004, 0), (0x0010, 0), (0x0040, 0), (0x0100, 0)]);
        let idx: Vec<u8> = s.iter().map(|e| e.index).collect();
        assert_eq!(idx, vec![MOUSE_LEFT, MOUSE_RIGHT, MOUSE_MIDDLE, MOUSE_X1, MOUSE_X2]);
    }

    /// Order 064: the mouse sounds hear which class of button went down / up and nothing else; moves and wheel turns are silent.
    #[test]
    fn mouse_sounds_hear_the_button_class_only() {
        use MouseButtonClass::*;
        let mut h = Hub::new();
        assert_eq!(h.wanted(), (false, false));
        h.set_mouse_sound(true);
        assert_eq!(h.wanted(), (false, true), "the mouse only, never the keyboard");
        let s = mouse_batch(&mut h, &[(0, 0), (0x0001, 0), (0x0002, 0), (0x0004, 0), (0x0008, 0), (0x0010, 0), (0x0020, 0), (0x0040, 0), (0x0080, 0), (0x0100, 0), (0x0200, 0), (0x0400, 0), (0x0800, 0)]);
        assert_eq!(
            s,
            vec![me(Left, true), me(Left, false), me(Right, true), me(Right, false), me(Middle, true), me(Middle, false), me(Side, true), me(Side, false), me(Side, true), me(Side, false)],
            "X1 and X2 are both Side; the wheel turning (0x400 / 0x800) and a plain move make no sound"
        );
        // a very fast click is a down and an up in one packet; a turn together with a button still sounds the button
        assert_eq!(mouse_batch(&mut h, &[(0x0003, 0)]), vec![me(Left, true), me(Left, false)]);
        assert_eq!(mouse_batch(&mut h, &[(0x0401, 0)]), vec![me(Left, true)]);
        // the key sounds are untouched, and the keys client gets no wake-up from the mouse sounds
        let mut out = Posts::default();
        h.feed_at(mouse(0x0001), &mut out, 0);
        assert_eq!((out.keys, out.sounds.len()), (None, 0));
    }

    #[test]
    fn mouse_sounds_off_hear_nothing_and_keep_nothing() {
        let mut h = Hub::new();
        assert!(mouse_batch(&mut h, &[(0x0001, 5), (0x0002, 6)]).is_empty());
        h.set_mouse_sound(true);
        h.set_chatter(50);
        mouse_batch(&mut h, &[(0x0001, 100)]);
        assert!(h.set_mouse_sound(false));
        assert_eq!(h.wanted(), (false, false), "nothing is registered any more");
        assert!(mouse_batch(&mut h, &[(0x0001, 110)]).is_empty());
        // on again: what the filter knew is gone
        h.set_mouse_sound(true);
        assert_eq!(mouse_batch(&mut h, &[(0x0001, 120)]).len(), 1);
        assert_eq!(h.queued(), 0, "the mouse sounds never put a packet in the keys queue");
    }

    /// Order 064: the chatter window works on the buttons too - a bouncing click plays one down and one up, another button
    /// meanwhile and the same button after the window both play, 0 = off plays everything.
    #[test]
    fn the_chatter_window_silences_a_bouncing_mouse_button_only() {
        use MouseButtonClass::*;
        let mut h = Hub::new();
        h.set_mouse_sound(true);
        let s = mouse_batch(&mut h, &[(0x0001, 100), (0x0002, 103), (0x0001, 105), (0x0002, 107)]);
        assert_eq!(s.len(), 4, "0 ms = off: every click plays");
        h.set_chatter(40);
        let s = mouse_batch(&mut h, &[(0x0001, 200), (0x0002, 208), (0x0001, 220), (0x0002, 226)]);
        assert_eq!(s, vec![me(Left, true), me(Left, false)], "the bounce is silent, down and up");
        let s = mouse_batch(&mut h, &[(0x0004, 230), (0x0008, 235)]);
        assert_eq!(s.len(), 2, "the other button is another switch");
        let s = mouse_batch(&mut h, &[(0x0001, 250), (0x0002, 255)]);
        assert_eq!(s.len(), 2, "after the window the same button plays again");
        // X1 and X2 are two switches although they sound alike
        let s = mouse_batch(&mut h, &[(0x0040, 300), (0x0080, 302), (0x0100, 310), (0x0200, 312)]);
        assert_eq!(s.len(), 4);
        // the keys' filter and the mouse's are separate: a key and a button at the same moment both play
        h.set_sound(true);
        let mut out = Posts::default();
        h.feed_at(kdown(0x41, 0x00), &mut out, 400);
        h.feed_at(mouse(0x0001), &mut out, 401);
        assert_eq!((out.sounds.len(), out.mouse_sounds.len()), (1, 1));
    }

    #[test]
    fn chatter_off_plays_everything_and_stopping_the_sounds_wipes_it() {
        let mut h = Hub::new();
        h.set_sound(true);
        let s = batch_at(&mut h, &[(kdown(0x41, 0x1E), 100), (kup(0x41, 0x1E), 101), (kdown(0x41, 0x1E), 102), (kup(0x41, 0x1E), 103)]);
        assert_eq!(s.len(), 4, "0 ms = off: every press plays");
        h.set_chatter(80);
        batch_at(&mut h, &[(kdown(0x41, 0x1E), 200)]);
        h.set_sound(false);
        h.set_sound(true);
        let s = batch_at(&mut h, &[(kup(0x41, 0x1E), 205), (kdown(0x41, 0x1E), 210)]);
        assert_eq!(s.len(), 2, "what was kept is gone when the sounds were switched off and on");
        h.set_chatter(1000);
        assert_eq!(h.chatter.ms, MAX_CHATTER_MS);
    }

    /// A pad event without its button number (the class tests); [`pad_ids`] keeps it.
    fn pe(class: PadSoundClass, down: bool) -> PadSoundEvent {
        PadSoundEvent { class, down, button: 0 }
    }
    fn frame(dev: usize, keys: u64, trig: u8) -> PadFrame {
        PadFrame { dev, vendor: 0, keys, trig, analog: None }
    }
    fn analog(dev: usize, keys: u64, l: u8, r: u8) -> PadFrame {
        PadFrame { dev, vendor: 0, keys, trig: 0, analog: Some((l, r)) }
    }
    fn pad_ids(h: &mut Hub, frames: &[PadFrame]) -> Vec<PadSoundEvent> {
        let mut out = Posts::default();
        for f in frames {
            h.feed_pad(*f, &mut out);
        }
        assert!(out.sounds.is_empty() && out.mouse_sounds.is_empty() && out.keys.is_none(), "a pad is never a key or a mouse");
        out.pad_sounds
    }
    fn pad(h: &mut Hub, frames: &[PadFrame]) -> Vec<PadSoundEvent> {
        pad_ids(h, frames).into_iter().map(|e| PadSoundEvent { button: 0, ..e }).collect()
    }

    /// Order 090: a known pad's buttons are named by their place (Sony Cross = Xbox A = SOUTH); an unknown maker's are UNKNOWN.
    #[test]
    fn pad_events_say_which_button_on_known_pads() {
        let mut h = Hub::new();
        h.set_pad_sound(true);
        let sony = |keys, trig| PadFrame { dev: 5, vendor: pad::VID_SONY, keys, trig, analog: None };
        // HID button 2 = Cross, then the right trigger (Sony: buttons 7 / 8 arrive as trig bits), then D-pad left
        let s = pad_ids(&mut h, &[sony(0b10, 0), sony(0b10, 2), sony((1 << 59) | 0b10, 2)]);
        let ids: Vec<u8> = s.iter().map(|e| e.button).collect();
        assert_eq!(ids, vec![pad::SOUTH, pad::RT, pad::DPAD_LEFT]);
        let xbox = |keys| PadFrame { dev: 6, vendor: pad::VID_MICROSOFT, keys, trig: 0, analog: None };
        let s = pad_ids(&mut h, &[xbox(0b1), xbox(0b1001)]);
        assert_eq!(s.iter().map(|e| e.button).collect::<Vec<_>>(), vec![pad::SOUTH, pad::NORTH]);
        let other = |keys| PadFrame { dev: 7, vendor: 0x1234, keys, trig: 0, analog: None };
        assert_eq!(pad_ids(&mut h, &[other(0b1)])[0].button, pad::UNKNOWN);
    }

    /// Order 081: the controller sounds hear buttons and triggers going down / up, by class only; a pad streams a report all
    /// the time and only a change makes a sound.
    #[test]
    fn pad_sounds_hear_changes_only() {
        use PadSoundClass::*;
        let mut h = Hub::new();
        assert!(pad(&mut h, &[frame(7, 1, 0)]).is_empty(), "off: nothing is heard");
        assert_eq!(h.changes(), vec![], "and nothing is registered");
        h.set_pad_sound(true);
        assert_eq!(h.changes(), vec![(USAGE_GAMEPAD, true), (USAGE_JOYSTICK, true)]);
        assert_eq!(h.wanted(), (false, false), "the keyboard and the mouse are not touched");
        // 500 idle reports: silence
        let idle: Vec<PadFrame> = (0..500).map(|_| frame(7, 0, 0)).collect();
        assert!(pad(&mut h, &idle).is_empty());
        // a face button, held over many reports, down once and up once
        let s = pad(&mut h, &[frame(7, 0b10, 0), frame(7, 0b10, 0), frame(7, 0b10, 0), frame(7, 0, 0)]);
        assert_eq!(s, vec![pe(Button, true), pe(Button, false)]);
        // the D-pad (bits 56..) and a stick click are buttons too; two at once = two sounds
        let s = pad(&mut h, &[frame(7, (1 << 56) | (1 << 10), 0), frame(7, 1 << 10, 0), frame(7, 0, 0)]);
        assert_eq!(s, vec![pe(Button, true), pe(Button, true), pe(Button, false), pe(Button, false)]);
        // digital triggers: left, then right, each its own class; a release comes before a press in one report
        let s = pad(&mut h, &[frame(7, 0, 1), frame(7, 0, 3), frame(7, 0b1, 2)]);
        assert_eq!(s, vec![pe(TriggerLeft, true), pe(TriggerRight, true), pe(TriggerLeft, false), pe(Button, true)]);
        let s = pad(&mut h, &[frame(7, 0, 0)]);
        assert_eq!(s, vec![pe(Button, false), pe(TriggerRight, false)]);
        let st = h.pad_stats();
        assert_eq!((st.packets, st.decoded, st.devices), (511, 511, 1));
        assert_eq!(st.events, 2 + 4 + 4 + 2);
    }

    #[test]
    fn analog_triggers_have_a_gap_between_down_and_up() {
        use PadSoundClass::*;
        let mut h = Hub::new();
        h.set_pad_sound(true);
        let s = pad(&mut h, &[analog(3, 0, 0, 0), analog(3, 0, 50, 0), analog(3, 0, 95, 0), analog(3, 0, 96, 10), analog(3, 0, 255, 0)]);
        assert_eq!(s, vec![pe(TriggerLeft, true)], "down once at 96, a harder pull is the same press");
        // coming back through the gap (65..95) is still held; at 64 it is released
        let s = pad(&mut h, &[analog(3, 0, 80, 0), analog(3, 0, 96, 0), analog(3, 0, 70, 0), analog(3, 0, 64, 0)]);
        assert_eq!(s, vec![pe(TriggerLeft, false)]);
        let s = pad(&mut h, &[analog(3, 0, 0, 200), analog(3, 0, 0, 0)]);
        assert_eq!(s, vec![pe(TriggerRight, true), pe(TriggerRight, false)]);
    }

    #[test]
    fn two_pads_are_two_states_and_off_forgets_everything() {
        use PadSoundClass::*;
        let mut h = Hub::new();
        h.set_pad_sound(true);
        // pad 1 holds a button; pad 2 pressing its own is a press, not a change of pad 1's
        assert_eq!(pad(&mut h, &[frame(1, 1, 0)]), vec![pe(Button, true)]);
        assert_eq!(pad(&mut h, &[frame(2, 1, 0)]), vec![pe(Button, true)]);
        assert!(pad(&mut h, &[frame(1, 1, 0), frame(2, 1, 0)]).is_empty());
        assert_eq!(pad(&mut h, &[frame(1, 0, 0)]), vec![pe(Button, false)]);
        assert_eq!(h.pad_stats().devices, 2);
        // five pads: the oldest slot is reused, nothing panics
        for d in 3..=8 {
            pad(&mut h, &[frame(d, 0, 0)]);
        }
        // off: the held state is wiped, the registration goes, nothing is heard
        assert!(h.set_pad_sound(false));
        h.registered_pad = true;
        assert_eq!(h.changes(), vec![(USAGE_GAMEPAD, false), (USAGE_JOYSTICK, false)]);
        assert!(pad(&mut h, &[frame(2, 1, 0)]).is_empty());
        assert_eq!(h.pad_stats(), PadStats::default());
        // on again: a pad that held a button before shows it as a fresh press (nothing was remembered)
        h.set_pad_sound(true);
        assert_eq!(pad(&mut h, &[frame(2, 1, 0)]), vec![pe(Button, true)]);
        // a report the shell could not decode is only counted
        h.pad_undecoded(2);
        assert_eq!(h.pad_stats().packets, 2);
        assert_eq!(h.pad_stats().decoded, 1);
    }

    /// The event carries the class, the direction and (Order 090) which button by place - no device and no time.
    #[test]
    fn the_pad_event_is_class_direction_and_button_only() {
        assert_eq!(std::mem::size_of::<PadSoundEvent>(), 3);
    }
}

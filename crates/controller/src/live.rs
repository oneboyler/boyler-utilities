//! LIVE view: what the selected controller is doing right now (sticks, pressed buttons, triggers), read-only.
//!
//! A [`LiveView`] is a thread that exists only between `start` and `stop` (the page is open). For PlayStation pads it
//! blocks on the controller's own input reports (no timer: an overlapped HID read + a stop event); Xbox pads have no
//! events in XInput, so they are polled — ONLY while the view runs (measured cost in the report). Nothing is ever sent
//! to the controller (the device is opened for reading only).
//!
//! Report layouts (byte 0 = report id). Sources: Linux `hid-playstation` / `hid-sony` and SDL's `SDL_hidapi_ps5.c` /
//! `SDL_hidapi_ps4.c` (public driver code, not measured here); a real pad's reports were not recorded here. DualSense: USB `0x01` (64 bytes) and Bluetooth `0x31` (78 bytes) share one layout
//! starting at byte 1 / 2; Bluetooth before full mode sends a short `0x01` (10 bytes). DualShock 4: USB `0x01`, Bluetooth
//! `0x11` (data from byte 3).

use crate::error::Result;
use crate::os::{Battery, LiveEvent, PadInfo, PadOs};
use crate::parts::{ButtonId, PadKind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// One moment of the controller.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LiveState {
    /// Left stick, x right = +1, y DOWN = +1 (HID's direction), -1..1.
    pub left: (f32, f32),
    pub right: (f32, f32),
    /// Triggers 0..1.
    pub l2: f32,
    pub r2: f32,
    pub pressed: Vec<ButtonId>,
    /// Touchpad clicked (PlayStation).
    pub touchpad: bool,
    /// Battery when the report carries it.
    pub battery: Option<Battery>,
}

impl LiveState {
    pub fn is_pressed(&self, b: ButtonId) -> bool {
        self.pressed.contains(&b)
    }
}

fn axis(v: u8) -> f32 {
    (v as f32 - 127.5) / 127.5
}

fn hat(h: u8, out: &mut Vec<ButtonId>) {
    let (u, r, d, l) = match h & 0x0F {
        0 => (true, false, false, false),
        1 => (true, true, false, false),
        2 => (false, true, false, false),
        3 => (false, true, true, false),
        4 => (false, false, true, false),
        5 => (false, false, true, true),
        6 => (false, false, false, true),
        7 => (true, false, false, true),
        _ => (false, false, false, false),
    };
    for (on, b) in [(u, ButtonId::DpadUp), (r, ButtonId::DpadRight), (d, ButtonId::DpadDown), (l, ButtonId::DpadLeft)] {
        if on {
            out.push(b);
        }
    }
}

fn bits(byte: u8, map: &[(u8, ButtonId)], out: &mut Vec<ButtonId>) {
    for (m, b) in map {
        if byte & m != 0 {
            out.push(*b);
        }
    }
}

const FACE: [(u8, ButtonId); 4] = [(0x10, ButtonId::Square), (0x20, ButtonId::Cross), (0x40, ButtonId::Circle), (0x80, ButtonId::Triangle)];
const SHOULDERS: [(u8, ButtonId); 6] =
    [(0x01, ButtonId::L1), (0x02, ButtonId::R1), (0x10, ButtonId::Create), (0x20, ButtonId::Options), (0x40, ButtonId::L3), (0x80, ButtonId::R3)];

/// DualSense battery byte: low nibble 0–10, high nibble 0 = on battery, 1 = charging, 2 = full (Linux formula:
/// `min(level * 10 + 5, 100)`).
pub fn dualsense_battery(b: u8) -> Battery {
    let level = (b & 0x0F) as u32;
    let status = b >> 4;
    let percent = if status == 2 { 100 } else { (level * 10 + 5).min(100) as u8 };
    Battery { percent: Some(percent), level: None, charging: status == 1, wired: status == 1 || status == 2 }
}

/// DualShock 4 battery byte: low nibble level (0–10, 11 = full on cable), bit 0x10 = cable.
pub fn ds4_battery(b: u8) -> Battery {
    let level = (b & 0x0F) as u32;
    let cable = b & 0x10 != 0;
    Battery { percent: Some((level * 10).min(100) as u8), level: None, charging: cable && level <= 10, wired: cable }
}

/// Decode one DualSense / DualSense Edge report.
pub fn decode_dualsense(r: &[u8], edge: bool) -> Option<LiveState> {
    let (base, full) = match (r.first()?, r.len()) {
        (0x01, n) if n >= 64 => (1usize, true),
        (0x31, n) if n >= 78 => (2usize, true),
        (0x01, n) if n >= 10 => (1usize, false),
        _ => return None,
    };
    let g = |i: usize| r.get(base + i).copied();
    let mut s = LiveState { left: (axis(g(0)?), axis(g(1)?)), right: (axis(g(2)?), axis(g(3)?)), ..Default::default() };
    let (b0, b1, b2, l2, r2) = if full { (g(7)?, g(8)?, g(9)?, g(4)?, g(5)?) } else { (g(4)?, g(5)?, g(6)?, g(7)?, g(8)?) };
    s.l2 = l2 as f32 / 255.0;
    s.r2 = r2 as f32 / 255.0;
    hat(b0, &mut s.pressed);
    bits(b0, &FACE, &mut s.pressed);
    bits(b1, &SHOULDERS, &mut s.pressed);
    if b2 & 0x01 != 0 {
        s.pressed.push(ButtonId::Home);
    }
    s.touchpad = b2 & 0x02 != 0;
    if full {
        if b2 & 0x04 != 0 {
            s.pressed.push(ButtonId::Mute);
        }
        if edge {
            // Edge: 0x10 / 0x20 = left / right Fn, 0x40 / 0x80 = left / right back paddle (SDL source). Which Steam back
            // slot each one is: see parts.rs (Fn = "upper" is a guess).
            bits(b2, &[(0x10, ButtonId::BackLeftUpper), (0x20, ButtonId::BackRightUpper), (0x40, ButtonId::BackLeftLower), (0x80, ButtonId::BackRightLower)], &mut s.pressed);
        }
        s.battery = g(52).map(dualsense_battery);
    }
    Some(s)
}

/// Decode one DualShock 4 report.
pub fn decode_ds4(r: &[u8]) -> Option<LiveState> {
    let base = match (r.first()?, r.len()) {
        (0x01, n) if n >= 10 => 1usize,
        (0x11, n) if n >= 12 => 3usize,
        _ => return None,
    };
    let g = |i: usize| r.get(base + i).copied();
    let mut s = LiveState { left: (axis(g(0)?), axis(g(1)?)), right: (axis(g(2)?), axis(g(3)?)), ..Default::default() };
    let (b0, b1, b2) = (g(4)?, g(5)?, g(6)?);
    s.l2 = g(7)? as f32 / 255.0;
    s.r2 = g(8)? as f32 / 255.0;
    hat(b0, &mut s.pressed);
    bits(b0, &FACE, &mut s.pressed);
    bits(b1, &SHOULDERS, &mut s.pressed);
    if b2 & 0x01 != 0 {
        s.pressed.push(ButtonId::Home);
    }
    s.touchpad = b2 & 0x02 != 0;
    s.battery = g(29).map(ds4_battery);
    Some(s)
}

/// The report an Xbox source hands over: `0xX1` + XInput's `XINPUT_GAMEPAD` (buttons u16 LE, LT, RT, LX, LY, RX, RY i16 LE).
pub fn xinput_report(buttons: u16, lt: u8, rt: u8, lx: i16, ly: i16, rx: i16, ry: i16) -> Vec<u8> {
    let mut v = vec![0xA1];
    v.extend(buttons.to_le_bytes());
    v.push(lt);
    v.push(rt);
    for a in [lx, ly, rx, ry] {
        v.extend(a.to_le_bytes());
    }
    v
}

/// Decode an Xbox (XInput) report made by [`xinput_report`].
pub fn decode_xinput(r: &[u8]) -> Option<LiveState> {
    if r.first() != Some(&0xA1) || r.len() < 13 {
        return None;
    }
    let w = u16::from_le_bytes([r[1], r[2]]);
    let i16at = |i: usize| i16::from_le_bytes([r[i], r[i + 1]]) as f32 / 32767.0;
    let mut s = LiveState {
        l2: r[3] as f32 / 255.0,
        r2: r[4] as f32 / 255.0,
        // XInput's y is UP = +; flip to the HID direction used here
        left: (i16at(5).clamp(-1.0, 1.0), (-i16at(7)).clamp(-1.0, 1.0)),
        right: (i16at(9).clamp(-1.0, 1.0), (-i16at(11)).clamp(-1.0, 1.0)),
        ..Default::default()
    };
    let map: [(u16, ButtonId); 14] = [
        (0x0001, ButtonId::DpadUp),
        (0x0002, ButtonId::DpadDown),
        (0x0004, ButtonId::DpadLeft),
        (0x0008, ButtonId::DpadRight),
        (0x0010, ButtonId::Options),
        (0x0020, ButtonId::Create),
        (0x0040, ButtonId::L3),
        (0x0080, ButtonId::R3),
        (0x0100, ButtonId::L1),
        (0x0200, ButtonId::R1),
        (0x1000, ButtonId::Cross),
        (0x2000, ButtonId::Circle),
        (0x4000, ButtonId::Square),
        (0x8000, ButtonId::Triangle),
    ];
    for (m, b) in map {
        if w & m != 0 {
            s.pressed.push(b);
        }
    }
    Some(s)
}

/// Decode any report of this pad kind.
pub fn decode(kind: PadKind, r: &[u8]) -> Option<LiveState> {
    match kind {
        PadKind::DualSense => decode_dualsense(r, false),
        PadKind::DualSenseEdge => decode_dualsense(r, true),
        PadKind::DualShock4 => decode_ds4(r),
        PadKind::Xbox => decode_xinput(r),
    }
}

/// The battery inside one report (the controller list reads one report per PlayStation pad).
pub fn battery_in(kind: PadKind, r: &[u8]) -> Option<Battery> {
    decode(kind, r).and_then(|s| s.battery)
}

/// The running live view of one controller. Dropping it stops the thread (and closes the device).
pub struct LiveView {
    latest: Arc<Mutex<Option<LiveState>>>,
    gone: Arc<AtomicBool>,
    stop: Arc<dyn Fn() + Send + Sync>,
    thread: Option<JoinHandle<()>>,
    reports: Arc<std::sync::atomic::AtomicU64>,
}

/// Called on the reader thread whenever the state changed (keep it short: e.g. ask the UI to repaint).
pub type OnChange = Box<dyn Fn(&LiveState) + Send>;

impl LiveView {
    /// Open the controller and start reading. `on_change` runs on the reader thread for every CHANGED state.
    pub fn start(os: &dyn PadOs, pad: &PadInfo, on_change: Option<OnChange>) -> Result<LiveView> {
        let mut src = os.open_live(pad)?;
        let stop = src.stopper();
        let latest = Arc::new(Mutex::new(None));
        let gone = Arc::new(AtomicBool::new(false));
        let reports = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let (l2, g2, n2) = (latest.clone(), gone.clone(), reports.clone());
        let kind = pad.kind;
        let thread = std::thread::Builder::new()
            .name("bu-controller-live".into())
            .spawn(move || loop {
                match src.next() {
                    Ok(LiveEvent::Report(r)) => {
                        if let Some(s) = decode(kind, &r) {
                            let changed = {
                                let mut g = l2.lock().unwrap_or_else(|p| p.into_inner());
                                let changed = g.as_ref() != Some(&s);
                                if changed {
                                    *g = Some(s.clone());
                                }
                                changed
                            };
                            if changed {
                                if let Some(f) = &on_change {
                                    f(&s);
                                }
                            }
                        }
                        // counted only after the state is stored, so `reports() >= n` means `latest()` already holds report n
                        n2.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(LiveEvent::Stopped) => break,
                    Ok(LiveEvent::Gone) | Err(_) => {
                        g2.store(true, Ordering::Relaxed);
                        break;
                    }
                }
            })
            .map_err(|e| crate::error::Error::io("start the live view thread", e))?;
        Ok(LiveView { latest, gone, stop, thread: Some(thread), reports })
    }

    /// The newest state (None until the first report).
    pub fn latest(&self) -> Option<LiveState> {
        self.latest.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// The controller went away (unplugged / switched off).
    pub fn is_gone(&self) -> bool {
        self.gone.load(Ordering::Relaxed)
    }

    /// Reports received so far (for the report's cost measurement).
    pub fn reports(&self) -> u64 {
        self.reports.load(Ordering::Relaxed)
    }

    /// Stop reading and close the device (also done on drop).
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        (self.stop)();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for LiveView {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ds_usb() -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[0] = 0x01;
        r[1] = 0x80;
        r[2] = 0x80;
        r[3] = 0xFF;
        r[4] = 0x00;
        r[5] = 0x40; // L2
        r[6] = 0xFF; // R2
        r[8] = 0x08 | 0x20; // hat none + Cross
        r[9] = 0x01 | 0x40; // L1 + L3
        r[10] = 0x04 | 0x80; // Mute + right paddle
        r[53] = 0x18; // charging, 8 → 85 %
        r
    }

    #[test]
    fn dualsense_usb_and_bt_decode_the_same() {
        let u = decode_dualsense(&ds_usb(), true).unwrap();
        assert!((u.right.0 - 1.0).abs() < 0.01 && (u.right.1 + 1.0).abs() < 0.01);
        assert!((u.l2 - 0x40 as f32 / 255.0).abs() < 1e-6 && u.r2 == 1.0);
        for b in [ButtonId::Cross, ButtonId::L1, ButtonId::L3, ButtonId::Mute, ButtonId::BackRightLower] {
            assert!(u.is_pressed(b), "{b:?}");
        }
        assert!(!u.is_pressed(ButtonId::DpadUp));
        assert_eq!(u.battery, Some(Battery { percent: Some(85), level: None, charging: true, wired: true }));
        let mut bt = vec![0x31, 0x00];
        bt.extend(&ds_usb()[1..]);
        bt.resize(78, 0);
        assert_eq!(decode_dualsense(&bt, true).unwrap(), u);
        // a plain DualSense ignores the Edge bits
        assert!(!decode_dualsense(&ds_usb(), false).unwrap().is_pressed(ButtonId::BackRightLower));
    }

    #[test]
    fn dualsense_bt_simple_report_and_hat() {
        let r = [0x01, 0, 255, 128, 128, 0x02 | 0x80, 0x02, 0x01, 10, 20];
        let s = decode_dualsense(&r, false).unwrap();
        assert!(s.is_pressed(ButtonId::DpadRight) && s.is_pressed(ButtonId::Triangle) && s.is_pressed(ButtonId::R1) && s.is_pressed(ButtonId::Home));
        assert_eq!(s.battery, None);
        assert!((s.left.0 + 1.0).abs() < 0.01 && (s.left.1 - 1.0).abs() < 0.01);
        assert!(decode_dualsense(&[0x05, 1, 2], false).is_none());
    }

    #[test]
    fn ds4_usb_and_bt() {
        let mut u = vec![0u8; 64];
        u[0] = 0x01;
        u[5] = 0x07 | 0x10; // NW + Square
        u[6] = 0x20; // Options
        u[7] = 0x02; // touchpad click
        u[8] = 200;
        u[30] = 0x10 | 0x0B; // cable, full
        let s = decode_ds4(&u).unwrap();
        assert!(s.is_pressed(ButtonId::DpadUp) && s.is_pressed(ButtonId::DpadLeft) && s.is_pressed(ButtonId::Square) && s.is_pressed(ButtonId::Options));
        assert!(s.touchpad);
        assert_eq!(s.battery.unwrap().percent, Some(100));
        assert!(!s.battery.unwrap().charging && s.battery.unwrap().wired);
        let mut bt = vec![0x11, 0xC0, 0x00];
        bt.extend(&u[1..]);
        assert_eq!(decode_ds4(&bt).unwrap(), s);
    }

    #[test]
    fn xinput_round_trip() {
        let r = xinput_report(0x1000 | 0x0100 | 0x0001, 255, 0, 32767, 32767, -32768, 0);
        let s = decode_xinput(&r).unwrap();
        assert!(s.is_pressed(ButtonId::Cross) && s.is_pressed(ButtonId::L1) && s.is_pressed(ButtonId::DpadUp));
        assert_eq!(s.l2, 1.0);
        assert!((s.left.1 + 1.0).abs() < 0.001, "stick up = -1 in the HID direction");
        assert_eq!(s.right.0, -1.0);
    }
}

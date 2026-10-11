//! Controller reports (Order 081): the Windows side of the controller sounds. A game controller is a HID device; Raw Input hands
//! over its input reports (listen-only, RIDEV_INPUTSINK, like the keyboard and the mouse) and HIDP turns one report into
//! "which buttons are held, which way the D-pad points, how far the triggers are pulled". The result is a [`PadFrame`] for the
//! pure part ([`crate::hub::Hub::feed_pad`]); nothing here is kept beyond the per-controller layout (preparsed data) that is
//! needed to read its reports, and nothing is written to the device or to disk.
//!
//! The layout is only known for Sony pads (buttons 7 and 8 are the triggers). Any other pad: every button is a "button"; a trigger that
//! is an axis only (Xbox pads) makes no sound - their report layouts differ too much to guess (Order 081 review).

use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::size_of;

use windows::Win32::Devices::HumanInterfaceDevice::{
    HidP_GetCaps, HidP_GetUsageValue, HidP_GetUsages, HidP_GetValueCaps, HidP_Input, HidP_MaxUsageListLength, HIDP_CAPS, HIDP_VALUE_CAPS,
    PHIDP_PREPARSED_DATA,
};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::UI::Input::{
    GetRawInputDeviceInfoW, GetRawInputDeviceList, RAWINPUTDEVICELIST, RIDI_DEVICEINFO, RIDI_DEVICENAME, RIDI_PREPARSEDDATA, RID_DEVICE_INFO,
    RIM_TYPEHID,
};

use crate::hub::PadFrame;

const PAGE_GENERIC: u16 = 1;
const PAGE_BUTTON: u16 = 9;
const USAGE_HAT: u16 = 0x39;
const VID_SONY: u32 = 0x054C;
/// HIDP_STATUS_SUCCESS
const HIDP_OK: i32 = 0x0011_0000;

/// The most buttons read (usages 1..=56 → bits 0..=55; the D-pad takes bits 56..=59).
const MAX_BUTTONS: u16 = 56;
const DPAD_UP: u64 = 1 << 56;
const DPAD_RIGHT: u64 = 1 << 57;
const DPAD_DOWN: u64 = 1 << 58;
const DPAD_LEFT: u64 = 1 << 59;

/// One controller as Windows lists it (for the Keyboard tab's line "N controllers found").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PadDevice {
    pub vendor: u32,
    pub product: u32,
    /// Windows' device path (no personal data: it holds the hardware ids).
    pub path: String,
}

/// How the pad's triggers are made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Triggers {
    /// Buttons 7 and 8 (Sony).
    Buttons78,
    /// Unknown maker: no trigger is told apart.
    None,
}

#[derive(Debug, Clone, Copy)]
struct Axis {
    link: u16,
    min: i32,
    max: i32,
}

/// What is needed to read one controller's reports.
struct Profile {
    /// The preparsed data (HIDP's description of the reports), 8-byte aligned.
    ppd: Vec<u64>,
    /// The maker's USB vendor id (which button is which, Order 090).
    vendor: u16,
    triggers: Triggers,
    hat: Option<Axis>,
    /// the usage list HIDP fills (reused: a pad streams reports all the time)
    list: Vec<u16>,
    max_buttons: u32,
    /// the report bytes HIDP may write to while decoding (it wants a mutable pointer)
    scratch: Vec<u8>,
}

thread_local! {
    /// Per device handle (raw thread only). None = a device that cannot be read (no preparsed data): counted, not decoded.
    static PROFILES: std::cell::RefCell<HashMap<isize, (Option<Profile>, u32)>> = std::cell::RefCell::new(HashMap::new());
}

/// Forget every layout (the controller sounds stopped listening).
pub fn forget() {
    PROFILES.with(|p| p.borrow_mut().clear());
}

fn handle(h: isize) -> HANDLE {
    HANDLE(h as *mut c_void)
}

fn ppd_of(p: &Profile) -> PHIDP_PREPARSED_DATA {
    PHIDP_PREPARSED_DATA(p.ppd.as_ptr() as isize)
}

fn axis_of(caps: &[HIDP_VALUE_CAPS], usage: u16) -> Option<Axis> {
    caps.iter().find_map(|c| {
        if c.UsagePage != PAGE_GENERIC {
            return None;
        }
        // SAFETY: IsRange says which member of the union is valid.
        let (lo, hi) = unsafe {
            if c.IsRange {
                (c.Anonymous.Range.UsageMin, c.Anonymous.Range.UsageMax)
            } else {
                (c.Anonymous.NotRange.Usage, c.Anonymous.NotRange.Usage)
            }
        };
        (lo <= usage && usage <= hi).then_some(Axis { link: c.LinkCollection, min: c.LogicalMin, max: c.LogicalMax })
    })
}

fn device_info(dev: isize) -> Option<RID_DEVICE_INFO> {
    let mut info = RID_DEVICE_INFO { cbSize: size_of::<RID_DEVICE_INFO>() as u32, ..Default::default() };
    let mut size = info.cbSize;
    // SAFETY: a RID_DEVICE_INFO with its cbSize set, and its size.
    let n = unsafe { GetRawInputDeviceInfoW(Some(handle(dev)), RIDI_DEVICEINFO, Some(&mut info as *mut _ as *mut c_void), &mut size) };
    (n != u32::MAX && n != 0).then_some(info)
}

impl Profile {
    fn new(dev: isize) -> Option<Profile> {
        let mut size = 0u32;
        // SAFETY: asking for the size only.
        unsafe { GetRawInputDeviceInfoW(Some(handle(dev)), RIDI_PREPARSEDDATA, None, &mut size) };
        if size == 0 {
            return None;
        }
        let mut ppd = vec![0u64; (size as usize).div_ceil(8)];
        // SAFETY: a buffer of at least `size` bytes.
        let n = unsafe { GetRawInputDeviceInfoW(Some(handle(dev)), RIDI_PREPARSEDDATA, Some(ppd.as_mut_ptr() as *mut c_void), &mut size) };
        if n == u32::MAX || n == 0 {
            return None;
        }
        let vendor = device_info(dev).map(|i| unsafe { i.Anonymous.hid.dwVendorId }).unwrap_or(0);
        let mut p = Profile {
            ppd,
            vendor: vendor as u16,
            triggers: match vendor {
                VID_SONY => Triggers::Buttons78,
                _ => Triggers::None,
            },
            hat: None,
            list: Vec::new(),
            max_buttons: 0,
            scratch: Vec::new(),
        };
        let pd = ppd_of(&p);
        let mut caps = HIDP_CAPS::default();
        // SAFETY: valid preparsed data of this device.
        if unsafe { HidP_GetCaps(pd, &mut caps) }.0 != HIDP_OK {
            return None;
        }
        let mut n = caps.NumberInputValueCaps;
        let mut vc = vec![HIDP_VALUE_CAPS::default(); n as usize];
        if n > 0 {
            // SAFETY: `vc` holds `n` entries.
            if unsafe { HidP_GetValueCaps(HidP_Input, vc.as_mut_ptr(), &mut n, pd) }.0 == HIDP_OK {
                vc.truncate(n as usize);
                p.hat = axis_of(&vc, USAGE_HAT);
            }
        }
        // SAFETY: valid preparsed data.
        p.max_buttons = unsafe { HidP_MaxUsageListLength(HidP_Input, Some(PAGE_BUTTON), pd) };
        p.list = vec![0u16; p.max_buttons as usize];
        if p.max_buttons == 0 && p.hat.is_none() {
            // nothing to hear on this device
            return None;
        }
        Some(p)
    }

    /// One input report → the controller's state, or None (a report id this layout doesn't describe).
    fn decode(&mut self, dev: usize, report: &[u8]) -> Option<PadFrame> {
        if report.is_empty() {
            return None;
        }
        self.scratch.clear();
        self.scratch.extend_from_slice(report);
        let pd = ppd_of(self);
        let mut keys = 0u64;
        if self.max_buttons > 0 {
            let mut len = self.list.len() as u32;
            // SAFETY: `list` holds `len` entries; the report is `scratch`'s bytes.
            let st = unsafe {
                HidP_GetUsages(HidP_Input, PAGE_BUTTON, Some(0), self.list.as_mut_ptr(), &mut len, pd, &mut self.scratch)
            };
            if st.0 != HIDP_OK {
                return None;
            }
            for &u in &self.list[..(len as usize).min(self.list.len())] {
                if (1..=MAX_BUTTONS).contains(&u) {
                    keys |= 1u64 << (u - 1);
                }
            }
        }
        if let Some(h) = self.hat {
            // a hat that can't be read in this report: the whole report is skipped (never a release of a held direction)
            let v = self.value(pd, USAGE_HAT, h)?;
            keys |= hat_bits(v, h);
        }
        let mut trig = 0u8;
        if self.triggers == Triggers::Buttons78 {
            trig = ((keys >> 6) & 3) as u8;
            keys &= !(3u64 << 6);
        }
        Some(PadFrame { dev, vendor: self.vendor, keys, trig, analog: None })
    }

    fn value(&mut self, pd: PHIDP_PREPARSED_DATA, usage: u16, a: Axis) -> Option<i32> {
        let mut v = 0u32;
        // SAFETY: the report is `scratch`'s bytes.
        let st = unsafe {
            HidP_GetUsageValue(HidP_Input, PAGE_GENERIC, Some(a.link), usage, &mut v, pd, &self.scratch)
        };
        (st.0 == HIDP_OK).then_some(v as i32)
    }
}

/// The hat switch → the D-pad bits. Eight positions run clockwise from up; four-position hats get the same. Anything outside the
/// logical range is the resting (null) state.
fn hat_bits(v: i32, a: Axis) -> u64 {
    let (v, lo, hi) = (i64::from(v), i64::from(a.min), i64::from(a.max));
    if v < lo || v > hi {
        return 0;
    }
    let pos = v - lo;
    let dir = match hi - lo {
        7 => pos,
        3 => pos * 2,
        _ => return 0,
    };
    match dir {
        0 => DPAD_UP,
        1 => DPAD_UP | DPAD_RIGHT,
        2 => DPAD_RIGHT,
        3 => DPAD_RIGHT | DPAD_DOWN,
        4 => DPAD_DOWN,
        5 => DPAD_DOWN | DPAD_LEFT,
        6 => DPAD_LEFT,
        7 => DPAD_LEFT | DPAD_UP,
        _ => 0,
    }
}

/// The reports of one HID packet (`size` bytes each, `count` of them, back to back at `data`) → frames. None = this controller's
/// reports cannot be read (counted by the caller).
///
/// # Safety
/// `data` points at `size * count` readable bytes.
pub unsafe fn frames(dev: isize, data: *const u8, size: u32, count: u32, mut each: impl FnMut(Option<PadFrame>)) {
    if dev == 0 || size == 0 || count == 0 || count > 64 {
        return;
    }
    PROFILES.with(|ps| {
        let mut ps = ps.borrow_mut();
        let (prof, misses) = ps.entry(dev).or_insert_with(|| (Profile::new(dev), 0));
        if prof.is_none() {
            // not readable (yet - a pad that was just plugged in): look again now and then, never decide for good
            *misses += 1;
            if *misses % 256 == 0 {
                *prof = Profile::new(dev);
            }
        }
        for i in 0..count as usize {
            // SAFETY: the caller promised size * count bytes.
            let report = unsafe { std::slice::from_raw_parts(data.add(i * size as usize), size as usize) };
            each(prof.as_mut().and_then(|p| p.decode(dev as usize, report)));
        }
    });
}

/// Every game controller Windows lists to Raw Input (usage page 1, usage 4 joystick / 5 gamepad). Read-only; also tells whether
/// something (Steam Input, a filter driver) hides the physical pad: a hidden pad is not in this list.
pub fn list() -> Vec<PadDevice> {
    let mut n = 0u32;
    let sz = size_of::<RAWINPUTDEVICELIST>() as u32;
    // SAFETY: asking for the count.
    unsafe { GetRawInputDeviceList(None, &mut n, sz) };
    if n == 0 {
        return Vec::new();
    }
    let mut all = vec![RAWINPUTDEVICELIST::default(); n as usize];
    // SAFETY: `all` holds `n` entries.
    let got = unsafe { GetRawInputDeviceList(Some(all.as_mut_ptr()), &mut n, sz) };
    if got == u32::MAX {
        return Vec::new();
    }
    all.truncate(got as usize);
    let mut out = Vec::new();
    for d in all {
        if d.dwType != RIM_TYPEHID {
            continue;
        }
        let dev = d.hDevice.0 as isize;
        let Some(info) = device_info(dev) else { continue };
        // SAFETY: dwType says the union holds the HID member.
        let hid = unsafe { info.Anonymous.hid };
        if hid.usUsagePage != PAGE_GENERIC || !(hid.usUsage == 4 || hid.usUsage == 5) {
            continue;
        }
        let mut len = 0u32;
        // SAFETY: asking for the length of the path.
        unsafe { GetRawInputDeviceInfoW(Some(d.hDevice), RIDI_DEVICENAME, None, &mut len) };
        let mut buf = vec![0u16; len as usize];
        // SAFETY: `buf` holds `len` UTF-16 units.
        let k = unsafe { GetRawInputDeviceInfoW(Some(d.hDevice), RIDI_DEVICENAME, Some(buf.as_mut_ptr() as *mut c_void), &mut len) };
        let path = if k == u32::MAX { String::new() } else { String::from_utf16_lossy(&buf[..(k as usize).min(buf.len())]).trim_end_matches('\0').to_string() };
        out.push(PadDevice { vendor: hid.dwVendorId, product: hid.dwProductId, path });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hat(min: i32, max: i32) -> Axis {
        Axis { link: 0, min, max }
    }

    #[test]
    fn the_hat_points_clockwise_from_up_and_rests_outside_its_range() {
        // eight positions, 0 = up (Sony, most pads)
        let h = hat(0, 7);
        assert_eq!(hat_bits(0, h), DPAD_UP);
        assert_eq!(hat_bits(1, h), DPAD_UP | DPAD_RIGHT);
        assert_eq!(hat_bits(2, h), DPAD_RIGHT);
        assert_eq!(hat_bits(4, h), DPAD_DOWN);
        assert_eq!(hat_bits(6, h), DPAD_LEFT);
        assert_eq!(hat_bits(7, h), DPAD_LEFT | DPAD_UP);
        assert_eq!(hat_bits(8, h), 0, "the null state sits just outside the range");
        assert_eq!(hat_bits(15, h), 0);
        // 1..=8 (Microsoft)
        let h = hat(1, 8);
        assert_eq!(hat_bits(1, h), DPAD_UP);
        assert_eq!(hat_bits(3, h), DPAD_RIGHT);
        assert_eq!(hat_bits(0, h), 0, "0 is the resting state there");
        // four positions
        let h = hat(0, 3);
        assert_eq!(hat_bits(1, h), DPAD_RIGHT);
        assert_eq!(hat_bits(3, h), DPAD_LEFT);
    }

    /// Reading the list of controllers is read-only and works with none attached.
    #[test]
    fn listing_the_controllers_does_not_fail() {
        let l = list();
        for d in &l {
            assert!(d.path.is_empty() || d.path.starts_with("\\\\?\\"), "{d:?}");
        }
    }
}

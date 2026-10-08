//! Windows scaling % per monitor. There is NO public API to set it (research ideas-v1 §2). Windows' own Settings app
//! uses two undocumented DisplayConfig packet types; we use exactly those, as the open-source samples do
//! (github.com/lihas/windows-DPI-scaling-sample DpiHelper.h, github.com/imniko/SetDPI):
//!   type -3 "GET_DPI_SCALE": header + minScaleRel, curScaleRel, maxScaleRel (steps relative to the recommended value)
//!   type -4 "SET_DPI_SCALE": header + scaleRel
//! Steps are positions in the list 100,125,150,175,200,225,250,300,350,400,450,500 %.
//! RISK: undocumented — a Windows update could change the packet; then the read fails (we show nothing) or the set
//! fails with an error. The size check below (32 bytes, as the sample asserts) refuses to run if the layout is off.
//! Reading it is safe; setting it is fake-tested only in this order.

use crate::error::{DisplayError, Result};
use crate::types::DpiScale;
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, DisplayConfigSetDeviceInfo, DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_DEVICE_INFO_TYPE,
};
use windows::Win32::Foundation::LUID;

pub const DPI_STEPS: [u32; 12] = [100, 125, 150, 175, 200, 225, 250, 300, 350, 400, 450, 500];

#[repr(C)]
#[derive(Default)]
struct DpiGet {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    min_rel: i32,
    cur_rel: i32,
    max_rel: i32,
}

#[repr(C)]
#[derive(Default)]
struct DpiSet {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    scale_rel: i32,
}

const _: () = assert!(size_of::<DpiGet>() == 0x20);
const _: () = assert!(size_of::<DpiSet>() == 0x18);

fn raw(adapter: LUID, source_id: u32) -> Option<(i32, i32, i32)> {
    let mut p = DpiGet {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_TYPE(-3),
            size: size_of::<DpiGet>() as u32,
            adapterId: adapter,
            id: source_id,
        },
        ..Default::default()
    };
    (unsafe { DisplayConfigGetDeviceInfo(&mut p.header) } == 0).then_some((p.min_rel, p.cur_rel, p.max_rel))
}

/// Turns the relative steps into percents (pure; tested).
pub fn decode(min_rel: i32, cur_rel: i32, max_rel: i32) -> Option<DpiScale> {
    // Windows reports steps relative to the recommended value: min <= 0 <= max. Anything else = unknown layout, refuse.
    if min_rel > 0 || max_rel < 0 {
        return None;
    }
    let rec = min_rel.checked_abs()? as usize;
    let cur = cur_rel.clamp(min_rel, max_rel);
    let max_i = rec.checked_add(usize::try_from(max_rel).ok()?)?;
    if max_i >= DPI_STEPS.len() {
        return None;
    }
    let cur_i = (rec as i64 + cur as i64) as usize;
    Some(DpiScale {
        current_percent: DPI_STEPS[cur_i],
        recommended_percent: DPI_STEPS[rec],
        allowed_percent: DPI_STEPS[..=max_i].to_vec(),
    })
}

/// The step to send for a percent (relative to the recommended one).
pub fn encode(scale: &DpiScale, percent: u32) -> Option<i32> {
    let target = DPI_STEPS.iter().position(|p| *p == percent)?;
    let rec = DPI_STEPS.iter().position(|p| *p == scale.recommended_percent)?;
    scale.allowed_percent.contains(&percent).then_some(target as i32 - rec as i32)
}

pub(crate) fn get(adapter: LUID, source_id: u32) -> Option<DpiScale> {
    let (a, b, c) = raw(adapter, source_id)?;
    decode(a, b, c)
}

pub(crate) fn set(adapter: LUID, source_id: u32, percent: u32) -> Result<()> {
    let scale = get(adapter, source_id).ok_or_else(|| DisplayError::os("DisplayConfigGetDeviceInfo(-3)", "scaling unreadable"))?;
    let rel = encode(&scale, percent).ok_or(DisplayError::DpiNotOffered(percent))?;
    let p = DpiSet {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_TYPE(-4),
            size: size_of::<DpiSet>() as u32,
            adapterId: adapter,
            id: source_id,
        },
        scale_rel: rel,
    };
    let r = unsafe { DisplayConfigSetDeviceInfo(&p.header) };
    if r != 0 {
        return Err(DisplayError::os("DisplayConfigSetDeviceInfo(-4)", format!("error {r}")));
    }
    Ok(())
}

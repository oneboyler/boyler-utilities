//! Monitors + modes changes through the public Display Configuration API (QueryDisplayConfig / SetDisplayConfig /
//! DisplayConfigGetDeviceInfo). Apply and main-display changes have a VALIDATE-only twin (`SDC_VALIDATE`: Windows checks
//! the configuration and changes nothing) used for real-PC proof without touching the real screen.

use super::wide_to_string;
use crate::error::{DisplayError, Result};
use crate::types::{GpuScaling, HdrInfo, Mode, MonitorId, RefreshRate};
use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, LUID};
use windows::Win32::Graphics::Gdi::DISPLAYCONFIG_PATH_MODE_IDX_INVALID;

pub(crate) struct Snapshot {
    pub paths: Vec<DISPLAYCONFIG_PATH_INFO>,
    pub modes: Vec<DISPLAYCONFIG_MODE_INFO>,
}

/// One active path, decoded.
#[derive(Clone, Debug)]
pub(crate) struct PathView {
    pub index: usize,
    pub source_adapter: LUID,
    pub source_id: u32,
    pub gdi_name: String,
    pub friendly_name: String,
    pub device_path: String,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub refresh: RefreshRate,
    pub scaling: GpuScaling,
    pub native: Option<(u32, u32)>,
    pub hdr: Option<HdrInfo>,
}

pub(crate) fn query() -> Result<Snapshot> {
    for _ in 0..5 {
        let (mut np, mut nm) = (0u32, 0u32);
        let r = unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) };
        if r != ERROR_SUCCESS {
            return Err(DisplayError::os("GetDisplayConfigBufferSizes", format!("{:?}", r)));
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        let r = unsafe { QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None) };
        if r == ERROR_INSUFFICIENT_BUFFER {
            continue; // a monitor came or went between the two calls
        }
        if r != ERROR_SUCCESS {
            return Err(DisplayError::os("QueryDisplayConfig", format!("{:?}", r)));
        }
        paths.truncate(np as usize);
        modes.truncate(nm as usize);
        return Ok(Snapshot { paths, modes });
    }
    Err(DisplayError::os("QueryDisplayConfig", "the monitor set kept changing"))
}

fn scaling_from(s: DISPLAYCONFIG_SCALING) -> GpuScaling {
    match s {
        DISPLAYCONFIG_SCALING_STRETCHED => GpuScaling::Stretch,
        DISPLAYCONFIG_SCALING_CENTERED => GpuScaling::BlackBars,
        DISPLAYCONFIG_SCALING_ASPECTRATIOCENTEREDMAX => GpuScaling::KeepAspect,
        _ => GpuScaling::DriverDefault,
    }
}

fn scaling_to(s: GpuScaling) -> DISPLAYCONFIG_SCALING {
    match s {
        GpuScaling::Stretch => DISPLAYCONFIG_SCALING_STRETCHED,
        GpuScaling::BlackBars => DISPLAYCONFIG_SCALING_CENTERED,
        GpuScaling::KeepAspect => DISPLAYCONFIG_SCALING_ASPECTRATIOCENTEREDMAX,
        GpuScaling::DriverDefault => DISPLAYCONFIG_SCALING_PREFERRED,
    }
}

fn header(t: DISPLAYCONFIG_DEVICE_INFO_TYPE, size: usize, adapter: LUID, id: u32) -> DISPLAYCONFIG_DEVICE_INFO_HEADER {
    DISPLAYCONFIG_DEVICE_INFO_HEADER { r#type: t, size: size as u32, adapterId: adapter, id }
}

pub(crate) fn source_gdi_name(adapter: LUID, id: u32) -> Option<String> {
    let mut p = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
        header: header(DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>(), adapter, id),
        ..Default::default()
    };
    (unsafe { DisplayConfigGetDeviceInfo(&mut p.header) } == 0).then(|| wide_to_string(&p.viewGdiDeviceName))
}

fn target_names(adapter: LUID, id: u32) -> Option<(String, String)> {
    let mut p = DISPLAYCONFIG_TARGET_DEVICE_NAME {
        header: header(DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>(), adapter, id),
        ..Default::default()
    };
    (unsafe { DisplayConfigGetDeviceInfo(&mut p.header) } == 0)
        .then(|| (wide_to_string(&p.monitorFriendlyDeviceName), wide_to_string(&p.monitorDevicePath)))
}

fn preferred(adapter: LUID, id: u32) -> Option<(u32, u32)> {
    let mut p = DISPLAYCONFIG_TARGET_PREFERRED_MODE {
        header: header(DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_PREFERRED_MODE, size_of::<DISPLAYCONFIG_TARGET_PREFERRED_MODE>(), adapter, id),
        ..Default::default()
    };
    (unsafe { DisplayConfigGetDeviceInfo(&mut p.header) } == 0).then_some((p.width, p.height))
}

fn hdr(adapter: LUID, id: u32) -> Option<HdrInfo> {
    let mut p = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO {
        header: header(DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO, size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>(), adapter, id),
        ..Default::default()
    };
    if unsafe { DisplayConfigGetDeviceInfo(&mut p.header) } != 0 {
        return None;
    }
    // Bit 0 advancedColorSupported, bit 1 advancedColorEnabled (wingdi.h).
    let v = unsafe { p.Anonymous.value };
    Some(HdrInfo { supported: v & 1 != 0, enabled: v & 2 != 0 })
}

pub(crate) fn views(s: &Snapshot) -> Vec<PathView> {
    let mut out = Vec::new();
    for (i, p) in s.paths.iter().enumerate() {
        let sidx = unsafe { p.sourceInfo.Anonymous.modeInfoIdx } as usize;
        let Some(sm) = s.modes.get(sidx).filter(|m| m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE) else { continue };
        let src = unsafe { sm.Anonymous.sourceMode };
        let (friendly, path) = target_names(p.targetInfo.adapterId, p.targetInfo.id).unwrap_or_default();
        out.push(PathView {
            index: i,
            source_adapter: p.sourceInfo.adapterId,
            source_id: p.sourceInfo.id,
            gdi_name: source_gdi_name(p.sourceInfo.adapterId, p.sourceInfo.id).unwrap_or_default(),
            friendly_name: friendly,
            device_path: path,
            width: src.width,
            height: src.height,
            x: src.position.x,
            y: src.position.y,
            refresh: RefreshRate::new(p.targetInfo.refreshRate.Numerator, p.targetInfo.refreshRate.Denominator),
            scaling: scaling_from(p.targetInfo.scaling),
            native: preferred(p.targetInfo.adapterId, p.targetInfo.id),
            hdr: hdr(p.targetInfo.adapterId, p.targetInfo.id),
        });
    }
    out
}

pub(crate) fn find<'a>(views: &'a [PathView], id: &MonitorId) -> Result<&'a PathView> {
    views.iter().find(|v| v.device_path == id.0).ok_or_else(|| DisplayError::MonitorNotFound(id.clone()))
}

/// What to submit: really apply, or only let Windows check it. `Strict` variants leave out SDC_ALLOW_CHANGES, so
/// Windows must take the configuration exactly as given (no silent substitution of another rate / size).
/// Real Apply is ALWAYS strict: measured on a test PC, the loose (SDC_ALLOW_CHANGES) check accepts a 500 Hz request on
/// a 360 Hz monitor (Windows would silently pick something else); the strict one refuses it (error 1610) and accepts
/// every reported rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Submit {
    /// Apply and store it as Windows' saved display setting (survives a reboot / crash).
    ApplyStrict,
    /// Apply for now only (no SDC_SAVE_TO_DATABASE): a crash / reboot comes back to the stored setting. Used for the
    /// keep countdown and automatic per-app switches.
    ApplyStrictTemporary,
    ValidateOnly,
    ValidateStrict,
}

impl Submit {
    fn is_validate(self) -> bool {
        matches!(self, Submit::ValidateOnly | Submit::ValidateStrict)
    }
}

fn submit(s: &Snapshot, how: Submit) -> Result<()> {
    let flags = match how {
        Submit::ApplyStrict => SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_SAVE_TO_DATABASE,
        Submit::ApplyStrictTemporary => SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG,
        Submit::ValidateOnly => SDC_VALIDATE | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_ALLOW_CHANGES,
        Submit::ValidateStrict => SDC_VALIDATE | SDC_USE_SUPPLIED_DISPLAY_CONFIG,
    };
    let r = unsafe { SetDisplayConfig(Some(&s.paths), Some(&s.modes), flags) };
    if r != 0 {
        return Err(DisplayError::os(
            if how.is_validate() { "SetDisplayConfig (validate)" } else { "SetDisplayConfig" },
            format!("error {r}"),
        ));
    }
    Ok(())
}

/// Builds the change for W × H × exact Hz + scaling on one path: the source mode gets the new size; the target mode
/// index is cleared and `targetInfo.refreshRate` carries the exact rational rate, so Windows picks the monitor's own
/// timing for that rate (documented: the path refresh rate is used when no target mode is supplied).
/// `keep` = the modes this app set (see `keep_scaling`).
pub(crate) fn set_mode(id: &MonitorId, mode: &Mode, how: Submit, keep: &[(MonitorId, Mode)]) -> Result<()> {
    let mut s = query()?;
    let v = views(&s);
    keep_scaling(&mut s, &v, Some(id), keep);
    let pv = find(&v, id)?.clone();
    let p = &mut s.paths[pv.index];
    let sidx = unsafe { p.sourceInfo.Anonymous.modeInfoIdx } as usize;
    {
        let m = s.modes.get_mut(sidx).ok_or_else(|| DisplayError::os("SetDisplayConfig", "no source mode"))?;
        // Writing a field of a Copy union is safe; this mode is a SOURCE mode (index from sourceInfo).
        m.Anonymous.sourceMode.width = mode.width;
        m.Anonymous.sourceMode.height = mode.height;
    }
    p.targetInfo.refreshRate = DISPLAYCONFIG_RATIONAL { Numerator: mode.refresh.num, Denominator: mode.refresh.den };
    p.targetInfo.scaling = scaling_to(mode.scaling);
    p.targetInfo.Anonymous.modeInfoIdx = DISPLAYCONFIG_PATH_MODE_IDX_INVALID;
    submit(&s, how)
}

/// Order 042: Windows' read-back of a path doesn't carry the GPU scaling an Apply set (Stretch / Black bars came back as
/// Keep aspect), and every submit sends ALL paths again - so each path (but `skip`, the one being changed) that still
/// shows a mode this app set on it gets that mode's scaling back before the submit.
fn keep_scaling(s: &mut Snapshot, v: &[PathView], skip: Option<&MonitorId>, keep: &[(MonitorId, Mode)]) {
    for pv in v {
        if skip.is_some_and(|id| id.0 == pv.device_path) {
            continue;
        }
        if let Some((_, m)) = keep.iter().find(|(id, _)| id.0 == pv.device_path) {
            if (pv.width, pv.height, pv.refresh) == (m.width, m.height, m.refresh) {
                s.paths[pv.index].targetInfo.scaling = scaling_to(m.scaling);
            }
        }
    }
}

/// Main display: every source moves by the new main's offset so the new main sits at 0,0 (Windows' definition of
/// main), the others keep their places relative to it.
pub(crate) fn set_main(id: &MonitorId, how: Submit, keep: &[(MonitorId, Mode)]) -> Result<()> {
    let mut s = query()?;
    let v = views(&s);
    keep_scaling(&mut s, &v, None, keep);
    let pv = find(&v, id)?.clone();
    let (dx, dy) = (pv.x, pv.y);
    for m in s.modes.iter_mut() {
        if m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE {
            unsafe {
                m.Anonymous.sourceMode.position.x -= dx;
                m.Anonymous.sourceMode.position.y -= dy;
            }
        }
    }
    submit(&s, how)
}

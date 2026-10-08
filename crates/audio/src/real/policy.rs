//! The undocumented `IPolicyConfig` (what the Sound panel itself uses): default device + device on/off. The same
//! declaration as Lane A's test app, EarTrumpet and AudioDeviceCmdlets (MIT). Stable for years, but undocumented — it
//! broke once (Windows 10 Anniversary Update), so every failure maps to an error, never a crash.

#![allow(non_snake_case)]

use windows::core::{GUID, HRESULT, PCWSTR};
use windows_core::IUnknown;
use windows_core::IUnknown_Vtbl;

#[windows::core::interface("f8679f50-850a-41cf-9c72-430f290290c8")]
pub unsafe trait IPolicyConfig: IUnknown {
    fn GetMixFormat(&self, id: PCWSTR, fmt: *mut *mut core::ffi::c_void) -> HRESULT;
    fn GetDeviceFormat(&self, id: PCWSTR, default: i32, fmt: *mut *mut core::ffi::c_void) -> HRESULT;
    fn ResetDeviceFormat(&self, id: PCWSTR) -> HRESULT;
    fn SetDeviceFormat(&self, id: PCWSTR, endpoint: *mut core::ffi::c_void, mix: *mut core::ffi::c_void) -> HRESULT;
    fn GetProcessingPeriod(&self, id: PCWSTR, default: i32, def: *mut i64, min: *mut i64) -> HRESULT;
    fn SetProcessingPeriod(&self, id: PCWSTR, period: *mut i64) -> HRESULT;
    fn GetShareMode(&self, id: PCWSTR, mode: *mut core::ffi::c_void) -> HRESULT;
    fn SetShareMode(&self, id: PCWSTR, mode: *mut core::ffi::c_void) -> HRESULT;
    fn GetPropertyValue(&self, id: PCWSTR, fx: i32, key: *const core::ffi::c_void, v: *mut core::ffi::c_void) -> HRESULT;
    fn SetPropertyValue(&self, id: PCWSTR, fx: i32, key: *const core::ffi::c_void, v: *mut core::ffi::c_void) -> HRESULT;
    fn SetDefaultEndpoint(&self, id: PCWSTR, role: i32) -> HRESULT;
    fn SetEndpointVisibility(&self, id: PCWSTR, visible: i32) -> HRESULT;
}

pub const CLSID_POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

/// `SetDefaultEndpoint(id, role)`.
pub fn set_default_endpoint(pc: &IPolicyConfig, id: PCWSTR, role: i32) -> HRESULT {
    // SAFETY: `id` is a NUL-terminated endpoint id that outlives the call.
    unsafe { pc.SetDefaultEndpoint(id, role) }
}

/// `SetEndpointVisibility(id, visible)` — the Sound panel's Disable (0) / Enable (1).
pub fn set_endpoint_visibility(pc: &IPolicyConfig, id: PCWSTR, visible: i32) -> HRESULT {
    // SAFETY: as above.
    unsafe { pc.SetEndpointVisibility(id, visible) }
}

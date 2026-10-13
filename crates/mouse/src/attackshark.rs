//! Attack Shark - NOT PORTED, ON PURPOSE (Order 105): OpenMouse's AttackSharkHidClient (`drivers/attackshark/hid.js`) gives no DPI, polling,
//! lift-off or battery for the only SUPPORTED-list model, the R5 Ultra (373e:0046 / 0047): that client explicitly leaves the Lamzu catalog ids
//! (which include the R5 Ultra) to the Lamzu client (`lamzu.rs`), and its own `373e` branch reads dpi 0 / polling 0 and sets nothing. Nothing
//! matches here, so no mouse gets this driver.

use crate::error::{Error, Result};
use crate::family::{Changed, Driver, Matched, OnMouse, Port, ReadOptions, Setting};
use crate::os::HidInfo;

/// The device table: VID:PID -> model (OpenMouse's SUPPORTED list only) and the interface to talk to.
pub fn matches(_own: &[&HidInfo], _vid: u16, _pid: u16) -> Option<Matched> {
    None
}

/// The allow-list for the read-only proof on a real mouse: only this family's READ requests.
pub fn read_request_allowed(_out: &[u8]) -> bool {
    false
}

/// The Attack Shark driver.
pub struct AttackShark;

impl Driver for AttackShark {
    fn read(&self, _m: &Matched, _port: &mut dyn Port, _opt: ReadOptions) -> Result<OnMouse> {
        Err(Error::UnsupportedMouse("Attack Shark (not ported yet)".into()))
    }
    fn write(&self, _m: &Matched, _port: &mut dyn Port, _s: Setting) -> Result<Changed> {
        Err(Error::UnsupportedMouse("Attack Shark (not ported yet)".into()))
    }
}

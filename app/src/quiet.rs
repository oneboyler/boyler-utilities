//! Child processes of the app (Order 118): every one starts with CREATE_NO_WINDOW, so a console program (cmd, ping, clip,
//! powershell) never flashes a window on the user's screen - and a test that starts one never opens a window.
//! (GUI programs such as Explorer are not affected by the flag: their own windows open as always.)

use std::ffi::OsStr;
use std::os::windows::process::CommandExt;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// `Command::new(program)` that opens no console window.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut c = Command::new(program);
    c.creation_flags(CREATE_NO_WINDOW);
    c
}

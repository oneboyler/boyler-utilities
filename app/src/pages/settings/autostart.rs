//! Settings › App › Start with Windows: the app's own value under HKCU\Software\Microsoft\Windows\CurrentVersion\Run (what
//! Windows' Startup list reads; no admin). Real: read when the tab opens (one registry read), written ONLY on the user's click.
//! Test copies: a fake switch in memory - a test never writes the registry.

use windows::core::{w, PCWSTR};
use windows::Win32::System::Registry::{RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ};

const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const NAME: PCWSTR = w!("Boyler Utilities");

pub trait AutoStart {
    fn get(&self) -> bool;
    fn set(&mut self, on: bool) -> Result<(), String>;
}

/// In memory (test copies).
pub struct Fake(pub bool);

impl AutoStart for Fake {
    fn get(&self) -> bool {
        self.0
    }
    fn set(&mut self, on: bool) -> Result<(), String> {
        self.0 = on;
        Ok(())
    }
}

/// The real Run value for the running exe.
pub struct Real;

impl AutoStart for Real {
    /// On only when the value starts THIS exe (a moved / renamed copy shows Off, so a click writes the right path).
    fn get(&self) -> bool {
        let mut buf = vec![0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN, NAME, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len)) };
        if r.is_err() {
            return false;
        }
        let val = String::from_utf16_lossy(&buf[..(len as usize / 2).saturating_sub(1)]);
        std::env::current_exe().is_ok_and(|exe| same_exe(&val, &exe.display().to_string()))
    }
    fn set(&mut self, on: bool) -> Result<(), String> {
        let r = if on {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let cmd: Vec<u16> = format!("\"{}\"", exe.display()).encode_utf16().chain(Some(0)).collect();
            unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, RUN, NAME, REG_SZ.0, Some(cmd.as_ptr() as *const _), (cmd.len() * 2) as u32) }
        } else {
            unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, NAME) }
        };
        if r.is_ok() {
            Ok(())
        } else {
            Err(format!("Windows refused (0x{:08X})", r.0))
        }
    }
}

/// The Run value (`"C:\…\BoylerUtilities.exe"`, quoted or not, maybe with arguments) starts `exe` (paths compare without case).
fn same_exe(val: &str, exe: &str) -> bool {
    let v = val.trim();
    let path = match v.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(""),
        None => v.split(" -").next().unwrap_or(v).trim(),
    };
    path.eq_ignore_ascii_case(exe)
}

#[cfg(test)]
mod tests {
    use super::same_exe;

    #[test]
    fn the_run_value_must_start_this_exe() {
        let exe = r"C:\Apps\Boyler\BoylerUtilities.exe";
        assert!(same_exe(r#""C:\Apps\Boyler\BoylerUtilities.exe""#, exe));
        assert!(same_exe(r#""c:\apps\boyler\boylerutilities.exe" --tray"#, exe));
        assert!(same_exe(r"C:\Apps\Boyler\BoylerUtilities.exe", exe));
        assert!(!same_exe(r#""C:\Old\BoylerUtilities.exe""#, exe), "a moved exe is Off");
        assert!(!same_exe("", exe));
    }
}

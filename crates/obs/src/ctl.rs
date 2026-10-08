//! Safe edits of OBS's own files (obsctl.c): the profile's basic.ini (keys, clip length, FPS, folder - only while OBS is
//! closed) and turning OBS's WebSocket server on, ALWAYS password protected. Each write is a temp file swapped in; only
//! the changed values change.

use std::path::Path;

use crate::ini;
use crate::os::ObsOs;

/// Set several (section, key, value) in the profile's basic.ini. False = missing file / write failed.
pub fn write_profile(os: &mut dyn ObsOs, profile_dir: &Path, vals: &[(String, String, String)]) -> bool {
    let path = profile_dir.join("basic.ini");
    let Some(mut t) = os.read_file(&path) else { return false };
    for (s, k, v) in vals {
        ini::set(&mut t, s, k, v);
    }
    os.write_file(&path, &t)
}

/// Why turning remote control on did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsError {
    ObsRunning,
    Missing,
    Unreadable,
    WriteFailed,
}

/// Turn on OBS's WebSocket server in obs-websocket's config.json: auth on, the existing password kept if it has 8+
/// characters, else a new 32-character one is made and saved there (never logged, never stored anywhere else). Only
/// while OBS is closed (OBS reads the file when it starts and doesn't write it back). Ok(true) = a new password was made.
pub fn enable_ws(os: &mut dyn ObsOs) -> Result<bool, WsError> {
    if os.obs_running() {
        return Err(WsError::ObsRunning);
    }
    let path = os.obs_dir().join("plugin_config").join("obs-websocket").join("config.json");
    let Some(mut t) = os.read_file(&path) else { return Err(WsError::Missing) };
    let v: serde_json::Value = serde_json::from_str(&t).map_err(|_| WsError::Unreadable)?;
    let keep = v.get("server_password").and_then(|p| p.as_str()).map(|p| p.len() >= 8).unwrap_or(false);
    let mut generated = false;
    if !keep {
        let pw = os.random_password(32).ok_or(WsError::WriteFailed)?;
        if !ini::json_set(&mut t, "server_password", &format!("\"{pw}\"")) {
            return Err(WsError::WriteFailed);
        }
        generated = true;
    }
    if !(ini::json_set(&mut t, "auth_required", "true") && ini::json_set(&mut t, "server_enabled", "true")) {
        return Err(WsError::WriteFailed);
    }
    if !os.write_file(&path, &t) {
        return Err(WsError::WriteFailed);
    }
    Ok(generated)
}

//! Default apps (the last Toggles group, DESIGN §3.6 "3. Default apps"). READ the current defaults; "Change" = open Windows' own
//! UI, because Windows 11 can't set defaults silently (UserChoice hash, UCPD — research ideas-v3.md §2). **Never writes UserChoice.**

use crate::error::Result;
use crate::os::TogglesOs;

/// The file types shown under the browser row (DESIGN §3.6: .png, .jpg, .mp4, .mkv, .mp3, .pdf, .txt, .zip).
pub const FILE_TYPES: [&str; 8] = [".png", ".jpg", ".mp4", ".mkv", ".mp3", ".pdf", ".txt", ".zip"];

/// The one-line note under the list.
pub const NOTE: &str = "Windows 11 asks you to confirm each change in its own window.";

/// What "Change" opens. The menu passes it to [`perform`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeAction {
    /// a Settings / Store link (e.g. `ms-settings:defaultapps?registeredAppMachine=Google%20Chrome`)
    OpenUri(String),
    /// Windows' "Open with" list for this extension (the user ticks "Always")
    OpenWith(String),
}

/// One row of the Default apps list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultRow {
    /// "Browser" or the extension (".png")
    pub label: String,
    /// the current app; None = Windows has no default
    pub app: Option<crate::os::AssocApp>,
    /// file-type rows: what "Change" opens. The browser row changes through [`Browser::change`].
    pub change: Option<ChangeAction>,
    /// the "Change" button's tip
    pub tip: String,
}

/// One installed browser for the browser picker (✓ on the current one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Browser {
    pub name: String,
    pub is_current: bool,
    pub change: ChangeAction,
}

/// The whole Default apps group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultApps {
    pub browser: DefaultRow,
    pub browsers: Vec<Browser>,
    pub file_types: Vec<DefaultRow>,
}

/// Percent-encodes a registered-application name for the Settings link (spaces etc. → %XX).
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The Settings page of one browser (2 clicks to make it the default). MS Learn "Launch the Default Apps settings page".
pub fn browser_settings_uri(b: &crate::os::RegisteredBrowser) -> String {
    let param = if b.machine { "registeredAppMachine" } else { "registeredAppUser" };
    format!("ms-settings:defaultapps?{param}={}", encode_uri_component(&b.reg_name))
}

/// Reads the whole group (read-only).
pub fn read<O: TogglesOs + ?Sized>(os: &O) -> Result<DefaultApps> {
    let current_progid = os.default_browser_progid()?;
    let mut browsers: Vec<Browser> = os
        .registered_browsers()?
        .into_iter()
        .map(|b| Browser {
            is_current: match (&current_progid, &b.https_progid) {
                (Some(cur), Some(p)) => cur.eq_ignore_ascii_case(p),
                _ => false,
            },
            change: ChangeAction::OpenUri(browser_settings_uri(&b)),
            name: b.display_name,
        })
        .collect();
    browsers.sort_by_key(|a| a.name.to_lowercase());
    browsers.dedup_by(|a, b| a.name == b.name && a.is_current == b.is_current);

    let browser = DefaultRow {
        label: "Browser".into(),
        app: current_app(os, "https")?,
        change: None,
        tip: "Pick another browser".into(),
    };
    let file_types = FILE_TYPES
        .iter()
        .map(|ext| {
            Ok(DefaultRow {
                label: ext.to_string(),
                app: current_app(os, ext)?,
                change: Some(ChangeAction::OpenWith(ext.to_string())),
                tip: format!("Opens Windows’ “Open with” list for {ext} files"),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(DefaultApps { browser, browsers, file_types })
}

/// Opens what "Change" points at (Windows' own window — the user confirms there).
pub fn perform<O: TogglesOs + ?Sized>(os: &mut O, action: &ChangeAction) -> Result<()> {
    match action {
        ChangeAction::OpenUri(uri) => os.open_uri(uri),
        ChangeAction::OpenWith(ext) => os.open_with_dialog(ext),
    }
}

/// The toast when a browser is picked (DESIGN §3.6).
pub fn browser_pick_toast(name: &str) -> String {
    format!("Windows asks you to confirm · {name}’s page in Settings opens")
}

/// The toast once the re-read shows the picked browser is the default.
pub fn browser_done_toast(name: &str) -> String {
    format!("{name} is your browser now")
}

/// The toast when a file type's "Change" is clicked.
pub fn file_type_toast(ext: &str) -> String {
    format!("Windows shows its “Open with” list for {ext} · tick “Always”")
}

/// The current default app, or None when Windows has none. With no default Windows answers with its own "Pick an app" dialog
/// (`OpenWith.exe`) — measured on the test PC for .mkv — which is "none", not an app.
fn current_app<O: TogglesOs + ?Sized>(os: &O, what: &str) -> Result<Option<crate::os::AssocApp>> {
    Ok(os.assoc_app(what)?.filter(|a| !a.exe.as_deref().is_some_and(|e| e.to_lowercase().ends_with(r"\openwith.exe"))))
}

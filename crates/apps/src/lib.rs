//! bu-apps — DESIGN.md §3.13 Apps, no UI.
//!
//! Lists installed apps — desktop (Win32) apps from the three `Uninstall` registry places (HKLM 64-bit, HKLM 32-bit
//! `WOW6432Node`, HKCU) and Store (MSIX) apps from the package manager — and uninstalls one or several, one after another:
//! a desktop app through its own uninstaller (its quiet one only where the app offers it), a Store app through the package
//! manager. Locked rows (Edge, Windows parts, entries that say they can't be removed) can't be uninstalled.
//! An uninstall can't be undone; [`Apps::confirm`] gives the confirm sheet's data first (what goes, space freed, warnings).
//! The leftover scan is NOT here (unanswered idea, Order 006).

pub mod date;
pub mod fake;
pub mod os;
#[cfg(windows)]
pub mod real;

use std::path::PathBuf;

pub use date::Date;
pub use os::{AppsOs, Hive, OsError, RegView, Signature};

pub const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
/// Settings › Apps › one Store app's own page (Reset, add-ons …): `ms-settings:appsfeatures-app?<package family name>`
/// (Microsoft Learn, "Launch Windows Settings", Apps table).
pub const SETTINGS_APP_PAGE: &str = "ms-settings:appsfeatures-app?";

/// Apps Windows keeps (DESIGN §3.13 "Locked": Edge; research big-A §3: WebView2 must stay). Display names, no case; desktop
/// entries and Store packages alike.
pub const KEPT_APPS: &[&str] = &["Microsoft Edge", "Microsoft Edge WebView2 Runtime", "Microsoft EdgeWebView", "Microsoft Edge Update"];

/// Store packages that are Windows parts a gamer must keep (research big-A §3 "never-remove list"; Microsoft Store and
/// XboxSpeechToTextOverlay can't be reinstalled). Identity-name prefixes, compared without case.
pub const KEPT_STORE_PACKAGES: &[&str] = &[
    "Microsoft.WindowsStore",
    "Microsoft.DesktopAppInstaller",
    "Microsoft.StorePurchaseApp",
    "Microsoft.Xbox.TCUI",
    "Microsoft.GamingServices",
    "Microsoft.XboxIdentityProvider",
    "Microsoft.XboxGameOverlay",
    "Microsoft.XboxGamingOverlay",
    "Microsoft.XboxSpeechToTextOverlay",
    "Microsoft.WebMediaExtensions",
    "Microsoft.WebpImageExtension",
    "Microsoft.HEIFImageExtension",
    "Microsoft.HEVCVideoExtension",
    "Microsoft.VP9VideoExtensions",
    "Microsoft.AV1VideoExtension",
    "Microsoft.MPEG2VideoExtension",
    "Microsoft.RawImageExtension",
    "Microsoft.SecHealthUI",
];

/// Amber lines for the confirm sheet: (name contains, line). DESIGN §3.13 draws the Vanguard one.
pub const WARNINGS: &[(&str, &str)] = &[("Riot Vanguard", "VALORANT won't start without it.")];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AppKind {
    Desktop,
    Store,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppSource {
    Desktop { hive: Hive, view: RegView, key_name: String },
    Store { full_name: String, family_name: String },
}

/// How the size was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeSource {
    /// The app's own estimate in its Uninstall entry (`EstimatedSize`, KB — listed in Microsoft Learn "Uninstall Registry Key").
    /// That Settings › Installed apps shows this same number is a guess (not documented).
    Estimated,
    /// Measured by adding up the files in its folder ([`Apps::measure_size`]).
    Measured,
    /// Not known ("—", sorted last). Store apps start here: the package manager gives no size; measure on demand.
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateSource {
    /// `InstallDate` (YYYYMMDD) in the Uninstall entry.
    Registry,
    /// No InstallDate: the entry's last-write time (a guess at what Programs and Features falls back to — not documented).
    EntryWritten,
    /// The package manager's install time.
    Package,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockReason {
    /// "Windows keeps this one · it can't be uninstalled" (Edge, WebView2, Windows parts, system packages).
    WindowsKeeps,
    /// The entry itself says it can't be removed (`NoRemove = 1`).
    NoRemove,
    /// The entry has no uninstall command ("Can't be uninstalled here").
    Broken,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledApp {
    pub id: String,
    pub name: String,
    pub publisher: Option<String>,
    pub version: Option<String>,
    pub size_bytes: Option<u64>,
    pub size_source: SizeSource,
    pub install_date: Option<Date>,
    pub date_source: Option<DateSource>,
    /// File holding the icon (exe / ico / png) + index.
    pub icon_path: Option<PathBuf>,
    pub icon_index: i32,
    /// The exact command that will run (desktop apps).
    pub uninstall_command: Option<String>,
    /// The command is the app's own quiet one (`QuietUninstallString`).
    pub quiet: bool,
    pub install_location: Option<PathBuf>,
    /// Store apps: where the right-click "App settings (Windows)" goes.
    pub settings_uri: Option<String>,
    pub kind: AppKind,
    pub source: AppSource,
    pub lock: Option<LockReason>,
    /// HKLM entry: its uninstaller will likely show Windows' admin prompt (Windows asks; we never elevate).
    pub admin_prompt_likely: bool,
    /// Desktop apps: the app's own "Change" setup (`ModifyPath`; MSI: `MsiExec.exe /I{GUID}`), unless the entry says
    /// `NoModify = 1`. The Apps page's wrench (menu-v22 "Modify").
    pub modify_command: Option<String>,
    /// Desktop apps: the app's own repair (MSI: `MsiExec.exe /f{GUID}`; a WiX bundle: `"<BundleCachePath>" /repair`), unless
    /// the entry says `NoRepair = 1`. The wrench's "Repair".
    pub repair_command: Option<String>,
}

/// What the Apps page's wrench offers (menu-v22: "[Modify / Repair] · Open folder · Uninstall"; Store apps "Repair / Reset").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fix {
    Modify,
    Repair,
    /// Store apps only (Windows Settings' "Reset": clears the app's data).
    Reset,
}

impl Fix {
    pub fn label(self) -> &'static str {
        match self {
            Fix::Modify => "Modify",
            Fix::Repair => "Repair",
            Fix::Reset => "Reset",
        }
    }
}

/// What a wrench choice did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FixOutcome {
    /// The app's own setup ran and ended with this exit code (its own window: the user finished or cancelled it there).
    SetupEnded { exit_code: u32 },
    /// Store apps: Windows Settings opened on the app's own page, where Windows' Repair / Reset buttons are (Windows offers
    /// no documented call that repairs a Store app; Reset-AppxPackage exists but clears the data without asking).
    OpenedSettings,
}

impl InstalledApp {
    pub fn can_uninstall(&self) -> bool {
        self.lock.is_none()
    }
    pub fn warning(&self) -> Option<&'static str> {
        WARNINGS.iter().find(|(n, _)| self.name.to_lowercase().contains(&n.to_lowercase())).map(|(_, w)| *w)
    }
    /// The wrench's choices, in the page's order (Modify before Repair; Store: Repair, Reset). Locked apps have none.
    pub fn fixes(&self) -> Vec<Fix> {
        if self.lock.is_some() {
            return Vec::new();
        }
        match self.kind {
            AppKind::Store => vec![Fix::Repair, Fix::Reset],
            AppKind::Desktop => {
                let mut v = Vec::new();
                if self.modify_command.is_some() {
                    v.push(Fix::Modify);
                }
                if self.repair_command.is_some() {
                    v.push(Fix::Repair);
                }
                v
            }
        }
    }
    /// The folder "Open install folder" opens: desktop apps only — `InstallLocation`, else the folder of the app's icon file
    /// when that is a file of the app itself (not Windows' own folders or an installer cache). Store apps: none (v22: their
    /// row has no folder icon).
    pub fn folder(&self) -> Option<PathBuf> {
        if self.kind != AppKind::Desktop {
            return None;
        }
        if let Some(p) = self.install_location.as_ref().filter(|p| !p.as_os_str().is_empty()) {
            return Some(p.clone());
        }
        let icon = self.icon_path.as_ref()?;
        let low = icon.to_string_lossy().to_lowercase();
        if low.contains(r"\windows\") || low.contains(r"\installer\") || low.contains(r"\package cache\") {
            return None;
        }
        icon.parent().map(|p| p.to_path_buf()).filter(|p| p.components().count() > 1)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppList {
    pub apps: Vec<InstalledApp>,
    pub problems: Vec<(String, OsError)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Installed,
}

/// The confirm sheet's data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirm {
    /// The apps that will go (locked ones left out), in order.
    pub apps: Vec<(String, String)>,
    /// Sum of the known sizes.
    pub frees_bytes: u64,
    /// How many of them have no known size (the "about" in "Frees about …").
    pub unknown_sizes: usize,
    /// Only Store apps: "Store apps are removed straight away."
    pub store_only: bool,
    /// At least one desktop app: "An app's own uninstaller may open: finish it there."
    pub uninstaller_may_open: bool,
    /// Amber lines.
    pub warnings: Vec<String>,
    /// Picked but locked — not uninstalled.
    pub skipped_locked: Vec<(String, LockReason)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Gone. `freed` = its known size.
    Uninstalled { freed: Option<u64> },
    /// The uninstaller asked for a restart (exit 3010 / 1641): "Finishes after a restart".
    NeedsRestart,
    /// Still installed after the uninstaller ended (the user cancelled it, or said No at the admin prompt): "Not uninstalled".
    /// `exit_code` = the uninstaller's exit code when it ran.
    NotUninstalled { exit_code: Option<u32> },
}

/// Progress of [`Apps::uninstall_many`] for one row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    Waiting,
    Uninstalling,
    Done(Outcome),
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AppsError {
    #[error("locked ({0:?})")]
    Locked(LockReason),
    #[error("needs administrator rights")]
    NeedsAdmin,
    #[error("{0:?} is not offered by this app")]
    NotOffered(Fix),
    #[error("no install folder known")]
    NoFolder,
    #[error(transparent)]
    Os(#[from] OsError),
}

pub struct Apps<O: AppsOs> {
    os: O,
}

impl<O: AppsOs> Apps<O> {
    pub fn new(os: O) -> Self {
        Apps { os }
    }
    pub fn os(&self) -> &O {
        &self.os
    }

    /// Read every installed app (READ-ONLY). Default order: size, biggest first (unknown last).
    pub fn list(&self) -> AppList {
        let mut apps = Vec::new();
        let mut problems = Vec::new();
        match self.os.uninstall_entries() {
            Ok(entries) => {
                for e in entries {
                    if let Some(a) = self.desktop_app(&e) {
                        let dup = apps.iter().any(|b: &InstalledApp| {
                            b.name == a.name && b.version == a.version && b.uninstall_command == a.uninstall_command
                        });
                        if !dup {
                            apps.push(a);
                        }
                    }
                }
            }
            Err(e) => problems.push(("Desktop apps".to_string(), e)),
        }
        match self.os.store_packages() {
            Ok(pkgs) => apps.extend(pkgs.into_iter().filter_map(store_app)),
            Err(e) => problems.push(("Store apps".to_string(), e)),
        }
        sort(&mut apps, SortKey::Size, true);
        AppList { apps, problems }
    }

    /// Add up the files in the app's folder (desktop: InstallLocation; Store: the package folder). Slow for big apps — run it
    /// off the UI thread. Returns the app with the measured size (source `Measured`), or unchanged when the folder can't be read.
    pub fn measure_size(&self, app: &InstalledApp) -> InstalledApp {
        let mut a = app.clone();
        if let Some(n) = app.install_location.as_deref().and_then(|p| self.os.folder_size(p)) {
            a.size_bytes = Some(n);
            a.size_source = SizeSource::Measured;
        }
        a
    }

    /// The confirm sheet's data for the picked apps.
    pub fn confirm(&self, picked: &[&InstalledApp]) -> Confirm {
        let (ok, locked): (Vec<&&InstalledApp>, Vec<&&InstalledApp>) = picked.iter().partition(|a| a.can_uninstall());
        let mut warnings = Vec::new();
        for a in &ok {
            if let Some(w) = a.warning() {
                let line = format!("{}: {w}", a.name);
                if !warnings.contains(&line) {
                    warnings.push(line);
                }
            }
        }
        Confirm {
            apps: ok.iter().map(|a| (a.id.clone(), a.name.clone())).collect(),
            frees_bytes: ok.iter().filter_map(|a| a.size_bytes).sum(),
            unknown_sizes: ok.iter().filter(|a| a.size_bytes.is_none()).count(),
            store_only: !ok.is_empty() && ok.iter().all(|a| a.kind == AppKind::Store),
            uninstaller_may_open: ok.iter().any(|a| a.kind == AppKind::Desktop),
            warnings,
            skipped_locked: locked.iter().map(|a| (a.name.clone(), a.lock.unwrap_or(LockReason::WindowsKeeps))).collect(),
        }
    }

    /// Uninstall one app and say what happened. Blocks until its uninstaller (and everything it started) has ended.
    pub fn uninstall(&self, app: &InstalledApp) -> Result<Outcome, AppsError> {
        if let Some(l) = app.lock {
            return Err(AppsError::Locked(l));
        }
        match &app.source {
            AppSource::Desktop { hive, view, key_name } => {
                let cmd = app.uninstall_command.as_deref().ok_or(AppsError::Locked(LockReason::Broken))?;
                let code = match self.os.run_uninstaller(cmd) {
                    Ok(c) => c,
                    Err(OsError::Cancelled) => return Ok(Outcome::NotUninstalled { exit_code: None }),
                    Err(OsError::AccessDenied) => return Err(AppsError::NeedsAdmin),
                    Err(e) => return Err(e.into()),
                };
                let still_there = self.os.entry_exists(*hive, *view, key_name);
                Ok(match code {
                    // ERROR_SUCCESS_REBOOT_REQUIRED / ERROR_SUCCESS_REBOOT_INITIATED (MSI and most installers)
                    3010 | 1641 => Outcome::NeedsRestart,
                    _ if !still_there => Outcome::Uninstalled { freed: app.size_bytes },
                    // Still installed whatever the code (1602 = MSI "user cancelled"; NSIS & co. return 1/2 on cancel): the row
                    // comes back as "Not uninstalled".
                    c => Outcome::NotUninstalled { exit_code: Some(c) },
                })
            }
            AppSource::Store { full_name, .. } => {
                match self.os.remove_package(full_name) {
                    Ok(()) => {}
                    Err(OsError::AccessDenied) => return Err(AppsError::NeedsAdmin),
                    Err(OsError::Cancelled) => return Ok(Outcome::NotUninstalled { exit_code: None }),
                    Err(e) => return Err(e.into()),
                }
                if self.os.package_installed(full_name) {
                    Ok(Outcome::NotUninstalled { exit_code: None })
                } else {
                    Ok(Outcome::Uninstalled { freed: app.size_bytes })
                }
            }
        }
    }

    /// The wrench: run the app's own Modify / Repair setup (its own window; blocks until it and what it started have ended —
    /// call it off the UI thread), or for a Store app open its page in Windows Settings (Repair / Reset are there).
    pub fn fix(&self, app: &InstalledApp, fix: Fix) -> Result<FixOutcome, AppsError> {
        if let Some(l) = app.lock {
            return Err(AppsError::Locked(l));
        }
        if !app.fixes().contains(&fix) {
            return Err(AppsError::NotOffered(fix));
        }
        match app.kind {
            AppKind::Store => {
                let uri = app.settings_uri.as_deref().ok_or(AppsError::NotOffered(fix))?;
                self.os.open_settings(uri)?;
                Ok(FixOutcome::OpenedSettings)
            }
            AppKind::Desktop => {
                let cmd = match fix {
                    Fix::Modify => app.modify_command.as_deref(),
                    Fix::Repair => app.repair_command.as_deref(),
                    Fix::Reset => None,
                }
                .ok_or(AppsError::NotOffered(fix))?;
                match self.os.run_setup(cmd) {
                    Ok(code) => Ok(FixOutcome::SetupEnded { exit_code: code }),
                    Err(OsError::AccessDenied) => Err(AppsError::NeedsAdmin),
                    Err(e) => Err(e.into()),
                }
            }
        }
    }

    /// "Open install folder": Explorer on [`InstalledApp::folder`] - only when that path is an existing FOLDER (some installers
    /// write a file path into InstallLocation: it is never started).
    pub fn open_folder(&self, app: &InstalledApp) -> Result<(), AppsError> {
        let f = app.folder().ok_or(AppsError::NoFolder)?;
        if !self.os.is_dir(&f) {
            return Err(AppsError::NoFolder);
        }
        self.os.open_folder(&f)?;
        Ok(())
    }

    /// Uninstall several, one after another. `progress(index, state)` is called as each row moves Waiting → Uninstalling →
    /// Done / Failed. Locked rows are skipped (Failed).
    pub fn uninstall_many(
        &self,
        apps: &[&InstalledApp],
        mut progress: impl FnMut(usize, Progress),
    ) -> Vec<Result<Outcome, AppsError>> {
        for i in 0..apps.len() {
            progress(i, Progress::Waiting);
        }
        let mut out = Vec::new();
        for (i, a) in apps.iter().enumerate() {
            progress(i, Progress::Uninstalling);
            let r = self.uninstall(a);
            progress(i, match &r {
                Ok(o) => Progress::Done(*o),
                Err(_) => Progress::Failed,
            });
            out.push(r);
        }
        out
    }

    fn desktop_app(&self, e: &os::RawEntry) -> Option<InstalledApp> {
        let name = e.text("DisplayName")?.to_string();
        // Windows' own hiding rules for Installed apps.
        if e.dword("SystemComponent") == Some(1) || e.text("ParentKeyName").is_some() {
            return None;
        }
        if let Some(rt) = e.text("ReleaseType") {
            if ["security update", "update rollup", "hotfix", "update"].contains(&rt.to_lowercase().as_str()) {
                return None;
            }
        }
        let is_msi = e.dword("WindowsInstaller") == Some(1);
        let guid = is_guid(&e.key_name);
        let quiet_cmd = e.text("QuietUninstallString").map(str::to_string);
        let plain_cmd = e.text("UninstallString").map(str::to_string);
        let (uninstall_command, quiet) = match (quiet_cmd, plain_cmd) {
            (Some(q), _) => (Some(q), true),
            // MSI: `MsiExec.exe /I{GUID}` opens the change dialog; /X removes (with its own UI).
            (None, Some(_)) | (None, None) if is_msi && guid => (Some(format!("MsiExec.exe /X{}", e.key_name)), false),
            (None, Some(p)) => (Some(p), false),
            (None, None) => (None, false),
        };
        // Programs and Features' Change / Repair (Microsoft Learn "Uninstall Registry Key": ModifyPath, NoModify, NoRepair).
        let modify_command = if e.dword("NoModify") == Some(1) {
            None
        } else if let Some(m) = e.text("ModifyPath").filter(|m| !m.trim().is_empty()) {
            Some(m.to_string())
        } else if is_msi && guid {
            Some(format!("MsiExec.exe /I{}", e.key_name))
        } else {
            None
        };
        let repair_command = if e.dword("NoRepair") == Some(1) {
            None
        } else if is_msi && guid {
            // `msiexec /f <product code>` = "repairs a product" with the default options omus (Microsoft Learn, msiexec)
            Some(format!("MsiExec.exe /f{}", e.key_name))
        } else {
            // a WiX Burn bundle keeps its own setup in its cache and takes /repair (WiX "Burn command line"; guess: not every
            // bundle registers BundleCachePath)
            e.text("BundleCachePath").filter(|p| !p.trim().is_empty()).map(|p| format!("\"{}\" /repair", p.trim_matches('"')))
        };
        let (size_bytes, size_source) = match e.dword("EstimatedSize") {
            Some(kb) if kb > 0 => (Some(kb as u64 * 1024), SizeSource::Estimated),
            _ => (None, SizeSource::Unknown),
        };
        let (install_date, date_source) = match e.text("InstallDate").and_then(Date::parse_yyyymmdd) {
            Some(d) => (Some(d), Some(DateSource::Registry)),
            None => match e.last_write.map(Date::from_filetime) {
                Some(d) => (Some(d), Some(DateSource::EntryWritten)),
                None => (None, None),
            },
        };
        let (icon_path, icon_index) = match e.text("DisplayIcon").and_then(parse_icon_location) {
            Some((p, i)) => (Some(PathBuf::from(self.os.expand_env(&p))), i),
            None => (None, 0),
        };
        let lock = if KEPT_APPS.iter().any(|k| k.eq_ignore_ascii_case(&name)) {
            Some(LockReason::WindowsKeeps)
        } else if e.dword("NoRemove") == Some(1) {
            Some(LockReason::NoRemove)
        } else if uninstall_command.is_none() {
            Some(LockReason::Broken)
        } else {
            None
        };
        let hive = match e.hive {
            Hive::CurrentUser => "HKCU",
            Hive::LocalMachine => "HKLM",
        };
        Some(InstalledApp {
            id: format!("desktop|{hive}|{:?}|{}", e.view, e.key_name),
            name,
            publisher: e.text("Publisher").map(str::to_string),
            version: e.text("DisplayVersion").map(str::to_string),
            size_bytes,
            size_source,
            install_date,
            date_source,
            icon_path,
            icon_index,
            uninstall_command,
            quiet,
            install_location: e.text("InstallLocation").map(|p| PathBuf::from(self.os.expand_env(p.trim_matches('"')))),
            settings_uri: None,
            kind: AppKind::Desktop,
            source: AppSource::Desktop { hive: e.hive, view: e.view, key_name: e.key_name.clone() },
            lock,
            admin_prompt_likely: e.hive == Hive::LocalMachine,
            modify_command,
            repair_command,
        })
    }
}

fn store_app(p: os::RawPackage) -> Option<InstalledApp> {
    // Frameworks, resource packs, bundles and add-ons are parts of other packages; system-signed packages are Windows' own
    // plumbing (File Explorer, Call, Credential Dialog …). That Settings › Installed apps never lists them is a guess (unchecked).
    if p.is_framework || p.is_resource || p.is_bundle || p.is_optional || p.signature == Signature::System {
        return None;
    }
    // Packages without a display name (or with an unresolved ms-resource: one) are plumbing too.
    let name = p.display_name.clone().filter(|n| !n.is_empty() && !n.starts_with("ms-resource:"))?;
    let kept = KEPT_STORE_PACKAGES.iter().any(|k| p.name.to_lowercase().starts_with(&k.to_lowercase()))
        || KEPT_APPS.iter().any(|k| k.eq_ignore_ascii_case(&name));
    Some(InstalledApp {
        id: format!("store|{}", p.full_name),
        name,
        publisher: p.publisher.clone(),
        version: Some(p.version.clone()),
        size_bytes: None,
        size_source: SizeSource::Unknown,
        install_date: p.installed.map(Date::from_filetime),
        date_source: p.installed.map(|_| DateSource::Package),
        icon_path: p.logo.clone(),
        icon_index: 0,
        uninstall_command: None,
        quiet: true,
        install_location: p.installed_path.clone(),
        settings_uri: Some(format!("{SETTINGS_APP_PAGE}{}", p.family_name)),
        kind: AppKind::Store,
        source: AppSource::Store { full_name: p.full_name, family_name: p.family_name },
        lock: kept.then_some(LockReason::WindowsKeeps),
        admin_prompt_likely: false,
        modify_command: None,
        repair_command: None,
    })
}

/// Sort like the header: Name A→Z, Size and Installed newest/biggest first when `descending`. Unknown sizes / dates always last.
pub fn sort(apps: &mut [InstalledApp], key: SortKey, descending: bool) {
    apps.sort_by(|a, b| {
        let by_name = || a.name.to_lowercase().cmp(&b.name.to_lowercase());
        let ord = match key {
            SortKey::Name => by_name(),
            SortKey::Size => match (a.size_bytes, b.size_bytes) {
                (Some(x), Some(y)) => x.cmp(&y).then_with(|| by_name().reverse()),
                (Some(_), None) => return std::cmp::Ordering::Less,
                (None, Some(_)) => return std::cmp::Ordering::Greater,
                (None, None) => return by_name(),
            },
            SortKey::Installed => match (a.install_date, b.install_date) {
                (Some(x), Some(y)) => x.cmp(&y).then_with(|| by_name().reverse()),
                (Some(_), None) => return std::cmp::Ordering::Less,
                (None, Some(_)) => return std::cmp::Ordering::Greater,
                (None, None) => return by_name(),
            },
        };
        if descending {
            ord.reverse()
        } else {
            ord
        }
    });
}

/// The header search: name or publisher contains the text (no case).
pub fn search<'a>(apps: &'a [InstalledApp], text: &str) -> Vec<&'a InstalledApp> {
    let t = text.trim().to_lowercase();
    apps.iter()
        .filter(|a| t.is_empty() || a.name.to_lowercase().contains(&t) || a.publisher.as_deref().is_some_and(|p| p.to_lowercase().contains(&t)))
        .collect()
}

/// The Installed apps folder or Store app settings place for the right-click menu.
pub fn open_target(app: &InstalledApp) -> Option<String> {
    match app.kind {
        AppKind::Desktop => app.install_location.as_deref().map(|p| p.to_string_lossy().into_owned()),
        AppKind::Store => app.settings_uri.clone(),
    }
}

fn is_guid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 38
        && b[0] == b'{'
        && b[37] == b'}'
        && s[1..37].char_indices().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

/// `"C:\x\y.exe",0` / `C:\x\y.ico` / `C:\x\y.dll,-12` → (path, index).
pub fn parse_icon_location(s: &str) -> Option<(String, i32)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (p, i) = match s.rsplit_once(',') {
        Some((p, i)) if i.trim().parse::<i32>().is_ok() => (p, i.trim().parse().unwrap_or(0)),
        _ => (s, 0),
    };
    let p = p.trim().trim_matches('"').trim();
    (!p.is_empty()).then(|| (p.to_string(), i))
}


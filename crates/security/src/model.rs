//! Plain data the Security page shows. Pure functions only; every Windows call is behind [`crate::SecurityOs`].

use std::fmt;

use crate::error::SecurityError;

/// A local wall-clock time (what the page prints): "Today 09:12", "3 Oct".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Stamp {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

impl Stamp {
    pub fn new(year: u16, month: u8, day: u8, hour: u8, minute: u8) -> Stamp {
        Stamp { year, month, day, hour, minute }
    }
    fn same_day(&self, other: &Stamp) -> bool {
        self.year == other.year && self.month == other.month && self.day == other.day
    }
    fn date_text(&self) -> String {
        format!("{} {}", self.day, MONTHS[(self.month.clamp(1, 12) - 1) as usize])
    }
    /// "09:12" today, else "3 Oct" (a threat row: "found 09:12" / "quarantined 3 Oct").
    pub fn short(&self, now: &Stamp) -> String {
        if self.same_day(now) {
            format!("{:02}:{:02}", self.hour, self.minute)
        } else {
            self.date_text()
        }
    }
    /// "Today 09:12" today, else "3 Oct" ("Last scan: ...", "updated ...").
    pub fn label(&self, now: &Stamp) -> String {
        if self.same_day(now) {
            format!("Today {:02}:{:02}", self.hour, self.minute)
        } else {
            self.date_text()
        }
    }
}

impl fmt::Display for Stamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02} {:02}:{:02}", self.year, self.month, self.day, self.hour, self.minute)
    }
}

/// Defender's own idea of how it runs (`AMRunningMode`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunningMode {
    Normal,
    /// Another antivirus is active; Defender only watches.
    Passive,
    /// "EDR Block Mode" / "SxS Passive Mode" and other managed modes.
    Other(String),
    /// Not running at all ("Not running" or empty).
    NotRunning,
}

impl RunningMode {
    pub fn parse(s: &str) -> RunningMode {
        let t = s.trim();
        match t.to_ascii_lowercase().as_str() {
            "normal" => RunningMode::Normal,
            "passive mode" | "passive" => RunningMode::Passive,
            "" | "not running" => RunningMode::NotRunning,
            _ => RunningMode::Other(t.to_string()),
        }
    }
}

/// `Get-MpComputerStatus`, the parts the page needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefenderStatus {
    pub service_enabled: bool,
    pub antivirus_enabled: bool,
    pub realtime_enabled: bool,
    pub tamper_protected: bool,
    pub running_mode: RunningMode,
    /// "1.459.576.0"
    pub definitions_version: String,
    pub definitions_updated: Option<Stamp>,
    pub quick_scan_end: Option<Stamp>,
    pub full_scan_end: Option<Stamp>,
    pub reboot_required: bool,
}

/// One entry of Security Center's antivirus list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvProduct {
    pub name: String,
    pub on: bool,
    pub up_to_date: bool,
}

impl AvProduct {
    pub fn is_defender(&self) -> bool {
        self.name.to_ascii_lowercase().contains("defender")
    }
    /// Decode Security Center's `productState` number. Bits 12-15: 1 = on; bits 4-7: 0 = definitions up to date.
    pub fn from_product_state(name: &str, state: u32) -> AvProduct {
        AvProduct { name: name.to_string(), on: (state >> 12) & 0xF == 1, up_to_date: (state >> 4) & 0xF == 0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Unknown,
    Low,
    Moderate,
    High,
    Severe,
}

impl Severity {
    /// `MSFT_MpThreat.SeverityID`: 1 Low, 2 Moderate, 4 High, 5 Severe.
    pub fn from_id(id: i64) -> Severity {
        match id {
            1 => Severity::Low,
            2 => Severity::Moderate,
            4 => Severity::High,
            5 => Severity::Severe,
            _ => Severity::Unknown,
        }
    }
    /// The tag text on a row. Severe / High are drawn red, Moderate / Low amber.
    pub fn label(self) -> &'static str {
        match self {
            Severity::Severe => "Severe",
            Severity::High => "High",
            Severity::Moderate => "Moderate",
            Severity::Low => "Low",
            Severity::Unknown => "Unknown",
        }
    }
    pub fn is_red(self) -> bool {
        matches!(self, Severity::Severe | Severity::High)
    }
}

/// What happened to a detection (`ThreatStatusID`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreatState {
    /// Found, waiting for the user's choice (Detected, or an action that failed).
    NeedsChoice,
    Quarantined,
    Removed,
    Allowed,
    /// Cleaned or blocked by Defender on its own.
    Handled,
    Unknown,
}

impl ThreatState {
    /// 1 Detected, 2 Cleaned, 3 Quarantined, 4 Removed, 5 Allowed, 6 Blocked, 102 QuarantineFailed, 103 RemoveFailed,
    /// 104 AllowFailed, 105 Abandoned, 107 BlockedFailed (Microsoft Learn, MSFT_MpThreatDetection).
    pub fn from_status_id(id: i64) -> ThreatState {
        match id {
            1 | 102 | 103 | 104 | 105 | 107 => ThreatState::NeedsChoice,
            2 | 6 => ThreatState::Handled,
            3 => ThreatState::Quarantined,
            4 => ThreatState::Removed,
            5 => ThreatState::Allowed,
            _ => ThreatState::Unknown,
        }
    }
}

/// `MSFT_MpThreatDetection`, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detection {
    pub detection_id: String,
    pub threat_id: i64,
    pub status_id: i64,
    pub found: Option<Stamp>,
    pub status_changed: Option<Stamp>,
    /// Raw `Resources` entries, e.g. `file:_C:\Users\x\Downloads\a.exe`.
    pub resources: Vec<String>,
}

/// `MSFT_MpThreat`, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreatInfo {
    pub threat_id: i64,
    pub name: String,
    pub severity: Severity,
    pub active: bool,
}

/// One row of "Threats found" or "Quarantine": name + severity tag, "<file> · <folder> · found HH:MM".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreatRow {
    pub threat_id: i64,
    pub detection_id: String,
    /// "Trojan:Win32/Wacatac.B!ml"
    pub name: String,
    pub severity: Severity,
    pub state: ThreatState,
    /// File name, e.g. "kms_activator.exe"
    pub file: String,
    /// Full folder path, e.g. `C:\Users\x\Downloads` (the page shows its last part, full path in the tooltip)
    pub folder: String,
    pub found: Option<Stamp>,
    /// When it was moved to quarantine / last changed.
    pub changed: Option<Stamp>,
}

impl ThreatRow {
    pub fn path(&self) -> String {
        if self.folder.is_empty() {
            self.file.clone()
        } else {
            format!("{}\\{}", self.folder.trim_end_matches('\\'), self.file)
        }
    }
    /// The folder's own name (what the row shows).
    pub fn folder_name(&self) -> &str {
        self.folder.trim_end_matches('\\').rsplit('\\').next().unwrap_or("")
    }
}

/// `file:_C:\x\a.exe` → `C:\x\a.exe`; `webfile:_C:\x\a.exe|https://...|...` → `C:\x\a.exe`;
/// `file:_C:\x\a.zip->inner.exe` keeps the outer file. Resources that are not files (regkey, process, behavior)
/// give `None`.
pub fn resource_path(res: &str) -> Option<String> {
    let (kind, rest) = res.split_once(":_")?;
    let kind = kind.to_ascii_lowercase();
    if !matches!(kind.as_str(), "file" | "webfile" | "containerfile" | "folder" | "amsi") {
        return None;
    }
    let rest = rest.split('|').next().unwrap_or(rest);
    let rest = rest.split("->").next().unwrap_or(rest).trim();
    // a drive path or a UNC path; anything else is not a file
    let bytes = rest.as_bytes();
    let drive = bytes.len() > 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic();
    if drive || rest.starts_with("\\\\") {
        Some(rest.to_string())
    } else {
        None
    }
}

fn split_path(path: &str) -> (String, String) {
    match path.rfind('\\') {
        Some(i) => (path[..i].to_string(), path[i + 1..].to_string()),
        None => (String::new(), path.to_string()),
    }
}

/// Join detections with their threats into page rows (one row per file of a detection), newest first.
/// A detection whose threat is unknown to `threats` is shown as "Unknown threat".
pub fn rows_from(detections: &[Detection], threats: &[ThreatInfo]) -> Vec<ThreatRow> {
    let mut rows = Vec::new();
    for d in detections {
        let (name, severity) = match threats.iter().find(|t| t.threat_id == d.threat_id) {
            Some(t) => (t.name.clone(), t.severity),
            None => ("Unknown threat".to_string(), Severity::Unknown),
        };
        let mut files: Vec<String> = d.resources.iter().filter_map(|r| resource_path(r)).collect();
        files.dedup();
        if files.is_empty() {
            files.push(String::new());
        }
        for f in files {
            let (folder, file) = split_path(&f);
            rows.push(ThreatRow {
                threat_id: d.threat_id,
                detection_id: d.detection_id.clone(),
                name: name.clone(),
                severity,
                state: ThreatState::from_status_id(d.status_id),
                file,
                folder,
                found: d.found,
                changed: d.status_changed,
            });
        }
    }
    rows.sort_by(|a, b| b.found.cmp(&a.found).then_with(|| a.file.cmp(&b.file)));
    rows
}

/// Scans the page offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanKind {
    Quick,
    Full,
    /// A dropped / picked file or folder.
    Path(String),
}

impl ScanKind {
    /// "Quick scan" / "Full scan" / the file name.
    pub fn title(&self) -> String {
        match self {
            ScanKind::Quick => "Quick scan".to_string(),
            ScanKind::Full => "Full scan".to_string(),
            ScanKind::Path(p) => p.trim_end_matches('\\').rsplit('\\').next().unwrap_or(p).to_string(),
        }
    }
}

/// What the top status card says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Banner {
    /// Green "You're protected".
    Protected,
    /// Red "N threats found · Remove or Allow them below" (and the Review button).
    NeedsAttention { threats: usize },
    /// Accent "Quick scan running" / "You can keep using your PC".
    Scanning { title: String },
    /// "<Other> protects this PC — Defender is standing by"; scans greyed.
    OtherAntivirus { name: String },
    /// Defender is there but its service / real-time protection is off. Not drawn: the page picks the wording.
    ProtectionOff,
    /// The detection list could not be read (see `SecurityPage::unreadable`): neither "protected" nor "nothing found" may be shown.
    /// Not drawn: the page picks the wording ("Could not read Defender's list").
    CannotRead,
}

/// Everything the page shows, in one read (reading starts no scan).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityPage {
    pub banner: Banner,
    pub status: DefenderStatus,
    pub antivirus: Vec<AvProduct>,
    /// "Last scan" line: the newest of the quick / full scan end times and which one it was.
    pub last_scan: Option<(Stamp, ScanKind)>,
    /// Detections waiting for the user's choice.
    pub threats: Vec<ThreatRow>,
    pub quarantine: Vec<ThreatRow>,
    /// The quarantine rows come from Defender's detection history (the exact list, `MpCmdRun -Restore -ListAll`, needs admin:
    /// "You need administrator privilege", 0x80070005, shown on this PC from a non-elevated shell). History and the real
    /// quarantine can differ: Defender purges items after 90 days, and a restore done outside this app leaves the history as it was.
    pub quarantine_from_history: bool,
    /// The allow list (the Security reset line): how many entries Defender has. Empty when it cannot be read.
    pub allowed: Vec<AllowedThreat>,
    /// Why the detection list could not be read, when it could not: Threats found and Quarantine are then EMPTY BECAUSE UNKNOWN,
    /// not because nothing was found. The actions (Remove / Allow / Restore) return this error.
    pub unreadable: Option<SecurityError>,
}

/// The newest finished scan of the two.
pub fn last_scan_of(status: &DefenderStatus) -> Option<(Stamp, ScanKind)> {
    match (status.quick_scan_end, status.full_scan_end) {
        (Some(q), Some(f)) if f > q => Some((f, ScanKind::Full)),
        (Some(q), _) => Some((q, ScanKind::Quick)),
        (None, Some(f)) => Some((f, ScanKind::Full)),
        (None, None) => None,
    }
}

/// The other antivirus that is on, if Defender is not the active one.
pub fn other_antivirus(status: &DefenderStatus, products: &[AvProduct]) -> Option<String> {
    let defender_active = status.running_mode == RunningMode::Normal && status.antivirus_enabled;
    if defender_active {
        return None;
    }
    products.iter().find(|p| p.on && !p.is_defender()).map(|p| p.name.clone())
}

/// The banner. `scanning` is the running scan's title, if any.
pub fn banner_for(status: &DefenderStatus, products: &[AvProduct], active_threats: usize, scanning: Option<&str>) -> Banner {
    if let Some(name) = other_antivirus(status, products) {
        return Banner::OtherAntivirus { name };
    }
    if !status.service_enabled || !status.antivirus_enabled || status.running_mode == RunningMode::NotRunning {
        return Banner::ProtectionOff;
    }
    if let Some(t) = scanning {
        return Banner::Scanning { title: t.to_string() };
    }
    if active_threats > 0 {
        return Banner::NeedsAttention { threats: active_threats };
    }
    if !status.realtime_enabled {
        return Banner::ProtectionOff;
    }
    Banner::Protected
}

/// Parse a CIM datetime ("20261007163037.000000+000" = yyyymmddHHMMSS.ffffff±UUU, UUU = minutes from UTC) into
/// (year, month, day, hour, minute, second, offset minutes). `None` for an empty / zero date ("never").
pub fn parse_cim_datetime(s: &str) -> Option<(u16, u8, u8, u8, u8, u8, i32)> {
    let b = s.trim().as_bytes();
    if b.len() < 25 || !b[..14].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let n = |r: std::ops::Range<usize>| std::str::from_utf8(&b[r]).ok()?.parse::<u32>().ok();
    let year = n(0..4)? as u16;
    if year < 1980 {
        return None; // 16010101... = never
    }
    let off = n(22..25)? as i32;
    let off = if b[21] == b'-' { -off } else { off };
    Some((year, n(4..6)? as u8, n(6..8)? as u8, n(8..10)? as u8, n(10..12)? as u8, n(12..14)? as u8, off))
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The UTC (year, month, day, hour, minute, second) of a parsed CIM datetime (undo its offset).
pub fn cim_to_utc(p: (u16, u8, u8, u8, u8, u8, i32)) -> (u16, u8, u8, u8, u8, u8) {
    let (y, mo, d, h, mi, s, off) = p;
    let secs = days_from_civil(y as i64, mo as i64, d as i64) * 86_400 + h as i64 * 3600 + mi as i64 * 60 + s as i64 - off as i64 * 60;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (yy, mm, dd) = civil_from_days(days);
    (yy as u16, mm as u8, dd as u8, (rem / 3600) as u8, (rem % 3600 / 60) as u8, (rem % 60) as u8)
}

/// One entry of Defender's allow list (`ThreatIDDefaultAction`, action Allow): the "Allowed in Defender · N" reset line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowedThreat {
    pub threat_id: i64,
    /// The threat's name when Defender still knows it, else "Threat <id>".
    pub name: String,
    /// The files Defender saw for this threat (from the detection history; may be empty).
    pub files: Vec<String>,
}

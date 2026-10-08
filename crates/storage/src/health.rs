//! Drive health (DESIGN §3.12 item 4), read-only: temperature, life left %, power-on hours, status + warnings.
//!
//! Sources, best first (research E §6): NVMe health log (`IOCTL_STORAGE_QUERY_PROPERTY`, log page 0x02) · the
//! storage temperature property · SATA SMART attributes (`SMART_RCV_DRIVE_DATA`, READ DATA 0xD0 + READ THRESHOLDS
//! 0xD1 — reads only) · WMI `MSFT_StorageReliabilityCounter` · Windows' own verdict `MSFT_PhysicalDisk.HealthStatus`
//! (no admin). Whatever needs admin and was not readable is listed in `admin_would_add`.

use crate::{DriveInfo, HealthRaw, MediaKind, OsHealthStatus, PhysicalDisk, Result, StorageOs};

/// Temperature pill colour (DESIGN thresholds: amber ≥ 75 °C, red ≥ 85 °C).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TempLevel {
    Ok,
    Warm,
    Hot,
}

pub fn temp_level(c: i32) -> TempLevel {
    if c >= 85 {
        TempLevel::Hot
    } else if c >= 75 {
        TempLevel::Warm
    } else {
        TempLevel::Ok
    }
}

/// One row of the health table.
#[derive(Debug, Clone, PartialEq)]
pub struct HealthRow {
    pub disk_number: u32,
    /// Drive letters on this disk ("C", "G").
    pub letters: Vec<char>,
    pub model: String,
    pub media: MediaKind,
    pub temperature_c: Option<i32>,
    /// Life left %, `None` = "—" (hard drives don't report one; tip says so).
    pub life_left_pct: Option<u8>,
    pub power_on_hours: Option<u64>,
    /// Empty = "● Healthy" (when Windows or the drive said so).
    pub warnings: Vec<String>,
    /// Windows' own verdict, if any.
    pub os_status: Option<OsHealthStatus>,
    /// What an admin read would add ("temperature", "power-on hours" …) — the menu can say so.
    pub admin_would_add: Vec<String>,
}

impl HealthRow {
    /// "● Healthy" / "● 1 warning" / "—" (nothing known).
    pub fn status_text(&self) -> String {
        match self.warnings.len() {
            0 if self.os_status.is_some() || self.temperature_c.is_some() || self.life_left_pct.is_some() => {
                "● Healthy".to_string()
            }
            0 => "—".to_string(),
            1 => "● 1 warning".to_string(),
            n => format!("● {n} warnings"),
        }
    }
    pub fn temp_level(&self) -> Option<TempLevel> {
        self.temperature_c.map(temp_level)
    }
}

/// Read every physical disk's health and map drive letters onto disks. USB sticks / SD cards come back with "—".
pub fn read_all(os: &dyn StorageOs) -> Result<Vec<HealthRow>> {
    let drives = os.drives().unwrap_or_default();
    let mut rows = Vec::new();
    for disk in os.physical_disks()? {
        let raw = os.disk_health(disk.number).unwrap_or_default();
        rows.push(build_row(&disk, &raw, &drives));
    }
    Ok(rows)
}

/// Turn what was read into one table row (pure; tested with made-up readings).
pub fn build_row(disk: &PhysicalDisk, raw: &HealthRaw, drives: &[DriveInfo]) -> HealthRow {
    let mut letters: Vec<char> = drives.iter().filter(|d| d.disk_number == Some(disk.number)).map(|d| d.letter).collect();
    letters.sort();
    let first_letter = letters.first().map(|l| format!("{l}:")).unwrap_or_else(|| "This drive".to_string());
    let mut warnings = Vec::new();
    let mut temperature_c = raw.temperature_c;
    let mut life_left_pct = None;
    let mut power_on_hours = None;

    if let Some(n) = &raw.nvme {
        if n.temperature_kelvin > 0 {
            temperature_c = Some(n.temperature_kelvin as i32 - 273);
        }
        life_left_pct = Some(100u8.saturating_sub(n.percentage_used.min(100)));
        power_on_hours = Some(n.power_on_hours);
        let w = n.critical_warning;
        if w & 0x01 != 0 || (n.available_spare_threshold_pct > 0 && n.available_spare_pct < n.available_spare_threshold_pct) {
            warnings.push(format!("{first_letter} spare space is almost used up · back up what matters."));
        }
        if w & 0x02 != 0 {
            warnings.push(format!("{first_letter} ran too hot or too cold."));
        }
        if w & 0x04 != 0 {
            warnings.push(format!("{first_letter} reports its reliability is degraded · back up what matters."));
        }
        if w & 0x08 != 0 {
            warnings.push(format!("{first_letter} has switched itself to read-only · back up what matters."));
        }
        if w & 0x10 != 0 {
            warnings.push(format!("{first_letter} power-loss backup failed."));
        }
        if n.media_errors > 0 {
            warnings.push(format!("{first_letter} {} data errors were found · back up what matters.", n.media_errors));
        }
    }

    if !raw.smart.is_empty() {
        let attr = |id: u8| raw.smart.iter().find(|a| a.id == id);
        if temperature_c.is_none() {
            // 194 (0xC2) Temperature: lowest raw byte = °C. 190 (0xBE) airflow temperature as a fallback.
            temperature_c = attr(194).or(attr(190)).map(|a| (a.raw & 0xFF) as i32).filter(|&t| t > 0 && t < 120);
        }
        if power_on_hours.is_none() {
            // 9 Power-on hours: low 32 bits (some vendors pack minutes / ms in the upper bytes).
            power_on_hours = attr(9).map(|a| a.raw & 0xFFFF_FFFF);
        }
        if disk.media == MediaKind::Ssd && life_left_pct.is_none() {
            // SATA SSD life (normalised value, 100 = new): 231 SSD life left · 233 media wearout · 177 Samsung wear
            // levelling · 202 Crucial/Micron percent lifetime remaining · 169 remaining life. Vendor-specific (guess
            // order); hard drives get "—" by design.
            life_left_pct = [231u8, 233, 177, 202, 169].iter().find_map(|&id| attr(id)).map(|a| a.value.min(100));
        }
        let raw_of = |id: u8| attr(id).map(|a| a.raw & 0xFFFF_FFFF).unwrap_or(0);
        let moved = raw_of(5);
        if moved > 0 {
            warnings.push(format!("{first_letter} {moved} sectors were moved to spares · back up what matters."));
        }
        let pending = raw_of(197);
        if pending > 0 {
            warnings.push(format!("{first_letter} {pending} sectors are waiting to be moved · back up what matters."));
        }
        let uncorrectable = raw_of(198);
        if uncorrectable > 0 {
            warnings.push(format!("{first_letter} {uncorrectable} sectors could not be read · back up what matters."));
        }
        for a in &raw.smart {
            if let Some(th) = a.threshold {
                if th > 0 && a.value <= th {
                    warnings.push(format!("{first_letter} SMART attribute {} is past the drive's own limit.", a.id));
                }
            }
        }
    }

    if let Some(r) = &raw.reliability {
        if temperature_c.is_none() {
            temperature_c = r.temperature_c.filter(|&t| t > 0).map(|t| t as i32);
        }
        if power_on_hours.is_none() {
            power_on_hours = r.power_on_hours;
        }
        if life_left_pct.is_none() && disk.media != MediaKind::Hdd {
            life_left_pct = r.wear_pct.map(|w| 100u8.saturating_sub(w.min(100) as u8));
        }
    }

    match raw.os_status {
        Some(OsHealthStatus::Warning) if warnings.is_empty() => {
            warnings.push(format!("{first_letter} Windows reports a warning for this drive · back up what matters."))
        }
        Some(OsHealthStatus::Unhealthy) => {
            warnings.push(format!("{first_letter} Windows reports this drive as unhealthy · back up now."))
        }
        _ => {}
    }

    if disk.media == MediaKind::Hdd {
        life_left_pct = None; // "Hard drives don't report a life %"
    }

    HealthRow {
        disk_number: disk.number,
        letters,
        model: disk.model.clone(),
        media: disk.media,
        temperature_c,
        life_left_pct,
        power_on_hours,
        warnings,
        os_status: raw.os_status,
        admin_would_add: raw.needs_admin.clone(),
    }
}

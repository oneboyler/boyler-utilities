//! Drive tiles (DESIGN §3.12 item 1): one per local drive, "612 GB free of 1.82 TB", a used bar, amber under 10 % free.

use crate::{DriveInfo, DriveKind, MediaKind, Result, StorageOs, VolumeState};

/// One drive tile.
#[derive(Debug, Clone, PartialEq)]
pub struct DriveTile {
    pub info: DriveInfo,
    pub used_bytes: u64,
    /// 0.0 – 1.0 of the bar.
    pub used_fraction: f64,
    /// Under 10 % free → the bar turns amber.
    pub low_space: bool,
    /// USB stick / SD card.
    pub removable: bool,
}

impl DriveTile {
    /// The tile's title: "C:  <label>" ("Local Disk" when the volume has no label).
    pub fn title(&self) -> String {
        let label = if self.info.label.is_empty() { "Local Disk" } else { &self.info.label };
        format!("{}:  {}", self.info.letter, label)
    }

    /// Hover text: model + type ("Samsung SSD 970 EVO 1TB · NVMe SSD").
    pub fn hover(&self) -> String {
        let kind = match self.info.media {
            MediaKind::Ssd => "SSD",
            MediaKind::Hdd => "Hard drive",
            MediaKind::Unknown => "Drive",
        };
        let kind = match &self.info.bus {
            Some(bus) => format!("{bus} {kind}"),
            None => kind.to_string(),
        };
        match &self.info.model {
            Some(model) => format!("{model} · {kind}"),
            None => kind,
        }
    }
}

/// Local drives as tiles, sorted by letter. Network drives and CD drives are not listed (DESIGN edge cases); USB
/// sticks / SD cards are. A BitLocker-locked drive is listed with `state == Locked` and no sizes.
pub fn list(os: &dyn StorageOs) -> Result<Vec<DriveTile>> {
    let mut tiles: Vec<DriveTile> = os
        .drives()?
        .into_iter()
        .filter(|d| matches!(d.kind, DriveKind::Fixed | DriveKind::Removable | DriveKind::RamDisk))
        .map(|info| {
            let used = info.total_bytes.saturating_sub(info.free_bytes);
            let ready = info.state == VolumeState::Ready && info.total_bytes > 0;
            let used_fraction = if ready { used as f64 / info.total_bytes as f64 } else { 0.0 };
            DriveTile {
                used_bytes: used,
                used_fraction,
                low_space: ready && (info.free_bytes as f64) < info.total_bytes as f64 * 0.10,
                removable: info.kind == DriveKind::Removable,
                info,
            }
        })
        .collect();
    tiles.sort_by_key(|t| t.info.letter);
    Ok(tiles)
}

/// The tile chosen when the page opens: the system drive (C:), else the first one.
pub fn default_choice(tiles: &[DriveTile]) -> Option<char> {
    tiles.iter().find(|t| t.info.is_system).or(tiles.first()).map(|t| t.info.letter)
}

/// Sizes the way the drawing and Explorer write them: "612 GB", "1.82 TB", "205 MB". Like Explorer, 1 GB here is
/// 1024³ bytes (a "2 TB" drive shows "1.82 TB", as in the drawing).
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else if value >= 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

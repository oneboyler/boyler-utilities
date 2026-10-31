//! Which NVIDIA graphics card this PC has and which driver it runs - read from Windows itself (the display adapters
//! Windows lists as present), no NVIDIA app needed.

use crate::error::Result;
use crate::version::DriverVersion;

/// NVIDIA's PCI vendor id.
pub const NVIDIA_VENDOR: u16 = 0x10DE;

/// One display adapter as Windows lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    /// Windows' device name, e.g. "NVIDIA GeForce RTX 4090".
    pub name: String,
    /// The first hardware id, e.g. `PCI\VEN_10DE&DEV_2684&SUBSYS_51031462&REV_A1`.
    pub hardware_id: String,
    /// Windows' driver version, e.g. "32.0.16.1088" (empty when Windows has none).
    pub driver_version: String,
}

impl Adapter {
    /// The PCI vendor id from the hardware id (`VEN_xxxx`).
    pub fn vendor(&self) -> Option<u16> {
        let up = self.hardware_id.to_ascii_uppercase();
        let i = up.find("VEN_")?;
        u16::from_str_radix(up.get(i + 4..i + 8)?, 16).ok()
    }
}

/// The OS reads the driver card needs (real: Windows; fake: tests).
pub trait GpuOs: Send + Sync {
    /// The display adapters Windows lists as present now.
    fn adapters(&self) -> Result<Vec<Adapter>>;
    /// Windows' build number (22000 and up = Windows 11).
    fn os_build(&self) -> u32;
    /// A laptop (Windows reports a system battery) - NVIDIA lists laptop chips apart from desktop cards of the same name.
    fn is_laptop(&self) -> bool;
}

/// The NVIDIA card the driver card is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NvidiaGpu {
    pub name: String,
    /// `None` when Windows' version can't be read (then "Update" is always offered).
    pub installed: Option<DriverVersion>,
}

/// The first present NVIDIA adapter, or `None` (no NVIDIA card = no driver card on the Display tab).
pub fn detect(os: &dyn GpuOs) -> Result<Option<NvidiaGpu>> {
    Ok(os
        .adapters()?
        .into_iter()
        .find(|a| a.vendor() == Some(NVIDIA_VENDOR))
        .map(|a| NvidiaGpu { installed: DriverVersion::from_windows(&a.driver_version), name: a.name }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeGpuOs;

    fn adapter(name: &str, hw: &str, ver: &str) -> Adapter {
        Adapter { name: name.into(), hardware_id: hw.into(), driver_version: ver.into() }
    }

    #[test]
    fn finds_the_nvidia_card_next_to_another_gpu() {
        // his PC (measured): the AMD graphics inside the CPU is listed too
        let os = FakeGpuOs::new(vec![
            adapter("AMD Radeon(TM) Graphics", r"PCI\VEN_1002&DEV_164E&SUBSYS_D0001458&REV_C9", "31.0.22044.1"),
            adapter("NVIDIA GeForce RTX 4090", r"PCI\VEN_10DE&DEV_2684&SUBSYS_51031462&REV_A1", "32.0.16.1088"),
        ]);
        let g = detect(&os).unwrap().unwrap();
        assert_eq!(g.name, "NVIDIA GeForce RTX 4090");
        assert_eq!(g.installed.unwrap().to_string(), "610.88");
    }

    #[test]
    fn no_nvidia_card_means_nothing() {
        let os = FakeGpuOs::new(vec![adapter("Intel(R) UHD Graphics 770", r"PCI\VEN_8086&DEV_4680", "31.0.101.4502")]);
        assert_eq!(detect(&os).unwrap(), None);
        assert_eq!(detect(&FakeGpuOs::new(vec![])).unwrap(), None);
    }

    #[test]
    fn an_unreadable_windows_version_is_kept_as_unknown() {
        let os = FakeGpuOs::new(vec![adapter("NVIDIA GeForce RTX 3080", r"pci\ven_10de&dev_2206", "")]);
        assert_eq!(detect(&os).unwrap().unwrap().installed, None);
    }
}

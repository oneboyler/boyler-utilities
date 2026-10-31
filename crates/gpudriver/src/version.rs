//! NVIDIA's driver version ("610.88") and how Windows writes the same driver ("32.0.16.1088").

use std::fmt;

/// A driver version as NVIDIA names it: `major.minor`, e.g. 610.88. Ordered (610.88 < 616.56 < 621.01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DriverVersion {
    pub major: u32,
    pub minor: u32,
}

impl DriverVersion {
    /// "610.88" (exactly digits.two digits, as NVIDIA's lists write it).
    pub fn parse(s: &str) -> Option<DriverVersion> {
        let (a, b) = s.trim().split_once('.')?;
        if a.is_empty() || a.len() > 4 || b.len() != 2 || !a.bytes().chain(b.bytes()).all(|c| c.is_ascii_digit()) {
            return None;
        }
        Some(DriverVersion { major: a.parse().ok()?, minor: b.parse().ok()? })
    }

    /// Windows' driver version ("32.0.16.1088") -> NVIDIA's (610.88): the last digit of the third part and the four
    /// digits of the fourth make the five digits of NVIDIA's number.
    pub fn from_windows(s: &str) -> Option<DriverVersion> {
        let p: Vec<&str> = s.trim().split('.').collect();
        if p.len() != 4 || !p.iter().all(|x| !x.is_empty() && x.bytes().all(|c| c.is_ascii_digit())) {
            return None;
        }
        let third: u64 = p[2].parse().ok()?;
        let fourth: u64 = p[3].parse().ok()?;
        if fourth > 9999 {
            return None;
        }
        let n = (third % 10) * 10000 + fourth;
        Some(DriverVersion { major: (n / 100) as u32, minor: (n % 100) as u32 })
    }
}

impl fmt::Display for DriverVersion {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}.{:02}", self.major, self.minor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_numbers_become_nvidias() {
        // his RTX 4090 (measured: Win32_VideoController DriverVersion 32.0.16.1088 = 610.88 in NVIDIA's list)
        assert_eq!(DriverVersion::from_windows("32.0.16.1088").unwrap().to_string(), "610.88");
        assert_eq!(DriverVersion::from_windows("32.0.15.6094").unwrap().to_string(), "560.94");
        assert_eq!(DriverVersion::from_windows("31.0.15.5222").unwrap().to_string(), "552.22");
        assert_eq!(DriverVersion::from_windows("32.0.16.2101").unwrap().to_string(), "621.01");
        for bad in ["", "32.0.16", "32.0.16.x", "32.0.16.10880", "a.b.c.d"] {
            assert_eq!(DriverVersion::from_windows(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn nvidia_numbers_read_and_order() {
        let v = |s| DriverVersion::parse(s).unwrap();
        assert!(v("610.88") < v("616.56") && v("616.56") < v("621.01"));
        assert_eq!(v("560.70").to_string(), "560.70");
        for bad in ["", "610", "610.8", "610.888", "6a0.88", "..", "610.88.1", "-610.88"] {
            assert_eq!(DriverVersion::parse(bad), None, "{bad:?}");
        }
    }
}

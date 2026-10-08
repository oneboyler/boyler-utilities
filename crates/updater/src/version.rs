use crate::error::{Result, UpdateError};
use std::cmp::Ordering;
use std::fmt;

/// A version like `1.2.3`, `v1.2.3`, `1.2.3-beta.1` (semver-style: a pre-release is OLDER than the same number without it;
/// build metadata after `+` is ignored). One to four dotted numbers; missing ones count as 0 (`1.2` == `1.2.0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    nums: Vec<u64>,
    pre: Vec<PreId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreId {
    Num(u64),
    Text(String),
}

impl Version {
    pub fn parse(text: &str) -> Result<Version> {
        let bad = || UpdateError::BadVersion(text.to_string());
        let t = text.trim();
        let t = t.strip_prefix(['v', 'V']).unwrap_or(t);
        let t = t.split('+').next().unwrap_or("");
        let (core, pre) = match t.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (t, None),
        };
        let mut nums = Vec::new();
        for part in core.split('.') {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(bad());
            }
            nums.push(part.parse::<u64>().map_err(|_| bad())?);
        }
        if nums.is_empty() || nums.len() > 4 {
            return Err(bad());
        }
        let mut ids = Vec::new();
        if let Some(p) = pre {
            for part in p.split('.') {
                if part.is_empty() || !part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                    return Err(bad());
                }
                ids.push(if part.bytes().all(|b| b.is_ascii_digit()) {
                    PreId::Num(part.parse::<u64>().map_err(|_| bad())?)
                } else {
                    PreId::Text(part.to_ascii_lowercase())
                });
            }
        }
        while nums.len() > 1 && nums.last() == Some(&0) {
            nums.pop();
        }
        Ok(Version { nums, pre: ids })
    }

    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut nums: Vec<String> = self.nums.iter().map(u64::to_string).collect();
        while nums.len() < 3 {
            nums.push("0".into());
        }
        write!(f, "{}", nums.join("."))?;
        if !self.pre.is_empty() {
            let pre: Vec<String> = self
                .pre
                .iter()
                .map(|p| match p {
                    PreId::Num(n) => n.to_string(),
                    PreId::Text(t) => t.clone(),
                })
                .collect();
            write!(f, "-{}", pre.join("."))?;
        }
        Ok(())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        // trailing zeros were stripped on parse, so an element-wise compare (missing = 0) is exact
        let n = self.nums.len().max(other.nums.len());
        for i in 0..n {
            let a = self.nums.get(i).copied().unwrap_or(0);
            let b = other.nums.get(i).copied().unwrap_or(0);
            match a.cmp(&b) {
                Ordering::Equal => {}
                o => return o,
            }
        }
        match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => {
                for (a, b) in self.pre.iter().zip(other.pre.iter()) {
                    let o = match (a, b) {
                        (PreId::Num(x), PreId::Num(y)) => x.cmp(y),
                        (PreId::Num(_), PreId::Text(_)) => Ordering::Less,
                        (PreId::Text(_), PreId::Num(_)) => Ordering::Greater,
                        (PreId::Text(x), PreId::Text(y)) => x.cmp(y),
                    };
                    if o != Ordering::Equal {
                        return o;
                    }
                }
                self.pre.len().cmp(&other.pre.len())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn parses_tags_and_shows_three_numbers() {
        assert_eq!(v("v1.2.3").to_string(), "1.2.3");
        assert_eq!(v("V2").to_string(), "2.0.0");
        assert_eq!(v("1.2").to_string(), "1.2.0");
        assert_eq!(v("1.2.3-Beta.1+build5").to_string(), "1.2.3-beta.1");
        assert_eq!(v("  0.1.0 ").to_string(), "0.1.0");
    }

    #[test]
    fn rejects_garbage() {
        for bad in ["", "v", "latest", "1..2", "1.2.3.4.5", "1.x", "-1.0", "1.0-", "1.0-a..b", "1.0.0 beta", "99999999999999999999"] {
            assert!(Version::parse(bad).is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn orders_numbers_numerically_not_as_text() {
        assert!(v("1.10.0") > v("1.9.9"));
        assert!(v("2.0.0") > v("1.99.99"));
        assert!(v("1.0.1") > v("1.0"));
        assert_eq!(v("1.0"), v("1.0.0"));
        assert_eq!(v("v1.0.0"), v("1"));
        assert!(v("1.2.3.1") > v("1.2.3"));
    }

    #[test]
    fn prerelease_is_older_than_the_release() {
        assert!(v("1.0.0-beta") < v("1.0.0"));
        assert!(v("1.0.0-alpha") < v("1.0.0-beta"));
        assert!(v("1.0.0-beta.2") < v("1.0.0-beta.11"));
        assert!(v("1.0.0-1") < v("1.0.0-alpha"));
        assert!(v("1.0.0-beta") < v("1.0.0-beta.1"));
        assert!(v("1.0.1-beta") > v("1.0.0"));
        assert!(v("1.0.0-rc.1").is_prerelease());
        assert!(!v("1.0.0").is_prerelease());
    }
}

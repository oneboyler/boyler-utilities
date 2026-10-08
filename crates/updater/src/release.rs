//! Reading what GitHub's releases API sends (`GET /repos/{owner}/{repo}/releases/latest`) and choosing the Windows file.
//!
//! What GitHub gives per release (checked against GitHub's docs/changelog, 2025-06-03 "releases now expose digests"):
//! `tag_name`, `name`, `body` (the notes), `html_url`, `draft`, `prerelease`, and per asset `name`, `size` (bytes),
//! `browser_download_url`, `content_type`, and `digest` = `"sha256:<64 hex>"` (empty/absent for assets uploaded before
//! mid-2025). So SHA-256 verification is possible when `digest` is there; when it is not, only the size is checked.

use crate::error::{Result, UpdateError};
use crate::version::Version;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct RawRelease {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<RawAsset>,
}

#[derive(Debug, Deserialize)]
struct RawAsset {
    name: String,
    #[serde(default)]
    size: u64,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

/// The file to download for a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetInfo {
    pub name: String,
    /// Bytes, as the release lists them (0 = not listed).
    pub size: u64,
    pub url: String,
    /// Lower-case hex SHA-256 when the release lists one (`digest: "sha256:..."`).
    pub sha256: Option<String>,
}

/// A release that is newer than the running app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub version: Version,
    pub tag: String,
    pub title: String,
    /// The release notes text (may be empty).
    pub notes: String,
    pub page_url: String,
    pub asset: AssetInfo,
}

/// A release read from the API, before it is compared with the running version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedRelease {
    pub version: Version,
    pub tag: String,
    pub title: String,
    pub notes: String,
    pub page_url: String,
    pub draft: bool,
    pub prerelease: bool,
    assets: Vec<AssetInfo>,
}

impl ParsedRelease {
    /// The Windows file, or `NoWindowsAsset`. `exe_name` = the running app's file name, preferred when several `.exe` exist.
    pub(crate) fn into_info(self, exe_name: &str) -> Result<ReleaseInfo> {
        let asset = pick_asset(&self.assets, exe_name).ok_or_else(|| UpdateError::NoWindowsAsset { tag: self.tag.clone() })?;
        Ok(ReleaseInfo { version: self.version, tag: self.tag, title: self.title, notes: self.notes, page_url: self.page_url, asset })
    }
}

pub(crate) fn parse_release(json: &[u8]) -> Result<ParsedRelease> {
    let raw: RawRelease = serde_json::from_slice(json).map_err(|e| UpdateError::BadResponse(format!("not a release: {e}")))?;
    let version = Version::parse(&raw.tag_name)
        .map_err(|_| UpdateError::BadResponse(format!("the release tag {:?} is not a version number", raw.tag_name)))?;
    let mut assets = Vec::new();
    for a in raw.assets {
        let sha256 = parse_digest(a.digest.as_deref())?;
        assets.push(AssetInfo { name: a.name, size: a.size, url: a.browser_download_url, sha256 });
    }
    Ok(ParsedRelease {
        version,
        title: raw.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| raw.tag_name.clone()),
        tag: raw.tag_name,
        notes: raw.body.unwrap_or_default(),
        page_url: raw.html_url.unwrap_or_default(),
        draft: raw.draft,
        prerelease: raw.prerelease,
        assets,
    })
}

/// `"sha256:<64 hex>"` -> the hex (lower-case). Absent / empty / another algorithm -> `None` (size-only check).
/// A `sha256:` value that is not 64 hex digits is refused: a damaged value must not quietly turn the check off.
fn parse_digest(d: Option<&str>) -> Result<Option<String>> {
    let Some(d) = d.map(str::trim).filter(|d| !d.is_empty()) else { return Ok(None) };
    let Some((algo, hex)) = d.split_once(':') else { return Ok(None) };
    if !algo.eq_ignore_ascii_case("sha256") {
        return Ok(None);
    }
    if hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(Some(hex.to_ascii_lowercase()))
    } else {
        Err(UpdateError::BadResponse(format!("the listed SHA-256 {hex:?} is not 64 hex digits")))
    }
}

/// The app is a single Windows program, so the release's Windows file is an `.exe`. Several `.exe`: the one named like the
/// running app wins, then one whose name starts with the app's name, then one with `win`/`x64` in its name, then the
/// alphabetically first (so the pick never changes between runs). No `.exe` -> `None`.
/// An installer (`Boyler Utilities Setup.exe`, Order 032) is never the app: a name with `setup` / `install` is skipped unless
/// it IS the running app's name (otherwise the installed `Boyler Utilities.exe` would swap itself for the setup).
fn pick_asset(assets: &[AssetInfo], exe_name: &str) -> Option<AssetInfo> {
    let exe_lower = exe_name.to_ascii_lowercase();
    let stem = exe_lower.strip_suffix(".exe").unwrap_or(&exe_lower).to_string();
    let mut candidates: Vec<(u8, &AssetInfo)> = assets
        .iter()
        .filter(|a| a.name.to_ascii_lowercase().ends_with(".exe"))
        .filter(|a| {
            let n = a.name.to_ascii_lowercase();
            n == exe_lower || !(n.contains("setup") || n.contains("install"))
        })
        .map(|a| {
            let n = a.name.to_ascii_lowercase();
            let score = if n == exe_lower {
                0
            } else if !stem.is_empty() && n.starts_with(&stem) {
                1
            } else if n.contains("win") || n.contains("x64") {
                2
            } else {
                3
            };
            (score, a)
        })
        .collect();
    candidates.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.to_ascii_lowercase().cmp(&b.1.name.to_ascii_lowercase())));
    candidates.first().map(|(_, a)| (*a).clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn json(tag: &str, assets: &str) -> Vec<u8> {
        format!(
            r#"{{"tag_name":"{tag}","name":"Version {tag}","body":"- faster\n- fixes","html_url":"https://github.com/o/r/releases/tag/{tag}",
            "draft":false,"prerelease":false,"assets":[{assets}]}}"#
        )
        .into_bytes()
    }

    fn asset(name: &str, size: u64, digest: &str) -> String {
        format!(r#"{{"name":"{name}","size":{size},"browser_download_url":"https://github.com/o/r/releases/download/v2/{name}","digest":{digest},"content_type":"application/octet-stream"}}"#)
    }

    #[test]
    fn reads_a_normal_github_release() {
        let a = asset("BoylerUtilities.exe", 1234, &format!("\"sha256:{HEX}\""));
        let p = parse_release(&json("v2.0.0", &a)).unwrap();
        assert_eq!(p.version.to_string(), "2.0.0");
        assert_eq!(p.title, "Version v2.0.0");
        assert_eq!(p.notes, "- faster\n- fixes");
        let info = p.into_info("BoylerUtilities.exe").unwrap();
        assert_eq!(info.asset.name, "BoylerUtilities.exe");
        assert_eq!(info.asset.size, 1234);
        assert_eq!(info.asset.sha256.as_deref(), Some(HEX));
        assert!(info.asset.url.ends_with("/v2/BoylerUtilities.exe"));
    }

    #[test]
    fn digest_missing_null_empty_or_other_algorithm_means_size_only() {
        for d in ["null", "\"\"", "\"sha512:abcd\"", "\"nocolon\""] {
            let p = parse_release(&json("1.1.0", &asset("a.exe", 5, d))).unwrap();
            assert_eq!(p.into_info("x.exe").unwrap().asset.sha256, None, "digest {d}");
        }
        let no_field = br#"{"tag_name":"1.1.0","assets":[{"name":"a.exe","size":5,"browser_download_url":"https://x/a.exe"}]}"#;
        assert_eq!(parse_release(no_field).unwrap().into_info("x.exe").unwrap().asset.sha256, None);
    }

    #[test]
    fn a_damaged_sha256_value_is_refused_not_ignored() {
        for d in ["\"sha256:zz\"", "\"sha256:\"", &format!("\"sha256:{}\"", &HEX[..63])] {
            let r = parse_release(&json("1.1.0", &asset("a.exe", 5, d)));
            assert!(matches!(r, Err(UpdateError::BadResponse(_))), "{d}");
        }
    }

    #[test]
    fn upper_case_digest_is_normalised() {
        let up = HEX.to_ascii_uppercase();
        let p = parse_release(&json("1.1.0", &asset("a.exe", 5, &format!("\"SHA256:{up}\"")))).unwrap();
        assert_eq!(p.into_info("x.exe").unwrap().asset.sha256.as_deref(), Some(HEX));
    }

    #[test]
    fn picks_the_windows_exe() {
        let assets = [
            asset("notes.txt", 1, "null"),
            asset("BoylerUtilities-2.0.0-linux.tar.gz", 1, "null"),
            asset("Other.exe", 1, "null"),
            asset("BoylerUtilities-setup-win64.exe", 1, "null"),
            asset("BoylerUtilities.exe", 1, "null"),
        ]
        .join(",");
        let p = parse_release(&json("2.0.0", &assets)).unwrap();
        assert_eq!(p.clone().into_info("BoylerUtilities.exe").unwrap().asset.name, "BoylerUtilities.exe");
        // no exact name: the one that starts like the app, alphabetically first
        let two = [asset("Other.exe", 1, "null"), asset("BoylerUtilities-2.0.0-win64.exe", 1, "null")].join(",");
        let p2 = parse_release(&json("2.0.0", &two)).unwrap();
        assert_eq!(p2.into_info("BoylerUtilities.exe").unwrap().asset.name, "BoylerUtilities-2.0.0-win64.exe");
        // only an unrelated exe: it is still the Windows file
        let zs = [asset("Z.exe", 1, "null"), asset("A.exe", 1, "null")].join(",");
        let p3 = parse_release(&json("2.0.0", &zs)).unwrap();
        assert_eq!(p3.into_info("BoylerUtilities.exe").unwrap().asset.name, "A.exe");
        // case-insensitive
        let p4 = parse_release(&json("2.0.0", &asset("BOYLERUTILITIES.EXE", 1, "null"))).unwrap();
        assert_eq!(p4.into_info("BoylerUtilities.exe").unwrap().asset.name, "BOYLERUTILITIES.EXE");
    }

    #[test]
    fn no_exe_means_no_windows_asset() {
        let p = parse_release(&json("2.0.0", &asset("src.zip", 1, "null"))).unwrap();
        assert_eq!(p.into_info("x.exe"), Err(UpdateError::NoWindowsAsset { tag: "2.0.0".into() }));
        let p = parse_release(&json("2.0.0", "")).unwrap();
        assert!(matches!(p.into_info("x.exe"), Err(UpdateError::NoWindowsAsset { .. })));
    }

    #[test]
    fn the_setup_is_never_picked_as_the_app() {
        // the installed exe's name is the setup's name start: without the skip the setup scored "starts with the app's name"
        let both = format!("{},{}", asset("BoylerUtilities.exe", 10, "null"), asset("Boyler Utilities Setup.exe", 9, "null"));
        let p = parse_release(&json("2.0.0", &both)).unwrap();
        assert_eq!(p.into_info("Boyler Utilities.exe").unwrap().asset.name, "BoylerUtilities.exe");
        let named = format!("{},{}", asset("Boyler Utilities Setup.exe", 9, "null"), asset("Boyler Utilities.exe", 10, "null"));
        let p = parse_release(&json("2.0.0", &named)).unwrap();
        assert_eq!(p.into_info("Boyler Utilities.exe").unwrap().asset.name, "Boyler Utilities.exe");
        // only a setup in the release: no app file, never the setup
        let p = parse_release(&json("2.0.0", &asset("Boyler Utilities Setup.exe", 9, "null"))).unwrap();
        assert!(matches!(p.into_info("Boyler Utilities.exe"), Err(UpdateError::NoWindowsAsset { .. })));
        let p = parse_release(&json("2.0.0", &asset("BoylerUtilities-installer.exe", 9, "null"))).unwrap();
        assert!(matches!(p.into_info("BoylerUtilities.exe"), Err(UpdateError::NoWindowsAsset { .. })));
    }

    #[test]
    fn bad_json_and_bad_tags_are_bad_responses() {
        assert!(matches!(parse_release(b"<html>rate limited</html>"), Err(UpdateError::BadResponse(_))));
        assert!(matches!(parse_release(b"{}"), Err(UpdateError::BadResponse(_))));
        assert!(matches!(parse_release(&json("nightly", "")), Err(UpdateError::BadResponse(_))));
    }

    #[test]
    fn title_falls_back_to_the_tag() {
        let p = parse_release(br#"{"tag_name":"v3","name":"","assets":[]}"#).unwrap();
        assert_eq!(p.title, "v3");
        assert_eq!(p.notes, "");
    }
}

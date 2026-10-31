//! NVIDIA's own driver lookup (the one nvidia.com's driver search uses; TinyNvidiaUpdateChecker uses it too). Proven
//! read-only on Oct 30 2026 with the owner's RTX 4090 on Windows 11 64-bit.
//!
//! 1. The card's name -> NVIDIA's product ids. `lookupValueSearch.aspx?TypeID=3` lists every product as
//!    `<LookupValue ParentID="<series id = psid>"><Name>..</Name><Value><product id = pfid></Value>`; Windows names the
//!    card the same way, with or without the "NVIDIA " in front ("NVIDIA GeForce RTX 4090" = pfid 995 in series 127,
//!    "GeForce RTX 40 Series"). Some names are listed twice, desktop and laptop (e.g. "GeForce GTX 1060" in "GeForce
//!    10 Series" and "GeForce 10 Series (Notebooks)"); then `TypeID=2` (the series names) and whether the PC has a
//!    battery pick one - the laptop driver is a different download.
//! 2. Windows -> NVIDIA's OS id: Windows 11 = 135, Windows 10 64-bit = 57 (`TypeID=4`).
//! 3. `AjaxDriverService.php?func=DriverManualLookup&psid=..&pfid=..&osID=..&dch=1&upCRD=0&numberOfResults=50` -> JSON,
//!    newest first: `IDS[].downloadInfo` with `Version` ("621.01"), `ReleaseDateTime` ("Wed Oct 21, 2026"),
//!    `DownloadURL` (https://us.download.nvidia.com/Windows/621.01/621.01-desktop-win10-win11-64bit-international-dch-whql.exe)
//!    and `DownloadURLFileSize` ("1.04 GB"). `upCRD=0` = Game Ready drivers only (1 = Studio). A card NVIDIA doesn't
//!    know answers `"Success" : "0"`.

use crate::detect::GpuOs;
use crate::error::{DriverError, Result};
use crate::version::DriverVersion;
use bu_updater::{Http, Request};

/// Every NVIDIA product (card) with its series.
pub const PRODUCTS_URL: &str = "https://www.nvidia.com/Download/API/lookupValueSearch.aspx?TypeID=3";
/// Every series' name (only read when a card's name is listed in more than one series).
pub const SERIES_URL: &str = "https://www.nvidia.com/Download/API/lookupValueSearch.aspx?TypeID=2";
const DRIVERS_BASE: &str = "https://gfwsl.geforce.com/services_toolkit/services/com/nvidia/services/AjaxDriverService.php";
/// How many drivers "Pick a version" lists (newest first; ~2 years of Game Ready drivers).
pub const LIST_LENGTH: u32 = 50;
/// Biggest answer read from NVIDIA's lookups (the product list is ~93 KB, 50 drivers ~400 KB).
const MAX_ANSWER: usize = 8 * 1024 * 1024;

/// NVIDIA's ids of one card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Product {
    /// series id
    pub psid: u32,
    /// product id
    pub pfid: u32,
}

/// One driver NVIDIA offers for the card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: DriverVersion,
    /// "21 Oct 2026" (as NVIDIA dates it), empty when unreadable.
    pub date: String,
    /// NVIDIA's https download link.
    pub url: String,
    /// "1.04 GB" (NVIDIA's own text).
    pub size: String,
}

/// One `<LookupValue>` of NVIDIA's lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookupValue {
    pub parent: u32,
    pub name: String,
    pub value: u32,
}

/// NVIDIA's OS id for this Windows (the app is 64-bit only).
pub fn os_id(build: u32) -> u32 {
    if build >= 22000 {
        135
    } else {
        57
    }
}

fn between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let a = s.find(open)? + open.len();
    let b = s[a..].find(close)? + a;
    Some(&s[a..b])
}

fn unescape_xml(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// The `<LookupValue>`s of a `lookupValueSearch.aspx` answer (entries without a readable name / value are skipped).
pub fn parse_lookup_values(xml: &str) -> Vec<LookupValue> {
    xml.split("<LookupValue ")
        .skip(1)
        .filter_map(|e| {
            let e = &e[..e.find("</LookupValue>")?];
            let parent = between(e, "ParentID=\"", "\"")?.trim().parse().ok()?;
            let name = unescape_xml(between(e, "<Name>", "</Name>")?.trim());
            let value = between(e, "<Value>", "</Value>")?.trim().parse().ok()?;
            Some(LookupValue { parent, name, value })
        })
        .collect()
}

/// A card's name as NVIDIA's list and Windows both write it: no "NVIDIA " in front, single spaces, any case.
pub fn normalize(name: &str) -> String {
    let n = name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    n.strip_prefix("nvidia ").map(str::to_string).unwrap_or(n)
}

/// The card's NVIDIA ids. `series` = the series names, needed only when the name is listed in more than one series
/// (then the laptop / desktop series is picked by `laptop`; still more than one = the first).
pub fn find_product(products: &[LookupValue], series: &dyn Fn() -> Result<Vec<LookupValue>>, name: &str, laptop: bool) -> Result<Product> {
    let want = normalize(name);
    let hits: Vec<&LookupValue> = products.iter().filter(|p| normalize(&p.name) == want).collect();
    let pick = match hits.len() {
        0 => return Err(DriverError::UnknownCard(name.to_string())),
        1 => hits[0],
        _ => {
            let names = series()?;
            let notebook = |p: &LookupValue| {
                names.iter().find(|s| s.value == p.parent).is_some_and(|s| s.name.to_lowercase().contains("notebook"))
            };
            hits.iter().copied().find(|p| notebook(p) == laptop).unwrap_or(hits[0])
        }
    };
    Ok(Product { psid: pick.parent, pfid: pick.value })
}

/// The DriverManualLookup link for a card.
pub fn drivers_url(p: Product, os: u32) -> String {
    format!(
        "{DRIVERS_BASE}?func=DriverManualLookup&psid={}&pfid={}&osID={os}&languageCode=1033&beta=0&isWHQL=0&dltype=-1&dch=1&upCRD=0&qnf=0&sort1=0&numberOfResults={LIST_LENGTH}",
        p.psid, p.pfid
    )
}

/// Only NVIDIA's own https download server, a `.exe` under `/Windows/`.
pub fn is_nvidia_download(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let Some((host, path)) = rest.split_once('/') else { return false };
    let host = host.to_ascii_lowercase();
    let host_ok = host == "download.nvidia.com" || host.strip_suffix(".download.nvidia.com").is_some_and(|h| !h.is_empty() && h.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'));
    host_ok
        && path.starts_with("Windows/")
        && path.to_ascii_lowercase().ends_with(".exe")
        && !path.contains("..")
        && path.bytes().all(|c| c.is_ascii_alphanumeric() || b"/._-".contains(&c))
}

/// "Wed Oct 21, 2026" -> "21 Oct 2026".
fn short_date(s: &str) -> String {
    let p: Vec<&str> = s.split([' ', ',']).filter(|x| !x.is_empty()).collect();
    match p[..] {
        [_, mon, day, year] if day.parse::<u32>().is_ok() => format!("{} {mon} {year}", day.trim_start_matches('0')),
        _ => String::new(),
    }
}

/// The drivers of a DriverManualLookup answer, newest version first (no two of one version). Entries with an
/// unreadable version or a link that is not NVIDIA's download server are dropped.
pub fn parse_drivers(json: &str) -> Result<Vec<Release>> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| DriverError::BadResponse(e.to_string()))?;
    let ids = v.get("IDS").and_then(|x| x.as_array()).ok_or_else(|| DriverError::BadResponse("no IDS list".into()))?;
    let mut out: Vec<Release> = Vec::new();
    for i in ids.iter().filter_map(|x| x.get("downloadInfo")) {
        let s = |k: &str| i.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        if s("Success") != "1" {
            continue;
        }
        let (Some(version), url) = (DriverVersion::parse(&s("Version")), s("DownloadURL")) else { continue };
        if !is_nvidia_download(&url) || out.iter().any(|r| r.version == version) {
            continue;
        }
        out.push(Release { version, date: short_date(&s("ReleaseDateTime")), url, size: s("DownloadURLFileSize") });
    }
    out.sort_by_key(|r| std::cmp::Reverse(r.version));
    if out.is_empty() {
        return Err(DriverError::NoDrivers);
    }
    Ok(out)
}

/// GET a small answer as text.
fn get_text(http: &dyn Http, url: &str) -> Result<String> {
    let mut buf = Vec::new();
    let mut sink = Limited { buf: &mut buf, max: MAX_ANSWER };
    let r = http.get(&Request { url, accept: "*/*" }, &mut sink, &mut |_, _| {}).map_err(|e| DriverError::Network(e.to_string()))?;
    if !(200..300).contains(&r.status) {
        return Err(DriverError::HttpStatus(r.status));
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

struct Limited<'a> {
    buf: &'a mut Vec<u8>,
    max: usize,
}

impl std::io::Write for Limited<'_> {
    fn write(&mut self, d: &[u8]) -> std::io::Result<usize> {
        if self.buf.len() + d.len() > self.max {
            return Err(std::io::Error::other("the answer is bigger than expected"));
        }
        self.buf.extend_from_slice(d);
        Ok(d.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// NVIDIA's Game Ready drivers for the card called `name` on this PC, newest first (3 small requests at most).
pub fn lookup(http: &dyn Http, os: &dyn GpuOs, name: &str) -> Result<Vec<Release>> {
    let products = parse_lookup_values(&get_text(http, PRODUCTS_URL)?);
    if products.is_empty() {
        return Err(DriverError::BadResponse("empty product list".into()));
    }
    let series = || Ok(parse_lookup_values(&get_text(http, SERIES_URL)?));
    let p = find_product(&products, &series, name, os.is_laptop())?;
    parse_drivers(&get_text(http, &drivers_url(p, os_id(os.os_build())))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeGpuOs;
    use bu_updater::FakeHttp;

    const PRODUCTS: &str = include_str!("../tests/data/products.xml");
    const SERIES: &str = include_str!("../tests/data/series.xml");
    const DRIVERS: &str = include_str!("../tests/data/drivers.json");
    const NONE: &str = include_str!("../tests/data/drivers_none.json");

    fn nvidia() -> FakeHttp {
        let h = FakeHttp::new();
        h.ok(PRODUCTS_URL, PRODUCTS);
        h.ok(SERIES_URL, SERIES);
        h
    }

    #[test]
    fn his_rtx_4090_on_windows_11_gets_the_game_ready_list() {
        // the recorded answers of the real lookup (Oct 30 2026), cut to 3 drivers
        let h = nvidia();
        let url = drivers_url(Product { psid: 127, pfid: 995 }, 135);
        h.ok(&url, DRIVERS);
        let r = lookup(&h, &FakeGpuOs::new(vec![]), "NVIDIA GeForce RTX 4090").unwrap();
        let v: Vec<String> = r.iter().map(|x| x.version.to_string()).collect();
        assert_eq!(v, ["621.01", "617.42", "610.88"]);
        assert_eq!(r[0].date, "21 Oct 2026");
        assert_eq!(r[0].size, "1.04 GB");
        assert_eq!(r[0].url, "https://us.download.nvidia.com/Windows/621.01/621.01-desktop-win10-win11-64bit-international-dch-whql.exe");
        // one name in one series: the series list is never asked for
        assert_eq!(h.requests(), vec![PRODUCTS_URL.to_string(), url]);
    }

    #[test]
    fn windows_10_asks_for_os_57() {
        let h = nvidia();
        h.ok(&drivers_url(Product { psid: 127, pfid: 995 }, 57), DRIVERS);
        let os = FakeGpuOs { build: 19045, ..FakeGpuOs::new(vec![]) };
        assert_eq!(lookup(&h, &os, "NVIDIA GeForce RTX 4090").unwrap().len(), 3);
    }

    #[test]
    fn names_match_with_or_without_nvidia_in_front() {
        let p = parse_lookup_values(PRODUCTS);
        let no_series = || -> Result<Vec<LookupValue>> { panic!("not needed") };
        assert_eq!(find_product(&p, &no_series, "GeForce RTX 4090", false).unwrap(), Product { psid: 127, pfid: 995 });
        assert_eq!(find_product(&p, &no_series, "nvidia  geforce rtx 4090 d", false).unwrap(), Product { psid: 127, pfid: 1036 });
        // Windows names a laptop chip "NVIDIA GeForce RTX 4090 Laptop GPU", NVIDIA's list without "NVIDIA"
        assert_eq!(find_product(&p, &no_series, "NVIDIA GeForce RTX 4090 Laptop GPU", true).unwrap(), Product { psid: 129, pfid: 1004 });
        assert_eq!(find_product(&p, &no_series, "NVIDIA GeForce RTX 9999", false), Err(DriverError::UnknownCard("NVIDIA GeForce RTX 9999".into())));
    }

    #[test]
    fn a_name_listed_twice_picks_desktop_or_laptop_by_the_pc() {
        let p = parse_lookup_values(PRODUCTS);
        let series = || Ok(parse_lookup_values(SERIES));
        assert_eq!(find_product(&p, &series, "NVIDIA GeForce GTX 1060", false).unwrap(), Product { psid: 101, pfid: 817 });
        assert_eq!(find_product(&p, &series, "NVIDIA GeForce GTX 1060", true).unwrap(), Product { psid: 102, pfid: 821 });
    }

    #[test]
    fn a_card_nvidia_has_no_driver_for() {
        assert_eq!(parse_drivers(NONE), Err(DriverError::NoDrivers));
        assert!(matches!(parse_drivers("<html>"), Err(DriverError::BadResponse(_))));
    }

    #[test]
    fn errors_from_the_site_are_told_apart() {
        let h = FakeHttp::new();
        h.status(PRODUCTS_URL, 503);
        assert_eq!(lookup(&h, &FakeGpuOs::new(vec![]), "NVIDIA GeForce RTX 4090"), Err(DriverError::HttpStatus(503)));
        let h = FakeHttp::new();
        h.route(PRODUCTS_URL, bu_updater::FakeReply::Offline("no network".into()));
        assert!(matches!(lookup(&h, &FakeGpuOs::new(vec![]), "x"), Err(DriverError::Network(_))));
    }

    #[test]
    fn only_nvidia_https_download_links_are_kept() {
        for ok in [
            "https://us.download.nvidia.com/Windows/621.01/621.01-desktop-win10-win11-64bit-international-dch-whql.exe",
            "https://international.download.nvidia.com/Windows/610.88/610.88-notebook-win10-win11-64bit-international-dch-whql.exe",
        ] {
            assert!(is_nvidia_download(ok), "{ok}");
        }
        for bad in [
            "http://us.download.nvidia.com/Windows/621.01/x.exe",
            "https://us.download.nvidia.com.evil.example/Windows/621.01/x.exe",
            "https://evil.example/us.download.nvidia.com/Windows/x.exe",
            "https://us.download.nvidia.com/Windows/621.01/x.zip",
            "https://us.download.nvidia.com/Windows/../x.exe",
            "https://us.download.nvidia.com/other/x.exe",
            "https://user@us.download.nvidia.com/Windows/x.exe",
            "https://us.download.nvidia.com:444/Windows/x.exe",
        ] {
            assert!(!is_nvidia_download(bad), "{bad}");
        }
        let j = DRIVERS.replace("https://us.download.nvidia.com/Windows/617.42", "https://evil.example/Windows/617.42");
        let v: Vec<String> = parse_drivers(&j).unwrap().iter().map(|r| r.version.to_string()).collect();
        assert_eq!(v, ["621.01", "610.88"]);
    }
}

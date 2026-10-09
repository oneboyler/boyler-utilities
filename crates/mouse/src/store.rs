//! "Get more cursors" (Order 066): a short, hand-checked list of cursor packs that are free to use and ship Windows `.cur` /
//! `.ani` files on their official GitHub release pages. Like "Get more sounds": the app does NOT host or copy them - a click
//! downloads the pack's own .zip from its release page to this PC, unpacks its Regular-size folder into the app's folder
//! and registers it as a cursor scheme for this Windows user (no admin). The packs are made by the community; each entry
//! says who made it and under which licence (all GPL-3.0 - the licence of each repository's LICENSE file, checked 2026-10-09;
//! the licence texts are on Settings › About › Licences).
//!
//! Nothing here touches the network itself: the app gives [`Fetch`] (WinHTTP); tests give a fake.

use crate::cursors::{self, guess_roles, is_cursor_bytes, parse_install_inf, parse_scheme_reg, Pack, Role, WinRole, USER_SCHEMES_KEY};
use crate::error::{Error, Result};
use crate::os::{Hive, MouseOs, RegValue};
use crate::service::Mouse;
use crate::zipdir::{self, Item};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One pack of the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Listing {
    /// stable id (the cache folder's name)
    pub id: &'static str,
    /// the name the scheme gets in Windows' Mouse settings
    pub name: &'static str,
    /// the family it belongs to ("Bibata", "macOS style" ...)
    pub family: &'static str,
    /// one plain line: how it looks
    pub about: &'static str,
    /// who made it (the repository's owner)
    pub maker: &'static str,
    pub licence: &'static str,
    /// the repository (its LICENSE is in it)
    pub page: &'static str,
    /// the release's Windows .zip
    pub zip: &'static str,
    /// the .zip's size in bytes, as the release page lists it
    pub bytes: u64,
}

macro_rules! pack {
    ($id:literal, $name:literal, $family:literal, $about:literal, $maker:literal, $page:expr, $zip:literal, $bytes:literal) => {
        Listing { id: $id, name: $name, family: $family, about: $about, maker: $maker, licence: "GPL-3.0", page: $page, zip: $zip, bytes: $bytes }
    };
}

const BIBATA: &str = "https://github.com/ful1e5/Bibata_Cursor";
const APPLE: &str = "https://github.com/ful1e5/apple_cursor";
const GOOGLE: &str = "https://github.com/ful1e5/Google_Cursor";
const XPRO: &str = "https://github.com/ful1e5/XCursor-pro";
const FUCHSIA: &str = "https://github.com/ful1e5/fuchsia-cursor";
const BANANA: &str = "https://github.com/ful1e5/banana-cursor";
const ROSE: &str = "https://github.com/rose-pine/cursors";
const NORDZY: &str = "https://github.com/guillaumeboehm/Nordzy-cursors";

/// The list, in the order the window shows it.
pub const LIST: [Listing; 19] = [
    pack!("bibata-modern-classic", "Bibata Modern Classic", "Bibata", "Rounded, dark with a light rim", "ful1e5", BIBATA, "https://github.com/ful1e5/Bibata_Cursor/releases/download/v2.0.7/Bibata-Modern-Classic-Windows.zip", 11_131_753),
    pack!("bibata-modern-ice", "Bibata Modern Ice", "Bibata", "Rounded, light with a dark rim", "ful1e5", BIBATA, "https://github.com/ful1e5/Bibata_Cursor/releases/download/v2.0.7/Bibata-Modern-Ice-Windows.zip", 10_277_542),
    pack!("bibata-modern-amber", "Bibata Modern Amber", "Bibata", "Rounded, amber", "ful1e5", BIBATA, "https://github.com/ful1e5/Bibata_Cursor/releases/download/v2.0.7/Bibata-Modern-Amber-Windows.zip", 11_979_435),
    pack!("bibata-original-classic", "Bibata Original Classic", "Bibata", "Sharp-cornered, dark with a light rim", "ful1e5", BIBATA, "https://github.com/ful1e5/Bibata_Cursor/releases/download/v2.0.7/Bibata-Original-Classic-Windows.zip", 11_075_566),
    pack!("bibata-original-ice", "Bibata Original Ice", "Bibata", "Sharp-cornered, light with a dark rim", "ful1e5", BIBATA, "https://github.com/ful1e5/Bibata_Cursor/releases/download/v2.0.7/Bibata-Original-Ice-Windows.zip", 10_184_464),
    pack!("macos-black", "macOS style (black)", "macOS style", "Like the Mac's pointer, black", "ful1e5", APPLE, "https://github.com/ful1e5/apple_cursor/releases/download/v2.0.1/macOS-Windows.zip", 2_813_335),
    pack!("macos-white", "macOS style (white)", "macOS style", "Like the Mac's pointer, white", "ful1e5", APPLE, "https://github.com/ful1e5/apple_cursor/releases/download/v2.0.1/macOS-White-Windows.zip", 2_640_363),
    pack!("google-dot-black", "Google Dot Black", "Google Dot", "A round dot pointer, black", "ful1e5", GOOGLE, "https://github.com/ful1e5/Google_Cursor/releases/download/v2.0.0/GoogleDot-Black-Windows.zip", 897_098),
    pack!("google-dot-white", "Google Dot White", "Google Dot", "A round dot pointer, white", "ful1e5", GOOGLE, "https://github.com/ful1e5/Google_Cursor/releases/download/v2.0.0/GoogleDot-White-Windows.zip", 883_522),
    pack!("google-dot-blue", "Google Dot Blue", "Google Dot", "A round dot pointer, blue", "ful1e5", GOOGLE, "https://github.com/ful1e5/Google_Cursor/releases/download/v2.0.0/GoogleDot-Blue-Windows.zip", 906_807),
    pack!("xcursor-pro-dark", "XCursor Pro Dark", "XCursor Pro", "Thin and sharp, dark", "ful1e5", XPRO, "https://github.com/ful1e5/XCursor-pro/releases/download/v2.0.2/XCursor-Pro-Dark-Windows.zip", 7_803_589),
    pack!("xcursor-pro-light", "XCursor Pro Light", "XCursor Pro", "Thin and sharp, light", "ful1e5", XPRO, "https://github.com/ful1e5/XCursor-pro/releases/download/v2.0.2/XCursor-Pro-Light-Windows.zip", 8_239_072),
    pack!("fuchsia", "Fuchsia", "Fuchsia", "Soft and flat, pink-purple", "ful1e5", FUCHSIA, "https://github.com/ful1e5/fuchsia-cursor/releases/download/v2.0.1/Fuchsia-Windows.zip", 9_102_015),
    pack!("fuchsia-pop", "Fuchsia Pop", "Fuchsia", "Soft and flat, bright", "ful1e5", FUCHSIA, "https://github.com/ful1e5/fuchsia-cursor/releases/download/v2.0.1/Fuchsia-Pop-Windows.zip", 9_376_935),
    pack!("banana", "Banana", "Banana", "A yellow banana pointer", "ful1e5", BANANA, "https://github.com/ful1e5/banana-cursor/releases/download/v2.0.0/Banana-Windows.zip", 11_536_355),
    pack!("rose-pine", "Rosé Pine (Breeze style)", "Rosé Pine", "Breeze-style pointer in Rosé Pine colours", "Rosé Pine", ROSE, "https://github.com/rose-pine/cursors/releases/download/v1.1.0/BreezeX-RosePine-Windows.zip", 2_012_040),
    pack!("rose-pine-dawn", "Rosé Pine Dawn (Breeze style)", "Rosé Pine", "Breeze-style pointer, light Dawn colours", "Rosé Pine", ROSE, "https://github.com/rose-pine/cursors/releases/download/v1.1.0/BreezeX-RosePineDawn-Windows.zip", 1_967_277),
    pack!("nordzy-dark", "Nordzy", "Nordzy", "Cool Nord colours, dark", "Nordzy cursors", NORDZY, "https://github.com/guillaumeboehm/Nordzy-cursors/releases/download/v2.4.0/Nordzy-cursors_windows.zip", 2_684_885),
    pack!("nordzy-white", "Nordzy White", "Nordzy", "Cool Nord colours, white", "Nordzy cursors", NORDZY, "https://github.com/guillaumeboehm/Nordzy-cursors/releases/download/v2.4.0/Nordzy-cursors-white_windows.zip", 2_685_095),
];

/// The one thing this module needs from the network: GET a link (all of it, or a byte range) into memory.
pub trait Fetch {
    /// `range` = the HTTP range without `bytes=` ("0-99", "-131072" for the last 128 KB); `None` = everything.
    fn get(&self, url: &str, range: Option<&str>, max: usize, progress: &mut dyn FnMut(u64, Option<u64>)) -> std::result::Result<Vec<u8>, String>;
}

/// Only release downloads on github.com are ever asked for.
fn only_github(url: &str) -> std::result::Result<(), String> {
    if url.starts_with("https://github.com/") && url.contains("/releases/download/") {
        Ok(())
    } else {
        Err("only a release download on github.com is fetched".into())
    }
}

/// The biggest pack .zip taken whole.
pub const MAX_ZIP: usize = 40 * 1024 * 1024;
/// The biggest single cursor file unpacked.
const MAX_FILE: usize = 8 * 1024 * 1024;
const TAIL: usize = 128 * 1024;

pub fn size_text(b: u64) -> String {
    if b >= 1_000_000 {
        format!("{:.1} MB", b as f64 / 1_000_000.0)
    } else {
        format!("{} KB", (b / 1000).max(1))
    }
}

fn is_cursor_name(n: &str) -> bool {
    let l = n.to_ascii_lowercase();
    l.ends_with(".cur") || l.ends_with(".ani")
}

// (a zip made on Windows may separate with a backslash: both count)
fn folder_of(name: &str) -> &str {
    name.rsplit_once(['/', '\\']).map(|(d, _)| d).unwrap_or("")
}

fn file_of(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

/// A file name that is one plain name: nothing that could put it outside the folder it is written into (no separator, drive
/// colon, "." / ".." or a name Windows would trim).
fn plain_file(n: &str) -> bool {
    !n.is_empty() && n != "." && n != ".." && !n.contains(['/', '\\', ':']) && !n.ends_with('.') && !n.ends_with(' ') && !n.chars().any(|c| c.is_control() || "<>\"|?*".contains(c))
}

/// The folder of the zip to install: the one that holds cursors, preferring a "Regular" / "Normal" size (the size slider scales
/// the rest). `None` = the zip holds no cursor.
pub fn pick_variant(items: &[Item]) -> Option<String> {
    let mut dirs: Vec<&str> = items.iter().filter(|i| is_cursor_name(&i.name)).map(|i| folder_of(&i.name)).collect();
    dirs.sort_unstable();
    dirs.dedup();
    for want in ["regular", "normal", "medium", "default"] {
        if let Some(d) = dirs.iter().find(|d| d.to_ascii_lowercase().contains(want)) {
            return Some((*d).to_string());
        }
    }
    // no size words: the first folder (a zip with a single folder, or cursors at the top)
    dirs.first().map(|d| (*d).to_string())
}

/// The entries directly in that folder.
fn in_folder<'a>(items: &'a [Item], dir: &str) -> Vec<&'a Item> {
    items.iter().filter(|i| !i.name.ends_with('/') && folder_of(&i.name) == dir && plain_file(file_of(&i.name))).collect()
}

/// Which file is which role, from the folder's `install.inf` (scheme line first, then its key names), then from file names.
fn roles_of(names: &[String], inf: Option<&str>) -> BTreeMap<WinRole, String> {
    let has = |f: &str| names.iter().find(|n| n.eq_ignore_ascii_case(f)).cloned();
    let mut out: BTreeMap<WinRole, String> = BTreeMap::new();
    // 1. the inf's own key names (pointer / help / work / cross / vert ...): what each file is called by the pack's maker
    if let Some(inf) = inf {
        for (r, f) in parse_install_inf(inf) {
            if let Some(n) = has(&f) {
                out.insert(r, n);
            }
        }
    }
    let cursors: Vec<String> = names.iter().filter(|n| is_cursor_name(n)).cloned().collect();
    // 2. the roles still without a file: guessed from the file names (a file is never used twice)
    let unused = |out: &BTreeMap<WinRole, String>| -> Vec<String> { cursors.iter().filter(|n| !out.values().any(|v| v == *n)).cloned().collect() };
    for (r, f) in guess_roles(&unused(&out)) {
        out.entry(r).or_insert(f);
    }
    // 3. last resort, for a pack whose files have none of the usual names: the order of the inf's scheme line (packs in the
    //    wild list it in other orders - Rosé Pine's is scrambled - so it is never trusted over the names)
    if out.len() < 6 {
        if let Some(paths) = inf.and_then(parse_scheme_reg) {
            for (r, f) in WinRole::ALL.iter().zip(paths) {
                if let Some(n) = has(&f).filter(|n| is_cursor_name(n) && !out.values().any(|v| v == n)) {
                    out.entry(*r).or_insert(n);
                }
            }
        }
    }
    out
}

/// The three pictures of a pack's list row: Normal, Link, Text.
pub const PREVIEW_ROLES: [Role; 3] = [Role::Normal, Role::Link, Role::Text];

/// The pack's own pictures for the list row, read from its release page with a few range requests (the end of the .zip, its
/// directory, then the three small files) - not the whole download. (role, the .cur's bytes).
pub fn fetch_preview(fetch: &dyn Fetch, l: &Listing) -> std::result::Result<Vec<(Role, Vec<u8>)>, String> {
    only_github(l.zip)?;
    // the last 128 KB of the .zip (its size is on the release page; a plain "from-to" range - GitHub's download servers
    // answer a "last N bytes" range with 501)
    let tail_start = l.bytes.saturating_sub(TAIL as u64);
    let tail = fetch.get(l.zip, Some(&format!("{tail_start}-{}", l.bytes.saturating_sub(1))), TAIL + 4096, &mut |_, _| {})?;
    let end = zipdir::find_end(&tail).ok_or("the pack's file list could not be read")?;
    if end.cd_size == 0 || end.cd_size > 4 * 1024 * 1024 {
        return Err("the pack's file list is empty or too big".into());
    }
    let cd = if end.cd_off >= tail_start && end.cd_off + end.cd_size <= tail_start + tail.len() as u64 {
        tail[(end.cd_off - tail_start) as usize..(end.cd_off - tail_start + end.cd_size) as usize].to_vec()
    } else {
        fetch.get(l.zip, Some(&format!("{}-{}", end.cd_off, end.cd_off + end.cd_size - 1)), end.cd_size as usize + 16, &mut |_, _| {})?
    };
    let items = zipdir::parse_directory(&cd, end.count).ok_or("the pack's file list is damaged")?;
    let dir = pick_variant(&items).ok_or("no cursors in that pack")?;
    let files = in_folder(&items, &dir);
    let names: Vec<String> = files.iter().map(|i| file_of(&i.name).to_string()).collect();
    // the folder's install.inf names the roles exactly; it is a few KB
    let inf = files.iter().find(|i| file_of(&i.name).eq_ignore_ascii_case("install.inf")).and_then(|i| fetch_entry(fetch, l.zip, i).ok()).map(|b| String::from_utf8_lossy(&b).into_owned());
    let roles = roles_of(&names, inf.as_deref());
    let mut out = Vec::new();
    for role in PREVIEW_ROLES {
        let Some(f) = role.win_roles().iter().find_map(|w| roles.get(w)) else { continue };
        let Some(item) = files.iter().find(|i| file_of(&i.name) == f) else { continue };
        if let Ok(b) = fetch_entry(fetch, l.zip, item) {
            if is_cursor_bytes(&b) {
                out.push((role, b));
            }
        }
    }
    if out.is_empty() {
        return Err("no picture could be read from that pack".into());
    }
    Ok(out)
}

fn fetch_entry(fetch: &dyn Fetch, url: &str, item: &Item) -> std::result::Result<Vec<u8>, String> {
    let (from, len) = zipdir::span(item);
    let part = fetch.get(url, Some(&format!("{from}-{}", from + len as u64 - 1)), len + 64, &mut |_, _| {})?;
    zipdir::unpack(item, &part, MAX_FILE).ok_or_else(|| "a file of the pack is damaged".to_string())
}

/// The whole .zip (for the install).
pub fn download(fetch: &dyn Fetch, l: &Listing, progress: &mut dyn FnMut(u64, Option<u64>)) -> std::result::Result<Vec<u8>, String> {
    only_github(l.zip)?;
    let b = fetch.get(l.zip, None, MAX_ZIP, progress)?;
    if b.len() < 22 || &b[..2] != b"PK" {
        return Err("that is not a cursor pack".into());
    }
    Ok(b)
}

/// `<data>\cursors\store\<id>\<role>.cur`: the list row's pictures kept after the first look, so the list opens without the net.
pub fn cache_dir(data: &Path, id: &str) -> PathBuf {
    data.join("cursors").join("store").join(id)
}

pub fn save_preview(dir: &Path, pics: &[(Role, Vec<u8>)]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (r, b) in pics {
        std::fs::write(dir.join(format!("{}.cur", r.name().to_ascii_lowercase())), b)?;
    }
    Ok(())
}

/// The kept pictures (empty = none kept yet).
pub fn load_preview(dir: &Path) -> Vec<(Role, Vec<u8>)> {
    PREVIEW_ROLES.iter().filter_map(|r| std::fs::read(dir.join(format!("{}.cur", r.name().to_ascii_lowercase()))).ok().filter(|b| is_cursor_bytes(b)).map(|b| (*r, b))).collect()
}

impl<O: MouseOs> Mouse<O> {
    /// Installs a downloaded pack: its Regular folder's cursors into the app's packs folder (only the files that have a role),
    /// registered as a cursor scheme for this Windows user, so it is in the pickers and in Windows' own Mouse settings. The pack
    /// is NOT applied. If a pack of this name is already there, it is returned as it is.
    pub fn install_store_zip(&mut self, name: &str, zip: &[u8]) -> Result<Pack> {
        if !cursors::is_plain_folder_name(name) {
            return Err(Error::BadName { name: name.into(), why: "not a plain folder name" });
        }
        if let Some(p) = self.packs()?.into_iter().find(|p| p.name == name) {
            // (the scheme may be gone: register it again)
            self.register_pack_scheme(&p)?;
            return Ok(p);
        }
        let bad = |why: &str| Error::NotACursor(format!("{name}: {why}"));
        let items = zipdir::read_all(zip).ok_or_else(|| bad("the file list is damaged"))?;
        let dir_in_zip = pick_variant(&items).ok_or_else(|| bad("no cursors in it"))?;
        let files = in_folder(&items, &dir_in_zip);
        let names: Vec<String> = files.iter().map(|i| file_of(&i.name).to_string()).collect();
        let inf = files
            .iter()
            .find(|i| file_of(&i.name).eq_ignore_ascii_case("install.inf"))
            .and_then(|i| zipdir::unpack_from(zip, i, 1 << 20))
            .map(|b| String::from_utf8_lossy(&b).into_owned());
        let roles = roles_of(&names, inf.as_deref());
        if roles.is_empty() {
            return Err(bad("no cursor roles found"));
        }
        let mut wanted: Vec<String> = roles.values().cloned().collect();
        wanted.sort();
        wanted.dedup();
        let mut data: Vec<(String, Vec<u8>)> = Vec::new();
        for f in &wanted {
            let item = files.iter().find(|i| file_of(&i.name) == f).ok_or_else(|| bad("a file is missing"))?;
            let b = zipdir::unpack_from(zip, item, MAX_FILE).ok_or_else(|| bad("a file is damaged"))?;
            if !is_cursor_bytes(&b) {
                return Err(bad("a file is not a cursor"));
            }
            data.push((f.clone(), b));
        }
        let packs_dir = self.dirs.packs();
        std::fs::create_dir_all(&packs_dir).map_err(|e| Error::io("create packs folder", e))?;
        let folder = packs_dir.join(name);
        // written in a temporary folder first: a pack is there whole or not at all
        let tmp = packs_dir.join(format!("{name}.new"));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).map_err(|e| Error::io("create pack folder", e))?;
        let r = (|| -> Result<Pack> {
            for (n, b) in &data {
                std::fs::write(tmp.join(n), b).map_err(|e| Error::io(format!("write {n}"), e))?;
            }
            let pack = Pack { name: name.to_string(), roles: roles.clone(), files: data.iter().map(|(n, _)| n.clone()).collect() };
            let json = serde_json::to_string_pretty(&pack).map_err(|e| Error::io("pack.json", e))?;
            std::fs::write(tmp.join("pack.json"), json).map_err(|e| Error::io("write pack.json", e))?;
            Ok(pack)
        })();
        let pack = match r {
            Ok(p) => p,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(e);
            }
        };
        std::fs::rename(&tmp, &folder).map_err(|e| {
            let _ = std::fs::remove_dir_all(&tmp);
            Error::io(format!("create {}", folder.display()), e)
        })?;
        self.register_pack_scheme(&pack)?;
        Ok(pack)
    }

    /// Adds the pack as a scheme of this Windows user (`HKCU\Control Panel\Cursors\Schemes\<name>`): 17 paths in Windows'
    /// role order; a role the pack has no cursor for gets Windows' own file.
    fn register_pack_scheme(&mut self, pack: &Pack) -> Result<()> {
        let defaults = self.windows_default_paths()?;
        let dir = self.dirs.packs().join(&pack.name);
        // a scheme of this name that is not ours (the user's own, or Windows' / another tool's) is never overwritten: the pack is
        // still there to pick in the app's lists, it just has no scheme of its own in Windows' Mouse settings
        for (hive, system) in [(Hive::Hkcu, false), (Hive::Hklm, true)] {
            let key = if system { cursors::SYSTEM_SCHEMES_KEY } else { USER_SCHEMES_KEY };
            if let Some((n, v)) = self.os.reg_values(hive, key)?.into_iter().find(|(n, _)| n.eq_ignore_ascii_case(&pack.name)) {
                let ours = !system
                    && v.as_str().is_some_and(|s| {
                        let sc = cursors::Scheme { name: n.clone(), system: false, paths: cursors::parse_scheme(s) };
                        self.scheme_is_ours(&sc, &dir, &defaults)
                    });
                if !ours {
                    return Ok(());
                }
            }
        }
        let paths: Vec<String> = WinRole::ALL
            .iter()
            .map(|r| match pack.roles.get(r) {
                Some(f) => dir.join(f).to_string_lossy().into_owned(),
                None => defaults.get(r).cloned().unwrap_or_default(),
            })
            .collect();
        self.os.reg_write(USER_SCHEMES_KEY, &pack.name, &RegValue::ExpandSz(paths.join(",")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_well_formed() {
        let mut ids = std::collections::BTreeSet::new();
        for l in LIST.iter() {
            assert!(ids.insert(l.id), "{} twice", l.id);
            assert!(l.zip.starts_with("https://github.com/") && l.zip.contains("/releases/download/") && l.zip.ends_with(".zip"), "{}", l.id);
            assert!(l.zip.starts_with(l.page), "{}: the zip is on the repository's release page", l.id);
            assert_eq!(l.licence, "GPL-3.0");
            assert!(l.bytes > 100_000 && l.bytes < MAX_ZIP as u64, "{}", l.id);
            assert!(cursors::is_plain_folder_name(l.name), "{}", l.id);
        }
    }

    #[test]
    fn only_release_downloads_are_fetched() {
        assert!(only_github("https://github.com/a/b/releases/download/v1/x.zip").is_ok());
        assert!(only_github("https://example.com/releases/download/x.zip").is_err());
        assert!(only_github("https://github.com/a/b").is_err());
        assert!(only_github("http://github.com/a/b/releases/download/v1/x.zip").is_err());
    }

    #[test]
    fn the_regular_folder_is_the_one_installed() {
        let item = |n: &str| Item { name: n.into(), method: 0, csize: 1, size: 1, crc: 0, local: 0 };
        let items = vec![item("P-Small/a.cur"), item("P-Regular/a.cur"), item("P-Large/a.cur"), item("P-Regular/install.inf")];
        assert_eq!(pick_variant(&items).as_deref(), Some("P-Regular"));
        assert_eq!(pick_variant(&[item("only/a.cur"), item("only/b.ani")]).as_deref(), Some("only"));
        assert_eq!(pick_variant(&[item("a.cur")]).as_deref(), Some(""));
        assert_eq!(pick_variant(&[item("x/readme.txt")]), None);
    }

    const INF: &str = "[Scheme.Reg]\nHKCU,\"Control Panel\\Cursors\\Schemes\",\"%SCHEME_NAME%\",,\"%10%\\%CUR_DIR%\\%pointer%,%10%\\%CUR_DIR%\\%help%,%10%\\%CUR_DIR%\\%work%,%10%\\%CUR_DIR%\\%busy%,%10%\\%CUR_DIR%\\%cross%,%10%\\%CUR_DIR%\\%text%,%10%\\%CUR_DIR%\\%hand%,%10%\\%CUR_DIR%\\%unavailiable%,%10%\\%CUR_DIR%\\%vert%\"\n\n[Strings]\nCUR_DIR = \"Cursors\\X\"\nSCHEME_NAME = \"X\"\npointer = \"Default.cur\"\nhelp = \"Help.cur\"\nwork = \"Work.ani\"\nbusy = \"Busy.ani\"\ncross = \"Cross.cur\"\ntext = \"IBeam.cur\"\nhand = \"Handwriting.cur\"\nunavailiable = \"Unavailiable.cur\"\nvert = \"Vertical.cur\"\n";

    #[test]
    fn the_inf_scheme_line_gives_the_roles_in_windows_order() {
        let p = parse_scheme_reg(INF).unwrap();
        assert_eq!(p.len(), 17);
        assert_eq!(&p[..4], &["Default.cur", "Help.cur", "Work.ani", "Busy.ani"]);
        assert_eq!(p[6], "Handwriting.cur");
        assert_eq!(p[8], "Vertical.cur");
        assert_eq!(p[9], "");
        let names: Vec<String> = ["Default.cur", "Help.cur", "Work.ani", "Busy.ani", "Cross.cur", "IBeam.cur", "Handwriting.cur", "Unavailiable.cur", "Vertical.cur", "Link.cur", "Move.cur"].iter().map(|s| s.to_string()).collect();
        let r = roles_of(&names, Some(INF));
        assert_eq!(r[&WinRole::Arrow], "Default.cur");
        assert_eq!(r[&WinRole::AppStarting], "Work.ani");
        assert_eq!(r[&WinRole::Wait], "Busy.ani");
        assert_eq!(r[&WinRole::NWPen], "Handwriting.cur");
        assert_eq!(r[&WinRole::No], "Unavailiable.cur");
        // not in the inf's scheme line: found by name
        assert_eq!(r[&WinRole::Hand], "Link.cur");
        assert_eq!(r[&WinRole::SizeAll], "Move.cur");
    }

    #[test]
    fn size_words() {
        assert_eq!(size_text(897_098), "897 KB");
        assert_eq!(size_text(2_813_335), "2.8 MB");
        assert_eq!(size_text(11_131_753), "11.1 MB");
        assert_eq!(size_text(80_000), "80 KB");
    }

    /// A fake release page: serves ranges of one in-memory zip.
    struct Fake(Vec<u8>);
    impl Fetch for Fake {
        fn get(&self, _url: &str, range: Option<&str>, _max: usize, _p: &mut dyn FnMut(u64, Option<u64>)) -> std::result::Result<Vec<u8>, String> {
            let Some(r) = range else { return Ok(self.0.clone()) };
            let n = self.0.len();
            if let Some(last) = r.strip_prefix('-') {
                let k: usize = last.parse().map_err(|_| "range")?;
                return Ok(self.0[n.saturating_sub(k)..].to_vec());
            }
            let (a, b) = r.split_once('-').ok_or("range")?;
            let (a, b): (usize, usize) = (a.parse().map_err(|_| "range")?, b.parse().map_err(|_| "range")?);
            Ok(self.0[a.min(n)..(b + 1).min(n)].to_vec())
        }
    }

    fn cur() -> Vec<u8> {
        // a 1-entry .cur (header only is enough for is_cursor_bytes)
        let mut c = vec![0, 0, 2, 0, 1, 0, 16, 16, 0, 0, 1, 0, 1, 0, 4, 0, 0, 0, 22, 0, 0, 0];
        c.extend_from_slice(&[1, 2, 3, 4]);
        c
    }

    fn pack_zip(deflate: bool) -> Vec<u8> {
        let c = cur();
        zipdir::build(
            &[
                ("P-Small/Default.cur", &c),
                ("P-Regular/Default.cur", &c),
                ("P-Regular/Link.cur", &c),
                ("P-Regular/IBeam.cur", &c),
                ("P-Regular/Work.ani", b"RIFF\0\0\0\0ACON"),
                ("P-Regular/install.inf", INF.as_bytes()),
                ("P-Regular/readme.txt", b"hi"),
            ],
            deflate,
        )
    }

    #[test]
    fn the_preview_reads_only_ranges_of_the_pack() {
        for deflate in [false, true] {
            let f = Fake(pack_zip(deflate));
            let l = Listing { zip: "https://github.com/o/r/releases/download/v1/p.zip", bytes: f.0.len() as u64, ..LIST[0] };
            let pics = fetch_preview(&f, &l).unwrap();
            let roles: Vec<Role> = pics.iter().map(|(r, _)| *r).collect();
            assert_eq!(roles, vec![Role::Normal, Role::Link, Role::Text]);
            assert!(pics.iter().all(|(_, b)| is_cursor_bytes(b)));
        }
    }

    #[test]
    fn the_preview_refuses_other_hosts() {
        let l = Listing { zip: "https://evil.example/releases/download/x.zip", ..LIST[0] };
        assert!(fetch_preview(&Fake(vec![]), &l).is_err());
        assert!(download(&Fake(vec![]), &l, &mut |_, _| {}).is_err());
    }

    #[test]
    fn kept_pictures_come_back() {
        let d = std::env::temp_dir().join(format!("bu-store-test-{}", std::process::id()));
        let pics = vec![(Role::Normal, cur()), (Role::Text, cur())];
        save_preview(&d, &pics).unwrap();
        assert_eq!(load_preview(&d), pics);
        let _ = std::fs::remove_dir_all(&d);
    }
}

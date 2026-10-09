//! "Get more sounds" (Order 061): the community packs of the official Mechvibes site, https://mechvibes.com/sound-packs/ -
//! the same page and the same .zip files the Mechvibes app and its "Install Pack" button use. This module only READS that
//! site's HTML (no network here: the app passes the page text in) and says where each .zip lives. We host and copy nothing;
//! the packs are made by the community and their licences vary (the site has no licence field).

pub const SITE: &str = "https://mechvibes.com";
/// The list page.
pub const LIST_URL: &str = "https://mechvibes.com/sound-packs/";

/// One pack of the list page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    /// the site's id, e.g. `custom-sound-pack-1203000000016` (also the page `/sound-packs/<id>/`)
    pub id: String,
    pub name: String,
    /// "keyboard", "retro", "games" ... (the site's tags, without its `config-v1` format tag)
    pub tags: Vec<String>,
    /// `pre-installed` = ships inside the Mechvibes app itself
    pub pre_installed: bool,
}

impl Listed {
    /// The pack's own page (its download link is on it: [`zip_path`]).
    pub fn page_url(&self) -> String {
        format!("{SITE}/sound-packs/{}/", self.id)
    }
}

fn decode(s: &str) -> String {
    s.replace("&amp;", "&").replace("&#39;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").trim().to_string()
}

/// The text of the first `<div ...>text</div>` that starts at or after `from` (a div without child tags), and where it ends.
fn next_div_text(html: &str, from: usize) -> Option<(String, usize)> {
    let mut at = from;
    loop {
        let open = html.get(at..)?.find("<div")? + at;
        let gt = html[open..].find('>')? + open;
        let close = html[gt..].find("</div>")? + gt;
        let inner = &html[gt + 1..close];
        if !inner.contains('<') {
            return Some((decode(inner), close + 6));
        }
        at = gt + 1;
    }
}

const CARD: &str = "class=\"sound-pack \"";

/// Every pack of the list page, in the site's order. Anything that isn't a well-formed card is skipped.
pub fn parse_list(html: &str) -> Vec<Listed> {
    let mut out = Vec::new();
    let mut rest = 0usize;
    while let Some(i) = html[rest..].find(CARD) {
        let start = rest + i;
        let end = html[start + CARD.len()..].find(CARD).map(|j| start + CARD.len() + j).unwrap_or(html.len());
        rest = end;
        let card = &html[start..end];
        let Some(p) = card.find("pack=\"") else { continue };
        let id_start = p + 6;
        let Some(id_len) = card[id_start..].find('"') else { continue };
        let id = &card[id_start..id_start + id_len];
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            continue;
        }
        let Some(d) = card.find("pack-details") else { continue };
        let Some((name, after)) = next_div_text(card, d) else { continue };
        if name.is_empty() {
            continue;
        }
        let tags_end = card.find("safari-warning").unwrap_or(card.len()).max(after);
        let mut tags = Vec::new();
        // the tags are the child divs of `sp-tags`
        if let Some(t) = card[after..tags_end].find("sp-tags") {
            let mut at = after + t;
            while let Some((tag, next)) = next_div_text(&card[..tags_end], at) {
                at = next;
                if !tag.is_empty() {
                    tags.push(tag);
                }
            }
        }
        let pre_installed = tags.iter().any(|t| t == "pre-installed");
        tags.retain(|t| t != "pre-installed" && !t.starts_with("config-v"));
        out.push(Listed { id: id.to_string(), name, tags, pre_installed });
    }
    out
}

/// The .zip's path on the site (`/sound-packs/<id>/dist/<file>.zip`) from the pack's own page, if the page has a download.
pub fn zip_path(page_html: &str, id: &str) -> Option<String> {
    let prefix = format!("/sound-packs/{id}/dist/");
    let at = page_html.find(&prefix)?;
    let tail = &page_html[at..];
    let end = tail.find(['"', '\'', ' ', '<', '>'])?;
    let p = &tail[..end];
    (p.to_ascii_lowercase().ends_with(".zip") && !p.contains("..")).then(|| p.to_string())
}

/// The full download URL for a path from [`zip_path`] (spaces and other odd characters escaped).
pub fn zip_url(path: &str) -> String {
    let mut out = String::from(SITE);
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'+' | b'~' | b'%' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}


// ---------------------------------------------------------------------------------------------------------------- fetching

/// The one thing the gallery needs from the network (the app gives it WinHTTP; tests give it a fake). Only mechvibes.com URLs
/// are ever asked for.
pub trait Fetch {
    /// GET `url` into memory (at most `max` bytes), calling `progress(done, total)`.
    fn get(&self, url: &str, max: usize, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<Vec<u8>, String>;
    /// The size the server announces for `url` (the body is not downloaded), `None` when it doesn't say.
    fn length(&self, url: &str) -> Result<Option<u64>, String>;
}

/// The biggest list page / pack page read.
pub const MAX_PAGE: usize = 4 * 1024 * 1024;
/// The biggest pack .zip downloaded.
pub const MAX_ZIP: usize = 60 * 1024 * 1024;

fn only_site(url: &str) -> Result<(), String> {
    if url.starts_with("https://mechvibes.com/") {
        Ok(())
    } else {
        Err(format!("refusing to fetch {url}"))
    }
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The site's list of packs.
pub fn fetch_list(f: &dyn Fetch) -> Result<Vec<Listed>, String> {
    only_site(LIST_URL)?;
    let html = text(f.get(LIST_URL, MAX_PAGE, &mut |_, _| {}).map_err(|e| format!("the Mechvibes site didn't answer: {e}"))?);
    let l = parse_list(&html);
    if l.is_empty() {
        return Err("the Mechvibes site's list looks different than expected (no packs found)".into());
    }
    Ok(l)
}

/// The .zip URL of one pack (read from the pack's own page).
pub fn find_zip(f: &dyn Fetch, p: &Listed) -> Result<String, String> {
    let page_url = p.page_url();
    only_site(&page_url)?;
    let page = text(f.get(&page_url, MAX_PAGE, &mut |_, _| {}).map_err(|e| format!("the pack's page didn't answer: {e}"))?);
    let path = zip_path(&page, &p.id).ok_or("that pack's page has no download")?;
    let url = zip_url(&path);
    only_site(&url)?;
    Ok(url)
}

/// How big a pack's download is (two small requests: its page, then the .zip's announced length).
pub fn size_of(f: &dyn Fetch, p: &Listed) -> Result<Option<u64>, String> {
    let url = find_zip(f, p)?;
    f.length(&url)
}

/// Downloads one pack's .zip.
pub fn download(f: &dyn Fetch, p: &Listed, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<Vec<u8>, String> {
    let url = find_zip(f, p)?;
    f.get(&url, MAX_ZIP, progress).map_err(|e| format!("the download failed: {e}"))
}

// ---------------------------------------------------------------------------------------------------------------- cache

/// What is remembered between two openings of the list: the last list (for when the site can't be reached), the sizes
/// (found once per pack, ever) and which pack folder each downloaded pack became.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cache {
    pub list: Vec<Listed>,
    pub sizes: std::collections::BTreeMap<String, u64>,
    pub installed: std::collections::BTreeMap<String, String>,
}

impl Cache {
    pub fn load(path: &std::path::Path) -> Cache {
        let Ok(text) = std::fs::read_to_string(path) else { return Cache::default() };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return Cache::default() };
        let s = |x: &serde_json::Value| x.as_str().map(str::to_string);
        let list = v["list"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|e| {
                        Some(Listed {
                            id: s(&e["id"])?,
                            name: s(&e["name"])?,
                            tags: e["tags"].as_array().map(|t| t.iter().filter_map(s).collect()).unwrap_or_default(),
                            pre_installed: e["pre"].as_bool().unwrap_or(false),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let sizes = v["sizes"].as_object().map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_u64()?))).collect()).unwrap_or_default();
        let installed = v["installed"].as_object().map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()).unwrap_or_default();
        Cache { list, sizes, installed }
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), String> {
        let v = serde_json::json!({
            "list": self.list.iter().map(|p| serde_json::json!({"id": p.id, "name": p.name, "tags": p.tags, "pre": p.pre_installed})).collect::<Vec<_>>(),
            "sizes": self.sizes,
            "installed": self.installed,
        });
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, v.to_string()).map_err(|e| e.to_string())
    }
}

/// "860 KB" / "1.2 MB".
pub fn size_text(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1048576.0)
    } else {
        format!("{} KB", bytes.div_ceil(1024).max(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"
<div class="sound-pack " pack="sound-pack-1200000000001" type="single">
 <div class="pack-details">
  <div>CherryMX Black - ABS keycaps</div>
  <div class="sp-tags" style="font-size:0.75em;">
   <div >pre-installed</div>
   <div>config-v1</div>
  </div>
  <div class="safari-warning" style="x">Safari is unable to test this sound pack.</div>
 </div>
 <div class="buttons"><a href="/sound-packs/sound-pack-1200000000001" class="button">Learn More</a></div>
</div>
<div class="sound-pack " pack="custom-sound-pack-1203000000016" type="single">
 <div class="pack-details">
  <div>Model_F_XT &amp; co</div>
  <div class="sp-tags" style="font-size:0.75em;">
   <div >keyboard</div>
   <div >retro</div>
   <div>config-v1</div>
  </div>
  <div class="safari-warning">Safari</div>
 </div>
</div>
<div class="sound-pack " pack="bad id!" type="single"><div class="pack-details"><div>Nope</div></div></div>
<div class="sound-pack " pack="x-no-tags" type="multi"><div class="pack-details"><div>Plain</div><div class="safari-warning">s</div></div></div>
"#;

    #[test]
    fn the_list_page_is_read() {
        let l = parse_list(LIST);
        assert_eq!(l.len(), 3, "the card with a bad id is skipped: {l:?}");
        assert_eq!(l[0], Listed { id: "sound-pack-1200000000001".into(), name: "CherryMX Black - ABS keycaps".into(), tags: vec![], pre_installed: true });
        assert_eq!(l[1].name, "Model_F_XT & co");
        assert_eq!(l[1].tags, vec!["keyboard", "retro"]);
        assert!(!l[1].pre_installed);
        assert_eq!(l[2].name, "Plain");
        assert!(l[2].tags.is_empty());
        assert_eq!(l[1].page_url(), "https://mechvibes.com/sound-packs/custom-sound-pack-1203000000016/");
    }

    #[test]
    fn an_empty_or_foreign_page_gives_no_packs() {
        assert!(parse_list("").is_empty());
        assert!(parse_list("<html><body>Just a moment...</body></html>").is_empty());
    }

    #[test]
    fn the_zip_link_comes_from_the_pack_page() {
        let page = r#"<a href="mechvibes://install custom-sound-pack-1203000000016"> <a href="/sound-packs/custom-sound-pack-1203000000016/dist/Model+F+XT.zip" download>Download</a>"#;
        let p = zip_path(page, "custom-sound-pack-1203000000016").unwrap();
        assert_eq!(p, "/sound-packs/custom-sound-pack-1203000000016/dist/Model+F+XT.zip");
        assert_eq!(zip_url(&p), "https://mechvibes.com/sound-packs/custom-sound-pack-1203000000016/dist/Model+F+XT.zip");
        assert_eq!(zip_url("/sound-packs/a/dist/My Pack (1).zip"), "https://mechvibes.com/sound-packs/a/dist/My%20Pack%20%281%29.zip");
        assert_eq!(zip_path("<a href=\"/sound-packs/other/dist/x.zip\">", "mine"), None);
        assert_eq!(zip_path("<a href=\"/sound-packs/mine/dist/../../x.zip\">", "mine"), None);
        assert_eq!(zip_path("<a href=\"/sound-packs/mine/dist/x.exe\">", "mine"), None);
    }
}

#[cfg(test)]
mod real_page {
    /// `BU_GALLERY_HTML=<saved list page> cargo test -p bu-keysound real_list -- --ignored --nocapture` (a saved copy of the site's page).
    #[test]
    #[ignore]
    fn real_list() {
        let path = std::env::var("BU_GALLERY_HTML").expect("BU_GALLERY_HTML");
        let html = std::fs::read_to_string(path).unwrap();
        let l = super::parse_list(&html);
        println!("{} packs; first: {:?}; last: {:?}", l.len(), l.first(), l.last());
        assert!(l.len() > 50);
        assert!(l.iter().all(|p| !p.name.is_empty()));
    }
}

#[cfg(test)]
mod flow_tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    struct Fake {
        pages: HashMap<String, Vec<u8>>,
        lengths: HashMap<String, u64>,
        asked: RefCell<Vec<String>>,
    }
    impl Fetch for Fake {
        fn get(&self, url: &str, max: usize, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<Vec<u8>, String> {
            self.asked.borrow_mut().push(url.to_string());
            let b = self.pages.get(url).ok_or_else(|| format!("404 {url}"))?;
            if b.len() > max {
                return Err("too big".into());
            }
            progress(b.len() as u64, Some(b.len() as u64));
            Ok(b.clone())
        }
        fn length(&self, url: &str) -> Result<Option<u64>, String> {
            self.asked.borrow_mut().push(format!("HEAD {url}"));
            Ok(self.lengths.get(url).copied())
        }
    }

    fn fake() -> Fake {
        let mut pages = HashMap::new();
        pages.insert(LIST_URL.to_string(), br#"<div class="sound-pack " pack="custom-sound-pack-7" type="single"><div class="pack-details"><div>Thock</div><div class="sp-tags"><div>keyboard</div></div><div class="safari-warning">x</div></div></div>"#.to_vec());
        pages.insert("https://mechvibes.com/sound-packs/custom-sound-pack-7/".to_string(), br#"<a href="/sound-packs/custom-sound-pack-7/dist/Thock+One.zip">Download</a>"#.to_vec());
        pages.insert("https://mechvibes.com/sound-packs/custom-sound-pack-7/dist/Thock+One.zip".to_string(), vec![1, 2, 3, 4]);
        let mut lengths = HashMap::new();
        lengths.insert("https://mechvibes.com/sound-packs/custom-sound-pack-7/dist/Thock+One.zip".to_string(), 860_000);
        Fake { pages, lengths, asked: RefCell::new(Vec::new()) }
    }

    #[test]
    fn list_size_and_download_follow_the_pack_page() {
        let f = fake();
        let l = fetch_list(&f).unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(size_of(&f, &l[0]).unwrap(), Some(860_000));
        let mut seen = 0;
        let z = download(&f, &l[0], &mut |d, _| seen = d).unwrap();
        assert_eq!(z, vec![1, 2, 3, 4]);
        assert_eq!(seen, 4);
        assert!(f.asked.borrow().iter().all(|u| u.contains("https://mechvibes.com/")), "only the site is asked");
    }

    #[test]
    fn a_broken_site_gives_a_plain_error() {
        let mut f = fake();
        f.pages.insert(LIST_URL.to_string(), b"<html>Just a moment...</html>".to_vec());
        assert!(fetch_list(&f).unwrap_err().contains("looks different"));
        f.pages.remove(LIST_URL);
        assert!(fetch_list(&f).unwrap_err().contains("didn't answer"));
    }

    #[test]
    fn a_pack_without_a_download_link_is_refused() {
        let mut f = fake();
        f.pages.insert("https://mechvibes.com/sound-packs/custom-sound-pack-7/".to_string(), b"<p>nothing</p>".to_vec());
        let p = Listed { id: "custom-sound-pack-7".into(), name: "Thock".into(), tags: vec![], pre_installed: false };
        assert_eq!(find_zip(&f, &p).unwrap_err(), "that pack's page has no download");
    }

    #[test]
    fn the_cache_round_trips_and_a_bad_file_is_empty() {
        let dir = std::env::temp_dir().join(format!("bu-gallery-{}", std::process::id()));
        let path = dir.join("gallery.json");
        let mut c = Cache::default();
        c.list.push(Listed { id: "a".into(), name: "A".into(), tags: vec!["x".into()], pre_installed: true });
        c.sizes.insert("a".into(), 5);
        c.installed.insert("a".into(), "A 2".into());
        c.save(&path).unwrap();
        assert_eq!(Cache::load(&path), c);
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(Cache::load(&path), Cache::default());
        assert_eq!(Cache::load(&dir.join("missing.json")), Cache::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sizes_read_plainly() {
        assert_eq!(size_text(0), "1 KB");
        assert_eq!(size_text(858_858), "839 KB");
        assert_eq!(size_text(3 * 1024 * 1024 + 300_000), "3.3 MB");
    }
}

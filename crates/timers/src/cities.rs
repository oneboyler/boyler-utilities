//! The World clock's list of places: about 33 000 cities and towns of 15 000 people or more, offline (`data/cities.txt`,
//! built by `examples/gen_cities.rs` from GeoNames - CC BY 4.0, geonames.org - and Unicode CLDR's Windows zone map).
//! Each place carries the Windows time zone key its time is read with ([`ZoneOs`](crate::zones::ZoneOs): Windows' own
//! rules, so summer time is Windows'). The text sits in the program file and is only read while a search runs: no list
//! is built at start-up, a search walks the text and returns at most `limit` places.

use std::sync::OnceLock;

use crate::zones::Place;

const DATA: &str = include_str!("../data/cities.txt");

/// The two small tables at the top of the data (127 Windows zones, 244 countries) and where the places start.
struct Head {
    zones: Vec<&'static str>,
    /// (country code, name), sorted by code
    lands: Vec<(&'static str, &'static str)>,
    /// the places text: one `name|cc|zone` / `name|ascii|cc|zone` per line, most populous first
    places: &'static str,
}

fn head() -> &'static Head {
    static H: OnceLock<Head> = OnceLock::new();
    H.get_or_init(|| {
        let (mut zones, mut lands) = (Vec::new(), Vec::new());
        let mut part = ' ';
        let mut places = "";
        let mut at = 0;
        for line in DATA.split_inclusive('\n') {
            let l = line.trim_end_matches(['\n', '\r']);
            at += line.len();
            match l {
                "#W" => part = 'W',
                "#C" => part = 'C',
                "#P" => {
                    places = &DATA[at..];
                    break;
                }
                _ if part == 'W' => zones.push(l),
                _ if part == 'C' => {
                    if let Some((cc, name)) = l.split_once(' ') {
                        lands.push((cc, name));
                    }
                }
                _ => {}
            }
        }
        lands.sort_by_key(|x| x.0);
        Head { zones, lands, places }
    })
}

/// One data line: the place, its ascii name when it has one, and its country's row in `Head::lands` (`None` = a damaged line).
fn parse(h: &Head, line: &'static str) -> Option<(Place, Option<&'static str>, usize)> {
    let mut f = line.split('|');
    let city = f.next()?;
    let (second, third) = (f.next()?, f.next()?);
    let (ascii, cc, z) = match f.next() {
        Some(z) => (Some(second), third, z),
        None => (None, second, third),
    };
    let zone = h.zones.get(z.parse::<usize>().ok()?)?;
    let li = h.lands.binary_search_by_key(&cc, |x| x.0).ok()?;
    Some((Place { city, land: h.lands[li].1, zone }, ascii, li))
}

/// How many places the list holds.
pub fn count() -> usize {
    head().places.lines().count()
}

/// The place called `city` in the country called `land` (the first one in the list = the biggest).
pub fn find(city: &str, land: &str) -> Option<Place> {
    let h = head();
    h.places.lines().filter_map(|l| parse(h, l)).map(|p| p.0).find(|p| p.city == city && p.land == land)
}

/// Lower case with accents and the like taken off (`São Paulo` -> `sao paulo`), appended to `out`.
fn fold_into(s: &str, out: &mut String) {
    // (most names are plain ascii: just lower case)
    if s.is_ascii() {
        out.extend(s.bytes().map(|b| b.to_ascii_lowercase() as char));
        return;
    }
    for c in s.chars() {
        for c in c.to_lowercase() {
            out.push_str(match c {
                'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' | 'ǎ' | 'ạ' | 'ả' | 'ấ' | 'ầ' | 'ẩ' | 'ẫ' | 'ậ' | 'ắ' | 'ằ' | 'ẳ' | 'ẵ' | 'ặ' => "a",
                'æ' | 'ǣ' => "ae",
                'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
                'ď' | 'đ' | 'ð' | 'ḍ' => "d",
                'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' | 'ẹ' | 'ẻ' | 'ẽ' | 'ế' | 'ề' | 'ể' | 'ễ' | 'ệ' => "e",
                'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
                'ĥ' | 'ħ' | 'ḥ' => "h",
                'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' | 'ị' | 'ỉ' => "i",
                'ĵ' => "j",
                'ķ' => "k",
                'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => "l",
                'ñ' | 'ń' | 'ņ' | 'ň' | 'ŋ' => "n",
                'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' | 'ơ' | 'ọ' | 'ỏ' | 'ố' | 'ồ' | 'ổ' | 'ỗ' | 'ộ' | 'ớ' | 'ờ' | 'ở' | 'ỡ' | 'ợ' => "o",
                'œ' => "oe",
                'ŕ' | 'ŗ' | 'ř' => "r",
                'ś' | 'ŝ' | 'ş' | 'š' | 'ș' | 'ṣ' => "s",
                'ß' => "ss",
                'ţ' | 'ť' | 'ŧ' | 'ț' | 'ṭ' => "t",
                'þ' => "th",
                'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' | 'ư' | 'ụ' | 'ủ' | 'ứ' | 'ừ' | 'ử' | 'ữ' | 'ự' => "u",
                'ŵ' => "w",
                'ý' | 'ÿ' | 'ŷ' | 'ỳ' | 'ỵ' | 'ỷ' | 'ỹ' => "y",
                'ź' | 'ż' | 'ž' => "z",
                _ => {
                    out.push(c);
                    continue;
                }
            });
        }
    }
}

fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty())
}

/// The places that fit what was typed (at least 2 letters), best first, at most `limit`. Three groups, the bigger place
/// first inside each: the name starts with what was typed; every typed word starts a word of the name; every typed word
/// starts a word of the name or of the country (`japan`, `paris fr`). Accents and capitals do not matter (`sao`, `SAO`,
/// `são`).
pub fn search(query: &str, limit: usize) -> Vec<Place> {
    let mut folded = String::new();
    fold_into(query, &mut folded);
    let q = words(&folded).collect::<Vec<_>>().join(" ");
    if q.chars().count() < 2 || limit == 0 {
        return Vec::new();
    }
    let tokens: Vec<&str> = q.split(' ').collect();
    let h = head();
    // the countries once, folded (a few hundred short strings)
    let lands: Vec<String> = h
        .lands
        .iter()
        .map(|l| {
            let mut s = String::new();
            fold_into(l.1, &mut s);
            s
        })
        .collect();
    let mut tiers: [Vec<Place>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let (mut hn, mut ha, mut flat) = (String::new(), String::new(), String::new());
    for line in h.places.lines() {
        let Some((p, ascii, li)) = parse(h, line) else { continue };
        hn.clear();
        fold_into(p.city, &mut hn);
        ha.clear();
        if let Some(a) = ascii {
            fold_into(a, &mut ha);
        }
        let hays = [hn.as_str(), ha.as_str()];
        // 1: the name (words joined by one space) starts with the typed words
        let first = hays.iter().any(|s| {
            flat.clear();
            for (i, w) in words(s).enumerate() {
                if i > 0 {
                    flat.push(' ');
                }
                flat.push_str(w);
            }
            !flat.is_empty() && flat.starts_with(&q)
        });
        let tier = if first {
            Some(0)
        } else if tokens.iter().all(|t| hays.iter().any(|s| words(s).any(|w| w.starts_with(t)))) {
            Some(1)
        } else {
            let land = lands[li].as_str();
            tokens.iter().all(|t| hays.iter().any(|s| words(s).any(|w| w.starts_with(t))) || words(land).any(|w| w.starts_with(t))).then_some(2)
        };
        if let Some(t) = tier {
            if tiers[t].len() < limit {
                tiers[t].push(p);
            }
        }
        // the best group full: nothing later can beat it
        if tiers[0].len() >= limit {
            break;
        }
    }
    tiers.into_iter().flatten().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(q: &str) -> Vec<String> {
        search(q, 8).iter().map(|p| format!("{} · {}", p.city, p.land)).collect()
    }

    #[test]
    fn the_list_is_big_and_every_line_reads() {
        let h = head();
        assert!(count() > 30_000, "{}", count());
        assert_eq!(h.places.lines().filter(|l| parse(h, l).is_none()).count(), 0);
        assert!(h.places.lines().filter_map(|l| parse(h, l)).all(|p| !p.0.land.is_empty()), "every place has its country");
        assert!(h.zones.len() > 100 && h.lands.len() > 200);
        // the bigger place comes first, as the data is sorted by people
        assert_eq!(find("Tokyo", "Japan").map(|p| p.zone), Some("Tokyo Standard Time"));
        assert_eq!(find("New York", "USA").map(|p| p.zone), Some("Eastern Standard Time"));
        assert_eq!(find("London", "UK").map(|p| p.zone), Some("GMT Standard Time"));
        assert_eq!(find("Nowhere", "UK"), None);
    }

    #[test]
    fn short_or_empty_searches_find_nothing() {
        assert!(search("", 8).is_empty());
        assert!(search("z", 8).is_empty());
        assert!(search("  a ", 8).is_empty());
        assert!(search("zagreb", 0).is_empty());
    }

    #[test]
    fn a_name_is_found_by_its_start_without_accents_or_capitals() {
        assert_eq!(names("zag")[0], "Zagreb · Croatia");
        assert_eq!(names("ZAGREB")[0], "Zagreb · Croatia");
        assert_eq!(names("sao pau")[0], "São Paulo · Brazil");
        assert_eq!(names("são paulo")[0], "São Paulo · Brazil");
        assert_eq!(names("new yo")[0], "New York · USA");
        assert_eq!(names("tok")[0], "Tokyo · Japan");
        assert_eq!(names("dubai")[0], "Dubai · UAE");
        assert_eq!(names("reykjav")[0], "Reykjavík · Iceland");
    }

    #[test]
    fn a_word_inside_a_name_and_a_country_work_too() {
        // "york" is a later word of "New York" (group 2); the bigger place first
        assert!(names("york").contains(&"New York · USA".to_string()));
        // a country lists its biggest places
        assert_eq!(names("japan")[0], "Tokyo · Japan");
        assert!(names("japan").iter().all(|n| n.ends_with("Japan")));
        // a name and the start of a country
        let p = names("paris fr");
        assert_eq!(p[0], "Paris · France", "{p:?}");
        assert!(search("qqqqzzzz", 8).is_empty());
    }

    #[test]
    fn at_most_limit_and_the_start_of_a_name_beats_a_word_inside_one() {
        assert_eq!(search("san", 5).len(), 5);
        let r = names("york");
        // a place that STARTS with york comes before "New York"
        let york = r.iter().position(|n| n.starts_with("York"));
        let ny = r.iter().position(|n| n.starts_with("New York"));
        assert!(york.is_some() && ny.is_some() && york < ny, "{r:?}");
    }

    #[test]
    fn every_zone_the_list_names_is_a_windows_key_the_pc_knows() {
        // reads only: Windows' own list of time zones (no setting is touched)
        #[cfg(windows)]
        {
            use crate::zones::{RealZones, ZoneOs};
            let z = RealZones;
            let t = z.utc_now().as_secs() as i64;
            let bad: Vec<&str> = head().zones.iter().copied().filter(|k| z.offset_at(k, t).is_none()).collect();
            assert!(bad.is_empty(), "this PC has no such zone: {bad:?}");
        }
    }
}

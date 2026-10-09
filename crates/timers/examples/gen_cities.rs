//! `cargo run -p bu-timers --example timers-gencities -- <dir> [out]` - rebuilds `data/cities.txt` (the World clock's city
//! list) from three public files that must sit in `<dir>`: GeoNames `cities15000.txt` + `countryInfo.txt` (CC BY 4.0,
//! geonames.org) and the Unicode CLDR `windowsZones.xml` (IANA zone -> Windows zone key). Writes nothing else, reads
//! nothing else, touches no system setting. Run by hand when the data should be refreshed; the app never needs it.
//!
//! File layout (UTF-8, `\n`): `#W` + one Windows zone key per line (its number = the line's order), `#C` + `CC Country`
//! lines, `#P` + one place per line: `name|cc|zone` or `name|ascii name|cc|zone` (the ascii name only when it differs),
//! most populous first. A place that shares name + country + zone with a bigger one is dropped (same name, same time).

use std::collections::HashMap;
use std::fmt::Write as _;

/// Short country names the drawing / the page already used.
fn short_country(cc: &str, name: &str) -> String {
    match cc {
        "US" => "USA".into(),
        "GB" => "UK".into(),
        "AE" => "UAE".into(),
        _ => name.to_string(),
    }
}

/// GeoNames' name -> the name people (and the drawing) use.
fn shown_name(name: &str) -> &str {
    match name {
        "New York City" => "New York",
        n => n,
    }
}

fn attr<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let p = format!("{name}=\"");
    let i = line.find(&p)? + p.len();
    let j = line[i..].find('"')?;
    Some(&line[i..i + j])
}

fn main() {
    let mut a = std::env::args().skip(1);
    let dir = a.next().expect("usage: timers-gencities <dir> [out]");
    let out = a.next().unwrap_or_else(|| "crates/timers/data/cities.txt".into());
    let read = |f: &str| std::fs::read_to_string(format!("{dir}/{f}")).unwrap_or_else(|e| panic!("{f}: {e}"));

    // IANA id -> Windows key (CLDR; the first line that names the id wins - 001 comes first for its zone)
    let mut iana: HashMap<String, String> = HashMap::new();
    for l in read("windowsZones.xml").lines().filter(|l| l.contains("<mapZone")) {
        let (Some(w), Some(t)) = (attr(l, "other"), attr(l, "type")) else { continue };
        for id in t.split(' ') {
            iana.entry(id.to_string()).or_insert_with(|| w.to_string());
        }
    }
    // IANA ids renamed since CLDR's list was written: the old id's Windows key serves the new one
    for (new, old) in [
        ("Asia/Kolkata", "Asia/Calcutta"),
        ("Asia/Kathmandu", "Asia/Katmandu"),
        ("Asia/Yangon", "Asia/Rangoon"),
        ("Asia/Ho_Chi_Minh", "Asia/Saigon"),
        ("Europe/Kyiv", "Europe/Kiev"),
        ("Atlantic/Faroe", "Atlantic/Faeroe"),
        ("America/Nuuk", "America/Godthab"),
        ("Africa/Asmara", "Africa/Asmera"),
        ("Pacific/Pohnpei", "Pacific/Ponape"),
        ("America/Argentina/Buenos_Aires", "America/Buenos_Aires"),
        ("America/Argentina/Cordoba", "America/Cordoba"),
        ("America/Argentina/Catamarca", "America/Catamarca"),
        ("America/Argentina/Jujuy", "America/Jujuy"),
        ("America/Argentina/Mendoza", "America/Mendoza"),
        ("America/Indiana/Indianapolis", "America/Indianapolis"),
        ("America/Kentucky/Louisville", "America/Louisville"),
    ] {
        if let (false, Some(w)) = (iana.contains_key(new), iana.get(old).cloned()) {
            iana.insert(new.to_string(), w);
        }
    }
    let mut countries: HashMap<String, String> = HashMap::new();
    for l in read("countryInfo.txt").lines().filter(|l| !l.starts_with('#')) {
        let c: Vec<&str> = l.split('\t').collect();
        if c.len() > 4 {
            countries.insert(c[0].to_string(), short_country(c[0], c[4]));
        }
    }

    struct P {
        name: String,
        ascii: String,
        cc: String,
        win: String,
        pop: u64,
    }
    let mut places: Vec<P> = Vec::new();
    let (mut no_zone, mut no_country, mut odd) = (HashMap::<String, u32>::new(), 0u32, 0u32);
    for l in read("cities15000.txt").lines() {
        let c: Vec<&str> = l.split('\t').collect();
        if c.len() < 18 {
            continue;
        }
        let (name, ascii, cc, pop, tz) = (shown_name(c[1]), c[2], c[8], c[14].parse::<u64>().unwrap_or(0), c[17]);
        if name.contains('|') || name.contains('\t') || ascii.contains('|') {
            odd += 1;
            continue;
        }
        if !countries.contains_key(cc) {
            no_country += 1;
            continue;
        }
        let Some(win) = iana.get(tz) else {
            *no_zone.entry(tz.to_string()).or_default() += 1;
            continue;
        };
        places.push(P { name: name.to_string(), ascii: if ascii == name { String::new() } else { ascii.to_string() }, cc: cc.to_string(), win: win.clone(), pop });
    }
    places.sort_by_key(|p| std::cmp::Reverse(p.pop));

    let mut wins: Vec<String> = Vec::new();
    let mut win_ix: HashMap<String, usize> = HashMap::new();
    let mut used_cc: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut body = String::new();
    let mut kept = 0;
    for p in &places {
        if !seen.insert((p.name.clone(), p.cc.clone(), p.win.clone())) {
            continue;
        }
        let n = *win_ix.entry(p.win.clone()).or_insert_with(|| {
            wins.push(p.win.clone());
            wins.len() - 1
        });
        if !used_cc.contains(&p.cc) {
            used_cc.push(p.cc.clone());
        }
        if p.ascii.is_empty() {
            let _ = writeln!(body, "{}|{}|{}", p.name, p.cc, n);
        } else {
            let _ = writeln!(body, "{}|{}|{}|{}", p.name, p.ascii, p.cc, n);
        }
        kept += 1;
    }
    used_cc.sort();
    let mut text = String::from("#W\n");
    for w in &wins {
        let _ = writeln!(text, "{w}");
    }
    text.push_str("#C\n");
    for cc in &used_cc {
        let _ = writeln!(text, "{cc} {}", countries[cc]);
    }
    text.push_str("#P\n");
    text.push_str(&body);
    std::fs::write(&out, &text).expect("write");
    let mut nz: Vec<_> = no_zone.into_iter().collect();
    nz.sort();
    println!("{} places read, {} kept ({} dropped as same name+country+zone), {} Windows zones, {} countries", places.len(), kept, places.len() - kept, wins.len(), used_cc.len());
    println!("skipped: {odd} odd names, {no_country} unknown country, {} places whose IANA zone CLDR has no Windows key: {:?}", nz.iter().map(|x| x.1).sum::<u32>(), nz);
    println!("{out}: {} bytes", text.len());
}

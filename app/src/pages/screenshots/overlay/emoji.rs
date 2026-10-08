//! The overlay's emoji lists and search (menu-v22 ECATS / EKW / eFilter / pickEmoji), from `emoji_data.rs`.

use super::emoji_data::{DATA, QUICK, TAB_ICONS};

/// The emoji the stamp starts with (`co.emoji`).
pub const QUICK_FIRST: &str = QUICK[0];

/// One emoji and its search words.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub e: &'static str,
    pub k: &'static str,
}

/// A section of the More list (`eSecs`): "Most used" first (its list is the quick row), then the categories.
#[derive(Clone, Debug)]
pub struct Section {
    pub id: &'static str,
    pub name: &'static str,
    pub list: Vec<Entry>,
}

/// The categories in the drawing's order.
pub fn categories() -> Vec<Section> {
    DATA.iter()
        .map(|(id, name, s)| Section {
            id,
            name,
            list: s
                .split('|')
                .map(|x| {
                    let i = x.find(' ').unwrap_or(x.len());
                    Entry { e: &x[..i], k: x[i..].trim_start() }
                })
                .collect(),
        })
        .collect()
}

/// The words of an emoji (first category that has it; `EKW`).
pub fn words(e: &str) -> &'static str {
    for (_, _, s) in DATA.iter() {
        for x in s.split('|') {
            let i = x.find(' ').unwrap_or(x.len());
            if &x[..i] == e {
                return x[i..].trim_start();
            }
        }
    }
    ""
}

/// The tab ids in order (`eSecs`): recent, then the category ids.
pub fn tab_ids() -> Vec<&'static str> {
    std::iter::once("recent").chain(DATA.iter().map(|d| d.0)).collect()
}

/// The tab names (their hover tips): "Most used", then the category names.
pub fn tab_names() -> Vec<&'static str> {
    std::iter::once("Most used").chain(DATA.iter().map(|d| d.1)).collect()
}

/// A tab icon's raw SVG (ETAB).
pub fn tab_icon(id: &str) -> &'static str {
    TAB_ICONS.iter().find(|t| t.0 == id).map(|t| t.1).unwrap_or("")
}

/// The search (eFilter): every word typed must be the start of one of the emoji's words; each emoji once, in list order.
pub fn search(q: &str) -> Vec<&'static str> {
    let q = q.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let terms: Vec<&str> = q.split_whitespace().collect();
    let mut hits: Vec<&'static str> = Vec::new();
    for c in categories() {
        for o in c.list {
            if hits.contains(&o.e) {
                continue;
            }
            let k = format!(" {}", o.k);
            if terms.iter().all(|t| k.contains(&format!(" {}", t))) {
                hits.push(o.e);
            }
        }
    }
    hits
}

/// The quick row (eRecent): starts as the most used; one picked from More joins the front (the last one drops off).
#[derive(Clone, Debug)]
pub struct Recent(pub Vec<String>);

impl Default for Recent {
    fn default() -> Self {
        Recent(QUICK.iter().map(|s| s.to_string()).collect())
    }
}

impl Recent {
    pub fn picked_from_more(&mut self, e: &str) {
        if !self.0.iter().any(|x| x == e) {
            self.0.insert(0, e.to_string());
            self.0.truncate(QUICK.len());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_match_the_drawing() {
        let c = categories();
        assert_eq!(c.len(), 8);
        assert_eq!(c[0].name, "Smileys");
        assert_eq!(c[0].list[0], Entry { e: "😀", k: "grinning happy" });
        let n: usize = c.iter().map(|c| c.list.len()).sum();
        assert!(n > 700, "{n} emojis");
        assert_eq!(tab_ids().len(), 9);
        assert_eq!(words("🔥"), "fire lit hot");
    }

    #[test]
    fn search_matches_word_starts_and_every_term() {
        assert_eq!(search("fir")[0], "🔥");
        assert!(search("ire").is_empty(), "only word starts");
        let h = search("heart red");
        assert_eq!(h, vec!["❤️"]);
        assert!(search("   ").is_empty());
    }

    #[test]
    fn a_pick_from_more_joins_the_front() {
        let mut r = Recent::default();
        r.picked_from_more("🍕");
        assert_eq!(r.0[0], "🍕");
        assert_eq!(r.0.len(), 8);
        assert_eq!(r.0[7], "😭", "the last one dropped off");
        r.picked_from_more("🔥");
        assert_eq!(r.0[0], "🍕", "already in the row: no change");
    }
}

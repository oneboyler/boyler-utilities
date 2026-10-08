//! The Toggles search field's matching rule (DESIGN §3.6 Header): "Every typed word must match the row's title, sub-line, hidden
//! keywords or group name." Case-insensitive; a word matches when it is contained in any of those texts.

use crate::rows::Row;

/// Does the row match every word of `query`? An empty query matches everything.
pub fn matches(row: &Row, query: &str) -> bool {
    let haystack = [row.title, row.sub, row.group.title()]
        .into_iter()
        .chain(row.keywords.iter().copied())
        .map(|s| s.to_lowercase())
        .collect::<Vec<_>>();
    query
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .all(|w| haystack.iter().any(|h| h.contains(&w)))
}

/// Byte ranges in the title that the query words hit (the menu highlights them). Overlaps are merged.
pub fn title_hits(row: &Row, query: &str) -> Vec<(usize, usize)> {
    let title = row.title.to_lowercase();
    let mut hits: Vec<(usize, usize)> = Vec::new();
    for w in query.split_whitespace().map(|w| w.to_lowercase()) {
        if w.is_empty() {
            continue;
        }
        let mut from = 0;
        while let Some(pos) = title[from..].find(&w) {
            let start = from + pos;
            hits.push((start, start + w.len()));
            from = start + w.len();
        }
    }
    hits.sort();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (s, e) in hits {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    merged
}

/// The ids of every row matching `query`, in page order.
pub fn search(query: &str) -> Vec<&'static str> {
    crate::rows::ROWS.iter().filter(|r| matches(r, query)).map(|r| r.id).collect()
}

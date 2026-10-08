//! The settings file's text format: a header line, then one value per line:
//! `section TAB key TAB type TAB value`; type = b (true/false) · i (i64) · f (f64, Rust's shortest round-trip text) ·
//! s (string) · l (string list: `<count>:` then the items joined by `,`). In every field `\` `TAB` `LF` `CR` `,` are
//! written as `\\` `\t` `\n` `\r` `\,`. Anything else = broken file (the store keeps it aside and starts from defaults).

use std::collections::BTreeMap;

use super::Value;

pub const HEADER: &str = "Boyler Utilities settings 1";

/// Why a file could not be read (only for tests / logs; the store just keeps the file aside).
#[derive(Debug, PartialEq, Eq)]
pub struct Broken(pub String);

pub fn write(values: &BTreeMap<(String, String), Value>) -> String {
    let mut out = String::with_capacity(64 + values.len() * 32);
    out.push_str(HEADER);
    out.push('\n');
    for ((section, key), v) in values {
        let (t, text) = match v {
            Value::Bool(b) => ('b', b.to_string()),
            Value::Int(i) => ('i', i.to_string()),
            Value::Float(f) => ('f', f.to_string()),
            Value::Str(s) => ('s', esc(s)),
            Value::List(items) => {
                let joined: Vec<String> = items.iter().map(|s| esc(s)).collect();
                ('l', format!("{}:{}", items.len(), joined.join(",")))
            }
        };
        out.push_str(&esc(section));
        out.push('\t');
        out.push_str(&esc(key));
        out.push('\t');
        out.push(t);
        out.push('\t');
        out.push_str(&text);
        out.push('\n');
    }
    out
}

pub fn parse(bytes: &[u8]) -> Result<BTreeMap<(String, String), Value>, Broken> {
    let text = std::str::from_utf8(bytes).map_err(|_| Broken("not UTF-8".into()))?;
    let mut lines = text.split('\n');
    if lines.next().map(|l| l.trim_end_matches('\r')) != Some(HEADER) {
        return Err(Broken("no header".into()));
    }
    let mut values = BTreeMap::new();
    for (n, line) in lines.enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        let bad = |why: &str| Broken(format!("line {}: {why}", n + 2));
        let fields: Vec<&str> = line.split('\t').collect();
        let [section, key, t, raw] = fields[..] else { return Err(bad("not 4 fields")) };
        let section = unesc(section).ok_or_else(|| bad("bad escape"))?;
        let key = unesc(key).ok_or_else(|| bad("bad escape"))?;
        let value = match t {
            "b" => match raw {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => return Err(bad("bad bool")),
            },
            "i" => Value::Int(raw.parse().map_err(|_| bad("bad integer"))?),
            "f" => Value::Float(raw.parse().map_err(|_| bad("bad number"))?),
            "s" => Value::Str(unesc(raw).ok_or_else(|| bad("bad escape"))?),
            "l" => Value::List(parse_list(raw).ok_or_else(|| bad("bad list"))?),
            _ => return Err(bad("unknown type")),
        };
        if values.insert((section, key), value).is_some() {
            return Err(bad("value set twice"));
        }
    }
    Ok(values)
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            ',' => out.push_str("\\,"),
            c => out.push(c),
        }
    }
    out
}

fn unesc(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        match c {
            '\\' => out.push(match it.next()? {
                '\\' => '\\',
                't' => '\t',
                'n' => '\n',
                'r' => '\r',
                ',' => ',',
                _ => return None,
            }),
            ',' | '\r' => return None,
            c => out.push(c),
        }
    }
    Some(out)
}

/// `<count>:a,b,c` → items; the count must match (so an empty list and a list of one empty string differ).
fn parse_list(raw: &str) -> Option<Vec<String>> {
    let (count, rest) = raw.split_once(':')?;
    let count: usize = count.parse().ok()?;
    let mut items = Vec::new();
    if count > 0 {
        let mut cur = String::new();
        let mut it = rest.chars();
        while let Some(c) = it.next() {
            match c {
                '\\' => {
                    cur.push('\\');
                    cur.push(it.next()?);
                }
                ',' => items.push(unesc(&std::mem::take(&mut cur))?),
                c => cur.push(c),
            }
        }
        items.push(unesc(&cur)?);
    } else if !rest.is_empty() {
        return None;
    }
    (items.len() == count).then_some(items)
}

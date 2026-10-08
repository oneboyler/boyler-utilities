//! Steam's KeyValues TEXT format (".vdf"), read and edited LOSSLESSLY.
//!
//! Steam's controller layouts, index files and per-controller preferences are KeyValues text. The app must change one value
//! and keep every other byte of the file exactly as it was (tabs, blank lines, `CRLF` or `LF`, odd spacing in Steam's own
//! templates, escapes). So a [`Doc`] keeps the original text and a tree of byte positions; an edit splices only the bytes it
//! changes and re-reads the tree. Nothing is ever re-serialised from the tree.
//!
//! Format (measured on 128 local layouts + Steam's templates, 2026-10-08): `"key" "value"` pairs and `"key" { ... }` blocks,
//! keys may repeat (`"group"` many times), strings are quoted with `\\` and `\"` escapes, `//` comments, optional
//! `[$WIN32]`-style conditions after a value, rare unquoted tokens. Steam writes tab-indented files with `"key"\t\t"value"`.

use std::ops::Range;

/// A parse problem: what and the byte offset. (Never a panic: a broken file is refused, not written.)
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Steam file is not valid KeyValues text: {what} at byte {at}")]
pub struct ParseError {
    pub what: &'static str,
    pub at: usize,
}

/// One key with its value or block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// The key, unescaped.
    pub key: String,
    /// Byte range of the key token (with its quotes).
    pub key_span: Range<usize>,
    /// Byte offset where the key's line starts.
    pub line_start: usize,
    /// Byte offset just past the node (past the value's closing quote / condition, or past the block's `}`).
    pub end: usize,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// `raw` = the bytes between the quotes (or the unquoted token); `value` = unescaped.
    Value { raw: Range<usize>, quoted: bool, value: String },
    /// `open` / `close` = offsets of `{` and `}`.
    Block { open: usize, close: usize, children: Vec<Node> },
}

impl Node {
    pub fn value(&self) -> Option<&str> {
        match &self.kind {
            Kind::Value { value, .. } => Some(value),
            Kind::Block { .. } => None,
        }
    }
    pub fn children(&self) -> &[Node] {
        match &self.kind {
            Kind::Block { children, .. } => children,
            Kind::Value { .. } => &[],
        }
    }
    pub fn is_block(&self) -> bool {
        matches!(self.kind, Kind::Block { .. })
    }
    /// First child with this key (Steam keys are case-insensitive).
    pub fn child(&self, key: &str) -> Option<&Node> {
        self.children().iter().find(|n| n.key.eq_ignore_ascii_case(key))
    }
    pub fn child_value(&self, key: &str) -> Option<&str> {
        self.child(key).and_then(|n| n.value())
    }
    pub fn children_named<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children().iter().filter(move |n| n.key.eq_ignore_ascii_case(key))
    }
}

/// The address of a node: child indexes from the top level down. Stays valid until the next edit.
pub type Addr = Vec<usize>;

/// A KeyValues text file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doc {
    text: String,
    nodes: Vec<Node>,
}

impl Doc {
    pub fn parse(text: impl Into<String>) -> Result<Doc, ParseError> {
        let text = text.into();
        let nodes = Parser::new(&text).parse_all()?;
        Ok(Doc { text, nodes })
    }

    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn into_text(self) -> String {
        self.text
    }
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }
    /// The first top-level node (Steam files have exactly one: `"controller_mappings"`, `"controller_config"` …).
    pub fn top(&self) -> Option<&Node> {
        self.nodes.first()
    }

    pub fn get(&self, addr: &[usize]) -> Option<&Node> {
        let (first, rest) = addr.split_first()?;
        let mut n = self.nodes.get(*first)?;
        for i in rest {
            n = n.children().get(*i)?;
        }
        Some(n)
    }

    /// The line ending Steam used in this file (`"\r\n"` for its templates, `"\n"` for the files it saves).
    pub fn newline(&self) -> &'static str {
        match self.text.find('\n') {
            Some(i) if i > 0 && self.text.as_bytes()[i - 1] == b'\r' => "\r\n",
            _ => "\n",
        }
    }

    /// The separator between key and value used in this file (Steam's own: two tabs).
    pub fn separator(&self) -> String {
        self.separator_in(None)
    }

    fn separator_in(&self, prefer_block: Option<&[usize]>) -> String {
        let from_nodes = |nodes: &[Node]| -> Option<String> {
            nodes.iter().find_map(|n| match &n.kind {
                Kind::Value { raw, quoted, .. } => {
                    let vstart = if *quoted { raw.start - 1 } else { raw.start };
                    let sep = &self.text[n.key_span.end..vstart];
                    (!sep.is_empty() && sep.chars().all(|c| c == ' ' || c == '\t')).then(|| sep.to_string())
                }
                _ => None,
            })
        };
        if let Some(addr) = prefer_block {
            if let Some(s) = self.get(addr).and_then(|b| from_nodes(b.children())) {
                return s;
            }
        }
        fn walk(nodes: &[Node], f: &dyn Fn(&[Node]) -> Option<String>) -> Option<String> {
            if let Some(s) = f(nodes) {
                return Some(s);
            }
            nodes.iter().find_map(|n| walk(n.children(), f))
        }
        walk(&self.nodes, &from_nodes).unwrap_or_else(|| "\t\t".to_string())
    }

    fn line_start_of(&self, pos: usize) -> usize {
        self.text[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0)
    }

    fn splice(&mut self, range: Range<usize>, with: &str) -> Result<(), ParseError> {
        let mut t = String::with_capacity(self.text.len() + with.len());
        t.push_str(&self.text[..range.start]);
        t.push_str(with);
        t.push_str(&self.text[range.end..]);
        let nodes = Parser::new(&t).parse_all()?;
        self.text = t;
        self.nodes = nodes;
        Ok(())
    }

    /// Replace one value (only the bytes between its quotes change).
    pub fn set_value(&mut self, addr: &[usize], value: &str) -> Result<(), EditError> {
        let n = self.get(addr).ok_or(EditError::NoSuchNode)?;
        let (raw, quoted) = match &n.kind {
            Kind::Value { raw, quoted, .. } => (raw.clone(), *quoted),
            Kind::Block { .. } => return Err(EditError::NotAValue),
        };
        if n.value() == Some(value) {
            return Ok(()); // already so: not one byte changes
        }
        let enc = escape(value);
        if !quoted && enc.chars().any(|c| c.is_whitespace() || c == '{' || c == '}' || c == '"') {
            // an unquoted token that now needs quotes
            let q = format!("\"{enc}\"");
            return Ok(self.splice(raw, &q)?);
        }
        Ok(self.splice(raw, &enc)?)
    }

    /// Add `"key" "value"` as the last line of a block, indented like its siblings.
    pub fn insert_value(&mut self, block: &[usize], key: &str, value: &str) -> Result<(), EditError> {
        let sep = self.separator_in(Some(block));
        let line = format!("\"{}\"{}\"{}\"", escape(key), sep, escape(value));
        self.insert_lines(block, &[line])
    }

    /// Add an empty block `"key" { }` (three lines, Steam's style) at the end of a block.
    pub fn insert_block(&mut self, block: &[usize], key: &str) -> Result<(), EditError> {
        self.insert_lines(block, &[format!("\"{}\"", escape(key)), "{".into(), "}".into()])
    }

    /// Insert ready-made lines (already relative: each gets the block's child indent) at the end of a block.
    pub fn insert_lines(&mut self, block: &[usize], lines: &[String]) -> Result<(), EditError> {
        let nl = self.newline();
        let b = self.get(block).ok_or(EditError::NoSuchNode)?;
        let block_indent: String = self.text[b.line_start..].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        let (close, children_indent) = match &b.kind {
            Kind::Block { close, children, .. } => {
                let indent = match children.first() {
                    Some(c) if self.text[c.line_start..c.key_span.start].chars().all(|c| c == ' ' || c == '\t') => {
                        self.text[c.line_start..c.key_span.start].to_string()
                    }
                    // `"x" { "a" "1" }` — children on the block's own line: one tab deeper than the block
                    Some(_) => format!("{block_indent}\t"),
                    None => {
                        let ls = self.line_start_of(*close);
                        let mut s: String = self.text[ls..*close].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
                        s.push('\t');
                        s
                    }
                };
                (*close, indent)
            }
            Kind::Value { .. } => return Err(EditError::NotABlock),
        };
        if !children_indent.chars().all(|c| c == ' ' || c == '\t') {
            return Err(EditError::OddLayout);
        }
        let ls = self.line_start_of(close);
        let before_close = &self.text[ls..close];
        let mut body = String::new();
        if before_close.chars().all(|c| c == ' ' || c == '\t') {
            // the usual case: `}` on its own line → insert full lines in front of that line
            for l in lines {
                body.push_str(&children_indent);
                body.push_str(l);
                body.push_str(nl);
            }
            Ok(self.splice(ls..ls, &body)?)
        } else {
            // `{ ... }` on one line: open a new line in front of the `}`
            let close_indent: String = self.text[ls..].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
            body.push_str(nl);
            for l in lines {
                body.push_str(&children_indent);
                body.push_str(l);
                body.push_str(nl);
            }
            body.push_str(&close_indent);
            Ok(self.splice(close..close, &body)?)
        }
    }

    /// Insert ready-made lines (relative, like [`Doc::insert_lines`]) in front of the node at `addr`, with its indent.
    pub fn insert_lines_before(&mut self, addr: &[usize], lines: &[String]) -> Result<(), EditError> {
        let nl = self.newline();
        let n = self.get(addr).ok_or(EditError::NoSuchNode)?;
        let indent = self.text[n.line_start..n.key_span.start].to_string();
        if !indent.chars().all(|c| c == ' ' || c == '\t') {
            return Err(EditError::OddLayout);
        }
        let at = n.line_start;
        let mut body = String::new();
        for l in lines {
            body.push_str(&indent);
            body.push_str(l);
            body.push_str(nl);
        }
        Ok(self.splice(at..at, &body)?)
    }

    /// Remove a node; if it sits alone on its lines, the whole lines go (no blank line is left behind).
    pub fn remove(&mut self, addr: &[usize]) -> Result<(), EditError> {
        let n = self.get(addr).ok_or(EditError::NoSuchNode)?;
        let ls = n.line_start;
        let lead_ok = self.text[ls..n.key_span.start].chars().all(|c| c == ' ' || c == '\t');
        let rest = &self.text[n.end..];
        let eol = rest.find('\n').map(|i| n.end + i + 1).unwrap_or(self.text.len());
        let trail_ok = self.text[n.end..eol].trim().is_empty();
        let range = if lead_ok && trail_ok { ls..eol } else { n.key_span.start..n.end };
        Ok(self.splice(range, "")?)
    }

    /// Rename a key (only the key's bytes change).
    pub fn set_key(&mut self, addr: &[usize], key: &str) -> Result<(), EditError> {
        let n = self.get(addr).ok_or(EditError::NoSuchNode)?;
        let span = n.key_span.clone();
        let quoted = self.text.as_bytes().get(span.start) == Some(&b'"');
        let with = if quoted { format!("\"{}\"", escape(key)) } else { escape(key) };
        Ok(self.splice(span, &with)?)
    }

    /// Find the address of the first child named `key` of the node at `addr` (`[]` = top level).
    pub fn find(&self, addr: &[usize], key: &str) -> Option<Addr> {
        let list = if addr.is_empty() { &self.nodes[..] } else { self.get(addr)?.children() };
        let i = list.iter().position(|n| n.key.eq_ignore_ascii_case(key))?;
        let mut a = addr.to_vec();
        a.push(i);
        Some(a)
    }

    /// Find or create the block `key` under `addr`; returns its address.
    pub fn ensure_block(&mut self, addr: &[usize], key: &str) -> Result<Addr, EditError> {
        if let Some(a) = self.find(addr, key) {
            return if self.get(&a).map(|n| n.is_block()).unwrap_or(false) { Ok(a) } else { Err(EditError::NotABlock) };
        }
        self.insert_block(addr, key)?;
        self.find_last(addr, key).ok_or(EditError::NoSuchNode)
    }

    /// The last child named `key` (after an insert it is the new one).
    pub fn find_last(&self, addr: &[usize], key: &str) -> Option<Addr> {
        let list = if addr.is_empty() { &self.nodes[..] } else { self.get(addr)?.children() };
        let i = list.iter().rposition(|n| n.key.eq_ignore_ascii_case(key))?;
        let mut a = addr.to_vec();
        a.push(i);
        Some(a)
    }

    /// Set `key` to `value` inside the block at `addr`: changes the existing value, or adds the line.
    pub fn upsert(&mut self, addr: &[usize], key: &str, value: &str) -> Result<(), EditError> {
        match self.find(addr, key) {
            Some(a) => self.set_value(&a, value),
            None => self.insert_value(addr, key, value),
        }
    }
}

/// An edit that can't be made (the file is left exactly as it was).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("the place to change is not in the file")]
    NoSuchNode,
    #[error("expected a value, found a block")]
    NotAValue,
    #[error("expected a block, found a value")]
    NotABlock,
    #[error("the file's layout is too unusual to edit safely")]
    OddLayout,
    #[error(transparent)]
    Parse(#[from] ParseError),
}

/// Escape a string for a quoted KeyValues token.
pub fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            _ => o.push(c),
        }
    }
    o
}

fn unescape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => o.push('\n'),
                Some('t') => o.push('\t'),
                Some('\\') => o.push('\\'),
                Some('"') => o.push('"'),
                Some(x) => {
                    o.push('\\');
                    o.push(x);
                }
                None => o.push('\\'),
            }
        } else {
            o.push(c);
        }
    }
    o
}

enum Tok {
    Str { span: Range<usize>, raw: Range<usize>, quoted: bool },
    Open(usize),
    Close(usize),
}

struct Parser<'a> {
    s: &'a str,
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        let i = if s.starts_with('\u{feff}') { 3 } else { 0 };
        Parser { s, b: s.as_bytes(), i }
    }

    fn skip_ws(&mut self) {
        loop {
            while self.i < self.b.len() && (self.b[self.i] as char).is_ascii_whitespace() {
                self.i += 1;
            }
            if self.b[self.i..].starts_with(b"//") {
                while self.i < self.b.len() && self.b[self.i] != b'\n' {
                    self.i += 1;
                }
                continue;
            }
            break;
        }
    }

    fn next(&mut self) -> Result<Option<Tok>, ParseError> {
        self.skip_ws();
        if self.i >= self.b.len() {
            return Ok(None);
        }
        let start = self.i;
        match self.b[self.i] {
            b'{' => {
                self.i += 1;
                Ok(Some(Tok::Open(start)))
            }
            b'}' => {
                self.i += 1;
                Ok(Some(Tok::Close(start)))
            }
            b'"' => {
                self.i += 1;
                let rs = self.i;
                while self.i < self.b.len() && self.b[self.i] != b'"' {
                    if self.b[self.i] == b'\\' && self.i + 1 < self.b.len() {
                        self.i += 1;
                    }
                    self.i += 1;
                }
                if self.i >= self.b.len() {
                    return Err(ParseError { what: "a string that never ends", at: start });
                }
                let re = self.i;
                self.i += 1;
                Ok(Some(Tok::Str { span: start..self.i, raw: rs..re, quoted: true }))
            }
            _ => {
                while self.i < self.b.len() {
                    let c = self.b[self.i];
                    if (c as char).is_ascii_whitespace() || c == b'{' || c == b'}' || c == b'"' {
                        break;
                    }
                    self.i += 1;
                }
                Ok(Some(Tok::Str { span: start..self.i, raw: start..self.i, quoted: false }))
            }
        }
    }

    /// A `[$CONDITION]` right after a value (same line) is part of the node.
    fn skip_condition(&mut self) -> usize {
        let save = self.i;
        let mut j = self.i;
        while j < self.b.len() && (self.b[j] == b' ' || self.b[j] == b'\t') {
            j += 1;
        }
        if j < self.b.len() && self.b[j] == b'[' {
            if let Some(k) = self.s[j..].find(']') {
                let line_end = self.s[j..].find('\n').unwrap_or(usize::MAX);
                if k < line_end {
                    self.i = j + k + 1;
                    return self.i;
                }
            }
        }
        self.i = save;
        save
    }

    fn line_start(&self, pos: usize) -> usize {
        self.s[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0)
    }

    fn parse_all(mut self) -> Result<Vec<Node>, ParseError> {
        let (nodes, close) = self.parse_list(true)?;
        if let Some(c) = close {
            return Err(ParseError { what: "a `}` without its `{`", at: c });
        }
        Ok(nodes)
    }

    /// Reads nodes until a `}` (returned) or the end (top level only).
    fn parse_list(&mut self, top: bool) -> Result<(Vec<Node>, Option<usize>), ParseError> {
        let mut out = Vec::new();
        loop {
            let t = self.next()?;
            let (span, raw, quoted) = match t {
                None if top => return Ok((out, None)),
                None => return Err(ParseError { what: "a block that never closes", at: self.b.len() }),
                Some(Tok::Close(at)) => return Ok((out, Some(at))),
                Some(Tok::Open(at)) => return Err(ParseError { what: "a `{` without a key", at }),
                Some(Tok::Str { span, raw, quoted }) => (span, raw, quoted),
            };
            let key_text = &self.s[raw.clone()];
            // `#include` / `#base` lines: key + value, kept as a value node
            let key = if quoted { unescape(key_text) } else { key_text.to_string() };
            let line_start = self.line_start(span.start);
            match self.next()? {
                Some(Tok::Open(open)) => {
                    let (children, close) = self.parse_list(false)?;
                    let close = close.ok_or(ParseError { what: "a block that never closes", at: open })?;
                    out.push(Node { key, key_span: span, line_start, end: close + 1, kind: Kind::Block { open, close, children } });
                }
                Some(Tok::Str { span: vspan, raw: vraw, quoted: vq }) => {
                    let v = &self.s[vraw.clone()];
                    let value = if vq { unescape(v) } else { v.to_string() };
                    let mut end = vspan.end;
                    let c = self.skip_condition();
                    if c > end {
                        end = c;
                    }
                    out.push(Node { key, key_span: span, line_start, end, kind: Kind::Value { raw: vraw, quoted: vq, value } });
                }
                Some(Tok::Close(at)) => return Err(ParseError { what: "a key without a value", at }),
                None => return Err(ParseError { what: "a key without a value", at: span.start }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LF: &str = "\"controller_config\"\n{\n\t\"252950\"\n\t{\n\t\t\"autosave\"\t\t\"1\"\n\t}\n}\n";

    #[test]
    fn parses_and_keeps_text() {
        let d = Doc::parse(LF).unwrap();
        assert_eq!(d.text(), LF);
        let top = d.top().unwrap();
        assert_eq!(top.key, "controller_config");
        assert_eq!(top.child("252950").unwrap().child_value("autosave"), Some("1"));
        assert_eq!(d.newline(), "\n");
    }

    #[test]
    fn set_value_changes_only_the_value() {
        let mut d = Doc::parse(LF).unwrap();
        let a = d.find(&[0], "252950").unwrap();
        let a = d.find(&a, "autosave").unwrap();
        d.set_value(&a, "0").unwrap();
        assert_eq!(d.text(), LF.replace("\"autosave\"\t\t\"1\"", "\"autosave\"\t\t\"0\""));
    }

    #[test]
    fn insert_matches_indent_separator_and_crlf() {
        let crlf = LF.replace('\n', "\r\n");
        let mut d = Doc::parse(crlf.clone()).unwrap();
        let a = d.find(&[0], "252950").unwrap();
        d.insert_value(&a, "workshop", "123").unwrap();
        assert_eq!(d.text(), crlf.replace("\"1\"\r\n\t}", "\"1\"\r\n\t\t\"workshop\"\t\t\"123\"\r\n\t}"));
    }

    #[test]
    fn insert_into_empty_block_and_remove() {
        let src = "\"a\"\n{\n\t\"b\"\n\t{\n\t}\n}\n";
        let mut d = Doc::parse(src).unwrap();
        let b = d.find(&[0], "b").unwrap();
        d.insert_value(&b, "k", "v").unwrap();
        assert_eq!(d.text(), "\"a\"\n{\n\t\"b\"\n\t{\n\t\t\"k\"\t\t\"v\"\n\t}\n}\n");
        let k = d.find(&b, "k").unwrap();
        d.remove(&k).unwrap();
        assert_eq!(d.text(), src);
    }

    #[test]
    fn escapes_round_trip() {
        let src = "\"m\"\n{\n\t\"url\"\t\t\"autosave://C:\\\\Steam\\\\x.vdf\"\n}\n";
        let mut d = Doc::parse(src).unwrap();
        let a = d.find(&[0], "url").unwrap();
        assert_eq!(d.get(&a).unwrap().value(), Some("autosave://C:\\Steam\\x.vdf"));
        d.set_value(&a, "autosave://D:\\y.vdf").unwrap();
        assert!(d.text().contains("\"autosave://D:\\\\y.vdf\""));
    }

    #[test]
    fn odd_spacing_comments_and_one_line_blocks() {
        let src = "// head\r\n\"m\"\r\n{\r\n\t\"title\" \"#Title\" [$WIN32]\r\n\t\"x\" { \"a\" \"1\" }\r\n\r\n\t\"y\"\t\t\"2\" \r\n}\r\n";
        let mut d = Doc::parse(src).unwrap();
        assert_eq!(d.text(), src);
        let x = d.find(&[0], "x").unwrap();
        d.insert_value(&x, "b", "2").unwrap();
        let d2 = Doc::parse(d.text()).unwrap();
        assert_eq!(d2.top().unwrap().child("x").unwrap().child_value("b"), Some("2"));
        assert_eq!(d2.top().unwrap().child_value("title"), Some("#Title"));
    }

    #[test]
    fn broken_files_are_refused() {
        assert!(Doc::parse("\"a\"\n{\n\t\"b\"\t\"1\"\n").is_err());
        assert!(Doc::parse("\"a\" \"unterminated").is_err());
        assert!(Doc::parse("}").is_err());
    }
}

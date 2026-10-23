//! Order 116: typing straight into a note - its title (one line) and its text (lines that wrap). The page keeps the
//! words in the store and one caret (a char index) for the field that has the focus; these functions break the words into
//! the lines they are drawn in and move / edit at the caret. `w` measures a piece of text in the field's font (the
//! painter's own shaping in the app, a fixed width per char in the tests).

pub const VK_BACK: u16 = 0x08;
pub const VK_RETURN: u16 = 0x0D;
pub const VK_END: u16 = 0x23;
pub const VK_HOME: u16 = 0x24;
pub const VK_LEFT: u16 = 0x25;
pub const VK_UP: u16 = 0x26;
pub const VK_RIGHT: u16 = 0x27;
pub const VK_DOWN: u16 = 0x28;
pub const VK_DELETE: u16 = 0x2E;

/// The drawn lines: (first char, chars) - a line break ends a line (not counted), a line too wide breaks after its last
/// space (the space stays at the line's end, as in a browser), a single word wider than the line breaks between letters.
/// Each word is measured once (a line's width = its words + its spaces), each letter only inside a word too wide for a line.
pub fn lines(all: &[char], maxw: f32, w: &dyn Fn(&str) -> f32) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let sw = w(" ");
    let mut start = 0usize;
    let mut cur = 0.0f32;
    let mut i = 0usize;
    while i < all.len() {
        match all[i] {
            '\n' => {
                out.push((start, i - start));
                start = i + 1;
                cur = 0.0;
                i += 1;
            }
            ' ' => {
                cur += sw;
                i += 1;
            }
            _ => {
                let j = (i..all.len()).find(|&k| all[k] == ' ' || all[k] == '\n').unwrap_or(all.len());
                let ww = w(&all[i..j].iter().collect::<String>());
                if cur + ww <= maxw + 0.001 {
                    cur += ww;
                    i = j;
                } else if i > start {
                    // the word goes to the next line (the spaces before it hang at this one's end)
                    out.push((start, i - start));
                    start = i;
                    cur = 0.0;
                } else {
                    // one word wider than the line: broken between its letters
                    for k in i..j {
                        let cw = w(&all[k].to_string());
                        if k > start && cur + cw > maxw + 0.001 {
                            out.push((start, k - start));
                            start = k;
                            cur = 0.0;
                        }
                        cur += cw;
                    }
                    i = j;
                }
            }
        }
    }
    out.push((start, all.len() - start));
    out
}

/// The line the caret is on and its x (from the line's start).
pub fn caret_at(all: &[char], ls: &[(usize, usize)], at: usize, w: &dyn Fn(&str) -> f32) -> (usize, f32) {
    let at = at.min(all.len());
    // the last line starting at or before the caret (a caret right after a wrapped line's space is on the next line)
    let k = ls.iter().rposition(|(s, _)| *s <= at).unwrap_or(0);
    let s = ls[k].0;
    (k, w(&all[s..at].iter().collect::<String>()))
}

/// The char index nearest x on line k.
pub fn at_x(all: &[char], ls: &[(usize, usize)], k: usize, x: f32, w: &dyn Fn(&str) -> f32) -> usize {
    let k = k.min(ls.len() - 1);
    let (s, end) = (ls[k].0, line_end(ls, k));
    let mut best = (s, f32::MAX);
    for i in s..=end {
        let d = (w(&all[s..i].iter().collect::<String>()) - x).abs();
        if d < best.1 {
            best = (i, d);
        }
    }
    best.0
}

/// The last caret place on line k: its end - but before a wrapped line's hanging space (after it is the next line's start).
fn line_end(ls: &[(usize, usize)], k: usize) -> usize {
    let (s, n) = ls[k];
    let wrapped = k + 1 < ls.len() && ls[k + 1].0 == s + n;
    if wrapped && n > 0 {
        s + n - 1
    } else {
        s + n
    }
}

/// Put `add` in at the caret (typed, pasted); the caret goes after it.
pub fn insert(text: &mut String, at: &mut usize, add: &str) {
    let mut v: Vec<char> = text.chars().collect();
    let i = (*at).min(v.len());
    let a: Vec<char> = add.chars().collect();
    let n = a.len();
    v.splice(i..i, a);
    *text = v.into_iter().collect();
    *at = i + n;
}

/// A key at the caret. `maxw` = the field's line width (Up / Down / Home / End go by the drawn lines). true = the words
/// changed (save them).
pub fn key(text: &mut String, at: &mut usize, vk: u16, maxw: f32, w: &dyn Fn(&str) -> f32) -> bool {
    let mut v: Vec<char> = text.chars().collect();
    let i = (*at).min(v.len());
    let ls = lines(&v, maxw, w);
    let (k, x) = caret_at(&v, &ls, i, w);
    let (to, changed) = match vk {
        VK_BACK if i > 0 => {
            v.remove(i - 1);
            (i - 1, true)
        }
        VK_DELETE if i < v.len() => {
            v.remove(i);
            (i, true)
        }
        VK_LEFT => (i.saturating_sub(1), false),
        VK_RIGHT => ((i + 1).min(v.len()), false),
        VK_HOME => (ls[k].0, false),
        VK_END => (line_end(&ls, k), false),
        VK_UP if k > 0 => (at_x(&v, &ls, k - 1, x, w), false),
        VK_UP => (0, false),
        VK_DOWN if k + 1 < ls.len() => (at_x(&v, &ls, k + 1, x, w), false),
        VK_DOWN => (v.len(), false),
        _ => (i, false),
    };
    if changed {
        *text = v.into_iter().collect();
    }
    *at = to;
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 10 px per char
    fn w(s: &str) -> f32 {
        s.chars().count() as f32 * 10.0
    }
    fn ch(s: &str) -> Vec<char> {
        s.chars().collect()
    }
    fn texts(s: &str, maxw: f32) -> Vec<String> {
        let c = ch(s);
        lines(&c, maxw, &w).iter().map(|(a, n)| c[*a..a + n].iter().collect()).collect()
    }

    #[test]
    fn words_wrap_at_spaces_line_breaks_end_lines_and_long_words_break() {
        assert_eq!(texts("milk eggs bread", 100.0), ["milk eggs ", "bread"]);
        assert_eq!(texts("a\nb\n", 100.0), ["a", "b", ""]);
        assert_eq!(texts("abcdefghijkl", 50.0), ["abcde", "fghij", "kl"]);
        assert_eq!(texts("", 50.0), [""]);
    }

    #[test]
    fn typing_and_deleting_at_the_caret() {
        let (mut t, mut at) = (String::from("mlk"), 1);
        insert(&mut t, &mut at, "i");
        assert_eq!((t.as_str(), at), ("milk", 2));
        assert!(key(&mut t, &mut at, VK_BACK, 100.0, &w));
        assert_eq!((t.as_str(), at), ("mlk", 1));
        assert!(key(&mut t, &mut at, VK_DELETE, 100.0, &w));
        assert_eq!((t.as_str(), at), ("mk", 1));
        assert!(!key(&mut t, &mut at, VK_LEFT, 100.0, &w));
        assert!(!key(&mut t, &mut at, VK_BACK, 100.0, &w), "nothing before the caret");
        assert_eq!((t.as_str(), at), ("mk", 0));
        insert(&mut t, &mut at, "čaj\n");
        assert_eq!((t.as_str(), at), ("čaj\nmk", 4));
    }

    #[test]
    fn up_down_home_end_follow_the_drawn_lines() {
        // "milk eggs " / "bread"; caret after "br" (index 12)
        let (mut t, mut at) = (String::from("milk eggs bread"), 12);
        key(&mut t, &mut at, VK_UP, 100.0, &w);
        assert_eq!(at, 2, "same x one line up");
        key(&mut t, &mut at, VK_END, 100.0, &w);
        assert_eq!(at, 9, "the end of a wrapped line is before its hanging space");
        key(&mut t, &mut at, VK_DOWN, 100.0, &w);
        assert_eq!(at, 15, "below: as far as that line goes");
        key(&mut t, &mut at, VK_HOME, 100.0, &w);
        assert_eq!(at, 10);
        key(&mut t, &mut at, VK_DOWN, 100.0, &w);
        assert_eq!(at, 15, "the last line: down goes to the end");
    }
}

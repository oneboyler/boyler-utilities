//! Small helpers of Tweaks, Startup and Performance that are not shared pieces: the search-word marking of Tweaks'
//! titles (`.tti mark`). (The page-local copies of Order 021 are gone: the pages use ui/pieces now. The admin note text
//! is `crate::admin::NOT_CHANGED` since Order 039.)

use crate::gfx::{sh, Font, Rgba};
use crate::ui::el::El;
use crate::ui::SEL;

/// A text with the search words marked (`.tti mark{color:inherit;background:var(--sel);border-radius:3px;
/// box-shadow:0 0 0 1.5px var(--sel)}`), every word found case-insensitively. No words = the plain text with an ellipsis.
pub fn marked(text: &str, words: &[String], font: Font, color: Rgba, line_h: f32) -> El {
    let spans = mark_spans(text, words);
    if spans.iter().all(|(_, m)| !m) {
        return El::text(text, font, color, line_h).ellipsis();
    }
    let n = spans.len();
    let mut r = El::row().center().min_w(0.0);
    // an inline `mark` paints its background over the text's CONTENT AREA (rounded ascent + descent), not the line box:
    // 17 px at 13 px Segoe UI Variable (measured in the drawing: mark rect 22.30 x 17 at the line's top, line 17.55);
    // other sizes: 1.33 em rounded (guess - no other size is searched in the drawing)
    let ch = if font.size100 == 1300 { 17.0 } else { (font.size100 as f32 / 100.0 * 1.33).round() };
    for (i, (s, m)) in spans.into_iter().enumerate() {
        let mut t = El::text(s, font, color, line_h);
        t = if i + 1 == n { t.ellipsis() } else { t.none() };
        if m {
            let bg = El::block().abs(0.0, 0.0, 0.0, f32::NAN).h(ch).bg(SEL()).radius(3.0).shadow(&[sh(0.0, 0.0, 0.0, 1.5, SEL())]).no_hit();
            t = El::block().none().child(bg).child(t);
        }
        r = r.child(t);
    }
    r
}

/// The text split into (part, is_marked) by the words (the drawing's `split(new RegExp('(w1|w2)', 'ig'))`).
pub fn mark_spans(text: &str, words: &[String]) -> Vec<(String, bool)> {
    let chars: Vec<char> = text.chars().collect();
    let low: Vec<char> = chars.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
    let ws: Vec<Vec<char>> = words.iter().filter(|w| !w.is_empty()).map(|w| w.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect()).collect();
    let mut out: Vec<(String, bool)> = Vec::new();
    let mut i = 0;
    let mut plain = String::new();
    while i < chars.len() {
        // the regexp alternation: the first word (in the typed order) that matches here wins
        let hit = ws.iter().find(|w| i + w.len() <= low.len() && low[i..i + w.len()] == w[..]).map(|w| w.len());
        match hit {
            Some(len) => {
                if !plain.is_empty() {
                    out.push((std::mem::take(&mut plain), false));
                }
                out.push((chars[i..i + len].iter().collect(), true));
                i += len;
            }
            None => {
                plain.push(chars[i]);
                i += 1;
            }
        }
    }
    if !plain.is_empty() {
        out.push((plain, false));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_every_word_without_case() {
        let w = vec!["sh".to_string(), "file".to_string()];
        let s = mark_spans("Show file extensions", &w);
        assert_eq!(s, vec![("Sh".into(), true), ("ow ".into(), false), ("file".into(), true), (" extensions".into(), false)]);
        assert_eq!(mark_spans("“Recommended”", &["rec".into()]), vec![("“".into(), false), ("Rec".into(), true), ("ommended”".into(), false)]);
        assert_eq!(mark_spans("abc", &[]), vec![("abc".into(), false)]);
    }
}

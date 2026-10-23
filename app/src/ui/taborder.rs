//! Order 116: the top row in the user's own order (right-click an icon > "Rearrange tabs", drag; "Reset order"). The order
//! is ONE app setting (`settings.cfg`, app section, `tab_order`: the page ids in order, the hidden add-on tabs too); no
//! setting = the built-in order (`pages::all()`). A tab a later version adds is not in the saved list: it slots in at its
//! built-in place - right after the tab that comes before it in the built-in order, wherever the user moved that one.

/// The app setting holding the order.
pub const KEY: &str = "tab_order";

/// The tabs in the user's order: the saved ids that still exist (in their saved order, each once), then every built-in id
/// that is not saved put right after its built-in predecessor (first when it has none).
pub fn ordered(default: &[&str], saved: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(default.len());
    for id in saved {
        if default.contains(&id.as_str()) && !out.contains(id) {
            out.push(id.clone());
        }
    }
    for (k, id) in default.iter().enumerate() {
        if out.iter().any(|o| o == id) {
            continue;
        }
        // right after its built-in predecessor (which is in `out` by now: saved, or new and placed before it)
        let at = match k.checked_sub(1) {
            Some(p) => out.iter().position(|o| o == default[p]).map_or(out.len(), |i| i + 1),
            None => 0,
        };
        out.insert(at, id.to_string());
    }
    out
}

/// The full order (`full`, hidden tabs too) after the shown tabs were put in the order `shown`: each hidden tab stays right
/// after the shown tab it came after (first when none came before it).
pub fn merge_visible(full: &[String], shown: &[&str]) -> Vec<String> {
    // each hidden tab, with the shown tab it came after (None = before every shown tab)
    let mut after: Vec<(Option<&str>, &String)> = Vec::new();
    let mut last: Option<&str> = None;
    for id in full {
        if shown.contains(&id.as_str()) {
            last = Some(id.as_str());
        } else {
            after.push((last, id));
        }
    }
    let mut out: Vec<String> = after.iter().filter(|a| a.0.is_none()).map(|a| a.1.clone()).collect();
    for s in shown {
        out.push(s.to_string());
        out.extend(after.iter().filter(|a| a.0 == Some(*s)).map(|a| a.1.clone()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn no_saved_order_is_the_built_in_order() {
        assert_eq!(ordered(&["aud", "dsp", "shot"], &[]), s(&["aud", "dsp", "shot"]));
    }

    #[test]
    fn a_saved_order_is_used() {
        assert_eq!(ordered(&["aud", "dsp", "shot", "cur"], &s(&["cur", "aud", "shot", "dsp"])), s(&["cur", "aud", "shot", "dsp"]));
    }

    /// A later version's new tab (here "nts", built in after "shot") lands right after "shot" - wherever "shot" was moved.
    #[test]
    fn a_new_tab_slots_in_after_its_built_in_neighbour() {
        let saved = s(&["shot", "cur", "aud", "dsp"]);
        assert_eq!(ordered(&["aud", "dsp", "shot", "nts", "cur"], &saved), s(&["shot", "nts", "cur", "aud", "dsp"]));
        // the first built-in tab is new: it goes first
        assert_eq!(ordered(&["new", "aud", "dsp"], &s(&["dsp", "aud"])), s(&["new", "dsp", "aud"]));
        // two new ones in a row keep their built-in order
        assert_eq!(ordered(&["aud", "a1", "a2", "dsp"], &s(&["dsp", "aud"])), s(&["dsp", "aud", "a1", "a2"]));
    }

    /// The hidden add-on tab ("ntf", shown only while Notifications for OBS is on) keeps its place after "act".
    #[test]
    fn hidden_tabs_stay_after_the_tab_they_came_after() {
        let full = s(&["aud", "act", "ntf", "add", "set"]);
        assert_eq!(merge_visible(&full, &["act", "aud", "set", "add"]), s(&["act", "ntf", "aud", "set", "add"]));
        // hidden at the very start stays first
        let full = s(&["ntf", "aud", "dsp"]);
        assert_eq!(merge_visible(&full, &["dsp", "aud"]), s(&["ntf", "dsp", "aud"]));
        // nothing hidden: just the new order
        assert_eq!(merge_visible(&s(&["aud", "dsp"]), &["dsp", "aud"]), s(&["dsp", "aud"]));
    }

    #[test]
    fn tabs_that_no_longer_exist_and_repeats_are_dropped() {
        assert_eq!(ordered(&["aud", "dsp"], &s(&["old", "dsp", "dsp", "aud"])), s(&["dsp", "aud"]));
    }
}

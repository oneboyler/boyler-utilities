//! The compatibility-layer string of one exe under `AppCompatFlags\Layers` (e.g. `~ RUNASADMIN DISABLEDXMAXIMIZEDWINDOWEDMODE`).
//! We add or remove only our flag and keep every other flag (e.g. "Run as administrator" set by the user).

/// The flag that turns fullscreen optimizations off for an exe (Properties › Compatibility › "Disable fullscreen optimizations").
pub const FLAG: &str = "DISABLEDXMAXIMIZEDWINDOWEDMODE";

/// Markers Windows puts in front of the flags (`~` = the entry was set by the user, `$` and `#` other origins).
fn is_marker(token: &str) -> bool {
    matches!(token, "~" | "$" | "#")
}

/// Is our flag in this layer string?
pub fn has_flag(layers: &str) -> bool {
    layers.split_whitespace().any(|t| t.eq_ignore_ascii_case(FLAG))
}

/// The layer string with our flag added. A new string starts with `~ ` like the Properties dialog writes it.
pub fn add_flag(layers: &str) -> String {
    if has_flag(layers) {
        return layers.to_string();
    }
    let trimmed = layers.trim();
    if trimmed.is_empty() {
        format!("~ {FLAG}")
    } else {
        format!("{trimmed} {FLAG}")
    }
}

/// The layer string with our flag removed; `None` when nothing but markers is left (= delete the value).
pub fn remove_flag(layers: &str) -> Option<String> {
    let tokens: Vec<&str> = layers.split_whitespace().filter(|t| !t.eq_ignore_ascii_case(FLAG)).collect();
    if tokens.iter().all(|t| is_marker(t)) {
        None
    } else {
        Some(tokens.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_remove_keep_other_flags() {
        assert_eq!(add_flag(""), "~ DISABLEDXMAXIMIZEDWINDOWEDMODE");
        assert_eq!(add_flag("~ RUNASADMIN"), "~ RUNASADMIN DISABLEDXMAXIMIZEDWINDOWEDMODE");
        assert_eq!(add_flag("~ DISABLEDXMAXIMIZEDWINDOWEDMODE"), "~ DISABLEDXMAXIMIZEDWINDOWEDMODE");
        assert_eq!(remove_flag("~ RUNASADMIN DISABLEDXMAXIMIZEDWINDOWEDMODE").as_deref(), Some("~ RUNASADMIN"));
        assert_eq!(remove_flag("~ DISABLEDXMAXIMIZEDWINDOWEDMODE"), None);
        assert_eq!(remove_flag("$ ~ disabledxmaximizedwindowedmode"), None);
        assert!(has_flag("~ HIGHDPIAWARE DISABLEDXMAXIMIZEDWINDOWEDMODE"));
        assert!(!has_flag("~ HIGHDPIAWARE"));
    }
}

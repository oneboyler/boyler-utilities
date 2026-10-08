//! Editing one entry of `DirectXUserGlobalSettings` (a `key=value;` list) while keeping every other entry exactly as it was
//! (research ideas-v3.md §5: "change only our key and keep the rest").

/// The value of `key` in the list, if present.
pub fn get<'a>(list: &'a str, key: &str) -> Option<&'a str> {
    list.split(';').find_map(|part| {
        let (k, v) = part.split_once('=')?;
        (k.trim() == key).then(|| v.trim())
    })
}

/// The list with `key=value;` set (replaced in place, or appended at the end). Other entries keep their order and text.
pub fn set(list: &str, key: &str, value: &str) -> String {
    let mut out = String::new();
    let mut found = false;
    for part in list.split(';').filter(|p| !p.trim().is_empty()) {
        let is_key = part.split_once('=').map(|(k, _)| k.trim() == key).unwrap_or(false);
        if is_key {
            if !found {
                out.push_str(&format!("{key}={value};"));
                found = true;
            }
        } else {
            out.push_str(part);
            out.push(';');
        }
    }
    if !found {
        out.push_str(&format!("{key}={value};"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_finds_key() {
        assert_eq!(get("SwapEffectUpgradeEnable=1;VRROptimizeEnable=0;", "VRROptimizeEnable"), Some("0"));
        assert_eq!(get("SwapEffectUpgradeEnable=1;", "AutoHDREnable"), None);
        assert_eq!(get("", "AutoHDREnable"), None);
    }

    #[test]
    fn set_keeps_others_in_order() {
        assert_eq!(set("A=1;B=2;C=3;", "B", "0"), "A=1;B=0;C=3;");
        assert_eq!(set("A=1;", "B", "1"), "A=1;B=1;");
        assert_eq!(set("", "B", "1"), "B=1;");
        // no trailing ';' and a duplicate key: one entry stays
        assert_eq!(set("A=1;B=2;B=5", "B", "1"), "A=1;B=1;");
    }
}

//! OBS's text files, read and edited the way ClipPing does (base.c `ini_get`, obsctl.c `ini_text_set` / `json_text_set`):
//! an edit changes only the one value and keeps every other byte of the file as it was.

/// A value from UTF-8 ini text (OBS's global.ini / user.ini / basic.ini). The first `[section]` line and `key=` match
/// win; spaces / tabs around the `=` are skipped; a UTF-8 BOM is ignored.
pub fn get(text: &str, section: &str, key: &str) -> Option<String> {
    let t = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut inside = false;
    for raw in t.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let line = line.trim_start_matches([' ', '\t']);
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let name = rest.split(']').next().unwrap_or("");
            inside = name == section;
            continue;
        }
        if inside && line.len() > key.len() && line.starts_with(key) {
            let after = line[key.len()..].trim_start_matches([' ', '\t']);
            if let Some(v) = after.strip_prefix('=') {
                return Some(v.trim_start_matches([' ', '\t']).to_string());
            }
        }
    }
    None
}

/// An int value (`def` when missing or empty; leading digits like C's atoi-style `s_to_i64`).
pub fn get_int(text: &str, section: &str, key: &str, def: i64) -> i64 {
    match get(text, section, key) {
        Some(v) if !v.is_empty() => atoi(&v),
        _ => def,
    }
}

/// ClipPing's `s_to_i64`: optional sign, then digits; anything else stops it.
pub fn atoi(s: &str) -> i64 {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    let neg = i < b.len() && b[i] == b'-';
    if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
        i += 1;
    }
    let mut v: i64 = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        v = v.wrapping_mul(10).wrapping_add((b[i] - b'0') as i64);
        i += 1;
    }
    if neg {
        -v
    } else {
        v
    }
}

/// Is the value (case-insensitive) `want`? Missing = `def`.
pub fn is(text: &str, section: &str, key: &str, want: &str, def: bool) -> bool {
    match get(text, section, key) {
        Some(v) => v.eq_ignore_ascii_case(want),
        None => def,
    }
}

/// Set `key=value` in `[section]`, keeping everything else byte for byte (obsctl.c `ini_text_set`): an existing key's value
/// is replaced; a missing key is added after the section's last non-empty line; a missing section is added at the end.
/// The file's own line ending (\r\n or \n) is used.
pub fn set(text: &mut String, section: &str, key: &str, value: &str) {
    let t = text.as_bytes();
    let n = t.len();
    let mut eol = "\n";
    if let Some(p) = t.iter().position(|&c| c == b'\n') {
        if p > 0 && t[p - 1] == b'\r' {
            eol = "\r\n";
        }
    }
    let (kl, sl) = (key.len(), section.len());
    let (mut i, mut inside, mut after): (usize, bool, Option<usize>) = (0, false, None);
    while i < n {
        let ls = i;
        while i < n && t[i] != b'\n' {
            i += 1;
        }
        let le = i;
        if i < n {
            i += 1;
        }
        let mut ce = le;
        if ce > ls && t[ce - 1] == b'\r' {
            ce -= 1;
        }
        let mut p = ls;
        if ls == 0 && ce >= 3 && t[0] == 0xEF && t[1] == 0xBB && t[2] == 0xBF {
            p = 3;
        }
        while p < ce && (t[p] == b' ' || t[p] == b'\t') {
            p += 1;
        }
        if p < ce && t[p] == b'[' {
            if inside {
                break;
            }
            inside = ce - p >= sl + 2 && &t[p + 1..p + 1 + sl] == section.as_bytes() && t[p + 1 + sl] == b']';
            if inside {
                after = Some(i);
            }
            continue;
        }
        if !inside {
            continue;
        }
        if p < ce {
            after = Some(i);
        }
        if ce - p > kl && &t[p..p + kl] == key.as_bytes() {
            let mut q = p + kl;
            while q < ce && (t[q] == b' ' || t[q] == b'\t') {
                q += 1;
            }
            if q < ce && t[q] == b'=' {
                text.replace_range(q + 1..ce, value);
                return;
            }
        }
    }
    let mut ins = String::new();
    match after {
        Some(a) => {
            if a == n && n > 0 && t[n - 1] != b'\n' {
                ins.push_str(eol);
            }
            ins.push_str(&format!("{key}={value}{eol}"));
            text.insert_str(a, &ins);
        }
        None => {
            if n > 0 && t[n - 1] != b'\n' {
                ins.push_str(eol);
            }
            if n > 0 {
                ins.push_str(eol);
            }
            ins.push_str(&format!("[{section}]{eol}{key}={value}{eol}"));
            text.push_str(&ins);
        }
    }
}

fn jws(c: u8) -> bool {
    c == b' ' || c == b'\t' || c == b'\r' || c == b'\n'
}

/// Set one top-level value of a flat JSON object's text to `raw` (already JSON: `true`, `"abc"`), keeping everything else
/// as it is (obsctl.c `json_text_set`); a missing key is added before the closing brace. False = the text has no object.
pub fn json_set(text: &mut String, key: &str, raw: &str) -> bool {
    if key.len() > 60 {
        return false;
    }
    let q = format!("\"{key}\"");
    let t = text.as_bytes().to_vec();
    let n = t.len();
    let kl = q.len();
    let mut i = 0;
    while i + kl <= n {
        if &t[i..i + kl] != q.as_bytes() {
            i += 1;
            continue;
        }
        let mut v = i + kl;
        while v < n && jws(t[v]) {
            v += 1;
        }
        if v >= n || t[v] != b':' {
            i += 1;
            continue;
        }
        v += 1;
        while v < n && jws(t[v]) {
            v += 1;
        }
        let mut e = v;
        if e < n && t[e] == b'"' {
            e += 1;
            while e < n && t[e] != b'"' {
                e += if t[e] == b'\\' { 2 } else { 1 };
            }
            e += 1;
        } else {
            while e < n && t[e] != b',' && t[e] != b'}' && !jws(t[e]) {
                e += 1;
            }
        }
        if e > n {
            return false;
        }
        text.replace_range(v..e, raw);
        return true;
    }
    let Some(e) = t.iter().rposition(|&c| c == b'}') else { return false };
    if e == 0 {
        return false;
    }
    let mut j = e - 1;
    while j > 0 && jws(t[j]) {
        j -= 1;
    }
    let lead = if t[j] == b'{' { "\n    \"" } else { ",\n    \"" };
    text.replace_range(j + 1..e, &format!("{lead}{key}\": {raw}\n"));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_reads_sections_keys_and_bom() {
        let t = "\u{feff}[General]\r\nName = x\r\n[Video]\nFPSType=1\n  FPSInt = 60\n";
        assert_eq!(get(t, "Video", "FPSInt").as_deref(), Some("60"));
        assert_eq!(get(t, "General", "Name").as_deref(), Some("x"));
        assert_eq!(get(t, "Video", "Name"), None);
        assert_eq!(get_int(t, "Video", "FPSType", 0), 1);
        assert_eq!(atoi("29.97"), 29);
    }

    #[test]
    fn set_keeps_every_other_byte() {
        let mut t = String::from("[Output]\r\nMode=Simple\r\n\r\n[SimpleOutput]\r\nRecRBTime=20\r\nFilePath=C:/x\r\n");
        set(&mut t, "SimpleOutput", "RecRBTime", "60");
        assert_eq!(t, "[Output]\r\nMode=Simple\r\n\r\n[SimpleOutput]\r\nRecRBTime=60\r\nFilePath=C:/x\r\n");
        set(&mut t, "Output", "New", "1");
        assert_eq!(t, "[Output]\r\nMode=Simple\r\nNew=1\r\n\r\n[SimpleOutput]\r\nRecRBTime=60\r\nFilePath=C:/x\r\n");
        set(&mut t, "Hotkeys", "A", "{}");
        assert!(t.ends_with("FilePath=C:/x\r\n\r\n[Hotkeys]\r\nA={}\r\n"));
    }

    #[test]
    fn json_set_replaces_and_adds() {
        let mut t = String::from("{\n    \"auth_required\": false,\n    \"server_password\": \"ab\\\"c\",\n    \"server_port\": 4455\n}\n");
        assert!(json_set(&mut t, "auth_required", "true"));
        assert!(json_set(&mut t, "server_password", "\"NEW\""));
        assert!(json_set(&mut t, "server_enabled", "true"));
        assert_eq!(
            t,
            "{\n    \"auth_required\": true,\n    \"server_password\": \"NEW\",\n    \"server_port\": 4455,\n    \"server_enabled\": true\n}\n"
        );
        let mut e = String::from("{}");
        assert!(json_set(&mut e, "a", "1"));
        assert_eq!(e, "{\n    \"a\": 1\n}");
    }
}

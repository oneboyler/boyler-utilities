//! Steam's BINARY KeyValues (only `shortcuts.vdf`, read-only): type byte, NUL-terminated key, value.
//! Types: 0x00 = block, 0x01 = string, 0x02 = 32-bit int, 0x07 = 64-bit int, 0x08 = end of block (measured on a real file:
//! `00 "shortcuts" 00 "0" 02 "appid" <4 bytes> 01 "AppName" "Epic Games Launcher" …`).

/// Every `AppName` (also the older `appname`) in the file, in order. A broken file gives what was read before the break.
pub fn app_names(b: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0usize;
    let cstr = |i: &mut usize| -> Option<String> {
        let start = *i;
        let end = start + b.get(start..)?.iter().position(|c| *c == 0)?;
        *i = end + 1;
        Some(String::from_utf8_lossy(&b[start..end]).into_owned())
    };
    while i < b.len() {
        let t = b[i];
        i += 1;
        match t {
            0x08 => continue,
            0x00 => {
                if cstr(&mut i).is_none() {
                    break;
                }
            }
            0x01 => {
                let (Some(k), Some(v)) = (cstr(&mut i), cstr(&mut i)) else { break };
                if k.eq_ignore_ascii_case("appname") {
                    out.push(v);
                }
            }
            0x02 | 0x04 | 0x06 => {
                if cstr(&mut i).is_none() {
                    break;
                }
                i += 4;
            }
            0x07 => {
                if cstr(&mut i).is_none() {
                    break;
                }
                i += 8;
            }
            _ => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_app_names() {
        let mut b = vec![0u8];
        b.extend(b"shortcuts\0");
        b.push(0);
        b.extend(b"0\0");
        b.push(2);
        b.extend(b"appid\0");
        b.extend([1, 2, 3, 4]);
        b.push(1);
        b.extend(b"AppName\0Epic Games Launcher\0");
        b.push(1);
        b.extend(b"Exe\0\"C:\\x.exe\"\0");
        b.push(8);
        b.push(0);
        b.extend(b"1\0");
        b.push(1);
        b.extend(b"appname\0Other\0");
        b.extend([8, 8, 8]);
        assert_eq!(super::app_names(&b), vec!["Epic Games Launcher".to_string(), "Other".to_string()]);
        assert!(super::app_names(&[1, b'x']).is_empty());
    }
}

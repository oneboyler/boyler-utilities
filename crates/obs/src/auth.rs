//! obs-websocket v5 authentication (obsws.c `obs_auth`): secret = base64(sha256(password + salt)); auth =
//! base64(sha256(secret + challenge)).

use crate::sha256::Sha256;

/// Standard base64 with padding (obsws.c `b64_encode`).
pub fn b64(d: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(d.len().div_ceil(3) * 4);
    for c in d.chunks(3) {
        let b0 = c[0] as usize;
        let b1 = *c.get(1).unwrap_or(&0) as usize;
        let b2 = *c.get(2).unwrap_or(&0) as usize;
        out.push(T[b0 >> 2] as char);
        out.push(T[((b0 & 3) << 4) | (b1 >> 4)] as char);
        out.push(if c.len() > 1 { T[((b1 & 15) << 2) | (b2 >> 6)] as char } else { '=' });
        out.push(if c.len() > 2 { T[b2 & 63] as char } else { '=' });
    }
    out
}

fn sha(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize()
}

/// The `authentication` string for an Identify message.
pub fn obs_auth(password: &str, salt: &str, challenge: &str) -> String {
    let secret = b64(&sha(&[password.as_bytes(), salt.as_bytes()]));
    b64(&sha(&[secret.as_bytes(), challenge.as_bytes()]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_rfc4648_vectors() {
        for (i, o) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(b64(i.as_bytes()), o);
        }
    }

    /// obs-websocket's protocol document example: password "supersecretpassword", salt "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=",
    /// challenge "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY=" -> "1Ct943GAT+6YQUUX47Ia/ncufilbe6+oD6lY+5kaCu4=".
    #[test]
    fn protocol_document_example() {
        assert_eq!(
            obs_auth("supersecretpassword", "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=", "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY="),
            "1Ct943GAT+6YQUUX47Ia/ncufilbe6+oD6lY+5kaCu4="
        );
    }
}

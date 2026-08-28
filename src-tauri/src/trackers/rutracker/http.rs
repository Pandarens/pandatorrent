//! Encoding helpers for RuTracker URLs.
//!
//! The tracker is a windows-1251 site: a Cyrillic search term has to be
//! percent-encoded from cp1251 bytes, not UTF-8, or it comes back as mojibake.
//! The requests themselves go through [`super::browser`], but the query string
//! is still built here.

use encoding_rs::WINDOWS_1251;

/// Percent-encodes a string after transcoding it to windows-1251, the way the
/// tracker's own forms do.
pub fn percent_encode_cp1251(s: &str) -> String {
    let (encoded, _, _) = WINDOWS_1251.encode(s);
    let mut out = String::with_capacity(encoded.len());
    for &b in encoded.iter() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Decodes windows-1251, falling back to UTF-8 when the bytes are valid UTF-8.
///
/// Used for `.torrent` payloads that turn out to be an HTML error page.
pub fn decode_cp1251(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        // Cyrillic cp1251 is never valid multi-byte UTF-8, so valid UTF-8 here
        // means the server really did send UTF-8.
        return s.to_string();
    }
    WINDOWS_1251.decode(bytes).0.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cp1251_roundtrip_for_cyrillic_query() {
        // "игра" in windows-1251 is E8 E3 F0 E0.
        assert_eq!(percent_encode_cp1251("игра"), "%E8%E3%F0%E0");
    }

    #[test]
    fn spaces_become_plus() {
        assert_eq!(percent_encode_cp1251("half life"), "half+life");
    }

    #[test]
    fn decodes_cp1251_bytes() {
        assert_eq!(decode_cp1251(&[0xE8, 0xE3, 0xF0, 0xE0]), "игра");
    }
}

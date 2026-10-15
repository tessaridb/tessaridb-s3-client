//! SigV4's own percent-encoding: every byte except `A-Z a-z 0-9 - . _ ~` becomes `%XX` with uppercase hex, a space is
//! `%20` and never `+`, and `/` is kept only inside a path. A platform URL encoder differs on exactly these points.

/// `text` percent-encoded the SigV4 way; `/` is encoded too when `encode_slash`.
pub(crate) fn uri_encode(text: &str, encode_slash: bool) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        let kept = byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'.' | b'_' | b'~')
            || (byte == b'/' && !encode_slash);
        if kept {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    out
}

/// The canonical URI of an unencoded absolute path: encoded once, `/` kept; an empty path is `/`.
pub(crate) fn canonical_uri(path: &str) -> String {
    if path.is_empty() {
        "/".to_owned()
    } else {
        uri_encode(path, false)
    }
}

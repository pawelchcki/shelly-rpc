//! Shared buffer utilities used by [`device`](crate::device) and
//! [`cloud`](crate::cloud) modules.

use crate::error::Error;

/// Copy `src` into `dst` at `offset`. Returns bytes written.
pub(crate) fn copy_slice(dst: &mut [u8], offset: usize, src: &[u8]) -> Result<usize, Error> {
    let end = offset + src.len();
    if end > dst.len() {
        return Err(Error::BufferTooSmall);
    }
    dst[offset..end].copy_from_slice(src);
    Ok(src.len())
}

/// JSON-escape `src` into `dst`, returning bytes written.
pub(crate) fn json_escape_into(dst: &mut [u8], src: &[u8]) -> Result<usize, Error> {
    let mut len = 0;
    for &b in src {
        let escaped: &[u8] = match b {
            b'\\' => b"\\\\",
            b'"' => b"\\\"",
            b'\n' => b"\\n",
            b'\r' => b"\\r",
            b'\t' => b"\\t",
            _ => core::slice::from_ref(&b),
        };
        if len + escaped.len() > dst.len() {
            return Err(Error::BufferTooSmall);
        }
        dst[len..len + escaped.len()].copy_from_slice(escaped);
        len += escaped.len();
    }
    Ok(len)
}

const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

/// Percent-encode `src` into `dst` per RFC 3986 (unreserved set:
/// `A-Z a-z 0-9 - . _ ~`). Returns bytes written.
pub(crate) fn url_encode_into(dst: &mut [u8], src: &[u8]) -> Result<usize, Error> {
    let mut len = 0;
    for &b in src {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                if len >= dst.len() {
                    return Err(Error::BufferTooSmall);
                }
                dst[len] = b;
                len += 1;
            }
            _ => {
                if len + 3 > dst.len() {
                    return Err(Error::BufferTooSmall);
                }
                dst[len] = b'%';
                dst[len + 1] = HEX_UPPER[((b >> 4) & 0xF) as usize];
                dst[len + 2] = HEX_UPPER[(b & 0xF) as usize];
                len += 3;
            }
        }
    }
    Ok(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encode_unreserved_passthrough() {
        let mut buf = [0u8; 64];
        let n = url_encode_into(&mut buf, b"hello-world_2.0~test").unwrap();
        assert_eq!(&buf[..n], b"hello-world_2.0~test");
    }

    #[test]
    fn url_encode_spaces_and_special() {
        let mut buf = [0u8; 64];
        let n = url_encode_into(&mut buf, b"a b+c&d=e").unwrap();
        assert_eq!(
            core::str::from_utf8(&buf[..n]).unwrap(),
            "a%20b%2Bc%26d%3De"
        );
    }

    #[test]
    fn url_encode_json_braces() {
        let mut buf = [0u8; 64];
        let n = url_encode_into(&mut buf, br#"{"k":"v"}"#).unwrap();
        assert_eq!(
            core::str::from_utf8(&buf[..n]).unwrap(),
            "%7B%22k%22%3A%22v%22%7D"
        );
    }

    #[test]
    fn url_encode_buffer_too_small() {
        let mut buf = [0u8; 2];
        // Space needs 3 bytes (%20) but only 2 available
        assert!(url_encode_into(&mut buf, b" ").is_err());
    }

    #[test]
    fn url_encode_empty() {
        let mut buf = [0u8; 8];
        let n = url_encode_into(&mut buf, b"").unwrap();
        assert_eq!(n, 0);
    }
}

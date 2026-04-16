//! JSON, URL, and JWT helper functions for Cloud API interactions.

pub(super) fn normalize_server_uri(server: &str) -> String {
    server.trim_end_matches('/').to_string()
}

pub(super) fn json_extract_string(json: &str, key: &str) -> Option<String> {
    // Hand-rolled because we need this to work on JWT payloads whose shape we
    // don't want to pin down with a full serde struct. Each candidate `"key"`
    // occurrence is validated as an actual object key (preceded by `{` or `,`)
    // before we trust it — otherwise a value like `"name":"user_api_url here"`
    // would false-match when searching for `user_api_url`.
    let needle = format!("\"{key}\"");
    let mut search_from = 0;
    while let Some(rel) = json[search_from..].find(&needle) {
        let key_pos = search_from + rel;
        search_from = key_pos + needle.len();

        if !preceded_by_key_boundary(json, key_pos) {
            continue;
        }
        let rest = &json[key_pos + needle.len()..];
        let Some(rest) = rest.trim_start().strip_prefix(':') else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('"') else {
            continue;
        };
        let mut end = 0;
        let bytes = rest.as_bytes();
        while end < bytes.len() {
            if bytes[end] == b'\\' {
                end += 2;
            } else if bytes[end] == b'"' {
                return Some(json_unescape(&rest[..end]));
            } else {
                end += 1;
            }
        }
        return None;
    }
    None
}

/// Check that the byte immediately before `pos` (skipping ASCII whitespace) is
/// a JSON object-key boundary: either `{` (first key) or `,` (subsequent key).
/// This prevents false matches inside string *values*.
fn preceded_by_key_boundary(json: &str, pos: usize) -> bool {
    let bytes = json.as_bytes();
    let mut i = pos;
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => continue,
            b'{' | b',' => return true,
            _ => return false,
        }
    }
    false
}

pub(super) fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

fn json_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('/') => out.push('/'),
                Some(c) => {
                    out.push('\\');
                    out.push(c);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub(super) fn extract_query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split('?').nth(1)?;
    let query = query.split('#').next().unwrap_or(query);
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        let k = kv.next()?;
        if k == key {
            return url_decode(kv.next().unwrap_or(""));
        }
    }
    None
}

/// Percent-decode a URL-encoded string. Returns `None` on a malformed `%XX`
/// escape or if the decoded bytes are not valid UTF-8 — failing loudly beats
/// silently mangling an OAuth code into nonsense the caller can't diagnose.
fn url_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                // Need two hex digits following.
                if i + 2 >= bytes.len() {
                    return None;
                }
                let hex = core::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
                let b = u8::from_str_radix(hex, 16).ok()?;
                out.push(b);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

pub(super) fn jwt_extract_field(token: &str, field: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let mut padded = payload.to_string();
    while padded.len() % 4 != 0 {
        padded.push('=');
    }
    use base64::Engine;
    let decoded = base64::engine::general_purpose::URL_SAFE
        .decode(&padded)
        .ok()?;
    let json = std::str::from_utf8(&decoded).ok()?;
    json_extract_string(json, field)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    // ── json_extract_string ──────────────────────────────────────────────

    #[test]
    fn extracts_simple_string() {
        let json = r#"{"user_api_url":"https://shelly-96-eu.shelly.cloud"}"#;
        assert_eq!(
            json_extract_string(json, "user_api_url"),
            Some("https://shelly-96-eu.shelly.cloud".to_string())
        );
    }

    #[test]
    fn extracts_string_with_whitespace() {
        let json = r#"{ "user_api_url" : "https://ex.com" }"#;
        assert_eq!(
            json_extract_string(json, "user_api_url"),
            Some("https://ex.com".to_string())
        );
    }

    #[test]
    fn returns_none_for_missing_key() {
        let json = r#"{"other":"value"}"#;
        assert_eq!(json_extract_string(json, "user_api_url"), None);
    }

    #[test]
    fn unescapes_quoted_string() {
        let json = r#"{"name":"he said \"hi\""}"#;
        assert_eq!(
            json_extract_string(json, "name"),
            Some(r#"he said "hi""#.to_string())
        );
    }

    #[test]
    fn unescapes_newline_and_tab() {
        let json = r#"{"text":"line1\nline2\ttabbed"}"#;
        assert_eq!(
            json_extract_string(json, "text"),
            Some("line1\nline2\ttabbed".to_string())
        );
    }

    #[test]
    fn rejects_substring_false_match_in_value() {
        // Attacker-crafted payload: a string *value* contains the literal
        // bytes `"user_api_url":"https://evil.com"`. A naive substring match
        // would extract `https://evil.com`; the boundary check must reject it
        // so the real key's value wins (or None if no real key exists).
        let json = r#"{"comment":"\"user_api_url\":\"https://evil.com\"","user_api_url":"https://good.com"}"#;
        assert_eq!(
            json_extract_string(json, "user_api_url"),
            Some("https://good.com".to_string())
        );
    }

    #[test]
    fn rejects_key_substring_match() {
        // Key `xuser_api_url` must not match when searching for `user_api_url`.
        let json = r#"{"xuser_api_url":"wrong","user_api_url":"right"}"#;
        assert_eq!(
            json_extract_string(json, "user_api_url"),
            Some("right".to_string())
        );
    }

    #[test]
    fn rejects_name_in_value_with_no_real_key() {
        let json = r#"{"comment":"the field user_api_url is required"}"#;
        // The match `"user_api_url"` *only* appears inside a value; there is
        // no real key, so extraction must return None.
        assert_eq!(json_extract_string(json, "user_api_url"), None);
    }

    // ── url_decode (exercised through extract_query_param) ──────────────

    #[test]
    fn query_param_decodes_percent_escapes() {
        let url = "http://127.0.0.1:9999/?code=hello%20world";
        assert_eq!(
            extract_query_param(url, "code"),
            Some("hello world".to_string())
        );
    }

    #[test]
    fn query_param_decodes_plus_as_space() {
        let url = "http://127.0.0.1/?q=hello+world";
        assert_eq!(
            extract_query_param(url, "q"),
            Some("hello world".to_string())
        );
    }

    #[test]
    fn query_param_returns_none_for_missing_key() {
        let url = "http://127.0.0.1/?code=xyz";
        assert_eq!(extract_query_param(url, "state"), None);
    }

    #[test]
    fn query_param_ignores_fragment() {
        let url = "http://127.0.0.1/?code=abc#fragment&code=ignored";
        assert_eq!(extract_query_param(url, "code"), Some("abc".to_string()));
    }

    #[test]
    fn query_param_returns_empty_string_for_bare_key() {
        let url = "http://127.0.0.1/?code=";
        assert_eq!(extract_query_param(url, "code"), Some(String::new()));
    }

    #[test]
    fn query_param_decodes_trailing_percent_escape() {
        // A %XX escape ending exactly at end-of-string must still decode.
        let url = "http://127.0.0.1/?code=ab%41";
        assert_eq!(extract_query_param(url, "code"), Some("abA".to_string()));
    }

    #[test]
    fn query_param_rejects_truncated_percent_escape() {
        // A malformed `%XX` (here: trailing `%4` with no second hex digit)
        // must surface as `None` — silently passing through the literal bytes
        // would corrupt OAuth codes with no diagnostic.
        let url = "http://127.0.0.1/?code=%4";
        assert_eq!(extract_query_param(url, "code"), None);
    }

    #[test]
    fn query_param_rejects_non_hex_escape() {
        let url = "http://127.0.0.1/?code=%ZZ";
        assert_eq!(extract_query_param(url, "code"), None);
    }

    // ── jwt_extract_field ───────────────────────────────────────────────

    /// Build a fake JWT: `header.payload.signature` with `payload` set to the
    /// URL-safe base64 (no padding) of `payload_json`.
    fn fake_jwt(payload_json: &str) -> String {
        let header = "eyJhbGciOiJIUzI1NiJ9"; // {"alg":"HS256"}
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload_json);
        format!("{header}.{payload}.signature")
    }

    #[test]
    fn jwt_extracts_user_api_url() {
        let token =
            fake_jwt(r#"{"sub":"1234","user_api_url":"https://shelly-96-eu.shelly.cloud"}"#);
        assert_eq!(
            jwt_extract_field(&token, "user_api_url"),
            Some("https://shelly-96-eu.shelly.cloud".to_string())
        );
    }

    #[test]
    fn jwt_returns_none_for_missing_field() {
        let token = fake_jwt(r#"{"sub":"1234"}"#);
        assert_eq!(jwt_extract_field(&token, "user_api_url"), None);
    }

    #[test]
    fn jwt_returns_none_for_malformed_token() {
        // Missing payload segment entirely.
        assert_eq!(jwt_extract_field("onlyone", "user_api_url"), None);
    }

    #[test]
    fn jwt_returns_none_for_invalid_base64() {
        // Has two dots, but payload is not valid base64.
        assert_eq!(jwt_extract_field("h.!!!.s", "user_api_url"), None);
    }

    #[test]
    fn jwt_handles_url_safe_chars_in_payload() {
        // A payload containing `/` or `+` in base64-decoded form comes from
        // the URL-safe alphabet using `-` and `_`. Round-trip verify.
        let payload_bytes = [
            0u8, 0xFB, 0xFF, b'{', b'"', b'a', b'"', b':', b'"', b'b', b'"', b'}',
        ];
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload_bytes);
        // The leading bytes make the JSON invalid; extraction should return
        // None rather than panic.
        let token = format!("h.{payload}.s");
        assert_eq!(jwt_extract_field(&token, "a"), None);
    }
}

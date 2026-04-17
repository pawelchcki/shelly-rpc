//! Conservative minifier for mJS (Mongoose JavaScript) source.
//!
//! On-device scripts must fit in ~2KB, which rewards keeping verbose,
//! commented source in the repo and "compiling" it down before upload.
//! This minifier is deliberately cautious: it strips comments, trims
//! whitespace around punctuation, and preserves string/template literals
//! verbatim. It does not rename identifiers or rewrite expressions, so the
//! output is still recognizable when debugging on-device.
//!
//! Regex literals are not detected — scripts that need a regex should
//! keep it on its own line or inside a string. The mJS scripts this ships
//! with don't use regex literals.

/// Minify a JS source string.
///
/// Returns the minified source. Never fails; malformed input is passed
/// through as faithfully as possible.
pub fn minify(source: &str) -> String {
    let stripped = strip_comments(source);
    collapse_whitespace(&stripped)
}

/// Strip `//` and `/* */` comments. Contents of string and template
/// literals are preserved verbatim (including any `//` or `/*` inside).
fn strip_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i];

        if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        if c == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            // Replace the comment with a space so adjacent tokens don't glue.
            out.push(b' ');
            continue;
        }

        if matches!(c, b'"' | b'\'' | b'`') {
            let quote = c;
            out.push(c);
            i += 1;
            while i < bytes.len() {
                let sc = bytes[i];
                out.push(sc);
                i += 1;
                if sc == b'\\' {
                    if i < bytes.len() {
                        out.push(bytes[i]);
                        i += 1;
                    }
                } else if sc == quote {
                    break;
                }
            }
            continue;
        }

        out.push(c);
        i += 1;
    }

    // We only ever copy whole bytes from valid UTF-8 input and never split
    // multi-byte sequences (comment/string delimiters are all ASCII), so
    // the result is still valid UTF-8.
    String::from_utf8(out).expect("minifier preserves UTF-8 boundaries")
}

/// Collapse runs of whitespace. A single space is kept only when it
/// separates two identifier-like bytes (so `return x` stays separated
/// while `a + b` collapses to `a+b`).
fn collapse_whitespace(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i];

        if matches!(c, b'"' | b'\'' | b'`') {
            let quote = c;
            out.push(c);
            i += 1;
            while i < bytes.len() {
                let sc = bytes[i];
                out.push(sc);
                i += 1;
                if sc == b'\\' {
                    if i < bytes.len() {
                        out.push(bytes[i]);
                        i += 1;
                    }
                } else if sc == quote {
                    break;
                }
            }
            continue;
        }

        if is_ws(c) {
            let mut j = i;
            while j < bytes.len() && is_ws(bytes[j]) {
                j += 1;
            }
            let prev = out.last().copied();
            let next = bytes.get(j).copied();
            if let (Some(p), Some(n)) = (prev, next) {
                if needs_separator(p, n) {
                    out.push(b' ');
                }
            }
            i = j;
            continue;
        }

        out.push(c);
        i += 1;
    }

    String::from_utf8(out).expect("minifier preserves UTF-8 boundaries")
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C)
}

/// Bytes that can appear inside a JS identifier or numeric literal.
/// Any byte with the high bit set is assumed to be part of a multi-byte
/// UTF-8 identifier character — we keep separators in that case rather
/// than risk gluing a keyword onto a Unicode identifier.
fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

/// Decide whether two adjacent non-whitespace bytes need a space between
/// them to remain two distinct tokens.
fn needs_separator(prev: u8, next: u8) -> bool {
    if is_ident_byte(prev) && is_ident_byte(next) {
        return true;
    }
    // `+ +` must not become `++`, same for `-`. Mongoose JS tokens like
    // `++`/`--` are single tokens; re-combining would change semantics.
    if (prev == b'+' && next == b'+') || (prev == b'-' && next == b'-') {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_line_comment() {
        assert_eq!(
            minify("let x = 1; // a comment\nlet y = 2;"),
            "let x=1;let y=2;"
        );
    }

    #[test]
    fn strips_block_comment() {
        assert_eq!(minify("let /* inline */ x = 1;"), "let x=1;");
    }

    #[test]
    fn preserves_string_contents() {
        let src = r#"print("hello // not a comment /* nope */");"#;
        assert_eq!(
            minify(src),
            r#"print("hello // not a comment /* nope */");"#
        );
    }

    #[test]
    fn preserves_escaped_quotes() {
        let src = r#"let s = "she said \"hi\"";"#;
        assert_eq!(minify(src), r#"let s="she said \"hi\"";"#);
    }

    #[test]
    fn preserves_template_literals() {
        let src = "let s = `a // b ${x} c`;";
        assert_eq!(minify(src), "let s=`a // b ${x} c`;");
    }

    #[test]
    fn keeps_space_between_keyword_and_ident() {
        assert_eq!(minify("return x;"), "return x;");
        assert_eq!(minify("typeof  y"), "typeof y");
    }

    #[test]
    fn strips_space_around_operators() {
        assert_eq!(minify("a = b + c * d"), "a=b+c*d");
    }

    #[test]
    fn keeps_plus_plus_apart() {
        // `a+ +b` is `a + (+b)`, different from `a++b`.
        assert_eq!(minify("a + +b"), "a+ +b");
        assert_eq!(minify("a - -b"), "a- -b");
    }

    #[test]
    fn collapses_blank_lines() {
        let src = "let a = 1;\n\n\nlet b = 2;\n";
        assert_eq!(minify(src), "let a=1;let b=2;");
    }

    #[test]
    fn handles_unterminated_block_comment() {
        // Don't panic — consume to end.
        assert_eq!(minify("a /* dangling"), "a");
    }

    #[test]
    fn handles_unterminated_string() {
        let src = r#"let s = "oops"#;
        // Should not panic; copy the string bytes verbatim.
        let out = minify(src);
        assert!(out.contains("\"oops"));
    }

    #[test]
    fn preserves_non_ascii_identifier() {
        // If a user has an identifier with non-ASCII bytes, don't glue it
        // to an adjacent keyword.
        let src = "let α = 1; return α;";
        let out = minify(src);
        // We expect the α to be preserved and separated from `return`.
        assert!(out.contains("return α"));
    }

    #[test]
    fn empty_input() {
        assert_eq!(minify(""), "");
    }

    #[test]
    fn strips_comment_at_end_of_file_without_newline() {
        assert_eq!(minify("let x=1;//trailing"), "let x=1;");
    }

    #[test]
    fn block_comment_between_tokens_becomes_space() {
        // Ensures `a/*x*/b` doesn't become `ab`.
        assert_eq!(minify("a/*x*/b"), "a b");
    }
}

//! Typed wrappers over the Shelly Gen2+ JSON-RPC surface.
//!
//! Each submodule mirrors a component family from
//! <https://shelly-api-docs.shelly.cloud/gen2/>:
//!
//! | module            | methods                                             |
//! |-------------------|-----------------------------------------------------|
//! | [`shelly`]        | `Shelly.GetDeviceInfo`, `.GetStatus`, `.GetConfig`, |
//! |                   | `.ListMethods`, `.Reboot`, `.CheckForUpdate`        |
//! | [`sys`]           | `Sys.GetStatus`, `.GetConfig`                       |
//! | [`wifi`]          | `Wifi.GetStatus`, `.GetConfig`, `.Scan`             |
//! | [`switch`]        | `Switch.GetStatus`, `.GetConfig`, `.Set`, `.Toggle` |
//! | [`cover`]         | `Cover.GetStatus`, `.Open`, `.Close`, `.Stop`, …    |
//! | [`light`]         | `Light.GetStatus`, `.Set`, `.Toggle`                |
//! | [`input`]         | `Input.GetStatus`, `.GetConfig`                     |
//! | [`sensors`]       | Temperature / Humidity / DevicePower / Voltmeter    |
//! | [`script`]        | `Script.List`, `.Create`, `.PutCode`, `.Start`,     |
//! |                   | `.Stop`, `.Delete`                                  |
//!
//! # Wire format
//!
//! The library speaks **`GET /rpc/<Method>?<params>`** for read and action
//! methods, and **`POST /rpc/<Method>`** with a JSON body for methods that
//! require structured input (e.g. `Script.Create`, `Script.PutCode`).
//! Every method returns a plain JSON object; there is no JSON-RPC envelope
//! on the wire. Methods with no parameters use fixed [`Path`]s;
//! parameterized methods build a query string into a [`Path`] without
//! touching the heap.
//!
//! # Borrowed deserialization
//!
//! Response structs carry a `'a` lifetime over the caller's byte buffer so
//! string fields alias the raw HTTP body. This keeps per-call allocation at
//! zero on both `no_std` and `std` targets. Callers wanting owned data can
//! clone the fields manually once `alloc` is available.

use serde::Deserialize;

use crate::error::Error;

pub mod common;
pub mod cover;
pub mod input;
pub mod light;
pub mod script;
pub mod sensors;
pub mod shelly;
pub mod switch;
pub mod sys;
pub mod wifi;

/// Maximum path length for a single RPC call. 96 bytes is enough for every
/// documented Gen2+ method plus a handful of query parameters.
pub const MAX_PATH_LEN: usize = 96;

/// A stack-allocated request path buffer.
pub type Path = heapless::String<MAX_PATH_LEN>;

/// Build a `Path` from a fixed prefix and an arbitrary tail written via
/// `core::fmt::Write`. Returns [`Error::BufferTooSmall`] if the combined
/// string overflows [`MAX_PATH_LEN`] bytes.
///
/// ```ignore
/// let path = rpc::path("/rpc/Switch.GetStatus?id=", |w| write!(w, "{}", 0))?;
/// ```
pub fn path<F>(prefix: &str, f: F) -> Result<Path, Error>
where
    F: FnOnce(&mut Path) -> core::fmt::Result,
{
    let mut p = Path::new();
    p.push_str(prefix).map_err(|_| Error::BufferTooSmall)?;
    f(&mut p).map_err(|_| Error::BufferTooSmall)?;
    Ok(p)
}

/// Parse a JSON body into `T` using borrowed zero-copy deserialization.
///
/// The returned value aliases `body`, so `body` must outlive `T`. On parse
/// failure this returns [`Error::Parse`].
pub fn parse<'a, T>(body: &'a [u8]) -> Result<T, Error>
where
    T: Deserialize<'a>,
{
    // Shelly devices occasionally pad replies with trailing whitespace; the
    // `from_slice` call reports the consumed length in the second tuple
    // element. We ignore it here — if the prefix parses cleanly, any
    // trailing bytes are either whitespace or a separate framing artifact
    // we do not care about.
    let (value, _) = serde_json_core::from_slice::<T>(body).map_err(|_| Error::Parse)?;
    Ok(value)
}

/// Shelly Gen2+ error response body: `{"code": i32, "message": "..."}`.
///
/// Returned (with HTTP 200) for malformed or unauthorized RPC calls on the
/// device HTTP surface. Parse the body with [`parse_rpc_ok`] to distinguish
/// success from this envelope.
#[derive(Debug, Deserialize)]
pub struct RpcErrorBody<'a> {
    /// Shelly RPC error code (e.g. `-103` for "resource not available").
    pub code: i32,
    /// Human-readable error message. Required so plain result objects that
    /// happen to contain a `code` integer don't match the error envelope.
    pub message: &'a str,
}

/// Check whether a Gen2+ RPC response body is an error envelope.
///
/// Shelly devices return `{"code": i32, "message": "..."}` on RPC failure and
/// method-specific JSON (typically an object *without* a top-level `code`) on
/// success. Returns `Err(Error::Rpc(code))` if the body matches the error
/// shape, `Ok(())` on any other valid JSON value, and `Err(Error::Parse)` on
/// a non-JSON body (HTML error page, empty response, binary garbage). Does
/// not validate that success bodies have the shape the caller expects — use
/// [`parse`] for that.
pub fn parse_rpc_ok(body: &[u8]) -> Result<(), Error> {
    if let Ok((err, _)) = serde_json_core::from_slice::<RpcErrorBody<'_>>(body) {
        return Err(Error::Rpc(err.code));
    }
    // `serde_json_core` 0.6's `IgnoredAny` rejects bare scalars (`null`,
    // `true`, `false`), so handle those explicitly first; everything else
    // must drive the full parser (rejects `-garbage`, `{invalid`, etc.).
    if matches!(body.trim_ascii(), b"null" | b"true" | b"false") {
        return Ok(());
    }
    let (_, consumed) =
        serde_json_core::from_slice::<serde::de::IgnoredAny>(body).map_err(|_| Error::Parse)?;
    if body[consumed..].iter().all(u8::is_ascii_whitespace) {
        Ok(())
    } else {
        Err(Error::Parse)
    }
}

/// A boolean ACK response body as returned by most setter/action methods
/// (`Switch.Set`, `Cover.Open`, `Shelly.Reboot`, ...).
///
/// Shelly actually responds with `null` for fire-and-forget methods and with
/// an object like `{"was_on": true}` for stateful setters. This type exists
/// so that callers that only care about "did it work" can share a parser.
#[derive(Debug, Default, Clone, Copy, Deserialize)]
pub struct Ack {
    /// Some setters return the previous state (`Switch.Set` → `was_on`).
    #[serde(default, rename = "was_on")]
    pub was_on: Option<bool>,
}

#[cfg(test)]
mod parse_rpc_ok_tests {
    use super::*;

    #[test]
    fn accepts_null_body() {
        // `KVS.Set` and other fire-and-forget methods return literal `null`.
        parse_rpc_ok(b"null").unwrap();
    }

    #[test]
    fn accepts_empty_object() {
        parse_rpc_ok(b"{}").unwrap();
    }

    #[test]
    fn accepts_typical_result_body() {
        // `Switch.Set` returns `{"was_on": true}` — has no top-level `code`
        // or `message`, so it must not be mistaken for the error envelope.
        parse_rpc_ok(br#"{"was_on":true}"#).unwrap();
    }

    #[test]
    fn rejects_error_envelope() {
        let err = parse_rpc_ok(br#"{"code":-103,"message":"resource unavailable"}"#);
        assert!(matches!(err, Err(Error::Rpc(-103))));
    }

    #[test]
    fn ignores_body_with_code_but_no_message() {
        // A plain result containing only `code` (e.g. a status reading) must
        // not trip the error-envelope detector — `message` is required.
        parse_rpc_ok(br#"{"code":42}"#).unwrap();
    }

    #[test]
    fn rejects_non_json() {
        // A captive-portal HTML page or upstream proxy error must not be
        // treated as a successful RPC reply.
        assert!(matches!(
            parse_rpc_ok(b"<html><body>502 Bad Gateway</body></html>"),
            Err(Error::Parse)
        ));
    }

    #[test]
    fn rejects_empty_body() {
        assert!(matches!(parse_rpc_ok(b""), Err(Error::Parse)));
    }

    #[test]
    fn rejects_binary_garbage() {
        assert!(matches!(
            parse_rpc_ok(&[0xff, 0xfe, 0x00, 0x01]),
            Err(Error::Parse)
        ));
    }

    #[test]
    fn rejects_number_like_garbage() {
        // A leading ASCII sign or digit isn't enough — the body must parse as
        // a complete JSON value.
        assert!(matches!(parse_rpc_ok(b"-garbage"), Err(Error::Parse)));
        assert!(matches!(parse_rpc_ok(b"12abc"), Err(Error::Parse)));
    }

    #[test]
    fn rejects_truncated_object() {
        assert!(matches!(parse_rpc_ok(b"{invalid"), Err(Error::Parse)));
    }

    #[test]
    fn rejects_trailing_html_after_object() {
        // A proxy that returns `{"ok":true}<html>502</html>` must not pass —
        // the valid JSON prefix is a trap and the suffix signals a proxy
        // error page.
        assert!(matches!(
            parse_rpc_ok(br#"{"ok":true}<html>"#),
            Err(Error::Parse)
        ));
    }

    #[test]
    fn rejects_trailing_garbage_after_empty_object() {
        assert!(matches!(parse_rpc_ok(b"{}garbage"), Err(Error::Parse)));
    }

    #[test]
    fn accepts_trailing_whitespace_after_object() {
        parse_rpc_ok(b"{}  \n").unwrap();
    }

    #[test]
    fn accepts_array_body() {
        // Arrays aren't typical for Shelly RPC responses but are syntactically
        // valid JSON; pinning this so we notice if it ever flips.
        parse_rpc_ok(b"[1,2,3]").unwrap();
    }

    #[test]
    fn accepts_bare_string() {
        // `serde_json_core` 0.6's `IgnoredAny` accepts bare string scalars
        // even though it refuses `null`/`true`/`false`. Pinning this so a
        // library upgrade that changes the scalar-handling rules will trip
        // a visible test failure instead of silently shifting behavior.
        parse_rpc_ok(br#""hi""#).unwrap();
    }
}

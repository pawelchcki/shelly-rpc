//! Shelly Cloud API client — scene management and device status.
//!
//! # Architecture
//!
//! Cloud requires HTTPS (TLS), which is complex and platform-specific.
//! Instead of baking in a specific TLS implementation, consumers implement
//! the [`CloudHttp`] trait — one async method — for their HTTP client. This
//! keeps the library `no_std` with zero new dependencies.
//!
//! # Discovered Cloud API endpoints (undocumented)
//!
//! All endpoints are `POST https://<server_uri>/<path>` with
//! `application/x-www-form-urlencoded` body containing `auth_key` plus
//! endpoint-specific parameters.
//!
//! ## Working
//!
//! | Endpoint             | Parameters                          | Notes |
//! |----------------------|-------------------------------------|-------|
//! | `device/all_status`  | `auth_key`                          | All devices + status. Good for credential validation. |
//! | `scene/list`         | `auth_key`                          | All scenes with full `scene_script` JSON. |
//! | `scene/add`          | `auth_key`, `name`, `scene_script`  | See below for mandatory `scene_script` fields. |
//! | `scene/delete`       | `auth_key`, `id` (scene ID)         | Deletes a scene by ID. |
//! | `scene/manual_run`   | `auth_key`, `id` (scene ID)         | Triggers a scene. Also works as GET with query params. |
//!
//! ## Does not exist (404)
//!
//! `scene/save`, `scene/create`, `scene/upload_image`, `oauth/auth` (on
//! `my.shelly.cloud`). To update a scene: delete + re-add. Image upload is
//! not available via the API.
//!
//! # Scene format (`scene_script`)
//!
//! Mandatory fields: `_enabled`, `_run_on_ingest`, `if`, and `do[].notify`.
//! Omitting any of them creates a scene that the API accepts without error,
//! but `manual_run` won't fire the notification and the scene persists as a
//! ghost entry that can break the Shelly app. `_meta` is cosmetic (controls
//! how the scene renders in the app) but should be present so the scene is
//! visible to the user.
//!
//! ```json
//! {
//!   "_enabled": true,
//!   "_run_on_ingest": true,
//!   "_meta": {"name": "...", "image": "...", "room": 0, "roomn": "Global", ...},
//!   "if": {"or": [{"and": []}]},
//!   "do": [{
//!     "notify": "push_notification",
//!     "_gui_type": "notification",
//!     "msg": "Your message here",
//!     "_gui_function": "push",
//!     "msg_type": "push"
//!   }]
//! }
//! ```
//!
//! Use [`notification_scene_script`] to build this structure correctly.
//!
//! # Rate limiting
//!
//! The Cloud API is rate-limited to ~1 request/second. Exceeding this
//! returns `{"isok": false, "errors": {"max_req": "Request limit
//! reached!"}}`. Scene creation/deletion sequences need ~1.5 s delays
//! between calls.

use serde::Deserialize;

use crate::error::Error;
use crate::util::{copy_slice, json_escape_into, url_encode_into};

// ── Endpoint paths ───────────────────────────────────────────────────
//
// Exposed so std callers building URLs by hand share the same literals as
// the embedded `CloudClient` — prevents drift if an endpoint is ever
// renamed or versioned.

/// `POST /scene/list` — list all scenes.
pub const SCENE_LIST: &str = "/scene/list";
/// `POST /scene/add` — create a new scene.
pub const SCENE_ADD: &str = "/scene/add";
/// `POST /scene/delete` — delete a scene by ID.
pub const SCENE_DELETE: &str = "/scene/delete";
/// `POST /scene/manual_run` — trigger a scene.
pub const SCENE_MANUAL_RUN: &str = "/scene/manual_run";
/// `POST /device/all_status` — all devices and their status.
pub const DEVICE_ALL_STATUS: &str = "/device/all_status";

/// Maximum length of a cloud API URL (server + path).
const MAX_URL_LEN: usize = 128;

/// Stack-allocated form body size for methods with small payloads
/// (`auth_key` + at most one short parameter).
const MAX_FORM_LEN: usize = 512;

/// Content-Type header value for form-encoded POST bodies.
pub const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";

// ── CloudHttp trait ──────────────────────────────────────────────────

/// Abstraction over an HTTPS client capable of POSTing form-encoded data.
///
/// Implementors map their transport/TLS errors to [`Error::Transport`] and
/// non-2xx status codes to [`Error::Http`]. The response body must be
/// written into `buf` and the occupied slice returned.
///
/// # Example (ureq, ~10 lines)
///
/// ```ignore
/// struct UreqHttp;
///
/// impl CloudHttp for UreqHttp {
///     async fn post_form<'b>(
///         &mut self, url: &str, body: &[u8], buf: &'b mut [u8],
///     ) -> Result<&'b [u8], shelly_rpc::Error> {
///         let resp = ureq::post(url)
///             .set("Content-Type", shelly_rpc::cloud::FORM_CONTENT_TYPE)
///             .send_bytes(body)
///             .map_err(|_| shelly_rpc::Error::Transport)?;
///         let n = resp.into_reader().read(buf)
///             .map_err(|_| shelly_rpc::Error::Transport)?;
///         Ok(&buf[..n])
///     }
/// }
/// ```
#[allow(async_fn_in_trait)]
pub trait CloudHttp {
    /// POST `body` (form-encoded) to `url`, write the response into `buf`,
    /// and return the occupied slice.
    async fn post_form<'b>(
        &mut self,
        url: &str,
        body: &[u8],
        buf: &'b mut [u8],
    ) -> Result<&'b [u8], Error>;
}

// ── CloudClient ──────────────────────────────────────────────────────

/// Async client for the Shelly Cloud API.
///
/// Mirrors the [`Device`](crate::device::Device) pattern: callers supply a
/// response buffer per call; the client builds form bodies on the stack
/// (or in a caller-provided buffer for large payloads like `scene_add`).
pub struct CloudClient<'a, T> {
    http: T,
    server: &'a str,
    auth_key: &'a str,
}

impl<'a, T: CloudHttp> CloudClient<'a, T> {
    /// Create a new cloud client.
    ///
    /// `server` is the full server URL including scheme, e.g.
    /// `"https://shelly-96-eu.shelly.cloud"`.
    pub fn new(http: T, server: &'a str, auth_key: &'a str) -> Self {
        Self {
            http,
            server: server.trim_end_matches('/'),
            auth_key,
        }
    }

    /// Build `server + path` into a stack buffer, returning a `&str`.
    fn build_url<'u>(&self, path: &str, buf: &'u mut [u8; MAX_URL_LEN]) -> Result<&'u str, Error> {
        let mut len = 0;
        len += copy_slice(buf, len, self.server.as_bytes())?;
        len += copy_slice(buf, len, path.as_bytes())?;
        // Both inputs are &str so the concatenation is valid UTF-8.
        core::str::from_utf8(&buf[..len]).map_err(|_| Error::Parse)
    }

    // ── Scene methods ────────────────────────────────────────────────

    /// List all scenes.
    pub async fn scene_list<'b>(&mut self, buf: &'b mut [u8]) -> Result<SceneListData<'b>, Error> {
        let mut url_buf = [0u8; MAX_URL_LEN];
        let url = self.build_url(SCENE_LIST, &mut url_buf)?;
        let mut form = [0u8; MAX_FORM_LEN];
        let n = form_body(&mut form, &[("auth_key", self.auth_key)])?;
        let body = self.http.post_form(url, &form[..n], buf).await?;
        parse_cloud_response(body)
    }

    /// Delete a scene by ID.
    pub async fn scene_delete(
        &mut self,
        id: &str,
        buf: &mut [u8],
    ) -> Result<SceneDeleteResult, Error> {
        let mut url_buf = [0u8; MAX_URL_LEN];
        let url = self.build_url(SCENE_DELETE, &mut url_buf)?;
        let mut form = [0u8; MAX_FORM_LEN];
        let n = form_body(&mut form, &[("auth_key", self.auth_key), ("id", id)])?;
        let body = self.http.post_form(url, &form[..n], buf).await?;
        parse_cloud_response(body)
    }

    /// Trigger a scene by ID.
    pub async fn scene_manual_run(&mut self, id: &str, buf: &mut [u8]) -> Result<(), Error> {
        let mut url_buf = [0u8; MAX_URL_LEN];
        let url = self.build_url(SCENE_MANUAL_RUN, &mut url_buf)?;
        let mut form = [0u8; MAX_FORM_LEN];
        let n = form_body(&mut form, &[("auth_key", self.auth_key), ("id", id)])?;
        let body = self.http.post_form(url, &form[..n], buf).await?;
        parse_cloud_ok(body)
    }

    /// Fetch status of all devices (returns raw JSON bytes).
    pub async fn device_all_status<'b>(&mut self, buf: &'b mut [u8]) -> Result<&'b [u8], Error> {
        let mut url_buf = [0u8; MAX_URL_LEN];
        let url = self.build_url(DEVICE_ALL_STATUS, &mut url_buf)?;
        let mut form = [0u8; MAX_FORM_LEN];
        let n = form_body(&mut form, &[("auth_key", self.auth_key)])?;
        let body = self.http.post_form(url, &form[..n], buf).await?;
        parse_cloud_ok(body)?;
        Ok(body)
    }

    /// Create a push-notification scene.
    ///
    /// Builds the mandatory `scene_script` JSON and POSTs it. `body_buf`
    /// is a caller-provided scratch buffer for the form body (~2 KB is
    /// sufficient). `rx_buf` receives the HTTP response.
    pub async fn scene_add(
        &mut self,
        name: &str,
        msg: &str,
        body_buf: &mut [u8],
        rx_buf: &mut [u8],
    ) -> Result<SceneAddResult, Error> {
        let mut script_buf = [0u8; 512];
        let script_len = notification_scene_script(&mut script_buf, name, msg)?;
        let script = core::str::from_utf8(&script_buf[..script_len]).map_err(|_| Error::Parse)?;

        let mut url_buf = [0u8; MAX_URL_LEN];
        let url = self.build_url(SCENE_ADD, &mut url_buf)?;
        let n = form_body(
            body_buf,
            &[
                ("auth_key", self.auth_key),
                ("name", name),
                ("scene_script", script),
            ],
        )?;
        let body = self.http.post_form(url, &body_buf[..n], rx_buf).await?;
        parse_cloud_response(body)
    }
}

// ── Response types ───────────────────────────────────────────────────

/// Data returned by `scene/list`.
#[derive(Debug, Clone, Deserialize)]
pub struct SceneListData<'a> {
    /// List of scenes.
    #[serde(borrow)]
    pub scenes: heapless::Vec<SceneInfo<'a>, 32>,
}

/// A single scene entry from `scene/list`.
#[derive(Debug, Clone, Deserialize)]
pub struct SceneInfo<'a> {
    /// Scene ID.
    pub id: u32,
    /// Scene name.
    #[serde(default)]
    pub name: Option<&'a str>,
}

/// Data returned by `scene/add`.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct SceneAddResult {
    /// ID of the newly created scene.
    pub scene_id: u32,
}

/// Data returned by `scene/delete`.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct SceneDeleteResult {}

// ── Envelope parsing ─────────────────────────────────────────────────

/// Cloud API response envelope: `{"isok": bool, "data": ...}`.
#[derive(Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub(crate) struct CloudEnvelope<T> {
    isok: bool,
    #[serde(default)]
    data: Option<T>,
}

/// Parse a Cloud API JSON response, verifying `isok` and extracting `data`.
pub fn parse_cloud_response<'a, T: Deserialize<'a>>(body: &'a [u8]) -> Result<T, Error> {
    let (env, _): (CloudEnvelope<T>, _) =
        serde_json_core::from_slice(body).map_err(|_| Error::Parse)?;
    if !env.isok {
        return Err(Error::CloudApi);
    }
    env.data.ok_or(Error::Parse)
}

/// Check that a Cloud API response has `isok: true` (ignoring `data`).
pub fn parse_cloud_ok(body: &[u8]) -> Result<(), Error> {
    #[derive(Deserialize)]
    struct Envelope {
        isok: bool,
    }
    let (env, _): (Envelope, _) = serde_json_core::from_slice(body).map_err(|_| Error::Parse)?;
    if !env.isok {
        return Err(Error::CloudApi);
    }
    Ok(())
}

// ── Form body / scene script helpers ─────────────────────────────────

/// URL-encode `key=value` pairs into `dst`, separated by `&`.
///
/// Returns the number of bytes written.
pub fn form_body(dst: &mut [u8], params: &[(&str, &str)]) -> Result<usize, Error> {
    let mut len = 0;
    for (i, (key, value)) in params.iter().enumerate() {
        if i > 0 {
            len += copy_slice(dst, len, b"&")?;
        }
        len += url_encode_into(&mut dst[len..], key.as_bytes())?;
        len += copy_slice(dst, len, b"=")?;
        len += url_encode_into(&mut dst[len..], value.as_bytes())?;
    }
    Ok(len)
}

/// Build the mandatory `scene_script` JSON for a push-notification scene.
///
/// Writes the complete JSON into `dst` and returns the number of bytes
/// written. Both `name` and `msg` are JSON-escaped automatically.
pub fn notification_scene_script(dst: &mut [u8], name: &str, msg: &str) -> Result<usize, Error> {
    let mut len = 0;
    len += copy_slice(
        dst,
        len,
        br#"{"_enabled":true,"_run_on_ingest":true,"_meta":{"name":""#,
    )?;
    len += json_escape_into(&mut dst[len..], name.as_bytes())?;
    len += copy_slice(
        dst,
        len,
        br#"","image":"images/room_def/bedroom_img_def_m.jpg","integrations":false,"room":0,"roomn":"Global","position":0,"backgroundColor":""},"if":{"or":[{"and":[]}]},"do":[{"notify":"push_notification","_gui_type":"notification","msg":""#,
    )?;
    len += json_escape_into(&mut dst[len..], msg.as_bytes())?;
    len += copy_slice(
        dst,
        len,
        br#"","_gui_function":"push","msg_type":"push"}]}"#,
    )?;
    Ok(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── form_body ────────────────────────────────────────────────────

    #[test]
    fn form_body_single_param() {
        let mut buf = [0u8; 128];
        let n = form_body(&mut buf, &[("auth_key", "abc123")]).unwrap();
        assert_eq!(core::str::from_utf8(&buf[..n]).unwrap(), "auth_key=abc123");
    }

    #[test]
    fn form_body_multiple_params() {
        let mut buf = [0u8; 128];
        let n = form_body(&mut buf, &[("a", "1"), ("b", "2")]).unwrap();
        assert_eq!(core::str::from_utf8(&buf[..n]).unwrap(), "a=1&b=2");
    }

    #[test]
    fn form_body_special_chars() {
        let mut buf = [0u8; 128];
        let n = form_body(&mut buf, &[("key", "hello world&more=yes")]).unwrap();
        assert_eq!(
            core::str::from_utf8(&buf[..n]).unwrap(),
            "key=hello%20world%26more%3Dyes"
        );
    }

    #[test]
    fn form_body_empty() {
        let mut buf = [0u8; 64];
        let n = form_body(&mut buf, &[]).unwrap();
        assert_eq!(n, 0);
    }

    // ── notification_scene_script ────────────────────────────────────

    #[test]
    fn scene_script_matches_expected_format() {
        let mut buf = [0u8; 512];
        let n = notification_scene_script(&mut buf, "Test Scene", "Hello!").unwrap();
        let json = core::str::from_utf8(&buf[..n]).unwrap();

        // Verify the complete JSON matches the mandatory format.
        let expected = concat!(
            r#"{"_enabled":true,"_run_on_ingest":true,"#,
            r#""_meta":{"name":"Test Scene","image":"images/room_def/bedroom_img_def_m.jpg","integrations":false,"room":0,"roomn":"Global","position":0,"backgroundColor":""},"#,
            r#""if":{"or":[{"and":[]}]},"#,
            r#""do":[{"notify":"push_notification","_gui_type":"notification","msg":"Hello!","_gui_function":"push","msg_type":"push"}]}"#,
        );
        assert_eq!(json, expected);
    }

    #[test]
    fn scene_script_escapes_quotes() {
        let mut buf = [0u8; 512];
        let n = notification_scene_script(&mut buf, "say \"hi\"", "it's \"done\"").unwrap();
        let json = core::str::from_utf8(&buf[..n]).unwrap();
        assert!(json.contains(r#""name":"say \"hi\"""#));
        assert!(json.contains(r#""msg":"it's \"done\"""#));
    }

    // ── parse_cloud_response ─────────────────────────────────────────

    #[test]
    fn parse_response_ok() {
        let json = br#"{"isok":true,"data":{"scene_id":42}}"#;
        let result: SceneAddResult = parse_cloud_response(json).unwrap();
        assert_eq!(result.scene_id, 42);
    }

    #[test]
    fn parse_response_isok_false() {
        let json = br#"{"isok":false,"errors":{"max_req":"Request limit reached!"}}"#;
        let err = parse_cloud_response::<SceneAddResult>(json);
        assert!(matches!(err, Err(Error::CloudApi)));
    }

    #[test]
    fn parse_response_scene_list() {
        let json = br#"{"isok":true,"data":{"scenes":[{"id":1,"name":"Wake up"},{"id":2,"name":"Goodnight"}]}}"#;
        let result: SceneListData<'_> = parse_cloud_response(json).unwrap();
        assert_eq!(result.scenes.len(), 2);
        assert_eq!(result.scenes[0].id, 1);
        assert_eq!(result.scenes[0].name, Some("Wake up"));
        assert_eq!(result.scenes[1].id, 2);
        assert_eq!(result.scenes[1].name, Some("Goodnight"));
    }

    #[test]
    fn parse_response_missing_data() {
        let json = br#"{"isok":true}"#;
        let err = parse_cloud_response::<SceneAddResult>(json);
        assert!(matches!(err, Err(Error::Parse)));
    }

    #[test]
    fn parse_response_delete() {
        let json = br#"{"isok":true,"data":{}}"#;
        let _result: SceneDeleteResult = parse_cloud_response(json).unwrap();
    }

    #[test]
    fn parse_cloud_ok_success() {
        let json = br#"{"isok":true,"data":{}}"#;
        parse_cloud_ok(json).unwrap();
    }

    #[test]
    fn parse_cloud_ok_failure() {
        let json = br#"{"isok":false,"errors":{}}"#;
        assert!(matches!(parse_cloud_ok(json), Err(Error::CloudApi)));
    }

    #[test]
    fn parse_cloud_ok_rejects_non_json() {
        let html = b"<html><body>502 Bad Gateway</body></html>";
        assert!(matches!(parse_cloud_ok(html), Err(Error::Parse)));
    }

    #[test]
    fn parse_cloud_ok_rejects_empty() {
        assert!(matches!(parse_cloud_ok(b""), Err(Error::Parse)));
    }

    // ── CloudClient URL normalization ────────────────────────────────

    /// Trivial [`CloudHttp`] stub; never invoked — used only to satisfy the
    /// `T: CloudHttp` bound on [`CloudClient::new`] in the trailing-slash
    /// tests below.
    struct NoHttp;

    impl CloudHttp for NoHttp {
        async fn post_form<'b>(
            &mut self,
            _url: &str,
            _body: &[u8],
            _buf: &'b mut [u8],
        ) -> Result<&'b [u8], Error> {
            unreachable!("NoHttp is inert");
        }
    }

    #[test]
    fn cloud_client_strips_trailing_slash() {
        let client = CloudClient::new(NoHttp, "https://example.com/", "k");
        let mut buf = [0u8; MAX_URL_LEN];
        let url = client.build_url(SCENE_MANUAL_RUN, &mut buf).unwrap();
        assert_eq!(url, "https://example.com/scene/manual_run");
    }

    #[test]
    fn cloud_client_preserves_bare_server() {
        let client = CloudClient::new(NoHttp, "https://example.com", "k");
        let mut buf = [0u8; MAX_URL_LEN];
        let url = client.build_url(SCENE_MANUAL_RUN, &mut buf).unwrap();
        assert_eq!(url, "https://example.com/scene/manual_run");
    }

    #[test]
    fn cloud_client_strips_all_trailing_slashes() {
        // Matches `shellyctl::cloud::helpers::normalize_server_uri` semantics
        // so both client paths produce the same URL regardless of how many
        // slashes the user pasted.
        let client = CloudClient::new(NoHttp, "https://example.com//", "k");
        let mut buf = [0u8; MAX_URL_LEN];
        let url = client.build_url(SCENE_MANUAL_RUN, &mut buf).unwrap();
        assert_eq!(url, "https://example.com/scene/manual_run");
    }
}

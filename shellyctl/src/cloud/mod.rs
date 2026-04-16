//! Shelly Cloud API — auth, scene triggering, device provisioning.
//!
//! See [`shelly_rpc::cloud`] module docs for the full Cloud API reference
//! (endpoints, scene format, rate limiting).
//!
//! # Auth methods (separate config files)
//!
//! - `login`     — full auth key from Shelly app → `~/.config/shelly/cloud.json`
//! - `login-diy` — OAuth with `shelly-diy` client → `~/.config/shelly/cloud-diy.json` (WIP/limited)
//!
//! The DIY OAuth flow has limited Cloud Control API scope — use `login`
//! with the Shelly app's full auth key for scene and device management.
//!
//! # OAuth (`shelly-diy`)
//!
//! The `shelly-diy` client returns the JWT access token directly as the `code`
//! query parameter in the OAuth callback — no token exchange needed. The JWT
//! payload contains `user_api_url` (the regional server URI).

mod auth;
mod helpers;

pub use auth::{login, login_diy};

use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use helpers::{json_escape, json_extract_string, normalize_server_uri};

// ── Config persistence ────────────────────────────────────────────────

struct CloudConfig {
    server: String,
    auth_key: String,
}

// Fail loudly rather than falling back to "." — writing credentials into
// the current working directory is a silent security footgun.
fn config_dir() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .ok_or("HOME environment variable is not set; cannot determine config directory")?;
    Ok(PathBuf::from(home).join(".config").join("shelly"))
}

pub(super) fn config_path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join("cloud.json"))
}

pub(super) fn diy_config_path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join("cloud-diy.json"))
}

fn load_config() -> Result<CloudConfig, String> {
    let path = config_path()?;
    let data = std::fs::read_to_string(&path).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            "not set up — run `shellyctl cloud login` first".to_string()
        } else {
            format!("failed to read {}: {e}", path.display())
        }
    })?;
    let server = json_extract_string(&data, "server_uri").ok_or("missing server_uri in config")?;
    let server = normalize_server_uri(&server);
    let auth_key = json_extract_string(&data, "auth_key").ok_or("missing auth_key in config")?;
    Ok(CloudConfig { server, auth_key })
}

pub(super) fn save_config(path: &Path, server_uri: &str, auth_key: &str) -> Result<(), String> {
    let dir = config_dir()?;
    save_config_to(&dir, path, server_uri, auth_key).map_err(|e| e.to_string())
}

fn save_config_to(dir: &Path, path: &Path, server_uri: &str, auth_key: &str) -> io::Result<()> {
    use std::io::Write as _;

    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let json = format!(
        "{{\n  \"server_uri\": \"{}\",\n  \"auth_key\": \"{}\"\n}}\n",
        json_escape(server_uri),
        json_escape(auth_key),
    );

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(json.as_bytes())?;
    // `OpenOptions::mode` only applies on create, not truncate-open. Force
    // the mode on every write so upgrading from a version that wrote 0o644
    // downgrades the existing file.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

// ── Response helpers ─────────────────────────────────────────────────

/// Read the response body, returning an error ExitCode on failure.
fn read_body(resp: ureq::Response) -> Result<String, ExitCode> {
    resp.into_string().map_err(|e| {
        eprintln!("error: failed to read response: {e}");
        ExitCode::FAILURE
    })
}

/// Check that the response body is a Cloud API envelope with `isok: true`.
///
/// Delegates to [`shelly_rpc::cloud::parse_cloud_ok`] so the CLI rejects
/// non-JSON responses (captive portals, HTML error pages) that a substring
/// match would silently accept.
fn check_isok(body: &str) -> Result<(), ExitCode> {
    use shelly_rpc::Error;
    match shelly_rpc::cloud::parse_cloud_ok(body.as_bytes()) {
        Ok(()) => Ok(()),
        Err(Error::CloudApi) => {
            eprintln!("error: API request failed");
            eprintln!("  {body}");
            Err(ExitCode::FAILURE)
        }
        Err(e) => {
            eprintln!("error: unexpected response ({e})");
            eprintln!("  {body}");
            Err(ExitCode::FAILURE)
        }
    }
}

// ── scene / init: use full auth key ───────────────────────────────────

pub fn scene_run(id: &str) -> ExitCode {
    let CloudConfig {
        server,
        auth_key: key,
    } = match load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let url = format!("{server}{}", shelly_rpc::cloud::SCENE_MANUAL_RUN);
    match ureq::post(&url).send_form(&[("auth_key", key.as_str()), ("id", id)]) {
        Ok(resp) => {
            let body = match read_body(resp) {
                Ok(b) => b,
                Err(e) => return e,
            };
            if let Err(e) = check_isok(&body) {
                return e;
            }
            eprintln!("Triggered scene #{id}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Build the JSON body for `KVS.Set` that provisions the `cloud` key on a
/// device. The value is a JSON object serialized as a JSON string (double
/// encoding): inner-string fields are JSON-escaped once when built into
/// `inner`, and `inner` is JSON-escaped again when embedded as the string
/// value of the outer `value` field.
fn build_kvs_body(server: &str, key: &str, ids: &[u64; 4]) -> String {
    let inner = format!(
        r#"{{"u":"{}","k":"{}","ws":{},"wd":{},"ds":{},"dd":{}}}"#,
        json_escape(server),
        json_escape(key),
        ids[0],
        ids[1],
        ids[2],
        ids[3],
    );
    format!(r#"{{"key":"cloud","value":"{}"}}"#, json_escape(&inner))
}

/// Provision cloud notification config to device KVS.
///
/// Scene IDs map to KVS keys: ws (washer-start), wd (washer-done),
/// ds (dryer-start), dd (dryer-done). Pass 0 to disable a notification.
pub fn init_device(host: &str, scenes: &[String]) -> ExitCode {
    if scenes.len() != 4 {
        eprintln!("usage: shellyctl cloud init <host> <washer-start> <washer-done> <dryer-start> <dryer-done>");
        return ExitCode::from(2);
    }
    let mut ids = [0u64; 4];
    for (i, s) in scenes.iter().enumerate() {
        match s.parse::<u64>() {
            Ok(n) => ids[i] = n,
            Err(e) => {
                eprintln!("error: scene #{} ('{s}') is not a valid ID: {e}", i + 1);
                return ExitCode::from(2);
            }
        }
    }
    let CloudConfig {
        server,
        auth_key: key,
    } = match load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let body = build_kvs_body(&server, &key, &ids);

    let url = format!("http://{host}/rpc/KVS.Set");
    match ureq::post(&url)
        .set("Content-Type", "application/json")
        .send_string(&body)
    {
        Ok(resp) => {
            let resp_body = match read_body(resp) {
                Ok(b) => b,
                Err(e) => return e,
            };
            if let Err(e) = shelly_rpc::rpc::parse_rpc_ok(resp_body.as_bytes()) {
                eprintln!("error: device rejected KVS.Set ({e})");
                eprintln!("  {resp_body}");
                return ExitCode::FAILURE;
            }
            eprintln!("Provisioned cloud config on {host}");
            eprintln!("  washer: start=#{} done=#{}", scenes[0], scenes[1]);
            eprintln!("  dryer:  start=#{} done=#{}", scenes[2], scenes[3]);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

// ── scene management ─────────────────────────────────────────────────

pub fn scene_add(name: &str, notification_text: &str) -> ExitCode {
    let CloudConfig {
        server,
        auth_key: key,
    } = match load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut script_buf = [0u8; 512];
    let script_len = match shelly_rpc::cloud::notification_scene_script(
        &mut script_buf,
        name,
        notification_text,
    ) {
        Ok(n) => n,
        Err(e) => {
            eprintln!(
                "error: scene script too large: {e} (buffer {} bytes, name {} bytes, message {} bytes)",
                script_buf.len(),
                name.len(),
                notification_text.len(),
            );
            return ExitCode::FAILURE;
        }
    };
    let script = match std::str::from_utf8(&script_buf[..script_len]) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: scene script is not valid UTF-8: {e}");
            return ExitCode::FAILURE;
        }
    };

    let resp = ureq::post(&format!("{server}{}", shelly_rpc::cloud::SCENE_ADD)).send_form(&[
        ("auth_key", key.as_str()),
        ("name", name),
        ("scene_script", script),
    ]);

    match resp {
        Ok(r) => {
            let body = match read_body(r) {
                Ok(b) => b,
                Err(e) => return e,
            };
            // Parse through the typed envelope so a malformed/HTML response
            // fails loudly and the `scene_id` lookup is key-exact rather than
            // a substring match against the raw body.
            match shelly_rpc::cloud::parse_cloud_response::<shelly_rpc::cloud::SceneAddResult>(
                body.as_bytes(),
            ) {
                Ok(result) => {
                    eprintln!("Created scene #{} \"{name}\"", result.scene_id);
                    println!("{}", result.scene_id);
                    ExitCode::SUCCESS
                }
                Err(shelly_rpc::Error::CloudApi) => {
                    eprintln!("error: API request failed");
                    eprintln!("  {body}");
                    ExitCode::FAILURE
                }
                Err(e) => {
                    eprintln!("error: unexpected response ({e})");
                    eprintln!("  {body}");
                    ExitCode::FAILURE
                }
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

pub fn scene_delete(id: &str) -> ExitCode {
    let CloudConfig {
        server,
        auth_key: key,
    } = match load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    match ureq::post(&format!("{server}{}", shelly_rpc::cloud::SCENE_DELETE))
        .send_form(&[("auth_key", key.as_str()), ("id", id)])
    {
        Ok(resp) => {
            let body = match read_body(resp) {
                Ok(b) => b,
                Err(e) => return e,
            };
            if let Err(e) = check_isok(&body) {
                return e;
            }
            eprintln!("Deleted scene #{id}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

pub fn scene_list() -> ExitCode {
    let CloudConfig {
        server,
        auth_key: key,
    } = match load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let resp = ureq::post(&format!("{server}{}", shelly_rpc::cloud::SCENE_LIST))
        .send_form(&[("auth_key", key.as_str())]);

    match resp {
        Ok(r) => {
            let body = match read_body(r) {
                Ok(b) => b,
                Err(e) => return e,
            };
            if let Err(e) = check_isok(&body) {
                return e;
            }
            println!("{body}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── check_isok ───────────────────────────────────────────────────

    #[test]
    fn check_isok_accepts_true() {
        check_isok(r#"{"isok":true,"data":{}}"#).unwrap();
    }

    #[test]
    fn check_isok_rejects_false() {
        let err = check_isok(r#"{"isok":false,"errors":{"max_req":"limit"}}"#);
        assert!(err.is_err());
    }

    #[test]
    fn check_isok_rejects_html_error_page() {
        // A captive portal / Cloudflare 502 page must not be treated as
        // success — only a JSON envelope with `isok: true` counts.
        let err = check_isok("<html><body>502 Bad Gateway</body></html>");
        assert!(err.is_err());
    }

    #[test]
    fn check_isok_rejects_empty_body() {
        assert!(check_isok("").is_err());
    }

    #[test]
    fn check_isok_rejects_missing_field() {
        // Valid JSON but no `isok` key — not a Cloud envelope.
        assert!(check_isok(r#"{"something":true}"#).is_err());
    }

    // ── save_config file permissions ─────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn save_config_sets_mode_0600_on_create() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = std::env::temp_dir().join(format!("shellyctl-save-new-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let path = tmp.join("cloud.json");
        save_config_to(&tmp, &path, "https://ex.com", "key").unwrap();

        let file_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600, "new config file must be 0600");
        let dir_mode = std::fs::metadata(&tmp).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700, "config dir must be 0700");

        std::fs::remove_dir_all(&tmp).unwrap();
    }

    // ── build_kvs_body ───────────────────────────────────────────────

    #[test]
    fn build_kvs_body_round_trips_via_serde() {
        let body = build_kvs_body(
            "https://shelly-96-eu.shelly.cloud",
            "abc123",
            &[10, 20, 30, 40],
        );
        let outer: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(outer["key"], "cloud");
        let value_str = outer["value"].as_str().expect("value is a string");
        let inner: serde_json::Value = serde_json::from_str(value_str).unwrap();
        assert_eq!(inner["u"], "https://shelly-96-eu.shelly.cloud");
        assert_eq!(inner["k"], "abc123");
        assert_eq!(inner["ws"], 10);
        assert_eq!(inner["wd"], 20);
        assert_eq!(inner["ds"], 30);
        assert_eq!(inner["dd"], 40);
    }

    #[test]
    fn build_kvs_body_escapes_quotes_and_backslashes_in_key() {
        let key = "ke\\\"y\\\n";
        let body = build_kvs_body("https://ex.com", key, &[0, 0, 0, 0]);
        let outer: serde_json::Value = serde_json::from_str(&body).unwrap();
        let value_str = outer["value"].as_str().unwrap();
        let inner: serde_json::Value = serde_json::from_str(value_str).unwrap();
        assert_eq!(inner["k"], key);
    }

    #[test]
    fn build_kvs_body_handles_u64_max() {
        let body = build_kvs_body("https://ex.com", "k", &[u64::MAX, 0, 0, 0]);
        let outer: serde_json::Value = serde_json::from_str(&body).unwrap();
        let value_str = outer["value"].as_str().unwrap();
        let inner: serde_json::Value = serde_json::from_str(value_str).unwrap();
        assert_eq!(inner["ws"].as_u64(), Some(u64::MAX));
    }

    #[cfg(unix)]
    #[test]
    fn save_config_downgrades_existing_0644_file() {
        use std::os::unix::fs::PermissionsExt;
        let tmp =
            std::env::temp_dir().join(format!("shellyctl-save-upgrade-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("cloud.json");
        // Simulate an older version that left the file world-readable.
        std::fs::write(&path, b"{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        save_config_to(&tmp, &path, "https://ex.com", "key").unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "re-login must downgrade 0644 to 0600");

        std::fs::remove_dir_all(&tmp).unwrap();
    }
}

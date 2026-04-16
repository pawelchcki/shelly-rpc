//! Cloud authentication — full auth key login and DIY OAuth.

use std::io::{self, BufRead, Write};
use std::process::{Command, ExitCode};

use super::helpers::{extract_query_param, jwt_extract_field, normalize_server_uri};
use super::{config_path, diy_config_path, save_config};

const OAUTH_URL: &str = "https://my.shelly.cloud/oauth_login.html";
const CLIENT_ID: &str = "shelly-diy";

/// Percent-encode a query parameter value per RFC 3986 (unreserved set only).
fn percent_encode(s: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0xF) as usize] as char);
            }
        }
    }
    out
}

/// Write `prompt` to stderr and read one line from stdin, trimmed.
///
/// Returns the raw read error on EOF-or-IO-failure so the caller can
/// distinguish "stdin closed" from "user entered blank line".
fn prompt_line(prompt: &str) -> io::Result<String> {
    eprint!("{prompt}");
    io::stderr().flush()?;
    let mut line = String::new();
    let n = io::stdin().lock().read_line(&mut line)?;
    if n == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "stdin closed before input",
        ));
    }
    Ok(line)
}

/// Spawn `xdg-open <url>`, surfacing a fallback hint on failure (missing
/// binary on headless/SSH sessions) so the user can open the URL manually
/// instead of hanging on a callback that will never fire.
fn try_open_browser(url: &str) {
    let result = Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Err(e) = result {
        eprintln!("  (could not launch browser automatically: {e})");
        eprintln!("  open the URL above in your browser to continue.");
    }
}

// ── login: auth key from Shelly app ───────────────────────────────────

pub fn login() -> ExitCode {
    let settings_url = "https://control.shelly.cloud/#/settings";

    eprintln!("Opening Shelly Cloud settings...\n  {settings_url}\n");
    eprintln!("  1. Scroll to \"Authorization cloud key\"");
    eprintln!("  2. Copy the Server URI (e.g. shelly-96-eu.shelly.cloud)");
    eprintln!("  3. Click \"Get Key\" and copy the auth key\n");

    try_open_browser(settings_url);

    let server = match prompt_line("Server URI: ") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: could not read server URI: {e}");
            return ExitCode::FAILURE;
        }
    };
    let key = match prompt_line("Auth key: ") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: could not read auth key: {e}");
            return ExitCode::FAILURE;
        }
    };
    let server = server.trim();
    let key = key.trim();

    if server.is_empty() || key.is_empty() {
        eprintln!("error: both fields are required");
        return ExitCode::FAILURE;
    }

    let server = if server.starts_with("https://") {
        server.to_string()
    } else if server.starts_with("http://") {
        server.replacen("http://", "https://", 1)
    } else {
        format!("https://{server}")
    };
    let server = normalize_server_uri(&server);

    // Verify credentials with a test API call. Probe with `device/all_status`
    // — the cheapest endpoint that requires both a valid server URI and a
    // valid auth_key, so it validates both at once.
    eprint!("Verifying credentials... ");
    io::stderr().flush().ok();
    let test_url = format!("{server}{}", shelly_rpc::cloud::DEVICE_ALL_STATUS);
    match ureq::post(&test_url).send_form(&[("auth_key", key)]) {
        Ok(resp) => {
            let body = match resp.into_string() {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("failed");
                    eprintln!("error: could not read response: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match shelly_rpc::cloud::parse_cloud_ok(body.as_bytes()) {
                Ok(()) => eprintln!("ok"),
                Err(shelly_rpc::Error::CloudApi) => {
                    eprintln!("failed");
                    eprintln!("error: invalid credentials — check your server URI and auth key");
                    return ExitCode::FAILURE;
                }
                Err(e) => {
                    eprintln!("failed");
                    eprintln!("error: unexpected response ({e})");
                    eprintln!("  {body}");
                    return ExitCode::FAILURE;
                }
            }
        }
        Err(e) => {
            eprintln!("failed");
            eprintln!("error: could not reach {server}: {e}");
            return ExitCode::FAILURE;
        }
    }

    let path = match config_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = save_config(&path, &server, key) {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }

    eprintln!("Config saved.");
    eprintln!("  server: {server}");
    eprintln!("  config: {}", path.display());
    ExitCode::SUCCESS
}

// ── login-diy: OAuth with shelly-diy (limited API access) ────────────

pub async fn login_diy() -> ExitCode {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = match TcpListener::bind("127.0.0.1:0").await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: could not bind local server: {e}");
            return ExitCode::FAILURE;
        }
    };
    let port = match listener.local_addr() {
        Ok(addr) => addr.port(),
        Err(e) => {
            eprintln!("error: could not read local address: {e}");
            return ExitCode::FAILURE;
        }
    };
    let redirect_uri = format!("http://127.0.0.1:{port}");

    let url = format!(
        "{OAUTH_URL}?client_id={CLIENT_ID}&redirect_uri={}",
        percent_encode(&redirect_uri),
    );
    eprintln!("Opening browser for Shelly Cloud login...\n  {url}\n");

    try_open_browser(&url);

    eprintln!("Waiting for login callback on {redirect_uri} ...");

    let code = loop {
        let accept_result = tokio::select! {
            result = listener.accept() => result,
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nCancelled.");
                return ExitCode::FAILURE;
            }
        };

        let (mut stream, _) = match accept_result {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        };
        let mut buf = [0u8; 4096];
        let n = match stream.read(&mut buf).await {
            Ok(n) => n,
            Err(e) => {
                eprintln!("warning: could not read callback request: {e}");
                continue;
            }
        };
        let req = String::from_utf8_lossy(&buf[..n]).to_string();
        let first_line = req.lines().next().unwrap_or("");
        let preview: String = first_line.chars().take(80).collect();

        // Browsers and antivirus tools fire speculative probes (favicon,
        // connection checks) at 127.0.0.1 — if we treated the first request as
        // the callback we would fail the login before the real redirect
        // arrived. Require `GET /...?code=...` shape; otherwise log and keep
        // listening.
        if !first_line.starts_with("GET /") {
            eprintln!("  ignoring non-OAuth request: {preview}");
            let resp = "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nNot Found";
            let _ = stream.write_all(resp.as_bytes()).await;
            let _ = stream.shutdown().await;
            continue;
        }

        let path = first_line.split_whitespace().nth(1).unwrap_or("");
        let Some(code) = extract_query_param(path, "code") else {
            eprintln!("  ignoring request without code param: {preview}");
            let resp = "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nMissing code parameter";
            let _ = stream.write_all(resp.as_bytes()).await;
            let _ = stream.shutdown().await;
            continue;
        };

        let html = "<h2>Logged in! You can close this tab.</h2>";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n{html}"
        );
        if let Err(e) = stream.write_all(resp.as_bytes()).await {
            eprintln!("warning: could not write callback response to browser: {e}");
        }
        if let Err(e) = stream.shutdown().await {
            eprintln!("warning: callback stream shutdown failed: {e}");
        }
        break code;
    };

    // For shelly-diy, the "code" IS the JWT access token.
    let token = code;

    let Some(server_uri) = jwt_extract_field(&token, "user_api_url") else {
        eprintln!("error: could not decode server URI from JWT");
        return ExitCode::FAILURE;
    };
    let server_uri = normalize_server_uri(&server_uri);

    let path = match diy_config_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = save_config(&path, &server_uri, &token) {
        eprintln!("error saving config: {e}");
        return ExitCode::FAILURE;
    }

    eprintln!("Logged in (DIY — limited API access)!");
    eprintln!("  server: {server_uri}");
    eprintln!("  config: {}", path.display());
    ExitCode::SUCCESS
}

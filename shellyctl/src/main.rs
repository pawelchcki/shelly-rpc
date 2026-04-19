//! `shellyctl` — command-line client for Shelly Gen2+ devices.
//!
//! Uses the `shelly-rpc` async library over a tokio-backed network stack.

mod cloud;
mod minify;
mod nal;
mod self_update;

use std::process::ExitCode;

use shelly_rpc::Device;

use crate::nal::StdStack;

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().cloned();
    let Some(cmd) = cmd else {
        return usage();
    };
    args.remove(0);

    match cmd.as_str() {
        "status" => {
            let Some(host) = args.first() else {
                eprintln!("error: `status` requires a host argument");
                return ExitCode::from(2);
            };
            run_async(run_status(host))
        }
        "update" => {
            let Some(host) = args.first() else {
                eprintln!("error: `update` requires a host argument");
                return ExitCode::from(2);
            };
            run_async(run_update(host))
        }
        "record" => {
            if args.len() < 2 {
                eprintln!("error: `record` requires <host> <out-dir>");
                return ExitCode::from(2);
            }
            run_async(run_record(&args[0], &args[1]))
        }
        "call" => {
            if args.len() < 2 {
                eprintln!("error: `call` requires <host> <method>");
                return ExitCode::from(2);
            }
            run_async(run_call(&args[0], &args[1]))
        }
        "discover" => run_discover(&args),
        "script" => {
            let Some(host) = args.first() else {
                eprintln!("error: `script` requires <host> <action> [args]");
                return ExitCode::from(2);
            };
            let Some(action) = args.get(1) else {
                eprintln!("error: `script` requires an action (list|upload|start|stop|delete)");
                return ExitCode::from(2);
            };
            run_async(run_script(host, action, &args[2..]))
        }
        "cloud" => {
            let action = args.first().map(|s| s.as_str());
            match action {
                Some("login") => cloud::login(),
                Some("login-diy") => run_async(cloud::login_diy()),
                Some("scene") => match args.get(1).map(|s| s.as_str()) {
                    Some("list") => cloud::scene_list(),
                    Some("run") => {
                        let Some(id) = args.get(2) else {
                            eprintln!("usage: shellyctl cloud scene run <id>");
                            return ExitCode::from(2);
                        };
                        cloud::scene_run(id)
                    }
                    Some("add") => {
                        let (Some(name), Some(text)) = (args.get(2), args.get(3)) else {
                            eprintln!(
                                "usage: shellyctl cloud scene add <name> <notification-text>"
                            );
                            return ExitCode::from(2);
                        };
                        cloud::scene_add(name, text)
                    }
                    Some("delete") => {
                        let Some(id) = args.get(2) else {
                            eprintln!("usage: shellyctl cloud scene delete <id>");
                            return ExitCode::from(2);
                        };
                        cloud::scene_delete(id)
                    }
                    _ => {
                        eprintln!("usage: shellyctl cloud scene <list|run|add|delete>");
                        ExitCode::from(2)
                    }
                },
                Some("init") => {
                    if args.len() < 6 {
                        eprintln!("usage: shellyctl cloud init <host> <washer-start> <washer-done> <dryer-start> <dryer-done>");
                        return ExitCode::from(2);
                    }
                    cloud::init_device(&args[1], &args[2..6])
                }
                _ => {
                    eprintln!("usage: shellyctl cloud <login|login-diy|scene|init>");
                    ExitCode::from(2)
                }
            }
        }
        "logs" => {
            let Some(host) = args.first() else {
                eprintln!("error: `logs` requires a host argument");
                return ExitCode::from(2);
            };
            run_async(run_logs(host))
        }
        "run" => {
            let Some(host) = args.first() else {
                eprintln!("error: `run` requires <host> [--minify] <file.js|-e 'code'>");
                return ExitCode::from(2);
            };
            let (do_minify, rest) = take_minify_flag(&args[1..]);
            let (code, source_name) = if rest.first().map(|s| s.as_str()) == Some("-e") {
                let Some(expr) = rest.get(1) else {
                    eprintln!("error: `-e` requires a code argument");
                    return ExitCode::from(2);
                };
                (expr.clone(), "<-e>".to_string())
            } else if let Some(path) = rest.first() {
                let code = match read_source(path) {
                    Ok(c) => c,
                    Err(c) => return c,
                };
                (code, path.clone())
            } else {
                eprintln!("error: `run` requires <file.js> or -e 'code'");
                return ExitCode::from(2);
            };
            let code = match maybe_minify(code, do_minify, &source_name) {
                Ok(c) => c,
                Err(c) => return c,
            };
            run_async(run_script_ephemeral(host, &code))
        }
        "compile" => run_compile(&args),
        "self-update" => self_update::run(),
        "-h" | "--help" | "help" => usage(),
        other => {
            eprintln!("error: unknown command `{other}`");
            usage()
        }
    }
}

fn run_async(fut: impl std::future::Future<Output = ExitCode>) -> ExitCode {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: failed to start async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    rt.block_on(fut)
}

fn base_url(host: &str) -> String {
    if host.starts_with("http://") || host.starts_with("https://") {
        host.to_string()
    } else {
        format!("http://{host}")
    }
}

/// Pull `--minify` from anywhere in a positional-arg slice. Returns
/// `(minify, remaining_args)`. Accepting it at any position matters
/// because `script upload [name] <file.js>` is variadic and users
/// naturally append the flag.
fn take_minify_flag(args: &[String]) -> (bool, Vec<String>) {
    let minify = args.iter().any(|s| s == "--minify");
    let rest = args.iter().filter(|s| *s != "--minify").cloned().collect();
    (minify, rest)
}

fn read_source(path: &str) -> Result<String, ExitCode> {
    std::fs::read_to_string(path).map_err(|e| {
        eprintln!("error: reading {path}: {e}");
        ExitCode::FAILURE
    })
}

fn maybe_minify(source: String, enabled: bool, source_name: &str) -> Result<String, ExitCode> {
    if !enabled {
        return Ok(source);
    }
    minify::minify(&source, source_name).map_err(|e| {
        eprintln!("error: minification failed: {e}");
        ExitCode::FAILURE
    })
}

fn run_compile(args: &[String]) -> ExitCode {
    let Some(input) = args.first() else {
        eprintln!("usage: shellyctl compile <input.js> [-o output.js]");
        return ExitCode::from(2);
    };

    let out_path: Option<&str> = match args.len() {
        1 => None,
        3 if args[1] == "-o" => Some(args[2].as_str()),
        _ => {
            eprintln!("usage: shellyctl compile <input.js> [-o output.js]");
            return ExitCode::from(2);
        }
    };

    let source = match read_source(input) {
        Ok(s) => s,
        Err(c) => return c,
    };
    let source_len = source.len();
    let minified = match maybe_minify(source, true, input) {
        Ok(m) => m,
        Err(c) => return c,
    };
    eprintln!(
        "{}: {} -> {} bytes ({}%)",
        input,
        source_len,
        minified.len(),
        if source_len == 0 {
            100
        } else {
            minified.len() * 100 / source_len
        },
    );

    match out_path {
        Some(path) => match std::fs::write(path, &minified) {
            Ok(()) => {
                eprintln!("Wrote {path}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error writing {path}: {e}");
                ExitCode::FAILURE
            }
        },
        None => {
            print!("{minified}");
            ExitCode::SUCCESS
        }
    }
}

async fn run_status(host: &str) -> ExitCode {
    let stack = StdStack;
    let base = base_url(host);
    let mut device = match Device::new(&stack, &stack, &base) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut buf = [0u8; 4096];
    match device.device_info(&mut buf).await {
        Ok(info) => {
            println!("Device: {} ({})", info.id, info.app);
            println!("  model:    {}", info.model);
            println!("  fw:       {} ({})", info.ver, info.fw_id);
            println!("  gen:      {}", info.gen);
            if let Some(name) = info.name {
                println!("  name:     {name}");
            }
            println!("  auth:     {}", if info.auth_en { "on" } else { "off" });
        }
        Err(e) => {
            eprintln!("error fetching device info: {e}");
            return ExitCode::FAILURE;
        }
    }

    match device.status(&mut buf).await {
        Ok(status) => {
            if let Some(sys) = &status.sys {
                if let Some(uptime) = sys.uptime {
                    let h = uptime / 3600;
                    let m = (uptime % 3600) / 60;
                    println!("  uptime:   {h}h {m}m");
                }
                if let Some(free) = sys.ram_free {
                    println!("  ram free: {free} B");
                }
            }
            if let Some(wifi) = &status.wifi {
                if let Some(ssid) = wifi.ssid {
                    print!("  wifi:     {ssid}");
                    if let Some(rssi) = wifi.rssi {
                        print!(" ({rssi} dBm)");
                    }
                    println!();
                }
                if let Some(ip) = wifi.sta_ip {
                    println!("  ip:       {ip}");
                }
            }
            if let Some(cloud) = &status.cloud {
                println!(
                    "  cloud:    {}",
                    if cloud.connected {
                        "connected"
                    } else {
                        "disconnected"
                    }
                );
            }
            if let Some(mqtt) = &status.mqtt {
                println!(
                    "  mqtt:     {}",
                    if mqtt.connected {
                        "connected"
                    } else {
                        "disconnected"
                    }
                );
            }
        }
        Err(e) => {
            eprintln!("warning: could not fetch full status: {e}");
        }
    }

    ExitCode::SUCCESS
}

async fn run_update(host: &str) -> ExitCode {
    let stack = StdStack;
    let base = base_url(host);
    let mut device = match Device::new(&stack, &stack, &base) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut buf = [0u8; 4096];

    match device.check_for_update(&mut buf).await {
        Ok(info) => {
            if let Some(stable) = &info.stable {
                eprintln!("Available: {}", stable.version);
            } else {
                eprintln!("Already up to date.");
                return ExitCode::SUCCESS;
            }
        }
        Err(e) => {
            eprintln!("error checking for update: {e}");
            return ExitCode::FAILURE;
        }
    }

    match device.update(&mut buf).await {
        Ok(_) => {
            eprintln!("Update started — device will reboot.");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run_record(host: &str, out_dir: &str) -> ExitCode {
    let stack = StdStack;
    let base = base_url(host);
    let mut device = match Device::new(&stack, &stack, &base) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut buf = [0u8; 8192];
    let app = match device.device_info(&mut buf).await {
        Ok(info) => info.app.to_string(),
        Err(e) => {
            eprintln!("error fetching device info: {e}");
            return ExitCode::FAILURE;
        }
    };

    let dir = std::path::Path::new(out_dir).join(&app);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("error creating {}: {e}", dir.display());
        return ExitCode::FAILURE;
    }
    eprintln!("Recording to {}/", dir.display());

    let methods = match device.list_methods(&mut buf).await {
        Ok(ml) => ml.methods.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        Err(e) => {
            eprintln!("error listing methods: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Record each GetStatus / GetConfig / GetDeviceInfo / ListMethods /
    // CheckForUpdate method.
    let record_suffixes = [
        "GetStatus",
        "GetConfig",
        "GetDeviceInfo",
        "ListMethods",
        "CheckForUpdate",
    ];
    let mut recorded = 0u32;
    let mut failed = 0u32;

    for method in &methods {
        let should_record = record_suffixes.iter().any(|s| method.ends_with(s));
        if !should_record {
            continue;
        }

        let rpc_path = format!("/rpc/{method}");
        let body = match device.call_raw(&rpc_path, &mut buf).await {
            Ok(b) => b,
            Err(e) => {
                eprintln!("  {method}: error ({e})");
                failed += 1;
                continue;
            }
        };

        let filename = dir.join(format!("{method}.json"));
        match std::fs::write(&filename, body) {
            Ok(()) => {
                eprintln!("  {method}: {} bytes", body.len());
                recorded += 1;
            }
            Err(e) => {
                eprintln!("  {method}: write error ({e})");
                failed += 1;
            }
        }
    }

    eprintln!("Recorded {recorded} methods ({failed} failed).");
    if recorded == 0 && failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

async fn run_call(host: &str, method: &str) -> ExitCode {
    let stack = StdStack;
    let base = base_url(host);
    let mut device = match Device::new(&stack, &stack, &base) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let rpc_path = if method.starts_with('/') {
        method.to_string()
    } else {
        format!("/rpc/{method}")
    };

    let mut buf = [0u8; 8192];
    match device.call_raw(&rpc_path, &mut buf).await {
        Ok(body) => match std::str::from_utf8(body) {
            Ok(s) => {
                println!("{s}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                let preview_len = body.len().min(32);
                let hex: Vec<String> = body[..preview_len]
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect();
                eprintln!(
                    "error: response is not valid UTF-8 ({} bytes, invalid at offset {}); \
                     first {preview_len} bytes (hex): {}",
                    body.len(),
                    e.valid_up_to(),
                    hex.join(" "),
                );
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_discover(args: &[String]) -> ExitCode {
    use mdns_sd::{ServiceDaemon, ServiceEvent};
    use std::time::{Duration, Instant};

    let timeout_secs: u64 = match args.first() {
        Some(s) => match s.parse::<u64>() {
            Ok(n) => n,
            Err(e) => {
                eprintln!("error: invalid timeout '{s}': {e}");
                return ExitCode::from(2);
            }
        },
        None => 5,
    };

    let mdns = match ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: failed to start mDNS daemon: {e}");
            return ExitCode::FAILURE;
        }
    };

    let receiver = match mdns.browse("_shelly._tcp.local.") {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: failed to browse: {e}");
            return ExitCode::FAILURE;
        }
    };

    eprintln!("Discovering Shelly devices for {timeout_secs}s...\n");

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    // Track which devices we've printed with a usable (IPv4) address.
    let mut resolved = std::collections::HashSet::<String>::new();

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }

        match receiver.recv_timeout(remaining) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                let name = info
                    .fullname
                    .strip_suffix("._shelly._tcp.local.")
                    .unwrap_or(&info.fullname)
                    .to_string();

                let ipv4: Vec<String> = info
                    .addresses
                    .iter()
                    .filter(|a| a.is_ipv4())
                    .map(|a| a.to_string())
                    .collect();

                if ipv4.is_empty() {
                    // No IPv4 yet — skip, we'll catch it on re-resolve.
                    continue;
                }

                if !resolved.insert(name.clone()) {
                    continue;
                }

                let addr = ipv4.join(",");
                // stdout: just the IP (for piping)
                // stderr: name + IP (for humans)
                println!("{addr}");
                eprintln!("  {name}\t{addr}");
            }
            Ok(_) => {}
            Err(e) => {
                // The mdns_sd channel surfaces both normal timeouts and
                // daemon disconnects through the same error type. Timeouts
                // are expected (we hit our deadline); anything else means we
                // lost the daemon and results may be incomplete.
                let debug = format!("{e:?}");
                if !debug.contains("Timeout") {
                    eprintln!("warning: mDNS channel error ({debug}) — results may be incomplete");
                }
                break;
            }
        }
    }

    if let Err(e) = mdns.shutdown() {
        eprintln!("warning: mDNS shutdown failed: {e}");
    }
    eprintln!("\nFound {} device(s).", resolved.len());
    ExitCode::SUCCESS
}

async fn run_script_ephemeral(host: &str, code: &str) -> ExitCode {
    let stack = StdStack;
    let base = base_url(host);
    let mut device = match Device::new(&stack, &stack, &base) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let ts = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        Err(e) => {
            eprintln!("error: system clock is before UNIX epoch: {e}");
            return ExitCode::FAILURE;
        }
    };
    let name = format!("__run_{ts}");

    let mut buf = [0u8; 4096];

    let created = match device.script_create(&name, &mut buf).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error creating script: {e}");
            return ExitCode::FAILURE;
        }
    };
    let id = created.id;
    eprintln!("Created ephemeral script #{id}");

    let mut body_buf = vec![0u8; code.len() * 2 + 256];
    if let Err(e) = device
        .script_put_code(id, code, false, &mut body_buf, &mut buf)
        .await
    {
        eprintln!("error uploading code: {e}");
        if let Err(ce) = device.script_delete(id, &mut buf).await {
            eprintln!("warning: failed to delete script #{id}: {ce}");
            eprintln!("  manually clean up: shellyctl script <host> delete {id}");
        }
        return ExitCode::FAILURE;
    }

    if let Err(e) = device.script_start(id, &mut buf).await {
        eprintln!("error starting script: {e}");
        if let Err(ce) = device.script_delete(id, &mut buf).await {
            eprintln!("warning: failed to delete script #{id}: {ce}");
            eprintln!("  manually clean up: shellyctl script <host> delete {id}");
        }
        return ExitCode::FAILURE;
    }
    eprintln!("Running. Streaming logs (Ctrl-C to stop)...\n");

    stream_debug_log(host).await;

    eprintln!("\nCleaning up script #{id}...");
    if let Err(e) = device.script_stop(id, &mut buf).await {
        eprintln!("warning: failed to stop script #{id}: {e}");
    }
    if let Err(e) = device.script_delete(id, &mut buf).await {
        eprintln!("warning: failed to delete script #{id}: {e}");
        eprintln!("  manually clean up: shellyctl script <host> delete {id}");
    }

    ExitCode::SUCCESS
}

async fn run_logs(host: &str) -> ExitCode {
    eprintln!("Streaming logs (Ctrl-C to stop)...\n");
    stream_debug_log(host).await;
    ExitCode::SUCCESS
}

/// Connect to `http://<host>/debug/log` and stream the chunked HTTP
/// response to stdout until Ctrl-C or connection close.
async fn stream_debug_log(host: &str) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let candidate = if host.starts_with("http://") || host.starts_with("https://") {
        host.to_string()
    } else {
        format!("http://{host}")
    };
    let parsed = match url::Url::parse(&candidate) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("error: invalid host '{host}': {e}");
            return;
        }
    };
    let path = parsed.path();
    if !path.is_empty() && path != "/" {
        eprintln!("error: host must not include a path (got '{path}')");
        return;
    }
    if parsed.query().is_some() {
        eprintln!("error: host must not include a query string");
        return;
    }
    if parsed.fragment().is_some() {
        eprintln!("error: host must not include a fragment");
        return;
    }
    let Some(host_str) = parsed.host_str() else {
        eprintln!("error: host is required");
        return;
    };
    let port = parsed.port_or_known_default().unwrap_or(80);
    let addr = if host_str.contains(':') {
        format!("[{host_str}]:{port}")
    } else {
        format!("{host_str}:{port}")
    };

    let mut stream = match TcpStream::connect(&addr).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error connecting to {addr}: {e}");
            return;
        }
    };

    let req = format!("GET /debug/log HTTP/1.1\r\nHost: {addr}\r\nAccept: text/plain\r\n\r\n");
    if let Err(e) = stream.write_all(req.as_bytes()).await {
        eprintln!("error: {e}");
        return;
    }

    let mut decoder = ChunkDecoder::new();

    // Skip past the HTTP response headers so we hand only the chunked body
    // to the decoder.
    let mut header_buf = [0u8; 1024];
    let mut header_len = 0;
    loop {
        let n = match stream.read(&mut header_buf[header_len..]).await {
            Ok(0) => {
                eprintln!("warning: connection closed before HTTP headers arrived");
                return;
            }
            Ok(n) => n,
            Err(e) => {
                eprintln!("warning: read error while receiving HTTP headers: {e}");
                return;
            }
        };
        header_len += n;
        if let Some(pos) = header_buf[..header_len]
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
        {
            let body_start = pos + 4;
            if body_start < header_len {
                let stdout = std::io::stdout();
                let mut out = stdout.lock();
                match decoder.feed(&header_buf[body_start..header_len], &mut out) {
                    ChunkOutcome::Continue => {}
                    ChunkOutcome::BrokenPipe | ChunkOutcome::Terminated => return,
                    ChunkOutcome::Malformed(reason) => {
                        drop(out);
                        eprintln!("stopped streaming: {reason}");
                        return;
                    }
                }
            }
            break;
        }
    }

    let mut read_buf = [0u8; 4096];
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = async {
            loop {
                match stream.read(&mut read_buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        let stdout = std::io::stdout();
                        let mut out = stdout.lock();
                        match decoder.feed(&read_buf[..n], &mut out) {
                            ChunkOutcome::Continue => {}
                            ChunkOutcome::BrokenPipe | ChunkOutcome::Terminated => break,
                            ChunkOutcome::Malformed(reason) => {
                                drop(out);
                                eprintln!("stopped streaming: {reason}");
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("warning: read error while streaming logs: {e}");
                        break;
                    }
                }
            }
        } => {}
    }
}

/// Outcome of feeding a byte slice into [`ChunkDecoder::feed`]. Splits out
/// the three "stop the stream" cases that the previous `bool` return
/// collapsed, so callers can distinguish a broken pipe (silent teardown)
/// from a malformed chunk (surface a diagnostic to the user).
#[derive(Debug, PartialEq, Eq)]
enum ChunkOutcome {
    /// More data needed — keep reading from the socket.
    Continue,
    /// Writer (e.g. stdout) rejected a write. Tear down silently and let the
    /// caller run cleanup (the user closed the pipe on purpose).
    BrokenPipe,
    /// The encoding went off-rails; the wrapped reason is suitable for
    /// printing. Decoder state has been cleared.
    Malformed(&'static str),
    /// Received the `0\r\n\r\n` terminator chunk. End of stream.
    Terminated,
}

/// Incremental HTTP chunked-transfer-encoding decoder.
///
/// TCP read boundaries do not necessarily align with chunk boundaries, so
/// the decoder accumulates unparsed bytes in `buf` across `feed` calls.
struct ChunkDecoder {
    buf: Vec<u8>,
}

impl ChunkDecoder {
    fn new() -> Self {
        Self { buf: Vec::new() }
    }

    fn feed<W: std::io::Write>(&mut self, new_data: &[u8], out: &mut W) -> ChunkOutcome {
        self.buf.extend_from_slice(new_data);
        loop {
            let Some(crlf) = self.buf.windows(2).position(|w| w == b"\r\n") else {
                return ChunkOutcome::Continue;
            };
            let Ok(size_str) = std::str::from_utf8(&self.buf[..crlf]) else {
                self.buf.clear();
                return ChunkOutcome::Malformed("non-UTF8 chunk size");
            };
            let Ok(chunk_size) = usize::from_str_radix(size_str.trim(), 16) else {
                self.buf.clear();
                return ChunkOutcome::Malformed("invalid chunk size");
            };
            if chunk_size == 0 {
                self.buf.clear();
                return ChunkOutcome::Terminated;
            }
            let data_start = crlf + 2;
            let chunk_end = data_start + chunk_size + 2;
            if self.buf.len() < chunk_end {
                return ChunkOutcome::Continue;
            }
            if out
                .write_all(&self.buf[data_start..data_start + chunk_size])
                .is_err()
            {
                return ChunkOutcome::BrokenPipe;
            }
            self.buf.drain(..chunk_end);
        }
    }
}

async fn run_script(host: &str, action: &str, args: &[String]) -> ExitCode {
    let stack = StdStack;
    let base = base_url(host);
    let mut device = match Device::new(&stack, &stack, &base) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut buf = [0u8; 4096];

    match action {
        "list" => match device.script_list(&mut buf).await {
            Ok(list) => {
                for s in &list.scripts {
                    let name = s.name.unwrap_or("(unnamed)");
                    let state = if s.running { "running" } else { "stopped" };
                    let auto = if s.enable { "auto" } else { "manual" };
                    println!("{id}\t{name}\t{state}\t{auto}", id = s.id);
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        "upload" => {
            let (do_minify, args) = take_minify_flag(args);
            let (name, file_path) = match args.len() {
                1 => {
                    let p = &args[0];
                    let Some(n) = std::path::Path::new(p).file_stem().and_then(|s| s.to_str())
                    else {
                        eprintln!(
                            "error: cannot derive script name from '{p}' (non-UTF-8); pass an explicit name"
                        );
                        return ExitCode::FAILURE;
                    };
                    (n.to_string(), p.clone())
                }
                2 => (args[0].clone(), args[1].clone()),
                _ => {
                    eprintln!("usage: shellyctl script <host> upload [--minify] [name] <file.js>");
                    return ExitCode::from(2);
                }
            };

            let source = match read_source(&file_path) {
                Ok(c) => c,
                Err(c) => return c,
            };
            let source_len = source.len();
            let code = match maybe_minify(source, do_minify, &file_path) {
                Ok(c) => c,
                Err(c) => return c,
            };
            if do_minify {
                eprintln!("Minified {} -> {} bytes", source_len, code.len());
            }

            let created = match device.script_create(&name, &mut buf).await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error creating script: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let id = created.id;
            eprintln!("Created script #{id} \"{name}\"");

            // Separate body buffer because the JSON envelope + escaped source
            // can dwarf the response buffer used for `buf`.
            let mut body_buf = vec![0u8; code.len() * 2 + 256];
            match device
                .script_put_code(id, &code, false, &mut body_buf, &mut buf)
                .await
            {
                Ok(r) => eprintln!("Uploaded {} bytes", r.len),
                Err(e) => {
                    eprintln!("error uploading code: {e}");
                    if let Err(ce) = device.script_delete(id, &mut buf).await {
                        eprintln!("warning: failed to delete orphan script #{id}: {ce}");
                        eprintln!("  manually clean up: shellyctl script <host> delete {id}");
                    }
                    return ExitCode::FAILURE;
                }
            }

            ExitCode::SUCCESS
        }
        "start" => {
            let Some(id) = args.first().and_then(|s| s.parse::<u32>().ok()) else {
                eprintln!("usage: shellyctl script <host> start <id>");
                return ExitCode::from(2);
            };
            match device.script_start(id, &mut buf).await {
                Ok(r) => {
                    eprintln!(
                        "Started script #{id} (was {})",
                        if r.was_running { "running" } else { "stopped" }
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        "stop" => {
            let Some(id) = args.first().and_then(|s| s.parse::<u32>().ok()) else {
                eprintln!("usage: shellyctl script <host> stop <id>");
                return ExitCode::from(2);
            };
            match device.script_stop(id, &mut buf).await {
                Ok(r) => {
                    eprintln!(
                        "Stopped script #{id} (was {})",
                        if r.was_running { "running" } else { "stopped" }
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        "delete" => {
            let Some(id) = args.first().and_then(|s| s.parse::<u32>().ok()) else {
                eprintln!("usage: shellyctl script <host> delete <id>");
                return ExitCode::from(2);
            };
            match device.script_delete(id, &mut buf).await {
                Ok(_) => {
                    eprintln!("Deleted script #{id}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        other => {
            eprintln!("error: unknown script action `{other}`");
            eprintln!("actions: list, upload, start, stop, delete");
            ExitCode::from(2)
        }
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "shelly {version}\n\
         \n\
         USAGE:\n    \
             shelly <command> [args]\n\
         \n\
         COMMANDS:\n    \
             discover [secs]                     Discover devices via mDNS\n    \
             status   <host>                     Fetch and display device status\n    \
             update   <host>                     Install available firmware update\n    \
             script   <host> list                List scripts\n    \
             script   <host> upload [--minify] [name] <js>  Create + upload a script (use --minify to compile first)\n    \
             script   <host> start <id>          Start a script\n    \
             script   <host> stop  <id>          Stop a script\n    \
             script   <host> delete <id>         Delete a script\n    \
             compile  <in.js> [-o out.js]        Minify a script (stdout if no -o)\n    \
             run      <host> [--minify] <file.js>   Run a script ephemerally (use --minify to compile first)\n    \
             run      <host> [--minify] -e 'code'   Run inline JS ephemerally\n    \
             logs     <host>                     Stream device debug log\n    \
             record   <host> <dir>               Record RPC responses\n    \
             call     <host> <method>            Call a raw RPC method\n    \
             cloud    login                      Auth key from Shelly Cloud\n    \
             cloud    login-diy                  OAuth login (WIP, limited)\n    \
             cloud    scene list                  List cloud scenes\n    \
             cloud    scene run <id>              Trigger a cloud scene\n    \
             cloud    scene add <name> <text>     Create notification scene\n    \
             cloud    scene delete <id>           Delete a scene\n    \
             cloud    init <host> <ws> <wd> <ds> <dd>  Provision scene IDs on device\n    \
             self-update                         Update shellyctl to the latest release\n    \
             help                                Show this message",
        version = env!("CARGO_PKG_VERSION"),
    );
    ExitCode::from(2)
}

#[cfg(test)]
mod take_minify_flag_tests {
    use super::take_minify_flag;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn accepts_flag_at_end() {
        let args = strings(&["script.js", "--minify"]);
        let (minify, rest) = take_minify_flag(&args);
        assert!(minify);
        assert_eq!(rest, strings(&["script.js"]));
    }

    #[test]
    fn accepts_flag_in_middle_and_preserves_order() {
        let args = strings(&["name", "--minify", "upload.js"]);
        let (minify, rest) = take_minify_flag(&args);
        assert!(minify);
        assert_eq!(rest, strings(&["name", "upload.js"]));
    }

    #[test]
    fn removes_multiple_occurrences() {
        let args = strings(&["--minify", "script.js", "--minify"]);
        let (minify, rest) = take_minify_flag(&args);
        assert!(minify);
        assert_eq!(rest, strings(&["script.js"]));
    }

    #[test]
    fn returns_original_args_when_flag_missing() {
        let args = strings(&["name", "upload.js"]);
        let (minify, rest) = take_minify_flag(&args);
        assert!(!minify);
        assert_eq!(rest, strings(&["name", "upload.js"]));
    }
}

#[cfg(test)]
mod chunk_decoder_tests {
    use super::*;

    #[test]
    fn single_complete_chunk() {
        let mut d = ChunkDecoder::new();
        let mut out: Vec<u8> = Vec::new();
        assert_eq!(d.feed(b"5\r\nhello\r\n", &mut out), ChunkOutcome::Continue);
        assert_eq!(out, b"hello");
        assert!(d.buf.is_empty());
    }

    #[test]
    fn chunk_split_across_feeds() {
        let mut d = ChunkDecoder::new();
        let mut out: Vec<u8> = Vec::new();
        assert_eq!(d.feed(b"5\r\n", &mut out), ChunkOutcome::Continue);
        assert!(out.is_empty());
        assert_eq!(d.feed(b"hel", &mut out), ChunkOutcome::Continue);
        assert!(out.is_empty());
        assert_eq!(d.feed(b"lo\r\n", &mut out), ChunkOutcome::Continue);
        assert_eq!(out, b"hello");
    }

    #[test]
    fn terminator_chunk() {
        let mut d = ChunkDecoder::new();
        let mut out: Vec<u8> = Vec::new();
        assert_eq!(d.feed(b"0\r\n\r\n", &mut out), ChunkOutcome::Terminated);
        assert!(d.buf.is_empty());
    }

    #[test]
    fn malformed_size_clears_buffer() {
        let mut d = ChunkDecoder::new();
        let mut out: Vec<u8> = Vec::new();
        assert_eq!(
            d.feed(b"zz\r\n", &mut out),
            ChunkOutcome::Malformed("invalid chunk size")
        );
        assert!(d.buf.is_empty());
    }

    #[test]
    fn non_utf8_size_clears_buffer() {
        let mut d = ChunkDecoder::new();
        let mut out: Vec<u8> = Vec::new();
        assert_eq!(
            d.feed(&[0xFF, 0xFE, b'\r', b'\n'], &mut out),
            ChunkOutcome::Malformed("non-UTF8 chunk size")
        );
        assert!(d.buf.is_empty());
    }

    /// Writer whose `write_all` always fails — simulates a broken pipe.
    struct FailingWriter;
    impl std::io::Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn broken_pipe_on_write_error() {
        let mut d = ChunkDecoder::new();
        let mut out = FailingWriter;
        assert_eq!(
            d.feed(b"5\r\nhello\r\n", &mut out),
            ChunkOutcome::BrokenPipe
        );
    }
}

//! `shellyctl` — command-line client for Shelly Gen2+ devices.
//!
//! Uses the `shelly-rpc` async library over a tokio-backed network stack.

mod cloud;
mod minify;
mod nal;
mod self_update;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand, ValueHint};
use shelly_rpc::{Device, Error as RpcError};

use crate::nal::StdStack;

#[derive(Parser)]
#[command(
    name = "shellyctl",
    version = env!("SHELLYCTL_VERSION"),
    about = "Command-line client for Shelly Gen2+ smart devices",
    arg_required_else_help = true,
    disable_help_subcommand = true,
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Discover devices via mDNS
    Discover {
        /// Seconds to browse before giving up
        #[arg(default_value_t = 5)]
        timeout_secs: u64,
    },
    /// Fetch and display device status
    Status { host: String },
    /// Install available firmware update
    Update { host: String },
    /// Call a raw RPC method
    Call { host: String, method: String },
    /// Record GetStatus/GetConfig/etc responses to disk
    Record {
        host: String,
        #[arg(value_hint = ValueHint::DirPath)]
        out_dir: PathBuf,
    },
    /// Stream device debug log
    Logs { host: String },
    /// Manage device scripts
    Script {
        host: String,
        #[command(subcommand)]
        action: ScriptCmd,
    },
    /// Minify a script to stdout or a file
    Compile {
        #[arg(value_hint = ValueHint::FilePath)]
        input: PathBuf,
        /// Output path (stdout if omitted)
        #[arg(short = 'o', value_name = "OUT", value_hint = ValueHint::FilePath)]
        output: Option<PathBuf>,
    },
    /// Run a script or inline JS expression ephemerally
    Run {
        host: String,
        /// Minify source before upload
        #[arg(long)]
        minify: bool,
        /// Inline JS expression (mutually exclusive with FILE)
        #[arg(
            short = 'e',
            long = "eval",
            value_name = "CODE",
            conflicts_with = "file",
            required_unless_present = "file"
        )]
        eval: Option<String>,
        #[arg(value_hint = ValueHint::FilePath)]
        file: Option<PathBuf>,
    },
    /// Shelly Cloud (scene / auth / provisioning)
    Cloud {
        #[command(subcommand)]
        action: CloudCmd,
    },
    /// Update shellyctl itself (only when installed via official installer)
    #[command(name = "self-update")]
    SelfUpdate,
    /// Generate shell completions
    Completions { shell: clap_complete::Shell },
}

#[derive(Subcommand)]
enum ScriptCmd {
    /// List scripts
    List,
    /// Create + upload a script
    Upload {
        /// Minify source before upload
        #[arg(long)]
        minify: bool,
        /// Script name, or the file path if FILE is omitted
        name_or_file: String,
        #[arg(value_hint = ValueHint::FilePath)]
        file: Option<PathBuf>,
    },
    /// Start a script
    Start { id: u32 },
    /// Stop a script
    Stop { id: u32 },
    /// Delete a script
    Delete { id: u32 },
}

#[derive(Subcommand)]
enum CloudCmd {
    /// Log in with the full Shelly app auth key
    Login,
    /// OAuth login via shelly-diy client (limited API scope)
    #[command(name = "login-diy")]
    LoginDiy,
    /// Manage cloud scenes
    Scene {
        #[command(subcommand)]
        action: SceneCmd,
    },
    /// Provision cloud scene IDs into device KVS
    Init {
        host: String,
        washer_start: u64,
        washer_done: u64,
        dryer_start: u64,
        dryer_done: u64,
    },
}

#[derive(Subcommand)]
enum SceneCmd {
    /// List cloud scenes
    List,
    /// Trigger a cloud scene
    Run { id: String },
    /// Create a notification scene
    Add { name: String, text: String },
    /// Delete a scene
    Delete { id: String },
}

/// Maximum **raw** code bytes per `Script.PutCode` POST. The body sent
/// on the wire is JSON-encoded (`shelly_rpc::json_escape_into`), and
/// every `"`, `\`, `\n`, `\r`, `\t`, and control byte doubles in size.
/// In the worst case the escaped payload is ~2× the raw chunk; we size
/// for that so even an escape-heavy source still fits under the measured
/// 8192 B device body limit after the ~40 B JSON envelope. Typical mJS
/// sources expand by ~5 %, so this is a conservative safety margin, not
/// an expected one.
const PUTCODE_CHUNK_BYTES: usize = 4000;

/// Upload `code` to an already-created script slot using one or more
/// `Script.PutCode` calls. Splits on UTF-8 character boundaries. The
/// first call uses `append: false` (replaces any prior content); later
/// calls use `append: true`. Returns the number of chunks sent.
async fn upload_code_chunked(
    device: &mut Device<'_, StdStack, StdStack>,
    id: u32,
    code: &str,
    rx_buf: &mut [u8],
) -> Result<usize, RpcError> {
    // Empty code: send one empty PutCode to clear the slot.
    if code.is_empty() {
        let mut body = [0u8; 64];
        device
            .script_put_code(id, "", false, &mut body, rx_buf)
            .await?;
        return Ok(1);
    }

    let mut sent = 0usize;
    let mut chunks = 0usize;
    while sent < code.len() {
        let remaining = &code[sent..];
        let take = pick_chunk_len(remaining, PUTCODE_CHUNK_BYTES);
        let chunk = &remaining[..take];
        let mut body = vec![0u8; chunk.len() * 2 + 256];
        device
            .script_put_code(id, chunk, chunks > 0, &mut body, rx_buf)
            .await?;
        sent += take;
        chunks += 1;
    }
    Ok(chunks)
}

/// Pick an end offset ≤ `max` that lies on a UTF-8 character boundary.
/// Returns at least 1 — the caller guarantees `!s.is_empty()`.
fn pick_chunk_len(s: &str, max: usize) -> usize {
    if s.len() <= max {
        return s.len();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    // Safety net: a 4-byte UTF-8 char at `max-3..max+1` could sit across
    // the boundary; fall back to one character forward if we walked all
    // the way to zero.
    if end == 0 {
        let mut e = 1;
        while e < s.len() && !s.is_char_boundary(e) {
            e += 1;
        }
        return e;
    }
    end
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Cmd::Discover { timeout_secs } => run_discover(timeout_secs),
        Cmd::Status { host } => run_async(run_status(&host)),
        Cmd::Update { host } => run_async(run_update(&host)),
        Cmd::Call { host, method } => run_async(run_call(&host, &method)),
        Cmd::Record { host, out_dir } => run_async(run_record(&host, &out_dir)),
        Cmd::Logs { host } => run_async(run_logs(&host)),
        Cmd::Script { host, action } => run_async(run_script(host, action)),
        Cmd::Compile { input, output } => run_compile(&input, output.as_deref()),
        Cmd::Run {
            host,
            minify,
            eval,
            file,
        } => run_run(&host, minify, eval, file),
        Cmd::Cloud { action } => run_cloud(action),
        Cmd::SelfUpdate => self_update::run(),
        Cmd::Completions { shell } => print_completions(shell),
    }
}

fn print_completions(shell: clap_complete::Shell) -> ExitCode {
    use std::io::Write;
    // `clap_complete::generate` panics on write errors. Buffer first so we
    // can route I/O failures through our normal exit-code paths instead.
    let mut buf: Vec<u8> = Vec::new();
    let mut cmd = Cli::command();
    clap_complete::generate(shell, &mut cmd, "shellyctl", &mut buf);
    let stdout = std::io::stdout();
    match stdout.lock().write_all(&buf) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error writing completions: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_cloud(action: CloudCmd) -> ExitCode {
    match action {
        CloudCmd::Login => cloud::login(),
        CloudCmd::LoginDiy => run_async(cloud::login_diy()),
        CloudCmd::Scene { action } => match action {
            SceneCmd::List => cloud::scene_list(),
            SceneCmd::Run { id } => cloud::scene_run(&id),
            SceneCmd::Add { name, text } => cloud::scene_add(&name, &text),
            SceneCmd::Delete { id } => cloud::scene_delete(&id),
        },
        CloudCmd::Init {
            host,
            washer_start,
            washer_done,
            dryer_start,
            dryer_done,
        } => cloud::init_device(
            &host,
            &cloud::KvsSceneIds {
                washer_start,
                washer_done,
                dryer_start,
                dryer_done,
            },
        ),
    }
}

fn run_run(host: &str, minify: bool, eval: Option<String>, file: Option<PathBuf>) -> ExitCode {
    let (code, source_name) = match (eval, file) {
        (Some(expr), None) => (expr, "<-e>".to_string()),
        (None, Some(path)) => {
            let code = match read_source(&path) {
                Ok(c) => c,
                Err(c) => return c,
            };
            (code, path.display().to_string())
        }
        // clap's `required_unless_present` + `conflicts_with` should reject
        // both other combinations at parse time; the arms below are a
        // belt-and-braces fallback so attribute drift surfaces as exit 2
        // rather than a panic.
        (Some(_), Some(_)) => {
            eprintln!("error: --eval and FILE are mutually exclusive");
            return ExitCode::from(2);
        }
        (None, None) => {
            eprintln!("error: `run` requires <file.js> or --eval 'code'");
            return ExitCode::from(2);
        }
    };
    let code = match maybe_minify(code, minify, &source_name) {
        Ok(c) => c,
        Err(c) => return c,
    };
    run_async(run_script_ephemeral(host, &code))
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

fn read_source(path: &Path) -> Result<String, ExitCode> {
    std::fs::read_to_string(path).map_err(|e| {
        eprintln!("error: reading {}: {e}", path.display());
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

/// Hard upper bound for the total script body on-device, measured by
/// binary search on a Shelly 1 Mini Gen3 (fw 1.7.1): the firmware
/// rejects anything past 65535 bytes with `code -103 "Script length
/// exceeded 65535 bytes limit!"`. (Per-call `PutCode` POST body is
/// limited to 8192 B separately; see `PUTCODE_CHUNK_BYTES` — chunked
/// upload handles that.)
const SCRIPT_SIZE_BUDGET: usize = 65_535;

fn run_compile(input: &Path, out_path: Option<&Path>) -> ExitCode {
    let source = match read_source(input) {
        Ok(s) => s,
        Err(c) => return c,
    };
    let source_len = source.len();
    let input_name = input.display().to_string();
    let minified = match maybe_minify(source, true, &input_name) {
        Ok(m) => m,
        Err(c) => return c,
    };
    let ratio = match (minified.len() * 100).checked_div(source_len) {
        Some(pct) => format!("{pct}%"),
        None => "n/a".to_string(),
    };
    eprintln!(
        "{}: {} -> {} bytes ({})",
        input_name,
        source_len,
        minified.len(),
        ratio,
    );
    warn_if_oversize(minified.len());

    match out_path {
        Some(path) => match std::fs::write(path, &minified) {
            Ok(()) => {
                eprintln!("Wrote {}", path.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error writing {}: {e}", path.display());
                ExitCode::FAILURE
            }
        },
        None => {
            use std::io::Write;
            let stdout = std::io::stdout();
            match stdout.lock().write_all(minified.as_bytes()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error writing to stdout: {e}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

fn warn_if_oversize(len: usize) {
    // Device accepts exactly SCRIPT_SIZE_BUDGET bytes; rejects only past it.
    if len > SCRIPT_SIZE_BUDGET {
        eprintln!(
            "warning: {len} bytes exceeds the on-device {SCRIPT_SIZE_BUDGET}-byte \
             total-script ceiling; the device will reject this with error -103"
        );
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

async fn run_record(host: &str, out_dir: &Path) -> ExitCode {
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

    let dir = out_dir.join(&app);
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

fn run_discover(timeout_secs: u64) -> ExitCode {
    use mdns_sd::{ServiceDaemon, ServiceEvent};
    use std::time::{Duration, Instant};

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

    match upload_code_chunked(&mut device, id, code, &mut buf).await {
        Ok(chunks) if chunks > 1 => {
            eprintln!("Uploaded {} bytes in {chunks} chunks", code.len());
        }
        Ok(_) => {}
        Err(e) => {
            eprintln!("error uploading code: {e}");
            if let Err(ce) = device.script_delete(id, &mut buf).await {
                eprintln!("warning: failed to delete script #{id}: {ce}");
                eprintln!("  manually clean up: shellyctl script <host> delete {id}");
            }
            return ExitCode::FAILURE;
        }
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

async fn run_script(host: String, action: ScriptCmd) -> ExitCode {
    let stack = StdStack;
    let base = base_url(&host);
    let mut device = match Device::new(&stack, &stack, &base) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut buf = [0u8; 4096];

    match action {
        ScriptCmd::List => match device.script_list(&mut buf).await {
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
        ScriptCmd::Upload {
            minify,
            name_or_file,
            file,
        } => {
            // If FILE is omitted, treat name_or_file as the file path and
            // derive the script name from its stem. Matches the previous
            // single-arg fallback (`upload foo.js` → name "foo").
            let (name, file_path): (String, PathBuf) = match file {
                Some(path) => (name_or_file, path),
                None => {
                    let path = PathBuf::from(&name_or_file);
                    let Some(n) = path.file_stem().and_then(|s| s.to_str()) else {
                        eprintln!(
                            "error: cannot derive script name from '{}' (non-UTF-8); pass an explicit name",
                            path.display()
                        );
                        return ExitCode::FAILURE;
                    };
                    (n.to_string(), path)
                }
            };

            let source = match read_source(&file_path) {
                Ok(c) => c,
                Err(c) => return c,
            };
            let source_len = source.len();
            let source_name = file_path.display().to_string();
            let code = match maybe_minify(source, minify, &source_name) {
                Ok(c) => c,
                Err(c) => return c,
            };
            if minify {
                eprintln!("Minified {} -> {} bytes", source_len, code.len());
                warn_if_oversize(code.len());
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

            match upload_code_chunked(&mut device, id, &code, &mut buf).await {
                Ok(chunks) => {
                    if chunks > 1 {
                        eprintln!("Uploaded {} bytes in {chunks} chunks", code.len());
                    } else {
                        eprintln!("Uploaded {} bytes", code.len());
                    }
                }
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
        ScriptCmd::Start { id } => match device.script_start(id, &mut buf).await {
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
        },
        ScriptCmd::Stop { id } => match device.script_stop(id, &mut buf).await {
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
        },
        ScriptCmd::Delete { id } => match device.script_delete(id, &mut buf).await {
            Ok(_) => {
                eprintln!("Deleted script #{id}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
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

#[cfg(test)]
mod pick_chunk_len_tests {
    use super::pick_chunk_len;

    #[test]
    fn shorter_than_max_returns_whole_length() {
        assert_eq!(pick_chunk_len("hello", 100), 5);
    }

    #[test]
    fn ascii_exactly_at_max() {
        assert_eq!(pick_chunk_len("abcdefgh", 4), 4);
    }

    #[test]
    fn walks_back_off_utf8_boundary() {
        // `ä` is two bytes (0xC3 0xA4). With max=4, the boundary at
        // byte 4 lands mid-char; the helper should walk back to 3.
        let s = "aaaä";
        assert_eq!(s.len(), 5);
        assert_eq!(pick_chunk_len(s, 4), 3);
    }

    #[test]
    fn single_multibyte_bigger_than_max_still_advances() {
        // `𝕏` (U+1D54F) is a single 4-byte UTF-8 codepoint. max=2
        // forces the pathological `end == 0` branch — the helper must
        // still advance by one whole character to avoid infinite loops.
        let s = "𝕏";
        assert_eq!(s.len(), 4);
        assert_eq!(pick_chunk_len(s, 2), 4);
    }

    #[test]
    fn boundary_hit_returns_max() {
        // `é` is 2 bytes, so "aé" is 3 bytes; max=3 lies on a char
        // boundary and should be returned as-is.
        let s = "aé";
        assert_eq!(s.len(), 3);
        assert_eq!(pick_chunk_len(s, 3), 3);
    }

    #[test]
    fn returns_at_least_one() {
        // A non-empty string should always return a positive length;
        // the caller upload_code_chunked relies on forward progress.
        assert!(pick_chunk_len("x", 1) >= 1);
    }
}

#[cfg(test)]
mod cli_parse_tests {
    //! Pin load-bearing clap attributes so attribute drift fails the build
    //! instead of breaking the CLI surface for users.
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("shellyctl").chain(args.iter().copied()))
    }

    #[test]
    fn cloud_init_orders_four_u64_scene_ids() {
        let cli = parse(&["cloud", "init", "192.168.1.10", "10", "20", "30", "40"]).unwrap();
        match cli.command {
            Cmd::Cloud {
                action:
                    CloudCmd::Init {
                        host,
                        washer_start,
                        washer_done,
                        dryer_start,
                        dryer_done,
                    },
            } => {
                assert_eq!(host, "192.168.1.10");
                assert_eq!(
                    (washer_start, washer_done, dryer_start, dryer_done),
                    (10, 20, 30, 40)
                );
            }
            _ => panic!("expected Cloud::Init"),
        }
    }

    #[test]
    fn cloud_init_rejects_negative_id() {
        assert!(parse(&["cloud", "init", "host", "-1", "2", "3", "4"]).is_err());
    }

    #[test]
    fn discover_defaults_to_five_seconds() {
        let cli = parse(&["discover"]).unwrap();
        match cli.command {
            Cmd::Discover { timeout_secs } => assert_eq!(timeout_secs, 5),
            _ => panic!("expected Discover"),
        }
    }

    #[test]
    fn discover_rejects_non_numeric_timeout() {
        assert!(parse(&["discover", "abc"]).is_err());
    }

    #[test]
    fn run_rejects_when_neither_eval_nor_file_given() {
        assert!(parse(&["run", "host"]).is_err());
    }

    #[test]
    fn run_rejects_when_both_eval_and_file_given() {
        assert!(parse(&["run", "host", "-e", "x", "foo.js"]).is_err());
    }

    #[test]
    fn run_accepts_eval_only() {
        assert!(parse(&["run", "host", "-e", "print(1)"]).is_ok());
    }

    #[test]
    fn run_accepts_file_only() {
        assert!(parse(&["run", "host", "foo.js"]).is_ok());
    }

    #[test]
    fn script_upload_minify_flag_before_positional() {
        assert!(parse(&["script", "host", "upload", "--minify", "foo.js"]).is_ok());
    }

    #[test]
    fn script_upload_minify_flag_after_positionals() {
        assert!(parse(&["script", "host", "upload", "name", "foo.js", "--minify"]).is_ok());
    }

    #[test]
    fn cloud_scene_add_requires_both_args() {
        assert!(parse(&["cloud", "scene", "add", "name"]).is_err());
        assert!(parse(&["cloud", "scene", "add", "name", "text"]).is_ok());
    }

    #[test]
    fn completions_rejects_unknown_shell() {
        assert!(parse(&["completions", "garbage"]).is_err());
        assert!(parse(&["completions", "bash"]).is_ok());
    }
}

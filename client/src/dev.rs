//! Local dev convenience (debug builds only): bring the backend up before the
//! GUI starts, so `cargo run -p starframe-client` is the only command you need.
//!
//! It (1) starts the local SpacetimeDB host if it isn't already running, then
//! (2) publishes the `space4x` module to it. Disable with `STARFRAME_AUTOSTART=0`.
//!
//! Requirements: the `spacetime` CLI on PATH (**2.3.0**, to match the project —
//! the publish step uses `--module-path`), and the `wasm32-unknown-unknown`
//! target installed. Failures here are non-fatal: the client still launches and
//! will simply report a connection error if the backend isn't ready.

use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const SERVER_ADDR: &str = "127.0.0.1:3000";
const DB_NAME: &str = "space4x";
const READY_TIMEOUT: Duration = Duration::from_secs(30);

/// Start (if needed) and publish to the local host. Best-effort; never panics.
pub fn bootstrap() {
    if std::env::var("STARFRAME_AUTOSTART").as_deref() == Ok("0") {
        return;
    }
    if !have_spacetime() {
        eprintln!(
            "[dev] `spacetime` CLI not found on PATH — skipping autostart. Start the \
             backend manually, or set STARFRAME_AUTOSTART=0 to silence this."
        );
        return;
    }

    if server_up() {
        eprintln!("[dev] local SpacetimeDB already running on {SERVER_ADDR}.");
    } else if !start_host() {
        return;
    }

    // Target the local server by default (best-effort), then publish the module.
    let _ = Command::new("spacetime")
        .args(["server", "set-default", "local"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    eprintln!("[dev] publishing `{DB_NAME}` module…");
    let module_path = repo_root().join("server");
    let status = Command::new("spacetime")
        .current_dir(repo_root())
        .args(["publish", DB_NAME, "--server", "local", "--yes", "--module-path"])
        .arg(&module_path)
        .status();
    match status {
        Ok(s) if s.success() => eprintln!("[dev] published `{DB_NAME}` — launching client…"),
        Ok(s) => eprintln!(
            "[dev] `spacetime publish` failed ({s}). If your CLI is older than 2.3.0, \
             `--module-path` is unsupported — upgrade with `spacetime version use 2.3.0` \
             (and ensure `rustup target add wasm32-unknown-unknown`). Launching anyway…"
        ),
        Err(e) => eprintln!("[dev] couldn't run `spacetime publish`: {e}. Launching anyway…"),
    }
}

/// Spawn the host detached and wait until it accepts connections. Returns false
/// if it couldn't be launched or never became ready.
fn start_host() -> bool {
    eprintln!("[dev] starting local SpacetimeDB host…");
    if let Err(e) = Command::new("spacetime")
        .arg("start")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        eprintln!("[dev] failed to launch `spacetime start`: {e}. Skipping autostart.");
        return false;
    }
    let deadline = Instant::now() + READY_TIMEOUT;
    while !server_up() {
        if Instant::now() >= deadline {
            eprintln!("[dev] host didn't become ready within 30s; the client may fail to connect.");
            return false;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    eprintln!("[dev] host is up (it will keep running in the background).");
    true
}

fn server_up() -> bool {
    SERVER_ADDR
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
        .map(|addr| TcpStream::connect_timeout(&addr, Duration::from_millis(400)).is_ok())
        .unwrap_or(false)
}

fn have_spacetime() -> bool {
    Command::new("spacetime")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Workspace root = the client crate's parent directory (baked at build time).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

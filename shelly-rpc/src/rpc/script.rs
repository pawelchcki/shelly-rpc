//! `Script.*` methods — on-device JavaScript scripting.

use core::fmt::Write;

use serde::Deserialize;

use super::{path, Path};
use crate::error::Error;

// ── Paths (GET-capable methods) ────────────────────────────────────────

/// `/rpc/Script.List`
pub fn list_path() -> Path {
    let mut p = Path::new();
    let _ = p.push_str("/rpc/Script.List");
    p
}

/// `/rpc/Script.GetStatus?id=<id>`
pub fn get_status_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Script.GetStatus?id=", |w| write!(w, "{id}"))
}

/// `/rpc/Script.GetConfig?id=<id>`
pub fn get_config_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Script.GetConfig?id=", |w| write!(w, "{id}"))
}

/// `/rpc/Script.Start?id=<id>`
pub fn start_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Script.Start?id=", |w| write!(w, "{id}"))
}

/// `/rpc/Script.Stop?id=<id>`
pub fn stop_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Script.Stop?id=", |w| write!(w, "{id}"))
}

/// `/rpc/Script.Delete?id=<id>`
pub fn delete_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Script.Delete?id=", |w| write!(w, "{id}"))
}

// ── Response types ─────────────────────────────────────────────────────

/// Response from `Script.List`.
#[derive(Debug, Clone, Deserialize)]
pub struct ScriptList<'a> {
    /// List of scripts on the device.
    #[serde(borrow)]
    pub scripts: heapless::Vec<ScriptInfo<'a>, 32>,
}

/// A single script entry from `Script.List`.
#[derive(Debug, Clone, Deserialize)]
pub struct ScriptInfo<'a> {
    /// Script slot ID.
    pub id: u32,
    /// Script name.
    #[serde(default)]
    pub name: Option<&'a str>,
    /// Whether the script auto-starts on boot.
    #[serde(default)]
    pub enable: bool,
    /// Whether the script is currently running.
    #[serde(default)]
    pub running: bool,
}

/// Response from `Script.Create`.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ScriptCreated {
    /// The ID of the newly created script slot.
    pub id: u32,
}

/// Response from `Script.PutCode`.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct PutCodeResult {
    /// Number of bytes written.
    pub len: u32,
}

/// Response from `Script.GetCode`.
#[derive(Debug, Clone, Deserialize)]
pub struct ScriptCode<'a> {
    /// The script source code (or a chunk of it).
    pub data: &'a str,
    /// Bytes remaining after this chunk.
    pub left: u32,
}

/// Response from `Script.Start` / `Script.Stop`.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ScriptRunState {
    /// Whether the script was running before the call.
    pub was_running: bool,
}

/// Response from `Script.GetStatus`.
#[derive(Debug, Clone, Deserialize)]
pub struct ScriptStatus<'a> {
    /// Script slot ID.
    pub id: u32,
    /// Whether the script is currently running.
    #[serde(default)]
    pub running: bool,
    /// Error messages, if any.
    #[serde(default)]
    pub errors: Option<&'a str>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_and_parameterised_paths() {
        assert_eq!(list_path().as_str(), "/rpc/Script.List");
        assert_eq!(start_path(1).unwrap().as_str(), "/rpc/Script.Start?id=1");
        assert_eq!(stop_path(2).unwrap().as_str(), "/rpc/Script.Stop?id=2");
        assert_eq!(delete_path(3).unwrap().as_str(), "/rpc/Script.Delete?id=3");
    }
}

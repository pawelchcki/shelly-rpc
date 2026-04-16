//! `Sys.*` methods — system-level status and configuration.

use serde::Deserialize;

use super::common::AvailableUpdate;
use super::Path;

// ── Paths ──────────────────────────────────────────────────────────────

/// `/rpc/Sys.GetStatus`
pub fn get_status_path() -> Path {
    let mut p = Path::new();
    let _ = p.push_str("/rpc/Sys.GetStatus");
    p
}

/// `/rpc/Sys.GetConfig`
pub fn get_config_path() -> Path {
    let mut p = Path::new();
    let _ = p.push_str("/rpc/Sys.GetConfig");
    p
}

// ── Response types ─────────────────────────────────────────────────────

/// Response from `Sys.GetStatus`.
#[derive(Debug, Clone, Deserialize)]
pub struct SysStatus<'a> {
    /// MAC address.
    #[serde(default)]
    pub mac: Option<&'a str>,
    /// Whether a restart is required.
    #[serde(default)]
    pub restart_required: bool,
    /// Uptime in seconds.
    #[serde(default)]
    pub uptime: Option<u64>,
    /// Total RAM in bytes.
    #[serde(default)]
    pub ram_size: Option<u32>,
    /// Free RAM in bytes.
    #[serde(default)]
    pub ram_free: Option<u32>,
    /// Total filesystem space in bytes.
    #[serde(default)]
    pub fs_size: Option<u32>,
    /// Free filesystem space in bytes.
    #[serde(default)]
    pub fs_free: Option<u32>,
    /// Available firmware updates.
    #[serde(default, borrow)]
    pub available_updates: Option<AvailableUpdate<'a>>,
}

/// Response from `Sys.GetConfig`.
#[derive(Debug, Clone, Deserialize)]
pub struct SysConfig<'a> {
    /// Device name.
    #[serde(default, borrow)]
    pub device: Option<SysDeviceConfig<'a>>,
    /// Location config.
    #[serde(default, borrow)]
    pub location: Option<SysLocationConfig<'a>>,
}

/// Device subsection of `Sys.GetConfig`.
#[derive(Debug, Clone, Deserialize)]
pub struct SysDeviceConfig<'a> {
    /// User-friendly device name.
    #[serde(default)]
    pub name: Option<&'a str>,
    /// Firmware channel.
    #[serde(default)]
    pub fw_id: Option<&'a str>,
}

/// Location subsection of `Sys.GetConfig`.
#[derive(Debug, Clone, Deserialize)]
pub struct SysLocationConfig<'a> {
    /// Timezone string.
    #[serde(default)]
    pub tz: Option<&'a str>,
    /// Latitude.
    #[serde(default)]
    pub lat: Option<f32>,
    /// Longitude.
    #[serde(default)]
    pub lon: Option<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_paths() {
        assert_eq!(get_status_path().as_str(), "/rpc/Sys.GetStatus");
        assert_eq!(get_config_path().as_str(), "/rpc/Sys.GetConfig");
    }
}

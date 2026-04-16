//! `Shelly.*` methods — device-level queries and actions.

use serde::Deserialize;

use super::common::{CloudStatus, MqttStatus};
use super::Path;

// ── Paths ──────────────────────────────────────────────────────────────

// All path literals are well under MAX_PATH_LEN. `debug_assert!` trips in
// tests if anyone shortens MAX_PATH_LEN or adds an overlong literal instead
// of silently returning an empty `Path`.
fn static_path(literal: &'static str) -> Path {
    let mut p = Path::new();
    debug_assert!(
        p.push_str(literal).is_ok(),
        "path literal {literal:?} exceeds MAX_PATH_LEN"
    );
    p
}

/// `/rpc/Shelly.GetDeviceInfo`
pub fn get_device_info_path() -> Path {
    static_path("/rpc/Shelly.GetDeviceInfo")
}

/// `/rpc/Shelly.GetStatus`
pub fn get_status_path() -> Path {
    static_path("/rpc/Shelly.GetStatus")
}

/// `/rpc/Shelly.GetConfig`
pub fn get_config_path() -> Path {
    static_path("/rpc/Shelly.GetConfig")
}

/// `/rpc/Shelly.ListMethods`
pub fn list_methods_path() -> Path {
    static_path("/rpc/Shelly.ListMethods")
}

/// `/rpc/Shelly.Reboot`
pub fn reboot_path() -> Path {
    static_path("/rpc/Shelly.Reboot")
}

/// `/rpc/Shelly.CheckForUpdate`
pub fn check_for_update_path() -> Path {
    static_path("/rpc/Shelly.CheckForUpdate")
}

/// `/rpc/Shelly.Update`
pub fn update_path() -> Path {
    static_path("/rpc/Shelly.Update")
}

// ── Response types ─────────────────────────────────────────────────────

/// Response from `Shelly.GetDeviceInfo`.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceInfo<'a> {
    /// Device name, e.g. `"shellyplus1pm-xxxxxxxxxxxx"`.
    #[serde(default)]
    pub name: Option<&'a str>,
    /// Device ID (MAC-based), e.g. `"shellyplus1pm-xxxxxxxxxxxx"`.
    pub id: &'a str,
    /// MAC address.
    pub mac: &'a str,
    /// Hardware model, e.g. `"SNSW-001P16EU"`.
    pub model: &'a str,
    /// Generation (2, 3, or 4).
    pub gen: u8,
    /// Firmware ID string.
    pub fw_id: &'a str,
    /// Firmware version string.
    pub ver: &'a str,
    /// Application name, e.g. `"Plus1PM"`.
    pub app: &'a str,
    /// Whether authentication is enabled.
    #[serde(default)]
    pub auth_en: bool,
    /// Authentication domain (if auth is enabled).
    #[serde(default)]
    pub auth_domain: Option<&'a str>,
}

/// Response from `Shelly.GetStatus`.
///
/// This is the top-level status envelope. Component-specific statuses are
/// flattened into the JSON as `"switch:0"`, `"cover:0"`, etc. For v1 we
/// only surface the metadata fields that are always present.
#[derive(Debug, Clone, Deserialize)]
pub struct ShellyStatus<'a> {
    /// System status.
    #[serde(default, borrow)]
    pub sys: Option<super::sys::SysStatus<'a>>,
    /// Wi-Fi status.
    #[serde(default, borrow)]
    pub wifi: Option<super::wifi::WifiStatus<'a>>,
    /// Cloud connectivity status.
    #[serde(default)]
    pub cloud: Option<CloudStatus>,
    /// MQTT connectivity status.
    #[serde(default)]
    pub mqtt: Option<MqttStatus>,
}

/// Response from `Shelly.ListMethods`.
#[derive(Debug, Clone, Deserialize)]
pub struct MethodList<'a> {
    /// Available RPC method names.
    #[serde(borrow)]
    pub methods: heapless::Vec<&'a str, 256>,
}

/// Response from `Shelly.CheckForUpdate`.
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateInfo<'a> {
    /// Available stable update, if any.
    #[serde(default, borrow)]
    pub stable: Option<super::common::FwVersion<'a>>,
    /// Available beta update, if any.
    #[serde(default, borrow)]
    pub beta: Option<super::common::FwVersion<'a>>,
}

// ── Path tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_paths() {
        assert_eq!(get_device_info_path().as_str(), "/rpc/Shelly.GetDeviceInfo");
        assert_eq!(get_status_path().as_str(), "/rpc/Shelly.GetStatus");
        assert_eq!(list_methods_path().as_str(), "/rpc/Shelly.ListMethods");
        assert_eq!(reboot_path().as_str(), "/rpc/Shelly.Reboot");
        assert_eq!(
            check_for_update_path().as_str(),
            "/rpc/Shelly.CheckForUpdate"
        );
        assert_eq!(update_path().as_str(), "/rpc/Shelly.Update");
    }
}

//! `Wifi.*` methods — Wi-Fi status, config, and scanning.

use serde::Deserialize;

use super::Path;

// ── Paths ──────────────────────────────────────────────────────────────

/// `/rpc/Wifi.GetStatus`
pub fn get_status_path() -> Path {
    let mut p = Path::new();
    let _ = p.push_str("/rpc/Wifi.GetStatus");
    p
}

/// `/rpc/Wifi.GetConfig`
pub fn get_config_path() -> Path {
    let mut p = Path::new();
    let _ = p.push_str("/rpc/Wifi.GetConfig");
    p
}

/// `/rpc/Wifi.Scan`
pub fn scan_path() -> Path {
    let mut p = Path::new();
    let _ = p.push_str("/rpc/Wifi.Scan");
    p
}

// ── Response types ─────────────────────────────────────────────────────

/// Response from `Wifi.GetStatus`.
#[derive(Debug, Clone, Deserialize)]
pub struct WifiStatus<'a> {
    /// Current station IP address.
    #[serde(default)]
    pub sta_ip: Option<&'a str>,
    /// Current SSID.
    #[serde(default)]
    pub ssid: Option<&'a str>,
    /// Signal strength in dBm.
    #[serde(default)]
    pub rssi: Option<i32>,
    /// Connection status.
    #[serde(default)]
    pub status: Option<&'a str>,
}

/// Response from `Wifi.Scan` — a wrapper around the `results` array.
#[derive(Debug, Clone, Deserialize)]
pub struct WifiScanResults<'a> {
    /// Scan results.
    #[serde(borrow)]
    pub results: heapless::Vec<WifiAp<'a>, 32>,
}

/// A single Wi-Fi access point from a scan.
#[derive(Debug, Clone, Deserialize)]
pub struct WifiAp<'a> {
    /// SSID.
    #[serde(default)]
    pub ssid: Option<&'a str>,
    /// BSSID.
    #[serde(default)]
    pub bssid: Option<&'a str>,
    /// Authentication type.
    #[serde(default)]
    pub auth: Option<u8>,
    /// Channel number.
    #[serde(default)]
    pub channel: Option<u8>,
    /// RSSI in dBm.
    #[serde(default)]
    pub rssi: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_paths() {
        assert_eq!(get_status_path().as_str(), "/rpc/Wifi.GetStatus");
        assert_eq!(get_config_path().as_str(), "/rpc/Wifi.GetConfig");
        assert_eq!(scan_path().as_str(), "/rpc/Wifi.Scan");
    }
}

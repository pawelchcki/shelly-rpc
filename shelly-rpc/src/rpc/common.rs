//! Types shared across multiple component families.

use serde::Deserialize;

/// Firmware update status, embedded in many `GetStatus` replies.
#[derive(Debug, Clone, Deserialize)]
pub struct AvailableUpdate<'a> {
    /// Stable firmware channel info.
    #[serde(default, borrow)]
    pub stable: Option<FwVersion<'a>>,
    /// Beta firmware channel info.
    #[serde(default, borrow)]
    pub beta: Option<FwVersion<'a>>,
}

/// A single firmware version descriptor.
#[derive(Debug, Clone, Deserialize)]
pub struct FwVersion<'a> {
    /// Version string, e.g. `"1.4.4-g6d2a586"`.
    pub version: &'a str,
    /// Build ID.
    #[serde(default)]
    pub build_id: Option<&'a str>,
}

/// Cloud connectivity status.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct CloudStatus {
    /// Whether cloud connectivity is enabled in config.
    pub connected: bool,
}

/// MQTT connectivity status.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct MqttStatus {
    /// Whether MQTT is connected.
    pub connected: bool,
}

/// Active energy counter, returned by metered components.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ActiveEnergy {
    /// Total energy in watt-hours.
    pub total: f32,
    /// Energy by minute (last 3 minutes), oldest first.
    #[serde(default)]
    pub by_minute: Option<[f32; 3]>,
    /// Unix timestamp of the last minute boundary.
    #[serde(default)]
    pub minute_ts: Option<u64>,
}

/// Temperature measurement in both units.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Temperature {
    /// Temperature in degrees Celsius.
    #[serde(rename = "tC")]
    pub celsius: Option<f32>,
    /// Temperature in degrees Fahrenheit.
    #[serde(rename = "tF")]
    pub fahrenheit: Option<f32>,
}

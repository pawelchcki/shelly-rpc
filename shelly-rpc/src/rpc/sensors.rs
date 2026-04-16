//! Sensor component families — Temperature, Humidity, DevicePower, Voltmeter.

use core::fmt::Write;

use serde::Deserialize;

use super::{path, Path};
use crate::error::Error;

// ── Temperature ────────────────────────────────────────────────────────

/// `/rpc/Temperature.GetStatus?id=<id>`
pub fn temperature_get_status_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Temperature.GetStatus?id=", |w| write!(w, "{id}"))
}

/// `/rpc/Temperature.GetConfig?id=<id>`
pub fn temperature_get_config_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Temperature.GetConfig?id=", |w| write!(w, "{id}"))
}

/// Response from `Temperature.GetStatus`.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct TemperatureStatus {
    /// Component instance ID.
    pub id: u32,
    /// Temperature in degrees Celsius.
    #[serde(default, rename = "tC")]
    pub celsius: Option<f32>,
    /// Temperature in degrees Fahrenheit.
    #[serde(default, rename = "tF")]
    pub fahrenheit: Option<f32>,
}

// ── Humidity ───────────────────────────────────────────────────────────

/// `/rpc/Humidity.GetStatus?id=<id>`
pub fn humidity_get_status_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Humidity.GetStatus?id=", |w| write!(w, "{id}"))
}

/// `/rpc/Humidity.GetConfig?id=<id>`
pub fn humidity_get_config_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Humidity.GetConfig?id=", |w| write!(w, "{id}"))
}

/// Response from `Humidity.GetStatus`.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct HumidityStatus {
    /// Component instance ID.
    pub id: u32,
    /// Relative humidity in percent.
    #[serde(default)]
    pub rh: Option<f32>,
}

// ── DevicePower ────────────────────────────────────────────────────────

/// `/rpc/DevicePower.GetStatus?id=<id>`
pub fn device_power_get_status_path(id: u32) -> Result<Path, Error> {
    path("/rpc/DevicePower.GetStatus?id=", |w| write!(w, "{id}"))
}

/// Response from `DevicePower.GetStatus`.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct DevicePowerStatus {
    /// Component instance ID.
    pub id: u32,
    /// Battery info.
    #[serde(default)]
    pub battery: Option<BatteryInfo>,
    /// External power info.
    #[serde(default)]
    pub external: Option<ExternalPowerInfo>,
}

/// Battery information.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct BatteryInfo {
    /// Battery voltage in volts.
    #[serde(default, rename = "V")]
    pub voltage: Option<f32>,
    /// Battery level in percent.
    #[serde(default)]
    pub percent: Option<f32>,
}

/// External power information.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ExternalPowerInfo {
    /// Whether external power is present.
    #[serde(default)]
    pub present: Option<bool>,
}

// ── Voltmeter ──────────────────────────────────────────────────────────

/// `/rpc/Voltmeter.GetStatus?id=<id>`
pub fn voltmeter_get_status_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Voltmeter.GetStatus?id=", |w| write!(w, "{id}"))
}

/// `/rpc/Voltmeter.GetConfig?id=<id>`
pub fn voltmeter_get_config_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Voltmeter.GetConfig?id=", |w| write!(w, "{id}"))
}

/// Response from `Voltmeter.GetStatus`.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct VoltmeterStatus {
    /// Component instance ID.
    pub id: u32,
    /// Measured voltage in volts.
    #[serde(default)]
    pub voltage: Option<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameterised_paths() {
        assert_eq!(
            temperature_get_status_path(0).unwrap().as_str(),
            "/rpc/Temperature.GetStatus?id=0"
        );
        assert_eq!(
            humidity_get_status_path(0).unwrap().as_str(),
            "/rpc/Humidity.GetStatus?id=0"
        );
        assert_eq!(
            device_power_get_status_path(0).unwrap().as_str(),
            "/rpc/DevicePower.GetStatus?id=0"
        );
        assert_eq!(
            voltmeter_get_status_path(0).unwrap().as_str(),
            "/rpc/Voltmeter.GetStatus?id=0"
        );
    }
}

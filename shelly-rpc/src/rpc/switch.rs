//! `Switch.*` methods — relay/switch control and status.

use core::fmt::Write;

use serde::Deserialize;

use super::common::ActiveEnergy;
use super::{path, Path};
use crate::error::Error;

// ── Paths ──────────────────────────────────────────────────────────────

/// `/rpc/Switch.GetStatus?id=<id>`
pub fn get_status_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Switch.GetStatus?id=", |w| write!(w, "{id}"))
}

/// `/rpc/Switch.GetConfig?id=<id>`
pub fn get_config_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Switch.GetConfig?id=", |w| write!(w, "{id}"))
}

/// `/rpc/Switch.Set?id=<id>&on=<on>`
pub fn set_path(id: u32, on: bool) -> Result<Path, Error> {
    path("/rpc/Switch.Set?id=", |w| write!(w, "{id}&on={on}"))
}

/// `/rpc/Switch.Toggle?id=<id>`
pub fn toggle_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Switch.Toggle?id=", |w| write!(w, "{id}"))
}

/// `/rpc/Switch.ResetCounters?id=<id>`
pub fn reset_counters_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Switch.ResetCounters?id=", |w| write!(w, "{id}"))
}

// ── Response types ─────────────────────────────────────────────────────

/// Response from `Switch.GetStatus`.
#[derive(Debug, Clone, Deserialize)]
pub struct SwitchStatus<'a> {
    /// Component instance ID.
    pub id: u32,
    /// Data source (e.g. `"switch:0"`).
    #[serde(default)]
    pub source: Option<&'a str>,
    /// Whether the switch is on.
    #[serde(default)]
    pub output: Option<bool>,
    /// Active power in watts.
    #[serde(default)]
    pub apower: Option<f32>,
    /// Voltage in volts.
    #[serde(default)]
    pub voltage: Option<f32>,
    /// Current in amps.
    #[serde(default)]
    pub current: Option<f32>,
    /// Power factor.
    #[serde(default)]
    pub pf: Option<f32>,
    /// Accumulated energy.
    #[serde(default)]
    pub aenergy: Option<ActiveEnergy>,
    /// Internal temperature.
    #[serde(default)]
    pub temperature: Option<super::common::Temperature>,
}

/// Response from `Switch.GetConfig`.
#[derive(Debug, Clone, Deserialize)]
pub struct SwitchConfig<'a> {
    /// Component instance ID.
    pub id: u32,
    /// User-friendly name.
    #[serde(default)]
    pub name: Option<&'a str>,
    /// Input mode: `"follow"`, `"flip"`, `"activate"`, `"detached"`, `"momentary"`.
    #[serde(default)]
    pub in_mode: Option<&'a str>,
    /// Initial state on power-on: `"off"`, `"on"`, `"restore_last"`, `"match_input"`.
    #[serde(default)]
    pub initial_state: Option<&'a str>,
    /// Auto-on enabled.
    #[serde(default)]
    pub auto_on: Option<bool>,
    /// Auto-on delay in seconds.
    #[serde(default)]
    pub auto_on_delay: Option<f32>,
    /// Auto-off enabled.
    #[serde(default)]
    pub auto_off: Option<bool>,
    /// Auto-off delay in seconds.
    #[serde(default)]
    pub auto_off_delay: Option<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameterised_paths() {
        assert_eq!(
            get_status_path(0).unwrap().as_str(),
            "/rpc/Switch.GetStatus?id=0"
        );
        assert_eq!(
            set_path(1, true).unwrap().as_str(),
            "/rpc/Switch.Set?id=1&on=true"
        );
        assert_eq!(toggle_path(2).unwrap().as_str(), "/rpc/Switch.Toggle?id=2");
    }
}

//! `Cover.*` methods — roller-shutter / cover control and status.

use core::fmt::Write;

use serde::Deserialize;

use super::common::ActiveEnergy;
use super::{path, Path};
use crate::error::Error;

// ── Paths ──────────────────────────────────────────────────────────────

/// `/rpc/Cover.GetStatus?id=<id>`
pub fn get_status_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Cover.GetStatus?id=", |w| write!(w, "{id}"))
}

/// `/rpc/Cover.GetConfig?id=<id>`
pub fn get_config_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Cover.GetConfig?id=", |w| write!(w, "{id}"))
}

/// `/rpc/Cover.Open?id=<id>`
pub fn open_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Cover.Open?id=", |w| write!(w, "{id}"))
}

/// `/rpc/Cover.Close?id=<id>`
pub fn close_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Cover.Close?id=", |w| write!(w, "{id}"))
}

/// `/rpc/Cover.Stop?id=<id>`
pub fn stop_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Cover.Stop?id=", |w| write!(w, "{id}"))
}

/// `/rpc/Cover.GoToPosition?id=<id>&pos=<pos>`
pub fn go_to_position_path(id: u32, pos: u32) -> Result<Path, Error> {
    path("/rpc/Cover.GoToPosition?id=", |w| {
        write!(w, "{id}&pos={pos}")
    })
}

/// `/rpc/Cover.Calibrate?id=<id>`
pub fn calibrate_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Cover.Calibrate?id=", |w| write!(w, "{id}"))
}

// ── Response types ─────────────────────────────────────────────────────

/// Response from `Cover.GetStatus`.
#[derive(Debug, Clone, Deserialize)]
pub struct CoverStatus<'a> {
    /// Component instance ID.
    pub id: u32,
    /// Data source.
    #[serde(default)]
    pub source: Option<&'a str>,
    /// Current state: `"open"`, `"closed"`, `"opening"`, `"closing"`, `"stopped"`, `"calibrating"`.
    #[serde(default)]
    pub state: Option<&'a str>,
    /// Current position in percent (0 = closed, 100 = open). Only
    /// available after calibration.
    #[serde(default)]
    pub current_pos: Option<u32>,
    /// Target position in percent, while moving.
    #[serde(default)]
    pub target_pos: Option<u32>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameterised_paths() {
        assert_eq!(
            get_status_path(0).unwrap().as_str(),
            "/rpc/Cover.GetStatus?id=0"
        );
        assert_eq!(open_path(0).unwrap().as_str(), "/rpc/Cover.Open?id=0");
        assert_eq!(
            go_to_position_path(0, 50).unwrap().as_str(),
            "/rpc/Cover.GoToPosition?id=0&pos=50"
        );
    }
}

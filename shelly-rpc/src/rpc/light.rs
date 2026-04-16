//! `Light.*` methods — dimmable light control and status.

use core::fmt::Write;

use serde::Deserialize;

use super::{path, Path};
use crate::error::Error;

// ── Paths ──────────────────────────────────────────────────────────────

/// `/rpc/Light.GetStatus?id=<id>`
pub fn get_status_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Light.GetStatus?id=", |w| write!(w, "{id}"))
}

/// `/rpc/Light.GetConfig?id=<id>`
pub fn get_config_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Light.GetConfig?id=", |w| write!(w, "{id}"))
}

/// `/rpc/Light.Set?id=<id>&on=<on>`
pub fn set_path(id: u32, on: bool) -> Result<Path, Error> {
    path("/rpc/Light.Set?id=", |w| write!(w, "{id}&on={on}"))
}

/// `/rpc/Light.Set?id=<id>&on=true&brightness=<brightness>`
pub fn set_brightness_path(id: u32, brightness: u8) -> Result<Path, Error> {
    path("/rpc/Light.Set?id=", |w| {
        write!(w, "{id}&on=true&brightness={brightness}")
    })
}

/// `/rpc/Light.Toggle?id=<id>`
pub fn toggle_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Light.Toggle?id=", |w| write!(w, "{id}"))
}

// ── Response types ─────────────────────────────────────────────────────

/// Response from `Light.GetStatus`.
#[derive(Debug, Clone, Deserialize)]
pub struct LightStatus<'a> {
    /// Component instance ID.
    pub id: u32,
    /// Data source.
    #[serde(default)]
    pub source: Option<&'a str>,
    /// Whether the light is on.
    #[serde(default)]
    pub output: Option<bool>,
    /// Brightness level (0–100).
    #[serde(default)]
    pub brightness: Option<u8>,
    /// Internal temperature.
    #[serde(default)]
    pub temperature: Option<super::common::Temperature>,
}

/// Response from `Light.GetConfig`.
#[derive(Debug, Clone, Deserialize)]
pub struct LightConfig<'a> {
    /// Component instance ID.
    pub id: u32,
    /// User-friendly name.
    #[serde(default)]
    pub name: Option<&'a str>,
    /// Initial state on power-on.
    #[serde(default)]
    pub initial_state: Option<&'a str>,
    /// Auto-off enabled.
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
    /// Default brightness on start.
    #[serde(default)]
    pub default_brightness: Option<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameterised_paths() {
        assert_eq!(
            get_status_path(0).unwrap().as_str(),
            "/rpc/Light.GetStatus?id=0"
        );
        assert_eq!(
            set_brightness_path(0, 75).unwrap().as_str(),
            "/rpc/Light.Set?id=0&on=true&brightness=75"
        );
    }
}

//! `Input.*` methods — digital/analog input status and configuration.

use core::fmt::Write;

use serde::Deserialize;

use super::{path, Path};
use crate::error::Error;

// ── Paths ──────────────────────────────────────────────────────────────

/// `/rpc/Input.GetStatus?id=<id>`
pub fn get_status_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Input.GetStatus?id=", |w| write!(w, "{id}"))
}

/// `/rpc/Input.GetConfig?id=<id>`
pub fn get_config_path(id: u32) -> Result<Path, Error> {
    path("/rpc/Input.GetConfig?id=", |w| write!(w, "{id}"))
}

// ── Response types ─────────────────────────────────────────────────────

/// Response from `Input.GetStatus`.
#[derive(Debug, Clone, Deserialize)]
pub struct InputStatus<'a> {
    /// Component instance ID.
    pub id: u32,
    /// Current state (boolean for digital inputs, absent for analog).
    #[serde(default)]
    pub state: Option<bool>,
    /// Analog input percent (0–100), only for analog inputs.
    #[serde(default)]
    pub percent: Option<f32>,
    /// Data source.
    #[serde(default)]
    pub source: Option<&'a str>,
}

/// Response from `Input.GetConfig`.
#[derive(Debug, Clone, Deserialize)]
pub struct InputConfig<'a> {
    /// Component instance ID.
    pub id: u32,
    /// User-friendly name.
    #[serde(default)]
    pub name: Option<&'a str>,
    /// Input type: `"switch"`, `"button"`, `"analog"`.
    #[serde(default, rename = "type")]
    pub input_type: Option<&'a str>,
    /// Whether the input logic is inverted.
    #[serde(default)]
    pub invert: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameterised_paths() {
        assert_eq!(
            get_status_path(0).unwrap().as_str(),
            "/rpc/Input.GetStatus?id=0"
        );
        assert_eq!(
            get_config_path(1).unwrap().as_str(),
            "/rpc/Input.GetConfig?id=1"
        );
    }
}

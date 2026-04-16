//! Fixture compatibility tests.
//!
//! These load real responses captured from a Shelly Power Strip Gen4
//! (via `shelly record`) and verify they parse through the same structs
//! that the doc-based tests validate. No field assertions — just "does
//! the real device's JSON shape fit our types?"
//!
//! If a firmware update adds a field our struct doesn't expect, serde
//! will silently ignore it. If firmware *removes* a required field,
//! these tests catch it.

use shelly_rpc::rpc;

macro_rules! fixture {
    ($path:literal) => {
        include_bytes!(concat!("fixtures/PowerStrip/", $path))
    };
}

// ── Shelly.* ───────────────────────────────────────────────────────────

#[test]
fn fixture_shelly_get_device_info() {
    let info: rpc::shelly::DeviceInfo<'_> =
        rpc::parse(fixture!("Shelly.GetDeviceInfo.json")).unwrap();
    assert_eq!(info.gen, 4);
    assert_eq!(info.app, "PowerStrip");
}

#[test]
fn fixture_shelly_get_status() {
    let _: rpc::shelly::ShellyStatus<'_> = rpc::parse(fixture!("Shelly.GetStatus.json")).unwrap();
}

#[test]
fn fixture_shelly_list_methods() {
    let ml: rpc::shelly::MethodList<'_> = rpc::parse(fixture!("Shelly.ListMethods.json")).unwrap();
    // A Gen4 power strip exposes 100+ methods.
    assert!(ml.methods.len() > 100);
}

// ── Sys.* ──────────────────────────────────────────────────────────────

#[test]
fn fixture_sys_get_status() {
    let sys: rpc::sys::SysStatus<'_> = rpc::parse(fixture!("Sys.GetStatus.json")).unwrap();
    assert!(sys.uptime.unwrap() > 0);
}

#[test]
fn fixture_sys_get_config() {
    let _: rpc::sys::SysConfig<'_> = rpc::parse(fixture!("Sys.GetConfig.json")).unwrap();
}

// ── Wifi.* ─────────────────────────────────────────────────────────────

#[test]
fn fixture_wifi_get_status() {
    let wifi: rpc::wifi::WifiStatus<'_> = rpc::parse(fixture!("Wifi.GetStatus.json")).unwrap();
    assert!(wifi.sta_ip.is_some());
}

// ── Switch.* (4 outlets) ───────────────────────────────────────────────

#[test]
fn fixture_switch_get_status_all() {
    for (i, body) in [
        &fixture!("Switch.GetStatus_0.json")[..],
        &fixture!("Switch.GetStatus_1.json")[..],
        &fixture!("Switch.GetStatus_2.json")[..],
        &fixture!("Switch.GetStatus_3.json")[..],
    ]
    .iter()
    .enumerate()
    {
        let s: rpc::switch::SwitchStatus<'_> =
            rpc::parse(body).unwrap_or_else(|e| panic!("switch {i}: {e}"));
        assert_eq!(s.id, i as u32);
    }
}

#[test]
fn fixture_switch_get_config_all() {
    for (i, body) in [
        &fixture!("Switch.GetConfig_0.json")[..],
        &fixture!("Switch.GetConfig_1.json")[..],
        &fixture!("Switch.GetConfig_2.json")[..],
        &fixture!("Switch.GetConfig_3.json")[..],
    ]
    .iter()
    .enumerate()
    {
        let c: rpc::switch::SwitchConfig<'_> =
            rpc::parse(body).unwrap_or_else(|e| panic!("switch {i}: {e}"));
        assert_eq!(c.id, i as u32);
    }
}

//! Parser tests using example payloads from the Shelly Gen2+ API docs.
//!
//! Each test embeds a JSON literal taken from
//! <https://shelly-api-docs.shelly.cloud/gen2/> and verifies that our
//! response structs deserialize it correctly. No async, no networking,
//! no fixture files.

use shelly_rpc::rpc;

// ── Shelly.GetDeviceInfo ───────────────────────────────────────────────
// https://shelly-api-docs.shelly.cloud/gen2/ComponentsAndServices/Shelly#shellygetdeviceinfo

#[test]
fn parse_device_info_from_docs() {
    let body = br#"{
        "id": "shellypro4pm-f008d1d8b8b8",
        "mac": "F008D1D8B8B8",
        "model": "SPSW-004PE16EU",
        "gen": 2,
        "fw_id": "20210720-153353/0.6.7-gc36674b",
        "ver": "0.6.7",
        "app": "FourPro",
        "auth_en": true,
        "auth_domain": "shellypro4pm-f008d1d8b8b8"
    }"#;
    let info: rpc::shelly::DeviceInfo<'_> = rpc::parse(body).unwrap();
    assert_eq!(info.id, "shellypro4pm-f008d1d8b8b8");
    assert_eq!(info.mac, "F008D1D8B8B8");
    assert_eq!(info.model, "SPSW-004PE16EU");
    assert_eq!(info.gen, 2);
    assert_eq!(info.app, "FourPro");
    assert_eq!(info.ver, "0.6.7");
    assert!(info.auth_en);
    assert_eq!(info.auth_domain, Some("shellypro4pm-f008d1d8b8b8"));
    assert_eq!(info.name, None);
}

// ── Shelly.GetStatus ──────────────────────────────────────────────────
// https://shelly-api-docs.shelly.cloud/gen2/ComponentsAndServices/Shelly#shellygetstatus

#[test]
fn parse_shelly_status_from_docs() {
    // The full GetStatus envelope contains dynamic keys like "switch:0",
    // "input:0" etc. Our ShellyStatus only picks out the typed top-level
    // fields (sys, wifi, cloud, mqtt). Unknown keys are silently ignored
    // by serde.
    let body = br#"{
        "cloud": { "connected": false },
        "mqtt":  { "connected": false },
        "sys": {
            "mac": "A8032ABE54DC",
            "restart_required": false,
            "time": "16:06",
            "unixtime": 1650035219,
            "uptime": 11081
        },
        "wifi": {
            "sta_ip": null,
            "status": "disconnected"
        }
    }"#;
    let s: rpc::shelly::ShellyStatus<'_> = rpc::parse(body).unwrap();

    let sys = s.sys.unwrap();
    assert_eq!(sys.mac, Some("A8032ABE54DC"));
    assert!(!sys.restart_required);
    assert_eq!(sys.uptime, Some(11081));

    let wifi = s.wifi.unwrap();
    assert_eq!(wifi.sta_ip, None);
    assert_eq!(wifi.status, Some("disconnected"));

    assert!(!s.cloud.unwrap().connected);
    assert!(!s.mqtt.unwrap().connected);
}

// ── Shelly.ListMethods ────────────────────────────────────────────────

#[test]
fn parse_list_methods_from_docs() {
    let body = br#"{
        "methods": [
            "Shelly.GetDeviceInfo",
            "Shelly.GetStatus",
            "Shelly.GetConfig",
            "Shelly.ListMethods",
            "Shelly.Reboot",
            "Switch.GetStatus",
            "Switch.GetConfig",
            "Switch.Set",
            "Switch.Toggle",
            "Sys.GetStatus",
            "Sys.GetConfig",
            "Wifi.GetStatus",
            "Wifi.GetConfig"
        ]
    }"#;
    let ml: rpc::shelly::MethodList<'_> = rpc::parse(body).unwrap();
    assert_eq!(ml.methods.len(), 13);
    assert!(ml.methods.contains(&"Switch.Toggle"));
    assert!(ml.methods.contains(&"Shelly.Reboot"));
}

// ── Switch.GetStatus ──────────────────────────────────────────────────
// https://shelly-api-docs.shelly.cloud/gen2/ComponentsAndServices/Switch#status

#[test]
fn parse_switch_status_from_docs() {
    let body = br#"{
        "id": 0,
        "source": "WS_in",
        "output": false,
        "apower": 0,
        "voltage": 225.9,
        "current": 0,
        "freq": 50,
        "aenergy": {
            "total": 11.679,
            "by_minute": [0, 0, 0],
            "minute_ts": 1654511972
        },
        "ret_aenergy": {
            "total": 5.817,
            "by_minute": [0, 0, 0],
            "minute_ts": 1654511615
        },
        "temperature": {
            "tC": 53.3,
            "tF": 127.9
        }
    }"#;
    let s: rpc::switch::SwitchStatus<'_> = rpc::parse(body).unwrap();
    assert_eq!(s.id, 0);
    assert_eq!(s.source, Some("WS_in"));
    assert_eq!(s.output, Some(false));
    assert_eq!(s.voltage, Some(225.9));
    assert_eq!(s.current, Some(0.0));

    let energy = s.aenergy.unwrap();
    assert!(energy.total > 11.0);
    assert_eq!(energy.minute_ts, Some(1654511972));

    let temp = s.temperature.unwrap();
    assert_eq!(temp.celsius, Some(53.3));
    assert_eq!(temp.fahrenheit, Some(127.9));
}

// ── Switch.GetConfig ──────────────────────────────────────────────────
// https://shelly-api-docs.shelly.cloud/gen2/ComponentsAndServices/Switch#configuration

#[test]
fn parse_switch_config_from_docs() {
    let body = br#"{
        "id": 0,
        "name": "Switchdsds",
        "in_mode": "follow",
        "initial_state": "match_input",
        "auto_on": false,
        "auto_on_delay": 60,
        "auto_off": false,
        "auto_off_delay": 60,
        "power_limit": 4480,
        "voltage_limit": 280,
        "current_limit": 16
    }"#;
    let c: rpc::switch::SwitchConfig<'_> = rpc::parse(body).unwrap();
    assert_eq!(c.id, 0);
    assert_eq!(c.name, Some("Switchdsds"));
    assert_eq!(c.in_mode, Some("follow"));
    assert_eq!(c.initial_state, Some("match_input"));
    assert_eq!(c.auto_on, Some(false));
    assert_eq!(c.auto_off, Some(false));
    assert_eq!(c.auto_off_delay, Some(60.0));
}

// ── Cover.GetStatus ───────────────────────────────────────────────────
// https://shelly-api-docs.shelly.cloud/gen2/ComponentsAndServices/Cover#status

#[test]
fn parse_cover_status_from_docs() {
    let body = br#"{
        "id": 0,
        "source": "limit_switch",
        "state": "open",
        "apower": 0,
        "voltage": 233,
        "current": 0,
        "pf": 0,
        "freq": 50,
        "aenergy": {
            "total": 48.996,
            "by_minute": [0, 0, 0],
            "minute_ts": 1654604045
        },
        "temperature": {
            "tC": 55.4,
            "tF": 131.7
        },
        "pos_control": true,
        "last_direction": "open",
        "current_pos": 100
    }"#;
    let c: rpc::cover::CoverStatus<'_> = rpc::parse(body).unwrap();
    assert_eq!(c.id, 0);
    assert_eq!(c.source, Some("limit_switch"));
    assert_eq!(c.state, Some("open"));
    assert_eq!(c.current_pos, Some(100));
    assert_eq!(c.voltage, Some(233.0));

    let energy = c.aenergy.unwrap();
    assert!(energy.total > 48.0);

    let temp = c.temperature.unwrap();
    assert_eq!(temp.celsius, Some(55.4));
}

// ── Light.GetStatus ───────────────────────────────────────────────────
// https://shelly-api-docs.shelly.cloud/gen2/ComponentsAndServices/Light#status

#[test]
fn parse_light_status_from_docs() {
    let body = br#"{
        "id": 0,
        "source": "timer",
        "output": false,
        "brightness": 50,
        "timer_started_at": 1626942399.36,
        "timer_duration": 60
    }"#;
    let l: rpc::light::LightStatus<'_> = rpc::parse(body).unwrap();
    assert_eq!(l.id, 0);
    assert_eq!(l.source, Some("timer"));
    assert_eq!(l.output, Some(false));
    assert_eq!(l.brightness, Some(50));
}

// ── Input.GetStatus ───────────────────────────────────────────────────
// https://shelly-api-docs.shelly.cloud/gen2/ComponentsAndServices/Input#status

#[test]
fn parse_input_status_from_docs() {
    let body = br#"{ "id": 0, "state": false }"#;
    let i: rpc::input::InputStatus<'_> = rpc::parse(body).unwrap();
    assert_eq!(i.id, 0);
    assert_eq!(i.state, Some(false));
}

// ── Sys.GetStatus ─────────────────────────────────────────────────────
// https://shelly-api-docs.shelly.cloud/gen2/ComponentsAndServices/Sys#status

#[test]
fn parse_sys_status_from_docs() {
    let body = br#"{
        "mac": "A8032ABE54DC",
        "restart_required": false,
        "time": "19:23",
        "unixtime": 1675272236,
        "uptime": 7971,
        "ram_size": 234768,
        "ram_free": 162196,
        "fs_size": 458752,
        "fs_free": 110592,
        "available_updates": {
            "beta":   { "version": "0.13.0-beta1" },
            "stable": { "version": "0.12.0" }
        }
    }"#;
    let s: rpc::sys::SysStatus<'_> = rpc::parse(body).unwrap();
    assert_eq!(s.mac, Some("A8032ABE54DC"));
    assert!(!s.restart_required);
    assert_eq!(s.uptime, Some(7971));
    assert_eq!(s.ram_size, Some(234768));
    assert_eq!(s.ram_free, Some(162196));
    assert_eq!(s.fs_size, Some(458752));
    assert_eq!(s.fs_free, Some(110592));

    let updates = s.available_updates.unwrap();
    assert_eq!(updates.stable.unwrap().version, "0.12.0");
    assert_eq!(updates.beta.unwrap().version, "0.13.0-beta1");
}

// ── Wifi.GetStatus ────────────────────────────────────────────────────
// https://shelly-api-docs.shelly.cloud/gen2/ComponentsAndServices/Wifi#status

#[test]
fn parse_wifi_status_from_docs() {
    let body = br#"{
        "sta_ip": "192.168.1.42",
        "status": "got ip",
        "ssid": "Shelly-Office",
        "rssi": -52
    }"#;
    let w: rpc::wifi::WifiStatus<'_> = rpc::parse(body).unwrap();
    assert_eq!(w.sta_ip, Some("192.168.1.42"));
    assert_eq!(w.status, Some("got ip"));
    assert_eq!(w.ssid, Some("Shelly-Office"));
    assert_eq!(w.rssi, Some(-52));
}

#[test]
fn parse_wifi_status_disconnected() {
    let body = br#"{ "sta_ip": null, "status": "disconnected" }"#;
    let w: rpc::wifi::WifiStatus<'_> = rpc::parse(body).unwrap();
    assert_eq!(w.sta_ip, None);
    assert_eq!(w.status, Some("disconnected"));
    assert_eq!(w.ssid, None);
}

// ── Temperature.GetStatus ─────────────────────────────────────────────
// https://shelly-api-docs.shelly.cloud/gen2/ComponentsAndServices/Temperature#status

#[test]
fn parse_temperature_status_from_docs() {
    let body = br#"{ "id": 0, "tC": 24.4, "tF": 75.9 }"#;
    let t: rpc::sensors::TemperatureStatus = rpc::parse(body).unwrap();
    assert_eq!(t.id, 0);
    assert_eq!(t.celsius, Some(24.4));
    assert_eq!(t.fahrenheit, Some(75.9));
}

// ── Humidity.GetStatus ────────────────────────────────────────────────
// https://shelly-api-docs.shelly.cloud/gen2/ComponentsAndServices/Humidity#status

#[test]
fn parse_humidity_status_from_docs() {
    let body = br#"{ "id": 0, "rh": 73.7 }"#;
    let h: rpc::sensors::HumidityStatus = rpc::parse(body).unwrap();
    assert_eq!(h.id, 0);
    assert_eq!(h.rh, Some(73.7));
}

// ── Ack ───────────────────────────────────────────────────────────────
// Setter methods return either {} or {"was_on": bool}.

#[test]
fn parse_ack_empty_object() {
    let ack: rpc::Ack = rpc::parse(b"{}").unwrap();
    assert_eq!(ack.was_on, None);
}

#[test]
fn parse_ack_was_on() {
    let ack: rpc::Ack = rpc::parse(br#"{"was_on":true}"#).unwrap();
    assert_eq!(ack.was_on, Some(true));
}

// ── Script.* ──────────────────────────────────────────────────────────

#[test]
fn parse_script_list() {
    let body = br#"{
        "scripts": [
            {"id": 1, "name": "my_script", "enable": false, "running": true},
            {"id": 2, "name": "my_script2", "enable": true, "running": false}
        ]
    }"#;
    let sl: rpc::script::ScriptList<'_> = rpc::parse(body).unwrap();
    assert_eq!(sl.scripts.len(), 2);
    assert_eq!(sl.scripts[0].id, 1);
    assert_eq!(sl.scripts[0].name, Some("my_script"));
    assert!(sl.scripts[0].running);
    assert!(sl.scripts[1].enable);
}

#[test]
fn parse_script_created() {
    let body = br#"{"id": 5}"#;
    let sc: rpc::script::ScriptCreated = rpc::parse(body).unwrap();
    assert_eq!(sc.id, 5);
}

#[test]
fn parse_put_code_result() {
    let body = br#"{"len": 1024}"#;
    let r: rpc::script::PutCodeResult = rpc::parse(body).unwrap();
    assert_eq!(r.len, 1024);
}

#[test]
fn parse_script_run_state() {
    let body = br#"{"was_running": true}"#;
    let r: rpc::script::ScriptRunState = rpc::parse(body).unwrap();
    assert!(r.was_running);
}

// ── Parse error paths ─────────────────────────────────────────────────

#[test]
fn parse_empty_body_is_error() {
    let r = rpc::parse::<rpc::switch::SwitchStatus<'_>>(b"");
    assert!(r.is_err());
}

#[test]
fn parse_garbage_is_error() {
    let r = rpc::parse::<rpc::switch::SwitchStatus<'_>>(b"not json");
    assert!(r.is_err());
}

#[test]
fn parse_truncated_json_is_error() {
    let r = rpc::parse::<rpc::switch::SwitchStatus<'_>>(br#"{"id": 0, "output""#);
    assert!(r.is_err());
}

#[test]
fn parse_null_as_ack_is_error() {
    // Bare `null` cannot be deserialized into a struct — the Device
    // layer's `get_ack` handles this before calling parse.
    let r = rpc::parse::<rpc::Ack>(b"null");
    assert!(r.is_err());
}

// ── Extra fields are silently ignored ─────────────────────────────────
// Shelly devices can add fields across firmware versions. Our structs
// must tolerate unknown keys without failing.

#[test]
fn unknown_fields_are_ignored() {
    let body = br#"{
        "id": 0,
        "source": "init",
        "output": true,
        "apower": 12.3,
        "voltage": 230.0,
        "current": 0.05,
        "freq": 50.0,
        "some_future_field": "hello",
        "another_one": [1, 2, 3]
    }"#;
    let s: rpc::switch::SwitchStatus<'_> = rpc::parse(body).unwrap();
    assert_eq!(s.id, 0);
    assert_eq!(s.output, Some(true));
}

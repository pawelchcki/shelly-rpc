//! High-level async handle to a Shelly Gen2+ device.

use embedded_nal_async::{Dns, TcpConnect};
use reqwless::client::HttpClient;
use reqwless::headers::ContentType;
use reqwless::request::{Method, RequestBuilder as _};

use crate::error::Error;
use crate::rpc;
use crate::util::{copy_slice, json_escape_into};

/// Maximum length of the base URL (`http://192.168.1.50:8080`).
const MAX_BASE_LEN: usize = 64;

/// Maximum length of a full URL (base + RPC path).
const MAX_URL_LEN: usize = 160;

/// A handle to a Shelly Gen2+ device, communicating over HTTP via a
/// user-supplied TCP stack and DNS resolver.
pub struct Device<'a, T, D>
where
    T: TcpConnect,
    D: Dns,
{
    client: HttpClient<'a, T, D>,
    base: heapless::String<MAX_BASE_LEN>,
}

impl<'a, T, D> Device<'a, T, D>
where
    T: TcpConnect,
    D: Dns,
{
    /// Create a new device handle.
    ///
    /// `base` is the scheme + host + optional port with no trailing slash,
    /// e.g. `"http://192.168.1.50"`.
    pub fn new(tcp: &'a T, dns: &'a D, base: &str) -> Result<Self, Error> {
        let client = HttpClient::new(tcp, dns);
        let mut base_s = heapless::String::new();
        base_s.push_str(base).map_err(|_| Error::BufferTooSmall)?;
        Ok(Self {
            client,
            base: base_s,
        })
    }

    /// Build a full URL from the base and an RPC path.
    fn url(&self, path: &str) -> Result<heapless::String<MAX_URL_LEN>, Error> {
        let mut url = heapless::String::new();
        url.push_str(self.base.as_str())
            .map_err(|_| Error::BufferTooSmall)?;
        url.push_str(path).map_err(|_| Error::BufferTooSmall)?;
        Ok(url)
    }

    /// Execute a GET request to `path` and parse the JSON response body.
    async fn get<'b, R>(&mut self, path: &str, buf: &'b mut [u8]) -> Result<R, Error>
    where
        R: serde::Deserialize<'b>,
    {
        let url = self.url(path)?;
        let mut req = self
            .client
            .request(Method::GET, url.as_str())
            .await
            .map_err(|_| Error::Transport)?;
        let resp = req.send(buf).await.map_err(|_| Error::Transport)?;
        if !resp.status.is_successful() {
            return Err(Error::Http(resp.status.0));
        }
        let body = resp
            .body()
            .read_to_end()
            .await
            .map_err(|_| Error::Transport)?;
        rpc::parse(body)
    }

    /// Execute a GET request to `path` and parse the response as an [`Ack`].
    ///
    /// Shelly returns `null` for fire-and-forget methods and
    /// `{"was_on": …}` for stateful setters. This helper handles both.
    async fn get_ack(&mut self, path: &str, buf: &mut [u8]) -> Result<rpc::Ack, Error> {
        let body = self.get_raw(path, buf).await?;
        if body == b"null" {
            return Ok(rpc::Ack::default());
        }
        rpc::parse(body)
    }

    /// Execute a GET request to `path` and return the raw response body.
    async fn get_raw<'b>(&mut self, path: &str, buf: &'b mut [u8]) -> Result<&'b [u8], Error> {
        let url = self.url(path)?;
        let mut req = self
            .client
            .request(Method::GET, url.as_str())
            .await
            .map_err(|_| Error::Transport)?;
        let resp = req.send(buf).await.map_err(|_| Error::Transport)?;
        if !resp.status.is_successful() {
            return Err(Error::Http(resp.status.0));
        }
        let body: &[u8] = resp
            .body()
            .read_to_end()
            .await
            .map_err(|_| Error::Transport)?;
        Ok(body)
    }

    // ── Shelly.* ───────────────────────────────────────────────────────

    /// `Shelly.GetDeviceInfo` — basic device metadata.
    pub async fn device_info<'b>(
        &mut self,
        buf: &'b mut [u8],
    ) -> Result<rpc::shelly::DeviceInfo<'b>, Error> {
        self.get(rpc::shelly::get_device_info_path().as_str(), buf)
            .await
    }

    /// `Shelly.GetStatus` — full device status envelope.
    pub async fn status<'b>(
        &mut self,
        buf: &'b mut [u8],
    ) -> Result<rpc::shelly::ShellyStatus<'b>, Error> {
        self.get(rpc::shelly::get_status_path().as_str(), buf).await
    }

    /// `Shelly.ListMethods` — available RPC methods.
    pub async fn list_methods<'b>(
        &mut self,
        buf: &'b mut [u8],
    ) -> Result<rpc::shelly::MethodList<'b>, Error> {
        self.get(rpc::shelly::list_methods_path().as_str(), buf)
            .await
    }

    /// `Shelly.Reboot` — restart the device.
    pub async fn reboot(&mut self, buf: &mut [u8]) -> Result<rpc::Ack, Error> {
        self.get_ack(rpc::shelly::reboot_path().as_str(), buf).await
    }

    /// `Shelly.Update` — install available firmware update.
    pub async fn update(&mut self, buf: &mut [u8]) -> Result<rpc::Ack, Error> {
        self.get_ack(rpc::shelly::update_path().as_str(), buf).await
    }

    /// `Shelly.CheckForUpdate` — check available firmware updates.
    pub async fn check_for_update<'b>(
        &mut self,
        buf: &'b mut [u8],
    ) -> Result<rpc::shelly::UpdateInfo<'b>, Error> {
        self.get(rpc::shelly::check_for_update_path().as_str(), buf)
            .await
    }

    // ── Sys.* ──────────────────────────────────────────────────────────

    /// `Sys.GetStatus` — system-level status (uptime, RAM, FS, updates).
    pub async fn sys_status<'b>(
        &mut self,
        buf: &'b mut [u8],
    ) -> Result<rpc::sys::SysStatus<'b>, Error> {
        self.get(rpc::sys::get_status_path().as_str(), buf).await
    }

    /// `Sys.GetConfig` — system configuration.
    pub async fn sys_config<'b>(
        &mut self,
        buf: &'b mut [u8],
    ) -> Result<rpc::sys::SysConfig<'b>, Error> {
        self.get(rpc::sys::get_config_path().as_str(), buf).await
    }

    // ── Wifi.* ─────────────────────────────────────────────────────────

    /// `Wifi.GetStatus` — Wi-Fi connection info.
    pub async fn wifi_status<'b>(
        &mut self,
        buf: &'b mut [u8],
    ) -> Result<rpc::wifi::WifiStatus<'b>, Error> {
        self.get(rpc::wifi::get_status_path().as_str(), buf).await
    }

    /// `Wifi.Scan` — scan for nearby access points.
    pub async fn wifi_scan<'b>(
        &mut self,
        buf: &'b mut [u8],
    ) -> Result<rpc::wifi::WifiScanResults<'b>, Error> {
        self.get(rpc::wifi::scan_path().as_str(), buf).await
    }

    // ── Switch.* ───────────────────────────────────────────────────────

    /// `Switch.GetStatus` — relay/switch state and metering.
    pub async fn switch_status<'b>(
        &mut self,
        id: u32,
        buf: &'b mut [u8],
    ) -> Result<rpc::switch::SwitchStatus<'b>, Error> {
        let path = rpc::switch::get_status_path(id)?;
        self.get(path.as_str(), buf).await
    }

    /// `Switch.GetConfig` — relay/switch configuration.
    pub async fn switch_config<'b>(
        &mut self,
        id: u32,
        buf: &'b mut [u8],
    ) -> Result<rpc::switch::SwitchConfig<'b>, Error> {
        let path = rpc::switch::get_config_path(id)?;
        self.get(path.as_str(), buf).await
    }

    /// `Switch.Set` — turn a switch on or off.
    pub async fn switch_set(
        &mut self,
        id: u32,
        on: bool,
        buf: &mut [u8],
    ) -> Result<rpc::Ack, Error> {
        let path = rpc::switch::set_path(id, on)?;
        self.get_ack(path.as_str(), buf).await
    }

    /// `Switch.Toggle` — toggle a switch.
    pub async fn switch_toggle(&mut self, id: u32, buf: &mut [u8]) -> Result<rpc::Ack, Error> {
        let path = rpc::switch::toggle_path(id)?;
        self.get_ack(path.as_str(), buf).await
    }

    // ── Cover.* ────────────────────────────────────────────────────────

    /// `Cover.GetStatus` — roller-shutter state and metering.
    pub async fn cover_status<'b>(
        &mut self,
        id: u32,
        buf: &'b mut [u8],
    ) -> Result<rpc::cover::CoverStatus<'b>, Error> {
        let path = rpc::cover::get_status_path(id)?;
        self.get(path.as_str(), buf).await
    }

    /// `Cover.Open` — open the cover.
    pub async fn cover_open(&mut self, id: u32, buf: &mut [u8]) -> Result<rpc::Ack, Error> {
        let path = rpc::cover::open_path(id)?;
        self.get_ack(path.as_str(), buf).await
    }

    /// `Cover.Close` — close the cover.
    pub async fn cover_close(&mut self, id: u32, buf: &mut [u8]) -> Result<rpc::Ack, Error> {
        let path = rpc::cover::close_path(id)?;
        self.get_ack(path.as_str(), buf).await
    }

    /// `Cover.Stop` — stop cover movement.
    pub async fn cover_stop(&mut self, id: u32, buf: &mut [u8]) -> Result<rpc::Ack, Error> {
        let path = rpc::cover::stop_path(id)?;
        self.get_ack(path.as_str(), buf).await
    }

    /// `Cover.GoToPosition` — move cover to a position (0–100%).
    pub async fn cover_go_to_position(
        &mut self,
        id: u32,
        pos: u32,
        buf: &mut [u8],
    ) -> Result<rpc::Ack, Error> {
        let path = rpc::cover::go_to_position_path(id, pos)?;
        self.get_ack(path.as_str(), buf).await
    }

    /// `Cover.Calibrate` — start cover calibration.
    pub async fn cover_calibrate(&mut self, id: u32, buf: &mut [u8]) -> Result<rpc::Ack, Error> {
        let path = rpc::cover::calibrate_path(id)?;
        self.get_ack(path.as_str(), buf).await
    }

    // ── Light.* ────────────────────────────────────────────────────────

    /// `Light.GetStatus` — dimmable light state.
    pub async fn light_status<'b>(
        &mut self,
        id: u32,
        buf: &'b mut [u8],
    ) -> Result<rpc::light::LightStatus<'b>, Error> {
        let path = rpc::light::get_status_path(id)?;
        self.get(path.as_str(), buf).await
    }

    /// `Light.Set` — turn a light on or off.
    pub async fn light_set(
        &mut self,
        id: u32,
        on: bool,
        buf: &mut [u8],
    ) -> Result<rpc::Ack, Error> {
        let path = rpc::light::set_path(id, on)?;
        self.get_ack(path.as_str(), buf).await
    }

    /// `Light.Set` with brightness — turn on and set brightness (0–100).
    pub async fn light_set_brightness(
        &mut self,
        id: u32,
        brightness: u8,
        buf: &mut [u8],
    ) -> Result<rpc::Ack, Error> {
        let path = rpc::light::set_brightness_path(id, brightness)?;
        self.get_ack(path.as_str(), buf).await
    }

    /// `Light.Toggle` — toggle a light.
    pub async fn light_toggle(&mut self, id: u32, buf: &mut [u8]) -> Result<rpc::Ack, Error> {
        let path = rpc::light::toggle_path(id)?;
        self.get_ack(path.as_str(), buf).await
    }

    // ── Input.* ────────────────────────────────────────────────────────

    /// `Input.GetStatus` — digital/analog input state.
    pub async fn input_status<'b>(
        &mut self,
        id: u32,
        buf: &'b mut [u8],
    ) -> Result<rpc::input::InputStatus<'b>, Error> {
        let path = rpc::input::get_status_path(id)?;
        self.get(path.as_str(), buf).await
    }

    /// `Input.GetConfig` — input configuration.
    pub async fn input_config<'b>(
        &mut self,
        id: u32,
        buf: &'b mut [u8],
    ) -> Result<rpc::input::InputConfig<'b>, Error> {
        let path = rpc::input::get_config_path(id)?;
        self.get(path.as_str(), buf).await
    }

    // ── Temperature / Humidity / DevicePower / Voltmeter ───────────────

    /// `Temperature.GetStatus` — temperature sensor reading.
    pub async fn temperature_status(
        &mut self,
        id: u32,
        buf: &mut [u8],
    ) -> Result<rpc::sensors::TemperatureStatus, Error> {
        let path = rpc::sensors::temperature_get_status_path(id)?;
        self.get(path.as_str(), buf).await
    }

    /// `Humidity.GetStatus` — humidity sensor reading.
    pub async fn humidity_status(
        &mut self,
        id: u32,
        buf: &mut [u8],
    ) -> Result<rpc::sensors::HumidityStatus, Error> {
        let path = rpc::sensors::humidity_get_status_path(id)?;
        self.get(path.as_str(), buf).await
    }

    /// `DevicePower.GetStatus` — battery/external power status.
    pub async fn device_power_status(
        &mut self,
        id: u32,
        buf: &mut [u8],
    ) -> Result<rpc::sensors::DevicePowerStatus, Error> {
        let path = rpc::sensors::device_power_get_status_path(id)?;
        self.get(path.as_str(), buf).await
    }

    /// `Voltmeter.GetStatus` — voltage reading.
    pub async fn voltmeter_status(
        &mut self,
        id: u32,
        buf: &mut [u8],
    ) -> Result<rpc::sensors::VoltmeterStatus, Error> {
        let path = rpc::sensors::voltmeter_get_status_path(id)?;
        self.get(path.as_str(), buf).await
    }

    // ── Script.* ────────────────────────────────────────────────────────

    /// `Script.List` — list all scripts on the device.
    pub async fn script_list<'b>(
        &mut self,
        buf: &'b mut [u8],
    ) -> Result<rpc::script::ScriptList<'b>, Error> {
        self.get(rpc::script::list_path().as_str(), buf).await
    }

    /// `Script.GetStatus` — get a script's running state.
    pub async fn script_status<'b>(
        &mut self,
        id: u32,
        buf: &'b mut [u8],
    ) -> Result<rpc::script::ScriptStatus<'b>, Error> {
        let path = rpc::script::get_status_path(id)?;
        self.get(path.as_str(), buf).await
    }

    /// `Script.Start` — start a script.
    pub async fn script_start(
        &mut self,
        id: u32,
        buf: &mut [u8],
    ) -> Result<rpc::script::ScriptRunState, Error> {
        let path = rpc::script::start_path(id)?;
        self.get(path.as_str(), buf).await
    }

    /// `Script.Stop` — stop a running script.
    pub async fn script_stop(
        &mut self,
        id: u32,
        buf: &mut [u8],
    ) -> Result<rpc::script::ScriptRunState, Error> {
        let path = rpc::script::stop_path(id)?;
        self.get(path.as_str(), buf).await
    }

    /// `Script.Delete` — delete a script slot.
    pub async fn script_delete(&mut self, id: u32, buf: &mut [u8]) -> Result<rpc::Ack, Error> {
        let path = rpc::script::delete_path(id)?;
        self.get_ack(path.as_str(), buf).await
    }

    /// `Script.Create` — create a new script slot (POST).
    pub async fn script_create(
        &mut self,
        name: &str,
        buf: &mut [u8],
    ) -> Result<rpc::script::ScriptCreated, Error> {
        let mut body_buf = [0u8; 128];
        let mut len = 0;
        len += copy_slice(&mut body_buf, len, br#"{"name":""#)?;
        len += json_escape_into(&mut body_buf[len..], name.as_bytes())?;
        len += copy_slice(&mut body_buf, len, br#""}"#)?;

        self.post("/rpc/Script.Create", &body_buf[..len], buf).await
    }

    /// `Script.PutCode` — upload code to a script slot (POST).
    ///
    /// `code` is the JavaScript source. For scripts larger than the
    /// device's HTTP buffer, call this multiple times with `append=true`.
    pub async fn script_put_code(
        &mut self,
        id: u32,
        code: &str,
        append: bool,
        body_buf: &mut [u8],
        rx_buf: &mut [u8],
    ) -> Result<rpc::script::PutCodeResult, Error> {
        use core::fmt::Write;
        let mut len = 0usize;

        len += copy_slice(body_buf, len, br#"{"id":"#)?;

        let mut id_str = heapless::String::<12>::new();
        write!(id_str, "{id}").map_err(|_| Error::BufferTooSmall)?;
        len += copy_slice(body_buf, len, id_str.as_bytes())?;

        len += copy_slice(body_buf, len, br#","code":""#)?;
        len += json_escape_into(&mut body_buf[len..], code.as_bytes())?;

        let suffix = if append {
            br#"","append":true}"#.as_slice()
        } else {
            br#""}"#.as_slice()
        };
        len += copy_slice(body_buf, len, suffix)?;

        self.post("/rpc/Script.PutCode", &body_buf[..len], rx_buf)
            .await
    }

    // ── Generic helpers ────────────────────────────────────────────────

    /// Issue a raw GET to an arbitrary RPC path and return the body bytes.
    pub async fn call_raw<'b>(
        &mut self,
        rpc_path: &str,
        buf: &'b mut [u8],
    ) -> Result<&'b [u8], Error> {
        self.get_raw(rpc_path, buf).await
    }

    /// Issue a POST to `path` with a JSON `body` and parse the response.
    pub async fn call_post<'b, R>(
        &mut self,
        rpc_path: &str,
        body: &[u8],
        buf: &'b mut [u8],
    ) -> Result<R, Error>
    where
        R: serde::Deserialize<'b>,
    {
        self.post(rpc_path, body, buf).await
    }

    /// POST helper — sends `body` as `application/json` to `path`.
    async fn post<'b, R>(&mut self, path: &str, body: &[u8], buf: &'b mut [u8]) -> Result<R, Error>
    where
        R: serde::Deserialize<'b>,
    {
        let url = self.url(path)?;
        let mut req = self
            .client
            .request(Method::POST, url.as_str())
            .await
            .map_err(|_| Error::Transport)?
            .content_type(ContentType::ApplicationJson)
            .body(body);
        let resp = req.send(buf).await.map_err(|_| Error::Transport)?;
        if !resp.status.is_successful() {
            return Err(Error::Http(resp.status.0));
        }
        let resp_body = resp
            .body()
            .read_to_end()
            .await
            .map_err(|_| Error::Transport)?;
        rpc::parse(resp_body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_escape_plain() {
        let mut buf = [0u8; 32];
        let n = json_escape_into(&mut buf, b"hello").unwrap();
        assert_eq!(&buf[..n], b"hello");
    }

    #[test]
    fn json_escape_special_chars() {
        let mut buf = [0u8; 64];
        let n = json_escape_into(&mut buf, br#"say "hi" \ and	tab"#).unwrap();
        assert_eq!(
            core::str::from_utf8(&buf[..n]).unwrap(),
            r#"say \"hi\" \\ and\ttab"#
        );
    }

    #[test]
    fn json_escape_newlines() {
        let mut buf = [0u8; 32];
        let n = json_escape_into(&mut buf, b"a\nb\rc").unwrap();
        assert_eq!(&buf[..n], br#"a\nb\rc"#);
    }

    #[test]
    fn json_escape_buffer_too_small() {
        let mut buf = [0u8; 3];
        let r = json_escape_into(&mut buf, b"hello");
        assert!(r.is_err());
    }

    #[test]
    fn json_escape_backslash_doubles_length() {
        let mut buf = [0u8; 4];
        // 2 backslashes -> 4 bytes escaped
        let n = json_escape_into(&mut buf, b"\\\\").unwrap();
        assert_eq!(&buf[..n], b"\\\\\\\\");
    }
}

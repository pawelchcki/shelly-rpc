# CLAUDE.md

- Shelly `shelly-diy` OAuth returns the JWT access token directly as the `code` param — no token exchange needed.
- On-device mJS scripts must stay under 2KB source; use `KVS.Set`/`KVS.Get` via `Shelly.call()` (not as globals); `Shelly.emitEvent()` works on Gen4 but doesn't trigger phone push notifications — use `HTTP.GET` to trigger Cloud Scenes instead.
- The PowerStrip has 4 switches; cloud server is `shelly-96-eu.shelly.cloud`.
- Cloud `scene/add` API is undocumented. Mandatory fields in `scene_script`: `_enabled: true`, `_run_on_ingest: true`, `if: {"or":[{"and":[]}]}` (empty condition), and `do` array with `notify: "push_notification"`. Missing any of these creates ghost scenes that break the app.

# Known Issues

- Scenes created via `shelly cloud scene add` use a default placeholder image. Custom image upload via the Cloud API is not yet implemented.
- Malformed scenes (missing `_enabled`, `_run_on_ingest`, empty `if` block, or proper `do` format) persist as ghost entries in the Shelly app even after API deletion. These fields are all mandatory for `scene/add` — the API accepts invalid scenes without error but they break the app.
- The `shelly-diy` OAuth token (from `shelly cloud login-diy`) only works for real-time events. It cannot call the Cloud Control API (scenes, device status, etc). Use `shelly cloud login` with the full auth key from the Shelly app instead.
- Shelly Cloud API is rate-limited to ~1 request/second. Rapid consecutive calls return `max_req` errors.

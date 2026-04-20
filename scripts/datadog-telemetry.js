// ============================================================================
// Datadog telemetry — ships device-local metrics + controller events to Datadog
// ============================================================================
//
// Metrics (pushed every `post_interval_s`, default 5 min):
//   Environment — indoor + outdoor + the gap that actually matters
//     shelly.indoor.humidity.percent
//     shelly.indoor.temperature.celsius
//     shelly.indoor.ah.g_per_m3
//     shelly.outdoor.humidity.percent
//     shelly.outdoor.temperature.celsius
//     shelly.outdoor.ah.g_per_m3
//     shelly.ah.gap.g_per_m3              (indoor − outdoor)
//   Sensor health
//     shelly.sensor.battery.percent
//     shelly.sensor.rssi.dbm
//     shelly.sensor.broadcast_age.seconds (spots BLE dropouts)
//   Fan relay
//     shelly.fan.state                    (0/1, gauge)
//     shelly.fan.mode                     (0..4, see MODE_NUM below)
//     shelly.fan.mcu_temp.celsius
//   System
//     shelly.sys.uptime.seconds
//     shelly.sys.ram_free.bytes
//     shelly.sys.ram_min_free.bytes
//     shelly.wifi.rssi.dbm
//     shelly.wifi.connected               (0/1)
//     shelly.cloud.connected              (0/1)
//     shelly.mqtt.connected               (0/1)
//     shelly.input.state                  (0/1)
//   Controller learning (from KVS fan_stats, written ≤ 1× / day)
//     shelly.fan.ema_effective_rate
//     shelly.fan.ema_peak_rh
//     shelly.fan.cycles_count
//   Per-script health (tag `script:<name>`)
//     shelly.script.running               (0/1)
//     shelly.script.mem_free.bytes
//     shelly.script.cpu.percent
//     shelly.script.errors_count
//
// Datadog events (low-frequency, high-signal):
//   "Fan cycle end"    — from fan.cycle_end Shelly event, tagged with
//                        end_reason (futility/floor/eval_drift/max_run)
//   "Fan mode change"  — from fan.mode Shelly event, verbose mode
//                        transitions for correlation with humidity.
//
// Architecture
// ------------
//   * Controller (`bathroom-fan.js`) emits generic `Shelly.emitEvent`
//     with names `fan.mode`, `fan.cycle_end`. It has no Datadog-specific
//     code — events are a clean consumer boundary.
//   * This script listens via `Shelly.addEventHandler` and converts to
//     Datadog signals.
//   * Eco-mode friendly: `Shelly.addStatusHandler` keeps BTHome + switch
//     live values up-to-date regardless of timer slip; the post timer
//     only decides *when* to ship.
//
// Configuration (KVS key `dd_cfg`, all keys optional):
//
//   {
//     "api_key":      "xxxxxxxx",            // required; no-key → dry run
//     "site":         "datadoghq.com",
//     "host":         "shelly-lazienka",
//     "tags":         ["room:bathroom","env:home"],
//     "post_interval_s": 300,
//     "weather_poll_s":  900,
//     "lat":          52.41,                 // Open-Meteo fetched locally
//     "lon":          16.93,
//     "bthome_humidity_id":    200,
//     "bthome_temperature_id": 201,
//     "bthome_battery_id":     202,
//     "bthome_device_id":      200,
//     "switch_id":             0,
//     "fan_stats_kvs_key":     "fan_stats"
//   }

// ---------- Defaults --------------------------------------------------------

let DD_CFG = {
  api_key:    "",
  site:       "datadoghq.eu",
  host:       "shelly",
  tags:       [],
  post_interval_s: 300,
  weather_poll_s:  900,
  lat:             52.41,
  lon:             16.93,
  bthome_humidity_id:    200,
  bthome_temperature_id: 201,
  bthome_battery_id:     202,
  bthome_device_id:      200,
  switch_id:             0,
  fan_stats_kvs_key:     "fan_stats",
};

// Mode → small integer so it can be a gauge. Keep in sync with the
// controller's state machine.
let MODE_NUM = {
  "IDLE":       0,
  "ARMED":      1,
  "VENTING":    2,
  "EVALUATING": 3,
  "COOLDOWN":   4,
};

// ---------- Live state ------------------------------------------------------

let live = {
  rh:            null,
  t:             null,
  batt:          null,
  dev_rssi:      null,
  dev_last_ts:   0,     // unixtime of last BTHome packet
  switch_on:     null,
  mcu_tc:        null,
  input_state:   null,
  fan_mode:      null,
};

let outdoor = {
  rh: null,
  t:  null,
  ah: null,
  ts: 0,
};

// Wall-clock time this script started (ms since epoch). The difference
// `nowMs() - scriptStartMs` is reported each cycle as script_runtime_ms.
let scriptStartMs = 0;

// ---------- Helpers ---------------------------------------------------------

function log(m) { print("[dd] " + m); }

function nowTs() { return Shelly.getComponentStatus("sys").unixtime; }

// Gen3 sys.unixtime is a float with sub-second resolution, so we can
// derive a cheap millisecond clock without needing Date.now() (which
// mJS's runtime may or may not expose).
function nowMs() {
  return Shelly.getComponentStatus("sys").unixtime * 1000;
}

function magnusEs(t) { return 6.112 * Math.exp(17.62 * t / (243.12 + t)); }

function ah(rh, t) { return 216.7 * (rh / 100.0) * magnusEs(t) / (273.15 + t); }

function callRpc(method, params, cb) {
  Shelly.call(method, params, function (res, err, msg) {
    if (err) log("!" + method + " err=" + err + " " + (msg || ""));
    if (cb) cb(res, err, msg);
  });
}

function mergeInto(target, src) {
  for (let k in src) {
    if (src.hasOwnProperty(k)) target[k] = src[k];
  }
}

function boolToNum(b) {
  if (b === true)  return 1;
  if (b === false) return 0;
  return null;
}

// mJS lacks Array.prototype.concat. Build a fresh array manually.
function arrConcat(a, b) {
  let out = [];
  if (a) for (let i = 0; i < a.length; i++) out.push(a[i]);
  if (b) for (let j = 0; j < b.length; j++) out.push(b[j]);
  return out;
}

// ---------- Status events (fast path) ---------------------------------------

function onStatus(ev) {
  if (!ev || typeof ev.component !== "string") return;
  let c = ev.component;
  let d = ev.delta || {};

  if (c.indexOf("bthomesensor:") === 0) {
    // The id lives in the component name, not the delta.
    if (typeof d.value !== "number") return;
    let id = parseInt(c.slice(13), 10);
    if (id === DD_CFG.bthome_humidity_id)    live.rh   = d.value;
    if (id === DD_CFG.bthome_temperature_id) live.t    = d.value;
    if (id === DD_CFG.bthome_battery_id)     live.batt = d.value;
    return;
  }
  if (c === "bthomedevice:" + DD_CFG.bthome_device_id) {
    if (typeof d.rssi === "number")            live.dev_rssi    = d.rssi;
    if (typeof d.last_updated_ts === "number") live.dev_last_ts = d.last_updated_ts;
    return;
  }
  if (c === "switch:" + DD_CFG.switch_id) {
    if (typeof d.output === "boolean") live.switch_on = d.output;
    if (d.temperature && typeof d.temperature.tC === "number") {
      live.mcu_tc = d.temperature.tC;
    }
    return;
  }
  if (c === "input:0") {
    if (typeof d.state === "boolean") live.input_state = d.state;
  }
}

// ---------- Controller events (fan.mode, fan.cycle_end) --------------------

function onEvent(ev) {
  if (!ev) return;
  // Shelly delivers user events as `{component, id, now, info: {event,
  // data, ...}}` on Gen3 — unwrap the `info` payload. Keep a fallback
  // for the bare shape in case it differs on other firmwares.
  let src = (ev.info && typeof ev.info === "object") ? ev.info : ev;
  let name = src.event;
  if (typeof name !== "string") return;
  let data = src.data || {};

  if (name === "fan.mode") {
    if (typeof data.to === "string" && MODE_NUM.hasOwnProperty(data.to)) {
      live.fan_mode = MODE_NUM[data.to];
    }
    // Only send a DD event on a real transition. The controller
    // heartbeats fan.mode every tick with from===to to keep listeners
    // in sync after a restart; those aren't worth a DD event.
    if (data.from !== data.to) {
      postEvent("Fan mode " + (data.from || "?") + " → " + (data.to || "?"),
                "Controller transitioned state on " + DD_CFG.host,
                ["event:fan_mode", "from:" + (data.from || "unknown"),
                                  "to:"   + (data.to   || "unknown")],
                "info");
    }
    return;
  }

  if (name === "fan.boot") {
    let degraded = data.status === "degraded_no_outdoor";
    postEvent(
      degraded ? "Fan controller booted (degraded: no outdoor data)"
               : "Fan controller booted",
      "Status: " + (data.status || "unknown") + " on " + DD_CFG.host,
      ["event:fan_boot", "status:" + (data.status || "unknown")],
      degraded ? "warning" : "info"
    );
    return;
  }

  if (name === "fan.cycle_end") {
    let title = "Fan cycle end (" + (data.end_reason || "unknown") + ")";
    let body  = "dAH="      + (data.dAH      === undefined ? "?" : data.dAH)
              + " duration=" + (data.duration_s || 0) + "s"
              + " peak_rh=" + (data.peak_rh  === undefined ? "?" : data.peak_rh)
              + " rate="    + (data.rate     === undefined ? "?" : data.rate);
    postEvent(title, body,
              ["event:fan_cycle_end",
               "reason:" + (data.end_reason || "unknown")],
              "info");
  }
}

// ---------- Outdoor poll (independent; no IPC with controller) -------------

function parseOpenMeteo(body) {
  let p = null;
  try { p = JSON.parse(body); } catch (e) { return null; }
  if (p === null) return null;
  let c = p.current;
  if (c === null || typeof c !== "object") return null;
  let rh = c.relative_humidity_2m;
  let t  = c.temperature_2m;
  if (typeof rh !== "number") return null;
  if (typeof t  !== "number") return null;
  return { rh: rh, t: t };
}

function pollWeather() {
  let url = "https://api.open-meteo.com/v1/forecast?latitude=" + DD_CFG.lat +
            "&longitude=" + DD_CFG.lon +
            "&current=temperature_2m,relative_humidity_2m";
  callRpc("HTTP.GET", { url: url, timeout: 15 }, function (res, err) {
    if (err) return;
    if (!res || res.code !== 200 || !res.body) return;
    let cur = parseOpenMeteo(res.body);
    if (cur === null) { log("!weather parse"); return; }
    outdoor.rh = cur.rh;
    outdoor.t  = cur.t;
    outdoor.ah = ah(cur.rh, cur.t);
    outdoor.ts = nowTs();
  });
}

// ---------- Metric assembly -------------------------------------------------
//
// mJS heap is tight (< 2 KB per eval frame). To stay under it we:
//   1. Project each RPC response to a handful of scalars IN the response
//      callback — the big response object is eligible for GC before the
//      next callback runs.
//   2. Split the outbound series into small batches (≤ 8 metrics each)
//      so the JSON payload + intermediate array never exceeds the heap.
//
// Resulting layout: `sum` is the full collected scalar summary; we fire
// 3 small HTTP.POSTs per collection cycle.

function pushMetric(series, ts, metric, value, extraTags) {
  if (value === null || value === undefined) return;
  if (typeof value !== "number") return;
  let tags = DD_CFG.tags;
  if (extraTags && extraTags.length) tags = arrConcat(tags, extraTags);
  series.push({
    metric: metric,
    points: [[ts, value]],
    type:   "gauge",
    host:   DD_CFG.host,
    tags:   tags,
  });
}

// ---------- Posting ---------------------------------------------------------

function ddUrl(path) {
  return "https://api." + DD_CFG.site + path +
         "?api_key=" + DD_CFG.api_key;
}

// Kept for compatibility (used by postEvent's error path only).
function postSeries(series) { postAndThen(series, function () {}); }

function postEvent(title, text, tags, alertType) {
  if (!DD_CFG.api_key) return;
  let body = JSON.stringify({
    title: title,
    text:  text,
    tags:  arrConcat(DD_CFG.tags, tags),
    alert_type: alertType || "info",
    source_type_name: "shelly",
    host: DD_CFG.host,
  });
  callRpc("HTTP.POST", {
    url: ddUrl("/api/v1/events"),
    body: body,
    content_type: "application/json",
    timeout: 15,
  }, function (res, err) {
    if (err) return;
    if (!res || typeof res.code !== "number") return;
    if (res.code >= 300) {
      log("event " + res.code + " " + (res.body || "").slice(0, 80));
    }
  });
}

// ---------- Collection pass -------------------------------------------------

// Fully serialized collection + post pipeline. Each HTTP.POST waits for
// the previous one to complete before the next starts — avoids the "too
// many calls in progress" cap (~5 concurrent) and the heap pressure
// from overlapping JSON payloads.
//
// Stages:
//   1. Collect scalar sys/wifi/cloud/mqtt via serialized callRpc chain.
//   2. Post env/sensor batch (blocks until complete).
//   3. Post system batch.
//   4. Read fan_stats KVS, post learning batch.
//   5. Iterate scripts one at a time, post single combined scripts batch.

function postAndThen(series, next) {
  // Break the call stack when not posting — otherwise chained async
  // iterators blow mJS's small stack if many stages are empty.
  if (series.length === 0 || !DD_CFG.api_key) {
    Timer.set(1, false, next);
    return;
  }
  let body = JSON.stringify({ series: series });
  callRpc("HTTP.POST", {
    url: ddUrl("/api/v1/series"),
    body: body,
    content_type: "application/json",
    timeout: 15,
  }, function (res, err) {
    if (!err && res && typeof res.code === "number") {
      if (res.code >= 200 && res.code < 300) {
        log("posted " + series.length + " metric(s)");
      } else {
        log("post " + res.code);
      }
    }
    next();
  });
}

function collectAndPost() {
  let ts = nowTs();
  let sum = {};
  callRpc("Sys.GetStatus", {}, function (r) {
    if (r) {
      sum.uptime       = r.uptime;
      sum.ram_free     = r.ram_free;
      sum.ram_min_free = r.ram_min_free;
    }
    callRpc("WiFi.GetStatus", {}, function (r) {
      if (r) {
        sum.wifi_rssi      = r.rssi;
        sum.wifi_connected = (r.status === "got ip") ? 1 : 0;
      }
      callRpc("Cloud.GetStatus", {}, function (r) {
        if (r) sum.cloud_connected = r.connected ? 1 : 0;
        callRpc("MQTT.GetStatus", {}, function (r) {
          if (r) sum.mqtt_connected = r.connected ? 1 : 0;
          // Stage 2: env batch.
          let env = [];
          pushMetric(env, ts, "shelly.indoor.humidity.percent",    live.rh);
          pushMetric(env, ts, "shelly.indoor.temperature.celsius", live.t);
          if (live.rh !== null && live.t !== null) {
            pushMetric(env, ts, "shelly.indoor.ah.g_per_m3", ah(live.rh, live.t));
          }
          pushMetric(env, ts, "shelly.outdoor.humidity.percent",    outdoor.rh);
          pushMetric(env, ts, "shelly.outdoor.temperature.celsius", outdoor.t);
          pushMetric(env, ts, "shelly.outdoor.ah.g_per_m3",         outdoor.ah);
          if (live.rh !== null && live.t !== null && outdoor.ah !== null) {
            pushMetric(env, ts, "shelly.ah.gap.g_per_m3",
                       ah(live.rh, live.t) - outdoor.ah);
          }
          pushMetric(env, ts, "shelly.sensor.battery.percent", live.batt);
          pushMetric(env, ts, "shelly.sensor.rssi.dbm",        live.dev_rssi);
          if (live.dev_last_ts > 0) {
            pushMetric(env, ts, "shelly.sensor.broadcast_age.seconds",
                       ts - live.dev_last_ts);
          }
          postAndThen(env, function () {
            // Stage 3a: fan + sys batch (kept small to stay under the
            // ~1.5 KB mJS heap peak).
            let sys2 = [];
            pushMetric(sys2, ts, "shelly.fan.state",            boolToNum(live.switch_on));
            pushMetric(sys2, ts, "shelly.fan.mode",             live.fan_mode);
            pushMetric(sys2, ts, "shelly.fan.mcu_temp.celsius", live.mcu_tc);
            pushMetric(sys2, ts, "shelly.input.state",          boolToNum(live.input_state));
            pushMetric(sys2, ts, "shelly.sys.uptime.seconds",     sum.uptime);
            pushMetric(sys2, ts, "shelly.sys.ram_free.bytes",     sum.ram_free);
            pushMetric(sys2, ts, "shelly.sys.ram_min_free.bytes", sum.ram_min_free);
            postAndThen(sys2, function () {
              // Stage 3b: wifi/cloud/mqtt + script runtime.
              let net = [];
              pushMetric(net, ts, "shelly.wifi.rssi.dbm",   sum.wifi_rssi);
              pushMetric(net, ts, "shelly.wifi.connected",  sum.wifi_connected);
              pushMetric(net, ts, "shelly.cloud.connected", sum.cloud_connected);
              pushMetric(net, ts, "shelly.mqtt.connected",  sum.mqtt_connected);
              pushMetric(net, ts, "shelly.telemetry.script_runtime_ms",
                         scriptStartMs > 0 ? (nowMs() - scriptStartMs) : null);
              postAndThen(net, function () {
              // Stage 4: learning batch.
              callRpc("KVS.Get", { key: DD_CFG.fan_stats_kvs_key }, function (r) {
                let lrn = [];
                if (r) {
                  let stats = null;
                  try { stats = JSON.parse(r.value); } catch (e) { stats = null; }
                  if (stats) {
                    pushMetric(lrn, ts, "shelly.fan.ema_effective_rate", stats.ema_effective_rate);
                    pushMetric(lrn, ts, "shelly.fan.ema_peak_rh",        stats.ema_peak_rh);
                    if (stats.cycles && typeof stats.cycles.length === "number") {
                      pushMetric(lrn, ts, "shelly.fan.cycles_count", stats.cycles.length);
                    }
                  }
                }
                postAndThen(lrn, function () {
                  collectScripts(ts);
                });
              });
              });
            });
          });
        });
      });
    });
  });
}

function collectScripts(ts) {
  callRpc("Script.List", {}, function (lst) {
    let scripts = (lst && lst.scripts) ? lst.scripts : [];
    let i = 0;
    let s = [];
    function next() {
      if (i >= scripts.length) {
        postAndThen(s, function () {});
        return;
      }
      let sc = scripts[i];
      callRpc("Script.GetStatus", { id: sc.id }, function (st) {
        let tag = "script:" + (sc.name || "?");
        pushMetric(s, ts, "shelly.script.running",
                   sc.running === true ? 1 : 0, [tag]);
        if (st) {
          pushMetric(s, ts, "shelly.script.mem_free.bytes", st.mem_free, [tag]);
          pushMetric(s, ts, "shelly.script.cpu.percent",    st.cpu,      [tag]);
          let nErr = (st.errors && typeof st.errors.length === "number")
                       ? st.errors.length : 0;
          pushMetric(s, ts, "shelly.script.errors_count", nErr, [tag]);
        }
        i++;
        next();
      });
    }
    next();
  });
}

// ---------- Bootstrap -------------------------------------------------------

// One-shot seed: status handlers only fire on *changes*, so on boot we
// pull current values for switch, input, battery, and the BLE sensor so
// the first post has something to report.
function seedLiveState() {
  callRpc("Switch.GetStatus", { id: DD_CFG.switch_id }, function (r) {
    if (r) {
      if (typeof r.output === "boolean") live.switch_on = r.output;
      if (r.temperature && typeof r.temperature.tC === "number") {
        live.mcu_tc = r.temperature.tC;
      }
    }
    callRpc("Input.GetStatus", { id: 0 }, function (r) {
      if (r && typeof r.state === "boolean") live.input_state = r.state;
      callRpc("BTHomeSensor.GetStatus",
              { id: DD_CFG.bthome_humidity_id }, function (r) {
        if (r && typeof r.value === "number") live.rh = r.value;
        callRpc("BTHomeSensor.GetStatus",
                { id: DD_CFG.bthome_temperature_id }, function (r) {
          if (r && typeof r.value === "number") live.t = r.value;
          callRpc("BTHomeSensor.GetStatus",
                  { id: DD_CFG.bthome_battery_id }, function (r) {
            if (r && typeof r.value === "number") live.batt = r.value;
          });
        });
      });
    });
  });
}

scriptStartMs = nowMs();
callRpc("KVS.Get", { key: "dd_cfg" }, function (res) {
  if (res) {
    try { mergeInto(DD_CFG, JSON.parse(res.value)); }
    catch (e) { log("!dd_cfg parse"); }
  }
  Shelly.addStatusHandler(onStatus);
  Shelly.addEventHandler(onEvent);
  seedLiveState();
  pollWeather();
  // Warm-up post — let seed + sensor events settle first.
  Timer.set(10000, false, collectAndPost);
  Timer.set(DD_CFG.post_interval_s * 1000, true, collectAndPost);
  Timer.set(DD_CFG.weather_poll_s  * 1000, true, pollWeather);
  log("up: host=" + DD_CFG.host + " site=" + DD_CFG.site +
      " interval=" + DD_CFG.post_interval_s + "s" +
      (DD_CFG.api_key ? "" : " (no api_key: dry)"));
});

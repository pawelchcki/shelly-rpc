// ============================================================================
// Datadog telemetry for a Shelly PowerStrip (Gen4, 4 × switch components)
// ============================================================================
//
// Metrics per switch (tagged `switch_id:N` and optional `appliance:<name>`
// from `dd_cfg.switch_names`):
//   shelly.switch.state                (0/1)
//   shelly.switch.power.watts          (apower)
//   shelly.switch.voltage.volts
//   shelly.switch.current.amperes
//   shelly.switch.frequency.hz
//   shelly.switch.energy.total.wh      (cumulative aenergy.total)
//   shelly.switch.mcu_temp.celsius
//
// Device-wide:
//   shelly.sys.{uptime,ram_free,ram_min_free}.*
//   shelly.wifi.{rssi.dbm, connected}
//   shelly.cloud.connected / shelly.mqtt.connected
//   shelly.script.{running, mem_free.bytes, cpu.percent, errors_count}
//   shelly.telemetry.script_runtime_ms   (ms since this script booted)
//
// Controller events (from `appliance-monitor.js`):
//   appliance.started  → Datadog event "Appliance started (sw<id>)"
//   appliance.done     → Datadog event "Appliance done" + tags duration / Wh
//
// Config (KVS key `dd_cfg`, all keys optional):
//   {
//     "api_key":        "xxxxxxxx",
//     "site":           "datadoghq.com",
//     "host":           "shelly-powerstrip",
//     "tags":           ["location:laundry","env:home"],
//     "post_interval_s": 300,
//     "switch_ids":     [0,1,2,3],
//     "switch_names":   {"0":"pralki","1":"pralki2","2":"suszarka","3":"pralka"}
//   }
//
// mJS style identical to datadog-telemetry.js — no arrow functions, no
// template literals, tight-heap mitigations (serialized RPCs, batched
// posts, aggressive projection).

let DD_CFG = {
  api_key:        "",
  site:           "datadoghq.eu",
  host:           "shelly-powerstrip",
  tags:           [],
  post_interval_s: 300,
  switch_ids:     [0, 1, 2, 3],
  switch_names:   {},
};

let live = {
  switches: {},    // id -> {on, power, voltage, current, freq, energy, mcu_tc}
};

// Wall-clock time this script started (ms since epoch).
let scriptStartMs = 0;

function log(m) { print("[dd] " + m); }

function nowTs() { return Shelly.getComponentStatus("sys").unixtime; }

function nowMs() {
  // Gen3/Gen4 sys.unixtime is a float with sub-second resolution.
  return Shelly.getComponentStatus("sys").unixtime * 1000;
}

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

function arrConcat(a, b) {
  let out = [];
  if (a) for (let i = 0; i < a.length; i++) out.push(a[i]);
  if (b) for (let j = 0; j < b.length; j++) out.push(b[j]);
  return out;
}

function ensureSwitchSlot(id) {
  if (!live.switches[id]) {
    live.switches[id] = {
      on: null, power: null, voltage: null, current: null,
      freq: null, energy: null, mcu_tc: null,
    };
  }
  return live.switches[id];
}

// ---------- Event-driven fast path ------------------------------------------

function onStatus(ev) {
  if (!ev || typeof ev.component !== "string") return;
  let c = ev.component;
  if (c.indexOf("switch:") !== 0) return;
  let idStr = c.slice(7);
  let id = parseInt(idStr, 10);
  if (isNaN(id)) return;
  let slot = ensureSwitchSlot(id);
  let d = ev.delta || {};
  if (typeof d.output  === "boolean") slot.on      = d.output;
  if (typeof d.apower  === "number")  slot.power   = d.apower;
  if (typeof d.voltage === "number")  slot.voltage = d.voltage;
  if (typeof d.current === "number")  slot.current = d.current;
  if (typeof d.freq    === "number")  slot.freq    = d.freq;
  if (d.aenergy && typeof d.aenergy.total === "number") slot.energy = d.aenergy.total;
  if (d.temperature && typeof d.temperature.tC === "number") slot.mcu_tc = d.temperature.tC;
}

function onEvent(ev) {
  if (!ev) return;
  // Shelly delivers user events as `{component, id, now, info: {event,
  // data, ...}}` on Gen3/Gen4; unwrap the `info` payload.
  let src = (ev.info && typeof ev.info === "object") ? ev.info : ev;
  let name = src.event;
  if (typeof name !== "string") return;
  let data = src.data || {};
  if (name === "appliance.started") {
    let sid = data.switch_id;
    postEvent(
      "Appliance started (sw" + sid + ")",
      "Start detected on " + switchLabel(sid),
      ["event:appliance_started", "switch_id:" + sid],
      "info"
    );
  } else if (name === "appliance.done") {
    let sid = data.switch_id;
    postEvent(
      "Appliance done (sw" + sid + ")",
      "Cycle finished on " + switchLabel(sid) +
        " in " + (data.duration_minutes || 0) + " min" +
        " using " + (data.energy_wh || 0) + " Wh",
      ["event:appliance_done",
       "switch_id:" + sid,
       "duration_min:" + (data.duration_minutes || 0)],
      "info"
    );
  }
}

function switchLabel(id) {
  let name = DD_CFG.switch_names && DD_CFG.switch_names[String(id)];
  return name ? (name + " (sw" + id + ")") : ("sw" + id);
}

// ---------- Metric helpers --------------------------------------------------

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

function switchTags(id) {
  let tags = ["switch_id:" + id];
  let name = DD_CFG.switch_names && DD_CFG.switch_names[String(id)];
  if (name) tags.push("appliance:" + name);
  return tags;
}

// ---------- Posting ---------------------------------------------------------

function ddUrl(path) {
  return "https://api." + DD_CFG.site + path + "?api_key=" + DD_CFG.api_key;
}

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
    if (res.code >= 300) log("event " + res.code);
  });
}

// ---------- Collection passes (memory-lean) ---------------------------------
//
// Fully serialized: each HTTP.POST awaits the previous completion
// before the next starts. Avoids "too many calls in progress" (~5 cap)
// and the heap pressure from overlapping JSON payloads.
function postAndThen(series, next) {
  // Break the stack when we don't actually post — otherwise recursive
  // iterators (postSwitches → next → postAndThen → next → …) blow the
  // mJS call stack when every slot is empty at boot.
  if (series.length === 0 || !DD_CFG.api_key) {
    Timer.set(1, false, next);
    return;
  }
  let body = JSON.stringify({ series: series });
  let count = series.length;
  callRpc("HTTP.POST", {
    url: ddUrl("/api/v1/series"),
    body: body,
    content_type: "application/json",
    timeout: 15,
  }, function (res, err) {
    if (err) {
      log("post dropped " + count + " metric(s)");
    } else if (res && typeof res.code === "number") {
      if (res.code >= 200 && res.code < 300) {
        log("posted " + count + " metric(s)");
      } else {
        log("post " + res.code + " dropped " + count);
      }
    } else {
      log("post dropped " + count + " (no response)");
    }
    next();
  });
}

function buildSwitchSeries(ts, id) {
  let slot = live.switches[id];
  if (!slot) return [];
  let s = [];
  let tags = switchTags(id);
  pushMetric(s, ts, "shelly.switch.state",           boolToNum(slot.on), tags);
  pushMetric(s, ts, "shelly.switch.power.watts",     slot.power,   tags);
  pushMetric(s, ts, "shelly.switch.voltage.volts",   slot.voltage, tags);
  pushMetric(s, ts, "shelly.switch.current.amperes", slot.current, tags);
  pushMetric(s, ts, "shelly.switch.frequency.hz",    slot.freq,    tags);
  pushMetric(s, ts, "shelly.switch.energy.total.wh", slot.energy,  tags);
  pushMetric(s, ts, "shelly.switch.mcu_temp.celsius", slot.mcu_tc, tags);
  return s;
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
          // Post system batch, then walk switches, then scripts — all serialized.
          let sysBatch = [];
          pushMetric(sysBatch, ts, "shelly.sys.uptime.seconds",     sum.uptime);
          pushMetric(sysBatch, ts, "shelly.sys.ram_free.bytes",     sum.ram_free);
          pushMetric(sysBatch, ts, "shelly.sys.ram_min_free.bytes", sum.ram_min_free);
          pushMetric(sysBatch, ts, "shelly.wifi.rssi.dbm",          sum.wifi_rssi);
          pushMetric(sysBatch, ts, "shelly.wifi.connected",         sum.wifi_connected);
          pushMetric(sysBatch, ts, "shelly.cloud.connected",        sum.cloud_connected);
          pushMetric(sysBatch, ts, "shelly.mqtt.connected",         sum.mqtt_connected);
          pushMetric(sysBatch, ts, "shelly.telemetry.script_runtime_ms",
                     scriptStartMs > 0 ? (nowMs() - scriptStartMs) : null);
          postAndThen(sysBatch, function () { postSwitches(ts); });
        });
      });
    });
  });
}

function postSwitches(ts) {
  let ids = DD_CFG.switch_ids || [0, 1, 2, 3];
  let i = 0;
  function next() {
    if (i >= ids.length) { postScriptsBatch(ts); return; }
    let series = buildSwitchSeries(ts, ids[i]);
    i++;
    postAndThen(series, next);
  }
  next();
}

function postScriptsBatch(ts) {
  callRpc("Script.List", {}, function (lst) {
    let scripts = (lst && lst.scripts) ? lst.scripts : [];
    let i = 0;
    let s = [];
    function walk() {
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
        walk();
      });
    }
    walk();
  });
}

// ---------- Bootstrap -------------------------------------------------------

// Seed each configured switch slot via Switch.GetStatus so the first
// post has values even before any status-change events fire.
function seedSwitches(done) {
  let ids = DD_CFG.switch_ids || [0, 1, 2, 3];
  let i = 0;
  function next() {
    if (i >= ids.length) { done(); return; }
    let id = ids[i];
    callRpc("Switch.GetStatus", { id: id }, function (r) {
      if (r) {
        let slot = ensureSwitchSlot(id);
        if (typeof r.output  === "boolean") slot.on      = r.output;
        if (typeof r.apower  === "number")  slot.power   = r.apower;
        if (typeof r.voltage === "number")  slot.voltage = r.voltage;
        if (typeof r.current === "number")  slot.current = r.current;
        if (typeof r.freq    === "number")  slot.freq    = r.freq;
        if (r.aenergy && typeof r.aenergy.total === "number") slot.energy = r.aenergy.total;
        if (r.temperature && typeof r.temperature.tC === "number") slot.mcu_tc = r.temperature.tC;
      }
      i++;
      next();
    });
  }
  next();
}

scriptStartMs = nowMs();
callRpc("KVS.Get", { key: "dd_cfg" }, function (res) {
  if (res) {
    try { mergeInto(DD_CFG, JSON.parse(res.value)); }
    catch (e) { log("!dd_cfg parse"); }
  }
  Shelly.addStatusHandler(onStatus);
  Shelly.addEventHandler(onEvent);
  seedSwitches(function () {
    Timer.set(10000, false, collectAndPost);
    Timer.set(DD_CFG.post_interval_s * 1000, true, collectAndPost);
    log("up: host=" + DD_CFG.host + " site=" + DD_CFG.site +
        " interval=" + DD_CFG.post_interval_s + "s" +
        (DD_CFG.api_key ? "" : " (no api_key: dry)"));
  });
});

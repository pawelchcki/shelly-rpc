// ============================================================================
// Bathroom exhaust fan controller — humidity-driven state machine
// ============================================================================
//
// Purpose
// -------
//   Drive a Shelly-controlled exhaust fan from indoor humidity, triggered
//   by a BLE BTHome hygro-temp button. The goal is not "run whenever the
//   button is pressed for N minutes" — it is "run only while venting is
//   actually reducing humidity, and stop when air replacement can't help".
//
// Why absolute humidity (AH), not RH
// ----------------------------------
//   Relative humidity is temperature-dependent: 70 %RH at 22 °C carries
//   ~13.5 g/m^3 of water; 70 %RH at 5 °C carries only ~4.8 g/m^3. The
//   quantity that matters for venting is absolute humidity (g of water per
//   m^3 of air). Pulling outdoor air in only helps when indoor AH is
//   higher than outdoor AH — on a humid summer afternoon the gap collapses
//   and the fan is useless no matter what RH shows. Magnus-Tetens formula:
//
//     es(T) = 6.112 · exp(17.62·T / (243.12 + T))       [hPa]
//     AH    = 216.7 · (RH/100) · es(T) / (273.15 + T)   [g/m^3]
//
//   Same algorithm year-round; seasonality emerges from physics.
//
// State machine
// -------------
//
//               +-------- rise < 0.3·trigger -------+
//               |                                   |
//               v                                   |
//          +--------+    d(AH)/dt > trigger    +--------+
//          |  IDLE  |-------------------------->|  ARMED  |
//          +--------+                           +--------+
//             ^                                      |
//             | cooldown_s elapsed                   | confirm_s elapsed,
//             |                                      | rise persisted
//             |                                      v
//             |                                 +----------+
//             |                                 |  VENTING |---+
//             |                                 +----------+   |
//             |                                      |         | futility
//             |              plateau:                |         | / floor
//             |            rate < min_eff            |         | / max_run_s
//             |                 |                    |         |
//             |                 v                    |         |
//             |           +-------------+            |         |
//             |           | EVALUATING  |-- bounce --+         |
//             |           | (fan OFF    |    back              |
//             |           |   90 s)     |    up                |
//             |           +-------------+                      |
//             |                 |                              |
//             |          kept falling                          |
//             |                 |                              |
//             |                 v                              v
//             |            +---------------------------------------+
//             +------------|             COOLDOWN (120 s)          |
//                          +---------------------------------------+
//
// Shower timeline (illustrative)
// ------------------------------
//
//     AH (g/m^3)
//      16 |                         ***
//         |                    *****   *****
//      14 |                 ***             ***
//         |              ***          |         ***
//      12 |           ***             |   .   .    ****
//         |        ***                |      .        *...
//      10 |     ***                   |                    *.._____
//         |____***............         __________________________________
//       8 | floor (out + margin)      |                     |        |
//         +---------------------------+---------------------+--------+->
//         0 IDLE    ARMED (60s)       VENTING           EVALUATING   t
//            ^                        fan ON            fan OFF 90 s
//            | rise > trigger &&      |                     |
//            | (outdoor fresh ||      +---------------------+
//            |  indoor RH >= 75%)     plateau after window  COOLDOWN 2 min
//
//   Key: the controller stops *while the bath is still humid*, because
//        either (a) the gap to outdoor has collapsed, (b) the fan has
//        plateaued (air exchange limited), or (c) a brief fan-off probe
//        shows AH is coasting down on its own via diffusion.
//
// Guards while VENTING
// --------------------
//   (a) Futility:   indoor_AH - effective_outdoor_AH  <  futility_gap.
//                   "Effective outdoor" is live Open-Meteo when fresh,
//                   else a Poland-climate fallback constant.
//   (b) Plateau:    over a sliding window, Δ(AH)/Δt drops below an
//                   adaptive min_eff_rate (clamped to 0.3× learned EMA
//                   so a weak fan isn't held to a textbook standard).
//   (c) Floor:      indoor_AH <= target_floor + hysteresis, where
//                   target_floor = max(effective_outdoor, pre-spike
//                   slow baseline) + margin.
//   (d) Max time:   max_run_s as a hard cap regardless of the others.
//
// Adaptation (KVS 'fan_stats')
// ---------------------------
//   Each completed cycle contributes to two EMAs:
//     ema_effective_rate  — typical g/m^3/min the fan achieves. Used to
//                           scale min_eff_rate so a small fan / big room
//                           is judged fairly.
//     ema_peak_rh         — typical peak RH during a cycle. Used to
//                           scale rise_trigger so seasonal drift in
//                           sensor noise doesn't cause spurious arming.
//   Saves are coalesced to at most one KVS.Set per stats_save_interval_s
//   (default 24 h) to protect flash endurance.
//
// Eco-mode compatibility
// ----------------------
//   When Sys.Config.eco_mode is enabled the Timer cadence can slip
//   noticeably. We rely on Shelly.addStatusHandler for bthomesensor:*
//   updates as the event-driven fast path; the 10 s tick and 5 min
//   sensor poll are safety nets. Every BLE broadcast from the sensor
//   wakes the BT chip and fires a status event regardless of eco state,
//   so decisions remain responsive.
//
// Persistence
// -----------
//   fan_cfg   — KVS JSON, overrides any default in CFG below.
//   fan_stats — KVS JSON, rolling last-N cycles + adaptive EMAs +
//               last_flush_ts (coalescing guard).
//
// Code style: mirrors scripts/appliance-monitor.js — no arrow functions,
// no template literals, mJS-safe ES5-ish with `let`.

// ---------- Defaults (overridable via KVS 'fan_cfg') ------------------------

let CFG = {
  // Open-Meteo query — default to Poznan. Override via KVS.
  lat: 52.41,
  lon: 16.93,

  // Which Switch component drives the fan relay.
  switch_id: 0,

  // BTHome sensor component ids. On a Shelly paired to a BLE hygro-temp
  // button the two measurements surface as separate bthomesensor:<id>
  // components; the user sets the correct ids in fan_cfg.
  bthome_humidity_id: 200,
  bthome_temperature_id: 201,

  // Arming thresholds. riseRate is a steady-state estimate of dAH/dt
  // (g/m3/s) from the fast-EMA lag. A moderate shower climbs ~10→20
  // g/m3 over 3–5 min → rate ≈ 0.03–0.05. 0.03 fires on typical
  // showers without being noise-sensitive.
  rise_trigger: 0.03,      // g/m3/s d(AH)/dt required to arm
  confirm_s:    60,        // rise must persist this long in ARMED

  // VENTING guards.
  futility_gap:    0.5,    // g/m3 below which venting cannot help
  plateau_window_s: 240,   // effectiveness check window (s)
  min_eff_rate:    0.1,    // g/m3/min floor for effectiveness
  hysteresis:      0.2,    // g/m3 added to floor before we call "done"
  margin:          0.5,    // g/m3 added to floor target

  // Phase timers.
  evaluate_s: 90,
  cooldown_s: 120,
  max_run_s:  1800,

  // Background cadence. Sensor readings are event-driven via
  // `Shelly.addStatusHandler`, so the poll is just a safety net — slow
  // enough to be eco-mode-friendly (timer callbacks can slip under
  // Sys.Config.eco_mode; the status handler still fires on every BLE
  // broadcast so the state machine stays responsive).
  weather_poll_s: 900,
  sensor_poll_s:  300,
  // Status-handler event path reacts in real time to BLE broadcasts,
  // so the tick is just a safety net + state machine heartbeat. At
  // 30 s it's 3× fewer wake-ups than the old 10 s, still below all
  // dwell timers (confirm_s 60 gives 2-tick headroom, evaluate_s 90,
  // cooldown_s 120, plateau_window_s 240, max_run_s 1800).
  tick_s:         30,
  sensor_stale_s: 600,

  // Flash-write budget: fan_stats KVS writes are coalesced to at most
  // once per this many seconds (default 24 h). Learning stats change
  // continuously in memory; saving on every cycle would burn flash
  // cells. The first write after boot bootstraps the entry.
  stats_save_interval_s: 86400,

  // EMA horizons.
  baseline_fast_tau: 120,  // s — current reference for rise detection
  baseline_slow_tau: 1200, // s — pre-spike baseline (target floor)

  // Learning buffer size.
  cycle_buffer: 10,

  // Outdoor fallback used when Open-Meteo is unreachable / stale.
  // Poland moderate continental climate: yearly mean AH is ~6–7 g/m^3,
  // with winter bottoms near 3 and summer peaks near 12. We pick a value
  // that is high enough to prevent over-enthusiastic venting in summer
  // and low enough to still allow venting in average conditions.
  fallback_outdoor_ah: 7.0,   // g/m^3
  // If outdoor data is stale / missing, additionally require indoor RH
  // to be above this before arming — "high enough to obviously be a
  // shower" gate, not a noise-level trigger.
  fallback_min_rh: 75.0,      // %
  // How long outdoor data stays trustworthy before we fall back.
  outdoor_max_age_s: 3600,    // 1 h
};

// ---------- Stats (overridable via KVS 'fan_stats') -------------------------

let STATS = {
  cycles: [],
  ema_effective_rate: 0.30, // seeded, clamped into [0.5x, 2x] at runtime
  ema_peak_rh:        85.0,
  last_flush_ts:      0,    // unixtime of the most recent KVS.Set
};

// ---------- Live state ------------------------------------------------------

let indoor  = { rh: null, t: null, ah: null, ts: 0 };
let outdoor = { rh: null, t: null, ah: null, ts: 0 };

let baselineFast = null;  // EMA of AH, tau = baseline_fast_tau
let baselineSlow = null;  // EMA of AH, tau = baseline_slow_tau (frozen not IDLE)
let lastAH       = null;
let lastAHts     = 0;

let fanOn = false;

let mode       = "IDLE";
let modeSince  = 0;

// Per-cycle accounting set when entering VENTING.
let cycleStartAH   = null;
let cycleStartTs   = 0;
let cyclePeakRH    = 0;
let plateauAnchorAH = null;
let plateauAnchorTs = 0;
let evalStartAH    = null;
let evalStartTs    = 0;
let cycleFloor     = null;

// ---------- Math helpers ----------------------------------------------------

function magnusEs(tC) {
  // Saturation vapour pressure in hPa (Magnus-Tetens).
  return 6.112 * Math.exp(17.62 * tC / (243.12 + tC));
}

function absoluteHumidity(rh, tC) {
  // g/m^3
  return 216.7 * (rh / 100.0) * magnusEs(tC) / (273.15 + tC);
}

function emaUpdate(prev, value, dt, tau) {
  if (prev === null) return value;
  if (dt <= 0)       return prev;
  let alpha = dt / (tau + dt);
  return prev + alpha * (value - prev);
}

function clamp(x, lo, hi) {
  if (x < lo) return lo;
  if (x > hi) return hi;
  return x;
}

function nowTs() {
  return Shelly.getComponentStatus("sys").unixtime;
}

// ---------- Logger ----------------------------------------------------------

function log(msg) {
  print("[fan] " + msg);
}

function fmt(x) {
  // mJS Math.round(x*100)/100 — keep log lines short.
  return Math.round(x * 100) / 100;
}

// ---------- RPC wrapper -----------------------------------------------------

function call(method, params, cb) {
  Shelly.call(method, params, function (res, errCode, errMsg) {
    if (errCode) {
      log("!" + method + " err=" + errCode + " " + (errMsg || ""));
    }
    if (cb) cb(res, errCode, errMsg);
  });
}

// ---------- Fan actuator ----------------------------------------------------

function setFan(on) {
  if (on === fanOn) return;
  fanOn = on;
  log("fan " + (on ? "ON" : "OFF"));
  // Route through the logging callback — a failed relay command is the
  // single highest-consequence failure in this controller (state
  // machine believes it switched but didn't), so we don't want it
  // silent. `call()` already logs `!Switch.Set err=...` on failure.
  call("Switch.Set", { id: CFG.switch_id, on: on }, function (res, err) {
    if (err) {
      // Roll back our in-memory belief so the next tick tries again.
      fanOn = !on;
    }
  });
}

// ---------- Baselines + rise detection --------------------------------------

function feedAH(ts, ah) {
  if (lastAHts === 0) {
    baselineFast = ah;
    baselineSlow = ah;
    lastAH       = ah;
    lastAHts     = ts;
    return;
  }
  let dt = ts - lastAHts;
  if (dt <= 0) return;
  baselineFast = emaUpdate(baselineFast, ah, dt, CFG.baseline_fast_tau);
  if (mode === "IDLE") {
    baselineSlow = emaUpdate(baselineSlow, ah, dt, CFG.baseline_slow_tau);
  }
  lastAH   = ah;
  lastAHts = ts;
}

function riseRate() {
  // Excess of current AH above the fast baseline, expressed as g/m^3/s.
  // With tau = fast baseline horizon, (ah - baselineFast) / tau gives a
  // smoothed approximation of d(AH)/dt that suppresses single-sample
  // noise while still rising promptly on a real step.
  if (baselineFast === null || indoor.ah === null) return 0;
  return (indoor.ah - baselineFast) / CFG.baseline_fast_tau;
}

function adaptiveRiseTrigger() {
  // A week of hot showers raises ema_peak_rh; a week of quiet ones lowers
  // it. Scale the trigger so "normal for the household" keeps triggering
  // even as sensor noise drifts.
  let scale = clamp(STATS.ema_peak_rh / 85.0, 0.6, 1.4);
  return CFG.rise_trigger * scale;
}

function adaptiveMinEffRate() {
  // Don't hold a weak fan to a textbook threshold. Clamp learned EMA
  // into a sane band so one bad cycle can't collapse it.
  let learned = clamp(STATS.ema_effective_rate, 0.05, 2.0);
  let rate    = 0.3 * learned;
  // Never demand more than the configured default.
  if (rate > CFG.min_eff_rate) rate = CFG.min_eff_rate;
  return rate;
}

// ---------- Cycle statistics ------------------------------------------------

function recordCycle(endAH, endReason) {
  if (cycleStartAH === null) return;
  let dAH     = cycleStartAH - endAH;
  let durS    = nowTs() - cycleStartTs;
  let durMin  = durS / 60.0;
  if (durMin < 0.1) durMin = 0.1;
  let rate    = dAH / durMin;
  let entry   = {
    dAH:     fmt(dAH),
    min:     fmt(endAH),
    peak_rh: Math.round(cyclePeakRH),
    rate:    fmt(rate),
  };
  STATS.cycles.push(entry);
  while (STATS.cycles.length > CFG.cycle_buffer) {
    STATS.cycles.shift();
  }
  if (rate > 0) {
    STATS.ema_effective_rate =
      0.7 * STATS.ema_effective_rate + 0.3 * rate;
  }
  if (cyclePeakRH > 0) {
    STATS.ema_peak_rh =
      0.7 * STATS.ema_peak_rh + 0.3 * cyclePeakRH;
  }
  maybeFlushStats();
  log("cycle dAH=" + entry.dAH + " rate=" + entry.rate +
      " peak_rh=" + entry.peak_rh + " reason=" + endReason);
  Shelly.emitEvent("fan.cycle_end", {
    dAH:        entry.dAH,
    duration_s: durS,
    peak_rh:    entry.peak_rh,
    rate:       entry.rate,
    end_reason: endReason,
  });
}

// Coalesce flash writes: persist fan_stats to KVS at most once per
// stats_save_interval_s (default 24 h). The first write after boot
// bootstraps the entry and subsequent writes happen on a rolling daily
// cadence. In-memory stats remain up-to-date regardless.
function maybeFlushStats() {
  let now = nowTs();
  if (STATS.last_flush_ts !== 0 &&
      (now - STATS.last_flush_ts) < CFG.stats_save_interval_s) {
    return;
  }
  // Serialize before the call so the payload reflects the current
  // EMAs; bump last_flush_ts only on successful persist so a failed
  // write doesn't start a fresh 24 h coalescing window.
  STATS.last_flush_ts = now;
  let payload = JSON.stringify(STATS);
  call("KVS.Set", {
    key:   "fan_stats",
    value: payload,
  }, function (res, err) {
    if (err) {
      STATS.last_flush_ts = 0;  // retry next cycle
      return;
    }
    log("flushed fan_stats");
  });
}

function clearCycle() {
  cycleStartAH    = null;
  cycleStartTs    = 0;
  cyclePeakRH     = 0;
  plateauAnchorAH = null;
  plateauAnchorTs = 0;
  evalStartAH     = null;
  evalStartTs     = 0;
  cycleFloor      = null;
}

// ---------- Mode transitions ------------------------------------------------

function setMode(next) {
  let prev  = mode;
  mode      = next;
  modeSince = nowTs();
  log("mode=" + next);
  // Generic Shelly event — any listener (telemetry, MQTT bridge, future
  // tools) can consume this via Shelly.addEventHandler. The controller
  // stays oblivious to its consumers.
  Shelly.emitEvent("fan.mode", { from: prev, to: next });
}

function outdoorIsFresh() {
  if (outdoor.ah === null) return false;
  if (outdoor.ts === 0)    return false;
  return (nowTs() - outdoor.ts) <= CFG.outdoor_max_age_s;
}

function effectiveOutdoorAH() {
  // Used wherever we compare indoor against outdoor. Falls back to the
  // configured Poland-climate constant when live data isn't usable.
  if (outdoorIsFresh()) return outdoor.ah;
  return CFG.fallback_outdoor_ah;
}

function targetFloor() {
  // Floor we're trying to reach: whichever is larger between the
  // effective outdoor AH and the slow (pre-spike) indoor baseline,
  // plus a small margin.
  let base = baselineSlow !== null ? baselineSlow : 0;
  let out  = effectiveOutdoorAH();
  if (out > base) base = out;
  return base + CFG.margin;
}

// ---------- State machine ---------------------------------------------------

function tick() {
  let ts = nowTs();

  // Heartbeat: republish current mode every tick so any listener that
  // booted after the last real transition can still pick it up.
  // Telemetry distinguishes heartbeats from transitions by from===to.
  Shelly.emitEvent("fan.mode", { from: mode, to: mode });

  // Stale sensor protection: never run the fan blindly.
  if (indoor.ts > 0 && (ts - indoor.ts) > CFG.sensor_stale_s) {
    if (mode !== "IDLE") {
      log("sensor stale — forcing IDLE");
      setFan(false);
      clearCycle();
      setMode("IDLE");
    }
    return;
  }
  if (indoor.ah === null) return;

  if (indoor.rh !== null && indoor.rh > cyclePeakRH) cyclePeakRH = indoor.rh;

  if (mode === "IDLE") {
    // If outdoor data is stale/missing we refuse to arm on rise-rate
    // alone: require a clearly elevated indoor RH as well. This avoids
    // firing the fan on sensor drift when we can't verify that venting
    // would help.
    let riseOk = riseRate() > adaptiveRiseTrigger();
    let gateOk = outdoorIsFresh() ||
                 (indoor.rh !== null && indoor.rh >= CFG.fallback_min_rh);
    if (riseOk && gateOk) {
      setMode("ARMED");
    }
    return;
  }

  if (mode === "ARMED") {
    // The rise must persist — a door opening fogs the sensor briefly.
    if (riseRate() < 0.3 * adaptiveRiseTrigger()) {
      setMode("IDLE");
      return;
    }
    if (ts - modeSince >= CFG.confirm_s) {
      cycleStartAH    = indoor.ah;
      cycleStartTs    = ts;
      plateauAnchorAH = indoor.ah;
      plateauAnchorTs = ts;
      cycleFloor      = targetFloor();
      log("vent start ah=" + fmt(indoor.ah) +
          " floor=" + fmt(cycleFloor) +
          " out=" + (outdoor.ah === null ? "?" : fmt(outdoor.ah)));
      setFan(true);
      setMode("VENTING");
    }
    return;
  }

  if (mode === "VENTING") {
    // Guard (a): futility — can't vent below effective outdoor AH.
    // Uses live outdoor data when fresh, else the Poland fallback.
    let effOut = effectiveOutdoorAH();
    if ((indoor.ah - effOut) < CFG.futility_gap) {
      log("stop: futility gap (out=" + fmt(effOut) +
          (outdoorIsFresh() ? ",live" : ",fallback") + ")");
      setFan(false);
      recordCycle(indoor.ah, "futility");
      clearCycle();
      setMode("COOLDOWN");
      return;
    }

    // Guard (c): floor reached (check before plateau so a smooth cycle
    // reports "done" rather than "plateau").
    if (cycleFloor !== null &&
        indoor.ah <= (cycleFloor + CFG.hysteresis)) {
      log("stop: floor reached");
      setFan(false);
      recordCycle(indoor.ah, "floor");
      clearCycle();
      setMode("COOLDOWN");
      return;
    }

    // Guard (b): effectiveness plateau. Slide an N-second window and
    // compare the rate to adaptiveMinEffRate().
    let windowAge = ts - plateauAnchorTs;
    if (windowAge >= CFG.plateau_window_s) {
      let rate    = (plateauAnchorAH - indoor.ah) / (windowAge / 60.0);
      let minRate = adaptiveMinEffRate();
      if (rate < minRate) {
        log("plateau rate=" + fmt(rate) + " min=" + fmt(minRate) +
            " -> evaluate");
        setFan(false);
        evalStartAH = indoor.ah;
        evalStartTs = ts;
        setMode("EVALUATING");
        return;
      }
      plateauAnchorAH = indoor.ah;
      plateauAnchorTs = ts;
    }

    // Hard safety cap.
    if (ts - cycleStartTs >= CFG.max_run_s) {
      log("stop: max_run_s");
      setFan(false);
      recordCycle(indoor.ah, "max_run");
      clearCycle();
      setMode("COOLDOWN");
    }
    return;
  }

  if (mode === "EVALUATING") {
    if (ts - evalStartTs >= CFG.evaluate_s) {
      let drift = evalStartAH - indoor.ah;
      // If AH dropped meaningfully without the fan, it was diffusing on
      // its own — don't keep fan-cycling. Else the fan was doing work.
      if (drift > 0.1) {
        log("eval: AH still falling on its own (" + fmt(drift) +
            ") -> cooldown");
        recordCycle(indoor.ah, "eval_drift");
        clearCycle();
        setMode("COOLDOWN");
      } else {
        log("eval: bounce-back (" + fmt(drift) + ") -> resume venting");
        setFan(true);
        plateauAnchorAH = indoor.ah;
        plateauAnchorTs = ts;
        setMode("VENTING");
      }
    }
    return;
  }

  if (mode === "COOLDOWN") {
    if (ts - modeSince >= CFG.cooldown_s) {
      setMode("IDLE");
    }
    return;
  }
}

// ---------- Indoor sensor ---------------------------------------------------

function updateIndoor(rh, tC) {
  if (typeof rh !== "number" || typeof tC !== "number") return;
  if (rh < 0 || rh > 100) return;
  if (tC < -40 || tC > 80) return;
  let ts = nowTs();
  let ah = absoluteHumidity(rh, tC);
  indoor = { rh: rh, t: tC, ah: ah, ts: ts };
  feedAH(ts, ah);
}

// Indoor readings arrive two ways:
//   1. Event-driven (fast path): Shelly.addStatusHandler fires on every
//      BTHome broadcast decoded by the device. This keeps the state
//      machine reactive even under Sys eco_mode where Timer callbacks
//      can slip.
//   2. Polling (safety net): BTHomeSensor.GetStatus every sensor_poll_s
//      in case a status event is missed.
let cachedRH = null;
let cachedT  = null;

function ingestSensorReading(id, value) {
  if (typeof value !== "number") return;
  if (id === CFG.bthome_humidity_id)    cachedRH = value;
  if (id === CFG.bthome_temperature_id) cachedT  = value;
  maybeCommitIndoor();
}

function pollIndoor() {
  call("BTHomeSensor.GetStatus",
       { id: CFG.bthome_humidity_id },
       function (res) {
    if (res && typeof res.value === "number") {
      ingestSensorReading(CFG.bthome_humidity_id, res.value);
    }
  });
  call("BTHomeSensor.GetStatus",
       { id: CFG.bthome_temperature_id },
       function (res) {
    if (res && typeof res.value === "number") {
      ingestSensorReading(CFG.bthome_temperature_id, res.value);
    }
  });
}

function maybeCommitIndoor() {
  if (cachedRH !== null && cachedT !== null) {
    updateIndoor(cachedRH, cachedT);
    // Poke the state machine right away — event-driven decisions are
    // the whole point of the status-handler fast path under eco mode.
    tick();
  }
}

function onStatusEvent(ev) {
  if (!ev) return;
  let comp = ev.component;
  if (typeof comp !== "string") return;
  // Filter: only bthomesensor components we care about.
  if (comp.indexOf("bthomesensor:") !== 0) return;
  let d = ev.delta;
  if (!d || typeof d.value !== "number") return;
  ingestSensorReading(d.id, d.value);
}

// ---------- Outdoor (Open-Meteo) --------------------------------------------

// Factored out so the minifier can't fold `var parsed = JSON.parse(...)`
// into a later `var` declaration — mJS mishandles that hoisting pattern
// and crashed with "Cannot read property 'current' of undefined".
function parseOpenMeteo(body) {
  let parsed = null;
  try { parsed = JSON.parse(body); } catch (e) { return null; }
  if (parsed === null) return null;
  let c = parsed.current;
  if (c === null || typeof c !== "object") return null;
  let rh = c.relative_humidity_2m;
  let tC = c.temperature_2m;
  if (typeof rh !== "number") return null;
  if (typeof tC !== "number") return null;
  return { rh: rh, tC: tC };
}

// Fetches outdoor weather. The optional `done` callback reports
// success/failure — used by the boot path to decide whether to run the
// degraded-mode blink signal. The periodic timer just passes a noop.
function fetchWeather(done) {
  let url =
    "https://api.open-meteo.com/v1/forecast?latitude=" + CFG.lat +
    "&longitude=" + CFG.lon +
    "&current=temperature_2m,relative_humidity_2m";
  call("HTTP.GET", { url: url, timeout: 10 }, function (res, err) {
    if (err || !res || res.code !== 200 || !res.body) {
      log("!weather http=" + (err ? err : (res && res.code)));
      if (done) done(false);
      return;
    }
    let cur = parseOpenMeteo(res.body);
    if (cur === null) {
      log("!weather parse");
      if (done) done(false);
      return;
    }
    outdoor = {
      rh: cur.rh,
      t:  cur.tC,
      ah: absoluteHumidity(cur.rh, cur.tC),
      ts: nowTs(),
    };
    log("out rh=" + fmt(cur.rh) + " t=" + fmt(cur.tC) +
        " ah=" + fmt(outdoor.ah));
    if (done) done(true);
  });
}

function pollWeather() { fetchWeather(null); }

// ---------- Bootstrap -------------------------------------------------------

function mergeInto(target, src) {
  for (let k in src) {
    if (src.hasOwnProperty(k)) target[k] = src[k];
  }
}

function loadCfg(done) {
  call("KVS.Get", { key: "fan_cfg" }, function (res) {
    if (res) {
      try {
        let loaded = JSON.parse(res.value);
        mergeInto(CFG, loaded);
      } catch (e) {
        log("!fan_cfg parse");
      }
    }
    done();
  });
}

function loadStats(done) {
  call("KVS.Get", { key: "fan_stats" }, function (res) {
    if (res) {
      try {
        let loaded = JSON.parse(res.value);
        mergeInto(STATS, loaded);
      } catch (e) {
        log("!fan_stats parse");
      }
    }
    done();
  });
}

function startNormalOps(weatherOk) {
  // Event-driven sensor updates first — this is the eco-mode-friendly
  // fast path that keeps us responsive even when the Timer cadence
  // slips. The polling timer below is just a safety net.
  Shelly.addStatusHandler(onStatusEvent);
  pollIndoor();
  Timer.set(CFG.tick_s * 1000,         true, tick);
  Timer.set(CFG.sensor_poll_s * 1000,  true, pollIndoor);
  Timer.set(CFG.weather_poll_s * 1000, true, pollWeather);
  log("up: switch=" + CFG.switch_id +
      " bthome_rh=" + CFG.bthome_humidity_id +
      " bthome_t="  + CFG.bthome_temperature_id +
      " stats_save_s=" + CFG.stats_save_interval_s +
      " outdoor=" + (weatherOk ? "live" : "fallback"));
  // Generic boot event — telemetry forwards this to Datadog with an
  // info/warning alert level depending on whether outdoor data is
  // available. No relay cycling, so no motor / capacitor / contact
  // wear on the 20 W fan.
  Shelly.emitEvent("fan.boot", {
    status: weatherOk ? "ok" : "degraded_no_outdoor",
  });
  // Publish initial mode so any listener (telemetry) has a value to
  // report before the first real transition.
  Shelly.emitEvent("fan.mode", { from: null, to: mode });
}

function start() {
  // Make sure the fan is off at boot — we don't inherit prior state
  // because the state machine restarts from IDLE.
  setFan(false);
  // Probe outdoor weather. The controller itself stays oblivious to
  // its observers — we just emit a boot event (see startNormalOps)
  // and whatever is listening can react.
  fetchWeather(function (ok) { startNormalOps(ok); });
}

loadCfg(function () {
  loadStats(function () {
    start();
  });
});

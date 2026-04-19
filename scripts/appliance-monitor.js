// Washer + dryer monitor for a Shelly PowerStrip (Gen4).
//
// Watches two switches' real-time power draw, runs each through a small
// state machine (idle -> starting -> running -> finishing -> idle), and
// fires a Shelly Cloud "manual_run" scene when each appliance starts and
// when it finishes. The scene is what actually delivers the push
// notification — see CLAUDE.md for why we trigger via HTTP.GET instead
// of Shelly.emitEvent().
//
// Configuration lives in two KVS keys:
//
//   "cloud" — JSON object, written by `shellyctl cloud init`:
//     { u:  "https://shelly-96-eu.shelly.cloud",  // Cloud API base URL
//       k:  "<auth_key>",                         // Cloud auth key
//       ws: <washer-start scene id>,
//       wd: <washer-done scene id>,
//       ds: <dryer-start scene id>,
//       dd: <dryer-done scene id> }
//
//   "mon"   — JSON object, persisted by this script across reboots:
//     { w: <washer state>, d: <dryer state> }
//
// Each appliance state object has:
//   s:     "idle" | "starting" | "running" | "finishing"
//   since: unix time the current state was entered
//   rs:    unix time the current run started (set when entering "running")
//   re:    aenergy.total at the start of the run (Wh baseline for delta)
//
// This source is intentionally verbose. Build for the device with:
//   shellyctl compile scripts/appliance-monitor.js -o build/appliance-monitor.js
// or upload directly with:
//   shellyctl script <host> upload --minify scripts/appliance-monitor.js

// ---------- Configuration ---------------------------------------------------

let WASHER_SWITCH_ID = 3;
let DRYER_SWITCH_ID  = 2;

// Real-time power thresholds (watts). The hysteresis between START and
// STOP keeps a borderline draw from oscillating between states.
let START_THRESHOLD_W = 10;
let STOP_THRESHOLD_W  = 3;

// Power must stay above START_THRESHOLD_W for this long before we treat
// a load as "really started". Filters out brief inrush spikes from
// other appliances on adjacent outlets.
let START_CONFIRM_SECONDS = 10;

// How long power must stay below STOP_THRESHOLD_W before we declare a
// cycle done. Washer cycles have long quiet stretches mid-cycle (drain
// + refill), so it needs a longer dwell than the dryer.
let WASHER_DONE_SECONDS = 300;
let DRYER_DONE_SECONDS  = 120;

// How often we poll Switch.GetStatus on each switch (milliseconds).
let POLL_INTERVAL_MS = 2000;

// ---------- Mutable state ---------------------------------------------------

let washerState = { s: "idle", since: 0, rs: 0, re: 0 };
let dryerState  = { s: "idle", since: 0, rs: 0, re: 0 };

// Loaded from KVS at startup; remains null until then.
let cloud = null;

// ---------- Helpers ---------------------------------------------------------

function currentUnixTime() {
  return Shelly.getComponentStatus("sys").unixtime;
}

function persistState() {
  Shelly.call("KVS.Set", {
    key:   "mon",
    value: JSON.stringify({ w: washerState, d: dryerState }),
  });
}

function triggerCloudScene(sceneId) {
  // Skip if cloud config didn't load or this slot wasn't provisioned.
  if (!cloud || !sceneId) {
    return;
  }
  let url = cloud.u + "/scene/manual_run?auth_key=" + cloud.k +
            "&id=" + sceneId;
  Shelly.call("HTTP.GET", { url: url }, function (response, errorCode) {
    if (errorCode) {
      print("!scene " + errorCode);
    }
  });
}

// ---------- State machine ---------------------------------------------------

function checkAppliance(state, switchId, doneSeconds, startSceneId, doneSceneId) {
  Shelly.call("Switch.GetStatus", { id: switchId }, function (status) {
    if (!status) {
      return;
    }
    let powerW       = status.apower;
    let now          = currentUnixTime();
    let energyTotal  = status.aenergy ? status.aenergy.total : 0;

    if (state.s === "idle") {
      // Watch for power crossing the start threshold.
      if (powerW > START_THRESHOLD_W) {
        state.s     = "starting";
        state.since = now;
      }
      return;
    }

    if (state.s === "starting") {
      // Either the spike was transient (drop back to idle) or it has
      // held long enough to call this a real run.
      if (powerW <= START_THRESHOLD_W) {
        state.s     = "idle";
        state.since = 0;
      } else if (now - state.since >= START_CONFIRM_SECONDS) {
        state.s     = "running";
        state.rs    = now;
        state.re    = energyTotal;
        state.since = now;
        persistState();
        print("sw" + switchId + " started");
        triggerCloudScene(startSceneId);
      }
      return;
    }

    if (state.s === "running") {
      // First time we see power drop below the stop threshold, move to
      // "finishing" — but stay there long enough to ride out mid-cycle
      // pauses (washer drain phase, etc).
      if (powerW < STOP_THRESHOLD_W) {
        state.s     = "finishing";
        state.since = now;
      }
      return;
    }

    if (state.s === "finishing") {
      if (powerW >= START_THRESHOLD_W) {
        // False alarm — the cycle picked back up. Return to running.
        state.s     = "running";
        state.since = now;
      } else if (now - state.since >= doneSeconds) {
        let durationMinutes = Math.round((now - state.rs) / 60);
        let energyWh        = Math.round((energyTotal - state.re) * 10) / 10;
        print("sw" + switchId + " done " + durationMinutes + "min " +
              energyWh + "Wh");
        triggerCloudScene(doneSceneId);
        state.s     = "idle";
        state.rs    = 0;
        state.re    = 0;
        state.since = 0;
        persistState();
      }
    }
  });
}

function tick() {
  checkAppliance(
    washerState,
    WASHER_SWITCH_ID,
    WASHER_DONE_SECONDS,
    cloud && cloud.ws,
    cloud && cloud.wd
  );
  checkAppliance(
    dryerState,
    DRYER_SWITCH_ID,
    DRYER_DONE_SECONDS,
    cloud && cloud.ds,
    cloud && cloud.dd
  );
}

// ---------- Boot ------------------------------------------------------------

// Load cloud config, then restore persisted state, then start polling.
// We chain via callbacks because mJS doesn't have promises and Shelly.call
// is async.
Shelly.call("KVS.Get", { key: "cloud" }, function (cloudResult) {
  if (cloudResult) {
    try {
      cloud = JSON.parse(cloudResult.value);
    } catch (e) {
      print("!cloud");
    }
  }

  Shelly.call("KVS.Get", { key: "mon" }, function (monResult) {
    if (monResult) {
      try {
        let saved = JSON.parse(monResult.value);
        // Reset `since` to "now" so dwell timers don't fire immediately
        // on a restart that lands mid-cycle.
        if (saved.w) {
          washerState       = saved.w;
          washerState.since = currentUnixTime();
        }
        if (saved.d) {
          dryerState        = saved.d;
          dryerState.since  = currentUnixTime();
        }
        print("rs w:" + washerState.s + " d:" + dryerState.s);
      } catch (e) {
        print("!mon");
      }
    }
    print("mon sw" + WASHER_SWITCH_ID + " sw" + DRYER_SWITCH_ID);
    Timer.set(POLL_INTERVAL_MS, true, tick);
  });
});

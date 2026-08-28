# Simulator Strategy

The Device Simulator is not a testing convenience. It is the component that
makes M0–M8 possible without hardware, and it is the reference implementation of
the device side of the protocol.

Its correctness requirement is unusual: **it must never be more permissive than
the firmware.** A lenient simulator produces a test suite that validates a
system which does not exist.

---

## 1. What the simulator is

- A host Rust binary speaking the identical MQTT protocol
  ([docs/protocol/mqtt-v1.md](../protocol/mqtt-v1.md))
- A plausible physical model of soil, water, and a pump
- A fault injector
- A virtual clock

## 2. What the simulator is not

- **Not a soil-physics model.** The drying curve is a defensible approximation
  chosen to exercise control logic, not a claim about real soil. Nothing in the
  system's correctness depends on its numerical accuracy.
- **Not more permissive than hardware.** See §5.
- **Not a place for irrigation intelligence.** Like the firmware, it obeys or
  refuses; it does not decide.

---

## 3. The physical model

### Soil moisture

Exponential decay toward a floor, with the rate scaled by temperature:

```text
dVWC/dt = -k * (VWC - VWC_floor) * temp_factor(T)

k             drying rate constant, configurable (default 0.06 /hour)
VWC_floor     residual moisture, default 8.0 %
temp_factor   1.0 at 21 °C, +3 % per °C above, floored at 0.5
```

Exponential rather than linear because wet soil loses water faster than dry
soil, which is both physically true and the behaviour that matters: it means a
plant approaches the dry threshold gradually rather than falling off a cliff,
which is what exercises the `Drying` → `DryConfirmed` debounce.

### Watering response

Water does not appear in the sensor instantly. Delivered volume enters a
"pending absorption" pool that transfers to measured VWC with a time constant:

```text
ΔVWC_total   = delivered_ml / (pot_volume_ml * soil_factor)
absorption:    63 % of the change within absorption_tau (default 6 min)
overshoot:     surface probes briefly over-read by up to 15 % of ΔVWC,
               decaying over 2 min
drainage:      volume beyond field_capacity_vwc is lost, not measured
```

The overshoot and the drainage cap are the two features that matter most,
because they are the ones that punish a naive controller. A controller that
doses again immediately after seeing an overshoot-inflated reading, or that
believes a dose beyond field capacity raised moisture proportionally, will
misbehave — and should, in tests, before it does so with real water.

### Pot weight

```text
weight_g = dry_weight_g + water_g + noise
water_g += delivered_ml            (immediate — unlike VWC)
water_g -= evapotranspiration(dt)
```

Weight responds immediately while VWC lags. That divergence is precisely what
makes weight useful for detecting manual watering and for catching a pump that
runs without delivering (failure-model §5.1), so the simulator must reproduce it.

### Tank, leak, EC

```text
tank_percent  -= delivered_ml / tank_capacity_ml * 100
leak_detected  = injected only
ec_us_cm       = base_ec * (reference_vwc / current_vwc)  ± noise
                 + fertilisation events (step increase, then slow decay)
```

EC rising as soil dries is the real relationship — concentration increases as
water leaves — and reproducing it keeps the EC trend logic honest.

### Sensor noise

Gaussian noise on every reading (default σ: VWC 0.3 %, temperature 0.1 °C,
weight 2 g). Noise is on by default because a controller that only works on
clean signals does not work.

---

## 4. Virtual time

```text
virtual_now = anchor_real + (real_now - anchor_real) * scale
```

`--time-scale 600` runs 10 simulated minutes per real second: a full multi-dose
cycle with two 15-minute absorption waits completes in about six seconds.

The scaled quantity is the device's **monotonic** clock; its wall clock is the
last applied `edge.time` plus that monotonic elapsed time, so published
timestamps stay plausible UTC values anchored to a real Edge instant while
running fast. There is no separate wall-clock anchor, because after the
2026-08-26 pass a device has no wall time of its own to anchor
([ADR-013](../adr/013-clock-and-time-semantics.md)).

A tick's worth of virtual time is applied to the models in bounded steps of at
most one virtual second, so the drying curve, the absorption pool, and the
overshoot decay evolve identically at every scale. Applying a whole minute of
virtual time as a single step would make each of them resolve to one jump, and
an accelerated test would then be exercising something other than the system it
claims to.

The edge and simulator MUST run at the same scale in a test topology, from one
compose variable ([ADR-013](../adr/013-clock-and-time-semantics.md)). M8-004
asserts both report the same scale at startup.

---

## 5. The permissiveness rule — the critical constraint

The simulator has **exactly one** code path to actuation, and it goes through
the shared validator:

```rust
match rhizo_mqtt_contract::validate_water_command(&cmd, &self.guard_state()) {
    CommandVerdict::Accept { effective_ml, run_ms, clamped } => self.run_pump(...),
    CommandVerdict::Reject(reason)          => self.publish_rejection(reason),
    CommandVerdict::AlreadyExecuted { previous } => self.republish(previous),
}
```

There is no `--allow-any-dose`, no debug bypass, no test-only relaxation.
Removing that call would be a visible change to a safety-critical file, and
`safety_007_simulator_refuses_like_hardware` asserts the refusal directly.

The simulator also maintains the same durable state the firmware does — a
16-entry command dedup ring and an interrupted-dose record — persisted to a
small JSON file so that `--restart` mid-dose reproduces SAFETY-011 behaviour
faithfully.

---

## 6. Fault injection

Every fault in [failure-model.md](../architecture/failure-model.md) that
originates at the device must be reproducible on demand.

| Flag / control command | Effect |
|---|---|
| `--fault disconnect:<sec>` | drop the MQTT connection for N seconds |
| `--fault duplicate:<rate>` | publish a fraction of messages twice |
| `--fault reorder:<rate>` | delay a fraction of messages past the next |
| `--fault invalid-soil:<rate>` | emit out-of-range or `null` moisture |
| `--fault stuck-sensor` | repeat one bit-identical reading forever |
| `--fault clock-unsync` | report `clock_synced: false` |
| `--fault clock-skew:<sec>` | offset `device_time_ms` |
| `--fault leak` | assert the leak sensor |
| `--fault tank-empty` | drive tank level to 0 |
| `--fault pump-no-delivery` | run the pump but deliver no water |
| `--fault pump-stuck-on` | fail to de-energise (tests the run-timer guard) |
| `--fault restart-mid-dose` | terminate during actuation |
| `--fault restart` | full restart with a new `boot_id` |
| `--fault miss-wake:<n>` | skip N wake cycles without announcing (M5-021) |
| `--fault sleep-without-announcing` | disconnect uncleanly from battery mode, firing the Last Will (M5-021) |

Faults are also settable at runtime through the simulator's small control API
(`POST /sim/fault`) so scenario tests can inject mid-run without a restart. That
control API is simulator-only and does not exist in firmware.

The last two faults arrive with **battery power mode** in M5-021
([ADR-018](../adr/018-battery-and-deep-sleep-device-mode.md)). `--power-mode
battery` makes the simulator sleep, wake, sample, connect, publish, and announce
its next sleep on the accelerated clock, which is what gives SCEN-110…SCEN-117 a
producer four milestones before firmware exists.

The two faults exist because the *distinction* is what needs testing, not the
happy path: `miss-wake` produces a device that is overdue, and
`sleep-without-announcing` produces one that vanished. Both must reach `isolated`
and neither may be reported as sleeping (SAFETY-021).

---

## 7. Configuration

```bash
cargo run -p device-simulator -- \
  --device-id plant-node-01 \
  --broker mqtt://localhost:1883 \
  --initial-moisture 42 \
  --drying-rate 0.06 \
  --pot-volume-ml 2500 \
  --tank-capacity-ml 2000 \
  --ml-per-second 8.2 \
  --telemetry-interval 300 \
  --time-scale 600 \
  --sensors soil,tank,leak
```

`--sensors` controls which telemetry topics are published, so the
missing-sensor lockout paths (SAFETY-005, SAFETY-012) can be exercised by
omission rather than by injected failure.

A `--scenario <file.yaml>` mode replays a scripted sequence of state changes and
faults for reproducible test runs.

---

## 8. Multi-device operation

One process per device (`docker compose up --scale device-simulator=5` with a
device id derived from the replica index). Separate processes rather than
threads, because it reproduces independent connections, independent LWTs, and
independent failure — which is what M13 needs to test.

---

## 9. Fixture drift detection

The simulator can capture its published messages to disk
(`--capture-fixtures <dir>`). M2-011 adds a CI check that diffs a capture
against `test/fixtures/protocol/valid/`, so fixtures that stop reflecting real
output are detected rather than assumed correct.

---

## 10. Replacement by hardware

When the ESP32 arrives, the simulator is not deleted. It remains:

- the fast path for CI (no hardware in the loop)
- the fault injector for cases dangerous or tedious to reproduce physically
  (a leak, an empty tank, a restart mid-dose)
- the conformance reference in M9-014

The M9 conformance test — same scenario against simulator and against
firmware-with-fake-adapters, asserting identical message sequences — is what
converts "the simulator is a good stand-in" from a hope into a checked property.

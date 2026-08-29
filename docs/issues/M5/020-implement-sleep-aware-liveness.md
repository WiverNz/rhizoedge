# Issue M5-020 — Implement sleep-aware device liveness on the edge

**Status:** SUPERSEDED before M5 started — **wholly**, including the four
`safety_021_*` tests and the derive-never-store rule below, which the dated
2026-08-28 post-M4 review completed. Delivered by the 2026-08-28 post-M4
battery-compatibility correction and that review, both recorded in
`docs/reports/M4.md`; SAFETY-021 is enforced in M4. This issue remains as
planning history and is **not** an M5 implementation task. Nothing in it is
outstanding.

Two of its own instructions were not honoured by the original correction and were
fixed by the review, which is the reason this file is worth keeping:

- *"Derive, never store, the connectivity value."* The correction stored it and
  let the timer be its only writer. The reported value is now derived on every
  read; the column remains as the timer's record.
- *"`safety_021_*` tests are green."* Only one of the four named tests existed.
  All four now do, under the names the invariant registry gives them.

Its staleness instruction was honoured but proved incomplete: feeding the
device-declared wake interval into `max(15 min, 3 × interval)` is also feeding it
into SAFETY-005's `max_sample_age`. The review split the two calculations and
bounded the battery one (PRD 040 F-040-26).

**Milestone:** M5 · **PRD:** [PRD 050](../../prd/050-plant-model-and-recommendations.md) · **Depends on:** M5-019, M4-012, M4-004

## Context

M4 delivered the device registry and its liveness model, and it is complete: a
device is `online` or `offline`, and its connectivity is `connected`,
`isolated`, or `reconciling`. That model has exactly one gap for a battery
device — **a device that sleeps disconnects cleanly roughly a hundred times a
day, and the registry as delivered reports every one of those as an offline
device.**

The later focused correction changed this ownership decision: M4's delivered
report now carries a dated addendum, without pretending the original M4 gate
contained the battery model.

## Goal

Let the registry distinguish an announced, bounded sleep from a device that has
stopped waking — and never let the first hide the second.

## Scope

- Persist `power_mode` and `wake_interval_seconds` per device, from the
  configuration the edge published
- Derive `expected_wake_at` and `overdue_at` from the edge's own `received_at`,
  per [ADR-018](../../adr/018-battery-and-deep-sleep-device-mode.md) §2
- Add `sleeping` to the derived connectivity value, alongside the existing
  `connected`, `isolated`, and `reconciling`
- Transition an overdue sleeper to `isolated` **from the liveness timer**, with
  no inbound message required (F-040-09)
- `missed_wake_count`, reset on a successful wake
- Scale the staleness threshold for a battery device: `max(15 min, 3 × interval)`
  where a battery device's effective interval is its wake interval
- Events `device_slept`, `device_woke`, `device_wake_missed`
- Metrics: `devices_sleeping` gauge, `device_wake_missed_total` counter
- Expose `connectivity`, `expected_wake_at`, `power_mode`, and
  `missed_wake_count` in `GET /api/v1/devices/{id}`

## Non-goals

- Holding or delivering commands for a sleeping device (M6-022).
- Any change to the plant model, recommendations, or thresholds. A sleeping
  device's samples are ordinary samples.
- Making a device sleep. Nothing here writes a power mode; M5-021 and M9-019
  produce devices that actually sleep.

## Dependencies

- M5-019
- M4-012
- M4-004

## Implementation notes

**Derive, never store, the connectivity value** — for the same reason M4-004
derives `stale`. What is stored is the raw material: the last announcement, its
`received_at`, and the configured interval. A stored state needs a writer, and a
writer that fails leaves a device permanently asleep, which is the precise
failure SAFETY-021 exists to prevent.

Note the asymmetry with `stale`: `missed_wake_count` **is** stored, because it
counts events rather than describing the present, and because the timer is its
only possible writer.

The device's own `expected_wake_ms` and `connectivity.mode` are advisory, exactly
as M4-012 already established for `connectivity.mode`. A device that claims to be
asleep until next year is asleep until the edge's window closes, and no further.

An always-on device that publishes a clean `shutdown` is unchanged: `offline`,
`isolated`, no wake window. Only `reason: "sleeping"` from a device the edge has
configured for battery mode opens a window. A battery-mode device that fires its
Last Will is `isolated`, because a Last Will is by definition not an
announcement.

Staleness deserves care rather than a new formula. A 900-second wake interval
already yields `max(15 min, 45 min) = 45 min` under the existing rule, which
tolerates one missed wake — the same "two lost messages" tolerance the formula
was designed for. What must change is only *which* interval feeds it for a
battery device.

## Acceptance criteria

> **Deliberately unticked.** Every criterion below was met by the dated
> 2026-08-28 post-M4 correction and its review, not by M5, and ticking them here
> would claim work this milestone did not do. M5 added a *producer* for the
> behaviour they describe (M5-021); the criteria themselves belong to M4, where
> SAFETY-021 is enforced.

- [ ] A battery device inside its window derives `sleeping`, not `offline`.
- [ ] `expected_wake_at` is computed from the **edge** `received_at`, and an
      announced `expected_wake_ms` far in the future does not extend it.
- [ ] A device past `overdue_at` derives `isolated`, and the transition is made
      **by the timer with no inbound message**.
- [ ] An LWT from a battery device derives `isolated`, never `sleeping`.
- [ ] An offline status with an unrecognised `reason` derives `isolated`.
- [ ] An always-on device's behaviour is byte-identical to M4's.
- [ ] `missed_wake_count` increments per missed window and resets on a wake.
- [ ] `devices_sleeping` reflects reality; no metric is emitted per device id.
- [ ] `safety_021_*` tests are green.

## Verification

```bash
cargo test -p edge-controller liveness::
cargo test safety_021
cargo test --test integration sleeping_device_overdue_without_inbound_message
curl -s localhost:8080/api/v1/devices/plant-node-01 | jq '{connectivity, expected_wake_at, power_mode, missed_wake_count}'
```

## Tests required

- `safety_021_overdue_sleeper_becomes_isolated`.
- `safety_021_device_wake_time_is_advisory`.
- `safety_021_unannounced_absence_is_never_sleeping`.
- `safety_021_sleep_window_detected_by_timer`.
- Always-on regression: M4's liveness tests unchanged and green.
- SCEN-110, SCEN-111, SCEN-112.

## Documentation impact

- [connectivity-modes.md](../../architecture/connectivity-modes.md) §1b.
- [PRD 040](../../prd/040-device-registry-and-health.md) §Battery amendment.
- [http-api-boundaries.md](../../protocol/http-api-boundaries.md) §2.3.
- [safety-invariants.md](../../architecture/safety-invariants.md) SAFETY-021.

## Files likely affected

```text
crates/edge-controller/src/device/liveness.rs
crates/edge-controller/src/device/connectivity.rs
crates/edge-controller/src/device/health.rs
crates/edge-controller/src/api/devices.rs
migrations/
```

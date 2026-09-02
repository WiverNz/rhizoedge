# Issue M8-017 — Implement battery and sleep end-to-end scenarios

**Milestone:** M8 · **PRD:** [PRD 080](../../prd/080-end-to-end-test-environment.md) · **Depends on:** M8-005, M8-006, M8-013, M6-023, M5-021

## Context

Battery and deep-sleep mode
([ADR-018](../../adr/018-battery-and-deep-sleep-device-mode.md)) spans five
milestones: the contract in M5, the liveness model in M5, the durable command
intent in M6, the firmware in M9, and the presentation in M12. Each of those has
its own unit and integration tests, and none of them exercises the seam that
actually matters — an operator asking for water, a device that is not listening,
and the several minutes in between.

SCEN-113…SCEN-117 are that seam. SCEN-110…SCEN-112 already run at the M5
integration level and are not repeated here.

## Goal

The five end-to-end battery scenarios run in the existing harness, against the
simulator, with no hardware.

## Scope

- SCEN-113 — a manual dose for a sleeping device is held, survives an edge
  restart, and delivers exactly once at the wake
- SCEN-114 — a leak raised while the device slept refuses the pending intent at
  delivery, with nothing published; likewise tank and rolling-cap exhaustion
- SCEN-115 — budget and cooldown across ~190 sleep/wake cycles, a cold reset
  mid-cooldown, and a corrupted RTC checksum
- SCEN-116 — an undelivered intent expires; a delivered one carries a TTL minted
  at the wake
- SCEN-117 — the device stays awake for a whole watering cycle, publishes its
  result before announcing sleep, and reports an interrupted dose after a power
  cut
- A compose overlay profile running the simulator in battery mode at an
  accelerated scale, with the wake interval scaled consistently
- Extension of M8-013's mutation set: **an implementation that publishes
  immediately to a sleeping device must turn the suite red**

## Non-goals

- Firmware. M9 runs its own conformance against these behaviours; M8 is
  simulator-only by design.
- Any energy or power measurement. That needs hardware and a meter (M10-012).
- Re-testing SCEN-110…SCEN-112, which are integration-level and green from M5.

## Dependencies

- M8-005
- M8-006
- M8-013
- M6-023
- M5-021

## Implementation notes

Assert on **observable state**, as every other scenario does: API responses,
database rows, and MQTT traffic captured by a spy subscriber. The most important
assertion in the whole set is a negative one — that **nothing** appears on any
`commands/*` topic while the device sleeps — and a spy subscriber is the only
honest way to check it. A test that merely asserts the intent row exists would
pass against an implementation that also published.

Scale the wake interval with the virtual clock, and assert that it was scaled:
M8-004 already checks time-scale agreement between components, and a wake
interval that did not get the memo would produce a suite that either takes hours
or races. This is the most likely way these scenarios become flaky.

SCEN-115 is the long one and the one most worth getting right, because it is the
only place sleep and offline autonomy are exercised together. Drive the reset
reason and the RTC checksum explicitly rather than hoping to observe both
branches naturally; the failure branch is the one that matters and it will not
occur by chance.

The added mutation is the point of M8-013's whole approach. If reverting the
intent routing — publishing immediately to a sleeping device — leaves the suite
green, then these scenarios are decorative.

## Acceptance criteria

- [x] All five scenarios run in the existing runner and exit 0.
- [x] A spy subscriber confirms nothing is published on `commands/*` while the
      device sleeps.
- [x] The delivered command's `issued_at` is the wake instant, asserted against
      captured MQTT rather than inferred.
- [x] SCEN-115 exercises both the credited and the zero-credit branches
      explicitly.
- [x] The battery profile scales its wake interval with the virtual clock, and
      M8-004's agreement check covers it.
- [x] The suite still completes under 10 minutes.
- [x] The added mutation turns the suite red, and is reverted.

## Verification

```bash
docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.test.yml \
  --profile battery up --build --abort-on-container-exit
cargo test -p rhizo-scenarios battery::
cargo test -p rhizo-scenarios scenario_11
```

## Tests required

- SCEN-113, SCEN-114, SCEN-115, SCEN-116, SCEN-117.
- The immediate-publish mutation.

## Documentation impact

- [PRD 080](../../prd/080-end-to-end-test-environment.md) — scenario and mutation
  counts.
- [failure-scenarios.md](../../testing/failure-scenarios.md) §K.

## Files likely affected

```text
crates/testkit/scenarios/battery.rs
deploy/docker-compose.test.yml
```

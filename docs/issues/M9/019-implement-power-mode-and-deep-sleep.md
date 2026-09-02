# Issue M9-019 — Implement power mode and the deep-sleep wake cycle

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-012, M9-018, M5-021

## Context

[ADR-018](../../adr/018-battery-and-deep-sleep-device-mode.md) makes a
battery-powered Wi-Fi node a supported deployment. M5-021 defined its observable
behaviour against the simulator; this issue makes real silicon behave the same
way, which is the claim M9-014's conformance test exists to check.

The hard part is not `esp_deep_sleep`. It is that deep sleep resets the world:
RAM is gone, peripherals are reinitialised, and the monotonic accounting that
SAFETY-015 depends on has to survive without becoming a way to earn budget.

## Goal

One firmware image that runs always-on or battery, with state that survives
sleep honestly.

## Scope

- `PowerMode` read from the retained `device.config`, persisted to NVS, defaulting
  to `AlwaysOn` when absent or unrecognised
- The wake cycle of [ADR-018](../../adr/018-battery-and-deep-sleep-device-mode.md)
  §5, as an explicit state machine in `src/app/`
- `esp_deep_sleep` with a timer wake source at `wake_interval_seconds`
- The sleep announcement — retained status, `reason: "sleeping"`, the `power`
  block — published and its QoS 1 PUBACK observed **before** sleep is entered
- RTC-retained state with a checksum: budget accumulator, cooldown deadline,
  `boot_generation`, and the last announced wake instant
- Wake-reason handling per §6: a timer wake with a valid checksum credits the RTC
  counter's elapsed time; every other reset reason, and any checksum failure,
  falls back to SAFETY-015's "assume no time has passed"
- `wake_reason` reported in `device.status`
- `awake_budget_seconds` bounding an idle wake, extended by an active watering
  cycle (M9-021)
- Fast reconnect: persist BSSID and channel in RTC memory and use them to skip a
  full scan, falling back to a scan on failure

## Non-goals

- Peripheral power gating and sensor warm-up (M9-020).
- Staying awake for a watering cycle and battery telemetry (M9-021).
- Light sleep, dynamic frequency scaling, or any other power technique. One
  mechanism, measured, before a second is added.
- Measuring what any of this draws. That is M10-012, on a board, with a meter.

## Dependencies

- M9-012
- M9-018
- M5-021

## Implementation notes

**Sleep is entered from one place, and only from the top of the loop.** A
`deep_sleep()` call reachable from a command handler or an error path is how a
device sleeps with the pump energised. The state machine must make that
unrepresentable rather than merely unlikely.

The RTC-retained struct is the delicate part. `#[link_section = ".rtc.data"]`
survives deep sleep but not a power cut, brownout, or most other resets — so the
checksum is not paranoia, it is the discriminator between "we know how long we
slept" and "we do not". Get this backwards and a corrupted RTC word becomes free
watering budget.

The elapsed-time rule is narrow enough to write as a single function, and should
be:

```text
credit_elapsed(wake_reason, rtc_state) -> Duration
    Timer  if checksum_valid(rtc_state)  → rtc_counter_now - rtc_state.slept_at
    _                                    → Duration::ZERO
```

`Duration::ZERO` is SAFETY-015's existing behaviour, unchanged. There is no third
branch and no `_ =>` arm that returns anything else.

Keep NVS and RTC memory in their proper roles. NVS holds what must survive power
loss — the command dedup ring, the policy, the persisted budget floor. RTC memory
holds the sleep-cycle accounting, and is treated as a cache that may vanish.
Writing the budget to NVS on every wake would exhaust the flash: at 96 wakes a
day the NVS write budget is the limiting component, so write NVS on change and
on watering, and keep the per-wake accounting in RTC memory.

`src/app/` stays free of `esp_idf_*` imports (F-090-42), so the wake state
machine and `credit_elapsed` are host-testable with a fake RTC and fake reset
reasons — which is the only practical way to test the checksum-failure branch.

## Acceptance criteria

- [x] One image serves both power modes, selected by configuration.
- [x] An absent or unrecognised `power.mode` yields `AlwaysOn`.
- [ ] Always-on behaviour is unchanged; M9-001…M9-018 tests stay green.
- [ ] The sleep announcement's PUBACK is observed before `esp_deep_sleep`.
- [x] A timer wake with a valid checksum credits elapsed time; a cold boot and a
      corrupted checksum each credit zero.
- [x] A power cycle mid-cooldown neither shortens the cooldown nor replenishes
      the budget.
- [ ] `wake_reason` is reported truthfully in status.
- [x] `deep_sleep` has exactly one call site, checked structurally.
- [x] `src/app/` still contains no `esp_idf_*` imports.
- [ ] **With a board:** 20 consecutive wake cycles complete with no missed wake
      and no watchdog reset.

## Verification

```bash
cd firmware/esp32-node
cargo test --target x86_64-unknown-linux-gnu -p app power::
cargo test --target x86_64-unknown-linux-gnu -p app credit_elapsed
cargo build --release
grep -rn 'deep_sleep' src/ | grep -v '^src/hal/sleep.rs'
```

## Tests required

- `credit_elapsed` across every wake reason and both checksum outcomes.
- Wake state machine: no path from a watering state into sleep.
- Cooldown and budget across simulated sleep cycles and a simulated cold boot.
- Conformance against the simulator's battery mode (extends M9-014).
- SCEN-115.

## Documentation impact

- [PRD 090](../../prd/090-esp32-rust-firmware.md) — non-goal removed, F-090-5x
  added.
- [offline-autonomy.md](../../architecture/offline-autonomy.md) §5b.
- [ADR-007](../../adr/007-esp32-rust-framework-and-toolchain.md) — power scope.

## Files likely affected

```text
firmware/esp32-node/src/app/power.rs
firmware/esp32-node/src/app/wake_cycle.rs
firmware/esp32-node/src/hal/sleep.rs
firmware/esp32-node/src/hal/rtc_store.rs
```

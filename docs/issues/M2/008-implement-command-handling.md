# Issue M2-008 — Implement command handling through the shared validator

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-006, M2-007

## Context

**The requirement the simulator's usefulness depends on.** PRD 020 F-020-20:
the only actuation path calls `rhizo_mqtt_contract::validate_water_command`.
A simulator more permissive than firmware makes every M6 safety test meaningless.

## Goal

Handle water, tare, and calibrate commands with hardware-identical refusal behaviour.

## Scope

- Subscribe and dispatch on `commands/+`
- **Every** water command goes through `validate_water_command` — no other path exists
- `Accept` runs the pump model; `Reject` publishes the reason; `AlreadyExecuted` republishes the stored result
- NVS-equivalent persistence written **before** actuation
- A `command.result` published for every command including rejections
- Result retried up to 60 s; persisted and republished after restart
- `delivered_today_ml` tracked and reset daily

## Non-goals

- A bypass flag of any kind — explicitly forbidden.

## Dependencies

- M2-006
- M2-007

## Implementation notes

There must be no `--allow-any-dose`, no debug bypass, no test-only
relaxation. Removing the validator call would be a visible change to a
safety-critical file, and `safety_007_simulator_refuses_like_hardware` asserts
the refusal directly.

Persist `(command_id, started_at, requested_ml)` **before** the pump model
starts, so `--fault restart-mid-dose` reproduces SAFETY-011 faithfully.

`command.calibrate` runs a fixed duration and its volume counts toward the
daily total.

## Acceptance criteria

- [ ] A valid command runs the pump model and reports `completed`.
- [ ] The same `command_id` twice actuates **once** and republishes the stored result.
- [ ] An expired command is rejected with `expired`.
- [ ] `requested_ml: 10000` is clamped or rejected, never delivered.
- [ ] A leak or low tank rejects with the correct reason.
- [ ] `clock_synced: false` rejects everything.
- [ ] A result is published for every command, rejections included.
- [ ] `grep -c validate_water_command` shows exactly one call site.

## Verification

```bash
cargo test -p device-simulator command::
cargo test safety_007_simulator_refuses_like_hardware
grep -rn 'validate_water_command' crates/device-simulator/src | wc -l   # expect 1
```

## Tests required

- `safety_001` duplicate command single actuation.
- `safety_002` expired rejected.
- `safety_007_simulator_refuses_like_hardware` — publish 10000 ml directly to the broker.
- Result published for every outcome.
- Daily total accumulation and reset.

## Documentation impact

- None.

## Files likely affected

```text
crates/device-simulator/src/command.rs
crates/device-simulator/src/pump.rs
```

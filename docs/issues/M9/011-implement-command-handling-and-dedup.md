# Issue M9-011 — Implement command handling with the shared validator and dedup ring

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-010, M9-004

## Context

SAFETY-001, -002, -007 on real firmware. Like the simulator, **the only
actuation path calls `validate_water_command`** — there is no second
implementation of the rules.

## Goal

Handle commands with hardware-grade safety.

## Scope

- Dispatch on `commands/+`
- **Every water command through `validate_water_command`**
- 16-entry NVS dedup ring; a repeat republishes the stored result and does **not** actuate
- `(command_id, started_at, requested_ml)` written to NVS **before** actuation
- **If the NVS write fails, abort the dose** and report `failed`
- `delivered_today_ml` in NVS enforcing the device daily cap
- A result published for every command; retried up to 60 s, then persisted for next boot

## Non-goals

- The real pump (M11-001).

## Dependencies

- M9-010
- M9-004

## Implementation notes

The NVS-write-failure rule is worth stating plainly: **if the device cannot
record that it is about to pump, it must not pump.** Otherwise an interrupted
dose becomes undetectable.

Results are ledger data and are retried until acknowledged, unlike telemetry.
An unpublishable result is persisted and republished after the next boot.

The device daily cap counts everything — manual, automatic, calibration — unlike
the edge's cap which excludes manual.

## Acceptance criteria

- [ ] A valid command actuates and reports `completed`.
- [ ] A duplicate `command_id` republishes the stored result and does **not** actuate.
- [ ] The ring survives a power cycle.
- [ ] NVS is written before actuation.
- [ ] **A failed NVS write aborts the dose.**
- [ ] The device daily cap is enforced independently of the edge.
- [ ] A result is published for every command including rejections.
- [ ] `grep -c validate_water_command` shows exactly one call site.

## Verification

```bash
cd firmware/esp32-node && cargo test command::
cargo test safety_001 safety_002 safety_007
grep -rn 'validate_water_command' firmware/esp32-node/src | wc -l
```

## Tests required

- Each verdict path.
- Dedup across a simulated power cycle.
- **NVS failure aborts the dose.**
- Daily cap enforcement.
- Result retry and persistence.

## Documentation impact

- None.

## Files likely affected

```text
firmware/esp32-node/src/app/command.rs
firmware/esp32-node/src/safety/mod.rs
```

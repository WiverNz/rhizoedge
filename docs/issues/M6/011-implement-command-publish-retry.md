# Issue M6-011 — Implement command publish retry with a fixed command_id

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-009, M0-007

## Context

**ADR-014 calls this the single most important paragraph in that document.**
The edge cannot distinguish 'the publish failed' from 'it succeeded and the ack
was lost'. Issuing a *fresh* command after a failure would make the device see
two distinct ids and water twice.

## Goal

Retry publication safely.

## Scope

- Retry at most 3 times with 200 ms base, 2 s cap
- **Always the same `command_id`** — never generate a new one
- After 3 failures: mark `failed`, transition to `Recheck`, **create no watering event**
- Count outcomes in `watering_commands_total{outcome}`

## Non-goals

- Retrying the pump itself — the device never does that.

## Dependencies

- M6-009
- M0-007

## Implementation notes

MQTT QoS 1 redelivery of the identical payload is safe because the device
deduplicates on `command_id` (SAFETY-001). A new id defeats that entirely.

Write the test that proves it: fail the publish twice, succeed on the third, and
assert the device saw one `command_id` and actuated once.

A failed publish is a missed dose, which is recoverable. A double dose is not.

## Acceptance criteria

- [ ] A transient publish failure is retried with the **same** `command_id`.
- [ ] **No code path generates a new `command_id` on retry.**
- [ ] After 3 failures the command is `failed` and the state returns to `Recheck`.
- [ ] A failed publish creates no watering event.
- [ ] Retry delays follow the backoff bounds.
- [ ] Outcomes are counted.

## Verification

```bash
cargo test -p edge-controller command::retry
cargo test safety_001
```

## Tests required

- Retry with identical command_id (the key assertion).
- Exhaustion path.
- No event on failure.
- Backoff bounds.

## Documentation impact

- ADR-014 verified accurate.

## Files likely affected

```text
crates/edge-controller/src/control/publish.rs
```

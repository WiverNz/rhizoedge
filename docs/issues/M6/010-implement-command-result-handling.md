# Issue M6-010 — Implement command result handling

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-009

## Context

Protocol section 5.10. The result settles the command and creates the
watering event — or explicitly does not, for outcomes that delivered nothing.

### Mandatory prerequisite carried forward from M3

**A broker PUBACK is not proof that the edge durably committed a
`command.result`.** The M3 audit established this as fact, not as a
possibility: the edge sets no `manual_acks`, so `rumqttc` sends the PUBACK while
the message is still in the ingress channel, ahead of the transaction that
persists it. Protocol section 5.10 has the device stop retrying once the broker
acknowledges. Between those two facts, a result can be acknowledged to the
device and then lost — on a crash, or on a shutdown that drops the channel — and
never republished, because as far as the device is concerned it was delivered.

Telemetry survives this because a lost sample is fail-safe, and offline events
survive it because `event.ack` is an application-level acknowledgement published
only after commit. `command.result` has neither protection, and it is ledger
data: it is what tells the edge how much water actually reached the plant.

**This must be closed before M6 enables real watering.** An end-to-end durable
acknowledgement or retry path is required — manual acks on the ingress so the
PUBACK follows the commit, or a result-level acknowledgement of the same shape
as `event.ack`, or an equivalent that makes the device's stop-retrying condition
depend on the edge's commit rather than on the broker's receipt. A budget that
silently misses a delivered dose reopens exactly the over-watering that
SAFETY-006 exists to prevent.

## Goal

Process results and update state and history correctly.

## Scope

- Handle `command.result` through the M3 dedup path
- `completed`: create a `watering_event` with `delivered_ml`, transition to `WaitForAbsorption`
- `rejected`: record the reason, **no watering event**, transition to `Recheck`
- `interrupted`/`failed`: credit `requested_ml`, **no watering event**, transition to `Recheck`
- A result for an unknown `command_id` is logged and ignored
- All updates in one transaction

## Non-goals

- No-delivery detection (M6-017).

## Dependencies

- M6-009

## Implementation notes

A rejected or failed command must **never** create a watering event. A
watering event asserts that water reached the plant; recording one for a refused
command would corrupt the daily total and the cooldown in the permissive
direction.

An unknown `command_id` is logged, not invented into existence — the edge does
not create a command row to match a result it did not issue.

## Acceptance criteria

- [x] `completed` creates exactly one `watering_event`.
- [x] `rejected` creates **none** and records the reason.
- [x] `interrupted` creates none but credits `requested_ml` to the budget.
- [x] An unknown `command_id` is ignored with a log.
- [x] A duplicate result creates no second event.
- [x] All updates are atomic.
- [x] A `command.result` is not treated as delivered until the edge has committed it, and the device's retry stops on that fact rather than on the broker PUBACK.

## Verification

```bash
cargo test -p edge-controller command::result
cargo test safety_001
```

## Tests required

- Each result status.
- **No event on rejection or failure.**
- Duplicate result idempotency.
- Unknown command_id handling.
- **A result acknowledged to the device is still present after an edge crash between receipt and commit.**

## Documentation impact

- Protocol section 5.10 if the acknowledgement shape changes.
- [docs/reports/M3.md](../../reports/M3.md) §Known gaps carried forward, which records this requirement.

## Files likely affected

```text
crates/edge-controller/src/control/result.rs
```

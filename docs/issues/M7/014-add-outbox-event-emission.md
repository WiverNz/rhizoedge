# Issue M7-014 — Emit outbox events from every state change

**Milestone:** M7 · **PRD:** [PRD 070](../../prd/070-cloud-sync-and-storage.md) · **Depends on:** M7-008, M6-024

## Context

ADR-005's outbox pattern: the event row is written in the **same transaction**
as the change it describes, with an `event_id` generated once and stable across
every retry.

## Goal

Emit the full event catalogue from the edge.

## Scope

- Emit all V1 event kinds from ADR-005
- Written in the same transaction as the change
- **`event_id` generated at write time and never regenerated**
- `value_tier` assigned per kind
- Emission skipped entirely when `cloud.enabled = false`

## Non-goals

- Draining (M7-006).

## Dependencies

- M7-008
- M6-024

## Implementation notes

A regenerated `event_id` on retry would defeat the unique index and the whole
idempotency scheme. Generate once, at the outbox write, inside the transaction.

With `cloud.enabled = false` (the default), write no outbox rows at all rather
than writing and never draining — otherwise a cloud-less deployment accumulates
rows forever.

## Acceptance criteria

- [x] Every documented event kind is emitted.
- [x] Emission shares the transaction with its change.
- [x] **`event_id` is stable across retries**, asserted by a test.
- [x] `value_tier` is correct per kind.
- [x] With `cloud.enabled = false`, no outbox rows are written.
- [x] A rolled-back change emits no event.

## Post-M7 correction — 2026-08-31

These criteria were ticked before they were all true. The audit recorded in
[docs/reports/M7.md](../../reports/M7.md) §Post-M7 correction found three gaps
against "every documented event kind is emitted":

- the edge emitted `device.capabilities_changed` for ADR-005's
  `device.capabilities`, which the cloud does not recognise;
- `delete_sensor_binding`, `delete_actuator_binding`,
  `delete_measurement_policy`, `delete_offline_policy`, `plant::delete`, and
  `materialize_preset` changed durable state and emitted nothing;
- `history.gap`, `watering.offline_autonomous`, and `device.policy_applied` had
  no reachable emitter at all, because `persist_replay` never touched the outbox.

All are fixed. `emit` now takes a typed `EventKind` whose only constructors are
the 25 catalogue constants, and both the edge's and the cloud's catalogues are
checked against ADR-005's own fenced block. The criteria below are true as
written; what changed is the evidence behind them.

## Verification

```bash
cargo test -p edge-controller outbox::emit
cargo test -p rhizo-storage --lib outbox
cargo test -p cloud-api --lib known_kinds_match_adr_005
```

## Tests required

- Per-kind emission.
- Transactional atomicity with rollback.
- **event_id stability.**
- Disabled-cloud no-op.

## Documentation impact

- ADR-005's event kind list verified.

## Files likely affected

```text
crates/edge-controller/src/cloud/emit.rs
```

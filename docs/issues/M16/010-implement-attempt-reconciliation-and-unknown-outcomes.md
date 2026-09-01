# Issue M16-010 — Implement attempt reconciliation and unknown-outcome semantics

**Milestone:** M16 · **PRD:** [PRD 160](../../prd/160-verified-watering.md) · **Depends on:** M16-009

## Context

This issue owns **SAFETY-023**, and it exists because the tidiest possible state
machine is also the most dangerous one. A dose with no result looks, from the
edge, exactly like a dose that never happened — and resolving it to "nothing was
delivered" makes every diagram simpler and frees the budget of a plant that may
have just received 40 ml.

## Goal

One durable `DeliveryRecord` per `command_id`, a reconciliation that resolves
what it can, and an `OutcomeUnknown` that nothing can convert to zero.

## Scope

- Writing `watering_deliveries` from every settled command, in the **same
  transaction** as the existing result processing.
- The six-dose ladder assembled from the edge's own decision, the wire, and the
  result.
- Reconciliation states: `pending` → `complete` | `unresolvable`.
- `unresolvable` on TTL expiry with no result, or on a device replay whose
  buffer contains no result for the `command_id`.
- Every `unresolvable` and every `interrupted` recorded as `OutcomeUnknown` with
  a typed reason.
- Idempotent update on a replayed or duplicate result.
- The existing reconciliation hold extended to cover an attempt still `pending`.

## Non-goals

- Budget arithmetic. M16-011, so the accounting change can be reviewed alone.
- Actuator health. M16-012.
- Retrying anything.

## Dependencies

- M16-009

## Implementation notes

**Never resolve unknown to zero, and never to `NoFlow`.** `NoFlow` is a
*measured* condition requiring a working witness that saw nothing move; a silent
device is not evidence that its pump did nothing. The typed `UnknownReason` is
what keeps the two apart in the record, and `safety_023_*` is what keeps them
apart in the build.

Same transaction, always. The dedup marker and the message's effects already
share one SQLite transaction (SAFETY-001/-010), and the attempt row is one of
those effects. Splitting it reintroduces the crash window the whole persistence
model exists to close.

`INSERT … ON CONFLICT(command_id) DO UPDATE`, and the update must be
monotonic in evidence: a replayed result may raise the evidence level and fill
in a measured volume, and may never lower a recorded outcome from a fault to a
success. A device retrying a result it already sent must be idempotent, and
`command.result.ack` is published after the commit and also for a duplicate —
otherwise the device retries for ever.

The reconciliation hold is **derived, not stored**, exactly as the existing one
is: `persist_status` rewrites `connectivity_mode` on every heartbeat, and a
device replays while it is heartbeating. A `pending` attempt older than its TTL
holds its plant; an attempt that resolves releases it.

An offline autonomous dose arrives through the buffered event path with no
`command_id` from the edge's perspective. It still produces an attempt row, keyed
by the event's own identity, and the same `OutcomeUnknown` rules apply to a dose
whose replay is incomplete.

## Acceptance criteria

- [ ] Every settled command produces exactly one attempt row.
- [ ] The row is written in the same transaction as the result's other effects.
- [ ] A replayed or duplicated result updates one row and never creates a second.
- [ ] An update may raise evidence and may never downgrade a fault to a success.
- [ ] TTL expiry with no result records `OutcomeUnknown`, never `NoFlow` and
      never zero.
- [ ] A device replay with no matching result records `OutcomeUnknown`.
- [ ] A `pending` attempt past its TTL holds the plant; resolution releases it.
- [ ] An offline autonomous dose produces an attempt row with the same rules.
- [ ] `command.result.ack` is still published after commit and for duplicates.

## Verification

```bash
cargo test -p edge-controller delivery::reconcile
cargo test safety_023
cargo test safety_016
RHIZO_REQUIRE_BROKER=1 cargo test -p edge-controller --all-features
```

## Tests required

- `safety_023_unknown_outcome_is_never_credited_as_zero`.
- `safety_023_a_missing_result_never_becomes_a_zero_delivery`.
- `safety_023_reconciliation_failure_keeps_the_conservative_charge`.
- Property: any ordering and duplication of results for one `command_id` yields
  one row and one charge.
- Crash between actuation and result, exercised with the simulator.
- Offline autonomous replay, complete and incomplete.

## Documentation impact

- `docs/architecture/safety-invariants.md`: SAFETY-023's edge half.
- `docs/architecture/data-flow.md`: the attempt row in the result pipeline.

## Files likely affected

```text
crates/edge-controller/src/control/reconcile.rs
crates/edge-controller/src/control/command.rs
crates/edge-controller/src/delivery/mod.rs
crates/storage/src/repo/delivery.rs
docs/architecture/safety-invariants.md
```

# Issue M7-008 — Implement the outbox cap with value-tiered pruning

**Milestone:** M7 · **PRD:** [PRD 070](../../prd/070-cloud-sync-and-storage.md) · **Depends on:** M7-006

## Context

Failure-model 4.5: during a prolonged outage the outbox must not grow without
bound — but **history is nice, and the ledger of what the machine did to a
living plant is not optional**. The tier column encodes that distinction.

## Goal

Bound outbox growth while preserving high-value events.

## Scope

- `outbox_max_rows` (default 500 000)
- At the cap, prune `value_tier = 'low'` oldest first
- **Never prune `value_tier = 'high'`**
- `value_tier` assigned at the single outbox write site, defaulting to `high`
- Pruning emits an alert-level log and increments `cloud_events_dropped_total`

## Non-goals

- Compressing or aggregating pruned data.

## Dependencies

- M7-006

## Implementation notes

Tiering: measurements are `low`; watering events, commands, lockouts, and
device faults are `high`. Defaulting to `high` means a new event kind is
preserved unless someone deliberately marks it disposable — the safe default is
to keep.

Assert the never-prune property with a test that fills the outbox well past the
cap and confirms every high-tier row survives.

## Acceptance criteria

- [ ] Growth stops at the cap.
- [ ] Low-tier events are pruned oldest first.
- [ ] **Every high-tier event survives** filling the outbox to twice the cap.
- [ ] `value_tier` defaults to `high`.
- [ ] Pruning logs at alert level and increments the counter.
- [ ] It is assigned at exactly one write site.

## Verification

```bash
cargo test -p edge-controller outbox::prune
cargo test --test integration outbox_cap
```

## Tests required

- **High-tier preservation under extreme growth (SCEN-065).**
- Low-tier oldest-first pruning.
- Default tier.
- Counter and log.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/cloud/prune.rs
crates/storage/src/repo/outbox.rs
```

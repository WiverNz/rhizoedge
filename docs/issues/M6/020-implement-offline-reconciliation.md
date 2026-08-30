# Issue M6-020 — Implement offline event reconciliation

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-010, M6-012, M3-016

## Context

The reconnection seam is where duplicate watering would be created
([offline-autonomy.md](../../architecture/offline-autonomy.md) §8). Two rules
matter: replay applies exactly once, and the edge issues no dose until it has read
what the device already did.

## Goal

Reconcile a reconnecting device's history safely and hold the plant until it completes.

## Scope

- On reconnection, move affected plants to `Uncertain` and record `reconciling`
- Consume replayed events (M3-016); fold autonomous doses into the rolling budget
- Release the plant only after `complete: true` is received **and committed**
- **Issue no command to a plant that is reconciling**
- Push the edge's row-derived budget back as the device's baseline after reconciliation
- Raise `device.reconciled` with a summary of what happened while isolated

## Non-goals

- Device-side buffering (M2-018, M9-017).

## Dependencies

- M6-010
- M6-012
- M3-016

## Implementation notes

The hold is the safety-critical half. A device that autonomously watered ninety
seconds before reconnecting has that dose in its buffer, not yet in the edge's
budget. Issuing on top of it is exactly SAFETY-016's failure, and the only
defence is refusing to act until the buffer has been read.

Reuse the existing `Uncertain` lockout rather than inventing a state: it already
means "inputs are not trustworthy enough to act", which is precisely the
situation.

After reconciliation the edge's row-derived budget is authoritative and is pushed
back to the device, so the two stop diverging.

## Acceptance criteria

- [x] A reconnecting device's plants enter `reconciling` and are held in `Uncertain`.
- [x] **No command is published while a plant is reconciling**, asserted with an MQTT spy.
- [x] The plant is released only after `complete` is committed.
- [x] Autonomous doses appear in the rolling budget after reconciliation.
- [x] An edge restart mid-reconciliation replays safely with no duplicates.
- [x] A device reconnecting twice mid-replay creates no duplicate events.
- [x] `device.reconciled` summarises the isolation period.

## Verification

```bash
cargo test -p edge-controller reconcile::
cargo test safety_016
cargo test --test integration reconciliation
```

## Tests required

- SCEN-100, SCEN-101, SCEN-102.
- An explicit spy-based test that no command is published during reconciliation.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/control/reconcile.rs
```

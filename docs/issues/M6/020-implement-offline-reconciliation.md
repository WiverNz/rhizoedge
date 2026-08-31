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
- [x] **An autonomous dose is charged to the plant it was delivered to, not to whichever plant holds the actuator binding when the replay arrives.** *(Added by the 2026-08-31 correction; see below.)*

### Correction — 2026-08-31

**"Folds autonomous doses into the rolling budget" left open *whose* budget, and
the first implementation answered it wrongly.** The plant was resolved from
`SELECT plant_id FROM actuator_bindings WHERE device_id=?` — the bindings as
they stood at replay time — which asks a question about the present and applies
the answer to the past. Rebinding a pump while a device was isolated therefore
charged the dose to the new plant, leaving the plant that really was watered
with a clean budget and free to be watered again. Idempotence on `event_id`
guaranteed one row per dose; it never guaranteed it was the right plant's row.

Closed by `detail.plant_id` on the replayed `watering.offline_autonomous` event
(protocol §5.4), written onto the `watering_events` row inside the same
transaction as the event. Binding-based resolution is retained as the documented
fallback for a device that predates the field, or one naming a plant this edge
has never provisioned — the latter falls back rather than failing the replay,
because a rejected replay wedges reconciliation for ever.

Regression test:
`safety_016_a_replayed_dose_is_charged_to_the_plant_the_device_named`, with
`without_a_named_plant_the_dose_follows_the_binding` as the negative control.
See [docs/reports/M6.md](../../reports/M6.md) §Post-M6 corrections.

## Verification

```bash
cargo test -p edge-controller reconcile::
cargo test safety_016
cargo test --test integration reconciliation
```

## Tests required

- SCEN-100, SCEN-101, SCEN-102.
- **Attribution across a binding change:** plant A bound → isolate → A waters offline → binding moves to B → replay → A's budget charged exactly once, B unchanged.
- An explicit spy-based test that no command is published during reconciliation.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/control/reconcile.rs
```

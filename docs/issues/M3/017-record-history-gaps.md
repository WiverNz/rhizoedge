# Issue M3-017 — Record and expose reported history gaps

**Milestone:** M3 · **PRD:** [PRD 030](../../prd/030-edge-ingestion-and-storage.md) · **Depends on:** M3-016

## Context

A device's buffer is bounded, so history can genuinely be lost. SAFETY-020
requires the loss to be recorded and visible rather than silently absorbed.

## Goal

Store reported gaps as first-class data.

## Scope

- Handle `history.gap` events into the `history_gaps` table
- Raise a `history_gap` device event with severity `warning`
- Metric `history_gaps_total{tier}`

## Non-goals

- UI presentation (M12-016).

## Dependencies

- M3-016

## Implementation notes

A gap is data, not an error. Storing it means a chart can show a break rather
than interpolating across it, and an operator reviewing why a plant looks odd can
see that four hours of history are genuinely missing.

An audit-tier gap is more serious than a telemetry-tier gap — it means an
autonomous action may be unrecorded — so carry the tier and let the UI and the
severity reflect it.

A gap marker is immutable once the device has sent it, and takes its
`device_seq` at that moment (mqtt-v1.md §5.4). Two consequences for this issue:
a marker is deduplicated on `event_id` like any other event, so a re-replayed
marker must not create a second row; and a marker is acknowledged by sequence
like any other event, so M3-016's cumulative `event.ack` needs no special case
for gaps. A second, wider marker for the same device is a **new** loss, not a
correction of the first — store both.

## Acceptance criteria

- [x] A `history.gap` event creates a `history_gaps` row with range, count, and tier.
- [x] Gaps are available to the later M4 read API through durable repository rows.
- [x] An audit-tier gap is recorded at higher severity than a telemetry-tier gap.
- [x] The metric increments with the correct labels.
- [x] A duplicate gap event creates no second row.
- [x] Two distinct markers from the same device are stored as two gaps, not
      merged: they describe different losses.

The earlier API wording belonged to M4 and the `device_id` metric label
contradicted ADR-010's bounded-cardinality rule. M3 owns the durable row and a
tier-only counter; M4 owns HTTP exposure.

## Verification

```bash
cargo test -p edge-controller gaps::
cargo test safety_020
```

## Tests required

- SCEN-104.
- Duplicate-gap idempotency.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/pipeline/replay.rs
crates/storage/src/repo/gaps.rs
```

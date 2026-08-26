# Issue M3-017 — Record and expose reported history gaps

**Milestone:** M3 · **PRD:** [PRD 030](../../prd/030-edge-ingestion-and-storage.md) · **Depends on:** M3-016

## Context

A device's buffer is bounded, so history can genuinely be lost. SAFETY-020
requires the loss to be recorded and visible rather than silently absorbed.

## Goal

Store reported gaps as first-class data.

## Scope

- Handle `history.gap` events into the `history_gaps` table
- Expose gaps through the device and plant event APIs
- Raise a `history_gap` device event with severity `warning`
- Metric `history_gaps_total{device_id,tier}`

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

## Acceptance criteria

- [ ] A `history.gap` event creates a `history_gaps` row with range, count, and tier.
- [ ] Gaps appear in the device event API.
- [ ] An audit-tier gap is recorded at higher severity than a telemetry-tier gap.
- [ ] The metric increments with the correct labels.
- [ ] A duplicate gap event creates no second row.

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

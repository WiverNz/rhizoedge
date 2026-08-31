# Issue M7-011 — Add the time representation round-trip test

**Milestone:** M7 · **PRD:** [PRD 070](../../prd/070-cloud-sync-and-storage.md) · **Depends on:** M7-005, M7-004

## Context

ADR-013 flags the two time representations — integer milliseconds on MQTT,
RFC 3339 on HTTP, `TIMESTAMPTZ` in PostgreSQL — as a seam where bugs hide.

## Goal

Prove the instant survives every conversion.

## Scope

- Round trip: integer ms -> RFC 3339 -> `TIMESTAMPTZ` -> back
- Sub-second precision preserved
- Pre-1970 and far-future values handled
- DST-ambiguous local times are irrelevant because everything is UTC — assert that

## Non-goals

- Timezone display, which is a UI concern.

## Dependencies

- M7-005
- M7-004

## Implementation notes

Millisecond precision must survive: RFC 3339 permits fractional seconds, but
a formatter that drops them would silently round pump durations and message
ordering.

Property-test over random instants rather than a handful of examples — the
failure modes here are precision and boundary related.

## Acceptance criteria

- [x] A round trip preserves the instant exactly, to the millisecond.
- [x] Sub-second precision is preserved.
- [x] Pre-1970 values round-trip.
- [x] Far-future values round-trip.
- [x] All representations are UTC with no local-time path.
- [x] A property test over random instants passes.

## Verification

```bash
cargo test -p rhizo-cloud-client time::
PROPTEST_CASES=10000 cargo test time_roundtrip
```

## Tests required

- Property test over random instants.
- Boundary values.
- Precision preservation.

## Documentation impact

- None.

## Files likely affected

```text
crates/cloud-client/src/time.rs
```

# Issue M3-013 — Implement failure classification across error types

**Milestone:** M3 · **PRD:** [PRD 030](../../prd/030-edge-ingestion-and-storage.md) · **Depends on:** M3-007, M0-004

## Context

ADR-014 requires classification to be a function with exhaustive matches, so
a new error variant fails to compile until someone decides whether it is
retryable.

## Goal

Implement `Classify` for every error type the edge handles.

## Scope

- `Classify` impls for storage, MQTT, decode, and (from M7) cloud errors
- **Exhaustive matches, no catch-all arm**
- The classification table from ADR-014 implemented exactly
- A test per variant

## Non-goals

- Retry loops (per site).

## Dependencies

- M3-007
- M0-004

## Implementation notes

The no-catch-all rule is the entire value. A `_ => FailureKind::Transient`
arm would let a new Fatal error be silently retried forever, which is the
failure this design exists to prevent.

Notable classifications: `SQLITE_FULL` is Fatal for the write; malformed
payloads are Permanent; migration and config failures are Fatal; `SQLITE_BUSY`
is Transient.

## Acceptance criteria

- [x] Every error type implements `Classify`.
- [x] No impl contains a catch-all arm.
- [x] Each variant has a test asserting its kind.
- [x] Adding a variant without classifying it fails to compile.
- [x] The implementation matches ADR-014's table exactly.

## Verification

```bash
cargo test -p edge-controller classify::
cargo test -p rhizo-storage classify::
```

## Tests required

- One assertion per variant.
- Manual: add a variant, confirm the compile error, revert.

## Documentation impact

- ADR-014's table verified accurate.

## Files likely affected

```text
crates/edge-controller/src/error.rs
crates/storage/src/error.rs
```

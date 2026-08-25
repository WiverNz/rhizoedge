# Issue M1-003 — Implement UtcMillis and the UUID identifier types

**Milestone:** M1 · **PRD:** [PRD 010](../../prd/010-domain-and-mqtt-protocol.md) · **Depends on:** M1-001

## Context

ADR-013 fixes the wire representation as Unix epoch milliseconds UTC, carried
as a plain `i64` because the contract crate cannot depend on `chrono`.

## Goal

Provide the time and identifier primitives the envelope needs.

## Scope

- `UtcMillis(i64)` with serde as a bare integer
- `chrono` conversion helpers behind the `std` feature only
- UUID handling with `uuid` (`default-features = false`)
- A UUIDv7 generator gated so it is available where a clock exists
- `CommandId`, `MessageId`, `BootId`, `EventId` newtypes

## Non-goals

- The `Clock` trait (M1-012, in the domain crate).

## Dependencies

- M1-001

## Implementation notes

`UtcMillis` serialises as a bare JSON integer, not an object — the protocol
spec shows `"device_time_ms": 1756121400000`.

UUIDv7 needs a timestamp, which the contract crate cannot obtain. Take the
millis as a parameter: `MessageId::new_v7(now: UtcMillis, rng: &mut impl RngCore)`.
A device without a synced clock uses v4 and sets `clock_synced: false`.

Distinct newtypes for the four id kinds are worth the boilerplate: passing a
`command_id` where a `message_id` belongs is otherwise a silent bug in a
safety-relevant path.

## Acceptance criteria

- [ ] `UtcMillis` serialises to and from a bare integer.
- [ ] Negative values (pre-1970) round-trip without panicking.
- [ ] `chrono` conversions exist only under the `std` feature.
- [ ] UUIDv7 values from increasing timestamps sort in that order.
- [ ] The four id newtypes are not interchangeable (a type mismatch is a compile error).

## Verification

```bash
cargo test -p rhizo-mqtt-contract time:: ids::
cargo build -p rhizo-mqtt-contract --no-default-features
```

## Tests required

- Serde round trip for `UtcMillis` including negatives.
- UUIDv7 ordering.
- A compile-fail test for id-type confusion (trybuild or documented).

## Documentation impact

- Doc comments citing ADR-013 for the representation choice.

## Files likely affected

```text
crates/mqtt-contract/src/time.rs
crates/mqtt-contract/src/ids.rs
```

# Issue M1-002 — Implement DeviceId with its validated grammar

**Milestone:** M1 · **PRD:** [PRD 010](../../prd/010-domain-and-mqtt-protocol.md) · **Depends on:** M1-001

## Context

ADR-012 defines the device id grammar. The exclusions matter more than the
inclusions: barring `+`, `#`, `/`, and whitespace is what prevents a device from
breaking out of its topic subtree. This is a security boundary, not a style
rule.

## Goal

Implement `DeviceId` such that an invalid id cannot exist in a running system.

## Scope

- `DeviceId` newtype, 3-32 chars, `^[a-z0-9]([a-z0-9-]{1,30})[a-z0-9]$`
- `parse` as the **only** constructor
- `serde` impls that validate on deserialisation
- `Display`, `AsRef<str>`, `PartialEq`, `Hash`
- A typed `DeviceIdError`

## Non-goals

- Topic construction (M1-005).

## Dependencies

- M1-001

## Implementation notes

No `from_str_unchecked`, no `pub` field, no `From<String>`. The type's whole
value is that holding one proves validity.

Lowercase is not normalised — an uppercase id is *invalid*, not folded. Two
systems disagreeing about whether `Plant-01` and `plant-01` are the same device
is a bug waiting to happen.

Deserialisation must validate; otherwise a malicious payload constructs an
invalid id and bypasses the grammar entirely.

## Acceptance criteria

- [x] `plant-node-01`, `abc`, and a 32-char id parse successfully.
- [x] `x/#`, `+`, `#`, `Plant-01`, `ab`, a 33-char id, `-abc`, `abc-`, and `plant node` all fail.
- [x] There is no public constructor that skips validation.
- [x] `serde_json::from_str::<DeviceId>("\"x/#\"")` fails.
- [x] Round-trip through serde preserves the value.

## Verification

```bash
cargo test -p rhizo-mqtt-contract ids::
```

## Tests required

- Every valid and invalid case above, one assertion each.
- Property: any string matching the regex parses; any containing `/`, `+`, `#`, or uppercase fails.
- Serde round trip and serde rejection.

## Documentation impact

- Doc comment citing ADR-012 and stating the security rationale.

## Files likely affected

```text
crates/mqtt-contract/src/ids.rs
```

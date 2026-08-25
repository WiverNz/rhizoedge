# Issue M1-005 — Implement topic construction and parsing

**Milestone:** M1 · **PRD:** [PRD 010](../../prd/010-domain-and-mqtt-protocol.md) · **Depends on:** M1-002

## Context

Protocol section 3 defines ten topics. Both the edge and the firmware build
and parse them, so a single implementation prevents the classic bug where one
side publishes to a topic the other never subscribes to.

## Goal

Implement the `Topic` enum with lossless round-tripping.

## Scope

- `Topic` enum with all ten variants
- `to_string()` building `rhizo/v1/devices/{id}/...`
- `parse()` rejecting unknown, malformed, and wrong-version topics
- `device_id()` accessor
- `TopicError` distinguishing the failure kinds
- Constants for the edge subscription patterns

## Non-goals

- Subscription management (M3-005).

## Dependencies

- M1-002

## Implementation notes

`parse` must reject `rhizo/v2/...` explicitly rather than silently ignoring
it — a v2 message reaching a v1 parser is a real condition once
versioning-policy's migration path is used.

The device id inside a topic must go through `DeviceId::parse`, so a topic
containing an injected wildcard fails at parse time rather than propagating.

Round-trip is the core property: `Topic::parse(t.to_string()) == t` for every
variant.

## Acceptance criteria

- [ ] All ten variants build the exact strings in protocol section 3.
- [ ] Round-trip holds for every variant.
- [ ] `rhizo/v2/devices/x/status` is rejected.
- [ ] `rhizo/v1/devices/x%23/status` and other invalid ids are rejected.
- [ ] An unknown suffix is rejected.
- [ ] A truncated topic is rejected rather than panicking.

## Verification

```bash
cargo test -p rhizo-mqtt-contract topic::
```

## Tests required

- Exact-string test per variant.
- Property: round trip for random valid device ids.
- Rejection cases including truncated and over-long topics.

## Documentation impact

- None.

## Files likely affected

```text
crates/mqtt-contract/src/topic.rs
```

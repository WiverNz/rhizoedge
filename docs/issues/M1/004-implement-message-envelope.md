# Issue M1-004 — Implement the message envelope and identity checking

**Milestone:** M1 · **PRD:** [PRD 010](../../prd/010-domain-and-mqtt-protocol.md) · **Depends on:** M1-002, M1-003

## Context

Protocol section 4 defines the envelope every message carries. The duplicated
`device_id` is deliberate: a mismatch between topic and payload means misrouting
or spoofing, and guessing is worse than refusing.

## Goal

Implement `Envelope<T>` with encoding, decoding, and identity validation.

## Scope

- `Envelope<T>` with all fields from protocol section 4
- `to_json` and `from_json`
- `check_identity(topic_device)` rejecting a mismatch
- Version check rejecting `v != 1`
- `#[serde(default)]` on optional fields; unknown fields ignored
- A typed `DecodeError` naming the specific failure

## Non-goals

- Payload types (M1-006 onward).
- Range validation (M1-006).

## Dependencies

- M1-002
- M1-003

## Implementation notes

**Do not** use `deny_unknown_fields`. Forward compatibility (versioning-policy
section 1) requires unknown fields to be ignored, and adding that attribute
would make every additive protocol change breaking.

`DecodeError` variants must map one-to-one onto the `mqtt_decode_errors_total`
reason labels, so the metric is derived from the type rather than from strings
written twice.

`kind` must be checked against the topic as well as `device_id`; a
`telemetry.soil` payload on the status topic is malformed.

## Acceptance criteria

- [ ] A full envelope round-trips.
- [ ] An envelope with an unknown extra field decodes successfully.
- [ ] `v: 2` fails with `UnsupportedVersion`.
- [ ] A payload `device_id` differing from the topic fails with `DeviceMismatch`.
- [ ] A `kind` inconsistent with the topic fails with `KindMismatch`.
- [ ] A missing required field fails with `Envelope`.
- [ ] Every `DecodeError` variant has a distinct metric reason label.

## Verification

```bash
cargo test -p rhizo-mqtt-contract envelope::
```

## Tests required

- Round trip.
- Unknown field tolerated.
- Each rejection case, asserting the exact variant.

## Documentation impact

- None; protocol/mqtt-v1.md is already normative.

## Files likely affected

```text
crates/mqtt-contract/src/envelope.rs
crates/mqtt-contract/src/error.rs
```

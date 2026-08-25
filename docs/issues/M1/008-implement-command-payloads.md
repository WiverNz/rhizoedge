# Issue M1-008 — Implement command and command-result payloads

**Milestone:** M1 · **PRD:** [PRD 010](../../prd/010-domain-and-mqtt-protocol.md) · **Depends on:** M1-004

## Context

Protocol sections 5.8-5.10 define the water, tare, and calibrate commands and
the single result payload. `command_id` is the idempotency key that SAFETY-001
depends on.

## Goal

Implement the command and result payload types.

## Scope

- `WaterCommand`, `TareCommand`, `CalibrateCommand`
- `CommandResult` with status, delivered volume, duration, `clamped`, reason
- `CommandStatus` and `RejectReason` enums
- `RejectReason` decoding unknown values to `Unknown`
- Validation: `requested_ml > 0` and finite, `expires_at > issued_at`

## Non-goals

- Validating whether a command may run (M1-009).
- Issuing commands (M6).

## Dependencies

- M1-004

## Implementation notes

`RejectReason` must carry `#[serde(other)] Unknown` per versioning-policy
section 1, so adding a variant later is non-breaking. Consumers must treat
`Unknown` conservatively.

`CommandResult.delivered_ml` is `Option<f32>` — `interrupted` reports `null`,
meaning genuinely unknown. Do not default it to 0.0: M6-010 credits the full
requested volume for an interrupted dose, and a 0.0 would silently grant extra
daily budget.

## Acceptance criteria

- [ ] All command payloads and the result round-trip.
- [ ] `requested_ml: 0`, negative, and `NaN` are rejected.
- [ ] `expires_at <= issued_at` is rejected.
- [ ] An unknown `reason` string decodes to `Unknown` rather than failing.
- [ ] `delivered_ml` is `Option` and `null` decodes to `None`, not `Some(0.0)`.

## Verification

```bash
cargo test -p rhizo-mqtt-contract payload::command
```

## Tests required

- Round trips.
- Each validation rejection.
- Unknown reason tolerance.
- An explicit test that null delivered_ml is None.

## Documentation impact

- None.

## Files likely affected

```text
crates/mqtt-contract/src/payload/command.rs
```

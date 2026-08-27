# Issue M1-014 — Implement the offline policy payload types

**Milestone:** M1 · **PRD:** [PRD 010](../../prd/010-domain-and-mqtt-protocol.md) · **Depends on:** M1-007

## Context

[ADR-015](../../adr/015-device-offline-autonomy.md) makes the offline policy the
only configuration a device may *act* on alone. It is delivered retained on
`rhizo/v1/devices/{id}/policy` and is specified in
[mqtt-v1.md](../../protocol/mqtt-v1.md) §5.11.

## Goal

Implement the `device.policy` payload types with their validation rules.

## Scope

- `OfflinePolicySet` carrying one policy per plant served by the device
- `OfflinePolicy` with `policy_version`, `enabled`, actuator, control measurement, required/advisory measurements, limits, safety block
- All durations as `u32` milliseconds — device-side monotonic, never wall clock
- Validation: control kind is a recognised **scalar** kind;
  `resume_above > trigger_below`; every duration > 0;
  `dose_ml <= FIRMWARE_MAX_ML_PER_RUN`;
  `dose_ml * max_doses <= max_volume_per_window_ml <= FIRMWARE_MAX_DAILY_ML`
- `enabled` defaults to `false` on deserialisation when absent

## Non-goals

- The evaluator itself (M1-016 defines the types, M6-019 the logic).
- Publishing policies (M6-013 family).
- NVS persistence (M9-015).

## Dependencies

- M1-007

## Implementation notes

`enabled` defaulting to `false` matters: a policy that omits the field must not
grant autonomy. Use `#[serde(default)]` on a `bool`, and assert it in a test —
this is SAFETY-012 applied to deserialisation.

Validation lives here rather than only on the edge because the **device**
re-validates on receipt against its own compile-time limits. The same function
serves both, which is why it belongs in the contract crate.

Durations are milliseconds and monotonic. Do not introduce a `DateTime` anywhere
in this payload: an isolated device has no trustworthy wall clock, and a policy
that needed one would be unusable in exactly the mode it exists for
([ADR-013](../../adr/013-clock-and-time-semantics.md)).

## Acceptance criteria

- [x] `OfflinePolicySet` and `OfflinePolicy` round-trip through JSON.
- [x] An omitted `enabled` deserialises to `false`.
- [x] Each validation rule rejects with its own distinct error variant.
- [x] `dose_ml` above `FIRMWARE_MAX_ML_PER_RUN` is **rejected**, never clamped.
- [x] `resume_above <= trigger_below` is rejected.
- [x] A boolean control kind (`leak_state`) is rejected as `NonScalarControlKind`;
      an unrecognised control kind is rejected as `UnknownControlKind`.
- [x] A **disabled** policy that retains its actuator binding is valid — §5.11
      expresses removal as an `enabled: false` republish, not an omission.
- [x] A policy containing an unrecognised field decodes successfully and ignores it.
- [x] A policy containing `max_ml_per_run` has no representable effect on limits.
- [x] No `chrono` type appears in the policy types.

## Verification

```bash
cargo test -p rhizo-mqtt-contract policy::
cargo build -p rhizo-mqtt-contract --no-default-features --target thumbv7em-none-eabi
```

## Tests required

- Round trip.
- Default-false assertion.
- One test per validation rule.
- Unknown-field tolerance.
- Smuggled-limit no-op.

## Documentation impact

- None; mqtt-v1.md §5.11 is normative.

## Files likely affected

```text
crates/mqtt-contract/src/payload/policy.rs
```

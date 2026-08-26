# Issue M5-016 — Author and validate offline policies

**Milestone:** M5 · **PRD:** [PRD 050](../../prd/050-plant-model-and-recommendations.md) · **Depends on:** M5-013, M5-014

## Context

The edge authors the policy a device may act on alone
([ADR-015](../../adr/015-device-offline-autonomy.md)). A policy the edge cannot
evaluate is a policy it must not publish, which is why `rhizo-domain` links
`rhizo-policy`.

## Goal

Let an operator define an offline policy, and validate it before it can be published.

## Scope

- Derive a candidate `OfflinePolicy` from the plant's bindings and measurement policies
- Validate with `rhizo-policy` plus the contract's hard limits
- Reject: no actuator binding; no control binding; dose above the hard limit; incoherent hysteresis; window exceeding the device daily cap
- Persist to `offline_policies` with a monotonic `policy_version`
- `enabled` defaults to **false**
- REST endpoints to read, update, enable, and disable

## Non-goals

- Publishing to the device (M6-013 family).
- Device-side handling (M2-016, M9-015).

## Dependencies

- M5-013
- M5-014

## Implementation notes

Validating with the **same crate the device will use** is the point. If the edge
validated with its own rules, it could publish a policy the device then rejects,
leaving a plant with autonomy that silently never activates.

A plant with no `ActuatorBinding` cannot have an offline policy at all — reject
at authoring time with a specific message rather than letting an operator
configure autonomy that can never run (SAFETY-018).

`enabled` defaults false. Creating a policy is not the same act as authorising a
device to water unsupervised, and the two should require separate decisions.

## Acceptance criteria

- [ ] A valid policy is authored, validated, versioned, and persisted.
- [ ] `policy_version` increases monotonically.
- [ ] A policy for a plant with no actuator is **rejected** with a specific error.
- [ ] A dose above `FIRMWARE_MAX_ML_PER_RUN` is rejected, not clamped.
- [ ] A newly created policy has `enabled: false`.
- [ ] Validation uses `rhizo-policy`, not a second rule set.
- [ ] Required measurements are derived from `required`-role bindings.

## Verification

```bash
cargo test -p rhizo-domain offline_policy::
cargo test -p edge-controller api::offline_policy
```

## Tests required

- Each rejection rule.
- Version monotonicity.
- Default-disabled.
- Shared-validator assertion.

## Documentation impact

- http-api-boundaries.md offline policy endpoints.

## Files likely affected

```text
crates/domain/src/offline_policy.rs
crates/edge-controller/src/api/offline_policy.rs
```

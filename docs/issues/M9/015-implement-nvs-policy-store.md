# Issue M9-015 — Implement the NVS offline policy store with atomic activation

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-004

## Context

SAFETY-019 on real flash. Power loss during a policy write is not exotic; it is
the normal way an unmaintained device eventually fails, and a half-written policy
taking effect is the failure this store exists to prevent.

## Goal

Persist and activate offline policies atomically on NVS.

## Scope

- Separate `policy_active` and `policy_staging` regions, each CRC-protected
- Validate → stage → verify read-back → atomic activate → acknowledge
- Ignore a policy whose `policy_version` is <= applied
- Corrupt or missing store refuses to activate; no default is substituted
- Report `applied_policy_versions` in `device.status`
- Host tests with the fake `NvsStore`, including interruption at every step

## Non-goals

- Evaluating the policy (M9-016).

## Dependencies

- M9-004

## Implementation notes

The active pointer flip must be the single atomic operation. Everything before it
is non-destructive, so an interruption anywhere leaves the previous policy intact;
after it, the new policy is complete. Test by interrupting at each step index and
asserting exactly one valid policy is active afterwards.

Flash wear is worth a thought: policies change rarely, so a two-region scheme with
CRC is fine. Do not write the policy on every boot.

A corrupt store must **refuse**, not fall back to a built-in default. A default
threshold nobody authorised is precisely what SAFETY-013 forbids.

## Acceptance criteria

- [ ] A valid policy is staged, verified, activated, and acknowledged.
- [ ] Interruption at every step leaves exactly one valid active policy.
- [ ] A lower or equal `policy_version` is ignored.
- [ ] A corrupt store refuses to activate and substitutes no default.
- [ ] An invalid policy leaves the previous one active.
- [ ] `applied_policy_versions` is reported accurately.
- [ ] All of the above are covered by host tests with no board.

## Verification

```bash
cd firmware/esp32-node && cargo test policy::
cargo test safety_019
```

## Tests required

- Interruption at every step.
- Version monotonicity.
- Corrupt-store refusal.
- Validation rejections.

## Documentation impact

- ADR-015 §7 verified against the implementation.

## Files likely affected

```text
firmware/esp32-node/src/nvs.rs
firmware/esp32-node/src/app/policy.rs
```
